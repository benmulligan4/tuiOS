/// GPU probes. sysinfo has no GPU support, so every platform gets its own source:
/// NVML for NVIDIA (Windows + Linux), Windows performance counters for every other
/// vendor, sysfs for AMD / Intel on Linux, vcgencmd + V3D job stats on a Raspberry Pi
/// and `ioreg` on macOS. Whatever a source cannot report stays `None` (shown as N/A).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct GpuReading {
    /// Stable identity across samples, e.g. "nvml:0" or "drm:card1".
    pub key: String,
    pub name: String,
    pub backend: &'static str,
    pub util: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub temp: Option<f32>,
    pub clock_mhz: Option<u32>,
}

#[cfg_attr(not(windows), allow(dead_code))]
const NVIDIA_VENDOR: u32 = 0x10DE;

pub struct GpuProbe {
    nvml: Option<nvml_wrapper::Nvml>,
    #[cfg(windows)]
    windows: Option<super::gpu_windows::PdhGpu>,
    drm: Vec<DrmCard>,
    pi: bool,
    apple: bool,
    tick: u64,
    apple_cache: Vec<GpuReading>,
    pi_cache: PiGpu,
}

#[derive(Default, Clone, Copy)]
struct PiGpu {
    temp: Option<f32>,
    clock_mhz: Option<u32>,
    mem: Option<u64>,
}

impl GpuProbe {
    /// Work out which sources exist on this machine. Slow-ish (loads NVML, walks
    /// sysfs, enumerates DXGI adapters) so it runs once on the sampler thread.
    pub fn detect() -> Self {
        let nvml = init_nvml();
        let pi = is_raspberry_pi();
        let drm = if cfg!(target_os = "linux") { detect_drm() } else { Vec::new() };
        Self {
            #[cfg(windows)]
            windows: super::gpu_windows::PdhGpu::new(nvml.as_ref().map(|_| NVIDIA_VENDOR)),
            nvml,
            drm,
            pi,
            apple: cfg!(target_os = "macos"),
            tick: 0,
            apple_cache: Vec::new(),
            pi_cache: PiGpu::default(),
        }
    }

