//! Checks the overlay tile conformance vectors under `conformance/overlay/`
//! (`matter-format.md` section 12, `client-compute.md` section 2.2).
//!
//! Every valid tile must validate under its key through [`overlay::validate`],
//! the top-level validator, and `gx_validate`, and decode and re-encode to
//! the same bytes. Every file in `invalid/index.json` must fail with its
//! code under an overlay layer (under `gx_validate`, bytes without the
//! `3GSO` magic are checked as a surface tile and fail with 981), and every code from 1051 to 1057 must have a file. Every case of
//! `layer/index.json` must give its code through [`validate_for_layer`] and
//! `gx_validate_layer`, and codes 1058 and 1059 must each have a case.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::key::{ChunkKey, SurfaceKey};
use gx_core::matter::Compression;
use gx_core::overlay::{self, Overlay, FLAG_ZSTD};
use gx_core::{gx_validate, gx_validate_layer, validate, validate_for_layer};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/overlay")
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(root().join(rel)).expect(rel)
}

fn read_json(rel: &str) -> Value {
    serde_json::from_slice(&read(rel)).expect("JSON")
}

fn surface_key(key: &str) -> SurfaceKey {
    match key.parse::<ChunkKey>().expect("key") {
        ChunkKey::Surface(k) => k,
        other => panic!("{other} is not a surface key"),
    }
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

fn ffi_layer(key: &str, bytes: &[u8], overlay: bool) -> i32 {
    let mut buf = [0u8; 64];
    // SAFETY: every pointer is valid for its length.
    unsafe {
        gx_validate_layer(
            key.as_ptr(),
            key.len(),
            bytes.as_ptr(),
            bytes.len(),
            i32::from(overlay),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    }
}

#[test]
fn valid_tiles() {
    let index = read_json("valid/index.json");
    let tiles = index["tiles"].as_object().unwrap();
    assert!(tiles.len() >= 4);
    for (file, want) in tiles {
        let key = want["key"].as_str().unwrap();
        let bytes = read(&format!("valid/{file}"));
        let k = surface_key(key);
        assert_eq!(validate(key, &bytes), Ok(()), "{file}");
        assert_eq!(ffi_validate(key, &bytes), 0, "{file}");
        match overlay::decode(&k, &bytes).expect(file) {
            Overlay::Empty(e) => assert_eq!(overlay::encode_empty(&e), bytes),
            Overlay::Tile(t) => {
                let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
                let c = if flags & FLAG_ZSTD != 0 {
                    Compression::Zstd
                } else {
                    Compression::None
                };
                assert_eq!(overlay::encode(&t, c), bytes, "{file}");
            }
        }
    }
}

#[test]
fn invalid_tiles() {
    let index = read_json("invalid/index.json");
    let key = index["key"].as_str().unwrap();
    let mut seen = BTreeSet::new();
    for (file, code) in index["vectors"].as_object().unwrap() {
        let code = code.as_u64().unwrap() as u16;
        let bytes = read(&format!("invalid/{file}"));
        // Under `gx_validate` bytes without the `3GSO` magic are a surface
        // tile, so the bad magic vector is checked under an overlay layer.
        let top = if code == 1052 { 981 } else { code };
        assert_eq!(validate(key, &bytes).unwrap_err().code, top, "{file}");
        assert_eq!(ffi_validate(key, &bytes), i32::from(top), "{file}");
        assert_eq!(ffi_layer(key, &bytes, true), i32::from(code), "{file}");
        assert_eq!(
            overlay::validate(&surface_key(key), &bytes)
                .unwrap_err()
                .code,
            code,
            "{file}"
        );
        seen.insert(code);
    }
    assert_eq!(seen, (1051..=1057).collect());
}

#[test]
fn layer_cases() {
    let index = read_json("layer/index.json");
    let mut seen = BTreeSet::new();
    for (file, case) in index["cases"].as_object().unwrap() {
        let key = case["key"].as_str().unwrap();
        let ov = case["overlay"].as_bool().unwrap();
        let code = case["code"].as_u64().unwrap() as u16;
        let bytes = read(&format!("layer/{file}"));
        let got = validate_for_layer(key, &bytes, ov).map_err(|e| e.code);
        assert_eq!(got, if code == 0 { Ok(()) } else { Err(code) }, "{file}");
        assert_eq!(ffi_layer(key, &bytes, ov), i32::from(code), "{file}");
        seen.insert(code);
    }
    assert!(seen.contains(&1058) && seen.contains(&1059) && seen.contains(&0));
}

#[test]
fn layer_ffi_null_pointers() {
    // SAFETY: null pointers with non-zero lengths are the cases under test.
    unsafe {
        assert_eq!(
            gx_validate_layer(
                core::ptr::null(),
                3,
                core::ptr::null(),
                0,
                1,
                core::ptr::null_mut(),
                0
            ),
            207
        );
        assert_eq!(
            gx_validate_layer(
                b"registry".as_ptr(),
                8,
                core::ptr::null(),
                4,
                1,
                core::ptr::null_mut(),
                0
            ),
            106
        );
    }
}
