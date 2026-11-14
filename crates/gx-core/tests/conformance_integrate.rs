//! Checks the integrator conformance vectors under `conformance/integrate/`
//! (`matter-format.md` section 8, `space-model.md` sections 6.1 and 6.2):
//! the registry integrated with the default scheme to every case's time
//! gives the recorded state bytes exactly.

use std::path::{Path, PathBuf};

use gx_core::frames::FrameSystem;
use gx_core::integrate::{advance, Scheme};
use gx_core::registry::{self, FrameTree};
use gx_core::units::Seconds;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/integrate")
}

/// Reads the `bits` of a recorded number and checks `decimal` agrees.
fn number(v: &Value) -> f64 {
    let bits = v["bits"].as_str().expect("bits is a string");
    let bits = u64::from_str_radix(bits.trim_start_matches("0x"), 16).expect("hex bits");
    let decimal: f64 = v["decimal"].as_str().unwrap().parse().unwrap();
    assert_eq!(decimal.to_bits(), bits, "decimal and bits disagree in {v}");
    f64::from_bits(bits)
}

#[test]
fn states_match() {
    let text = std::fs::read_to_string(root().join("cases.json")).expect("cases.json");
    let index: Value = serde_json::from_str(&text).expect("JSON");
    let bytes = std::fs::read(root().join(index["registry"].as_str().unwrap())).unwrap();
    let reg = registry::decode(&bytes).expect("a valid registry");
    let cases = index["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 3);
    for case in cases {
        let mut s =
            FrameSystem::from_tree(FrameTree::from_registries(std::slice::from_ref(&reg)).unwrap());
        advance(
            &mut s,
            Seconds::new(number(&case["to_seconds"])),
            Seconds::new(number(&case["max_step_seconds"])),
            Scheme::default(),
        );
        let want = std::fs::read(root().join(case["state"].as_str().unwrap())).unwrap();
        assert_eq!(s.state_bytes(), want, "case {}", case["name"]);
    }
}
