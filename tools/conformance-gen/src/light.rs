//! Light-time conformance vectors, written under `light/`
//! (`space-model.md` sections 6.4 and rule 7).
//!
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
use gx_core::registry::{self, FrameTree, Registry};
use gx_core::units::{Seconds, Vec3};
use serde_json::{json, Value};

use crate::{number, pretty, File};

fn system(reg: &Registry) -> FrameSystem {
    FrameSystem::from_tree(FrameTree::from_registries(std::slice::from_ref(reg)).unwrap())
}

fn numbers(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|&x| number(x)).collect())
}

/// `(name, frame, observer, seconds after the epoch, interval, step limit)`.
type LightCase = (&'static str, u64, [f64; 3], f64, f64, f64);

/// Light cases against the integrator vectors' registry (epoch 0, the origin).
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
    let mut files = Vec::new();
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
