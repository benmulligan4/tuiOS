/// Network extras for the Task Manager's Network tab: a TCP latency probe, Wi-Fi details
/// and the public IP. They block on I/O, so they run on their own worker thread, and only
/// while the Task Manager is open (plus a minute after) — no traffic otherwise.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock, RwLockReadGuard};
use std::thread;
use std::time::{Duration, Instant};

use lazy_static::lazy_static;

use super::clean_text;
use super::config;
use super::metrics::Ring;

pub const LATENCY_EVERY: Duration = Duration::from_secs(2);
const WIFI_EVERY: Duration = Duration::from_secs(20);
const IP_EVERY: Duration = Duration::from_secs(300);
const IDLE_EXIT: Duration = Duration::from_secs(60);
/// 30 minutes of probes.
const LATENCY_SAMPLES: usize = 900;

#[derive(Debug, Clone, PartialEq)]
pub enum Wifi {
    Unknown,
    Unavailable(String),
    Disconnected,
    Connected { ssid: String, signal: Option<u8>, rssi: Option<i32> },
}

pub struct NetExtras {
    /// Round-trip of a TCP handshake in ms; None = the probe failed.
    pub latency: Ring<Option<f32>>,
    pub target: String,
    pub wifi: Wifi,
    pub public_ip: Option<String>,
    pub public_ip_note: String,
    pub ip_checked: Option<Instant>,
    pub online: Option<bool>,
}

lazy_static! {
    static ref EXTRAS: RwLock<NetExtras> = RwLock::new(NetExtras {
        latency: Ring::new(LATENCY_SAMPLES),
        target: String::new(),
        wifi: Wifi::Unknown,
        public_ip: None,
        public_ip_note: "Not checked yet".into(),
        ip_checked: None,
        online: None,
    });
    static ref LAST_TOUCH: Mutex<Instant> = Mutex::new(Instant::now());
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static IP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Keep the worker alive; called every frame the Task Manager is drawn.
pub fn touch() {
    if let Ok(mut t) = LAST_TOUCH.lock() {
        *t = Instant::now();
    }
    if !RUNNING.swap(true, Ordering::SeqCst) {
        if thread::Builder::new().name("tuios-netinfo".into()).spawn(worker).is_err() {
            RUNNING.store(false, Ordering::SeqCst);
        }
    }
}

/// Look the public IP up again on the worker's next pass.
pub fn request_public_ip() {
    IP_REQUESTED.store(true, Ordering::SeqCst);
    touch();
}

pub fn read() -> RwLockReadGuard<'static, NetExtras> {
    EXTRAS.read().unwrap_or_else(|e| e.into_inner())
}

fn idle_for() -> Duration {
    LAST_TOUCH.lock().map(|t| t.elapsed()).unwrap_or(IDLE_EXIT)
}

fn worker() {
    let now = Instant::now();
    let mut next_latency = now;
    let mut next_wifi = now;
    let mut next_ip = now;
    let mut cfg = config::load();
    loop {
        if idle_for() >= IDLE_EXIT {
            RUNNING.store(false, Ordering::SeqCst);
            // A touch() that raced with shutting down would otherwise leave no worker
            if idle_for() < IDLE_EXIT && !RUNNING.swap(true, Ordering::SeqCst) {
                continue;
            }
            return;
        }
        let now = Instant::now();

        if now >= next_latency {
            cfg = config::load();
            let rtt = probe_latency(&cfg.latency_target);
            if let Ok(mut x) = EXTRAS.write() {
                if x.target != cfg.latency_target {
                    x.latency.clear();
                    x.target = cfg.latency_target.clone();
                }
                x.latency.push(rtt);
                x.online = Some(rtt.is_some());
            }
            next_latency = now + LATENCY_EVERY;
        }

        if now >= next_wifi {
            let wifi = query_wifi();
            if let Ok(mut x) = EXTRAS.write() {
                x.wifi = wifi;
            }
            next_wifi = now + WIFI_EVERY;
        }

        if IP_REQUESTED.swap(false, Ordering::SeqCst) || now >= next_ip {
            if cfg.public_ip {
                if let Ok(mut x) = EXTRAS.write() {
                    x.public_ip_note = "Checking…".into();
                }
                let result = lookup_public_ip();
                if let Ok(mut x) = EXTRAS.write() {
                    match result {
                        Ok(ip) => {
                            x.public_ip = Some(ip);
                            x.public_ip_note = String::new();
                        }
                        Err(e) => x.public_ip_note = e,
                    }
                    x.ip_checked = Some(Instant::now());
                }
            } else if let Ok(mut x) = EXTRAS.write() {
                x.public_ip = None;
                x.public_ip_note = "Lookup disabled in Settings".into();
            }
            next_ip = now + IP_EVERY;
        }

        thread::sleep(Duration::from_millis(250));
    }
}

