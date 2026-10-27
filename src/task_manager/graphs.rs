/// Shared drawing helpers for the Task Manager: time-series graphs (braille line charts
/// or block sparklines), scaled previews and number formatting.

use std::time::Duration;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Chart, Dataset, GraphType, Paragraph, Sparkline},
    Frame,
};

use super::metrics::Ring;

/// One line on a graph: values oldest → newest, None for gaps.
pub struct Series {
    pub name: String,
    pub color: Color,
    pub values: Vec<Option<f64>>,
}

/// The newest `n` samples of a ring, padded at the front so the graph scrolls in from the right.
pub fn window<T: Copy>(ring: &Ring<T>, n: usize, f: impl Fn(T) -> Option<f64>) -> Vec<Option<f64>> {
    let mut out: Vec<Option<f64>> = Vec::with_capacity(n);
    let have = ring.len().min(n);
    out.resize(n - have, None);
    out.extend(ring.tail(n).map(|v| f(*v)));
    out
}

/// Highest value across all series (for auto-scaling byte graphs).
pub fn peak(series: &[Series]) -> f64 {
    series
        .iter()
        .flat_map(|s| s.values.iter().flatten())
        .fold(0.0_f64, |a, b| a.max(*b))
}

/// Squeeze `values` into `width` columns, keeping each bucket's highest value.
fn resample(values: &[Option<f64>], width: usize) -> Vec<Option<f64>> {
    if width == 0 || values.len() <= width {
        return values.to_vec();
    }
    (0..width)
        .map(|i| {
            let start = i * values.len() / width;
            let end = ((i + 1) * values.len() / width).max(start + 1);
            values[start..end].iter().flatten().fold(None, |acc: Option<f64>, v| Some(acc.map_or(*v, |a| a.max(*v))))
        })
        .collect()
}

/// Draw a time-series graph. `line` picks a braille chart, otherwise one sparkline per
/// series is stacked. `max` is the top of the scale; `fmt` labels the y axis.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    max: f64,
    line: bool,
    window_secs: u64,
    fmt: &dyn Fn(f64) -> String,
) {
    if area.width < 4 || area.height < 1 || series.is_empty() {
        return;
    }
    let max = if max > 0.0 { max } else { 1.0 };
    if line && area.height >= 3 {
        draw_chart(frame, area, series, max, window_secs, fmt);
    } else {
        draw_sparklines(frame, area, series, max);
    }
}

fn draw_chart(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    max: f64,
    window_secs: u64,
    fmt: &dyn Fn(f64) -> String,
) {
    // Each unbroken run becomes its own dataset so gaps stay gaps
    let mut runs: Vec<(Color, Vec<(f64, f64)>)> = Vec::new();
    let mut len = 1usize;
    for s in series {
        len = len.max(s.values.len());
        let mut run: Vec<(f64, f64)> = Vec::new();
        for (i, v) in s.values.iter().enumerate() {
            match v {
                Some(v) => run.push((i as f64, v.min(max).max(0.0))),
                None if !run.is_empty() => runs.push((s.color, std::mem::take(&mut run))),
                None => {}
            }
        }
        if !run.is_empty() {
            runs.push((s.color, run));
        }
    }
    let datasets: Vec<Dataset> = runs
        .iter()
        .map(|(color, points)| {
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(points)
        })
        .collect();

    let roomy = area.width >= 30 && area.height >= 5;
    let axis_style = Style::default().fg(Color::DarkGray);
    let mut x_axis = Axis::default().bounds([0.0, (len.max(2) - 1) as f64]).style(axis_style);
    let mut y_axis = Axis::default().bounds([0.0, max]).style(axis_style);
    if roomy {
        x_axis = x_axis.labels(vec![Span::raw(format!("-{}", short_duration(window_secs))), Span::raw("now")]);
        y_axis = y_axis.labels(vec![Span::raw("0"), Span::raw(fmt(max))]);
    }
    frame.render_widget(Chart::new(datasets).x_axis(x_axis).y_axis(y_axis), area);
}

fn draw_sparklines(frame: &mut Frame, area: Rect, series: &[Series], max: f64) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, series.len() as u32); series.len()])
        .split(area);
    for (s, rect) in series.iter().zip(rows.iter()) {
        if rect.height == 0 {
            continue;
        }
        let values = resample(&s.values, rect.width as usize);
        let data: Vec<Option<u64>> = values
            .iter()
            .map(|v| v.map(|v| ((v / max).clamp(0.0, 1.0) * 1000.0) as u64))
            .collect();
        frame.render_widget(
            Sparkline::default().data(&data).max(1000).style(Style::default().fg(s.color)),
            *rect,
        );
    }
}

