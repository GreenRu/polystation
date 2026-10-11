//! Player object: the settings a game's first-person controller will read once the custom language and
//! player exist. In the editor it is an ordinary object (move/rotate it to set the spawn point) drawn as
//! a capsule-like stand-in of the player's real size, with a visor on the side it looks towards (-Z).
use crate::obj::Face;
use crate::prims::{self, Prim};
use crate::scatter::{scaled, shifted};

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerParams {
    /// Walking speed, world units per second.
    pub speed: f32,
    /// Speed multiplier while sprinting.
    pub sprint_mult: f32,
    /// Downward acceleration, units per second squared.
    pub gravity: f32,
    /// How high a jump reaches, world units.
    pub jump_height: f32,
    /// Standing height of the collision body.
    pub height: f32,
    /// Radius of the collision body.
    pub radius: f32,
    /// Camera height above the feet.
    pub eye_height: f32,
    /// Vertical field of view in degrees.
    pub fov: f32,
    /// Vertical camera sway while walking (world units; 0 = off).
    pub bob_amount: f32,
    /// Sway cycles per second at walking speed.
    pub bob_speed: f32,
    /// Sound played on every footstep (path; empty = silent).
    pub step_sound: String,
    pub step_volume: f32,
    /// Distance walked between two footsteps.
    pub step_distance: f32,
    /// Look sensitivity, degrees per pixel of mouse movement.
    pub mouse_sens: f32,
    /// Tallest ledge the player walks up without jumping.
    pub max_step: f32,
    /// Steering control while in the air (0 = none, 1 = as on the ground).
    pub air_control: f32,
    /// Makes the player bigger (or smaller) without touching the scene: size, speed, jump, step height,
    /// gravity and head bob all scale with it.
    pub scale: f32,
}

impl Default for PlayerParams {
    fn default() -> Self {
        PlayerParams {
            speed: 5.0,
            sprint_mult: 1.6,
            gravity: 9.81,
            jump_height: 1.1,
            height: 1.8,
            radius: 0.35,
            eye_height: 1.65,
            fov: 75.0,
            bob_amount: 0.05,
            bob_speed: 2.0,
            step_sound: String::new(),
            step_volume: 0.8,
            step_distance: 2.2,
            mouse_sens: 0.15,
            max_step: 0.4,
            air_control: 0.3,
            scale: 1.0,
        }
    }
}

impl PlayerParams {
    pub fn sanitize(&mut self) {
        self.speed = self.speed.clamp(0.0, 10_000.0);
        self.sprint_mult = self.sprint_mult.clamp(0.0, 100.0);
        self.gravity = self.gravity.clamp(-10_000.0, 10_000.0);
        self.jump_height = self.jump_height.clamp(0.0, 10_000.0);
        self.height = self.height.clamp(0.05, 10_000.0);
        self.radius = self.radius.clamp(0.01, 10_000.0);
        self.eye_height = self.eye_height.clamp(0.0, self.height);
        self.fov = self.fov.clamp(10.0, 170.0);
        self.bob_amount = self.bob_amount.clamp(0.0, 100.0);
        self.bob_speed = self.bob_speed.clamp(0.0, 100.0);
        self.step_volume = self.step_volume.clamp(0.0, 1.0);
        self.step_distance = self.step_distance.clamp(0.05, 1000.0);
        self.mouse_sens = self.mouse_sens.clamp(0.001, 10.0);
        self.max_step = self.max_step.clamp(0.0, self.height);
        self.air_control = self.air_control.clamp(0.0, 1.0);
        self.scale = self.scale.clamp(0.01, 1000.0);
    }

    /// The settings with everything that has a size multiplied by `scale`.
    pub fn scaled(&self) -> PlayerParams {
        let k = self.scale;
        PlayerParams {
            speed: self.speed * k,
            gravity: self.gravity * k,
            jump_height: self.jump_height * k,
            height: self.height * k,
            radius: self.radius * k,
            eye_height: self.eye_height * k,
            bob_amount: self.bob_amount * k,
            step_distance: self.step_distance * k,
            max_step: self.max_step * k,
            scale: 1.0,
            ..self.clone()
        }
    }
}

/// The editor stand-in: a pill of the player's size standing on y = 0.
pub fn faces(p: &PlayerParams) -> Vec<Face> {
    let p = &p.scaled();
    let (h, r, eye) = (p.height.max(0.05), p.radius.max(0.01), p.eye_height.clamp(0.0, p.height.max(0.05)));
    let _ = eye;
    pill(r, h)
}

