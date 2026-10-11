//! The in-game GUI: screen-space elements (text, panels, bars, buttons) that a game shows on top of the 3D
//! view, like a score counter, a health bar or a menu button. They are designed in the GUI editor (View >
//! GUI Editor), saved with the project, drawn while playing (and in a compiled game) and controlled by
//! scripts (`gui_text`, `gui_value`, `gui_show`, `gui_move`, `gui_color`, `clicked`, `cursor`).
//!
//! Positions are in a 1280 x 720 design space and scale to the window (the smaller of the two scales,
//! so the layout keeps its shape). An element is attached to one of nine anchor points of the screen and
//! `x`, `y` are the distance from that edge, inward (from the centre: right / down).
use crate::ui::render::{alpha, mix, Color};
use crate::ui::style::*;

pub const REF_W: f32 = 1280.0;
pub const REF_H: f32 = 720.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Panel,
    Bar,
    Button,
}

impl Kind {
    pub const NAMES: [&'static str; 4] = ["Text", "Panel", "Bar", "Button"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Kind {
        [Kind::Text, Kind::Panel, Kind::Bar, Kind::Button][i.min(3)]
    }

    pub fn from_name(s: &str) -> Option<Kind> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(Kind::from_index)
    }
}

/// How the mouse works while a game with a GUI plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// The mouse turns the camera and is hidden. GUI buttons cannot be clicked (a heads-up display only).
    Look,
    /// The mouse is a pointer: it is shown, clicks GUI buttons and does not turn the camera (menus, point and click).
    Cursor,
    /// Looks around like `Look`; Tab switches the pointer on and off.
    #[default]
    Toggle,
    /// Looks around; the pointer is there while Ctrl is held.
    Hold,
}

impl Mode {
    pub const NAMES: [&'static str; 4] = ["Look", "Cursor", "Tab toggle", "Hold Ctrl"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Mode {
        [Mode::Look, Mode::Cursor, Mode::Toggle, Mode::Hold][i.min(3)]
    }

    /// Accepts the names scripts and tools use: look, cursor, toggle (tab), hold (ctrl).
    pub fn from_name(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "look" | "mouselook" => Some(Mode::Look),
            "cursor" | "pointer" | "menu" => Some(Mode::Cursor),
            "toggle" | "tab" | "tab toggle" => Some(Mode::Toggle),
            "hold" | "ctrl" | "hold ctrl" => Some(Mode::Hold),
            _ => None,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Mode::Look => "The mouse turns the camera. Buttons cannot be clicked: a heads-up display only.",
            Mode::Cursor => "The mouse is a pointer that clicks buttons. It does not turn the camera (menus, point and click).",
            Mode::Toggle => "The mouse turns the camera; press Tab to switch the pointer on and off.",
            Mode::Hold => "The mouse turns the camera; hold Ctrl to get the pointer.",
        }
    }

    /// The reminder shown in the game while the pointer is off / on.
    pub fn hint(self, pointer_on: bool) -> Option<&'static str> {
        match (self, pointer_on) {
            (Mode::Toggle, false) => Some("TAB: MOUSE"),
            (Mode::Toggle, true) => Some("TAB: LOOK"),
            (Mode::Hold, false) => Some("HOLD CTRL: MOUSE"),
            _ => None,
        }
    }
}

pub const ANCHOR_NAMES: [&str; 9] = ["top left", "top", "top right", "left", "center", "right", "bottom left", "bottom", "bottom right"];

#[derive(Clone, Debug, PartialEq)]
pub struct GuiElem {
    pub name: String,
    pub kind: Kind,
    /// 0..=8: row-major over the screen (top left, top, top right, left, centre ...).
    pub anchor: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub text: String,
    pub color: [f32; 3],
    pub alpha: f32,
    pub text_color: [f32; 3],
    /// 1 .. 4: how many screen pixels each pixel of the font becomes (at the design size).
    pub text_size: u8,
    /// Fill of a bar, 0..1.
    pub value: f32,
    pub visible: bool,
}

