/// Task Manager bar — every running app as a coloured tile beside the navbar, and the
/// system stats line that sits under (or above) the navbar.

use chrono::Local;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::tuios::models::{Action, FocusTarget, TuiosState};
use crate::tuios::sessions::{Body, Kind, Phase, Session, SessionEvent, SessionId, Sessions};

use super::bar_config::{BarConfig, Status};
use super::state::{Focus, RowId, Tab, TmAction};
use super::{graphs, keys, metrics};

/// Rows the header takes when the bar is on (tiles need a full box).
pub const HEIGHT: u16 = 3;
/// Seconds a crashed app stays on the bar before it disappears.
const CRASH_LINGER_SECS: i64 = 5;
const TILE_MIN: u16 = 12;
const TILE_MAX: u16 = 24;
const MARKER_W: u16 = 3;
/// Narrowest the navbar + stats side may get in Auto width.
const LEFT_MIN: u16 = 44;
/// Below this the bar is hidden rather than squeezed.
const BAR_MIN: u16 = 18;

/// Identifies a tile across frames: one app (all its instances), or one session when
/// grouping is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileKey {
    key: String,
    kind: Kind,
    solo: Option<SessionId>,
}

pub struct Tile {
    pub ident: TileKey,
    pub label: String,
    /// Oldest first.
    pub ids: Vec<SessionId>,
    pub status: Status,
    pub info: String,
}

#[derive(Debug, Clone)]
pub enum Menu {
    /// A grouped tile: pick an instance (the last row is Quit All).
    Instances { cursor: usize },
    Actions { id: SessionId, cursor: usize, from_group: bool },
}

#[derive(Debug, Clone, Default)]
pub struct TaskBarState {
    /// Keyboard focus is on the bar (only while the navbar row has focus).
    pub focused: bool,
    pub selected: Option<TileKey>,
    pub scroll: usize,
    pub menu: Option<Menu>,
    /// The bar was drawn last frame (it hides when off or when the screen is too narrow).
    pub visible: bool,
}

/// Where the header's parts go.
pub struct HeaderLayout {
    pub nav: Rect,
    pub stats: Option<Rect>,
    pub bar: Option<Rect>,
}

// ── Tiles ────────────────────────────────────────────────────────────────────

fn crashed_recently(s: &Session) -> bool {
    match &s.phase {
        Phase::Exited { code, at } => {
            *code != Some(0) && (Local::now() - *at).num_seconds() < CRASH_LINGER_SECS
        }
        _ => false,
    }
}

fn on_screen(s: &Session, state: &TuiosState, sessions: &Sessions) -> bool {
    if state.active_page.is_some() {
        return false;
    }
    match s.kind {
        Kind::App => state.active_app.is_some() && sessions.foreground == Some(s.id),
        Kind::Dashboard => state.active_app.is_none() && state.active_dashboard == s.key,
    }
}

fn status_of(s: &Session, state: &TuiosState, sessions: &Sessions) -> Status {
    match &s.phase {
        Phase::Exited { .. } => Status::Crashed,
        Phase::Stopping { .. } => Status::Stopping,
        Phase::Suspended => Status::Suspended,
        Phase::Running => match &s.body {
            Body::External(e) if e.pid.is_none() => Status::Locating,
            Body::External(_) => Status::Window,
            _ if on_screen(s, state, sessions) => Status::Shown,
            _ => Status::Background,
        },
    }
}

/// Which status a grouped tile shows when its instances differ.
fn rank(status: Status) -> u8 {
    match status {
        Status::Shown => 0,
        Status::Crashed => 1,
        Status::Stopping => 2,
        Status::Suspended => 3,
        Status::Background => 4,
        Status::Window => 5,
        Status::Locating => 6,
    }
}

fn status_text(status: Status) -> &'static str {
    match status {
        Status::Shown => "On screen",
        Status::Background => "Background",
        Status::Window => "Own window",
        Status::Suspended => "Suspended",
        Status::Stopping => "Quitting…",
        Status::Locating => "Finding window",
        Status::Crashed => "✗ Crashed",
    }
}

