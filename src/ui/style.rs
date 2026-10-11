//! Colors and the shared drawing helpers of the UI.
use super::font::Font;
use super::render::{alpha, mix, rgb, Color, Renderer};

pub fn c_bg() -> Color { rgb(0x1e1f24) }
pub fn c_panel() -> Color { rgb(0x202126) }
pub fn c_field() -> Color { rgb(0x2a2b32) }
pub fn c_bar() -> Color { rgb(0x17181c) }
pub fn c_tree_bg() -> Color { rgb(0x222328) }
pub fn c_text() -> Color { rgb(0xe4e6eb) }
pub fn c_muted() -> Color { rgb(0x9a9ea8) }
pub fn c_dim() -> Color { rgb(0x6c707a) }
pub fn c_btn() -> Color { rgb(0x32343c) }
pub fn c_btn_hot() -> Color { rgb(0x42454f) }
pub fn c_btn_down() -> Color { rgb(0x27282f) }
pub fn c_border() -> Color { rgb(0x484b56) }
pub fn c_border_soft() -> Color { rgb(0x34363e) }
pub fn c_accent() -> Color { rgb(0x4d8eff) }
pub fn c_accent_hot() -> Color { rgb(0x6ca3ff) }
pub fn c_accent_down() -> Color { rgb(0x3972d8) }
pub fn c_selection() -> Color { rgb(0x2f5fb8) }
pub fn c_code_bg() -> Color { rgb(0x15161a) }
pub fn c_menu_bg() -> Color { rgb(0x25262c) }
pub fn c_menu_hot() -> Color { rgb(0x384b70) }
pub fn c_section() -> Color { rgb(0x8ba8e8) }
pub fn c_good() -> Color { rgb(0x7ec88a) }
pub fn c_error() -> Color { rgb(0xff7b72) }

/// What the UI draws with: the renderer, the fonts and the DPI scale.
pub struct Fonts {
    pub ui: Font,
    pub bold: Font,
    pub mono: Font,
    /// The pixel font of the game GUI.
    pub pixel: Font,
}

pub struct Ctx<'a> {
    pub r: &'a mut Renderer,
    pub f: &'a Fonts,
    pub s: f32,
}

impl Ctx<'_> {
    pub fn px(&self, v: f32) -> f32 {
        (v * self.s).round()
    }

    /// Text centered vertically in a box; returns its width.
    pub fn text_in(&mut self, font: &Font, s: &str, x: f32, y: f32, h: f32, color: Color) -> f32 {
        self.r.text(font, s, x, y + ((h - font.line_h) * 0.5).floor(), color)
    }

    pub fn text_center(&mut self, font: &Font, s: &str, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let tw = font.measure(s);
        self.r.text(font, s, x + ((w - tw) * 0.5).floor(), y + ((h - font.line_h) * 0.5).floor(), color);
    }

    /// A push button.
    #[allow(clippy::too_many_arguments)]
    pub fn button(&mut self, rect: [f32; 4], label: &str, hot: bool, down: bool, enabled: bool, accent: bool, on: bool) {
        let [x, y, w, h] = rect;
        let primary = accent || on;
        let fill = match (primary, down && enabled, hot && enabled) {
            (true, true, _) => c_accent_down(),
            (true, false, true) => c_accent_hot(),
            (true, _, _) => c_accent(),
            (false, true, _) => c_btn_down(),
            (false, false, true) => c_btn_hot(),
            _ => c_btn(),
        };
        let border = if primary { fill } else { c_border_soft() };
        let fill = if enabled { fill } else { mix(fill, c_bg(), 0.35) };
        self.r.rect(x, y, w, h, self.px(7.0), fill, 1.0, border);
        let color = if !enabled { c_dim() } else if primary { [1.0, 1.0, 1.0, 1.0] } else { c_text() };
        let font = &self.f.ui;
        let text = font.fit(label, w - self.px(10.0));
        let dy = if down && enabled { 1.0 } else { 0.0 };
        self.text_center(font, &text, x, y + dy, w, h, color);
    }

    /// A check mark made of two lines.
    pub fn check_mark(&mut self, x: f32, y: f32, size: f32, color: Color) {
        let t = (size * 0.14).max(1.5);
        self.r.line(x + size * 0.2, y + size * 0.54, x + size * 0.42, y + size * 0.76, t, color);
        self.r.line(x + size * 0.42, y + size * 0.76, x + size * 0.8, y + size * 0.26, t, color);
    }

    /// A small triangle: `open` points down, otherwise right.
    pub fn arrow(&mut self, cx: f32, cy: f32, size: f32, open: bool, color: Color) {
        let t = (size * 0.2).max(1.4);
        if open {
            self.r.line(cx - size * 0.5, cy - size * 0.22, cx, cy + size * 0.28, t, color);
            self.r.line(cx, cy + size * 0.28, cx + size * 0.5, cy - size * 0.22, t, color);
        } else {
            self.r.line(cx - size * 0.22, cy - size * 0.5, cx + size * 0.28, cy, t, color);
            self.r.line(cx + size * 0.28, cy, cx - size * 0.22, cy + size * 0.5, t, color);
        }
    }

    /// A thin vertical scroll bar thumb along the right edge of `track`.
    pub fn scrollbar(&mut self, thumb: [f32; 4], hot: bool) {
        let w = if hot { self.px(8.0) } else { self.px(5.0) };
        let x = thumb[0] + thumb[2] - w - self.px(2.0);
        self.r.rect(x, thumb[1], w, thumb[3], w * 0.5, alpha(c_text(), if hot { 0.34 } else { 0.2 }), 0.0, c_text());
    }

    /// A text-box frame (focus ring in the accent color).
    pub fn field_frame(&mut self, rect: [f32; 4], focused: bool, enabled: bool, bg: Color) {
        let [x, y, w, h] = rect;
        let border = if focused { c_accent() } else { c_border_soft() };
        self.r.rect(x, y, w, h, self.px(5.0), if enabled { bg } else { mix(bg, c_bg(), 0.4) }, 1.0, border);
    }
}
