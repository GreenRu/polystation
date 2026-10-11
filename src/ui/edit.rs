//! The text editing model behind every text box of the UI (one line or many): the text, a caret and a
//! selection, with the usual editing keys, word movement, auto-indent, clipboard operations and undo.
//! It knows nothing about drawing, so it can be tested on its own.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    Enter,
    Tab,
}

#[derive(Clone)]
struct Snapshot {
    chars: Vec<char>,
    caret: usize,
    anchor: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    None,
    Typing,
    Deleting,
}

#[derive(Clone)]
pub struct Edit {
    pub chars: Vec<char>,
    pub caret: usize,
    pub anchor: usize,
    pub multiline: bool,
    /// Column to return to when moving up and down through lines of different lengths.
    want_col: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Last,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Edit {
    pub fn new(text: &str, multiline: bool) -> Edit {
        let mut e = Edit { chars: Vec::new(), caret: 0, anchor: 0, multiline, want_col: None, undo: Vec::new(), redo: Vec::new(), last: Last::None };
        e.set_text(text);
        e
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Replaces the whole text without touching the undo history (the caret goes to the end).
    pub fn set_text(&mut self, text: &str) {
        self.chars = text.replace("\r\n", "\n").replace('\r', "\n").chars().filter(|&c| self.multiline || c != '\n').collect();
        self.caret = self.chars.len();
        self.anchor = self.caret;
        self.want_col = None;
        self.last = Last::None;
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        self.chars[a..b].iter().collect()
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.chars.len();
        self.want_col = None;
    }

    pub fn set_caret(&mut self, pos: usize, extend: bool) {
        self.caret = pos.min(self.chars.len());
        if !extend {
            self.anchor = self.caret;
        }
        self.want_col = None;
        self.last = Last::None;
    }

    // ---------------------------------------------------------------- lines
    pub fn line_count(&self) -> usize {
        1 + self.chars.iter().filter(|&&c| c == '\n').count()
    }

    /// Index of the first character of a line.
    pub fn line_start(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let mut seen = 0;
        for (i, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                seen += 1;
                if seen == line {
                    return i + 1;
                }
            }
        }
        self.chars.len()
    }

    /// Index just past the last character of a line (before its line break).
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        self.chars[start..].iter().position(|&c| c == '\n').map_or(self.chars.len(), |p| start + p)
    }

