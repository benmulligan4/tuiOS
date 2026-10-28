/// Task Manager bar settings — opened from the Navigation Bar category. The stats
/// shown and the status colours are sub-pages so no page outgrows the settings pane.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use serde_json::Value;

use crate::settings::persistence;
use crate::settings::state::SubPage;
use crate::task_manager::bar_config::{self as bc, Stat, Status, TileInfo};
use crate::task_manager::config::choice;

use super::cycle_option;

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Enabled,
    Width,
    StatsPosition,
    StatsPage,
    TileInfo,
    RotationPage,
    Group,
    ColoursPage,
    Stat(Stat),
    Colour(Status),
    Rotate(TileInfo),
    RotateEvery,
}

enum Kind {
    Cycle { key: &'static str, options: &'static [&'static str], default: &'static str },
    Toggle { key: &'static str, default: bool },
    Open(SubPage),
}

fn rows(page: SubPage) -> Vec<Row> {
    match page {
        SubPage::TaskBarStats => Stat::ALL.iter().map(|s| Row::Stat(*s)).collect(),
        SubPage::TaskBarColours => Status::ALL.iter().map(|s| Row::Colour(*s)).collect(),
        SubPage::TaskBarRotation => {
            let mut rows: Vec<Row> = TileInfo::ROTATABLE.iter().map(|t| Row::Rotate(*t)).collect();
            rows.push(Row::RotateEvery);
            rows
        }
        _ => vec![
            Row::Enabled,
            Row::Width,
            Row::StatsPosition,
            Row::StatsPage,
            Row::TileInfo,
            Row::RotationPage,
            Row::Group,
            Row::ColoursPage,
        ],
    }
}

fn row_at(page: SubPage, cursor: usize) -> Option<Row> {
    rows(page).get(cursor).copied()
}

fn spec(row: Row) -> (String, Kind, &'static str) {
    use Kind::*;
    match row {
        Row::Enabled => (
            "Task Manager Bar".into(),
            Toggle { key: bc::KEY_ENABLED, default: true },
            "Running apps beside the navbar, with a stats line under it. Off = the one-line navbar.",
        ),
        Row::Width => (
            "Bar Width".into(),
            Cycle { key: bc::KEY_WIDTH, options: bc::WIDTHS, default: "Auto" },
            "Auto: the navbar takes what its items need and the bar gets the rest.",
        ),
        Row::StatsPosition => (
            "Stats Line Position".into(),
            Cycle { key: bc::KEY_STATS_POSITION, options: bc::STATS_POSITIONS, default: "Below Navbar" },
            "Put the stats line below or above the navbar, wherever the navbar is.",
        ),
        Row::StatsPage => (
            "Stats Shown".into(),
            Open(SubPage::TaskBarStats),
            "Choose what the stats line shows (CPU and RAM by default).",
        ),
        Row::TileInfo => (
            "Tile Shows".into(),
            Cycle { key: bc::KEY_TILE_INFO, options: bc::TILE_INFOS, default: "Rotate" },
            "The line under each app's name. Rotate cycles through your picks; Off = name only.",
        ),
        Row::RotationPage => (
            "Rotation".into(),
            Open(SubPage::TaskBarRotation),
            "What Rotate cycles through, and how long each one shows.",
        ),
        Row::Group => (
            "Group App Instances".into(),
            Toggle { key: bc::KEY_GROUP, default: true },
            "Show several copies of one app as a single tile, e.g. Notes ×2.",
        ),
        Row::ColoursPage => ("Status Colours".into(), Open(SubPage::TaskBarColours), "Pick the border colour for each app status."),
        Row::Stat(stat) => (
            stat.label().into(),
            Toggle { key: stat.key(), default: stat.on_by_default() },
            "Shown on the stats line beside the navbar. Stats that do not fit are left out.",
        ),
        Row::Colour(status) => (
            status.label().into(),
            Cycle { key: status.key(), options: bc::COLOURS, default: status.default_colour() },
            "Border colour for tiles in this state. Accent = your tuiOS colour.",
        ),
        Row::Rotate(info) => (
            info.label().into(),
            Toggle { key: info.rotate_key(), default: info.rotates_by_default() },
            match info {
                TileInfo::CpuMemory => "e.g. 3% · 120 MB, added up across grouped copies.",
                TileInfo::CpuGraph => "A small graph of recent CPU use, newest on the right.",
                _ => "Included when Tile Shows is set to Rotate.",
            },
        ),
        Row::RotateEvery => (
            "Rotate Every".into(),
            Cycle { key: bc::KEY_ROTATE_EVERY, options: bc::ROTATE_INTERVALS, default: "3 s" },
            "How long each one shows before the tiles move on to the next.",
        ),
    }
}

fn value(row: Row, s: &Value) -> String {
    match spec(row).1 {
        Kind::Cycle { key, options, default } => choice(s, key, options, default),
        Kind::Toggle { key, default } => {
            if persistence::get_bool(s, key, default) { "Enabled".into() } else { "Disabled".into() }
        }
        Kind::Open(_) => "Open ›".into(),
    }
}

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, editing: bool, page: SubPage) {
    let settings = persistence::load();
    let accent = crate::settings::pages::appearance::color_from_name(&persistence::get_str(
        &settings,
        "appearance.accent_color",
        "Cyan",
    ));
    let mut lines: Vec<Line> = Vec::new();
    let rows = rows(page);
    for (i, row) in rows.iter().enumerate() {
        let (label, kind, _) = spec(*row);
        let value = value(*row, &settings);
        let active = i == cursor;
        let prefix = if active { "  » " } else { "    " };
        let style = if active {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let mut spans = vec![Span::styled(format!("{}{:<26}", prefix, label), style)];
        if let Row::Colour(_) = row {
            let swatch = if value == "Accent" { accent } else { crate::settings::pages::appearance::color_from_name(&value) };
            spans.push(Span::styled("  ■", Style::default().fg(swatch)));
        }
        if active && editing && matches!(kind, Kind::Cycle { .. }) {
            spans.push(Span::styled("  ◄ ", Style::default().fg(Color::Yellow)));
            spans.push(Span::styled(value, Style::default().fg(Color::White)));
            spans.push(Span::styled(" ►", Style::default().fg(Color::Yellow)));
        } else {
            spans.push(Span::styled(format!("  {}", value), style));
        }
        lines.push(Line::from(spans));
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

pub fn item_count(page: SubPage) -> usize {
    rows(page).len()
}

pub fn is_edit_mode_item(cursor: usize, page: SubPage) -> bool {
    row_at(page, cursor).map_or(false, |row| matches!(spec(row).1, Kind::Cycle { .. }))
}

/// The sub-page the row under the cursor opens, if any.
pub fn opens(cursor: usize, page: SubPage) -> Option<SubPage> {
    match spec(row_at(page, cursor)?).1 {
        Kind::Open(sub) => Some(sub),
        _ => None,
    }
}

pub fn handle_cycle(cursor: usize, forward: bool, page: SubPage) {
    let Some(row) = row_at(page, cursor) else { return };
    let (label, kind, _) = spec(row);
    if let Kind::Cycle { key, options, default } = kind {
        let mut settings = persistence::load();
        let current = choice(&settings, key, options, default);
        let next = cycle_option(&mut settings, key, options, &current, forward);
        persistence::save(&settings);
        crate::utilities::logging::settings(&format!("Task Manager bar: {} set to {}", label, next));
    }
}

pub fn handle_enter(cursor: usize, page: SubPage) {
    let Some(row) = row_at(page, cursor) else { return };
    let (label, kind, _) = spec(row);
    if let Kind::Toggle { key, default } = kind {
        let mut settings = persistence::load();
        let on = !persistence::get_bool(&settings, key, default);
        persistence::set(&mut settings, key, Value::Bool(on));
        persistence::save(&settings);
        crate::utilities::logging::settings(&format!(
            "Task Manager bar: {} {}",
            label,
            if on { "enabled" } else { "disabled" }
        ));
    }
}
