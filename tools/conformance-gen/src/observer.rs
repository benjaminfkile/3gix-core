//! Observer and light-time conformance vectors, written under `observer/`
//! and `light/` (`space-model.md` sections 6.3, 6.4, and rule 7).
//!
//! - `observer/registry.bin` is a valid registry: a massless root, a massive
//!   central frame, an oblate spinning frame with a tilted axis, and a small
//!   dense frame for a gas profile.
//! - `observer/cases.json` lists cases: the frame, a radial profile (radii
//!   and densities, each as decimal text and IEEE 754 bit pattern), the
//!   state code inside it, the initial position and velocity, the walking
//!   force, the step, the step count, and the file that holds the expected
//!   final state. The probe is `observer::RadialProbe` turning with the
//!   frame at its registry angular velocity, the constants are
//!   `ObserverParams::default()`, and the run is `observer::simulate`.
//! - `observer/NAME.bin` is `ObserverState::to_bytes` of the final state.
//! - `light/registry.bin` is the integrator vectors' registry.
//! - `light/cases.json` lists cases: the frame, the observer position, the
//!   time, and the history's checkpoint interval and step limit (capacity
//!   64, default scheme, extended to the time first), and the file that
//!   holds `light::RetardedState::to_bytes`.
//!
//! Every input is a fixed literal or computed from literals with `+`, `-`,
//! `*`, and `/` only, so the inputs are the same on every machine.

use gx_core::frames::FrameSystem;
use gx_core::history::History;
use gx_core::integrate::Scheme;
use gx_core::light::retarded_state;
use gx_core::matter::State;
use gx_core::observer::{simulate, ObserverParams, ObserverState, RadialProbe};
use gx_core::registry::{self, Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};
use serde_json::{json, Value};

use crate::{number, pretty, File};

/// Epoch of the observer registry: seconds of TDB since J2000.
const EPOCH: f64 = 8.1e8;

/// The step of every observer case, seconds.
const DT: f64 = 1.0 / 60.0;

fn frame(id: u64, parent: u64, mass: f64, position: Vec3, velocity: Vec3) -> Frame {
    Frame {
        frame_id: id,
        parent_frame_id: parent,
        root_extent: Meters::new(1.0e12),
        max_depth: 10,
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

fn observer_registry() -> Registry {
    let mut oblate = frame(
        3,
        1,
        6.0e24,
        Vec3::new(1.5e11, 0.0, 0.0),
        Vec3::new(0.0, 2.98e4, 0.0),
    );
    oblate.j2 = 1.08e-3;
    oblate.reference_radius = Meters::new(6.4e6);
    oblate.orientation = Quat::new(0.28, 0.0, 0.0, 0.96);
    oblate.angular_velocity = Vec3::new(0.0, 0.0, 7.29e-5);
    let small = frame(
        4,
        1,
        3.2e12,
        Vec3::new(-4.0e11, 0.0, 0.0),
        Vec3::new(0.0, -1.8e4, 0.0),
    );
    let frames = vec![
        frame(1, ROOT_PARENT, 0.0, Vec3::zero(), Vec3::zero()),
        frame(2, 1, 2.0e30, Vec3::zero(), Vec3::zero()),
        oblate,
        small,
    ];
    Registry::new(Seconds::new(EPOCH), frames).expect("a fixed valid registry")
}

fn system(reg: &Registry) -> FrameSystem {
    FrameSystem::from_tree(FrameTree::from_registries(std::slice::from_ref(reg)).unwrap())
}

/// One observer case.
struct Case {
    name: &'static str,
    frame: u64,
    radii: Vec<f64>,
    densities: Vec<f64>,
    state: State,
    position: Vec3,
    velocity: Vec3,
    intent: Vec3,
    steps: u32,
}

fn cases(reg: &Registry) -> Vec<Case> {
    let s = system(reg);
    // Half a meter above the oblate frame's surface, in its axes along
    // (3, 4, 12) / 13, moving with the surface.
    let q = s.root_orientation(3);
    let w = s.state(3).unwrap().angular_velocity;
    let radius = 6.4e6;
    let dir = Vec3::new(3.0 / 13.0, 4.0 / 13.0, 12.0 / 13.0);
    let pf = dir.scale(radius + 0.5);
    let position = q.rotate(pf);
    let velocity = q.rotate(w.cross(pf));
    let solid = || (vec![0.0, radius], vec![5500.0, 5500.0]);
    let (sr, sd) = solid();
    let (wr, wd) = solid();
    vec![
        Case {
            name: "stand",
            frame: 3,
            radii: sr,
            densities: sd,
            state: State::Solid,
            position,
            velocity,
            intent: Vec3::zero(),
            steps: 120,
        },
        Case {
            name: "walk",
            frame: 3,
            radii: wr,
            densities: wd,
            state: State::Solid,
            position,
            velocity,
            intent: Vec3::new(60.0, -20.0, 10.0),
            steps: 180,
        },
        Case {
            name: "sink",
            frame: 4,
            radii: vec![0.0, 100.0, 200.0, 300.0, 400.0, 500.0],
            densities: vec![20000.0, 15000.0, 9000.0, 4000.0, 1200.0, 0.0],
            state: State::Gas,
            position: Vec3::new(0.0, 300.0, 400.0),
            velocity: Vec3::zero(),
            intent: Vec3::zero(),
            steps: 600,
        },
    ]
}

/// The final state bytes of one case.
fn run(reg: &Registry, c: &Case) -> Vec<u8> {
    let mut s = system(reg);
    let probe = RadialProbe {
        radii: c.radii.clone(),
        densities: c.densities.clone(),
        state: c.state,
        angular_velocity: s.state(c.frame).unwrap().angular_velocity,
    };
    let mut o = ObserverState::new(c.position, c.velocity);
    simulate(
        &mut o,
        c.frame,
        &mut s,
        &probe,
        c.intent,
        Seconds::new(DT),
        c.steps,
        &ObserverParams::default(),
    );
    o.to_bytes()
}

fn numbers(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|&x| number(x)).collect())
}

