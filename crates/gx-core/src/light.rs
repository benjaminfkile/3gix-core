//! Light time: where a frame was when the light now reaching a point left
//! it.
//!
//! Implements `space-model.md` section 6.4. Light travels at
//! [`crate::gravity::C`]. The renderer draws every frame where it was when
//! the light now reaching the observer left it, and lights every frame from
//! each emitting frame's position one light-travel time earlier. Both are
//! the same equation, the light-time equation
//!
//! ```text
//! t_r = t - |x_obs - x_frame(t_r)| / c
//! ```
//!
//! for the retarded time `t_r` of a frame seen from the point `x_obs` (root
//! coordinates) at time `t`, solved against a [`History`] of the integrator.
//! Only the position of the receiving point at `t` enters, never its
//! velocity, so the solution is well defined however fast the receiving
//! point moves (fly mode may move faster than light). Gravitational bending
//! and aberration are not modeled (section 6.4).
//!
//! # Solution
//!
//! Fixed-point iteration from `t_r = t`:
//!
//! ```text
//! s_0 = t,   s_{k+1} = max(epoch, t - |x_obs - x_frame(s_k)| / c)
//! ```
//!
//! The map's Lipschitz constant is at most `v_max / c`, with `v_max` the
//! largest speed of the frame's origin over the times visited, because
//! `| |a - x(s1)| - |a - x(s2)| | <= |x(s1) - x(s2)| <= v_max |s1 - s2|`.
//! Every frame moves slower than light, so the map is a contraction for any
//! observer position, near or far: the iteration converges to the unique
//! solution from any start, and each pass shrinks the error by the factor
//! `v / c` (about `1e-4` for a fast orbit), so two to four passes reach the
//! tolerance. Clamping at the epoch keeps it a contraction.
//!
//! The iteration stops when two successive iterates differ by at most
//! [`TOLERANCE`] (1e-9 s), or by at most two units in the last place of
//! `t`, whichever is coarser: far from the origin an `f64` time cannot
//! resolve 1e-9 s (its spacing near `7e8` s is about `1.2e-7` s), and the iterates
//! then settle on the nearest representable times. It also stops after
//! [`MAX_ITERATIONS`] passes, which a frame slower than light never needs.
//!
//! Each pass reads the system at one time with [`History::peek`], which
//! integrates from the nearest checkpoint at or before it. A caller that
//! extends the history to `t` first ([`History::extend_to`]) therefore pays
//! at most one checkpoint interval of integration per pass.
//!
//! # Before the epoch
//!
//! A history holds no state before its epoch. When the light now arriving
//! left the frame before the epoch, the retarded time is clamped to the
//! epoch and [`LightTime::clamped`] reports it. The clamped fixed point is
//! exact in the sense that the unclamped solution lies before the epoch if
//! and only if the iteration settles at the epoch with the light-time map
//! there still pointing earlier.
//!
//! # Determinism
//!
//! The states come from [`History::peek`], which is a pure function of the
//! history and the time. The iteration uses `+`, `-`, `/`, and `sqrt` in a
//! fixed order with a fixed stopping rule, so results are bitwise identical
//! on every platform.

use crate::frames::FrameState;
use crate::gravity::C;
use crate::history::History;
use crate::units::{Seconds, Vec3};

/// The light-time tolerance, seconds. The iteration may stop at a coarser
/// tolerance where `f64` times cannot resolve this one (module docs).
pub const TOLERANCE: f64 = 1e-9;

/// The most fixed-point passes of one light-time solution.
pub const MAX_ITERATIONS: u32 = 32;

/// The solution of one light-time equation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LightTime {
    /// The retarded time, seconds since the origin: when the light now
    /// arriving left the frame, or the history's epoch if that was earlier.
    pub time: Seconds,
    /// Whether the retarded time lay before the history's epoch and was
    /// clamped to it.
    pub clamped: bool,
    /// The fixed-point passes taken, at most [`MAX_ITERATIONS`].
    pub iterations: u32,
}