/// TCP handshake time to port 443 — needs no admin rights, unlike ICMP ping.
fn probe_latency(target: &str) -> Option<f32> {
    let addr: SocketAddr = format!("{}:443", target).parse().ok()?;
    let start = Instant::now();
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    Some(start.elapsed().as_secs_f32() * 1000.0)
}

fn lookup_public_ip() -> Result<String, String> {
    let body = ureq::get("https://api.ipify.org")
        .timeout(Duration::from_secs(5))
        .call()
        .map_err(|_| "Offline or lookup blocked".to_string())?
        .into_string()
        .map_err(|_| "Unreadable reply".to_string())?;
    // Only ever display something that really is an address
    body.trim()
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| "Unexpected reply".to_string())
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

fn query_wifi() -> Wifi {
    if cfg!(windows) {
        match run("netsh", &["wlan", "show", "interfaces"]) {
            Some(text) => parse_netsh(&text),
            None => Wifi::Unavailable("netsh not available".into()),
        }
    } else if cfg!(target_os = "macos") {
        let airport = "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport";
        if let Some(w) = run(airport, &["-I"]).and_then(|t| parse_airport(&t)) {
            return w;
        }
        match run("system_profiler", &["SPAirPortDataType"]) {
            Some(text) => parse_system_profiler(&text),
            None => Wifi::Unavailable("Not reported".into()),
        }
    } else {
        if let Some(text) = run("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL", "dev", "wifi", "list", "--rescan", "no"]) {
            if let Some(w) = parse_nmcli(&text) {
                return w;
            }
        }
        let ssid = run("iwgetid", &["-r"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let quality = std::fs::read_to_string("/proc/net/wireless").ok().and_then(|t| parse_proc_wireless(&t));
        match (ssid, quality) {
            (Some(ssid), q) => Wifi::Connected {
                ssid: clean_text(&ssid),
                signal: q.map(|(pct, _)| pct),
                rssi: q.and_then(|(_, rssi)| rssi),
            },
            (None, Some(_)) => Wifi::Disconnected,
            (None, None) => Wifi::Unavailable("No Wi-Fi adapter".into()),
        }
    }
}

fn rssi_to_pct(rssi: i32) -> u8 {
    (2 * (rssi + 100)).clamp(0, 100) as u8
}

pub fn parse_netsh(text: &str) -> Wifi {
    if text.to_lowercase().contains("location permission") {
        return Wifi::Unavailable("Needs Windows location permission".into());
    }
    let mut ssid = None;
    let mut signal = None;
    let mut state = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "SSID" => ssid = Some(value.to_string()),
            "Signal" => signal = value.trim_end_matches('%').trim().parse::<u8>().ok(),
            "State" => state = Some(value.to_lowercase()),
            _ => {}
        }
    }
    match (ssid, state) {
        (Some(ssid), _) if !ssid.is_empty() => Wifi::Connected { ssid: clean_text(&ssid), signal, rssi: None },
        (_, Some(_)) => Wifi::Disconnected,
        _ if text.to_lowercase().contains("there is no wireless interface") => {
            Wifi::Unavailable("No Wi-Fi adapter".into())
        }
        _ => Wifi::Unavailable("Not reported".into()),
    }
}

/// Split a terse nmcli row on the colons that are not escaped as `\:`.
fn split_terse(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.last_mut().unwrap().push(next);
                }
            }
            ':' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

pub fn parse_nmcli(text: &str) -> Option<Wifi> {
    if text.trim().is_empty() || text.trim_start().starts_with("Error") {
        return None;
    }
    for line in text.lines() {
        let fields = split_terse(line);
        if fields.len() >= 3 && fields[0] == "yes" {
            return Some(Wifi::Connected {
                ssid: clean_text(&fields[1]),
                signal: fields[2].trim().parse().ok(),
                rssi: None,
            });
        }
    }
    Some(Wifi::Disconnected)
}

