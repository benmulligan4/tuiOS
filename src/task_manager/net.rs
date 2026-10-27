/// Network tab: throughput graph (all interfaces or the selected one), TCP latency graph,
/// Wi-Fi / public IP / connectivity details and a per-interface table.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use super::graphs::{self, Series};
use super::metrics::{self, IfaceStat, Metrics};
use super::netinfo::{self, NetExtras, Wifi};
use super::state::{Focus, TaskManagerState};

pub fn render(frame: &mut Frame, area: Rect, tm: &TaskManagerState, accent: Color, line: bool, btype: BorderType) {
    let m = metrics::read();
    let x = netinfo::read();
    let window = tm.window_secs();
    let ifaces: Vec<&IfaceStat> = m.shown_ifaces().collect();

    let table_h = ((ifaces.len() + 1) as u16 + 3).min(10).min(area.height / 3).max(4);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(table_h)])
        .split(area);
    frame.render_widget(Paragraph::new(totals_line(&m)), rows[0]);

    // Wide: details beside the graphs. Narrow: two summary lines above them.
    let (graphs_rect, details_rect, compact_details) = if rows[1].width >= 90 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(36)])
            .split(rows[1]);
        (cols[0], cols[1], false)
    } else {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Fill(1)])
            .split(rows[1]);
        (parts[1], parts[0], true)
    };
    let graph_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(graphs_rect);

    let selected = tm.net_cursor.checked_sub(1).and_then(|i| ifaces.get(i).copied());
    render_throughput(frame, graph_rows[0], &m, selected, window, line, btype);
    render_latency(frame, graph_rows[1], &x, window, line, btype);
    if compact_details {
        frame.render_widget(Paragraph::new(compact_detail_lines(&m, &x)), details_rect);
    } else {
        render_details(frame, details_rect, &m, &x, btype);
    }
    render_table(frame, rows[2], &m, &ifaces, tm, accent, btype);
}

fn block(title: String, btype: BorderType) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(btype)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(title, Style::default().fg(Color::Gray)))
}

fn totals_line(m: &Metrics) -> Line<'static> {
    let (rx_s, tx_s, rx_b, tx_b) = m.net_totals();
    let dim = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::White);
    Line::from(vec![
        Span::styled(" This session ", dim),
        Span::styled(format!("↓ {}  ↑ {}", graphs::bytes(rx_s), graphs::bytes(tx_s)), val),
        Span::styled("   Since boot ", dim),
        Span::styled(format!("↓ {}  ↑ {}", graphs::bytes(rx_b), graphs::bytes(tx_b)), val),
    ])
}

fn render_throughput(
    frame: &mut Frame,
    area: Rect,
    m: &Metrics,
    iface: Option<&IfaceStat>,
    window: u64,
    line: bool,
    btype: BorderType,
) {
    let n = m.samples_for(window);
    let (name, rx, tx) = match iface {
        Some(i) => (super::clean_text(&i.name), &i.rx_rate, &i.tx_rate),
        None => ("All interfaces".to_string(), &m.net_rx, &m.net_tx),
    };
    let series = vec![
        Series { name: "Download".into(), color: Color::Green, values: graphs::window(rx, n, Some) },
        Series { name: "Upload".into(), color: Color::LightBlue, values: graphs::window(tx, n, Some) },
    ];
    let b = block(
        format!(
            " {} · ↓ {} ↑ {} ",
            name,
            graphs::rate(rx.last().copied().unwrap_or(0.0)),
            graphs::rate(tx.last().copied().unwrap_or(0.0))
        ),
        btype,
    );
    let inner = b.inner(area);
    frame.render_widget(b, area);
    if inner.height == 0 {
        return;
    }
    let top = (graphs::peak(&series) * 1.2).max(1024.0);
    if inner.height >= 4 {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(inner);
        graphs::draw(frame, parts[0], &series, top, line, window, &graphs::rate);
        graphs::text(frame, parts[1], graphs::legend(&[("Download".into(), Color::Green), ("Upload".into(), Color::LightBlue)]));
    } else {
        graphs::draw(frame, inner, &series, top, line, window, &graphs::rate);
    }
}

/// (average, min, max, failed probes, probes) over the newest `n` probes.
fn latency_stats(x: &NetExtras, n: usize) -> (Option<f32>, Option<f32>, Option<f32>, usize, usize) {
    let samples: Vec<Option<f32>> = x.latency.tail(n).copied().collect();
    let ok: Vec<f32> = samples.iter().flatten().copied().collect();
    let avg = (!ok.is_empty()).then(|| ok.iter().sum::<f32>() / ok.len() as f32);
    let min = ok.iter().copied().reduce(f32::min);
    let max = ok.iter().copied().reduce(f32::max);
    (avg, min, max, samples.len() - ok.len(), samples.len())
}

