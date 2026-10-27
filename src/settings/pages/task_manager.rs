/// Task Manager settings — sampling, graphs, stopping apps, network checks — and the
/// Resource Alerts sub-page opened from it.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use serde_json::Value;

use crate::settings::persistence;
use crate::task_manager::config as tm;

use super::cycle_option;

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Interval,
    BackgroundScan,
    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
    Nvml,
    History,
    GraphStyle,
    KillTimeout,
    ExitNotify,
    WarnOnExit,
    Scrollback,
    LatencyTarget,
    PublicIp,
    AlertsPage,
    Alerts,
    AlertCpu,
    AlertRam,
    AlertTemp,
    AlertSustain,
    PiPower,
}

const MAIN_ROWS: &[Row] = &[
    Row::Interval,
    Row::BackgroundScan,
    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
    Row::Nvml,
    Row::History,
    Row::GraphStyle,
    Row::KillTimeout,
    Row::ExitNotify,
    Row::WarnOnExit,
    Row::Scrollback,
    Row::LatencyTarget,
    Row::PublicIp,
    Row::AlertsPage,
];

const ALERT_ROWS: &[Row] = &[
    Row::Alerts,
    Row::AlertCpu,
    Row::AlertRam,
    Row::AlertTemp,
    Row::AlertSustain,
    Row::PiPower,
];

enum Kind {
    Cycle { key: &'static str, options: &'static [&'static str], default: &'static str },
    Toggle { key: &'static str, default: bool },
    Open,
}

fn spec(row: Row) -> (&'static str, Kind, &'static str) {
    use Kind::*;
    match row {
        Row::Interval => (
            "Sample Interval",
            Cycle { key: tm::KEY_INTERVAL, options: tm::INTERVALS, default: "1 s" },
            "How often CPU, memory, GPU and network are measured. Faster costs more CPU.",
        ),
        Row::BackgroundScan => (
            "Background App Stats",
            Cycle { key: tm::KEY_BACKGROUND_SCAN, options: tm::BACKGROUND_SCANS, default: "5 s" },
            "How often apps' CPU / memory are measured while the Task Manager is closed. Off = lightest.",
        ),
        #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
        Row::Nvml => (
            "NVIDIA GPU Stats",
            Toggle { key: tm::KEY_NVML, default: true },
            "Read NVIDIA cards via the driver's NVML library. Turn off if this PC has no NVIDIA card.",
        ),
        Row::History => (
            "History Length",
            Cycle { key: tm::KEY_HISTORY, options: tm::HISTORIES, default: "15 min" },
            "How far back the graphs can go (+ / - in the Task Manager zooms).",
        ),
        Row::GraphStyle => (
            "Default Graph Style",
            Cycle { key: tm::KEY_GRAPH_STYLE, options: tm::GRAPH_STYLES, default: "Line" },
            "Line = smooth braille charts; Sparkline = solid bars. g switches while open.",
        ),
        Row::KillTimeout => (
            "Force-Quit After",
            Cycle { key: tm::KEY_KILL_TIMEOUT, options: tm::KILL_TIMEOUTS, default: "3 s" },
            "Quit asks politely first (Ctrl+C), then forces the app closed after this long.",
        ),
        Row::ExitNotify => (
            "Exit Notifications",
            Toggle { key: tm::KEY_EXIT_NOTIFY, default: true },
            "Show a message when a background app quits or crashes.",
        ),
        Row::WarnOnExit => (
            "Warn Before Shutdown",
            Toggle { key: tm::KEY_WARN_ON_EXIT, default: true },
            "Ask before Shutdown / Restart closes apps that are still running.",
        ),
        Row::Scrollback => (
            "Embedded Scrollback",
            Cycle { key: tm::KEY_SCROLLBACK, options: tm::SCROLLBACKS, default: "1000 lines" },
            "History kept for apps in the container (Shift+PgUp / Shift+PgDn). New apps only.",
        ),
        Row::LatencyTarget => (
            "Latency Target",
            Cycle { key: tm::KEY_LATENCY_TARGET, options: tm::LATENCY_TARGETS, default: "1.1.1.1" },
            "Host the Network tab measures round-trip time to (TCP port 443).",
        ),
        Row::PublicIp => (
            "Public IP Lookup",
            Toggle { key: tm::KEY_PUBLIC_IP, default: true },
            "Ask api.ipify.org for your public address while the Task Manager is open.",
        ),
        Row::AlertsPage => ("Resource Alerts", Open, "Warnings when CPU, memory or temperature stay high."),
        Row::Alerts => (
            "Resource Alerts",
            Toggle { key: tm::KEY_ALERTS, default: true },
            "Show a message when usage stays above a threshold.",
        ),
        Row::AlertCpu => (
            "CPU Above",
            Cycle { key: tm::KEY_ALERT_CPU, options: tm::USAGE_THRESHOLDS, default: "90%" },
            "Total CPU usage that counts as high.",
        ),
        Row::AlertRam => (
            "Memory Above",
            Cycle { key: tm::KEY_ALERT_RAM, options: tm::USAGE_THRESHOLDS, default: "90%" },
            "Memory usage that counts as high.",
        ),
        Row::AlertTemp => (
            "Temperature Above",
            Cycle { key: tm::KEY_ALERT_TEMP, options: tm::TEMP_THRESHOLDS, default: "85 °C" },
            "Hottest CPU or GPU sensor that counts as high.",
        ),
        Row::AlertSustain => (
            "For At Least",
            Cycle { key: tm::KEY_ALERT_SUSTAIN, options: tm::SUSTAIN_TIMES, default: "10 s" },
            "How long a value must stay high before you are told (then 5 min quiet).",
        ),
        Row::PiPower => (
            "Pi Power Warnings",
            Toggle { key: tm::KEY_PI_POWER, default: true },
            "Raspberry Pi only: warn on under-voltage, throttling or the soft temperature limit.",
        ),
    }
}

fn rows(alerts_page: bool) -> &'static [Row] {
    if alerts_page { ALERT_ROWS } else { MAIN_ROWS }
}

fn row_at(cursor: usize, alerts_page: bool) -> Option<Row> {
    rows(alerts_page).get(cursor).copied()
}

fn value(row: Row, s: &Value) -> String {
    match spec(row).1 {
        Kind::Cycle { key, options, default } => tm::choice(s, key, options, default),
        Kind::Toggle { key, default } => {
            if persistence::get_bool(s, key, default) { "Enabled".into() } else { "Disabled".into() }
        }
        Kind::Open => "Open ›".into(),
    }
}

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, editing: bool, alerts_page: bool) {
    let settings = persistence::load();
    let rows = rows(alerts_page);
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let (label, kind, _) = spec(*row);
        let value = value(*row, &settings);
        let is_active = i == cursor;
        let prefix = if is_active { "  » " } else { "    " };
        let style = if is_active {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        if is_active && editing && matches!(kind, Kind::Cycle { .. }) {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<26}", prefix, label), style),
                Span::styled("  ◄ ", Style::default().fg(Color::Yellow)),
                Span::styled(value, Style::default().fg(Color::White)),
                Span::styled(" ►", Style::default().fg(Color::Yellow)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<26}", prefix, label), style),
                Span::styled(format!("  {}", value), style),
            ]));
        }
    }
    if let Some(row) = rows.get(cursor) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("    {}", spec(*row).2),
            Style::default().fg(Color::DarkGray),
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub fn item_count(alerts_page: bool) -> usize {
    rows(alerts_page).len()
}

