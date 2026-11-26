//! Tests of the v2 laws against real measurements (`space-model.md`
//! sections 6.1 and 6.2): first post-Newtonian gravity and rotation under
//! torques. Scenarios are named by the number they had in the law tests.
//!
//! measured-reference: scenario 1 checks the measured anomalous perihelion advance of Mercury, 42.98 arcseconds per century, with Mercury's orbit around the Sun; scenario 3 checks the measured precession of the Earth's spin axis, 50.3 arcseconds per year, and its obliquity of 23.44 degrees, with the Sun, the Earth, and the Moon. A made-up body has no measured value to compare with, so these tests use the real bodies' values.

use core::f64::consts::PI;

use gx_core::frames::FrameSystem;
use gx_core::gravity::G;
use gx_core::integrate::{step, Scheme};
use gx_core::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};

const DAY: f64 = 86_400.0;
const YEAR: f64 = 365.25 * DAY;
const ARCSEC: f64 = PI / (180.0 * 3600.0);

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
// Scenario 1: perihelion advance.

const GM1: f64 = 1.327_124_400_18e20;
const A1: f64 = 5.7909e10;
const E1: f64 = 0.2056;

/// A massive root with `GM1` and a massless frame at the periapsis of an
/// orbit with semi-major axis `A1` and eccentricity `E1`.
fn scenario_1() -> FrameSystem {
    let rp = A1 * (1.0 - E1);
    let vp = (GM1 * (1.0 + E1) / rp).sqrt();
    system(vec![
        frame(1, ROOT_PARENT, GM1 / G, Vec3::zero(), Vec3::zero()),
        frame(2, 1, 0.0, Vec3::new(rp, 0.0, 0.0), Vec3::new(0.0, vp, 0.0)),
    ])
}

/// Integrates scenario 1 for one century of 36 525 days with `steps_per_orbit`
/// fixed steps per orbit and returns the fitted rate of the periapsis
/// angle in arcseconds per century. At every periapsis passage (the
/// radial velocity turning from negative to positive) the in-plane angle of the
/// eccentricity vector is recorded; sampling at the same orbital phase every
/// time removes the periodic part of the osculating elements.
fn periapsis_rate(scheme: Scheme, steps_per_orbit: f64) -> f64 {
    let mut s = scenario_1();
    let period = 2.0 * PI * (A1 * A1 * A1 / GM1).sqrt();
    let dt = period / steps_per_orbit;
    let total = 100.0 * YEAR;
    let count = (total / dt).ceil() as u64;
    let mut points = Vec::new();
    let mut previous_radial = -1.0;
    let mut last_angle = 0.0;
    for _ in 0..count {
        step(&mut s, Seconds::new(dt), scheme);
        let r = s.root_position(2) - s.root_position(1);
        let v = s.root_velocity(2) - s.root_velocity(1);
        let radial = r.dot(v);
        if previous_radial < 0.0 && radial >= 0.0 {
            let h = r.cross(v);
            let e = v.cross(h) - r.scale(GM1 / r.length());
            let angle = unwrap(last_angle, e.y.atan2(e.x));
            last_angle = angle;
            points.push((s.time().value(), angle));
        }
        previous_radial = radial;
    }
    assert!(points.len() > 400, "{} passages", points.len());
    slope(&points) * 100.0 * YEAR / ARCSEC
}

#[test]
fn scenario_1_periapsis_advance() {
    let rel = periapsis_rate(Scheme::Relativistic, 3000.0);
    assert!(
        (rel - 42.98).abs() < 0.02 * 42.98,
        "first post-Newtonian advance {rel} arcsec per century"
    );
    let newton = periapsis_rate(Scheme::Yoshida4, 3000.0);
    assert!(
        newton.abs() < 0.1,
        "Newtonian advance {newton} arcsec per century"
    );
}
// Scenario 3: spin axis precession under two torquing masses.

const M_CENTRAL: f64 = 1.989e30;
const D_CENTRAL: f64 = 1.496e11;
const M_BODY: f64 = 5.972e24;
const R_BODY: f64 = 6.378_137e6;
const M_COMPANION: f64 = 7.342e22;
const D_COMPANION: f64 = 3.844e8;

/// A massless root (the barycenter), the central mass (frame 2), the
/// oblate rigid body (frame 3) on a circular orbit of radius `D_CENTRAL`
/// together with its companion (frame 4, circular orbit of radius
/// `D_COMPANION` around frame 3, in the same plane). Frame 3 spins once in
/// 23.93 hours about its z axis, tilted 23.44 degrees from the orbit normal.
fn scenario_3() -> FrameSystem {
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
    let c = 0.3307 * M_BODY * R_BODY * R_BODY;
    let j2 = 1.0826e-3;
    let a = c - j2 * M_BODY * R_BODY * R_BODY;
    body.j2 = j2;
    body.reference_radius = Meters::new(R_BODY);
    body.principal_moments = Vec3::new(a, a, c);
    body.orientation = axis_angle(Vec3::new(1.0, 0.0, 0.0), 23.44f64.to_radians());
    body.angular_velocity = Vec3::new(0.0, 0.0, 2.0 * PI / (23.93 * 3600.0));
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

#[test]
fn scenario_3_spin_axis_precession() {
    let mut s = scenario_3();
    let dt = 3600.0;
    let mut points = Vec::new();
    let mut last = -PI / 2.0;
    let samples = (20.0 * YEAR / DAY) as u64;
    for _ in 0..samples {
        for _ in 0..24 {
            step(&mut s, Seconds::new(dt), Scheme::default());
        }
        let axis = s.root_orientation(3).rotate(Vec3::new(0.0, 0.0, 1.0));
        let angle = unwrap(last, axis.y.atan2(axis.x));
        last = angle;
        points.push((s.time().value(), angle));
    }
    let rate = slope(&points) * YEAR / ARCSEC;
    assert!(rate < 0.0, "the axis precesses against the orbit: {rate}");
    let err = (rate.abs() - 50.3).abs() / 50.3;
    assert!(err < 0.05, "precession {rate} arcsec per year, error {err}");
    // The obliquity stays near its start.
    let axis = s.root_orientation(3).rotate(Vec3::new(0.0, 0.0, 1.0));
    let obliquity = axis.z.acos().to_degrees();
    assert!((obliquity - 23.44).abs() < 0.1, "obliquity {obliquity}");
}
