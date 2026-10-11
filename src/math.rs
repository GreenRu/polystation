//! Minimal column-major math (matches OpenGL's memory layout).
pub type Vec3 = [f32; 3];
pub type Mat4 = [f32; 16];

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn normalize(a: Vec3) -> Vec3 {
    let l = dot(a, a).sqrt();
    if l == 0.0 { a } else { [a[0] / l, a[1] / l, a[2] / l] }
}

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[c * 4 + k]).sum();
        }
    }
    r
}

pub fn translate(t: Vec3) -> Mat4 {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

/// Right-handed, OpenGL clip space (z in [-1, 1]).
pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy * 0.5).tan();
    let mut m = [0.0; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = (far + near) / (near - far);
    m[11] = -1.0;
    m[14] = 2.0 * far * near / (near - far);
    m
}

pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Mat4 {
    let f = normalize(sub(center, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    let mut m = [0.0; 16];
    m[0] = s[0]; m[1] = u[0]; m[2] = -f[0];
    m[4] = s[1]; m[5] = u[1]; m[6] = -f[1];
    m[8] = s[2]; m[9] = u[2]; m[10] = -f[2];
    m[12] = -dot(s, eye);
    m[13] = -dot(u, eye);
    m[14] = dot(f, eye);
    m[15] = 1.0;
    m
}

pub fn identity() -> Mat4 {
    translate([0.0; 3])
}

pub fn scale(s: Vec3) -> Mat4 {
    let mut m = identity();
    m[0] = s[0];
    m[5] = s[1];
    m[10] = s[2];
    m
}

pub fn rot_x(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    let mut m = identity();
    m[5] = c;
    m[6] = s;
    m[9] = -s;
    m[10] = c;
    m
}

pub fn rot_y(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    let mut m = identity();
    m[0] = c;
    m[2] = -s;
    m[8] = s;
    m[10] = c;
    m
}

pub fn rot_z(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    let mut m = identity();
    m[0] = c;
    m[1] = s;
    m[4] = -s;
    m[5] = c;
    m
}

fn safe_scale(s: Vec3) -> Vec3 {
    s.map(|v| if v.abs() < 1e-4 { 1e-4 } else { v })
}

/// Object transform: scale then rotate (Euler XYZ, degrees) about `pivot`, then move the pivot to `pos`.
/// The mesh data is stored in world space, so identity = (pos == pivot, no rotation, scale 1).
pub fn model_matrix(pos: Vec3, rot_deg: Vec3, sc: Vec3, pivot: Vec3) -> Mat4 {
    let r = mul(&rot_z(rot_deg[2].to_radians()), &mul(&rot_y(rot_deg[1].to_radians()), &rot_x(rot_deg[0].to_radians())));
    mul(&translate(pos), &mul(&r, &mul(&scale(safe_scale(sc)), &translate([-pivot[0], -pivot[1], -pivot[2]]))))
}

/// Exact inverse of [`model_matrix`].
pub fn inverse_model_matrix(pos: Vec3, rot_deg: Vec3, sc: Vec3, pivot: Vec3) -> Mat4 {
    let s = safe_scale(sc);
    let r_inv = mul(&rot_x(-rot_deg[0].to_radians()), &mul(&rot_y(-rot_deg[1].to_radians()), &rot_z(-rot_deg[2].to_radians())));
    mul(&translate(pivot), &mul(&scale([1.0 / s[0], 1.0 / s[1], 1.0 / s[2]]), &mul(&r_inv, &translate([-pos[0], -pos[1], -pos[2]]))))
}

pub fn transform_point(m: &Mat4, p: Vec3) -> Vec3 {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

pub fn transform_dir(m: &Mat4, d: Vec3) -> Vec3 {
    [
        m[0] * d[0] + m[4] * d[1] + m[8] * d[2],
        m[1] * d[0] + m[5] * d[1] + m[9] * d[2],
        m[2] * d[0] + m[6] * d[1] + m[10] * d[2],
    ]
}

/// Moller-Trumbore ray/triangle test, double-sided. Returns the ray parameter t > 0 of the hit.
pub fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let t0 = sub(o, a);
    let u = dot(t0, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(t0, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > 1e-6).then_some(t)
}

/// Rotation matrix for Euler angles (degrees, applied X, then Y, then Z - same order as [`model_matrix`]).
pub fn rot_matrix(rot_deg: Vec3) -> Mat4 {
    mul(&rot_z(rot_deg[2].to_radians()), &mul(&rot_y(rot_deg[1].to_radians()), &rot_x(rot_deg[0].to_radians())))
}

/// Inverse of [`rot_matrix`]: Euler angles in degrees.
pub fn euler_from_matrix(m: &Mat4) -> Vec3 {
    let r = |row: usize, col: usize| m[col * 4 + row];
    let sb = (-r(2, 0)).clamp(-1.0, 1.0);
    let beta = sb.asin();
    let (alpha, gamma) = if sb.abs() < 0.99999 {
        (r(2, 1).atan2(r(2, 2)), r(1, 0).atan2(r(0, 0)))
    } else {
        // gimbal lock: only alpha -/+ gamma is defined, put it all into alpha
        ((-r(1, 2)).atan2(r(1, 1)), 0.0)
    };
    [alpha.to_degrees(), beta.to_degrees(), gamma.to_degrees()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_model_round_trips() {
        let (pos, rot, sc, pivot) = ([1.0, 2.0, -3.0], [30.0, 45.0, 60.0], [2.0, 0.5, 1.5], [0.5, 1.0, 0.25]);
        let m = model_matrix(pos, rot, sc, pivot);
        let inv = inverse_model_matrix(pos, rot, sc, pivot);
        let p = [0.3, -1.2, 4.0];
        let back = transform_point(&inv, transform_point(&m, p));
        for k in 0..3 {
            assert!((back[k] - p[k]).abs() < 1e-4, "{back:?} vs {p:?}");
        }
        // the pivot itself lands on `pos`
        let at = transform_point(&m, pivot);
        for k in 0..3 {
            assert!((at[k] - pos[k]).abs() < 1e-5);
        }
    }

    #[test]
    fn ray_hits_triangle() {
        let t = ray_triangle([0.2, 0.2, 5.0], [0.0, 0.0, -1.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((t.unwrap() - 5.0).abs() < 1e-5);
        assert!(ray_triangle([2.0, 2.0, 5.0], [0.0, 0.0, -1.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn euler_round_trip() {
        for rot in [[10.0, 20.0, 30.0], [-120.0, 45.0, 170.0], [0.0, 0.0, 0.0], [30.0, 90.0, 0.0], [30.0, -90.0, 0.0], [200.0, -70.0, -10.0]] {
            let a = rot_matrix(rot);
            let b = rot_matrix(euler_from_matrix(&a));
            for k in 0..16 {
                assert!((a[k] - b[k]).abs() < 1e-4, "{rot:?}: {a:?} vs {b:?}");
            }
        }
    }
}
