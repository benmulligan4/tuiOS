/// Notes — internal tuiOS app.
///
/// A full notes app: create, edit, rename, pin, search, sort, Markdown preview
/// with clickable checklists, undo/redo, trash with restore, and export to
/// .md/.txt. Notes are stored in config/notes.json (created on first note).
///
/// Every feature is reachable from the actions menu (→ / m) so the app works
/// with arrow keys, WASD, the numpad (8/2/4/6 move, 9 select, 7 back) and the
/// on-screen keyboard as well as with a physical keyboard.

pub mod editor;
pub mod markdown;
pub mod store;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::utilities::onscreen_keyboard::{OnScreenKeyboard, OskResult};
use editor::{visual_rows, EditorBuffer};
use store::{NoteStore, TRASH_RETENTION_DAYS};

const AUTOSAVE_DELAY: Duration = Duration::from_millis(1500);
const STATUS_TTL: Duration = Duration::from_secs(3);
const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesKeyResult {
    /// The key was consumed by the app.
    Handled,
    /// The app does not use this key; the shell may handle it.
    Ignored,
    /// The user asked to close the app.
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    List,
    Editor,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuItem {
    Open,
    OpenPreview,
    NewNote,
    Rename,
    TogglePin,
    TogglePreview,
    ToggleChecklist,
    Undo,
    Redo,
    ExportMd,
    ExportTxt,
    MoveToTrash,
    CycleSort,
    ShowTrash,
    ShowNotes,
    Restore,
    DeleteForever,
    EmptyTrash,
    BackToList,
    Help,
}

enum InputPurpose {
    NewNote,
    Rename(u64),
}

enum ConfirmPurpose {
    DeleteForever(u64),
    EmptyTrash,
}

enum Overlay {
    None,
    Menu { items: Vec<MenuItem>, cursor: usize },
    Input { purpose: InputPurpose, text: String },
    Confirm { purpose: ConfirmPurpose, yes: bool },
    Help,
}

/// Navigation intent shared by arrows, WASD and numpad keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
}

fn nav_of(key: &KeyEvent) -> Option<Nav> {
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return None;
    }
    Some(match key.code {
        KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('8') => Nav::Up,
        KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('2') => Nav::Down,
        KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('4') => Nav::Left,
        KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('6') => Nav::Right,
        KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('9') => Nav::Select,
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('7') => {
            Nav::Back
        }
        _ => return None,
    })
}

fn ctrl_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

pub struct NotesApp {
    store: NoteStore,
    pane: Pane,
    trash_view: bool,
    search: String,
    searching: bool,
    visible: Vec<u64>,
    list_cursor: usize,
    list_state: ListState,
    open_id: Option<u64>,
    buffer: EditorBuffer,
    dirty: bool,
    last_edit: Instant,
    /// Physical typing goes straight into the note.
    insert_mode: bool,
    prefer_preview: bool,
    osk_enabled: bool,
    pub osk: OnScreenKeyboard,
    editor_scroll: usize,
    wrap_width: usize,
    page_rows: usize,
    preview_line: usize,
    preview_scroll: usize,
    overlay: Overlay,
    status: Option<(String, Instant)>,
}

impl NotesApp {
    pub fn new() -> Self {
        Self::new_with_store(NoteStore::load())
    }

    fn new_with_store(store: NoteStore) -> Self {
        Self {
            store,
            pane: Pane::List,
            trash_view: false,
            search: String::new(),
            searching: false,
            visible: Vec::new(),
            list_cursor: 0,
            list_state: ListState::default(),
            open_id: None,
            buffer: EditorBuffer::from_text(""),
            dirty: false,
            last_edit: Instant::now(),
            insert_mode: false,
            prefer_preview: false,
            osk_enabled: false,
            osk: OnScreenKeyboard::new(),
            editor_scroll: 0,
            wrap_width: 80,
            page_rows: 10,
            preview_line: 0,
            preview_scroll: 0,
            overlay: Overlay::None,
            status: None,
        }
    }

    pub fn start(&mut self) {
        let settings = crate::settings::persistence::load();
        self.osk_enabled =
            crate::settings::persistence::get_bool(&settings, "utilities.onscreen_keyboard_enabled", false);
        self.refresh_visible(None);
        if let Some(err) = self.store.load_error.take() {
            self.set_status(err);
        }
        // Start on the first note rather than the "New note" row when notes exist
        if !self.visible.is_empty() {
            self.list_cursor = self.row_offset();
        }
    }

    pub fn stop(&mut self) {
        self.commit_and_save();
    }

    /// True while the app is consuming free text, so letters/Enter must not be
    /// treated as shell navigation shortcuts.
    pub fn captures_text(&self) -> bool {
        matches!(self.overlay, Overlay::Input { .. })
            || self.searching
            || self.osk.visible
            || (self.pane == Pane::Editor && self.insert_mode)
    }

    // ------------------------------------------------------------------
    // State helpers
    // ------------------------------------------------------------------

    fn row_offset(&self) -> usize {
        if self.trash_view { 0 } else { 1 }
    }

    fn row_count(&self) -> usize {
        self.visible.len() + self.row_offset()
    }

    fn selected_id(&self) -> Option<u64> {
        self.list_cursor
            .checked_sub(self.row_offset())
            .and_then(|i| self.visible.get(i).copied())
    }

    /// The note actions apply to: the open note, else the highlighted one.
    fn target_id(&self) -> Option<u64> {
        if self.pane == Pane::List { self.selected_id() } else { self.open_id }
    }

