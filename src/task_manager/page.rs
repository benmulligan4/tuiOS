/// Task Manager page — rendered in the tuiOS main container.
///
/// Tabs: Apps (running sessions with details, live preview and actions), Performance
/// (all live graphs), Network (traffic, latency, Wi-Fi) and Startup (apps launched
/// in the background at boot).

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Sparkline},
    Frame,
};

use crate::tuios::models::TuiosState;
use crate::tuios::sessions::{Body, Kind, Phase, Session, SessionId, Sessions};

use super::graphs;
use super::metrics;
use super::state::{self, actions_for, Confirm, Focus, RowId, Tab, TaskManagerState};
use super::{config, net, netinfo, perf};

pub struct Look {
    pub border_style: Style,
    pub border_type: BorderType,
    pub accent: Color,
    pub focused: bool,
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut TuiosState, sessions: &mut Sessions, look: Look) {
    // Keeps the latency / Wi-Fi / public IP worker running while the page is open
    netinfo::touch();
    let line = state.task_manager.line_graphs(config::load().graph_line);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(look.border_type)
        .title(" Task Manager ")
        .title_alignment(Alignment::Right)
        .style(look.border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)])
        .split(inner);
    let content = rows[1];
    render_tabs(frame, rows[0], &state.task_manager, sessions, &look);

    if let Some(id) = state.task_manager.viewer {
        render_viewer(frame, content, sessions, id, &look);
    } else {
        let tab = state.task_manager.tab;
        match tab {
            Tab::Apps => render_apps(frame, content, state, sessions, &look),
            Tab::Performance => {
                state.task_manager.perf_cols = perf::layout_columns(content);
                perf::render(frame, content, &state.task_manager, look.accent, line, look.border_type)
            }
            Tab::Network => net::render(frame, content, &state.task_manager, look.accent, line, look.border_type),
            Tab::Startup => render_startup(frame, content, &state.task_manager, &look),
        }
    }
    render_footer(frame, rows[2], &state.task_manager, line);

    if state.task_manager.confirm.is_some() {
        render_confirm(frame, content, &state.task_manager, sessions, &state.active_dashboard);
    }
}

// ── Tabs and footer ─────────────────────────────────────────────────────────

