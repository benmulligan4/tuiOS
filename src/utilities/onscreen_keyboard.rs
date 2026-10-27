/// On-Screen Keyboard — reusable QWERTY keyboard component.
///
/// Navigated by arrow keys / WASD. Enter/E to press the selected key.
/// Q closes the keyboard. Includes Backspace, Space, cursor left/right,
/// Shift, and Enter keys.
///
/// This component can be embedded into any tuiOS app or used with
/// third-party dashboards by rendering it in a sub-area of the frame.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Keys on the keyboard — either a character or a special action.
#[derive(Debug, Clone, PartialEq)]
pub enum OskKey {
    Char(char),
    Backspace,
    Space,
    Enter,
    CursorLeft,
    CursorRight,
    Shift,
    Tab,
}

impl OskKey {
    /// Display label for the key.
    fn label(&self, shifted: bool) -> &str {
        match self {
            OskKey::Char(_) => "", // handled separately
            OskKey::Backspace => "Bksp ⌫",
            OskKey::Space => "Space",
            OskKey::Enter => "Enter ↵",
            OskKey::CursorLeft => " ◂ ",
            OskKey::CursorRight => " ▸ ",
            OskKey::Shift => if shifted { "⇧ SHIFT" } else { "⇧ Shift" },
            OskKey::Tab => "Tab ⇥",
        }
    }

    /// Width of the key cell in characters (including padding).
    fn width(&self) -> u16 {
        match self {
            OskKey::Char(_) => 5,
            OskKey::Backspace => 10,
            OskKey::Space => 25,
            OskKey::Enter => 10,
            OskKey::CursorLeft | OskKey::CursorRight => 5,
            OskKey::Shift => 10,
            OskKey::Tab => 8,
        }
    }
}

/// Result of pressing a key on the OSK.
#[derive(Debug, Clone)]
pub enum OskResult {
    /// A character was typed.
    Char(char),
    /// Backspace — delete character before cursor.
    Backspace,
    /// Enter / newline.
    Enter,
    /// Move text cursor left.
    CursorLeft,
    /// Move text cursor right.
    CursorRight,
    /// Close the keyboard.
    Close,
    /// No action (internal state change like shift).
    None,
}

/// The keyboard layout rows.
fn build_rows() -> Vec<Vec<OskKey>> {
    vec![
        // Row 0: number row
        vec![
            OskKey::Char('1'), OskKey::Char('2'), OskKey::Char('3'),
            OskKey::Char('4'), OskKey::Char('5'), OskKey::Char('6'),
            OskKey::Char('7'), OskKey::Char('8'), OskKey::Char('9'),
            OskKey::Char('0'), OskKey::Char('-'), OskKey::Char('='),
            OskKey::Backspace,
        ],
        // Row 1: QWERTY row
        vec![
            OskKey::Tab,
            OskKey::Char('q'), OskKey::Char('w'), OskKey::Char('e'),
            OskKey::Char('r'), OskKey::Char('t'), OskKey::Char('y'),
            OskKey::Char('u'), OskKey::Char('i'), OskKey::Char('o'),
            OskKey::Char('p'), OskKey::Char('['), OskKey::Char(']'),
            OskKey::Char('\\'),
        ],
        // Row 2: home row
        vec![
            OskKey::Shift,
            OskKey::Char('a'), OskKey::Char('s'), OskKey::Char('d'),
            OskKey::Char('f'), OskKey::Char('g'), OskKey::Char('h'),
            OskKey::Char('j'), OskKey::Char('k'), OskKey::Char('l'),
            OskKey::Char(';'), OskKey::Char('\''),
            OskKey::Enter,
        ],
        // Row 3: bottom row
        vec![
            OskKey::Char('z'), OskKey::Char('x'), OskKey::Char('c'),
            OskKey::Char('v'), OskKey::Char('b'), OskKey::Char('n'),
            OskKey::Char('m'), OskKey::Char(','), OskKey::Char('.'),
            OskKey::Char('/'),
        ],
        // Row 4: space bar & navigation
        vec![
            OskKey::CursorLeft,
            OskKey::Space,
            OskKey::CursorRight,
        ],
    ]
}

/// Shifted equivalents for character keys.
fn shift_char(c: char) -> char {
    match c {
        '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%',
        '6' => '^', '7' => '&', '8' => '*', '9' => '(', '0' => ')',
        '-' => '_', '=' => '+',
        '[' => '{', ']' => '}', '\\' => '|',
        ';' => ':', '\'' => '"',
        ',' => '<', '.' => '>', '/' => '?',
        c if c.is_ascii_lowercase() => c.to_ascii_uppercase(),
        c => c,
    }
}

