//! A symplectic N-body integrator over the frame registry.
//!
//! Implements the laws of motion of `space-model.md` section 1 (rule 2) and
//! the time model of section 6: every frame is integrated from the epoch to
//! the simulation time under Newtonian gravity ([`crate::gravity`]) from
//! every massive frame. Gravity acts between all frames regardless of the
//! frame tree (section 5); the tree only decides how states are stored.
//!
//! Each [`step`]:
//!
//! 1. converts every frame to root coordinates (position and velocity),
//! 2. advances all frames together with the chosen [`Scheme`]: frames with a
//!    mass above zero pull on every other frame, massless frames feel gravity
//!    but exert none,
//! 3. advances every orientation by its angular velocity, held constant (no
//!    torques in v1),
//! 4. converts back to parent-relative state and advances the time.
//!
//! A root frame with zero mass is the coordinate origin of the tree (a
//! barycenter, for example) rather than a body, and is held at rest. A
//! massive root moves like every other massive frame, so momentum is
//! conserved.
//!
//! Forces are computed once per stage over all pairs: O(n^2) per stage with
//! `n` the frame count, which is fine for hundreds of frames and not meant
//! for more. Everything is `f64` with plain operators and `sqrt`, iterated by
//! ascending `frame_id`, so a run is bitwise identical on every platform.
//!
//! Not modeled: relativity, torques, collisions, and the gravity of matter
//! grids (see [`crate::gravity::acceleration_from_section`]).

use crate::frames::FrameSystem;
use crate::units::{Kilograms, Quat, Seconds, Vec3};

/// A fixed-step symplectic integration scheme.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    /// Second order kick-drift-kick velocity Verlet: two force evaluations
    /// per step.
    VelocityVerlet,
    /// Fourth order Yoshida composition of three Verlet stages: three force
    /// evaluations per step. The default.
    #[default]
    Yoshida4,
}

/// The cube root of 2, written out so no platform `cbrt` enters the
/// coefficients.
const CBRT_2: f64 = 1.259_921_049_894_873_2;

/// Yoshida's outer stage weight `1 / (2 - 2^(1/3))`.
const YOSHIDA_W1: f64 = 1.0 / (2.0 - CBRT_2);

/// Yoshida's middle stage weight `-2^(1/3) / (2 - 2^(1/3))`.
const YOSHIDA_W0: f64 = -CBRT_2 / (2.0 - CBRT_2);

/// Advances every frame by `dt` (negative to go backwards) with one step of
/// `scheme`, then advances the system time by `dt`.
pub fn step(system: &mut FrameSystem, dt: Seconds, scheme: Scheme) {
    let h = dt.value();
    let n = system.tree().frames().len();
    let masses: Vec<f64> = system
        .tree()
        .frames()
        .iter()
        .map(|f| f.mass.value())
        .collect();
    let parents = system.parent_indices().to_vec();
    let root = parents
        .iter()
        .position(Option::is_none)
        .expect("a frame tree has a root");
    let ids: Vec<u64> = system.tree().frames().iter().map(|f| f.frame_id).collect();

    let mut x: Vec<Vec3> = ids.iter().map(|&id| system.root_position(id)).collect();
    let mut v: Vec<Vec3> = ids.iter().map(|&id| system.root_velocity(id)).collect();
    // Frames that move: all of them, except a massless root.
    let moves: Vec<bool> = (0..n).map(|i| i != root || masses[i] > 0.0).collect();
    let bodies = Bodies {
        masses: &masses,
        moves: &moves,
    };

    match scheme {
        Scheme::VelocityVerlet => {
            bodies.kick(&mut v, &x, 0.5 * h);
            bodies.drift(&mut x, &v, h);
            bodies.kick(&mut v, &x, 0.5 * h);
        }
        Scheme::Yoshida4 => {
            let (w0, w1) = (YOSHIDA_W0, YOSHIDA_W1);
            bodies.drift(&mut x, &v, 0.5 * w1 * h);
            bodies.kick(&mut v, &x, w1 * h);
            bodies.drift(&mut x, &v, 0.5 * (w0 + w1) * h);
            bodies.kick(&mut v, &x, w0 * h);
            bodies.drift(&mut x, &v, 0.5 * (w0 + w1) * h);
            bodies.kick(&mut v, &x, w1 * h);
            bodies.drift(&mut x, &v, 0.5 * w1 * h);
        }
    }

    let states = system.states_mut();
    for i in 0..n {
        let s = &mut states[i];
        match parents[i] {
            Some(p) => {
                s.position = x[i] - x[p];
                s.velocity = v[i] - v[p];
            }
            None => {
                s.position = x[i];
                s.velocity = v[i];
            }
        }
        s.orientation = rotate(s.orientation, s.angular_velocity, h);
    }
    let t = system.time() + dt;
    system.set_time(t);
}