impl GuiElem {
    pub fn new(kind: Kind, name: &str) -> GuiElem {
        let (w, h, text, color, alpha, anchor) = match kind {
            Kind::Text => (300.0, 40.0, "Text", [1.0, 1.0, 1.0], 0.0, 0),
            Kind::Panel => (300.0, 120.0, "", [0.1, 0.1, 0.45], 0.9, 0),
            Kind::Bar => (300.0, 26.0, "", [0.2, 0.8, 0.2], 1.0, 6),
            Kind::Button => (180.0, 48.0, "Button", [0.3, 0.3, 0.65], 1.0, 4),
        };
        GuiElem { name: name.to_string(), kind, anchor, x: 24.0, y: 24.0, w, h, text: text.to_string(), color, alpha, text_color: [1.0, 1.0, 1.0], text_size: 1, value: 0.5, visible: true }
    }

    /// The element's rectangle (x, y, w, h) on a screen area given in pixels.
    pub fn rect(&self, area: [f32; 4]) -> [f32; 4] {
        let k = scale(area);
        let (w, h) = (self.w * k, self.h * k);
        let (col, row) = (self.anchor % 3, (self.anchor / 3).min(2));
        let x = match col {
            0 => area[0] + self.x * k,
            1 => area[0] + (area[2] - w) * 0.5 + self.x * k,
            _ => area[0] + area[2] - w - self.x * k,
        };
        let y = match row {
            0 => area[1] + self.y * k,
            1 => area[1] + (area[3] - h) * 0.5 + self.y * k,
            _ => area[1] + area[3] - h - self.y * k,
        };
        [x.round(), y.round(), w.round(), h.round()]
    }

    /// Which way x / y grow on screen (inward from the anchored edge).
    pub fn direction(&self) -> (f32, f32) {
        let (col, row) = (self.anchor % 3, (self.anchor / 3).min(2));
        (if col == 2 { -1.0 } else { 1.0 }, if row == 2 { -1.0 } else { 1.0 })
    }
}

/// How much the design space is scaled on a screen area.
pub fn scale(area: [f32; 4]) -> f32 {
    (area[2] / REF_W).min(area[3] / REF_H).max(0.01)
}

/// The topmost clickable element (a visible button) under a point, as an index.
pub fn hit_button(elems: &[GuiElem], area: [f32; 4], p: (f32, f32)) -> Option<usize> {
    elems.iter().enumerate().rev().find(|(_, e)| e.visible && e.kind == Kind::Button && inside(p, e.rect(area))).map(|(i, _)| i)
}

/// The topmost visible element of any kind under a point (for picking in the editor).
pub fn hit_any(elems: &[GuiElem], area: [f32; 4], p: (f32, f32)) -> Option<usize> {
    elems.iter().enumerate().rev().find(|(_, e)| e.visible && inside(p, e.rect(area))).map(|(i, _)| i)
}

fn inside(p: (f32, f32), r: [f32; 4]) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

fn rgba(c: [f32; 3], a: f32) -> Color {
    [c[0], c[1], c[2], a]
}

/// How many screen pixels one pixel of the font becomes: a whole number, so the text stays crisp.
fn text_mul(size: u8, k: f32) -> f32 {
    (size.clamp(1, 4) as f32 * k).round().max(1.0)
}

/// Text in the pixel font with a hard black drop shadow.
fn put(c: &mut Ctx, text: &str, x: f32, y: f32, m: f32, color: Color) {
    let f = c.f;
    c.r.text_scaled(&f.pixel, text, x + m, y + m, [0.0, 0.0, 0.0, color[3]], m);
    c.r.text_scaled(&f.pixel, text, x, y, color, m);
}

/// A flat rectangle with a square, one-colour frame (no rounding anywhere).
fn boxed(c: &mut Ctx, r: [f32; 4], bw: f32, fill: Color, frame: Color) {
    c.r.fill(r[0], r[1], r[2], r[3], frame);
    c.r.fill(r[0] + bw, r[1] + bw, (r[2] - 2.0 * bw).max(0.0), (r[3] - 2.0 * bw).max(0.0), fill);
}

