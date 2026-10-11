//! Modal dialogs drawn by the UI itself: a message box, a color picker and a file browser. Each one is
//! a small state machine that gets mouse and key events and draws itself; the window runs it in a
//! loop over a dimmed copy of the last frame until it reports it is done.
use super::list::{self, List};

use super::style::*;
use super::widgets::{Clipboard, Field};
use super::{inside, Mods, Rect};
use std::path::{Path, PathBuf};

pub enum DlgEvent {
    Move((f32, f32)),
    Down(u8, (f32, f32), bool),
    Up(u8, (f32, f32)),
    Wheel(f32, (f32, f32)),
    Key(u32, Mods),
    Char(char),
}

pub trait Dialog {
    /// Desired size in pixels.
    fn size(&self, c: &Ctx) -> (f32, f32);
    fn event(&mut self, ev: &DlgEvent, clip: &mut dyn Clipboard);
    fn draw(&mut self, c: &mut Ctx, rect: Rect);
    fn done(&self) -> bool;
}

/// Clickable areas registered while drawing: (id, rectangle), later ones on top.
#[derive(Default)]
pub(super) struct Hits {
    areas: Vec<(u32, Rect)>,
    pub(super) mouse: (f32, f32),
    pub(super) pressed: Option<u32>,
}

impl Hits {
    pub(super) fn clear(&mut self) {
        self.areas.clear();
    }

    pub(super) fn add(&mut self, id: u32, r: Rect) {
        self.areas.push((id, r));
    }

    pub(super) fn at(&self, p: (f32, f32)) -> Option<u32> {
        self.areas.iter().rev().find(|(_, r)| inside(p, *r)).map(|(i, _)| *i)
    }

    pub(super) fn hot(&self, id: u32) -> bool {
        self.at(self.mouse) == Some(id)
    }

    pub(super) fn down(&self, id: u32) -> bool {
        self.pressed == Some(id) && self.hot(id)
    }
}

pub(super) fn frame(c: &mut Ctx, r: Rect, title: &str) {
    let s = c.s;
    c.r.shadow(r[0], r[1] + 8.0 * s, r[2], r[3], 12.0 * s, 28.0 * s, [0.0, 0.0, 0.0, 0.6]);
    c.r.rect(r[0], r[1], r[2], r[3], 12.0 * s, c_menu_bg(), 1.0, c_border());
    c.text_in(&c.f.bold, title, r[0] + 20.0 * s, r[1], 40.0 * s, c_text());
    c.r.fill(r[0] + 1.0, r[1] + 40.0 * s, r[2] - 2.0, 1.0, c_border_soft());
}

pub(super) fn dialog_button(c: &mut Ctx, hits: &mut Hits, id: u32, r: Rect, label: &str, primary: bool, enabled: bool) {
    hits.add(id, r);
    let (hot, down) = (hits.hot(id), hits.down(id));
    c.button(r, label, hot, down, enabled, primary, false);
}

// ------------------------------------------------------------------------------------------
// Message box
// ------------------------------------------------------------------------------------------
pub struct MessageDialog {
    title: String,
    text: String,
    buttons: Vec<String>,
    /// Index of the button chosen (None while open).
    pub result: Option<usize>,
    hits: Hits,
}

impl MessageDialog {
    pub fn new(title: &str, text: &str, buttons: &[&str]) -> MessageDialog {
        MessageDialog { title: title.to_string(), text: text.to_string(), buttons: buttons.iter().map(|b| b.to_string()).collect(), result: None, hits: Hits::default() }
    }
}

impl Dialog for MessageDialog {
    fn size(&self, c: &Ctx) -> (f32, f32) {
        let w = c.px(440.0);
        let lines = c.f.ui.wrap(&self.text, w - c.px(40.0)).len().min(18) as f32;
        (w, c.px(40.0) + c.px(24.0) + lines * (c.f.ui.line_h + 3.0) + c.px(24.0) + c.px(52.0))
    }

