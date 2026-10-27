/// Performance tab: every live graph on one screen — CPU, per-core, memory, GPU,
/// GPU memory, temperatures and a network summary. Arrows pick a panel, Enter zooms it.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use super::graphs::{self, Series};
use super::metrics::{self, Metrics};
use super::state::{Focus, TaskManagerState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Cpu,
    Cores,
    Memory,
    Gpu,
    Vram,
    Temps,
    Network,
}

pub const PANELS: [Panel; 7] = [
    Panel::Cpu,
    Panel::Cores,
    Panel::Memory,
    Panel::Gpu,
    Panel::Vram,
    Panel::Temps,
    Panel::Network,
];

const GPU_COLORS: [Color; 4] = [Color::Magenta, Color::LightBlue, Color::LightGreen, Color::LightRed];

/// Panels per row for a given width.
pub fn columns(width: u16) -> usize {
    if width >= 120 {
        3
    } else if width >= 70 {
        2
    } else {
        1
    }
}

/// Too short for a grid of bordered graphs: one line per panel instead.
fn compact(body: Rect) -> bool {
    let rows = PANELS.len().div_ceil(columns(body.width));
    (body.height as usize) < rows * 6
}

/// Panels per row as the arrow keys should see them for this content area.
pub fn layout_columns(content: Rect) -> usize {
    // Same split as `render`: the summary line, plus the Pi line when there is one
    let top = 1 + metrics::read().pi.is_some() as u16;
    let body = Rect { height: content.height.saturating_sub(top), ..content };
    if compact(body) { 1 } else { columns(content.width) }
}

pub fn render(frame: &mut Frame, area: Rect, tm: &TaskManagerState, accent: Color, line: bool, btype: BorderType) {
    let m = metrics::read();
    if !m.ready {
        frame.render_widget(
            Paragraph::new("  Collecting the first samples…").style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let mut constraints = vec![Constraint::Length(1)];
    if m.pi.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Fill(1));
    let rows = Layout::default().direction(Direction::Vertical).constraints(constraints).split(area);
    frame.render_widget(Paragraph::new(summary_line(&m)), rows[0]);
    if m.pi.is_some() {
        frame.render_widget(Paragraph::new(pi_line(&m)), rows[1]);
    }
    let body = rows[rows.len() - 1];

    let focused = tm.focus == Focus::Content;
    let ctx = Ctx { m: &m, window: tm.window_secs(), line, accent, btype };
    if tm.perf_zoom {
        let panel = PANELS[tm.perf_cursor.min(PANELS.len() - 1)];
        render_panel(frame, body, panel, &ctx, focused);
        return;
    }
    if compact(body) {
        render_compact(frame, body, tm, &ctx, focused);
        return;
    }

    let cols = columns(body.width);
    let grid_rows = PANELS.len().div_ceil(cols);
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, grid_rows as u32); grid_rows])
        .split(body);
    for (r, row_rect) in row_rects.iter().enumerate() {
        let in_row = (PANELS.len() - r * cols).min(cols);
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, in_row as u32); in_row])
            .split(*row_rect);
        for (c, cell) in cells.iter().enumerate() {
            let i = r * cols + c;
            render_panel(frame, *cell, PANELS[i], &ctx, focused && tm.perf_cursor == i);
        }
    }
}

struct Ctx<'a> {
    m: &'a Metrics,
    window: u64,
    line: bool,
    accent: Color,
    btype: BorderType,
}

fn summary_line(m: &Metrics) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::White);
    let cores = match m.physical_cores {
        Some(p) => format!("{} cores / {} threads", p, m.cores.len()),
        None => format!("{} threads", m.cores.len()),
    };
    let uptime = graphs::duration(std::time::Duration::from_secs(m.uptime));
    let mut spans = vec![
        Span::styled(" ", dim),
        Span::styled(m.cpu_brand.clone(), val),
        Span::styled(format!(" · {}", cores), dim),
        Span::styled(" · Up ", dim),
        Span::styled(uptime, val),
    ];
    if m.process_count > 0 {
        spans.push(Span::styled(" · Processes ", dim));
        spans.push(Span::styled(m.process_count.to_string(), val));
    }
    Line::from(spans)
}

