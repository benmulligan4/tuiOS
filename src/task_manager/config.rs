/// Task Manager settings — every `task_manager.*` key in settings.json, parsed once.
///
/// The option lists double as the ◄ ► choices on the Settings page, so a stored value
/// that is not in its list falls back to the default instead of being misread.

use std::time::Duration;

use serde_json::Value;

use crate::settings::persistence;

pub const INTERVALS: &[&str] = &["0.5 s", "1 s", "2 s", "5 s"];
pub const BACKGROUND_SCANS: &[&str] = &["Off", "5 s", "10 s", "30 s", "60 s"];
pub const HISTORIES: &[&str] = &["5 min", "15 min", "30 min", "60 min"];
pub const GRAPH_STYLES: &[&str] = &["Line", "Sparkline"];
pub const KILL_TIMEOUTS: &[&str] = &["1 s", "3 s", "5 s", "10 s"];
pub const SCROLLBACKS: &[&str] = &["Off", "500 lines", "1000 lines", "5000 lines"];
pub const LATENCY_TARGETS: &[&str] = &["1.1.1.1", "8.8.8.8", "9.9.9.9"];
pub const USAGE_THRESHOLDS: &[&str] = &["70%", "80%", "90%", "95%"];
pub const TEMP_THRESHOLDS: &[&str] = &["70 °C", "75 °C", "80 °C", "85 °C", "90 °C", "95 °C"];
pub const SUSTAIN_TIMES: &[&str] = &["5 s", "10 s", "30 s", "60 s"];

pub const KEY_INTERVAL: &str = "task_manager.sample_interval";
pub const KEY_BACKGROUND_SCAN: &str = "task_manager.background_scan";
pub const KEY_NVML: &str = "task_manager.nvidia_stats";
pub const KEY_HISTORY: &str = "task_manager.history_length";
pub const KEY_GRAPH_STYLE: &str = "task_manager.graph_style";
pub const KEY_KILL_TIMEOUT: &str = "task_manager.kill_timeout";
pub const KEY_EXIT_NOTIFY: &str = "task_manager.exit_notifications";
pub const KEY_WARN_ON_EXIT: &str = "task_manager.warn_on_exit";
pub const KEY_SCROLLBACK: &str = "task_manager.scrollback";
pub const KEY_LATENCY_TARGET: &str = "task_manager.latency_target";
pub const KEY_PUBLIC_IP: &str = "task_manager.public_ip_lookup";
pub const KEY_ALERTS: &str = "task_manager.alerts_enabled";
pub const KEY_ALERT_CPU: &str = "task_manager.alert_cpu";
pub const KEY_ALERT_RAM: &str = "task_manager.alert_ram";
pub const KEY_ALERT_TEMP: &str = "task_manager.alert_temp";
pub const KEY_ALERT_SUSTAIN: &str = "task_manager.alert_sustain";
pub const KEY_PI_POWER: &str = "task_manager.pi_power_alerts";
pub const KEY_STARTUP: &str = "task_manager.startup_apps";

pub const KEY_SWITCHER: &str = "button_mapping.switcher_key";
pub const KEY_TASK_MANAGER: &str = "button_mapping.task_manager_key";
pub const DEFAULT_SWITCHER_KEY: &str = "F12";
pub const DEFAULT_TASK_MANAGER_KEY: &str = "Shift+F12";

/// Defaults written into a fresh settings.json (and by Restore Defaults).
pub fn default_settings() -> Value {
    serde_json::json!({
        "sample_interval": "1 s",
        "background_scan": "5 s",
        "nvidia_stats": true,
        "history_length": "15 min",
        "graph_style": "Line",
        "kill_timeout": "3 s",
        "exit_notifications": true,
        "warn_on_exit": true,
        "scrollback": "1000 lines",
        "latency_target": "1.1.1.1",
        "public_ip_lookup": true,
        "alerts_enabled": true,
        "alert_cpu": "90%",
        "alert_ram": "90%",
        "alert_temp": "85 °C",
        "alert_sustain": "10 s",
        "pi_power_alerts": true,
        "startup_apps": []
    })
}

#[derive(Debug, Clone)]
pub struct TmConfig {
    pub interval: Duration,
    /// How often apps' usage is measured while the Task Manager is closed; None = never.
    pub background_scan: Option<Duration>,
    pub nvml: bool,
    pub history_secs: u64,
    pub graph_line: bool,
    pub kill_timeout: Duration,
    pub exit_notifications: bool,
    pub warn_on_exit: bool,
    pub scrollback: usize,
    pub latency_target: String,
    pub public_ip: bool,
    pub alerts: bool,
    pub alert_cpu: f32,
    pub alert_ram: f32,
    pub alert_temp: f32,
    pub alert_sustain: Duration,
    pub pi_power_alerts: bool,
}

