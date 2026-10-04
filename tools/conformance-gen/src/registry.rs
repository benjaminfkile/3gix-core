//! Frame registry conformance vectors, written under `registry/`
//! (`matter-format.md` sections 5 and 8).
//!
//! - `valid/NAME.bin` is a valid registry and `valid/NAME.json` gives its
//!   epoch, frame count, and every record's fields. Every `f64` appears as a
//!   decimal string and as its bit pattern in hex.
//! - `invalid/CODE-RULE.bin` breaks exactly one rule one registry can be
//!   checked against. `invalid/index.json` maps each file name to its code.
//! - `union/NAME.bin` are registries that are each valid alone.
//!   `union/index.json` lists cases: which files form the union, in order,
//!   and either the expected tree or the expected code.
//!
//! Every value is a fixed literal. Orientations are rational unit
//! quaternions, so no trigonometry enters the output.

use std::collections::BTreeMap;

use gx_core::error::codes;
use gx_core::registry::{self, Frame, FrameTree, Registry, HEADER_LEN, RECORD_LEN, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};
use serde_json::{json, Value};

use crate::{number, pretty, File};

/// Epoch shared by every vector: seconds of Barycentric Dynamical Time since
/// J2000.
const EPOCH: f64 = 8.1e8;

#[allow(clippy::too_many_arguments)]
fn frame(
    frame_id: u64,
    parent_frame_id: u64,
    root_extent: f64,
    max_depth: u8,
    mass: f64,
    position: [f64; 3],
    velocity: [f64; 3],
    orientation: [f64; 4],
    angular_velocity: [f64; 3],
) -> Frame {
    let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
    let [qx, qy, qz, qw] = orientation;
    Frame {
        frame_id,
        parent_frame_id,
        root_extent: Meters::new(root_extent),
        max_depth,
        mass: Kilograms::new(mass),
        position: v(position),
        velocity: v(velocity),
        orientation: Quat::new(qx, qy, qz, qw),
        angular_velocity: v(angular_velocity),
    }
}

fn registry(epoch: f64, frames: Vec<Frame>) -> Registry {
    Registry::new(Seconds::new(epoch), frames).expect("fixed registries are valid")
}

/// The root of every tree: a heavy central mass with a slow spin.
fn root() -> Frame {
    frame(
        1,
        ROOT_PARENT,
        1.2e13,
        14,
        1.989e30,
        [0.0; 3],
        [0.0; 3],
        [0.0, 0.0, 0.6, 0.8],
        [0.0, 0.0, 2.865e-6],
    )
}

/// The four children of the root, in near circular motion around it.
fn children() -> [Frame; 4] {
    [
        frame(
            10,
            1,
            2.0e7,
            18,
            3.301e23,
            [5.79e10, 0.0, 0.0],
            [0.0, 4.736e4, 0.0],
            [0.28, 0.0, 0.0, 0.96],
            [0.0, 0.0, 1.24e-6],
        ),
        frame(
            20,
            1,
            4.0e7,
            19,
            4.867e24,
            [-7.6e10, 7.8e10, 0.0],
            [-2.47e4, -2.40e4, 1.1e2],
            [0.0, 0.6, 0.0, 0.8],
            [0.0, 0.0, -2.99e-7],
        ),
        frame(
            30,
            1,
            4.0e8,
            20,
            5.972e24,
            [0.0, -1.496e11, 2.0e6],
            [2.978e4, 0.0, 0.0],
            [0.48, 0.6, 0.0, 0.64],
            [0.0, 0.0, 7.292e-5],
        ),
        frame(
            40,
            1,
            2.0e7,
            19,
            6.417e23,
            [1.6e11, 1.6e11, -4.0e9],
            [-1.7e4, 1.7e4, 0.0],
            [0.5, 0.5, 0.5, 0.5],
            [0.0, 0.0, 7.088e-5],
        ),
    ]
}

/// A satellite of frame 30.
fn grandchild() -> Frame {
    frame(
        31,
        30,
        8.0e6,
        17,
        7.342e22,
        [3.844e8, 0.0, 0.0],
        [0.0, 1.022e3, 0.0],
        [0.0, 0.0, -0.28, 0.96],
        [0.0, 0.0, 2.6617e-6],
    )
}

