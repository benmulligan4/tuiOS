/// Clock faces for Dashboard 1.
///
/// Five looks the user cycles with `c`. Digit shapes come from a seven-segment
/// model so each style can be generated at whatever size the panel allows,
/// rather than being stored as fixed art.

use chrono::{DateTime, Local, Timelike};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::canvas::{Canvas, Circle, Line as CanvasLine},
    Frame,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClockStyle {
    Block,
    Shadow,
    Outline,
    Flip,
    Analog,
}

pub const ALL: [ClockStyle; 5] = [
    ClockStyle::Block,
    ClockStyle::Shadow,
    ClockStyle::Outline,
    ClockStyle::Flip,
    ClockStyle::Analog,
];

impl ClockStyle {
    pub fn name(self) -> &'static str {
        match self {
            ClockStyle::Block => "Block",
            ClockStyle::Shadow => "Shadow",
            ClockStyle::Outline => "Outline",
            ClockStyle::Flip => "Flip",
            ClockStyle::Analog => "Analog",
        }
    }

    pub fn from_name(name: &str) -> Self {
        ALL.into_iter()
            .find(|s| s.name() == name)
            .unwrap_or(ClockStyle::Block)
    }

    pub fn next(self, forward: bool) -> Self {
        let index = ALL.iter().position(|s| *s == self).unwrap_or(0);
        let len = ALL.len();
        ALL[if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        }]
    }
}

// ---------------------------------------------------------------------------
// Segment model
// ---------------------------------------------------------------------------

const A: u8 = 1 << 0;
const B: u8 = 1 << 1;
const C: u8 = 1 << 2;
const D: u8 = 1 << 3;
const E: u8 = 1 << 4;
const F: u8 = 1 << 5;
const G: u8 = 1 << 6;

const DIGITS: [u8; 10] = [
    A | B | C | D | E | F,
    B | C,
    A | B | D | E | G,
    A | B | C | D | G,
    B | C | F | G,
    A | C | D | F | G,
    A | C | D | E | F | G,
    A | B | C,
    A | B | C | D | E | F | G,
    A | B | C | D | F | G,
];

/// Glyph geometry: `w` columns wide, strokes `th` rows / `tv` columns thick,
/// with `gap` rows between the three horizontal bars.
#[derive(Clone, Copy)]
pub struct Size {
    pub w: usize,
    pub tv: usize,
    pub th: usize,
    pub gap: usize,
}

impl Size {
    pub fn height(self) -> usize {
        3 * self.th + 2 * self.gap
    }
}

const FILLED_SIZES: &[Size] = &[
    Size { w: 14, tv: 3, th: 3, gap: 4 },
    Size { w: 12, tv: 3, th: 2, gap: 3 },
    Size { w: 10, tv: 2, th: 2, gap: 2 },
    Size { w: 8, tv: 2, th: 1, gap: 3 },
    Size { w: 7, tv: 2, th: 1, gap: 2 },
    Size { w: 6, tv: 2, th: 1, gap: 1 },
    Size { w: 4, tv: 1, th: 1, gap: 1 },
];

// Outline strokes are always a single line thick.
const OUTLINE_SIZES: &[Size] = &[
    Size { w: 12, tv: 1, th: 1, gap: 5 },
    Size { w: 10, tv: 1, th: 1, gap: 4 },
    Size { w: 8, tv: 1, th: 1, gap: 3 },
    Size { w: 7, tv: 1, th: 1, gap: 2 },
    Size { w: 6, tv: 1, th: 1, gap: 1 },
    Size { w: 4, tv: 1, th: 1, gap: 1 },
];

fn sizes(style: ClockStyle) -> &'static [Size] {
    match style {
        ClockStyle::Outline => OUTLINE_SIZES,
        _ => FILLED_SIZES,
    }
}

// ---------------------------------------------------------------------------
// Cell grid
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Empty,
    Lit,
    Dim,
}

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    ink: Ink,
}

const BLANK: Cell = Cell { ch: ' ', ink: Ink::Empty };