/// A frame's state in root coordinates at its retarded time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RetardedState {
    /// The frame's root state: `position` and `velocity` of its origin in
    /// root coordinates, `orientation` of its axes relative to root axes,
    /// and `angular_velocity` about its own axes.
    pub state: FrameState,
    /// The light-time solution the state was taken at.
    pub light: LightTime,
}

impl RetardedState {
    /// The solution and the state as bytes, for comparing two runs bit for
    /// bit. Little-endian, no padding:
    ///
    /// ```text
    /// time             f64
    /// clamped          u8 (0 or 1)
    /// iterations       u32
    /// position         f64 x3
    /// velocity         f64 x3
    /// orientation      f64 x4 (x, y, z, w)
    /// angular_velocity f64 x3
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 1 + 4 + 13 * 8);
        out.extend_from_slice(&self.light.time.value().to_le_bytes());
        out.push(u8::from(self.light.clamped));
        out.extend_from_slice(&self.light.iterations.to_le_bytes());
        let s = &self.state;
        let q = s.orientation;
        let values = [
            s.position.x,
            s.position.y,
            s.position.z,
            s.velocity.x,
            s.velocity.y,
            s.velocity.z,
            q.x,
            q.y,
            q.z,
            q.w,
            s.angular_velocity.x,
            s.angular_velocity.y,
            s.angular_velocity.z,
        ];
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// A frame origin's root position at its retarded time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RetardedPoint {
    /// Position of the frame origin, meters, root coordinates.
    pub position: Vec3,
    /// The light-time solution the position was taken at.
    pub light: LightTime,
}

/// Solves the light-time equation for `frame_id` seen from `observer_root`
/// (root coordinates) at time `t` (module docs), reporting a clamp to the
/// history's epoch.
///
/// # Panics
///
/// Panics if `t` or `observer_root` is not finite, or if `frame_id` is not
/// in the history's frame tree.
pub fn retarded_time(
    observer_root: Vec3,
    t: Seconds,
    history: &History,
    frame_id: u64,
) -> LightTime {
    solve(observer_root, t, history, frame_id).0
}

/// The root state of `frame_id` at its retarded time seen from
/// `observer_root` at time `t`: what the renderer draws.
///
/// # Panics
///
/// As [`retarded_time`].
pub fn retarded_state(
    observer_root: Vec3,
    t: Seconds,
    history: &History,
    frame_id: u64,
) -> RetardedState {
    let (light, system) = solve(observer_root, t, history, frame_id);
    let state = FrameState {
        position: system.root_position(frame_id),
        velocity: system.root_velocity(frame_id),
        orientation: system.root_orientation(frame_id),
        angular_velocity: system
            .state(frame_id)
            .expect("solve checked the frame id")
            .angular_velocity,
    };
    RetardedState { state, light }
}

/// Where the emitting frame `emitter` was, in root coordinates, when the
/// light now arriving at `lit_root` (root coordinates) at time `t` left it:
/// the position a frame is lit from.
///
/// # Panics
///
/// As [`retarded_time`].
pub fn retarded_emitter(
    lit_root: Vec3,
    t: Seconds,
    history: &History,
    emitter: u64,
) -> RetardedPoint {
    let (light, system) = solve(lit_root, t, history, emitter);
    RetardedPoint {
        position: system.root_position(emitter),
        light,
    }
}

