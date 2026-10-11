//! The retained widgets of the main window (labels, buttons, text boxes, drop-down lists) and the text
//! field they share with the dialogs: drawing, caret and selection handling, mouse and key input.
use super::edit::{Edit, KeyCode};
use super::font::Font;
use super::render::{alpha, rgb, Color};
use super::scroll::VScroll;
use super::style::*;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Static,
    /// A small heading above a group of fields.
    Section,
    Button,
    Edit,
    Combo,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    Good,
    Error,
    Status,
}

/// How a text box colors its text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Highlight {
    None,
    Script,
    Guide,
}

pub struct Field {
    pub edit: Edit,
    pub multi: bool,
    pub readonly: bool,
    pub highlight: Highlight,
    pub gutter: bool,
    pub scroll: VScroll,
    pub scroll_x: f32,
    pub selecting: bool,
    pub blink: Instant,
    last_click: Option<(Instant, usize)>,
}

impl Field {
    pub fn new(multi: bool, readonly: bool) -> Field {
        Field { edit: Edit::new("", multi), multi, readonly, highlight: Highlight::None, gutter: false, scroll: VScroll::default(), scroll_x: 0.0, selecting: false, blink: Instant::now(), last_click: None }
    }

    fn line_h(&self, font: &Font) -> f32 {
        if self.multi { font.line_h.round() + 3.0 } else { font.line_h }
    }

    fn gutter_w(&self, font: &Font, s: f32) -> f32 {
        if self.gutter {
            let digits = self.edit.line_count().to_string().len().max(2) as f32;
            (digits * font.advance('0') + 18.0 * s).round()
        } else {
            0.0
        }
    }

    /// Where the text starts inside the box: (left, top) before scrolling.
    fn origin(&self, rect: [f32; 4], font: &Font, s: f32) -> (f32, f32) {
        if self.multi {
            (rect[0] + self.gutter_w(font, s) + 8.0 * s, rect[1] + 6.0 * s)
        } else {
            (rect[0] + 8.0 * s, rect[1] + ((rect[3] - font.line_h) * 0.5).floor())
        }
    }

    pub fn content_size(&self, font: &Font, s: f32) -> (f32, f32) {
        let lh = self.line_h(font);
        let n = self.edit.line_count();
        let w = (0..n).map(|l| font.measure(&self.edit.line_text(l))).fold(0.0, f32::max);
        (w + self.gutter_w(font, s) + 24.0 * s, n as f32 * lh + 12.0 * s)
    }

    /// The character index nearest to a window point.
    pub fn pos_at(&self, p: (f32, f32), rect: [f32; 4], font: &Font, s: f32) -> usize {
        let (ox, oy) = self.origin(rect, font, s);
        let line = if self.multi { (((p.1 - oy + self.scroll.offset) / self.line_h(font)).floor().max(0.0) as usize).min(self.edit.line_count() - 1) } else { 0 };
        let text = self.edit.line_text(line);
        let target = p.0 - ox + self.scroll_x;
        let mut x = 0.0;
        let mut col = 0;
        for (i, ch) in text.chars().enumerate() {
            let a = font.advance(ch);
            if target < x + a * 0.5 {
                break;
            }
            x += a;
            col = i + 1;
        }
        self.edit.pos_from(line, col)
    }

    /// Scrolls so that the caret shows.
    pub fn show_caret(&mut self, rect: [f32; 4], font: &Font, s: f32) {
        let (line, col) = self.edit.line_col(self.edit.caret);
        let text: String = self.edit.line_text(line).chars().take(col).collect();
        let cx = font.measure(&text);
        let (ox, _) = self.origin(rect, font, s);
        let inner_w = rect[0] + rect[2] - ox - 8.0 * s - if self.multi { 12.0 * s } else { 0.0 };
        if cx - self.scroll_x > inner_w {
            self.scroll_x = cx - inner_w;
        }
        if cx < self.scroll_x {
            self.scroll_x = (cx - 20.0 * s).max(0.0);
        }
        if !self.multi {
            let total = font.measure(&self.edit.text());
            self.scroll_x = self.scroll_x.min((total - inner_w + 2.0).max(0.0)).max(0.0);
        } else {
            let lh = self.line_h(font);
            let (_, h) = self.content_size(font, s);
            let view = rect[3] - 12.0 * s;
            self.scroll.reveal(line as f32 * lh, (line + 1) as f32 * lh + 6.0 * s, h, view + 12.0 * s);
        }
        self.blink = Instant::now();
    }

