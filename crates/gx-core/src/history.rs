//! Checkpoints of the frame system, so a past state costs at most one
//! checkpoint interval of integration.
//!
//! Implements `space-model.md` section 7 item 6 ("the renderer keeps
//! checkpoints so a jump back costs at most one checkpoint interval of
//! integration") and serves the light-time solution of section 6.4, which
//! needs the states of frames at retarded times.
//!
//! A [`History`] starts from a [`FrameSystem`] at its current time, the
//! history's epoch (normally the registry epoch, the universe's origin).
//! Checkpoint `k` is the full system at `epoch + k * interval`, produced by
//! integrating checkpoint `k - 1` forward over one interval with
//! [`crate::integrate::advance`]. [`History::state_at`] clones the nearest
//! checkpoint at or before the requested time and integrates forward from
//! it, adding checkpoints first if the time lies beyond the last one.
//!
//! # Memory bound
//!
//! A history never holds more than `capacity` checkpoints. When adding one
//! would exceed that, every checkpoint with an odd index is dropped and the
//! interval doubles, so the survivors are again `interval` apart from the
//! epoch. Memory is therefore at most `capacity` copies of the frame system
//! (each about 400 bytes per frame: its state, its registry record, and its
//! place in the tree), and the integration cost of a jump to any time
//! between the epoch and the last checkpoint is at most one current
//! interval, which is the initial interval times `2^d` after `d` doublings.
//! With the initial interval `I` and capacity `K`, `d` stays 0 while the
//! history covers less than `K * I` seconds and grows by one each time the
//! covered span doubles.
//!
//! # Determinism
//!
//! Checkpoint times are `epoch + k * interval` computed in `f64`; doubling
//! the interval is exact, so a surviving checkpoint keeps its time bit for
//! bit. Every state is a pure function of the starting system, the initial
//! interval, the capacity, the step limit, the scheme, and the order of the
//! calls that extended the history.

use crate::frames::FrameSystem;
use crate::integrate::{advance, Scheme};
use crate::units::Seconds;

/// Checkpoints of a frame system at fixed intervals from its epoch (module
/// docs).
#[derive(Clone, Debug)]
pub struct History {
    /// `checkpoints[k]` is the system at `epoch + k * interval`.
    checkpoints: Vec<FrameSystem>,
    epoch: Seconds,
    interval: Seconds,
    max_step: Seconds,
    scheme: Scheme,
    capacity: usize,
}

impl History {
    /// A history whose epoch is the system's current time, with checkpoints
    /// every `interval` seconds, integrated in steps no longer than
    /// `max_step` with `scheme`, holding at most `capacity` checkpoints.
    ///
    /// # Panics
    ///
    /// Panics if `interval` or `max_step` is not finite and above zero, or
    /// if `capacity` is below 2.
    pub fn new(
        system: FrameSystem,
        interval: Seconds,
        max_step: Seconds,
        scheme: Scheme,
        capacity: usize,
    ) -> History {
        for (what, s) in [("interval", interval), ("max_step", max_step)] {
            let v = s.value();
            assert!(
                v.is_finite() && v > 0.0,
                "{what} must be finite and above zero, got {v}"
            );
        }
        assert!(capacity >= 2, "capacity must be at least 2, got {capacity}");
        History {
            epoch: system.time(),
            checkpoints: vec![system],
            interval,
            max_step,
            scheme,
            capacity,
        }
    }

    /// The time of checkpoint 0, seconds since the origin.
    pub fn epoch(&self) -> Seconds {
        self.epoch
    }

    /// The current spacing of the checkpoints, seconds. Doubles each time
    /// the history thins itself.
    pub fn interval(&self) -> Seconds {
        self.interval
    }

    /// The most checkpoints the history holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The number of checkpoints held, at least 1.
    pub fn len(&self) -> usize {
        self.checkpoints.len()
    }

    /// Always false: a history holds at least its epoch checkpoint.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The time of the last checkpoint.
    pub fn latest(&self) -> Seconds {
        self.checkpoint_time(self.checkpoints.len() - 1)
    }

    /// The checkpoints, in time order, checkpoint `k` at
    /// `epoch + k * interval`.
    pub fn checkpoints(&self) -> &[FrameSystem] {
        &self.checkpoints
    }

    /// Adds checkpoints until the last one is at or after `t` minus one
    /// interval, that is, until a checkpoint exists at or before `t` with no
    /// further checkpoint due before `t`. Does nothing for `t` before the
    /// next checkpoint time.
    ///
    /// # Panics
    ///
    /// Panics if `t` is not finite.
    pub fn extend_to(&mut self, t: Seconds) {
        assert!(t.value().is_finite(), "time {} is not finite", t.value());
        loop {
            let next = self.checkpoint_time(self.checkpoints.len());
            if next > t {
                return;
            }
            let mut s = self
                .checkpoints
                .last()
                .expect("a history holds at least one checkpoint")
                .clone();
            advance(&mut s, next, self.max_step, self.scheme);
            self.checkpoints.push(s);
            if self.checkpoints.len() > self.capacity {
                self.thin();
            }
        }
    }

