//! Image loading: PNG, JPEG (baseline), TGA and BMP, written from scratch (no libraries). Every loader
//! returns an `Atlas` (RGBA, rows bottom-up like OpenGL wants them). Used for textures, sky panoramas and
//! GUI images; scripts can ask for an image by path too.
use crate::atlas::Atlas;
use std::path::Path;

pub fn supported(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "bmp" | "png" | "jpg" | "jpeg" | "tga"))
}

/// Loads an image file by its extension.
pub fn load(path: &str) -> Result<Atlas, String> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "bmp" {
        return Atlas::load_bmp(path);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let r = match ext.as_str() {
        "png" => decode_png(&bytes),
        "jpg" | "jpeg" => decode_jpeg(&bytes),
        "tga" => decode_tga(&bytes),
        _ => Err("unsupported image type".to_string()),
    };
    r.map_err(|e| format!("{path}: {e}"))
}

/// Top-down RGBA rows -> an Atlas (bottom-up).
fn atlas_from_top_down(w: usize, h: usize, rgba: Vec<u8>) -> Result<Atlas, String> {
    if w == 0 || h == 0 || w > 16384 || h > 16384 || rgba.len() != w * h * 4 {
        return Err("bad image size".into());
    }
    let mut out = vec![0u8; rgba.len()];
    for y in 0..h {
        out[(h - 1 - y) * w * 4..(h - y) * w * 4].copy_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }
    Ok(Atlas { w, h, rgba: out })
}

// ------------------------------------------------------------------------------------------------
// inflate (RFC 1951), as in zlib's "puff"
// ------------------------------------------------------------------------------------------------
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    bit: u32,
    cnt: u32,
}

impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<(), String> {
        while self.cnt < n {
            let b = *self.d.get(self.pos).ok_or("the compressed data ends too early")?;
            self.pos += 1;
            self.bit |= (b as u32) << self.cnt;
            self.cnt += 8;
        }
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.cnt -= n;
        Ok(v)
    }
}

struct Huff {
    count: [u16; 16],
    symbol: Vec<u16>,
}

fn huff(lengths: &[u8]) -> Huff {
    let mut count = [0u16; 16];
    for &l in lengths {
        count[l as usize] += 1;
    }
    let mut offs = [0u16; 16];
    for i in 1..15 {
        offs[i + 1] = offs[i] + count[i];
    }
    let mut symbol = vec![0u16; lengths.len()];
    for (s, &l) in lengths.iter().enumerate() {
        if l != 0 {
            symbol[offs[l as usize] as usize] = s as u16;
            offs[l as usize] += 1;
        }
    }
    Huff { count, symbol }
}

fn decode_sym(b: &mut Bits, h: &Huff) -> Result<u16, String> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    for len in 1..16 {
        code |= b.bits(1)? as i32;
        let count = h.count[len] as i32;
        if code - count < first {
            return Ok(h.symbol[(index + (code - first)) as usize]);
        }
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err("bad compressed data (code)".into())
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff, limit: usize) -> Result<(), String> {
    loop {
        let s = decode_sym(b, lit)? as usize;
        if s < 256 {
            out.push(s as u8);
        } else if s == 256 {
            return Ok(());
        } else {
            let s = s - 257;
            if s >= 29 {
                return Err("bad compressed data (length)".into());
            }
            let len = LBASE[s] as usize + b.bits(LEXT[s] as u32)? as usize;
            let ds = decode_sym(b, dist)? as usize;
            if ds >= 30 {
                return Err("bad compressed data (distance)".into());
            }
            let d = DBASE[ds] as usize + b.bits(DEXT[ds] as u32)? as usize;
            if d > out.len() {
                return Err("bad compressed data (distance too far)".into());
            }
            for _ in 0..len {
                out.push(out[out.len() - d]);
            }
        }
        if out.len() > limit {
            return Err("the image is bigger than it says".into());
        }
    }
}

