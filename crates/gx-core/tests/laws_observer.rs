//! Tests for the observer body (`space-model.md` section 6.3): standing on a
//! spinning solid ball, free fall in vacuum, terminal velocity in a gas, and
//! sinking in a gas whose density rises with depth, with the tolerances the
//! task sets. Every matter distribution is a probe built here, not a
//! section.

use core::f64::consts::PI;

use gx_core::frames::FrameSystem;
use gx_core::gravity::G;
use gx_core::integrate::{advance, Scheme};
use gx_core::matter::State;
use gx_core::observer::{step_observer, MatterProbe, ObserverParams, ObserverState, Sample};
use gx_core::registry::{Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Density, Kilograms, Meters, Quat, Seconds, Vec3};

/// The renderer's frame time in these tests, seconds.
const DT: f64 = 1.0 / 60.0;

/// One massive frame at the root, spinning about its z axis at `spin`
/// radians per second.
fn system(mass: f64, spin: f64) -> FrameSystem {
    let f = Frame {
        frame_id: 1,
        parent_frame_id: ROOT_PARENT,
        root_extent: Meters::new(1.0e10),
        max_depth: 8,
        mass: Kilograms::new(mass),
        position: Vec3::zero(),
        velocity: Vec3::zero(),
        orientation: Quat::identity(),
        angular_velocity: Vec3::new(0.0, 0.0, spin),
        j2: 0.0,
        reference_radius: Meters::new(0.0),
        principal_moments: Vec3::zero(),
    };
    let reg = Registry::new(Seconds::new(0.0), vec![f]).unwrap();
    FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap())
}

/// A uniform solid ball centered on the frame origin, turning with the
/// frame.
struct Ball {
    radius: f64,
    density: f64,
    spin: f64,
}

impl MatterProbe for Ball {
    fn sample(&self, p: Vec3) -> Option<Sample> {
        let velocity = Vec3::new(0.0, 0.0, self.spin).cross(p);
        let inside = p.length() <= self.radius;
        Some(Sample {
            density: Density::new(if inside { self.density } else { 0.0 }),
            state: if inside { State::Solid } else { State::Vacuum },
            velocity,
        })
    }
}

/// Nothing anywhere.
struct Empty;

impl MatterProbe for Empty {
    fn sample(&self, _: Vec3) -> Option<Sample> {
        None
    }
}

/// Gas of one density at rest everywhere.
struct UniformGas(f64);

impl MatterProbe for UniformGas {
    fn sample(&self, _: Vec3) -> Option<Sample> {
        Some(Sample {
            density: Density::new(self.0),
            state: State::Gas,
            velocity: Vec3::zero(),
        })
    }
}

/// A gas body with the density profile of a polytrope of index 1,
/// `rho_c sin(pi r / R) / (pi r / R)`, at rest.
struct Polytrope {
    radius: f64,
    central_density: f64,
}

impl Polytrope {
    fn density(&self, r: f64) -> f64 {
        if r >= self.radius {
            return 0.0;
        }
        let x = PI * r / self.radius;
        if x == 0.0 {
            self.central_density
        } else {
            self.central_density * x.sin() / x
        }
    }

    /// The mass of the profile, `4 rho_c R^3 / pi`.
    fn mass(&self) -> f64 {
        4.0 * self.central_density * self.radius.powi(3) / PI
    }
}

impl MatterProbe for Polytrope {
    fn sample(&self, p: Vec3) -> Option<Sample> {
        let rho = self.density(p.length());
        Some(Sample {
            density: Density::new(rho),
            state: if rho > 0.0 { State::Gas } else { State::Vacuum },
            velocity: Vec3::zero(),
        })
    }
}