fn lit(ch: char) -> Cell {
    Cell { ch, ink: Ink::Lit }
}

fn dim(ch: char) -> Cell {
    Cell { ch, ink: Ink::Dim }
}

type Grid = Vec<Vec<Cell>>;

fn blank_grid(w: usize, h: usize) -> Grid {
    vec![vec![BLANK; w]; h]
}

fn grid_width(grid: &Grid) -> usize {
    grid.first().map_or(0, |row| row.len())
}

/// Mask of the cells covered by the given segments.
fn fill(mask: &mut [Vec<bool>], rows: std::ops::Range<usize>, cols: std::ops::Range<usize>) {
    for row in rows {
        for col in cols.clone() {
            mask[row][col] = true;
        }
    }
}

fn seg_mask(segs: u8, s: Size) -> Vec<Vec<bool>> {
    let h = s.height();
    let mut mask = vec![vec![false; s.w]; h];
    let on = |seg: u8| segs & seg != 0;
    let mid = s.th + s.gap;
    let mid_end = mid + s.th;

    if on(A) {
        fill(&mut mask, 0..s.th, 0..s.w);
    }
    if on(G) {
        fill(&mut mask, mid..mid_end, 0..s.w);
    }
    if on(D) {
        fill(&mut mask, h - s.th..h, 0..s.w);
    }

    // A stroke with no bar capping it runs to the glyph edge (so 4 and 7 are
    // full height), and with no middle bar the two halves of a side join up.
    let top = if on(A) { s.th } else { 0 };
    let bottom = if on(D) { h - s.th } else { h };

    for (upper, lower, start_col) in [(F, E, 0), (B, C, s.w - s.tv)] {
        let cols = start_col..start_col + s.tv;
        match (on(upper), on(lower)) {
            (true, true) if !on(G) => fill(&mut mask, top..bottom, cols),
            (true, true) => {
                fill(&mut mask, top..mid, cols.clone());
                fill(&mut mask, mid_end..bottom, cols);
            }
            (true, false) => fill(&mut mask, top..mid, cols),
            (false, true) => fill(&mut mask, mid_end..bottom, cols),
            (false, false) => {}
        }
    }
    mask
}

/// Rows holding the two colon dots, one per gap.
fn dot_rows(s: Size) -> (usize, usize) {
    let inset = s.gap.saturating_sub(s.th) / 2;
    (s.th + inset, 2 * s.th + s.gap + inset)
}

/// A seven-segment "1" is only the right-hand bar, which reads as a stray
/// stroke, so draw a real numeral: stem, top-left serif and a base.
fn one_mask(s: Size) -> Vec<Vec<bool>> {
    let h = s.height();
    let mut mask = vec![vec![false; s.w]; h];
    let stem = (s.w - s.tv) / 2;
    let serif = stem.saturating_sub(s.tv);

    for row in mask.iter_mut().take(h - s.th) {
        for col in stem..stem + s.tv {
            row[col] = true;
        }
    }
    for row in mask.iter_mut().take(2 * s.th).skip(s.th) {
        for col in serif..stem + s.tv {
            row[col] = true;
        }
    }
    for row in mask.iter_mut().skip(h - s.th) {
        row.iter_mut().for_each(|c| *c = true);
    }
    mask
}

fn digit_mask(value: usize, s: Size) -> Vec<Vec<bool>> {
    if value == 1 {
        one_mask(s)
    } else {
        seg_mask(DIGITS[value.min(9)], s)
    }
}

// ---------------------------------------------------------------------------
// Filled glyphs (Block / Shadow / Flip)
// ---------------------------------------------------------------------------

fn filled_digit(value: usize, s: Size) -> Grid {
    digit_mask(value, s)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|on| if on { lit('█') } else { BLANK })
                .collect()
        })
        .collect()
}

fn filled_colon(s: Size) -> Grid {
    let mut grid = blank_grid(s.tv, s.height());
    let (top, bottom) = dot_rows(s);
    for start in [top, bottom] {
        for row in start..(start + s.th).min(s.height()) {
            grid[row].iter_mut().for_each(|c| *c = lit('█'));
        }
    }
    grid
}

