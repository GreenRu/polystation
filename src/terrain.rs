//! Procedural terrain: a heightfield built from up to five stacked noise layers. Each layer has a
//! noise type, a seed, a scale (feature size) and a contribution (amplitude); heights are summed.
//! The terrain is generated in the unit square [-0.5, 0.5] on X/Z; the object's transform scales it.
use crate::obj::Face;
use std::f32::consts::TAU;

pub const MAX_LAYERS: usize = 5;
pub const MIN_RES: u32 = 2;
pub const MAX_RES: u32 = 256;
pub const MAX_VIEW_DIST: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    Perlin,
    Value,
    Ridged,
    Billow,
    Cellular,
}

impl NoiseKind {
    pub const ALL: [NoiseKind; 5] = [NoiseKind::Perlin, NoiseKind::Value, NoiseKind::Ridged, NoiseKind::Billow, NoiseKind::Cellular];
    pub const NAMES: [&'static str; 5] = ["Perlin", "Value", "Ridged", "Billow", "Cellular"];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&k| k == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> NoiseKind {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub kind: NoiseKind,
    pub seed: u32,
    /// Feature size as a fraction of the terrain width (smaller = finer detail).
    pub scale: f32,
    /// Contribution to the height, in units of terrain width.
    pub amp: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainParams {
    /// Quads per side (per chunk when `chunked`).
    pub res: u32,
    pub layers: Vec<Layer>,
    /// Endless terrain loaded in chunks around the camera instead of one fixed square. One chunk is one
    /// "terrain width" (the object's scale decides the world size); chunk (0, 0) is the fixed square.
    pub chunked: bool,
    /// Chunk render distance: chunks within this many chunks of the camera are loaded.
    pub view_dist: u32,
}

impl Default for TerrainParams {
    fn default() -> Self {
        TerrainParams {
            chunked: false,
            view_dist: 3,
            res: 32,
            layers: vec![
                Layer { kind: NoiseKind::Perlin, seed: 1, scale: 0.4, amp: 0.3 },
                Layer { kind: NoiseKind::Ridged, seed: 2, scale: 0.15, amp: 0.1 },
            ],
        }
    }
}

// ------------------------------------------------------------------ noise
pub(crate) fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9e37_79b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

fn rand01(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Smoothly interpolated random lattice values, in [-1, 1].
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (fade(x - x0), fade(y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let v = |dx: i32, dy: i32| rand01(hash2(ix + dx, iy + dy, seed));
    lerp(lerp(v(0, 0), v(1, 0), fx), lerp(v(0, 1), v(1, 1), fx), fy) * 2.0 - 1.0
}

/// Gradient (Perlin) noise, in [-1, 1].
fn perlin(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let g = |dx: i32, dy: i32, px: f32, py: f32| {
        let a = rand01(hash2(ix + dx, iy + dy, seed)) * TAU;
        a.cos() * px + a.sin() * py
    };
    let (u, v) = (fade(fx), fade(fy));
    let n = lerp(lerp(g(0, 0, fx, fy), g(1, 0, fx - 1.0, fy), u), lerp(g(0, 1, fx, fy - 1.0), g(1, 1, fx - 1.0, fy - 1.0), u), v);
    (n * std::f32::consts::SQRT_2).clamp(-1.0, 1.0)
}

/// Distance to the nearest jittered feature point, mapped so cell centres are high: [-1, 1].
fn cellular(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let mut d = f32::MAX;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (ix + dx, iy + dy);
            let px = cx as f32 + rand01(hash2(cx, cy, seed));
            let py = cy as f32 + rand01(hash2(cx, cy, seed ^ 0xA511_E9B3));
            d = d.min((x - px).hypot(y - py));
        }
    }
    1.0 - 2.0 * d.min(1.0)
}

/// One layer's contribution at terrain coordinates (u, v) in [0, 1].
pub fn layer_height(l: &Layer, u: f32, v: f32) -> f32 {
    // offset each seed to a different part of the noise field as well as re-hashing the lattice
    let (ox, oy) = ((l.seed as f32 * 12.9898) % 97.0, (l.seed as f32 * 78.233) % 89.0);
    let s = l.scale.max(0.005);
    let (x, y) = (u / s + ox, v / s + oy);
    let n = match l.kind {
        NoiseKind::Perlin => perlin(x, y, l.seed),
        NoiseKind::Value => value_noise(x, y, l.seed),
        NoiseKind::Ridged => 1.0 - 2.0 * perlin(x, y, l.seed).abs(),
        NoiseKind::Billow => 2.0 * perlin(x, y, l.seed).abs() - 1.0,
        NoiseKind::Cellular => cellular(x, y, l.seed),
    };
    n * l.amp
}

pub fn height(p: &TerrainParams, u: f32, v: f32) -> f32 {
    p.layers.iter().take(MAX_LAYERS).map(|l| layer_height(l, u, v)).sum()
}

/// Chunk containing the terrain-local point (x, z).
pub fn chunk_of(x: f32, z: f32) -> (i32, i32) {
    ((x + 0.5).floor() as i32, (z + 0.5).floor() as i32)
}

/// One chunk of the endless terrain; neighbouring chunks share their edge heights, so they join seamlessly.
pub fn chunk_faces(p: &TerrainParams, cx: i32, cz: i32) -> Vec<Face> {
    let n = p.res.clamp(MIN_RES, MAX_RES) as usize;
    let stride = n + 1;
    let mut pts = Vec::with_capacity(stride * stride);
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (cx as f32 + i as f32 / n as f32, cz as f32 + j as f32 / n as f32);
            pts.push(([u - 0.5, height(p, u, v), v - 0.5], [i as f32 / n as f32, j as f32 / n as f32]));
        }
    }
    let at = |i: usize, j: usize| pts[j * stride + i];
    let mut out = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            out.push(vec![at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    out
}

/// Coordinates of the chunks within the render distance of chunk (cx, cz): a rounded block.
pub fn chunk_window(p: &TerrainParams, cx: i32, cz: i32) -> Vec<(i32, i32)> {
    let r = p.view_dist.clamp(1, MAX_VIEW_DIST) as i32;
    let mut out = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            if dx * dx + dz * dz <= r * r + r {
                out.push((cx + dx, cz + dz));
            }
        }
    }
    out
}

