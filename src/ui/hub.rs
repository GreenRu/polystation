//! The start page ("hub"): recent projects, new projects from templates, open buttons. It is a full-window
//! dialog shown when the program starts without a project, and again from File > New Project.
use super::dialogs::{Dialog, DlgEvent};
use super::render::{alpha, mix, Color};
use super::style::*;
use super::widgets::{Clipboard, Field};
use super::{inside, Rect};
use crate::templates::{Template, TEMPLATES};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum HubChoice {
    /// Open a project file (or model) by path.
    Open(String),
    New { template: String, name: String, dir: String },
    /// Show the file browser for a project.
    BrowseProject,
    /// Show the file browser for a model.
    BrowseModel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Projects,
    New,
}

pub struct RecentItem {
    pub name: String,
    pub path: String,
    pub when: String,
}

pub struct Hub {
    pub page: Page,
    pub recents: Vec<RecentItem>,
    template: usize,
    name: Field,
    dir: Field,
    focus: Option<u8>,
    pub error: String,
    pub result: Option<HubChoice>,
    closed: bool,
    mouse: (f32, f32),
    pressed: Option<u32>,
    /// Click targets of the last frame: (id, rectangle).
    areas: Vec<(u32, Rect)>,
    scroll: f32,
    version: String,
    /// When the hub was opened from the editor (File > New Project) it can be cancelled.
    pub cancellable: bool,
}

fn default_dir() -> String {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let docs = home.join("Documents");
    let base = if docs.is_dir() { docs } else { home };
    base.join("Polystation Projects").to_string_lossy().into_owned()
}

impl Hub {
    pub fn new(page: Page, recents: Vec<RecentItem>, cancellable: bool) -> Hub {
        let mut name = Field::new(false, false);
        name.edit.set_text("My Game");
        let mut dir = Field::new(false, false);
        dir.edit.set_text(&default_dir());
        Hub { page: if recents.is_empty() && page == Page::Projects { Page::New } else { page }, recents, template: 0, name, dir, focus: None, error: String::new(), result: None, closed: false, mouse: (0.0, 0.0), pressed: None, areas: Vec::new(), scroll: 0.0, version: env!("CARGO_PKG_VERSION").to_string(), cancellable }
    }

    pub fn template(&self) -> &'static Template {
        &TEMPLATES[self.template.min(TEMPLATES.len() - 1)]
    }

    fn at(&self, p: (f32, f32)) -> Option<u32> {
        self.areas.iter().rev().find(|(_, r)| inside(p, *r)).map(|(i, _)| *i)
    }

    fn hot(&self, id: u32) -> bool {
        self.at(self.mouse) == Some(id)
    }

    fn create(&mut self) {
        let (name, dir) = (self.name.edit.text(), self.dir.edit.text());
        if crate::templates::clean_name(&name).is_empty() {
            self.error = "Give the project a name.".into();
            return;
        }
        if dir.trim().is_empty() {
            self.error = "Choose a folder for the project.".into();
            return;
        }
        self.error.clear();
        self.result = Some(HubChoice::New { template: self.template().id.to_string(), name, dir: dir.trim().to_string() });
        self.closed = true;
    }

    fn click(&mut self, id: u32) {
        match id {
            1 => self.page = Page::Projects,
            2 => self.page = Page::New,
            3 => {
                self.result = Some(HubChoice::BrowseProject);
                self.closed = true;
            }
            4 => {
                self.result = Some(HubChoice::BrowseModel);
                self.closed = true;
            }
            5 => self.create(),
            6 => self.closed = true, // cancel
            100..=199 => self.template = (id - 100) as usize,
            200..=299 => {
                if let Some(r) = self.recents.get((id - 200) as usize) {
                    self.result = Some(HubChoice::Open(r.path.clone()));
                    self.closed = true;
                }
            }
            _ => {}
        }
    }
}

fn pixel(c: &mut Ctx, text: &str, x: f32, y: f32, k: f32, color: Color) -> f32 {
    let f = c.f;
    c.r.text_scaled(&f.pixel, text, x + k, y + k, [0.0, 0.0, 0.0, 0.55 * color[3]], k);
    c.r.text_scaled(&f.pixel, text, x, y, color, k)
}

