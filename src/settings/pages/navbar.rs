/// Navigation bar settings — opened from the Appearance category.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use serde_json::Value;

use crate::settings::persistence;

use super::cycle_option;

const POSITIONS: &[&str] = &["Top", "Bottom"];

/// Rows in display order. The menu style only applies while apps are grouped.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Position,
    Organise,
    MenuStyle,
}

impl Row {
    /// True for rows picked from a fixed list with ◄ ►.
    fn cycles(self) -> bool {
        matches!(self, Row::Position)
    }
}

fn rows(settings: &Value) -> Vec<Row> {
    let mut rows = vec![Row::Position, Row::Organise];
    if group_by_category(settings) {
        rows.push(Row::MenuStyle);
    }
    rows
}

fn row_at(settings: &Value, cursor: usize) -> Option<Row> {
    rows(settings).get(cursor).copied()
}

pub fn group_by_category(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.group_apps_by_category", true)
}

pub fn category_submenu(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.app_category_submenu", false)
}

pub fn position(settings: &Value) -> String {
    let stored = persistence::get_str(settings, "appearance.navbar_position", "Top");
    if POSITIONS.contains(&stored.as_str()) { stored } else { "Top".into() }
}

fn label_value(row: Row, s: &Value) -> (&'static str, String) {
    match row {
        Row::Position => ("Position", position(s)),
        Row::Organise => (
            "Organise Apps by Category",
            if group_by_category(s) { "Enabled".into() } else { "Disabled".into() },
        ),
        Row::MenuStyle => (
            "Category Menu",
            if category_submenu(s) { "Submenu".into() } else { "Inline list".into() },
        ),
    }
}

fn hint(row: Row) -> &'static str {
    match row {
        Row::Position => "Draw the navigation bar along the top or the bottom of the screen.",
        Row::Organise => "Group the Apps dropdown under App Store category headings.",
        Row::MenuStyle => {
            "Submenu: categories open a second panel to the right (Enter/e opens, q/← goes back)."
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, _scroll: usize, editing: bool) {
    let settings = persistence::load();
    let rows = rows(&settings);

    let value_style = Style::default().fg(Color::White);
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
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

    if let Some(row) = rows.get(cursor) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("    {}", hint(*row)),
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Returns true for items that use left/right cycling.
pub fn is_edit_mode_item(cursor: usize) -> bool {
    row_at(&persistence::load(), cursor).map_or(false, |row| row.cycles())
}

pub fn item_count() -> usize {
    rows(&persistence::load()).len()
}

/// Cycle a multi-option item forward (+1) or backward (-1).
pub fn handle_cycle(cursor: usize, forward: bool) {
    let mut settings = persistence::load();
    if row_at(&settings, cursor) != Some(Row::Position) {
        return;
    }
    let current = position(&settings);
    let next = cycle_option(&mut settings, "appearance.navbar_position", POSITIONS, &current, forward);
    crate::utilities::logging::settings(&format!("Navigation bar moved to {}", next.to_lowercase()));
    persistence::save(&settings);
}

/// Handle Enter for the binary toggles. Returns true when the navbar needs rebuilding.
pub fn handle_enter(cursor: usize) -> bool {
    let mut settings = persistence::load();
    let row = match row_at(&settings, cursor) {
        Some(row) => row,
        None => return false,
    };
    let rebuild = match row {
        Row::Position => false,
        Row::Organise => {
            let current = group_by_category(&settings);
            persistence::set(
                &mut settings,
                "appearance.group_apps_by_category",
                Value::Bool(!current),
            );
            crate::utilities::logging::settings(&format!(
                "Apps dropdown grouping {}",
                if !current { "enabled" } else { "disabled" }
            ));
            true
        }
        Row::MenuStyle => {
            let current = category_submenu(&settings);
            persistence::set(
                &mut settings,
                "appearance.app_category_submenu",
                Value::Bool(!current),
            );
            crate::utilities::logging::settings(&format!(
                "Apps category menu set to {}",
                if !current { "submenu" } else { "inline list" }
            ));
            true
        }
    };
    persistence::save(&settings);
    rebuild
}
