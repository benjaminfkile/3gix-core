//! Frame record conformance vectors, written under `frame-record/`
//! (`matter-format.md` section 11).
//!
//! - `valid/registry.bin` is a registry declaring frames 1 (the root, no
//!   moments), 2 (moments), and 3 (`j2`, reference radius, and moments).
//! - `valid/NAME.bin` is a valid record. `valid/index.json` maps each file to
//!   the key it validates under and the result `gx_frame_record_agrees`
//!   gives against `registry.bin`: 0 when it agrees, -1 when the registry
//!   does not declare the frame, -2 to -8 for the first differing field.
//! - `invalid/CODE-RULE.bin` breaks exactly one rule of section 11.3.
//!   `invalid/index.json` gives the key every file is validated under and
//!   maps each file name to its code.

use std::collections::BTreeMap;

use gx_core::error::codes;
use gx_core::frame_record::{self, FrameRecord};
use gx_core::registry::{self, Frame, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};
use serde_json::json;

use crate::{pretty, File};

/// The frame the invalid vectors describe.
const INVALID_FRAME: u64 = 3;

fn frame(id: u64, parent: u64, j2: f64, radius: f64, moments: Vec3) -> Frame {
    let root = parent == ROOT_PARENT;
    let at = |v: f64| if root { 0.0 } else { v };
    Frame {
        frame_id: id,
        parent_frame_id: parent,
        root_extent: Meters::new(1.0e7 * id as f64),
        max_depth: 10 + id as u8,
        mass: Kilograms::new(2.5e23 * id as f64),
        position: Vec3::new(at(1.13e11), at(-2.0e9), at(3.0e8)),
        velocity: Vec3::new(at(1.0e3), at(3.99e4), at(-5.0)),
        orientation: Quat::new(0.0, 0.0, 0.0, 1.0),
        angular_velocity: Vec3::new(0.0, 0.0, 6.1e-5),
        j2,
        reference_radius: Meters::new(radius),
        principal_moments: moments,
    }
}

fn frames() -> Vec<Frame> {
    vec![
        frame(1, ROOT_PARENT, 0.0, 0.0, Vec3::new(0.0, 0.0, 0.0)),
        frame(2, 1, 0.0, 0.0, Vec3::new(4.0e35, 4.5e35, 5.0e35)),
        frame(3, 1, 1.37e-3, 5.71e6, Vec3::new(4.41e37, 4.41e37, 4.43e37)),
    ]
}

fn valid() -> Vec<File> {
    let fs = frames();
    let registry = Registry::new(Seconds::new(0.0), fs.clone()).expect("valid registry");
    let mut out = vec![(
        "frame-record/valid/registry.bin".to_string(),
        registry::encode(&registry),
    )];
    let mut mass = FrameRecord::from_registry_frame(&fs[2]);
    mass.mass = Kilograms::new(mass.mass.value() * 2.0);
    let records: [(&str, FrameRecord, i32); 5] = [
        ("root", FrameRecord::from_registry_frame(&fs[0]), 0),
        ("moments", FrameRecord::from_registry_frame(&fs[1]), 0),
        ("j2", FrameRecord::from_registry_frame(&fs[2]), 0),
        ("empty", FrameRecord::empty(4), -1),
        ("disagrees-mass", mass, -5),
    ];
    let mut index = BTreeMap::new();
    for (name, r, agrees) in records {
        let file = format!("{name}.bin");
        index.insert(
            file.clone(),
            json!({ "key": format!("frame-{}", r.frame_id), "agrees": agrees }),
        );
        out.push((
            format!("frame-record/valid/{file}"),
            frame_record::encode(&r),
        ));
    }
    out.push((
        "frame-record/valid/index.json".into(),
        pretty(&json!({ "registry": "registry.bin", "records": index })),
    ));
    out
}

fn put_f64(b: &mut [u8], at: usize, v: f64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn invalid() -> Vec<File> {
    let base = frame_record::encode(&FrameRecord::from_registry_frame(&frames()[2]));
    let empty = frame_record::encode(&FrameRecord::empty(INVALID_FRAME));
    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, from: &[u8], f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = from.to_vec();
        f(&mut b);
        vectors.push((name, code, b));
    };
    add("length-short", codes::FRAME_RECORD_LENGTH, &base, &|b| {
        b.pop();
    });
    add("length-long", codes::FRAME_RECORD_LENGTH, &base, &|b| {
        b.push(0)
    });
    add("bad-magic", codes::FRAME_RECORD_BAD_MAGIC, &base, &|b| {
        b[3] = b'S'
    });
    add(
        "unsupported-version",
        codes::FRAME_RECORD_UNSUPPORTED_VERSION,
        &base,
        &|b| b[4] = 2,
    );
    add(
        "unknown-flags",
        codes::FRAME_RECORD_UNKNOWN_FLAGS,
        &base,
        &|b| b[6] = 2,
    );
    add(
        "reserved-nonzero",
        codes::FRAME_RECORD_RESERVED_NONZERO,
        &base,
        &|b| b[36] = 1,
    );
    add(
        "id-mismatch",
        codes::FRAME_RECORD_ID_MISMATCH,
        &base,
        &|b| b[8] = 2,
    );
    add(
        "empty-nonzero",
        codes::FRAME_RECORD_EMPTY_NONZERO,
        &empty,
        &|b| put_f64(b, 40, 1.0),
    );
    add("extent-zero", codes::FRAME_RECORD_FIELD, &base, &|b| {
        put_f64(b, 24, 0.0)
    });
    add("max-depth", codes::FRAME_RECORD_FIELD, &base, &|b| {
        b[32] = 32
    });
    add("mass-negative", codes::FRAME_RECORD_FIELD, &base, &|b| {
        put_f64(b, 40, -1.0)
    });
    add(
        "radius-without-j2",
        codes::FRAME_RECORD_SHAPE,
        &base,
        &|b| put_f64(b, 48, 0.0),
    );
    add("moments-triangle", codes::FRAME_RECORD_SHAPE, &base, &|b| {
        put_f64(b, 80, 1.7e38)
    });

    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("frame-record/invalid/{file}"), bytes));
    }
    out.push((
        "frame-record/invalid/index.json".into(),
        pretty(&json!({ "key": format!("frame-{INVALID_FRAME}"), "vectors": index })),
    ));
    out
}

/// Every frame record vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out
}
