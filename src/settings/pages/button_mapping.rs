/// Button Mapping settings.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::settings::persistence;
use crate::settings::state::SettingsState;
use crate::task_manager::config as tm;

/// (label, settings key, default) for each rebindable key, in display order.
const BINDINGS: &[(&str, &str, &str)] = &[
    ("Nav/Main Toggle Key", "button_mapping.focus_toggle_key", "Tab"),
    ("App Switcher Key", tm::KEY_SWITCHER, tm::DEFAULT_SWITCHER_KEY),
    ("Task Manager Key", tm::KEY_TASK_MANAGER, tm::DEFAULT_TASK_MANAGER_KEY),
];

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, _scroll: usize, awaiting_key: bool) {
    let settings = persistence::load();
    let mut lines: Vec<Line> = Vec::new();

    for (i, (label, key, default)) in BINDINGS.iter().enumerate() {
        let selected = i == cursor;
        let display_value = if selected && awaiting_key {
            "Press a key...".to_string()
        } else {
            persistence::get_str(&settings, key, default)
        };
        let prefix = if selected { "  » " } else { "    " };
        let style = if selected {
            if awaiting_key {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            }
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<24}", prefix, label), style),
            Span::styled(format!("  {}", display_value), style),
        ]));
    }

    if cursor >= 1 && cursor < BINDINGS.len() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "    Works everywhere, even inside apps — pick a key your apps do not use.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

pub fn item_count() -> usize {
    BINDINGS.len()
}

/// Called when Enter is pressed — starts key capture mode.
pub fn handle_enter(ss: &mut SettingsState) {
    ss.awaiting_key = true;
}

/// Called when a key is captured during awaiting_key mode.
pub fn capture_key(ss: &mut SettingsState, key_name: &str) {
    let (label, key, _) = BINDINGS[ss.right_cursor.min(BINDINGS.len() - 1)];
    let mut settings = persistence::load();
    persistence::set(&mut settings, key, serde_json::Value::String(key_name.to_string()));
    persistence::save(&settings);
    crate::utilities::logging::settings(&format!("{} set to {}", label, key_name));
    ss.awaiting_key = false;
}
