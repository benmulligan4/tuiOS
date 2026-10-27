/// Dashboard 2 — System Stats (CPU, memory, disk).
///
/// tuiOS dashboard API:
///     render(frame, area)  — draw the dashboard inside the given area

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

fn bar(used: f64, total: f64, width: usize) -> String {
    let pct = if total == 0.0 { 0.0 } else { used / total };
    let filled = (pct * width as f64) as usize;
    let empty = width.saturating_sub(filled);
    let bar_str: String = "█".repeat(filled) + &"░".repeat(empty);
    format!("[{}] {:.1}%", bar_str, pct * 100.0)
}

fn color_for_pct(pct: f64) -> Color {
    if pct >= 85.0 {
        Color::Red
    } else if pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

#[allow(dead_code)]
fn fmt_bytes(n: u64) -> String {
    let mut val = n as f64;
    for unit in &["B", "KB", "MB", "GB", "TB"] {
        if val < 1024.0 {
            return format!("{:.1} {}", val, unit);
        }
        val /= 1024.0;
    }
    format!("{:.1} PB", val)
}

pub fn render(frame: &mut Frame, area: Rect, border_style: Style) {
    let (cpu_pct, mem_used, mem_total, disk_used, disk_total) = {
        let m = crate::task_manager::metrics::read();
        (m.cpu_now() as f64, m.mem_now(), m.mem_total, m.disk_used, m.disk_total)
    };
    let mem_pct = if mem_total > 0 {
        (mem_used as f64 / mem_total as f64) * 100.0
    } else {
        0.0
    };
    let disk_pct = if disk_total > 0 {
        (disk_used as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    // Single border with title on top-right
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Dashboard 2 — System Stats ")
        .title_alignment(Alignment::Right)
        .style(border_style);
    frame.render_widget(block, area);
    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Fill(1),
        ])
        .split(inner);

    let cpu_bar = bar(cpu_pct, 100.0, 20);
    let cpu_text = format!("  CPU      {}   {:.1}%", cpu_bar, cpu_pct);

    let mem_used_gb = mem_used as f64 / (1024.0 * 1024.0 * 1024.0);
    let mem_total_gb = mem_total as f64 / (1024.0 * 1024.0 * 1024.0);
    let mem_bar = bar(mem_used as f64, mem_total as f64, 20);
    let mem_text = format!(
        "  Memory   {}   {:.1} GB / {:.1} GB",
        mem_bar, mem_used_gb, mem_total_gb
    );

    let disk_used_gb = disk_used as f64 / (1024.0 * 1024.0 * 1024.0);
    let disk_total_gb = disk_total as f64 / (1024.0 * 1024.0 * 1024.0);
    let disk_bar = bar(disk_used as f64, disk_total as f64, 20);
    let disk_text = format!(
        "  Disk     {}   {:.1} GB / {:.1} GB",
        disk_bar, disk_used_gb, disk_total_gb
    );

    frame.render_widget(
        Paragraph::new(Text::raw(cpu_text))
            .style(Style::default().fg(color_for_pct(cpu_pct))),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Text::raw(mem_text))
            .style(Style::default().fg(color_for_pct(mem_pct))),
        rows[3],
    );
    frame.render_widget(
        Paragraph::new(Text::raw(disk_text))
            .style(Style::default().fg(color_for_pct(disk_pct))),
        rows[5],
    );
}
