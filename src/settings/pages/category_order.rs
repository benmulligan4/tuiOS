/// Custom category order editor — opened from the Navigation Bar page.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::settings::persistence;
use crate::tuios::registry;

use super::navbar;

/// Every category that currently has an app in the Apps dropdown, in the saved order.
pub fn categories() -> Vec<String> {
    let apps = registry::load_apps();
    let registered = registry::load_registered_apps();

    let mut present: Vec<String> = Vec::new();
    for key in apps.keys() {
        let category = registered
            .get(key)
            .and_then(|m| m.get("category"))
            .and_then(|v| v.as_str())
            .unwrap_or("Other")
            .to_string();
        if !present.contains(&category) {
            present.push(category);
        }
    }

    let saved = navbar::custom_order(&persistence::load());
    present.sort_by_key(|c| {
        (
            saved.iter().position(|s| s == c).unwrap_or(usize::MAX),
            c.to_lowercase(),
        )
    });
    present
}

pub fn item_count() -> usize {
    categories().len()
}

/// Every row is picked up with Enter and then moved with ↑ ↓.
pub fn is_edit_mode_item(_cursor: usize) -> bool {
    true
}

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, _scroll: usize, editing: bool) {
    let cats = categories();
    if cats.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "    No app categories yet.",
                Style::default().fg(Color::DarkGray),
            ))),
            area,
        );
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in cats.iter().enumerate() {
        let is_active = i == cursor;
        let moving = is_active && editing;
        let prefix = if moving {
            "  ▲▼"
        } else if is_active {
            "  » "
        } else {
            "    "
        };
        let style = if moving {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let suffix = if moving { "  ← moving" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{:>2}. {:<24}", prefix, i + 1, name), style),
            Span::styled(suffix, Style::default().fg(Color::Yellow)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        if editing {
            "    ↑ ↓ move the category · Enter or q drops it here."
        } else {
            "    Enter picks a category up, then ↑ ↓ move it."
        },
        Style::default().fg(if editing { Color::Yellow } else { Color::DarkGray }),
    )));

    frame.render_widget(Paragraph::new(lines), area);
}

/// Move the selected category one place (forward = down). Returns its new index.
pub fn handle_cycle(cursor: usize, forward: bool) -> usize {
    let mut cats = categories();
    if cats.is_empty() {
        return 0;
    }
    let target = if forward {
        if cursor + 1 >= cats.len() { return cursor; }
        cursor + 1
    } else {
        if cursor == 0 { return cursor; }
        cursor - 1
    };
    cats.swap(cursor, target);

    let mut settings = persistence::load();
    navbar::set_custom_order(&mut settings, &cats);
    persistence::save(&settings);
    crate::utilities::logging::settings(&format!(
        "Category '{}' moved to position {}",
        cats[target],
        target + 1
    ));
    target
}
