/// Individual tiles drawn by Dashboard 1.
///
/// Every panel is responsible for degrading gracefully: each one is handed a
/// rect that can be anything from a few rows in a grid cell to the whole screen
/// when the user zooms it.

use chrono::{DateTime, Local, TimeZone};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Sparkline, Wrap},
    Frame,
};

use super::clockface::{self, ClockStyle};
use super::metrics::{fmt_bytes, fmt_duration, fmt_rate, ProcInfo, Snapshot};
use super::online::{self, OnlineState, Status};

const MUTED: Color = Color::DarkGray;
const LABEL: Color = Color::Gray;
const VALUE: Color = Color::White;
const OK: Color = Color::Green;
const WARN: Color = Color::Yellow;
const HIGH: Color = Color::Rgb(255, 150, 40);
const CRIT: Color = Color::Red;

pub struct Ctx<'a> {
    pub snap: &'a Snapshot,
    pub online: &'a OnlineState,
    pub now: DateTime<Local>,
    pub accent: Color,
    pub border: BorderType,
    pub focused: bool,
    pub use_24h: bool,
    pub show_seconds: bool,
    pub clock_style: ClockStyle,
}

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

fn tile(ctx: &Ctx, title: &str, selected: bool) -> Block<'static> {
    let (border, text) = if selected && ctx.focused {
        (ctx.accent, ctx.accent)
    } else if selected {
        (LABEL, VALUE)
    } else {
        (MUTED, LABEL)
    };
    let mut title_style = Style::default().fg(text);
    if selected {
        title_style = title_style.add_modifier(Modifier::BOLD);
    }
    Block::default()
        .borders(Borders::ALL)
        .border_type(ctx.border)
        .border_style(Style::default().fg(border))
        .title_top(Line::from(Span::styled(format!(" {} ", title), title_style)))
}

/// Draw a tile frame and return the padded area to draw content into.
fn open(frame: &mut Frame, area: Rect, ctx: &Ctx, title: &str, selected: bool) -> Rect {
    let block = tile(ctx, title, selected);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner.inner(Margin { horizontal: 1, vertical: 0 })
}

fn pct_color(pct: f64) -> Color {
    if pct >= 90.0 {
        CRIT
    } else if pct >= 75.0 {
        HIGH
    } else if pct >= 50.0 {
        WARN
    } else {
        OK
    }
}

fn bar(pct: f64, width: usize) -> String {
    let w = width.max(1);
    let filled = ((pct / 100.0) * w as f64).round().clamp(0.0, w as f64) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(w - filled))
}

fn meter(label: &str, pct: f64, detail: &str, width: usize) -> Line<'static> {
    let color = pct_color(pct);
    let detail = if width >= 46 { detail } else { "" };
    let fixed = 7 + 1 + 6 + 1 + detail.chars().count() + 1;
    let bar_w = width.saturating_sub(fixed).clamp(6, 48);
    let mut spans = vec![
        Span::styled(format!("{:<7}", label), Style::default().fg(LABEL)),
        Span::styled(bar(pct, bar_w), Style::default().fg(color)),
        Span::styled(
            format!(" {:>5.1}%", pct),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ];
    if !detail.is_empty() {
        spans.push(Span::styled(format!("  {}", detail), Style::default().fg(MUTED)));
    }
    Line::from(spans)
}

fn kv(key: &str, value: impl Into<String>, key_width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<width$}", key, width = key_width),
            Style::default().fg(MUTED),
        ),
        Span::styled(value.into(), Style::default().fg(VALUE)),
    ])
}

fn note(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), Style::default().fg(MUTED)))
}