/// Draws the elements on a screen area. `hover` / `down`: the button under the mouse and whether the
/// mouse button is held. The look is early-console: one pixel font, flat colours, square corners and
/// bevelled edges.
pub fn draw(c: &mut Ctx, elems: &[GuiElem], area: [f32; 4], hover: Option<usize>, down: bool) {
    let k = scale(area);
    let bw = (2.0 * k).round().max(1.0);
    let f = c.f;
    let line_h = f.pixel.line_h;
    for (i, e) in elems.iter().enumerate() {
        if !e.visible {
            continue;
        }
        let r = e.rect(area);
        let m = text_mul(e.text_size, k);
        let text_y = |r: [f32; 4]| (r[1] + ((r[3] - line_h * m) * 0.5).floor()).round();
        let col = e.anchor % 3;
        match e.kind {
            Kind::Panel => {
                let a = e.alpha.max(0.15);
                boxed(c, r, bw, rgba(e.color, a), { let mut l = mix(rgba(e.color, 1.0), [1.0; 4], 0.8); l[3] = a; l });
            }
            Kind::Bar => {
                let a = e.alpha.max(0.3);
                boxed(c, r, bw, [0.0, 0.0, 0.0, 0.85 * a], rgba([0.9, 0.9, 0.9], a));
                let inner = [r[0] + bw * 2.0, r[1] + bw * 2.0, (r[2] - bw * 4.0).max(0.0), (r[3] - bw * 4.0).max(0.0)];
                // the fill grows in whole blocks, like the segmented bars of the era
                let block = (inner[3] * 0.9).round().max(2.0);
                let blocks = (inner[2] / block).floor().max(1.0);
                let shown = if e.value <= 0.0 { 0 } else { (blocks * e.value.clamp(0.0, 1.0)).ceil() as usize };
                let mut lit = mix(rgba(e.color, 1.0), [1.0; 4], 0.4);
                lit[3] = e.alpha;
                for n in 0..shown {
                    let bx = (inner[0] + n as f32 * (inner[2] / blocks)).round();
                    let bwid = (inner[2] / blocks - bw).max(1.0).round();
                    c.r.fill(bx, inner[1], bwid, inner[3], rgba(e.color, e.alpha));
                    c.r.fill(bx, inner[1], bwid, (inner[3] * 0.35).round().max(1.0), lit);
                }
                if !e.text.is_empty() {
                    let tw = f.pixel.measure(&e.text) * m;
                    put(c, &e.text, (r[0] + (r[2] - tw) * 0.5).round(), text_y(r), m, rgba(e.text_color, 1.0));
                }
            }
            Kind::Button => {
                let pressed = hover == Some(i) && down;
                let base = rgba(e.color, e.alpha.max(0.3));
                let fill = if hover == Some(i) && !pressed { mix(base, [1.0, 1.0, 1.0, base[3]], 0.2) } else { base };
                let (light, dark) = (mix(base, [1.0, 1.0, 1.0, base[3]], 0.55), mix(base, [0.0, 0.0, 0.0, base[3]], 0.6));
                let (tl, br) = if pressed { (dark, light) } else { (light, dark) };
                c.r.fill(r[0], r[1], r[2], r[3], br); // the shaded edges are the whole block, the lit ones go on top
                c.r.fill(r[0], r[1], r[2] - bw, r[3] - bw, tl);
                c.r.fill(r[0] + bw, r[1] + bw, r[2] - 2.0 * bw, r[3] - 2.0 * bw, fill);
                let tw = f.pixel.measure(&e.text) * m;
                let nudge = if pressed { bw } else { 0.0 };
                put(c, &e.text, (r[0] + (r[2] - tw) * 0.5).round() + nudge, text_y(r) + nudge, m, rgba(e.text_color, 1.0));
            }
            Kind::Text => {
                if e.alpha > 0.0 {
                    c.r.fill(r[0], r[1], r[2], r[3], rgba(e.color, e.alpha * 0.6));
                }
                let tw = f.pixel.measure(&e.text) * m;
                let pad = (6.0 * k).round();
                let tx = match col {
                    0 => r[0] + pad,
                    1 => r[0] + ((r[2] - tw) * 0.5).floor(),
                    _ => r[0] + r[2] - tw - pad,
                };
                c.r.push_clip(r[0], r[1], r[2], r[3]);
                put(c, &e.text, tx.round(), text_y(r), m, rgba(e.text_color, 1.0));
                c.r.pop_clip();
            }
        }
    }
}

