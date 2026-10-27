/// Dashboard settings — options for Dashboard 1 — Detailed.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use serde_json::Value;

use crate::dashboards::dashboard_1::online;
use crate::settings::persistence;

/// Presets for the weather location. Any city name can be set by hand in
/// config/settings.json — a stored value outside this list is kept as-is.
const LOCATIONS: &[&str] = &[
    "London", "Edinburgh", "Dublin", "Paris", "Berlin", "Madrid", "Rome", "Amsterdam",
    "Stockholm", "Reykjavik", "New York", "Chicago", "Denver", "Los Angeles", "Toronto",
    "Mexico City", "São Paulo", "Lagos", "Cairo", "Dubai", "Mumbai", "Singapore",
    "Hong Kong", "Tokyo", "Seoul", "Sydney", "Auckland",
];

const UNITS: &[&str] = &["Celsius", "Fahrenheit"];

#[derive(Clone, Copy, PartialEq)]
enum Row {
    OnlinePanels,
    WeatherLocation,
    WeatherUnits,
    ClockSegments,
}

impl Row {
    /// True for rows picked from a fixed list with ◄ ►.
    fn cycles(self) -> bool {
        matches!(self, Row::WeatherLocation | Row::WeatherUnits)
    }
}

fn rows() -> Vec<Row> {
    vec![
        Row::OnlinePanels,
        Row::WeatherLocation,
        Row::WeatherUnits,
        Row::ClockSegments,
    ]
}

fn row_at(cursor: usize) -> Option<Row> {
    rows().get(cursor).copied()
}

fn label_value(row: Row, s: &Value) -> (&'static str, String) {
    match row {
        Row::OnlinePanels => (
            "Online Panels",
            enabled(persistence::get_bool(s, "dashboard.online_panels", true)),
        ),
        Row::WeatherLocation => ("Weather Location", online::configured_location(s)),
        Row::WeatherUnits => (
            "Weather Units",
            persistence::get_str(s, "dashboard.weather_units", "Celsius"),
        ),
        Row::ClockSegments => (
            "Clock Segments",
            if persistence::get_bool(s, "dashboard.clock_ghost_segments", true) {
                "Show unlit".into()
            } else {
                "Lit only".into()
            },
        ),
    }
}

fn enabled(on: bool) -> String {
    if on {
        "Enabled".into()
    } else {
        "Disabled".into()
    }
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
        if is_active && editing && row.cycles() {
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

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    Weather and connectivity use open-meteo.com and api.ipify.org.",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "    Set any city by editing dashboard.weather_location in settings.json.",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), area);
}

pub fn is_edit_mode_item(cursor: usize) -> bool {
    row_at(cursor).map_or(false, |row| row.cycles())
}

pub fn item_count() -> usize {
    rows().len()
}

pub fn handle_cycle(cursor: usize, forward: bool) {
    let mut settings = persistence::load();
    let Some(row) = row_at(cursor) else { return };
    match row {
        Row::WeatherLocation => {
            let current = online::configured_location(&settings);
            // A hand-edited city is not in the preset list; start from the top.
            let idx = LOCATIONS.iter().position(|&l| l == current);
            let len = LOCATIONS.len();
            let next = match idx {
                Some(i) => LOCATIONS[if forward { (i + 1) % len } else { (i + len - 1) % len }],
                None => LOCATIONS[0],
            };
            persistence::set(
                &mut settings,
                "dashboard.weather_location",
                Value::String(next.to_string()),
            );
            crate::utilities::logging::settings(&format!("Weather location changed to {}", next));
        }
        Row::WeatherUnits => {
            let current = persistence::get_str(&settings, "dashboard.weather_units", "Celsius");
            let next = super::cycle_option(
                &mut settings,
                "dashboard.weather_units",
                UNITS,
                &current,
                forward,
            );
            crate::utilities::logging::settings(&format!("Weather units changed to {}", next));
        }
        _ => return,
    }
    persistence::save(&settings);
}

pub fn handle_enter(cursor: usize) {
    let mut settings = persistence::load();
    let Some(row) = row_at(cursor) else { return };
    match row {
        Row::OnlinePanels => {
            let next = !persistence::get_bool(&settings, "dashboard.online_panels", true);
            persistence::set(&mut settings, "dashboard.online_panels", Value::Bool(next));
            crate::utilities::logging::settings(&format!(
                "Dashboard online panels {}",
                if next { "enabled" } else { "disabled" }
            ));
        }
        Row::ClockSegments => {
            let next = !persistence::get_bool(&settings, "dashboard.clock_ghost_segments", true);
            persistence::set(
                &mut settings,
                "dashboard.clock_ghost_segments",
                Value::Bool(next),
            );
            crate::utilities::logging::settings(&format!(
                "Dashboard clock unlit segments {}",
                if next { "shown" } else { "hidden" }
            ));
        }
        _ => return,
    }
    persistence::save(&settings);
}
