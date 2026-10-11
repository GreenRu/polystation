//! Random objects scattered over a terrain (trees, rocks, grass ...).
//!
//! A scatter layer is an object of its own; it points at a terrain by name. Its geometry is built in
//! the terrain's local space (so it moves, rotates and scales with the terrain, and sits exactly on
//! the heightfield). Shapes are sized in world units: the terrain's scale is divided out, so a tree
//! is not squashed when the terrain is stretched. Placement is deterministic from the seed and, for
//! chunked terrain, from each chunk's coordinates, so objects appear and disappear with their chunk.
use crate::obj::Face;
use crate::prims;
use crate::terrain::{self, TerrainParams};
use crate::trees::{self, TreeKind, TreeParams};
use std::f32::consts::TAU;

/// Safety cap on the number of instances in one layer.
pub const MAX_INSTANCES: usize = 20_000;
pub const MAX_COUNT: u32 = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Tree,
    Rock,
    Cone,
    Sphere,
    Cube,
    Cylinder,
    /// Two crossed upright quads, for grass and flowers.
    Grass,
    Pine,
    Oak,
    Birch,
    Palm,
    Dead,
    /// Any object of the scene (see `ScatterParams::source`): model your own tree in Blender or from
    /// primitives, and plant copies of it.
    Custom,
}

