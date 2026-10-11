//! Built-in primitives, generated as polygon faces (quads wherever the shape allows).
use crate::obj::Face;
use crate::scatter::{scaled, shifted, Shape};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prim {
    Cube,
    Plane,
    Sphere,
    Cylinder,
    Cone,
    Torus,
    /// Procedural heightfield; its geometry comes from the Extra's terrain parameters.
    Terrain,
    /// Billboard particles around the camera; no geometry of its own.
    Particles,
    /// Random objects on a terrain (see the scatter module). Not in [`ALL`]: it needs a terrain to sit on.
    Scatter,
    /// Settings + a capsule stand-in for the game's first-person player (see the player module).
    Player,
    Capsule,
    Pyramid,
    /// A wedge: a slope rising towards -Z.
    Ramp,
    Stairs,
    /// A thin round plate (coins, pads, plates).
    Disc,
    /// Two pillars and a beam: the player can walk through the gap.
    Arch,
    Tree,
    Rock,
    /// A light (see the light module). No geometry but a small marker.
    Light,
}

pub const ALL: [Prim; 18] = [
    Prim::Cube, Prim::Plane, Prim::Sphere, Prim::Cylinder, Prim::Cone, Prim::Torus, Prim::Terrain, Prim::Particles, Prim::Player,
    Prim::Capsule, Prim::Pyramid, Prim::Ramp, Prim::Stairs, Prim::Disc, Prim::Arch, Prim::Tree, Prim::Rock, Prim::Light,
];

impl Prim {
    pub fn name(self) -> &'static str {
        match self {
            Prim::Cube => "Cube",
            Prim::Plane => "Plane",
            Prim::Sphere => "Sphere",
            Prim::Cylinder => "Cylinder",
            Prim::Cone => "Cone",
            Prim::Torus => "Torus",
            Prim::Terrain => "Terrain",
            Prim::Particles => "Particles",
            Prim::Scatter => "Scatter",
            Prim::Player => "Player",
            Prim::Capsule => "Capsule",
            Prim::Pyramid => "Pyramid",
            Prim::Ramp => "Ramp",
            Prim::Stairs => "Stairs",
            Prim::Disc => "Disc",
            Prim::Arch => "Arch",
            Prim::Tree => "Tree",
            Prim::Rock => "Rock",
            Prim::Light => "Light",
        }
    }

    pub fn from_name(s: &str) -> Option<Prim> {
        ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(s))
    }

    /// Faces of a roughly unit-sized shape centred on the origin.
    pub fn faces(self) -> Vec<Face> {
        match self {
            // cut into small pieces: light is computed per vertex, so big faces would light badly
            Prim::Cube => subdivide(cube(), 3),
            Prim::Plane => subdivide(
                vec![vec![
                    ([-0.5, 0.0, 0.5], [0.0, 0.0]),
                    ([0.5, 0.0, 0.5], [1.0, 0.0]),
                    ([0.5, 0.0, -0.5], [1.0, 1.0]),
                    ([-0.5, 0.0, -0.5], [0.0, 1.0]),
                ]],
                12,
            ),
            Prim::Sphere => sphere(12, 24),
            Prim::Cylinder => cylinder(24),
            Prim::Cone => cone(24),
            Prim::Torus => torus(24, 12),
            Prim::Terrain => crate::terrain::faces(&crate::terrain::TerrainParams::default()),
            Prim::Particles | Prim::Scatter => Vec::new(),
            Prim::Player => crate::player::faces(&Default::default()),
            Prim::Capsule => capsule(),
            Prim::Pyramid => pyramid(),
            Prim::Ramp => ramp(),
            Prim::Stairs => stairs(5),
            Prim::Disc => scaled(cylinder(24), [1.0, 0.12, 1.0]),
            Prim::Arch => arch(),
            Prim::Tree => shifted(Shape::Tree.faces(), [0.0, -0.5, 0.0]),
            Prim::Rock => shifted(Shape::Rock.faces(), [0.0, -0.3, 0.0]),
            Prim::Light => crate::light::marker(&Default::default()),
        }
    }
}

pub(crate) fn cube() -> Vec<Face> {
    // (normal, u axis, v axis); u x v = normal, so the corner order below is counter-clockwise.
    let axes: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    axes.iter()
        .map(|&(n, u, v)| {
            [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|&(a, b)| {
                    let p = [(n[0] + u[0] * a + v[0] * b) * 0.5, (n[1] + u[1] * a + v[1] * b) * 0.5, (n[2] + u[2] * a + v[2] * b) * 0.5];
                    (p, [(a + 1.0) * 0.5, (b + 1.0) * 0.5])
                })
                .collect()
        })
        .collect()
}

