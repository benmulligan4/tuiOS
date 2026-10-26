/// Retro intro — CRT-style "tuiOS" block wordmark with a loading bar.

use std::io;
use std::time::{Duration, Instant};

use ratatui::{
    backend::Backend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};

use super::{bar_spans, center, eased, skip_requested, HOLD_MS, STAGES};

const MARK_ROWS: usize = 9;
const SMALL_ROWS: usize = 5;
const BIG_ROWS: usize = 7;
const BIG_COLS: usize = 5;
const LETTER_GAP: usize = 1;

/// Solid badge holding the knocked-out "tui".
const BADGE_W: usize = 23;
const BADGE_PAD_RIGHT: usize = 2;
const BADGE_GAP: usize = 2;
/// Row the small letters start on, so their baseline matches "OS".
const SMALL_TOP: usize = 3;
/// Row "OS" starts on, leaving a one-row margin against the badge edges.
const BIG_TOP: usize = 1;

const SMALL_TUI: &[[&str; SMALL_ROWS]] = &[
    [
        ".#.",
        "###",
        ".#.",
        ".#.",
        ".##",
    ],
    [
        "...",
        "#.#",
        "#.#",
        "#.#",
        ".##",
    ],
    [
        "#",
        ".",
        "#",
        "#",
        "#",
    ],
];

const BIG_OS: &[[&str; BIG_ROWS]] = &[
    [
        ".###.",
        "#...#",
        "#...#",
        "#...#",
        "#...#",
        "#...#",
        ".###.",
    ],
    [
        ".####",
        "#....",
        "#....",
        ".###.",
        "....#",
        "....#",
        "####.",
    ],
];

const SMALL_W: usize = 3 + LETTER_GAP + 3 + LETTER_GAP + 1;
const MARK_COLS: usize = BADGE_W + BADGE_GAP + BIG_COLS * 2 + LETTER_GAP;

/// Play the intro animation. Returns early if the user presses a key.
pub fn play<B: Backend>(
    terminal: &mut Terminal<B>,
    accent: Color,
    duration_ms: u64,
) -> io::Result<()> {
    let start = Instant::now();
    let total = Duration::from_millis(duration_ms);

    loop {
        let elapsed = start.elapsed();
        if elapsed >= total {
            break;
        }
        let tick = elapsed.as_millis() as u64;
        let progress = eased(tick as f64 / duration_ms as f64);

        terminal.draw(|frame| render(frame, frame.area(), progress, tick, accent, false))?;

        if skip_requested()? {
            return Ok(());
        }
    }

    // Hold on "SYSTEM READY" so the boot reads as finished
    let hold = Instant::now();
    loop {
        let elapsed = hold.elapsed();
        if elapsed >= Duration::from_millis(HOLD_MS) {
            break;
        }
        let tick = duration_ms + elapsed.as_millis() as u64;
        terminal.draw(|frame| render(frame, frame.area(), 1.0, tick, accent, true))?;
        if skip_requested()? {
            break;
        }
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Cell {
    Empty,
    White,
    Black,
}

/// Build the wordmark grid: "tui" in black on a solid white badge, "OS" in white beside it.
fn wordmark() -> [[Cell; MARK_COLS]; MARK_ROWS] {
    let mut grid = [[Cell::Empty; MARK_COLS]; MARK_ROWS];

    for row in grid.iter_mut() {
        for cell in row[..BADGE_W].iter_mut() {
            *cell = Cell::White;
        }
    }

    let mut x = BADGE_W - BADGE_PAD_RIGHT - SMALL_W;
    for glyph in SMALL_TUI {
        for (r, line) in glyph.iter().enumerate() {
            for (c, px) in line.chars().enumerate() {
                if px == '#' {
                    grid[SMALL_TOP + r][x + c] = Cell::Black;
                }
            }
        }
        x += glyph[0].len() + LETTER_GAP;
    }

    let mut x = BADGE_W + BADGE_GAP;
    for glyph in BIG_OS {
        for (r, line) in glyph.iter().enumerate() {
            for (c, px) in line.chars().enumerate() {
                if px == '#' {
                    grid[BIG_TOP + r][x + c] = Cell::White;
                }
            }
        }
        x += BIG_COLS + LETTER_GAP;
    }

    grid
}

/// Coloured spaces rather than block glyphs, so rows join without seams.
fn mark_line(row: &[Cell; MARK_COLS], scale: usize) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let cell = row[i];
        let mut j = i;
        while j < row.len() && row[j] == cell {
            j += 1;
        }
        let style = match cell {
            Cell::Empty => Style::default(),
            Cell::White => Style::default().bg(Color::White),
            Cell::Black => Style::default().bg(Color::Black),
        };
        spans.push(Span::styled(" ".repeat((j - i) * scale), style));
        i = j;
    }
    Line::from(spans)
}