/// Inflates a zlib stream. `limit` stops runaway data.
pub fn zlib_inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if data.len() < 2 || data[0] & 0x0F != 8 {
        return Err("not a zlib stream".into());
    }
    let mut b = Bits { d: data, pos: 2, bit: 0, cnt: 0 };
    let mut out = Vec::new();
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.bit = 0;
                b.cnt = 0;
                let p = b.pos;
                if p + 4 > data.len() {
                    return Err("truncated stored block".into());
                }
                let len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
                b.pos = p + 4;
                if b.pos + len > data.len() {
                    return Err("truncated stored block".into());
                }
                out.extend_from_slice(&data[b.pos..b.pos + len]);
                b.pos += len;
            }
            1 => {
                let mut l = [0u8; 288];
                for (i, v) in l.iter_mut().enumerate() {
                    *v = if i < 144 { 8 } else if i < 256 { 9 } else if i < 280 { 7 } else { 8 };
                }
                codes(&mut b, &mut out, &huff(&l), &huff(&[5u8; 30]), limit)?;
            }
            2 => {
                let (nlen, ndist, ncode) = (b.bits(5)? as usize + 257, b.bits(5)? as usize + 1, b.bits(4)? as usize + 4);
                if nlen > 286 || ndist > 30 {
                    return Err("bad compressed data (counts)".into());
                }
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut lengths = [0u8; 320];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = b.bits(3)? as u8;
                }
                let lencode = huff(&lengths[..19]);
                let mut idx = 0;
                let mut all = [0u8; 320];
                while idx < nlen + ndist {
                    let sym = decode_sym(&mut b, &lencode)?;
                    if sym < 16 {
                        all[idx] = sym as u8;
                        idx += 1;
                    } else {
                        let (prev, rep) = match sym {
                            16 => {
                                if idx == 0 {
                                    return Err("bad compressed data (repeat)".into());
                                }
                                (all[idx - 1], 3 + b.bits(2)? as usize)
                            }
                            17 => (0, 3 + b.bits(3)? as usize),
                            _ => (0, 11 + b.bits(7)? as usize),
                        };
                        if idx + rep > nlen + ndist {
                            return Err("bad compressed data (repeat too long)".into());
                        }
                        for _ in 0..rep {
                            all[idx] = prev;
                            idx += 1;
                        }
                    }
                }
                codes(&mut b, &mut out, &huff(&all[..nlen]), &huff(&all[nlen..nlen + ndist]), limit)?;
            }
            _ => return Err("bad compressed data (block type)".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

// ------------------------------------------------------------------------------------------------
// PNG
// ------------------------------------------------------------------------------------------------
fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

pub fn decode_png(d: &[u8]) -> Result<Atlas, String> {
    if d.len() < 33 || &d[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG file".into());
    }
    let (mut pos, mut hdr, mut plte, mut trns, mut idat) = (8usize, None, Vec::new(), Vec::new(), Vec::new());
    while pos + 8 <= d.len() {
        let len = u32::from_be_bytes(d[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &d[pos + 4..pos + 8];
        let body = d.get(pos + 8..pos + 8 + len).ok_or("truncated PNG")?;
        match kind {
            b"IHDR" if len >= 13 => hdr = Some((u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize, u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize, body[8], body[9], body[12])),
            b"PLTE" => plte = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let (w, h, depth, ctype, interlace) = hdr.ok_or("the PNG has no header")?;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err("bad PNG size".into());
    }
    let channels = match ctype {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err("unknown PNG color type".into()),
    };
    if !matches!((ctype, depth), (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) | (2 | 4 | 6, 8 | 16)) {
        return Err("unsupported PNG bit depth".into());
    }
    let bits_pp = channels * depth as usize;
    let bpp = bits_pp.div_ceil(8);
    let limit = (w * bits_pp).div_ceil(8) * h * 2 + h * 16 + 1024;
    let raw = zlib_inflate(&idat, limit)?;
    let mut rgba = vec![0u8; w * h * 4];

    // the passes: (x0, y0, dx, dy); a plain image is one pass
    let passes: &[(usize, usize, usize, usize)] = if interlace == 1 { &[(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)] } else { &[(0, 0, 1, 1)] };
    let mut at = 0usize;
    for &(x0, y0, dx, dy) in passes {
        let pw = if x0 >= w { 0 } else { (w - x0).div_ceil(dx) };
        let ph = if y0 >= h { 0 } else { (h - y0).div_ceil(dy) };
        if pw == 0 || ph == 0 {
            continue;
        }
        let stride = (pw * bits_pp).div_ceil(8);
        let mut prev = vec![0u8; stride];
        for py in 0..ph {
            let row_end = at + 1 + stride;
            let src = raw.get(at..row_end).ok_or("the PNG data is cut short")?;
            let filter = src[0];
            let mut cur = src[1..].to_vec();
            for i in 0..stride {
                let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
                let b = prev[i] as i32;
                let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                let add = match filter {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => (a + b) / 2,
                    4 => paeth(a, b, c),
                    _ => return Err("bad PNG filter".into()),
                };
                cur[i] = cur[i].wrapping_add(add as u8);
            }
            for px in 0..pw {
                let sample = |ch: usize| -> u16 {
                    let bit = (px * channels + ch) * depth as usize;
                    match depth {
                        16 => u16::from_be_bytes([cur[bit / 8], cur[bit / 8 + 1]]),
                        8 => cur[bit / 8] as u16,
                        n => ((cur[bit / 8] >> (8 - n as usize - bit % 8)) & ((1u8 << n) - 1)) as u16,
                    }
                };
                let to8 = |v: u16| -> u8 { if depth == 16 { (v >> 8) as u8 } else if depth == 8 { v as u8 } else { (v as u32 * 255 / ((1u32 << depth) - 1)) as u8 } };
                let (r, g, b, a) = match ctype {
                    0 => {
                        let v = sample(0);
                        let a = if trns.len() >= 2 && v == u16::from_be_bytes([trns[0], trns[1]]) { 0 } else { 255 };
                        (to8(v), to8(v), to8(v), a)
                    }
                    2 => {
                        let (r, g, b) = (sample(0), sample(1), sample(2));
                        let a = if trns.len() >= 6 && r == u16::from_be_bytes([trns[0], trns[1]]) && g == u16::from_be_bytes([trns[2], trns[3]]) && b == u16::from_be_bytes([trns[4], trns[5]]) { 0 } else { 255 };
                        (to8(r), to8(g), to8(b), a)
                    }
                    3 => {
                        let i = sample(0) as usize;
                        let p = plte.get(i * 3..i * 3 + 3).ok_or("bad PNG palette index")?;
                        (p[0], p[1], p[2], trns.get(i).copied().unwrap_or(255))
                    }
                    4 => (to8(sample(0)), to8(sample(0)), to8(sample(0)), to8(sample(1))),
                    _ => (to8(sample(0)), to8(sample(1)), to8(sample(2)), to8(sample(3))),
                };
                let (x, y) = (x0 + px * dx, y0 + py * dy);
                let o = (y * w + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[r, g, b, a]);
            }
            prev = cur;
            at = row_end;
        }
    }
    atlas_from_top_down(w, h, rgba)
}

// ------------------------------------------------------------------------------------------------
// TGA
// ------------------------------------------------------------------------------------------------
pub fn decode_tga(d: &[u8]) -> Result<Atlas, String> {
    if d.len() < 18 {
        return Err("not a TGA file".into());
    }
    let (id_len, cmap_type, ty) = (d[0] as usize, d[1], d[2]);
    let (cmap_first, cmap_len, cmap_bits) = (u16::from_le_bytes([d[3], d[4]]) as usize, u16::from_le_bytes([d[5], d[6]]) as usize, d[7] as usize);
    let (w, h, bpp, desc) = (u16::from_le_bytes([d[12], d[13]]) as usize, u16::from_le_bytes([d[14], d[15]]) as usize, d[16] as usize, d[17]);
    if w == 0 || h == 0 || !matches!(ty, 1 | 2 | 3 | 9 | 10 | 11) {
        return Err("unsupported TGA type".into());
    }
    let mut pos = 18 + id_len;
    let cmap_bytes = (cmap_bits / 8).max(1);
    let palette = d.get(pos..pos + cmap_len * cmap_bytes).filter(|_| cmap_type == 1).map(|p| p.to_vec());
    if cmap_type == 1 {
        pos += cmap_len * cmap_bytes;
    }
    let bytes = (bpp / 8).max(1);
    let rle = ty >= 9;
    let mut raw = Vec::with_capacity(w * h * bytes);
    if rle {
        while raw.len() < w * h * bytes {
            let head = *d.get(pos).ok_or("the TGA data is cut short")?;
            pos += 1;
            let n = (head & 0x7F) as usize + 1;
            if head & 0x80 != 0 {
                let px = d.get(pos..pos + bytes).ok_or("the TGA data is cut short")?;
                pos += bytes;
                for _ in 0..n {
                    raw.extend_from_slice(px);
                }
            } else {
                let run = d.get(pos..pos + n * bytes).ok_or("the TGA data is cut short")?;
                pos += n * bytes;
                raw.extend_from_slice(run);
            }
        }
    } else {
        raw.extend_from_slice(d.get(pos..pos + w * h * bytes).ok_or("the TGA data is cut short")?);
    }
    let mut rgba = vec![255u8; w * h * 4];
    let from_pixel = |p: &[u8], bits: usize| -> [u8; 4] {
        match bits {
            32 => [p[2], p[1], p[0], p[3]],
            24 => [p[2], p[1], p[0], 255],
            16 | 15 => {
                let v = u16::from_le_bytes([p[0], p[1]]);
                let e = |s: u16| (((v >> s) & 31) as u32 * 255 / 31) as u8;
                [e(10), e(5), e(0), 255]
            }
            _ => [p[0], p[0], p[0], 255],
        }
    };
    for i in 0..w * h {
        let p = &raw[i * bytes..(i + 1) * bytes];
        let px = if matches!(ty, 1 | 9) {
            let idx = (if bytes == 2 { u16::from_le_bytes([p[0], p[1]]) as usize } else { p[0] as usize }).saturating_sub(cmap_first);
            let pal = palette.as_ref().ok_or("the TGA has no palette")?;
            let e = pal.get(idx * cmap_bytes..idx * cmap_bytes + cmap_bytes).ok_or("bad TGA palette index")?;
            from_pixel(e, cmap_bits)
        } else if matches!(ty, 3 | 11) && bpp == 16 {
            [p[0], p[0], p[0], p[1]] // gray + alpha
        } else {
            from_pixel(p, bpp)
        };
        // TGA is bottom-up unless bit 5 of the descriptor is set; x may be mirrored with bit 4
        let (x, y) = (i % w, i / w);
        let x = if desc & 0x10 != 0 { w - 1 - x } else { x };
        let y = if desc & 0x20 != 0 { y } else { h - 1 - y };
        rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px);
    }
    atlas_from_top_down(w, h, rgba)
}

// ------------------------------------------------------------------------------------------------
// JPEG (baseline and extended sequential, Huffman coded)
// ------------------------------------------------------------------------------------------------
const ZIGZAG: [usize; 64] = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63];

struct JTable {
    /// code lengths 1..16: first code, count, index offset
    min: [i32; 17],
    max: [i32; 18],
    off: [i32; 17],
    vals: Vec<u8>,
}

fn jtable(counts: &[u8], vals: &[u8]) -> JTable {
    let (mut min, mut max, mut off) = ([0i32; 17], [-1i32; 18], [0i32; 17]);
    let (mut code, mut k) = (0i32, 0i32);
    for l in 1..=16 {
        let n = counts[l - 1] as i32;
        min[l] = code;
        off[l] = k - code;
        code += n;
        k += n;
        max[l] = if n > 0 { code - 1 } else { -1 };
        code <<= 1;
    }
    JTable { min, max, off, vals: vals.to_vec() }
}

struct JBits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u32,
    cnt: u32,
}

impl JBits<'_> {
    fn bit(&mut self) -> Result<u32, String> {
        if self.cnt == 0 {
            let mut b = *self.d.get(self.pos).ok_or("the JPEG data ends too early")?;
            self.pos += 1;
            if b == 0xFF {
                let n = *self.d.get(self.pos).unwrap_or(&0);
                if n == 0 {
                    self.pos += 1;
                } else if (0xD0..=0xD7).contains(&n) {
                    // a restart marker where data was expected: treat as zero bits
                    b = 0;
                } else {
                    b = 0;
                    self.pos -= 1;
                }
            }
            self.acc = b as u32;
            self.cnt = 8;
        }
        self.cnt -= 1;
        Ok((self.acc >> self.cnt) & 1)
    }

    fn receive(&mut self, n: u32) -> Result<i32, String> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()? as i32;
        }
        Ok(v)
    }

    fn decode(&mut self, t: &JTable) -> Result<u8, String> {
        let mut code = 0i32;
        for l in 1..=16 {
            code = (code << 1) | self.bit()? as i32;
            if t.max[l] >= 0 && code <= t.max[l] && code >= t.min[l] {
                return t.vals.get((code + t.off[l]) as usize).copied().ok_or_else(|| "bad JPEG code".to_string());
            }
        }
        Err("bad JPEG code".into())
    }
}