fn filled_glyph(ch: char, s: Size) -> Grid {
    match ch {
        '0'..='9' => filled_digit(ch as usize - '0' as usize, s),
        ':' => filled_colon(s),
        _ => blank_grid(s.w / 2, s.height()),
    }
}

fn shadow_glyph(ch: char, s: Size) -> Grid {
    let base = filled_glyph(ch, s);
    let (w, h) = (grid_width(&base), base.len());
    let mut grid = blank_grid(w + 1, h + 1);
    for (r, row) in base.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.ink != Ink::Empty {
                grid[r + 1][c + 1] = dim('█');
            }
        }
    }
    for (r, row) in base.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.ink != Ink::Empty {
                grid[r][c] = *cell;
            }
        }
    }
    grid
}

fn flip_glyph(ch: char, s: Size) -> Grid {
    let h = s.height();
    if ch == ':' {
        let mut grid = blank_grid(s.tv + 2, h + 2);
        let (top, bottom) = dot_rows(s);
        for start in [top, bottom] {
            for row in start..(start + s.th).min(h) {
                for col in 0..s.tv {
                    grid[row + 1][col + 1] = lit('█');
                }
            }
        }
        return grid;
    }

    let base = filled_glyph(ch, s);
    let w = grid_width(&base);
    let mut grid = blank_grid(w + 2, h + 2);
    grid[0][0] = dim('╭');
    grid[0][w + 1] = dim('╮');
    grid[h + 1][0] = dim('╰');
    grid[h + 1][w + 1] = dim('╯');
    for col in 1..=w {
        grid[0][col] = dim('─');
        grid[h + 1][col] = dim('─');
    }
    // The flap seam runs across the card on the middle-bar row.
    let seam = s.th + s.gap;
    for row in 0..h {
        let edge = if row == seam { '├' } else { '│' };
        grid[row + 1][0] = dim(edge);
        grid[row + 1][w + 1] = dim(if row == seam { '┤' } else { '│' });
        for col in 0..w {
            let cell = base[row][col];
            grid[row + 1][col + 1] = match (cell.ink, row == seam) {
                (Ink::Empty, true) => dim('─'),
                (Ink::Empty, false) => BLANK,
                _ => cell,
            };
        }
    }
    grid
}

// ---------------------------------------------------------------------------
// Outline glyphs
// ---------------------------------------------------------------------------

fn junction(up: bool, down: bool, left: bool, right: bool) -> char {
    match (up, down, left, right) {
        (true, true, true, true) => '┼',
        (true, true, true, false) => '┤',
        (true, true, false, true) => '├',
        (true, true, false, false) => '│',
        (true, false, true, true) => '┴',
        (true, false, true, false) => '╯',
        (true, false, false, true) => '╰',
        (true, false, false, false) => '╵',
        (false, true, true, true) => '┬',
        (false, true, true, false) => '╮',
        (false, true, false, true) => '╭',
        (false, true, false, false) => '╷',
        (false, false, true, true) => '─',
        (false, false, true, false) => '╴',
        (false, false, false, true) => '╶',
        (false, false, false, false) => ' ',
    }
}

fn outline_one(s: Size) -> Grid {
    let h = s.height();
    let mut grid = blank_grid(s.w, h);
    let stem = (s.w - 1) / 2;
    const SERIF: usize = 1;

    grid[0][stem] = lit('╷');
    for row in 1..h - 1 {
        grid[row][stem] = lit('│');
    }
    for col in 0..stem {
        grid[SERIF][col] = lit('─');
    }
    if stem > 0 {
        grid[SERIF][0] = lit('╶');
        grid[SERIF][stem] = lit('┤');
    }
    for col in 0..s.w {
        grid[h - 1][col] = lit('─');
    }
    grid[h - 1][0] = lit('╶');
    grid[h - 1][s.w - 1] = lit('╴');
    grid[h - 1][stem] = lit('┴');
    grid
}