#[test]
fn standing_on_a_spinning_ball() {
    let mass = 5.972e24;
    let radius = 6.371e6;
    let spin = 2.0 * PI / 86_164.0;
    let ball = Ball {
        radius,
        density: mass / (4.0 / 3.0 * PI * radius.powi(3)),
        spin,
    };
    let mut s = system(mass, spin);
    let params = ObserverParams::default();
    // Half a meter above the surface at 30 degrees from the equator,
    // moving with the surface below.
    let lat = PI / 6.0;
    let dir = Vec3::new(lat.cos(), 0.0, lat.sin());
    let start = dir.scale(radius + 0.5);
    let mut o = ObserverState::new(start, Vec3::new(0.0, 0.0, spin).cross(start));
    let frame_axes = |s: &FrameSystem, p: Vec3| s.root_orientation(1).conjugate().rotate(p);
    let mut landed_at = None;
    for n in 0..(65 * 60) {
        step_observer(
            &mut o,
            1,
            &s,
            &ball,
            Vec3::zero(),
            Seconds::new(DT),
            &params,
        );
        let to = Seconds::new((n + 1) as f64 * DT);
        advance(&mut s, to, Seconds::new(DT), Scheme::default());
        if n < 5 * 60 {
            continue;
        }
        // From 5 s on: on the surface and moving with it.
        let pf = frame_axes(&s, o.position);
        let edge = pf.length() - radius;
        assert!(edge.abs() < 0.01, "{edge} m from the edge at step {n}");
        let surface = s
            .root_orientation(1)
            .rotate(Vec3::new(0.0, 0.0, spin).cross(pf));
        let rel = (o.velocity - surface).length();
        assert!(rel < 0.01, "{rel} m/s relative to the surface at step {n}");
        assert!(o.contact.is_some(), "contact lost at step {n}");
        let at = *landed_at.get_or_insert(pf);
        assert!((pf - at).length() < 0.01, "slid {} m", (pf - at).length());
    }
    // The surface moved about 24 km in root axes over the minute; the
    // observer went with it.
    let travelled = (o.position - start).length();
    assert!(travelled > 2.0e4, "{travelled}");
}

#[test]
fn free_fall_in_vacuum() {
    // A distant mass, so gravity changes by less than 1e-6 over the fall.
    let r = 1.0e8;
    let g = 9.8;
    let mass = g * r * r / G;
    let s = system(mass, 0.0);
    let params = ObserverParams::default();
    let mut o = ObserverState::new(Vec3::new(r, 0.0, 0.0), Vec3::zero());
    let g0 = G * mass / (r * r);
    for n in 1..=180 {
        step_observer(
            &mut o,
            1,
            &s,
            &Empty,
            Vec3::zero(),
            Seconds::new(DT),
            &params,
        );
        // Checked each whole second: the spacing of f64 positions this far
        // from the frame origin (1.5e-8 m) is too coarse for a relative
        // 1e-6 on the first millimeters.
        if n % 60 != 0 {
            continue;
        }
        let t = n as f64 * DT;
        let want = 0.5 * g0 * t * t;
        let got = r - o.position.x;
        assert!(
            ((got - want) / want).abs() < 1e-6,
            "t = {t}: fell {got} m, want {want} m"
        );
    }
    assert!(o.contact.is_none());
}

#[test]
fn terminal_velocity_in_a_gas() {
    let r = 1.0e8;
    let g = 9.8;
    let s = system(g * r * r / G, 0.0);
    let params = ObserverParams::default();
    let rho = 1.2;
    let mut o = ObserverState::new(Vec3::new(r, 0.0, 0.0), Vec3::zero());
    for _ in 0..(60 * 60) {
        step_observer(
            &mut o,
            1,
            &s,
            &UniformGas(rho),
            Vec3::zero(),
            Seconds::new(DT),
            &params,
        );
    }
    let m = params.mass.value();
    let want = (2.0 * m * g / (rho * params.drag_coefficient * params.frontal_area.value())).sqrt();
    let got = -o.velocity.x;
    assert!(
        ((got - want) / want).abs() < 0.01,
        "terminal speed {got} m/s, want {want} m/s"
    );
}

#[test]
fn sinking_in_a_gas_body() {
    // A small dense body, so the observer settles in about an hour of
    // simulated time; its equilibrium depth is about 23.6 m.
    let body = Polytrope {
        radius: 500.0,
        central_density: 2.0e4,
    };
    let s = system(body.mass(), 0.0);
    let params = ObserverParams::default();
    let target = params.density.value();
    // The radius where the profile equals the observer density, by
    // bisection on the falling profile.
    let (mut lo, mut hi) = (0.0, body.radius);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if body.density(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let depth = body.radius - 0.5 * (lo + hi);
    // Released at rest at the edge of the gas.
    let mut o = ObserverState::new(Vec3::new(0.0, 0.0, body.radius), Vec3::zero());
    let steps = 4000 * 60;
    for _ in 0..steps {
        step_observer(
            &mut o,
            1,
            &s,
            &body,
            Vec3::zero(),
            Seconds::new(DT),
            &params,
        );
    }
    let got = body.radius - o.position.length();
    assert!(
        ((got - depth) / depth).abs() < 0.01,
        "rests at depth {got} m, want {depth} m"
    );
    let speed = o.velocity.length();
    assert!(speed < 1e-3, "still moving at {speed} m/s");
    assert!(o.contact.is_none());
}
