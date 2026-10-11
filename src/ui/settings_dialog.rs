//! The settings dialog: a list of switches and number boxes. It knows nothing about what they
//! configure; the caller turns its settings into `ItemSpec`s and reads the values back.
use super::dialogs::{dialog_button, frame, Dialog, DlgEvent, Hits};
use super::style::*;
use super::widgets::{Clipboard, Field};
use super::Rect;

/// One line of the dialog.
#[derive(Clone)]
pub enum ItemSpec {
    Section(String),
    /// label, hint, value, default
    Text(String, String, String, String),
    /// label, hint, value, default
    Toggle(String, String, bool, bool),
    /// label, hint, value, default, min, max, whole numbers only
    Number(String, String, f32, f32, f32, f32, bool),
}

struct Row {
    spec: ItemSpec,
    on: bool,
    field: Field,
}

/// `result` holds one value per non-section item (switches as 0 / 1) after OK.
pub struct SettingsDialog {
    title: String,
    rows: Vec<Row>,
    pub result: Option<Vec<f32>>,
    closed: bool,
    hits: Hits,
    focus: Option<usize>,
    ok_label: String,
}

fn number_text(v: f32, whole: bool) -> String {
    if whole { format!("{}", v.round() as i64) } else { format!("{v}") }
}

impl SettingsDialog {
    pub fn new(title: &str, specs: Vec<ItemSpec>) -> SettingsDialog {
        let rows = specs
            .into_iter()
            .map(|spec| {
                let mut field = Field::new(false, false);
                let mut on = false;
                match &spec {
                    ItemSpec::Toggle(_, _, v, _) => on = *v,
                    ItemSpec::Text(_, _, v, _) => field.edit.set_text(v),
                    ItemSpec::Number(_, _, v, _, _, _, whole) => field.edit.set_text(&number_text(*v, *whole)),
                    ItemSpec::Section(_) => {}
                }
                Row { spec, on, field }
            })
            .collect();
        SettingsDialog { title: title.to_string(), rows, result: None, closed: false, hits: Hits::default(), focus: None, ok_label: "OK".into() }
    }

    /// The text on the confirming button.
    pub fn with_ok(mut self, label: &str) -> SettingsDialog {
        self.ok_label = label.to_string();
        self
    }

    /// The texts of the text boxes, in order.
    pub fn texts(&self) -> Vec<String> {
        self.rows.iter().filter(|r| matches!(r.spec, ItemSpec::Text(..))).map(|r| r.field.edit.text()).collect()
    }

    /// The values as the dialog currently reads them (numbers clamped, unreadable text = the default).
    pub fn values(&self) -> Vec<f32> {
        self.rows
            .iter()
            .filter_map(|r| match &r.spec {
                ItemSpec::Section(_) | ItemSpec::Text(..) => None,
                ItemSpec::Toggle(..) => Some(r.on as u8 as f32),
                ItemSpec::Number(_, _, _, def, min, max, whole) => {
                    let v = r.field.edit.text().trim().replace(',', ".").parse::<f32>().unwrap_or(*def).clamp(*min, *max);
                    Some(if *whole { v.round() } else { v })
                }
            })
            .collect()
    }

    fn reset(&mut self) {
        for r in &mut self.rows {
            match &r.spec {
                ItemSpec::Toggle(_, _, _, def) => r.on = *def,
                ItemSpec::Number(_, _, _, def, _, _, whole) => r.field.edit.set_text(&number_text(*def, *whole)),
                ItemSpec::Text(_, _, _, def) => r.field.edit.set_text(def),
                ItemSpec::Section(_) => {}
            }
        }
    }

    fn row_h(s: f32) -> f32 {
        46.0 * s
    }

    fn step_focus(&mut self, back: bool) {
        let idx: Vec<usize> = (0..self.rows.len()).filter(|&i| matches!(self.rows[i].spec, ItemSpec::Number(..) | ItemSpec::Text(..))).collect();
        if idx.is_empty() {
            return;
        }
        let pos = self.focus.and_then(|f| idx.iter().position(|&i| i == f));
        let next = match (pos, back) {
            (None, _) => idx[0],
            (Some(p), false) => idx[(p + 1) % idx.len()],
            (Some(p), true) => idx[(p + idx.len() - 1) % idx.len()],
        };
        self.focus = Some(next);
        self.rows[next].field.edit.select_all();
    }
}

impl Dialog for SettingsDialog {
    fn size(&self, c: &Ctx) -> (f32, f32) {
        let s = c.s;
        let body: f32 = self.rows.iter().map(|r| if matches!(r.spec, ItemSpec::Section(_)) { 34.0 * s } else { Self::row_h(s) }).sum();
        (c.px(540.0), 48.0 * s + body + 70.0 * s)
    }