/// "Notes (2)" → "Notes".
fn base_label(s: &Session) -> String {
    let suffix = format!(" ({})", s.instance);
    s.label.strip_suffix(&suffix).unwrap_or(&s.label).to_string()
}

fn info(cfg: &BarConfig, status: Status, members: &[&Session]) -> String {
    if status == Status::Crashed || cfg.tile_info == "Status" {
        return status_text(status).into();
    }
    if cfg.tile_info == "Uptime" {
        let up = members.iter().map(|s| s.uptime()).max().unwrap_or_default();
        return format!("Up {}", graphs::duration(up));
    }
    if members.iter().all(|s| s.is_builtin()) {
        return "Built-in".into();
    }
    let m = metrics::read();
    let samples: Vec<f32> = members
        .iter()
        .filter_map(|s| s.pid())
        .filter_map(|pid| m.procs.get(&pid).map(|p| p.cpu))
        .collect();
    if samples.is_empty() {
        "CPU –".into()
    } else {
        format!("CPU {:.0}%", samples.iter().sum::<f32>())
    }
}

/// Every running app (tuiOS itself is never a session), plus crashes from the last
/// few seconds, in launch order.
pub fn tiles(state: &TuiosState, sessions: &Sessions, cfg: &BarConfig) -> Vec<Tile> {
    let shown: Vec<&Session> = sessions.iter().filter(|s| s.is_live() || crashed_recently(s)).collect();
    let mut groups: Vec<(TileKey, Vec<&Session>)> = Vec::new();
    for s in shown {
        let ident = TileKey {
            key: s.key.clone(),
            kind: s.kind,
            solo: if cfg.group { None } else { Some(s.id) },
        };
        match groups.iter_mut().find(|(k, _)| *k == ident) {
            Some((_, members)) => members.push(s),
            None => groups.push((ident, vec![s])),
        }
    }
    groups
        .into_iter()
        .map(|(ident, members)| {
            let status = members
                .iter()
                .map(|s| status_of(s, state, sessions))
                .min_by_key(|st| rank(*st))
                .unwrap_or(Status::Background);
            let mut label = if ident.solo.is_some() { members[0].label.clone() } else { base_label(members[0]) };
            if members.len() > 1 {
                label.push_str(&format!(" ×{}", members.len()));
            }
            Tile {
                info: info(cfg, status, &members),
                label: super::clean_text(&label),
                ids: members.iter().map(|s| s.id).collect(),
                status,
                ident,
            }
        })
        .collect()
}

fn selected_index(bar: &TaskBarState, tiles: &[Tile]) -> usize {
    bar.selected
        .as_ref()
        .and_then(|k| tiles.iter().position(|t| t.ident == *k))
        .unwrap_or(0)
        .min(tiles.len().saturating_sub(1))
}

fn tile_width(t: &Tile) -> u16 {
    let text = t.label.chars().count().max(t.info.chars().count()) as u16;
    (text + 4).clamp(TILE_MIN, TILE_MAX)
}

fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(width.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

// ── Layout ───────────────────────────────────────────────────────────────────

/// Split the 3-row header into navbar, stats (2 rows) and the bar. `nav_needed` is
/// the width the navbar's items and clock take.
pub fn split_header(area: Rect, cfg: &BarConfig, nav_needed: u16) -> HeaderLayout {
    let left_w = match cfg.width_pct {
        Some(pct) => area.width - area.width * pct.min(90) / 100,
        None => nav_needed.max(LEFT_MIN).min(area.width.saturating_sub(BAR_MIN)),
    };
    let bar_w = area.width.saturating_sub(left_w);
    let (left_w, bar) = if bar_w < BAR_MIN {
        (area.width, None)
    } else {
        (left_w, Some(Rect { x: area.x + left_w, y: area.y, width: bar_w, height: area.height }))
    };
    let row = |dy: u16, h: u16| Rect { x: area.x, y: area.y + dy, width: left_w, height: h };
    let (nav, stats) = if cfg.stats_above { (row(2, 1), row(0, 2)) } else { (row(0, 1), row(1, 2)) };
    HeaderLayout { nav, stats: Some(stats), bar }
}

// ── Stats line ───────────────────────────────────────────────────────────────

const EIGHTHS: [&str; 8] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// "CPU ▕████▌░░░░▏ 42%" filling `area` (one row). `short` replaces `suffix` when the
/// bar would otherwise get too thin to read.
fn meter(frame: &mut Frame, area: Rect, label: &str, pct: f32, suffix: &str, short: &str) {
    let label = format!(" {} ", label);
    let room = |s: &str| (area.width as usize).saturating_sub(label.chars().count() + s.chars().count() + 4);
    let suffix = if room(suffix) >= 8 { format!(" {} ", suffix) } else { format!(" {} ", short) };
    let width = room(suffix.trim());
    let pct = pct.clamp(0.0, 100.0);
    let colour = graphs::heat(pct as f64);
    let eighths = (pct as f64 / 100.0 * width as f64 * 8.0).round() as usize;
    let (full, part) = (eighths / 8, eighths % 8);
    let mut filled = "█".repeat(full.min(width));
    if full < width && part > 0 {
        filled.push_str(EIGHTHS[part]);
    }
    let empty = width.saturating_sub(filled.chars().count());
    let line = Line::from(vec![
        Span::styled(label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("▕", Style::default().fg(Color::DarkGray)),
        Span::styled(filled, Style::default().fg(colour)),
        Span::styled("░".repeat(empty), Style::default().fg(Color::DarkGray)),
        Span::styled("▏", Style::default().fg(Color::DarkGray)),
        Span::styled(suffix, Style::default().fg(colour)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// "9.8/16.0 GB"
fn used_of(used: u64, total: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    if total as f64 >= GB {
        format!("{:.1}/{:.1} GB", used as f64 / GB, total as f64 / GB)
    } else {
        format!("{}/{} MB", used / 1024 / 1024, total / 1024 / 1024)
    }
}

fn uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, (secs / 3600) % 24, (secs / 60) % 60);
    if d > 0 {
        format!("{}d {}h", d, h)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}

fn extras(cfg: &BarConfig, m: &metrics::Metrics, width: usize) -> Line<'static> {
    let label = Style::default().fg(Color::DarkGray);
    let value = Style::default().fg(Color::White);
    let mut items: Vec<Vec<Span<'static>>> = Vec::new();
    if cfg.show_gpu {
        if let Some(gpu) = m.gpu_now() {
            items.push(vec![
                Span::styled("GPU ", label),
                Span::styled(format!("{:.0}%", gpu), Style::default().fg(graphs::heat(gpu as f64))),
            ]);
        }
    }
    if cfg.show_temperature {
        let temp = m.cpu_temp.last().copied().flatten().or_else(|| m.gpu_temp.last().copied().flatten());
        if let Some(t) = temp {
            let colour = if t >= 80.0 { Color::Red } else if t >= 65.0 { Color::Yellow } else { Color::Green };
            items.push(vec![Span::styled("Temp ", label), Span::styled(format!("{:.0}°C", t), Style::default().fg(colour))]);
        }
    }
    if cfg.show_network {
        let rx = m.net_rx.last().copied().unwrap_or(0.0);
        let tx = m.net_tx.last().copied().unwrap_or(0.0);
        items.push(vec![
            Span::styled("↓ ", label),
            Span::styled(graphs::rate(rx), value),
            Span::styled("  ↑ ", label),
            Span::styled(graphs::rate(tx), value),
        ]);
    }
    if cfg.show_swap && m.swap_total > 0 {
        items.push(vec![Span::styled("Swap ", label), Span::styled(used_of(m.swap_used, m.swap_total), value)]);
    }
    if cfg.show_uptime && m.uptime > 0 {
        items.push(vec![Span::styled("Up ", label), Span::styled(uptime(m.uptime), value)]);
    }
    // Whole items only: one cut off mid-way reads as a different number
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for item in items {
        let sep = if used > 1 { "  · " } else { "" };
        let w: usize = sep.chars().count() + item.iter().map(|s| s.content.chars().count()).sum::<usize>();
        if used + w > width {
            break;
        }
        if !sep.is_empty() {
            spans.push(Span::styled(sep, label));
        }
        spans.extend(item);
        used += w;
    }
    Line::from(spans)
}

pub fn render_stats(frame: &mut Frame, area: Rect, cfg: &BarConfig) {
    let m = metrics::read();
    if !m.ready {
        let text = Span::styled(" Collecting system stats…", Style::default().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(text), Rect { height: 1, ..area });
        return;
    }
    let cpu = m.cpu_now();
    let ram_pct = m.mem_pct();
    let cpu_text = format!("{:.0}%", cpu);
    let ram_text = format!("{:.0}%", ram_pct);
    let ram_full = format!("{} {}", ram_text, used_of(m.mem_now(), m.mem_total));
    let top = Rect { height: 1, ..area };
    let bottom = Rect { y: area.y + 1, height: area.height.saturating_sub(1).min(1), ..area };
    if cfg.extra_row {
        let half = area.width / 2;
        meter(frame, Rect { width: half, ..top }, "CPU", cpu, &cpu_text, &cpu_text);
        meter(frame, Rect { x: top.x + half, width: top.width - half, ..top }, "RAM", ram_pct, &ram_full, &ram_text);
        frame.render_widget(Paragraph::new(extras(cfg, &m, bottom.width as usize)), bottom);
    } else {
        meter(frame, top, "CPU", cpu, &cpu_text, &cpu_text);
        meter(frame, bottom, "RAM", ram_pct, &ram_full, &ram_text);
    }
}

// ── Bar ──────────────────────────────────────────────────────────────────────

/// Tiles that fit from `scroll`: (end index, left marker shown, right marker shown).
fn visible_range(widths: &[u16], scroll: usize, total: u16) -> (usize, bool, bool) {
    let left = scroll > 0;
    let take = |avail: u16| {
        let mut used = 0u16;
        let mut end = scroll;
        while end < widths.len() && used + widths[end] <= avail {
            used += widths[end];
            end += 1;
        }
        end
    };
    let avail = total.saturating_sub(if left { MARKER_W } else { 0 });
    let end = take(avail);
    if end < widths.len() {
        // Room for the right marker, but always at least one tile
        (take(avail.saturating_sub(MARKER_W)).max(scroll + 1).min(widths.len()), left, true)
    } else {
        (end, left, false)
    }
}

/// Draw the bar; returns the selected tile's rectangle (for its menu).
pub fn render_bar(
    frame: &mut Frame,
    area: Rect,
    state: &mut TuiosState,
    sessions: &Sessions,
    cfg: &BarConfig,
    border_type: BorderType,
) -> Option<Rect> {
    let tiles = tiles(state, sessions, cfg);
    let focused = state.task_bar.focused && state.focus == FocusTarget::Navbar;
    state.task_bar.visible = true;
    // 1 column gap from the navbar side
    let area = Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area };
    if tiles.is_empty() {
        state.task_bar.focused = false;
        state.task_bar.menu = None;
        let mid = Rect { y: area.y + 1, height: 1, ..area };
        frame.render_widget(
            Paragraph::new("No apps running").alignment(Alignment::Center).style(Style::default().fg(Color::DarkGray)),
            mid,
        );
        return None;
    }

    let sel = selected_index(&state.task_bar, &tiles);
    if state.task_bar.selected.is_none() || focused {
        state.task_bar.selected = Some(tiles[sel].ident.clone());
    }
    let widths: Vec<u16> = tiles.iter().map(tile_width).collect();
    let bar = &mut state.task_bar;
    bar.scroll = bar.scroll.min(tiles.len() - 1);
    if focused && sel < bar.scroll {
        bar.scroll = sel;
    }
    while focused && sel >= visible_range(&widths, bar.scroll, area.width).0 && bar.scroll < sel {
        bar.scroll += 1;
    }
    let (end, left_marker, right_marker) = visible_range(&widths, bar.scroll, area.width);
    let marker_style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD);
    let mid_y = area.y + 1;

    let mut x = area.x;
    if left_marker {
        let text = format!("◀{}", bar.scroll);
        frame.render_widget(Paragraph::new(text).style(marker_style), Rect { x, y: mid_y, width: MARKER_W, height: 1 });
        x += MARKER_W;
    }
    let mut selected_rect = None;
    for (i, tile) in tiles.iter().enumerate().take(end).skip(bar.scroll) {
        let width = widths[i].min(area.right().saturating_sub(x));
        if width < 4 {
            break;
        }
        let rect = Rect { x, y: area.y, width, height: HEIGHT.min(area.height) };
        let colour = cfg.colour(tile.status);
        let is_sel = focused && i == sel;
        let title_style = if is_sel {
            Style::default().fg(Color::Black).bg(colour).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        };
        let inner_w = width.saturating_sub(2) as usize;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(if is_sel { BorderType::Thick } else { border_type })
            .border_style(Style::default().fg(colour))
            .title(Span::styled(fit(&format!(" {} ", tile.label), inner_w), title_style));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        frame.render_widget(
            Paragraph::new(fit(&tile.info, inner_w))
                .alignment(Alignment::Center)
                .style(Style::default().fg(if tile.status == Status::Crashed { colour } else { Color::Gray })),
            inner,
        );
        if i == sel {
            selected_rect = Some(rect);
        }
        x += width;
    }
    if right_marker {
        let hidden = tiles.len() - end;
        let rx = area.right().saturating_sub(MARKER_W);
        frame.render_widget(
            Paragraph::new(format!("{}▶", hidden)).alignment(Alignment::Right).style(marker_style),
            Rect { x: rx, y: mid_y, width: MARKER_W, height: 1 },
        );
    }
    selected_rect.filter(|_| focused)
}

// ── Tile menu ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Do(TmAction),
    ShowInTaskManager,
}

fn label(item: Item, s: &Session) -> &'static str {
    match item {
        Item::Do(TmAction::OpenInContainer) if s.is_external() => "Open in Container (restarts it)",
        Item::Do(TmAction::OpenInContainer) if s.kind == Kind::Dashboard => "Show on Home Screen",
        Item::Do(TmAction::OpenInContainer) => "Show",
        Item::Do(TmAction::OpenInWindow) => "Open in New Window (restarts it)",
        Item::Do(TmAction::Restart) => "Restart",
        Item::Do(TmAction::Quit) => "Quit",
        Item::Do(TmAction::LaunchAnother) => "Launch Another Instance",
        Item::Do(_) => "",
        Item::ShowInTaskManager => "Show in Task Manager",
    }
}