/// A small, distant child of the root, unrelated to the others.
fn far_child() -> Frame {
    frame(
        50,
        1,
        2.0e6,
        16,
        9.4e20,
        [-4.1e11, 0.0, 1.0e10],
        [0.0, -1.79e4, 0.0],
        [0.0, 0.8, 0.0, 0.6],
        [0.0, 0.0, 1.9e-5],
    )
}

fn tree() -> Registry {
    let mut frames = vec![root(), grandchild()];
    frames.extend(children());
    registry(EPOCH, frames)
}

fn summary(r: &Registry) -> Value {
    let v3 = |v: Vec3| json!([number(v.x), number(v.y), number(v.z)]);
    let frames: Vec<Value> = r
        .frames()
        .iter()
        .map(|f| {
            let q = f.orientation;
            json!({
                "frame_id": f.frame_id,
                "parent_frame_id": f.parent_frame_id,
                "root_extent": number(f.root_extent.value()),
                "max_depth": f.max_depth,
                "mass": number(f.mass.value()),
                "position": v3(f.position),
                "velocity": v3(f.velocity),
                "orientation": [number(q.x), number(q.y), number(q.z), number(q.w)],
                "angular_velocity": v3(f.angular_velocity),
            })
        })
        .collect();
    json!({
        "epoch": number(r.epoch().value()),
        "frame_count": r.frames().len(),
        "frames": frames,
    })
}

fn valid() -> Vec<File> {
    let regs = [
        ("empty", registry(EPOCH, vec![])),
        ("single-root", registry(EPOCH, vec![root()])),
        ("tree", tree()),
    ];
    let mut out = Vec::new();
    for (name, r) in regs {
        out.push((format!("registry/valid/{name}.bin"), registry::encode(&r)));
        out.push((format!("registry/valid/{name}.json"), pretty(&summary(&r))));
    }
    out
}

fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn put_f64(b: &mut [u8], at: usize, v: f64) {
    put_u64(b, at, v.to_bits());
}

/// Byte offset of record `i`, plus `field`.
fn rec(i: usize, field: usize) -> usize {
    HEADER_LEN + RECORD_LEN * i + field
}