fn render_tabs(frame: &mut Frame, area: Rect, tm: &TaskManagerState, sessions: &Sessions, look: &Look) {
    let live = sessions.live_count();
    let mut spans = vec![Span::raw(" ")];
    for tab in Tab::ALL {
        let label = match tab {
            Tab::Apps if live > 0 => format!(" Apps ({}) ", live),
            _ => format!(" {} ", tab.label()),
        };
        let style = if tab == tm.tab && tm.focus == Focus::Tabs && look.focused {
            Style::default().fg(Color::Black).bg(look.accent).add_modifier(Modifier::BOLD)
        } else if tab == tm.tab {
            Style::default().fg(look.accent).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }
    spans.pop();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // A one-line health summary on the right, visible from every tab
    let m = metrics::read();
    let mut summary = format!("CPU {} · RAM {}", graphs::pct(m.cpu_now() as f64), graphs::pct(m.mem_pct() as f64));
    if let Some(gpu) = m.gpu_now() {
        summary = format!("{} · GPU {}", summary, graphs::pct(gpu as f64));
    }
    let width = summary.chars().count() as u16 + 1;
    if area.width > width + 50 {
        frame.render_widget(
            Paragraph::new(summary).style(Style::default().fg(Color::DarkGray)),
            Rect { x: area.x + area.width - width, width, ..area },
        );
    }
}

fn render_footer(frame: &mut Frame, area: Rect, tm: &TaskManagerState, line: bool) {
    let style_name = if line { "Line" } else { "Sparkline" };
    let window = graphs::short_duration(tm.window_secs());
    let hint = if tm.viewer.is_some() {
        "↑↓ scroll · PgUp/PgDn page · Home/End · q close".to_string()
    } else if tm.focus == Focus::Tabs {
        "←→ switch tab · ↓ enter tab · q close Task Manager".to_string()
    } else {
        match (tm.tab, tm.focus) {
            (Tab::Apps, Focus::Actions) => "↑↓ choose · Enter run · ← back".to_string(),
            (Tab::Apps, _) => "↑↓ select · Enter actions · o sort · f filter · x end all · [ ] tabs".to_string(),
            (Tab::Performance, _) => format!(
                "arrows panel · Enter {} · g graph: {} · +/- window: {} · [ ] tabs",
                if tm.perf_zoom { "unzoom" } else { "zoom" },
                style_name,
                window
            ),
            (Tab::Network, _) => format!(
                "↑↓ interface · g graph: {} · +/- window: {} · r public IP · [ ] tabs",
                style_name, window
            ),
            (Tab::Startup, _) => "↑↓ select · Enter toggle launch at startup · [ ] tabs".to_string(),
        }
    };
    frame.render_widget(
        Paragraph::new(format!(" {}", hint)).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

// ── Apps tab ─────────────────────────────────────────────────────────────────

fn status_style(s: &Session, shown: bool) -> (String, Color) {
    match &s.phase {
        Phase::Exited { code: Some(0), .. } => ("✓".into(), Color::DarkGray),
        Phase::Exited { .. } => ("✗".into(), Color::Red),
        Phase::Stopping { .. } => ("…".into(), Color::Yellow),
        Phase::Suspended => ("‖".into(), Color::Yellow),
        Phase::Running if s.is_external() => ("↗".into(), Color::LightBlue),
        Phase::Running if shown => ("●".into(), Color::Green),
        Phase::Running => ("●".into(), Color::Cyan),
    }
}

/// Is this session the one on screen (foreground app, or the home dashboard)?
fn is_shown(s: &Session, sessions: &Sessions, home: &str) -> bool {
    match s.kind {
        Kind::App => sessions.foreground == Some(s.id),
        Kind::Dashboard => s.key == home,
    }
}

fn fit(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        format!("{:<width$}", text, width = width)
    } else if width > 1 {
        let mut out: String = text.chars().take(width - 1).collect();
        out.push('…');
        out
    } else {
        String::new()
    }
}

fn render_apps(frame: &mut Frame, area: Rect, state: &mut TuiosState, sessions: &mut Sessions, look: &Look) {
    let home = state.active_dashboard.clone();
    let tm = &mut state.task_manager;
    let (rows, sel) = {
        let m = metrics::read();
        let rows = state::rows(sessions, tm, &m);
        let sel = state::selected_index(&rows, tm.selected, tm.last_index);
        (rows, sel)
    };
    tm.selected = rows[sel];
    tm.last_index = sel;
    if tm.focus == Focus::Actions && !matches!(tm.selected, RowId::Session(_)) {
        tm.focus = Focus::Content;
    }

    let wide = area.width >= 96;
    let panes = if wide {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Percentage(46)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Fill(1)])
            .split(area)
    };

    render_list(frame, panes[0], tm, sessions, &rows, sel, &home, look);
    render_details(frame, panes[1], tm, sessions, &home, look);
}