/// The stored option, or `default` when it is missing or not one of `options`.
pub fn choice(settings: &Value, key: &str, options: &[&str], default: &str) -> String {
    let stored = persistence::get_str(settings, key, default);
    if options.contains(&stored.as_str()) {
        stored
    } else {
        default.to_string()
    }
}

/// The number a label starts with: "0.5 s" → 0.5, "90%" → 90, "Off" → None.
pub fn leading_number(label: &str) -> Option<f64> {
    let num: String = label
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().ok()
}

fn secs(label: &str, fallback: f64) -> Duration {
    Duration::from_secs_f64(leading_number(label).unwrap_or(fallback))
}

pub fn from_settings(s: &Value) -> TmConfig {
    let history_mins = leading_number(&choice(s, KEY_HISTORY, HISTORIES, "15 min")).unwrap_or(15.0);
    TmConfig {
        interval: secs(&choice(s, KEY_INTERVAL, INTERVALS, "1 s"), 1.0),
        background_scan: leading_number(&choice(s, KEY_BACKGROUND_SCAN, BACKGROUND_SCANS, "5 s"))
            .map(Duration::from_secs_f64),
        nvml: persistence::get_bool(s, KEY_NVML, true),
        history_secs: (history_mins * 60.0) as u64,
        graph_line: choice(s, KEY_GRAPH_STYLE, GRAPH_STYLES, "Line") == "Line",
        kill_timeout: secs(&choice(s, KEY_KILL_TIMEOUT, KILL_TIMEOUTS, "3 s"), 3.0),
        exit_notifications: persistence::get_bool(s, KEY_EXIT_NOTIFY, true),
        warn_on_exit: persistence::get_bool(s, KEY_WARN_ON_EXIT, true),
        scrollback: leading_number(&choice(s, KEY_SCROLLBACK, SCROLLBACKS, "1000 lines"))
            .unwrap_or(0.0) as usize,
        latency_target: choice(s, KEY_LATENCY_TARGET, LATENCY_TARGETS, "1.1.1.1"),
        public_ip: persistence::get_bool(s, KEY_PUBLIC_IP, true),
        alerts: persistence::get_bool(s, KEY_ALERTS, true),
        alert_cpu: leading_number(&choice(s, KEY_ALERT_CPU, USAGE_THRESHOLDS, "90%")).unwrap_or(90.0) as f32,
        alert_ram: leading_number(&choice(s, KEY_ALERT_RAM, USAGE_THRESHOLDS, "90%")).unwrap_or(90.0) as f32,
        alert_temp: leading_number(&choice(s, KEY_ALERT_TEMP, TEMP_THRESHOLDS, "85 °C")).unwrap_or(85.0) as f32,
        alert_sustain: secs(&choice(s, KEY_ALERT_SUSTAIN, SUSTAIN_TIMES, "10 s"), 10.0),
        pi_power_alerts: persistence::get_bool(s, KEY_PI_POWER, true),
    }
}

pub fn load() -> TmConfig {
    from_settings(&persistence::load())
}

/// Navbar actions ("app:X" / "dashboard:X") launched in the background at boot.
pub fn startup_apps(s: &Value) -> Vec<String> {
    persistence::get(s, KEY_STARTUP)
        .and_then(|v| v.as_array().cloned())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn set_startup_apps(s: &mut Value, actions: &[String]) {
    let list = actions.iter().map(|a| Value::String(a.clone())).collect();
    persistence::set(s, KEY_STARTUP, Value::Array(list));
}

pub fn switcher_key(s: &Value) -> String {
    persistence::get_str(s, KEY_SWITCHER, DEFAULT_SWITCHER_KEY)
}

pub fn task_manager_key(s: &Value) -> String {
    persistence::get_str(s, KEY_TASK_MANAGER, DEFAULT_TASK_MANAGER_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_labels() {
        assert_eq!(leading_number("0.5 s"), Some(0.5));
        assert_eq!(leading_number("90%"), Some(90.0));
        assert_eq!(leading_number("85 °C"), Some(85.0));
        assert_eq!(leading_number("Off"), None);
    }

    #[test]
    fn unknown_values_fall_back() {
        let s = serde_json::json!({ "task_manager": { "sample_interval": "7 s", "scrollback": "Off" } });
        let c = from_settings(&s);
        assert_eq!(c.interval, Duration::from_secs(1));
        assert_eq!(c.scrollback, 0);
        assert_eq!(c.history_secs, 900);
        assert_eq!(c.background_scan, Some(Duration::from_secs(5)));
        let off = serde_json::json!({ "task_manager": { "background_scan": "Off" } });
        assert_eq!(from_settings(&off).background_scan, None);
    }
}
