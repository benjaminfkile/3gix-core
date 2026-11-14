//! Checks the observer and light-time conformance vectors under
//! `conformance/observer/` and `conformance/light/` (`space-model.md`
//! sections 6.3, 6.4, and rule 7): every case recomputed natively gives the
//! recorded bytes exactly. `scripts/wasm-node-check.mjs` checks the same
//! files against the WebAssembly build under Node.

use std::path::{Path, PathBuf};

use gx_core::frames::FrameSystem;
use gx_core::history::History;
use gx_core::integrate::Scheme;
use gx_core::light::retarded_state;
use gx_core::matter::State;
use gx_core::observer::{simulate, ObserverParams, ObserverState, RadialProbe};
use gx_core::registry::{self, FrameTree, Registry};
use gx_core::units::{Seconds, Vec3};
use serde_json::Value;

fn root(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(dir)
}

/// Reads the `bits` of a recorded number and checks `decimal` agrees.
fn number(v: &Value) -> f64 {
    let bits = v["bits"].as_str().expect("bits is a string");
    let bits = u64::from_str_radix(bits.trim_start_matches("0x"), 16).expect("hex bits");
    let decimal: f64 = v["decimal"].as_str().unwrap().parse().unwrap();
    assert_eq!(decimal.to_bits(), bits, "decimal and bits disagree in {v}");
    f64::from_bits(bits)
}

fn numbers(v: &Value) -> Vec<f64> {
    v.as_array().expect("an array").iter().map(number).collect()
}

fn vec3(v: &Value) -> Vec3 {
    let n = numbers(v);
    assert_eq!(n.len(), 3);
    Vec3::new(n[0], n[1], n[2])
}

fn load(dir: &str) -> (Value, Registry) {
    let text = std::fs::read_to_string(root(dir).join("cases.json")).expect("cases.json");
    let index: Value = serde_json::from_str(&text).expect("JSON");
    let bytes = std::fs::read(root(dir).join(index["registry"].as_str().unwrap())).unwrap();
    (index, registry::decode(&bytes).expect("a valid registry"))
}

fn system(reg: &Registry) -> FrameSystem {
    FrameSystem::from_tree(FrameTree::from_registries(std::slice::from_ref(reg)).unwrap())
}

#[test]
fn observer_states_match() {
    let (index, reg) = load("observer");
    let cases = index["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 3);
    for case in cases {
        let frame = case["frame_id"].as_u64().unwrap();
        let mut s = system(&reg);
        let probe = RadialProbe {
            radii: numbers(&case["radii"]),
            densities: numbers(&case["densities"]),
            state: State::from_u8(case["state"].as_u64().unwrap() as u8).unwrap(),
            angular_velocity: s.state(frame).unwrap().angular_velocity,
        };
        let mut o = ObserverState::new(vec3(&case["position"]), vec3(&case["velocity"]));
        simulate(
            &mut o,
            frame,
            &mut s,
            &probe,
            vec3(&case["intent"]),
            Seconds::new(number(&case["dt_seconds"])),
            case["steps"].as_u64().unwrap() as u32,
            &ObserverParams::default(),
        );
        let want =
            std::fs::read(root("observer").join(case["state_file"].as_str().unwrap())).unwrap();
        assert_eq!(o.to_bytes(), want, "case {}", case["name"]);
    }
}

#[test]
fn light_states_match() {
    let (index, reg) = load("light");
    let cases = index["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 4);
    let mut clamped = 0;
    for case in cases {
        let t = Seconds::new(number(&case["t_seconds"]));
        let mut h = History::new(
            system(&reg),
            Seconds::new(number(&case["interval_seconds"])),
            Seconds::new(number(&case["max_step_seconds"])),
            Scheme::default(),
            64,
        );
        h.extend_to(t);
        let r = retarded_state(
            vec3(&case["observer"]),
            t,
            &h,
            case["frame_id"].as_u64().unwrap(),
        );
        clamped += usize::from(r.light.clamped);
        let want = std::fs::read(root("light").join(case["state_file"].as_str().unwrap())).unwrap();
        assert_eq!(r.to_bytes(), want, "case {}", case["name"]);
    }
    assert_eq!(clamped, 1, "exactly one case is clamped to the epoch");
}
