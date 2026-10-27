/// Live system metrics for Dashboard 1.
///
/// Dashboards are drawn from a stateless `render()` call, so everything that has
/// to survive between frames — the `sysinfo` handles, the rolling history and the
/// previous network byte counters — lives here instead. Sampling runs on its own
/// thread so a slow process/disk refresh never stalls a frame.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use sysinfo::{Components, Disks, Networks, ProcessesToUpdate, System};

/// Samples retained for the history sparklines.
pub const HISTORY_LEN: usize = 320;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(800);
/// Process/disk/sensor refreshes are expensive, so they run every Nth sample.
const SLOW_EVERY: u32 = 4;
const BATTERY_EVERY: u32 = 12;
/// The sampler shuts down once the dashboard has not been drawn for this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const TOP_N: usize = 10;

#[derive(Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
}

#[derive(Clone, Default)]
pub struct NetInfo {
    pub iface: String,
    pub ip: Option<String>,
    pub mac: Option<String>,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub iface_count: usize,
}

#[derive(Clone, Default)]
pub struct BatteryInfo {
    pub percent: f32,
    pub state: String,
    /// Seconds until full when charging, until empty when discharging.
    pub remaining: Option<u64>,
    pub health: Option<f32>,
    pub power_w: f32,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub cycles: Option<u32>,
    pub temp_c: Option<f32>,
}

#[derive(Clone, Default)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub os_version: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_brand: String,
    pub cpu_cores: usize,
    pub cpu_physical: Option<usize>,
    pub cpu_mhz: u64,
}

#[derive(Clone, Default)]
pub struct DiskInfo {
    pub mount: String,
    pub fs: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub ready: bool,
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk: DiskInfo,
    pub load: Option<(f64, f64, f64)>,
    pub uptime: u64,
    pub boot_time: u64,
    pub process_count: usize,
    pub top_cpu: Vec<ProcInfo>,
    pub top_mem: Vec<ProcInfo>,
    pub cpu_history: Vec<u64>,
    pub mem_history: Vec<u64>,
    pub net_history: Vec<u64>,
    pub net: NetInfo,
    pub battery: Option<BatteryInfo>,
    pub temp: Option<(String, f32)>,
    pub host: HostInfo,
}

impl Snapshot {
    pub fn mem_pct(&self) -> f64 {
        pct(self.mem_used, self.mem_total)
    }
    pub fn swap_pct(&self) -> f64 {
        pct(self.swap_used, self.swap_total)
    }
    pub fn disk_pct(&self) -> f64 {
        pct(self.disk.used, self.disk.total)
    }
}

fn pct(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64) * 100.0
    }
}

lazy_static! {
    static ref SNAPSHOT: Mutex<Snapshot> = Mutex::new(Snapshot::default());
    static ref LAST_READ: Mutex<Option<Instant>> = Mutex::new(None);
    static ref SAMPLER_RUNNING: AtomicBool = AtomicBool::new(false);
}

/// Latest sampled metrics. Starts the background sampler on first call and keeps
/// it alive for as long as the dashboard keeps asking for data.
pub fn snapshot() -> Snapshot {
    *LAST_READ.lock().unwrap() = Some(Instant::now());
    ensure_sampler();
    SNAPSHOT.lock().unwrap().clone()
}

fn ensure_sampler() {
    if SAMPLER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut collector = Collector::new();
        let mut tick: u32 = 0;
        loop {
            collector.sample(tick);
            *SNAPSHOT.lock().unwrap() = collector.snapshot.clone();
            tick = tick.wrapping_add(1);
            std::thread::sleep(SAMPLE_INTERVAL);

            let idle = LAST_READ
                .lock()
                .unwrap()
                .map_or(true, |t| t.elapsed() > IDLE_TIMEOUT);
            if idle {
                SAMPLER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        }
    });
}

struct Collector {
    sys: System,
    networks: Networks,
    components: Components,
    cpu_history: VecDeque<u64>,
    mem_history: VecDeque<u64>,
    net_history: VecDeque<u64>,
    last_net: Option<Instant>,
    snapshot: Snapshot,
}

