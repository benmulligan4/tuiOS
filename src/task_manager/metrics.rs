/// Always-on system sampler shared by the Task Manager, the status bar and Dashboard 2.
///
/// One background thread owns the `sysinfo` handles and the GPU probes and publishes
/// rolling histories behind a lock, so the UI thread never samples anything itself.
/// (The status bar used to build a fresh `System` every frame, which was slow and always
/// read ~0 % CPU because usage needs two refreshes some time apart.)

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, Once, RwLock, RwLockReadGuard};
use std::thread;
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use sysinfo::{Components, Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::config::{self, TmConfig};
use super::gpu::{self, GpuProbe, GpuReading};

/// Fixed-capacity history, oldest sample first.
#[derive(Debug, Clone)]
pub struct Ring<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T> Ring<T> {
    pub fn new(cap: usize) -> Self {
        Self { buf: VecDeque::new(), cap: cap.max(1) }
    }

    pub fn push(&mut self, value: T) {
        while self.buf.len() >= self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(value);
    }

    pub fn last(&self) -> Option<&T> {
        self.buf.back()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// The newest `n` samples, oldest first.
    pub fn tail(&self, n: usize) -> impl Iterator<Item = &T> {
        self.buf.iter().skip(self.buf.len().saturating_sub(n))
    }

    pub fn set_capacity(&mut self, cap: usize) {
        self.cap = cap.max(1);
        while self.buf.len() > self.cap {
            self.buf.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

#[derive(Debug, Clone)]
pub struct GpuStat {
    pub key: String,
    pub name: String,
    pub backend: &'static str,
    pub util: Ring<Option<f32>>,
    pub vram: Ring<Option<u64>>,
    pub vram_total: Option<u64>,
    pub temp: Option<f32>,
    pub clock_mhz: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct IfaceStat {
    pub name: String,
    pub mac: String,
    pub ips: Vec<String>,
    /// Bytes per second.
    pub rx_rate: Ring<f64>,
    pub tx_rate: Ring<f64>,
    /// Since the interface came up (≈ since boot).
    pub rx_total: u64,
    pub tx_total: u64,
    /// Since tuiOS started.
    pub rx_session: u64,
    pub tx_session: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    baseline: (u64, u64),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcSample {
    /// Share of the whole machine (all cores), 0-100.
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Debug, Clone)]
pub struct ProcHist {
    pub cpu: Ring<f32>,
    pub mem: Ring<u64>,
}

/// One running process, kept only while something is waiting to find a new window's PID.
#[derive(Debug, Clone)]
pub struct ProcEntry {
    pub pid: u32,
    /// Lower-case executable name without extension.
    pub stem: String,
    pub start: u64,
}

/// `vcgencmd get_throttled` flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PiHealth(pub u32);

impl PiHealth {
    pub fn under_voltage(&self) -> bool { self.0 & 0x1 != 0 }
    pub fn freq_capped(&self) -> bool { self.0 & 0x2 != 0 }
    pub fn throttled(&self) -> bool { self.0 & 0x4 != 0 }
    pub fn soft_temp_limit(&self) -> bool { self.0 & 0x8 != 0 }
    pub fn under_voltage_occurred(&self) -> bool { self.0 & 0x1_0000 != 0 }
    pub fn throttled_occurred(&self) -> bool { self.0 & 0x4_0000 != 0 }

    /// Problems happening right now, worst first.
    pub fn active_problems(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.under_voltage() { out.push("Under-voltage"); }
        if self.throttled() { out.push("Throttled"); }
        if self.freq_capped() { out.push("Frequency capped"); }
        if self.soft_temp_limit() { out.push("Soft temperature limit"); }
        out
    }
}

pub struct Metrics {
    pub ready: bool,
    pub interval_secs: f64,
    pub history_secs: u64,
    pub cpu_brand: String,
    pub physical_cores: Option<usize>,
    pub cpu: Ring<f32>,
    pub cores: Vec<Ring<f32>>,
    pub core_freq: Vec<u64>,
    pub mem_total: u64,
    pub mem_used: Ring<u64>,
    pub swap_total: u64,
    pub swap_used: u64,
    pub cpu_temp: Ring<Option<f32>>,
    pub gpu_temp: Ring<Option<f32>>,
    pub gpus: Vec<GpuStat>,
    pub ifaces: Vec<IfaceStat>,
    pub net_rx: Ring<f64>,
    pub net_tx: Ring<f64>,
    pub disk_used: u64,
    pub disk_total: u64,
    pub uptime: u64,
    pub process_count: usize,
    pub self_pid: u32,
    /// Latest usage of each watched PID (with its children when asked).
    pub procs: HashMap<u32, ProcSample>,
    pub proc_hist: HashMap<u32, ProcHist>,
    pub proc_index: Vec<ProcEntry>,
    pub pi: Option<PiHealth>,
}

impl Metrics {
    fn empty() -> Self {
        Self {
            ready: false,
            interval_secs: 1.0,
            history_secs: 900,
            cpu_brand: String::new(),
            physical_cores: None,
            cpu: Ring::new(900),
            cores: Vec::new(),
            core_freq: Vec::new(),
            mem_total: 0,
            mem_used: Ring::new(900),
            swap_total: 0,
            swap_used: 0,
            cpu_temp: Ring::new(900),
            gpu_temp: Ring::new(900),
            gpus: Vec::new(),
            ifaces: Vec::new(),
            net_rx: Ring::new(900),
            net_tx: Ring::new(900),
            disk_used: 0,
            disk_total: 0,
            uptime: 0,
            process_count: 0,
            self_pid: 0,
            procs: HashMap::new(),
            proc_hist: HashMap::new(),
            proc_index: Vec::new(),
            pi: None,
        }
    }

    fn capacity(&self) -> usize {
        ((self.history_secs as f64 / self.interval_secs).ceil() as usize).max(2)
    }

    /// How many samples cover `secs` of history.
    pub fn samples_for(&self, secs: u64) -> usize {
        ((secs as f64 / self.interval_secs).ceil() as usize).max(2)
    }

    pub fn cpu_now(&self) -> f32 {
        self.cpu.last().copied().unwrap_or(0.0)
    }

    pub fn mem_now(&self) -> u64 {
        self.mem_used.last().copied().unwrap_or(0)
    }

    pub fn mem_pct(&self) -> f32 {
        if self.mem_total == 0 { 0.0 } else { self.mem_now() as f32 / self.mem_total as f32 * 100.0 }
    }

    /// Busiest GPU right now, for one-line summaries.
    pub fn gpu_now(&self) -> Option<f32> {
        self.gpus
            .iter()
            .filter_map(|g| g.util.last().copied().flatten())
            .fold(None, |acc: Option<f32>, v| Some(acc.map_or(v, |a| a.max(v))))
    }

    /// Interfaces worth listing: ones with an IPv4 address or traffic since tuiOS started
    /// (hides the idle virtual adapters every OS creates).
    pub fn shown_ifaces(&self) -> impl Iterator<Item = &IfaceStat> {
        self.ifaces
            .iter()
            .filter(|i| i.ips.iter().any(|ip| !ip.contains(':')) || i.rx_session > 0 || i.tx_session > 0)
    }

    /// (session ↓, session ↑, since boot ↓, since boot ↑) across all listed interfaces.
    pub fn net_totals(&self) -> (u64, u64, u64, u64) {
        self.shown_ifaces().fold((0, 0, 0, 0), |acc, i| {
            (acc.0 + i.rx_session, acc.1 + i.tx_session, acc.2 + i.rx_total, acc.3 + i.tx_total)
        })
    }

    fn resize(&mut self) {
        let cap = self.capacity();
        self.cpu.set_capacity(cap);
        self.cores.iter_mut().for_each(|r| r.set_capacity(cap));
        self.mem_used.set_capacity(cap);
        self.cpu_temp.set_capacity(cap);
        self.gpu_temp.set_capacity(cap);
        self.net_rx.set_capacity(cap);
        self.net_tx.set_capacity(cap);
        for g in &mut self.gpus {
            g.util.set_capacity(cap);
            g.vram.set_capacity(cap);
        }
        for i in &mut self.ifaces {
            i.rx_rate.set_capacity(cap);
            i.tx_rate.set_capacity(cap);
        }
        for h in self.proc_hist.values_mut() {
            h.cpu.set_capacity(cap);
            h.mem.set_capacity(cap);
        }
    }

    /// Samples taken at a different interval would stretch the time axis, so drop them.
    fn clear_history(&mut self) {
        self.cpu.clear();
        self.cores.iter_mut().for_each(Ring::clear);
        self.mem_used.clear();
        self.cpu_temp.clear();
        self.gpu_temp.clear();
        self.net_rx.clear();
        self.net_tx.clear();
        for g in &mut self.gpus {
            g.util.clear();
            g.vram.clear();
        }
        for i in &mut self.ifaces {
            i.rx_rate.clear();
            i.tx_rate.clear();
        }
        self.proc_hist.clear();
    }
}

#[derive(Debug, Clone, Default)]
struct Watch {
    /// (pid, include its child processes)
    pids: Vec<(u32, bool)>,
    discover: bool,
    /// The Task Manager is open: measure processes every tick.
    detailed: bool,
}

lazy_static! {
    static ref METRICS: RwLock<Metrics> = RwLock::new(Metrics::empty());
    static ref WATCH: Mutex<Watch> = Mutex::new(Watch::default());
    static ref ALERTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
}

static START: Once = Once::new();
static STARTED_AT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Start the sampler thread (idempotent — tuiOS restarts re-enter `run_app`).
pub fn start() {
    STARTED_AT.get_or_init(Instant::now);
    START.call_once(|| {
        let _ = thread::Builder::new()
            .name("tuios-metrics".into())
            .spawn(run);
    });
}

/// How long this tuiOS process has been running.
pub fn tuios_uptime() -> Duration {
    STARTED_AT.get().map(|t| t.elapsed()).unwrap_or_default()
}

pub fn read() -> RwLockReadGuard<'static, Metrics> {
    METRICS.read().unwrap_or_else(|e| e.into_inner())
}

/// PIDs whose usage the Task Manager shows. `discover` keeps a full process index
/// so a freshly opened terminal window's app can be found by name; `detailed` (the
/// Task Manager is on screen) samples them every tick rather than on the background
/// scan interval from Settings.
pub fn watch(pids: Vec<(u32, bool)>, discover: bool, detailed: bool) {
    if let Ok(mut w) = WATCH.lock() {
        w.pids = pids;
        w.discover = discover;
        w.detailed = detailed;
    }
}

/// Resource / Raspberry Pi warnings raised since the last call.
pub fn take_alerts() -> Vec<String> {
    ALERTS.lock().map(|mut a| std::mem::take(&mut *a)).unwrap_or_default()
}

fn raise(msg: String) {
    crate::utilities::logging::warn(&format!("Task Manager alert: {}", msg));
    if let Ok(mut a) = ALERTS.lock() {
        a.push(msg);
    }
}

/// The newest process called `stem` that started at or after `since` (unix secs),
/// skipping PIDs that already belong to a session.
pub fn find_process(stem: &str, since: u64, exclude: &HashSet<u32>) -> Option<(u32, u64)> {
    let m = read();
    m.proc_index
        .iter()
        .filter(|p| p.start + 2 >= since && !exclude.contains(&p.pid) && stem_matches(&p.stem, stem))
        .max_by_key(|p| p.start)
        .map(|p| (p.pid, p.start))
}

/// Linux truncates process names to 15 characters, so a prefix match counts there.
fn stem_matches(found: &str, wanted: &str) -> bool {
    found == wanted || (found.len() == 15 && wanted.starts_with(found))
}

pub fn file_stem_lower(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

// ── Sampler thread ───────────────────────────────────────────────────────────

fn cpu_temp_label(l: &str) -> bool {
    ["cpu", "package", "tctl", "tdie", "core", "soc", "k10temp", "coretemp", "acpitz"]
        .iter()
        .any(|k| l.contains(k))
}

fn gpu_temp_label(l: &str) -> bool {
    ["gpu", "amdgpu", "edge", "junction", "nouveau", "radeon"].iter().any(|k| l.contains(k))
}

/// Hottest CPU-ish and GPU-ish sensors.
fn classify_temps(components: &Components) -> (Option<f32>, Option<f32>) {
    let mut cpu: Option<f32> = None;
    let mut gpu: Option<f32> = None;
    for c in components.list() {
        let t = c.temperature();
        if !t.is_finite() || t <= 0.0 || t > 150.0 {
            continue;
        }
        let l = c.label().to_lowercase();
        if gpu_temp_label(&l) {
            gpu = Some(gpu.map_or(t, |g| g.max(t)));
        } else if cpu_temp_label(&l) {
            cpu = Some(cpu.map_or(t, |v| v.max(t)));
        }
    }
    (cpu, gpu)
}

fn is_loopback(name: &str) -> bool {
    let l = name.to_lowercase();
    l == "lo" || l.starts_with("lo0") || l.contains("loopback")
}

fn root_disk(disks: &Disks) -> (u64, u64) {
    let cwd = std::env::current_dir().unwrap_or_default();
    disks
        .list()
        .iter()
        .filter(|d| cwd.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.total_space().saturating_sub(d.available_space()), d.total_space()))
        .unwrap_or((0, 0))
}

#[derive(Default)]
struct AlertState {
    cpu_since: Option<Instant>,
    ram_since: Option<Instant>,
    temp_since: Option<Instant>,
    fired: HashMap<&'static str, Instant>,
    pi_last: Option<u32>,
}

const ALERT_COOLDOWN: Duration = Duration::from_secs(300);

impl AlertState {
    /// Fire once a value has stayed over its threshold for the sustain time, then stay
    /// quiet for the cooldown so a busy machine does not get a toast every second.
    fn check(&mut self, key: &'static str, over: bool, cfg: &TmConfig, msg: impl FnOnce() -> String) {
        let since = match key {
            "cpu" => &mut self.cpu_since,
            "ram" => &mut self.ram_since,
            _ => &mut self.temp_since,
        };
        if !over {
            *since = None;
            return;
        }
        let start = *since.get_or_insert_with(Instant::now);
        if start.elapsed() < cfg.alert_sustain {
            return;
        }
        if self.fired.get(key).map_or(false, |t| t.elapsed() < ALERT_COOLDOWN) {
            return;
        }
        self.fired.insert(key, Instant::now());
        raise(msg());
    }

    fn check_pi(&mut self, flags: u32, cfg: &TmConfig) {
        let previous = self.pi_last.replace(flags).unwrap_or(0);
        if !cfg.pi_power_alerts {
            return;
        }
        let newly = flags & !previous & 0xF;
        let h = PiHealth(newly);
        if h.under_voltage() {
            raise("Raspberry Pi under-voltage detected — check the power supply".into());
        }
        if h.throttled() {
            raise("Raspberry Pi CPU is being throttled".into());
        }
        if h.soft_temp_limit() {
            raise("Raspberry Pi reached its soft temperature limit".into());
        }
    }
}

fn run() {
    let mut cfg = config::load();
    let mut sys = System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let mut networks = Networks::new_with_refreshed_list();
    let mut components = Components::new_with_refreshed_list();
    let mut disks = Disks::new_with_refreshed_list();
    let mut probe = GpuProbe::detect(cfg.nvml);
    let pi = gpu::is_raspberry_pi();
    let self_pid = sysinfo::get_current_pid().map(|p| p.as_u32()).unwrap_or(0);
    let mut alerts = AlertState::default();

    {
        let mut m = METRICS.write().unwrap_or_else(|e| e.into_inner());
        m.interval_secs = cfg.interval.as_secs_f64();
        m.history_secs = cfg.history_secs;
        let cap = m.capacity();
        m.cpu_brand = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
        m.physical_cores = sys.physical_core_count();
        m.cores = (0..sys.cpus().len()).map(|_| Ring::new(cap)).collect();
        m.self_pid = self_pid;
        m.resize();
    }

    let mut tick: u64 = 0;
    let mut last = Instant::now();
    let mut last_scan: Option<Instant> = None;
    loop {
        // CPU usage needs two refreshes at least 200 ms apart; the interval covers that
        thread::sleep(cfg.interval);
        let elapsed = last.elapsed().as_secs_f64().max(0.001);
        last = Instant::now();
        tick += 1;

        let mut interval_changed = false;
        let mut history_changed = false;
        if tick % 10 == 0 {
            let fresh = config::load();
            interval_changed = fresh.interval != cfg.interval;
            history_changed = fresh.history_secs != cfg.history_secs;
            if fresh.nvml != cfg.nvml {
                probe = GpuProbe::detect(fresh.nvml);
            }
            cfg = fresh;
        }

        sys.refresh_cpu_usage();
        if tick % 5 == 1 {
            sys.refresh_cpu_frequency();
        }
        sys.refresh_memory();
        if tick % 30 == 0 {
            networks.refresh_list();
        } else {
            networks.refresh();
        }
        if tick % 2 == 1 {
            components.refresh();
        }
        if tick % 60 == 0 {
            components.refresh_list();
        }
        if tick % 30 == 1 {
            disks.refresh_list();
        }
        let gpus = probe.sample();
        let pi_flags = if pi && tick % 5 == 1 { gpu::read_pi_throttled() } else { None };

        let watch = WATCH.lock().map(|w| w.clone()).unwrap_or_default();
        let mut samples: HashMap<u32, ProcSample> = HashMap::new();
        let mut index: Vec<ProcEntry> = Vec::new();
        let mut process_count = None;
        // A full process scan is the most expensive part of a tick (noticeable on a Pi)
        let background_due = cfg.background_scan.map_or(false, |every| {
            // Half a tick of slack so sleep jitter does not push a scan a whole tick late
            last_scan.map_or(true, |t| t.elapsed() + cfg.interval / 2 >= every)
        });
        let scan = watch.discover || (!watch.pids.is_empty() && (watch.detailed || background_due));
        if scan {
            last_scan = Some(Instant::now());
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::new()
                    .with_cpu()
                    .with_memory()
                    .with_exe(UpdateKind::OnlyIfNotSet),
            );
            let ncpu = sys.cpus().len().max(1) as f32;
            // Linux lists threads as processes too; they would double-count memory
            let real = |p: &sysinfo::Process| p.thread_kind().is_none();
            let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
            if watch.pids.iter().any(|(_, tree)| *tree) {
                for (pid, p) in sys.processes() {
                    if let (Some(parent), true) = (p.parent(), real(p)) {
                        children.entry(parent.as_u32()).or_default().push(pid.as_u32());
                    }
                }
            }
            for (root, tree) in &watch.pids {
                let mut cpu = 0.0;
                let mut mem = 0;
                let mut found = false;
                let mut stack = vec![*root];
                let mut seen = HashSet::new();
                while let Some(pid) = stack.pop() {
                    if !seen.insert(pid) {
                        continue;
                    }
                    if let Some(p) = sys.process(Pid::from_u32(pid)) {
                        found = true;
                        cpu += p.cpu_usage();
                        mem += p.memory();
                    }
                    if *tree {
                        if let Some(kids) = children.get(&pid) {
                            stack.extend(kids);
                        }
                    }
                }
                if found {
                    samples.insert(*root, ProcSample { cpu: (cpu / ncpu).min(100.0), mem });
                }
            }
            if watch.discover {
                index = sys
                    .processes()
                    .values()
                    .filter(|p| real(p))
                    .map(|p| {
                        let stem = p
                            .exe()
                            .and_then(|e| e.file_stem())
                            .map(|s| s.to_string_lossy().to_lowercase())
                            .unwrap_or_else(|| file_stem_lower(&p.name().to_string_lossy()));
                        ProcEntry { pid: p.pid().as_u32(), stem, start: p.start_time() }
                    })
                    .collect();
            }
            process_count = Some(sys.processes().values().filter(|p| real(p)).count());
        }

        let (cpu_temp, comp_gpu_temp) = classify_temps(&components);
        let backend_gpu_temp = gpus
            .iter()
            .filter_map(|g| g.temp)
            .fold(None, |acc: Option<f32>, t| Some(acc.map_or(t, |a| a.max(t))));
        let gpu_temp = backend_gpu_temp.or(comp_gpu_temp);
        let cpu_now = sys.global_cpu_usage();
        let mem_pct = if sys.total_memory() == 0 {
            0.0
        } else {
            sys.used_memory() as f32 / sys.total_memory() as f32 * 100.0
        };

        {
            let mut m = METRICS.write().unwrap_or_else(|e| e.into_inner());
            if interval_changed || history_changed {
                m.interval_secs = cfg.interval.as_secs_f64();
                m.history_secs = cfg.history_secs;
                if interval_changed {
                    m.clear_history();
                }
                m.resize();
            }
            let cap = m.capacity();
            m.cpu.push(cpu_now);
            if m.cores.len() != sys.cpus().len() {
                m.cores = (0..sys.cpus().len()).map(|_| Ring::new(cap)).collect();
            }
            for (ring, core) in m.cores.iter_mut().zip(sys.cpus()) {
                ring.push(core.cpu_usage());
            }
            m.core_freq = sys.cpus().iter().map(|c| c.frequency()).collect();
            m.mem_total = sys.total_memory();
            m.mem_used.push(sys.used_memory());
            m.swap_total = sys.total_swap();
            m.swap_used = sys.used_swap();
            m.cpu_temp.push(cpu_temp);
            m.gpu_temp.push(gpu_temp);
            update_gpus(&mut m.gpus, gpus, cap);
            update_networks(&mut m, &networks, elapsed, cap);
            if tick % 30 == 1 {
                let (used, total) = root_disk(&disks);
                m.disk_used = used;
                m.disk_total = total;
            }
            m.uptime = System::uptime();
            if let Some(count) = process_count {
                m.process_count = count;
            }
            let watched: HashSet<u32> = watch.pids.iter().map(|(p, _)| *p).collect();
            m.proc_hist.retain(|pid, _| watched.contains(pid));
            if scan {
                m.procs = samples;
                let procs: Vec<(u32, ProcSample)> = m.procs.iter().map(|(k, v)| (*k, *v)).collect();
                for (pid, s) in procs {
                    let h = m
                        .proc_hist
                        .entry(pid)
                        .or_insert_with(|| ProcHist { cpu: Ring::new(cap), mem: Ring::new(cap) });
                    h.cpu.push(s.cpu);
                    h.mem.push(s.mem);
                }
                m.proc_index = index;
            }
            if let Some(flags) = pi_flags {
                m.pi = Some(PiHealth(flags));
            }
            m.ready = true;
        }

        if cfg.alerts {
            alerts.check("cpu", cpu_now >= cfg.alert_cpu, &cfg, || {
                format!("High CPU usage: {:.0}% for over {} s", cpu_now, cfg.alert_sustain.as_secs())
            });
            alerts.check("ram", mem_pct >= cfg.alert_ram, &cfg, || {
                format!("High memory usage: {:.0}% for over {} s", mem_pct, cfg.alert_sustain.as_secs())
            });
            let hottest = cpu_temp.into_iter().chain(gpu_temp).fold(f32::MIN, f32::max);
            alerts.check("temp", hottest >= cfg.alert_temp, &cfg, || {
                format!("High temperature: {:.0} °C", hottest)
            });
        }
        if let Some(flags) = pi_flags {
            alerts.check_pi(flags, &cfg);
        }
    }
}

fn update_gpus(stats: &mut Vec<GpuStat>, readings: Vec<GpuReading>, cap: usize) {
    // Keep each GPU's history across samples, keyed by its stable id
    let mut previous: HashMap<String, GpuStat> = stats.drain(..).map(|g| (g.key.clone(), g)).collect();
    for r in readings {
        let mut g = previous.remove(&r.key).unwrap_or_else(|| GpuStat {
            key: r.key.clone(),
            name: String::new(),
            backend: r.backend,
            util: Ring::new(cap),
            vram: Ring::new(cap),
            vram_total: None,
            temp: None,
            clock_mhz: None,
        });
        g.name = super::clean_text(&r.name);
        g.backend = r.backend;
        g.util.push(r.util);
        g.vram.push(r.vram_used);
        g.vram_total = r.vram_total;
        g.temp = r.temp;
        g.clock_mhz = r.clock_mhz;
        stats.push(g);
    }
}

fn update_networks(m: &mut Metrics, networks: &Networks, elapsed: f64, cap: usize) {
    let mut previous: HashMap<String, IfaceStat> = m.ifaces.drain(..).map(|i| (i.name.clone(), i)).collect();
    let mut rx_sum = 0.0;
    let mut tx_sum = 0.0;
    let mut list: Vec<IfaceStat> = Vec::new();
    for (name, data) in networks.list() {
        if is_loopback(name) {
            continue;
        }
        let mut ips: Vec<String> = data
            .ip_networks()
            .iter()
            .filter(|n| !n.addr.is_loopback())
            .map(|n| format!("{}/{}", n.addr, n.prefix))
            .collect();
        // IPv4 first — it is the one people look for
        ips.sort_by_key(|ip| ip.contains(':'));
        let has_ipv4 = ips.iter().any(|ip| !ip.contains(':'));
        let rx_total = data.total_received();
        let tx_total = data.total_transmitted();
        if !has_ipv4 && rx_total == 0 && tx_total == 0 {
            continue;
        }
        let mut stat = previous.remove(name).unwrap_or_else(|| IfaceStat {
            name: name.clone(),
            mac: String::new(),
            ips: Vec::new(),
            rx_rate: Ring::new(cap),
            tx_rate: Ring::new(cap),
            rx_total,
            tx_total,
            rx_session: 0,
            tx_session: 0,
            rx_packets: 0,
            tx_packets: 0,
            rx_errors: 0,
            tx_errors: 0,
            baseline: (rx_total, tx_total),
        });
        let rx = data.received() as f64 / elapsed;
        let tx = data.transmitted() as f64 / elapsed;
        rx_sum += rx;
        tx_sum += tx;
        stat.rx_rate.push(rx);
        stat.tx_rate.push(tx);
        stat.mac = if data.mac_address().is_unspecified() { String::new() } else { data.mac_address().to_string() };
        stat.ips = ips;
        // Counters can reset (interface re-plugged); rebase instead of going negative
        if rx_total < stat.baseline.0 || tx_total < stat.baseline.1 {
            stat.baseline = (rx_total.saturating_sub(stat.rx_session), tx_total.saturating_sub(stat.tx_session));
        }
        stat.rx_total = rx_total;
        stat.tx_total = tx_total;
        stat.rx_session = rx_total - stat.baseline.0;
        stat.tx_session = tx_total - stat.baseline.1;
        stat.rx_packets = data.total_packets_received();
        stat.tx_packets = data.total_packets_transmitted();
        stat.rx_errors = data.total_errors_on_received();
        stat.tx_errors = data.total_errors_on_transmitted();
        list.push(stat);
    }
    list.sort_by(|a, b| {
        let a_v4 = a.ips.iter().any(|ip| !ip.contains(':'));
        let b_v4 = b.ips.iter().any(|ip| !ip.contains(':'));
        b_v4.cmp(&a_v4).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    m.ifaces = list;
    m.net_rx.push(rx_sum);
    m.net_tx.push(tx_sum);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_newest() {
        let mut r = Ring::new(3);
        for i in 0..5 {
            r.push(i);
        }
        assert_eq!(r.tail(10).copied().collect::<Vec<_>>(), vec![2, 3, 4]);
        assert_eq!(r.tail(2).copied().collect::<Vec<_>>(), vec![3, 4]);
        r.set_capacity(1);
        assert_eq!(r.last(), Some(&4));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn truncated_linux_names_match() {
        assert!(stem_matches("connect-four-pl", "connect-four-play"));
        assert!(stem_matches("sigye", "sigye"));
        assert!(!stem_matches("sig", "sigye"));
    }
}