/// `/proc/net/wireless` → (link quality %, signal dBm) for the first interface.
pub fn parse_proc_wireless(text: &str) -> Option<(u8, Option<i32>)> {
    let line = text.lines().nth(2)?;
    let (_, rest) = line.split_once(':')?;
    let cols: Vec<&str> = rest.split_whitespace().collect();
    let quality: f32 = cols.get(1)?.trim_end_matches('.').parse().ok()?;
    let rssi = cols.get(2).and_then(|v| v.trim_end_matches('.').parse::<i32>().ok());
    Some(((quality / 70.0 * 100.0).clamp(0.0, 100.0) as u8, rssi))
}

pub fn parse_airport(text: &str) -> Option<Wifi> {
    let mut ssid = None;
    let mut rssi = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        match key.trim() {
            "SSID" => ssid = Some(value.trim().to_string()),
            "agrCtlRSSI" => rssi = value.trim().parse::<i32>().ok(),
            "AirPort" if value.trim() == "Off" => return Some(Wifi::Disconnected),
            _ => {}
        }
    }
    let ssid = ssid?;
    Some(Wifi::Connected { ssid: clean_text(&ssid), signal: rssi.map(rssi_to_pct), rssi })
}

pub fn parse_system_profiler(text: &str) -> Wifi {
    let mut lines = text.lines().map(str::trim);
    if !lines.any(|l| l == "Current Network Information:") {
        return Wifi::Disconnected;
    }
    let Some(ssid_line) = lines.find(|l| !l.is_empty()) else { return Wifi::Disconnected };
    let ssid = ssid_line.trim_end_matches(':').to_string();
    let rssi = lines
        .find(|l| l.starts_with("Signal / Noise:"))
        .and_then(|l| l.trim_start_matches("Signal / Noise:").split_whitespace().next()?.parse::<i32>().ok());
    Wifi::Connected { ssid: clean_text(&ssid), signal: rssi.map(rssi_to_pct), rssi }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netsh_connected() {
        let text = "There is 1 interface on the system:\n\n    Name                   : Wi-Fi\n    State                  : connected\n    SSID                   : Home Net\n    BSSID                  : aa:bb:cc:dd:ee:ff\n    Signal                 : 92%\n";
        assert_eq!(
            parse_netsh(text),
            Wifi::Connected { ssid: "Home Net".into(), signal: Some(92), rssi: None }
        );
    }

    #[test]
    fn netsh_disconnected_and_permission() {
        assert_eq!(parse_netsh("    Name : Wi-Fi\n    State : disconnected\n"), Wifi::Disconnected);
        assert!(matches!(
            parse_netsh("Network shell commands need location permission to access WLAN information."),
            Wifi::Unavailable(_)
        ));
    }

    #[test]
    fn nmcli_handles_escaped_colons() {
        let text = "no:Other:40\nyes:Cafe\\:Guest:78\n";
        assert_eq!(
            parse_nmcli(text),
            Some(Wifi::Connected { ssid: "Cafe:Guest".into(), signal: Some(78), rssi: None })
        );
        assert_eq!(parse_nmcli("no:Other:40\n"), Some(Wifi::Disconnected));
    }

    #[test]
    fn proc_wireless_quality() {
        let text = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n wlan0: 0000   58.  -52.  -256        0      0      0      0      0        0\n";
        assert_eq!(parse_proc_wireless(text), Some((82, Some(-52))));
    }

    #[test]
    fn system_profiler_ssid_and_signal() {
        let text = "Wi-Fi:\n\n      Interfaces:\n        en0:\n          Current Network Information:\n            HomeNet:\n              PHY Mode: 802.11ax\n              Signal / Noise: -55 dBm / -92 dBm\n";
        assert_eq!(
            parse_system_profiler(text),
            Wifi::Connected { ssid: "HomeNet".into(), signal: Some(90), rssi: Some(-55) }
        );
    }

    #[test]
    fn ssids_cannot_smuggle_escape_codes() {
        let text = "yes:Evil\u{1b}[2J:50\n";
        match parse_nmcli(text) {
            Some(Wifi::Connected { ssid, .. }) => assert!(!ssid.contains('\u{1b}')),
            other => panic!("unexpected {:?}", other),
        }
    }
}