#[derive(Debug, Clone)]
pub struct OnScreenKeyboard {
    rows: Vec<Vec<OskKey>>,
    /// Currently selected row.
    pub cursor_row: usize,
    /// Currently selected column within that row.
    pub cursor_col: usize,
    /// Shift active.
    pub shifted: bool,
    /// Whether the keyboard is visible/active.
    pub visible: bool,
}

impl OnScreenKeyboard {
    pub fn new() -> Self {
        Self {
            rows: build_rows(),
            cursor_row: 1,
            cursor_col: 1, // Start on 'q'
            shifted: false,
            visible: false,
        }
    }

    pub fn show(&mut self) {
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Handle a navigation/activation key. Returns the resulting action.
    /// Navigation: Up/Down/Left/Right (arrow keys — WASD is intercepted
    /// by the caller before reaching here since W/A/S/D are keyboard keys).
    pub fn handle_key(&mut self, code: &str) -> OskResult {
        match code {
            "q" => {
                self.hide();
                return OskResult::Close;
            }
            "Up" => {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.clamp_col();
                }
            }
            "Down" => {
                if self.cursor_row < self.rows.len() - 1 {
                    self.cursor_row += 1;
                    self.clamp_col();
                }
            }
            "Left" => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            "Right" => {
                if self.cursor_col < self.rows[self.cursor_row].len() - 1 {
                    self.cursor_col += 1;
                }
            }
            "Enter" => {
                return self.press_selected();
            }
            _ => {}
        }
        OskResult::None
    }

    /// Press the currently selected key.
    fn press_selected(&mut self) -> OskResult {
        let key = &self.rows[self.cursor_row][self.cursor_col];
        match key {
            OskKey::Char(c) => {
                let ch = if self.shifted { shift_char(*c) } else { *c };
                // Auto-release shift after one character (like phone keyboards)
                self.shifted = false;
                OskResult::Char(ch)
            }
            OskKey::Backspace => OskResult::Backspace,
            OskKey::Space => OskResult::Char(' '),
            OskKey::Enter => OskResult::Enter,
            OskKey::CursorLeft => OskResult::CursorLeft,
            OskKey::CursorRight => OskResult::CursorRight,
            OskKey::Shift => {
                self.shifted = !self.shifted;
                OskResult::None
            }
            OskKey::Tab => OskResult::Char('\t'),
        }
    }

    fn clamp_col(&mut self) {
        let max = self.rows[self.cursor_row].len().saturating_sub(1);
        if self.cursor_col > max {
            self.cursor_col = max;
        }
    }

    /// Render the keyboard in the given area.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Keyboard ─ Q close ")
            .title_alignment(Alignment::Right)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(block, area);
        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

        // Each keyboard row gets 1 line of height
        let row_count = self.rows.len();
        let mut constraints: Vec<Constraint> = Vec::with_capacity(row_count + 1);
        for _ in 0..row_count {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Fill(1)); // absorb remaining

        let row_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        // Staggered indent per row (like a real keyboard)
        let row_indents = [" ", "  ", "   ", "    ", "  "];

        for (r, row_keys) in self.rows.iter().enumerate() {
            let mut spans: Vec<Span> = Vec::new();

            // Row indent
            let indent = row_indents.get(r).unwrap_or(&" ");
            spans.push(Span::raw(*indent));

            for (c, key) in row_keys.iter().enumerate() {
                let selected = r == self.cursor_row && c == self.cursor_col;

                let label = match key {
                    OskKey::Char(ch) => {
                        let display_ch = if self.shifted {
                            shift_char(*ch)
                        } else {
                            *ch
                        };
                        format!(" {} ", display_ch)
                    }
                    other => {
                        let w = other.width() as usize;
                        let text = other.label(self.shifted);
                        let text_len = text.chars().count();
                        let total_pad = w.saturating_sub(text_len);
                        let left_pad = total_pad / 2;
                        let right_pad = total_pad - left_pad;
                        format!(
                            "{}{}{}",
                            " ".repeat(left_pad),
                            text,
                            " ".repeat(right_pad),
                        )
                    }
                };

                let (fg, bg) = if selected {
                    (Color::Black, Color::Cyan)
                } else {
                    match key {
                        OskKey::Char(_) => (Color::White, Color::Indexed(236)),
                        _ => (Color::Yellow, Color::Indexed(236)),
                    }
                };

                let mut style = Style::default().fg(fg).bg(bg);
                if selected {
                    style = style.add_modifier(Modifier::BOLD);
                }

                spans.push(Span::styled(label, style));
                // Gap between keys
                spans.push(Span::raw(" "));
            }

            let line = Line::from(spans);
            frame.render_widget(
                Paragraph::new(Text::from(line)),
                row_areas[r],
            );
        }
    }

    /// Height needed to render the keyboard (rows + border).
    pub fn height(&self) -> u16 {
        self.rows.len() as u16 + 2 // +2 for top/bottom border
    }
}
