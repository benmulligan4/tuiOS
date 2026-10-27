/// Settings for the Task Manager bar and the system stats line beside the navbar
/// (`task_bar.*` in settings.json).

use ratatui::style::Color;
use serde_json::Value;

use crate::settings::pages::appearance::color_from_name;
use crate::settings::persistence;

use super::config::choice;

pub const WIDTHS: &[&str] = &["Auto", "30%", "40%", "50%", "60%"];
pub const STATS_POSITIONS: &[&str] = &["Below Navbar", "Above Navbar"];
pub const TILE_INFOS: &[&str] = &["Status", "CPU", "Uptime"];
pub const COLOURS: &[&str] = &[
    "Accent", "Cyan", "Green", "Yellow", "Blue", "Magenta", "Red", "White", "Light Blue", "Orange", "Pink",
    "Teal", "Purple", "Grey",
];

pub const KEY_ENABLED: &str = "task_bar.enabled";
pub const KEY_WIDTH: &str = "task_bar.width";
pub const KEY_STATS_POSITION: &str = "task_bar.stats_position";
pub const KEY_EXTRA_ROW: &str = "task_bar.extra_stats";
pub const KEY_TILE_INFO: &str = "task_bar.tile_info";
pub const KEY_GROUP: &str = "task_bar.group_instances";

/// Items on the extra stats row: (settings key, label, on by default).
pub const EXTRAS: &[(&str, &str, bool)] = &[
    ("task_bar.show_gpu", "GPU Usage", true),
    ("task_bar.show_temperature", "Temperature", true),
    ("task_bar.show_network", "Network Speed", true),
    ("task_bar.show_swap", "Swap", false),
    ("task_bar.show_uptime", "Uptime", true),
];

/// What an app tile's border colour means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Shown,
    Background,
    Window,
    Suspended,
    Stopping,
    Locating,
    Crashed,
}

impl Status {
    pub const ALL: [Status; 7] = [
        Status::Shown,
        Status::Background,
        Status::Window,
        Status::Suspended,
        Status::Stopping,
        Status::Locating,
        Status::Crashed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Status::Shown => "task_bar.colour_shown",
            Status::Background => "task_bar.colour_background",
            Status::Window => "task_bar.colour_window",
            Status::Suspended => "task_bar.colour_suspended",
            Status::Stopping => "task_bar.colour_stopping",
            Status::Locating => "task_bar.colour_locating",
            Status::Crashed => "task_bar.colour_crashed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Shown => "On Screen",
            Status::Background => "Background",
            Status::Window => "Own Window",
            Status::Suspended => "Suspended",
            Status::Stopping => "Quitting",
            Status::Locating => "Finding Window",
            Status::Crashed => "Crashed",
        }
    }

    pub fn default_colour(self) -> &'static str {
        match self {
            Status::Shown => "Green",
            Status::Background => "Accent",
            Status::Window => "Magenta",
            Status::Suspended => "Yellow",
            Status::Stopping => "Orange",
            Status::Locating => "Grey",
            Status::Crashed => "Red",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BarConfig {
    pub enabled: bool,
    /// Share of the screen for the Task Manager bar; None = Auto (the navbar takes
    /// what its items need, the bar gets the rest).
    pub width_pct: Option<u16>,
    pub stats_above: bool,
    pub extra_row: bool,
    pub show_gpu: bool,
    pub show_temperature: bool,
    pub show_network: bool,
    pub show_swap: bool,
    pub show_uptime: bool,
    pub tile_info: String,
    pub group: bool,
    colours: [Color; 7],
}

impl BarConfig {
    pub fn colour(&self, status: Status) -> Color {
        let i = Status::ALL.iter().position(|s| *s == status).unwrap_or(0);
        self.colours[i]
    }
}

fn extra(s: &Value, i: usize) -> bool {
    let (key, _, default) = EXTRAS[i];
    persistence::get_bool(s, key, default)
}

pub fn colour_name(s: &Value, status: Status) -> String {
    choice(s, status.key(), COLOURS, status.default_colour())
}

pub fn from_settings(s: &Value, accent: Color) -> BarConfig {
    let width = choice(s, KEY_WIDTH, WIDTHS, "Auto");
    let colours = Status::ALL.map(|status| match colour_name(s, status).as_str() {
        "Accent" => accent,
        name => color_from_name(name),
    });
    BarConfig {
        enabled: persistence::get_bool(s, KEY_ENABLED, true),
        width_pct: width.trim_end_matches('%').parse().ok(),
        stats_above: choice(s, KEY_STATS_POSITION, STATS_POSITIONS, "Below Navbar") == "Above Navbar",
        extra_row: persistence::get_bool(s, KEY_EXTRA_ROW, true),
        show_gpu: extra(s, 0),
        show_temperature: extra(s, 1),
        show_network: extra(s, 2),
        show_swap: extra(s, 3),
        show_uptime: extra(s, 4),
        tile_info: choice(s, KEY_TILE_INFO, TILE_INFOS, "Status"),
        group: persistence::get_bool(s, KEY_GROUP, true),
        colours,
    }
}

pub fn load() -> BarConfig {
    let s = persistence::load();
    let accent = color_from_name(&persistence::get_str(&s, "appearance.accent_color", "Cyan"));
    from_settings(&s, accent)
}

/// Defaults written into a fresh settings.json.
pub fn default_settings() -> Value {
    let mut out = serde_json::json!({
        "enabled": true,
        "width": "Auto",
        "stats_position": "Below Navbar",
        "extra_stats": true,
        "tile_info": "Status",
        "group_instances": true,
    });
    for (key, _, default) in EXTRAS {
        out[key.trim_start_matches("task_bar.")] = Value::Bool(*default);
    }
    for status in Status::ALL {
        out[status.key().trim_start_matches("task_bar.")] = Value::String(status.default_colour().into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let s = serde_json::json!({ "task_bar": default_settings() });
        let c = from_settings(&s, Color::Cyan);
        assert!(c.enabled && c.extra_row && c.group && !c.show_swap);
        assert_eq!(c.width_pct, None);
        assert_eq!(c.colour(Status::Background), Color::Cyan);
        assert_eq!(c.colour(Status::Crashed), Color::Red);
        let s = serde_json::json!({ "task_bar": { "width": "40%", "stats_position": "Above Navbar" } });
        let c = from_settings(&s, Color::Cyan);
        assert_eq!(c.width_pct, Some(40));
        assert!(c.stats_above);
    }
}
