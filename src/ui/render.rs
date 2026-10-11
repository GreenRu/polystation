//! The UI's 2D renderer. Everything the UI draws (rounded rectangles, circles, lines, gradients, soft
//! shadows, text) is a quad with a signed-distance-field fragment shader, collected into one vertex
//! buffer per frame and drawn in a few batches (a new batch starts when the clip rectangle or the
//! texture changes). Positions are in pixels with the origin at the top left.
use super::font::Font;
use crate::gl::{self, Gl};
use std::ffi::c_void;

pub type Color = [f32; 4];

const FLOATS: usize = 19;
/// Texture unit the UI uses, kept away from the units of the 3D scene.
const UNIT: u32 = gl::TEXTURE0 + 7;

const VS: &str = "#version 330 core
layout(location = 0) in vec2 pos;
layout(location = 1) in vec4 loc;
layout(location = 2) in vec2 rb;
layout(location = 3) in vec4 fill;
layout(location = 4) in vec4 bcol;
layout(location = 5) in vec3 uvm;
uniform vec2 screen;
out vec4 v_loc;
out vec2 v_rb;
out vec4 v_fill;
out vec4 v_bcol;
out vec3 v_uvm;
void main() {
    gl_Position = vec4(pos.x / screen.x * 2.0 - 1.0, 1.0 - pos.y / screen.y * 2.0, 0.0, 1.0);
    v_loc = loc; v_rb = rb; v_fill = fill; v_bcol = bcol; v_uvm = uvm;
}
";

const FS: &str = "#version 330 core
in vec4 v_loc;
in vec2 v_rb;
in vec4 v_fill;
in vec4 v_bcol;
in vec3 v_uvm;
uniform sampler2D tex;
out vec4 color;
float box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void main() {
    float mode = v_uvm.z;
    if (mode < 0.5) {
        float sd = box(v_loc.xy, v_loc.zw, v_rb.x);
        float a = clamp(0.5 - sd, 0.0, 1.0);
        vec4 c = v_fill;
        if (v_rb.y > 0.0) {
            float inner = clamp(0.5 - (sd + v_rb.y), 0.0, 1.0);
            c = vec4(mix(v_bcol.rgb, v_fill.rgb, inner), mix(v_bcol.a, v_fill.a, inner));
        }
        color = vec4(c.rgb, c.a * a);
    } else if (mode < 1.5) {
        color = vec4(v_fill.rgb, v_fill.a * texture(tex, v_uvm.xy).r);
    } else if (mode < 2.5) {
        vec4 t = texture(tex, v_uvm.xy);
        color = vec4(t.rgb * v_fill.rgb, v_fill.a);
    } else {
        float sd = box(v_loc.xy, v_loc.zw, v_rb.x);
        float k = clamp(1.0 - sd / max(v_rb.y, 1.0), 0.0, 1.0);
        color = vec4(v_fill.rgb, v_fill.a * k * k);
    }
}
";

struct Batch {
    first: i32,
    count: i32,
    clip: [i32; 4],
    tex: u32,
}

pub struct Renderer {
    g: Gl,
    prog: u32,
    vao: u32,
    vbo: u32,
    loc_screen: i32,
    verts: Vec<f32>,
    batches: Vec<Batch>,
    clips: Vec<[i32; 4]>,
    clip: [i32; 4],
    tex: u32,
    pub screen: (i32, i32),
    snap_tex: u32,
    snap_size: (i32, i32),
}

