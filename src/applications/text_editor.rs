/// Text Editor — internal tuiOS app.
///
/// A simple text editor that uses the On-Screen Keyboard for input.
/// Navigate to the text area and press Enter/E to open the keyboard.
/// Arrow keys navigate the keyboard; typed characters appear in the editor.
/// Q closes the keyboard. Backspace/Q on the editor itself closes the app.
///
/// This serves as the reference example for how other apps (built-in or
/// third-party) can integrate the On-Screen Keyboard component.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::utilities::onscreen_keyboard::{OnScreenKeyboard, OskResult};

pub struct TextEditorApp {
    /// All text lines.
    lines: Vec<String>,
    /// Cursor row in the text.
    cursor_row: usize,
    /// Cursor column (byte offset within the line).
    cursor_col: usize,
    /// Vertical scroll offset.
    scroll_offset: usize,
    /// The on-screen keyboard.
    pub osk: OnScreenKeyboard,
    /// Direct typing mode (when OSK is disabled in settings).
    /// In this mode, physical keyboard chars are inserted directly.
    pub typing_mode: bool,
}

impl TextEditorApp {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            osk: OnScreenKeyboard::new(),
            typing_mode: false,
        }
    }

    pub fn start(&mut self) {
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.osk = OnScreenKeyboard::new();
        self.typing_mode = false;
    }

    pub fn stop(&mut self) {}

    /// Returns true if the keyboard/typing mode is active and consuming keys.
    pub fn keyboard_active(&self) -> bool {
        self.osk.visible || self.typing_mode
    }

    // ----- Text manipulation -----

    pub fn insert_char(&mut self, ch: char) {
        if ch == '\t' {
            // Insert 4 spaces for tab
            for _ in 0..4 {
                self.lines[self.cursor_row].insert(self.cursor_col, ' ');
                self.cursor_col += 1;
            }
            return;
        }
        self.lines[self.cursor_row].insert(self.cursor_col, ch);
        self.cursor_col += ch.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        let rest = self.lines[self.cursor_row][self.cursor_col..].to_string();
        self.lines[self.cursor_row].truncate(self.cursor_col);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, rest);
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            // Find the previous char boundary
            let prev = self.lines[self.cursor_row][..self.cursor_col]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.lines[self.cursor_row].remove(prev);
            self.cursor_col = prev;
        } else if self.cursor_row > 0 {
            // Merge with previous line
            let current = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current);
        }
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_col > 0 {
            let prev = self.lines[self.cursor_row][..self.cursor_col]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_col = prev;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
    }

    fn move_cursor_right(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            let next = self.lines[self.cursor_row][self.cursor_col..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_col + i)
                .unwrap_or(line_len);
            self.cursor_col = next;
        } else if self.cursor_row < self.lines.len() - 1 {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    fn move_cursor_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            let line_len = self.lines[self.cursor_row].len();
            if self.cursor_col > line_len {
                self.cursor_col = line_len;
            }
        }
    }

    fn move_cursor_down(&mut self) {
        if self.cursor_row < self.lines.len() - 1 {
            self.cursor_row += 1;
            let line_len = self.lines[self.cursor_row].len();
            if self.cursor_col > line_len {
                self.cursor_col = line_len;
            }
        }
    }

    // ----- Key handling -----

    /// Handle a key code. Returns Some(TextEditorAction) for actions
    /// the caller should handle (like closing the app).
    pub fn handle_key(&mut self, code: &str) -> Option<TextEditorAction> {
        if self.osk.visible {
            // Keyboard is active — forward to OSK
            let result = self.osk.handle_key(code);
            match result {
                OskResult::Char(ch) => self.insert_char(ch),
                OskResult::Backspace => self.backspace(),
                OskResult::Enter => self.insert_newline(),
                OskResult::CursorLeft => self.move_cursor_left(),
                OskResult::CursorRight => self.move_cursor_right(),
                OskResult::Close => { /* keyboard hidden by OSK */ }
                OskResult::None => {}
            }
            None
        } else if self.typing_mode {
            // Direct typing mode — physical keyboard chars go into editor
            match code {
                "q" | "Q" => {
                    // Esc/Q exits typing mode (back to navigation)
                    self.typing_mode = false;
                    None
                }
                "Up" => { self.move_cursor_up(); None }
                "Down" => { self.move_cursor_down(); None }
                "Left" => { self.move_cursor_left(); None }
                "Right" => { self.move_cursor_right(); None }
                "Enter" => { self.insert_newline(); None }
                _ => None, // Single chars handled via raw key forwarding below
            }
        } else {
            // Keyboard not active — editor navigation mode
            match code {
                "q" | "Q" | "Backspace" => Some(TextEditorAction::Back),
                "Up" => { self.move_cursor_up(); None }
                "Down" => { self.move_cursor_down(); None }
                "Left" => { self.move_cursor_left(); None }
                "Right" => { self.move_cursor_right(); None }
                "Enter" => {
                    // Check if OSK is enabled in settings
                    let settings = crate::settings::persistence::load();
                    let osk_enabled = crate::settings::persistence::get_bool(
                        &settings, "utilities.onscreen_keyboard_enabled", false
                    );
                    if osk_enabled {
                        self.osk.show();
                    } else {
                        self.typing_mode = true;
                    }
                    None
                }
                _ => None,
            }
        }
    }

    // ----- Rendering -----

    pub fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        if self.osk.visible {
            // Split: top for text editor, bottom for keyboard
            let osk_h = self.osk.height();
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(osk_h),
                ])
                .split(area);

            self.render_editor(frame, rows[0], border_style);
            self.osk.render(frame, rows[1]);
        } else {
            self.render_editor(frame, area, border_style);
        }
    }

    fn render_editor(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        let title = if self.osk.visible {
            " Text Editor — typing... "
        } else {
            " Text Editor — Enter to type, Q to close "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_alignment(Alignment::Right)
            .style(border_style);
        frame.render_widget(block, area);

        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let visible_rows = inner.height as usize;

        // Adjust scroll to keep cursor visible
        if self.cursor_row < self.scroll_offset {
            self.scroll_offset = self.cursor_row;
        }
        if self.cursor_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.cursor_row - visible_rows + 1;
        }

        let end_row = (self.scroll_offset + visible_rows).min(self.lines.len());

        let mut text_lines: Vec<Line> = Vec::new();

        for r in self.scroll_offset..end_row {
            let line_str = &self.lines[r];
            let max_width = inner.width as usize;

            if r == self.cursor_row {
                // Render line with cursor highlight
                let (before, cursor_ch, after) = split_at_cursor(line_str, self.cursor_col);

                let before_display = truncate_str(&before, max_width);
                let remaining = max_width.saturating_sub(before_display.len());

                let cursor_display = if remaining > 0 {
                    truncate_str(&cursor_ch, remaining)
                } else {
                    String::new()
                };

                let remaining2 = remaining.saturating_sub(cursor_display.len());
                let after_display = if remaining2 > 0 {
                    truncate_str(&after, remaining2)
                } else {
                    String::new()
                };

                text_lines.push(Line::from(vec![
                    Span::styled(before_display, Style::default().fg(Color::White)),
                    Span::styled(
                        cursor_display,
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(after_display, Style::default().fg(Color::White)),
                ]));
            } else {
                let display = truncate_str(line_str, max_width);
                text_lines.push(Line::from(Span::styled(
                    display,
                    Style::default().fg(Color::White),
                )));
            }
        }

        // Fill remaining lines
        for _ in text_lines.len()..visible_rows {
            text_lines.push(Line::from(Span::styled(
                "~",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let paragraph = Paragraph::new(Text::from(text_lines));
        frame.render_widget(paragraph, inner);
    }
}

/// Split a string at byte position `pos` into (before, cursor_char, after).
fn split_at_cursor(s: &str, pos: usize) -> (String, String, String) {
    let pos = pos.min(s.len());
    let before = s[..pos].to_string();

    if pos >= s.len() {
        // Cursor is at the end — show a block cursor on empty space
        return (before, " ".to_string(), String::new());
    }

    // Find the char at cursor position
    let rest = &s[pos..];
    let ch_len = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    let cursor_ch = s[pos..pos + ch_len].to_string();
    let after = s[pos + ch_len..].to_string();

    (before, cursor_ch, after)
}

/// Truncate a string to at most `max_width` characters.
fn truncate_str(s: &str, max_width: usize) -> String {
    s.chars().take(max_width).collect()
}

#[derive(Debug, Clone)]
pub enum TextEditorAction {
    Back,
}