impl Shape {
    pub const ALL: [Shape; 13] = [Shape::Tree, Shape::Rock, Shape::Cone, Shape::Sphere, Shape::Cube, Shape::Cylinder, Shape::Grass, Shape::Pine, Shape::Oak, Shape::Birch, Shape::Palm, Shape::Dead, Shape::Custom];
    pub const NAMES: [&'static str; 13] = ["Tree", "Rock", "Cone", "Sphere", "Cube", "Cylinder", "Grass (cross)", "Pine", "Oak", "Birch", "Palm", "Dead tree", "Custom object"];

    /// The kind of tree, for the shapes that are trees (they are colored by the tree palette).
    pub fn tree_kind(self) -> Option<TreeKind> {
        Some(match self {
            Shape::Tree | Shape::Oak => TreeKind::Oak,
            Shape::Pine => TreeKind::Pine,
            Shape::Birch => TreeKind::Birch,
            Shape::Palm => TreeKind::Palm,
            Shape::Dead => TreeKind::Dead,
            _ => return None,
        })
    }

    /// A few different versions of the shape (trees vary, the other shapes are all the same).
    pub fn variants(self) -> Vec<Vec<Face>> {
        match self.tree_kind() {
            Some(kind) => (1..=6).map(|seed| trees::faces(&TreeParams { kind, seed, ..Default::default() })).collect(),
            None => vec![self.faces()],
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&s| s == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Shape {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    /// The shape inside the box [-0.5, 0.5] x [0, 1] x [-0.5, 0.5]: standing on y = 0, about 1 unit tall.
    pub fn faces(self) -> Vec<Face> {
        match self {
            Shape::Custom => Vec::new(), // comes from the source object
            Shape::Cone => shifted(prims::cone(8), [0.0, 0.5, 0.0]),
            Shape::Sphere => shifted(prims::sphere(6, 10), [0.0, 0.5, 0.0]),
            Shape::Cube => shifted(prims::cube(), [0.0, 0.5, 0.0]),
            Shape::Cylinder => shifted(prims::cylinder(8), [0.0, 0.5, 0.0]),
            Shape::Rock => shifted(scaled(prims::sphere(5, 8), [1.0, 0.6, 0.85]), [0.0, 0.3, 0.0]),
            Shape::Tree | Shape::Pine | Shape::Oak | Shape::Birch | Shape::Palm | Shape::Dead => trees::faces(&TreeParams { kind: self.tree_kind().unwrap_or_default(), ..Default::default() }),
            Shape::Grass => {
                let q = |a: [f32; 3], b: [f32; 3]| -> Face { vec![(a, [0.0, 0.0]), (b, [1.0, 0.0]), ([b[0], 1.0, b[2]], [1.0, 1.0]), ([a[0], 1.0, a[2]], [0.0, 1.0])] };
                vec![q([-0.5, 0.0, 0.0], [0.5, 0.0, 0.0]), q([0.0, 0.0, 0.5], [0.0, 0.0, -0.5])]
            }
        }
    }
}

pub(crate) fn shifted(faces: Vec<Face>, d: [f32; 3]) -> Vec<Face> {
    faces.into_iter().map(|f| f.into_iter().map(|(p, uv)| ([p[0] + d[0], p[1] + d[1], p[2] + d[2]], uv)).collect()).collect()
}

pub(crate) fn scaled(faces: Vec<Face>, s: [f32; 3]) -> Vec<Face> {
    faces.into_iter().map(|f| f.into_iter().map(|(p, uv)| ([p[0] * s[0], p[1] * s[1], p[2] * s[2]], uv)).collect()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScatterParams {
    /// Name of the terrain object to scatter on.
    pub target: String,
    pub shape: Shape,
    /// Objects per chunk of terrain (the whole terrain when it is not chunked).
    pub count: u32,
    pub seed: u32,
    /// Size range in world units; each object gets a random size in between.
    pub min_size: f32,
    pub max_size: f32,
    /// Objects are not placed where the ground is steeper than this (degrees; 90 = anywhere).
    pub max_slope: f32,
    /// How far (world units) objects are sunk into the ground.
    pub sink: f32,
    /// With the Custom shape: the object (or collection) whose geometry is planted.
    pub source: String,
}

impl Default for ScatterParams {
    fn default() -> Self {
        ScatterParams { target: String::new(), shape: Shape::Tree, count: 40, seed: 1, min_size: 0.5, max_size: 1.0, max_slope: 40.0, sink: 0.0, source: String::new() }
    }
}

impl ScatterParams {
    pub fn sanitize(&mut self) {
        self.count = self.count.min(MAX_COUNT);
        self.min_size = self.min_size.clamp(0.001, 100_000.0);
        self.max_size = self.max_size.clamp(0.001, 100_000.0);
        self.max_slope = self.max_slope.clamp(0.0, 90.0);
        self.sink = self.sink.clamp(-100_000.0, 100_000.0);
    }
}

fn rnd(cx: i32, cz: i32, seed: u32, i: u32, k: u32) -> f32 {
    let s = seed.wrapping_add(i.wrapping_mul(0x9E37_79B1)).wrapping_add(k.wrapping_mul(0x85EB_CA6B));
    ((terrain::hash2(cx, cz, s) >> 8) as f32) / 16_777_216.0
}

/// The scattered objects as polygons in the terrain's local space.
/// `tscale` is the terrain object's scale (used to keep shapes undistorted).
/// A polygon and the atlas rectangle of its material (zero = none).
pub type Placed = (Face, [f32; 4]);

#[cfg(test)]
pub fn faces(p: &ScatterParams, t: &TerrainParams, tscale: [f32; 3], view: (i32, i32)) -> Vec<Face> {
    place(p, t, tscale, view, None).into_iter().map(|(f, _)| f).collect()
}

/// The scattered objects. `template` is the planted geometry for the Custom shape (see
/// `scene::custom_template`); other shapes make their own.
pub fn place(p: &ScatterParams, t: &TerrainParams, tscale: [f32; 3], view: (i32, i32), template: Option<&[Placed]>) -> Vec<Placed> {
    let chunks: Vec<(i32, i32)> = if t.chunked { terrain::chunk_window(t, view.0, view.1) } else { vec![(0, 0)] };
    let per = (p.count.min(MAX_COUNT) as usize).min(MAX_INSTANCES / chunks.len().max(1)) as u32;
    let variants: Vec<Vec<Placed>> = match template {
        Some(t) => vec![t.to_vec()],
        None if p.shape == Shape::Custom => Vec::new(),
        None => p.shape.variants().into_iter().map(|v| v.into_iter().map(|f| (f, [0.0; 4])).collect()).collect(),
    };
    if variants.is_empty() {
        return Vec::new();
    }
    let sc = tscale.map(|v| v.abs().max(1e-3));
    let (lo, hi) = (p.min_size.min(p.max_size), p.min_size.max(p.max_size));
    let mut out = Vec::new();
    for &(cx, cz) in &chunks {
        for i in 0..per {
            let (u, v) = (cx as f32 + rnd(cx, cz, p.seed, i, 0), cz as f32 + rnd(cx, cz, p.seed, i, 1));
            let h = terrain::height(t, u, v);
            if p.max_slope < 89.9 {
                // steepness of the ground in world units
                let e = 0.01;
                let dx = (terrain::height(t, u + e, v) - terrain::height(t, u - e, v)) / (2.0 * e);
                let dz = (terrain::height(t, u, v + e) - terrain::height(t, u, v - e)) / (2.0 * e);
                let slope = ((dx * sc[1] / sc[0]).powi(2) + (dz * sc[1] / sc[2]).powi(2)).sqrt().atan().to_degrees();
                if slope > p.max_slope {
                    continue;
                }
            }
            let size = lo + (hi - lo) * rnd(cx, cz, p.seed, i, 2);
            let (sin, cos) = (rnd(cx, cz, p.seed, i, 3) * TAU).sin_cos();
            let origin = [u - 0.5, h, v - 0.5];
            let shape = &variants[(rnd(cx, cz, p.seed, i, 4) * variants.len() as f32) as usize % variants.len()];
            for (f, rect) in shape {
                out.push((
                    f.iter()
                        .map(|&(q, uv)| {
                            let (x, y, z) = (q[0] * size, q[1] * size - p.sink, q[2] * size);
                            let (rx, rz) = (x * cos + z * sin, -x * sin + z * cos); // turn around the vertical axis
                            ([origin[0] + rx / sc[0], origin[1] + y / sc[1], origin[2] + rz / sc[2]], uv)
                        })
                        .collect(),
                    *rect,
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain() -> TerrainParams {
        TerrainParams::default()
    }

    #[test]
    fn instance_count_and_determinism() {
        let p = ScatterParams { count: 10, max_slope: 90.0, shape: Shape::Cube, ..Default::default() };
        let per_shape = p.shape.faces().len();
        let a = faces(&p, &terrain(), [10.0, 4.0, 10.0], (0, 0));
        assert_eq!(a.len(), 10 * per_shape);
        assert_eq!(a, faces(&p, &terrain(), [10.0, 4.0, 10.0], (0, 0)));
        let b = faces(&ScatterParams { seed: 2, ..p.clone() }, &terrain(), [10.0, 4.0, 10.0], (0, 0));
        assert_ne!(a, b, "another seed must give another layout");
    }

    #[test]
    fn slope_limit_removes_objects_on_steep_ground() {
        let steep = TerrainParams { res: 16, layers: vec![terrain::Layer { kind: terrain::NoiseKind::Ridged, seed: 3, scale: 0.05, amp: 0.6 }], ..Default::default() };
        let any = ScatterParams { count: 200, max_slope: 90.0, shape: Shape::Cube, ..Default::default() };
        let flat_only = ScatterParams { max_slope: 10.0, ..any.clone() };
        let (n_any, n_flat) = (faces(&any, &steep, [10.0; 3], (0, 0)).len(), faces(&flat_only, &steep, [10.0; 3], (0, 0)).len());
        assert!(n_flat < n_any, "{n_flat} should be fewer than {n_any}");
    }

    #[test]
    fn objects_stand_on_the_terrain_and_follow_chunks() {
        let t = TerrainParams { chunked: true, view_dist: 2, ..terrain() };
        let p = ScatterParams { count: 5, shape: Shape::Cube, max_slope: 90.0, sink: 0.0, ..Default::default() };
        let scale = [10.0, 10.0, 10.0];
        // every object's lowest vertex is at ground level (sink = 0)
        for f in faces(&p, &t, scale, (0, 0)).chunks(6) {
            let ground_ok = f.iter().flatten().map(|(v, _)| v[1]).fold(f32::MAX, f32::min);
            let (u, w) = (f[0][0].0[0] + 0.5, f[0][0].0[2] + 0.5);
            let h = terrain::height(&t, u, w);
            assert!((ground_ok - h).abs() < 0.2, "object floats or is buried: {ground_ok} vs ground {h}");
        }
        // more chunks in range -> more objects; moving to another chunk changes where they are
        let near = faces(&p, &t, scale, (0, 0));
        assert_eq!(near.len(), terrain::chunk_window(&t, 0, 0).len() * 5 * 6);
        assert_ne!(near, faces(&p, &t, scale, (3, 3)));
        assert!(faces(&p, &TerrainParams { view_dist: 4, ..t.clone() }, scale, (0, 0)).len() > near.len());
    }

    #[test]
    fn sizes_stay_in_range_and_ignore_terrain_stretch() {
        let p = ScatterParams { count: 30, min_size: 2.0, max_size: 3.0, shape: Shape::Cube, max_slope: 90.0, ..Default::default() };
        for scale in [[10.0, 10.0, 10.0], [10.0, 40.0, 10.0]] {
            for f in faces(&p, &terrain(), scale, (0, 0)).iter().take(6) {
                // a cube face is a square of the object's world size (before rotation): edge length * scale in 2.0..=3.0
                let (a, b) = (f[0].0, f[1].0);
                let d = [(b[0] - a[0]) * scale[0], (b[1] - a[1]) * scale[1], (b[2] - a[2]) * scale[2]];
                let edge = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                assert!((1.99..=3.01).contains(&edge), "edge {edge} for scale {scale:?}");
            }
        }
    }

    #[test]
    fn custom_objects_are_planted_with_their_materials() {
        let tpl: Vec<Placed> = vec![(vec![([0.0, 0.0, 0.0], [0.0, 0.0]), ([1.0, 0.0, 0.0], [1.0, 0.0]), ([0.0, 1.0, 0.0], [0.0, 1.0])], [5.0, 6.0, 7.0, 8.0])];
        let p = ScatterParams { count: 7, shape: Shape::Custom, max_slope: 90.0, ..Default::default() };
        let out = place(&p, &terrain(), [10.0; 3], (0, 0), Some(&tpl));
        assert_eq!(out.len(), 7);
        assert!(out.iter().all(|(_, r)| *r == [5.0, 6.0, 7.0, 8.0]));
        assert!(place(&p, &terrain(), [10.0; 3], (0, 0), None).is_empty(), "no source object: nothing is planted");
    }

    #[test]
    fn every_shape_stays_in_its_unit_box() {
        for s in Shape::ALL.into_iter().filter(|&s| s != Shape::Custom) {
            let f = s.faces();
            assert!(!f.is_empty());
            for (p, _) in f.iter().flatten() {
                assert!(p[0].abs() <= 0.55 && p[2].abs() <= 0.55 && p[1] >= -0.01 && p[1] <= 1.01, "{s:?}: {p:?}");
            }
        }
    }
}