    fn event(&mut self, ev: &DlgEvent, _: &mut dyn Clipboard) {
        match ev {
            DlgEvent::Move(p) => self.hits.mouse = *p,
            DlgEvent::Down(0, p, _) => self.hits.pressed = self.hits.at(*p),
            DlgEvent::Up(0, p) => {
                if let (Some(id), Some(up)) = (self.hits.pressed.take(), self.hits.at(*p)) {
                    if id == up {
                        self.result = Some(id as usize);
                    }
                }
            }
            DlgEvent::Key(0x0D, _) => self.result = Some(0),
            DlgEvent::Key(0x1B, _) => self.result = Some(self.buttons.len().saturating_sub(1)),
            _ => {}
        }
    }

    fn draw(&mut self, c: &mut Ctx, r: Rect) {
        let s = c.s;
        self.hits.clear();
        frame(c, r, &self.title);
        let font = &c.f.ui;
        let lh = font.line_h + 3.0;
        for (i, l) in font.wrap(&self.text, r[2] - 40.0 * s).iter().take(18).enumerate() {
            c.r.text(font, l, r[0] + 20.0 * s, r[1] + 56.0 * s + i as f32 * lh, c_text());
        }
        let (bw, bh) = (c.px(96.0), c.px(30.0));
        let mut x = r[0] + r[2] - 20.0 * s - bw;
        for (i, b) in self.buttons.iter().enumerate().rev() {
            dialog_button(c, &mut self.hits, i as u32, [x, r[1] + r[3] - 16.0 * s - bh, bw, bh], b, i == 0, true);
            x -= bw + 10.0 * s;
        }
    }

    fn done(&self) -> bool {
        self.result.is_some()
    }
}

// ------------------------------------------------------------------------------------------
// Color picker
// ------------------------------------------------------------------------------------------
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(360.0)) / 60.0;
    let (i, f) = (h.floor() as i32, h - h.floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

pub fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    let h = if d <= 1e-6 {
        0.0
    } else if max == c[0] {
        60.0 * (((c[1] - c[2]) / d).rem_euclid(6.0))
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    (h, if max <= 1e-6 { 0.0 } else { d / max }, max)
}

pub fn hex_of(c: [f32; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8)
}

pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(t, 16).ok()?;
    Some([((v >> 16) & 0xFF) as f32 / 255.0, ((v >> 8) & 0xFF) as f32 / 255.0, (v & 0xFF) as f32 / 255.0])
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Square,
    Hue,
}

pub struct ColorDialog {
    old: [f32; 3],
    hsv: (f32, f32, f32),
    /// The picked color when OK was pressed (None while open or cancelled).
    pub result: Option<[f32; 3]>,
    closed: bool,
    hits: Hits,
    drag: Option<Drag>,
    /// R, G, B (0-255) and the hex text.
    fields: Vec<Field>,
    focus: Option<usize>,
    square: Rect,
    hue: Rect,
}

impl ColorDialog {
    pub fn new(init: [f32; 3]) -> ColorDialog {
        let mut d = ColorDialog { old: init, hsv: rgb_to_hsv(init), result: None, closed: false, hits: Hits::default(), drag: None, fields: (0..4).map(|_| Field::new(false, false)).collect(), focus: None, square: [0.0; 4], hue: [0.0; 4] };
        d.sync_fields();
        d
    }

    fn rgb(&self) -> [f32; 3] {
        hsv_to_rgb(self.hsv.0, self.hsv.1, self.hsv.2)
    }

    fn sync_fields(&mut self) {
        let c = self.rgb();
        for k in 0..3 {
            if self.focus != Some(k) {
                self.fields[k].edit.set_text(&((c[k] * 255.0 + 0.5) as u8).to_string());
            }
        }
        if self.focus != Some(3) {
            self.fields[3].edit.set_text(&hex_of(c));
        }
    }

    fn apply_fields(&mut self, k: usize) {
        let t = self.fields[k].edit.text();
        if k < 3 {
            if let Ok(v) = t.trim().parse::<f32>() {
                let mut c = self.rgb();
                c[k] = (v / 255.0).clamp(0.0, 1.0);
                let (h, s, v2) = rgb_to_hsv(c);
                self.hsv = (if s <= 1e-4 { self.hsv.0 } else { h }, s, v2);
            }
        } else if let Some(c) = parse_hex(&t) {
            let (h, s, v) = rgb_to_hsv(c);
            self.hsv = (if s <= 1e-4 { self.hsv.0 } else { h }, s, v);
        }
        self.sync_fields();
    }