    pub fn mouse_down(&mut self, p: (f32, f32), rect: [f32; 4], font: &Font, s: f32, shift: bool) {
        let pos = self.pos_at(p, rect, font, s);
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, at)| now.duration_since(t).as_millis() < 450 && at == pos);
        self.last_click = Some((now, pos));
        if double {
            let (a, b) = self.edit.word_at(pos);
            self.edit.anchor = a;
            self.edit.caret = b;
        } else {
            self.edit.set_caret(pos, shift);
        }
        self.selecting = true;
        self.blink = now;
    }

    pub fn mouse_drag(&mut self, p: (f32, f32), rect: [f32; 4], font: &Font, s: f32) {
        if self.selecting {
            let pos = self.pos_at(p, rect, font, s);
            self.edit.caret = pos;
            self.show_caret(rect, font, s);
        }
    }

    pub fn wheel(&mut self, notches: f32, rect: [f32; 4], font: &Font, s: f32) {
        if self.multi {
            let (_, h) = self.content_size(font, s);
            self.scroll.wheel(notches, self.line_h(font) * 3.0, h, rect[3]);
        }
    }

    /// A key press (virtual-key code). Returns (text changed, key was used).
    pub fn key(&mut self, vk: u32, ctrl: bool, shift: bool, clip: &mut dyn Clipboard) -> (bool, bool) {
        let code = match vk {
            0x25 => Some(KeyCode::Left),
            0x27 => Some(KeyCode::Right),
            0x26 => Some(KeyCode::Up),
            0x28 => Some(KeyCode::Down),
            0x24 => Some(KeyCode::Home),
            0x23 => Some(KeyCode::End),
            0x21 => Some(KeyCode::PageUp),
            0x22 => Some(KeyCode::PageDown),
            0x08 => Some(KeyCode::Backspace),
            0x2E => Some(KeyCode::Delete),
            0x0D => Some(KeyCode::Enter),
            0x09 => Some(KeyCode::Tab),
            _ => None,
        };
        if ctrl {
            match vk {
                0x41 => {
                    self.edit.select_all();
                    return (false, true);
                }
                0x43 => {
                    if let Some(t) = self.edit.copy() {
                        clip.set(&t);
                    }
                    return (false, true);
                }
                0x58 => {
                    if self.readonly {
                        return (false, true);
                    }
                    if let Some(t) = self.edit.cut() {
                        clip.set(&t);
                        return (true, true);
                    }
                    return (false, true);
                }
                0x56 => {
                    if self.readonly {
                        return (false, true);
                    }
                    let t = clip.get();
                    return (self.edit.insert(&t), true);
                }
                0x5A | 0x59 => {
                    if self.readonly {
                        return (false, true);
                    }
                    let done = if vk == 0x5A && !shift { self.edit.undo() } else { self.edit.redo() };
                    return (done, true);
                }
                _ => {}
            }
        }
        let Some(code) = code else { return (false, false) };
        let editing = matches!(code, KeyCode::Backspace | KeyCode::Delete | KeyCode::Enter | KeyCode::Tab);
        if self.readonly && editing {
            return (false, true);
        }
        if !self.multi && matches!(code, KeyCode::Enter | KeyCode::Tab) {
            return (false, false); // the owner decides (leave the box)
        }
        let changed = self.edit.key(code, shift, ctrl);
        (changed, true)
    }

    pub fn char_input(&mut self, c: char) -> bool {
        !self.readonly && self.edit.char_input(c)
    }

    pub fn draw(&mut self, c: &mut Ctx, rect: [f32; 4], font: &Font, focused: bool, enabled: bool, bg: Color) {
        let s = c.s;
        let [x, y, w, h] = rect;
        c.field_frame(rect, focused, enabled, bg);
        let (ox, oy) = self.origin(rect, font, s);
        let lh = self.line_h(font);
        c.r.push_clip(x + 2.0, y + 2.0, w - 4.0, h - 4.0);
        let sel = self.edit.selection();
        let blink_on = focused && !self.readonly && (self.blink.elapsed().as_millis() % 1060) < 640;
        let text_color = if enabled { c_text() } else { c_dim() };
        let n = self.edit.line_count();
        let (first, last) = if self.multi {
            (((self.scroll.offset / lh).floor() as usize).min(n - 1), (((self.scroll.offset + h) / lh).ceil() as usize + 1).min(n))
        } else {
            (0, 1)
        };
        for line in first..last {
            let ly = if self.multi { oy + line as f32 * lh - self.scroll.offset } else { oy };
            let text = self.edit.line_text(line);
            let start = self.edit.line_start(line);
            let chars: Vec<char> = text.chars().collect();
            let tx = ox - self.scroll_x;
            // current line highlight (text areas)
            if self.multi && focused && !self.edit.has_selection() && self.edit.line_col(self.edit.caret).0 == line {
                c.r.fill(x + self.gutter_w(font, s), ly - 1.0, w, lh, alpha([1.0; 4], 0.035));
            }
            // selection
            let (a, b) = (sel.0.max(start), sel.1.min(start + chars.len() + 1));
            if a < b && self.edit.has_selection() {
                let pre: String = chars.iter().take(a - start).collect();
                let mid: String = chars.iter().skip(a - start).take(b - a.min(b)).collect();
                let extra = if sel.1 > start + chars.len() { font.advance(' ') * 0.6 } else { 0.0 };
                c.r.fill(tx + font.measure(&pre), ly - 1.0, font.measure(&mid) + extra, lh, alpha(c_selection(), if focused { 0.85 } else { 0.45 }));
            }
            // text
            match self.highlight {
                Highlight::Script => {
                    let mut x0 = tx;
                    for (s0, s1, style) in highlight_line(&text) {
                        let part: String = chars[s0..s1].iter().collect();
                        x0 += c.r.text(font, &part, x0, ly, style_color(style));
                    }
                }
                Highlight::Guide => {
                    let heading = !text.is_empty() && text.chars().any(|ch| ch.is_ascii_uppercase()) && !text.chars().any(|ch| ch.is_ascii_lowercase()) && !text.starts_with(' ');
                    c.r.text(font, &text, tx, ly, if heading { c_section() } else { text_color });
                }
                Highlight::None => {
                    c.r.text(font, &text, tx, ly, text_color);
                }
            }
            if self.gutter {
                let g = self.gutter_w(font, s);
                let num = (line + 1).to_string();
                let cur = self.edit.line_col(self.edit.caret).0 == line;
                c.r.fill(x + 1.0, ly - 1.0, g, lh, alpha(c_bg(), 0.0));
                c.r.text(font, &num, x + g - 10.0 * s - font.measure(&num), ly, if cur { c_muted() } else { c_dim() });
            }
            // caret
            if blink_on && self.edit.line_col(self.edit.caret).0 == line {
                let col = self.edit.line_col(self.edit.caret).1;
                let pre: String = chars.iter().take(col).collect();
                let cx = (tx + font.measure(&pre)).round();
                c.r.fill(cx, ly - 1.0, (1.5 * s).max(1.0), lh, c_text());
            }
        }
        c.r.pop_clip();
        if self.gutter {
            let g = self.gutter_w(font, s);
            c.r.fill(x + g, y + 2.0, 1.0, h - 4.0, c_border_soft());
        }
        if self.multi {
            let (cw, ch) = self.content_size(font, s);
            self.scroll.clamp(ch, h);
            let track = [x + w - 12.0 * s, y, 12.0 * s, h];
            if let Some(t) = self.scroll.thumb(track, ch, h) {
                c.scrollbar(t, self.scroll.dragging());
            }
            let _ = cw;
        }
    }
}