/// Coloured "● name" legend entries on one line.
pub fn legend(series: &[(String, Color)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (name, color) in series {
        spans.push(Span::styled("● ", Style::default().fg(*color)));
        spans.push(Span::styled(format!("{}  ", name), Style::default().fg(Color::Gray)));
    }
    Line::from(spans)
}

pub fn text(frame: &mut Frame, area: Rect, line: Line<'static>) {
    frame.render_widget(Paragraph::new(line), area);
}

/// Green → yellow → red as a percentage climbs.
pub fn heat(pct: f64) -> Color {
    if pct >= 85.0 {
        Color::Red
    } else if pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

// ── Formatting ───────────────────────────────────────────────────────────────

pub fn bytes(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 {
            return if unit == "B" { format!("{} B", n) } else { format!("{:.1} {}", v, unit) };
        }
        v /= 1024.0;
    }
    format!("{:.1} PB", v)
}

pub fn rate(bytes_per_sec: f64) -> String {
    format!("{}/s", bytes(bytes_per_sec.max(0.0) as u64))
}

/// 3725 s → "1:02:05"; under an hour → "2:05".
pub fn duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn short_duration(secs: u64) -> String {
    if secs >= 60 && secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

pub fn pct(v: f64) -> String {
    format!("{:.0}%", v)
}

// ── Previews ─────────────────────────────────────────────────────────────────

/// The colour a cell contributes to a minimap: text shows as its foreground, blank
/// cells as their background (None = nothing there).
fn cell_colour(cell: &ratatui::buffer::Cell) -> Option<Color> {
    let style = cell.style();
    if cell.symbol().trim().is_empty() {
        style.bg.filter(|c| *c != Color::Reset)
    } else {
        Some(style.fg.filter(|c| *c != Color::Reset).unwrap_or(Color::Gray))
    }
}

/// Copy `src` into `area` of `dst`: 1:1 when it fits, otherwise a half-block minimap
/// (two source rows per cell) — shrunken text is unreadable anyway, while the minimap
/// still shows the app's layout and colours.
pub fn blit_minimap(src: &Buffer, dst: &mut Buffer, area: Rect) {
    let (sw, sh) = (src.area.width, src.area.height);
    if sw == 0 || sh == 0 || area.width == 0 || area.height == 0 {
        return;
    }
    if sw <= area.width && sh <= area.height {
        for y in 0..sh {
            for x in 0..sw {
                if let (Some(cell), Some(out)) = (
                    src.cell((src.area.x + x, src.area.y + y)),
                    dst.cell_mut((area.x + x, area.y + y)),
                ) {
                    out.set_symbol(cell.symbol()).set_style(cell.style());
                }
            }
        }
        return;
    }
    // Keep the aspect ratio: a cell is about twice as tall as it is wide
    let scale = (sw as f64 / area.width as f64)
        .max(sh as f64 / (area.height as f64 * 2.0))
        .max(1.0);
    let w = ((sw as f64 / scale) as u16).clamp(1, area.width);
    let h = ((sh as f64 / scale / 2.0).ceil() as u16).clamp(1, area.height);
    let sample = |x: u16, half_row: u16| -> Option<Color> {
        let sx = ((x as f64 * scale) as u16).min(sw - 1);
        let sy = ((half_row as f64 * scale) as u16).min(sh - 1);
        src.cell((src.area.x + sx, src.area.y + sy)).and_then(cell_colour)
    };
    for y in 0..h {
        for x in 0..w {
            let (top, bottom) = (sample(x, y * 2), sample(x, y * 2 + 1));
            let Some(out) = dst.cell_mut((area.x + x, area.y + y)) else { continue };
            match (top, bottom) {
                (None, None) => out.set_symbol(" ").set_style(Style::default()),
                (Some(t), None) => out.set_symbol("▀").set_style(Style::default().fg(t)),
                (None, Some(b)) => out.set_symbol("▄").set_style(Style::default().fg(b)),
                (Some(t), Some(b)) => out.set_symbol("▀").set_style(Style::default().fg(t).bg(b)),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(duration(Duration::from_secs(3725)), "1:02:05");
        assert_eq!(duration(Duration::from_secs(125)), "2:05");
        assert_eq!(short_duration(300), "5m");
        assert_eq!(short_duration(30), "30s");
    }

    #[test]
    fn resample_keeps_peaks() {
        let v = vec![Some(1.0), Some(5.0), None, Some(2.0)];
        assert_eq!(resample(&v, 2), vec![Some(5.0), Some(2.0)]);
    }

    #[test]
    fn window_pads_front() {
        let mut r = Ring::new(10);
        r.push(1.0_f32);
        r.push(2.0);
        let w = window(&r, 4, |v| Some(v as f64));
        assert_eq!(w, vec![None, None, Some(1.0), Some(2.0)]);
    }
}