fn paragraph(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn tail(data: &[u64], n: usize) -> Vec<u64> {
    if n == 0 {
        return Vec::new();
    }
    if data.len() > n {
        data[data.len() - n..].to_vec()
    } else {
        data.to_vec()
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

pub fn clock(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Clock", selected);
    if inner.width < 8 || inner.height == 0 {
        return;
    }

    let fmt = match (ctx.use_24h, ctx.show_seconds) {
        (true, true) => "%H:%M:%S",
        (true, false) => "%H:%M",
        (false, true) => "%I:%M:%S",
        (false, false) => "%I:%M",
    };
    let time_text = ctx.now.format(fmt).to_string();
    let suffix = if ctx.use_24h {
        String::new()
    } else {
        ctx.now.format(" %p").to_string()
    };

    let date_line = Line::from(Span::styled(
        ctx.now.format("%A, %-d %B %Y").to_string(),
        Style::default().fg(VALUE),
    ))
    .alignment(Alignment::Center);
    let greeting_line = Line::from(Span::styled(greeting(ctx), Style::default().fg(MUTED)))
        .alignment(Alignment::Center);

    if ctx.clock_style == ClockStyle::Analog {
        analog_clock(frame, inner, ctx, &time_text, &suffix, date_line, greeting_line);
        return;
    }

    let shade = shade_of(ctx.accent);
    let fitted = clockface::best_size(
        ctx.clock_style,
        &time_text,
        (inner.width as usize).saturating_sub(suffix.chars().count()),
        (inner.height as usize).saturating_sub(2),
    );

    let Some(size) = fitted else {
        let mut lines = vec![Line::from(Span::styled(
            format!("{}{}", time_text, suffix),
            Style::default().fg(ctx.accent).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center)];
        if inner.height >= 2 {
            lines.push(date_line);
        }
        if inner.height >= 3 {
            lines.push(greeting_line);
        }
        let pad = (inner.height as usize).saturating_sub(lines.len()) / 2;
        for _ in 0..pad {
            lines.insert(0, Line::raw(""));
        }
        paragraph(frame, inner, lines);
        return;
    };

    let (digits, digit_w) = clockface::render(ctx.clock_style, &time_text, size, ctx.accent, shade);
    let height = digits.len() as u16;
    let total_w = digit_w + suffix.chars().count();
    let pad_top = inner.height.saturating_sub(height + 2) / 2;
    let x = inner.x + (inner.width.saturating_sub(total_w as u16)) / 2;
    let y = inner.y + pad_top;

    paragraph(
        frame,
        Rect {
            x,
            y,
            width: digit_w as u16,
            height,
        },
        digits,
    );

    if !suffix.is_empty() {
        paragraph(
            frame,
            Rect {
                x: x + digit_w as u16,
                y: y + height - 1,
                width: suffix.chars().count() as u16,
                height: 1,
            },
            vec![Line::from(Span::styled(
                suffix.trim_start().to_string(),
                Style::default().fg(ctx.accent),
            ))],
        );
    }

    let below = Rect {
        x: inner.x,
        y: y + height,
        width: inner.width,
        height: (inner.y + inner.height).saturating_sub(y + height),
    };
    if below.height > 0 {
        let mut lines = vec![date_line];
        if below.height > 1 {
            lines.push(greeting_line);
        }
        paragraph(frame, below, lines);
    }
}

fn analog_clock(
    frame: &mut Frame,
    inner: Rect,
    ctx: &Ctx,
    time_text: &str,
    suffix: &str,
    date_line: Line<'static>,
    greeting_line: Line<'static>,
) {
    let captions = 3.min(inner.height.saturating_sub(4)) as u16;
    let face = Rect {
        height: inner.height - captions,
        ..inner
    };
    clockface::analog(
        frame,
        face,
        ctx.now,
        ctx.accent,
        shade_of(ctx.accent),
        ctx.show_seconds,
    );
    if captions == 0 {
        return;
    }
    let mut lines = vec![Line::from(Span::styled(
        format!("{}{}", time_text, suffix),
        Style::default().fg(ctx.accent).add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Center)];
    if captions > 1 {
        lines.push(date_line);
    }
    if captions > 2 {
        lines.push(greeting_line);
    }
    paragraph(
        frame,
        Rect {
            x: inner.x,
            y: inner.y + face.height,
            width: inner.width,
            height: captions,
        },
        lines,
    );
}

/// A dark tint of the accent, used for shadows, dial rings and flap seams.
fn shade_of(accent: Color) -> Color {
    match accent {
        Color::Rgb(r, g, b) => Color::Rgb(r / 4, g / 4, b / 4),
        _ => Color::Rgb(58, 58, 58),
    }
}

fn greeting(ctx: &Ctx) -> String {
    let part = match ctx.now.format("%H").to_string().parse::<u32>().unwrap_or(12) {
        0..=4 => "Good night",
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    match std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        Ok(user) if !user.trim().is_empty() => format!("{}, {}", part, user.trim()),
        _ => part.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Vitals
// ---------------------------------------------------------------------------

pub fn vitals(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Vitals", selected);
    if inner.height == 0 {
        return;
    }
    let s = ctx.snap;
    if !s.ready {
        paragraph(frame, inner, vec![note("Collecting system metrics…")]);
        return;
    }

    let w = inner.width as usize;
    let mut lines = vec![
        meter(
            "CPU",
            s.cpu as f64,
            &format!("{} cores", s.host.cpu_cores.max(s.cores.len())),
            w,
        ),
        meter(
            "Memory",
            s.mem_pct(),
            &format!("{} / {}", fmt_bytes(s.mem_used), fmt_bytes(s.mem_total)),
            w,
        ),
    ];
    if s.swap_total > 0 {
        lines.push(meter(
            "Swap",
            s.swap_pct(),
            &format!("{} / {}", fmt_bytes(s.swap_used), fmt_bytes(s.swap_total)),
            w,
        ));
    }
    lines.push(meter(
        "Disk",
        s.disk_pct(),
        &format!("{} / {}", fmt_bytes(s.disk.used), fmt_bytes(s.disk.total)),
        w,
    ));

    if (lines.len() as u16) < inner.height {
        lines.push(Line::raw(""));
    }
    if (lines.len() as u16) < inner.height {
        let mut spans = vec![
            Span::styled("Uptime ", Style::default().fg(MUTED)),
            Span::styled(fmt_duration(s.uptime), Style::default().fg(VALUE)),
        ];
        if let Some((one, five, fifteen)) = s.load {
            spans.push(Span::styled("   Load ", Style::default().fg(MUTED)));
            spans.push(Span::styled(
                format!("{:.2} {:.2} {:.2}", one, five, fifteen),
                Style::default().fg(VALUE),
            ));
        }
        if let Some((_, temp)) = &s.temp {
            spans.push(Span::styled("   Temp ", Style::default().fg(MUTED)));
            spans.push(Span::styled(
                format!("{:.0}°C", temp),
                Style::default().fg(pct_color(*temp as f64)),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Any remaining room goes to a CPU trend strip.
    let used = lines.len() as u16;
    paragraph(
        frame,
        Rect { height: used.min(inner.height), ..inner },
        lines,
    );
    if inner.height > used + 1 {
        let strip = Rect {
            x: inner.x,
            y: inner.y + used + 1,
            width: inner.width,
            height: inner.height - used - 1,
        };
        sparkline_block(frame, strip, "CPU", &s.cpu_history, 100, ctx.accent);
    }
}

// ---------------------------------------------------------------------------
// Per-core load
// ---------------------------------------------------------------------------

pub fn cores(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Processor cores", selected);
    if inner.height == 0 || inner.width < 10 {
        return;
    }
    let cores = &ctx.snap.cores;
    if cores.is_empty() {
        paragraph(frame, inner, vec![note("Collecting core usage…")]);
        return;
    }

    const CELL: usize = 17;
    let columns = (((inner.width as usize) + 1) / (CELL + 1)).max(1);
    let rows_needed = cores.len().div_ceil(columns);

    if rows_needed <= inner.height as usize {
        let lines = cores
            .chunks(columns)
            .enumerate()
            .map(|(row, chunk)| {
                let mut spans = Vec::new();
                for (col, usage) in chunk.iter().enumerate() {
                    if col > 0 {
                        spans.push(Span::raw(" "));
                    }
                    let index = row * columns + col;
                    let pct = *usage as f64;
                    spans.push(Span::styled(
                        format!("{:>3} ", format!("c{}", index)),
                        Style::default().fg(MUTED),
                    ));
                    spans.push(Span::styled(bar(pct, 8), Style::default().fg(pct_color(pct))));
                    spans.push(Span::styled(
                        format!("{:>4.0}%", pct),
                        Style::default().fg(LABEL),
                    ));
                }
                Line::from(spans)
            })
            .collect();
        paragraph(frame, inner, lines);
        return;
    }

    // Too many cores for a grid — one column per core instead.
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let spans: Vec<Span<'static>> = cores
        .iter()
        .take(inner.width as usize)
        .map(|usage| {
            let pct = (*usage as f64).clamp(0.0, 100.0);
            let level = ((pct / 100.0) * (LEVELS.len() - 1) as f64).round() as usize;
            Span::styled(LEVELS[level].to_string(), Style::default().fg(pct_color(pct)))
        })
        .collect();
    let mut lines = vec![Line::from(spans)];
    lines.push(note(&format!(
        "{} logical cores · average {:.1}%",
        cores.len(),
        ctx.snap.cpu
    )));
    paragraph(frame, inner, lines);
}

// ---------------------------------------------------------------------------
// History sparklines
// ---------------------------------------------------------------------------

pub fn history(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "History", selected);
    if inner.height < 3 {
        return;
    }
    let s = ctx.snap;
    let net_max = s.net_history.iter().copied().max().unwrap_or(1).max(1);
    let charts: [(&str, &[u64], u64, Color); 3] = [
        ("CPU", &s.cpu_history, 100, ctx.accent),
        ("Memory", &s.mem_history, 100, Color::Rgb(120, 170, 255)),
        ("Network", &s.net_history, net_max, Color::Rgb(180, 140, 255)),
    ];
    let slots = (inner.height as usize / 3).clamp(1, 3);
    let constraints: Vec<Constraint> = (0..slots).map(|_| Constraint::Ratio(1, slots as u32)).collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (slot, (label, data, max, color)) in charts.iter().take(slots).enumerate() {
        sparkline_block(frame, rows[slot], label, data, *max, *color);
    }
}

fn sparkline_block(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    data: &[u64],
    max: u64,
    color: Color,
) {
    if area.height == 0 || area.width < 6 {
        return;
    }
    let current = data.last().copied().unwrap_or(0);
    let caption = if label == "Network" {
        format!("{}  {} KB/s", label, current)
    } else {
        format!("{}  {}%", label, current)
    };
    paragraph(
        frame,
        Rect { height: 1, ..area },
        vec![Line::from(vec![
            Span::styled(caption, Style::default().fg(MUTED)),
        ])],
    );
    if area.height < 2 {
        return;
    }
    let chart = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height - 1,
    };
    frame.render_widget(
        Sparkline::default()
            .data(tail(data, chart.width as usize))
            .max(max.max(1))
            .style(Style::default().fg(color)),
        chart,
    );
}

// ---------------------------------------------------------------------------
// Host / OS
// ---------------------------------------------------------------------------

pub fn host(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "System", selected);
    if inner.height == 0 {
        return;
    }
    let s = ctx.snap;
    let h = &s.host;
    let booted = Local
        .timestamp_opt(s.boot_time as i64, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into());

    let mut cpu_detail = format!("{} logical", h.cpu_cores.max(s.cores.len()));
    if let Some(physical) = h.cpu_physical {
        cpu_detail = format!("{} logical / {} physical", h.cpu_cores, physical);
    }
    if h.cpu_mhz > 100 {
        cpu_detail.push_str(&format!(" @ {} MHz", h.cpu_mhz));
    }

    let kw = 11;
    let mut lines = vec![
        kv("Host", h.hostname.clone(), kw),
        kv("OS", format!("{} {}", h.os, h.os_version), kw),
        kv("Kernel", h.kernel.clone(), kw),
        kv("Arch", h.arch.clone(), kw),
        kv(
            "Processor",
            truncate(
                if h.cpu_brand.is_empty() { "—" } else { &h.cpu_brand },
                inner.width.saturating_sub(kw as u16) as usize,
            ),
            kw,
        ),
        kv("Cores", cpu_detail, kw),
        kv("Uptime", fmt_duration(s.uptime), kw),
        kv("Booted", booted, kw),
        kv(
            "Storage",
            format!(
                "{} ({})  {} free",
                s.disk.mount,
                if s.disk.fs.is_empty() { "—" } else { &s.disk.fs },
                fmt_bytes(s.disk.total.saturating_sub(s.disk.used))
            ),
            kw,
        ),
        kv("Processes", s.process_count.to_string(), kw),
    ];
    if let Some((label, temp)) = &s.temp {
        lines.push(kv(
            "Sensor",
            format!("{} {:.1}°C", truncate(label, 18), temp),
            kw,
        ));
    }
    lines.push(kv("tuiOS", format!("v{}", env!("CARGO_PKG_VERSION")), kw));
    paragraph(frame, inner, lines);
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

pub fn network(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Network", selected);
    if inner.height == 0 {
        return;
    }
    let n = &ctx.snap.net;
    if n.iface.is_empty() {
        paragraph(frame, inner, vec![note("No active network interface.")]);
        return;
    }

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                truncate(&n.iface, 12),
                Style::default().fg(ctx.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                n.ip.clone().unwrap_or_else(|| "no address".into()),
                Style::default().fg(VALUE),
            ),
        ]),
        Line::from(vec![
            Span::styled("↓ ", Style::default().fg(OK)),
            Span::styled(
                format!("{:<12}", fmt_rate(n.rx_rate)),
                Style::default().fg(OK).add_modifier(Modifier::BOLD),
            ),
            Span::styled("↑ ", Style::default().fg(Color::Rgb(120, 170, 255))),
            Span::styled(
                fmt_rate(n.tx_rate),
                Style::default()
                    .fg(Color::Rgb(120, 170, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("total ", Style::default().fg(MUTED)),
            Span::styled(
                format!("↓ {:<11}", fmt_bytes(n.rx_total)),
                Style::default().fg(LABEL),
            ),
            Span::styled(
                format!("↑ {}", fmt_bytes(n.tx_total)),
                Style::default().fg(LABEL),
            ),
        ]),
    ];
    if let Some(mac) = &n.mac {
        lines.push(note(&format!(
            "{} · {} interface{}",
            mac,
            n.iface_count,
            if n.iface_count == 1 { "" } else { "s" }
        )));
    }

    let used = lines.len() as u16;
    paragraph(frame, Rect { height: used.min(inner.height), ..inner }, lines);
    if inner.height > used {
        let chart = Rect {
            x: inner.x,
            y: inner.y + used,
            width: inner.width,
            height: inner.height - used,
        };
        let max = ctx.snap.net_history.iter().copied().max().unwrap_or(1).max(1);
        frame.render_widget(
            Sparkline::default()
                .data(tail(&ctx.snap.net_history, chart.width as usize))
                .max(max)
                .style(Style::default().fg(Color::Rgb(180, 140, 255))),
            chart,
        );
    }
}

// ---------------------------------------------------------------------------
// Battery
// ---------------------------------------------------------------------------

pub fn battery(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Power", selected);
    if inner.height == 0 {
        return;
    }
    let Some(b) = &ctx.snap.battery else {
        paragraph(
            frame,
            inner,
            vec![note("No battery detected — running on mains power.")],
        );
        return;
    };

    let pct = b.percent as f64;
    // Charge colours run the opposite way to load colours.
    let color = if pct <= 15.0 {
        CRIT
    } else if pct <= 35.0 {
        HIGH
    } else {
        OK
    };
    let icon = match b.state.as_str() {
        "Charging" => "⚡",
        "Full" => "▮",
        _ => "▯",
    };

    let bar_w = (inner.width as usize).saturating_sub(18).clamp(6, 30);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{} ", icon), Style::default().fg(color)),
            Span::styled(bar(pct, bar_w), Style::default().fg(color)),
            Span::styled(
                format!(" {:>3.0}%", pct),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(b.state.clone(), Style::default().fg(VALUE)),
            Span::styled(
                match b.remaining {
                    Some(secs) if secs > 0 => format!(
                        " · {} {}",
                        fmt_duration(secs),
                        if b.state == "Charging" { "to full" } else { "left" }
                    ),
                    _ => String::new(),
                },
                Style::default().fg(MUTED),
            ),
        ]),
    ];
    if inner.height > 2 {
        lines.push(note(&format!("Draw {:.1} W", b.power_w)));
    }
    if inner.height > 3 {
        let mut parts = Vec::new();
        if let Some(health) = b.health {
            parts.push(format!("Health {:.0}%", health));
        }
        if let Some(cycles) = b.cycles {
            parts.push(format!("{} cycles", cycles));
        }
        if let Some(temp) = b.temp_c {
            parts.push(format!("{:.1}°C", temp));
        }
        if !parts.is_empty() {
            lines.push(note(&parts.join(" · ")));
        }
    }
    if inner.height > 4 {
        let name = match (&b.vendor, &b.model) {
            (Some(v), Some(m)) => format!("{} {}", v, m),
            (Some(v), None) => v.clone(),
            (None, Some(m)) => m.clone(),
            _ => String::new(),
        };
        if !name.is_empty() {
            lines.push(note(&truncate(&name, inner.width as usize)));
        }
    }
    paragraph(frame, inner, lines);
}

// ---------------------------------------------------------------------------
// Weather
// ---------------------------------------------------------------------------

pub fn weather(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Weather", selected);
    if inner.height == 0 {
        return;
    }
    match (&ctx.online.weather_status, &ctx.online.weather) {
        (Status::Disabled, _) => {
            paragraph(
                frame,
                inner,
                vec![note("Online panels are turned off in Settings → Dashboard.")],
            );
        }
        (Status::Error(err), None) => {
            paragraph(
                frame,
                inner,
                vec![
                    Line::from(Span::styled(
                        "Weather unavailable".to_string(),
                        Style::default().fg(WARN),
                    )),
                    note(err),
                ],
            );
        }
        (_, None) => {
            paragraph(frame, inner, vec![note("Fetching forecast…")]);
        }
        (status, Some(w)) => {
            let (icon, description) = online::describe(w.code, w.is_day);
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        format!("{}  ", icon),
                        Style::default().fg(ctx.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:.0}°{}", w.temp, w.unit),
                        Style::default().fg(ctx.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("   {}", description),
                        Style::default().fg(VALUE),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        format!("feels {:.0}°", w.feels_like),
                        Style::default().fg(LABEL),
                    ),
                    Span::styled(
                        format!("   H {:.0}°  L {:.0}°", w.high, w.low),
                        Style::default().fg(MUTED),
                    ),
                ]),
            ];
            if inner.height > 2 {
                lines.push(note(&format!(
                    "humidity {}%   wind {:.0} {}",
                    w.humidity, w.wind, w.wind_unit
                )));
            }
            if inner.height > 3 {
                lines.push(note(&format!("↑ {}   ↓ {}", w.sunrise, w.sunset)));
            }
            if inner.height > 4 {
                let age = ctx
                    .online
                    .weather_age
                    .map(|d| format!(" · {} ago", fmt_duration(d.as_secs())))
                    .unwrap_or_default();
                let stale = matches!(status, Status::Error(_));
                lines.push(Line::from(Span::styled(
                    format!(
                        "{}{}{}",
                        w.place,
                        age,
                        if stale { " · refresh failed" } else { "" }
                    ),
                    Style::default().fg(if stale { WARN } else { MUTED }),
                )));
            }
            paragraph(frame, inner, lines);
        }
    }
}

// ---------------------------------------------------------------------------
// Connectivity
// ---------------------------------------------------------------------------

pub fn connectivity(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool) {
    let inner = open(frame, area, ctx, "Connectivity", selected);
    if inner.height == 0 {
        return;
    }
    if ctx.online.conn_status == Status::Disabled {
        paragraph(
            frame,
            inner,
            vec![note("Online panels are turned off in Settings → Dashboard.")],
        );
        return;
    }
    let Some(conn) = &ctx.online.conn else {
        paragraph(frame, inner, vec![note("Checking connection…")]);
        return;
    };

    let (dot, label, color) = if conn.online {
        ("●", "Online", OK)
    } else {
        ("●", "Offline", CRIT)
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", dot), Style::default().fg(color)),
        Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            conn.latency_ms
                .map(|ms| format!("   {} ms", ms))
                .unwrap_or_default(),
            Style::default().fg(MUTED),
        ),
    ])];
    lines.push(kv(
        "Public IP",
        conn.public_ip.clone().unwrap_or_else(|| "—".into()),
        11,
    ));
    lines.push(kv("Interfaces", ctx.snap.net.iface_count.to_string(), 11));
    if inner.height > 3 {
        lines.push(kv(
            "Local IP",
            ctx.snap.net.ip.clone().unwrap_or_else(|| "—".into()),
            11,
        ));
    }
    paragraph(frame, inner, lines);
}

// ---------------------------------------------------------------------------
// Process tables
// ---------------------------------------------------------------------------

pub fn processes(frame: &mut Frame, area: Rect, ctx: &Ctx, selected: bool, by_memory: bool) {
    let title = if by_memory {
        "Top processes — memory"
    } else {
        "Top processes — CPU"
    };
    let inner = open(frame, area, ctx, title, selected);
    if inner.height < 2 || inner.width < 24 {
        return;
    }
    let rows: &Vec<ProcInfo> = if by_memory {
        &ctx.snap.top_mem
    } else {
        &ctx.snap.top_cpu
    };
    if rows.is_empty() {
        paragraph(frame, inner, vec![note("Reading process table…")]);
        return;
    }

    let name_w = (inner.width as usize).saturating_sub(7 + 7 + 10 + 3).max(8);
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{:<7}{:<name_w$}  {:>6} {:>9}",
            "PID",
            "NAME",
            "CPU%",
            "MEM",
            name_w = name_w
        ),
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    ))];

    for p in rows.iter().take(inner.height as usize - 1) {
        let emphasis = if by_memory {
            pct_color((p.memory as f64 / ctx.snap.mem_total.max(1) as f64) * 100.0 * 3.0)
        } else {
            pct_color(p.cpu as f64)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<7}", p.pid), Style::default().fg(MUTED)),
            Span::styled(
                format!("{:<name_w$}", truncate(&p.name, name_w), name_w = name_w),
                Style::default().fg(VALUE),
            ),
            Span::styled(format!("  {:>5.1}", p.cpu), Style::default().fg(emphasis)),
            Span::styled(
                format!(" {:>9}", fmt_bytes(p.memory)),
                Style::default().fg(LABEL),
            ),
        ]));
    }
    paragraph(frame, inner, lines);
}

// ---------------------------------------------------------------------------
// Fallback
// ---------------------------------------------------------------------------

pub fn too_small(frame: &mut Frame, area: Rect, _ctx: &Ctx) {
    frame.render_widget(
        Paragraph::new("Terminal too small for this dashboard.\nResize to at least 40×10.")
            .style(Style::default().fg(MUTED))
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center),
        area,
    );
}
