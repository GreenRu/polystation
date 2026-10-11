//! Particle system settings. A particle system is an object in the explorer (an "extra" like the
//! primitives); its texture, colour and opacity are the object's own, the rest lives here.
//!
//! The particles fill a cube centred on the camera. Every particle has a fixed random place in the
//! world; it drifts with `velocity` and `gravity` and, whenever it would leave the cube around the
//! camera, wraps to the opposite side - so the camera always looks at an endless field of them
//! (snow, rain, dust, sparks ...).

pub const MAX_PARTICLES: u32 = 50_000;

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleParams {
    /// World units per second.
    pub velocity: [f32; 3],
    /// Downward acceleration in units per second squared (negative = upward).
    pub gravity: f32,
    /// Side length of each square billboard.
    pub size: f32,
    pub amount: u32,
    /// Side length of the cube around the camera that the particles live in.
    pub box_size: f32,
    /// Seconds until a particle starts over (its motion restarts, hidden by a fade).
    pub lifetime: f32,
}

impl Default for ParticleParams {
    fn default() -> Self {
        ParticleParams { velocity: [0.0, -1.5, 0.0], gravity: 0.0, size: 0.1, amount: 1000, box_size: 20.0, lifetime: 6.0 }
    }
}

impl ParticleParams {
    /// Sensible defaults for a scene of the given radius (so the particles are visible at its scale).
    pub fn for_scene(scene_radius: f32) -> Self {
        let box_size = (scene_radius * 1.2).max(10.0);
        ParticleParams { velocity: [0.0, -box_size * 0.08, 0.0], size: (scene_radius * 0.01).max(0.05), box_size, ..Default::default() }
    }

    /// Clamps every value into a range the renderer can handle.
    pub fn sanitize(&mut self) {
        self.amount = self.amount.min(MAX_PARTICLES);
        self.size = self.size.clamp(0.001, 1000.0);
        self.box_size = self.box_size.clamp(0.1, 100_000.0);
        self.lifetime = self.lifetime.clamp(0.1, 600.0);
        for v in &mut self.velocity {
            *v = v.clamp(-10_000.0, 10_000.0);
        }
        self.gravity = self.gravity.clamp(-10_000.0, 10_000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_clamps() {
        let mut p = ParticleParams { amount: 10_000_000, size: -1.0, box_size: 0.0, lifetime: 0.0, gravity: 1e9, velocity: [1e9, 0.0, -1e9] };
        p.sanitize();
        assert_eq!(p.amount, MAX_PARTICLES);
        assert!(p.size > 0.0 && p.box_size >= 0.1 && p.lifetime >= 0.1);
        assert_eq!((p.velocity[0], p.velocity[2], p.gravity), (10_000.0, -10_000.0, 10_000.0));
    }

    #[test]
    fn scene_defaults_scale_with_the_scene() {
        let (small, big) = (ParticleParams::for_scene(1.0), ParticleParams::for_scene(500.0));
        assert!(big.box_size > small.box_size && big.size > small.size);
        assert!(big.velocity[1] < 0.0);
    }
}
