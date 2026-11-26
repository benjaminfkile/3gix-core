//! Tests for light time (`space-model.md` section 6.4): a frame at rest
//! 1.13e11 m away, and a frame in a circular orbit drawn at its
//! retarded position, with the tolerances the task sets. Every body is
//! made up.

use gx_core::frames::FrameSystem;
use gx_core::gravity::{C, G};
use gx_core::history::History;
use gx_core::integrate::Scheme;
use gx_core::light::{retarded_emitter, retarded_state, retarded_time};
use gx_core::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};

/// The test distance, meters: a made-up value.
const D: f64 = 1.13e11;

/// A made-up unit of time in seconds for the orbit samples: 21 hours.
const UNIT: f64 = 75_600.0;

fn frame(id: u64, parent: u64, mass: f64, position: Vec3, velocity: Vec3) -> Frame {
    Frame {
        frame_id: id,
        parent_frame_id: parent,
        root_extent: Meters::new(1.0e13),
        max_depth: 8,
        mass: Kilograms::new(mass),
        position,
        velocity,
        orientation: Quat::identity(),
        angular_velocity: Vec3::zero(),
        j2: 0.0,
        reference_radius: Meters::new(0.0),
        principal_moments: Vec3::zero(),
    }
}

/// A history of `frames` in a registry with epoch 0, the origin.
fn history(frames: Vec<Frame>, interval: f64, max_step: f64) -> History {
    let reg = Registry::new(Seconds::new(0.0), frames).unwrap();
    let system = FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap());
    History::new(
        system,
        Seconds::new(interval),
        Seconds::new(max_step),
        Scheme::Yoshida4,
        256,
    )
}

#[test]
fn a_frame_at_rest_far_away() {
    let h = history(
        vec![
            frame(1, ROOT_PARENT, 0.0, Vec3::zero(), Vec3::zero()),
            frame(2, 1, 0.0, Vec3::new(D, 0.0, 0.0), Vec3::zero()),
        ],
        3600.0,
        600.0,
    );
    let t = 1000.0;
    let l = retarded_time(Vec3::zero(), Seconds::new(t), &h, 2);
    assert!(!l.clamped);
    let delay = t - l.time.value();
    // D / c = 376.927428 s.
    assert!((delay - D / C).abs() < 1e-6, "delay {delay} s");
    assert!((delay - 376.927).abs() < 1e-3);
    let r = retarded_state(Vec3::zero(), Seconds::new(t), &h, 2);
    assert_eq!(r.state.position, Vec3::new(D, 0.0, 0.0));
}

#[test]
fn a_frame_in_circular_orbit_is_drawn_where_it_was() {
    let mass = 2.7e30;
    let r = D;
    let mu = G * mass;
    let speed = (mu / r).sqrt();
    let omega = speed / r;
    let mut h = history(
        vec![
            frame(1, ROOT_PARENT, mass, Vec3::zero(), Vec3::zero()),
            frame(
                2,
                1,
                0.0,
                Vec3::new(r, 0.0, 0.0),
                Vec3::new(0.0, speed, 0.0),
            ),
        ],
        3600.0,
        300.0,
    );
    let analytic = |t: f64| Vec3::new(r * (omega * t).cos(), r * (omega * t).sin(), 0.0);
    let observer = Vec3::new(-1.0e11, 2.0e11, 3.0e10);
    for units in [2.0, 10.0, 30.0] {
        let t = units * UNIT;
        // As the renderer does: checkpoints up to now, so each light-time
        // pass integrates at most one interval.
        h.extend_to(Seconds::new(t));
        // The analytic retarded time, by the same fixed point on the
        // analytic orbit, iterated well past convergence.
        let mut tr = t;
        for _ in 0..50 {
            tr = t - (observer - analytic(tr)).length() / C;
        }
        let want = analytic(tr);
        let got = retarded_state(observer, Seconds::new(t), &h, 2);
        let miss = (got.state.position - want).length();
        assert!(
            miss < 1.0,
            "unit {units}: {miss} m from the analytic position"
        );
        assert!((got.light.time.value() - tr).abs() < 1e-6);
        assert!(got.light.iterations <= 6, "{} passes", got.light.iterations);
        // Lighting uses the same solution.
        let lit = retarded_emitter(observer, Seconds::new(t), &h, 2);
        assert_eq!(lit.position, got.state.position);
    }
}
