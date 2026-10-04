//! Checks the frame registry conformance vectors under
//! `conformance/registry/` (`matter-format.md` sections 5 and 8).
//!
//! Every `valid/NAME.bin` must decode and match every recorded value bit for
//! bit, and re-encode to the same bytes. Every file in `invalid/index.json`
//! must fail with exactly its listed code, and every code one registry's
//! bytes can trigger must have at least one file. Every case in
//! `union/index.json` must produce its recorded tree or code, and every
//! union rule must have a case.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::error::codes;
use gx_core::registry::{self, FrameTree, Registry};
use gx_core::units::Vec3;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/registry")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

/// Reads the `bits` of a recorded number and checks `decimal` agrees.
fn number(v: &Value) -> u64 {
    let bits = v["bits"].as_str().expect("bits is a string");
    let bits = u64::from_str_radix(bits.trim_start_matches("0x"), 16).expect("hex bits");
    let decimal: f64 = v["decimal"]
        .as_str()
        .expect("decimal is a string")
        .parse()
        .expect("decimal parses");
    assert_eq!(decimal.to_bits(), bits, "decimal and bits disagree in {v}");
    bits
}

fn check(name: &str, field: &str, want: &Value, got: f64) {
    assert_eq!(
        number(want),
        got.to_bits(),
        "{name}: {field} is {got:e}, expected {}",
        want["decimal"]
    );
}

fn check_vec(name: &str, field: &str, want: &Value, got: Vec3) {
    for (i, g) in [got.x, got.y, got.z].into_iter().enumerate() {
        check(name, field, &want[i], g);
    }
}

#[test]
fn valid_vectors_decode_and_match() {
    let mut names: Vec<String> = std::fs::read_dir(root().join("valid"))
        .expect("valid directory")
        .map(|e| e.expect("entry").file_name().into_string().expect("UTF-8"))
        .filter_map(|n| n.strip_suffix(".bin").map(str::to_string))
        .collect();
    names.sort();
    assert_eq!(names, ["empty", "single-root", "tree"]);
    for name in &names {
        let bytes = std::fs::read(root().join(format!("valid/{name}.bin"))).expect("bin");
        let want = read_json(&format!("valid/{name}.json"));
        let r = registry::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(registry::validate(&bytes).is_ok());
        assert_eq!(registry::encode(&r), bytes, "{name}: re-encode differs");

        check(name, "epoch", &want["epoch"], r.epoch().value());
        let count = want["frame_count"].as_u64().expect("frame_count");
        assert_eq!(r.frames().len() as u64, count, "{name}: frame_count");
        let frames = want["frames"].as_array().expect("frames");
        assert_eq!(frames.len() as u64, count);
        for (f, w) in r.frames().iter().zip(frames) {
            assert_eq!(Some(f.frame_id), w["frame_id"].as_u64(), "{name}");
            assert_eq!(
                Some(f.parent_frame_id),
                w["parent_frame_id"].as_u64(),
                "{name}"
            );
            assert_eq!(
                Some(u64::from(f.max_depth)),
                w["max_depth"].as_u64(),
                "{name}"
            );
            check(
                name,
                "root_extent",
                &w["root_extent"],
                f.root_extent.value(),
            );
            check(name, "mass", &w["mass"], f.mass.value());
            check_vec(name, "position", &w["position"], f.position);
            check_vec(name, "velocity", &w["velocity"], f.velocity);
            let q = f.orientation;
            for (i, g) in [q.x, q.y, q.z, q.w].into_iter().enumerate() {
                check(name, "orientation", &w["orientation"][i], g);
            }
            check_vec(
                name,
                "angular_velocity",
                &w["angular_velocity"],
                f.angular_velocity,
            );
        }
    }

    // The tree: a root, four children, one grandchild, every orientation
    // other than the identity.
    let r = registry::decode(&std::fs::read(root().join("valid/tree.bin")).unwrap()).unwrap();
    let t = FrameTree::from_registries(std::slice::from_ref(&r)).expect("tree is a tree");
    assert_eq!(t.children(t.root().frame_id).len(), 4);
    assert_eq!(
        t.frames()
            .iter()
            .filter(|f| t.depth(f.frame_id) == 2)
            .count(),
        1
    );
    assert!(r.frames().iter().all(|f| f.orientation.w != 1.0));
}