    fn refresh_visible(&mut self, keep: Option<u64>) {
        self.visible = self.store.visible(self.trash_view, &self.search);
        if let Some(pos) = keep.and_then(|id| self.visible.iter().position(|&v| v == id)) {
            self.list_cursor = pos + self.row_offset();
        }
        self.list_cursor = self.list_cursor.min(self.row_count().saturating_sub(1));
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    fn save_store(&mut self) {
        if let Err(e) = self.store.save() {
            self.set_status(format!("Could not save notes: {}", e));
        }
    }

    /// Push the editor buffer into the store; returns true if it changed.
    fn commit(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        match self.open_id {
            Some(id) => self.store.set_body(id, self.buffer.text()),
            None => false,
        }
    }

    fn commit_and_save(&mut self) {
        if self.commit() {
            self.save_store();
        }
    }

    fn autosave_tick(&mut self) {
        if self.dirty && self.last_edit.elapsed() >= AUTOSAVE_DELAY {
            self.commit_and_save();
        }
    }

    fn mark_edited(&mut self) {
        self.dirty = true;
        self.last_edit = Instant::now();
    }

    fn open_note(&mut self, id: u64, preview: bool) {
        self.commit_and_save();
        let Some(note) = self.store.get(id) else { return };
        self.buffer = EditorBuffer::from_text(&note.body);
        self.open_id = Some(id);
        self.dirty = false;
        self.pane = if preview { Pane::Preview } else { Pane::Editor };
        self.insert_mode = !preview && !self.osk_enabled;
        self.editor_scroll = 0;
        self.preview_line = 0;
        self.preview_scroll = 0;
    }

    fn close_editor(&mut self) {
        self.commit_and_save();
        self.osk.hide();
        self.insert_mode = false;
        self.pane = Pane::List;
        let id = self.open_id.take();
        self.refresh_visible(id);
    }

    fn set_pane(&mut self, pane: Pane) {
        if self.open_id.is_none() {
            return;
        }
        self.pane = pane;
        self.prefer_preview = pane == Pane::Preview;
        if pane == Pane::Preview {
            self.insert_mode = false;
            self.osk.hide();
            self.preview_line = self.buffer.row;
        } else {
            self.insert_mode = !self.osk_enabled;
        }
    }

    fn open_input(&mut self, purpose: InputPurpose, initial: String) {
        self.overlay = Overlay::Input { purpose, text: initial };
        if self.osk_enabled {
            self.osk.show();
        }
    }

    fn open_menu(&mut self) {
        let items = self.menu_items();
        self.overlay = Overlay::Menu { items, cursor: 0 };
    }

    fn menu_items(&self) -> Vec<MenuItem> {
        use MenuItem::*;
        let mut items = Vec::new();
        if self.pane != Pane::List {
            items.push(TogglePreview);
            if self.pane == Pane::Editor {
                items.push(ToggleChecklist);
            }
            items.extend([Undo, Redo, Rename, TogglePin, ExportMd, ExportTxt, MoveToTrash, BackToList]);
        } else if self.trash_view {
            if self.selected_id().is_some() {
                items.extend([Restore, DeleteForever]);
            }
            if self.store.count(true) > 0 {
                items.push(EmptyTrash);
            }
            items.push(ShowNotes);
        } else {
            if self.selected_id().is_some() {
                items.extend([Open, OpenPreview, Rename, TogglePin, ExportMd, ExportTxt, MoveToTrash]);
            }
            items.extend([NewNote, CycleSort, ShowTrash]);
        }
        items.push(Help);
        items
    }

    fn menu_label(&self, item: MenuItem) -> String {
        let pinned = self.target_id().and_then(|id| self.store.get(id)).is_some_and(|n| n.pinned);
        match item {
            MenuItem::Open => "Open".into(),
            MenuItem::OpenPreview => "Open in preview".into(),
            MenuItem::NewNote => "New note".into(),
            MenuItem::Rename => "Rename".into(),
            MenuItem::TogglePin => if pinned { "Unpin".into() } else { "Pin to top".into() },
            MenuItem::TogglePreview => {
                if self.pane == Pane::Preview { "Switch to editor".into() } else { "Markdown preview".into() }
            }
            MenuItem::ToggleChecklist => "Toggle checklist item".into(),
            MenuItem::Undo => "Undo".into(),
            MenuItem::Redo => "Redo".into(),
            MenuItem::ExportMd => "Export as Markdown (.md)".into(),
            MenuItem::ExportTxt => "Export as text (.txt)".into(),
            MenuItem::MoveToTrash => "Move to Trash".into(),
            MenuItem::CycleSort => format!("Sort: {}", self.store.sort.label()),
            MenuItem::ShowTrash => format!("View Trash ({})", self.store.count(true)),
            MenuItem::ShowNotes => "Back to Notes".into(),
            MenuItem::Restore => "Restore".into(),
            MenuItem::DeleteForever => "Delete forever".into(),
            MenuItem::EmptyTrash => "Empty Trash".into(),
            MenuItem::BackToList => "Close note".into(),
            MenuItem::Help => "Keyboard shortcuts".into(),
        }
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    fn run(&mut self, item: MenuItem) {
        self.overlay = Overlay::None;
        let target = self.target_id();
        match item {
            MenuItem::Open => {
                if let Some(id) = target { self.open_note(id, self.prefer_preview) }
            }
            MenuItem::OpenPreview => {
                if let Some(id) = target { self.open_note(id, true) }
            }
            MenuItem::NewNote => self.open_input(InputPurpose::NewNote, String::new()),
            MenuItem::Rename => {
                if let Some(note) = target.and_then(|id| self.store.get(id)) {
                    let (id, title) = (note.id, note.title.clone());
                    self.open_input(InputPurpose::Rename(id), title);
                }
            }
            MenuItem::TogglePin => {
                if let Some(pinned) = target.and_then(|id| self.store.toggle_pin(id)) {
                    self.save_store();
                    self.refresh_visible(target);
                    self.set_status(if pinned { "Pinned" } else { "Unpinned" });
                }
            }
            MenuItem::TogglePreview => {
                let pane = if self.pane == Pane::Preview { Pane::Editor } else { Pane::Preview };
                self.set_pane(pane);
            }
            MenuItem::ToggleChecklist => {
                self.buffer.toggle_checklist();
                self.mark_edited();
            }
            MenuItem::Undo => self.undo(),
            MenuItem::Redo => self.redo(),
            MenuItem::ExportMd => {
                if let Some(id) = target { self.export(id, "md") }
            }
            MenuItem::ExportTxt => {
                if let Some(id) = target { self.export(id, "txt") }
            }
            MenuItem::MoveToTrash => {
                if let Some(id) = target { self.move_to_trash(id) }
            }
            MenuItem::CycleSort => {
                self.store.sort = self.store.sort.next();
                self.save_store();
                self.refresh_visible(target);
                self.set_status(format!("Sorted by {}", self.store.sort.label()));
            }
            MenuItem::ShowTrash => self.set_trash_view(true),
            MenuItem::ShowNotes => self.set_trash_view(false),
            MenuItem::Restore => {
                if let Some(id) = target {
                    self.store.restore(id);
                    self.save_store();
                    self.refresh_visible(None);
                    self.set_status("Note restored");
                }
            }
            MenuItem::DeleteForever => {
                if let Some(id) = target {
                    self.overlay = Overlay::Confirm { purpose: ConfirmPurpose::DeleteForever(id), yes: false };
                }
            }
            MenuItem::EmptyTrash => {
                if self.store.count(true) > 0 {
                    self.overlay = Overlay::Confirm { purpose: ConfirmPurpose::EmptyTrash, yes: false };
                }
            }
            MenuItem::BackToList => self.close_editor(),
            MenuItem::Help => self.overlay = Overlay::Help,
        }
    }

    fn undo(&mut self) {
        if self.buffer.undo() {
            self.mark_edited();
        } else {
            self.set_status("Nothing to undo");
        }
    }

    fn redo(&mut self) {
        if self.buffer.redo() {
            self.mark_edited();
        } else {
            self.set_status("Nothing to redo");
        }
    }

    fn set_trash_view(&mut self, trash: bool) {
        self.trash_view = trash;
        self.search.clear();
        self.searching = false;
        self.list_cursor = 0;
        self.refresh_visible(None);
        if !trash && !self.visible.is_empty() {
            self.list_cursor = 1;
        }
    }

    fn move_to_trash(&mut self, id: u64) {
        if self.open_id == Some(id) {
            self.close_editor();
        }
        if self.store.trash(id) {
            self.save_store();
            self.refresh_visible(None);
            self.set_status("Moved to Trash");
        }
    }

    fn confirm(&mut self, purpose: ConfirmPurpose) {
        match purpose {
            ConfirmPurpose::DeleteForever(id) => {
                self.store.delete_forever(id);
                self.set_status("Note deleted forever");
            }
            ConfirmPurpose::EmptyTrash => {
                let n = self.store.empty_trash();
                self.set_status(format!("Deleted {} note{}", n, if n == 1 { "" } else { "s" }));
            }
        }
        self.save_store();
        self.refresh_visible(None);
    }

    fn submit_input(&mut self, purpose: InputPurpose, text: String) {
        self.osk.hide();
        match purpose {
            InputPurpose::NewNote => {
                let id = self.store.create(&text);
                self.save_store();
                self.trash_view = false;
                self.search.clear();
                self.searching = false;
                self.refresh_visible(Some(id));
                self.open_note(id, false);
                if self.osk_enabled {
                    self.osk.show();
                }
            }
            InputPurpose::Rename(id) => {
                if self.store.rename(id, &text) {
                    self.save_store();
                    self.refresh_visible(Some(id));
                    self.set_status("Renamed");
                }
            }
        }
    }

    fn export(&mut self, id: u64, ext: &str) {
        self.commit_and_save();
        let Some(note) = self.store.get(id).cloned() else { return };
        let dir = export_dir();
        let content = if ext == "md" {
            format!("# {}\n\n{}\n", note.title, note.body)
        } else {
            format!("{}\n{}\n\n{}\n", note.title, "=".repeat(note.title.chars().count()), note.body)
        };
        let result = std::fs::create_dir_all(&dir).and_then(|_| {
            let path = unique_path(&dir, &file_stem(&note.title), ext);
            std::fs::write(&path, content).map(|_| path)
        });
        match result {
            Ok(path) => self.set_status(format!("Exported to {}", path.display())),
            Err(e) => self.set_status(format!("Export failed: {}", e)),
        }
    }

    // ------------------------------------------------------------------
    // Key handling
    // ------------------------------------------------------------------

    /// Entry point for keys forwarded as abstract names by the shell
    /// (numpad / mapped navigation fallback).
    pub fn handle_key(&mut self, code: &str) -> NotesKeyResult {
        let key = match code {
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "Enter" => KeyCode::Enter,
            "q" => KeyCode::Char('q'),
            _ => return NotesKeyResult::Ignored,
        };
        self.handle_key_event(KeyEvent::new(key, KeyModifiers::NONE))
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> NotesKeyResult {
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::None => {}
            Overlay::Help => return NotesKeyResult::Handled,
            Overlay::Menu { items, cursor } => return self.handle_menu_key(key, items, cursor),
            Overlay::Confirm { purpose, yes } => return self.handle_confirm_key(key, purpose, yes),
            Overlay::Input { purpose, text } => return self.handle_input_key(key, purpose, text),
        }
        if self.osk.visible && self.pane == Pane::Editor {
            if let Some(res) = self.osk_feed(&key) {
                self.apply_osk_to_editor(res);
            }
            return NotesKeyResult::Handled;
        }
        if self.searching {
            return self.handle_search_key(key);
        }
        match self.pane {
            Pane::List => self.handle_list_key(key),
            Pane::Editor => self.handle_editor_key(key),
            Pane::Preview => self.handle_preview_key(key),
        }
    }

    /// Translate a raw key into an on-screen keyboard result. Navigation keys
    /// drive the OSK; other printable keys type directly.
    fn osk_feed(&mut self, key: &KeyEvent) -> Option<OskResult> {
        match key.code {
            KeyCode::Esc => {
                self.osk.hide();
                return Some(OskResult::Close);
            }
            KeyCode::Backspace => return Some(OskResult::Backspace),
            _ => {}
        }
        let code = match nav_of(key) {
            Some(Nav::Up) => "Up",
            Some(Nav::Down) => "Down",
            Some(Nav::Left) => "Left",
            Some(Nav::Right) => "Right",
            Some(Nav::Select) => "Enter",
            Some(Nav::Back) => "q",
            None => {
                return match key.code {
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Some(OskResult::Char(c)),
                    _ => None,
                };
            }
        };
        Some(self.osk.handle_key(code))
    }

    fn apply_osk_to_editor(&mut self, res: OskResult) {
        match res {
            OskResult::Char(c) => {
                self.buffer.insert_char(c);
                self.mark_edited();
            }
            OskResult::Backspace => {
                self.buffer.backspace();
                self.mark_edited();
            }
            OskResult::Enter => {
                self.buffer.newline();
                self.mark_edited();
            }
            OskResult::CursorLeft => self.buffer.move_left(),
            OskResult::CursorRight => self.buffer.move_right(),
            OskResult::Close | OskResult::None => {}
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent, items: Vec<MenuItem>, mut cursor: usize) -> NotesKeyResult {
        match nav_of(&key) {
            Some(Nav::Up) => cursor = cursor.checked_sub(1).unwrap_or(items.len() - 1),
            Some(Nav::Down) => cursor = (cursor + 1) % items.len(),
            Some(Nav::Select) | Some(Nav::Right) => {
                self.run(items[cursor]);
                return NotesKeyResult::Handled;
            }
            Some(Nav::Back) | Some(Nav::Left) => return NotesKeyResult::Handled,
            None => {}
        }
        self.overlay = Overlay::Menu { items, cursor };
        NotesKeyResult::Handled
    }

    fn handle_confirm_key(&mut self, key: KeyEvent, purpose: ConfirmPurpose, mut yes: bool) -> NotesKeyResult {
        match (key.code, nav_of(&key)) {
            (KeyCode::Char('y') | KeyCode::Char('Y'), _) => {
                self.confirm(purpose);
                return NotesKeyResult::Handled;
            }
            (KeyCode::Char('n') | KeyCode::Char('N'), _) | (_, Some(Nav::Back)) => {
                return NotesKeyResult::Handled;
            }
            (_, Some(Nav::Select)) => {
                if yes {
                    self.confirm(purpose);
                }
                return NotesKeyResult::Handled;
            }
            (_, Some(_)) => yes = !yes,
            _ => {}
        }
        self.overlay = Overlay::Confirm { purpose, yes };
        NotesKeyResult::Handled
    }

    fn handle_input_key(&mut self, key: KeyEvent, purpose: InputPurpose, mut text: String) -> NotesKeyResult {
        let res = if self.osk.visible {
            self.osk_feed(&key)
        } else {
            match key.code {
                KeyCode::Esc => Some(OskResult::Close),
                KeyCode::Enter => Some(OskResult::Enter),
                KeyCode::Backspace => Some(OskResult::Backspace),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Some(OskResult::Char(c)),
                _ => None,
            }
        };
        match res {
            Some(OskResult::Char(c)) => text.push(c),
            Some(OskResult::Backspace) => {
                text.pop();
            }
            Some(OskResult::Enter) => {
                self.submit_input(purpose, text);
                return NotesKeyResult::Handled;
            }
            Some(OskResult::Close) => {
                self.osk.hide();
                return NotesKeyResult::Handled;
            }
            _ => {}
        }
        self.overlay = Overlay::Input { purpose, text };
        NotesKeyResult::Handled
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> NotesKeyResult {
        match key.code {
            KeyCode::Esc => {
                self.search.clear();
                self.searching = false;
            }
            KeyCode::Enter | KeyCode::Down => {
                self.searching = false;
                if !self.visible.is_empty() {
                    self.list_cursor = self.row_offset();
                }
                return NotesKeyResult::Handled;
            }
            KeyCode::Backspace => {
                self.search.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => self.search.push(c),
            _ => return NotesKeyResult::Handled,
        }
        self.refresh_visible(None);
        self.list_cursor = if self.visible.is_empty() { 0 } else { self.row_offset() };
        NotesKeyResult::Handled
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> NotesKeyResult {
        let page = self.page_rows.max(1) / 2;
        match key.code {
            KeyCode::Char('n') if !self.trash_view => self.run(MenuItem::NewNote),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('p') if !self.trash_view => self.run(MenuItem::TogglePin),
            KeyCode::Char('r') if !self.trash_view => self.run(MenuItem::Rename),
            KeyCode::Char('o') if !self.trash_view => self.run(MenuItem::CycleSort),
            KeyCode::Char('E') if !self.trash_view => self.run(MenuItem::ExportMd),
            KeyCode::Char('t') => self.set_trash_view(!self.trash_view),
            KeyCode::Char('u') if self.trash_view => self.run(MenuItem::Restore),
            KeyCode::Char('x') | KeyCode::Delete => {
                self.run(if self.trash_view { MenuItem::DeleteForever } else { MenuItem::MoveToTrash })
            }
            KeyCode::Char('m') => self.open_menu(),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Home => self.list_cursor = 0,
            KeyCode::End => self.list_cursor = self.row_count().saturating_sub(1),
            KeyCode::PageUp => self.list_cursor = self.list_cursor.saturating_sub(page),
            KeyCode::PageDown => {
                self.list_cursor = (self.list_cursor + page).min(self.row_count().saturating_sub(1))
            }
            KeyCode::Esc if !self.search.is_empty() => {
                self.search.clear();
                let keep = self.selected_id();
                self.refresh_visible(keep);
            }
            _ => match nav_of(&key) {
                Some(Nav::Up) => self.list_cursor = self.list_cursor.saturating_sub(1),
                Some(Nav::Down) => {
                    self.list_cursor = (self.list_cursor + 1).min(self.row_count().saturating_sub(1))
                }
                Some(Nav::Select) => match self.selected_id() {
                    Some(_) if self.trash_view => self.open_menu(),
                    Some(id) => self.open_note(id, self.prefer_preview),
                    None if !self.trash_view => self.run(MenuItem::NewNote),
                    None => {}
                },
                Some(Nav::Right) => self.open_menu(),
                Some(Nav::Back) => {
                    if self.trash_view {
                        self.set_trash_view(false);
                    } else {
                        return NotesKeyResult::Close;
                    }
                }
                Some(Nav::Left) => {}
                None => return NotesKeyResult::Ignored,
            },
        }
        NotesKeyResult::Handled
    }

    /// Ctrl shortcuts available in both the editor and the preview.
    fn handle_note_ctrl(&mut self, c: char) {
        match c {
            's' => {
                self.commit_and_save();
                self.set_status("Saved");
            }
            'z' => self.undo(),
            'y' => self.redo(),
            'p' => self.run(MenuItem::TogglePreview),
            'k' => self.open_menu(),
            'l' if self.pane == Pane::Editor => self.run(MenuItem::ToggleChecklist),
            _ => {}
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) -> NotesKeyResult {
        if let Some(c) = ctrl_char(&key) {
            // Ctrl+Shift+Z is the common alternative redo binding
            let c = if c == 'z' && key.modifiers.contains(KeyModifiers::SHIFT) { 'y' } else { c };
            self.handle_note_ctrl(c);
            return NotesKeyResult::Handled;
        }
        let width = self.wrap_width;
        let page = self.page_rows.max(1);
        match key.code {
            KeyCode::Home => return self.moved(|b| b.home()),
            KeyCode::End => return self.moved(|b| b.end()),
            KeyCode::PageUp => return self.moved(|b| (0..page).for_each(|_| b.move_up(width))),
            KeyCode::PageDown => return self.moved(|b| (0..page).for_each(|_| b.move_down(width))),
            KeyCode::Up => return self.moved(|b| b.move_up(width)),
            KeyCode::Down => return self.moved(|b| b.move_down(width)),
            KeyCode::Left => return self.moved(|b| b.move_left()),
            KeyCode::Right => return self.moved(|b| b.move_right()),
            _ => {}
        }

        if self.insert_mode {
            match key.code {
                KeyCode::Esc => self.close_editor(),
                KeyCode::Enter => self.edited(|b| b.newline()),
                KeyCode::Backspace => self.edited(|b| b.backspace()),
                KeyCode::Delete => self.edited(|b| b.delete()),
                KeyCode::Tab => self.edited(|b| b.insert_str("    ")),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
                    self.edited(|b| b.insert_char(c))
                }
                _ => return NotesKeyResult::Ignored,
            }
            return NotesKeyResult::Handled;
        }

        // Navigation mode (on-screen keyboard users)
        match key.code {
            KeyCode::Char('i') => self.insert_mode = true,
            KeyCode::Char('m') => self.open_menu(),
            KeyCode::Char('v') | KeyCode::Char('p') => self.set_pane(Pane::Preview),
            KeyCode::Delete => self.edited(|b| b.delete()),
            _ => match nav_of(&key) {
                Some(Nav::Up) => return self.moved(|b| b.move_up(width)),
                Some(Nav::Down) => return self.moved(|b| b.move_down(width)),
                Some(Nav::Left) => return self.moved(|b| b.move_left()),
                Some(Nav::Right) => return self.moved(|b| b.move_right()),
                Some(Nav::Select) => {
                    if self.osk_enabled { self.osk.show() } else { self.insert_mode = true }
                }
                Some(Nav::Back) => self.close_editor(),
                None => return NotesKeyResult::Ignored,
            },
        }
        NotesKeyResult::Handled
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> NotesKeyResult {
        if let Some(c) = ctrl_char(&key) {
            self.handle_note_ctrl(c);
            return NotesKeyResult::Handled;
        }
        let last = self.buffer.lines.len().saturating_sub(1);
        let page = self.page_rows.max(1);
        match key.code {
            KeyCode::Home => self.preview_line = 0,
            KeyCode::End => self.preview_line = last,
            KeyCode::PageUp => self.preview_line = self.preview_line.saturating_sub(page),
            KeyCode::PageDown => self.preview_line = (self.preview_line + page).min(last),
            KeyCode::Char('i') | KeyCode::Char('v') | KeyCode::Char('p') => {
                self.buffer.set_cursor(self.preview_line, 0);
                self.set_pane(Pane::Editor);
            }
            KeyCode::Char('m') => self.open_menu(),
            KeyCode::Char(' ') => {
                self.toggle_preview_line();
            }
            _ => match nav_of(&key) {
                Some(Nav::Up) => self.preview_line = self.preview_line.saturating_sub(1),
                Some(Nav::Down) => self.preview_line = (self.preview_line + 1).min(last),
                Some(Nav::Select) => {
                    if !self.toggle_preview_line() {
                        // Not a checklist item: jump into the editor at this line
                        self.buffer.set_cursor(self.preview_line, usize::MAX);
                        self.set_pane(Pane::Editor);
                    }
                }
                Some(Nav::Right) => self.open_menu(),
                Some(Nav::Back) => self.close_editor(),
                Some(Nav::Left) => {}
                None => return NotesKeyResult::Ignored,
            },
        }
        NotesKeyResult::Handled
    }

    fn toggle_preview_line(&mut self) -> bool {
        let toggled = self.buffer.toggle_checkbox_at(self.preview_line);
        if toggled {
            self.mark_edited();
        }
        toggled
    }

    fn moved(&mut self, f: impl FnOnce(&mut EditorBuffer)) -> NotesKeyResult {
        f(&mut self.buffer);
        NotesKeyResult::Handled
    }

    fn edited(&mut self, f: impl FnOnce(&mut EditorBuffer)) {
        f(&mut self.buffer);
        self.mark_edited();
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    pub fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        self.autosave_tick();
        if self.status.as_ref().is_some_and(|(_, t)| t.elapsed() >= STATUS_TTL) {
            self.status = None;
        }

        let (main, osk_area) = if self.osk.visible {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Fill(1), Constraint::Length(self.osk.height())])
                .split(area);
            (rows[0], Some(rows[1]))
        } else {
            (area, None)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Notes ")
            .title_alignment(Alignment::Right)
            .style(border_style);
        frame.render_widget(block, main);
        let inner = main.inner(Margin { horizontal: 1, vertical: 1 });
        if inner.width < 10 || inner.height < 4 {
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(inner);

        // Narrow terminals show one pane at a time
        if rows[0].width < 60 {
            if self.pane == Pane::List {
                self.render_list(frame, rows[0]);
            } else {
                self.render_note(frame, rows[0]);
            }
        } else {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(34), Constraint::Fill(1)])
                .split(rows[0]);
            self.render_list(frame, cols[0]);
            self.render_note(frame, cols[1]);
        }
        self.render_footer(frame, rows[1]);
        self.render_overlay(frame, main);

        if let Some(osk_area) = osk_area {
            self.osk.render(frame, osk_area);
        }
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.pane == Pane::List && !self.searching;
        let title = if self.trash_view {
            format!(" Trash ({}) ", self.store.count(true))
        } else {
            format!(" Notes ({}) ", self.store.count(false))
        };
        let bottom = if self.trash_view {
            format!(" Auto-deletes after {} days ", TRASH_RETENTION_DAYS)
        } else {
            format!(" ↕ {} ", self.store.sort.label())
        };
        let block = pane_block(title, focused).title_bottom(Line::from(bottom).right_aligned());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height < 2 {
            return;
        }

        let search_line = if self.searching {
            Line::from(vec![
                Span::styled(" / ", Style::default().fg(Color::Yellow)),
                Span::styled(self.search.clone(), Style::default().fg(Color::White)),
                Span::styled("▏", Style::default().fg(Color::Yellow)),
            ])
        } else if !self.search.is_empty() {
            Line::from(vec![
                Span::styled(" / ", Style::default().fg(Color::Yellow)),
                Span::styled(self.search.clone(), Style::default().fg(Color::White)),
                Span::styled(format!("  {} found", self.visible.len()), Style::default().fg(DIM)),
            ])
        } else {
            Line::from(Span::styled(" / Search notes", Style::default().fg(DIM)))
        };
        frame.render_widget(Paragraph::new(search_line), Rect { height: 1, ..inner });
        let list_area = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
        self.page_rows = list_area.height as usize;

        let width = list_area.width.saturating_sub(2) as usize;
        let mut items: Vec<ListItem> = Vec::new();
        if !self.trash_view {
            items.push(ListItem::new(Line::from(Span::styled(
                "+ New note",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ))));
        }
        for &id in &self.visible {
            let Some(note) = self.store.get(id) else { continue };
            let mut title_spans = Vec::new();
            if note.pinned {
                title_spans.push(Span::styled("★ ", Style::default().fg(Color::Yellow)));
            }
            title_spans.push(Span::styled(
                truncate(&note.title, width.saturating_sub(2)),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ));

            let mut meta = if let Some(deleted) = note.deleted_at {
                let left = TRASH_RETENTION_DAYS - (store::now() - deleted) / 86_400;
                format!("Deleted {} · {}d left", format_time(deleted), left.max(0))
            } else {
                format_time(note.updated_at)
            };
            let (done, total) = markdown::checklist_progress(&note.body);
            if total > 0 {
                meta.push_str(&format!(" · ☑ {}/{}", done, total));
            }
            let snippet = note
                .body
                .lines()
                .map(|l| l.trim().trim_start_matches('#').trim())
                .find(|l| !l.is_empty() && !l.eq_ignore_ascii_case(&note.title))
                .unwrap_or("");
            if !snippet.is_empty() && note.deleted_at.is_none() {
                meta.push_str(" · ");
                meta.push_str(snippet);
            }
            items.push(ListItem::new(vec![
                Line::from(title_spans),
                Line::from(Span::styled(truncate(&meta, width), Style::default().fg(DIM))),
            ]));
        }

        if self.visible.is_empty() {
            let msg = if !self.search.is_empty() {
                "No matching notes"
            } else if self.trash_view {
                "Trash is empty"
            } else {
                "No notes yet"
            };
            items.push(ListItem::new(Line::from(Span::styled(msg, Style::default().fg(DIM)))));
        }

        let highlight = if focused {
            Style::default().bg(Color::Rgb(40, 60, 80))
        } else {
            Style::default().bg(Color::Rgb(40, 40, 40))
        };
        let list = List::new(items).highlight_style(highlight).highlight_symbol("▌");
        if self.row_count() > 0 {
            self.list_state.select(Some(self.list_cursor));
        } else {
            self.list_state.select(None);
        }
        frame.render_stateful_widget(list, list_area, &mut self.list_state);
    }

    fn render_note(&mut self, frame: &mut Frame, area: Rect) {
        let note_id = if self.pane == Pane::List { self.selected_id() } else { self.open_id };
        let Some(note) = note_id.and_then(|id| self.store.get(id)).cloned() else {
            let block = pane_block(" Welcome ".into(), false);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            frame.render_widget(Paragraph::new(welcome_text()).wrap(Wrap { trim: false }), inner);
            return;
        };

        let mode = match self.pane {
            Pane::List if note.is_trashed() => " Trash · read-only ",
            Pane::List => " Enter to open ",
            Pane::Preview => " Preview ",
            Pane::Editor if self.insert_mode || self.osk.visible => " Editing ",
            Pane::Editor => " Editor · Enter to type ",
        };
        let title = format!(" {}{} ", if note.pinned { "★ " } else { "" }, note.title);
        let dates = format!(
            " Edited {} · Created {} ",
            format_time(note.updated_at),
            format_time(note.created_at)
        );
        let block = pane_block(title, self.pane != Pane::List)
            .title(Line::from(mode).right_aligned())
            .title_bottom(Line::from(dates).right_aligned());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        match self.pane {
            Pane::Editor => self.render_editor(frame, inner),
            Pane::Preview => {
                self.page_rows = inner.height as usize;
                let lines = self.buffer.lines.clone();
                self.preview_line = self.preview_line.min(lines.len().saturating_sub(1));
                self.preview_scroll =
                    render_markdown(frame, inner, &lines, Some(self.preview_line), self.preview_scroll);
            }
            Pane::List => {
                let lines: Vec<String> = note.body.split('\n').map(str::to_string).collect();
                render_markdown(frame, inner, &lines, None, 0);
            }
        }
    }

    fn render_editor(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width as usize;
        let height = area.height as usize;
        self.wrap_width = width.max(1);
        self.page_rows = height;

        // Visual rows: (line index, first char of the row)
        let mut rows: Vec<(usize, usize)> = Vec::new();
        let mut cursor_vrow = 0;
        for (i, line) in self.buffer.lines.iter().enumerate() {
            let len = line.chars().count();
            let mut n = visual_rows(len, width);
            if i == self.buffer.row {
                n = n.max(self.buffer.col / width + 1);
                cursor_vrow = rows.len() + self.buffer.col / width;
            }
            rows.extend((0..n).map(|r| (i, r * width)));
        }

        if cursor_vrow < self.editor_scroll {
            self.editor_scroll = cursor_vrow;
        } else if cursor_vrow >= self.editor_scroll + height {
            self.editor_scroll = cursor_vrow + 1 - height;
        }

        let show_cursor = self.overlay_is_none();
        let cursor_style = if self.insert_mode || self.osk.visible {
            Style::default().fg(Color::Black).bg(Color::White)
        } else {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        };

        let mut in_code = false;
        let mut line_styles = Vec::with_capacity(self.buffer.lines.len());
        for line in &self.buffer.lines {
            line_styles.push(editor_line_style(line, &mut in_code));
        }

        let mut out: Vec<Line> = Vec::new();
        for (vrow, &(li, start)) in rows.iter().enumerate().skip(self.editor_scroll).take(height) {
            let chars: Vec<char> = self.buffer.lines[li].chars().skip(start).take(width).collect();
            let style = line_styles[li];
            if show_cursor && vrow == cursor_vrow {
                let at = self.buffer.col - start;
                let before: String = chars.iter().take(at).collect();
                let cur: String = chars.get(at).map(|c| c.to_string()).unwrap_or_else(|| " ".into());
                let after: String = chars.iter().skip(at + 1).collect();
                out.push(Line::from(vec![
                    Span::styled(before, style),
                    Span::styled(cur, cursor_style),
                    Span::styled(after, style),
                ]));
            } else {
                out.push(Line::from(Span::styled(chars.into_iter().collect::<String>(), style)));
            }
        }

        if self.buffer.is_empty() && out.len() == 1 {
            out[0].spans.push(Span::styled(
                " Start typing… (# heading, - list, ^L checklist)",
                Style::default().fg(DIM),
            ));
        }
        while out.len() < height {
            out.push(Line::from(Span::styled("~", Style::default().fg(Color::Rgb(60, 60, 60)))));
        }
        frame.render_widget(Paragraph::new(Text::from(out)), area);
    }

    fn overlay_is_none(&self) -> bool {
        matches!(self.overlay, Overlay::None)
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let hint = match (&self.overlay, self.pane) {
            _ if self.searching => "Type to filter · Enter done · Esc clear",
            (Overlay::Menu { .. }, _) => "↑↓ choose · Enter select · Esc close",
            (Overlay::Input { .. }, _) => "Type a title · Enter confirm · Esc cancel",
            (Overlay::Confirm { .. }, _) => "←→ choose · Enter confirm · y/n",
            (Overlay::Help, _) => "Any key to close",
            _ if self.osk.visible => "Arrows pick a key · Enter press · Q/Esc hide keyboard",
            (_, Pane::List) if self.trash_view => "Enter/→ actions · u restore · x delete forever · t notes · ? help",
            (_, Pane::List) => "Enter open · n new · / search · p pin · r rename · x trash · o sort · → menu · ? help",
            (_, Pane::Editor) if self.insert_mode => "Esc close · ^S save · ^Z/^Y undo/redo · ^L checklist · ^P preview · ^K menu",
            (_, Pane::Editor) => "Enter keyboard · i type · v preview · m menu · Q back",
            (_, Pane::Preview) => "↑↓ line · Enter tick/edit · i edit · → menu · Q back",
        };

        let mut right: Vec<Span> = Vec::new();
        if self.open_id.is_some() && self.pane != Pane::List {
            right.push(Span::styled(
                format!("  {} words · {} chars  ", self.buffer.word_count(), self.buffer.char_count()),
                Style::default().fg(DIM),
            ));
            right.push(if self.dirty {
                Span::styled("● Unsaved ", Style::default().fg(Color::Yellow))
            } else {
                Span::styled("✓ Saved ", Style::default().fg(Color::Green))
            });
        }
        let right_width: u16 = right.iter().map(|s| s.content.chars().count() as u16).sum();

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(right_width)])
            .split(area);
        let left = match &self.status {
            Some((msg, _)) => Span::styled(format!(" {}", msg), Style::default().fg(Color::Yellow)),
            None => Span::styled(format!(" {}", hint), Style::default().fg(DIM)),
        };
        frame.render_widget(Paragraph::new(Line::from(left)), cols[0]);
        frame.render_widget(Paragraph::new(Line::from(right)), cols[1]);
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        match &self.overlay {
            Overlay::None => {}
            Overlay::Menu { items, cursor } => {
                let labels: Vec<String> = items.iter().map(|&i| self.menu_label(i)).collect();
                let w = labels.iter().map(|l| l.chars().count()).max().unwrap_or(10) as u16 + 6;
                let rect = centered(area, w, items.len() as u16 + 2);
                frame.render_widget(Clear, rect);
                let list_items: Vec<ListItem> = labels
                    .into_iter()
                    .zip(items)
                    .map(|(label, item)| {
                        let style = match item {
                            MenuItem::MoveToTrash | MenuItem::DeleteForever | MenuItem::EmptyTrash => {
                                Style::default().fg(Color::LightRed)
                            }
                            _ => Style::default().fg(Color::White),
                        };
                        ListItem::new(Span::styled(label, style))
                    })
                    .collect();
                let list = List::new(list_items)
                    .block(dialog_block(" Actions "))
                    .highlight_style(Style::default().bg(Color::Rgb(40, 60, 80)).add_modifier(Modifier::BOLD))
                    .highlight_symbol("▶ ");
                let mut state = ListState::default().with_selected(Some(*cursor));
                frame.render_stateful_widget(list, rect, &mut state);
            }
            Overlay::Input { purpose, text } => {
                let title = match purpose {
                    InputPurpose::NewNote => " New note — title ",
                    InputPurpose::Rename(_) => " Rename note ",
                };
                let rect = centered(area, 52, 3);
                frame.render_widget(Clear, rect);
                let inner_w = rect.width.saturating_sub(4) as usize;
                let shown: String = {
                    let n = text.chars().count();
                    text.chars().skip(n.saturating_sub(inner_w)).collect()
                };
                let line = Line::from(vec![
                    Span::styled(shown, Style::default().fg(Color::White)),
                    Span::styled("▏", Style::default().fg(Color::Yellow)),
                    if text.is_empty() {
                        Span::styled("Untitled", Style::default().fg(DIM))
                    } else {
                        Span::raw("")
                    },
                ]);
                frame.render_widget(Paragraph::new(line).block(dialog_block(title)), rect);
            }
            Overlay::Confirm { purpose, yes } => {
                let msg = match purpose {
                    ConfirmPurpose::DeleteForever(id) => format!(
                        "Permanently delete \"{}\"?",
                        self.store.get(*id).map(|n| n.title.as_str()).unwrap_or("note")
                    ),
                    ConfirmPurpose::EmptyTrash => {
                        format!("Permanently delete all {} notes in Trash?", self.store.count(true))
                    }
                };
                let w = (msg.chars().count() as u16 + 6).max(36);
                let rect = centered(area, w, 6);
                frame.render_widget(Clear, rect);
                let button = |label: &str, active: bool, color: Color| {
                    if active {
                        Span::styled(format!(" {} ", label), Style::default().fg(Color::Black).bg(color))
                    } else {
                        Span::styled(format!(" {} ", label), Style::default().fg(color))
                    }
                };
                let text = vec![
                    Line::from(msg),
                    Line::from(Span::styled("This cannot be undone.", Style::default().fg(DIM))),
                    Line::from(""),
                    Line::from(vec![
                        button("Delete", *yes, Color::LightRed),
                        Span::raw("   "),
                        button("Cancel", !*yes, Color::White),
                    ])
                    .centered(),
                ];
                frame.render_widget(
                    Paragraph::new(text).alignment(Alignment::Center).block(dialog_block(" Confirm ")),
                    rect,
                );
            }
            Overlay::Help => {
                let text = help_text();
                let rect = centered(area, 64, text.len() as u16 + 2);
                frame.render_widget(Clear, rect);
                frame.render_widget(Paragraph::new(text).block(dialog_block(" Notes — shortcuts ")), rect);
            }
        }
    }
}

// The shell may replace or drop the app without calling stop() (navbar
// switch, quitting tuiOS), so flush pending edits here too.
impl Drop for NotesApp {
    fn drop(&mut self) {
        self.commit_and_save();
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Render Markdown with optional source-line highlight; returns the new scroll
/// offset (in visual rows) that keeps the highlighted line visible.
fn render_markdown(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    highlight: Option<usize>,
    scroll: usize,
) -> usize {
    let width = area.width as usize;
    let height = area.height as usize;
    if lines.len() == 1 && lines[0].trim().is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("Empty note", Style::default().fg(DIM))),
            area,
        );
        return 0;
    }

    let mut rows: Vec<(usize, Line<'static>)> = Vec::new();
    for (i, line) in markdown::render_document(lines, width).iter().enumerate() {
        rows.extend(markdown::wrap_line(line, width).into_iter().map(|l| (i, l)));
    }

    let mut scroll = scroll;
    if let Some(sel) = highlight {
        let first = rows.iter().position(|(i, _)| *i == sel).unwrap_or(0);
        let last = rows.iter().rposition(|(i, _)| *i == sel).unwrap_or(first);
        if first < scroll {
            scroll = first;
        } else if last >= scroll + height {
            scroll = (last + 1).saturating_sub(height).min(first);
        }
    }
    scroll = scroll.min(rows.len().saturating_sub(1));

    let out: Vec<Line> = rows
        .into_iter()
        .skip(scroll)
        .take(height)
        .map(|(i, line)| {
            if Some(i) == highlight {
                line.style(Style::default().bg(Color::Rgb(40, 60, 80)))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(out)), area);
    scroll
}

fn editor_line_style(line: &str, in_code: &mut bool) -> Style {
    if markdown::is_code_fence(line) {
        *in_code = !*in_code;
        return Style::default().fg(DIM);
    }
    if *in_code {
        return Style::default().fg(Color::Green);
    }
    let t = line.trim_start();
    if t.starts_with('#') {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else if t.starts_with('>') {
        Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC)
    } else if markdown::checkbox_state(line) == Some(true) {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(Color::White)
    }
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { ACCENT } else { DIM }))
        .title(Span::styled(
            title,
            Style::default().fg(if focused { ACCENT } else { Color::Gray }).add_modifier(Modifier::BOLD),
        ))
}

fn dialog_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(title, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(Color::Black))
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn format_time(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(dt) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let today = Local::now().date_naive();
    let date = dt.date_naive();
    if date == today {
        dt.format("Today %H:%M").to_string()
    } else if today.pred_opt() == Some(date) {
        dt.format("Yesterday %H:%M").to_string()
    } else if dt.format("%Y").to_string() == Local::now().format("%Y").to_string() {
        dt.format("%-d %b").to_string()
    } else {
        dt.format("%-d %b %Y").to_string()
    }
}

fn welcome_text() -> Text<'static> {
    let key = |k: &'static str| Span::styled(k, Style::default().fg(Color::Yellow));
    let dim = |t: &'static str| Span::styled(t, Style::default().fg(Color::Gray));
    Text::from(vec![
        Line::from(Span::styled("Notes", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![key("Enter"), dim(" on “+ New note” or "), key("n"), dim(" to create a note")]),
        Line::from(vec![key("→"), dim(" or "), key("m"), dim(" opens the actions menu")]),
        Line::from(vec![key("?"), dim(" shows every keyboard shortcut")]),
        Line::from(""),
        Line::from(dim("Notes support Markdown: # headings, **bold**, *italic*,")),
        Line::from(dim("`code`, - lists, 1. numbered lists and - [ ] checklists.")),
    ])
}

fn help_text() -> Vec<Line<'static>> {
    let section = |t: &'static str| {
        Line::from(Span::styled(t, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
    };
    let row = |k: &'static str, d: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {:<16}", k), Style::default().fg(Color::Yellow)),
            Span::styled(d, Style::default().fg(Color::White)),
        ])
    };
    vec![
        section("Note list"),
        row("↑↓ / W S / 8 2", "Move"),
        row("Enter / 9", "Open note"),
        row("→ / m", "Actions menu"),
        row("n", "New note"),
        row("/", "Search titles and text"),
        row("p · r · o", "Pin · Rename · Change sort"),
        row("x / Del", "Move to Trash (delete forever in Trash)"),
        row("E", "Export as Markdown"),
        row("t · u", "Toggle Trash view · Restore"),
        row("Q / Esc / 7", "Close Notes"),
        section("Editor"),
        row("Esc", "Save and return to the list"),
        row("Ctrl+S", "Save now (notes also autosave)"),
        row("Ctrl+Z / Ctrl+Y", "Undo / Redo"),
        row("Ctrl+L", "Toggle checklist item"),
        row("Ctrl+P", "Markdown preview"),
        row("Ctrl+K", "Actions menu"),
        section("Preview"),
        row("Enter / Space", "Tick checklist item (or edit line)"),
        row("i", "Back to the editor"),
    ]
}