impl Renderer {
    pub fn new(g: Gl) -> Renderer {
        unsafe {
            let prog = g.compile_program(VS, FS).unwrap_or_else(|e| panic!("{e}"));
            let (mut vao, mut vbo, mut snap) = (0, 0, 0);
            (g.glGenVertexArrays)(1, &mut vao);
            (g.glGenBuffers)(1, &mut vbo);
            (g.glGenTextures)(1, &mut snap);
            (g.glBindVertexArray)(vao);
            (g.glBindBuffer)(gl::ARRAY_BUFFER, vbo);
            let stride = (FLOATS * 4) as i32;
            for (loc, size, offset) in [(0u32, 2, 0usize), (1, 4, 2), (2, 2, 6), (3, 4, 8), (4, 4, 12), (5, 3, 16)] {
                (g.glEnableVertexAttribArray)(loc);
                (g.glVertexAttribPointer)(loc, size, gl::FLOAT, 0, stride, (offset * 4) as *const c_void);
            }
            (g.glUseProgram)(prog);
            let loc_screen = (g.glGetUniformLocation)(prog, b"screen\0".as_ptr());
            (g.glUniform1i)((g.glGetUniformLocation)(prog, b"tex\0".as_ptr()), 7);
            Renderer { g, prog, vao, vbo, loc_screen, verts: Vec::new(), batches: Vec::new(), clips: Vec::new(), clip: [0, 0, 1, 1], tex: 0, screen: (1, 1), snap_tex: snap, snap_size: (0, 0) }
        }
    }

    /// Starts a frame for a window of w x h pixels.
    pub fn begin(&mut self, w: i32, h: i32) {
        self.screen = (w, h);
        self.verts.clear();
        self.batches.clear();
        self.clips.clear();
        self.clip = [0, 0, w, h];
        self.tex = 0;
    }

    /// Size of the window in pixels, as of the last `begin`.
    pub fn screen(&self) -> (f32, f32) {
        (self.screen.0 as f32, self.screen.1 as f32)
    }

