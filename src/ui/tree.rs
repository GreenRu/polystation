//! The hierarchy: a tree of rows with an expand arrow, a visibility checkbox and a name.
use super::render::alpha;
use super::scroll::VScroll;
use super::style::*;

pub struct Node {
    parent: Option<usize>,
    children: Vec<usize>,
    pub text: String,
    pub checked: bool,
    pub expanded: bool,
    pub bold: bool,
    /// This object has a script: a small button that opens it.
    pub badge: bool,
}

/// Where a dragged row would land relative to the row under the mouse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropAt {
    Before,
    After,
    Inside,
    /// Below an expanded row: as its first child.
    First,
}

pub enum Hit {
    Row(usize),
    Arrow(usize),
    Check(usize),
    Badge(usize),
    Nothing,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    pub selected: Option<usize>,
    pub hover: Option<usize>,
    pub scroll: VScroll,
    /// The row being dragged and where the mouse is (only once the drag has started).
    pub drag: Option<(usize, (f32, f32))>,
    /// Where it would land.
    pub drop: Option<(usize, DropAt)>,
}

pub const ROW_H: f32 = 22.0;

impl Tree {
    pub fn clear(&mut self) {
        self.drag = None;
        self.drop = None;
        self.nodes.clear();
        self.roots.clear();
        self.selected = None;
        self.hover = None;
    }

    /// Adds a node (under `parent`, or at the top) and returns its index.
    pub fn add(&mut self, parent: Option<usize>, text: &str, checked: bool) -> usize {
        let i = self.nodes.len();
        self.nodes.push(Node { parent, children: Vec::new(), text: text.to_string(), checked, expanded: false, bold: false, badge: false });
        match parent {
            Some(p) if p < i => self.nodes[p].children.push(i),
            _ => self.roots.push(i),
        }
        i
    }

    pub fn node(&self, i: usize) -> Option<&Node> {
        self.nodes.get(i)
    }

    pub fn node_mut(&mut self, i: usize) -> Option<&mut Node> {
        self.nodes.get_mut(i)
    }