fn wordmark_width(scale: usize) -> u16 {
    (MARK_COLS * scale) as u16
}

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    progress: f64,
    tick: u64,
    accent: Color,
    ready: bool,
) {
    let progress = progress.clamp(0.0, 1.0);

    let dim = Color::Rgb(70, 70, 70);
    let text = Color::Rgb(170, 170, 170);

    // Pick the largest wordmark scale that fits, else fall back to plain text
    let scale = if area.width >= wordmark_width(2) + 4 { 2 } else { 1 };
    let mark_w = wordmark_width(scale);
    let use_mark = area.width >= mark_w + 2 && area.height >= 16;
    let content_w = if use_mark {
        mark_w
    } else {
        area.width.saturating_sub(4).clamp(12, 40)
    };

    let mut lines: Vec<Line> = Vec::new();

    if use_mark {
        for row in wordmark().iter() {
            lines.push(mark_line(row, scale));
        }
    } else {
        let pad = (content_w as usize).saturating_sub(7) / 2;
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(pad)),
            Span::styled(
                " tui ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "OS",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    lines.push(Line::default());

    let subtitle = if content_w >= 24 {
        "Created by Ben Mulligan"
    } else {
        "Ben Mulligan"
    };
    lines.push(Line::from(center(
        Span::styled(subtitle, Style::default().fg(dim)),
        content_w,
    )));

    lines.push(Line::default());
    lines.push(Line::default());

    // Progress bar: solid accent fill, dark trough, framed by dim half-blocks
    let bar_w = content_w.saturating_sub(2) as usize;
    let mut bar: Vec<Span> = vec![Span::styled("▐", Style::default().fg(dim))];
    bar.extend(bar_spans(bar_w, progress, accent, dim));
    bar.push(Span::styled("▌", Style::default().fg(dim)));
    lines.push(Line::from(bar));

    lines.push(Line::default());

    // Status line: stage on the left with a blinking cursor, percentage on the right
    let stage = if ready {
        "SYSTEM READY"
    } else {
        STAGES[((progress * STAGES.len() as f64) as usize).min(STAGES.len() - 1)]
    };
    let percent = format!("{:>3}%", (progress * 100.0).round() as u16);
    let cursor = if (tick / 400) % 2 == 0 { "█" } else { " " };
    let used = 2 + stage.chars().count() + 1 + percent.chars().count();
    let gap = (content_w as usize).saturating_sub(used);
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(accent)),
        Span::styled(stage, Style::default().fg(text)),
        Span::styled(cursor, Style::default().fg(accent)),
        Span::raw(" ".repeat(gap)),
        Span::styled(percent, Style::default().fg(accent)),
    ]));

    let content_h = lines.len() as u16;
    let rect = Rect {
        x: area.x + area.width.saturating_sub(content_w) / 2,
        y: area.y + area.height.saturating_sub(content_h) / 2,
        width: content_w.min(area.width),
        height: content_h.min(area.height),
    };

    frame.render_widget(Paragraph::new(lines), rect);
}
