/// Quick app switcher: a hotkey overlay listing the home screen and every running app,
/// most recently used first (Alt+Tab style). Pressing the hotkey again moves down.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::tuios::keybind;
use crate::tuios::models::{FocusTarget, TuiosState};
use crate::tuios::sessions::{Kind, SessionEvent, SessionId, Sessions};

use super::config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Home,
    Session(SessionId),
    TaskManager,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub target: Target,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct SwitcherState {
    pub cursor: usize,
    pub entries: Vec<Entry>,
}

/// Open the switcher, or move down if it is already open.
pub fn open(state: &mut TuiosState, sessions: &Sessions) {
    if let Some(sw) = state.switcher.as_mut() {
        sw.cursor = (sw.cursor + 1) % sw.entries.len().max(1);
        return;
    }

    let home_label = crate::tuios::registry::load_dashboards()
        .get(&state.active_dashboard)
        .and_then(|m| m.get("label"))
        .and_then(|v| v.as_str())
        .unwrap_or(&state.active_dashboard)
        .to_string();
    let mut entries = vec![Entry {
        target: Target::Home,
        label: "Home".into(),
        detail: home_label,
    }];
    let mut live: Vec<_> = sessions.iter().filter(|s| s.is_live() && !s.is_stopping()).collect();
    live.sort_by(|a, b| b.last_focus.cmp(&a.last_focus));
    for s in &live {
        let detail = match (s.kind, s.is_external()) {
            (_, true) => "own window".to_string(),
            (Kind::Dashboard, _) => "dashboard".to_string(),
            (Kind::App, _) => s.status_label(sessions.foreground == Some(s.id)).to_lowercase(),
        };
        entries.push(Entry { target: Target::Session(s.id), label: s.label.clone(), detail });
    }
    entries.push(Entry { target: Target::TaskManager, label: "Task Manager".into(), detail: String::new() });

    // Like Alt+Tab: start on the most recent app that is not already on screen
    let showing = if state.active_page.is_some() { None } else { sessions.foreground };
    let cursor = entries
        .iter()
        .position(|e| matches!(e.target, Target::Session(id) if Some(id) != showing))
        .unwrap_or(0);
    state.switcher = Some(SwitcherState { cursor, entries });
}

pub fn handle_key(key: KeyEvent, state: &mut TuiosState, sessions: &mut Sessions) -> Vec<SessionEvent> {
    let Some(sw) = state.switcher.as_mut() else { return Vec::new() };
    let len = sw.entries.len().max(1);
    let hotkey = config::switcher_key(&crate::settings::persistence::load());
    if keybind::matches(&key, &hotkey) {
        sw.cursor = (sw.cursor + 1) % len;
        return Vec::new();
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('8') | KeyCode::BackTab => {
            sw.cursor = (sw.cursor + len - 1) % len;
        }
        KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('2') | KeyCode::Tab => {
            sw.cursor = (sw.cursor + 1) % len;
        }
        KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('9') => {
            let target = sw.entries.get(sw.cursor).map(|e| e.target);
            state.switcher = None;
            return match target {
                Some(t) => activate(t, state, sessions),
                None => Vec::new(),
            };
        }
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace | KeyCode::Char('7') => {
            state.switcher = None;
        }
        _ => {}
    }
    Vec::new()
}

fn activate(target: Target, state: &mut TuiosState, sessions: &Sessions) -> Vec<SessionEvent> {
    match target {
        Target::Home => {
            state.active_page = None;
            state.active_app = None;
            state.page_return = None;
            state.focus = FocusTarget::Main;
            Vec::new()
        }
        Target::Session(id) => match sessions.get(id) {
            Some(s) if s.is_external() => vec![SessionEvent::Toast(format!(
                "{} is in its own window — switch to it there, or use the Task Manager to bring it into tuiOS",
                s.label
            ))],
            Some(_) => vec![SessionEvent::Foreground(id)],
            None => Vec::new(),
        },
        Target::TaskManager => {
            state.pending_nav_action = Some("page:taskmanager".into());
            Vec::new()
        }
    }
}

pub fn render(frame: &mut Frame, state: &TuiosState, _sessions: &Sessions) {
    let Some(sw) = &state.switcher else { return };
    let settings = crate::settings::persistence::load();
    let accent = crate::settings::pages::appearance::color_from_name(&crate::settings::persistence::get_str(
        &settings,
        "appearance.accent_color",
        "Cyan",
    ));
    let area = frame.area();
    let width = 52u16.min(area.width);
    let height = (sw.entries.len() as u16 + 4).min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(" Switch To ", Style::default().fg(accent).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(rect).inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(block, rect);

    let visible = inner.height.saturating_sub(2) as usize;
    let scroll = sw.cursor.saturating_sub(visible.saturating_sub(1));
    let mut lines = Vec::new();
    for (i, e) in sw.entries.iter().enumerate().skip(scroll).take(visible) {
        let selected = i == sw.cursor;
        let icon = match e.target {
            Target::Home => "⌂",
            Target::Session(_) => "●",
            Target::TaskManager => "≡",
        };
        let base = if selected {
            Style::default().fg(Color::Black).bg(accent)
        } else {
            Style::default().fg(Color::White)
        };
        let detail_style = if selected { base } else { Style::default().fg(Color::DarkGray) };
        let text_w = inner.width as usize;
        let detail = if e.detail.is_empty() { String::new() } else { format!("{} ", e.detail) };
        let label_w = text_w.saturating_sub(detail.chars().count() + 4);
        let label: String = e.label.chars().take(label_w).collect();
        let pad = label_w.saturating_sub(label.chars().count());
        lines.push(Line::from(vec![
            Span::styled(format!(" {} {}{}", icon, label, " ".repeat(pad)), base),
            Span::styled(format!(" {}", detail), detail_style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ choose · Enter switch · Esc close",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}