/// The light-time solution and the system at the retarded time.
fn solve(
    observer_root: Vec3,
    t: Seconds,
    history: &History,
    frame_id: u64,
) -> (LightTime, crate::frames::FrameSystem) {
    let tv = t.value();
    assert!(tv.is_finite(), "time {tv} is not finite");
    for v in [observer_root.x, observer_root.y, observer_root.z] {
        assert!(
            v.is_finite(),
            "observer position {observer_root:?} is not finite"
        );
    }
    let epoch = history.epoch().value();
    let tolerance = TOLERANCE.max(2.0 * tv.abs() * f64::EPSILON);
    // A time before the epoch is never asked of the history: the first
    // iterate is `t` itself, clamped like every later one.
    let mut s = tv.max(epoch);
    let mut system = history.peek(Seconds::new(s));
    assert!(
        system.state(frame_id).is_some(),
        "frame_id {frame_id} is not in the frame tree"
    );
    let mut clamped = tv < epoch;
    let mut iterations = 0;
    while iterations < MAX_ITERATIONS {
        iterations += 1;
        let distance = (observer_root - system.root_position(frame_id)).length();
        let unclamped = tv - distance / C;
        clamped = unclamped < epoch;
        let next = unclamped.max(epoch);
        let settled = (next - s).abs() <= tolerance;
        if next != s {
            s = next;
            system = history.peek(Seconds::new(s));
        }
        if settled {
            break;
        }
    }
    let light = LightTime {
        time: Seconds::new(s),
        clamped,
        iterations,
    };
    (light, system)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::FrameSystem;
    use crate::integrate::Scheme;
    use crate::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
    use crate::units::{Kilograms, Meters, Quat};

    fn history(position: Vec3, velocity: Vec3) -> History {
        let frame = |id, parent, position, velocity| Frame {
            frame_id: id,
            parent_frame_id: parent,
            root_extent: Meters::new(1.0e12),
            max_depth: 4,
            mass: Kilograms::new(0.0),
            position,
            velocity,
            orientation: Quat::identity(),
            angular_velocity: Vec3::new(0.0, 0.0, 1.0e-5),
            j2: 0.0,
            reference_radius: Meters::new(0.0),
            principal_moments: Vec3::zero(),
        };
        let reg = Registry::new(
            Seconds::new(0.0),
            vec![
                frame(1, ROOT_PARENT, Vec3::zero(), Vec3::zero()),
                frame(2, 1, position, velocity),
            ],
        )
        .unwrap();
        let system = FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap());
        History::new(
            system,
            Seconds::new(100.0),
            Seconds::new(50.0),
            Scheme::Yoshida4,
            16,
        )
    }

    #[test]
    fn at_the_observer_the_delay_is_zero() {
        let h = history(Vec3::new(10.0, 0.0, 0.0), Vec3::zero());
        let l = retarded_time(Vec3::new(10.0, 0.0, 0.0), Seconds::new(5.0), &h, 2);
        assert_eq!(l.time, Seconds::new(5.0));
        assert!(!l.clamped);
    }

    #[test]
    fn before_the_epoch_is_clamped() {
        let h = history(Vec3::new(3.0e9, 0.0, 0.0), Vec3::zero());
        // About 10 s of light time, asked 4 s after the epoch.
        let l = retarded_time(Vec3::zero(), Seconds::new(4.0), &h, 2);
        assert_eq!(l.time, Seconds::new(0.0));
        assert!(l.clamped);
        // Asked 20 s after the epoch the light left after it.
        let l = retarded_time(Vec3::zero(), Seconds::new(20.0), &h, 2);
        assert!(!l.clamped);
        assert!((l.time.value() - (20.0 - 3.0e9 / C)).abs() < 1e-9);
        // A time before the epoch is clamped too.
        let l = retarded_time(Vec3::zero(), Seconds::new(-5.0), &h, 2);
        assert_eq!(l.time, Seconds::new(0.0));
        assert!(l.clamped);
    }

    #[test]
    fn a_moving_frame_is_seen_where_it_was() {
        let v = Vec3::new(0.0, 3.0e4, 0.0);
        let h = history(Vec3::new(1.0e11, 0.0, 0.0), v);
        let t = Seconds::new(2000.0);
        let r = retarded_state(Vec3::zero(), t, &h, 2);
        let tr = r.light.time.value();
        // The frame moves in a straight line: x(t) = x0 + v t.
        let x = Vec3::new(1.0e11, 3.0e4 * tr, 0.0);
        assert!((r.state.position - x).length() < 1e-3);
        assert!((tr - (2000.0 - x.length() / C)).abs() < 1e-9);
        assert_eq!(r.state.velocity, v);
        let e = retarded_emitter(Vec3::zero(), t, &h, 2);
        assert_eq!(e.position, r.state.position);
        assert_eq!(e.light, r.light);
    }
}
