/// Seven-segment style block digits for the Dashboard 1 clock.
///
/// Glyphs are generated from a segment table rather than stored as art so the
/// clock can be re-rendered at whatever size the panel allows.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Segment bits, in the usual seven-segment order a,b,c,d,e,f,g.
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

const GAP: usize = 1;

fn digit_cells(value: usize, w: usize, h: usize) -> Vec<Vec<bool>> {
    cells_for(DIGITS[value.min(9)], w, h)
}

fn cells_for(segs: u8, w: usize, h: usize) -> Vec<Vec<bool>> {
    let mid = h / 2;
    let last = h - 1;
    let mut cells = vec![vec![false; w]; h];

    let on = |s: u8| segs & s != 0;
    if on(A) {
        cells[0].iter_mut().for_each(|c| *c = true);
    }
    if on(G) {
        cells[mid].iter_mut().for_each(|c| *c = true);
    }
    if on(D) {
        cells[last].iter_mut().for_each(|c| *c = true);
    }
    for row in 1..mid {
        cells[row][0] = on(F);
        cells[row][w - 1] = on(B);
    }
    for row in (mid + 1)..last {
        cells[row][0] = on(E);
        cells[row][w - 1] = on(C);
    }
    cells
}

fn colon_cells(h: usize) -> Vec<Vec<bool>> {
    let mut cells = vec![vec![false; 2]; h];
    let top = (h / 4).max(1);
    let bottom = h - 1 - top;
    for col in 0..2 {
        cells[top][col] = true;
        cells[bottom][col] = true;
    }
    cells
}

/// Lit cells for a character, plus the mask of cells belonging to *any* segment.
/// The second grid is where the unlit "ghost" segments are painted.
fn glyph(ch: char, w: usize, h: usize) -> (Vec<Vec<bool>>, Vec<Vec<bool>>) {
    match ch {
        '0'..='9' => (
            digit_cells(ch as usize - '0' as usize, w, h),
            cells_for(DIGITS[8], w, h),
        ),
        ':' => {
            let cells = colon_cells(h);
            (cells.clone(), cells)
        }
        _ => {
            let blank = vec![vec![false; w.max(1) / 2]; h];
            (blank.clone(), blank)
        }
    }
}

/// Columns the rendered text will occupy at the given digit width.
pub fn width(text: &str, w: usize) -> usize {
    let mut total = 0;
    for (i, ch) in text.chars().enumerate() {
        if i > 0 {
            total += GAP;
        }
        total += match ch {
            '0'..='9' => w,
            ':' => 2,
            _ => w.max(1) / 2,
        };
    }
    total
}

/// Render `text` as block digits. `ghost` paints the unlit segments for an LCD
/// look; pass `None` to leave them blank.
pub fn render(text: &str, w: usize, h: usize, on: Color, ghost: Option<Color>) -> Vec<Line<'static>> {
    let glyphs: Vec<(Vec<Vec<bool>>, Vec<Vec<bool>>)> =
        text.chars().map(|ch| glyph(ch, w, h)).collect();
    let on_style = Style::default().fg(on);
    let ghost_style = ghost.map(|c| Style::default().fg(c));

    (0..h)
        .map(|row| {
            // 2 = lit, 1 = unlit segment, 0 = background
            let mut cells: Vec<u8> = Vec::new();
            for (i, (lit, mask)) in glyphs.iter().enumerate() {
                if i > 0 {
                    cells.extend(std::iter::repeat(0u8).take(GAP));
                }
                for (on_cell, in_mask) in lit[row].iter().zip(mask[row].iter()) {
                    cells.push(if *on_cell {
                        2
                    } else if *in_mask {
                        1
                    } else {
                        0
                    });
                }
            }

            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut iter = cells.into_iter();
            if let Some(first) = iter.next() {
                let mut run_kind = first;
                let mut run_len = 1usize;
                for kind in iter {
                    if kind == run_kind {
                        run_len += 1;
                    } else {
                        spans.push(span_for(run_kind, run_len, on_style, ghost_style));
                        run_kind = kind;
                        run_len = 1;
                    }
                }
                spans.push(span_for(run_kind, run_len, on_style, ghost_style));
            }
            Line::from(spans)
        })
        .collect()
}

fn span_for(kind: u8, len: usize, on: Style, ghost: Option<Style>) -> Span<'static> {
    match (kind, ghost) {
        (2, _) => Span::styled("█".repeat(len), on),
        (1, Some(style)) => Span::styled("█".repeat(len), style),
        _ => Span::raw(" ".repeat(len)),
    }
}
