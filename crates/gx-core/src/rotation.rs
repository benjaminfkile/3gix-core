//! Rotation: Euler's equations for a rigid frame under the torques other
//! masses exert on its shape, and constant spin for a frame without moments.
//!
//! Implements `space-model.md` section 6.2. A frame whose
//! `principal_moments` (`matter-format.md` section 5.2) are all above zero
//! is a rigid body with inertia tensor `I = diag(A, B, C)` in its own axes.
//! Its angular momentum in its own axes is `L = I w` and it obeys Euler's
//! equations
//!
//! ```text
//! dL/dt = L x w + tau
//! ```
//!
//! with `w` its angular velocity about its own axes and `tau` the torque in
//! its own axes. A frame whose moments are all zero keeps the constant
//! angular velocity it was given.
//!
//! # Angular velocity and orientation
//!
//! `angular_velocity` is the frame's angular velocity relative to inertial
//! (root) axes, expressed in the frame's own axes. The orientation therefore
//! evolves as `dQ/dt = Q * (0, w) / 2` for the frame's root orientation `Q`
//! (frame axes relative to root axes). The integrator advances `Q` and
//! stores the orientation relative to the parent's new axes, so a frame does
//! not inherit its parent's spin. Every orientation stays a unit quaternion,
//! renormalized every step.
//!
//! # Torque
//!
//! [`torque`] is MacCullagh's formula, the torque of a point mass `m` at `r`
//! from the frame's origin on the frame's second moments:
//!
//! ```text
//! tau = 3 G m / |r|^5 * r x (I r)          (r and tau in the frame's axes)
//! ```
//!
//! For a frame symmetric about its z axis (`A = B`) this is
//! `3 G m (C - A) (r . z)(r x z) / |r|^5`, the torque on the oblate shape
//! whose force is [`crate::gravity::oblateness_accelerations`] with
//! `j2 = (C - A) / (M R^2)`. Unequal `A` and `B` add the torque about the z
//! axis that holds a prolate frame pointed at the mass it orbits. The
//! translational force of that second, triaxial part is not modeled; its
//! effect on an orbit is smaller than the oblateness force by the ratio
//! `(B - A) / (C - A)` and is out of scope.
//!
//! # Free rotation
//!
//! [`free_rotation`] advances a rigid frame with no torque for a time `h`
//! with the splitting of the kinetic energy
//!
//! ```text
//! T = |L|^2 / (2 I_r) + L_a^2 / 2 (1/I_a - 1/I_r) + L_b^2 / 2 (1/I_b - 1/I_r)
//! ```
//!
//! into flows that are each a rotation about a fixed axis (Dullweber, Reich
//! and Leimkuhler 1997; Touma and Wisdom 1994): the first rotates the frame
//! about `L` at rate `|L| / I_r` and leaves `L` fixed in the frame, the
//! others rotate the frame about its own axis `a` (or `b`) at rate
//! `L_a (1/I_a - 1/I_r)` and turn `L` the opposite way about that axis. The
//! step is `a(h/2) b(h) a(h/2)` followed by the first flow for `h`, which
//! commutes with both. The reference axis `r` is chosen as one of a pair of
//! equal moments when there is one, so the `a` flow vanishes and the step is
//! the exact solution for a frame symmetric about an axis; for a frame with
//! three different moments it is second order and conserves `|L|` and the
//! energy to rounding.
//!
//! # Determinism
//!
//! Only `+`, `-`, `*`, `/`, `sqrt`, and the written-out sine and cosine of
//! the private `detmath` module are used, so every result is bitwise
//! identical on every platform.

use crate::detmath;
use crate::gravity::G;
use crate::units::{Kilograms, Quat, Vec3};

/// Whether the moments describe a rigid frame whose spin evolves: all three
/// above zero. A registry frame has either all three above zero or all
/// three zero (`matter-format.md` section 5.2).
pub fn has_moments(moments: Vec3) -> bool {
    moments.x > 0.0 && moments.y > 0.0 && moments.z > 0.0
}