fn vec3(v: Vec3) -> Value {
    numbers(&[v.x, v.y, v.z])
}

/// `(name, frame, observer, seconds after the epoch, interval, step limit)`.
type LightCase = (&'static str, u64, [f64; 3], f64, f64, f64);

/// Light cases against the integrator vectors' registry (epoch 8.1e8 s).
const LIGHT: [LightCase; 4] = [
    (
        "companion-thirty-days",
        4,
        [1.5e11, 1.0e7, 0.0],
        30.0 * 86_400.0,
        86_400.0,
        3_600.0,
    ),
    (
        "oblate-one-day",
        3,
        [-4.0e11, 1.0e11, 0.0],
        86_400.0,
        21_600.0,
        900.0,
    ),
    ("clamped", 6, [0.0, 0.0, 0.0], 100.0, 3_600.0, 600.0),
    (
        "probe-two-days",
        5,
        [0.0, 0.0, 0.0],
        2.0 * 86_400.0,
        43_200.0,
        1_800.0,
    ),
];

fn light_bytes(reg: &Registry, c: &LightCase) -> Vec<u8> {
    let (_, frame, observer, offset, interval, max_step) = *c;
    let t = Seconds::new(reg.epoch().value() + offset);
    let mut h = History::new(
        system(reg),
        Seconds::new(interval),
        Seconds::new(max_step),
        Scheme::default(),
        64,
    );
    h.extend_to(t);
    let o = Vec3::new(observer[0], observer[1], observer[2]);
    retarded_state(o, t, &h, frame).to_bytes()
}

pub(crate) fn files() -> Vec<File> {
    let reg = observer_registry();
    let mut files = vec![("observer/registry.bin".to_string(), registry::encode(&reg))];
    let mut index: Vec<Value> = Vec::new();
    for c in cases(&reg) {
        let file = format!("{}.bin", c.name);
        index.push(json!({
            "name": c.name,
            "frame_id": c.frame,
            "radii": numbers(&c.radii),
            "densities": numbers(&c.densities),
            "state": c.state as u8,
            "position": vec3(c.position),
            "velocity": vec3(c.velocity),
            "intent": vec3(c.intent),
            "dt_seconds": number(DT),
            "steps": c.steps,
            "state_file": file,
        }));
        files.push((format!("observer/{file}"), run(&reg, &c)));
    }
    files.push((
        "observer/cases.json".to_string(),
        pretty(&json!({ "registry": "registry.bin", "cases": index })),
    ));

    let reg = crate::integrate::registry();
    files.push(("light/registry.bin".to_string(), registry::encode(&reg)));
    let mut index: Vec<Value> = Vec::new();
    for c in &LIGHT {
        let (name, frame, observer, offset, interval, max_step) = *c;
        let file = format!("{name}.bin");
        index.push(json!({
            "name": name,
            "frame_id": frame,
            "observer": numbers(&observer),
            "t_seconds": number(reg.epoch().value() + offset),
            "interval_seconds": number(interval),
            "max_step_seconds": number(max_step),
            "state_file": file,
        }));
        files.push((format!("light/{file}"), light_bytes(&reg, c)));
    }
    files.push((
        "light/cases.json".to_string(),
        pretty(&json!({ "registry": "registry.bin", "cases": index })),
    ));
    files
}