pub fn is_edit_mode_item(cursor: usize, alerts_page: bool) -> bool {
    row_at(cursor, alerts_page).map_or(false, |row| matches!(spec(row).1, Kind::Cycle { .. }))
}

/// True when the cursor is on the row that opens the Resource Alerts page.
pub fn is_alerts_page(cursor: usize) -> bool {
    row_at(cursor, false) == Some(Row::AlertsPage)
}

pub fn handle_cycle(cursor: usize, forward: bool, alerts_page: bool) {
    let Some(row) = row_at(cursor, alerts_page) else { return };
    let (label, kind, _) = spec(row);
    if let Kind::Cycle { key, options, default } = kind {
        let mut settings = persistence::load();
        let current = tm::choice(&settings, key, options, default);
        let next = cycle_option(&mut settings, key, options, &current, forward);
        persistence::save(&settings);
        crate::utilities::logging::settings(&format!("Task Manager: {} set to {}", label, next));
    }
}

/// Flip a toggle row.
pub fn handle_enter(cursor: usize, alerts_page: bool) {
    let Some(row) = row_at(cursor, alerts_page) else { return };
    let (label, kind, _) = spec(row);
    if let Kind::Toggle { key, default } = kind {
        let mut settings = persistence::load();
        let on = !persistence::get_bool(&settings, key, default);
        persistence::set(&mut settings, key, Value::Bool(on));
        persistence::save(&settings);
        crate::utilities::logging::settings(&format!(
            "Task Manager: {} {}",
            label,
            if on { "enabled" } else { "disabled" }
        ));
    }
}