fn extend(v: i32, n: u32) -> i32 {
    if n == 0 { 0 } else if v < (1 << (n - 1)) { v - (1 << n) + 1 } else { v }
}

fn idct(block: &mut [f32; 64]) {
    // separable float IDCT
    let mut tmp = [0f32; 64];
    let c = |u: usize| if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
    let mut cos = [[0f32; 8]; 8];
    for x in 0..8 {
        for u in 0..8 {
            cos[x][u] = c(u) * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos() * 0.5;
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            tmp[y * 8 + x] = (0..8).map(|u| cos[x][u] * block[y * 8 + u]).sum();
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            block[y * 8 + x] = (0..8).map(|v| cos[y][v] * tmp[v * 8 + x]).sum();
        }
    }
}

pub fn decode_jpeg(d: &[u8]) -> Result<Atlas, String> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return Err("not a JPEG file".into());
    }
    let mut qt = [[1u16; 64]; 4];
    let (mut dc, mut ac): (Vec<Option<JTable>>, Vec<Option<JTable>>) = ((0..4).map(|_| None).collect(), (0..4).map(|_| None).collect());
    struct Comp {
        id: u8,
        h: usize,
        v: usize,
        tq: usize,
        td: usize,
        ta: usize,
        data: Vec<f32>,
        stride: usize,
        pred: i32,
    }
    let (mut comps, mut w, mut h, mut restart) = (Vec::<Comp>::new(), 0usize, 0usize, 0usize);
    let mut pos = 2;
    let mut scan_start = None;
    while pos + 4 <= d.len() {
        if d[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let m = d[pos + 1];
        if m == 0xFF || m == 0x00 || m == 0x01 || (0xD0..=0xD8).contains(&m) {
            pos += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([d[pos + 2], d[pos + 3]]) as usize;
        let seg = d.get(pos + 4..pos + 2 + len).ok_or("truncated JPEG")?;
        match m {
            0xDB => {
                let mut i = 0;
                while i < seg.len() {
                    let (prec, id) = (seg[i] >> 4, (seg[i] & 15) as usize);
                    i += 1;
                    for k in 0..64 {
                        let v = if prec == 0 { *seg.get(i + k).ok_or("bad JPEG tables")? as u16 } else { u16::from_be_bytes([*seg.get(i + 2 * k).ok_or("bad JPEG tables")?, seg[i + 2 * k + 1]]) };
                        if id < 4 {
                            qt[id][ZIGZAG[k]] = v;
                        }
                    }
                    i += if prec == 0 { 64 } else { 128 };
                }
            }
            0xC4 => {
                let mut i = 0;
                while i + 17 <= seg.len() {
                    let (class, id) = (seg[i] >> 4, (seg[i] & 15) as usize);
                    let counts = &seg[i + 1..i + 17];
                    let total: usize = counts.iter().map(|&c| c as usize).sum();
                    let vals = seg.get(i + 17..i + 17 + total).ok_or("bad JPEG tables")?;
                    let t = jtable(counts, vals);
                    if id < 4 {
                        if class == 0 { dc[id] = Some(t) } else { ac[id] = Some(t) }
                    }
                    i += 17 + total;
                }
            }
            0xC0 | 0xC1 => {
                if seg.len() < 6 || seg[0] != 8 {
                    return Err("only 8-bit JPEGs are supported".into());
                }
                (h, w) = (u16::from_be_bytes([seg[1], seg[2]]) as usize, u16::from_be_bytes([seg[3], seg[4]]) as usize);
                for c in 0..seg[5] as usize {
                    let s = seg.get(6 + c * 3..9 + c * 3).ok_or("bad JPEG frame")?;
                    comps.push(Comp { id: s[0], h: (s[1] >> 4).max(1) as usize, v: (s[1] & 15).max(1) as usize, tq: (s[2] & 3) as usize, td: 0, ta: 0, data: Vec::new(), stride: 0, pred: 0 });
                }
            }
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return Err("only baseline JPEGs are supported (progressive files are not): re-save it as a baseline JPEG or a PNG".into()),
            0xDD => restart = u16::from_be_bytes([seg[0], seg[1]]) as usize,
            0xDA => {
                let n = seg[0] as usize;
                for i in 0..n {
                    let (cid, t) = (seg[1 + i * 2], seg[2 + i * 2]);
                    if let Some(c) = comps.iter_mut().find(|c| c.id == cid) {
                        (c.td, c.ta) = ((t >> 4) as usize & 3, (t & 15) as usize & 3);
                    }
                }
                scan_start = Some(pos + 2 + len);
                break;
            }
            _ => {}
        }
        pos += 2 + len;
    }
    let start = scan_start.ok_or("the JPEG has no image data")?;
    if comps.is_empty() || w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err("bad JPEG size".into());
    }
    let (hmax, vmax) = (comps.iter().map(|c| c.h).max().unwrap_or(1), comps.iter().map(|c| c.v).max().unwrap_or(1));
    let (mcux, mcuy) = (w.div_ceil(8 * hmax), h.div_ceil(8 * vmax));
    for c in &mut comps {
        c.stride = mcux * c.h * 8;
        c.data = vec![0.0; c.stride * mcuy * c.v * 8];
    }
    let mut jb = JBits { d, pos: start, acc: 0, cnt: 0 };
    let mut count = 0usize;
    for my in 0..mcuy {
        for mx in 0..mcux {
            if restart > 0 && count > 0 && count % restart == 0 {
                jb.cnt = 0;
                // skip to and past the restart marker
                while jb.pos + 1 < d.len() && !(d[jb.pos] == 0xFF && (0xD0..=0xD7).contains(&d[jb.pos + 1])) {
                    jb.pos += 1;
                }
                jb.pos += 2;
                for c in &mut comps {
                    c.pred = 0;
                }
            }
            count += 1;
            for ci in 0..comps.len() {
                let (ch, cv, tq, td, ta) = (comps[ci].h, comps[ci].v, comps[ci].tq, comps[ci].td, comps[ci].ta);
                for by in 0..cv {
                    for bx in 0..ch {
                        let mut blk = [0f32; 64];
                        let dct = dc[td].as_ref().ok_or("missing JPEG table")?;
                        let act = ac[ta].as_ref().ok_or("missing JPEG table")?;
                        let t = jb.decode(dct)? as u32;
                        let diff = extend(jb.receive(t)?, t);
                        comps[ci].pred += diff;
                        blk[0] = (comps[ci].pred * qt[tq][0] as i32) as f32;
                        let mut k = 1;
                        while k < 64 {
                            let rs = jb.decode(act)?;
                            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
                            if s == 0 {
                                if r == 15 {
                                    k += 16;
                                    continue;
                                }
                                break;
                            }
                            k += r;
                            if k > 63 {
                                break;
                            }
                            let v = extend(jb.receive(s)?, s);
                            blk[ZIGZAG[k]] = (v * qt[tq][ZIGZAG[k]] as i32) as f32;
                            k += 1;
                        }
                        idct(&mut blk);
                        let c = &mut comps[ci];
                        let (ox, oy) = ((mx * c.h + bx) * 8, (my * c.v + by) * 8);
                        for yy in 0..8 {
                            for xx in 0..8 {
                                c.data[(oy + yy) * c.stride + ox + xx] = blk[yy * 8 + xx] + 128.0;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut rgba = vec![255u8; w * h * 4];
    let sample = |c: &Comp, x: usize, y: usize| -> f32 { c.data[(y * c.v / vmax) * c.stride + x * c.h / hmax] };
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            let clamp = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            if comps.len() >= 3 {
                let (yy, cb, cr) = (sample(&comps[0], x, y), sample(&comps[1], x, y) - 128.0, sample(&comps[2], x, y) - 128.0);
                rgba[o] = clamp(yy + 1.402 * cr);
                rgba[o + 1] = clamp(yy - 0.344136 * cb - 0.714136 * cr);
                rgba[o + 2] = clamp(yy + 1.772 * cb);
            } else {
                let v = clamp(sample(&comps[0], x, y));
                rgba[o..o + 3].copy_from_slice(&[v, v, v]);
            }
        }
    }
    atlas_from_top_down(w, h, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored-block zlib stream around `raw` (no compression), to build test PNGs without a compressor.
    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        let mut z = vec![0x78, 0x01];
        let mut chunks = raw.chunks(65535).peekable();
        while let Some(c) = chunks.next() {
            z.push(chunks.peek().is_none() as u8);
            z.extend_from_slice(&(c.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
            z.extend_from_slice(c);
        }
        z.extend_from_slice(&[0, 0, 0, 0]); // (the checksum is not checked)
        z
    }

    fn png(w: u32, h: u32, depth: u8, ctype: u8, raw: &[u8], extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |k: &[u8; 4], body: &[u8]| {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(k);
            out.extend_from_slice(body);
            out.extend_from_slice(&[0; 4]);
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[depth, ctype, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        for (k, b) in extra {
            chunk(k, b);
        }
        chunk(b"IDAT", &zlib_stored(raw));
        chunk(b"IEND", &[]);
        out
    }

    #[test]
    fn inflate_handles_stored_and_a_real_fixed_huffman_stream() {
        assert_eq!(zlib_inflate(&zlib_stored(b"hello"), 100).unwrap(), b"hello");
        // real zlib output: a fixed-Huffman block, and a dynamic-Huffman block (text repeated + all byte values x3)
        let z = [120, 218, 75, 76, 132, 129, 36, 56, 0, 0, 79, 221, 7, 159];
        assert_eq!(zlib_inflate(&z, 100).unwrap(), b"aaaaaaaaaabbbbbbbbbb".to_vec());
        let hex = "78da2bc94855282ccd4cce56482aca2fcf5348cbaf50c82acd2d2856c82f4b2d5228014ae72456552aa4e4a7eb8179a38a47158f2aa6aa6206462666165636760e4e2e6e1e5e3e7e0141216111513171094929691959397905452565155535750d4d2d6d1d5d3d7d03432363135333730b4b2b6b1b5b3b7b07472767175737770f4f2f6f1f5f3fff80c0a0e090d0b0f088c8a8e898d8b8f884c4a4e494d4b4f48cccacec9cdcbcfc82c2a2e292d2b2f28acaaaea9adabafa86c6a6e696d6b6f68eceaeee9edebefe0913274d9e3275daf4193367cd9e3377defc050b172d5eb274d9f2152b57ad5eb376ddfa0d1b376ddeb275dbf61d3b77eddeb377dffe03070f1d3e72f4d8f113274f9d3e73f6dcf90b172f5dbe72f5daf51b376fddbe73f7defd070f1f3d7ef2f4d9f3172f5fbd7ef3f6ddfb0f1f3f7dfef2f5dbf71f3f7ffdfef3f7dfff51ff8f6cff030062d1c42b";
        let dynamic: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let mut want = b"the quick brown fox jumps over the lazy dog. ".repeat(20);
        for _ in 0..3 {
            want.extend(0..=255u8);
        }
        assert_eq!(zlib_inflate(&dynamic, 100_000).unwrap(), want);
        assert!(zlib_inflate(b"junk", 10).is_err());
    }

    #[test]
    fn png_rgba_gray_and_palette_decode_and_flip_to_bottom_up() {
        // 2x2 RGBA: top row red, green; bottom row blue, white (semi transparent)
        let raw = [0, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255, 0, 0, 255, 255, 255, 255, 128];
        let a = decode_png(&png(2, 2, 8, 6, &raw, &[])).unwrap();
        assert_eq!((a.w, a.h), (2, 2));
        assert_eq!(&a.rgba[0..4], &[0, 0, 255, 0], "row 0 is the bottom row");
        assert_eq!(&a.rgba[8..12], &[255, 0, 0, 255], "the top row comes last");
        // 4x1 gray, 2 bits per pixel: 0 1 2 3
        let g = decode_png(&png(4, 1, 2, 0, &[0, 0b0001_1011], &[])).unwrap();
        assert_eq!(g.rgba.iter().step_by(4).copied().collect::<Vec<_>>(), vec![0, 85, 170, 255]);
        // palette with a transparent first entry
        let p = decode_png(&png(2, 1, 8, 3, &[0, 0, 1], &[(b"PLTE", vec![10, 20, 30, 40, 50, 60]), (b"tRNS", vec![0])])).unwrap();
        assert_eq!(&p.rgba[0..8], &[10, 20, 30, 0, 40, 50, 60, 255]);
    }

    #[test]
    fn png_filters_are_undone() {
        // 3x2 gray 8-bit: row 0 "sub" filtered 10 10 10 -> 10 20 30; row 1 "up" filtered 1 1 1 -> 11 21 31
        let raw = [1, 10, 10, 10, 2, 1, 1, 1];
        let a = decode_png(&png(3, 2, 8, 0, &raw, &[])).unwrap();
        let top: Vec<u8> = (0..3).map(|x| a.rgba[(3 + x) * 4]).collect(); // the bottom-up atlas stores the top row last
        let bottom: Vec<u8> = (0..3).map(|x| a.rgba[x * 4]).collect();
        assert_eq!(top, vec![10, 20, 30]);
        assert_eq!(bottom, vec![11, 21, 31]);
    }

    #[test]
    fn tga_truecolor_with_and_without_rle() {
        let mut t = vec![0u8; 18];
        (t[2], t[12], t[14], t[16], t[17]) = (2, 2, 2, 24, 0);
        // bottom-up: first row is the bottom row: blue, green; then red, white
        t.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        let a = decode_tga(&t).unwrap();
        assert_eq!(&a.rgba[0..8], &[0, 0, 255, 255, 0, 255, 0, 255]);
        let mut r = vec![0u8; 18];
        (r[2], r[12], r[14], r[16], r[17]) = (10, 2, 1, 24, 0x20);
        r.extend_from_slice(&[0x81, 9, 8, 7]); // a run of 2 identical pixels
        let a = decode_tga(&r).unwrap();
        assert_eq!(&a.rgba[0..8], &[7, 8, 9, 255, 7, 8, 9, 255]);
    }

    #[test]
    fn bad_files_are_errors_not_crashes() {
        assert!(decode_png(b"nope").is_err());
        assert!(decode_png(&png(2, 2, 8, 6, &[0; 3], &[])).is_err(), "too little data");
        assert!(decode_jpeg(b"\xFF\xD8\xFF\xD9").is_err());
        assert!(decode_tga(&[0; 18]).is_err());
        assert!(!supported("x.psd") && supported("A.PNG") && supported("b.jpeg"));
    }

    #[test]
    fn a_tiny_baseline_jpeg_decodes_to_the_right_colour() {
        // 8x8 solid gray (value 128) baseline JPEG, 1 component, standard tables (hand-assembled)
        let mut j: Vec<u8> = vec![0xFF, 0xD8];
        j.extend_from_slice(&[0xFF, 0xDB, 0, 67, 0]);
        j.extend_from_slice(&[1u8; 64]);
        j.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        // DC table: one code of length 1 -> category 0; AC table: one code of length 1 -> EOB (0x00)
        j.extend_from_slice(&[0xFF, 0xC4, 0, 20, 0x00, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        j.extend_from_slice(&[0xFF, 0xC4, 0, 20, 0x10, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        j.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0x00, 0, 63, 0]);
        j.push(0); // DC code "0" then AC code "0" -> bits 00 padded with 1s: 0b0011_1111
        let n = j.len();
        j[n - 1] = 0b0011_1111;
        j.extend_from_slice(&[0xFF, 0xD9]);
        let a = decode_jpeg(&j).unwrap();
        assert_eq!((a.w, a.h), (8, 8));
        assert!(a.rgba.chunks(4).all(|p| p[0] == 128 && p[1] == 128 && p[2] == 128 && p[3] == 255), "{:?}", &a.rgba[..8]);
    }
}
