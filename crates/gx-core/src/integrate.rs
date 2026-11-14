//! The N-body and rotation integrator over the frame registry.
//!
//! Implements the laws of motion and rotation of `space-model.md` sections
//! 6.1 and 6.2 and the integration from the epoch of section 7: every frame
//! is integrated under the gravity of every massive frame ([`crate::gravity`])
//! and every rigid frame's spin under the torques on its shape
//! ([`crate::rotation`]). Gravity acts between all frames regardless of the
//! frame tree (section 5); the tree only decides how states are stored.
//!
//! Each [`step`]:
//!
//! 1. converts every frame to root coordinates: position, velocity, and
//!    orientation (frame axes relative to root axes),
//! 2. advances all frames together with the chosen [`Scheme`]: frames with a
//!    mass above zero pull on every other frame, massless frames feel gravity
//!    but exert none,
//! 3. converts back to parent-relative state (the orientation relative to
//!    the parent's new axes, renormalized) and advances the time.
//!
//! A root frame with zero mass is the coordinate origin of the tree (a
//! barycenter, for example) rather than a body, and is held at rest. A
//! massive root moves like every other massive frame, so momentum is
//! conserved.
//!
//! # Schemes
//!
//! [`Scheme::VelocityVerlet`] and [`Scheme::Yoshida4`] are the Newtonian
//! point-mass schemes of v1: symplectic kick-drift-kick, second and fourth
//! order. Under them a rigid frame turns freely (Euler's equations with no
//! torque) and a frame without moments keeps its constant spin.
//!
//! [`Scheme::Relativistic`], the default, and [`Scheme::Oblate`] split the
//! dynamics into the Newtonian point-mass part `N` and a perturbation `P`:
//! the first post-Newtonian corrections (`Relativistic` only), the
//! oblateness force, and the torques on rigid frames. One step of length `h`
//! is the symmetric composition
//!
//! ```text
//! P(h/2)  then  N(h) by Yoshida4, with free rotation for h  then  P(h/2)
//! ```
//!
//! where `P(h/2)` holds positions and orientations fixed, kicks every
//! rigid frame's angular momentum by `h/2` times its torque, and kicks every
//! velocity by `h/2` times the perturbing acceleration. The first
//! post-Newtonian acceleration depends on velocity, so that kick is the
//! implicit midpoint rule `v1 = v0 + h/2 * P(x, (v0 + v1) / 2)`, solved by
//! fixed-point iteration from `v1 = v0` until an iterate repeats bit for bit
//! or after [`KICK_ITERATIONS`] iterations. The perturbation is about `1e-8`
//! of the Newtonian acceleration at the scales in scope, so the iteration
//! contracts by that factor and converges in two or three passes.
//!
//! Every stage is time symmetric, so the step is time symmetric and second
//! order. The Newtonian part keeps the symplectic fourth order Yoshida
//! error, and the splitting adds an error of order `eps h^2` with `eps` the
//! ratio of the perturbation to the Newtonian acceleration: the local error
//! per step is `O(h^5) + O(eps h^3)` and the global error over a fixed time
//! `O(h^4) + O(eps h^2)`. Because `eps` is small the splitting error stays
//! below the Yoshida error for the steps the renderer uses.
//!
//! Forces are computed once per stage over all pairs: O(n^2) per stage with
//! `n` the frame count, which is fine for hundreds of frames and not meant
//! for more. Everything is `f64` with plain operators, `sqrt`, and the
//! written-out sine and cosine of [`crate::rotation`], iterated by ascending
//! `frame_id`, so a run is bitwise identical on every platform.
//!
//! Not modeled: collisions, and the gravity of matter grids (see
//! [`crate::gravity::acceleration_from_section`]).

use crate::frames::FrameSystem;
use crate::gravity::{self, Body};
use crate::rotation;
use crate::units::{Kilograms, Quat, Seconds, Vec3};

/// A fixed-step integration scheme.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    /// Newtonian point masses only, second order kick-drift-kick velocity
    /// Verlet: two force evaluations per step. No torques.
    VelocityVerlet,
    /// Newtonian point masses only, fourth order Yoshida composition of
    /// three Verlet stages: three force evaluations per step. No torques.
    Yoshida4,
    /// Newtonian point masses by [`Scheme::Yoshida4`], with oblateness and
    /// torques as a perturbation kick (module docs). No relativistic terms.
    Oblate,
    /// Newtonian point masses by [`Scheme::Yoshida4`], with the first
    /// post-Newtonian corrections, oblateness, and torques as a perturbation
    /// kick (module docs). The default.
    #[default]
    Relativistic,
}