    /// The rows that are showing: (node, depth), top to bottom.
    pub fn rows(&self) -> Vec<(usize, usize)> {
        fn walk(t: &Tree, list: &[usize], depth: usize, out: &mut Vec<(usize, usize)>) {
            for &n in list {
                out.push((n, depth));
                if t.nodes[n].expanded {
                    walk(t, &t.nodes[n].children, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &self.roots, 0, &mut out);
        out
    }

    pub fn has_children(&self, i: usize) -> bool {
        self.nodes.get(i).is_some_and(|n| !n.children.is_empty())
    }

    /// Opens every parent of a node so that it shows.
    pub fn reveal(&mut self, mut i: usize) {
        while let Some(p) = self.nodes.get(i).and_then(|n| n.parent) {
            self.nodes[p].expanded = true;
            i = p;
        }
    }

    pub fn content_height(&self, s: f32) -> f32 {
        self.rows().len() as f32 * (ROW_H * s).round() + 8.0 * s
    }

    fn indent(depth: usize, s: f32) -> f32 {
        (6.0 + depth as f32 * 18.0) * s
    }

    /// What is at a point (window pixels) of a tree drawn in `rect`.
    pub fn hit(&self, p: (f32, f32), rect: [f32; 4], s: f32) -> Hit {
        let rh = (ROW_H * s).round();
        let row = ((p.1 - rect[1] + self.scroll.offset) / rh).floor();
        let rows = self.rows();
        if row < 0.0 || row as usize >= rows.len() || p.0 < rect[0] || p.0 > rect[0] + rect[2] {
            return Hit::Nothing;
        }
        let (n, depth) = rows[row as usize];
        let x = p.0 - rect[0];
        let ind = Self::indent(depth, s);
        if self.nodes[n].badge && x >= rect[2] - 46.0 * s && x < rect[2] - 16.0 * s {
            Hit::Badge(n)
        } else if self.has_children(n) && x >= ind - 4.0 * s && x < ind + 14.0 * s {
            Hit::Arrow(n)
        } else if x >= ind + 16.0 * s && x < ind + 34.0 * s {
            Hit::Check(n)
        } else {
            Hit::Row(n)
        }
    }

    /// True when `anc` is `i` or above it in this tree.
    pub fn is_ancestor_or_self(&self, anc: usize, mut i: usize) -> bool {
        loop {
            if i == anc {
                return true;
            }
            match self.nodes.get(i).and_then(|n| n.parent) {
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    /// Where dropping `dragged` at point `p` of a tree drawn in `rect` would put it.
    pub fn drop_target(&self, p: (f32, f32), rect: [f32; 4], s: f32, dragged: usize) -> Option<(usize, DropAt)> {
        let rh = (ROW_H * s).round();
        let rows = self.rows();
        if rows.is_empty() || p.0 < rect[0] || p.0 > rect[0] + rect[2] || p.1 < rect[1] || p.1 > rect[1] + rect[3] {
            return None;
        }
        let y = p.1 - rect[1] + self.scroll.offset;
        let (row, frac) = if y >= rows.len() as f32 * rh { (rows.len() - 1, 1.0) } else { ((y / rh).floor() as usize, (y / rh).fract()) };
        let n = rows[row].0;
        if self.is_ancestor_or_self(dragged, n) {
            return None;
        }
        let at = if frac < 0.28 {
            DropAt::Before
        } else if frac > 0.72 {
            if self.nodes[n].expanded && !self.nodes[n].children.is_empty() { DropAt::First } else { DropAt::After }
        } else {
            DropAt::Inside
        };
        Some((n, at))
    }

    /// Moves the selection one row up / down (keyboard). Returns the new selection.
    pub fn step(&mut self, dir: i32) -> Option<usize> {
        let rows = self.rows();
        let pos = self.selected.and_then(|s| rows.iter().position(|r| r.0 == s));
        let next = match pos {
            Some(p) => (p as i32 + dir).clamp(0, rows.len() as i32 - 1),
            None => 0,
        };
        let n = rows.get(next as usize)?.0;
        self.selected = Some(n);
        Some(n)
    }

    pub fn draw(&mut self, c: &mut Ctx, rect: [f32; 4], focused: bool) {
        let s = c.s;
        let [x, y, w, h] = rect;
        c.r.fill(x, y, w, h, c_tree_bg());
        let rh = (ROW_H * s).round();
        let content = self.content_height(s);
        self.scroll.clamp(content, h);
        c.r.push_clip(x, y, w, h);
        let rows = self.rows();
        let first = (self.scroll.offset / rh).floor() as usize;
        let last = (((self.scroll.offset + h) / rh).ceil() as usize + 1).min(rows.len());
        for (i, &(n, depth)) in rows.iter().enumerate().take(last).skip(first) {
            let ry = y + i as f32 * rh - self.scroll.offset;
            let node = &self.nodes[n];
            if self.selected == Some(n) {
                c.r.rect(x + 4.0 * s, ry + 1.0, w - 8.0 * s, rh - 2.0, 5.0 * s, if focused { c_selection() } else { alpha(c_selection(), 0.5) }, 0.0, c_selection());
            } else if self.hover == Some(n) {
                c.r.rect(x + 4.0 * s, ry + 1.0, w - 8.0 * s, rh - 2.0, 5.0 * s, alpha([1.0; 4], 0.05), 0.0, c_selection());
            }
            let ind = x + Self::indent(depth, s);
            if !node.children.is_empty() {
                c.arrow(ind + 4.0 * s, ry + rh * 0.5, 5.0 * s, node.expanded, c_muted());
            }
            let cb = (14.0 * s).round();
            let (cx, cy) = (ind + 18.0 * s, ry + ((rh - cb) * 0.5).floor());
            if node.checked {
                c.r.rect(cx, cy, cb, cb, 4.0 * s, c_accent(), 0.0, c_accent());
                c.check_mark(cx, cy, cb, [1.0; 4]);
            } else {
                c.r.rect(cx, cy, cb, cb, 4.0 * s, c_field(), 1.0, c_border());
            }
            let font = if node.bold { &c.f.bold } else { &c.f.ui };
            let color = if node.checked { c_text() } else { c_dim() };
            c.text_in(font, &node.text, cx + cb + 8.0 * s, ry, rh, color);
            if node.badge {
                let b = [x + w - 46.0 * s, ry + ((rh - 16.0 * s) * 0.5).floor(), 30.0 * s, 16.0 * s];
                c.r.rect(b[0], b[1], b[2], b[3], 4.0 * s, if self.hover == Some(n) { c_accent() } else { alpha(c_accent(), 0.55) }, 0.0, c_accent());
                c.text_center(&c.f.bold, "{ }", b[0], b[1], b[2], b[3], [1.0; 4]);
            }
        }
        // drag and drop feedback
        if let Some((n, at)) = self.drop {
            if let Some(i) = rows.iter().position(|r| r.0 == n) {
                let ry = y + i as f32 * rh - self.scroll.offset;
                let ind = x + Self::indent(rows[i].1, s);
                match at {
                    DropAt::Inside => c.r.rect(x + 4.0 * s, ry + 1.0, w - 8.0 * s, rh - 2.0, 5.0 * s, alpha(c_accent(), 0.22), 1.5, c_accent()),
                    DropAt::Before => c.r.fill(ind, ry - 1.0, x + w - ind - 8.0 * s, 2.0, c_accent()),
                    DropAt::After => c.r.fill(ind, ry + rh - 1.0, x + w - ind - 8.0 * s, 2.0, c_accent()),
                    DropAt::First => c.r.fill(ind + 18.0 * s, ry + rh - 1.0, x + w - ind - 26.0 * s, 2.0, c_accent()),
                }
            }
        }
        if let Some((n, m)) = self.drag {
            let label = self.nodes[n].text.clone();
            let tw = c.f.ui.measure(&label) + 20.0 * s;
            c.r.rect(m.0 + 12.0 * s, m.1 + 6.0 * s, tw, rh, 6.0 * s, alpha(c_selection(), 0.9), 0.0, c_selection());
            c.text_in(&c.f.ui, &label, m.0 + 22.0 * s, m.1 + 6.0 * s, rh, c_text());
        }
        c.r.pop_clip();
        let track = [x + w - 12.0 * s, y, 12.0 * s, h];
        if let Some(t) = self.scroll.thumb(track, content, h) {
            c.scrollbar(t, self.scroll.dragging());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_expansion_and_hits_find_the_zones() {
        let mut t = Tree::default();
        let scene = t.add(None, "Scene", true);
        let a = t.add(Some(scene), "A", true);
        let b = t.add(Some(scene), "B", false);
        let c = t.add(Some(a), "C", true);
        assert_eq!(t.rows(), vec![(scene, 0)], "collapsed");
        t.node_mut(scene).unwrap().expanded = true;
        assert_eq!(t.rows(), vec![(scene, 0), (a, 1), (b, 1)]);
        t.reveal(c);
        assert_eq!(t.rows().len(), 4);
        let rect = [0.0, 0.0, 280.0, 400.0];
        // row 1 is "A" at depth 1: indent 24 -> arrow at ~24, checkbox from 40, name after that
        assert!(matches!(t.hit((26.0, 33.0), rect, 1.0), Hit::Arrow(n) if n == a));
        assert!(matches!(t.hit((46.0, 33.0), rect, 1.0), Hit::Check(n) if n == a));
        assert!(matches!(t.hit((120.0, 33.0), rect, 1.0), Hit::Row(n) if n == a));
        assert!(matches!(t.hit((120.0, 300.0), rect, 1.0), Hit::Nothing));
    }

    #[test]
    fn keyboard_steps_through_the_visible_rows() {
        let mut t = Tree::default();
        let r = t.add(None, "r", true);
        let x = t.add(Some(r), "x", true);
        let _y = t.add(Some(r), "y", true);
        assert_eq!(t.step(1), Some(r));
        assert_eq!(t.step(1), Some(r), "children are hidden while collapsed");
        t.node_mut(r).unwrap().expanded = true;
        assert_eq!(t.step(1), Some(x));
        assert_eq!(t.step(-1), Some(r));
        assert_eq!(t.step(-1), Some(r));
    }
}