fn invalid() -> Vec<File> {
    // Records of the tree in order: 0 root (1), 1 (10), 2 (20), 3 (30),
    // 4 (31), 5 (40).
    let base = registry::encode(&tree());
    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = base.clone();
        f(&mut b);
        vectors.push((name, code, b));
    };

    // Header.
    add("header-too-short", codes::REGISTRY_HEADER_TOO_SHORT, &|b| {
        b.truncate(HEADER_LEN - 1)
    });
    add(
        "header-zero-length",
        codes::REGISTRY_HEADER_TOO_SHORT,
        &|b| b.clear(),
    );
    add("bad-magic", codes::REGISTRY_BAD_MAGIC, &|b| b[3] = 0x53);
    add(
        "unsupported-version",
        codes::REGISTRY_UNSUPPORTED_VERSION,
        &|b| b[4] = 2,
    );
    add(
        "reserved-u16-nonzero",
        codes::REGISTRY_RESERVED_U16_NONZERO,
        &|b| b[7] = 0x80,
    );
    add(
        "reserved-u32-nonzero",
        codes::REGISTRY_RESERVED_U32_NONZERO,
        &|b| b[20] = 1,
    );
    add("epoch-nan", codes::REGISTRY_EPOCH_NOT_FINITE, &|b| {
        put_f64(b, 8, f64::NAN)
    });
    add("epoch-infinite", codes::REGISTRY_EPOCH_NOT_FINITE, &|b| {
        put_f64(b, 8, f64::NEG_INFINITY)
    });
    add("length-short", codes::REGISTRY_LENGTH_MISMATCH, &|b| {
        b.pop();
    });
    add("length-long", codes::REGISTRY_LENGTH_MISMATCH, &|b| {
        b.push(0)
    });
    add("frame-count-high", codes::REGISTRY_LENGTH_MISMATCH, &|b| {
        b[16] += 1
    });
    add("frame-count-zero", codes::REGISTRY_LENGTH_MISMATCH, &|b| {
        b[16] = 0
    });

    // Records.
    add(
        "record-reserved-nonzero",
        codes::FRAME_RESERVED_NONZERO,
        &|b| b[rec(2, 28)] = 1,
    );
    add("records-not-sorted", codes::FRAME_NOT_SORTED, &|b| {
        let one = b[rec(1, 0)..rec(2, 0)].to_vec();
        let two = b[rec(2, 0)..rec(3, 0)].to_vec();
        b[rec(1, 0)..rec(2, 0)].copy_from_slice(&two);
        b[rec(2, 0)..rec(3, 0)].copy_from_slice(&one);
    });
    add("duplicate-id", codes::FRAME_DUPLICATE_ID, &|b| {
        put_u64(b, rec(2, 0), 10)
    });
    add("extent-nan", codes::FRAME_EXTENT_NOT_FINITE, &|b| {
        put_f64(b, rec(1, 16), f64::NAN)
    });
    add("extent-infinite", codes::FRAME_EXTENT_NOT_FINITE, &|b| {
        put_f64(b, rec(1, 16), f64::INFINITY)
    });
    add("extent-zero", codes::FRAME_EXTENT_NOT_POSITIVE, &|b| {
        put_f64(b, rec(1, 16), 0.0)
    });
    add("extent-negative", codes::FRAME_EXTENT_NOT_POSITIVE, &|b| {
        put_f64(b, rec(1, 16), -2.0e7)
    });
    add("max-depth-32", codes::FRAME_MAX_DEPTH_OUT_OF_RANGE, &|b| {
        b[rec(3, 24)] = 32
    });
    add("max-depth-255", codes::FRAME_MAX_DEPTH_OUT_OF_RANGE, &|b| {
        b[rec(3, 24)] = 255
    });
    add("mass-infinite", codes::FRAME_MASS_NOT_FINITE, &|b| {
        put_f64(b, rec(4, 32), f64::INFINITY)
    });
    add("mass-negative", codes::FRAME_MASS_NEGATIVE, &|b| {
        put_f64(b, rec(4, 32), -7.342e22)
    });
    add("position-nan", codes::FRAME_POSITION_NOT_FINITE, &|b| {
        put_f64(b, rec(5, 56), f64::NAN)
    });
    add(
        "velocity-infinite",
        codes::FRAME_VELOCITY_NOT_FINITE,
        &|b| put_f64(b, rec(5, 64), f64::INFINITY),
    );
    add(
        "orientation-nan",
        codes::FRAME_ORIENTATION_NOT_FINITE,
        &|b| put_f64(b, rec(2, 112), f64::NAN),
    );
    add(
        "orientation-zero",
        codes::FRAME_ORIENTATION_NOT_UNIT,
        &|b| {
            for c in 0..4 {
                put_f64(b, rec(2, 88 + 8 * c), 0.0);
            }
        },
    );
    add(
        "orientation-slightly-long",
        codes::FRAME_ORIENTATION_NOT_UNIT,
        &|b| {
            for (c, v) in [0.0, 0.0, 0.0, 1.0 + 2.0e-9].into_iter().enumerate() {
                put_f64(b, rec(2, 88 + 8 * c), v);
            }
        },
    );
    add(
        "angular-velocity-nan",
        codes::FRAME_ANGULAR_VELOCITY_NOT_FINITE,
        &|b| put_f64(b, rec(1, 136), f64::NAN),
    );
    add(
        "root-position-nonzero",
        codes::ROOT_POSITION_NONZERO,
        &|b| put_f64(b, rec(0, 40), 1.0),
    );
    add(
        "root-velocity-nonzero",
        codes::ROOT_VELOCITY_NONZERO,
        &|b| put_f64(b, rec(0, 72), -1.0e-3),
    );

    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("registry/invalid/{file}"), bytes));
    }
    out.push((
        "registry/invalid/index.json".into(),
        pretty(&json!({ "vectors": index })),
    ));
    out
}

/// One union case: files in order and the expected code, or `None` for a
/// valid union.
struct Case {
    name: &'static str,
    files: &'static [&'static str],
    code: Option<u16>,
}

/// `union/valid-a.bin`: the root and its four children.
pub(crate) fn valid_a() -> Registry {
    let mut a = vec![root()];
    a.extend(children());
    registry(EPOCH, a)
}

/// `union/valid-b.bin`: the frames that hang below `valid_a`'s children.
pub(crate) fn valid_b() -> Registry {
    registry(EPOCH, vec![grandchild(), far_child()])
}