/// Codes the bytes of one registry can trigger: 601 to 625 except 608, which
/// only the Rust constructor reaches.
const BYTE_CODES: &[u16] = &[
    codes::REGISTRY_HEADER_TOO_SHORT,
    codes::REGISTRY_BAD_MAGIC,
    codes::REGISTRY_UNSUPPORTED_VERSION,
    codes::REGISTRY_RESERVED_U16_NONZERO,
    codes::REGISTRY_RESERVED_U32_NONZERO,
    codes::REGISTRY_EPOCH_NOT_FINITE,
    codes::REGISTRY_LENGTH_MISMATCH,
    codes::FRAME_RESERVED_NONZERO,
    codes::FRAME_NOT_SORTED,
    codes::FRAME_DUPLICATE_ID,
    codes::FRAME_EXTENT_NOT_FINITE,
    codes::FRAME_EXTENT_NOT_POSITIVE,
    codes::FRAME_MAX_DEPTH_OUT_OF_RANGE,
    codes::FRAME_MASS_NOT_FINITE,
    codes::FRAME_MASS_NEGATIVE,
    codes::FRAME_POSITION_NOT_FINITE,
    codes::FRAME_VELOCITY_NOT_FINITE,
    codes::FRAME_ORIENTATION_NOT_FINITE,
    codes::FRAME_ORIENTATION_NOT_UNIT,
    codes::FRAME_ANGULAR_VELOCITY_NOT_FINITE,
    codes::ROOT_POSITION_NONZERO,
    codes::ROOT_VELOCITY_NONZERO,
];

#[test]
fn invalid_vectors_fail_with_their_code() {
    let index = read_json("invalid/index.json");
    let vectors = index["vectors"].as_object().expect("vectors");
    let mut seen = BTreeSet::new();
    for (file, code) in vectors {
        let code = u16::try_from(code.as_u64().expect("code")).expect("u16 code");
        assert!(file.starts_with(&format!("{code}-")), "{file}: name/code");
        let bytes = std::fs::read(root().join("invalid").join(file)).expect(file);
        let err = registry::decode(&bytes).expect_err(file);
        assert_eq!(err.code, code, "{file}: {err}");
        assert_eq!(registry::validate(&bytes).unwrap_err().code, code);
        seen.insert(code);
    }
    let want: BTreeSet<u16> = BYTE_CODES.iter().copied().collect();
    assert_eq!(seen, want, "every byte rule has a vector and nothing else");

    let mut on_disk: Vec<String> = std::fs::read_dir(root().join("invalid"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    on_disk.sort();
    let listed: Vec<String> = vectors.keys().cloned().collect();
    assert_eq!(on_disk, listed, "index and directory agree");
}

#[test]
fn union_cases_match() {
    let index = read_json("union/index.json");
    let cases = index["cases"].as_object().expect("cases");
    let mut seen = BTreeSet::new();
    let mut ok_cases = 0;
    for (name, case) in cases {
        let regs: Vec<Registry> = case["files"]
            .as_array()
            .expect("files")
            .iter()
            .map(|f| {
                let f = f.as_str().expect("file name");
                let bytes = std::fs::read(root().join("union").join(f)).expect(f);
                registry::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {f}: {e}"))
            })
            .collect();
        let got = FrameTree::from_registries(&regs);
        match case["outcome"].as_str().expect("outcome") {
            "ok" => {
                ok_cases += 1;
                let t = got.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(Some(t.root().frame_id), case["root"].as_u64(), "{name}");
                assert_eq!(
                    Some(t.frames().len() as u64),
                    case["frame_count"].as_u64(),
                    "{name}"
                );
                let paths = case["paths_to_root"].as_object().expect("paths");
                assert_eq!(paths.len(), t.frames().len());
                for (id, path) in paths {
                    let id: u64 = id.parse().expect("id");
                    let want: Vec<u64> = path
                        .as_array()
                        .expect("path")
                        .iter()
                        .map(|v| v.as_u64().expect("id"))
                        .collect();
                    assert_eq!(t.path_to_root(id), want, "{name}: path from {id}");
                    assert_eq!(t.depth(id), want.len() - 1, "{name}: depth of {id}");
                }
            }
            "error" => {
                let code = case["code"].as_u64().expect("code") as u16;
                let err = got.expect_err(name);
                assert_eq!(err.code, code, "{name}: {err}");
                seen.insert(code);
            }
            other => panic!("{name}: unknown outcome {other}"),
        }
    }
    assert!(ok_cases >= 1);
    let want: BTreeSet<u16> = [
        codes::UNION_EPOCH_MISMATCH,
        codes::UNION_DUPLICATE_ID,
        codes::UNION_NO_ROOT,
        codes::UNION_MULTIPLE_ROOTS,
        codes::UNION_PARENT_MISSING,
        codes::UNION_CYCLE,
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, want, "every union rule has a case");
}

#[test]
fn valid_pair_grandchild_path_has_length_three() {
    let load = |f: &str| registry::decode(&std::fs::read(root().join("union").join(f)).unwrap());
    let a = load("valid-a.bin").unwrap();
    let b = load("valid-b.bin").unwrap();
    let t = FrameTree::from_registries(&[a, b]).expect("valid pair");
    let grandchild = t
        .frames()
        .iter()
        .find(|f| t.depth(f.frame_id) == 2)
        .expect("a grandchild");
    let path = t.path_to_root(grandchild.frame_id);
    assert_eq!(path.len(), 3);
    assert_eq!(path[0], grandchild.frame_id);
    assert_eq!(path[2], t.root().frame_id);
}