/// Advances the system from its current time to `to` in equal steps no
/// longer than `max_step`: `ceil(|to - time| / max_step)` calls to [`step`]
/// with `dt = (to - time) / count`, so there is never a short last step.
/// `to` may be earlier than the current time; the steps are then negative.
/// After the last step the time is set to exactly `to`. Does nothing if `to`
/// is the current time.
///
/// # Panics
///
/// Panics if `max_step` is not finite and above zero, or if the interval is
/// not finite.
pub fn advance(system: &mut FrameSystem, to: Seconds, max_step: Seconds, scheme: Scheme) {
    let limit = max_step.value();
    assert!(
        limit.is_finite() && limit > 0.0,
        "max_step must be finite and above zero, got {limit}"
    );
    let interval = (to - system.time()).value();
    assert!(
        interval.is_finite(),
        "interval to {} is not finite",
        to.value()
    );
    if interval == 0.0 {
        return;
    }
    let count = (interval.abs() / limit).ceil();
    let dt = Seconds::new(interval / count);
    let count = count as u64;
    for _ in 0..count {
        step(system, dt, scheme);
    }
    system.set_time(to);
}

/// The masses and the frames that move, for the stages of one step.
struct Bodies<'a> {
    masses: &'a [f64],
    moves: &'a [bool],
}

impl Bodies<'_> {
    /// `v += a(x) * h` for every frame that moves, with the forces of every
    /// massive frame computed once over all pairs.
    fn kick(&self, v: &mut [Vec3], x: &[Vec3], h: f64) {
        let sources: Vec<(Vec3, Kilograms)> = x
            .iter()
            .zip(self.masses)
            .filter(|(_, &m)| m > 0.0)
            .map(|(&p, &m)| (p, Kilograms::new(m)))
            .collect();
        for i in 0..v.len() {
            if self.moves[i] {
                v[i] = v[i] + crate::gravity::acceleration_at(x[i], &sources).scale(h);
            }
        }
    }

    /// `x += v * h` for every frame that moves.
    fn drift(&self, x: &mut [Vec3], v: &[Vec3], h: f64) {
        for i in 0..x.len() {
            if self.moves[i] {
                x[i] = x[i] + v[i].scale(h);
            }
        }
    }
}

/// One step of constant angular velocity `w` (about the frame axes) over
/// `h` seconds: `normalize(q + 0.5 * h * q * (0, w))`.
fn rotate(q: Quat, w: Vec3, h: f64) -> Quat {
    let dq = q * Quat::new(w.x, w.y, w.z, 0.0);
    let k = 0.5 * h;
    let next = Quat::new(
        q.x + k * dq.x,
        q.y + k * dq.y,
        q.z + k * dq.z,
        q.w + k * dq.w,
    );
    next.normalized().unwrap_or(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yoshida_weights_sum_to_one() {
        assert!((2.0 * YOSHIDA_W1 + YOSHIDA_W0 - 1.0).abs() < 1e-15);
        assert!((CBRT_2 * CBRT_2 * CBRT_2 - 2.0).abs() < 1e-15);
        assert_eq!(Scheme::default(), Scheme::Yoshida4);
    }

    #[test]
    fn zero_rate_keeps_orientation() {
        let q = Quat::new(0.0, 0.6, 0.0, 0.8);
        assert_eq!(rotate(q, Vec3::zero(), 10.0), q);
    }
}
