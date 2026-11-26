//! Integrator conformance vectors, written under `integrate/`
//! (`matter-format.md` section 8: "a valid registry with several frames and
//! the integrator's state for each frame at a fixed later time";
//! `space-model.md` sections 6.1, 6.2, and rule 7).
//!
//! - `registry.bin` is a valid registry: a massless root, a massive central
//!   frame, an oblate rigid frame with a tilted spin, a companion around it,
//!   a massless frame around the companion, and a rigid frame with three
//!   different moments.
//! - `cases.json` lists cases: a target time and a step limit (each as
//!   decimal text and IEEE 754 bit pattern), and the name of the file that
//!   holds the expected state.
//! - `NAME.bin` is `FrameSystem::state_bytes` after integrating the
//!   registry from its epoch to the target time with
//!   `integrate::advance` and the default scheme.
//!
//! Every value is a fixed literal. Orientations are rational unit
//! quaternions. A second implementation, or the WebAssembly build, must
//! produce the state files byte for byte.

use gx_core::frames::FrameSystem;
use gx_core::integrate::{advance, Scheme};
use gx_core::registry::{self, Frame, FrameTree, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};
use serde_json::{json, Value};

use crate::{number, pretty, File};

/// Epoch of the registry: 0, the origin (`matter-format.md` section 5).
const EPOCH: f64 = 0.0;

fn frame(id: u64, parent: u64, mass: f64, position: [f64; 3], velocity: [f64; 3]) -> Frame {
    let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
    Frame {
        frame_id: id,
        parent_frame_id: parent,
        root_extent: Meters::new(1.0e12),
        max_depth: 10,
        mass: Kilograms::new(mass),
        position: v(position),
        velocity: v(velocity),
        orientation: Quat::identity(),
        angular_velocity: Vec3::zero(),
        j2: 0.0,
        reference_radius: Meters::new(0.0),
        principal_moments: Vec3::zero(),
    }
}

/// The registry every case starts from.
pub(crate) fn registry() -> Registry {
    let mut oblate = frame(3, 1, 4.1e24, [1.13e11, 0.0, 0.0], [0.0, 3.99e4, 0.0]);
    oblate.j2 = 1.37e-3;
    oblate.reference_radius = Meters::new(5.71e6);
    oblate.principal_moments = Vec3::new(4.41e37, 4.41e37, 4.43e37);
    oblate.orientation = Quat::new(0.28, 0.0, 0.0, 0.96);
    oblate.angular_velocity = Vec3::new(0.0, 0.0, 6.1e-5);
    let companion = frame(41, 3, 5.6e22, [2.9e8, 0.0, 0.0], [0.0, 9.7e2, 30.0]);
    let probe = frame(5, 41, 0.0, [0.0, 2.0e6, 0.0], [-1.37e3, 0.0, 0.0]);
    let mut triaxial = frame(6, 1, 3.0e20, [0.0, -4.0e11, 1.0e9], [2.12e4, 0.0, 0.0]);
    triaxial.principal_moments = Vec3::new(1.0e30, 1.3e30, 1.5e30);
    triaxial.orientation = Quat::new(0.36, 0.48, 0.0, 0.8);
    triaxial.angular_velocity = Vec3::new(1.0e-5, 2.0e-6, 3.0e-4);
    let frames = vec![
        frame(1, ROOT_PARENT, 0.0, [0.0; 3], [0.0; 3]),
        frame(2, 1, 2.7e30, [0.0; 3], [0.0; 3]),
        oblate,
        companion,
        probe,
        triaxial,
    ];
    Registry::new(Seconds::new(EPOCH), frames).expect("a fixed valid registry")
}

/// Length of the cases' made-up unit of time in seconds: 21 hours, no
/// body's day.
pub(crate) const UNIT: f64 = 75_600.0;

/// `(name, seconds after the epoch, step limit)`.
const CASES: [(&str, f64, f64); 3] = [
    ("one-unit", UNIT, 600.0),
    ("thirty-units", 30.0 * UNIT, 3_600.0),
    ("back-half-unit", -0.5 * UNIT, 900.0),
];

/// The state bytes of the registry integrated to `EPOCH + offset`.
pub(crate) fn state(offset: f64, max_step: f64) -> Vec<u8> {
    let tree = FrameTree::from_registries(&[registry()]).expect("one valid registry");
    let mut s = FrameSystem::from_tree(tree);
    advance(
        &mut s,
        Seconds::new(EPOCH + offset),
        Seconds::new(max_step),
        Scheme::default(),
    );
    s.state_bytes()
}

pub(crate) fn files() -> Vec<File> {
    let mut files = vec![(
        "integrate/registry.bin".to_string(),
        registry::encode(&registry()),
    )];
    let mut cases: Vec<Value> = Vec::new();
    for (name, offset, max_step) in CASES {
        let file = format!("{name}.bin");
        cases.push(json!({
            "name": name,
            "to_seconds": number(EPOCH + offset),
            "max_step_seconds": number(max_step),
            "state": file,
        }));
        files.push((format!("integrate/{file}"), state(offset, max_step)));
    }
    files.push((
        "integrate/cases.json".to_string(),
        pretty(&json!({ "registry": "registry.bin", "cases": cases })),
    ));
    files
}
