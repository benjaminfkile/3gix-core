//! Checks the frame record conformance vectors under `conformance/frame-record/`
//! (`matter-format.md` section 11).
//!
//! Every valid record must validate under its key through the Rust API,
//! the top-level validator, and `gx_validate`, decode and re-encode to the
//! same bytes, and give its recorded `gx_frame_record_agrees` result against
//! `valid/registry.bin`, which [`frame_record::agrees`] must match. Every
//! file in `invalid/index.json` must fail with its code, and every code from
//! 1021 to 1029 must have a file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::error::codes;
use gx_core::frame_record::{self, Field};
use gx_core::{gx_frame_record_agrees, gx_validate, registry, validate};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/frame-record")
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(root().join(rel)).expect(rel)
}

fn read_json(rel: &str) -> Value {
    serde_json::from_slice(&read(rel)).expect("JSON")
}

fn ffi_validate(key: &str, bytes: &[u8]) -> i32 {
    // SAFETY: every pointer is valid for its length; no error buffer.
    unsafe {
        gx_validate(
            key.as_ptr(),
            key.len(),
            bytes.as_ptr(),
            bytes.len(),
            core::ptr::null_mut(),
            0,
        )
    }
}

fn ffi_agrees(registry: &[u8], record: &[u8]) -> i32 {
    // SAFETY: both pointers are valid for their lengths.
    unsafe {
        gx_frame_record_agrees(
            registry.as_ptr(),
            registry.len(),
            record.as_ptr(),
            record.len(),
        )
    }
}

fn field_code(f: Field) -> i32 {
    match f {
        Field::ParentFrameId => -2,
        Field::RootExtent => -3,
        Field::MaxDepth => -4,
        Field::Mass => -5,
        Field::J2 => -6,
        Field::ReferenceRadius => -7,
        Field::PrincipalMoments => -8,
    }
}

#[test]
fn valid_records() {
    let index = read_json("valid/index.json");
    let reg_bytes = read(
        index["registry"]
            .as_str()
            .map(|f| format!("valid/{f}"))
            .as_deref()
            .unwrap(),
    );
    let reg = registry::decode(&reg_bytes).expect("registry");
    let records = index["records"].as_object().expect("records");
    let mut empty = 0;
    for (file, want) in records {
        let bytes = read(&format!("valid/{file}"));
        let key = want["key"].as_str().expect("key");
        let id: u64 = key.strip_prefix("frame-").unwrap().parse().unwrap();
        assert_eq!(frame_record::validate(id, &bytes), Ok(()), "{file}");
        assert_eq!(validate(key, &bytes), Ok(()), "{file}");
        assert_eq!(ffi_validate(key, &bytes), 0, "{file}");
        let r = frame_record::decode(&bytes).expect(file);
        assert_eq!(frame_record::encode(&r), bytes, "{file}: re-encode differs");
        if r.empty {
            empty += 1;
            assert_eq!(r, frame_record::FrameRecord::empty(id), "{file}");
        }
        let expected = want["agrees"].as_i64().expect("agrees") as i32;
        assert_eq!(ffi_agrees(&reg_bytes, &bytes), expected, "{file}");
        let rust = match reg.frames().iter().find(|f| f.frame_id == id) {
            None => -1,
            Some(f) => frame_record::agrees(f, &r).map_or_else(field_code, |()| 0),
        };
        assert_eq!(rust, expected, "{file}");
    }
    assert_eq!(empty, 1, "one empty record");
    // Every registry frame's own record agrees.
    for f in reg.frames() {
        let r = frame_record::FrameRecord::from_registry_frame(f);
        assert_eq!(ffi_agrees(&reg_bytes, &frame_record::encode(&r)), 0);
    }
}

#[test]
fn invalid_records() {
    let index = read_json("invalid/index.json");
    let key = index["key"].as_str().expect("key");
    let id: u64 = key.strip_prefix("frame-").unwrap().parse().unwrap();
    let mut seen = BTreeSet::new();
    for (file, want) in index["vectors"].as_object().expect("vectors") {
        let bytes = read(&format!("invalid/{file}"));
        let want = u16::try_from(want.as_u64().expect("code")).unwrap();
        assert_eq!(
            frame_record::validate(id, &bytes).unwrap_err().code,
            want,
            "{file}"
        );
        assert_eq!(validate(key, &bytes).unwrap_err().code, want, "{file}");
        assert_eq!(ffi_validate(key, &bytes), i32::from(want), "{file}");
        seen.insert(want);
    }
    let all: BTreeSet<u16> = (codes::FRAME_RECORD_LENGTH..=codes::FRAME_RECORD_SHAPE).collect();
    assert_eq!(seen, all, "a vector for every code from 1021 to 1029");
}

#[test]
fn agreement_bad_inputs() {
    let reg = read("valid/registry.bin");
    let rec = read("valid/root.bin");
    // SAFETY: null pointers with non-zero lengths are refused, not read.
    let null = unsafe { gx_frame_record_agrees(core::ptr::null(), 3, rec.as_ptr(), rec.len()) };
    assert_eq!(null, -i32::from(codes::BYTES_POINTER_NULL));
    // SAFETY: as above.
    let null = unsafe { gx_frame_record_agrees(reg.as_ptr(), reg.len(), core::ptr::null(), 3) };
    assert_eq!(null, -i32::from(codes::BYTES_POINTER_NULL));
    assert_eq!(
        ffi_agrees(&rec, &rec),
        -i32::from(codes::REGISTRY_BAD_MAGIC)
    );
    assert_eq!(
        ffi_agrees(&reg, &reg),
        -i32::from(codes::FRAME_RECORD_LENGTH)
    );
    // The record's own frame_id names the frame: frame 3's fields under
    // frame_id 2 differ from frame 2 first at root_extent.
    let renamed = read("invalid/1026-id-mismatch.bin");
    assert_eq!(ffi_agrees(&reg, &renamed), -3);
}