impl Collector {
    fn new() -> Self {
        Self {
            sys: System::new(),
            networks: Networks::new_with_refreshed_list(),
            components: Components::new_with_refreshed_list(),
            cpu_history: VecDeque::with_capacity(HISTORY_LEN),
            mem_history: VecDeque::with_capacity(HISTORY_LEN),
            net_history: VecDeque::with_capacity(HISTORY_LEN),
            last_net: None,
            snapshot: Snapshot {
                host: host_info(),
                ..Default::default()
            },
        }
    }

    fn sample(&mut self, tick: u32) {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();

        let s = &mut self.snapshot;
        s.cpu = self.sys.global_cpu_usage();
        s.cores = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        s.mem_used = self.sys.used_memory();
        s.mem_total = self.sys.total_memory();
        s.swap_used = self.sys.used_swap();
        s.swap_total = self.sys.total_swap();
        s.uptime = System::uptime();
        s.boot_time = System::boot_time();

        let load = System::load_average();
        s.load = if load.one == 0.0 && load.five == 0.0 && load.fifteen == 0.0 {
            None
        } else {
            Some((load.one, load.five, load.fifteen))
        };

        if s.host.cpu_mhz == 0 {
            if let Some(cpu) = self.sys.cpus().first() {
                s.host.cpu_mhz = cpu.frequency();
                if s.host.cpu_brand.is_empty() {
                    s.host.cpu_brand = cpu.brand().trim().to_string();
                }
            }
            s.host.cpu_cores = self.sys.cpus().len();
            s.host.cpu_physical = self.sys.physical_core_count();
        }

        push(&mut self.cpu_history, s.cpu.clamp(0.0, 100.0) as u64);
        push(&mut self.mem_history, s.mem_pct().round() as u64);
        s.cpu_history = self.cpu_history.iter().copied().collect();
        s.mem_history = self.mem_history.iter().copied().collect();

        self.sample_network();

        if tick % SLOW_EVERY == 0 {
            self.sample_processes();
            self.sample_disk();
            self.sample_temperature();
        }
        if tick % BATTERY_EVERY == 0 {
            self.snapshot.battery = read_battery();
        }

        self.snapshot.ready = true;
    }

    fn sample_network(&mut self) {
        self.networks.refresh();
        let elapsed = self
            .last_net
            .map(|t| t.elapsed().as_secs_f64())
            .filter(|s| *s > 0.05)
            .unwrap_or(SAMPLE_INTERVAL.as_secs_f64());
        self.last_net = Some(Instant::now());

        let mut best: Option<(&String, &sysinfo::NetworkData)> = None;
        let mut rx_delta = 0u64;
        let mut tx_delta = 0u64;
        let mut count = 0usize;
        for (name, data) in self.networks.list() {
            if is_loopback(name) {
                continue;
            }
            count += 1;
            rx_delta += data.received();
            tx_delta += data.transmitted();
            let better = best.map_or(true, |(_, b)| {
                data.total_received() + data.total_transmitted()
                    > b.total_received() + b.total_transmitted()
            });
            if better {
                best = Some((name, data));
            }
        }

        let mut info = NetInfo {
            rx_rate: rx_delta as f64 / elapsed,
            tx_rate: tx_delta as f64 / elapsed,
            iface_count: count,
            ..Default::default()
        };
        if let Some((name, data)) = best {
            info.iface = name.clone();
            info.rx_total = data.total_received();
            info.tx_total = data.total_transmitted();
            info.ip = data
                .ip_networks()
                .iter()
                .find(|n| n.addr.is_ipv4())
                .or_else(|| data.ip_networks().first())
                .map(|n| n.to_string());
            let mac = data.mac_address();
            if mac != sysinfo::MacAddr::UNSPECIFIED {
                info.mac = Some(mac.to_string());
            }
        }
        push(
            &mut self.net_history,
            ((info.rx_rate + info.tx_rate) / 1024.0).round() as u64,
        );
        self.snapshot.net_history = self.net_history.iter().copied().collect();
        self.snapshot.net = info;
    }

    fn sample_processes(&mut self) {
        self.sys
            .refresh_processes(ProcessesToUpdate::All, true);
        let mut procs: Vec<ProcInfo> = self
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| ProcInfo {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().to_string(),
                cpu: p.cpu_usage(),
                memory: p.memory(),
            })
            .collect();
        self.snapshot.process_count = procs.len();