    fn drag_to(&mut self, p: (f32, f32)) {
        match self.drag {
            Some(Drag::Square) => {
                let r = self.square;
                self.hsv.1 = ((p.0 - r[0]) / r[2]).clamp(0.0, 1.0);
                self.hsv.2 = 1.0 - ((p.1 - r[1]) / r[3]).clamp(0.0, 1.0);
            }
            Some(Drag::Hue) => {
                let r = self.hue;
                self.hsv.0 = ((p.1 - r[1]) / r[3]).clamp(0.0, 0.9999) * 360.0;
            }
            None => return,
        }
        self.sync_fields();
    }
}

impl Dialog for ColorDialog {
    fn size(&self, c: &Ctx) -> (f32, f32) {
        (c.px(440.0), c.px(392.0))
    }

    fn event(&mut self, ev: &DlgEvent, clip: &mut dyn Clipboard) {
        match ev {
            DlgEvent::Move(p) => {
                self.hits.mouse = *p;
                self.drag_to(*p);
            }
            DlgEvent::Down(0, p, _) => {
                self.hits.pressed = self.hits.at(*p);
                match self.hits.pressed {
                    Some(1) => {
                        self.drag = Some(Drag::Square);
                        self.focus = None;
                        self.drag_to(*p);
                    }
                    Some(2) => {
                        self.drag = Some(Drag::Hue);
                        self.focus = None;
                        self.drag_to(*p);
                    }
                    Some(id) if (10..14).contains(&id) => {
                        let k = (id - 10) as usize;
                        self.focus = Some(k);
                        self.fields[k].edit.select_all();
                    }
                    _ => {}
                }
            }
            DlgEvent::Up(0, p) => {
                self.drag = None;
                if let (Some(id), Some(up)) = (self.hits.pressed.take(), self.hits.at(*p)) {
                    if id == up {
                        match id {
                            100 => {
                                self.result = Some(self.rgb());
                                self.closed = true;
                            }
                            101 => self.closed = true,
                            _ => {}
                        }
                    }
                }
            }
            DlgEvent::Key(vk, m) => {
                if *vk == 0x1B {
                    self.closed = true;
                } else if *vk == 0x0D {
                    if let Some(k) = self.focus {
                        self.apply_fields(k);
                        self.focus = None;
                    } else {
                        self.result = Some(self.rgb());
                        self.closed = true;
                    }
                } else if let Some(k) = self.focus {
                    if *vk == 0x09 {
                        self.apply_fields(k);
                        let next = (k + if m.shift { 3 } else { 1 }) % 4;
                        self.focus = Some(next);
                        self.fields[next].edit.select_all();
                    } else {
                        let (changed, _) = self.fields[k].key(*vk, m.ctrl, m.shift, clip);
                        if changed {
                            self.apply_fields_live(k);
                        }
                    }
                }
            }
            DlgEvent::Char(ch) => {
                if let Some(k) = self.focus {
                    if self.fields[k].char_input(*ch) {
                        self.apply_fields_live(k);
                    }
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, c: &mut Ctx, r: Rect) {
        let s = c.s;
        self.hits.clear();
        frame(c, r, "Pick a color");
        // saturation / value square
        let sq = [r[0] + 20.0 * s, r[1] + 58.0 * s, 240.0 * s, 240.0 * s];
        self.square = sq;
        let hue_c = hsv_to_rgb(self.hsv.0, 1.0, 1.0);
        let hue_c = [hue_c[0], hue_c[1], hue_c[2], 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        c.r.push_clip(sq[0], sq[1], sq[2], sq[3]);
        c.r.gradient(sq[0], sq[1], sq[2], sq[3], [white, hue_c, hue_c, white]);
        c.r.gradient(sq[0], sq[1], sq[2], sq[3], [[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]]);
        c.r.pop_clip();
        c.r.rect(sq[0], sq[1], sq[2], sq[3], 4.0 * s, [0.0; 4], 1.0, c_border());
        self.hits.add(1, sq);
        let (mx, my) = (sq[0] + self.hsv.1 * sq[2], sq[1] + (1.0 - self.hsv.2) * sq[3]);
        c.r.circle(mx, my, 7.0 * s, [0.0; 4], 2.0, [1.0; 4]);
        c.r.circle(mx, my, 8.5 * s, [0.0; 4], 1.0, [0.0, 0.0, 0.0, 0.6]);
        // hue bar
        let hb = [sq[0] + sq[2] + 16.0 * s, sq[1], 22.0 * s, sq[3]];
        self.hue = hb;
        let stops = 6;
        for k in 0..stops {
            let (a, b) = (hsv_to_rgb(k as f32 * 60.0, 1.0, 1.0), hsv_to_rgb((k + 1) as f32 * 60.0, 1.0, 1.0));
            let (ca, cb) = ([a[0], a[1], a[2], 1.0], [b[0], b[1], b[2], 1.0]);
            c.r.gradient(hb[0], hb[1] + hb[3] * k as f32 / stops as f32, hb[2], hb[3] / stops as f32 + 0.5, [ca, ca, cb, cb]);
        }
        c.r.rect(hb[0], hb[1], hb[2], hb[3], 4.0 * s, [0.0; 4], 1.0, c_border());
        self.hits.add(2, hb);
        let hy = hb[1] + self.hsv.0 / 360.0 * hb[3];
        c.r.rect(hb[0] - 3.0 * s, hy - 3.0 * s, hb[2] + 6.0 * s, 6.0 * s, 3.0 * s, [0.0; 4], 2.0, [1.0; 4]);
        // preview: new over old
        let pv = [hb[0] + hb[2] + 16.0 * s, sq[1], 56.0 * s, 56.0 * s];
        let new = self.rgb();
        c.r.rect(pv[0], pv[1], pv[2], pv[3] * 0.5, 6.0 * s, [new[0], new[1], new[2], 1.0], 0.0, [0.0; 4]);
        c.r.rect(pv[0], pv[1] + pv[3] * 0.5, pv[2], pv[3] * 0.5, 6.0 * s, [self.old[0], self.old[1], self.old[2], 1.0], 0.0, [0.0; 4]);
        c.r.rect(pv[0], pv[1], pv[2], pv[3], 6.0 * s, [0.0; 4], 1.0, c_border());
        c.text_in(&c.f.ui, "new", pv[0] + pv[2] + 6.0 * s, pv[1], pv[3] * 0.5, c_muted());
        c.text_in(&c.f.ui, "old", pv[0] + pv[2] + 6.0 * s, pv[1] + pv[3] * 0.5, pv[3] * 0.5, c_muted());
        // numbers
        let fy = sq[1] + sq[3] + 16.0 * s;
        let labels = ["R", "G", "B"];
        for k in 0..3 {
            let fx = r[0] + 20.0 * s + k as f32 * 92.0 * s;
            c.text_in(&c.f.ui, labels[k], fx, fy, 26.0 * s, c_muted());
            let fr = [fx + 16.0 * s, fy, 62.0 * s, 26.0 * s];
            self.hits.add(10 + k as u32, fr);
            self.fields[k].draw(c, fr, &c.f.ui, self.focus == Some(k), true, c_field());
        }
        let hx = r[0] + 20.0 * s + 3.0 * 92.0 * s;
        let fr = [hx + 4.0 * s, fy, 88.0 * s, 26.0 * s];
        self.hits.add(13, fr);
        self.fields[3].draw(c, fr, &c.f.ui, self.focus == Some(3), true, c_field());
        let (bw, bh) = (c.px(96.0), c.px(30.0));
        dialog_button(c, &mut self.hits, 100, [r[0] + r[2] - 20.0 * s - bw, r[1] + r[3] - 16.0 * s - bh, bw, bh], "OK", true, true);
        dialog_button(c, &mut self.hits, 101, [r[0] + r[2] - 30.0 * s - bw * 2.0, r[1] + r[3] - 16.0 * s - bh, bw, bh], "Cancel", false, true);
    }

    fn done(&self) -> bool {
        self.closed
    }
}

impl ColorDialog {
    /// Typing in a field: take over valid values right away (the field being typed in keeps its text).
    fn apply_fields_live(&mut self, k: usize) {
        self.apply_fields(k);
        let t = self.fields[k].edit.text();
        self.fields[k].edit.set_text(&t);
    }
}

// ------------------------------------------------------------------------------------------
// File dialog
// ------------------------------------------------------------------------------------------
/// Does `name` match a pattern list like "*.blend;*.obj" (case-insensitive)?
pub fn matches_filter(name: &str, pattern: &str) -> bool {
    let name = name.to_lowercase();
    pattern.split(';').map(str::trim).any(|p| {
        let p = p.to_lowercase();
        if p == "*" || p == "*.*" {
            true
        } else if let Some(ext) = p.strip_prefix("*.") {
            name.ends_with(&format!(".{ext}"))
        } else {
            name == p
        }
    })
}

/// Folders first, then files, each alphabetical; only files matching the filter are listed.
pub fn list_dir(dir: &Path, pattern: &str) -> Vec<(String, bool, u64)> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = e.metadata() else { continue };
            if name.starts_with('.') {
                continue;
            }
            if meta.is_dir() {
                dirs.push((name, true, 0));
            } else if matches_filter(&name, pattern) {
                files.push((name, false, meta.len()));
            }
        }
    }
    dirs.sort_by_key(|d| d.0.to_lowercase());
    files.sort_by_key(|f| f.0.to_lowercase());
    dirs.extend(files);
    dirs
}

pub struct FileDialog {
    save: bool,
    title: String,
    filters: Vec<(String, String)>,
    filter: usize,
    default_ext: String,
    dir: PathBuf,
    entries: Vec<(String, bool, u64)>,
    list: List,
    path_field: Field,
    name_field: Field,
    focus: Option<u8>,
    /// The chosen path (None while open or cancelled).
    pub result: Option<PathBuf>,
    closed: bool,
    hits: Hits,
    list_rect: Rect,
    confirm: bool,
    error: String,
    drives: Vec<String>,
    last_click: Option<(std::time::Instant, usize)>,
    last_scale: std::cell::Cell<f32>,
}

fn drive_list() -> Vec<String> {
    (b'A'..=b'Z').map(|c| format!("{}:\\", c as char)).filter(|d| Path::new(d).exists()).collect()
}

impl FileDialog {
    pub fn new(save: bool, filters: &[(&str, &str)], default_ext: &str, initial_name: &str, start_dir: &Path) -> FileDialog {
        let mut d = FileDialog {
            save,
            title: if save { "Save as".into() } else { "Open".into() },
            filters: filters.iter().map(|&(a, b)| (a.to_string(), b.to_string())).collect(),
            filter: 0,
            default_ext: default_ext.to_string(),
            dir: start_dir.to_path_buf(),
            entries: Vec::new(),
            list: List::new(&[("Name", 330.0), ("Type", 90.0), ("Size", 80.0)], true),
            path_field: Field::new(false, false),
            name_field: Field::new(false, false),
            focus: Some(1),
            result: None,
            closed: false,
            hits: Hits::default(),
            list_rect: [0.0; 4],
            confirm: false,
            error: String::new(),
            drives: drive_list(),
            last_click: None,
            last_scale: std::cell::Cell::new(1.0),
        };
        d.name_field.edit.set_text(initial_name);
        d.name_field.edit.select_all();
        d.refresh();
        d
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn pattern(&self) -> String {
        self.filters.get(self.filter).map_or("*.*".to_string(), |f| f.1.clone())
    }

    fn refresh(&mut self) {
        let pattern = self.pattern();
        self.entries = list_dir(&self.dir, &pattern);
        self.list.clear();
        if self.dir.parent().is_some() {
            self.list.add(vec!["..".into(), "Parent folder".into(), String::new()]);
        }
        let has_parent = self.dir.parent().is_some();
        for (name, is_dir, size) in &self.entries {
            let kind = if *is_dir {
                "Folder".to_string()
            } else {
                match Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
                    Some("polystation" | "gpr") => "Project".into(),
                    Some("blend" | "obj" | "fbx" | "gltf" | "glb" | "stl" | "ply" | "dae" | "abc" | "usd" | "usda" | "usdc" | "usdz" | "x3d" | "wrl" | "3ds") => "Model".into(),
                    Some("bmp" | "png" | "jpg" | "jpeg" | "tga" | "tif" | "tiff") => "Image".into(),
                    Some("wav" | "ogg" | "mp3" | "flac") => "Sound".into(),
                    _ => "File".into(),
                }
            };
            let size = if *is_dir { String::new() } else if *size < 1024 { format!("{size} B") } else { format!("{} KB", size / 1024) };
            self.list.add(vec![name.clone(), kind, size]);
        }
        let _ = has_parent;
        self.path_field.edit.set_text(&self.dir.to_string_lossy());
        self.confirm = false;
        self.error.clear();
    }

