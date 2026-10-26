/// Appearance settings — clock, default dashboard, tuiOS colour.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{BorderType, Paragraph},
    Frame,
};

use serde_json::Value;

use crate::settings::persistence;

const ACCENT_COLORS: &[&str] = &[
    "Cyan", "Green", "Yellow", "Blue", "Magenta", "Red", "White",
    "Light Blue", "Orange", "Pink", "Teal", "Purple", "Grey", "Rainbow",
];

const BORDER_STYLES: &[&str] = &["Rounded", "Single", "Double", "None"];

/// Rows in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Accent,
    Clock,
    ClockFormat,
    ClockSeconds,
    Dashboard,
    Border,
    StatusBar,
    NewWindow,
    NavBar,
    BootSettings,
}

impl Row {
    /// True for rows picked from a fixed list with ◄ ►.
    fn cycles(self) -> bool {
        matches!(self, Row::Accent | Row::Dashboard | Row::Border)
    }
}

fn rows() -> Vec<Row> {
    vec![
        Row::Accent,
        Row::Clock,
        Row::ClockFormat,
        Row::ClockSeconds,
        Row::Dashboard,
        Row::Border,
        Row::StatusBar,
        Row::NewWindow,
        Row::NavBar,
        Row::BootSettings,
    ]
}

fn row_at(cursor: usize) -> Option<Row> {
    rows().get(cursor).copied()
}

/// True when the cursor is on the row that opens the boot animation page.
pub fn is_boot_settings(cursor: usize) -> bool {
    row_at(cursor) == Some(Row::BootSettings)
}

/// Step to the next option in a fixed list and store it.
fn cycle_option(settings: &mut Value, key: &str, options: &[&str], current: &str, forward: bool) -> String {
    let idx = options.iter().position(|&o| o == current).unwrap_or(0);
    let len = options.len();
    let next = options[if forward { (idx + 1) % len } else { (idx + len - 1) % len }];
    persistence::set(settings, key, Value::String(next.to_string()));
    next.to_string()
}

/// Convert a border style name to a ratatui BorderType.
pub fn border_type_from_name(name: &str) -> BorderType {
    match name {
        "Double" => BorderType::Double,
        "Rounded" => BorderType::Rounded,
        "None" => BorderType::Plain,
        _ => BorderType::Plain, // "Single" and fallback
    }
}

/// Convert a colour name string to a ratatui Color.
/// "Rainbow" animates through the spectrum on every call.
pub fn color_from_name(name: &str) -> Color {
    match name {
        "Cyan"       => Color::Cyan,
        "Green"      => Color::Green,
        "Yellow"     => Color::Yellow,
        "Blue"       => Color::Blue,
        "Magenta"    => Color::Magenta,
        "Red"        => Color::Red,
        "White"      => Color::White,
        "Light Blue" => Color::Rgb(135, 206, 235),
        "Orange"     => Color::Rgb(255, 140, 0),
        "Pink"       => Color::Rgb(255, 105, 180),
        "Teal"       => Color::Rgb(0, 178, 178),
        "Purple"     => Color::Rgb(148, 0, 211),
        "Grey"       => Color::Rgb(160, 160, 160),
        "Rainbow" => {
            // Animate through hues based on current time (changes ~10x per second)
            let ms = chrono::Local::now().timestamp_millis() as u64;
            let hue = (ms / 80) % 360;
            let (r, g, b) = hue_to_rgb(hue as f32);
            Color::Rgb(r, g, b)
        }
        _ => Color::Cyan,
    }
}

/// Convert a HSV hue (0-360, s=1, v=1) to an RGB triple.
pub fn hue_to_rgb(hue: f32) -> (u8, u8, u8) {
    let h = hue / 60.0;
    let i = h as u32 % 6;
    let f = h - h.floor();
    let q = ((1.0 - f) * 255.0) as u8;
    let t = (f * 255.0) as u8;
    match i {
        0 => (255, t,   0),
        1 => (q,   255, 0),
        2 => (0,   255, t),
        3 => (0,   q,   255),
        4 => (t,   0,   255),
        _ => (255, 0,   q),
    }
}

fn label_value(row: Row, s: &Value) -> (&'static str, String) {
    match row {
        Row::Accent => ("tuiOS Colour", persistence::get_str(s, "appearance.accent_color", "Cyan")),
        Row::Clock => ("Clock", enabled(persistence::get_bool(s, "appearance.clock_enabled", false))),
        Row::ClockFormat => (
            "Clock Format",
            if persistence::get_bool(s, "appearance.clock_format_24h", true) { "24 hour".into() } else { "12 hour".into() },
        ),
        Row::ClockSeconds => (
            "Show Seconds",
            if persistence::get_bool(s, "appearance.clock_show_seconds", false) { "Yes".into() } else { "No".into() },
        ),
        Row::Dashboard => ("Default Dashboard", persistence::get_str(s, "default_dashboard", "Dashboard-1")),
        Row::Border => ("Border Style", persistence::get_str(s, "appearance.border_style", "Rounded")),
        Row::StatusBar => ("Status Bar", enabled(persistence::get_bool(s, "appearance.status_bar_enabled", false))),
        Row::NewWindow => (
            "New Window Apps",
            if persistence::get_bool(s, "appearance.new_window_keeps_tuios_open", false) {
                "Keep tuiOS open".into()
            } else {
                "Close tuiOS while running".into()
            },
        ),
        Row::NavBar => ("Navigation Bar", persistence::get_str(s, "appearance.navbar_position", "Top")),
        Row::BootSettings => ("Boot Up Animation", "Open ›".into()),
    }
}