    pub fn sample(&mut self) -> Vec<GpuReading> {
        self.tick += 1;
        let mut out = Vec::new();

        if let Some(nvml) = &self.nvml {
            out.extend(sample_nvml(nvml));
        }

        #[cfg(windows)]
        if let Some(win) = self.windows.as_mut() {
            out.extend(win.sample());
        }

        // vcgencmd is a process spawn, so the Pi's slow-moving values refresh every 5th tick
        if self.pi && (self.tick % 5 == 1) {
            self.pi_cache = sample_vcgencmd();
        }

        let mut saw_v3d = false;
        for card in &mut self.drm {
            let mut reading = card.sample();
            if card.kind == DrmKind::V3d {
                saw_v3d = true;
                reading.temp = self.pi_cache.temp;
                reading.clock_mhz = self.pi_cache.clock_mhz;
                reading.vram_total = self.pi_cache.mem;
            }
            out.push(reading);
        }
        if self.pi && !saw_v3d {
            out.push(GpuReading {
                key: "pi".into(),
                name: "Raspberry Pi VideoCore".into(),
                backend: "vcgencmd",
                temp: self.pi_cache.temp,
                clock_mhz: self.pi_cache.clock_mhz,
                vram_total: self.pi_cache.mem,
                ..Default::default()
            });
        }

        if self.apple {
            // ioreg is a process spawn too; every other tick keeps it cheap
            if self.tick % 2 == 1 || self.apple_cache.is_empty() {
                if let Some(text) = run("ioreg", &["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"]) {
                    self.apple_cache = parse_ioreg(&text);
                }
            }
            out.extend(self.apple_cache.iter().cloned());
        }

        out
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub fn is_raspberry_pi() -> bool {
    cfg!(target_os = "linux")
        && read_trim("/proc/device-tree/model")
            .map(|m| m.contains("Raspberry Pi"))
            .unwrap_or(false)
}

// ── NVML ─────────────────────────────────────────────────────────────────────

fn init_nvml() -> Option<nvml_wrapper::Nvml> {
    if let Ok(nvml) = nvml_wrapper::Nvml::init() {
        return Some(nvml);
    }
    // Older Windows drivers kept the library beside nvidia-smi instead of in System32
    if cfg!(windows) {
        let path = std::ffi::OsStr::new(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll");
        if let Ok(nvml) = nvml_wrapper::Nvml::builder().lib_path(path).init() {
            return Some(nvml);
        }
    }
    None
}

fn sample_nvml(nvml: &nvml_wrapper::Nvml) -> Vec<GpuReading> {
    use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
    let count = nvml.device_count().unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..count {
        let Ok(device) = nvml.device_by_index(i) else { continue };
        let memory = device.memory_info().ok();
        out.push(GpuReading {
            key: format!("nvml:{}", i),
            name: device.name().unwrap_or_else(|_| format!("NVIDIA GPU {}", i)),
            backend: "NVML",
            util: device.utilization_rates().ok().map(|u| u.gpu as f32),
            vram_used: memory.as_ref().map(|m| m.used),
            vram_total: memory.as_ref().map(|m| m.total),
            temp: device.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32),
            clock_mhz: device.clock_info(Clock::Graphics).ok(),
        });
    }
    out
}

// ── Linux DRM (AMD / Intel / Raspberry Pi V3D) ──────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum DrmKind {
    Amd,
    Intel,
    V3d,
}

struct DrmCard {
    key: String,
    card: PathBuf,
    device: PathBuf,
    kind: DrmKind,
    name: String,
    hwmon: Option<PathBuf>,
    v3d_prev: Option<HashMap<String, (u64, u64)>>,
}

fn detect_drm() -> Vec<DrmCard> {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.len() > 4 && n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit()))
        .collect();
    names.sort();

    let mut cards = Vec::new();
    for name in names {
        let card = Path::new("/sys/class/drm").join(&name);
        let device = card.join("device");
        let driver = fs::read_link(device.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_default();
        let kind = match driver.as_str() {
            "amdgpu" | "radeon" => DrmKind::Amd,
            "i915" | "xe" => DrmKind::Intel,
            "v3d" => DrmKind::V3d,
            // nvidia/nouveau expose nothing useful here (NVML covers NVIDIA); vc4 is display only
            _ => continue,
        };
        let hwmon = fs::read_dir(device.join("hwmon"))
            .ok()
            .and_then(|mut d| d.next())
            .and_then(|e| e.ok())
            .map(|e| e.path());
        let name_label = match kind {
            DrmKind::Amd => read_trim(device.join("product_name"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "AMD Radeon".into()),
            DrmKind::Intel => "Intel Graphics".into(),
            DrmKind::V3d => "Raspberry Pi V3D".into(),
        };
        cards.push(DrmCard {
            key: format!("drm:{}", name),
            card,
            device,
            kind,
            name: name_label,
            hwmon,
            v3d_prev: None,
        });
    }
    cards
}

impl DrmCard {
    fn sample(&mut self) -> GpuReading {
        let mut r = GpuReading {
            key: self.key.clone(),
            name: self.name.clone(),
            backend: "sysfs",
            ..Default::default()
        };
        match self.kind {
            DrmKind::Amd => {
                r.util = read_u64(self.device.join("gpu_busy_percent")).map(|v| v as f32);
                r.vram_used = read_u64(self.device.join("mem_info_vram_used"));
                r.vram_total = read_u64(self.device.join("mem_info_vram_total"));
                if let Some(hwmon) = &self.hwmon {
                    r.temp = read_u64(hwmon.join("temp1_input")).map(|m| m as f32 / 1000.0);
                    r.clock_mhz = read_u64(hwmon.join("freq1_input")).map(|hz| (hz / 1_000_000) as u32);
                }
                if r.clock_mhz.is_none() {
                    r.clock_mhz = read_trim(self.device.join("pp_dpm_sclk")).and_then(|t| parse_dpm_clock(&t));
                }
            }
            DrmKind::Intel => {
                r.clock_mhz = read_u64(self.card.join("gt_act_freq_mhz"))
                    .or_else(|| read_u64(self.device.join("tile0/gt0/freq0/act_freq")))
                    .map(|v| v as u32);
            }
            DrmKind::V3d => {
                r.backend = "V3D";
                if let Some(text) = read_trim(self.device.join("gpu_stats")) {
                    let now = parse_v3d_stats(&text);
                    if let Some(prev) = &self.v3d_prev {
                        r.util = v3d_utilisation(prev, &now);
                    }
                    self.v3d_prev = Some(now);
                }
            }
        }
        r
    }
}

/// The active line of `pp_dpm_sclk`, e.g. "1: 1800Mhz *" → 1800.
pub fn parse_dpm_clock(text: &str) -> Option<u32> {
    let line = text.lines().find(|l| l.trim_end().ends_with('*'))?;
    let value = line.split(':').nth(1)?.trim();
    value
        .trim_end_matches('*')
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .ok()
}

/// `gpu_stats` rows: queue, timestamp (ns), jobs, runtime (ns).
pub fn parse_v3d_stats(text: &str) -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        if let (Ok(ts), Ok(runtime)) = (cols[1].parse::<u64>(), cols[3].parse::<u64>()) {
            out.insert(cols[0].to_string(), (ts, runtime));
        }
    }
    out
}