    pub fn line_text(&self, line: usize) -> String {
        self.chars[self.line_start(line)..self.line_end(line)].iter().collect()
    }

    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.chars.len());
        let line = self.chars[..pos].iter().filter(|&&c| c == '\n').count();
        (line, pos - self.line_start(line))
    }

    pub fn pos_from(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count() - 1);
        (self.line_start(line) + col).min(self.line_end(line))
    }

    // ---------------------------------------------------------------- undo
    fn snapshot(&self) -> Snapshot {
        Snapshot { chars: self.chars.clone(), caret: self.caret, anchor: self.anchor }
    }

    fn push_undo(&mut self, kind: Last) {
        // consecutive typing (or deleting) forms one undo step
        if kind == Last::None || self.last != kind || self.has_selection() {
            self.undo.push(self.snapshot());
            if self.undo.len() > 300 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = kind;
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(self.snapshot());
        (self.chars, self.caret, self.anchor) = (s.chars, s.caret, s.anchor);
        self.last = Last::None;
        self.want_col = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(self.snapshot());
        (self.chars, self.caret, self.anchor) = (s.chars, s.caret, s.anchor);
        self.last = Last::None;
        self.want_col = None;
        true
    }

    // ---------------------------------------------------------------- editing
    fn delete_selection(&mut self) {
        let (a, b) = self.selection();
        self.chars.drain(a..b);
        self.caret = a;
        self.anchor = a;
    }

    /// Types text at the caret (replacing the selection). Line breaks are dropped in one-line boxes.
    pub fn insert(&mut self, s: &str) -> bool {
        let s: String = s.replace("\r\n", "\n").replace('\r', "\n").chars().filter(|&c| (self.multiline || c != '\n') && (c == '\n' || c == '\t' || !c.is_control())).collect();
        if s.is_empty() && !self.has_selection() {
            return false;
        }
        self.push_undo(if s.chars().count() == 1 && !s.contains('\n') && !self.has_selection() { Last::Typing } else { Last::None });
        if self.has_selection() {
            self.delete_selection();
        }
        for (i, c) in s.chars().enumerate() {
            self.chars.insert(self.caret + i, c);
        }
        self.caret += s.chars().count();
        self.anchor = self.caret;
        self.want_col = None;
        true
    }

    pub fn char_input(&mut self, c: char) -> bool {
        if c == '\r' || c == '\n' || c == '\t' || c.is_control() {
            return false;
        }
        self.insert(&c.to_string())
    }

    pub fn copy(&self) -> Option<String> {
        self.has_selection().then(|| self.selected_text())
    }

    pub fn cut(&mut self) -> Option<String> {
        let t = self.copy()?;
        self.push_undo(Last::None);
        self.delete_selection();
        Some(t)
    }

    fn word_left(&self, mut p: usize) -> usize {
        while p > 0 && !is_word(self.chars[p - 1]) && self.chars[p - 1] != '\n' {
            p -= 1;
        }
        while p > 0 && is_word(self.chars[p - 1]) {
            p -= 1;
        }
        p
    }

    fn word_right(&self, mut p: usize) -> usize {
        let n = self.chars.len();
        while p < n && !is_word(self.chars[p]) && self.chars[p] != '\n' {
            p += 1;
        }
        while p < n && is_word(self.chars[p]) {
            p += 1;
        }
        p
    }

    /// The word around a position (for double-clicks).
    pub fn word_at(&self, pos: usize) -> (usize, usize) {
        let n = self.chars.len();
        let pos = pos.min(n);
        if pos < n && is_word(self.chars[pos]) || pos > 0 && is_word(self.chars[pos - 1]) {
            let mut a = pos;
            while a > 0 && is_word(self.chars[a - 1]) {
                a -= 1;
            }
            let mut b = pos;
            while b < n && is_word(self.chars[b]) {
                b += 1;
            }
            (a, b)
        } else {
            (pos, (pos + 1).min(n))
        }
    }

    fn indent_of(&self, line: usize) -> usize {
        self.line_text(line).chars().take_while(|&c| c == ' ').count()
    }

    /// Handles an editing / navigation key. Returns true when the text changed.
    pub fn key(&mut self, k: KeyCode, shift: bool, ctrl: bool) -> bool {
        use KeyCode::*;
        let n = self.chars.len();
        let mut vertical = false;
        let mut changed = false;
        match k {
            Left => {
                let to = if self.has_selection() && !shift { self.selection().0 } else if ctrl { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
                self.caret = to;
            }
            Right => {
                let to = if self.has_selection() && !shift { self.selection().1 } else if ctrl { self.word_right(self.caret) } else { (self.caret + 1).min(n) };
                self.caret = to;
            }
            Up | Down | PageUp | PageDown => {
                if !self.multiline {
                    self.caret = if matches!(k, Up | PageUp) { 0 } else { n };
                } else {
                    vertical = true;
                    let (line, col) = self.line_col(self.caret);
                    let want = *self.want_col.get_or_insert(col);
                    let step = if matches!(k, PageUp | PageDown) { 12 } else { 1 };
                    let target = if matches!(k, Up | PageUp) { line.saturating_sub(step) } else { (line + step).min(self.line_count() - 1) };
                    self.caret = if matches!(k, Up | PageUp) && line == 0 { 0 } else if matches!(k, Down | PageDown) && line + 1 >= self.line_count() { n } else { self.pos_from(target, want) };
                }
            }
            Home => {
                self.caret = if ctrl || !self.multiline {
                    0
                } else {
                    let (line, _) = self.line_col(self.caret);
                    let start = self.line_start(line);
                    let first = start + self.indent_of(line);
                    if self.caret == first { start } else { first } // Home goes to the text first, then to the very start
                };
            }
            End => {
                self.caret = if ctrl || !self.multiline {
                    n
                } else {
                    let (line, _) = self.line_col(self.caret);
                    self.line_end(line)
                };
            }
            Backspace | Delete => {
                if self.has_selection() {
                    self.push_undo(Last::None);
                    self.delete_selection();
                    changed = true;
                } else {
                    let range = if k == Backspace {
                        (if ctrl { self.word_left(self.caret) } else { self.caret.saturating_sub(1) }, self.caret)
                    } else {
                        (self.caret, if ctrl { self.word_right(self.caret) } else { (self.caret + 1).min(n) })
                    };
                    if range.0 < range.1 {
                        self.push_undo(Last::Deleting);
                        self.chars.drain(range.0..range.1);
                        self.caret = range.0;
                        changed = true;
                    }
                }
                self.anchor = self.caret;
                self.want_col = None;
                return changed;
            }
            Enter => {
                if !self.multiline {
                    return false;
                }
                let (line, col) = self.line_col(self.caret.min(self.selection().0));
                let before: Vec<char> = self.chars[self.line_start(line)..self.selection().0].to_vec();
                let mut indent = self.indent_of(line).min(col);
                let opens = before.iter().rev().find(|c| !c.is_whitespace()) == Some(&'{');
                let closes = self.chars.get(self.selection().1..).and_then(|r| r.iter().find(|c| !c.is_whitespace())) == Some(&'}');
                let mut text = String::from("\n");
                if opens {
                    indent += 4;
                }
                text.push_str(&" ".repeat(indent));
                if opens && closes {
                    text.push('\n');
                    text.push_str(&" ".repeat(indent - 4));
                    self.insert(&text);
                    // the caret goes on the indented middle line
                    self.caret -= 1 + indent - 4;
                    self.anchor = self.caret;
                    return true;
                }
                return self.insert(&text);
            }
            Tab => {
                if !self.multiline {
                    return false;
                }
                if self.has_selection() && self.selected_text().contains('\n') {
                    // indent / unindent every selected line
                    self.push_undo(Last::None);
                    let (a, b) = self.selection();
                    let (l0, l1) = (self.line_col(a).0, self.line_col(b.saturating_sub(if self.chars.get(b.wrapping_sub(1)) == Some(&'\n') { 1 } else { 0 })).0);
                    let (mut caret, mut anchor) = (self.caret, self.anchor);
                    for line in (l0..=l1).rev() {
                        let s = self.line_start(line);
                        if shift {
                            let remove = self.indent_of(line).min(4);
                            self.chars.drain(s..s + remove);
                            for p in [&mut caret, &mut anchor] {
                                if *p > s {
                                    *p -= remove.min(*p - s);
                                }
                            }
                        } else {
                            for _ in 0..4 {
                                self.chars.insert(s, ' ');
                            }
                            for p in [&mut caret, &mut anchor] {
                                if *p >= s {
                                    *p += 4;
                                }
                            }
                        }
                    }
                    (self.caret, self.anchor) = (caret, anchor);
                    return true;
                }
                if shift {
                    let (line, col) = self.line_col(self.caret);
                    let remove = self.indent_of(line).min(4).min(col.max(self.indent_of(line)));
                    if remove == 0 {
                        return false;
                    }
                    self.push_undo(Last::None);
                    let s = self.line_start(line);
                    self.chars.drain(s..s + remove);
                    self.caret = self.caret.saturating_sub(remove).max(s);
                    self.anchor = self.caret;
                    return true;
                }
                let (_, col) = self.line_col(self.selection().0);
                return self.insert(&" ".repeat(4 - col % 4));
            }
        }
        if !shift {
            self.anchor = self.caret;
        }
        if !vertical {
            self.want_col = None;
        }
        self.last = Last::None;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(e: &mut Edit, s: &str) {
        for c in s.chars() {
            e.char_input(c);
        }
    }

    #[test]
    fn typing_deleting_and_selecting() {
        let mut e = Edit::new("", false);
        typed(&mut e, "hello world");
        assert_eq!(e.text(), "hello world");
        e.key(KeyCode::Backspace, false, false);
        assert_eq!(e.text(), "hello worl");
        e.key(KeyCode::Left, false, true); // word left
        assert_eq!(e.caret, 6);
        e.key(KeyCode::End, true, false); // select to the end
        assert_eq!(e.selected_text(), "worl");
        typed(&mut e, "X");
        assert_eq!(e.text(), "hello X");
        e.select_all();
        assert_eq!(e.cut().as_deref(), Some("hello X"));
        assert!(e.is_empty());
        assert!(!e.insert("a\nb") || e.text() == "ab", "line breaks are dropped in one-line boxes");
    }

    #[test]
    fn word_movement_and_word_delete() {
        let mut e = Edit::new("alpha beta_2  gamma", false);
        e.set_caret(0, false);
        e.key(KeyCode::Right, false, true);
        assert_eq!(e.caret, 5);
        e.key(KeyCode::Right, false, true);
        assert_eq!(e.caret, 12);
        e.key(KeyCode::Backspace, false, true);
        assert_eq!(e.text(), "alpha   gamma");
        assert_eq!(e.word_at(1), (0, 5));
    }

    #[test]
    fn lines_keep_their_column_when_moving_up_and_down() {
        let mut e = Edit::new("long line here\nab\nanother long one", true);
        e.set_caret(10, false); // column 10 of the first line
        e.key(KeyCode::Down, false, false);
        assert_eq!(e.line_col(e.caret), (1, 2)); // the short line: end of it
        e.key(KeyCode::Down, false, false);
        assert_eq!(e.line_col(e.caret), (2, 10)); // back to the wanted column
        e.key(KeyCode::Up, false, false);
        e.key(KeyCode::Up, false, false);
        assert_eq!(e.line_col(e.caret), (0, 10));
        assert_eq!((e.line_count(), e.line_text(1)), (3, "ab".to_string()));
    }

    #[test]
    fn enter_keeps_the_indent_and_opens_braces() {
        let mut e = Edit::new("on update {", true);
        e.key(KeyCode::Enter, false, false);
        assert_eq!(e.text(), "on update {\n    ");
        typed(&mut e, "x = 1");
        e.key(KeyCode::Enter, false, false);
        assert_eq!(e.text(), "on update {\n    x = 1\n    ");
        // between a pair of braces the closing one moves to its own line
        let mut e = Edit::new("fn f() {}", true);
        e.set_caret(8, false);
        e.key(KeyCode::Enter, false, false);
        assert_eq!(e.text(), "fn f() {\n    \n}");
        assert_eq!(e.line_col(e.caret), (1, 4));
    }

    #[test]
    fn tab_inserts_spaces_and_indents_selected_lines() {
        let mut e = Edit::new("a\nb", true);
        e.set_caret(0, false);
        e.key(KeyCode::Tab, false, false);
        assert_eq!(e.text(), "    a\nb");
        e.select_all();
        e.key(KeyCode::Tab, false, false);
        assert_eq!(e.text(), "        a\n    b");
        e.key(KeyCode::Tab, true, false);
        assert_eq!(e.text(), "    a\nb");
    }

    #[test]
    fn undo_and_redo_group_typing() {
        let mut e = Edit::new("", true);
        typed(&mut e, "abc");
        e.key(KeyCode::Enter, false, false);
        typed(&mut e, "de");
        assert_eq!(e.text(), "abc\nde");
        assert!(e.undo());
        assert_eq!(e.text(), "abc\n"); // "de" went in one step
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(e.redo() && e.redo());
        assert_eq!(e.text(), "abc\n");
        assert!(!Edit::new("x", false).undo());
    }

    #[test]
    fn paste_replaces_the_selection_and_cleans_the_text() {
        let mut e = Edit::new("one two", false);
        e.set_caret(4, false);
        e.set_caret(7, true);
        e.insert("2\r\n\u{7}x");
        assert_eq!(e.text(), "one 2x");
        let mut m = Edit::new("", true);
        m.insert("a\r\nb\tc");
        assert_eq!(m.text(), "a\nb\tc");
    }
}