fn export_dir() -> PathBuf {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if home.join("Documents").is_dir() => home.join("Documents").join("tuiOS Notes"),
        Some(home) => home.join("tuiOS Notes"),
        None => crate::tuios::registry::config_dir().join("exports"),
    }
}

fn file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() { "note".to_string() } else { truncate(&cleaned, 80) }
}

fn unique_path(dir: &std::path::Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{}.{}", stem, ext));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{} ({}).{}", stem, n, ext));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn app_with_temp_store(name: &str) -> (NotesApp, PathBuf) {
        let dir = std::env::temp_dir().join(format!("tuios-notes-app-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("notes.json");
        let mut app = NotesApp::new_with_store(NoteStore::load_from(path.clone()));
        app.osk_enabled = false;
        app.refresh_visible(None);
        (app, path)
    }

    fn press(app: &mut NotesApp, code: KeyCode) -> NotesKeyResult {
        app.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(app: &mut NotesApp, c: char) {
        app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    fn type_str(app: &mut NotesApp, s: &str) {
        for c in s.chars() {
            press(app, if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) });
        }
    }

    fn screen(app: &mut NotesApp) -> String {
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|f| app.render(f, f.area(), Style::default())).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn create_edit_checklist_trash_and_delete_via_keys() {
        let (mut app, path) = app_with_temp_store("flow");
        assert!(!path.exists());

        // Enter on "+ New note" opens the title dialog
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Groceries");
        press(&mut app, KeyCode::Enter);
        assert!(path.exists(), "notes.json is created with the first note");
        assert_eq!(app.pane, Pane::Editor);

        // q/w/s/e are plain text while typing
        type_str(&mut app, "quest\nmilk");
        ctrl(&mut app, 'l');
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "eggs");
        assert_eq!(app.buffer.text(), "quest\n- [ ] milk\n- [ ] eggs");
        ctrl(&mut app, 'z');
        assert_eq!(app.buffer.text(), "quest\n- [ ] milk\n- [ ] ");

        // Preview: tick the first checklist item
        ctrl(&mut app, 'p');
        assert_eq!(app.pane, Pane::Preview);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        let shown = screen(&mut app);
        assert!(shown.contains("☑ milk"), "{}", shown);

        // Esc saves and returns to the list
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.pane, Pane::List);
        let id = app.selected_id().unwrap();
        assert_eq!(NoteStore::load_from(path.clone()).get(id).unwrap().body, "quest\n- [x] milk\n- [ ] ");
        let shown = screen(&mut app);
        assert!(shown.contains("Groceries") && shown.contains("☑ 1/2"), "{}", shown);

        // Search filters the list
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "zzz");
        assert!(screen(&mut app).contains("No matching notes"));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible.len(), 1);

        // Move to trash, then delete forever through the menu + confirm dialog
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.store.count(true), 1);
        press(&mut app, KeyCode::Char('t'));
        assert!(app.trash_view);
        press(&mut app, KeyCode::Enter); // opens the trash actions menu
        let shown = screen(&mut app);
        assert!(shown.contains("Restore") && shown.contains("Delete forever"), "{}", shown);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter); // Delete forever -> confirm
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.store.notes.len(), 0);
        assert!(!path.exists(), "notes.json is removed when no notes remain");

        // Back leaves trash, a second Back closes the app
        assert_eq!(press(&mut app, KeyCode::Char('q')), NotesKeyResult::Handled);
        assert_eq!(press(&mut app, KeyCode::Char('q')), NotesKeyResult::Close);
    }
}
