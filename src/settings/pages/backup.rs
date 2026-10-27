/// Backup & Restore settings — export, import (with a preview of what changes) and
/// Restore Defaults. Import's list of backups is the `SubPage::Backups` sub-page.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::settings::backup::{self, BackupInfo, BackupKind, ImportPlan};
use crate::settings::persistence;
use crate::settings::state::{SettingsState, SubPage};
use crate::tuios::models::Action;
use crate::utilities::onscreen_keyboard::{OnScreenKeyboard, OskResult};

const LABEL_MAX: usize = 32;
const MENU: [&str; 3] = ["Preview & Import", "Delete", "Cancel"];

#[derive(Debug, Clone)]
pub enum Dialog {
    /// Optional label for a new export.
    Label { text: String },
    /// What to do with the selected backup.
    Menu { index: usize, cursor: usize },
    ConfirmDelete { index: usize, cursor: usize },
    /// Everything the import would change. Buttons: Import, Cancel.
    Preview { plan: ImportPlan, scroll: usize, cursor: usize },
    /// "N apps will be added to the install queue". Buttons: queue, settings only, back.
    ConfirmQueue { plan: ImportPlan, cursor: usize },
}

#[derive(Debug, Clone)]
pub struct BackupUi {
    pub list: Vec<BackupInfo>,
    pub dialog: Option<Dialog>,
    pub osk: OnScreenKeyboard,
    /// A confirmed import (plan, also queue missing apps) for the main loop, which owns
    /// the App Store's install queue.
    pub pending: Option<(ImportPlan, bool)>,
}

impl BackupUi {
    pub fn new() -> Self {
        Self { list: Vec::new(), dialog: None, osk: OnScreenKeyboard::new(), pending: None }
    }

    pub fn refresh(&mut self) {
        self.list = backup::list();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Export,
    Import,
    RestoreDefaults,
}

const ROWS: &[Row] = &[Row::Export, Row::Import, Row::RestoreDefaults];

fn row_label(row: Row) -> &'static str {
    match row {
        Row::Export => "Export Settings",
        Row::Import => "Import Settings ›",
        Row::RestoreDefaults => "Restore tuiOS Settings to Default",
    }
}

fn row_style(selected: bool) -> Style {
    if selected {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    }
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

// ── Rendering ────────────────────────────────────────────────────────────────

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, ui: &BackupUi) {
    let mut lines: Vec<Line> = ROWS
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let prefix = if i == cursor { "  » " } else { "    " };
            Line::from(Span::styled(format!("{}{}", prefix, row_label(*row)), row_style(i == cursor)))
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("    Backups are saved in:", dim())));
    lines.push(Line::from(Span::styled(
        format!("    {}", backup::backups_dir().display()),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));
    for note in [
        "    An export holds your settings plus the list of installed apps and dashboards.",
        "    To move settings to another device, copy the file into its backups folder,",
        "    then use Import Settings there. Device-specific settings and startup apps",
        "    are never imported. Every import or restore saves an automatic backup first.",
    ] {
        lines.push(Line::from(Span::styled(note, dim())));
    }
    if let Some(row) = ROWS.get(cursor) {
        let hint = match row {
            Row::Export => "Save everything to a new backup file (you can add a label).",
            Row::Import => "Pick a backup, see exactly what changes, then import it.",
            Row::RestoreDefaults => "Reset every setting to its default (your current settings are backed up first).",
        };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("    {}", hint), dim())));
    }
    frame.render_widget(Paragraph::new(lines), area);

    if let Some(Dialog::Label { text }) = &ui.dialog {
        render_label(frame, area, text, &ui.osk);
    }
}

fn list_row(info: &BackupInfo) -> (String, String) {
    let tag = if info.kind == BackupKind::Auto { "Auto" } else { "Export" };
    (
        format!("{:<7}{}", tag, info.title()),
        format!("{} · {} · {}", info.device, info.os, info.created),
    )
}