/// Splits every quad into n x n smaller quads (UVs interpolated); other faces stay as they are.
pub(crate) fn subdivide(faces: Vec<Face>, n: usize) -> Vec<Face> {
    let mut out = Vec::new();
    for f in faces {
        if f.len() != 4 || n < 2 {
            out.push(f);
            continue;
        }
        let at = |u: f32, v: f32| {
            let lerp = |a: ([f32; 3], [f32; 2]), b: ([f32; 3], [f32; 2]), t: f32| {
                ([a.0[0] + (b.0[0] - a.0[0]) * t, a.0[1] + (b.0[1] - a.0[1]) * t, a.0[2] + (b.0[2] - a.0[2]) * t], [a.1[0] + (b.1[0] - a.1[0]) * t, a.1[1] + (b.1[1] - a.1[1]) * t])
            };
            lerp(lerp(f[0], f[1], u), lerp(f[3], f[2], u), v)
        };
        for j in 0..n {
            for i in 0..n {
                let (u0, u1, v0, v1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32, j as f32 / n as f32, (j + 1) as f32 / n as f32);
                out.push(vec![at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)]);
            }
        }
    }
    out
}

/// A polygon from points, turned around if needed so that it faces away from `inside`.
pub(crate) fn poly(points: &[([f32; 3], [f32; 2])], inside: [f32; 3]) -> Face {
    let k = points.len();
    let c = points.iter().fold([0.0f32; 3], |a, (p, _)| [a[0] + p[0] / k as f32, a[1] + p[1] / k as f32, a[2] + p[2] / k as f32]);
    let mut n = [0.0f32; 3];
    for i in 0..k {
        let (a, b) = (points[i].0, points[(i + 1) % k].0);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let out = [c[0] - inside[0], c[1] - inside[1], c[2] - inside[2]];
    let mut f = points.to_vec();
    if n[0] * out[0] + n[1] * out[1] + n[2] * out[2] < 0.0 {
        f.reverse();
    }
    f
}

fn boxf(min: [f32; 3], max: [f32; 3]) -> Vec<Face> {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    shifted(scaled(cube(), size), [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5])
}

fn pyramid() -> Vec<Face> {
    let base = [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]];
    let apex = [0.0, 0.5, 0.0];
    let inside = [0.0, -0.15, 0.0];
    let mut f = vec![poly(&[(base[0], [0.0, 0.0]), (base[1], [1.0, 0.0]), (base[2], [1.0, 1.0]), (base[3], [0.0, 1.0])], inside)];
    for i in 0..4 {
        f.push(poly(&[(base[i], [0.0, 0.0]), (base[(i + 1) % 4], [1.0, 0.0]), (apex, [0.5, 1.0])], inside));
    }
    f
}

