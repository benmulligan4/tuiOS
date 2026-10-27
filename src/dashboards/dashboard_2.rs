/// Dashboard 2 — Clock & Date.
///
/// tuiOS dashboard API:
///     render(frame, area)  — draw the dashboard inside the given area

use chrono::Local;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, border_style: Style) {
    let now = Local::now();
    let time_str = now.format("%H:%M:%S").to_string();
    let date_str = now.format("%A, %B %d %Y").to_string();

    // Single border with title on top-right
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Dashboard 2 — Clock & Date ")
        .title_alignment(Alignment::Right)
        .style(border_style);
    frame.render_widget(block, area);
    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

    // Split vertically: top padding | time | gap | date | bottom padding
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .split(inner);

    // Time — large, cyan
    let time_paragraph = Paragraph::new(Text::raw(time_str))
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center);
    frame.render_widget(time_paragraph, rows[1]);

    // Date — white
    let date_paragraph = Paragraph::new(Text::raw(date_str))
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);
    frame.render_widget(date_paragraph, rows[3]);
}
