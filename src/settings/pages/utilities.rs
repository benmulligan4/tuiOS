/// Utility settings — on-screen keyboard toggle, clear logs.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::settings::persistence;

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, _scroll: usize) {
    let settings = persistence::load();
    let osk_enabled = persistence::get_bool(&settings, "utilities.onscreen_keyboard_enabled", false);

    let value_style = Style::default().fg(Color::White);

    let items: &[(&str, String)] = &[
        ("On-Screen Keyboard", if osk_enabled { "Enabled".into() } else { "Disabled".into() }),
        ("Clear Logs", "".into()),
    ];

    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value)) in items.iter().enumerate() {
        let is_active = i == cursor;
        let prefix = if is_active { "  » " } else { "    " };
        let style = if is_active {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            value_style
        };
        if value.is_empty() {
            lines.push(Line::from(Span::styled(format!("{}{}", prefix, label), style)));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<24}", prefix, label), style),
                Span::styled(format!("  {}", value), style),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

pub fn item_count() -> usize { 2 }

pub fn handle_enter(cursor: usize) -> bool {
    match cursor {
        0 => {
            let mut settings = persistence::load();
            let current = persistence::get_bool(&settings, "utilities.onscreen_keyboard_enabled", false);
            persistence::set(&mut settings, "utilities.onscreen_keyboard_enabled", serde_json::Value::Bool(!current));
            persistence::save(&settings);
            crate::utilities::logging::settings(&format!("On-Screen Keyboard {}", if !current { "enabled" } else { "disabled" }));
            false
        }
        1 => {
            let _ = std::fs::write("tuios.log", "");
            true // Show popup
        }
        _ => false,
    }
}
