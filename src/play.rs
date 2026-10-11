//! Play mode: a first-person walker driven by the Player object's settings. The scene is flattened to
//! world-space triangles in an xz grid; the walker only needs ray casts against it (down for the
//! ground, sideways for walls), so it works on terrain, imported models and primitives alike.
use crate::math;
use crate::player::PlayerParams;
use std::collections::HashMap;

pub type Tri = [[f32; 3]; 3];

const CELL: f32 = 2.0;
/// A triangle that would fill more cells than this is tested by every query instead.
const MAX_CELLS_PER_TRI: i64 = 2048;

pub struct World {
    tris: Vec<Tri>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    big: Vec<u32>,
}

fn cell_of(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl World {
    pub fn new(tris: Vec<Tri>) -> World {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut big = Vec::new();
        for (i, t) in tris.iter().enumerate() {
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in t {
                for (k, a) in [v[0], v[2]].into_iter().enumerate() {
                    lo[k] = lo[k].min(a);
                    hi[k] = hi[k].max(a);
                }
            }
            if !lo.iter().chain(hi.iter()).all(|v| v.is_finite()) {
                continue;
            }
            let (x0, x1, z0, z1) = (cell_of(lo[0]), cell_of(hi[0]), cell_of(lo[1]), cell_of(hi[1]));
            if (x1 - x0 + 1) as i64 * (z1 - z0 + 1) as i64 > MAX_CELLS_PER_TRI {
                big.push(i as u32);
                continue;
            }
            for cx in x0..=x1 {
                for cz in z0..=z1 {
                    cells.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        World { tris, cells, big }
    }

    /// Distance along the ray to the nearest triangle (two-sided), if within `max_t`.
    pub fn ray(&self, o: [f32; 3], d: [f32; 3], max_t: f32) -> Option<f32> {
        let end = [o[0] + d[0] * max_t, o[2] + d[2] * max_t];
        let (x0, x1) = (cell_of(o[0].min(end[0])), cell_of(o[0].max(end[0])));
        let (z0, z1) = (cell_of(o[2].min(end[1])), cell_of(o[2].max(end[1])));
        let mut best: Option<f32> = None;
        let mut test = |i: u32| {
            let t = &self.tris[i as usize];
            if let Some(h) = math::ray_triangle(o, d, t[0], t[1], t[2]) {
                if h >= 0.0 && h <= max_t && best.is_none_or(|b| h < b) {
                    best = Some(h);
                }
            }
        };
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if let Some(list) = self.cells.get(&(cx, cz)) {
                    list.iter().for_each(|&i| test(i));
                }
            }
        }
        self.big.iter().for_each(|&i| test(i));
        best
    }

    /// Every triangle that might touch the xz box (each once).
    fn candidates(&self, lo: [f32; 2], hi: [f32; 2]) -> Vec<u32> {
        let mut out: Vec<u32> = self.big.clone();
        for cx in cell_of(lo[0])..=cell_of(hi[0]) {
            for cz in cell_of(lo[1])..=cell_of(hi[1]) {
                if let Some(list) = self.cells.get(&(cx, cz)) {
                    out.extend_from_slice(list);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Height of the highest surface at or below `from_y` under (x, z).
    pub fn ground(&self, x: f32, z: f32, from_y: f32) -> Option<f32> {
        self.ray([x, from_y, z], [0.0, -1.0, 0.0], 1.0e6).map(|t| from_y - t)
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// The point of triangle `t` nearest to `p`.
fn closest_on_triangle(p: [f32; 3], t: &Tri) -> [f32; 3] {
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (math::sub(b, a), math::sub(c, a), math::sub(p, a));
    let (d1, d2) = (math::dot(ab, ap), math::dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = math::sub(p, b);
    let (d3, d4) = (math::dot(ab, bp), math::dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, scale(ab, d1 / (d1 - d3)));
    }
    let cp = math::sub(p, c);
    let (d5, d6) = (math::dot(ab, cp), math::dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(b, scale(math::sub(c, b), w));
    }
    let denom = 1.0 / (va + vb + vc);
    add(a, add(scale(ab, vb * denom), scale(ac, vc * denom)))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StepResult {
    /// A footstep sound is due.
    pub stepped: bool,
    pub landed: bool,
    pub jumped: bool,
}

pub struct Walker {
    /// Position of the feet.
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub on_ground: bool,
    bob_phase: f32,
    bob_amp: f32,
    step_dist: f32,
}

impl Walker {
    pub fn new(pos: [f32; 3]) -> Walker {
        Walker { pos, vel: [0.0; 3], on_ground: false, bob_phase: 0.0, bob_amp: 0.0, step_dist: 0.0 }
    }

    /// Camera position: eye height plus the walking sway.
    pub fn eye(&self, p: &PlayerParams) -> [f32; 3] {
        [self.pos[0], self.pos[1] + p.eye_height + self.bob_phase.sin() * p.bob_amount * self.bob_amp, self.pos[2]]
    }

    /// Heights (above the feet) of the spheres the body is made of: a column of spheres of the body radius
    /// from the step height up to the head, so walls, thin posts, overhangs and ceilings all push back.
    /// The lowest one starts above `max_step`, so ledges that low are stepped onto instead of blocking.
    fn sphere_heights(p: &PlayerParams) -> Vec<f32> {
        let r = p.radius.max(0.01);
        let low = (p.max_step + 0.02 + r).min(p.height - r);
        let top = p.height - r;
        if top <= low {
            return vec![p.height * 0.5];
        }
        let n = ((top - low) / r).ceil().max(1.0) as usize;
        (0..=n).map(|i| low + (top - low) * i as f32 / n as f32).collect()
    }

    /// Pushes the body out of every nearby triangle (a few passes, so corners settle). Sliding: the speed
    /// into a wall is removed, along it is kept. Returns true when the head hit a ceiling.
    fn collide(&mut self, p: &PlayerParams, w: &World, tris: &[u32], heights: &[f32]) -> bool {
        let r = p.radius.max(0.01);
        let mut ceiling = false;
        for _ in 0..5 {
            let mut pushed = false;
            for &h in heights {
                let c = [self.pos[0], self.pos[1] + h, self.pos[2]];
                for &i in tris {
                    let t = &w.tris[i as usize];
                    let (lo, hi) = (t[0][1].min(t[1][1]).min(t[2][1]), t[0][1].max(t[1][1]).max(t[2][1]));
                    if hi < c[1] - r || lo > c[1] + r {
                        continue;
                    }
                    let q = closest_on_triangle(c, t);
                    let d = math::sub(c, q);
                    let dist2 = math::dot(d, d);
                    if dist2 >= r * r {
                        continue;
                    }
                    let dist = dist2.sqrt();
                    let n = if dist > 1e-5 {
                        scale(d, 1.0 / dist)
                    } else {
                        // the centre is on the surface: use the face normal, towards the player's side of it
                        let fnm = math::normalize(math::cross(math::sub(t[1], t[0]), math::sub(t[2], t[0])));
                        if math::dot(fnm, math::sub(c, t[0])) < 0.0 { scale(fnm, -1.0) } else { fnm }
                    };
                    self.pos = add(self.pos, scale(n, r - dist + 1e-3));
                    let walkable = n[1] > 0.7; // ground: the ground logic owns the vertical speed
                    let into = math::dot(self.vel, n);
                    if into < 0.0 && !walkable {
                        self.vel = math::sub(self.vel, scale(n, into));
                    }
                    if n[1] < -0.6 {
                        ceiling = true;
                    }
                    pushed = true;
                }
            }
            if !pushed {
                break;
            }
        }
        ceiling
    }

    /// Advances by `dt`. `input` = (forward, right) in -1..1; `yaw` as in the editor camera.
    /// `water` = how deep the player's body is in water (0 = dry).
    pub fn step(&mut self, p: &PlayerParams, input: (f32, f32), yaw: f32, sprint: bool, jump: bool, water: f32, dt: f32, w: &World) -> StepResult {
        let mut res = StepResult::default();
        let (s, c) = yaw.sin_cos();
        let mut wish = [s * input.0 + c * input.1, -c * input.0 + s * input.1];
        let l = wish[0].hypot(wish[1]);
        if l > 1.0 {
            wish = [wish[0] / l, wish[1] / l];
        }
        let wet = (water / p.height.max(0.01)).clamp(0.0, 1.0);
        let swimming = wet > 0.55;
        let speed = p.speed * if sprint { p.sprint_mult } else { 1.0 } * (1.0 - 0.5 * wet);
        let k = if self.on_ground { 1.0 } else { p.air_control };
        let a = (k * 14.0 * dt).min(1.0);
        self.vel[0] += (wish[0] * speed - self.vel[0]) * a;
        self.vel[2] += (wish[1] * speed - self.vel[2]) * a;
        // the walls: move in small substeps (never more than a fraction of the body radius at a time) and
        // push the body out of whatever it ran into after each one
        let heights = Self::sphere_heights(p);
        let reach = p.radius.max(0.01) + self.vel[0].abs().max(self.vel[2].abs()) * dt + p.height + 1.0;
        let tris = w.candidates([self.pos[0] - reach, self.pos[2] - reach], [self.pos[0] + reach, self.pos[2] + reach]);
        let moved = self.vel[0].abs().max(self.vel[2].abs()) * dt;
        let subs = (moved / (0.4 * p.radius.max(0.01))).ceil().clamp(1.0, 16.0) as usize;
        for _ in 0..subs {
            self.pos[0] += self.vel[0] * dt / subs as f32;
            self.pos[2] += self.vel[2] * dt / subs as f32;
            self.collide(p, w, &tris, &heights);
        }

        if jump && self.on_ground {
            self.vel[1] = (2.0 * p.gravity.max(0.0) * p.jump_height).sqrt();
            self.on_ground = false;
            res.jumped = true;
        }
        let old_y = self.pos[1];
        let gravity = if swimming { p.gravity * 0.08 } else { p.gravity * (1.0 - 0.5 * wet) };
        if swimming {
            self.vel[1] *= (1.0 - 3.0 * dt).max(0.0); // water drag
            if jump {
                self.vel[1] = (self.vel[1] + p.speed * 4.0 * dt).min(p.speed * 0.9); // holding jump swims up
            }
        }
        self.vel[1] -= gravity * dt;
        self.pos[1] += self.vel[1] * dt;
        if self.vel[1] <= 0.0 {
            match w.ground(self.pos[0], self.pos[2], old_y + p.max_step) {
                Some(h) if self.pos[1] <= h || (self.on_ground && self.pos[1] - h <= p.max_step) => {
                    res.landed = !self.on_ground;
                    self.pos[1] = h;
                    self.vel[1] = 0.0;
                    self.on_ground = true;
                }
                _ => self.on_ground = false,
            }
        } else {
            self.on_ground = false;
        }

        if self.collide(p, w, &tris, &heights) && self.vel[1] > 0.0 {
            self.vel[1] = 0.0; // bumped the head
        }

        let hs = self.vel[0].hypot(self.vel[2]);
        let moving = self.on_ground && hs > 0.1;
        self.bob_amp += ((moving as u8 as f32) - self.bob_amp) * (10.0 * dt).min(1.0);
        if moving {
            self.bob_phase += dt * p.bob_speed * std::f32::consts::TAU * (hs / p.speed.max(0.01)).min(2.0);
            self.step_dist += hs * dt;
            if self.step_dist >= p.step_distance {
                self.step_dist -= p.step_distance;
                res.stepped = true;
            }
        }
        res.stepped |= res.landed;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> Vec<Tri> {
        vec![[a, b, c], [a, c, d]]
    }

    fn floor() -> Vec<Tri> {
        quad([-50.0, 0.0, -50.0], [50.0, 0.0, -50.0], [50.0, 0.0, 50.0], [-50.0, 0.0, 50.0])
    }

    fn run(wk: &mut Walker, w: &World, p: &PlayerParams, secs: f32, input: (f32, f32), yaw: f32, jump_first: bool) {
        let dt = 1.0 / 120.0;
        for i in 0..(secs / dt) as usize {
            wk.step(p, input, yaw, false, jump_first && i == 0, 0.0, dt, w);
        }
    }

    #[test]
    fn falls_and_lands_on_the_floor() {
        let w = World::new(floor());
        let p = PlayerParams::default();
        let mut wk = Walker::new([0.0, 3.0, 0.0]);
        run(&mut wk, &w, &p, 2.0, (0.0, 0.0), 0.0, false);
        assert!(wk.on_ground && wk.pos[1].abs() < 1e-3, "{:?}", wk.pos);
    }

    #[test]
    fn jump_reaches_the_set_height() {
        let w = World::new(floor());
        let p = PlayerParams { jump_height: 1.5, gravity: 12.0, ..Default::default() };
        let mut wk = Walker::new([0.0, 0.0, 0.0]);
        run(&mut wk, &w, &p, 0.5, (0.0, 0.0), 0.0, false);
        let mut peak = 0.0f32;
        for i in 0..240 {
            wk.step(&p, (0.0, 0.0), 0.0, false, i == 0, 0.0, 1.0 / 240.0, &w);
            peak = peak.max(wk.pos[1]);
        }
        assert!((peak - 1.5).abs() < 0.08, "peak {peak}");
    }

    #[test]
    fn walks_at_the_set_speed_and_sprints_faster() {
        let w = World::new(floor());
        let p = PlayerParams { speed: 4.0, ..Default::default() };
        let mut wk = Walker::new([0.0, 0.0, 0.0]);
        run(&mut wk, &w, &p, 0.5, (0.0, 0.0), 0.0, false);
        let z0 = wk.pos[2];
        run(&mut wk, &w, &p, 2.0, (1.0, 0.0), 0.0, false); // yaw 0 looks down -Z
        let walked = z0 - wk.pos[2];
        assert!((walked - 8.0).abs() < 1.0, "walked {walked}");
    }

    #[test]
    fn walls_block_and_low_ledges_are_stepped_over() {
        let mut tris = floor();
        // wall across x = 5, 3 tall
        tris.extend(quad([5.0, 0.0, -10.0], [5.0, 0.0, 10.0], [5.0, 3.0, 10.0], [5.0, 3.0, -10.0]));
        let w = World::new(tris);
        let p = PlayerParams::default();
        let mut wk = Walker::new([0.0, 0.0, 0.0]);
        run(&mut wk, &w, &p, 3.0, (1.0, 0.0), std::f32::consts::FRAC_PI_2, false); // yaw pi/2 looks along +X
        assert!(wk.pos[0] < 5.0 - p.radius + 0.05 && wk.pos[0] > 3.5, "x = {}", wk.pos[0]);

        // a 0.3 high ledge (below max_step) is climbed, a 1.0 high one is not
        for (h, climbs) in [(0.3, true), (1.0, false)] {
            let mut t = floor();
            t.extend(quad([5.0, h, -10.0], [20.0, h, -10.0], [20.0, h, 10.0], [5.0, h, 10.0])); // top
            t.extend(quad([5.0, 0.0, -10.0], [5.0, 0.0, 10.0], [5.0, h, 10.0], [5.0, h, -10.0])); // face
            let w = World::new(t);
            let mut wk = Walker::new([0.0, 0.0, 0.0]);
            run(&mut wk, &w, &p, 4.0, (1.0, 0.0), std::f32::consts::FRAC_PI_2, false);
            assert_eq!(wk.pos[0] > 6.0, climbs, "ledge {h}: x = {}", wk.pos[0]);
            if climbs {
                assert!((wk.pos[1] - h).abs() < 0.01);
            }
        }
    }

    #[test]
    fn slides_along_a_slanted_wall_instead_of_sticking() {
        let mut tris = floor();
        // a wall at 45 degrees (the line x = z + 4): walking +X runs into it and should slide along it, towards +Z
        tris.extend(quad([4.0, 0.0, 0.0], [4.0 + 20.0, 0.0, 20.0], [4.0 + 20.0, 4.0, 20.0], [4.0, 4.0, 0.0]));
        tris.extend(quad([4.0, 0.0, 0.0], [4.0 - 20.0, 0.0, -20.0], [4.0 - 20.0, 4.0, -20.0], [4.0, 4.0, 0.0]));
        let w = World::new(tris);
        let p = PlayerParams::default();
        let mut wk = Walker::new([0.0, 0.0, 0.0]);
        run(&mut wk, &w, &p, 3.0, (1.0, 0.0), std::f32::consts::FRAC_PI_2, false);
        assert!(wk.pos[0] - wk.pos[2] < 4.0, "stayed on this side of the wall: {:?}", wk.pos);
        assert!(wk.pos[2] > 2.0, "slid along the wall: {:?}", wk.pos);
    }

    #[test]
    fn a_thin_post_stops_the_body() {
        let mut tris = floor();
        // a post 0.1 wide, 3 tall, at x = 4 (narrower than the gaps a ray test could miss)
        for (a, b) in [([4.0, 0.0, -0.05], [4.0, 0.0, 0.05]), ([4.1, 0.0, -0.05], [4.1, 0.0, 0.05])] {
            tris.extend(quad(a, b, [b[0], 3.0, b[2]], [a[0], 3.0, a[2]]));
        }
        tris.extend(quad([4.0, 0.0, -0.05], [4.1, 0.0, -0.05], [4.1, 3.0, -0.05], [4.0, 3.0, -0.05]));
        tris.extend(quad([4.0, 0.0, 0.05], [4.1, 0.0, 0.05], [4.1, 3.0, 0.05], [4.0, 3.0, 0.05]));
        let w = World::new(tris);
        let p = PlayerParams::default();
        // aim a little off-centre: a single centre ray would miss the post, the body must not
        let mut wk = Walker::new([0.0, 0.0, 0.2]);
        for _ in 0..360 {
            wk.step(&p, (1.0, 0.0), std::f32::consts::FRAC_PI_2, false, false, 0.0, 1.0 / 120.0, &w);
            // distance from the body axis to the post's footprint
            let (dx, dz) = ((4.0 - wk.pos[0]).max(wk.pos[0] - 4.1).max(0.0), (wk.pos[2].abs() - 0.05).max(0.0));
            assert!(dx.hypot(dz) >= p.radius - 0.03, "body inside the post at {:?}", wk.pos);
        }
        assert!(wk.pos[0] > 4.5, "slid around the post instead of getting stuck: {:?}", wk.pos);
    }

    #[test]
    fn a_ceiling_stops_a_jump_and_a_low_gap_is_closed() {
        let mut tris = floor();
        // ceiling at 2.2 over x in [3, 20]
        tris.extend(quad([3.0, 2.2, -10.0], [20.0, 2.2, -10.0], [20.0, 2.2, 10.0], [3.0, 2.2, 10.0]));
        let w = World::new(tris);
        let p = PlayerParams { jump_height: 3.0, ..Default::default() };
        let mut wk = Walker::new([6.0, 0.0, 0.0]);
        let mut peak = 0.0f32;
        for i in 0..480 {
            wk.step(&p, (0.0, 0.0), 0.0, false, i == 120, 0.0, 1.0 / 240.0, &w);
            peak = peak.max(wk.pos[1] + p.height);
        }
        assert!(peak < 2.2 + 0.05, "head went through the ceiling: {peak}");
        assert!(wk.on_ground && wk.pos[1].abs() < 0.01, "back on the floor: {:?}", wk.pos);
        // a gap lower than the player cannot be entered
        let mut low = floor();
        low.extend(quad([3.0, 1.0, -10.0], [20.0, 1.0, -10.0], [20.0, 1.0, 10.0], [3.0, 1.0, 10.0]));
        let w = World::new(low);
        let mut wk = Walker::new([0.0, 0.0, 0.0]);
        run(&mut wk, &w, &p, 3.0, (1.0, 0.0), std::f32::consts::FRAC_PI_2, false);
        assert!(wk.pos[0] < 3.0, "walked under a ceiling lower than the body: {:?}", wk.pos);
    }

    #[test]
    fn closest_point_on_a_triangle_covers_face_edge_and_corner() {
        let t: Tri = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 2.0]];
        assert_eq!(closest_on_triangle([0.5, 1.0, 0.5], &t), [0.5, 0.0, 0.5]);
        assert_eq!(closest_on_triangle([-1.0, 0.0, -1.0], &t), [0.0, 0.0, 0.0]);
        assert_eq!(closest_on_triangle([3.0, 0.0, 3.0], &t), [1.0, 0.0, 1.0]);
        assert_eq!(closest_on_triangle([1.0, 0.0, -1.0], &t), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn water_slows_the_player_and_lets_them_swim_up() {
        let w = World::new(floor());
        let p = PlayerParams::default();
        // wading: slower than on dry land
        let (mut dry, mut wet) = (Walker::new([0.0, 0.0, 0.0]), Walker::new([0.0, 0.0, 0.0]));
        for _ in 0..240 {
            dry.step(&p, (1.0, 0.0), 0.0, false, false, 0.0, 1.0 / 120.0, &w);
            wet.step(&p, (1.0, 0.0), 0.0, false, false, 1.0, 1.0 / 120.0, &w);
        }
        assert!(dry.pos[2].abs() > wet.pos[2].abs() * 1.2, "{} vs {}", dry.pos[2], wet.pos[2]);
        // deep water: holding jump swims up, and without it the player barely sinks
        let mut s = Walker::new([0.0, 0.0, 0.0]);
        for _ in 0..120 {
            s.step(&p, (0.0, 0.0), 0.0, false, true, p.height, 1.0 / 120.0, &w);
        }
        assert!(s.pos[1] > 1.0, "swam up to {}", s.pos[1]);
        let mut f = Walker::new([0.0, 5.0, 0.0]);
        for _ in 0..60 {
            f.step(&p, (0.0, 0.0), 0.0, false, false, p.height, 1.0 / 120.0, &w);
        }
        assert!(f.pos[1] > 4.5, "sank too fast: {}", f.pos[1]);
    }

    #[test]
    fn footsteps_follow_distance_and_bob_only_when_walking() {
        let w = World::new(floor());
        let p = PlayerParams { speed: 5.0, step_distance: 2.0, bob_amount: 0.1, ..Default::default() };
        let mut wk = Walker::new([0.0, 0.0, 0.0]);
        run(&mut wk, &w, &p, 0.5, (0.0, 0.0), 0.0, false);
        assert!((wk.eye(&p)[1] - p.eye_height).abs() < 1e-4, "no sway while standing");
        let (mut steps, dt) = (0, 1.0 / 120.0);
        let mut max_off = 0.0f32;
        for _ in 0..(2.0 / dt) as usize {
            steps += wk.step(&p, (1.0, 0.0), 0.0, false, false, 0.0, dt, &w).stepped as u32;
            max_off = max_off.max((wk.eye(&p)[1] - p.eye_height).abs());
        }
        assert!((4..=6).contains(&steps), "{steps} steps in ~10 units");
        assert!(max_off > 0.03 && max_off <= 0.1001, "{max_off}");
    }
}
