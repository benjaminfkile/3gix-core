//! Tests for the v2 laws (`space-model.md` sections 6.1, 6.2, and 7 item 6):
//! oblateness, rotation under torques, and the checkpoint history, with the
//! tolerances the task sets, on made-up bodies. Scenarios are named by
//! number; scenarios 1 and 3 compare with real measurements and live in
//! `laws_measured.rs`.

use core::f64::consts::PI;

use gx_core::frames::FrameSystem;
use gx_core::gravity::G;
use gx_core::history::History;
use gx_core::integrate::{advance, step, Scheme};
use gx_core::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};

/// A made-up unit of time in seconds for the history tests: 21 hours.
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

fn system(frames: Vec<Frame>) -> FrameSystem {
    let reg = Registry::new(Seconds::new(0.0), frames).unwrap();
    FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap())
}

/// Least-squares slope of `y` against `x`.
fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for &(x, y) in points {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    sxy / sxx
}

/// `angle` moved by whole turns to within half a turn of `previous`.
fn unwrap(previous: f64, angle: f64) -> f64 {
    angle - 2.0 * PI * ((angle - previous) / (2.0 * PI)).round()
}

/// A rotation by `angle` radians about the unit vector `axis`.
fn axis_angle(axis: Vec3, angle: f64) -> Quat {
    let s = (0.5 * angle).sin();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, (0.5 * angle).cos())
}
// Scenario 2: node precession of a low orbit around an oblate frame.

#[test]
fn scenario_2_node_precession() {
    let mass = 4.1e24;
    let radius = 5.71e6;
    let j2 = 1.37e-3;
    let a = 6.3e6;
    let inc = 45.0f64.to_radians();
    let gm = G * mass;
    let speed = (gm / a).sqrt();
    let mut root = frame(1, ROOT_PARENT, mass, Vec3::zero(), Vec3::zero());
    root.j2 = j2;
    root.reference_radius = Meters::new(radius);
    let mut s = system(vec![
        root,
        frame(
            2,
            1,
            0.0,
            Vec3::new(a, 0.0, 0.0),
            Vec3::new(0.0, speed * inc.cos(), speed * inc.sin()),
        ),
    ]);
    let n = (gm / (a * a * a)).sqrt();
    let period = 2.0 * PI / n;
    let dt = period / 200.0;
    let mut points = Vec::new();
    let mut last = 0.0;
    for _ in 0..(200 * 150) {
        step(&mut s, Seconds::new(dt), Scheme::default());
        let r = s.root_position(2) - s.root_position(1);
        let v = s.root_velocity(2) - s.root_velocity(1);
        let h = r.cross(v);
        let node = unwrap(last, h.x.atan2(-h.y));
        last = node;
        points.push((s.time().value(), node));
    }
    let rate = slope(&points);
    let expect = -1.5 * n * j2 * (radius / a) * (radius / a) * inc.cos();
    let err = (rate - expect).abs() / expect.abs();
    assert!(
        err < 0.01,
        "node rate {rate} rad/s, analytic {expect}, error {err}"
    );
}
// A made-up spinning system: a central mass, an oblate rigid body on a
// circular orbit, and its companion. The values match no real body.

const M_CENTRAL: f64 = 2.7e30;
const D_CENTRAL: f64 = 1.13e11;
const M_BODY: f64 = 4.1e24;
const R_BODY: f64 = 5.71e6;
const M_COMPANION: f64 = 5.6e22;
const D_COMPANION: f64 = 2.9e8;

