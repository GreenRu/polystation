//! Text for the custom UI. A system font (Segoe UI, Consolas ...) is rasterised once by GDI into a
//! glyph atlas that lives on the GPU; after that every piece of text is just textured quads, so the
//! UI looks the same everywhere and costs almost nothing to draw.
#![allow(non_snake_case)]
use crate::gl::{self, Gl};
use std::collections::HashMap;
use std::ffi::c_void;

type HDC = isize;

#[repr(C)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_ppm: i32,
    y_ppm: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
#[derive(Default)]
struct TEXTMETRICW {
    tmHeight: i32,
    tmAscent: i32,
    tmDescent: i32,
    tmInternalLeading: i32,
    tmExternalLeading: i32,
    tmAveCharWidth: i32,
    tmMaxCharWidth: i32,
    tmWeight: i32,
    tmOverhang: i32,
    tmDigitizedAspectX: i32,
    tmDigitizedAspectY: i32,
    tmFirstChar: u16,
    tmLastChar: u16,
    tmDefaultChar: u16,
    tmBreakChar: u16,
    tmItalic: u8,
    tmUnderlined: u8,
    tmStruckOut: u8,
    tmPitchAndFamily: u8,
    tmCharSet: u8,
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: HDC) -> HDC;
    fn CreateDIBSection(dc: HDC, bmi: *const BITMAPINFOHEADER, usage: u32, bits: *mut *mut c_void, section: isize, offset: u32) -> isize;
    fn SelectObject(dc: HDC, obj: isize) -> isize;
    fn DeleteObject(obj: isize) -> i32;
    fn DeleteDC(dc: HDC) -> i32;
    fn CreateFontW(h: i32, w: i32, esc: i32, orient: i32, weight: i32, italic: u32, underline: u32, strike: u32, charset: u32, out: u32, clip: u32, quality: u32, pitch: u32, face: *const u16) -> isize;
    fn GetTextMetricsW(dc: HDC, tm: *mut TEXTMETRICW) -> i32;
    fn GetCharWidth32W(dc: HDC, first: u32, last: u32, buf: *mut i32) -> i32;
    fn TextOutW(dc: HDC, x: i32, y: i32, text: *const u16, n: i32) -> i32;
    fn SetTextColor(dc: HDC, color: u32) -> u32;
    fn SetBkColor(dc: HDC, color: u32) -> u32;
    fn GdiFlush() -> i32;
}

/// Characters in every atlas: printable ASCII plus a few symbols the UI uses.
fn charset() -> Vec<char> {
    let mut v: Vec<char> = (32u8..127).map(|b| b as char).collect();
    v.extend("…•°×–—·±²³½→←↑↓✓▸▾".chars());
    v
}

/// The rasterised font before it goes to the GPU (everything but the texture).
pub struct FontBitmap {
    pub w: usize,
    pub h: usize,
    /// One byte per pixel: glyph coverage.
    pub alpha: Vec<u8>,
    pub cell_w: usize,
    pub cell_h: usize,
    pub pad: usize,
    pub cols: usize,
    pub chars: Vec<(char, f32)>,
    pub line_h: f32,
    pub ascent: f32,
}

/// Draws every glyph of the font with GDI into a bitmap.
pub fn rasterize(face: &str, px: i32, bold: bool) -> FontBitmap {
    rasterize_with(face, px, bold, 4)
}

