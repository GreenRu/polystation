//! Texture atlas: a procedural 2x2 atlas, or an uncompressed 24/32-bit .bmp supplied by the user.
//! Rows are stored bottom-up so that row 0 is v = 0, matching OBJ and OpenGL conventions.

#[derive(Clone)]
pub struct Atlas {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Atlas {
    /// 256x256 atlas of four 128x128 tiles: checker, bricks, stripes, rings.
    /// Tile k occupies u in [kx/2, kx/2+0.5], v in [ky/2, ky/2+0.5] with kx = k%2, ky = k/2.
    pub fn procedural() -> Self {
        let (w, h, t) = (256usize, 256usize, 128usize);
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (tile, lx, ly) = ((y / t) * 2 + x / t, x % t, y % t);
                let c: [u8; 3] = match tile {
                    0 => if (lx / 16 + ly / 16) % 2 == 0 { [235, 235, 235] } else { [190, 50, 40] },
                    1 => {
                        let row = ly / 16;
                        let off = if row % 2 == 0 { 0 } else { 16 };
                        if ly % 16 < 2 || (lx + off) % 32 < 2 { [210, 205, 190] } else { [150, 70, 50] }
                    }
                    2 => if ((lx + ly) / 12) % 2 == 0 { [250, 200, 40] } else { [30, 30, 30] },
                    _ => {
                        let (dx, dy) = (lx as f32 - 63.5, ly as f32 - 63.5);
                        let r = (dx * dx + dy * dy).sqrt();
                        if (r / 8.0) as i32 % 2 == 0 { [40, 120, 200] } else { [230, 240, 250] }
                    }
                };
                let i = (y * w + x) * 4;
                rgba[i..i + 3].copy_from_slice(&c);
            }
        }
        Self { w, h, rgba }
    }

    pub fn load_bmp(path: &str) -> Result<Self, String> {
        let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
        let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        if b.len() < 54 || &b[0..2] != b"BM" {
            return Err("not a BMP file".into());
        }
        let off = u32at(10) as usize;
        let w = u32at(18) as i32;
        let hraw = u32at(22) as i32;
        let (bpp, comp) = (u16at(28), u32at(30));
        if comp != 0 || (bpp != 24 && bpp != 32) || w <= 0 {
            return Err("only uncompressed 24/32-bit BMP is supported".into());
        }
        let (w, h, top_down) = (w as usize, hraw.unsigned_abs() as usize, hraw < 0);
        let bytes = bpp / 8;
        let stride = (w * bytes + 3) & !3;
        if b.len() < off + stride * h {
            return Err("truncated BMP".into());
        }
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            let src_row = if top_down { h - 1 - y } else { y };
            for x in 0..w {
                let s = off + src_row * stride + x * bytes;
                let d = (y * w + x) * 4;
                rgba[d] = b[s + 2];
                rgba[d + 1] = b[s + 1];
                rgba[d + 2] = b[s];
                if bytes == 4 {
                    rgba[d + 3] = b[s + 3];
                }
            }
        }
        Ok(Self { w, h, rgba })
    }
}

/// Writes a 32-bit BMP. `rgba` rows are bottom-up (OpenGL order), which is also BMP's order.
pub fn write_bmp(path: &str, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    let mut data = Vec::with_capacity(54 + rgba.len());
    data.extend_from_slice(b"BM");
    data.extend_from_slice(&((54 + rgba.len()) as u32).to_le_bytes());
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(&54u32.to_le_bytes());
    data.extend_from_slice(&40u32.to_le_bytes());
    data.extend_from_slice(&(w as i32).to_le_bytes());
    data.extend_from_slice(&(h as i32).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(&[0; 4]); // BI_RGB
    data.extend_from_slice(&(rgba.len() as u32).to_le_bytes());
    data.extend_from_slice(&[0x13, 0x0B, 0, 0, 0x13, 0x0B, 0, 0]); // 2835 px/m
    data.extend_from_slice(&[0; 8]);
    for px in rgba.chunks_exact(4) {
        data.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    std::fs::write(path, data).map_err(|e| format!("{path}: {e}"))
}