/// The little scene on a template card.
fn preview(c: &mut Ctx, t: &Template, r: Rect) {
    let s = c.s;
    let [sky, ground, accent] = t.colors;
    let rgb = |v: [f32; 3], a: f32| -> Color { [v[0], v[1], v[2], a] };
    let horizon = r[1] + r[3] * 0.58;
    let (sky_hi, gnd_lo) = (mix(rgb(sky, 1.0), [1.0; 4], 0.25), mix(rgb(ground, 1.0), [0.0, 0.0, 0.0, 1.0], 0.45));
    c.r.gradient(r[0], r[1], r[2], horizon - r[1], [rgb(sky, 1.0), rgb(sky, 1.0), sky_hi, sky_hi]);
    c.r.gradient(r[0], horizon, r[2], r[1] + r[3] - horizon, [rgb(ground, 1.0), rgb(ground, 1.0), gnd_lo, gnd_lo]);
    let b = (6.0 * s).round();
    let block = |c: &mut Ctx, x: f32, y: f32, w: f32, h: f32, col: [f32; 3]| {
        c.r.fill(x.round(), y.round(), w.round(), h.round(), rgb(col, 1.0));
        c.r.fill(x.round(), y.round(), w.round(), (h * 0.22).round().max(1.0), mix(rgb(col, 1.0), [1.0; 4], 0.35));
    };
    match t.id {
        "wetlands" => {
            for (i, x) in [0.12, 0.3, 0.72, 0.86].iter().enumerate() {
                let (tx, h) = (r[0] + r[2] * x, r[3] * (0.3 + 0.06 * (i % 2) as f32));
                block(c, tx - b * 0.4, horizon - h * 0.2, b * 0.8, h * 0.4, [0.35, 0.22, 0.12]);
                for k in 0..3 {
                    let w = b * (3.0 - k as f32 * 0.8);
                    block(c, tx - w * 0.5, horizon - h * 0.2 - (k as f32 + 1.0) * h * 0.26, w, h * 0.26, [0.15, 0.4, 0.2]);
                }
            }
            block(c, r[0] + r[2] * 0.4, horizon + r[3] * 0.1, r[2] * 0.28, r[3] * 0.1, accent);
        }
        "coins" => {
            for (x, y) in [(0.25, 0.7), (0.5, 0.82), (0.74, 0.68)] {
                let (cx, cy) = (r[0] + r[2] * x, r[1] + r[3] * y);
                block(c, cx - b, cy - b, b * 2.0, b * 2.0, accent);
            }
            block(c, r[0] + r[2] * 0.1, horizon - r[3] * 0.2, b * 1.6, r[3] * 0.4, [0.5, 0.35, 0.65]);
            block(c, r[0] + r[2] * 0.8, horizon - r[3] * 0.2, b * 1.6, r[3] * 0.4, [0.5, 0.35, 0.65]);
        }
        "hall" => {
            block(c, r[0], r[1], r[2], r[3] * 0.16, [0.2, 0.17, 0.16]);
            for x in [0.2, 0.5, 0.8] {
                block(c, r[0] + r[2] * x - b * 0.7, r[1] + r[3] * 0.16, b * 1.4, horizon - r[1] - r[3] * 0.16, [0.5, 0.45, 0.4]);
            }
            for x in [0.08, 0.92] {
                let (tx, ty) = (r[0] + r[2] * x, r[1] + r[3] * 0.38);
                c.r.circle(tx, ty, b * 2.6, alpha(rgb(accent, 1.0), 0.18), 0.0, [0.0; 4]);
                block(c, tx - b * 0.4, ty - b * 0.4, b * 0.8, b * 1.3, accent);
            }
        }
        _ => {
            block(c, r[0] + r[2] * 0.62, horizon - r[3] * 0.1, b * 3.0, b * 3.0, [0.75, 0.5, 0.3]);
            c.r.circle(r[0] + r[2] * 0.2, r[1] + r[3] * 0.2, b * 1.6, rgb(accent, 0.9), 0.0, [0.0; 4]);
        }
    }
}

impl Dialog for Hub {
    fn size(&self, c: &Ctx) -> (f32, f32) {
        c.r.screen()
    }

