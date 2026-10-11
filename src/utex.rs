//! Packs the textures that were assigned to objects (and particle systems) into one RGBA atlas, so
//! the shader can read any of them with a single texture binding. Rows are bottom-up like the BMP
//! loader's output, so a texture's first row is its v = 0.
use crate::atlas::Atlas;
use std::collections::HashMap;

const ATLAS_W: usize = 2048;
const MAX_H: usize = 4096;

/// Shrinks (nearest neighbour) so neither side exceeds `cap`.
fn fit(a: &Atlas, cap: usize) -> Atlas {
    if a.w <= cap && a.h <= cap {
        return a.clone();
    }
    let k = cap as f32 / a.w.max(a.h) as f32;
    let (w, h) = (((a.w as f32 * k) as usize).max(1), ((a.h as f32 * k) as usize).max(1));
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x * a.w / w, y * a.h / h);
            let (s, d) = ((sy * a.w + sx) * 4, (y * w + x) * 4);
            rgba[d..d + 4].copy_from_slice(&a.rgba[s..s + 4]);
        }
    }
    Atlas { w, h, rgba }
}

/// Returns the combined atlas and, for each input name, its rectangle (x, y, w, h) in pixels.
pub fn pack(images: &[(String, Atlas)]) -> (Atlas, HashMap<String, [f32; 4]>) {
    if images.is_empty() {
        return (Atlas { w: 1, h: 1, rgba: vec![255; 4] }, HashMap::new());
    }
    let mut cap = 1024usize;
    loop {
        let tiles: Vec<Atlas> = images.iter().map(|(_, a)| fit(a, cap)).collect();
        let mut order: Vec<usize> = (0..tiles.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(tiles[i].h));
        let mut pos = vec![(0usize, 0usize); tiles.len()];
        let (mut x, mut y, mut row_h) = (0usize, 0usize, 0usize);
        for &i in &order {
            let t = &tiles[i];
            if x + t.w > ATLAS_W {
                x = 0;
                y += row_h;
                row_h = 0;
            }
            pos[i] = (x, y);
            x += t.w;
            row_h = row_h.max(t.h);
        }
        let height = (y + row_h).max(1);
        if height > MAX_H && cap > 16 {
            cap /= 2; // too tall: shrink every texture and try again
            continue;
        }
        let mut rgba = vec![0u8; ATLAS_W * height * 4];
        let mut rects = HashMap::new();
        for (i, t) in tiles.iter().enumerate() {
            let (px, py) = pos[i];
            for row in 0..t.h.min(height.saturating_sub(py)) {
                let d = ((py + row) * ATLAS_W + px) * 4;
                rgba[d..d + t.w * 4].copy_from_slice(&t.rgba[row * t.w * 4..(row + 1) * t.w * 4]);
            }
            rects.insert(images[i].0.clone(), [px as f32, py as f32, t.w as f32, t.h as f32]);
        }
        return (Atlas { w: ATLAS_W, h: height, rgba }, rects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, c: [u8; 4]) -> Atlas {
        Atlas { w, h, rgba: c.iter().copied().cycle().take(w * h * 4).collect() }
    }

    #[test]
    fn rectangles_hold_the_right_pixels_and_do_not_overlap() {
        let imgs = vec![("a".to_string(), solid(4, 4, [255, 0, 0, 255])), ("b".to_string(), solid(10, 3, [0, 255, 0, 255])), ("c".to_string(), solid(2, 8, [0, 0, 255, 255]))];
        let (atlas, rects) = pack(&imgs);
        assert_eq!(atlas.w, ATLAS_W);
        for (name, color) in [("a", [255, 0, 0]), ("b", [0, 255, 0]), ("c", [0, 0, 255])] {
            let r = rects[name];
            let (x, y) = (r[0] as usize, r[1] as usize);
            for (dx, dy) in [(0, 0), (r[2] as usize - 1, r[3] as usize - 1)] {
                let i = ((y + dy) * atlas.w + x + dx) * 4;
                assert_eq!(&atlas.rgba[i..i + 3], &color, "{name}");
            }
        }
        let overlap = |p: [f32; 4], q: [f32; 4]| p[0] < q[0] + q[2] && q[0] < p[0] + p[2] && p[1] < q[1] + q[3] && q[1] < p[1] + p[3];
        assert!(!overlap(rects["a"], rects["b"]) && !overlap(rects["a"], rects["c"]) && !overlap(rects["b"], rects["c"]));
    }

    #[test]
    fn huge_textures_are_shrunk_to_fit() {
        let (atlas, rects) = pack(&[("big".to_string(), solid(3000, 3000, [9, 9, 9, 255]))]);
        let r = rects["big"];
        assert!(r[2] <= 1024.0 && r[3] <= 1024.0, "{r:?}");
        assert!(atlas.h <= MAX_H);
    }

    #[test]
    fn nothing_packs_to_a_dummy_atlas() {
        let (a, r) = pack(&[]);
        assert_eq!((a.w, a.h), (1, 1));
        assert!(r.is_empty());
    }
}
