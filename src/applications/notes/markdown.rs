/// Notes — lightweight Markdown helpers: list/checkbox parsing shared with the
/// editor, and a line-oriented renderer for the preview pane.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMarker {
    Bullet(char),
    Ordered(u64, char),
    Check { bullet: char, checked: bool },
}

impl ListMarker {
    /// The marker to start the next item with when pressing Enter on this one.
    pub fn continuation(self) -> String {
        match self {
            ListMarker::Bullet(c) => format!("{} ", c),
            ListMarker::Ordered(n, d) => format!("{}{} ", n + 1, d),
            ListMarker::Check { bullet, .. } => format!("{} [ ] ", bullet),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListPrefix {
    /// Byte length of the leading indentation.
    pub indent: usize,
    /// Byte offset where the item's content starts.
    pub end: usize,
    pub marker: ListMarker,
}

pub fn parse_list_prefix(line: &str) -> Option<ListPrefix> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let first = rest.chars().next()?;

    if matches!(first, '-' | '*' | '+') && rest[1..].starts_with(' ') {
        let after = &rest[2..];
        let checked = match after.get(..3) {
            Some("[ ]") => Some(false),
            Some("[x]") | Some("[X]") => Some(true),
            _ => None,
        };
        if let Some(checked) = checked {
            if after.len() == 3 || after[3..].starts_with(' ') {
                let end = indent + 2 + after.len().min(4);
                return Some(ListPrefix { indent, end, marker: ListMarker::Check { bullet: first, checked } });
            }
        }
        return Some(ListPrefix { indent, end: indent + 2, marker: ListMarker::Bullet(first) });
    }

    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) {
        let after = &rest[digits..];
        if after.starts_with(". ") || after.starts_with(") ") {
            let n = rest[..digits].parse().ok()?;
            let d = after.chars().next()?;
            return Some(ListPrefix { indent, end: indent + digits + 2, marker: ListMarker::Ordered(n, d) });
        }
    }
    None
}

/// Checklist state of a line, if it is a checklist item.
pub fn checkbox_state(line: &str) -> Option<bool> {
    match parse_list_prefix(line)?.marker {
        ListMarker::Check { checked, .. } => Some(checked),
        _ => None,
    }
}

/// Flip `[ ]` <-> `[x]` on a checklist line. Returns None if not a checklist item.
pub fn toggle_checkbox(line: &str) -> Option<String> {
    let prefix = parse_list_prefix(line)?;
    let ListMarker::Check { checked, .. } = prefix.marker else { return None };
    let mark_at = prefix.indent + 3;
    let mut out = String::with_capacity(line.len());
    out.push_str(&line[..mark_at]);
    out.push(if checked { ' ' } else { 'x' });
    out.push_str(&line[mark_at + 1..]);
    Some(out)
}

/// (done, total) checklist items in a note body.
pub fn checklist_progress(body: &str) -> (usize, usize) {
    body.lines().filter_map(checkbox_state).fold((0, 0), |(d, t), c| (d + c as usize, t + 1))
}

pub fn is_code_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const TEXT: Color = Color::White;
const DIM: Color = Color::DarkGray;
const CODE: Color = Color::Green;

/// Render every source line to exactly one styled `Line` (before wrapping).
pub fn render_document(lines: &[String], width: usize) -> Vec<Line<'static>> {
    let mut in_code = false;
    lines.iter().map(|l| render_line(l, &mut in_code, width)).collect()
}

fn render_line(line: &str, in_code: &mut bool, width: usize) -> Line<'static> {
    if is_code_fence(line) {
        *in_code = !*in_code;
        return Line::from(Span::styled(line.to_string(), Style::default().fg(DIM)));
    }
    if *in_code {
        return Line::from(Span::styled(line.to_string(), Style::default().fg(CODE)));
    }

    let trimmed = line.trim();
    if is_rule(trimmed) {
        return Line::from(Span::styled("─".repeat(width.max(3)), Style::default().fg(DIM)));
    }

    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        let style = match hashes {
            1 => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            _ => Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD),
        };
        let mut spans = Vec::new();
        inline(trimmed[hashes..].trim_start(), style, &mut spans);
        return Line::from(spans);
    }

    if let Some(quote) = trimmed.strip_prefix('>') {
        let mut spans = vec![Span::styled("▎ ", Style::default().fg(Color::Magenta))];
        inline(quote.trim_start(), Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC), &mut spans);
        return Line::from(spans);
    }

    if let Some(prefix) = parse_list_prefix(line) {
        let indent = " ".repeat(prefix.indent);
        let content = &line[prefix.end..];
        let (marker, style) = match prefix.marker {
            ListMarker::Bullet(_) => (
                Span::styled("• ", Style::default().fg(Color::Yellow)),
                Style::default().fg(TEXT),
            ),
            ListMarker::Ordered(n, _) => (
                Span::styled(format!("{}. ", n), Style::default().fg(Color::Yellow)),
                Style::default().fg(TEXT),
            ),
            ListMarker::Check { checked: false, .. } => (
                Span::styled("☐ ", Style::default().fg(Color::Yellow)),
                Style::default().fg(TEXT),
            ),
            ListMarker::Check { checked: true, .. } => (
                Span::styled("☑ ", Style::default().fg(Color::Green)),
                Style::default().fg(DIM).add_modifier(Modifier::CROSSED_OUT),
            ),
        };
        let mut spans = vec![Span::raw(indent), marker];
        inline(content, style, &mut spans);
        return Line::from(spans);
    }

    let mut spans = Vec::new();
    inline(line, Style::default().fg(TEXT), &mut spans);
    Line::from(spans)
}

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_'].iter().any(|&m| compact.chars().all(|c| c == m))
}

#[derive(Clone, Copy)]
enum Inline {
    Code,
    Bold,
    Italic,
    Strike,
    Link,
}

