/// Dashboard 1 — Detailed: the default home screen.
///
/// A responsive tile grid spread over three pages. Left/Right switch page,
/// Up/Down move the selection, Enter zooms the selected tile to fill the
/// container.
///
/// tuiOS dashboard API:
///     render(frame, area, border_style)  — draw the dashboard inside `area`
///     handle_action(action)              — returns true when the key was used

pub mod bigclock;
pub mod metrics;
pub mod online;
pub mod panels;

use std::sync::Mutex;

use lazy_static::lazy_static;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
    Frame,
};

use crate::settings::persistence;
use crate::tuios::models::Action;
use panels::Ctx;

pub const KEY: &str = "Dashboard-1";
pub const LABEL: &str = "Dashboard 1 — Detailed";
pub const DESCRIPTION: &str =
    "The tuiOS home screen — clock, live system vitals, processes, network, battery and weather.";

const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 8;
const WIDE: u16 = 84;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Clock,
    Vitals,
    Cores,
    History,
    Host,
    Network,
    Battery,
    Weather,
    Connectivity,
    TopCpu,
    TopMem,
}

const PAGE_NAMES: [&str; 3] = ["Overview", "Performance", "System"];

struct Ui {
    page: usize,
    panel: usize,
    zoomed: bool,
}

lazy_static! {
    static ref UI: Mutex<Ui> = Mutex::new(Ui {
        page: 0,
        panel: 0,
        zoomed: false,
    });
}

/// Handle a navigation key while this dashboard is on screen.
/// Returns true when the key was consumed.
pub fn handle_action(action: Action) -> bool {
    let mut ui = UI.lock().unwrap();
    match action {
        Action::Left | Action::Right if !ui.zoomed => {
            let forward = action == Action::Right;
            ui.page = step(ui.page, PAGE_NAMES.len(), forward);
            ui.panel = 0;
            true
        }
        Action::Up | Action::Down | Action::Left | Action::Right => {
            let forward = matches!(action, Action::Down | Action::Right);
            // The real panel count depends on the rendered layout; the renderer
            // clamps this, so wrapping on a generous bound is good enough.
            ui.panel = step(ui.panel, panel_count(ui.page), forward);
            true
        }
        Action::Enter => {
            ui.zoomed = !ui.zoomed;
            true
        }
        Action::Back if ui.zoomed => {
            ui.zoomed = false;
            true
        }
        _ => false,
    }
}

