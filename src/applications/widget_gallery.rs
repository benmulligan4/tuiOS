/// Widget Gallery — a built-in showcase of Ratatui widget patterns.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, Chart, Dataset, GraphType, List, ListItem, ListState,
        Paragraph, Row, Table, Tabs,
    },
    Frame,
};

const DEMOS: &[(&str, &str)] = &[
    ("ScrollView", "tui-scrollview"),
    ("Tree View", "tui-tree-widget"),
    ("Code Editor", "ratatui-code-editor"),
    ("Text Area", "ratatui-textarea"),
    ("Logger", "tui-logger"),
    ("Popup", "tui-popup"),
    ("Overlay", "tui-overlay"),
    ("Notifications", "ratatui-comfy-toaster"),
    ("Tabs", "tui-tabs"),
    ("Widget List", "tui-widget-list"),
    ("Terminal", "tui-term"),
    ("Charts", "malevich"),
    ("Image", "ratatui-image"),
    ("Animation", "tachyonfx"),
];

pub struct WidgetGalleryApp {
    selected: usize,
    scroll_offset: u16,
    list_state: ListState,
    popup_visible: bool,
}

impl WidgetGalleryApp {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { selected: 0, scroll_offset: 0, list_state, popup_visible: false }
    }

    pub fn start(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
        self.popup_visible = false;
        self.list_state.select(Some(0));
    }

    pub fn stop(&mut self) {}

    pub fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Widget Gallery — ←→ choose, ↑↓ scroll/select, Enter popup, Q close ")
            .title_alignment(Alignment::Right)
            .style(border_style);
        frame.render_widget(block, area);

        let inner = area.inner(ratatui::layout::Margin { horizontal: 1, vertical: 1 });
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(20)])
            .split(inner);

        let items: Vec<ListItem> = DEMOS.iter().enumerate().map(|(i, (name, package))| {
            let prefix = if i == self.selected { "▶ " } else { "  " };
            ListItem::new(vec![
                Line::from(Span::styled(format!("{}{}", prefix, name), if i == self.selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                })),
                Line::from(Span::styled(format!("  {}", package), Style::default().fg(Color::DarkGray))),
            ])
        }).collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::RIGHT).title(" Demos "))
            .highlight_symbol("");
        frame.render_stateful_widget(list, columns[0], &mut self.list_state);

        let tabs = DEMOS.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        frame.render_widget(
            Tabs::new(tabs)
                .select(self.selected)
                .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD))
                .divider("│")
                .block(Block::default().borders(Borders::BOTTOM)),
            Rect::new(columns[1].x, columns[1].y, columns[1].width, 3),
        );

        let demo_area = Rect::new(columns[1].x, columns[1].y + 3, columns[1].width, columns[1].height.saturating_sub(3));
        self.render_demo(frame, demo_area);

        if self.popup_visible {
            let w = demo_area.width.saturating_mul(60).saturating_div(100).max(20).min(demo_area.width);
            let h = demo_area.height.saturating_mul(35).saturating_div(100).max(7).min(demo_area.height);
            let x = demo_area.x + (demo_area.width.saturating_sub(w)) / 2;
            let y = demo_area.y + (demo_area.height.saturating_sub(h)) / 2;
            let popup_area = Rect::new(x, y, w, h);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(" Popup / Overlay Demo", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                    Line::from(""),
                    Line::from(" This window can be drawn above the current widget."),
                    Line::from(" Press Enter or Escape to dismiss."),
                ])
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)).title(" Notification ")),
                popup_area,
            );
        }
    }

    fn render_demo(&mut self, frame: &mut Frame, area: Rect) {
        match self.selected {
            0 => self.demo_scroll(frame, area),
            1 => self.demo_tree(frame, area),
            2 => self.demo_code(frame, area),
            3 => self.demo_text_area(frame, area),
            4 => self.demo_logger(frame, area),
            5 | 6 | 7 => self.demo_popup(frame, area),
            8 => self.demo_tabs(frame, area),
            9 => self.demo_widget_list(frame, area),
            10 => self.demo_terminal(frame, area),
            11 => self.demo_chart(frame, area),
            12 => self.demo_image(frame, area),
            _ => self.demo_animation(frame, area),
        }
    }

    fn demo_scroll(&mut self, frame: &mut Frame, area: Rect) {
        let mut lines = vec![Line::from(Span::styled("Scrollable content", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))];
        for i in 1..=40 { lines.push(Line::from(format!("Line {:02}: a long scrollable widget can contain rich Ratatui content", i))); }
        frame.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" tui-scrollview / Scrollable Paragraph ")).scroll((self.scroll_offset, 0)), area);
    }

    fn demo_tree(&mut self, frame: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from("Applications"), Line::from("├── System"), Line::from("│   ├── Task Manager"),
            Line::from("│   └── Settings"), Line::from("├── Productivity"), Line::from("│   ├── Notes"),
            Line::from("│   └── Widget Gallery"), Line::from("└── Media"), Line::from("    └── Markdown Viewer"),
        ];
        frame.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" tui-tree-widget pattern ")), area);
    }

    fn demo_code(&mut self, frame: &mut Frame, area: Rect) {
        let code = vec![
            Line::from(Span::styled("fn main() {", Style::default().fg(Color::Yellow))),
            Line::from(vec![Span::styled("    let ", Style::default().fg(Color::Magenta)), Span::styled("widget", Style::default().fg(Color::Cyan)), Span::raw(" = "), Span::styled("\"ratatui\"", Style::default().fg(Color::Green)), Span::raw(";")]),
            Line::from(Span::styled("    println!(\"{widget}\");", Style::default().fg(Color::White))),
            Line::from(Span::styled("}", Style::default().fg(Color::Yellow))),
        ];
        frame.render_widget(Paragraph::new(code).block(Block::default().borders(Borders::ALL).title(" ratatui-code-editor — sample ")), area);
    }

    fn demo_text_area(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Paragraph::new(vec![Line::from("Example multiline text area"), Line::from(""), Line::from("  This text demonstrates a textarea surface."), Line::from("  [cursor █]  ")]).block(Block::default().borders(Borders::ALL).title(" ratatui-textarea / editing state ")), area);
    }

    fn demo_logger(&mut self, frame: &mut Frame, area: Rect) {
        let rows = vec![
            Row::new(["15:18:01", "INFO", "Widget Gallery opened"]),
            Row::new(["15:18:02", "DEBUG", "Demo selected: Logger"]),
            Row::new(["15:18:03", "WARN", "Example warning entry"]),
            Row::new(["15:18:04", "INFO", "tui-logger view refreshed"]),
        ];
        frame.render_widget(Table::new(rows, [Constraint::Length(10), Constraint::Length(7), Constraint::Min(12)])
            .header(Row::new(["Time", "Level", "Message"]).style(Style::default().add_modifier(Modifier::BOLD)))
            .block(Block::default().borders(Borders::ALL).title(" tui-logger style viewer ")), area);
    }

    fn demo_popup(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Paragraph::new(vec![
            Line::from(Span::styled("Overlay and notification patterns", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
            Line::from(""), Line::from("Press Enter to open a popup."), Line::from("A toast can appear independently in a screen corner."),
            Line::from("┌──────────────────────────────┐"), Line::from("│ ✓ Widget installed           │"), Line::from("└──────────────────────────────┘"),
        ]).block(Block::default().borders(Borders::ALL).title(" tui-popup / tui-overlay / toaster ")), area);
    }

    fn demo_tabs(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Tabs::new(["General", "Appearance", "Network", "Advanced"])
            .select(1).highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title(" tui-tabs / comfy-tabs ")), area);
    }

    fn demo_widget_list(&mut self, frame: &mut Frame, area: Rect) {
        let items = vec![ListItem::new("Widget item one"), ListItem::new("Widget item two"), ListItem::new("Widget item three")];
        frame.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(" tui-widget-list pattern ")), area);
    }

    fn demo_terminal(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Paragraph::new(vec![Line::from("user@tuios:~$ ls"), Line::from(" applications  examples  src  Cargo.toml"), Line::from("user@tuios:~$ ")]).block(Block::default().borders(Borders::ALL).title(" tui-term embedded session demo ")), area);
    }

    fn demo_chart(&mut self, frame: &mut Frame, area: Rect) {
        let dataset = Dataset::default().name("CPU").marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Cyan))
            .data(&[(0.0, 10.0), (1.0, 28.0), (2.0, 18.0), (3.0, 62.0), (4.0, 42.0), (5.0, 75.0)]);
        frame.render_widget(Chart::new(vec![dataset]).block(Block::default().borders(Borders::ALL).title(" malevich-style metrics chart ")).x_axis(ratatui::widgets::Axis::default().title("Time").bounds([0.0, 5.0])        .labels(vec![Line::from("0"), Line::from("2"), Line::from("5")])).y_axis(ratatui::widgets::Axis::default().title("%").bounds([0.0, 100.0])        .labels(vec![Line::from("0"), Line::from("50"), Line::from("100")])), area);
    }

    fn demo_image(&mut self, frame: &mut Frame, area: Rect) {
        let image = vec![
            "     ▄▄▄▄▄     ", "   ▄███████▄   ", "  ██░░▀█▀░░██  ", "  ██░░███░░██  ", "   ▀███████▀   ", "     ▀▀▀▀▀     ",
        ];
        frame.render_widget(Paragraph::new(image.join("\n")).alignment(Alignment::Center).block(Block::default().borders(Borders::ALL).title(" ratatui-image / Unicode fallback ")), area);
    }

    fn demo_animation(&mut self, frame: &mut Frame, area: Rect) {
        let frames = ["◐", "◓", "◑", "◒"];
        let frame_index = (self.scroll_offset as usize) % frames.len();
        frame.render_widget(Paragraph::new(vec![Line::from(""), Line::from(Span::styled(format!("{} Loading widget...", frames[frame_index]), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))), Line::from("Fade • slide • blink • progress are tachyonfx-friendly effects")]).alignment(Alignment::Center).block(Block::default().borders(Borders::ALL).title(" tachyonfx animation pattern ")), area);
    }

    pub fn handle_key(&mut self, code: &str) -> Option<WidgetGalleryAction> {
        if self.popup_visible {
            if matches!(code, "Enter" | "Esc" | "Escape") {
                self.popup_visible = false;
                return None;
            }
        }
        match code {
            "q" | "Q" | "Backspace" | "Escape" | "Esc" => Some(WidgetGalleryAction::Back),
            "Left" | "a" => self.select_previous(),
            "Right" | "d" => self.select_next(),
            "Up" | "w" => { self.scroll_offset = self.scroll_offset.saturating_sub(1); None }
            "Down" | "s" => { self.scroll_offset = self.scroll_offset.saturating_add(1); None }
            "Enter" => { self.popup_visible = true; None }
            _ => None,
        }
    }

    fn select_previous(&mut self) -> Option<WidgetGalleryAction> {
        self.selected = if self.selected == 0 { DEMOS.len() - 1 } else { self.selected - 1 };
        self.scroll_offset = 0;
        self.list_state.select(Some(self.selected));
        None
    }

    fn select_next(&mut self) -> Option<WidgetGalleryAction> {
        self.selected = (self.selected + 1) % DEMOS.len();
        self.scroll_offset = 0;
        self.list_state.select(Some(self.selected));
        None
    }
}

#[derive(Debug, Clone)]
pub enum WidgetGalleryAction { Back }