/// The loaded chunks around chunk (cx, cz) as polygons.
pub fn faces_around(p: &TerrainParams, cx: i32, cz: i32) -> Vec<Face> {
    chunk_window(p, cx, cz).into_iter().flat_map(|(x, z)| chunk_faces(p, x, z)).collect()
}

/// The terrain surface as quads (counter-clockwise seen from above).
pub fn faces(p: &TerrainParams) -> Vec<Face> {
    let n = p.res.clamp(MIN_RES, MAX_RES) as usize;
    let stride = n + 1;
    let mut pts = Vec::with_capacity(stride * stride);
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            pts.push(([u - 0.5, height(p, u, v), v - 0.5], [u, v]));
        }
    }
    let at = |i: usize, j: usize| pts[j * stride + i];
    let mut out = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            out.push(vec![at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(kind: NoiseKind, seed: u32) -> Layer {
        Layer { kind, seed, scale: 0.2, amp: 1.0 }
    }

    #[test]
    fn deterministic_and_seed_dependent() {
        for kind in NoiseKind::ALL {
            let a = layer_height(&layer(kind, 5), 0.37, 0.61);
            assert_eq!(a, layer_height(&layer(kind, 5), 0.37, 0.61), "{kind:?} must be repeatable");
            let differs = (0..20).any(|i| {
                let (u, v) = (i as f32 * 0.047, i as f32 * 0.031);
                layer_height(&layer(kind, 5), u, v) != layer_height(&layer(kind, 6), u, v)
            });
            assert!(differs, "{kind:?}: another seed should give another terrain");
        }
    }

    #[test]
    fn noise_stays_in_range_and_varies() {
        for kind in NoiseKind::ALL {
            let l = layer(kind, 3);
            let vals: Vec<f32> = (0..400).map(|i| layer_height(&l, (i % 20) as f32 / 20.0, (i / 20) as f32 / 20.0)).collect();
            assert!(vals.iter().all(|v| v.is_finite() && v.abs() <= 1.0001), "{kind:?}");
            let (lo, hi) = vals.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            assert!(hi - lo > 0.2, "{kind:?} is nearly flat: {lo}..{hi}");
        }
    }

    #[test]
    fn contribution_scales_the_height() {
        let mut p = TerrainParams { res: 8, layers: vec![layer(NoiseKind::Perlin, 1)], ..Default::default() };
        let a = height(&p, 0.3, 0.3);
        p.layers[0].amp = 0.5;
        assert!((height(&p, 0.3, 0.3) - a * 0.5).abs() < 1e-6);
    }

    #[test]
    fn faces_cover_the_grid_and_face_up() {
        let p = TerrainParams { res: 6, layers: vec![], ..Default::default() }; // no layers: flat
        let f = faces(&p);
        assert_eq!(f.len(), 36);
        for face in &f {
            assert_eq!(face.len(), 4);
            let (mut n, k) = ([0.0f32; 3], face.len());
            for i in 0..k {
                let (a, b) = (face[i].0, face[(i + 1) % k].0);
                n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                n[2] += (a[0] - b[0]) * (a[1] + b[1]);
            }
            assert!(n[1] > 0.0, "terrain faces must point up, got {n:?}");
        }
    }

    #[test]
    fn resolution_is_clamped() {
        assert_eq!(faces(&TerrainParams { res: 0, layers: vec![], ..Default::default() }).len(), (MIN_RES * MIN_RES) as usize);
        assert_eq!(faces(&TerrainParams { res: 100_000, layers: vec![], ..Default::default() }).len(), (MAX_RES * MAX_RES) as usize);
    }

    #[test]
    fn chunks_join_seamlessly_and_follow_the_render_distance() {
        let p = TerrainParams { res: 8, chunked: true, view_dist: 2, ..Default::default() };
        // chunk (0,0) is the fixed terrain square, so the endless terrain agrees with it
        let fixed = faces(&p);
        let chunk = chunk_faces(&p, 0, 0);
        assert_eq!(fixed.len(), chunk.len());
        for (a, b) in fixed.iter().zip(&chunk) {
            for (pa, pb) in a.iter().zip(b) {
                assert!((pa.0[1] - pb.0[1]).abs() < 1e-6);
            }
        }
        // the right edge of chunk 0 meets the left edge of chunk 1 at the same heights
        let (c0, c1) = (chunk_faces(&p, 0, 0), chunk_faces(&p, 1, 0));
        let right_edge = |faces: &Vec<Face>, j: usize| faces[j * 8 + 7][3].0; // last quad of row j, its (i+1, j) corner
        let left_edge = |faces: &Vec<Face>, j: usize| faces[j * 8][0].0; // first quad of row j, its (i, j) corner
        for j in 0..8 {
            let (a, b) = (right_edge(&c0, j), left_edge(&c1, j));
            assert!((a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5 && (a[2] - b[2]).abs() < 1e-5);
        }
        // larger render distance -> more chunks
        let more = TerrainParams { view_dist: 4, ..p.clone() };
        assert!(faces_around(&more, 0, 0).len() > faces_around(&p, 0, 0).len());
        assert_eq!(chunk_of(0.2, -0.3), (0, 0));
        assert_eq!(chunk_of(0.6, 1.7), (1, 2));
        assert_eq!(chunk_of(-0.6, -0.5), (-1, 0));
    }
}