/// A small reminder in the bottom-right corner of the screen area.
pub fn hint(c: &mut Ctx, area: [f32; 4], text: &str) {
    let k = scale(area);
    let m = k.round().max(1.0);
    let tw = c.f.pixel.measure(text) * m;
    let (x, y) = (area[0] + area[2] - tw - (12.0 * k).round(), area[1] + area[3] - c.f.pixel.line_h * m - (10.0 * k).round());
    put(c, text, x.round(), y.round(), m, [1.0, 1.0, 1.0, 0.85]);
}

/// Parses `#rrggbb` / `rrggbb`.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0])
}

pub fn to_hex(c: [f32; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8)
}

/// Starts an element called `base` that does not clash with an existing name.
pub fn unique_name(elems: &[GuiElem], base: &str) -> String {
    crate::scene::unique_name(base, |n| elems.iter().any(|e| e.name == n))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: [f32; 4] = [0.0, 0.0, 1280.0, 720.0];

    #[test]
    fn anchors_place_elements_from_their_edges() {
        let mut e = GuiElem::new(Kind::Panel, "p");
        (e.w, e.h, e.x, e.y) = (100.0, 50.0, 10.0, 20.0);
        e.anchor = 0;
        assert_eq!(e.rect(SCREEN), [10.0, 20.0, 100.0, 50.0]);
        e.anchor = 8; // bottom right: inward
        assert_eq!(e.rect(SCREEN), [1280.0 - 100.0 - 10.0, 720.0 - 50.0 - 20.0, 100.0, 50.0]);
        e.anchor = 4; // centre: offsets go right / down
        assert_eq!(e.rect(SCREEN), [590.0 + 10.0, 335.0 + 20.0, 100.0, 50.0]);
    }

    #[test]
    fn the_layout_scales_with_the_window_and_keeps_its_shape() {
        let mut e = GuiElem::new(Kind::Panel, "p");
        (e.w, e.h, e.x, e.y, e.anchor) = (100.0, 50.0, 10.0, 20.0, 0);
        assert_eq!(e.rect([0.0, 0.0, 640.0, 360.0]), [5.0, 10.0, 50.0, 25.0]);
        // a wide window scales by its height, a tall one by its width
        assert!((scale([0.0, 0.0, 2560.0, 720.0]) - 1.0).abs() < 1e-6);
        assert!((scale([0.0, 0.0, 640.0, 2000.0]) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn buttons_are_hit_from_the_top_down_and_hidden_ones_are_not() {
        let mut a = GuiElem::new(Kind::Button, "a");
        let mut b = GuiElem::new(Kind::Button, "b");
        (a.anchor, b.anchor) = (0, 0);
        (a.x, a.y, b.x, b.y) = (0.0, 0.0, 0.0, 0.0);
        let list = vec![a.clone(), b.clone()];
        assert_eq!(hit_button(&list, SCREEN, (10.0, 10.0)), Some(1), "later = on top");
        b.visible = false;
        assert_eq!(hit_button(&[a, b], SCREEN, (10.0, 10.0)), Some(0));
        let panel = GuiElem::new(Kind::Panel, "p");
        assert_eq!(hit_button(&[panel.clone()], SCREEN, (30.0, 30.0)), None, "panels do not click");
        assert_eq!(hit_any(&[panel], SCREEN, (30.0, 30.0)), Some(0));
    }

    #[test]
    fn hex_colors_round_trip() {
        assert_eq!(parse_hex("#ff8000"), Some([1.0, 128.0 / 255.0, 0.0]));
        assert_eq!(to_hex([1.0, 0.0, 0.5]), "#FF0080");
        assert_eq!(parse_hex("zz"), None);
    }

    #[test]
    fn names_stay_unique() {
        let list = vec![GuiElem::new(Kind::Text, "Score"), GuiElem::new(Kind::Text, "Score.001")];
        assert_eq!(unique_name(&list, "Score"), "Score.002");
    }
}