/// Parse inline Markdown (`code`, **bold**, *italic*, ~~strike~~, [link](url)).
fn inline(text: &str, base: Style, out: &mut Vec<Span<'static>>) {
    let mut buf = String::new();
    let mut i = 0;
    let mut prev: Option<char> = None;
    while i < text.len() {
        let rest = &text[i..];
        if let Some((consumed, inner, kind)) = match_inline(rest, prev) {
            if !buf.is_empty() {
                out.push(Span::styled(std::mem::take(&mut buf), base));
            }
            match kind {
                Inline::Code => out.push(Span::styled(
                    inner.to_string(),
                    Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 40, 40)),
                )),
                Inline::Bold => inline(inner, base.add_modifier(Modifier::BOLD), out),
                Inline::Italic => inline(inner, base.add_modifier(Modifier::ITALIC), out),
                Inline::Strike => inline(inner, base.add_modifier(Modifier::CROSSED_OUT), out),
                Inline::Link => out.push(Span::styled(
                    inner.to_string(),
                    base.fg(Color::LightBlue).add_modifier(Modifier::UNDERLINED),
                )),
            }
            prev = text[..i + consumed].chars().last();
            i += consumed;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        buf.push(ch);
        prev = Some(ch);
        i += ch.len_utf8();
    }
    if !buf.is_empty() {
        out.push(Span::styled(buf, base));
    }
}

fn match_inline(rest: &str, prev: Option<char>) -> Option<(usize, &str, Inline)> {
    const DELIMS: [(&str, Inline); 6] = [
        ("`", Inline::Code),
        ("**", Inline::Bold),
        ("__", Inline::Bold),
        ("~~", Inline::Strike),
        ("*", Inline::Italic),
        ("_", Inline::Italic),
    ];
    for (delim, kind) in DELIMS {
        if !rest.starts_with(delim) {
            continue;
        }
        // Underscores inside words (snake_case) are not emphasis
        if delim.starts_with('_') && prev.is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        let body = &rest[delim.len()..];
        if body.starts_with(' ') {
            continue;
        }
        if let Some(end) = body.find(delim) {
            if end > 0 {
                return Some((delim.len() * 2 + end, &body[..end], kind));
            }
        }
    }
    if rest.starts_with('[') {
        let close = rest.find("](")?;
        let paren = rest[close + 2..].find(')')?;
        if close > 1 {
            return Some((close + 2 + paren + 1, &rest[1..close], Inline::Link));
        }
    }
    None
}

/// Word-wrap a styled line to `width` columns (character based).
pub fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    if cells.len() <= width {
        return vec![line.clone()];
    }

    let mut rows = Vec::new();
    let mut start = 0;
    while start < cells.len() {
        let mut end = (start + width).min(cells.len());
        if end < cells.len() {
            if let Some(space) = cells[start..end].iter().rposition(|(c, _)| *c == ' ') {
                if space > 0 {
                    end = start + space + 1;
                }
            }
        }
        rows.push(cells_to_line(&cells[start..end], line.style));
        start = end;
    }
    rows
}

fn cells_to_line(cells: &[(char, Style)], line_style: Style) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut style: Option<Style> = None;
    for &(c, s) in cells {
        if style != Some(s) {
            if let Some(prev) = style {
                spans.push(Span::styled(std::mem::take(&mut buf), prev));
            }
            style = Some(s);
        }
        buf.push(c);
    }
    if let Some(s) = style {
        spans.push(Span::styled(buf, s));
    }
    Line::from(spans).style(line_style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn parses_list_prefixes() {
        assert_eq!(parse_list_prefix("- item").unwrap().marker, ListMarker::Bullet('-'));
        assert_eq!(parse_list_prefix("  3. step").unwrap().marker, ListMarker::Ordered(3, '.'));
        let p = parse_list_prefix("  - [x] done").unwrap();
        assert_eq!(p.marker, ListMarker::Check { bullet: '-', checked: true });
        assert_eq!(&"  - [x] done"[p.end..], "done");
        assert!(parse_list_prefix("**bold**").is_none());
        assert!(parse_list_prefix("---").is_none());
        assert_eq!(parse_list_prefix("- [link](x)").unwrap().marker, ListMarker::Bullet('-'));
    }

    #[test]
    fn toggles_checkboxes_and_counts_progress() {
        assert_eq!(toggle_checkbox("- [ ] milk").as_deref(), Some("- [x] milk"));
        assert_eq!(toggle_checkbox("  * [X] eggs").as_deref(), Some("  * [ ] eggs"));
        assert_eq!(toggle_checkbox("- plain"), None);
        assert_eq!(checklist_progress("- [x] a\n- [ ] b\ntext\n- [x] c"), (2, 3));
    }

    #[test]
    fn renders_inline_and_block_markup() {
        let doc = render_document(
            &[
                "# Title".into(),
                "Some **bold** and `code` with snake_case_name".into(),
                "- [x] done".into(),
                "```".into(),
                "# not a heading".into(),
                "```".into(),
            ],
            20,
        );
        assert_eq!(plain(&doc[0]), "Title");
        assert_eq!(plain(&doc[1]), "Some bold and code with snake_case_name");
        assert!(doc[1].spans.iter().any(|s| s.content == "bold" && s.style.add_modifier.contains(Modifier::BOLD)));
        assert_eq!(plain(&doc[2]), "☑ done");
        assert_eq!(plain(&doc[4]), "# not a heading");
    }

    #[test]
    fn wraps_on_word_boundaries() {
        let line = Line::from("hello brave new world");
        let rows: Vec<String> = wrap_line(&line, 11).iter().map(plain).collect();
        assert_eq!(rows, vec!["hello ", "brave new ", "world"]);
    }
}