/// A massless root (the barycenter), the central mass (frame 2), the
/// oblate rigid body (frame 3) on a circular orbit of radius `D_CENTRAL`
/// together with its companion (frame 4, circular orbit of radius
/// `D_COMPANION` around frame 3, in the same plane). Frame 3 spins once in
/// 21.3 hours about its z axis, tilted 17.2 degrees from the orbit normal.
fn spinning_system() -> FrameSystem {
    let pair = M_BODY + M_COMPANION;
    let v_pair = (G * (M_CENTRAL + pair) / D_CENTRAL).sqrt();
    let v_rel = (G * pair / D_COMPANION).sqrt();
    // Positions and velocities of the central mass and of the pair's
    // barycenter about the root, so the total momentum is zero.
    let central_share = pair / (M_CENTRAL + pair);
    let pair_share = M_CENTRAL / (M_CENTRAL + pair);
    let central = frame(
        2,
        1,
        M_CENTRAL,
        Vec3::new(-central_share * D_CENTRAL, 0.0, 0.0),
        Vec3::new(0.0, -central_share * v_pair, 0.0),
    );
    let body_off = M_COMPANION / pair;
    let mut body = frame(
        3,
        1,
        M_BODY,
        Vec3::new(pair_share * D_CENTRAL - body_off * D_COMPANION, 0.0, 0.0),
        Vec3::new(0.0, pair_share * v_pair - body_off * v_rel, 0.0),
    );
    let c = 0.341 * M_BODY * R_BODY * R_BODY;
    let j2 = 1.37e-3;
    let a = c - j2 * M_BODY * R_BODY * R_BODY;
    body.j2 = j2;
    body.reference_radius = Meters::new(R_BODY);
    body.principal_moments = Vec3::new(a, a, c);
    body.orientation = axis_angle(Vec3::new(1.0, 0.0, 0.0), 17.2f64.to_radians());
    body.angular_velocity = Vec3::new(0.0, 0.0, 2.0 * PI / (21.3 * 3600.0));
    let companion = frame(
        4,
        3,
        M_COMPANION,
        Vec3::new(D_COMPANION, 0.0, 0.0),
        Vec3::new(0.0, v_rel, 0.0),
    );
    system(vec![
        frame(1, ROOT_PARENT, 0.0, Vec3::zero(), Vec3::zero()),
        central,
        body,
        companion,
    ])
}

// Scenario 4: a locked companion stays locked.

const M_PARENT: f64 = M_BODY;
const R_LOCKED: f64 = 1.42e6;
/// Length of each lock run in seconds: about 160 orbits of the companion.
const LOCK_RUN: f64 = 3.0e8;
/// Seconds between two samples of the lock angle: 24 steps of one hour.
const SAMPLE: f64 = 24.0 * 3600.0;

/// A massive root (the parent) and a companion of `M_COMPANION` on a
/// circular orbit of radius `D_COMPANION` in the root's x y plane, started
/// at `+x` moving toward `+y`.
///
/// The companion is set up locked: its orientation is a half turn about z,
/// so its x axis points at the parent, and it spins about its z axis (the
/// orbit normal) at `spin_ratio` times the orbital mean motion. When `rigid`
/// is true its principal moments are `A < B < C` with `C = 0.394 m R^2`,
/// `(C - A) / C = 6.31e-4`, and `(B - A) / C = 2.28e-4`: the smallest moment
/// is about its x axis, so it is prolate along the line to the parent, and
/// the torque of the parent on that shape pulls the x axis back toward the
/// parent. When `rigid` is false its moments are zero and it keeps its
/// starting spin.
fn scenario_4(spin_ratio: f64, rigid: bool) -> FrameSystem {
    let v = (G * (M_PARENT + M_COMPANION) / D_COMPANION).sqrt();
    let n = v / D_COMPANION;
    let mut companion = frame(
        2,
        1,
        M_COMPANION,
        Vec3::new(D_COMPANION, 0.0, 0.0),
        Vec3::new(0.0, v, 0.0),
    );
    companion.orientation = Quat::new(0.0, 0.0, 1.0, 0.0);
    companion.angular_velocity = Vec3::new(0.0, 0.0, spin_ratio * n);
    if rigid {
        let c = 0.394 * M_COMPANION * R_LOCKED * R_LOCKED;
        let a = c * (1.0 - 6.31e-4);
        let b = a + 2.28e-4 * c;
        companion.principal_moments = Vec3::new(a, b, c);
    }
    system(vec![
        frame(1, ROOT_PARENT, M_PARENT, Vec3::zero(), Vec3::zero()),
        companion,
    ])
}

