//! The GUI editor: a list of the game's GUI elements, a live preview of the screen where they can be
//! dragged around, and the properties of the selected element.
use super::dialogs::{dialog_button, frame, Dialog, DlgEvent, Hits};
use super::render::alpha;
use super::style::*;
use super::widgets::{Clipboard, Field};
use super::{inside, Rect};
use crate::gui::{self, GuiElem, Kind, Mode};

const ROW_H: f32 = 26.0;

// field indices
const F_NAME: usize = 0;
const F_TEXT: usize = 1;
const F_X: usize = 2;
const F_Y: usize = 3;
const F_W: usize = 4;
const F_H: usize = 5;
const F_VALUE: usize = 6;
const F_ALPHA: usize = 7;
const F_SIZE: usize = 8;
const F_COLOR: usize = 9;
const F_TEXT_COLOR: usize = 10;
const FIELDS: usize = 11;

pub struct GuiDialog {
    elems: Vec<GuiElem>,
    sel: Option<usize>,
    /// The edited elements once Apply was pressed (None while open or cancelled).
    pub result: Option<Vec<GuiElem>>,
    closed: bool,
    hits: Hits,
    fields: Vec<Field>,
    focus: Option<usize>,
    /// Dragging an element in the preview: (mouse at the start, x, y at the start).
    drag: Option<((f32, f32), f32, f32)>,
    preview: Rect,
    first_row: usize,
    /// How the mouse works while playing.
    pub mode: Mode,
}

fn num(v: f32) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl GuiDialog {
    pub fn new(elems: Vec<GuiElem>, mode: Mode) -> GuiDialog {
        let mut d = GuiDialog { mode, sel: (!elems.is_empty()).then_some(0), elems, result: None, closed: false, hits: Hits::default(), fields: (0..FIELDS).map(|_| Field::new(false, false)).collect(), focus: None, drag: None, preview: [0.0; 4], first_row: 0 };
        d.load();
        d
    }

    /// Puts the selected element's values into the boxes.
    fn load(&mut self) {
        let Some(e) = self.sel.and_then(|i| self.elems.get(i)).cloned() else {
            for f in &mut self.fields {
                f.edit.set_text("");
            }
            return;
        };
        let texts = [e.name.clone(), e.text.clone(), num(e.x), num(e.y), num(e.w), num(e.h), num(e.value), num(e.alpha), e.text_size.to_string(), gui::to_hex(e.color), gui::to_hex(e.text_color)];
        for (f, t) in self.fields.iter_mut().zip(texts) {
            f.edit.set_text(&t);
        }
    }

    /// Reads box `k` back into the selected element (boxes that do not parse leave it as it was).
    fn commit(&mut self, k: usize) {
        let Some(i) = self.sel else { return };
        let text = self.fields[k].edit.text();
        let number = || text.trim().replace(',', ".").parse::<f32>().ok().filter(|v| v.is_finite());
        match k {
            F_NAME => {
                let name = text.trim().to_string();
                if !name.is_empty() && !self.elems.iter().enumerate().any(|(j, e)| j != i && e.name == name) {
                    self.elems[i].name = name;
                }
            }
            F_TEXT => self.elems[i].text = text,
            F_X | F_Y | F_W | F_H => {
                if let Some(v) = number() {
                    let e = &mut self.elems[i];
                    match k {
                        F_X => e.x = v.clamp(-4000.0, 4000.0),
                        F_Y => e.y = v.clamp(-4000.0, 4000.0),
                        F_W => e.w = v.clamp(1.0, 4000.0),
                        _ => e.h = v.clamp(1.0, 4000.0),
                    }
                }
            }
            F_VALUE => {
                if let Some(v) = number() {
                    self.elems[i].value = v.clamp(0.0, 1.0);
                }
            }
            F_ALPHA => {
                if let Some(v) = number() {
                    self.elems[i].alpha = v.clamp(0.0, 1.0);
                }
            }
            F_SIZE => {
                if let Some(v) = number() {
                    self.elems[i].text_size = (v.round() as i32).clamp(1, 4) as u8;
                }
            }
            F_COLOR => {
                if let Some(c) = gui::parse_hex(&text) {
                    self.elems[i].color = c;
                }
            }
            _ => {
                if let Some(c) = gui::parse_hex(&text) {
                    self.elems[i].text_color = c;
                }
            }
        }
    }

    fn add(&mut self, kind: Kind) {
        let name = gui::unique_name(&self.elems, kind_label(kind));
        self.elems.push(GuiElem::new(kind, &name));
        self.sel = Some(self.elems.len() - 1);
        self.load();
        self.focus = None;
    }

    fn delete(&mut self) {
        if let Some(i) = self.sel {
            self.elems.remove(i);
            self.sel = if self.elems.is_empty() { None } else { Some(i.min(self.elems.len() - 1)) };
            self.load();
        }
    }

    /// Moves the selected element in the drawing order (later = in front).
    fn reorder(&mut self, forward: bool) {
        let Some(i) = self.sel else { return };
        let j = if forward { i + 1 } else { i.wrapping_sub(1) };
        if j < self.elems.len() {
            self.elems.swap(i, j);
            self.sel = Some(j);
        }
    }

    fn select(&mut self, i: usize) {
        if i < self.elems.len() && self.sel != Some(i) {
            self.sel = Some(i);
            self.load();
        }
        self.focus = None;
    }

    fn step_focus(&mut self, back: bool) {
        if self.sel.is_none() {
            return;
        }
        let order: Vec<usize> = (0..FIELDS).filter(|&k| self.enabled(k)).collect();
        let pos = self.focus.and_then(|f| order.iter().position(|&k| k == f));
        let next = match (pos, back) {
            (None, _) => order[0],
            (Some(p), false) => order[(p + 1) % order.len()],
            (Some(p), true) => order[(p + order.len() - 1) % order.len()],
        };
        self.focus = Some(next);
        self.fields[next].edit.select_all();
    }

    /// Boxes that do something for the selected kind.
    fn enabled(&self, k: usize) -> bool {
        let Some(e) = self.sel.and_then(|i| self.elems.get(i)) else { return false };
        match k {
            F_TEXT => e.kind != Kind::Panel,
            F_VALUE => e.kind == Kind::Bar,
            F_SIZE | F_TEXT_COLOR => e.kind != Kind::Panel,
            _ => true,
        }
    }
}