fn pi_line(m: &Metrics) -> Line<'static> {
    let Some(pi) = m.pi else { return Line::from("") };
    let problems = pi.active_problems();
    let mut spans = vec![Span::styled(" Raspberry Pi power: ", Style::default().fg(Color::DarkGray))];
    if problems.is_empty() {
        spans.push(Span::styled("OK", Style::default().fg(Color::Green)));
    } else {
        spans.push(Span::styled(problems.join(", "), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
    }
    let mut past = Vec::new();
    if pi.under_voltage_occurred() && !pi.under_voltage() {
        past.push("under-voltage");
    }
    if pi.throttled_occurred() && !pi.throttled() {
        past.push("throttling");
    }
    if !past.is_empty() {
        spans.push(Span::styled(
            format!("  (since boot: {})", past.join(", ")),
            Style::default().fg(Color::Yellow),
        ));
    }
    Line::from(spans)
}

fn panel_block(title: String, focused: bool, ctx: &Ctx) -> Block<'static> {
    let style = if focused {
        Style::default().fg(ctx.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(ctx.btype)
        .border_style(style)
        .title(Span::styled(title, style))
}

fn note(frame: &mut Frame, area: Rect, text: &str) {
    frame.render_widget(
        Paragraph::new(format!(" {}", text)).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn render_panel(frame: &mut Frame, area: Rect, panel: Panel, ctx: &Ctx, focused: bool) {
    let block = panel_block(panel_title(panel, ctx.m), focused, ctx);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    panel_body(frame, inner, panel, ctx);
}

fn panel_title(panel: Panel, m: &Metrics) -> String {
    match panel {
        Panel::Cpu => {
            let now = m.cpu_now() as f64;
            let ghz = m.core_freq.iter().copied().max().unwrap_or(0) as f64 / 1000.0;
            // Apple Silicon reports no usable frequency
            if ghz >= 0.1 {
                format!(" CPU {} · {:.1} GHz ", graphs::pct(now), ghz)
            } else {
                format!(" CPU {} ", graphs::pct(now))
            }
        }
        Panel::Cores => format!(" Per Core ({}) ", m.cores.len()),
        Panel::Memory => format!(
            " Memory {} · {} / {} ",
            graphs::pct(m.mem_pct() as f64),
            graphs::bytes(m.mem_now()),
            graphs::bytes(m.mem_total)
        ),
        Panel::Gpu => match (m.gpus.len(), m.gpu_now()) {
            (0, _) => " GPU ".to_string(),
            (1, Some(now)) => format!(" GPU {} · {} ", graphs::pct(now as f64), m.gpus[0].name),
            (1, None) => format!(" GPU · {} ", m.gpus[0].name),
            (count, Some(now)) => format!(" GPU {} · {} GPUs ", graphs::pct(now as f64), count),
            (count, None) => format!(" GPU · {} GPUs ", count),
        },
        Panel::Vram => {
            let (used, total, any) = vram_totals(m);
            if !any {
                " GPU Memory ".to_string()
            } else if total > 0 {
                format!(" GPU Memory {} / {} ", graphs::bytes(used), graphs::bytes(total))
            } else {
                format!(" GPU Memory {} (shared) ", graphs::bytes(used))
            }
        }
        Panel::Temps => {
            let mut parts = Vec::new();
            if let Some(c) = m.cpu_temp.last().copied().flatten() {
                parts.push(format!("CPU {:.0} °C", c));
            }
            if let Some(g) = m.gpu_temp.last().copied().flatten() {
                parts.push(format!("GPU {:.0} °C", g));
            }
            if parts.is_empty() {
                " Temperatures ".to_string()
            } else {
                format!(" Temperatures {} ", parts.join(" · "))
            }
        }
        Panel::Network => {
            let rx = m.net_rx.last().copied().unwrap_or(0.0);
            let tx = m.net_tx.last().copied().unwrap_or(0.0);
            format!(" Network ↓ {} ↑ {} ", graphs::rate(rx), graphs::rate(tx))
        }
    }
}

/// (used, total, whether any GPU reports memory)
fn vram_totals(m: &Metrics) -> (u64, u64, bool) {
    let mut used = 0;
    let mut total = 0;
    let mut any = false;
    for g in &m.gpus {
        if let Some(u) = g.vram.last().copied().flatten() {
            any = true;
            used += u;
            total += g.vram_total.unwrap_or(0);
        }
    }
    (used, total, any)
}

fn panel_body(frame: &mut Frame, area: Rect, panel: Panel, ctx: &Ctx) {
    let m = ctx.m;
    let n = m.samples_for(ctx.window);
    match panel {
        Panel::Cpu => {
            let series = vec![Series {
                name: "CPU".into(),
                color: ctx.accent,
                values: graphs::window(&m.cpu, n, |v| Some(v as f64)),
            }];
            graphs::draw(frame, area, &series, 100.0, ctx.line, ctx.window, &graphs::pct);
        }
        Panel::Cores => render_cores(frame, area, ctx),
        Panel::Memory => {
            let series = vec![Series {
                name: "RAM".into(),
                color: Color::Yellow,
                values: graphs::window(&m.mem_used, n, |v| Some(v as f64)),
            }];
            let mut graph_area = area;
            if m.swap_total > 0 && area.height >= 4 {
                graph_area.height -= 1;
                note(
                    frame,
                    Rect { y: area.y + area.height - 1, height: 1, ..area },
                    &format!("Swap {} / {}", graphs::bytes(m.swap_used), graphs::bytes(m.swap_total)),
                );
            }
            graphs::draw(frame, graph_area, &series, m.mem_total as f64, ctx.line, ctx.window, &|v| graphs::bytes(v as u64));
        }
        Panel::Gpu => {
            let series: Vec<Series> = m
                .gpus
                .iter()
                .enumerate()
                .filter(|(_, g)| g.util.tail(n).any(|v| v.is_some()))
                .map(|(i, g)| Series {
                    name: g.name.clone(),
                    color: GPU_COLORS[i % GPU_COLORS.len()],
                    values: graphs::window(&g.util, n, |v| v.map(|x| x as f64)),
                })
                .collect();
            if series.is_empty() {
                note(frame, area, &gpu_note(m));
            } else {
                draw_with_legend(frame, area, &series, 100.0, ctx, &graphs::pct);
            }
        }
        Panel::Vram => {
            let series: Vec<Series> = m
                .gpus
                .iter()
                .enumerate()
                .filter(|(_, g)| g.vram.tail(n).any(|v| v.is_some()))
                .map(|(i, g)| Series {
                    name: g.name.clone(),
                    color: GPU_COLORS[i % GPU_COLORS.len()],
                    values: graphs::window(&g.vram, n, |v| v.map(|x| x as f64)),
                })
                .collect();
            let (_, total, _) = vram_totals(m);
            let top = if total > 0 { total as f64 } else { graphs::peak(&series) * 1.25 };
            if series.is_empty() {
                let text = if m.gpus.is_empty() { gpu_note(m) } else { "This GPU does not report its memory use".into() };
                note(frame, area, &text);
            } else {
                draw_with_legend(frame, area, &series, top, ctx, &|v| graphs::bytes(v as u64));
            }
        }
        Panel::Temps => {
            let mut series = Vec::new();
            if m.cpu_temp.tail(n).any(|v| v.is_some()) {
                series.push(Series {
                    name: "CPU".into(),
                    color: Color::LightRed,
                    values: graphs::window(&m.cpu_temp, n, |v| v.map(|x| x as f64)),
                });
            }
            if m.gpu_temp.tail(n).any(|v| v.is_some()) {
                series.push(Series {
                    name: "GPU".into(),
                    color: Color::LightMagenta,
                    values: graphs::window(&m.gpu_temp, n, |v| v.map(|x| x as f64)),
                });
            }
            if series.is_empty() {
                note(frame, area, "No temperature sensors reported (Windows needs admin rights for them)");
            } else {
                let top = (graphs::peak(&series) + 10.0).max(100.0);
                draw_with_legend(frame, area, &series, top, ctx, &|v| format!("{:.0}°", v));
            }
        }
        Panel::Network => {
            let series = vec![
                Series { name: "Download".into(), color: Color::Green, values: graphs::window(&m.net_rx, n, Some) },
                Series { name: "Upload".into(), color: Color::LightBlue, values: graphs::window(&m.net_tx, n, Some) },
            ];
            let top = (graphs::peak(&series) * 1.2).max(1024.0);
            draw_with_legend(frame, area, &series, top, ctx, &graphs::rate);
        }
    }
}

/// Small screens: one line (or a few) per panel — name, current value, then history.
/// Enter still zooms the highlighted panel to the full graph.
fn render_compact(frame: &mut Frame, area: Rect, tm: &TaskManagerState, ctx: &Ctx, focused: bool) {
    let m = ctx.m;
    let n = m.samples_for(ctx.window);
    let rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, PANELS.len() as u32); PANELS.len()])
        .split(area);
    let label_w = 28u16.min(area.width);
    for (i, (panel, rect)) in PANELS.iter().zip(rects.iter()).enumerate() {
        if rect.height == 0 {
            continue;
        }
        let selected = focused && tm.perf_cursor == i;
        let (name, value, series, max): (&str, String, Vec<Series>, f64) = match panel {
            Panel::Cpu => (
                "CPU",
                graphs::pct(m.cpu_now() as f64),
                vec![Series { name: String::new(), color: ctx.accent, values: graphs::window(&m.cpu, n, |v| Some(v as f64)) }],
                100.0,
            ),
            Panel::Cores => {
                let cores: Vec<String> = m.cores.iter().map(|c| format!("{:.0}", c.last().copied().unwrap_or(0.0))).collect();
                ("Cores", cores.join(" "), Vec::new(), 100.0)
            }
            Panel::Memory => (
                "Memory",
                format!("{} {}", graphs::pct(m.mem_pct() as f64), graphs::bytes(m.mem_now())),
                vec![Series { name: String::new(), color: Color::Yellow, values: graphs::window(&m.mem_used, n, |v| Some(v as f64)) }],
                m.mem_total as f64,
            ),
            Panel::Gpu => {
                let series: Vec<Series> = m
                    .gpus
                    .iter()
                    .enumerate()
                    .map(|(i, g)| Series {
                        name: String::new(),
                        color: GPU_COLORS[i % GPU_COLORS.len()],
                        values: graphs::window(&g.util, n, |v| v.map(|x| x as f64)),
                    })
                    .take(1)
                    .collect();
                let value = m.gpu_now().map(|v| graphs::pct(v as f64)).unwrap_or_else(|| "N/A".into());
                ("GPU", value, series, 100.0)
            }
            Panel::Vram => {
                let (used, total, any) = vram_totals(m);
                let value = if !any {
                    "N/A".into()
                } else if total > 0 {
                    format!("{} / {}", graphs::bytes(used), graphs::bytes(total))
                } else {
                    graphs::bytes(used)
                };
                let series: Vec<Series> = m
                    .gpus
                    .iter()
                    .filter(|g| g.vram.last().map_or(false, |v| v.is_some()))
                    .take(1)
                    .map(|g| Series { name: String::new(), color: Color::Magenta, values: graphs::window(&g.vram, n, |v| v.map(|x| x as f64)) })
                    .collect();
                let top = if total > 0 { total as f64 } else { graphs::peak(&series) * 1.25 };
                ("GPU Mem", value, series, top)
            }
            Panel::Temps => {
                let cpu = m.cpu_temp.last().copied().flatten();
                let gpu = m.gpu_temp.last().copied().flatten();
                let value = match (cpu, gpu) {
                    (Some(c), Some(g)) => format!("CPU {:.0}° GPU {:.0}°", c, g),
                    (Some(c), None) => format!("CPU {:.0} °C", c),
                    (None, Some(g)) => format!("GPU {:.0} °C", g),
                    (None, None) => "N/A".into(),
                };
                let series = if cpu.is_some() {
                    vec![Series { name: String::new(), color: Color::LightRed, values: graphs::window(&m.cpu_temp, n, |v| v.map(|x| x as f64)) }]
                } else {
                    Vec::new()
                };
                ("Temp", value, series, 100.0)
            }
            Panel::Network => {
                let rx = m.net_rx.last().copied().unwrap_or(0.0);
                let tx = m.net_tx.last().copied().unwrap_or(0.0);
                let series = vec![Series { name: String::new(), color: Color::Green, values: graphs::window(&m.net_rx, n, Some) }];
                let top = (graphs::peak(&series) * 1.2).max(1024.0);
                ("Network", format!("↓{} ↑{}", graphs::rate(rx), graphs::rate(tx)), series, top)
            }
        };
        let style = if selected {
            Style::default().fg(Color::Black).bg(ctx.accent)
        } else {
            Style::default().fg(Color::White)
        };
        // Rows without a graph (per-core figures) may use the whole width
        let width = if series.is_empty() { rect.width } else { label_w };
        let text = format!(" {:<8}{}", name, value);
        let text: String = text.chars().take(width as usize).collect();
        let label_rect = Rect { width, height: 1, ..*rect };
        frame.render_widget(Paragraph::new(format!("{:<w$}", text, w = width as usize)).style(style), label_rect);
        let graph = Rect { x: rect.x + label_w + 1, width: rect.width.saturating_sub(label_w + 1), ..*rect };
        if !series.is_empty() && graph.width > 4 {
            graphs::draw(frame, graph, &series, max, ctx.line && graph.height >= 3, ctx.window, &graphs::pct);
        }
    }
}

fn gpu_note(m: &Metrics) -> String {
    if m.gpus.is_empty() {
        "No GPU statistics available on this system".into()
    } else {
        "This GPU does not report its utilisation".into()
    }
}

/// Graph plus a one-line legend when there is more than one series.
fn draw_with_legend(frame: &mut Frame, area: Rect, series: &[Series], max: f64, ctx: &Ctx, fmt: &dyn Fn(f64) -> String) {
    if series.len() > 1 && area.height >= 4 {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(area);
        graphs::draw(frame, parts[0], series, max, ctx.line, ctx.window, fmt);
        let names: Vec<(String, Color)> = series.iter().map(|s| (s.name.clone(), s.color)).collect();
        graphs::text(frame, parts[1], graphs::legend(&names));
    } else {
        graphs::draw(frame, area, series, max, ctx.line, ctx.window, fmt);
    }
}

/// A grid of small per-core graphs: "C3  42%" then the history.
fn render_cores(frame: &mut Frame, area: Rect, ctx: &Ctx) {
    let m = ctx.m;
    let count = m.cores.len();
    if count == 0 || area.width < 10 || area.height == 0 {
        return;
    }
    let n = m.samples_for(ctx.window);
    let cols = ((area.width / 18).max(1) as usize).min(count);
    let rows_needed = count.div_ceil(cols);
    // Taller cells get a braille line; one-row cells fall back to sparklines
    let cell_h = if ctx.line && rows_needed * 3 <= area.height as usize { 3 } else { 1 };
    let fit_rows = (area.height as usize / cell_h).max(1);
    let shown = (fit_rows * cols).min(count);
    let cell_w = area.width / cols as u16;

    for i in 0..shown {
        let (r, c) = (i / cols, i % cols);
        let rect = Rect {
            x: area.x + c as u16 * cell_w,
            y: area.y + (r * cell_h) as u16,
            width: cell_w.saturating_sub(1),
            height: cell_h as u16,
        };
        let now = m.cores[i].last().copied().unwrap_or(0.0) as f64;
        let label = format!("C{:<2}{:>4} ", i, graphs::pct(now));
        let label_w = (label.chars().count() as u16).min(rect.width);
        frame.render_widget(
            Paragraph::new(Span::styled(label, Style::default().fg(graphs::heat(now)))),
            Rect { height: 1, width: label_w, ..rect },
        );
        let graph = Rect { x: rect.x + label_w, width: rect.width.saturating_sub(label_w), ..rect };
        let series = vec![Series {
            name: String::new(),
            color: ctx.accent,
            values: graphs::window(&m.cores[i], n, |v| Some(v as f64)),
        }];
        graphs::draw(frame, graph, &series, 100.0, cell_h >= 3, ctx.window, &graphs::pct);
    }
    if shown < count {
        let more = format!(" +{} more — Enter to zoom ", count - shown);
        let w = more.chars().count() as u16;
        if area.width > w {
            frame.render_widget(
                Paragraph::new(more).style(Style::default().fg(Color::DarkGray)),
                Rect { x: area.x + area.width - w, y: area.y + area.height - 1, width: w, height: 1 },
            );
        }
    }
}