pub fn render_list(frame: &mut Frame, area: Rect, cursor: usize, ui: &BackupUi) {
    let mut lines: Vec<Line> = Vec::new();
    if ui.list.is_empty() {
        lines.push(Line::from(Span::styled("    No backups yet.", Style::default().fg(Color::White))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "    Use Export Settings, or copy a backup file from another device into:",
            dim(),
        )));
        lines.push(Line::from(Span::styled(format!("    {}", backup::backups_dir().display()), dim())));
    } else {
        let visible = (area.height as usize).saturating_sub(2).max(1);
        let scroll = cursor.saturating_sub(visible - 1).min(ui.list.len().saturating_sub(1));
        let title_w = (area.width as usize).saturating_sub(8).min(40);
        for (i, info) in ui.list.iter().enumerate().skip(scroll).take(visible) {
            let selected = i == cursor;
            let prefix = if selected { "  » " } else { "    " };
            let (title, details) = list_row(info);
            let title: String = title.chars().take(title_w).collect();
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<w$}", prefix, title, w = title_w), row_style(selected)),
                Span::styled(format!("  {}", details), if selected { row_style(true) } else { dim() }),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("    Enter: preview & import, or delete · newest first", dim())));
    }
    frame.render_widget(Paragraph::new(lines), area);

    match &ui.dialog {
        Some(Dialog::Menu { index, cursor }) => {
            let title = ui.list.get(*index).map(|i| i.title()).unwrap_or_default();
            let lines: Vec<Line> = MENU
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let prefix = if i == *cursor { " » " } else { "   " };
                    Line::from(Span::styled(format!("{}{}", prefix, label), row_style(i == *cursor)))
                })
                .collect();
            popup(frame, area, &format!(" {} ", title), lines, 36, Color::Cyan);
        }
        Some(Dialog::ConfirmDelete { index, cursor }) => {
            let name = ui.list.get(*index).map(|i| i.file_name()).unwrap_or_default();
            let lines = vec![
                Line::from(Span::styled("Delete this backup? This cannot be undone.", Style::default().fg(Color::White))),
                Line::from(Span::styled(name, dim())),
                Line::from(""),
                buttons(&["Delete", "Cancel"], *cursor),
            ];
            popup(frame, area, " Delete Backup ", lines, 60, Color::Red);
        }
        Some(Dialog::Preview { plan, scroll, cursor }) => render_preview(frame, area, plan, *scroll, *cursor),
        Some(Dialog::ConfirmQueue { plan, cursor }) => {
            let n = plan.queue_count();
            let unknown = plan.missing.iter().filter(|m| m.location.is_some() && m.unknown_here()).count();
            let mut lines = vec![
                Line::from(Span::styled(
                    format!(
                        "Are you sure? {} app{} will be added to the install queue,",
                        n,
                        if n == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled("and the App Store will open to install them.", Style::default().fg(Color::White))),
            ];
            if unknown > 0 {
                lines.push(Line::from(Span::styled(
                    format!("{} of them will be added to this App Store first (experimental).", unknown),
                    Style::default().fg(Color::Yellow),
                )));
            }
            lines.push(Line::from(""));
            lines.push(buttons(&[&format!("Import & Queue {}", n), "Settings Only", "Back"], *cursor));
            popup(frame, area, " Reinstall Missing Apps? ", lines, 66, Color::Yellow);
        }
        _ => {}
    }
}