fn kind_label(k: Kind) -> &'static str {
    Kind::NAMES[k.index()]
}

fn kind_icon(k: Kind) -> &'static str {
    ["T", "P", "B", "O"][k.index()]
}

impl Dialog for GuiDialog {
    fn size(&self, c: &Ctx) -> (f32, f32) {
        (c.px(1090.0), c.px(620.0))
    }

    fn event(&mut self, ev: &DlgEvent, clip: &mut dyn Clipboard) {
        match ev {
            DlgEvent::Move(p) => {
                self.hits.mouse = *p;
                if let (Some(((sx, sy), x0, y0)), Some(i)) = (self.drag, self.sel) {
                    let k = gui::scale(self.preview);
                    let (dx, dy) = self.elems[i].direction();
                    self.elems[i].x = (x0 + (p.0 - sx) / k * dx).round();
                    self.elems[i].y = (y0 + (p.1 - sy) / k * dy).round();
                    self.fields[F_X].edit.set_text(&num(self.elems[i].x));
                    self.fields[F_Y].edit.set_text(&num(self.elems[i].y));
                }
            }
            DlgEvent::Wheel(n, p) => {
                if self.hits.at(*p).is_some_and(|id| id == 99 || id >= 100) {
                    let max = self.elems.len().saturating_sub(1);
                    self.first_row = (self.first_row as i32 - n.signum() as i32 * 2).clamp(0, max as i32) as usize;
                }
            }
            DlgEvent::Down(0, p, _) => {
                self.hits.pressed = self.hits.at(*p);
                if let Some(id) = self.hits.pressed {
                    match id {
                        50 => {
                            self.focus = None;
                            if let Some(i) = gui::hit_any(&self.elems, self.preview, *p) {
                                self.select(i);
                                self.drag = Some((*p, self.elems[i].x, self.elems[i].y));
                            }
                        }
                        id if (40..40 + FIELDS as u32).contains(&id) => {
                            let k = (id - 40) as usize;
                            if self.enabled(k) {
                                self.focus = Some(k);
                                self.fields[k].edit.select_all();
                            }
                        }
                        _ => self.focus = None,
                    }
                } else {
                    self.focus = None;
                }
            }
            DlgEvent::Up(0, p) => {
                self.drag = None;
                if let (Some(id), Some(up)) = (self.hits.pressed.take(), self.hits.at(*p)) {
                    if id != up {
                        return;
                    }
                    match id {
                        1 => {
                            self.result = Some(self.elems.clone());
                            self.closed = true;
                        }
                        2 => self.closed = true,
                        10..=13 => self.add(Kind::from_index((id - 10) as usize)),
                        60..=63 => self.mode = Mode::from_index((id - 60) as usize),
                        14 => self.delete(),
                        15 => self.reorder(false),
                        16 => self.reorder(true),
                        20..=28 => {
                            if let Some(i) = self.sel {
                                // keep the element where it is on screen as far as possible: only the anchor changes
                                self.elems[i].anchor = (id - 20) as u8;
                            }
                        }
                        30 => {
                            if let Some(i) = self.sel {
                                self.elems[i].visible = !self.elems[i].visible;
                            }
                        }
                        id if id >= 100 => self.select(id as usize - 100),
                        _ => {}
                    }
                }
            }
            DlgEvent::Key(vk, m) => match *vk {
                0x1B => self.closed = true,
                0x09 => self.step_focus(m.shift),
                0x0D if self.focus.is_none() => {
                    self.result = Some(self.elems.clone());
                    self.closed = true;
                }
                0x0D => self.focus = None,
                0x2E if self.focus.is_none() => self.delete(),
                _ => {
                    if let Some(k) = self.focus {
                        let (changed, _) = self.fields[k].key(*vk, m.ctrl, m.shift, clip);
                        if changed {
                            self.commit(k);
                        }
                    }
                }
            },
            DlgEvent::Char(ch) => {
                if let Some(k) = self.focus {
                    if self.fields[k].char_input(*ch) {
                        self.commit(k);
                    }
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, c: &mut Ctx, r: Rect) {
        let s = c.s;
        self.hits.clear();
        frame(c, r, "GUI editor");
        let (lx, top) = (r[0] + 16.0 * s, r[1] + 54.0 * s);

        // ---- element list
        let list = [lx, top, 220.0 * s, 318.0 * s];
        c.r.rect(list[0], list[1], list[2], list[3], 8.0 * s, c_tree_bg(), 1.0, c_border_soft());
        self.hits.add(99, list);
        let rh = (ROW_H * s).round();
        let rows = ((list[3] - 8.0 * s) / rh) as usize;
        if self.elems.is_empty() {
            c.text_in(&c.f.ui, "No elements yet. Add one below.", list[0] + 12.0 * s, list[1] + 8.0 * s, rh, c_dim());
        }
        c.r.push_clip(list[0], list[1], list[2], list[3]);
        for (n, i) in (self.first_row..self.elems.len()).take(rows).enumerate() {
            let e = &self.elems[i];
            let row = [list[0] + 4.0 * s, list[1] + 4.0 * s + n as f32 * rh, list[2] - 8.0 * s, rh];
            self.hits.add(100 + i as u32, row);
            if self.sel == Some(i) {
                c.r.rect(row[0], row[1], row[2], row[3], 5.0 * s, c_selection(), 0.0, [0.0; 4]);
            } else if self.hits.hot(100 + i as u32) {
                c.r.rect(row[0], row[1], row[2], row[3], 5.0 * s, alpha([1.0; 4], 0.06), 0.0, [0.0; 4]);
            }
            c.text_center(&c.f.bold, kind_icon(e.kind), row[0] + 2.0 * s, row[1], 22.0 * s, row[3], c_accent_hot());
            c.text_in(&c.f.ui, &e.name, row[0] + 30.0 * s, row[1], row[3], if e.visible { c_text() } else { c_dim() });
        }
        c.r.pop_clip();
        // add / order / delete buttons
        let (bw, bh, gap) = (105.0 * s, 28.0 * s, 10.0 * s);
        let by = list[1] + list[3] + 10.0 * s;
        for (n, label) in ["+ Text", "+ Panel", "+ Bar", "+ Button"].iter().enumerate() {
            let rect = [lx + (n % 2) as f32 * (bw + gap), by + (n / 2) as f32 * (bh + 6.0 * s), bw, bh];
            dialog_button(c, &mut self.hits, 10 + n as u32, rect, label, false, true);
        }
        let on = self.sel.is_some();
        dialog_button(c, &mut self.hits, 15, [lx, by + 2.0 * (bh + 6.0 * s), bw, bh], "Send back", false, on);
        dialog_button(c, &mut self.hits, 16, [lx + bw + gap, by + 2.0 * (bh + 6.0 * s), bw, bh], "Bring front", false, on);
        dialog_button(c, &mut self.hits, 14, [lx, by + 3.0 * (bh + 6.0 * s), 2.0 * bw + gap, bh], "Delete", false, on);

        // ---- preview (16:9)
        let px = lx + 220.0 * s + 16.0 * s;
        let pw = 512.0 * s;
        self.preview = [px, top, pw, (pw * 9.0 / 16.0).round()];
        let pv = self.preview;
        c.r.rect(pv[0] - 1.0, pv[1] - 1.0, pv[2] + 2.0, pv[3] + 2.0, 4.0 * s, [0.0; 4], 1.0, c_border());
        c.r.push_clip(pv[0], pv[1], pv[2], pv[3]);
        c.r.gradient(pv[0], pv[1], pv[2], pv[3], [[0.2, 0.28, 0.4, 1.0], [0.2, 0.28, 0.4, 1.0], [0.08, 0.1, 0.14, 1.0], [0.08, 0.1, 0.14, 1.0]]);
        gui::draw(c, &self.elems, pv, None, false);
        if let Some(i) = self.sel.filter(|&i| self.elems.get(i).is_some_and(|e| e.visible)) {
            let e = self.elems[i].rect(pv);
            c.r.rect(e[0] - 1.0, e[1] - 1.0, e[2] + 2.0, e[3] + 2.0, 3.0 * s, [0.0; 4], 1.5, c_accent_hot());
        }
        c.r.pop_clip();
        self.hits.add(50, pv);
        c.text_in(&c.f.ui, "Preview of a 16:9 screen. Click an element to select it, drag to move it.", pv[0], pv[1] + pv[3] + 6.0 * s, 20.0 * s, c_dim());
        c.text_in(&c.f.ui, "x, y = distance from the anchor edge. Scripts: gui_text gui_value gui_show gui_move gui_color clicked cursor.", pv[0], pv[1] + pv[3] + 26.0 * s, 20.0 * s, c_dim());

        // ---- properties
        let rx = px + pw + 22.0 * s;
        let rw = r[0] + r[2] - 16.0 * s - rx;
        let mut y = top;
        let row_h = 34.0 * s;
        let has = self.sel.is_some();
        let (label_w, fh) = (74.0 * s, 28.0 * s);
        let mut field_at = |c: &mut Ctx, this: &mut GuiDialog, k: usize, label: &str, x: f32, y: f32, w: f32| {
            c.text_in(&c.f.ui, label, x, y, fh, if has && this.enabled(k) { c_muted() } else { c_dim() });
            let fr = [x + label_w.min(w * 0.45), y, w - label_w.min(w * 0.45), fh];
            this.hits.add(40 + k as u32, fr);
            let (focused, enabled, font) = (this.focus == Some(k), has && this.enabled(k), &c.f.ui);
            this.fields[k].draw(c, fr, font, focused, enabled, c_field());
        };
        field_at(c, self, F_NAME, "Name", rx, y, rw);
        y += row_h;
        field_at(c, self, F_TEXT, "Text", rx, y, rw);
        y += row_h;
        // anchor grid
        c.text_in(&c.f.ui, "Anchor", rx, y, 28.0 * s, c_muted());
        let (cw, chh) = (38.0 * s, 24.0 * s);
        for a in 0..9usize {
            let cell = [rx + label_w + (a % 3) as f32 * (cw + 4.0 * s), y + (a / 3) as f32 * (chh + 3.0 * s), cw, chh];
            self.hits.add(20 + a as u32, cell);
            let current = self.sel.and_then(|i| self.elems.get(i)).is_some_and(|e| e.anchor as usize == a);
            let hot = self.hits.hot(20 + a as u32);
            c.r.rect(cell[0], cell[1], cell[2], cell[3], 5.0 * s, if current { c_accent() } else if hot { c_btn_hot() } else { c_btn() }, 1.0, c_border_soft());
            if current {
                c.r.circle(cell[0] + cell[2] * 0.5, cell[1] + cell[3] * 0.5, 4.0 * s, [1.0; 4], 0.0, [0.0; 4]);
            }
        }
        c.text_in(&c.f.ui, gui::ANCHOR_NAMES[self.sel.and_then(|i| self.elems.get(i)).map_or(0, |e| e.anchor as usize % 9)], rx + label_w + 3.0 * (cw + 4.0 * s) + 8.0 * s, y + 24.0 * s, 24.0 * s, c_dim());
        y += 3.0 * (chh + 3.0 * s) + 8.0 * s;
        let half = (rw - 8.0 * s) * 0.5;
        field_at(c, self, F_X, "X", rx, y, half);
        field_at(c, self, F_Y, "Y", rx + half + 8.0 * s, y, half);
        y += row_h;
        field_at(c, self, F_W, "Width", rx, y, half + 20.0 * s);
        field_at(c, self, F_H, "Height", rx + half + 28.0 * s, y, half - 20.0 * s);
        y += row_h;
        field_at(c, self, F_VALUE, "Value", rx, y, half + 20.0 * s);
        field_at(c, self, F_ALPHA, "Opacity", rx + half + 28.0 * s, y, half - 20.0 * s);
        y += row_h;
        field_at(c, self, F_SIZE, "Text size", rx, y, rw);
        c.text_in(&c.f.ui, "pixel size 1 .. 4", rx + rw - 110.0 * s, y, fh, c_dim());
        y += row_h;
        field_at(c, self, F_COLOR, "Color", rx, y, rw);
        y += row_h;
        field_at(c, self, F_TEXT_COLOR, "Text color", rx, y, rw);
        y += row_h;
        // visible switch
        c.text_in(&c.f.ui, "Visible", rx, y, 24.0 * s, if has { c_muted() } else { c_dim() });
        let vis = self.sel.and_then(|i| self.elems.get(i)).is_some_and(|e| e.visible);
        let (sw, sh) = (40.0 * s, 22.0 * s);
        let sx = rx + label_w;
        self.hits.add(30, [sx, y, sw, sh]);
        c.r.rect(sx, y + 1.0, sw, sh, sh * 0.5, if vis && has { c_accent() } else { c_border() }, 0.0, [0.0; 4]);
        c.r.circle(if vis && has { sx + sw - sh * 0.5 } else { sx + sh * 0.5 }, y + 1.0 + sh * 0.5, sh * 0.5 - 3.0 * s, [1.0; 4], 0.0, [0.0; 4]);

        let (bw2, bh2) = (c.px(96.0), c.px(30.0));
        let by2 = r[1] + r[3] - 16.0 * s - bh2;
        dialog_button(c, &mut self.hits, 1, [r[0] + r[2] - 20.0 * s - bw2, by2, bw2, bh2], "Apply", true, true);
        dialog_button(c, &mut self.hits, 2, [r[0] + r[2] - 30.0 * s - bw2 * 2.0, by2, bw2, bh2], "Cancel", false, true);
        // how the mouse works while playing
        c.text_in(&c.f.ui, self.mode.describe(), r[0] + 16.0 * s, by2 - 26.0 * s, 22.0 * s, c_dim());
        c.text_in(&c.f.ui, "Mouse", r[0] + 16.0 * s, by2, bh2, c_muted());
        for (n, name) in Mode::NAMES.iter().enumerate() {
            let rect = [r[0] + 74.0 * s + n as f32 * (112.0 * s), by2, 106.0 * s, bh2];
            self.hits.add(60 + n as u32, rect);
            let (hot, down) = (self.hits.hot(60 + n as u32), self.hits.down(60 + n as u32));
            c.button(rect, name, hot, down, true, false, self.mode.index() == n);
        }
        let _ = inside;
    }

    fn done(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_selecting_ordering_and_deleting() {
        let mut d = GuiDialog::new(Vec::new(), Mode::default());
        assert_eq!(d.sel, None);
        d.add(Kind::Text);
        d.add(Kind::Bar);
        assert_eq!((d.elems.len(), d.sel), (2, Some(1)));
        assert_eq!(d.elems[1].kind, Kind::Bar);
        d.reorder(false);
        assert_eq!((d.elems[0].kind, d.sel), (Kind::Bar, Some(0)));
        d.reorder(false); // already at the back
        assert_eq!(d.sel, Some(0));
        d.delete();
        assert_eq!((d.elems.len(), d.sel), (1, Some(0)));
        d.add(Kind::Text);
        assert_eq!(d.elems[1].name, "Text.001");
    }

    #[test]
    fn typed_values_reach_the_element_and_bad_ones_are_ignored() {
        let mut d = GuiDialog::new(vec![GuiElem::new(Kind::Bar, "Health")], Mode::default());
        d.fields[F_X].edit.set_text("40");
        d.commit(F_X);
        d.fields[F_VALUE].edit.set_text("7");
        d.commit(F_VALUE);
        d.fields[F_COLOR].edit.set_text("#ff0000");
        d.commit(F_COLOR);
        d.fields[F_Y].edit.set_text("abc");
        d.commit(F_Y);
        let e = &d.elems[0];
        assert_eq!((e.x, e.value, e.color), (40.0, 1.0, [1.0, 0.0, 0.0]));
        assert_eq!(e.y, 24.0, "unreadable text leaves the old value");
        d.fields[F_NAME].edit.set_text("");
        d.commit(F_NAME);
        assert_eq!(d.elems[0].name, "Health", "names cannot be empty");
    }

    #[test]
    fn only_the_boxes_that_matter_are_enabled() {
        let d = GuiDialog::new(vec![GuiElem::new(Kind::Panel, "Back"), GuiElem::new(Kind::Bar, "Hp")], Mode::default());
        assert!(!d.enabled(F_TEXT) && !d.enabled(F_VALUE) && d.enabled(F_X));
        let mut d = d;
        d.select(1);
        assert!(d.enabled(F_VALUE) && d.enabled(F_TEXT));
    }
}