/// Clipboard access for the fields.
pub trait Clipboard {
    fn get(&mut self) -> String;
    fn set(&mut self, text: &str);
}

// ------------------------------------------------------------------------------------------
// Syntax coloring of scripts
// ------------------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Plain,
    Keyword,
    Builtin,
    Prop,
    Str,
    Comment,
    Number,
}

const KEYWORDS: [&str; 17] = ["var", "fn", "on", "if", "else", "while", "for", "in", "return", "break", "continue", "true", "false", "nil", "and", "or", "not"];
const BUILTINS: [&str; 41] = [
    "sin", "cos", "tan", "atan2", "abs", "sqrt", "floor", "ceil", "round", "sign", "pow", "min", "max", "clamp", "lerp", "random", "print", "key", "pressed", "distance", "object", "exists", "move", "spin", "set", "get", "add",
    "destroy", "launch", "hud", "sound", "str", "start", "update", "touch", "leave", "every", "pi", "dt", "time", "self",
];
const PROPS: [&str; 10] = ["position", "rotation", "scale", "color", "alpha", "visible", "solid", "intensity", "player", "name"];

pub fn style_color(s: Style) -> Color {
    match s {
        Style::Plain => rgb(0xd6deeb),
        Style::Keyword => rgb(0xc586c0),
        Style::Builtin => rgb(0xdcdcaa),
        Style::Prop => rgb(0x9cdcfe),
        Style::Str => rgb(0xce9178),
        Style::Comment => rgb(0x6a9955),
        Style::Number => rgb(0xb5cea8),
    }
}