    fn event(&mut self, ev: &DlgEvent, clip: &mut dyn Clipboard) {
        match ev {
            DlgEvent::Move(p) => self.mouse = *p,
            DlgEvent::Wheel(n, _) => self.scroll = (self.scroll - n * 40.0).max(0.0),
            DlgEvent::Down(0, p, _) => {
                self.pressed = self.at(*p);
                self.focus = match self.pressed {
                    Some(10) => Some(0),
                    Some(11) => Some(1),
                    _ => None,
                };
                if let Some(k) = self.focus {
                    let f = if k == 0 { &mut self.name } else { &mut self.dir };
                    f.edit.select_all();
                }
            }
            DlgEvent::Up(0, p) => {
                if let (Some(id), Some(up)) = (self.pressed.take(), self.at(*p)) {
                    if id == up {
                        self.click(id);
                    }
                }
            }
            DlgEvent::Key(vk, m) => match *vk {
                0x1B if self.cancellable => self.closed = true,
                0x0D if self.page == Page::New => self.create(),
                0x09 if self.focus.is_some() => {
                    self.focus = Some(1 - self.focus.unwrap_or(0));
                    let f = if self.focus == Some(0) { &mut self.name } else { &mut self.dir };
                    f.edit.select_all();
                }
                _ => {
                    if let Some(k) = self.focus {
                        let f = if k == 0 { &mut self.name } else { &mut self.dir };
                        f.key(*vk, m.ctrl, m.shift, clip);
                        self.error.clear();
                    }
                }
            },
            DlgEvent::Char(ch) => {
                if let Some(k) = self.focus {
                    let f = if k == 0 { &mut self.name } else { &mut self.dir };
                    f.char_input(*ch);
                    self.error.clear();
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, c: &mut Ctx, r: Rect) {
        let s = c.s;
        self.areas.clear();
        c.r.fill(r[0], r[1], r[2], r[3], c_bg());
        // ---- sidebar
        let sw = (250.0 * s).round();
        c.r.fill(r[0], r[1], sw, r[3], c_bar());
        c.r.fill(r[0] + sw - 1.0, r[1], 1.0, r[3], c_border_soft());
        let k = (3.0 * s).round().max(2.0);
        let lk = (1.5 * s).round().max(1.0); // the logo: whole pixels per logo pixel
        let logo = super::logo::SIZE as f32 * lk;
        super::logo::draw(c, r[0] + 28.0 * s, r[1] + 26.0 * s, lk);
        let ty = r[1] + 26.0 * s + logo + 16.0 * s;
        pixel(c, "POLY", r[0] + 28.0 * s, ty, k, [0.95, 0.95, 1.0, 1.0]);
        pixel(c, "STATION", r[0] + 28.0 * s, ty + c.f.pixel.line_h * k + 2.0 * s, k, [0.4, 0.62, 1.0, 1.0]);
        c.text_in(&c.f.ui, "a PS1-style 3D engine", r[0] + 28.0 * s, ty + c.f.pixel.line_h * k * 2.0 + 10.0 * s, 20.0 * s, c_dim());
        let mut y = ty + c.f.pixel.line_h * k * 2.0 + 70.0 * s;
        for (id, label, page) in [(1u32, "Projects", Page::Projects), (2, "New Project", Page::New)] {
            let row = [r[0] + 14.0 * s, y, sw - 28.0 * s, 40.0 * s];
            self.areas.push((id, row));
            let active = self.page == page;
            if active {
                c.r.rect(row[0], row[1], row[2], row[3], 8.0 * s, c_selection(), 0.0, [0.0; 4]);
                c.r.fill(row[0], row[1] + 8.0 * s, 3.0 * s, row[3] - 16.0 * s, c_accent_hot());
            } else if self.hot(id) {
                c.r.rect(row[0], row[1], row[2], row[3], 8.0 * s, alpha([1.0; 4], 0.06), 0.0, [0.0; 4]);
            }
            c.text_in(&c.f.bold, label, row[0] + 20.0 * s, row[1], row[3], if active { [1.0; 4] } else { c_muted() });
            y += 46.0 * s;
        }
        c.text_in(&c.f.ui, &format!("version {}", self.version), r[0] + 28.0 * s, r[1] + r[3] - 40.0 * s, 24.0 * s, c_dim());
        if self.cancellable {
            let b = [r[0] + 14.0 * s, r[1] + r[3] - 90.0 * s, sw - 28.0 * s, 36.0 * s];
            self.areas.push((6, b));
            c.button(b, "Back to editor", self.hot(6), self.pressed == Some(6) && self.hot(6), true, false, false);
        }

        // ---- content
        let cx = r[0] + sw + 44.0 * s;
        let cw = r[0] + r[2] - cx - 44.0 * s;
        match self.page {
            Page::Projects => self.draw_projects(c, cx, r[1] + 40.0 * s, cw, r[3]),
            Page::New => self.draw_new(c, cx, r[1] + 40.0 * s, cw, r[3]),
        }
    }

    fn done(&self) -> bool {
        self.closed
    }
}

impl Hub {
    fn draw_projects(&mut self, c: &mut Ctx, x: f32, y: f32, w: f32, h: f32) {
        let s = c.s;
        let k = (2.0 * s).round().max(2.0);
        pixel(c, "PROJECTS", x, y, k, [1.0; 4]);
        let by = y + 4.0 * s;
        let (bw, bh) = (150.0 * s, 36.0 * s);
        let open = [x + w - bw * 2.0 - 12.0 * s, by, bw, bh];
        let model = [x + w - bw, by, bw, bh];
        self.areas.push((3, open));
        self.areas.push((4, model));
        c.button(open, "Open project...", self.hot(3), self.pressed == Some(3) && self.hot(3), true, false, false);
        c.button(model, "Open a model...", self.hot(4), self.pressed == Some(4) && self.hot(4), true, false, false);
        let top = y + 24.0 * s + c.f.pixel.line_h * k;
        c.r.fill(x, top, w, 1.0, c_border_soft());
        let rh = (64.0 * s).round();
        let view = [x, top + 8.0 * s, w, h - top - 20.0 * s];
        c.r.push_clip(view[0], view[1], view[2], view[3]);
        let max_scroll = (self.recents.len() as f32 * (rh + 8.0 * s) - view[3]).max(0.0);
        self.scroll = self.scroll.min(max_scroll);
        for (i, item) in self.recents.iter().enumerate() {
            let row = [x, view[1] + i as f32 * (rh + 8.0 * s) - self.scroll, w, rh];
            if row[1] + rh < view[1] || row[1] > view[1] + view[3] {
                continue;
            }
            self.areas.push((200 + i as u32, row));
            let hot = self.hot(200 + i as u32);
            c.r.rect(row[0], row[1], row[2], row[3], 10.0 * s, if hot { c_btn_hot() } else { c_btn() }, 1.0, if hot { c_accent() } else { c_border_soft() });
            let icon = [row[0] + 14.0 * s, row[1] + 12.0 * s, 40.0 * s, 40.0 * s];
            c.r.rect(icon[0], icon[1], icon[2], icon[3], 8.0 * s, c_accent_down(), 0.0, [0.0; 4]);
            c.text_center(&c.f.bold, &item.name.chars().next().map_or("?".to_string(), |ch| ch.to_uppercase().to_string()), icon[0], icon[1], icon[2], icon[3], [1.0; 4]);
            c.text_in(&c.f.bold, &item.name, row[0] + 70.0 * s, row[1] + 8.0 * s, 26.0 * s, c_text());
            let path = c.f.ui.fit(&item.path, row[2] - 250.0 * s);
            c.text_in(&c.f.ui, &path, row[0] + 70.0 * s, row[1] + 32.0 * s, 24.0 * s, c_dim());
            let wt = c.f.ui.measure(&item.when);
            c.text_in(&c.f.ui, &item.when, row[0] + row[2] - wt - 18.0 * s, row[1], row[3], c_muted());
        }
        c.r.pop_clip();
    }

    fn draw_new(&mut self, c: &mut Ctx, x: f32, y: f32, w: f32, h: f32) {
        let s = c.s;
        let k = (2.0 * s).round().max(2.0);
        pixel(c, "NEW PROJECT", x, y, k, [1.0; 4]);
        let top = y + 24.0 * s + c.f.pixel.line_h * k;
        c.r.fill(x, top, w, 1.0, c_border_soft());
        c.text_in(&c.f.ui, "Pick a starting scene", x, top + 14.0 * s, 24.0 * s, c_muted());
        // template cards
        let gap = 18.0 * s;
        let cols = if w > 980.0 * s { 4 } else { 2 };
        let cw = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let ch = (cw * 0.52).round() + 76.0 * s;
        let cards_top = top + 46.0 * s;
        for (i, t) in TEMPLATES.iter().enumerate() {
            let card = [x + (i % cols) as f32 * (cw + gap), cards_top + (i / cols) as f32 * (ch + gap), cw, ch];
            self.areas.push((100 + i as u32, card));
            let sel = self.template == i;
            let hot = self.hot(100 + i as u32);
            c.r.shadow(card[0], card[1] + 4.0 * s, card[2], card[3], 10.0 * s, 14.0 * s, [0.0, 0.0, 0.0, if sel { 0.5 } else { 0.3 }]);
            c.r.rect(card[0], card[1], card[2], card[3], 10.0 * s, if hot && !sel { c_btn_hot() } else { c_btn() }, if sel { 2.0 } else { 1.0 }, if sel { c_accent_hot() } else { c_border_soft() });
            let pv = [card[0] + 8.0 * s, card[1] + 8.0 * s, card[2] - 16.0 * s, (card[2] * 0.52 - 16.0 * s).round()];
            c.r.push_clip(pv[0], pv[1], pv[2], pv[3]);
            preview(c, t, pv);
            c.r.pop_clip();
            c.r.rect(pv[0], pv[1], pv[2], pv[3], 6.0 * s, [0.0; 4], 1.0, alpha([1.0; 4], 0.1));
            c.text_in(&c.f.bold, t.name, card[0] + 14.0 * s, pv[1] + pv[3] + 4.0 * s, 26.0 * s, c_text());
            let lines = c.f.ui.wrap(t.blurb, card[2] - 28.0 * s);
            for (n, l) in lines.iter().take(2).enumerate() {
                c.text_in(&c.f.ui, l, card[0] + 14.0 * s, pv[1] + pv[3] + 28.0 * s + n as f32 * 18.0 * s, 18.0 * s, c_dim());
            }
            if sel {
                let tag = [card[0] + card[2] - 30.0 * s, card[1] + 14.0 * s, 20.0 * s, 20.0 * s];
                c.r.circle(tag[0] + 10.0 * s, tag[1] + 10.0 * s, 10.0 * s, c_accent(), 0.0, [0.0; 4]);
                c.check_mark(tag[0] + 4.0 * s, tag[1] + 4.0 * s, 12.0 * s, [1.0; 4]);
            }
        }
        let rows = TEMPLATES.len().div_ceil(cols) as f32;
        let form_y = cards_top + rows * (ch + gap) + 6.0 * s;
        // the form
        c.text_in(&c.f.ui, "Project name", x, form_y, 24.0 * s, c_muted());
        let nf = [x, form_y + 26.0 * s, (w * 0.34).round().min(380.0 * s), 34.0 * s];
        self.areas.push((10, nf));
        let (f0, f1) = (self.focus == Some(0), self.focus == Some(1));
        self.name.draw(c, nf, &c.f.ui, f0, true, c_field());
        let lx = nf[0] + nf[2] + 24.0 * s;
        c.text_in(&c.f.ui, "Location", lx, form_y, 24.0 * s, c_muted());
        let bw = 190.0 * s;
        let lf = [lx, form_y + 26.0 * s, (x + w - lx - bw - 16.0 * s).max(160.0 * s), 34.0 * s];
        self.areas.push((11, lf));
        self.dir.draw(c, lf, &c.f.ui, f1, true, c_field());
        let create = [x + w - bw, form_y + 26.0 * s, bw, 34.0 * s];
        self.areas.push((5, create));
        c.button(create, "Create project", self.hot(5), self.pressed == Some(5) && self.hot(5), true, true, false);
        let name = crate::templates::clean_name(&self.name.edit.text());
        let path = format!("{}\\{}\\{}.{}", self.dir.edit.text().trim_end_matches(['\\', '/']), name, name, crate::project::EXT);
        let line = if self.error.is_empty() { c.f.ui.fit(&path, w) } else { self.error.clone() };
        c.text_in(&c.f.ui, &line, x, form_y + 70.0 * s, 24.0 * s, if self.error.is_empty() { c_dim() } else { c_error() });
        let _ = h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoClip;
    impl Clipboard for NoClip {
        fn get(&mut self) -> String {
            String::new()
        }
        fn set(&mut self, _: &str) {}
    }

    #[test]
    fn with_no_recent_projects_it_opens_on_the_new_project_page() {
        assert_eq!(Hub::new(Page::Projects, Vec::new(), false).page, Page::New);
        let r = vec![RecentItem { name: "A".into(), path: "C:\\a.polystation".into(), when: "today".into() }];
        assert_eq!(Hub::new(Page::Projects, r, false).page, Page::Projects);
    }

    #[test]
    fn creating_needs_a_name_and_returns_the_choice() {
        let mut h = Hub::new(Page::New, Vec::new(), false);
        h.name.edit.set_text("   ");
        h.create();
        assert!(h.result.is_none() && !h.error.is_empty());
        h.name.edit.set_text("Swamp");
        h.click(101);
        h.create();
        assert!(matches!(&h.result, Some(HubChoice::New { template, name, .. }) if template == "wetlands" && name == "Swamp"));
        assert!(h.done());
    }

    #[test]
    fn recent_rows_and_browse_buttons_answer_with_choices() {
        let r = vec![RecentItem { name: "A".into(), path: "C:\\a.polystation".into(), when: String::new() }];
        let mut h = Hub::new(Page::Projects, r, false);
        h.click(200);
        assert_eq!(h.result, Some(HubChoice::Open("C:\\a.polystation".into())));
        let mut h = Hub::new(Page::Projects, Vec::new(), false);
        h.click(3);
        assert_eq!(h.result, Some(HubChoice::BrowseProject));
        let _ = NoClip;
    }
}