    fn enter_dir(&mut self, dir: PathBuf) {
        if dir.is_dir() {
            self.dir = dir;
            self.refresh();
        } else {
            self.error = "That folder does not exist".into();
        }
    }

    /// The path the Open / Save button would pick.
    fn chosen(&self) -> Option<PathBuf> {
        let name = self.name_field.edit.text().trim().to_string();
        if name.is_empty() {
            return None;
        }
        let mut p = if Path::new(&name).is_absolute() { PathBuf::from(&name) } else { self.dir.join(&name) };
        if p.extension().is_none() && !self.default_ext.is_empty() && !p.is_dir() {
            p.set_extension(self.default_ext.trim_start_matches('.'));
        }
        Some(p)
    }

    fn accept(&mut self) {
        let Some(p) = self.chosen() else { return };
        if p.is_dir() {
            self.enter_dir(p);
            self.name_field.edit.set_text("");
            return;
        }
        if self.save {
            if p.exists() && !self.confirm {
                self.confirm = true;
                return;
            }
        } else if !p.is_file() {
            self.error = "That file does not exist".into();
            return;
        }
        self.result = Some(p);
        self.closed = true;
    }

    fn row_entry(&self, row: usize) -> Option<(String, bool)> {
        let has_parent = self.dir.parent().is_some();
        if has_parent && row == 0 {
            return Some(("..".into(), true));
        }
        let i = row - has_parent as usize;
        self.entries.get(i).map(|e| (e.0.clone(), e.1))
    }