    /// The system at time `t`: the nearest checkpoint at or before `t`,
    /// integrated forward to `t`. Checkpoints are added first if `t` lies
    /// beyond the last one. A time before the epoch is integrated backwards
    /// from the epoch checkpoint.
    ///
    /// # Panics
    ///
    /// Panics if `t` is not finite.
    pub fn state_at(&mut self, t: Seconds) -> FrameSystem {
        self.extend_to(t);
        let mut s = self.checkpoints[self.index_at_or_before(t)].clone();
        advance(&mut s, t, self.max_step, self.scheme);
        s
    }

    /// The system at time `t` from the checkpoints already held, without
    /// adding any: the nearest checkpoint at or before `t`, integrated
    /// forward to `t` (backwards from the epoch checkpoint for a time before
    /// the epoch). Gives the same result as [`History::state_at`] for any
    /// `t` at or before [`History::latest`] plus one interval; beyond that
    /// it integrates from the last checkpoint, which costs more but is
    /// still a pure function of the history and `t`. This is what the
    /// light-time solution of [`crate::light`] reads through a shared
    /// reference.
    ///
    /// # Panics
    ///
    /// Panics if `t` is not finite.
    pub fn peek(&self, t: Seconds) -> FrameSystem {
        assert!(t.value().is_finite(), "time {} is not finite", t.value());
        let mut s = self.checkpoints[self.index_at_or_before(t)].clone();
        advance(&mut s, t, self.max_step, self.scheme);
        s
    }

    /// The time of checkpoint `k`.
    fn checkpoint_time(&self, k: usize) -> Seconds {
        Seconds::new(self.epoch.value() + k as f64 * self.interval.value())
    }

    /// The index of the last checkpoint at or before `t`, or 0 for a time
    /// before the epoch.
    fn index_at_or_before(&self, t: Seconds) -> usize {
        let last = self.checkpoints.len() - 1;
        let raw = ((t - self.epoch).value() / self.interval.value()).floor();
        if raw.is_nan() || raw <= 0.0 {
            return 0;
        }
        let mut k = if raw >= last as f64 {
            last
        } else {
            raw as usize
        };
        while k > 0 && self.checkpoint_time(k) > t {
            k -= 1;
        }
        while k < last && self.checkpoint_time(k + 1) <= t {
            k += 1;
        }
        k
    }

    /// Drops every checkpoint with an odd index and doubles the interval.
    fn thin(&mut self) {
        let mut k = 0;
        self.checkpoints.retain(|_| {
            let keep = k % 2 == 0;
            k += 1;
            keep
        });
        self.interval = Seconds::new(self.interval.value() * 2.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
    use crate::units::{Kilograms, Meters, Quat, Vec3};

    fn system() -> FrameSystem {
        let frame = |id, parent, mass, x: f64, v: f64| Frame {
            frame_id: id,
            parent_frame_id: parent,
            root_extent: Meters::new(1.0e12),
            max_depth: 4,
            mass: Kilograms::new(mass),
            position: Vec3::new(x, 0.0, 0.0),
            velocity: Vec3::new(0.0, v, 0.0),
            orientation: Quat::identity(),
            angular_velocity: Vec3::zero(),
            j2: 0.0,
            reference_radius: Meters::new(0.0),
            principal_moments: Vec3::zero(),
        };
        let reg = Registry::new(
            Seconds::new(0.0),
            vec![
                frame(1, ROOT_PARENT, 1.0e30, 0.0, 0.0),
                frame(2, 1, 0.0, 1.0e11, 2.6e4),
            ],
        )
        .unwrap();
        FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap())
    }

    #[test]
    fn thinning_keeps_capacity_and_times() {
        let mut h = History::new(
            system(),
            Seconds::new(1000.0),
            Seconds::new(500.0),
            Scheme::Yoshida4,
            4,
        );
        assert_eq!(h.epoch(), Seconds::new(0.0));
        h.extend_to(Seconds::new(3999.0));
        assert_eq!(h.len(), 4);
        assert_eq!(h.interval(), Seconds::new(1000.0));
        h.extend_to(Seconds::new(4000.0));
        assert_eq!(h.len(), 3);
        assert_eq!(h.interval(), Seconds::new(2000.0));
        h.extend_to(Seconds::new(50_000.0));
        assert!(h.len() <= 4);
        for (k, c) in h.checkpoints().iter().enumerate() {
            assert_eq!(c.time(), Seconds::new(k as f64 * h.interval().value()));
        }
        assert!(h.latest() <= Seconds::new(50_000.0));
        assert!(h.latest() + h.interval() > Seconds::new(50_000.0));
    }

    #[test]
    fn state_at_checkpoint_and_before_epoch() {
        let mut h = History::new(
            system(),
            Seconds::new(1000.0),
            Seconds::new(250.0),
            Scheme::Relativistic,
            8,
        );
        let at = h.state_at(Seconds::new(2000.0));
        assert_eq!(at, h.checkpoints()[2]);
        assert_eq!(h.peek(Seconds::new(2000.0)), at);
        assert_eq!(
            h.peek(Seconds::new(2500.0)),
            h.state_at(Seconds::new(2500.0))
        );
        let before = h.state_at(Seconds::new(-50.0));
        assert_eq!(before.time(), Seconds::new(-50.0));
        assert!(!h.is_empty());
        assert_eq!(h.capacity(), 8);
    }
}