    /// Restricts drawing to a rectangle (intersected with the current one) until `pop_clip`.
    pub fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.clips.push(self.clip);
        let (x0, y0) = ((x.floor() as i32).max(self.clip[0]), (y.floor() as i32).max(self.clip[1]));
        let (x1, y1) = (((x + w).ceil() as i32).min(self.clip[0] + self.clip[2]), ((y + h).ceil() as i32).min(self.clip[1] + self.clip[3]));
        self.clip = [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)];
    }

    pub fn pop_clip(&mut self) {
        if let Some(c) = self.clips.pop() {
            self.clip = c;
        }
    }

    pub fn clip_rect(&self) -> [i32; 4] {
        self.clip
    }

    fn vertex(&mut self, v: [f32; FLOATS]) {
        self.verts.extend_from_slice(&v);
    }

    fn quad(&mut self, p: [[f32; 2]; 4], loc: [[f32; 2]; 4], half: [f32; 2], rb: [f32; 2], fills: [Color; 4], bcol: Color, uv: [[f32; 2]; 4], mode: f32, tex: u32) {
        let count = (self.verts.len() / FLOATS) as i32;
        match self.batches.last_mut() {
            Some(b) if b.clip == self.clip && b.tex == tex && b.first + b.count == count => b.count += 6,
            _ => self.batches.push(Batch { first: count, count: 6, clip: self.clip, tex }),
        }
        for &i in &[0usize, 1, 2, 0, 2, 3] {
            let (f, uvi) = (fills[i], uv[i]);
            self.vertex([p[i][0], p[i][1], loc[i][0], loc[i][1], half[0], half[1], rb[0], rb[1], f[0], f[1], f[2], f[3], bcol[0], bcol[1], bcol[2], bcol[3], uvi[0], uvi[1], mode]);
        }
    }

    /// A rounded rectangle with an optional border (drawn inside the edge).
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, fill: Color, border: f32, border_color: Color) {
        if w <= 0.0 || h <= 0.0 || (fill[3] <= 0.0 && border <= 0.0) {
            return;
        }
        let (hx, hy, e) = (w * 0.5, h * 0.5, 1.0);
        let r = radius.min(hx).min(hy);
        let (l, t, rr, b) = (-hx - e, -hy - e, hx + e, hy + e);
        let (cx, cy) = (x + hx, y + hy);
        self.quad(
            [[cx + l, cy + t], [cx + rr, cy + t], [cx + rr, cy + b], [cx + l, cy + b]],
            [[l, t], [rr, t], [rr, b], [l, b]],
            [hx, hy],
            [r, border],
            [fill; 4],
            border_color,
            [[0.0; 2]; 4],
            0.0,
            0,
        );
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.rect(x, y, w, h, 0.0, color, 0.0, color);
    }

    /// A rectangle whose corners have different colors (top-left, top-right, bottom-right, bottom-left).
    pub fn gradient(&mut self, x: f32, y: f32, w: f32, h: f32, c: [Color; 4]) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (hx, hy) = (w * 0.5, h * 0.5);
        self.quad([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], [[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]], [hx + 1.0, hy + 1.0], [0.0, 0.0], c, [0.0; 4], [[0.0; 2]; 4], 0.0, 0);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Color, border: f32, border_color: Color) {
        self.rect(cx - r, cy - r, r * 2.0, r * 2.0, r, fill, border, border_color);
    }

    /// A line with round ends.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Color) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt();
        let (ax, ay) = if len < 1e-4 { (1.0, 0.0) } else { (dx / len, dy / len) };
        let (nx, ny) = (-ay, ax);
        let (hx, hy) = (len * 0.5 + width * 0.5, width * 0.5);
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (ex, ey) = (hx + 1.0, hy + 1.0);
        let corner = |sx: f32, sy: f32| [cx + ax * ex * sx + nx * ey * sy, cy + ay * ex * sx + ny * ey * sy];
        self.quad(
            [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)],
            [[-ex, -ey], [ex, -ey], [ex, ey], [-ex, ey]],
            [hx, hy],
            [hy, 0.0],
            [color; 4],
            color,
            [[0.0; 2]; 4],
            0.0,
            0,
        );
    }

    /// A soft shadow under a rounded rectangle.
    pub fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, color: Color) {
        let (hx, hy) = (w * 0.5, h * 0.5);
        let (ex, ey) = (hx + blur, hy + blur);
        let (cx, cy) = (x + hx, y + hy);
        self.quad(
            [[cx - ex, cy - ey], [cx + ex, cy - ey], [cx + ex, cy + ey], [cx - ex, cy + ey]],
            [[-ex, -ey], [ex, -ey], [ex, ey], [-ex, ey]],
            [hx, hy],
            [radius, blur],
            [color; 4],
            color,
            [[0.0; 2]; 4],
            3.0,
            0,
        );
    }

    /// Text with every pixel of the font made `k` x `k` screen pixels (a pixel font stays crisp).
    pub fn text_scaled(&mut self, font: &Font, s: &str, x: f32, y: f32, color: Color, k: f32) -> f32 {
        let k = k.round().max(1.0);
        let mut pen = x.round();
        let top = y.round() - font.pad * k;
        for c in s.chars() {
            let adv = font.advance(c) * k;
            if c != ' ' && c != '\t' && c != '\n' && c != '\r' {
                if let Some(g) = font.glyph(c) {
                    let left = pen - font.pad * k;
                    let (w, h) = (font.cell_w * k, font.cell_h * k);
                    self.quad([[left, top], [left + w, top], [left + w, top + h], [left, top + h]], [[0.0; 2]; 4], [0.0, 0.0], [0.0, 0.0], [color; 4], color, [[g.u0, g.v0], [g.u1, g.v0], [g.u1, g.v1], [g.u0, g.v1]], 1.0, font.tex);
                }
            }
            pen += adv;
        }
        pen - x.round()
    }

    /// Draws text with its top-left corner at (x, y); returns its width.
    pub fn text(&mut self, font: &Font, s: &str, x: f32, y: f32, color: Color) -> f32 {
        let mut pen = x.round();
        let top = y.round() - font.pad;
        for c in s.chars() {
            let adv = font.advance(c);
            if c != ' ' && c != '\t' && c != '\n' && c != '\r' {
                if let Some(g) = font.glyph(c) {
                    let left = pen - font.pad;
                    self.quad(
                        [[left, top], [left + font.cell_w, top], [left + font.cell_w, top + font.cell_h], [left, top + font.cell_h]],
                        [[0.0; 2]; 4],
                        [0.0, 0.0],
                        [0.0, 0.0],
                        [color; 4],
                        color,
                        [[g.u0, g.v0], [g.u1, g.v0], [g.u1, g.v1], [g.u0, g.v1]],
                        1.0,
                        font.tex,
                    );
                }
            }
            pen += adv;
        }
        pen - x.round()
    }

    /// An image (the saved frame) stretched over a rectangle; `flip` turns it upside down (GL's origin).
    pub fn image(&mut self, tex: u32, x: f32, y: f32, w: f32, h: f32, flip: bool, tint: Color) {
        let (v0, v1) = if flip { (1.0, 0.0) } else { (0.0, 1.0) };
        self.quad([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], [[0.0; 2]; 4], [0.0, 0.0], [0.0, 0.0], [tint; 4], tint, [[0.0, v0], [1.0, v0], [1.0, v1], [0.0, v1]], 2.0, tex);
    }

    /// The texture holding the last saved frame (see [`Renderer::snapshot`]).
    pub fn snapshot_texture(&self) -> u32 {
        self.snap_tex
    }

    /// Copies what is in the back buffer into a texture (for dialogs to draw over).
    pub fn snapshot(&mut self, w: i32, h: i32) {
        unsafe {
            (self.g.glActiveTexture)(UNIT);
            (self.g.glBindTexture)(gl::TEXTURE_2D, self.snap_tex);
            if self.snap_size != (w, h) {
                (self.g.glTexImage2D)(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, w, h, 0, gl::RGBA, gl::UNSIGNED_BYTE, std::ptr::null());
                for (p, v) in [(gl::TEXTURE_MIN_FILTER, gl::LINEAR), (gl::TEXTURE_MAG_FILTER, gl::LINEAR), (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE)] {
                    (self.g.glTexParameteri)(gl::TEXTURE_2D, p, v);
                }
                self.snap_size = (w, h);
            }
            (self.g.glCopyTexSubImage2D)(gl::TEXTURE_2D, 0, 0, 0, 0, 0, w, h);
            (self.g.glActiveTexture)(gl::TEXTURE0);
        }
    }

    /// Uploads the frame's geometry and draws it.
    pub fn end(&mut self) {
        if self.verts.is_empty() {
            return;
        }
        let g = &self.g;
        unsafe {
            (g.glViewport)(0, 0, self.screen.0, self.screen.1);
            (g.glDisable)(gl::DEPTH_TEST);
            (g.glDisable)(gl::CULL_FACE);
            (g.glEnable)(gl::BLEND);
            (g.glBlendFunc)(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            (g.glEnable)(gl::SCISSOR_TEST);
            (g.glUseProgram)(self.prog);
            (g.glUniform2f)(self.loc_screen, self.screen.0 as f32, self.screen.1 as f32);
            (g.glBindVertexArray)(self.vao);
            (g.glBindBuffer)(gl::ARRAY_BUFFER, self.vbo);
            (g.glBufferData)(gl::ARRAY_BUFFER, (self.verts.len() * 4) as isize, self.verts.as_ptr() as *const c_void, gl::DYNAMIC_DRAW);
            (g.glActiveTexture)(UNIT);
            let mut bound = u32::MAX;
            for b in &self.batches {
                if b.clip[2] <= 0 || b.clip[3] <= 0 {
                    continue;
                }
                let tex = if b.tex == 0 { self.snap_tex } else { b.tex };
                if tex != bound {
                    (g.glBindTexture)(gl::TEXTURE_2D, tex);
                    bound = tex;
                }
                // GL's scissor origin is the bottom left
                (g.glScissor)(b.clip[0], self.screen.1 - b.clip[1] - b.clip[3], b.clip[2], b.clip[3]);
                (g.glDrawArrays)(gl::TRIANGLES, b.first, b.count);
            }
            (g.glActiveTexture)(gl::TEXTURE0);
            (g.glDisable)(gl::SCISSOR_TEST);
            (g.glDisable)(gl::BLEND);
            (g.glEnable)(gl::DEPTH_TEST);
        }
    }
}

/// Color helpers.
pub fn rgb(hex: u32) -> Color {
    [((hex >> 16) & 0xFF) as f32 / 255.0, ((hex >> 8) & 0xFF) as f32 / 255.0, (hex & 0xFF) as f32 / 255.0, 1.0]
}

pub fn alpha(c: Color, a: f32) -> Color {
    [c[0], c[1], c[2], c[3] * a]
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}