#[allow(clippy::too_many_arguments)]
fn render_list(
    frame: &mut Frame,
    area: Rect,
    tm: &mut TaskManagerState,
    sessions: &Sessions,
    rows: &[RowId],
    sel: usize,
    home: &str,
    look: &Look,
) {
    let focused = look.focused && tm.focus == Focus::Content;
    let border = if focused { Style::default().fg(look.accent) } else { Style::default().fg(Color::DarkGray) };
    let title = format!(" Running · sort {} · {} ", tm.sort.label(), tm.filter.label());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(look.border_type)
        .border_style(border)
        .title(Span::styled(title, border.add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }

    // Columns shrink away from the right as the pane narrows
    let w = inner.width as usize;
    let show_type = w >= 58;
    let show_uptime = w >= 68;
    let fixed = 3 + 12 + 7 + 10 + if show_type { 11 } else { 0 } + if show_uptime { 9 } else { 0 };
    let name_w = w.saturating_sub(fixed).max(8);

    let mut header = format!("   {}", fit("Name", name_w));
    if show_type {
        header.push_str(&fit(" Type", 11));
    }
    header.push_str(&fit(" Status", 12));
    header.push_str(&format!("{:>7}{:>10}", "CPU", "Memory"));
    if show_uptime {
        header.push_str(&format!("{:>9}", "Uptime"));
    }
    let mut lines = vec![Line::from(Span::styled(header, Style::default().fg(Color::DarkGray)))];

    let visible = inner.height as usize - 1;
    tm.list_height = visible;
    let scroll = sel.saturating_sub(visible.saturating_sub(1));
    let m = metrics::read();

    for (i, row) in rows.iter().enumerate().skip(scroll).take(visible) {
        let selected = i == sel;
        let line = match row {
            RowId::Shell => {
                let usage = m.procs.get(&m.self_pid).copied();
                let mut text = format!(" ◆ {}", fit("tuiOS", name_w));
                if show_type {
                    text.push_str(&fit(" Shell", 11));
                }
                text.push_str(&fit(" Running", 12));
                text.push_str(&format!(
                    "{:>7}{:>10}",
                    usage.map(|u| format!("{:.1}%", u.cpu)).unwrap_or_else(|| "—".into()),
                    usage.map(|u| graphs::bytes(u.mem)).unwrap_or_else(|| "—".into())
                ));
                if show_uptime {
                    text.push_str(&format!("{:>9}", graphs::duration(metrics::tuios_uptime())));
                }
                let style = row_style(selected, focused, look, Color::Magenta);
                Line::from(Span::styled(text, style))
            }
            RowId::EndAll => {
                let style = if selected && focused {
                    Style::default().fg(Color::Black).bg(Color::Red)
                } else {
                    Style::default().fg(Color::Red)
                };
                Line::from(Span::styled(" ✗ End All Background Apps", style))
            }
            RowId::Session(id) => {
                let Some(s) = sessions.get(*id) else { continue };
                let shown = is_shown(s, sessions, home);
                let (glyph, color) = status_style(s, shown);
                let usage = state::usage(s, &m);
                let mut text = fit(&s.label, name_w);
                if show_type {
                    let kind = if s.kind == Kind::Dashboard { "Dashboard" } else { s.mode_label() };
                    text.push_str(&fit(&format!(" {}", kind), 11));
                }
                text.push_str(&fit(&format!(" {}", s.status_label(shown)), 12));
                let (cpu, mem) = match (usage, &s.body) {
                    (Some(u), _) => (format!("{:.1}%", u.0), graphs::bytes(u.1)),
                    (None, Body::Internal(_)) => ("—".into(), "in tuiOS".into()),
                    _ => ("—".into(), "—".into()),
                };
                text.push_str(&format!("{:>7}{:>10}", cpu, mem));
                if show_uptime {
                    let up = if s.is_live() { graphs::duration(s.uptime()) } else { "—".into() };
                    text.push_str(&format!("{:>9}", up));
                }
                let base = row_style(selected, focused, look, Color::White);
                let glyph_style = if selected && focused { base } else { Style::default().fg(color) };
                Line::from(vec![Span::styled(format!(" {} ", glyph), glyph_style), Span::styled(text, base)])
            }
        };
        lines.push(line);
    }

    if rows.len() == 1 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "   No apps running. Anything you open keeps running here",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "   in the background until you close it.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn row_style(selected: bool, focused: bool, look: &Look, normal: Color) -> Style {
    if selected && focused {
        Style::default().fg(Color::Black).bg(look.accent)
    } else if selected {
        Style::default().fg(look.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(normal)
    }
}

fn kv(key: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<10}", key), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

/// "CPU ▁▂▃▅ 4.2%" style history row.
fn spark_row(frame: &mut Frame, area: Rect, label: &str, data: Vec<u64>, max: u64, value: String, color: Color) {
    if area.width < 20 || area.height == 0 {
        return;
    }
    let value_w = value.chars().count() as u16 + 1;
    frame.render_widget(
        Paragraph::new(Span::styled(format!(" {:<6}", label), Style::default().fg(Color::DarkGray))),
        Rect { width: 7, ..area },
    );
    let spark = Rect { x: area.x + 7, width: area.width.saturating_sub(7 + value_w), ..area };
    let tail: Vec<u64> = data.iter().rev().take(spark.width as usize).rev().copied().collect();
    frame.render_widget(Sparkline::default().data(&tail).max(max.max(1)).style(Style::default().fg(color)), spark);
    frame.render_widget(
        Paragraph::new(value).alignment(Alignment::Right),
        Rect { x: area.x + area.width - value_w, width: value_w, ..area },
    );
}

fn render_details(
    frame: &mut Frame,
    area: Rect,
    tm: &mut TaskManagerState,
    sessions: &mut Sessions,
    home: &str,
    look: &Look,
) {
    let focused = look.focused && tm.focus == Focus::Actions;
    let border = if focused { Style::default().fg(look.accent) } else { Style::default().fg(Color::DarkGray) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(look.border_type)
        .border_style(border)
        .title(Span::styled(" Details ", border.add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 3 {
        return;
    }

    match tm.selected {
        RowId::Shell => render_shell_details(frame, inner, sessions, home),
        RowId::EndAll => {
            let count = end_all_targets(sessions, home).len();
            let lines = vec![
                Line::from(Span::styled(" End All Background Apps", Style::default().add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(Span::styled(
                    format!(" Quits the {} app{} running in the background,", count, if count == 1 { "" } else { "s" }),
                    Style::default().fg(Color::Gray),
                )),
                Line::from(Span::styled(
                    " including ones in their own windows. The dashboard on",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(Span::styled(" the home screen keeps running.", Style::default().fg(Color::Gray))),
                Line::from(""),
                Line::from(Span::styled(" Press Enter (or x) to choose.", Style::default().fg(Color::DarkGray))),
            ];
            frame.render_widget(Paragraph::new(lines), inner);
        }
        RowId::Session(id) => render_session_details(frame, inner, tm, sessions, id, home, look),
    }
}

fn render_shell_details(frame: &mut Frame, area: Rect, sessions: &Sessions, home: &str) {
    let m = metrics::read();
    let usage = m.procs.get(&m.self_pid).copied();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" tuiOS", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("  shell", Style::default().fg(Color::Magenta)),
        ]),
        kv("PID", m.self_pid.to_string()),
        kv("Uptime", graphs::duration(metrics::tuios_uptime())),
        kv("Home", home.to_string()),
        kv(
            "Sessions",
            format!("{} running · {} in own windows", sessions.live_count(), sessions.iter().filter(|s| s.is_external() && s.is_live()).count()),
        ),
        kv("Built-ins", "Built-in apps run inside this process".into()),
    ];
    if usage.is_none() {
        lines.push(kv("Usage", "Measuring…".into()));
    }
    let text_h = lines.len() as u16;
    frame.render_widget(Paragraph::new(lines), area);
    if let Some(h) = m.proc_hist.get(&m.self_pid) {
        let y = area.y + text_h + 1;
        if y + 2 <= area.y + area.height {
            let cpu: Vec<u64> = h.cpu.tail(400).map(|v| (*v * 10.0) as u64).collect();
            let mem: Vec<u64> = h.mem.tail(400).copied().collect();
            let peak = mem.iter().copied().max().unwrap_or(1);
            let now = usage.unwrap_or_default();
            spark_row(frame, Rect { y, height: 1, ..area }, "CPU", cpu, 1000, format!("{:.1}%", now.cpu), Color::Cyan);
            spark_row(frame, Rect { y: y + 1, height: 1, ..area }, "Memory", mem, peak, graphs::bytes(now.mem), Color::Yellow);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_session_details(
    frame: &mut Frame,
    area: Rect,
    tm: &mut TaskManagerState,
    sessions: &mut Sessions,
    id: SessionId,
    home: &str,
    look: &Look,
) {
    let Some(s) = sessions.get(id) else { return };
    let shown = is_shown(s, sessions, home);
    let actions = actions_for(s);
    tm.action_cursor = tm.action_cursor.min(actions.len().saturating_sub(1));

    let kind = match (s.kind, s.mode_label()) {
        (Kind::Dashboard, mode) => format!("Dashboard · {}", mode),
        (Kind::App, mode) => format!("App · {}", mode),
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!(" {}", s.label), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", kind), Style::default().fg(Color::Cyan)),
        ]),
        kv("Status", s.status_label(shown)),
    ];
    if let Some(pid) = s.pid() {
        lines.push(kv("PID", pid.to_string()));
    }
    let mut started = s.started.format("%H:%M:%S").to_string();
    if s.is_live() {
        started = format!("{} (up {})", started, graphs::duration(s.uptime()));
    }
    if s.restarts > 0 {
        started = format!("{} · restarted {}×", started, s.restarts);
    }
    lines.push(kv("Started", started));
    if let Phase::Exited { code, at } = &s.phase {
        let code = code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
        lines.push(kv("Exited", format!("{} with code {}", at.format("%H:%M:%S"), code)));
    }
    let command = if s.cmd.is_empty() { "Built into tuiOS".to_string() } else { s.cmd.join(" ") };
    lines.push(kv("Command", super::clean_text(&command)));
    if let Body::External(e) = &s.body {
        if e.lost {
            lines.push(Line::from(Span::styled(
                " Could not find this app's process — close its window by hand.",
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    // Actions always fit first; the info lines give way on short screens
    let info_h = lines.len() as u16;
    let actions_h = (actions.len() as u16 + 1).min(area.height);
    let info_shown = info_h.min(area.height - actions_h);
    frame.render_widget(Paragraph::new(lines), Rect { height: info_shown, ..area });

    // Per-app usage history
    let mut y = area.y + info_shown;
    let pid = s.pid();
    let label = s.label.clone();
    let external = s.is_external();
    let live = s.is_live();
    if let Some(pid) = pid.filter(|_| live) {
        let m = metrics::read();
        if let Some(h) = m.proc_hist.get(&pid) {
            if y + 2 + actions_h <= area.y + area.height {
                let now = m.procs.get(&pid).copied().unwrap_or_default();
                let cpu: Vec<u64> = h.cpu.tail(400).map(|v| (*v * 10.0) as u64).collect();
                let mem: Vec<u64> = h.mem.tail(400).copied().collect();
                let peak = mem.iter().copied().max().unwrap_or(1);
                spark_row(frame, Rect { y, height: 1, ..area }, "CPU", cpu, 1000, format!("{:.1}%", now.cpu), Color::Cyan);
                spark_row(frame, Rect { y: y + 1, height: 1, ..area }, "Memory", mem, peak, graphs::bytes(now.mem), Color::Yellow);
                y += 2;
            }
        }
    }

    // Live preview between the info and the actions
    let actions_top = area.y + area.height - actions_h;
    if actions_top > y + 4 {
        let preview_area = Rect { x: area.x, y, width: area.width, height: actions_top - y };
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Preview ", Style::default().fg(Color::DarkGray)));
        let inner = block.inner(preview_area);
        frame.render_widget(block, preview_area);
        if !sessions.render_preview(id, frame, inner) {
            let text = if external {
                " Running in its own terminal window — no preview."
            } else {
                " No preview available."
            };
            frame.render_widget(Paragraph::new(text).style(Style::default().fg(Color::DarkGray)), inner);
        }
    }

    // Actions
    {
        let Some(s) = sessions.get(id) else { return };
        let mut lines = vec![Line::from(Span::styled(
            format!(" Actions for {}", label),
            Style::default().fg(Color::DarkGray),
        ))];
        for (i, action) in actions.iter().enumerate() {
            let selected = tm.focus == Focus::Actions && i == tm.action_cursor;
            let danger = matches!(action, state::TmAction::Quit | state::TmAction::ForceKill);
            let style = if selected {
                Style::default().fg(Color::Black).bg(if danger { Color::Red } else { look.accent })
            } else if danger {
                Style::default().fg(Color::LightRed)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if selected { " » " } else { "   " };
            lines.push(Line::from(Span::styled(format!("{}{}", prefix, action.label(s)), style)));
        }
        frame.render_widget(
            Paragraph::new(lines),
            Rect { y: actions_top, height: area.y + area.height - actions_top, ..area },
        );
    }
}

/// Live sessions "End All" stops: everything except the dashboard on the home screen.
pub fn end_all_targets(sessions: &Sessions, home: &str) -> Vec<SessionId> {
    sessions
        .iter()
        .filter(|s| s.is_live() && !s.is_stopping())
        .filter(|s| !(s.kind == Kind::Dashboard && s.key == home))
        .map(|s| s.id)
        .collect()
}

// ── Startup tab ──────────────────────────────────────────────────────────────

fn render_startup(frame: &mut Frame, area: Rect, tm: &TaskManagerState, look: &Look) {
    let settings = crate::settings::persistence::load();
    let enabled = config::startup_apps(&settings);
    let focused = look.focused && tm.focus == Focus::Content;
    let border = if focused { Style::default().fg(look.accent) } else { Style::default().fg(Color::DarkGray) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(look.border_type)
        .border_style(border)
        .title(Span::styled(
            format!(" Launch at Startup · {} selected ", enabled.len()),
            border.add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![
        Line::from(Span::styled(
            " Ticked apps start in the background when tuiOS boots; find them in the Apps tab.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    if tm.startup_rows.is_empty() {
        lines.push(Line::from(Span::styled(" Nothing installed yet.", Style::default().fg(Color::DarkGray))));
    }
    let visible = inner.height.saturating_sub(2) as usize;
    let scroll = tm.startup_cursor.saturating_sub(visible.saturating_sub(1));
    let name_w = (inner.width as usize).saturating_sub(34).max(10);
    for (i, row) in tm.startup_rows.iter().enumerate().skip(scroll).take(visible) {
        let on = enabled.contains(&row.action);
        let selected = i == tm.startup_cursor;
        let style = row_style(selected, focused, look, Color::White);
        let tick = if on { "[✓]" } else { "[ ]" };
        let tick_style = if selected && focused {
            style
        } else if on {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", tick), tick_style),
            Span::styled(format!("{} {:<11}{:<12}", fit(&row.label, name_w), row.kind, row.mode), style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Overlays ─────────────────────────────────────────────────────────────────

fn render_confirm(frame: &mut Frame, area: Rect, tm: &TaskManagerState, sessions: &Sessions, home: &str) {
    let Some(Confirm::EndAll) = tm.confirm else { return };
    let targets = end_all_targets(sessions, home);
    let windows = targets.iter().filter(|id| sessions.get(**id).map_or(false, |s| s.is_external())).count();
    let width = 54u16.min(area.width);
    let height = 8u16.min(area.height);
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
        .title(" End All Background Apps? ")
        .style(Style::default().fg(Color::Red).bg(Color::Black));
    let inner = block.inner(rect).inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(block, rect);
    let mut lines = vec![Line::from(Span::styled(
        format!("{} app{} will be quit.", targets.len(), if targets.len() == 1 { "" } else { "s" }),
        Style::default().fg(Color::White),
    ))];
    if windows > 0 {
        lines.push(Line::from(Span::styled(
            format!("{} of them run in their own window.", windows),
            Style::default().fg(Color::Gray),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Unsaved work in those apps will be lost.",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));
    let button = |text: &'static str, on: bool, color: Color| {
        if on {
            Span::styled(format!(" {} ", text), Style::default().fg(Color::Black).bg(color))
        } else {
            Span::styled(format!(" {} ", text), Style::default().fg(Color::White))
        }
    };
    lines.push(Line::from(vec![
        button("Cancel", tm.confirm_cursor == 0, Color::Gray),
        Span::raw("   "),
        button("End All", tm.confirm_cursor == 1, Color::Red),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// "View Output": the app's screen at full size, scrollable through its history.
fn render_viewer(frame: &mut Frame, area: Rect, sessions: &mut Sessions, id: SessionId, look: &Look) {
    let label = sessions.get(id).map(|s| s.label.clone()).unwrap_or_default();
    let Some(pty) = sessions.pty_mut(id) else {
        frame.render_widget(Paragraph::new(" This app has no output to show."), area);
        return;
    };
    let (offset, max) = pty.scroll_info();
    let title = if offset > 0 {
        format!(" Output — {} · {} / {} lines back ", label, offset, max)
    } else {
        format!(" Output — {} · live ", label)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(look.border_type)
        .border_style(Style::default().fg(look.accent))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let screen = pty.screen_buffer();
    blit_bottom_left(&screen, frame.buffer_mut(), inner);
}

/// Copy the bottom-left of `src` into `area` 1:1 — the newest output stays visible
/// when the app's screen is larger than the viewer.
fn blit_bottom_left(src: &Buffer, dst: &mut Buffer, area: Rect) {
    let rows = src.area.height.min(area.height);
    let cols = src.area.width.min(area.width);
    let first = src.area.height - rows;
    for y in 0..rows {
        for x in 0..cols {
            if let (Some(cell), Some(out)) = (
                src.cell((src.area.x + x, src.area.y + first + y)),
                dst.cell_mut((area.x + x, area.y + y)),
            ) {
                out.set_symbol(cell.symbol()).set_style(cell.style());
            }
        }
    }
}