fn outline_digit(value: usize, s: Size) -> Grid {
    if value == 1 {
        return outline_one(s);
    }
    let segs = DIGITS[value.min(9)];
    let on = |seg: u8| segs & seg != 0;
    let h = s.height();
    let mid = 1 + s.gap;
    let mut grid = blank_grid(s.w, h);

    for col in 1..s.w - 1 {
        if on(A) {
            grid[0][col] = lit('─');
        }
        if on(G) {
            grid[mid][col] = lit('─');
        }
        if on(D) {
            grid[h - 1][col] = lit('─');
        }
    }
    for row in 1..mid {
        if on(F) {
            grid[row][0] = lit('│');
        }
        if on(B) {
            grid[row][s.w - 1] = lit('│');
        }
    }
    for row in mid + 1..h - 1 {
        if on(E) {
            grid[row][0] = lit('│');
        }
        if on(C) {
            grid[row][s.w - 1] = lit('│');
        }
    }

    let corners = [
        (0, 0, false, on(F), false, on(A)),
        (0, s.w - 1, false, on(B), on(A), false),
        (mid, 0, on(F), on(E), false, on(G)),
        (mid, s.w - 1, on(B), on(C), on(G), false),
        (h - 1, 0, on(E), false, false, on(D)),
        (h - 1, s.w - 1, on(C), false, on(D), false),
    ];
    for (row, col, up, down, left, right) in corners {
        let ch = junction(up, down, left, right);
        grid[row][col] = if ch == ' ' { BLANK } else { lit(ch) };
    }
    grid
}

fn outline_glyph(ch: char, s: Size) -> Grid {
    match ch {
        '0'..='9' => outline_digit(ch as usize - '0' as usize, s),
        ':' => {
            let mut grid = blank_grid(1, s.height());
            let (top, bottom) = dot_rows(s);
            grid[top.min(s.height() - 1)][0] = lit('▪');
            grid[bottom.min(s.height() - 1)][0] = lit('▪');
            grid
        }
        _ => blank_grid(s.w / 2, s.height()),
    }
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

fn glyph(style: ClockStyle, ch: char, s: Size) -> Grid {
    match style {
        ClockStyle::Shadow => shadow_glyph(ch, s),
        ClockStyle::Outline => outline_glyph(ch, s),
        ClockStyle::Flip => flip_glyph(ch, s),
        _ => filled_glyph(ch, s),
    }
}

fn char_gap(style: ClockStyle, s: Size) -> usize {
    match style {
        ClockStyle::Outline | ClockStyle::Flip => 1,
        _ if s.tv > 1 => 2,
        _ => 1,
    }
}

fn build(style: ClockStyle, text: &str, s: Size) -> Grid {
    let glyphs: Vec<Grid> = text.chars().map(|ch| glyph(style, ch, s)).collect();
    let rows = glyphs.iter().map(|g| g.len()).max().unwrap_or(0);
    let gap = char_gap(style, s);
    (0..rows)
        .map(|row| {
            let mut line: Vec<Cell> = Vec::new();
            for (index, g) in glyphs.iter().enumerate() {
                if index > 0 {
                    line.extend(std::iter::repeat(BLANK).take(gap));
                }
                match g.get(row) {
                    Some(cells) => line.extend(cells.iter().copied()),
                    None => line.extend(std::iter::repeat(BLANK).take(grid_width(g))),
                }
            }
            line
        })
        .collect()
}

fn to_lines(grid: &Grid, accent: Color, shade: Color) -> Vec<Line<'static>> {
    let styles = |ink: Ink| match ink {
        Ink::Lit => Style::default().fg(accent),
        Ink::Dim => Style::default().fg(shade),
        Ink::Empty => Style::default(),
    };
    grid.iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run = String::new();
            let mut run_ink = row.first().map_or(Ink::Empty, |c| c.ink);
            for cell in row {
                if cell.ink != run_ink {
                    spans.push(Span::styled(std::mem::take(&mut run), styles(run_ink)));
                    run_ink = cell.ink;
                }
                run.push(cell.ch);
            }
            spans.push(Span::styled(run, styles(run_ink)));
            Line::from(spans)
        })
        .collect()
}