fn ramp() -> Vec<Face> {
    let (a, b, c, d) = ([-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [-0.5, -0.5, -0.5]);
    let (e, f) = ([-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]);
    let inside = [0.0, -0.2, 0.0];
    let quad = |p: [[f32; 3]; 4]| poly(&[(p[0], [0.0, 0.0]), (p[1], [1.0, 0.0]), (p[2], [1.0, 1.0]), (p[3], [0.0, 1.0])], inside);
    vec![
        quad([a, b, c, d]),                                                   // bottom
        quad([a, b, f, e]),                                                   // the slope
        quad([d, c, f, e]),                                                   // back wall
        poly(&[(a, [0.0, 0.0]), (d, [1.0, 0.0]), (e, [1.0, 1.0])], inside), // sides
        poly(&[(b, [0.0, 0.0]), (c, [1.0, 0.0]), (f, [1.0, 1.0])], inside),
    ]
}

fn stairs(n: usize) -> Vec<Face> {
    let step = 1.0 / n as f32;
    (0..n).flat_map(|i| boxf([-0.5, -0.5, -0.5], [0.5, -0.5 + (i + 1) as f32 * step, -0.5 + (n - i) as f32 * step])).collect()
}

fn arch() -> Vec<Face> {
    let mut f = boxf([-0.5, -0.5, -0.15], [-0.3, 0.3, 0.15]);
    f.extend(boxf([0.3, -0.5, -0.15], [0.5, 0.3, 0.15]));
    f.extend(boxf([-0.5, 0.3, -0.15], [0.5, 0.5, 0.15]));
    f
}

fn capsule() -> Vec<Face> {
    let centre_y = |f: &Face| f.iter().map(|(p, _)| p[1]).sum::<f32>() / f.len() as f32;
    let (mut top, mut bottom) = (Vec::new(), Vec::new());
    for f in sphere(8, 16) {
        if centre_y(&f) >= 0.0 { top.push(f) } else { bottom.push(f) }
    }
    let mut out = shifted(scaled(top, [0.6; 3]), [0.0, 0.2, 0.0]);
    out.extend(shifted(scaled(bottom, [0.6; 3]), [0.0, -0.2, 0.0]));
    // the tube between the two halves, without its end caps
    let tube: Vec<Face> = cylinder(16).into_iter().filter(|f| f.len() == 4).collect();
    out.extend(scaled(tube, [0.6, 0.4, 0.6]));
    out
}

pub(crate) fn sphere(rings: usize, segs: usize) -> Vec<Face> {
    let pt = |r: usize, s: usize| {
        let (th, ph) = (PI * r as f32 / rings as f32, TAU * s as f32 / segs as f32);
        ([0.5 * th.sin() * ph.cos(), 0.5 * th.cos(), 0.5 * th.sin() * ph.sin()], [s as f32 / segs as f32, 1.0 - r as f32 / rings as f32])
    };
    let mut faces = Vec::new();
    for r in 0..rings {
        for s in 0..segs {
            let q = [pt(r, s), pt(r, s + 1), pt(r + 1, s + 1), pt(r + 1, s)];
            if r == 0 {
                faces.push(vec![q[0], q[2], q[3]]); // top pole: triangle
            } else if r == rings - 1 {
                faces.push(vec![q[0], q[1], q[2]]); // bottom pole: triangle
            } else {
                faces.push(q.to_vec());
            }
        }
    }
    faces
}

pub(crate) fn cylinder(segs: usize) -> Vec<Face> {
    let at = |s: usize, y: f32| {
        let a = TAU * s as f32 / segs as f32;
        [0.5 * a.cos(), y, 0.5 * a.sin()]
    };
    let u = |s: usize| s as f32 / segs as f32;
    let mut faces: Vec<Face> = (0..segs)
        .map(|s| vec![(at(s, -0.5), [u(s), 0.0]), (at(s, 0.5), [u(s), 1.0]), (at(s + 1, 0.5), [u(s + 1), 1.0]), (at(s + 1, -0.5), [u(s + 1), 0.0])])
        .collect();
    let cap = |y: f32, rev: bool| -> Face {
        let mut f: Face = (0..segs).map(|s| (at(s, y), [0.5 + at(s, y)[0], 0.5 + at(s, y)[2]])).collect();
        if rev {
            f.reverse();
        }
        f
    };
    faces.push(cap(0.5, true)); // top: counter-clockwise seen from above = decreasing angle
    faces.push(cap(-0.5, false));
    faces
}

pub(crate) fn cone(segs: usize) -> Vec<Face> {
    let base = |s: usize| {
        let a = TAU * s as f32 / segs as f32;
        [0.5 * a.cos(), -0.5, 0.5 * a.sin()]
    };
    let u = |s: usize| s as f32 / segs as f32;
    let mut faces: Vec<Face> = (0..segs).map(|s| vec![(base(s), [u(s), 0.0]), ([0.0, 0.5, 0.0], [u(s) + 0.5 / segs as f32, 1.0]), (base(s + 1), [u(s + 1), 0.0])]).collect();
    faces.push((0..segs).map(|s| (base(s), [0.5 + base(s)[0], 0.5 + base(s)[2]])).collect());
    faces
}

fn torus(segs: usize, rings: usize) -> Vec<Face> {
    let (big, small) = (0.35f32, 0.15f32);
    let pt = |u: usize, v: usize| {
        let (a, b) = (TAU * u as f32 / segs as f32, TAU * v as f32 / rings as f32);
        let r = big + small * b.cos();
        ([r * a.cos(), small * b.sin(), r * a.sin()], [u as f32 / segs as f32, v as f32 / rings as f32])
    };
    let mut faces = Vec::new();
    for u in 0..segs {
        for v in 0..rings {
            faces.push(vec![pt(u, v), pt(u, v + 1), pt(u + 1, v + 1), pt(u + 1, v)]);
        }
    }
    faces
}

/// Radius of the smallest origin-centred sphere containing the shape.
pub fn radius(faces: &[Face]) -> f32 {
    faces.iter().flatten().map(|(p, _)| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_outward_and_sane() {
        for p in ALL {
            let faces = p.faces();
            if p == Prim::Particles {
                assert!(faces.is_empty());
                continue;
            }
            assert!(!faces.is_empty());
            assert!(faces.iter().all(|f| f.len() >= 3), "{p:?}");
            if matches!(p, Prim::Stairs | Prim::Arch | Prim::Tree | Prim::Rock | Prim::Ramp) {
                continue; // not convex around the origin: only checked for being well formed above
            }
            if p == Prim::Plane || p == Prim::Terrain || p == Prim::Player || p == Prim::Light {
                continue; // open surfaces
            }
            // closed solids: each polygon normal must point away from the solid
            // (from the origin; for the torus, from the centre line of the tube)
            let bad = faces
                .iter()
                .filter(|f| {
                    let k = f.len();
                    let c = f.iter().fold([0.0f32; 3], |a, (p, _)| [a[0] + p[0] / k as f32, a[1] + p[1] / k as f32, a[2] + p[2] / k as f32]);
                    let mut n = [0.0f32; 3];
                    for i in 0..k {
                        let (a, b) = (f[i].0, f[(i + 1) % k].0);
                        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
                    }
                    let out = if p == Prim::Torus {
                        let l = (c[0] * c[0] + c[2] * c[2]).sqrt().max(1e-6);
                        [c[0] - 0.35 * c[0] / l, c[1], c[2] - 0.35 * c[2] / l]
                    } else {
                        c
                    };
                    n[0] * out[0] + n[1] * out[1] + n[2] * out[2] < -1e-6
                })
                .count();
            assert_eq!(bad, 0, "{p:?} has inward-facing polygons");
        }
    }
}