/// The most fixed-point iterations of one velocity-dependent kick.
pub const KICK_ITERATIONS: usize = 8;

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
    let frames = system.tree().frames();
    let n = frames.len();
    let masses: Vec<f64> = frames.iter().map(|f| f.mass.value()).collect();
    let j2: Vec<f64> = frames.iter().map(|f| f.j2).collect();
    let radii: Vec<f64> = frames.iter().map(|f| f.reference_radius.value()).collect();
    let moments: Vec<Vec3> = frames.iter().map(|f| f.principal_moments).collect();
    let rigid: Vec<bool> = moments.iter().map(|&m| rotation::has_moments(m)).collect();
    let ids: Vec<u64> = frames.iter().map(|f| f.frame_id).collect();
    let parents = system.parent_indices().to_vec();
    let root = parents
        .iter()
        .position(Option::is_none)
        .expect("a frame tree has a root");

    let mut x: Vec<Vec3> = ids.iter().map(|&id| system.root_position(id)).collect();
    let mut v: Vec<Vec3> = ids.iter().map(|&id| system.root_velocity(id)).collect();
    let mut q: Vec<Quat> = ids.iter().map(|&id| system.root_orientation(id)).collect();
    // Angular velocity about the frame axes; for a rigid frame, its angular
    // momentum in its own axes while the step runs.
    let mut w: Vec<Vec3> = (0..n)
        .map(|i| {
            let spin = system.state(ids[i]).expect("own id").angular_velocity;
            if rigid[i] {
                mul(moments[i], spin)
            } else {
                spin
            }
        })
        .collect();
    // Frames that move: all of them, except a massless root.
    let moves: Vec<bool> = (0..n).map(|i| i != root || masses[i] > 0.0).collect();
    let bodies = Bodies {
        masses: &masses,
        moves: &moves,
        j2: &j2,
        radii: &radii,
        moments: &moments,
        rigid: &rigid,
    };

    let perturbation = match scheme {
        Scheme::VelocityVerlet | Scheme::Yoshida4 => None,
        Scheme::Oblate => Some(false),
        Scheme::Relativistic => Some(true),
    };
    if let Some(relativistic) = perturbation {
        bodies.perturb(&x, &mut v, &q, &mut w, 0.5 * h, relativistic);
    }
    match scheme {
        Scheme::VelocityVerlet => {
            bodies.kick(&mut v, &x, 0.5 * h);
            bodies.drift(&mut x, &v, h);
            bodies.kick(&mut v, &x, 0.5 * h);
        }
        Scheme::Yoshida4 | Scheme::Oblate | Scheme::Relativistic => {
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
    for i in 0..n {
        if rigid[i] {
            (q[i], w[i]) = rotation::free_rotation(q[i], w[i], moments[i], h);
        } else {
            q[i] = rotation::advance_orientation(q[i], w[i], h);
        }
    }
    if let Some(relativistic) = perturbation {
        bodies.perturb(&x, &mut v, &q, &mut w, 0.5 * h, relativistic);
    }

    let states = system.states_mut();
    for i in 0..n {
        let s = &mut states[i];
        match parents[i] {
            Some(p) => {
                s.position = x[i] - x[p];
                s.velocity = v[i] - v[p];
                s.orientation = (q[p].conjugate() * q[i])
                    .normalized()
                    .unwrap_or(s.orientation);
            }
            None => {
                s.position = x[i];
                s.velocity = v[i];
                s.orientation = q[i];
            }
        }
        s.angular_velocity = if rigid[i] {
            div(w[i], moments[i])
        } else {
            w[i]
        };
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

/// The per-frame constants of one step, in ascending `frame_id` order.
struct Bodies<'a> {
    masses: &'a [f64],
    moves: &'a [bool],
    j2: &'a [f64],
    radii: &'a [f64],
    moments: &'a [Vec3],
    rigid: &'a [bool],
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

    /// The perturbation kick of the module docs over `h` seconds at fixed
    /// positions `x` and root orientations `q`: every rigid frame's angular
    /// momentum `l` (in its own axes) gains `h` times its torque, and every
    /// moving frame's velocity gains `h` times the oblateness acceleration
    /// plus, when `relativistic`, the first post-Newtonian correction at the
    /// midpoint velocity.
    fn perturb(
        &self,
        x: &[Vec3],
        v: &mut [Vec3],
        q: &[Quat],
        l: &mut [Vec3],
        h: f64,
        relativistic: bool,
    ) {
        let sources: Vec<(Vec3, Kilograms)> = x
            .iter()
            .zip(self.masses)
            .filter(|(_, &m)| m > 0.0)
            .map(|(&p, &m)| (p, Kilograms::new(m)))
            .collect();
        for i in 0..l.len() {
            if self.rigid[i] {
                let tau = rotation::torque(self.moments[i], q[i], x[i], &sources);
                l[i] = l[i] + tau.scale(h);
            }
        }

        let z = Vec3::new(0.0, 0.0, 1.0);
        let mut bodies: Vec<Body> = (0..x.len())
            .map(|i| Body {
                position: x[i],
                velocity: v[i],
                mass: Kilograms::new(self.masses[i]),
                j2: self.j2[i],
                reference_radius: self.radii[i],
                axis: q[i].rotate(z),
            })
            .collect();
        let oblate = gravity::oblateness_accelerations(&bodies);
        let v0 = v.to_vec();
        if !relativistic {
            for i in 0..v.len() {
                if self.moves[i] {
                    v[i] = v0[i] + oblate[i].scale(h);
                }
            }
            return;
        }
        let mut v1 = v0.clone();
        for _ in 0..KICK_ITERATIONS {
            for (b, (a, c)) in bodies.iter_mut().zip(v0.iter().zip(&v1)) {
                b.velocity = (*a + *c).scale(0.5);
            }
            let pn = gravity::relativistic_corrections(&bodies);
            let next: Vec<Vec3> = (0..v.len())
                .map(|i| {
                    if self.moves[i] {
                        v0[i] + (pn[i] + oblate[i]).scale(h)
                    } else {
                        v0[i]
                    }
                })
                .collect();
            let done = next == v1;
            v1 = next;
            if done {
                break;
            }
        }
        v.copy_from_slice(&v1);
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

/// Componentwise product.
fn mul(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

/// Componentwise quotient.
fn div(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x / b.x, a.y / b.y, a.z / b.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yoshida_weights_sum_to_one() {
        assert!((2.0 * YOSHIDA_W1 + YOSHIDA_W0 - 1.0).abs() < 1e-15);
        assert!((CBRT_2 * CBRT_2 * CBRT_2 - 2.0).abs() < 1e-15);
        assert_eq!(Scheme::default(), Scheme::Relativistic);
    }
}
