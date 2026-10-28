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

use super::bar_config::{BarConfig, Stat, Status};
use super::state::{Focus, RowId, Tab, TmAction};
use super::{graphs, keys, metrics};

/// Rows the header takes when the bar is on: navbar + stats line beside two-row tiles.
pub const HEIGHT: u16 = 2;
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
/// few seconds. The most recently opened app comes first.
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
    // Session ids grow with every launch, so the newest instance dates the tile
    groups.sort_by_key(|(_, members)| std::cmp::Reverse(members.iter().map(|s| s.id).max().unwrap_or(0)));
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

/// Split the 2-row header into the navbar, the stats line and the bar. `nav_needed` is
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
    let row = |dy: u16| Rect { x: area.x, y: area.y + dy, width: left_w, height: 1 };
    let (nav, stats) = if cfg.stats_above { (row(1), row(0)) } else { (row(0), row(1)) };
    HeaderLayout { nav, stats: Some(stats), bar }
}

// ── Stats line ───────────────────────────────────────────────────────────────

const EIGHTHS: [&str; 8] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// "CPU ▕████▌░░░░▏ 42%" in `area` (one row). Falls back to the `short` suffix, then to
/// plain "CPU 42%", as the room for the bar shrinks.
fn meter(frame: &mut Frame, area: Rect, label: &str, pct: f32, long: &str, short: &str) {
    let pct = pct.clamp(0.0, 100.0);
    let colour = graphs::heat(pct as f64);
    let bold = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    // "CPU " + "▕" + bar + "▏" + " " + suffix
    let room = |s: &str| (area.width as usize).saturating_sub(label.chars().count() + s.chars().count() + 4);
    let suffix = if room(long) >= 6 { long } else { short };
    let width = room(suffix);
    if width < 3 {
        let line = Line::from(vec![
            Span::styled(format!("{} ", label), bold),
            Span::styled(short.to_string(), Style::default().fg(colour)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let eighths = (pct as f64 / 100.0 * width as f64 * 8.0).round() as usize;
    let (full, part) = (eighths / 8, eighths % 8);
    let mut filled = "█".repeat(full.min(width));
    if full < width && part > 0 {
        filled.push_str(EIGHTHS[part]);
    }
    let empty = width.saturating_sub(filled.chars().count());
    let line = Line::from(vec![
        Span::styled(format!("{} ", label), bold),
        Span::styled("▕", Style::default().fg(Color::DarkGray)),
        Span::styled(filled, Style::default().fg(colour)),
        Span::styled("░".repeat(empty), Style::default().fg(Color::DarkGray)),
        Span::styled("▏", Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {}", suffix), Style::default().fg(colour)),
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

/// One item on the stats line.
enum Piece {
    /// label, percent, long suffix, short suffix
    Meter(&'static str, f32, String, String),
    Text(Vec<Span<'static>>),
}

impl Piece {
    fn text_width(&self) -> usize {
        match self {
            Piece::Text(spans) => spans.iter().map(|s| s.content.chars().count()).sum(),
            Piece::Meter(..) => 0,
        }
    }
}

/// Narrowest a meter can be and still show a usable bar: "CPU ▕█████▏ 42%".
const METER_MIN: usize = 15;
const GAP: usize = 3;

fn pieces(cfg: &BarConfig, m: &metrics::Metrics) -> Vec<Piece> {
    let dim = Style::default().fg(Color::DarkGray);
    let white = Style::default().fg(Color::White);
    let pct = |v: f32| format!("{:.0}%", v);
    cfg.stats
        .iter()
        .map(|stat| match stat {
            Stat::Cpu => Piece::Meter("CPU", m.cpu_now(), pct(m.cpu_now()), pct(m.cpu_now())),
            Stat::Ram => {
                let p = m.mem_pct();
                Piece::Meter("RAM", p, format!("{} {}", pct(p), used_of(m.mem_now(), m.mem_total)), pct(p))
            }
            Stat::Gpu => match m.gpu_now() {
                Some(g) => Piece::Meter("GPU", g, pct(g), pct(g)),
                None => Piece::Text(vec![Span::styled("GPU n/a", dim)]),
            },
            Stat::Swap if m.swap_total > 0 => {
                let p = m.swap_used as f32 / m.swap_total as f32 * 100.0;
                Piece::Meter("Swap", p, format!("{} {}", pct(p), used_of(m.swap_used, m.swap_total)), pct(p))
            }
            Stat::Swap => Piece::Text(vec![Span::styled("Swap off", dim)]),
            Stat::Temperature => {
                match m.cpu_temp.last().copied().flatten().or_else(|| m.gpu_temp.last().copied().flatten()) {
                    Some(t) => {
                        let colour = if t >= 80.0 { Color::Red } else if t >= 65.0 { Color::Yellow } else { Color::Green };
                        Piece::Text(vec![Span::styled("Temp ", dim), Span::styled(format!("{:.0}°C", t), Style::default().fg(colour))])
                    }
                    None => Piece::Text(vec![Span::styled("Temp n/a", dim)]),
                }
            }
            Stat::Network => Piece::Text(vec![
                Span::styled("↓ ", dim),
                Span::styled(graphs::rate(m.net_rx.last().copied().unwrap_or(0.0)), white),
                Span::styled(" ↑ ", dim),
                Span::styled(graphs::rate(m.net_tx.last().copied().unwrap_or(0.0)), white),
            ]),
            Stat::Uptime => Piece::Text(vec![Span::styled("Up ", dim), Span::styled(uptime(m.uptime), white)]),
        })
        .collect()
}

pub fn render_stats(frame: &mut Frame, area: Rect, cfg: &BarConfig) {
    let m = metrics::read();
    if !m.ready {
        let text = Span::styled(" Collecting system stats…", Style::default().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(text), area);
        return;
    }
    let mut pieces = pieces(cfg, &m);
    // 1 column of padding at the left
    let width = (area.width as usize).saturating_sub(1);
    let needed = |pieces: &[Piece]| {
        let meters = pieces.iter().filter(|p| matches!(p, Piece::Meter(..))).count();
        let texts: usize = pieces.iter().map(Piece::text_width).sum();
        texts + pieces.len().saturating_sub(1) * GAP + meters * METER_MIN
    };
    // Too much for the space: drop items from the end, so the ones listed first keep
    // a full-size bar
    while pieces.len() > 1 && needed(&pieces) > width {
        pieces.pop();
    }

    let meters = pieces.iter().filter(|p| matches!(p, Piece::Meter(..))).count();
    let texts: usize = pieces.iter().map(Piece::text_width).sum();
    let spare = width.saturating_sub(texts + pieces.len().saturating_sub(1) * GAP);
    let (meter_w, mut extra) = if meters > 0 { (spare / meters, spare % meters) } else { (0, 0) };
    let mut x = area.x + 1;
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            x += GAP as u16;
        }
        let w = match piece {
            Piece::Meter(..) => {
                let w = meter_w + if extra > 0 { 1 } else { 0 };
                extra = extra.saturating_sub(1);
                w
            }
            Piece::Text(_) => piece.text_width(),
        } as u16;
        let w = w.min(area.right().saturating_sub(x));
        if w == 0 {
            break;
        }
        let rect = Rect { x, width: w, ..area };
        match piece {
            Piece::Meter(label, pct, long, short) => meter(frame, rect, label, *pct, long, short),
            Piece::Text(spans) => frame.render_widget(Paragraph::new(Line::from(spans.clone())), rect),
        }
        x += w;
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

/// The box-drawing character for a tile edge meeting the main container's border.
/// `from_above`: the tile sits above that border (header at the top of the screen).
fn junction(existing: &str, from_above: bool, double: bool) -> Option<&'static str> {
    Some(match (existing, from_above, double) {
        ("─", true, false) => "┴",
        ("─", false, false) => "┬",
        ("─", true, true) => "╨",
        ("─", false, true) => "╥",
        ("═", true, true) => "╩",
        ("═", false, true) => "╦",
        ("═", true, false) => "╧",
        ("═", false, false) => "╤",
        ("╮" | "┐", true, false) | ("╯" | "┘", false, false) => "┤",
        ("╭" | "┌", true, false) | ("╰" | "└", false, false) => "├",
        ("╗", true, true) | ("╝", false, true) => "╣",
        ("╔", true, true) | ("╚", false, true) => "╠",
        _ => return None,
    })
}

fn join(frame: &mut Frame, x: u16, y: u16, colour: Color, from_above: bool, double: bool) {
    if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
        // Titles and other content on the border line are left alone
        if let Some(symbol) = junction(cell.symbol(), from_above, double) {
            cell.set_symbol(symbol).set_fg(colour);
        }
    }
}

/// Draw the bar; returns the selected tile's rectangle (for its menu). Tiles are open on
/// the side facing the main container, and their edges join its border at `joint_row`.
pub fn render_bar(
    frame: &mut Frame,
    area: Rect,
    state: &mut TuiosState,
    sessions: &Sessions,
    cfg: &BarConfig,
    border_type: BorderType,
    header_top: bool,
    joint_row: Option<u16>,
) -> Option<Rect> {
    let tiles = tiles(state, sessions, cfg);
    let focused = state.task_bar.focused && state.focus == FocusTarget::Navbar;
    state.task_bar.visible = true;
    // 1 column gap from the navbar side
    let area = Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area };
    // The row inside the tiles; the other one is their border carrying the name
    let info_y = if header_top { area.y + 1 } else { area.y };
    if tiles.is_empty() {
        state.task_bar.focused = false;
        state.task_bar.menu = None;
        frame.render_widget(
            Paragraph::new("No apps running").alignment(Alignment::Center).style(Style::default().fg(Color::DarkGray)),
            Rect { y: info_y, height: 1, ..area },
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
    let double = border_type == BorderType::Double;
    let borders = Borders::LEFT | Borders::RIGHT | if header_top { Borders::TOP } else { Borders::BOTTOM };

    let mut x = area.x;
    if left_marker {
        let text = format!("◀{}", bar.scroll);
        frame.render_widget(Paragraph::new(text).style(marker_style), Rect { x, y: info_y, width: MARKER_W, height: 1 });
        x += MARKER_W;
    }
    let mut selected_rect = None;
    let limit = area.right().saturating_sub(if right_marker { MARKER_W } else { 0 });
    for (i, tile) in tiles.iter().enumerate().take(end).skip(bar.scroll) {
        let width = widths[i].min(limit.saturating_sub(x));
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
        let title = Span::styled(fit(&format!(" {} ", tile.label), inner_w), title_style);
        let block = Block::default()
            .borders(borders)
            .border_type(border_type)
            .border_style(Style::default().fg(colour));
        let block = if header_top { block.title(title) } else { block.title_bottom(title) };
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let info_style = if tile.status == Status::Crashed {
            Style::default().fg(colour)
        } else if is_sel {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        frame.render_widget(Paragraph::new(fit(&tile.info, inner_w)).alignment(Alignment::Center).style(info_style), inner);
        if let Some(y) = joint_row {
            join(frame, rect.x, y, colour, header_top, double);
            join(frame, rect.right() - 1, y, colour, header_top, double);
        }
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
            Rect { x: rx, y: info_y, width: MARKER_W, height: 1 },
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