fn union() -> Vec<File> {
    let regs: [(&str, Registry); 9] = [
        ("valid-a", valid_a()),
        ("valid-b", valid_b()),
        ("empty", registry(EPOCH, vec![])),
        (
            "valid-b-later-epoch",
            registry(EPOCH + 1.0, vec![grandchild(), far_child()]),
        ),
        (
            "duplicate-of-a",
            registry(EPOCH, vec![grandchild(), children()[1]]),
        ),
        (
            "second-root",
            registry(
                EPOCH,
                vec![Frame {
                    frame_id: 2,
                    ..root()
                }],
            ),
        ),
        (
            "missing-parent",
            registry(
                EPOCH,
                vec![Frame {
                    frame_id: 60,
                    parent_frame_id: 99,
                    ..far_child()
                }],
            ),
        ),
        (
            "cycle",
            registry(
                EPOCH,
                vec![
                    Frame {
                        frame_id: 70,
                        parent_frame_id: 71,
                        ..far_child()
                    },
                    Frame {
                        frame_id: 71,
                        parent_frame_id: 70,
                        ..far_child()
                    },
                ],
            ),
        ),
        (
            "self-parent",
            registry(
                EPOCH,
                vec![Frame {
                    frame_id: 80,
                    parent_frame_id: 80,
                    ..far_child()
                }],
            ),
        ),
    ];
    let cases = [
        Case {
            name: "valid-pair",
            files: &["valid-a", "valid-b"],
            code: None,
        },
        Case {
            name: "valid-pair-reversed-with-empty",
            files: &["empty", "valid-b", "valid-a"],
            code: None,
        },
        Case {
            name: "epoch-mismatch",
            files: &["valid-a", "valid-b-later-epoch"],
            code: Some(codes::UNION_EPOCH_MISMATCH),
        },
        Case {
            name: "duplicate-id",
            files: &["valid-a", "duplicate-of-a"],
            code: Some(codes::UNION_DUPLICATE_ID),
        },
        Case {
            name: "no-root",
            files: &["valid-b", "empty"],
            code: Some(codes::UNION_NO_ROOT),
        },
        Case {
            name: "only-empty",
            files: &["empty", "empty"],
            code: Some(codes::UNION_NO_ROOT),
        },
        Case {
            name: "two-roots",
            files: &["valid-a", "second-root"],
            code: Some(codes::UNION_MULTIPLE_ROOTS),
        },
        Case {
            name: "missing-parent",
            files: &["valid-a", "missing-parent"],
            code: Some(codes::UNION_PARENT_MISSING),
        },
        Case {
            name: "cycle",
            files: &["valid-a", "cycle"],
            code: Some(codes::UNION_CYCLE),
        },
        Case {
            name: "self-parent",
            files: &["valid-a", "self-parent"],
            code: Some(codes::UNION_CYCLE),
        },
    ];

    let lookup = |name: &str| -> &Registry {
        &regs
            .iter()
            .find(|(n, _)| *n == name)
            .expect("case names a registry")
            .1
    };
    let mut index = BTreeMap::new();
    for case in &cases {
        let members: Vec<Registry> = case.files.iter().map(|n| lookup(n).clone()).collect();
        let got = FrameTree::from_registries(&members);
        let files: Vec<String> = case.files.iter().map(|n| format!("{n}.bin")).collect();
        let entry = match (case.code, got) {
            (None, Ok(t)) => {
                let paths: BTreeMap<String, Vec<u64>> = t
                    .frames()
                    .iter()
                    .map(|f| (f.frame_id.to_string(), t.path_to_root(f.frame_id)))
                    .collect();
                json!({
                    "files": files,
                    "outcome": "ok",
                    "root": t.root().frame_id,
                    "frame_count": t.frames().len(),
                    "paths_to_root": paths,
                })
            }
            (Some(code), Err(e)) if e.code == code => json!({
                "files": files,
                "outcome": "error",
                "code": code,
            }),
            (want, got) => panic!("union case {}: expected {want:?}, got {got:?}", case.name),
        };
        index.insert(case.name, entry);
    }

    let mut out: Vec<File> = regs
        .iter()
        .map(|(n, r)| (format!("registry/union/{n}.bin"), registry::encode(r)))
        .collect();
    out.push((
        "registry/union/index.json".into(),
        pretty(&json!({ "cases": index })),
    ));
    out
}

/// Every registry vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out.extend(union());
    out
}