/// Busiest queue's share of the elapsed time between two `gpu_stats` reads.
pub fn v3d_utilisation(
    prev: &HashMap<String, (u64, u64)>,
    now: &HashMap<String, (u64, u64)>,
) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (queue, (ts, runtime)) in now {
        let Some((pts, pruntime)) = prev.get(queue) else { continue };
        let dt = ts.saturating_sub(*pts);
        if dt == 0 {
            continue;
        }
        let pct = (runtime.saturating_sub(*pruntime) as f64 / dt as f64 * 100.0).clamp(0.0, 100.0) as f32;
        best = Some(best.map_or(pct, |b| b.max(pct)));
    }
    best
}

fn sample_vcgencmd() -> PiGpu {
    let temp = run("vcgencmd", &["measure_temp"]).and_then(|t| parse_vc_temp(&t));
    let clock = run("vcgencmd", &["measure_clock", "v3d"])
        .and_then(|t| parse_vc_clock(&t))
        .or_else(|| run("vcgencmd", &["measure_clock", "core"]).and_then(|t| parse_vc_clock(&t)));
    let mem = run("vcgencmd", &["get_mem", "gpu"]).and_then(|t| parse_vc_mem(&t));
    PiGpu { temp, clock_mhz: clock, mem }
}

/// "temp=48.3'C" → 48.3
pub fn parse_vc_temp(text: &str) -> Option<f32> {
    let value = text.trim().strip_prefix("temp=")?;
    value.trim_end_matches("'C").parse().ok()
}

/// "frequency(46)=500000000" → 500 MHz; 0 means the block is off.
pub fn parse_vc_clock(text: &str) -> Option<u32> {
    let hz: u64 = text.trim().split('=').nth(1)?.parse().ok()?;
    (hz > 0).then(|| (hz / 1_000_000) as u32)
}

/// "gpu=76M" → bytes
pub fn parse_vc_mem(text: &str) -> Option<u64> {
    let value = text.trim().strip_prefix("gpu=")?;
    let mb: u64 = value.trim_end_matches('M').parse().ok()?;
    Some(mb * 1024 * 1024)
}

/// Raw `vcgencmd get_throttled` flags, e.g. "throttled=0x50005".
pub fn parse_throttled(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix("throttled=0x")?;
    u32::from_str_radix(hex, 16).ok()
}

