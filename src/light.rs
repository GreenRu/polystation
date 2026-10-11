//! Lights. A light is an object in the scene (Add > Light). Lighting is computed per VERTEX, in the
//! vertex shader, like on the consoles this engine imitates: every vertex adds up the light that
//! reaches it and the result is blended across the triangle. Up to [`MAX_LIGHTS`] lights count; with
//! none in the scene the old single sun is used and nothing changes.
//!
//! A light shines down its object's -Y axis (spot lights and suns): rotate the object to aim it.
use crate::obj::Face;
use crate::prims::{self, poly};
use crate::scatter::{scaled, shifted};

pub const MAX_LIGHTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LightKind {
    /// Shines in every direction and fades out with distance.
    #[default]
    Point,
    /// A cone of light.
    Spot,
    /// Parallel light from far away (the sun): position does not matter, only the direction.
    Sun,
}

impl LightKind {
    pub const NAMES: [&'static str; 3] = ["Point", "Spot", "Sun"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> LightKind {
        [LightKind::Point, LightKind::Spot, LightKind::Sun][i.min(2)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightParams {
    pub kind: LightKind,
    /// Brightness multiplier (the object's color tints the light).
    pub intensity: f32,
    /// Where a point / spot light has faded out completely, in world units.
    pub range: f32,
    /// Full opening angle of a spot light, in degrees.
    pub angle: f32,
}

impl Default for LightParams {
    fn default() -> Self {
        LightParams { kind: LightKind::Point, intensity: 1.0, range: 15.0, angle: 50.0 }
    }
}

impl LightParams {
    pub fn sanitize(&mut self) {
        self.intensity = self.intensity.clamp(0.0, 100.0);
        self.range = self.range.clamp(0.1, 100_000.0);
        self.angle = self.angle.clamp(5.0, 170.0);
    }
}

/// The little shape that marks a light in the editor (hidden while playing): a diamond, with a cone
/// for a spot light and a pole for the sun to show which way it shines.
pub fn marker(p: &LightParams) -> Vec<Face> {
    let r = 0.25;
    let v = [[r, 0.0, 0.0], [-r, 0.0, 0.0], [0.0, r, 0.0], [0.0, -r, 0.0], [0.0, 0.0, r], [0.0, 0.0, -r]];
    let mut faces = Vec::new();
    for &(a, b, c) in &[(0usize, 2usize, 4usize), (4, 2, 1), (1, 2, 5), (5, 2, 0), (4, 3, 0), (1, 3, 4), (5, 3, 1), (0, 3, 5)] {
        faces.push(poly(&[(v[a], [0.0, 0.0]), (v[b], [1.0, 0.0]), (v[c], [0.5, 1.0])], [0.0; 3]));
    }
    match p.kind {
        LightKind::Point => {}
        LightKind::Spot => {
            // a cone pointing down (the shape's apex is up, so turn it over)
            let cone: Vec<Face> = scaled(prims::cone(10), [0.5, 0.7, 0.5]).into_iter().map(|f| f.into_iter().map(|(p, uv)| ([p[0], -p[1], p[2]], uv)).rev().collect::<Face>()).collect();
            faces.extend(shifted(cone, [0.0, -0.55, 0.0]));
        }
        LightKind::Sun => faces.extend(shifted(scaled(prims::cylinder(6), [0.06, 1.2, 0.06]), [0.0, -0.85, 0.0])),
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_exist_for_every_kind() {
        for k in 0..3 {
            let f = marker(&LightParams { kind: LightKind::from_index(k), ..Default::default() });
            assert!(f.len() >= 8 && f.iter().all(|f| f.len() >= 3));
        }
    }

    #[test]
    fn sanitize_clamps() {
        let mut p = LightParams { intensity: -3.0, range: 0.0, angle: 999.0, kind: LightKind::Spot };
        p.sanitize();
        assert_eq!((p.intensity, p.angle), (0.0, 170.0));
        assert!(p.range > 0.0);
    }
}
