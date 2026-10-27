/// Task Manager keys. Navigation arrives as `Action`s (arrows, WASD and the numpad all
/// work); letter shortcuts arrive as raw key events through `handle_raw`.

use crossterm::event::{KeyCode, KeyEvent};

use crate::app_store::actions as store;
use crate::settings::persistence;
use crate::tuios::models::{Action, TuiosState};
use crate::tuios::sessions::{SessionEvent, SessionId, Sessions};
use crate::utilities::logging;

use super::state::{self, actions_for, Confirm, Focus, RowId, Tab, TmAction, WINDOWS};
use super::{config, metrics, net, netinfo, page, perf};

fn toast(msg: impl Into<String>) -> Vec<SessionEvent> {
    vec![SessionEvent::Toast(msg.into())]
}

pub fn handle_action(action: Action, state: &mut TuiosState, sessions: &mut Sessions) -> Vec<SessionEvent> {
    if state.task_manager.confirm.is_some() {
        return confirm_key(action, state, sessions);
    }
    if let Some(id) = state.task_manager.viewer {
        viewer_key(action, state, sessions, id);
        return Vec::new();
    }
    if state.task_manager.focus == Focus::Tabs {
        tabs_key(action, state);
        return Vec::new();
    }
    match state.task_manager.tab {
        Tab::Apps => apps_key(action, state, sessions),
        Tab::Performance => {
            perf_key(action, state);
            Vec::new()
        }
        Tab::Network => {
            net_key(action, state);
            Vec::new()
        }
        Tab::Startup => {
            startup_key(action, state);
            Vec::new()
        }
    }
}

/// Esc closes the innermost Task Manager overlay. False when there was none, so Esc
/// keeps its usual meaning.
pub fn close_overlay(state: &mut TuiosState) -> bool {
    let tm = &mut state.task_manager;
    if tm.confirm.take().is_some() {
        return true;
    }
    if tm.viewer.take().is_some() {
        return true;
    }
    if tm.perf_zoom {
        tm.perf_zoom = false;
        return true;
    }
    if tm.focus == Focus::Actions {
        tm.focus = Focus::Content;
        return true;
    }
    false
}

fn tabs_key(action: Action, state: &mut TuiosState) {
    let tm = &mut state.task_manager;
    match action {
        Action::Left => tm.tab = tm.tab.step(false),
        Action::Right => tm.tab = tm.tab.step(true),
        Action::Down | Action::Enter => tm.focus = Focus::Content,
        Action::Back => {
            state.active_page = None;
            state.active_app = None;
        }
        _ => {}
    }
}

// ── Apps tab ─────────────────────────────────────────────────────────────────

fn apps_key(action: Action, state: &mut TuiosState, sessions: &mut Sessions) -> Vec<SessionEvent> {
    let rows = {
        let m = metrics::read();
        state::rows(sessions, &state.task_manager, &m)
    };
    let tm = &mut state.task_manager;
    let sel = state::selected_index(&rows, tm.selected, tm.last_index);

    if tm.focus == Focus::Actions {
        let RowId::Session(id) = rows[sel] else {
            tm.focus = Focus::Content;
            return Vec::new();
        };
        let Some(s) = sessions.get(id) else {
            tm.focus = Focus::Content;
            return Vec::new();
        };
        let actions = actions_for(s);
        match action {
            Action::Up => tm.action_cursor = tm.action_cursor.saturating_sub(1),
            Action::Down => tm.action_cursor = (tm.action_cursor + 1).min(actions.len().saturating_sub(1)),
            Action::Left | Action::Back => tm.focus = Focus::Content,
            Action::Enter | Action::Right => {
                if let Some(chosen) = actions.get(tm.action_cursor).copied() {
                    tm.focus = Focus::Content;
                    return run_action(chosen, id, state, sessions);
                }
            }
            _ => {}
        }
        return Vec::new();
    }

    match action {
        Action::Up => {
            if sel == 0 {
                tm.focus = Focus::Tabs;
            } else {
                tm.selected = rows[sel - 1];
                tm.last_index = sel - 1;
            }
        }
        Action::Down => {
            if sel + 1 < rows.len() {
                tm.selected = rows[sel + 1];
                tm.last_index = sel + 1;
            }
        }
        Action::Enter | Action::Right => match rows[sel] {
            RowId::Session(_) => {
                tm.focus = Focus::Actions;
                tm.action_cursor = 0;
            }
            RowId::EndAll => return ask_end_all(state, sessions),
            RowId::Shell => {}
        },
        Action::Back => tm.focus = Focus::Tabs,
        _ => {}
    }
    Vec::new()
}

