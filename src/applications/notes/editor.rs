/// Notes — multi-line text buffer with undo/redo, list continuation and
/// checklist helpers. The cursor column is a *char* index, not a byte offset.

use super::markdown::{parse_list_prefix, toggle_checkbox, ListMarker};

const MAX_UNDO: usize = 200;

#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
    Other,
}

pub struct EditorBuffer {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<EditKind>,
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(s.len())
}

impl EditorBuffer {
    pub fn from_text(text: &str) -> Self {
        Self {
            lines: text.split('\n').map(str::to_string).collect(),
            row: 0,
            col: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            last_kind: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    pub fn word_count(&self) -> usize {
        self.lines.iter().map(|l| l.split_whitespace().count()).sum()
    }

    pub fn char_count(&self) -> usize {
        self.lines.iter().map(|l| char_len(l)).sum::<usize>() + self.lines.len() - 1
    }

    fn line_len(&self) -> usize {
        char_len(&self.lines[self.row])
    }

    fn cursor_byte(&self) -> usize {
        byte_idx(&self.lines[self.row], self.col)
    }

    // ----- Undo / redo -----

    fn snapshot(&self) -> Snapshot {
        Snapshot { lines: self.lines.clone(), row: self.row, col: self.col }
    }

    fn restore(&mut self, s: Snapshot) {
        self.lines = s.lines;
        self.row = s.row.min(self.lines.len() - 1);
        self.col = s.col.min(self.line_len());
    }

    /// Record an undo point. Consecutive edits of the same kind (typing a
    /// word, holding backspace) are grouped into one undo step.
    fn checkpoint(&mut self, kind: EditKind) {
        if self.last_kind != Some(kind) || kind == EditKind::Other {
            self.undo.push(self.snapshot());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_kind = Some(kind);
    }

    /// End the current undo group (e.g. after the cursor moves).
    pub fn break_undo_group(&mut self) {
        self.last_kind = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(self.snapshot());
        self.restore(s);
        self.last_kind = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(self.snapshot());
        self.restore(s);
        self.last_kind = None;
        true
    }

    // ----- Editing -----

    pub fn insert_char(&mut self, ch: char) {
        if ch == '\n' {
            return self.newline();
        }
        if ch.is_whitespace() {
            self.break_undo_group();
        }
        self.checkpoint(EditKind::Insert);
        let at = self.cursor_byte();
        self.lines[self.row].insert(at, ch);
        self.col += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.insert_char(ch);
        }
    }

    /// Split the line at the cursor. On a list item the new line continues the
    /// list; pressing Enter on an empty list item removes its marker instead.
    pub fn newline(&mut self) {
        self.checkpoint(EditKind::Other);
        let line = &self.lines[self.row];
        let at = self.cursor_byte();

        if let Some(prefix) = parse_list_prefix(line) {
            if at >= prefix.end {
                if line[prefix.end..].trim().is_empty() {
                    self.lines[self.row] = line[..prefix.indent].to_string();
                    self.col = prefix.indent;
                    return;
                }
                let lead = format!("{}{}", &line[..prefix.indent], prefix.marker.continuation());
                let rest = line[at..].to_string();
                self.lines[self.row].truncate(at);
                self.row += 1;
                self.col = char_len(&lead);
                self.lines.insert(self.row, lead + &rest);
                self.renumber_from(self.row);
                return;
            }
        }

        let rest = self.lines[self.row].split_off(at);
        self.row += 1;
        self.lines.insert(self.row, rest);
        self.col = 0;
    }

    /// Keep an ordered list sequential after inserting an item.
    fn renumber_from(&mut self, row: usize) {
        let Some(first) = parse_list_prefix(&self.lines[row]) else { return };
        let ListMarker::Ordered(mut n, _) = first.marker else { return };
        for r in row + 1..self.lines.len() {
            let Some(p) = parse_list_prefix(&self.lines[r]) else { break };
            match p.marker {
                ListMarker::Ordered(_, d) if p.indent == first.indent => {
                    n += 1;
                    let line = &self.lines[r];
                    self.lines[r] = format!("{}{}{} {}", &line[..p.indent], n, d, &line[p.end..]);
                }
                _ => break,
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.col > 0 {
            self.checkpoint(EditKind::Delete);
            self.col -= 1;
            let at = self.cursor_byte();
            self.lines[self.row].remove(at);
        } else if self.row > 0 {
            self.checkpoint(EditKind::Other);
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len();
            self.lines[self.row].push_str(&current);
        }
    }

    pub fn delete(&mut self) {
        if self.col < self.line_len() {
            self.checkpoint(EditKind::Delete);
            let at = self.cursor_byte();
            self.lines[self.row].remove(at);
        } else if self.row + 1 < self.lines.len() {
            self.checkpoint(EditKind::Other);
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    /// Ctrl+L: tick/untick a checklist item, or turn the line into one.
    pub fn toggle_checklist(&mut self) {
        self.checkpoint(EditKind::Other);
        let line = self.lines[self.row].clone();
        if let Some(toggled) = toggle_checkbox(&line) {
            self.lines[self.row] = toggled;
            return;
        }
        let (at, insert) = match parse_list_prefix(&line) {
            Some(p) if matches!(p.marker, ListMarker::Bullet(_)) => (p.end, "[ ] ".to_string()),
            Some(p) => {
                // Replace a numbered marker with a checkbox
                self.lines[self.row] = format!("{}- [ ] {}", &line[..p.indent], &line[p.end..]);
                let removed = char_len(&line[p.indent..p.end]);
                self.col = (self.col + 6).saturating_sub(removed).min(self.line_len());
                return;
            }
            None => {
                let indent = line.len() - line.trim_start_matches(' ').len();
                (indent, "- [ ] ".to_string())
            }
        };
        self.lines[self.row].insert_str(at, &insert);
        if self.col >= char_len(&line[..at]) {
            self.col += char_len(&insert);
        }
    }

    /// Toggle the checkbox on a given line (used by the preview). Returns true
    /// if the line was a checklist item.
    pub fn toggle_checkbox_at(&mut self, row: usize) -> bool {
        let Some(toggled) = self.lines.get(row).and_then(|l| toggle_checkbox(l)) else {
            return false;
        };
        self.checkpoint(EditKind::Other);
        self.lines[row] = toggled;
        true
    }

    // ----- Cursor movement (soft-wrap aware for Up/Down) -----

    pub fn move_left(&mut self) {
        self.break_undo_group();
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len();
        }
    }

    pub fn move_right(&mut self) {
        self.break_undo_group();
        if self.col < self.line_len() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    pub fn move_up(&mut self, width: usize) {
        self.break_undo_group();
        let w = width.max(1);
        if self.col >= w {
            self.col -= w;
        } else if self.row > 0 {
            let offset = self.col;
            self.row -= 1;
            let len = self.line_len();
            let last_seg = len.saturating_sub(1) / w;
            self.col = (last_seg * w + offset).min(len);
        } else {
            self.col = 0;
        }
    }

    pub fn move_down(&mut self, width: usize) {
        self.break_undo_group();
        let w = width.max(1);
        let len = self.line_len();
        if (self.col / w + 1) * w < len {
            self.col = (self.col + w).min(len);
        } else if self.row + 1 < self.lines.len() {
            let offset = self.col % w;
            self.row += 1;
            self.col = offset.min(self.line_len());
        } else {
            self.col = len;
        }
    }

    pub fn home(&mut self) {
        self.break_undo_group();
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.break_undo_group();
        self.col = self.line_len();
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.break_undo_group();
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.line_len());
    }
}

/// Number of visual rows a line of `len` chars occupies at `width`.
pub fn visual_rows(len: usize, width: usize) -> usize {
    if len == 0 {
        1
    } else {
        len.div_ceil(width.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> EditorBuffer {
        let mut b = EditorBuffer::from_text("");
        b.insert_str(text);
        b
    }

    #[test]
    fn edits_multibyte_text_by_char() {
        let mut b = typed("héllo");
        b.move_left();
        b.move_left();
        b.backspace();
        assert_eq!(b.text(), "hélo");
        b.delete();
        assert_eq!(b.text(), "héo");
        assert_eq!(b.char_count(), 3);
    }

    #[test]
    fn continues_and_ends_lists() {
        let mut b = typed("- [x] milk\n");
        assert_eq!(b.text(), "- [x] milk\n- [ ] ");
        b.newline();
        assert_eq!(b.text(), "- [x] milk\n");

        let mut b = EditorBuffer::from_text("1. one\n2. two");
        b.set_cursor(0, 6);
        b.insert_str("\nmid");
        assert_eq!(b.text(), "1. one\n2. mid\n3. two");
    }

    #[test]
    fn undo_groups_words_and_redo_restores() {
        let mut b = typed("hello world");
        assert!(b.undo());
        assert_eq!(b.text(), "hello");
        assert!(b.undo());
        assert_eq!(b.text(), "");
        assert!(!b.undo());
        assert!(b.redo());
        assert!(b.redo());
        assert_eq!(b.text(), "hello world");
        b.insert_char('!');
        assert!(!b.redo(), "a new edit clears the redo stack");
    }

    #[test]
    fn toggle_checklist_converts_and_ticks() {
        let mut b = typed("buy milk");
        b.toggle_checklist();
        assert_eq!(b.text(), "- [ ] buy milk");
        assert_eq!(b.col, 14);
        b.toggle_checklist();
        assert_eq!(b.text(), "- [x] buy milk");
        let mut b = typed("- eggs");
        b.toggle_checklist();
        assert_eq!(b.text(), "- [ ] eggs");
        assert!(b.toggle_checkbox_at(0));
        assert_eq!(b.text(), "- [x] eggs");
    }

    #[test]
    fn vertical_movement_follows_soft_wrap() {
        let mut b = EditorBuffer::from_text("abcdefghij\nxy");
        b.set_cursor(0, 2);
        b.move_down(4);
        assert_eq!((b.row, b.col), (0, 6));
        b.move_down(4);
        assert_eq!((b.row, b.col), (0, 10));
        b.move_down(4);
        assert_eq!((b.row, b.col), (1, 2));
        b.move_up(4);
        assert_eq!((b.row, b.col), (0, 10));
    }
}