fn render_latency(frame: &mut Frame, area: Rect, x: &NetExtras, window: u64, line: bool, btype: BorderType) {
    let n = ((window as f64 / netinfo::LATENCY_EVERY.as_secs_f64()).ceil() as usize).max(2);
    let now = x.latency.last().copied().flatten();
    let target = if x.target.is_empty() { "…".to_string() } else { x.target.clone() };
    let title = match now {
        Some(ms) => format!(" Latency to {} · {:.0} ms ", target, ms),
        None if x.latency.len() > 0 => format!(" Latency to {} · no reply ", target),
        None => format!(" Latency to {} ", target),
    };
    let b = block(title, btype);
    let inner = b.inner(area);
    frame.render_widget(b, area);
    let series = vec![Series {
        name: "Latency".into(),
        color: Color::Cyan,
        values: graphs::window(&x.latency, n, |v| v.map(|ms| ms as f64)),
    }];
    let top = (graphs::peak(&series) * 1.2).max(50.0);
    graphs::draw(frame, inner, &series, top, line, window, &|v| format!("{:.0}ms", v));
}

fn kv(key: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<11}", key), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

fn signal_bar(pct: u8) -> String {
    let filled = ((pct as usize + 10) / 20).min(5);
    format!("{}{} {}%", "▮".repeat(filled), "▯".repeat(5 - filled), pct)
}

fn wifi_text(wifi: &Wifi) -> (String, Option<String>, Color) {
    match wifi {
        Wifi::Connected { ssid, signal, rssi } => {
            let signal = signal.map(|pct| match rssi {
                Some(dbm) => format!("{} ({} dBm)", signal_bar(pct), dbm),
                None => signal_bar(pct),
            });
            (ssid.clone(), signal, Color::White)
        }
        Wifi::Disconnected => ("Not connected".into(), None, Color::Gray),
        Wifi::Unavailable(why) => (why.clone(), None, Color::DarkGray),
        Wifi::Unknown => ("Checking…".into(), None, Color::DarkGray),
    }
}

fn first_ipv4(m: &Metrics) -> Option<String> {
    m.shown_ifaces().flat_map(|i| i.ips.iter()).find(|ip| !ip.contains(':')).cloned()
}

fn render_details(frame: &mut Frame, area: Rect, m: &Metrics, x: &NetExtras, btype: BorderType) {
    let b = block(" Connection ".into(), btype);
    let inner = b.inner(area);
    frame.render_widget(b, area);

    let mut lines = Vec::new();
    let (wifi, signal, color) = wifi_text(&x.wifi);
    lines.push(kv("Wi-Fi", wifi, color));
    if let Some(signal) = signal {
        lines.push(kv("Signal", signal, Color::White));
    }
    match x.online {
        Some(true) => lines.push(kv("Internet", "Online".into(), Color::Green)),
        Some(false) => lines.push(kv("Internet", "Unreachable".into(), Color::Red)),
        None => lines.push(kv("Internet", "Checking…".into(), Color::DarkGray)),
    }
    match &x.public_ip {
        Some(ip) => lines.push(kv("Public IP", ip.clone(), Color::White)),
        None => lines.push(kv("Public IP", x.public_ip_note.clone(), Color::DarkGray)),
    }
    if let Some(local) = first_ipv4(m) {
        lines.push(kv("Local IP", local, Color::White));
    }
    let (avg, min, max, lost, total) = latency_stats(x, 30);
    if let Some(avg) = avg {
        lines.push(kv(
            "Latency",
            format!("{:.0} ms avg ({:.0}–{:.0})", avg, min.unwrap_or(avg), max.unwrap_or(avg)),
            Color::White,
        ));
    }
    if total > 0 {
        let loss = lost as f64 / total as f64 * 100.0;
        lines.push(kv("Probe loss", format!("{:.0}% last minute", loss), graphs::heat(loss * 4.0)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" r refreshes the public IP", Style::default().fg(Color::DarkGray))));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Two-line version of the Connection panel for narrow screens.
fn compact_detail_lines(m: &Metrics, x: &NetExtras) -> Vec<Line<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let (wifi, signal, color) = wifi_text(&x.wifi);
    let mut first = vec![Span::styled(" Wi-Fi ", dim), Span::styled(wifi, Style::default().fg(color))];
    if let Some(signal) = signal {
        first.push(Span::styled(format!("  {}", signal), Style::default().fg(Color::White)));
    }
    let online = match x.online {
        Some(true) => Span::styled("Online", Style::default().fg(Color::Green)),
        Some(false) => Span::styled("Unreachable", Style::default().fg(Color::Red)),
        None => Span::styled("Checking…", dim),
    };
    let ip = x.public_ip.clone().unwrap_or_else(|| x.public_ip_note.clone());
    vec![
        Line::from(first),
        Line::from(vec![
            Span::styled(" Internet ", dim),
            online,
            Span::styled("  Public ", dim),
            Span::styled(ip, Style::default().fg(Color::White)),
            Span::styled("  Local ", dim),
            Span::styled(first_ipv4(m).unwrap_or_default(), Style::default().fg(Color::White)),
        ]),
    ]
}