        // Fall back to memory so the list stays meaningful on platforms that
        // report no per-process CPU time.
        procs.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.memory.cmp(&a.memory))
        });
        self.snapshot.top_cpu = procs.iter().take(TOP_N).cloned().collect();

        procs.sort_by(|a, b| b.memory.cmp(&a.memory));
        self.snapshot.top_mem = procs.into_iter().take(TOP_N).collect();
    }

    fn sample_disk(&mut self) {
        let disks = Disks::new_with_refreshed_list();
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let pick = disks
            .list()
            .iter()
            .find(|d| d.mount_point() == std::path::Path::new(root))
            .or_else(|| disks.list().iter().max_by_key(|d| d.total_space()));
        if let Some(d) = pick {
            self.snapshot.disk = DiskInfo {
                mount: d.mount_point().display().to_string(),
                fs: d.file_system().to_string_lossy().to_string(),
                used: d.total_space().saturating_sub(d.available_space()),
                total: d.total_space(),
            };
        }
    }

    fn sample_temperature(&mut self) {
        self.components.refresh();
        let pick = self
            .components
            .list()
            .iter()
            .find(|c| {
                let l = c.label().to_ascii_lowercase();
                l.contains("package") || l.contains("tdie") || l.contains("cpu")
            })
            .or_else(|| {
                self.components
                    .list()
                    .iter()
                    .max_by(|a, b| a.temperature().total_cmp(&b.temperature()))
            });
        self.snapshot.temp = pick
            .filter(|c| c.temperature() > 0.0)
            .map(|c| (c.label().to_string(), c.temperature()));
    }
}

fn push(buf: &mut VecDeque<u64>, value: u64) {
    if buf.len() >= HISTORY_LEN {
        buf.pop_front();
    }
    buf.push_back(value);
}

fn is_loopback(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("lo") || n.contains("loopback")
}

fn host_info() -> HostInfo {
    HostInfo {
        hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
        os: System::name().unwrap_or_else(|| "unknown".into()),
        os_version: System::os_version().unwrap_or_else(|| "—".into()),
        kernel: System::kernel_version().unwrap_or_else(|| "—".into()),
        arch: System::cpu_arch().unwrap_or_else(|| "—".into()),
        ..Default::default()
    }
}

fn read_battery() -> Option<BatteryInfo> {
    use battery::units::{
        power::watt, ratio::percent, thermodynamic_temperature::degree_celsius, time::second,
    };

    let manager = battery::Manager::new().ok()?;
    let bat = manager.batteries().ok()?.next()?.ok()?;

    let state = match bat.state() {
        battery::State::Charging => "Charging",
        battery::State::Discharging => "Discharging",
        battery::State::Empty => "Empty",
        battery::State::Full => "Full",
        _ => "Unknown",
    };
    let remaining = match bat.state() {
        battery::State::Charging => bat.time_to_full(),
        _ => bat.time_to_empty(),
    }
    .map(|t| t.get::<second>() as u64);

    Some(BatteryInfo {
        percent: bat.state_of_charge().get::<percent>(),
        state: state.to_string(),
        remaining,
        health: Some(bat.state_of_health().get::<percent>()),
        power_w: bat.energy_rate().get::<watt>(),
        vendor: bat.vendor().map(|s| s.trim().to_string()),
        model: bat.model().map(|s| s.trim().to_string()),
        cycles: bat.cycle_count(),
        temp_c: bat.temperature().map(|t| t.get::<degree_celsius>()),
    })
}

// ---------------------------------------------------------------------------
// Formatting helpers shared by the panels
// ---------------------------------------------------------------------------

pub fn fmt_bytes(n: u64) -> String {
    let mut val = n as f64;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if val < 1024.0 {
            return if unit == "B" {
                format!("{:.0} {}", val, unit)
            } else {
                format!("{:.1} {}", val, unit)
            };
        }
        val /= 1024.0;
    }
    format!("{:.1} PB", val)
}

pub fn fmt_rate(bytes_per_sec: f64) -> String {
    let mut val = bytes_per_sec;
    for unit in ["B/s", "KB/s", "MB/s", "GB/s"] {
        if val < 1024.0 {
            return format!("{:.1} {}", val, unit);
        }
        val /= 1024.0;
    }
    format!("{:.1} TB/s", val)
}

pub fn fmt_duration(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m {}s", mins, secs % 60)
    }
}