    fn click_row(&mut self, row: usize, double: bool) {
        self.list.selected = Some(row);
        let Some((name, is_dir)) = self.row_entry(row) else { return };
        if is_dir {
            if double {
                let target = if name == ".." { self.dir.parent().map(Path::to_path_buf) } else { Some(self.dir.join(&name)) };
                if let Some(t) = target {
                    self.enter_dir(t);
                }
            }
        } else {
            self.name_field.edit.set_text(&name);
            self.confirm = false;
            if double {
                self.accept();
            }
        }
    }
}

impl Dialog for FileDialog {
    fn size(&self, c: &Ctx) -> (f32, f32) {
        (c.px(720.0), c.px(520.0))
    }

    fn event(&mut self, ev: &DlgEvent, clip: &mut dyn Clipboard) {
        match ev {
            DlgEvent::Move(p) => {
                self.hits.mouse = *p;
                self.list.hover = None;
                let lr = self.list_rect;
                if inside(*p, lr) {
                    self.list.hover = self.list.row_at(*p, lr, self.list_scale());
                }
            }
            DlgEvent::Wheel(n, p) => {
                if inside(*p, self.list_rect) {
                    let s = self.list_scale();
                    let hh = (list::HEADER_H * s).round();
                    self.list.scroll.wheel(*n, 3.0 * (list::ROW_H * s).round(), self.list.content_height(s), self.list_rect[3] - hh);
                }
            }
            DlgEvent::Down(0, p, double) => {
                self.hits.pressed = self.hits.at(*p);
                let lr = self.list_rect;
                if inside(*p, lr) {
                    if let Some(row) = self.list.row_at(*p, lr, self.list_scale()) {
                        let now = std::time::Instant::now();
                        let dbl = *double || self.last_click.is_some_and(|(t, r)| r == row && now.duration_since(t).as_millis() < 450);
                        self.last_click = Some((now, row));
                        self.click_row(row, dbl);
                    }
                    self.focus = None;
                }
                match self.hits.pressed {
                    Some(10) => {
                        self.focus = Some(0);
                        self.path_field.edit.select_all();
                    }
                    Some(11) => {
                        self.focus = Some(1);
                        self.name_field.edit.select_all();
                    }
                    _ => {}
                }
            }
            DlgEvent::Up(0, p) => {
                if let (Some(id), Some(up)) = (self.hits.pressed.take(), self.hits.at(*p)) {
                    if id == up {
                        match id {
                            100 => self.accept(),
                            101 => self.closed = true,
                            102 => {
                                if let Some(par) = self.dir.parent().map(Path::to_path_buf) {
                                    self.enter_dir(par);
                                }
                            }
                            103 => {
                                self.filter = (self.filter + 1) % self.filters.len().max(1);
                                self.refresh();
                            }
                            d if (200..300).contains(&d) => {
                                if let Some(dr) = self.drives.get((d - 200) as usize).cloned() {
                                    self.enter_dir(PathBuf::from(dr));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            DlgEvent::Key(vk, m) => match *vk {
                0x1B => self.closed = true,
                0x0D => {
                    if self.focus == Some(0) {
                        let t = self.path_field.edit.text();
                        self.enter_dir(PathBuf::from(t.trim()));
                        self.focus = Some(1);
                    } else {
                        self.accept();
                    }
                }
                0x09 => self.focus = Some(if self.focus == Some(1) { 0 } else { 1 }),
                0x26 | 0x28 if self.focus.is_none() => {
                    let view = self.list_rect[3];
                    let s = self.list_scale();
                    self.list.step(if *vk == 0x28 { 1 } else { -1 }, view, s);
                    if let Some(row) = self.list.selected {
                        if let Some((name, false)) = self.row_entry(row) {
                            self.name_field.edit.set_text(&name);
                        }
                    }
                }
                _ => {
                    let f = if self.focus == Some(0) { &mut self.path_field } else { &mut self.name_field };
                    if self.focus.is_some() {
                        let (changed, _) = f.key(*vk, m.ctrl, m.shift, clip);
                        if changed {
                            self.confirm = false;
                        }
                    }
                }
            },
            DlgEvent::Char(ch) => {
                let f = if self.focus == Some(0) { &mut self.path_field } else { &mut self.name_field };
                if self.focus.is_some() && f.char_input(*ch) {
                    self.confirm = false;
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, c: &mut Ctx, r: Rect) {
        let s = c.s;
        self.hits.clear();
        frame(c, r, &self.title);
        let (lx, rx) = (r[0] + 20.0 * s, r[0] + r[2] - 20.0 * s);
        // path bar
        let y0 = r[1] + 52.0 * s;
        let up = [lx, y0, 34.0 * s, 28.0 * s];
        dialog_button(c, &mut self.hits, 102, up, "Up", false, self.dir.parent().is_some());
        let path_r = [lx + 42.0 * s, y0, rx - lx - 42.0 * s, 28.0 * s];
        self.hits.add(10, path_r);
        self.path_field.draw(c, path_r, &c.f.ui, self.focus == Some(0), true, c_field());
        // drives
        let mut dx = lx;
        let dy = y0 + 36.0 * s;
        for (i, d) in self.drives.clone().iter().enumerate() {
            let dr = [dx, dy, 44.0 * s, 24.0 * s];
            dialog_button(c, &mut self.hits, 200 + i as u32, dr, d.trim_end_matches('\\'), false, true);
            dx += 50.0 * s;
        }
        // the list
        let lr = [lx, dy + 32.0 * s, rx - lx, r[1] + r[3] - (dy + 32.0 * s) - 122.0 * s];
        self.list_rect = lr;
        self.list_scale_cache(s);
        self.list.draw(c, lr, self.focus.is_none());
        c.r.rect(lr[0], lr[1], lr[2], lr[3], 4.0 * s, [0.0; 4], 1.0, c_border_soft());
        // file name + filter
        let ny = lr[1] + lr[3] + 14.0 * s;
        c.text_in(&c.f.ui, "File name", lx, ny, 28.0 * s, c_muted());
        let nr = [lx + 84.0 * s, ny, rx - lx - 84.0 * s - 190.0 * s, 28.0 * s];
        self.hits.add(11, nr);
        self.name_field.draw(c, nr, &c.f.ui, self.focus == Some(1), true, c_field());
        let fr = [nr[0] + nr[2] + 10.0 * s, ny, 180.0 * s, 28.0 * s];
        let label = self.filters.get(self.filter).map_or("All files".to_string(), |f| f.0.clone());
        dialog_button(c, &mut self.hits, 103, fr, &label, false, self.filters.len() > 1);
        // message line + buttons
        let by = r[1] + r[3] - 46.0 * s;
        let msg = if !self.error.is_empty() {
            Some((self.error.clone(), c_error()))
        } else if self.confirm {
            Some(("That file exists. Press Replace to overwrite it.".to_string(), c_error()))
        } else {
            None
        };
        if let Some((m, col)) = msg {
            c.text_in(&c.f.ui, &m, lx, by, 30.0 * s, col);
        }
        let (bw, bh) = (c.px(100.0), c.px(30.0));
        let label = if !self.save { "Open" } else if self.confirm { "Replace" } else { "Save" };
        dialog_button(c, &mut self.hits, 100, [rx - bw, by, bw, bh], label, true, true);
        dialog_button(c, &mut self.hits, 101, [rx - bw * 2.0 - 10.0 * s, by, bw, bh], "Cancel", false, true);
    }

    fn done(&self) -> bool {
        self.closed
    }
}

impl FileDialog {
    fn list_scale(&self) -> f32 {
        self.last_scale.get()
    }

    fn list_scale_cache(&self, s: f32) {
        self.last_scale.set(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for c in [[1.0, 0.0, 0.0], [0.2, 0.6, 0.9], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.9, 0.8, 0.1]] {
            let (h, s, v) = rgb_to_hsv(c);
            let back = hsv_to_rgb(h, s, v);
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-4, "{c:?} -> {back:?}");
            }
        }
        assert_eq!(rgb_to_hsv([0.0, 1.0, 0.0]).0, 120.0);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(hex_of([1.0, 0.5, 0.0]), "#FF8000");
        assert_eq!(parse_hex("#FF8000"), Some([1.0, 128.0 / 255.0, 0.0]));
        assert_eq!(parse_hex("00ff00"), Some([0.0, 1.0, 0.0]));
        assert_eq!(parse_hex("#12"), None);
        assert_eq!(parse_hex("#GGGGGG"), None);
    }

    #[test]
    fn filters_and_listing() {
        assert!(matches_filter("Scene.BLEND", "*.blend;*.obj"));
        assert!(matches_filter("a.obj", "*.blend;*.obj"));
        assert!(!matches_filter("a.png", "*.blend;*.obj"));
        assert!(matches_filter("anything", "*.*"));
        let dir = std::env::temp_dir().join("gpr_dialog_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zeta")).unwrap();
        std::fs::create_dir_all(dir.join("Alpha")).unwrap();
        std::fs::write(dir.join("b.obj"), b"x").unwrap();
        std::fs::write(dir.join("a.obj"), b"x").unwrap();
        std::fs::write(dir.join("c.png"), b"x").unwrap();
        let names: Vec<String> = list_dir(&dir, "*.obj").into_iter().map(|e| e.0).collect();
        assert_eq!(names, vec!["Alpha", "zeta", "a.obj", "b.obj"], "folders first, then the matching files");
    }
}