    fn event(&mut self, ev: &DlgEvent, clip: &mut dyn Clipboard) {
        match ev {
            DlgEvent::Move(p) => self.hits.mouse = *p,
            DlgEvent::Down(0, p, _) => {
                self.hits.pressed = self.hits.at(*p);
                self.focus = None;
                if let Some(id) = self.hits.pressed {
                    if (1000..2000).contains(&id) {
                        let k = (id - 1000) as usize;
                        self.focus = Some(k);
                        self.rows[k].field.edit.select_all();
                    }
                }
            }
            DlgEvent::Up(0, p) => {
                if let (Some(id), Some(up)) = (self.hits.pressed.take(), self.hits.at(*p)) {
                    if id == up {
                        match id {
                            1 => {
                                self.result = Some(self.values());
                                self.closed = true;
                            }
                            2 => self.closed = true,
                            3 => self.reset(),
                            id if (2000..3000).contains(&id) => {
                                let k = (id - 2000) as usize;
                                self.rows[k].on = !self.rows[k].on;
                            }
                            _ => {}
                        }
                    }
                }
            }
            DlgEvent::Key(vk, m) => match *vk {
                0x1B => self.closed = true,
                0x0D => {
                    self.result = Some(self.values());
                    self.closed = true;
                }
                0x09 => self.step_focus(m.shift),
                _ => {
                    if let Some(k) = self.focus {
                        self.rows[k].field.key(*vk, m.ctrl, m.shift, clip);
                    }
                }
            },
            DlgEvent::Char(ch) => {
                if let Some(k) = self.focus {
                    // number boxes take digits only
                    let numeric = matches!(self.rows[k].spec, ItemSpec::Number(..));
                    if !numeric || ch.is_ascii_digit() || matches!(ch, '.' | ',' | '-') || ch.is_control() {
                        self.rows[k].field.char_input(*ch);
                    }
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, c: &mut Ctx, r: Rect) {
        let s = c.s;
        self.hits.clear();
        frame(c, r, &self.title);
        let mut y = r[1] + 48.0 * s;
        let (x0, x1) = (r[0] + 24.0 * s, r[0] + r[2] - 24.0 * s);
        for k in 0..self.rows.len() {
            let on = self.rows[k].on;
            match self.rows[k].spec.clone() {
                ItemSpec::Section(t) => {
                    c.text_in(&c.f.bold, &t.to_uppercase(), x0, y + 6.0 * s, 24.0 * s, c_section());
                    c.r.fill(x0, y + 30.0 * s, x1 - x0, 1.0, c_border_soft());
                    y += 34.0 * s;
                }
                ItemSpec::Toggle(label, hint, ..) => {
                    let row = [x0 - 8.0 * s, y, x1 - x0 + 16.0 * s, Self::row_h(s)];
                    self.hits.add(2000 + k as u32, row);
                    if self.hits.hot(2000 + k as u32) {
                        c.r.rect(row[0], row[1], row[2], row[3], 8.0 * s, c_btn(), 0.0, [0.0; 4]);
                    }
                    c.text_in(&c.f.ui, &label, x0, y + 4.0 * s, 20.0 * s, c_text());
                    c.text_in(&c.f.ui, &hint, x0, y + 23.0 * s, 18.0 * s, c_dim());
                    let (sw, sh) = (40.0 * s, 22.0 * s);
                    let sx = x1 - sw;
                    let sy = y + (Self::row_h(s) - sh) * 0.5;
                    c.r.rect(sx, sy, sw, sh, sh * 0.5, if on { c_accent() } else { c_border() }, 0.0, [0.0; 4]);
                    let kx = if on { sx + sw - sh * 0.5 } else { sx + sh * 0.5 };
                    c.r.circle(kx, sy + sh * 0.5, sh * 0.5 - 3.0 * s, [1.0, 1.0, 1.0, 1.0], 0.0, [0.0; 4]);
                    y += Self::row_h(s);
                }
                ItemSpec::Number(label, hint, ..) | ItemSpec::Text(label, hint, ..) => {
                    let wide = matches!(self.rows[k].spec, ItemSpec::Text(..));
                    c.text_in(&c.f.ui, &label, x0, y + 4.0 * s, 20.0 * s, c_text());
                    c.text_in(&c.f.ui, &hint, x0, y + 23.0 * s, 18.0 * s, c_dim());
                    let fw = if wide { 200.0 * s } else { 96.0 * s };
                    let fr = [x1 - fw, y + (Self::row_h(s) - 28.0 * s) * 0.5, fw, 28.0 * s];
                    self.hits.add(1000 + k as u32, fr);
                    let focused = self.focus == Some(k);
                    let font = &c.f.ui;
                    self.rows[k].field.draw(c, fr, font, focused, true, c_field());
                    y += Self::row_h(s);
                }
            }
        }
        let (bw, bh) = (c.px(96.0), c.px(30.0));
        let by = r[1] + r[3] - 16.0 * s - bh;
        dialog_button(c, &mut self.hits, 1, [r[0] + r[2] - 20.0 * s - bw, by, bw, bh], &self.ok_label.clone(), true, true);
        dialog_button(c, &mut self.hits, 2, [r[0] + r[2] - 30.0 * s - bw * 2.0, by, bw, bh], "Cancel", false, true);
        dialog_button(c, &mut self.hits, 3, [r[0] + 20.0 * s, by, c.px(140.0), bh], "Reset to defaults", false, true);
    }

    fn done(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ItemSpec> {
        vec![
            ItemSpec::Section("Display".into()),
            ItemSpec::Toggle("VSync".into(), "".into(), true, false),
            ItemSpec::Number("FPS limit".into(), "".into(), 144.0, 0.0, 0.0, 1000.0, true),
        ]
    }

    #[test]
    fn values_follow_the_widgets_and_clamp() {
        let mut d = SettingsDialog::new("Settings", specs());
        assert_eq!(d.values(), vec![1.0, 144.0]);
        d.rows[2].field.edit.set_text("5000");
        assert_eq!(d.values()[1], 1000.0);
        d.rows[2].field.edit.set_text("abc");
        assert_eq!(d.values()[1], 0.0); // unreadable text falls back to the default
    }

    #[test]
    fn reset_restores_the_defaults() {
        let mut d = SettingsDialog::new("Settings", specs());
        d.reset();
        assert_eq!(d.values(), vec![0.0, 0.0]);
    }
}