fn ask_end_all(state: &mut TuiosState, sessions: &Sessions) -> Vec<SessionEvent> {
    if page::end_all_targets(sessions, &state.active_dashboard).is_empty() {
        return toast("No background apps to end");
    }
    state.task_manager.confirm = Some(Confirm::EndAll);
    state.task_manager.confirm_cursor = 0;
    Vec::new()
}

fn confirm_key(action: Action, state: &mut TuiosState, sessions: &mut Sessions) -> Vec<SessionEvent> {
    let tm = &mut state.task_manager;
    match action {
        Action::Left | Action::Right | Action::Up | Action::Down | Action::Tab => {
            tm.confirm_cursor = 1 - tm.confirm_cursor.min(1);
        }
        Action::Back | Action::Quit => tm.confirm = None,
        Action::Enter => {
            let confirmed = tm.confirm_cursor == 1;
            tm.confirm = None;
            if confirmed {
                let targets = page::end_all_targets(sessions, &state.active_dashboard);
                logging::info(&format!("Task Manager: ending {} background apps", targets.len()));
                let mut events = Vec::new();
                for id in &targets {
                    events.extend(sessions.close(*id));
                }
                events.push(SessionEvent::Toast(format!(
                    "Ending {} app{}…",
                    targets.len(),
                    if targets.len() == 1 { "" } else { "s" }
                )));
                return events;
            }
        }
    }
    Vec::new()
}

fn run_action(action: TmAction, id: SessionId, state: &mut TuiosState, sessions: &mut Sessions) -> Vec<SessionEvent> {
    let Some(s) = sessions.get(id) else { return Vec::new() };
    let label = s.label.clone();
    let external = s.is_external();
    let builtin = s.is_builtin();
    match action {
        TmAction::OpenInContainer => {
            if external {
                let mut ev = sessions.move_to_container(id);
                ev.push(SessionEvent::Toast(format!("Moving {} into tuiOS…", label)));
                ev
            } else {
                vec![SessionEvent::Foreground(id)]
            }
        }
        TmAction::OpenInWindow => {
            let note = if store::can_open_new_window() {
                format!("Opening {} in a new window…", label)
            } else {
                format!("No window system — {} will take over this terminal", label)
            };
            let mut ev = sessions.move_to_window(id);
            ev.push(SessionEvent::Toast(note));
            ev
        }
        TmAction::Restart => {
            let mut ev = sessions.restart(id);
            if !builtin {
                ev.push(SessionEvent::Toast(format!("Restarting {}…", label)));
            }
            ev
        }
        TmAction::Suspend => {
            if sessions.suspend(id) {
                toast(format!("Suspended {}", label))
            } else {
                toast(format!("Could not suspend {}", label))
            }
        }
        TmAction::Resume => {
            sessions.resume(id);
            toast(format!("Resumed {}", label))
        }
        TmAction::Quit => {
            let mut ev = sessions.close(id);
            if !builtin {
                ev.push(SessionEvent::Toast(format!("Quitting {}…", label)));
            }
            ev
        }
        TmAction::ForceKill => {
            sessions.force_kill(id);
            toast(format!("Force-killed {}", label))
        }
        TmAction::LaunchAnother => match sessions.launch_another(id) {
            Some(new) if sessions.get(new).map_or(false, |s| s.is_external()) => {
                toast(format!("Launched another {} in a new window", label))
            }
            Some(new) => vec![SessionEvent::Foreground(new)],
            None => toast(format!("Could not start another {}", label)),
        },
        TmAction::ViewOutput => {
            state.task_manager.viewer = Some(id);
            Vec::new()
        }
        TmAction::Dismiss => sessions.dismiss(id),
    }
}

fn viewer_key(action: Action, state: &mut TuiosState, sessions: &mut Sessions, id: SessionId) {
    let Some(pty) = sessions.pty_mut(id) else {
        state.task_manager.viewer = None;
        return;
    };
    match action {
        Action::Up => pty.scroll_by(1),
        Action::Down => pty.scroll_by(-1),
        Action::Back => {
            pty.scroll_to_live();
            state.task_manager.viewer = None;
        }
        _ => {}
    }
}

// ── Performance / Network / Startup ─────────────────────────────────────────