fn step(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

fn panel_count(page: usize) -> usize {
    match page {
        0 => 7,
        1 => 4,
        _ => 6,
    }
}

pub fn render(frame: &mut Frame, area: Rect, border_style: Style) {
    let settings = persistence::load();
    let accent = crate::settings::pages::appearance::color_from_name(&persistence::get_str(
        &settings,
        "appearance.accent_color",
        "Cyan",
    ));
    let border = crate::settings::pages::appearance::border_type_from_name(&persistence::get_str(
        &settings,
        "appearance.border_style",
        "Rounded",
    ));
    // render_main only sets a foreground colour when the container has focus.
    let focused = border_style.fg.is_some();

    let ctx = Ctx {
        snap: &metrics::snapshot(),
        online: &online::state(),
        now: chrono::Local::now(),
        accent,
        border,
        focused,
        use_24h: persistence::get_bool(&settings, "appearance.clock_format_24h", true),
        show_seconds: persistence::get_bool(&settings, "appearance.clock_show_seconds", true),
        ghost_segments: persistence::get_bool(&settings, "dashboard.clock_ghost_segments", true),
    };

    let (page, panel, zoomed) = {
        let ui = UI.lock().unwrap();
        (ui.page, ui.panel, ui.zoomed)
    };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(border)
        .border_style(border_style)
        .title_top(tab_line(page, area.width, accent, focused))
        .title_top(if area.width >= 46 {
            Line::from(Span::styled(
                format!(" {} ", LABEL),
                Style::default().add_modifier(Modifier::BOLD),
            ))
            .right_aligned()
        } else {
            Line::from("")
        })
        .title_bottom(hint_line(zoomed, area.width, focused).centered());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if inner.width < MIN_WIDTH || inner.height < MIN_HEIGHT {
        panels::too_small(frame, inner, &ctx);
        return;
    }

    let tiles = layout(page, inner);
    if tiles.is_empty() {
        panels::too_small(frame, inner, &ctx);
        return;
    }
    let selection = panel.min(tiles.len() - 1);

    if zoomed {
        draw(frame, tiles[selection].0, inner, &ctx, true);
        return;
    }
    for (index, (kind, rect)) in tiles.iter().enumerate() {
        draw(frame, *kind, *rect, &ctx, index == selection);
    }
}

fn draw(frame: &mut Frame, panel: Panel, area: Rect, ctx: &Ctx, selected: bool) {
    match panel {
        Panel::Clock => panels::clock(frame, area, ctx, selected),
        Panel::Vitals => panels::vitals(frame, area, ctx, selected),
        Panel::Cores => panels::cores(frame, area, ctx, selected),
        Panel::History => panels::history(frame, area, ctx, selected),
        Panel::Host => panels::host(frame, area, ctx, selected),
        Panel::Network => panels::network(frame, area, ctx, selected),
        Panel::Battery => panels::battery(frame, area, ctx, selected),
        Panel::Weather => panels::weather(frame, area, ctx, selected),
        Panel::Connectivity => panels::connectivity(frame, area, ctx, selected),
        Panel::TopCpu => panels::processes(frame, area, ctx, selected, false),
        Panel::TopMem => panels::processes(frame, area, ctx, selected, true),
    }
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

fn tab_line(page: usize, width: u16, accent: Color, focused: bool) -> Line<'static> {
    let active = Style::default()
        .fg(if focused { accent } else { Color::Gray })
        .add_modifier(Modifier::BOLD);
    if width < 58 {
        return Line::from(vec![
            Span::raw(" "),
            Span::styled(PAGE_NAMES[page].to_string(), active),
            Span::styled(
                format!(" {}/{} ", page + 1, PAGE_NAMES.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
    }
    let mut spans = vec![Span::raw(" ")];
    for (index, name) in PAGE_NAMES.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(if index == page {
            Span::styled((*name).to_string(), active)
        } else {
            Span::styled((*name).to_string(), Style::default().fg(Color::DarkGray))
        });
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn hint_line(zoomed: bool, width: u16, focused: bool) -> Line<'static> {
    if !focused || width < 52 {
        return Line::from("");
    }
    let text = if zoomed {
        " ⏎ / Q  back   ▲ ▼  panel "
    } else {
        " ◀ ▶  page   ▲ ▼  panel   ⏎  zoom "
    };
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Color::DarkGray),
    ))
}

// ---------------------------------------------------------------------------
// Page layouts
// ---------------------------------------------------------------------------

fn split(area: Rect, direction: Direction, constraints: &[Constraint]) -> Vec<Rect> {
    Layout::default()
        .direction(direction)
        .constraints(constraints.to_vec())
        .split(area)
        .to_vec()
}

fn rows(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    split(area, Direction::Vertical, constraints)
}

fn cols(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    split(area, Direction::Horizontal, constraints)
}

fn layout(page: usize, area: Rect) -> Vec<(Panel, Rect)> {
    match page {
        0 => overview(area),
        1 => performance(area),
        _ => system(area),
    }
}

fn overview(area: Rect) -> Vec<(Panel, Rect)> {
    if area.width < WIDE {
        let clock_h = if area.height >= 22 { 11 } else { 7 };
        let r = rows(
            area,
            &[
                Constraint::Length(clock_h),
                Constraint::Fill(1),
                Constraint::Length(7),
            ],
        );
        return vec![
            (Panel::Clock, r[0]),
            (Panel::Vitals, r[1]),
            (Panel::Network, r[2]),
        ];
    }

    if area.height < 26 {
        let r = rows(area, &[Constraint::Length(11), Constraint::Fill(1)]);
        let top = cols(r[0], &[Constraint::Percentage(62), Constraint::Percentage(38)]);
        let bottom = cols(r[1], &[Constraint::Percentage(62), Constraint::Percentage(38)]);
        let right = rows(bottom[1], &[Constraint::Fill(1), Constraint::Length(5)]);
        return vec![
            (Panel::Clock, top[0]),
            (Panel::Host, top[1]),
            (Panel::Vitals, bottom[0]),
            (Panel::Weather, right[0]),
            (Panel::Battery, right[1]),
        ];
    }

    let r = rows(
        area,
        &[
            Constraint::Length(11),
            Constraint::Fill(1),
            Constraint::Length(10),
        ],
    );
    let top = cols(r[0], &[Constraint::Percentage(62), Constraint::Percentage(38)]);
    let mid = cols(r[1], &[Constraint::Percentage(62), Constraint::Percentage(38)]);
    let bottom = cols(r[2], &[Constraint::Percentage(62), Constraint::Percentage(38)]);
    let mid_right = rows(mid[1], &[Constraint::Fill(1), Constraint::Length(7)]);
    vec![
        (Panel::Clock, top[0]),
        (Panel::Host, top[1]),
        (Panel::Vitals, mid[0]),
        (Panel::Weather, mid_right[0]),
        (Panel::Battery, mid_right[1]),
        (Panel::TopCpu, bottom[0]),
        (Panel::Network, bottom[1]),
    ]
}

fn performance(area: Rect) -> Vec<(Panel, Rect)> {
    if area.width < WIDE {
        let r = rows(
            area,
            &[Constraint::Fill(2), Constraint::Fill(2), Constraint::Fill(3)],
        );
        return vec![
            (Panel::Cores, r[0]),
            (Panel::History, r[1]),
            (Panel::TopCpu, r[2]),
        ];
    }
    let r = rows(area, &[Constraint::Fill(3), Constraint::Fill(3)]);
    let top = cols(r[0], &[Constraint::Percentage(55), Constraint::Percentage(45)]);
    let bottom = cols(r[1], &[Constraint::Percentage(50), Constraint::Percentage(50)]);
    vec![
        (Panel::Cores, top[0]),
        (Panel::History, top[1]),
        (Panel::TopCpu, bottom[0]),
        (Panel::TopMem, bottom[1]),
    ]
}

fn system(area: Rect) -> Vec<(Panel, Rect)> {
    if area.width < WIDE {
        let r = rows(
            area,
            &[Constraint::Fill(3), Constraint::Fill(2), Constraint::Fill(2)],
        );
        return vec![
            (Panel::Host, r[0]),
            (Panel::Network, r[1]),
            (Panel::Connectivity, r[2]),
        ];
    }
    let r = rows(area, &[Constraint::Fill(3), Constraint::Fill(2)]);
    let top = cols(r[0], &[Constraint::Percentage(50), Constraint::Percentage(50)]);
    let bottom = cols(
        r[1],
        &[
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ],
    );
    vec![
        (Panel::Host, top[0]),
        (Panel::Vitals, top[1]),
        (Panel::Network, bottom[0]),
        (Panel::Battery, bottom[1]),
        (Panel::Connectivity, bottom[2]),
    ]
}