/// A pill (capsule): a tube with a half sphere on each end, `h` tall and `2 r` wide, standing on y = 0.
pub fn pill(r: f32, h: f32) -> Vec<Face> {
    let r = r.min(h * 0.5);
    let tube_h = (h - 2.0 * r).max(0.0);
    let (mut top, mut bottom) = (Vec::new(), Vec::new());
    for f in prims::sphere(8, 16) {
        if f.iter().map(|(p, _)| p[1]).sum::<f32>() >= 0.0 { top.push(f) } else { bottom.push(f) }
    }
    let up = h * 0.5 - r;
    let mut out = shifted(scaled(top, [r * 2.0; 3]), [0.0, up, 0.0]);
    out.extend(shifted(scaled(bottom, [r * 2.0; 3]), [0.0, -up, 0.0]));
    if tube_h > 1e-4 {
        let tube: Vec<Face> = prims::cylinder(16).into_iter().filter(|f| f.len() == 4).collect();
        out.extend(scaled(tube, [r * 2.0, tube_h, r * 2.0]));
    }
    shifted(out, [0.0, h * 0.5, 0.0])
}

/// Reads a .wav file and scales its samples by `volume` (0 - 1), ready for in-memory playback.
/// Only 8 and 16 bit PCM are scaled; other sample formats play at full volume.
pub fn load_wav_scaled(path: &str, volume: f32) -> Result<Vec<u8>, String> {
    let mut b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(format!("{path}: step sounds must be .wav files"));
    }
    let (mut pos, mut bits, mut tag, mut data) = (12usize, 0u16, 0u16, None);
    while pos + 8 <= b.len() {
        let size = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        let body = pos + 8;
        match &b[pos..pos + 4] {
            b"fmt " if body + 16 <= b.len() => {
                tag = u16::from_le_bytes([b[body], b[body + 1]]);
                bits = u16::from_le_bytes([b[body + 14], b[body + 15]]);
            }
            b"data" => data = Some((body, (body + size).min(b.len()))),
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    let (lo, hi) = data.ok_or_else(|| format!("{path}: no sound data"))?;
    let v = volume.clamp(0.0, 1.0);
    if tag == 1 || tag == 0xFFFE {
        match bits {
            16 => {
                for s in b[lo..hi - (hi - lo) % 2].chunks_exact_mut(2) {
                    let x = (i16::from_le_bytes([s[0], s[1]]) as f32 * v) as i16;
                    s.copy_from_slice(&x.to_le_bytes());
                }
            }
            8 => {
                for s in &mut b[lo..hi] {
                    *s = ((*s as f32 - 128.0) * v + 128.0).clamp(0.0, 255.0) as u8;
                }
            }
            _ => {}
        }
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_volume_scales_samples() {
        let mut wav = b"RIFF".to_vec();
        wav.extend(36u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend([1, 0, 1, 0]); // PCM, mono
        wav.extend(8000u32.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend([2, 0, 16, 0]);
        wav.extend(b"data");
        wav.extend(4u32.to_le_bytes());
        wav.extend(1000i16.to_le_bytes());
        wav.extend((-2000i16).to_le_bytes());
        let path = std::env::temp_dir().join("gpr_test_step.wav");
        std::fs::write(&path, &wav).unwrap();
        let out = load_wav_scaled(path.to_str().unwrap(), 0.5).unwrap();
        let n = out.len();
        assert_eq!(i16::from_le_bytes([out[n - 4], out[n - 3]]), 500);
        assert_eq!(i16::from_le_bytes([out[n - 2], out[n - 1]]), -1000);
        assert!(load_wav_scaled("Cargo.toml", 1.0).is_err());
    }

    #[test]
    fn body_has_the_players_size() {
        let p = PlayerParams { height: 2.0, radius: 0.5, ..Default::default() };
        let f = faces(&p);
        let top = f.iter().flatten().map(|(v, _)| v[1]).fold(f32::MIN, f32::max);
        let bottom = f.iter().flatten().map(|(v, _)| v[1]).fold(f32::MAX, f32::min);
        assert!((top - 2.0).abs() < 0.01 && bottom.abs() < 0.01, "{bottom}..{top}");
    }

    #[test]
    fn scale_makes_the_player_bigger_everywhere() {
        let p = PlayerParams { scale: 3.0, ..Default::default() };
        let s = p.scaled();
        assert!((s.height - 5.4).abs() < 1e-4 && (s.speed - 15.0).abs() < 1e-4 && (s.max_step - 1.2).abs() < 1e-4);
        assert_eq!(s.fov, p.fov);
        let top = faces(&p).iter().flatten().map(|(v, _)| v[1]).fold(f32::MIN, f32::max);
        assert!((top - 5.4).abs() < 0.01, "{top}");
    }

    #[test]
    fn sanitize_keeps_values_in_range() {
        let mut p = PlayerParams { fov: 500.0, eye_height: 99.0, height: 1.0, step_volume: 7.0, radius: -1.0, ..Default::default() };
        p.sanitize();
        assert_eq!((p.fov, p.eye_height, p.step_volume), (170.0, 1.0, 1.0));
        assert!(p.radius > 0.0);
    }
}
