/// Installed app / dashboard runner — spawns third-party TUI apps in a PTY and
/// renders their output inside the tuiOS main container.
///
/// Uses `portable-pty` to create a pseudo-terminal sized to the container area,
/// and `vt100` to parse the terminal output into cells we can render with ratatui.
/// The child handle is kept so the Task Manager can see the PID, notice when the
/// app exits, and stop it.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders},
    Frame,
};
use vt100::Parser;

use super::super::tuios::models::Action;

/// Manages a running installed app or dashboard subprocess.
pub struct InstalledDashboard {
    label: String,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<Parser>>,
    /// Last known size so we can detect resizes
    last_cols: u16,
    last_rows: u16,
    /// Lines scrolled back from the live screen (0 = live).
    scroll: usize,
    exit_code: Option<u32>,
    _reader_handle: thread::JoinHandle<()>,
}

impl InstalledDashboard {
    /// Spawn the command in a PTY sized to `cols` x `rows`, keeping `scrollback`
    /// lines of history that scroll off the top.
    pub fn start(
        label: &str,
        cmd: &[String],
        cols: u16,
        rows: u16,
        scrollback: usize,
    ) -> Option<Self> {
        if cmd.is_empty() {
            return None;
        }
        let (cols, rows) = (cols.max(10), rows.max(3));

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok()?;

        let mut command = CommandBuilder::new(&cmd[0]);
        if cmd.len() > 1 {
            command.args(&cmd[1..]);
        }
        // Set TERM so the child knows it's in a terminal
        command.env("TERM", "xterm-256color");
        // Set columns/rows env vars some apps check
        command.env("COLUMNS", cols.to_string());
        command.env("LINES", rows.to_string());

        let child = pair.slave.spawn_command(command).ok()?;
        drop(pair.slave); // Close slave side in parent

        let parser = Arc::new(Mutex::new(Parser::new(rows, cols, scrollback)));
        let parser_clone = Arc::clone(&parser);

        let mut reader = pair.master.try_clone_reader().ok()?;
        let writer = pair.master.take_writer().ok()?;

        // Background thread reads PTY output and feeds it to the vt100 parser
        let reader_handle = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut p) = parser_clone.lock() {
                            p.process(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Some(Self {
            label: label.to_string(),
            writer,
            master: pair.master,
            child,
            parser,
            last_cols: cols,
            last_rows: rows,
            scroll: 0,
            exit_code: None,
            _reader_handle: reader_handle,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// (cols, rows) of the virtual terminal.
    pub fn size(&self) -> (u16, u16) {
        (self.last_cols, self.last_rows)
    }

    /// Exit code once the process has ended; stays set afterwards.
    pub fn poll_exit(&mut self) -> Option<u32> {
        if self.exit_code.is_none() {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exit_code = Some(status.exit_code());
            }
        }
        self.exit_code
    }

    /// Ask the app to quit the way a user would (Ctrl+C).
    pub fn interrupt(&mut self) {
        let _ = self.writer.write_all(b"\x03");
        let _ = self.writer.flush();
    }

    /// Force the process to stop.
    pub fn kill(&mut self) {
        if self.exit_code.is_none() {
            let _ = self.child.kill();
        }
    }

    /// Resize the PTY if the container area changed.
    pub fn resize_if_needed(&mut self, cols: u16, rows: u16) {
        if cols != self.last_cols || rows != self.last_rows {
            self.last_cols = cols;
            self.last_rows = rows;
            let _ = self.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
            if let Ok(mut p) = self.parser.lock() {
                p.set_size(rows, cols);
            }
        }
    }

    // ── Scrollback ───────────────────────────────────────────────────────

    /// (lines scrolled back, lines of history available)
    pub fn scroll_info(&self) -> (usize, usize) {
        let Ok(mut p) = self.parser.lock() else { return (0, 0) };
        // set_scrollback clamps to the history length, which is how we read it
        p.set_scrollback(usize::MAX);
        let max = p.screen().scrollback();
        p.set_scrollback(self.scroll.min(max));
        (self.scroll.min(max), max)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let (current, max) = self.scroll_info();
        self.scroll = (current as isize + delta).clamp(0, max as isize) as usize;
        if let Ok(mut p) = self.parser.lock() {
            p.set_scrollback(self.scroll);
        }
    }

    pub fn scroll_to_live(&mut self) {
        self.scroll = 0;
        if let Ok(mut p) = self.parser.lock() {
            p.set_scrollback(0);
        }
    }

    pub fn is_scrolled(&self) -> bool {
        self.scroll > 0
    }

    // ── Drawing ──────────────────────────────────────────────────────────

    /// Render the PTY screen contents into the given ratatui area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        let mut title = format!(" {} ", self.label);
        if self.scroll > 0 {
            let (offset, max) = self.scroll_info();
            title = format!(" {} — scrollback {}/{} · Shift+PgDn returns ", self.label, offset, max);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_alignment(Alignment::Right)
            .style(border_style);
        frame.render_widget(block, area);

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // An exited app keeps its last screen at its last size
        if self.exit_code.is_none() {
            self.resize_if_needed(inner.width, inner.height);
        }
        self.draw_screen(frame.buffer_mut(), inner);
    }

    /// Copy the virtual screen 1:1 into `area` of `buf`, clipping at its edges.
    pub fn draw_screen(&self, buf: &mut Buffer, area: Rect) {
        let Ok(parser) = self.parser.lock() else { return };
        let screen = parser.screen();
        for row in 0..area.height {
            let mut col = 0u16;
            while col < area.width {
                let (x, y) = (area.x + col, area.y + row);
                let Some(cell) = screen.cell(row, col) else {
                    col += 1;
                    continue;
                };
                if cell.is_wide_continuation() {
                    col += 1;
                    continue;
                }
                let wide = cell.is_wide();
                // A wide glyph that would straddle the right edge is dropped
                if wide && col + 1 >= area.width {
                    break;
                }
                let contents = cell.contents();
                let symbol: String = if contents.is_empty() {
                    " ".into()
                } else {
                    // Control characters would reach the real terminal as escapes
                    contents.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
                };
                let style = cell_style(cell);
                if let Some(target) = buf.cell_mut((x, y)) {
                    target.set_symbol(&symbol).set_style(style);
                }
                if wide {
                    if let Some(next) = buf.cell_mut((x + 1, y)) {
                        next.set_symbol("").set_style(style);
                    }
                    col += 2;
                } else {
                    col += 1;
                }
            }
        }
    }

    /// The whole virtual screen as an off-screen buffer (used for previews).
    pub fn screen_buffer(&self) -> Buffer {
        let area = Rect::new(0, 0, self.last_cols, self.last_rows);
        let mut buf = Buffer::empty(area);
        self.draw_screen(&mut buf, area);
        buf
    }

    /// Forward a key action to the PTY.
    pub fn send_key(&mut self, action: Action) {
        let bytes: &[u8] = match action {
            Action::Up => b"\x1b[A",
            Action::Down => b"\x1b[B",
            Action::Right => b"\x1b[C",
            Action::Left => b"\x1b[D",
            Action::Enter => b"\r",
            Action::Back => b"q",
            Action::Tab => b"\t",
            Action::Quit => b"\x1b",
        };
        let _ = self.writer.write_all(bytes);
    }

    /// Forward a raw crossterm KeyEvent directly to the PTY.
    /// This allows all keypresses (characters, function keys, etc.) to reach
    /// the third-party app running inside the container.
    pub fn send_key_event(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        let bytes: Vec<u8> = match key.code {
            KeyCode::Char(c) => {
                if ctrl {
                    // Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
                    let ctrl_byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                    vec![ctrl_byte]
                } else {
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    s.as_bytes().to_vec()
                }
            }
            KeyCode::Enter => vec![b'\r'],
            KeyCode::Backspace => vec![0x7f],
            KeyCode::Tab => vec![b'\t'],
            KeyCode::BackTab => b"\x1b[Z".to_vec(),
            KeyCode::Esc => vec![0x1b],
            KeyCode::Up => b"\x1b[A".to_vec(),
            KeyCode::Down => b"\x1b[B".to_vec(),
            KeyCode::Right => b"\x1b[C".to_vec(),
            KeyCode::Left => b"\x1b[D".to_vec(),
            KeyCode::Home => b"\x1b[H".to_vec(),
            KeyCode::End => b"\x1b[F".to_vec(),
            KeyCode::PageUp => b"\x1b[5~".to_vec(),
            KeyCode::PageDown => b"\x1b[6~".to_vec(),
            KeyCode::Insert => b"\x1b[2~".to_vec(),
            KeyCode::Delete => b"\x1b[3~".to_vec(),
            KeyCode::F(n) => match n {
                1 => b"\x1bOP".to_vec(),
                2 => b"\x1bOQ".to_vec(),
                3 => b"\x1bOR".to_vec(),
                4 => b"\x1bOS".to_vec(),
                5 => b"\x1b[15~".to_vec(),
                6 => b"\x1b[17~".to_vec(),
                7 => b"\x1b[18~".to_vec(),
                8 => b"\x1b[19~".to_vec(),
                9 => b"\x1b[20~".to_vec(),
                10 => b"\x1b[21~".to_vec(),
                11 => b"\x1b[23~".to_vec(),
                12 => b"\x1b[24~".to_vec(),
                _ => return,
            },
            _ => return,
        };

        let _ = self.writer.write_all(&bytes);
    }

    /// Send a raw character to the PTY (for passthrough keys).
    #[allow(dead_code)]
    pub fn send_raw(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let _ = self.writer.write_all(s.as_bytes());
    }
}

impl Drop for InstalledDashboard {
    fn drop(&mut self) {
        // Closing the master hangs the child up; make sure it really is gone
        self.kill();
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    let fg = convert_vt100_color(cell.fgcolor());
    let bg = convert_vt100_color(cell.bgcolor());
    if fg != Color::Reset {
        style = style.fg(fg);
    }
    if bg != Color::Reset {
        style = style.bg(bg);
    }
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

/// Convert a vt100 Color to a ratatui Color.
fn convert_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
