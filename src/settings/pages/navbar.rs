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
pub const CATEGORY_SORTS: &[&str] = &["A → Z", "Z → A", "Custom"];
pub const APP_SORTS: &[&str] = &["A → Z", "Z → A", "Recently Installed"];

/// Rows in display order. The category rows only apply while apps are grouped.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Position,
    Organise,
    MenuStyle,
    CategorySort,
    CustomOrder,
    AppSort,
    Counts,
    Wrap,
}

impl Row {
    /// True for rows picked from a fixed list with ◄ ►.
    fn cycles(self) -> bool {
        matches!(self, Row::Position | Row::CategorySort | Row::AppSort)
    }
}

fn rows(settings: &Value) -> Vec<Row> {
    let mut rows = vec![Row::Position, Row::Organise];
    if group_by_category(settings) {
        rows.extend([Row::MenuStyle, Row::CategorySort]);
        if category_sort(settings) == "Custom" {
            rows.push(Row::CustomOrder);
        }
    }
    rows.push(Row::AppSort);
    if group_by_category(settings) {
        rows.push(Row::Counts);
    }
    rows.push(Row::Wrap);
    rows
}

fn row_at(settings: &Value, cursor: usize) -> Option<Row> {
    rows(settings).get(cursor).copied()
}

pub fn group_by_category(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.group_apps_by_category", true)
}

pub fn category_submenu(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.app_category_submenu", true)
}

pub fn show_counts(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.nav_show_counts", true)
}

pub fn wrap_around(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.nav_wrap_around", true)
}

pub fn position(settings: &Value) -> String {
    fixed(persistence::get_str(settings, "appearance.navbar_position", "Top"), POSITIONS)
}

pub fn category_sort(settings: &Value) -> String {
    fixed(
        persistence::get_str(settings, "appearance.nav_category_sort", CATEGORY_SORTS[0]),
        CATEGORY_SORTS,
    )
}

pub fn app_sort(settings: &Value) -> String {
    fixed(
        persistence::get_str(settings, "appearance.nav_app_sort", APP_SORTS[0]),
        APP_SORTS,
    )
}

/// The user's hand-ordered category list, used when the category sort is "Custom".
pub fn custom_order(settings: &Value) -> Vec<String> {
    persistence::get(settings, "appearance.nav_category_order")
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn set_custom_order(settings: &mut Value, order: &[String]) {
    let list = order.iter().map(|s| Value::String(s.clone())).collect();
    persistence::set(settings, "appearance.nav_category_order", Value::Array(list));
}

/// Fall back to the first option when the stored value is not one we know.
fn fixed(stored: String, options: &[&str]) -> String {
    if options.contains(&stored.as_str()) { stored } else { options[0].to_string() }
}

fn label_value(row: Row, s: &Value) -> (&'static str, String) {
    match row {
        Row::Position => ("Position", position(s)),
        Row::Organise => ("Organise Apps by Category", enabled(group_by_category(s))),
        Row::MenuStyle => (
            "Category Menu",
            if category_submenu(s) { "Submenu".into() } else { "Inline list".into() },
        ),
        Row::CategorySort => ("Category Order", category_sort(s)),
        Row::CustomOrder => ("Edit Custom Order", "Open ›".into()),
        Row::AppSort => ("App Order", app_sort(s)),
        Row::Counts => ("Show App Counts", enabled(show_counts(s))),
        Row::Wrap => ("Wrap-Around Menus", enabled(wrap_around(s))),
    }
}

fn enabled(on: bool) -> String {
    if on { "Enabled".into() } else { "Disabled".into() }
}

fn hint(row: Row) -> &'static str {
    match row {
        Row::Position => "Draw the navigation bar along the top or the bottom of the screen.",
        Row::Organise => "Group the Apps dropdown under App Store category headings.",
        Row::MenuStyle => {
            "Submenu: categories open a second panel to the right (Enter/e opens, q/← goes back)."
        }
        Row::CategorySort => "How the category headings are ordered in the Apps dropdown.",
        Row::CustomOrder => "Pick a category up with Enter, then move it with ↑ ↓.",
        Row::AppSort => "How apps are ordered, both inside a category and in a flat list.",
        Row::Counts => "Show how many apps each category holds, e.g. Games (4).",
        Row::Wrap => "Moving down past the last menu row jumps back to the first.",
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

/// True when the cursor is on the row that opens the custom order editor.
pub fn is_custom_order(cursor: usize) -> bool {
    row_at(&persistence::load(), cursor) == Some(Row::CustomOrder)
}

pub fn item_count() -> usize {
    rows(&persistence::load()).len()
}

/// Cycle a multi-option item. Returns true when the navbar needs rebuilding.
pub fn handle_cycle(cursor: usize, forward: bool) -> bool {
    let mut settings = persistence::load();
    let row = match row_at(&settings, cursor) {
        Some(row) => row,
        None => return false,
    };
    let rebuild = match row {
        Row::Position => {
            let current = position(&settings);
            let next = cycle_option(&mut settings, "appearance.navbar_position", POSITIONS, &current, forward);
            crate::utilities::logging::settings(&format!("Navigation bar moved to {}", next.to_lowercase()));
            false
        }
        Row::CategorySort => {
            let current = category_sort(&settings);
            let next = cycle_option(&mut settings, "appearance.nav_category_sort", CATEGORY_SORTS, &current, forward);
            crate::utilities::logging::settings(&format!("Apps category order set to {}", next));
            true
        }
        Row::AppSort => {
            let current = app_sort(&settings);
            let next = cycle_option(&mut settings, "appearance.nav_app_sort", APP_SORTS, &current, forward);
            crate::utilities::logging::settings(&format!("Apps dropdown order set to {}", next));
            true
        }
        _ => return false,
    };
    persistence::save(&settings);
    rebuild
}

/// Handle Enter for the binary toggles. Returns true when the navbar needs rebuilding.
pub fn handle_enter(cursor: usize) -> bool {
    let mut settings = persistence::load();
    let row = match row_at(&settings, cursor) {
        Some(row) => row,
        None => return false,
    };
    let rebuild = match row {
        Row::Organise => {
            let current = group_by_category(&settings);
            persistence::set(&mut settings, "appearance.group_apps_by_category", Value::Bool(!current));
            crate::utilities::logging::settings(&format!(
                "Apps dropdown grouping {}",
                if !current { "enabled" } else { "disabled" }
            ));
            true
        }
        Row::MenuStyle => {
            let current = category_submenu(&settings);
            persistence::set(&mut settings, "appearance.app_category_submenu", Value::Bool(!current));
            crate::utilities::logging::settings(&format!(
                "Apps category menu set to {}",
                if !current { "submenu" } else { "inline list" }
            ));
            true
        }
        Row::Counts => {
            let current = show_counts(&settings);
            persistence::set(&mut settings, "appearance.nav_show_counts", Value::Bool(!current));
            crate::utilities::logging::settings(&format!(
                "Apps dropdown counts {}",
                if !current { "shown" } else { "hidden" }
            ));
            true
        }
        Row::Wrap => {
            let current = wrap_around(&settings);
            persistence::set(&mut settings, "appearance.nav_wrap_around", Value::Bool(!current));
            crate::utilities::logging::settings(&format!(
                "Menu wrap-around {}",
                if !current { "enabled" } else { "disabled" }
            ));
            false
        }
        _ => return false,
    };
    persistence::save(&settings);
    rebuild
}
