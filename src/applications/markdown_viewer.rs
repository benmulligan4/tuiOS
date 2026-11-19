/// Markdown Viewer — internal tuiOS app powered by `tui-markdown`.

use std::fs;
use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub struct MarkdownViewerApp {
    title: String,
    markdown: String,
    text: Text<'static>,
    scroll_offset: u16,
}

impl MarkdownViewerApp {
    pub fn new() -> Self {
        Self {
            title: "README.md".to_string(),
            markdown: String::new(),
            text: Text::raw(""),
            scroll_offset: 0,
        }
    }

    pub fn start(&mut self) {
        self.open_example();
    }

    pub fn stop(&mut self) {}

    pub fn open_example(&mut self) {
        let sample_path = PathBuf::from("examples").join("markdown-viewer.md");
        let markdown = fs::read_to_string(&sample_path)
            .unwrap_or_else(|_| include_str!("../../examples/markdown-viewer.md").to_string());
        self.open_path(marked_file_name(&sample_path), markdown);
    }

    pub fn open_path(&mut self, name: String, markdown: String) {
        self.title = name;
        self.markdown = markdown;
        let rendered = tui_markdown::from_str(&self.markdown);
        self.text = Text::raw(rendered.to_string());
        self.scroll_offset = 0;
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} — ↑↓ / PgUp PgDn scroll, Q closes ", self.title))
            .title_alignment(Alignment::Right)
            .style(border_style);
        frame.render_widget(block, area);

        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
        let rendered = self.text.clone();
        let line_count = rendered.lines.len().saturating_add(1) as u16;
        let max_offset = line_count.saturating_sub(inner.height);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }

        frame.render_widget(
            Paragraph::new(rendered)
                .style(Style::default().fg(Color::White))
                .scroll((self.scroll_offset, 0)),
            inner,
        );
    }

    pub fn handle_key(&mut self, code: &str) -> Option<MarkdownViewerAction> {
        match code {
            "q" | "Q" | "Backspace" | "Esc" | "Escape" => Some(MarkdownViewerAction::Back),
            "Up" | "w" => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                None
            }
            "Down" | "s" => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                None
            }
            "PageUp" => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                None
            }
            "PageDown" => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                None
            }
            "Home" => {
                self.scroll_offset = 0;
                None
            }
            _ => None,
        }
    }
}

fn marked_file_name(path: &PathBuf) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("README.md")
        .to_string()
}

#[derive(Debug, Clone)]
pub enum MarkdownViewerAction {
    Back,
}