fn items_for(s: &Session) -> Vec<Item> {
    use TmAction::*;
    let locating = matches!(&s.body, Body::External(e) if e.pid.is_none());
    match s.phase {
        Phase::Exited { .. } => vec![Item::Do(Restart), Item::ShowInTaskManager],
        Phase::Stopping { .. } => vec![Item::ShowInTaskManager],
        _ if locating => vec![Item::ShowInTaskManager],
        _ => {
            let mut out = vec![Item::Do(OpenInContainer)];
            if s.is_pty() {
                out.push(Item::Do(OpenInWindow));
            }
            out.extend([Item::Do(Restart), Item::Do(Quit), Item::ShowInTaskManager]);
            if s.kind == Kind::App {
                out.push(Item::Do(LaunchAnother));
            }
            out
        }
    }
}

/// Rows of the open menu.
fn menu_rows(menu: &Menu, tile: &Tile, sessions: &Sessions) -> Vec<String> {
    match menu {
        Menu::Instances { .. } => {
            let mut rows: Vec<String> = tile
                .ids
                .iter()
                .filter_map(|id| sessions.get(*id))
                .map(|s| format!("{}  ·  {}", s.label, status_text_short(s)))
                .collect();
            rows.push("Quit All".into());
            rows
        }
        Menu::Actions { id, .. } => match sessions.get(*id) {
            Some(s) => items_for(s).into_iter().map(|i| label(i, s).to_string()).collect(),
            None => Vec::new(),
        },
    }
}