struct TableRow {
    name: String,
    addr: String,
    mac: String,
    rx: f64,
    tx: f64,
    rx_session: u64,
    tx_session: u64,
    rx_boot: u64,
    tx_boot: u64,
    errors: u64,
}

/// Columns drop away from the right as the table narrows (tier 2 = everything).
fn table_line(tier: u8, r: &TableRow) -> String {
    match tier {
        2 => format!(
            " {:<14} {:<18} {:<17} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9} {:>5}",
            clip(&r.name, 14), clip(&r.addr, 18), r.mac, graphs::rate(r.rx), graphs::rate(r.tx),
            graphs::bytes(r.rx_session), graphs::bytes(r.tx_session),
            graphs::bytes(r.rx_boot), graphs::bytes(r.tx_boot), r.errors
        ),
        1 => format!(
            " {:<14} {:<18} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9}",
            clip(&r.name, 14), clip(&r.addr, 18), graphs::rate(r.rx), graphs::rate(r.tx),
            graphs::bytes(r.rx_session), graphs::bytes(r.tx_session),
            graphs::bytes(r.rx_boot), graphs::bytes(r.tx_boot)
        ),
        _ => format!(
            " {:<12} {:<16} {:>10} {:>10} {:>9} {:>9}",
            clip(&r.name, 12), clip(&r.addr, 16), graphs::rate(r.rx), graphs::rate(r.tx),
            graphs::bytes(r.rx_boot), graphs::bytes(r.tx_boot)
        ),
    }
}

fn table_header(tier: u8) -> String {
    match tier {
        2 => format!(
            " {:<14} {:<18} {:<17} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9} {:>5}",
            "Interface", "Address", "MAC", "↓ Now", "↑ Now", "↓ Session", "↑ Session", "↓ Boot", "↑ Boot", "Errs"
        ),
        1 => format!(
            " {:<14} {:<18} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9}",
            "Interface", "Address", "↓ Now", "↑ Now", "↓ Session", "↑ Session", "↓ Boot", "↑ Boot"
        ),
        _ => format!(" {:<12} {:<16} {:>10} {:>10} {:>9} {:>9}", "Interface", "Address", "↓ Now", "↑ Now", "↓ Boot", "↑ Boot"),
    }
}

fn render_table(
    frame: &mut Frame,
    area: Rect,
    m: &Metrics,
    ifaces: &[&IfaceStat],
    tm: &TaskManagerState,
    accent: Color,
    btype: BorderType,
) {
    let focused = tm.focus == Focus::Content;
    let border = if focused { Style::default().fg(accent) } else { Style::default().fg(Color::DarkGray) };
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(btype)
        .border_style(border)
        .title(Span::styled(" Interfaces ", border.add_modifier(Modifier::BOLD)));
    let inner = b.inner(area);
    frame.render_widget(b, area);

    let tier = if inner.width >= 121 { 2 } else if inner.width >= 97 { 1 } else { 0 };
    let mut lines = vec![Line::from(Span::styled(table_header(tier), Style::default().fg(Color::DarkGray)))];

    let (rx_s, tx_s, rx_b, tx_b) = m.net_totals();
    let mut table = vec![TableRow {
        name: "All".into(),
        addr: String::new(),
        mac: String::new(),
        rx: m.net_rx.last().copied().unwrap_or(0.0),
        tx: m.net_tx.last().copied().unwrap_or(0.0),
        rx_session: rx_s,
        tx_session: tx_s,
        rx_boot: rx_b,
        tx_boot: tx_b,
        errors: ifaces.iter().map(|i| i.rx_errors + i.tx_errors).sum(),
    }];
    for i in ifaces {
        table.push(TableRow {
            name: super::clean_text(&i.name),
            addr: i.ips.first().cloned().unwrap_or_default(),
            mac: i.mac.clone(),
            rx: i.rx_rate.last().copied().unwrap_or(0.0),
            tx: i.tx_rate.last().copied().unwrap_or(0.0),
            rx_session: i.rx_session,
            tx_session: i.tx_session,
            rx_boot: i.rx_total,
            tx_boot: i.tx_total,
            errors: i.rx_errors + i.tx_errors,
        });
    }

    let visible = inner.height.saturating_sub(1) as usize;
    let scroll = tm.net_cursor.saturating_sub(visible.saturating_sub(1));
    for (idx, row) in table.iter().enumerate().skip(scroll).take(visible) {
        let style = if idx == tm.net_cursor && focused {
            Style::default().fg(Color::Black).bg(accent)
        } else if idx == tm.net_cursor {
            Style::default().fg(accent)
        } else if row.errors > 0 && idx > 0 {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(table_line(tier, row), style)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Rows in the interface table (the "All" row plus one per listed interface).
pub fn row_count() -> usize {
    metrics::read().shown_ifaces().count() + 1
}