/// Largest geometry whose rendered block fits inside `max_w` × `max_h`.
pub fn best_size(style: ClockStyle, text: &str, max_w: usize, max_h: usize) -> Option<Size> {
    sizes(style).iter().copied().find(|s| {
        let grid = build(style, text, *s);
        grid.len() <= max_h && grid_width(&grid) <= max_w
    })
}

/// Rendered clock plus the columns it occupies.
pub fn render(
    style: ClockStyle,
    text: &str,
    s: Size,
    accent: Color,
    shade: Color,
) -> (Vec<Line<'static>>, usize) {
    let grid = build(style, text, s);
    let width = grid_width(&grid);
    (to_lines(&grid, accent, shade), width)
}

// ---------------------------------------------------------------------------
// Analog face
// ---------------------------------------------------------------------------

/// Draw an analog face filling `area`. Braille cells are twice as tall as they
/// are wide, so the x bounds are scaled to keep the dial circular.
pub fn analog(
    frame: &mut Frame,
    area: Rect,
    now: DateTime<Local>,
    accent: Color,
    shade: Color,
    show_seconds: bool,
) {
    if area.width < 8 || area.height < 4 {
        return;
    }
    let span = f64::from(area.width) / (2.0 * f64::from(area.height));
    let radius = span.min(1.0) * 0.92;
    // One braille pixel in canvas units, used to give the hands some weight.
    let pixel = 2.0 / (f64::from(area.height) * 4.0);

    let minute = f64::from(now.minute()) + f64::from(now.second()) / 60.0;
    let hour = f64::from(now.hour() % 12) + minute / 60.0;
    let hour_angle = hour / 12.0 * std::f64::consts::TAU;
    let minute_angle = minute / 60.0 * std::f64::consts::TAU;
    let second_angle = f64::from(now.second()) / 60.0 * std::f64::consts::TAU;

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([-span, span])
        .y_bounds([-1.0, 1.0])
        .paint(move |ctx| {
            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius,
                color: shade,
            });
            for tick in 0..12 {
                let angle = f64::from(tick) / 12.0 * std::f64::consts::TAU;
                let inner = if tick % 3 == 0 { 0.78 } else { 0.88 };
                ctx.draw(&CanvasLine {
                    x1: angle.sin() * radius * inner,
                    y1: angle.cos() * radius * inner,
                    x2: angle.sin() * radius * 0.97,
                    y2: angle.cos() * radius * 0.97,
                    color: if tick % 3 == 0 { accent } else { shade },
                });
            }
            ctx.layer();
            hand(ctx, hour_angle, radius * 0.52, accent, 3, pixel);
            hand(ctx, minute_angle, radius * 0.78, accent, 2, pixel);
            if show_seconds {
                hand(
                    ctx,
                    second_angle,
                    radius * 0.86,
                    Color::Rgb(255, 120, 90),
                    1,
                    pixel,
                );
            }
            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius: radius * 0.03,
                color: accent,
            });
        });
    frame.render_widget(canvas, area);
}

/// Hands are drawn as a few parallel lines so they read as solid, not hairline.
fn hand(
    ctx: &mut ratatui::widgets::canvas::Context,
    angle: f64,
    length: f64,
    color: Color,
    weight: usize,
    pixel: f64,
) {
    let (dx, dy) = (angle.sin(), angle.cos());
    let (px, py) = (dy * pixel, -dx * pixel);
    for step in 0..weight {
        let offset = step as f64 - (weight as f64 - 1.0) / 2.0;
        ctx.draw(&CanvasLine {
            x1: px * offset - dx * length * 0.12,
            y1: py * offset - dy * length * 0.12,
            x2: dx * length + px * offset,
            y2: dy * length + py * offset,
            color,
        });
    }
}