fn status_text_short(s: &Session) -> String {
    s.status_label(false).to_lowercase()
}

/// The open tile menu, dropped from `tile` into `area` (the main container side).
pub fn render_menu(frame: &mut Frame, area: Rect, tile: Rect, upward: bool, state: &TuiosState, sessions: &Sessions, cfg: &BarConfig) {
    let Some(menu) = &state.task_bar.menu else { return };
    let tiles = tiles(state, sessions, cfg);
    let Some(t) = tiles.get(selected_index(&state.task_bar, &tiles)) else { return };
    let rows = menu_rows(menu, t, sessions);
    if rows.is_empty() {
        return;
    }
    let cursor = match menu {
        Menu::Instances { cursor } | Menu::Actions { cursor, .. } => *cursor,
    };
    let width = (rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16 + 6).min(area.width);
    let height = (rows.len() as u16 + 2).min(area.height);
    let x = tile.x.min(area.right().saturating_sub(width));
    let y = if upward { area.bottom().saturating_sub(height) } else { area.y };
    let rect = Rect { x, y, width, height };
    frame.render_widget(Clear, rect);
    let colour = cfg.colour(t.status);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colour))
        .title(Span::styled(fit(&format!(" {} ", t.label), width.saturating_sub(2) as usize), Style::default().fg(colour)))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, text)| {
            let style = if i == cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if matches!(menu, Menu::Instances { .. }) && i + 1 == rows.len() {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if i == cursor { " » " } else { "   " };
            Line::from(Span::styled(format!("{}{} ", prefix, text), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Keys ─────────────────────────────────────────────────────────────────────

pub enum Outcome {
    None,
    Events(Vec<SessionEvent>),
    /// Focus goes back to the navbar's last item.
    LeaveLeft,
    /// Focus wraps round to the navbar's first item.
    LeaveRight,
}

/// Move focus from the navbar onto the bar (Right off the last item, or Left off
/// the first). False when there is nothing to focus.
pub fn focus_from_navbar(state: &mut TuiosState, sessions: &Sessions, from_left: bool) -> bool {
    let cfg = super::bar_config::load();
    if !cfg.enabled || !state.task_bar.visible {
        return false;
    }
    let tiles = tiles(state, sessions, &cfg);
    let Some(tile) = (if from_left { tiles.first() } else { tiles.last() }) else { return false };
    state.task_bar.selected = Some(tile.ident.clone());
    state.task_bar.focused = true;
    state.task_bar.menu = None;
    true
}

fn step(cursor: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        0
    } else if forward {
        (cursor + 1) % len
    } else {
        (cursor + len - 1) % len
    }
}

fn show_in_task_manager(state: &mut TuiosState, id: SessionId) {
    let tm = &mut state.task_manager;
    tm.tab = Tab::Apps;
    tm.focus = Focus::Content;
    tm.selected = RowId::Session(id);
    state.pending_nav_action = Some("page:taskmanager".into());
}

pub fn handle_action(action: Action, state: &mut TuiosState, sessions: &mut Sessions) -> Outcome {
    let cfg = super::bar_config::load();
    let tiles = tiles(state, sessions, &cfg);
    if tiles.is_empty() {
        state.task_bar.focused = false;
        state.task_bar.menu = None;
        return Outcome::LeaveLeft;
    }
    let sel = selected_index(&state.task_bar, &tiles);
    let tile = &tiles[sel];

    match state.task_bar.menu.clone() {
        None => match action {
            Action::Left if sel == 0 => {
                state.task_bar.focused = false;
                Outcome::LeaveLeft
            }
            Action::Right if sel + 1 == tiles.len() => {
                state.task_bar.focused = false;
                Outcome::LeaveRight
            }
            Action::Left | Action::Right => {
                let next = if action == Action::Right { sel + 1 } else { sel - 1 };
                state.task_bar.selected = Some(tiles[next].ident.clone());
                Outcome::None
            }
            Action::Enter | Action::Down => {
                state.task_bar.menu = Some(if tile.ids.len() > 1 {
                    Menu::Instances { cursor: 0 }
                } else {
                    Menu::Actions { id: tile.ids[0], cursor: 0, from_group: false }
                });
                Outcome::None
            }
            Action::Back => {
                state.task_bar.focused = false;
                Outcome::LeaveLeft
            }
            _ => Outcome::None,
        },
        Some(Menu::Instances { cursor }) => {
            let len = tile.ids.len() + 1;
            match action {
                Action::Up => state.task_bar.menu = Some(Menu::Instances { cursor: step(cursor, len, false) }),
                Action::Down => state.task_bar.menu = Some(Menu::Instances { cursor: step(cursor, len, true) }),
                Action::Enter | Action::Right => {
                    if let Some(id) = tile.ids.get(cursor) {
                        state.task_bar.menu = Some(Menu::Actions { id: *id, cursor: 0, from_group: true });
                    } else if action == Action::Enter {
                        state.task_bar.menu = None;
                        let mut events = Vec::new();
                        for id in &tile.ids {
                            events.extend(sessions.close(*id));
                        }
                        events.push(SessionEvent::Toast(format!("Quitting all {}…", tile.label)));
                        return Outcome::Events(events);
                    }
                }
                Action::Back | Action::Left => state.task_bar.menu = None,
                _ => {}
            }
            Outcome::None
        }
        Some(Menu::Actions { id, cursor, from_group }) => {
            let Some(s) = sessions.get(id) else {
                state.task_bar.menu = None;
                return Outcome::None;
            };
            let items = items_for(s);
            match action {
                Action::Up => {
                    state.task_bar.menu = Some(Menu::Actions { id, cursor: step(cursor, items.len(), false), from_group })
                }
                Action::Down => {
                    state.task_bar.menu = Some(Menu::Actions { id, cursor: step(cursor, items.len(), true), from_group })
                }
                Action::Enter => {
                    state.task_bar.menu = None;
                    return match items.get(cursor) {
                        Some(Item::Do(act)) => Outcome::Events(keys::run_action(*act, id, state, sessions)),
                        Some(Item::ShowInTaskManager) => {
                            show_in_task_manager(state, id);
                            Outcome::None
                        }
                        None => Outcome::None,
                    };
                }
                Action::Back | Action::Left => {
                    state.task_bar.menu = if from_group {
                        let back = tile.ids.iter().position(|i| *i == id).unwrap_or(0);
                        Some(Menu::Instances { cursor: back })
                    } else {
                        None
                    };
                }
                _ => {}
            }
            Outcome::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_keeps_markers_and_one_tile() {
        let widths = [14, 14, 14, 14, 14];
        // Everything fits
        assert_eq!(visible_range(&widths, 0, 80), (5, false, false));
        // Overflow: right marker takes 3 columns
        assert_eq!(visible_range(&widths, 0, 40), (2, false, true));
        // Scrolled: left marker too
        assert_eq!(visible_range(&widths, 2, 40), (4, true, true));
        // Last page: no right marker
        assert_eq!(visible_range(&widths, 3, 40), (5, true, false));
        // Never zero tiles, even when one does not fit
        assert_eq!(visible_range(&widths, 1, 10).0, 2);
    }

    #[test]
    fn uptime_text() {
        assert_eq!(uptime(59), "0m");
        assert_eq!(uptime(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(uptime(2 * 86_400 + 4 * 3600), "2d 4h");
    }
}
