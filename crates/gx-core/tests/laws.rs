//! Orbit tests for the laws of motion: frames, gravity, and the integrator
//! (`space-model.md` sections 5 and 6), with the tolerances the task sets.

use core::f64::consts::PI;

use gx_core::frames::{FrameState, FrameSystem};
use gx_core::gravity::G;
use gx_core::integrate::{advance, Scheme};
use gx_core::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};

const M: f64 = 1.0e30;
const R: f64 = 1.0e11;

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
    }
}

fn system(frames: Vec<Frame>) -> FrameSystem {
    let reg = Registry::new(Seconds::new(0.0), frames).unwrap();
    FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap())
}

fn period(r: f64, mass: f64) -> f64 {
    2.0 * PI * (r * r * r / (G * mass)).sqrt()
}

/// A root of mass `M` and a 1 kg child on a circular orbit of radius `R`.
fn two_body() -> FrameSystem {
    let speed = (G * M / R).sqrt();
    system(vec![
        frame(1, ROOT_PARENT, M, Vec3::zero(), Vec3::zero()),
        frame(
            2,
            1,
            1.0,
            Vec3::new(R, 0.0, 0.0),
            Vec3::new(0.0, speed, 0.0),
        ),
    ])
}

/// Kinetic plus pairwise potential energy of every frame, in root
/// coordinates.
fn total_energy(s: &FrameSystem) -> f64 {
    let frames = s.tree().frames();
    let mut e = 0.0;
    for (i, a) in frames.iter().enumerate() {
        let ma = a.mass.value();
        e += 0.5 * ma * s.root_velocity(a.frame_id).length_squared();
        for b in &frames[i + 1..] {
            let r = (s.root_position(b.frame_id) - s.root_position(a.frame_id)).length();
            e -= G * ma * b.mass.value() / r;
        }
    }
    e
}

fn total_momentum(s: &FrameSystem) -> Vec3 {
    s.tree().frames().iter().fold(Vec3::zero(), |p, f| {
        p + s.root_velocity(f.frame_id).scale(f.mass.value())
    })
}

#[test]
fn two_body_circular_orbit_yoshida4() {
    let mut s = two_body();
    let t = period(R, M);
    let start = s.root_position(2);
    let e0 = total_energy(&s);
    advance(
        &mut s,
        Seconds::new(10.0 * t),
        Seconds::new(t / 2000.0),
        Scheme::Yoshida4,
    );
    assert_eq!(s.time(), Seconds::new(10.0 * t));
    let err = (s.root_position(2) - start).length();
    assert!(err < 1e-6 * R, "position error {err} m");
    let drift = ((total_energy(&s) - e0) / e0).abs();
    assert!(drift < 1e-9, "relative energy drift {drift}");
}

#[test]
fn two_body_circular_orbit_velocity_verlet() {
    let mut s = two_body();
    let t = period(R, M);
    let start = s.root_position(2);
    advance(
        &mut s,
        Seconds::new(10.0 * t),
        Seconds::new(t / 2000.0),
        Scheme::VelocityVerlet,
    );
    let err = (s.root_position(2) - start).length();
    assert!(err < 1e-3 * R, "position error {err} m");
}

const CHILD_MASS: f64 = 1.0e25;
const R2: f64 = 1.0e7;

/// Root, a child of mass `CHILD_MASS` on a circular orbit of radius `R`, and a
/// massless grandchild on a circular orbit of radius `R2` around the child.
fn hierarchy() -> FrameSystem {
    let v1 = (G * (M + CHILD_MASS) / R).sqrt();
    let v2 = (G * CHILD_MASS / R2).sqrt();
    let mut child = frame(
        2,
        1,
        CHILD_MASS,
        Vec3::new(R, 0.0, 0.0),
        Vec3::new(0.0, v1, 0.0),
    );
    child.angular_velocity = Vec3::new(1.0e-5, -2.0e-5, 7.0e-5);
    system(vec![
        frame(1, ROOT_PARENT, M, Vec3::zero(), Vec3::zero()),
        child,
        frame(3, 2, 0.0, Vec3::new(0.0, R2, 0.0), Vec3::new(0.0, 0.0, v2)),
    ])
}

#[test]
fn three_level_hierarchy() {
    let mut s = hierarchy();
    let t2 = period(R2, CHILD_MASS);
    let start = s.state(3).unwrap().position;
    advance(
        &mut s,
        Seconds::new(t2),
        Seconds::new(t2 / 2000.0),
        Scheme::Yoshida4,
    );
    let rel = s.state(3).unwrap().position;
    let err = (rel - start).length();
    assert!(err < 1e-4 * R2, "relative position error {err} m");
    assert_eq!(s.root_position(3), s.root_position(2) + rel);
    // The child has moved a long way along its own orbit meanwhile.
    assert!((s.root_position(2) - Vec3::new(R, 0.0, 0.0)).length() > 1.0e7);
}

