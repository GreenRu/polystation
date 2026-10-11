//! Procedural trees: the Tree object, and the tree shapes scatter layers can plant.
//!
//! Every tree is built unit-tall, standing on y = 0, from a few tubes, cones and blobs. The colors come
//! from a small built-in palette texture (`builtin:trees`): the faces' UVs point into the palette's
//! columns (bark, birch bark, leaves ...), so a tree is brown and green without any texture work.
use crate::atlas::Atlas;
use crate::obj::Face;
use crate::prims::{poly, sphere};
use crate::scatter::{scaled, shifted};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TreeKind {
    Pine,
    #[default]
    Oak,
    Birch,
    Palm,
    Dead,
}

impl TreeKind {
    pub const NAMES: [&'static str; 5] = ["Pine", "Oak", "Birch", "Palm", "Dead tree"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> TreeKind {
        [TreeKind::Pine, TreeKind::Oak, TreeKind::Birch, TreeKind::Palm, TreeKind::Dead][i.min(4)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeParams {
    pub kind: TreeKind,
    /// How much of the height is bare trunk (0.1 .. 0.7).
    pub trunk: f32,
    /// Trunk thickness as a fraction of the height.
    pub trunk_width: f32,
    /// Width of the crown as a fraction of the height.
    pub crown: f32,
    /// Pine: number of layers. Oak / birch: number of leaf clumps. Palm: number of fronds. Dead: branches.
    pub detail: u32,
    /// Another seed gives another tree of the same kind.
    pub seed: u32,
}

impl Default for TreeParams {
    fn default() -> Self {
        TreeParams { kind: TreeKind::Oak, trunk: 0.4, trunk_width: 0.07, crown: 0.55, detail: 5, seed: 1 }
    }
}

impl TreeParams {
    pub fn sanitize(&mut self) {
        self.trunk = self.trunk.clamp(0.1, 0.7);
        self.trunk_width = self.trunk_width.clamp(0.02, 0.2);
        self.crown = self.crown.clamp(0.2, 1.0);
        self.detail = self.detail.clamp(2, 8);
    }
}

/// Palette columns (the faces' UV `u` points at one of them).
const BARK: usize = 0;
const BIRCH_BARK: usize = 1;
const OAK_LEAF: usize = 2;
const PINE_LEAF: usize = 3;
const PALM_LEAF: usize = 4;
const BIRCH_LEAF: usize = 5;
const DEAD_WOOD: usize = 6;
const COLUMNS: usize = 8;

/// The name under which the palette is used as a texture.
pub const PALETTE: &str = "builtin:trees";

fn col_u(c: usize) -> f32 {
    (c as f32 + 0.5) / COLUMNS as f32
}

/// The palette texture: one column per material, dark at the bottom and light at the top, with a
/// little dither noise so it does not look flat.
pub fn palette() -> Atlas {
    const COLORS: [([u8; 3], [u8; 3]); COLUMNS] = [
        ([88, 56, 32], [124, 84, 52]),
        ([196, 192, 182], [240, 236, 226]),
        ([34, 98, 36], [100, 178, 62]),
        ([20, 70, 44], [52, 128, 74]),
        ([62, 130, 46], [136, 196, 72]),
        ([104, 142, 38], [196, 206, 72]),
        ([88, 80, 72], [136, 124, 110]),
        ([168, 66, 20], [244, 176, 40]),
    ];
    let (w, h, cw) = (COLUMNS * 8, 16usize, 8usize);
    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (lo, hi) = COLORS[x / cw];
            let t = y as f32 / (h - 1) as f32;
            let noise = ((x * 7 + y * 13) % 5) as f32 - 2.0;
            let i = (y * w + x) * 4;
            for k in 0..3 {
                rgba[i + k] = (lo[k] as f32 + (hi[k] as f32 - lo[k] as f32) * t + noise * 3.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
    Atlas { w, h, rgba }
}

fn rnd(seed: u32, i: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B1).wrapping_add(i.wrapping_mul(0x85EB_CA6B)).wrapping_add(0x1234_5678);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / 16_777_216.0
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// A tube (a cone when r1 = 0) from p0 to p1 with its UVs inside palette column `col`, v from v0 to v1.
#[allow(clippy::too_many_arguments)]
fn tube(p0: [f32; 3], p1: [f32; 3], r0: f32, r1: f32, sides: usize, col: usize, v0: f32, v1: f32) -> Vec<Face> {
    let axis = norm(sub(p1, p0));
    let helper = if axis[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let a = norm(cross(axis, helper));
    let b = cross(axis, a);
    let ring = |c: [f32; 3], r: f32, k: usize| {
        let t = TAU * k as f32 / sides as f32;
        [c[0] + (a[0] * t.cos() + b[0] * t.sin()) * r, c[1] + (a[1] * t.cos() + b[1] * t.sin()) * r, c[2] + (a[2] * t.cos() + b[2] * t.sin()) * r]
    };
    let mid = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5, (p0[2] + p1[2]) * 0.5];
    let u = col_u(col);
    let mut out = Vec::new();
    for k in 0..sides {
        let (a0, a1) = (ring(p0, r0, k), ring(p0, r0, k + 1));
        if r1 <= 1e-5 {
            out.push(poly(&[(a0, [u, v0]), (a1, [u, v0]), (p1, [u, v1])], mid));
        } else {
            let (b0, b1) = (ring(p1, r1, k), ring(p1, r1, k + 1));
            out.push(poly(&[(a0, [u, v0]), (a1, [u, v0]), (b1, [u, v1]), (b0, [u, v1])], mid));
        }
    }
    out
}

/// A rough low-poly ball.
fn blob(center: [f32; 3], rx: f32, ry: f32, rz: f32, col: usize, v0: f32, v1: f32) -> Vec<Face> {
    let u = col_u(col);
    let faces: Vec<Face> = sphere(4, 7)
        .into_iter()
        .map(|f| f.into_iter().map(|(p, _)| (p, [u, v0 + (v1 - v0) * (p[1] + 0.5)])).collect())
        .collect();
    shifted(scaled(faces, [rx * 2.0, ry * 2.0, rz * 2.0]), center)
}

/// A flat leaf strip (both sides visible).
#[allow(clippy::too_many_arguments)]
fn strip(p0: [f32; 3], p1: [f32; 3], w0: f32, w1: f32, col: usize, v0: f32, v1: f32, out: &mut Vec<Face>) {
    let d = norm(sub(p1, p0));
    let side = norm(cross(d, [0.0, 1.0, 0.0]));
    let u = col_u(col);
    let pts = [
        ([p0[0] - side[0] * w0, p0[1] - side[1] * w0, p0[2] - side[2] * w0], [u, v0]),
        ([p0[0] + side[0] * w0, p0[1] + side[1] * w0, p0[2] + side[2] * w0], [u, v0]),
        ([p1[0] + side[0] * w1, p1[1] + side[1] * w1, p1[2] + side[2] * w1], [u, v1]),
        ([p1[0] - side[0] * w1, p1[1] - side[1] * w1, p1[2] - side[2] * w1], [u, v1]),
    ];
    let front: Face = pts.to_vec();
    let back: Face = pts.iter().rev().copied().collect();
    out.push(front);
    out.push(back);
}

/// The tree, one unit tall, standing on y = 0 around the y axis.
pub fn faces(p: &TreeParams) -> Vec<Face> {
    let mut p = *p;
    p.sanitize();
    let (th, tw, crown, n, seed) = (p.trunk, p.trunk_width, p.crown, p.detail as usize, p.seed);
    let mut f = Vec::new();
    match p.kind {
        TreeKind::Pine => {
            let bottom = th * 0.8;
            f.extend(tube([0.0, 0.0, 0.0], [0.0, bottom + (1.0 - bottom) * 0.5, 0.0], tw * 0.6, tw * 0.3, 7, BARK, 0.1, 0.8));
            let span = 1.0 - bottom;
            for i in 0..n {
                let t = i as f32 / n as f32;
                let r = crown * 0.5 * (1.0 - t * 0.78);
                let base = bottom + span * t * 0.8;
                let top = if i + 1 == n { 1.0 } else { base + span / n as f32 * 1.7 };
                f.extend(tube([0.0, base, 0.0], [0.0, top, 0.0], r, 0.0, 9, PINE_LEAF, 0.15 + 0.15 * (1.0 - t), 0.45 + 0.5 * t));
            }
        }
        TreeKind::Oak => {
            let lean = [(rnd(seed, 90) - 0.5) * 0.06, (rnd(seed, 91) - 0.5) * 0.06];
            f.extend(tube([0.0, 0.0, 0.0], [lean[0], th + (1.0 - th) * 0.35, lean[1]], tw * 0.7, tw * 0.4, 7, BARK, 0.1, 0.9));
            let room = 1.0 - th;
            for i in 0..n {
                let ang = TAU * i as f32 / n as f32 + rnd(seed, 1) * TAU;
                let d = crown * (0.12 + 0.14 * rnd(seed, 10 + i as u32));
                let r = crown * (0.2 + 0.14 * rnd(seed, 20 + i as u32));
                let ry = r.min(room * 0.34);
                let y = th + room * (0.3 + 0.4 * rnd(seed, 30 + i as u32));
                f.extend(blob([ang.cos() * d, y.min(1.0 - ry), ang.sin() * d], r, ry, r, OAK_LEAF, 0.1, 0.95));
            }
            let ry = (crown * 0.24).min(room * 0.36);
            f.extend(blob([0.0, 1.0 - ry, 0.0], crown * 0.26, ry, crown * 0.26, OAK_LEAF, 0.4, 1.0));
        }
        TreeKind::Birch => {
            let lean = [(rnd(seed, 90) - 0.5) * 0.05, (rnd(seed, 91) - 0.5) * 0.05];
            f.extend(tube([0.0, 0.0, 0.0], [lean[0], 0.9, lean[1]], tw * 0.5, tw * 0.25, 6, BIRCH_BARK, 0.1, 0.9));
            let count = n.min(5);
            for i in 0..count {
                let ang = TAU * i as f32 / count as f32 + rnd(seed, 2) * TAU;
                let y = th.max(0.35) + (1.0 - th.max(0.35)) * (0.2 + 0.7 * i as f32 / count as f32);
                let r = crown * 0.2;
                f.extend(blob([ang.cos() * crown * 0.14 + lean[0], y.min(1.0 - r * 0.9), ang.sin() * crown * 0.14 + lean[1]], r, r * 0.9, r, BIRCH_LEAF, 0.2, 1.0));
            }
        }
        TreeKind::Palm => {
            let steps = 5;
            let at = |t: f32| [0.16 * t * t, 0.82 * t, 0.0];
            for s in 0..steps {
                let (t0, t1) = (s as f32 / steps as f32, (s + 1) as f32 / steps as f32);
                f.extend(tube(at(t0), at(t1), tw * (0.7 - 0.25 * t0), tw * (0.7 - 0.25 * t1), 6, BARK, t0 * 0.8 + 0.1, t1 * 0.8 + 0.1));
            }
            let top = at(1.0);
            let fronds = n + 3;
            for i in 0..fronds {
                let a = TAU * i as f32 / fronds as f32 + rnd(seed, 3) * 0.5;
                let dir = [a.cos(), 0.0, a.sin()];
                let len = crown * 0.55;
                let mid = [top[0] + dir[0] * len * 0.55, top[1] + 0.13, top[2] + dir[2] * len * 0.55];
                let tip = [top[0] + dir[0] * len, top[1] - 0.02, top[2] + dir[2] * len];
                strip(top, mid, 0.01, 0.045, PALM_LEAF, 0.4, 0.9, &mut f);
                strip(mid, tip, 0.045, 0.0, PALM_LEAF, 0.9, 0.5, &mut f);
            }
        }
        TreeKind::Dead => {
            let lean = [(rnd(seed, 90) - 0.5) * 0.1, (rnd(seed, 91) - 0.5) * 0.1];
            f.extend(tube([0.0, 0.0, 0.0], [lean[0], 0.9, lean[1]], tw * 0.7, tw * 0.18, 6, DEAD_WOOD, 0.1, 0.9));
            for i in 0..n {
                let ang = TAU * i as f32 / n as f32 + rnd(seed, 4) * TAU;
                let y = 0.35 + 0.5 * rnd(seed, 40 + i as u32);
                let (s, c) = ang.sin_cos();
                let len = crown * (0.25 + 0.3 * rnd(seed, 50 + i as u32));
                let start = [lean[0] * y / 0.9, y, lean[1] * y / 0.9];
                let end = [start[0] + c * len, y + len * 0.7, start[2] + s * len];
                f.extend(tube(start, end, tw * 0.3, 0.0, 5, DEAD_WOOD, 0.2, 0.9));
            }
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(f: &[Face]) -> ([f32; 3], [f32; 3]) {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (p, _) in f.iter().flatten() {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn every_tree_stands_on_the_ground_and_is_about_one_tall() {
        for k in 0..5 {
            let f = faces(&TreeParams { kind: TreeKind::from_index(k), ..Default::default() });
            assert!(f.len() > 12 && f.iter().all(|f| f.len() >= 3), "{k}");
            let (lo, hi) = bounds(&f);
            assert!(lo[1] > -0.01 && lo[1] < 0.05, "{k}: bottom {}", lo[1]);
            assert!(hi[1] > 0.85 && hi[1] < 1.08, "{k}: top {}", hi[1]);
            assert!(lo[0] > -0.8 && hi[0] < 0.8 && lo[2] > -0.8 && hi[2] < 0.8, "{k}: too wide");
            assert!(f.iter().flatten().all(|(_, uv)| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1])), "{k}: uv outside the palette");
        }
    }

    #[test]
    fn seeds_and_settings_change_the_tree() {
        let a = faces(&TreeParams::default());
        assert_eq!(a, faces(&TreeParams::default()));
        assert_ne!(a, faces(&TreeParams { seed: 2, ..Default::default() }));
        assert_ne!(a.len(), faces(&TreeParams { detail: 8, ..Default::default() }).len());
        let thin = bounds(&faces(&TreeParams { crown: 0.3, ..Default::default() }));
        let wide = bounds(&faces(&TreeParams { crown: 0.9, ..Default::default() }));
        assert!(wide.1[0] - wide.0[0] > thin.1[0] - thin.0[0]);
    }

    #[test]
    fn the_palette_has_a_column_per_material() {
        let a = palette();
        assert_eq!((a.w, a.h), (COLUMNS * 8, 16));
        // bark is brown (red > blue), oak leaves are green (green > red)
        let px = |x: usize| &a.rgba[(8 * a.w + x) * 4..(8 * a.w + x) * 4 + 3];
        assert!(px(BARK * 8 + 3)[0] > px(BARK * 8 + 3)[2]);
        assert!(px(OAK_LEAF * 8 + 3)[1] > px(OAK_LEAF * 8 + 3)[0]);
    }
}