/// The unit quaternion of a rotation by `|v|` radians about `v`:
/// `(sin(|v|/2) v / |v|, cos(|v|/2))`, the identity for a zero vector.
pub fn rotation_quat(v: Vec3) -> Quat {
    let angle = v.length();
    if angle == 0.0 {
        return Quat::identity();
    }
    let half = 0.5 * angle;
    let s = detmath::sin(half) / angle;
    Quat::new(v.x * s, v.y * s, v.z * s, detmath::cos(half))
}

/// The orientation `q` (frame axes relative to root axes) after `h` seconds
/// of constant angular velocity `w` about the frame's own axes:
/// `normalize(q * rotation_quat(w h))`. Exact for constant `w`.
pub fn advance_orientation(q: Quat, w: Vec3, h: f64) -> Quat {
    renormalize(q * rotation_quat(w.scale(h)), q)
}

/// The torque, in N m and in the frame's own axes, that point masses exert
/// on a frame with principal moments `moments` (kg m^2 about its own axes),
/// root orientation `orientation`, and origin `position` (root coordinates).
/// `sources` are positions in root coordinates with masses, summed in slice
/// order; a source at exactly `position` is skipped. Formula in the module
/// docs.
pub fn torque(
    moments: Vec3,
    orientation: Quat,
    position: Vec3,
    sources: &[(Vec3, Kilograms)],
) -> Vec3 {
    let to_body = orientation.conjugate();
    let mut tau = Vec3::zero();
    for &(p, m) in sources {
        let r = p - position;
        let r2 = r.length_squared();
        if r2 == 0.0 {
            continue;
        }
        let rb = to_body.rotate(r);
        let ir = Vec3::new(moments.x * rb.x, moments.y * rb.y, moments.z * rb.z);
        let r5 = r2 * r2 * r2.sqrt();
        tau = tau + rb.cross(ir).scale(3.0 * G * m.value() / r5);
    }
    tau
}

/// Advances a rigid frame with no torque for `h` seconds: its root
/// orientation `q` and its angular momentum `l` in its own axes (kg m^2/s),
/// with principal moments `moments` (all above zero). Returns the new
/// orientation and angular momentum. The splitting is in the module docs.
pub fn free_rotation(q: Quat, l: Vec3, moments: Vec3, h: f64) -> (Quat, Vec3) {
    let i = [moments.x, moments.y, moments.z];
    let (r, a, b) = if i[0] == i[1] {
        (0, 1, 2)
    } else if i[1] == i[2] {
        (1, 2, 0)
    } else if i[0] == i[2] {
        (0, 2, 1)
    } else {
        (0, 1, 2)
    };
    let inv_r = 1.0 / i[r];
    let mut q = q;
    let mut l = l;
    for (axis, t) in [(a, 0.5 * h), (b, h), (a, 0.5 * h)] {
        let rate = 1.0 / i[axis] - inv_r;
        if rate == 0.0 {
            continue;
        }
        let angle = component(l, axis) * rate * t;
        let turn = rotation_quat(unit(axis).scale(angle));
        l = turn.conjugate().rotate(l);
        q = renormalize(q * turn, q);
    }
    q = renormalize(q * rotation_quat(l.scale(inv_r * h)), q);
    (q, l)
}