#[test]
fn two_body_momentum_is_conserved() {
    let m = 1.0e29;
    let speed = (G * (M + m) / R).sqrt();
    let mut s = system(vec![
        frame(1, ROOT_PARENT, M, Vec3::zero(), Vec3::zero()),
        frame(2, 1, m, Vec3::new(R, 0.0, 0.0), Vec3::new(0.0, speed, 0.0)),
    ]);
    let t = period(R, M + m);
    let p0 = total_momentum(&s);
    advance(
        &mut s,
        Seconds::new(10.0 * t),
        Seconds::new(t / 2000.0),
        Scheme::Yoshida4,
    );
    // The root has moved: it is a massive body, not a fixed point.
    assert!(s.root_position(1).length() > 1.0e9);
    let err = (total_momentum(&s) - p0).length() / p0.length();
    assert!(err < 1e-9, "relative momentum error {err}");
}

#[test]
fn constant_rotation_quarter_turn() {
    let mut root = frame(1, ROOT_PARENT, 0.0, Vec3::zero(), Vec3::zero());
    root.angular_velocity = Vec3::new(0.0, 0.0, 2.0 * PI / 86400.0);
    let mut s = system(vec![root]);
    advance(
        &mut s,
        Seconds::new(21600.0),
        Seconds::new(0.5),
        Scheme::Yoshida4,
    );
    let p = s.from_frame(Vec3::new(1.0, 0.0, 0.0), 1) - s.root_position(1);
    let err = (p - Vec3::new(0.0, 1.0, 0.0)).length();
    assert!(err < 1e-9, "rotation error {err}");
}

#[test]
fn floating_origin_casts_to_f32() {
    let child_pos = Vec3::new(1.234_567_890_123e11, -4.567_890_123_4e10, 2.345_678_901e10);
    let gc_pos = Vec3::new(1_234.567_8, -987.654_321, 42.424_242);
    let gc_q = Quat::new(0.36, 0.48, 0.0, 0.8);
    let mut gc = frame(3, 2, 0.0, gc_pos, Vec3::new(0.1, -0.2, 0.3));
    gc.orientation = gc_q;
    let mut s = system(vec![
        frame(1, ROOT_PARENT, M, Vec3::zero(), Vec3::zero()),
        frame(2, 1, 0.0, child_pos, Vec3::new(-1.2e4, 3.0e4, 5.0e3)),
        gc,
    ]);
    let point = Vec3::new(0.125, -3.75, 2.5);
    let camera = Vec3::zero();

    let check = |s: &FrameSystem, expect: Vec3| {
        assert!(s.root_position(3).length() > 1.0e11);
        let v = s.relative(point, 3, camera, 2);
        let cast = [v.x as f32, v.y as f32, v.z as f32];
        for (c, e) in cast.iter().zip([expect.x, expect.y, expect.z]) {
            let err = (f64::from(*c) - e).abs();
            assert!(err < 1e-3, "f32 error {err} m");
        }
    };

    // At the epoch, against the offset computed from the inputs alone.
    check(&s, gc_pos + gc_q.rotate(point));

    // After some integration, against the same offset from the new states.
    advance(
        &mut s,
        Seconds::new(600.0),
        Seconds::new(10.0),
        Scheme::Yoshida4,
    );
    let st: FrameState = *s.state(3).unwrap();
    check(&s, st.position + st.orientation.rotate(point));
}

#[test]
fn integration_is_deterministic() {
    let run = || {
        let mut s = hierarchy();
        advance(
            &mut s,
            Seconds::new(5.0e4),
            Seconds::new(100.0),
            Scheme::Yoshida4,
        );
        advance(
            &mut s,
            Seconds::new(2.0e4),
            Seconds::new(70.0),
            Scheme::VelocityVerlet,
        );
        s
    };
    let bits = |s: &FrameSystem| -> Vec<u64> {
        let mut out = vec![s.time().value().to_bits()];
        for f in s.tree().frames() {
            let st = s.state(f.frame_id).unwrap();
            for v in [st.position, st.velocity, st.angular_velocity] {
                out.extend([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
            }
            let q = st.orientation;
            out.extend([q.x.to_bits(), q.y.to_bits(), q.z.to_bits(), q.w.to_bits()]);
        }
        out
    };
    let (a, b) = (run(), run());
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(a, b);
}