/// `quality` is the GDI font quality: 4 = anti-aliased, 3 = none (hard pixels, what bitmap fonts need).
pub fn rasterize_with(face: &str, px: i32, bold: bool, quality: u32) -> FontBitmap {
    let chars = charset();
    let pad = 2usize;
    unsafe {
        let dc = CreateCompatibleDC(0);
        let wide: Vec<u16> = face.encode_utf16().chain(Some(0)).collect();
        // ANTIALIASED_QUALITY: grey-scale smoothing (no colour fringes), DEFAULT_CHARSET
        let font = CreateFontW(-px, 0, 0, 0, if bold { 700 } else { 400 }, 0, 0, 0, 1, 0, 0, quality, 0, wide.as_ptr());
        let old_font = SelectObject(dc, font);
        let mut tm = TEXTMETRICW::default();
        GetTextMetricsW(dc, &mut tm);
        let line_h = tm.tmHeight.max(1) as usize;
        let mut advances = Vec::with_capacity(chars.len());
        for &c in &chars {
            let mut w = 0i32;
            GetCharWidth32W(dc, c as u32, c as u32, &mut w);
            advances.push(w.max(0) as usize);
        }
        let cell_w = advances.iter().copied().max().unwrap_or(8) + pad * 2;
        let cell_h = line_h + pad * 2;
        let cols = (1024 / cell_w).max(1);
        let rows = chars.len().div_ceil(cols);
        let (w, h) = (cols * cell_w, rows * cell_h);
        let info = BITMAPINFOHEADER { size: 40, width: w as i32, height: -(h as i32), planes: 1, bit_count: 32, compression: 0, size_image: 0, x_ppm: 0, y_ppm: 0, clr_used: 0, clr_important: 0 };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &info, 0, &mut bits, 0, 0);
        let old_bmp = SelectObject(dc, bmp);
        std::ptr::write_bytes(bits as *mut u8, 0, w * h * 4); // black
        SetTextColor(dc, 0x00FF_FFFF);
        SetBkColor(dc, 0);
        for (i, &c) in chars.iter().enumerate() {
            let (cx, cy) = ((i % cols) * cell_w, (i / cols) * cell_h);
            let mut buf = [0u16; 2];
            let s = c.encode_utf16(&mut buf);
            TextOutW(dc, (cx + pad) as i32, (cy + pad) as i32, s.as_ptr(), s.len() as i32);
        }
        GdiFlush();
        let px_data = std::slice::from_raw_parts(bits as *const u8, w * h * 4);
        let alpha: Vec<u8> = px_data.chunks_exact(4).map(|p| p[1]).collect();
        SelectObject(dc, old_bmp);
        SelectObject(dc, old_font);
        DeleteObject(bmp);
        DeleteObject(font);
        DeleteDC(dc);
        FontBitmap {
            w,
            h,
            alpha,
            cell_w,
            cell_h,
            pad,
            cols,
            chars: chars.iter().zip(&advances).map(|(&c, &a)| (c, a as f32)).collect(),
            line_h: line_h as f32,
            ascent: tm.tmAscent as f32,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Glyph {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub adv: f32,
}

pub struct Font {
    pub tex: u32,
    pub line_h: f32,
    pub ascent: f32,
    /// Size of one glyph cell (the quad drawn per character) and its padding around the glyph.
    pub cell_w: f32,
    pub cell_h: f32,
    pub pad: f32,
    glyphs: HashMap<char, Glyph>,
}

impl Font {
    /// The blocky monospace bitmap font of the game GUI: Windows' own raster font "Terminal" (8x12
    /// pixels per character), drawn without any smoothing and scaled only by whole numbers.
    pub fn new_pixel(g: &Gl) -> Font {
        Font::upload(g, rasterize_with("Terminal", 12, false, 3))
    }

    /// Rasterises the font and uploads it. `px` is the pixel height.
    pub fn new(g: &Gl, face: &str, px: i32, bold: bool) -> Font {
        Font::upload(g, rasterize(face, px, bold))
    }

    fn upload(g: &Gl, bm: FontBitmap) -> Font {
        let mut tex = 0;
        unsafe {
            (g.glGenTextures)(1, &mut tex);
            (g.glActiveTexture)(gl::TEXTURE0);
            (g.glBindTexture)(gl::TEXTURE_2D, tex);
            (g.glPixelStorei)(gl::UNPACK_ALIGNMENT, 1);
            (g.glTexImage2D)(gl::TEXTURE_2D, 0, gl::R8, bm.w as i32, bm.h as i32, 0, gl::RED, gl::UNSIGNED_BYTE, bm.alpha.as_ptr() as *const c_void);
            for (p, v) in [(gl::TEXTURE_MIN_FILTER, gl::NEAREST), (gl::TEXTURE_MAG_FILTER, gl::NEAREST), (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE)] {
                (g.glTexParameteri)(gl::TEXTURE_2D, p, v);
            }
            (g.glPixelStorei)(gl::UNPACK_ALIGNMENT, 4);
        }
        Font::from_bitmap(&bm, tex)
    }

    /// The font's metrics and glyph rectangles for an already uploaded atlas texture.
    pub fn from_bitmap(bm: &FontBitmap, tex: u32) -> Font {
        let mut glyphs = HashMap::new();
        for (i, &(c, adv)) in bm.chars.iter().enumerate() {
            let (cx, cy) = (((i % bm.cols) * bm.cell_w) as f32, ((i / bm.cols) * bm.cell_h) as f32);
            glyphs.insert(
                c,
                Glyph { u0: cx / bm.w as f32, v0: cy / bm.h as f32, u1: (cx + bm.cell_w as f32) / bm.w as f32, v1: (cy + bm.cell_h as f32) / bm.h as f32, adv },
            );
        }
        Font { tex, line_h: bm.line_h, ascent: bm.ascent, cell_w: bm.cell_w as f32, cell_h: bm.cell_h as f32, pad: bm.pad as f32, glyphs }
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    pub fn advance(&self, c: char) -> f32 {
        if c == '\t' {
            return self.advance(' ') * 4.0;
        }
        self.glyph(c).map_or(0.0, |g| g.adv)
    }

    /// Width of a string in pixels (no line breaks).
    pub fn measure(&self, s: &str) -> f32 {
        s.chars().map(|c| self.advance(c)).sum()
    }

    /// Splits text into lines of at most `width` pixels, at spaces where possible. Explicit line
    /// breaks are kept.
    pub fn wrap(&self, text: &str, width: f32) -> Vec<String> {
        let mut out = Vec::new();
        for para in text.replace("\r\n", "\n").split('\n') {
            let (mut line, mut line_w) = (String::new(), 0.0f32);
            for word in para.split(' ') {
                let w = self.measure(word);
                let space = if line.is_empty() { 0.0 } else { self.advance(' ') };
                if !line.is_empty() && line_w + space + w > width {
                    out.push(std::mem::take(&mut line));
                    line_w = 0.0;
                }
                if !line.is_empty() {
                    line.push(' ');
                    line_w += self.advance(' ');
                }
                line.push_str(word);
                line_w += w;
            }
            out.push(line);
        }
        out
    }

    /// The text cut to fit `width`, with an ellipsis when something was removed.
    pub fn fit(&self, s: &str, width: f32) -> String {
        if self.measure(s) <= width {
            return s.to_string();
        }
        let dots = self.advance('…').max(self.advance('.') * 3.0);
        let mut out = String::new();
        let mut w = 0.0;
        for c in s.chars() {
            let a = self.advance(c);
            if w + a + dots > width {
                break;
            }
            out.push(c);
            w += a;
        }
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_have_sensible_widths_and_ink() {
        let b = rasterize("Segoe UI", 16, false);
        let adv = |c: char| b.chars.iter().find(|(x, _)| *x == c).map(|(_, a)| *a).unwrap();
        assert!(adv('W') > adv('i') && adv('i') > 0.0 && adv(' ') > 0.0);
        assert!(b.line_h >= 16.0 && b.ascent > 8.0);
        // the letter 'A' left some ink in its cell
        let idx = b.chars.iter().position(|(c, _)| *c == 'A').unwrap();
        let (cx, cy) = ((idx % b.cols) * b.cell_w, (idx / b.cols) * b.cell_h);
        let ink: usize = (0..b.cell_h).flat_map(|y| (0..b.cell_w).map(move |x| (x, y))).filter(|&(x, y)| b.alpha[(cy + y) * b.w + cx + x] > 100).count();
        assert!(ink > 20, "ink = {ink}");
        // bold has more ink than regular
        let bold = rasterize("Segoe UI", 16, true);
        assert!(bold.alpha.iter().map(|&a| a as u64).sum::<u64>() > b.alpha.iter().map(|&a| a as u64).sum::<u64>());
    }

    #[test]
    fn measuring_wrapping_and_fitting() {
        let f = Font::from_bitmap(&rasterize("Segoe UI", 14, false), 0);
        assert!(f.measure("hello world") > f.measure("hello"));
        let lines = f.wrap("the quick brown fox jumps over the lazy dog", f.measure("the quick brown") + 1.0);
        assert!(lines.len() >= 3 && lines.iter().all(|l| f.measure(l) <= f.measure("the quick brown") + 1.0), "{lines:?}");
        assert_eq!(f.wrap("a
b", 500.0), vec!["a", "b"]);
        let fitted = f.fit("a very long label that will not fit", 60.0);
        assert!(fitted.ends_with('…') && f.measure(&fitted) <= 60.0, "{fitted}");
        assert_eq!(f.fit("short", 500.0), "short");
    }

    #[test]
    fn monospace_advances_are_equal() {
        let b = rasterize("Consolas", 14, false);
        let adv = |c: char| b.chars.iter().find(|(x, _)| *x == c).map(|(_, a)| *a).unwrap();
        assert_eq!(adv('i'), adv('W'));
    }
}