/// The largest angle, in degrees, between the companion's x axis and the
/// direction to the parent over `LOCK_RUN`, sampled every `SAMPLE`.
fn worst_lock_angle(mut s: FrameSystem) -> f64 {
    let mut worst: f64 = 0.0;
    for _ in 0..((LOCK_RUN / SAMPLE) as u64) {
        for _ in 0..24 {
            step(&mut s, Seconds::new(3600.0), Scheme::default());
        }
        let x = s.root_orientation(2).rotate(Vec3::new(1.0, 0.0, 0.0));
        let to_parent = s.root_position(1) - s.root_position(2);
        let cos = x.dot(to_parent) / to_parent.length();
        let sin = x.cross(to_parent).length() / to_parent.length();
        worst = worst.max(sin.atan2(cos).to_degrees());
    }
    worst
}

#[test]
fn scenario_4_lock_is_stable() {
    let exact = worst_lock_angle(scenario_4(1.0, true));
    assert!(exact < 1.0, "started locked: worst angle {exact} degrees");
    // A spin 2e-4 faster than the orbit: the torque turns the drift into a
    // libration that stays within a degree, while the same frame without
    // moments drifts many degrees away.
    let off = worst_lock_angle(scenario_4(1.0 + 2.0e-4, true));
    assert!(off < 1.0, "started slightly off: worst angle {off} degrees");
    let free = worst_lock_angle(scenario_4(1.0 + 2.0e-4, false));
    assert!(free > 5.0, "without moments: worst angle {free} degrees");
}
// Determinism and the history.

/// The spinning system, run for 30 units of time with the
/// default scheme in uneven calls.
fn short_run() -> FrameSystem {
    let mut s = spinning_system();
    advance(
        &mut s,
        Seconds::new(10.0 * UNIT),
        Seconds::new(1800.0),
        Scheme::default(),
    );
    advance(
        &mut s,
        Seconds::new(30.0 * UNIT),
        Seconds::new(2700.0),
        Scheme::default(),
    );
    s
}

#[test]
fn relativistic_scheme_is_deterministic() {
    let (a, b) = (short_run(), short_run());
    assert_eq!(a.state_bytes(), b.state_bytes());
    assert_eq!(a, b);
}

/// SplitMix64, for repeatable random times.
struct SplitMix64(u64);

impl SplitMix64 {
    fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn history_matches_direct_integration() {
    let max_step = Seconds::new(1800.0);
    let mut history = History::new(
        spinning_system(),
        Seconds::new(UNIT),
        max_step,
        Scheme::default(),
        16,
    );
    let mut rng = SplitMix64(7);
    let mut times: Vec<f64> = (0..12).map(|_| rng.unit() * 60.0 * UNIT).collect();
    // Out of order on purpose: later times extend the history, earlier ones
    // restore checkpoints.
    times.swap(0, 11);
    for t in times {
        let from_history = history.state_at(Seconds::new(t));
        let mut direct = spinning_system();
        advance(&mut direct, Seconds::new(t), max_step, Scheme::default());
        assert_eq!(from_history.time(), direct.time());
        for id in [2, 3, 4] {
            for (h, d) in [
                (from_history.root_position(id), direct.root_position(id)),
                (from_history.root_velocity(id), direct.root_velocity(id)),
            ] {
                let err = (h - d).length() / d.length();
                assert!(err < 1e-9, "frame {id} at {t} s: relative error {err}");
            }
        }
        let qh = from_history.root_orientation(3);
        let qd = direct.root_orientation(3);
        let x = Vec3::new(1.0, 0.0, 0.0);
        let err = (qh.rotate(x) - qd.rotate(x)).length();
        assert!(err < 1e-9, "orientation at {t} s: error {err}");
    }
    assert!(history.len() <= 16);
    assert!(
        history.interval().value() > UNIT,
        "the history thinned itself"
    );
}