/// Component `k` (0, 1, 2 for x, y, z) of `v`.
fn component(v: Vec3, k: usize) -> f64 {
    match k {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// The unit vector along axis `k` (0, 1, 2 for x, y, z).
fn unit(k: usize) -> Vec3 {
    match k {
        0 => Vec3::new(1.0, 0.0, 0.0),
        1 => Vec3::new(0.0, 1.0, 0.0),
        _ => Vec3::new(0.0, 0.0, 1.0),
    }
}

/// `q` scaled to unit length, or `fallback` if `q` has no length.
fn renormalize(q: Quat, fallback: Quat) -> Quat {
    q.normalized().unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3, tol: f64) -> bool {
        (a - b).length() <= tol
    }

    #[test]
    fn rotation_quat_quarter_turn() {
        let q = rotation_quat(Vec3::new(0.0, 0.0, core::f64::consts::FRAC_PI_2));
        assert!(close(
            q.rotate(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(0.0, 1.0, 0.0),
            1e-15
        ));
        assert_eq!(rotation_quat(Vec3::zero()), Quat::identity());
    }

    #[test]
    fn zero_rate_keeps_orientation() {
        let q = Quat::new(0.0, 0.6, 0.0, 0.8);
        assert_eq!(advance_orientation(q, Vec3::zero(), 10.0), q);
        assert!(!has_moments(Vec3::zero()));
        assert!(has_moments(Vec3::new(1.0, 1.0, 1.5)));
    }

    #[test]
    fn symmetric_free_rotation_is_exact() {
        // A frame symmetric about z with a tilted angular momentum: the
        // body-frame momentum turns about z at (1/C - 1/A) L_z and the
        // inertial momentum stays fixed.
        let m = Vec3::new(2.0, 2.0, 3.0);
        let l0 = Vec3::new(0.3, 0.0, 6.0);
        let q0 = Quat::new(0.1, 0.2, 0.3, 0.9).normalized().unwrap();
        let fixed = q0.rotate(l0);
        let mut q = q0;
        let mut l = l0;
        for _ in 0..1000 {
            (q, l) = free_rotation(q, l, m, 0.37);
        }
        let angle: f64 = -(1.0 / 3.0 - 1.0 / 2.0) * 6.0 * 370.0;
        let expect = Vec3::new(0.3 * angle.cos(), 0.3 * angle.sin(), 6.0);
        assert!(close(l, expect, 1e-11), "{l:?} {expect:?}");
        assert!(close(q.rotate(l), fixed, 1e-11));
        // One long step equals many short ones.
        let (q1, l1) = free_rotation(q0, l0, m, 370.0);
        assert!(close(l1, l, 1e-10));
        assert!(close(
            q1.rotate(Vec3::new(1.0, 0.0, 0.0)),
            q.rotate(Vec3::new(1.0, 0.0, 0.0)),
            1e-10
        ));
    }

    #[test]
    fn triaxial_free_rotation_conserves() {
        let m = Vec3::new(1.0, 2.0, 3.0);
        let l0 = Vec3::new(0.5, 0.1, 2.0);
        let energy = |l: Vec3| 0.5 * (l.x * l.x / m.x + l.y * l.y / m.y + l.z * l.z / m.z);
        let q0 = Quat::identity();
        let fixed = q0.rotate(l0);
        let (mut q, mut l) = (q0, l0);
        for _ in 0..10_000 {
            (q, l) = free_rotation(q, l, m, 0.01);
        }
        assert!((l.length() - l0.length()).abs() < 1e-13);
        assert!(close(q.rotate(l), fixed, 1e-12));
        assert!((energy(l) - energy(l0)).abs() / energy(l0) < 1e-5);
    }

    #[test]
    fn torque_on_symmetric_frame() {
        // A mass off the xy plane of a frame symmetric about z: the torque
        // is the closed form 3 G m (C - A)(r . z)(r x z) / r^5.
        let m = Vec3::new(4.0e37, 4.0e37, 4.1e37);
        let r = Vec3::new(3.0e8, 0.0, 4.0e8);
        let src = [(r, Kilograms::new(7.0e22))];
        let tau = torque(m, Quat::identity(), Vec3::zero(), &src);
        let z = Vec3::new(0.0, 0.0, 1.0);
        let rl = r.length();
        let expect = r
            .cross(z)
            .scale(3.0 * G * 7.0e22 * 0.1e37 * r.dot(z) / (rl * rl * rl * rl * rl));
        assert!(close(tau, expect, 1e-12 * expect.length()));
        assert_eq!(torque(m, Quat::identity(), r, &src), Vec3::zero());
    }
}