fn perf_key(action: Action, state: &mut TuiosState) {
    let tm = &mut state.task_manager;
    let len = perf::PANELS.len();
    if tm.perf_zoom {
        match action {
            Action::Left => tm.perf_cursor = (tm.perf_cursor + len - 1) % len,
            Action::Right => tm.perf_cursor = (tm.perf_cursor + 1) % len,
            Action::Enter | Action::Back => tm.perf_zoom = false,
            _ => {}
        }
        return;
    }
    let cols = tm.perf_cols.max(1);
    let cur = tm.perf_cursor.min(len - 1);
    match action {
        Action::Left if cur % cols > 0 => tm.perf_cursor = cur - 1,
        Action::Right if cur % cols + 1 < cols && cur + 1 < len => tm.perf_cursor = cur + 1,
        Action::Up => {
            if cur >= cols {
                tm.perf_cursor = cur - cols;
            } else {
                tm.focus = Focus::Tabs;
            }
        }
        Action::Down if cur + cols < len => tm.perf_cursor = cur + cols,
        Action::Enter => tm.perf_zoom = true,
        Action::Back => tm.focus = Focus::Tabs,
        _ => {}
    }
}

fn net_key(action: Action, state: &mut TuiosState) {
    let tm = &mut state.task_manager;
    let len = net::row_count();
    match action {
        Action::Up => {
            if tm.net_cursor == 0 {
                tm.focus = Focus::Tabs;
            } else {
                tm.net_cursor -= 1;
            }
        }
        Action::Down => tm.net_cursor = (tm.net_cursor + 1).min(len.saturating_sub(1)),
        Action::Back => tm.focus = Focus::Tabs,
        _ => {}
    }
}

fn startup_key(action: Action, state: &mut TuiosState) {
    let tm = &mut state.task_manager;
    let len = tm.startup_rows.len();
    match action {
        Action::Up => {
            if tm.startup_cursor == 0 {
                tm.focus = Focus::Tabs;
            } else {
                tm.startup_cursor -= 1;
            }
        }
        Action::Down => tm.startup_cursor = (tm.startup_cursor + 1).min(len.saturating_sub(1)),
        Action::Enter => {
            let Some(row) = tm.startup_rows.get(tm.startup_cursor) else { return };
            let mut settings = persistence::load();
            let mut list = config::startup_apps(&settings);
            let on = if let Some(i) = list.iter().position(|a| *a == row.action) {
                list.remove(i);
                false
            } else {
                list.push(row.action.clone());
                true
            };
            config::set_startup_apps(&mut settings, &list);
            persistence::save(&settings);
            logging::settings(&format!(
                "{} {} launching at startup",
                row.label,
                if on { "now" } else { "no longer" }
            ));
        }
        Action::Back => tm.focus = Focus::Tabs,
        _ => {}
    }
}

// ── Letter shortcuts ─────────────────────────────────────────────────────────

/// Raw keys the Task Manager claims. None = not ours; let normal handling continue.
pub fn handle_raw(key: KeyEvent, state: &mut TuiosState, sessions: &mut Sessions) -> Option<Vec<SessionEvent>> {
    if state.task_manager.confirm.is_some() {
        return None;
    }

    if let Some(id) = state.task_manager.viewer {
        let pty = sessions.pty_mut(id)?;
        let page = (pty.size().1 / 2).max(1) as isize;
        match key.code {
            KeyCode::PageUp => pty.scroll_by(page),
            KeyCode::PageDown => pty.scroll_by(-page),
            KeyCode::Home => pty.scroll_by(isize::MAX / 2),
            KeyCode::End => pty.scroll_to_live(),
            _ => return None,
        }
        return Some(Vec::new());
    }

    let tm = &mut state.task_manager;
    let KeyCode::Char(c) = key.code else {
        if key.code == KeyCode::Delete && tm.tab == Tab::Apps && tm.focus != Focus::Tabs {
            if let RowId::Session(id) = tm.selected {
                return Some(run_action(TmAction::Quit, id, state, sessions));
            }
        }
        return None;
    };
    match (tm.tab, c) {
        (_, '[') | (_, ']') => {
            tm.tab = tm.tab.step(c == ']');
            if tm.focus == Focus::Actions {
                tm.focus = Focus::Content;
            }
            tm.perf_zoom = false;
        }
        (Tab::Apps, 'o' | 'O') => tm.sort = tm.sort.next(),
        (Tab::Apps, 'f' | 'F') => tm.filter = tm.filter.next(),
        (Tab::Apps, 'x' | 'X') => return Some(ask_end_all(state, sessions)),
        (Tab::Performance | Tab::Network, 'g' | 'G') => {
            let current = tm.line_graphs(config::load().graph_line);
            tm.graph_line = Some(!current);
        }
        (Tab::Performance | Tab::Network, '+' | '=') => tm.window_idx = (tm.window_idx + 1).min(WINDOWS.len() - 1),
        (Tab::Performance | Tab::Network, '-' | '_') => tm.window_idx = tm.window_idx.saturating_sub(1),
        (Tab::Network, 'r' | 'R') => {
            netinfo::request_public_ip();
            return Some(toast("Checking the public IP…"));
        }
        _ => return None,
    }
    Some(Vec::new())
}