fn buttons(labels: &[&str], cursor: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let style = if i == cursor {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(format!("[ {} ]", label), style));
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

/// A centred box over `area`, sized to its content.
fn popup(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, width: u16, colour: Color) {
    let width = width.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
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
        .title(title.to_string())
        .style(Style::default().fg(colour).bg(Color::Black));
    let inner = block.inner(rect).inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_label(frame: &mut Frame, area: Rect, text: &str, osk: &OnScreenKeyboard) {
    let (top, osk_area) = if osk.visible {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(osk.height())])
            .split(area);
        (rows[0], Some(rows[1]))
    } else {
        (area, None)
    };
    let hint = if osk.visible {
        "Arrows pick a key · Enter on ↵ to export · q cancel"
    } else {
        "Type a label · Enter export · Esc cancel"
    };
    let lines = vec![
        Line::from(Span::styled("Label (optional):", Style::default().fg(Color::White))),
        Line::from(vec![
            Span::styled(text.to_string(), Style::default().fg(Color::Yellow)),
            Span::styled("▏", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(hint, dim())),
    ];
    popup(frame, top, " Export Settings ", lines, 60, Color::Cyan);
    if let Some(rect) = osk_area {
        frame.render_widget(Clear, rect);
        osk.render(frame, rect);
    }
}

fn preview_lines(plan: &ImportPlan) -> Vec<Line<'static>> {
    let white = Style::default().fg(Color::White);
    let head = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let info = &plan.info;
    let mut lines = vec![
        Line::from(Span::styled(format!("{} — from {} ({}), {}", info.title(), info.device, info.os, info.created), white)),
        Line::from(""),
    ];
    if plan.is_empty() {
        lines.push(Line::from(Span::styled("Nothing to import — this device already matches.", white)));
    }

    if !plan.changes.is_empty() {
        lines.push(Line::from(Span::styled(format!("Settings that will change ({})", plan.changes.len()), head)));
        for c in &plan.changes {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}: ", c.name), white),
                Span::styled(c.old.clone(), dim()),
                Span::styled(" → ", dim()),
                Span::styled(c.new.clone(), Style::default().fg(Color::Green)),
            ]));
            if let Some(app) = &c.needs {
                lines.push(Line::from(Span::styled(format!("    only if {} is reinstalled", app), dim())));
            }
        }
        lines.push(Line::from(""));
    }

    if !plan.missing.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Installed there but not here ({})", plan.missing.len()),
            head,
        )));
        for m in &plan.missing {
            let kind = if m.dashboard { "Dashboard" } else { "App" };
            match m.location {
                Some(loc) => {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", m.label), white),
                        Span::styled(format!("({}) → {}", kind, loc.label()), dim()),
                    ]));
                    if m.unknown_here() {
                        lines.push(Line::from(Span::styled(
                            format!("    new to this App Store: {}", m.repository),
                            Style::default().fg(Color::Yellow),
                        )));
                    }
                }
                None => {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", m.label), white),
                        Span::styled(format!("({})", kind), dim()),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "    can't be installed: not in either App Store list",
                        Style::default().fg(Color::Red),
                    )));
                }
            }
        }
        lines.push(Line::from(""));
    }

    let skipped = [
        ("Not imported — device-specific", &plan.device_specific),
        ("Not imported — the app they use can't be installed here", &plan.unavailable),
    ];
    for (title, names) in skipped {
        if names.is_empty() {
            continue;
        }
        lines.push(Line::from(Span::styled(format!("{} ({})", title, names.len()), dim().add_modifier(Modifier::BOLD))));
        for name in names {
            lines.push(Line::from(Span::styled(format!("  {}", name), dim())));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn render_preview(frame: &mut Frame, area: Rect, plan: &ImportPlan, scroll: usize, cursor: usize) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Import Preview ")
        .style(Style::default().fg(Color::Cyan).bg(Color::Black));
    let inner = block.inner(area).inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let lines = preview_lines(plan);
    let max_scroll = lines.len().saturating_sub(rows[0].height as usize);
    let shown: Vec<Line> = lines.into_iter().skip(scroll.min(max_scroll)).collect();
    frame.render_widget(Paragraph::new(shown), rows[0]);
    let hint = if max_scroll > 0 { "↑↓ scroll · ←→ choose · Enter confirm" } else { "←→ choose · Enter confirm" };
    frame.render_widget(Paragraph::new(Span::styled(hint, dim())), rows[1]);
    frame.render_widget(Paragraph::new(buttons(&["Import", "Cancel"], cursor)), rows[2]);
}

// ── Keys ─────────────────────────────────────────────────────────────────────

pub fn item_count() -> usize {
    ROWS.len()
}

pub fn list_count(ui: &BackupUi) -> usize {
    ui.list.len()
}

/// Enter on the Backup & Restore page. Some(message) = show a popup.
pub fn handle_enter(ss: &mut SettingsState) -> Option<String> {
    match ROWS.get(ss.right_cursor)? {
        Row::Export => {
            let settings = persistence::load();
            if persistence::get_bool(&settings, "utilities.onscreen_keyboard_enabled", false) {
                ss.backup.osk.show();
            } else {
                ss.backup.osk.hide();
            }
            ss.backup.dialog = Some(Dialog::Label { text: String::new() });
            None
        }
        Row::Import => {
            ss.backup.refresh();
            ss.subpage = Some(SubPage::Backups);
            ss.right_cursor = 0;
            None
        }
        Row::RestoreDefaults => Some(backup::restore_defaults().unwrap_or_else(|e| e)),
    }
}

/// Enter on a backup in the Import list.
pub fn handle_list_enter(ss: &mut SettingsState) {
    if ss.right_cursor < ss.backup.list.len() {
        ss.backup.dialog = Some(Dialog::Menu { index: ss.right_cursor, cursor: 0 });
    }
}

fn do_export(ss: &mut SettingsState, label: &str) -> Option<String> {
    ss.backup.dialog = None;
    ss.backup.osk.hide();
    let result = backup::export(label, BackupKind::Export);
    ss.backup.refresh();
    Some(match result {
        Ok(info) => format!("Settings exported to backups/{}", info.file_name()),
        Err(e) => e,
    })
}

fn step(cursor: &mut usize, len: usize, forward: bool) {
    *cursor = if forward { (*cursor + 1).min(len - 1) } else { cursor.saturating_sub(1) };
}

/// Keys while a dialog is open. Some(message) = show a popup.
pub fn handle_dialog(action: Action, ss: &mut SettingsState) -> Option<String> {
    let dialog = ss.backup.dialog.take()?;
    match dialog {
        Dialog::Label { mut text } => {
            if action == Action::Back {
                ss.backup.osk.hide();
                return None;
            }
            if !ss.backup.osk.visible {
                if action == Action::Enter {
                    return do_export(ss, &text);
                }
                ss.backup.dialog = Some(Dialog::Label { text });
                return None;
            }
            let Some(code) = crate::tuios::input_handler::action_to_key_name(action) else {
                ss.backup.dialog = Some(Dialog::Label { text });
                return None;
            };
            match ss.backup.osk.handle_key(code) {
                OskResult::Char(c) if !c.is_control() && text.chars().count() < LABEL_MAX => text.push(c),
                OskResult::Backspace => {
                    text.pop();
                }
                OskResult::Enter => return do_export(ss, &text),
                _ => {}
            }
            ss.backup.dialog = Some(Dialog::Label { text });
            None
        }
        Dialog::Menu { index, mut cursor } => {
            match action {
                Action::Up | Action::Left => step(&mut cursor, MENU.len(), false),
                Action::Down | Action::Right => step(&mut cursor, MENU.len(), true),
                Action::Enter => match cursor {
                    0 => {
                        let info = ss.backup.list.get(index)?.clone();
                        return match backup::plan(&info) {
                            Ok(plan) => {
                                ss.backup.dialog = Some(Dialog::Preview { plan, scroll: 0, cursor: 0 });
                                None
                            }
                            Err(e) => Some(e),
                        };
                    }
                    1 => {
                        ss.backup.dialog = Some(Dialog::ConfirmDelete { index, cursor: 1 });
                        return None;
                    }
                    _ => return None,
                },
                Action::Back => return None,
                _ => {}
            }
            ss.backup.dialog = Some(Dialog::Menu { index, cursor });
            None
        }
        Dialog::ConfirmDelete { index, mut cursor } => {
            match action {
                Action::Left | Action::Up => cursor = 0,
                Action::Right | Action::Down => cursor = 1,
                Action::Enter if cursor == 0 => {
                    let info = ss.backup.list.get(index)?.clone();
                    let result = backup::delete(&info);
                    ss.backup.refresh();
                    ss.right_cursor = ss.right_cursor.min(ss.backup.list.len().saturating_sub(1));
                    return Some(match result {
                        Ok(()) => format!("Deleted {}", info.file_name()),
                        Err(e) => e,
                    });
                }
                Action::Enter | Action::Back => return None,
                _ => {}
            }
            ss.backup.dialog = Some(Dialog::ConfirmDelete { index, cursor });
            None
        }
        Dialog::Preview { plan, mut scroll, mut cursor } => {
            match action {
                Action::Up => scroll = scroll.saturating_sub(1),
                // Clamped against the real height when drawn
                Action::Down => scroll = (scroll + 1).min(preview_lines(&plan).len()),
                Action::Left => cursor = 0,
                Action::Right => cursor = 1,
                Action::Enter if cursor == 0 => {
                    if plan.is_empty() {
                        return Some("Nothing to import — this device already matches that backup.".into());
                    }
                    if plan.queue_count() > 0 {
                        ss.backup.dialog = Some(Dialog::ConfirmQueue { plan, cursor: 0 });
                    } else {
                        ss.backup.pending = Some((plan, false));
                    }
                    return None;
                }
                Action::Enter | Action::Back => return None,
                _ => {}
            }
            ss.backup.dialog = Some(Dialog::Preview { plan, scroll, cursor });
            None
        }
        Dialog::ConfirmQueue { plan, mut cursor } => {
            match action {
                Action::Left | Action::Up => step(&mut cursor, 3, false),
                Action::Right | Action::Down => step(&mut cursor, 3, true),
                Action::Enter if cursor < 2 => {
                    ss.backup.pending = Some((plan, cursor == 0));
                    return None;
                }
                Action::Enter | Action::Back => {
                    ss.backup.dialog = Some(Dialog::Preview { plan, scroll: 0, cursor: 0 });
                    return None;
                }
                _ => {}
            }
            ss.backup.dialog = Some(Dialog::ConfirmQueue { plan, cursor });
            None
        }
    }
}

/// Raw keys while a dialog is open: Esc closes it, and typing a label without the
/// on-screen keyboard. None = not handled here; Some(popup) = handled.
pub fn handle_raw(key: KeyEvent, ss: &mut SettingsState) -> Option<Option<String>> {
    if key.code == KeyCode::Esc {
        return Some(handle_dialog(Action::Back, ss));
    }
    let typing = matches!(ss.backup.dialog, Some(Dialog::Label { .. })) && !ss.backup.osk.visible;
    if !typing {
        return None;
    }
    let Some(Dialog::Label { text }) = ss.backup.dialog.as_mut() else { return None };
    match key.code {
        KeyCode::Char(c) if !c.is_control() => {
            if text.chars().count() < LABEL_MAX {
                text.push(c);
            }
            Some(None)
        }
        KeyCode::Backspace => {
            text.pop();
            Some(None)
        }
        KeyCode::Enter => {
            let text = text.clone();
            Some(do_export(ss, &text))
        }
        _ => Some(None),
    }
}