/// Splits a line into colored pieces: (start, end, style) in characters.
pub fn highlight_line(line: &str) -> Vec<(usize, usize, Style)> {
    let c: Vec<char> = line.chars().collect();
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    let mut push = |a: usize, b: usize, s: Style| {
        if a < b {
            out.push((a, b, s));
        }
    };
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            push(i, c.len(), Style::Comment);
            break;
        } else if ch == '"' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '"' {
                if c[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(c.len());
            push(i, end, Style::Str);
            i = end;
        } else if ch.is_ascii_digit() {
            let mut j = i;
            while j < c.len() && (c[j].is_ascii_digit() || (c[j] == '.' && c.get(j + 1) != Some(&'.'))) {
                j += 1;
            }
            push(i, j, Style::Number);
            i = j;
        } else if ch.is_alphabetic() || ch == '_' {
            let mut j = i;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_') {
                j += 1;
            }
            let word: String = c[i..j].iter().collect();
            let after_dot = i > 0 && c[i - 1] == '.';
            let style = if KEYWORDS.contains(&word.as_str()) {
                Style::Keyword
            } else if PROPS.contains(&word.as_str()) || (after_dot && matches!(word.as_str(), "x" | "y" | "z" | "r" | "g" | "b")) {
                Style::Prop
            } else if BUILTINS.contains(&word.as_str()) && c.get(j) == Some(&'(') || (BUILTINS.contains(&word.as_str()) && !after_dot) {
                Style::Builtin
            } else {
                Style::Plain
            };
            push(i, j, style);
            i = j;
        } else {
            let mut j = i;
            while j < c.len() && !(c[j].is_alphanumeric() || c[j] == '_' || c[j] == '"' || (c[j] == '/' && c.get(j + 1) == Some(&'/'))) {
                j += 1;
            }
            let j = j.max(i + 1);
            push(i, j, Style::Plain);
            i = j;
        }
    }
    out
}

// ------------------------------------------------------------------------------------------
// Widgets
// ------------------------------------------------------------------------------------------
pub struct Widget {
    pub id: u16,
    pub kind: Kind,
    pub group: u8,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub text: String,
    pub enabled: bool,
    pub toggle: bool,
    pub on: bool,
    pub accent: bool,
    pub tone: Tone,
    pub field: Option<Box<Field>>,
    pub items: Vec<String>,
    pub sel: usize,
}

impl Widget {
    pub fn new(id: u16, kind: Kind, group: u8, x: i32, y: i32, w: i32, h: i32, text: &str) -> Widget {
        Widget { id, kind, group, x, y, w, h, text: text.to_string(), enabled: true, toggle: false, on: false, accent: false, tone: Tone::Normal, field: None, items: Vec::new(), sel: 0 }
    }
}

pub fn draw_static(c: &mut Ctx, rect: [f32; 4], w: &Widget) {
    let [x, y, rw, rh] = rect;
    match w.kind {
        Kind::Section => {
            c.text_in(&c.f.bold, &w.text, x, y, rh, c_section());
        }
        _ => {
            let color = match w.tone {
                Tone::Normal => c_text(),
                Tone::Good => c_good(),
                Tone::Error => c_error(),
                Tone::Status => c_muted(),
            };
            let font = &c.f.ui;
            let lh = font.line_h + 2.0;
            let lines = font.wrap(&w.text, rw);
            c.r.push_clip(x, y, rw, rh.max(lh));
            for (i, l) in lines.iter().enumerate() {
                let ly = if lines.len() == 1 && rh <= lh + 6.0 { y + ((rh - font.line_h) * 0.5).floor() } else { y + i as f32 * lh };
                c.r.text(font, l, x, ly, if w.enabled { color } else { c_dim() });
            }
            c.r.pop_clip();
        }
    }
}

pub fn draw_combo(c: &mut Ctx, rect: [f32; 4], w: &Widget, hot: bool, open: bool) {
    let [x, y, rw, _] = rect;
    let h = c.px(24.0);
    let fill = if open { c_btn_hot() } else if hot && w.enabled { c_btn_hot() } else { c_btn() };
    c.r.rect(x, y, rw, h, c.px(5.0), if w.enabled { fill } else { mix_dim(fill) }, 1.0, if open { c_accent() } else { c_border_soft() });
    let text = w.items.get(w.sel).map(String::as_str).unwrap_or("");
    let font = &c.f.ui;
    let fitted = font.fit(text, rw - c.px(34.0));
    c.text_in(font, &fitted, x + c.px(8.0), y, h, if w.enabled { c_text() } else { c_dim() });
    c.arrow(x + rw - c.px(14.0), y + h * 0.5 - c.px(1.0), c.px(4.5), true, if w.enabled { c_muted() } else { c_dim() });
}

fn mix_dim(c: Color) -> Color {
    super::render::mix(c, c_bg(), 0.35)
}