pub fn read_pi_throttled() -> Option<u32> {
    run("vcgencmd", &["get_throttled"]).and_then(|t| parse_throttled(&t))
}

// ── macOS ────────────────────────────────────────────────────────────────────

/// Parse `ioreg -r -d 1 -w 0 -c IOAccelerator`. Apple Silicon reports unified memory,
/// so "In use system memory" is the GPU's share and there is no separate total.
pub fn parse_ioreg(text: &str) -> Vec<GpuReading> {
    fn finish(reading: Option<GpuReading>, class: &str, out: &mut Vec<GpuReading>) {
        if let Some(mut r) = reading {
            if r.name.is_empty() {
                r.name = if class.is_empty() { "GPU".into() } else { class.to_string() };
            }
            r.key = format!("apple:{}", out.len());
            out.push(r);
        }
    }

    let mut out = Vec::new();
    let mut current: Option<GpuReading> = None;
    let mut class = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("+-o") {
            finish(current.take(), &class, &mut out);
            class.clear();
            current = Some(GpuReading { backend: "IOKit", ..Default::default() });
            continue;
        }
        let Some(reading) = current.as_mut() else { continue };
        if let Some(value) = ioreg_value(trimmed, "model") {
            reading.name = value;
        } else if let Some(value) = ioreg_value(trimmed, "IOClass") {
            class = value;
        } else if trimmed.starts_with("\"PerformanceStatistics\"") {
            let stats = parse_ioreg_dict(trimmed);
            reading.util = stats
                .get("Device Utilization %")
                .or_else(|| stats.get("GPU Activity(%)"))
                .map(|v| *v as f32);
            if let Some(used) = stats.get("vramUsedBytes") {
                reading.vram_used = Some(*used as u64);
                reading.vram_total = stats.get("vramFreeBytes").map(|f| *f as u64 + *used as u64);
            } else {
                reading.vram_used = stats.get("In use system memory").map(|v| *v as u64);
            }
        }
    }
    finish(current.take(), &class, &mut out);
    out
}

/// `"key" = "value"` or `"key" = <"value">` on one ioreg line.
fn ioreg_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(&format!("\"{}\" = ", key))?;
    let rest = rest.trim_start_matches('<');
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Numeric entries of an inline ioreg dictionary: `{"A"=1,"B"=2}`.
fn parse_ioreg_dict(line: &str) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    let Some(start) = line.find('{') else { return out };
    let body = line[start + 1..].trim_end_matches('}');
    for entry in body.split(',') {
        let Some((key, value)) = entry.split_once('=') else { continue };
        if let Ok(num) = value.trim().parse::<f64>() {
            out.insert(key.trim().trim_matches('"').to_string(), num);
        }
    }
    out
}

// ── Windows performance-counter instance names ──────────────────────────────
// Plain string handling, kept here so it is unit-tested on every platform.