fn enabled(on: bool) -> String {
    if on { "Enabled".into() } else { "Disabled".into() }
}

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, _scroll: usize, editing: bool) {
    let settings = persistence::load();

    let value_style = Style::default().fg(Color::White);
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows().iter().enumerate() {
        let (label, value) = label_value(*row, &settings);
        let is_active = i == cursor;
        let prefix = if is_active { "  » " } else { "    " };
        let style = if is_active {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            value_style
        };
        // For multi-option items in edit mode, show ◄ value ►
        if is_active && editing && row.cycles() {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<22}", prefix, label), style),
                Span::styled("  ◄ ", Style::default().fg(Color::Yellow)),
                Span::styled(value, Style::default().fg(Color::White)),
                Span::styled(" ►", Style::default().fg(Color::Yellow)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<22}", prefix, label), style),
                Span::styled(format!("  {}", value), style),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Returns true for items that use left/right cycling.
pub fn is_edit_mode_item(cursor: usize) -> bool {
    row_at(cursor).map_or(false, |row| row.cycles())
}

pub fn item_count() -> usize {
    rows().len()
}

/// Cycle a multi-option item forward (+1) or backward (-1).
pub fn handle_cycle(cursor: usize, forward: bool) {
    let mut settings = persistence::load();
    let row = match row_at(cursor) {
        Some(row) => row,
        None => return,
    };
    match row {
        Row::Accent => {
            let current = persistence::get_str(&settings, "appearance.accent_color", "Cyan");
            let next = cycle_option(&mut settings, "appearance.accent_color", ACCENT_COLORS, &current, forward);
            crate::utilities::logging::settings(&format!("tuiOS Colour changed to {}", next));
        }
        Row::Dashboard => {
            let dash_names = persistence::load_dashboard_names();
            if !dash_names.is_empty() {
                let current = persistence::get_str(&settings, "default_dashboard", "Dashboard-1");
                let idx = dash_names.iter().position(|d| d == &current).unwrap_or(0);
                let len = dash_names.len();
                let next_idx = if forward { (idx + 1) % len } else { (idx + len - 1) % len };
                let next = &dash_names[next_idx];
                persistence::set(&mut settings, "default_dashboard", serde_json::Value::String(next.clone()));
                crate::utilities::logging::settings(&format!("Default dashboard changed to {}", next));
            }
        }
        Row::Border => {
            let current = persistence::get_str(&settings, "appearance.border_style", "Rounded");
            let next = cycle_option(&mut settings, "appearance.border_style", BORDER_STYLES, &current, forward);
            crate::utilities::logging::settings(&format!("Border style changed to {}", next));
        }
        _ => {}
    }
    persistence::save(&settings);
}

/// Handle Enter for binary-toggle items (Clock, Clock Format, Show Seconds).
pub fn handle_enter(cursor: usize) {
    let mut settings = persistence::load();
    let row = match row_at(cursor) {
        Some(row) => row,
        None => return,
    };
    match row {
        Row::Clock => {
            let current = persistence::get_bool(&settings, "appearance.clock_enabled", false);
            persistence::set(&mut settings, "appearance.clock_enabled", serde_json::Value::Bool(!current));
            crate::utilities::logging::settings(&format!("Clock {}", if !current { "enabled" } else { "disabled" }));
        }
        Row::ClockFormat => {
            let current = persistence::get_bool(&settings, "appearance.clock_format_24h", true);
            persistence::set(&mut settings, "appearance.clock_format_24h", serde_json::Value::Bool(!current));
            crate::utilities::logging::settings(&format!("Clock format set to {}", if !current { "24h" } else { "12h" }));
        }
        Row::ClockSeconds => {
            let current = persistence::get_bool(&settings, "appearance.clock_show_seconds", false);
            persistence::set(&mut settings, "appearance.clock_show_seconds", serde_json::Value::Bool(!current));
            crate::utilities::logging::settings(&format!("Clock seconds {}", if !current { "shown" } else { "hidden" }));
        }
        Row::StatusBar => {
            let current = persistence::get_bool(&settings, "appearance.status_bar_enabled", false);
            persistence::set(&mut settings, "appearance.status_bar_enabled", serde_json::Value::Bool(!current));
            crate::utilities::logging::settings(&format!("Status bar {}", if !current { "enabled" } else { "disabled" }));
        }
        Row::NewWindow => {
            let current = persistence::get_bool(&settings, "appearance.new_window_keeps_tuios_open", false);
            persistence::set(&mut settings, "appearance.new_window_keeps_tuios_open", serde_json::Value::Bool(!current));
            crate::utilities::logging::settings(&format!(
                "New window apps: {}",
                if !current { "tuiOS stays open" } else { "tuiOS closes while running" }
            ));
        }
        Row::NavBar => {
            let current = persistence::get_str(&settings, "appearance.navbar_position", "Top");
            let next = if current == "Bottom" { "Top" } else { "Bottom" };
            persistence::set(&mut settings, "appearance.navbar_position", serde_json::Value::String(next.to_string()));
            crate::utilities::logging::settings(&format!("Navigation bar moved to {}", next.to_lowercase()));
        }
        _ => {}
    }
    persistence::save(&settings);
}
