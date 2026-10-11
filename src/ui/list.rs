//! A list with column headers (the file explorer) and a file-dialog flavoured variant of it.
use super::render::{alpha, rgb, Color};
use super::scroll::VScroll;
use super::style::*;

pub const ROW_H: f32 = 24.0;
pub const HEADER_H: f32 = 26.0;

pub struct List {
    /// Column title and width (96-dpi units); the last column takes what is left.
    pub cols: Vec<(String, f32)>,
    pub rows: Vec<Vec<String>>,
    pub selected: Option<usize>,
    pub hover: Option<usize>,
    pub scroll: VScroll,
    /// Show a small colored icon in front of the first column, chosen by the text of the second one.
    pub icons: bool,
}

pub fn kind_color(kind: &str) -> Color {
    match kind {
        "Folder" | "Parent folder" => rgb(0xe3b341),
        "Model" => rgb(0x58a6ff),
        "Image" => rgb(0x56d364),
        "Project" => rgb(0xbc8cff),
        "Sound" => rgb(0xff9bce),
        _ => rgb(0x8b949e),
    }
}

impl List {
    pub fn new(cols: &[(&str, f32)], icons: bool) -> List {
        List { cols: cols.iter().map(|&(t, w)| (t.to_string(), w)).collect(), rows: Vec::new(), selected: None, hover: None, scroll: VScroll::default(), icons }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.selected = None;
        self.hover = None;
        self.scroll.offset = 0.0;
    }

    pub fn add(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    pub fn content_height(&self, s: f32) -> f32 {
        self.rows.len() as f32 * (ROW_H * s).round() + 4.0 * s
    }

    /// The row at a point (window pixels) of a list drawn in `rect`.
    pub fn row_at(&self, p: (f32, f32), rect: [f32; 4], s: f32) -> Option<usize> {
        let top = rect[1] + (HEADER_H * s).round();
        if p.1 < top || p.0 < rect[0] || p.0 > rect[0] + rect[2] || p.1 > rect[1] + rect[3] {
            return None;
        }
        let i = ((p.1 - top + self.scroll.offset) / (ROW_H * s).round()).floor() as usize;
        (i < self.rows.len()).then_some(i)
    }

    pub fn step(&mut self, dir: i32, view_h: f32, s: f32) {
        if self.rows.is_empty() {
            return;
        }
        let next = match self.selected {
            Some(i) => (i as i32 + dir).clamp(0, self.rows.len() as i32 - 1) as usize,
            None => 0,
        };
        self.selected = Some(next);
        let rh = (ROW_H * s).round();
        let content = self.content_height(s);
        self.scroll.reveal(next as f32 * rh, (next + 1) as f32 * rh, content, view_h - (HEADER_H * s).round());
    }

    pub fn draw(&mut self, c: &mut Ctx, rect: [f32; 4], focused: bool) {
        let s = c.s;
        let [x, y, w, h] = rect;
        let hh = (HEADER_H * s).round();
        let rh = (ROW_H * s).round();
        c.r.fill(x, y, w, h, c_tree_bg());
        let view = h - hh;
        let content = self.content_height(s);
        self.scroll.clamp(content, view);
        // column positions
        let mut xs = Vec::new();
        let mut cx = x + 12.0 * s;
        for (i, (_, cw)) in self.cols.iter().enumerate() {
            xs.push(cx);
            cx += if i + 1 == self.cols.len() { 0.0 } else { cw * s };
        }
        // rows
        c.r.push_clip(x, y + hh, w, view);
        let first = (self.scroll.offset / rh).floor() as usize;
        let last = (((self.scroll.offset + view) / rh).ceil() as usize + 1).min(self.rows.len());
        for i in first..last {
            let ry = y + hh + i as f32 * rh - self.scroll.offset;
            if i % 2 == 1 {
                c.r.fill(x, ry, w, rh, alpha([1.0; 4], 0.02));
            }
            if self.selected == Some(i) {
                c.r.rect(x + 4.0 * s, ry + 1.0, w - 8.0 * s, rh - 2.0, 5.0 * s, if focused { c_selection() } else { alpha(c_selection(), 0.5) }, 0.0, c_selection());
            } else if self.hover == Some(i) {
                c.r.rect(x + 4.0 * s, ry + 1.0, w - 8.0 * s, rh - 2.0, 5.0 * s, alpha([1.0; 4], 0.05), 0.0, c_selection());
            }
            for (k, cell) in self.rows[i].iter().enumerate().take(self.cols.len()) {
                let mut tx = xs[k];
                let col_w = if k + 1 == self.cols.len() { x + w - tx - 10.0 * s } else { self.cols[k].1 * s - 10.0 * s };
                if k == 0 && self.icons {
                    let kind = self.rows[i].get(1).map_or("", |s| s.as_str());
                    let sz = (12.0 * s).round();
                    c.r.rect(tx, ry + ((rh - sz) * 0.5).floor(), sz, sz, 3.0 * s, kind_color(kind), 0.0, [0.0; 4]);
                    tx += sz + 8.0 * s;
                }
                let color = if k == 0 { c_text() } else { c_muted() };
                let text = c.f.ui.fit(cell, (col_w - (tx - xs[k])).max(10.0));
                c.text_in(&c.f.ui, &text, tx, ry, rh, color);
            }
        }
        c.r.pop_clip();
        // header
        c.r.fill(x, y, w, hh, c_bar());
        c.r.fill(x, y + hh - 1.0, w, 1.0, c_border_soft());
        for (k, (title, _)) in self.cols.iter().enumerate() {
            c.text_in(&c.f.bold, &title.to_uppercase(), xs[k], y, hh, c_muted());
            if k > 0 {
                c.r.fill(xs[k] - 8.0 * s, y + 6.0 * s, 1.0, hh - 12.0 * s, c_border_soft());
            }
        }
        let track = [x + w - 12.0 * s, y + hh, 12.0 * s, view];
        if let Some(t) = self.scroll.thumb(track, content, view) {
            c.scrollbar(t, self.scroll.dragging());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_found_below_the_header_and_follow_the_scroll() {
        let mut l = List::new(&[("Name", 100.0), ("Type", 60.0)], true);
        for i in 0..10 {
            l.add(vec![format!("f{i}"), "File".into()]);
        }
        let rect = [0.0, 100.0, 300.0, 200.0];
        assert_eq!(l.row_at((50.0, 100.0 + 30.0), rect, 1.0), Some(0));
        assert_eq!(l.row_at((50.0, 100.0 + 26.0 + 25.0), rect, 1.0), Some(1));
        assert_eq!(l.row_at((50.0, 110.0), rect, 1.0), None, "that is the header");
        l.scroll.offset = 48.0;
        assert_eq!(l.row_at((50.0, 100.0 + 26.0 + 1.0), rect, 1.0), Some(2));
        assert_eq!(l.row_at((50.0, 100.0 + 26.0 + 24.0 * 9.0), rect, 1.0), None);
    }

    #[test]
    fn keyboard_selection_keeps_the_row_visible() {
        let mut l = List::new(&[("Name", 100.0)], false);
        for i in 0..20 {
            l.add(vec![format!("f{i}")]);
        }
        for _ in 0..12 {
            l.step(1, 150.0, 1.0);
        }
        assert_eq!(l.selected, Some(11));
        assert!(l.scroll.offset > 0.0);
        let rh = 24.0;
        assert!(l.scroll.offset <= 11.0 * rh && l.scroll.offset + (150.0 - 26.0) >= 12.0 * rh - 0.5);
    }
}
