//! The Polystation logo: 47 x 47 pixels, drawn as blocks so it stays sharp at any size.
use super::style::Ctx;

/// RGBA, top row first.
const PIXELS: &[u8] = include_bytes!("../../assets/logo47.rgba");
pub const SIZE: usize = 47;

/// Draws the logo with its top left corner at (x, y); every logo pixel is a `k` x `k` block.
pub fn draw(c: &mut Ctx, x: f32, y: f32, k: f32) {
    for row in 0..SIZE {
        let px = |col: usize| &PIXELS[(row * SIZE + col) * 4..(row * SIZE + col) * 4 + 4];
        let mut col = 0;
        while col < SIZE {
            let p = px(col);
            if p[3] < 8 {
                col += 1;
                continue;
            }
            let start = col;
            while col < SIZE && px(col) == p {
                col += 1;
            }
            let color = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0];
            c.r.fill(x + start as f32 * k, y + row as f32 * k, (col - start) as f32 * k, k, color);
        }
    }
}
