pub mod about;
pub mod appearance;
pub mod audio;
pub mod boot;
pub mod button_mapping;
pub mod display;
pub mod git;
pub mod hotkeys;
pub mod power;
pub mod utilities;
pub mod wifi_bluetooth;

use serde_json::Value;

use super::persistence;

/// Step to the next option in a fixed list and store it.
pub(crate) fn cycle_option(
    settings: &mut Value,
    key: &str,
    options: &[&str],
    current: &str,
    forward: bool,
) -> String {
    let idx = options.iter().position(|&o| o == current).unwrap_or(0);
    let len = options.len();
    let next = options[if forward { (idx + 1) % len } else { (idx + len - 1) % len }];
    persistence::set(settings, key, Value::String(next.to_string()));
    next.to_string()
}