/// "pid_1_luid_0x00000000_0x0000D1B4_phys_0_eng_0_engtype_3D" → "0x00000000_0x0000d1b4".
#[cfg_attr(not(windows), allow(dead_code))]
pub fn pdh_luid(instance: &str) -> Option<String> {
    let start = instance.find("luid_")? + "luid_".len();
    let rest = &instance[start..];
    let end = rest.find("_phys").unwrap_or(rest.len());
    Some(rest[..end].to_ascii_lowercase())
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn pdh_engtype(instance: &str) -> Option<&str> {
    let start = instance.find("engtype_")? + "engtype_".len();
    Some(&instance[start..])
}

/// Adapter utilisation the way Windows Task Manager reports it: per adapter, the
/// busiest engine type, where each engine type sums every process using it.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn adapter_utilisation(samples: &[(String, f64)]) -> HashMap<String, f32> {
    let mut per_engine: HashMap<(String, String), f64> = HashMap::new();
    for (instance, value) in samples {
        let (Some(luid), Some(engine)) = (pdh_luid(instance), pdh_engtype(instance)) else { continue };
        *per_engine.entry((luid, engine.to_string())).or_default() += value;
    }
    let mut out: HashMap<String, f32> = HashMap::new();
    for ((luid, _), total) in per_engine {
        let pct = total.clamp(0.0, 100.0) as f32;
        let slot = out.entry(luid).or_insert(0.0);
        *slot = slot.max(pct);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const IOREG_M4: &str = r#"+-o AGXAcceleratorG16G  <class AGXAcceleratorG16G, id 0x1000003c1, registered, matched, active, busy 0 (0 ms), retain 98>
    {
      "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=2495234048,"Tiler Utilization %"=34,"recoveryCount"=0,"lastRecoveryTime"=0,"Renderer Utilization %"=32,"TiledSceneBytes"=950272,"Device Utilization %"=34,"SplitSceneCount"=0,"Allocated PB Size"=56885248,"In use system memory"=681181184}
      "model" = "Apple M4"
      "gpu-core-count" = 10
      "IOClass" = "AGXAcceleratorG16G"
    }
"#;

    #[test]
    fn parses_apple_silicon_ioreg() {
        let gpus = parse_ioreg(IOREG_M4);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Apple M4");
        assert_eq!(gpus[0].util, Some(34.0));
        assert_eq!(gpus[0].vram_used, Some(681181184));
        assert_eq!(gpus[0].vram_total, None);
    }

    #[test]
    fn parses_intel_mac_amd_ioreg() {
        let text = "+-o AMDRadeonX6000  <class AMDRadeonX6000>\n  \"model\" = <\"AMD Radeon Pro 5500M\">\n  \"PerformanceStatistics\" = {\"Device Utilization %\"=12,\"vramUsedBytes\"=1073741824,\"vramFreeBytes\"=3221225472}\n";
        let gpus = parse_ioreg(text);
        assert_eq!(gpus[0].name, "AMD Radeon Pro 5500M");
        assert_eq!(gpus[0].vram_total, Some(4294967296));
    }

    #[test]
    fn parses_vcgencmd() {
        assert_eq!(parse_vc_temp("temp=48.3'C\n"), Some(48.3));
        assert_eq!(parse_vc_clock("frequency(46)=500000000\n"), Some(500));
        assert_eq!(parse_vc_clock("frequency(46)=0\n"), None);
        assert_eq!(parse_vc_mem("gpu=76M\n"), Some(76 * 1024 * 1024));
        assert_eq!(parse_throttled("throttled=0x50005\n"), Some(0x50005));
    }

    #[test]
    fn v3d_utilisation_uses_busiest_queue() {
        let a = parse_v3d_stats("queue\ttimestamp\tjobs\truntime\nbin\t1000\t1\t100\nrender\t1000\t1\t200\n");
        let b = parse_v3d_stats("queue\ttimestamp\tjobs\truntime\nbin\t2000\t2\t200\nrender\t2000\t3\t700\n");
        assert_eq!(v3d_utilisation(&a, &b), Some(50.0));
    }

    #[test]
    fn dpm_clock_reads_starred_row() {
        assert_eq!(parse_dpm_clock("0: 500Mhz\n1: 1800Mhz *\n"), Some(1800));
    }

    #[test]
    fn pdh_instances_group_by_engine_type() {
        let samples = vec![
            ("pid_1_luid_0x00000000_0x0000D1B4_phys_0_eng_0_engtype_3D".to_string(), 30.0),
            ("pid_2_luid_0x00000000_0x0000D1B4_phys_0_eng_0_engtype_3D".to_string(), 25.0),
            ("pid_2_luid_0x00000000_0x0000D1B4_phys_0_eng_5_engtype_VideoDecode".to_string(), 40.0),
            ("pid_3_luid_0x00000000_0x0000AAAA_phys_0_eng_0_engtype_3D".to_string(), 5.0),
        ];
        let util = adapter_utilisation(&samples);
        assert_eq!(util["0x00000000_0x0000d1b4"], 55.0);
        assert_eq!(util["0x00000000_0x0000aaaa"], 5.0);
    }
}
