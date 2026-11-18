//! Checks the epoch index conformance vectors under `conformance/epochs/`
//! (`client-compute.md` section 5.4).
//!
//! Every `valid/NAME.bin` must decode to its recorded epoch seconds, entries,
//! and hash, re-encode to the same bytes, and give every recorded epoch rule
//! case through the Rust API and the C ABI. Every file in
//! `invalid/index.json` must fail with its code, through the Rust API and
//! both C ABI functions, and every byte rule must have a file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::epochs;
use gx_core::error::codes;
use gx_core::sha256::to_hex;
use gx_core::{gx_epochs_manifest, gx_epochs_validate};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/epochs")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

fn int(v: &Value) -> u64 {
    v.as_str().expect("decimal string").parse().expect("u64")
}

fn abi_validate(bytes: &[u8]) -> i32 {
    let mut buf = [0u8; 256];
    unsafe {
        gx_epochs_validate(
            bytes.as_ptr(),
            bytes.len(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    }
}

fn abi_manifest(bytes: &[u8], span_seconds: u64, span_index: u64) -> (i32, [u8; 32]) {
    let mut out = [0x7fu8; 32];
    let rc = unsafe {
        gx_epochs_manifest(
            bytes.as_ptr(),
            bytes.len(),
            span_seconds,
            span_index,
            out.as_mut_ptr(),
        )
    };
    (rc, out)
}

#[test]
fn valid_vectors_decode_and_match() {
    for name in ["daily", "empty", "widest"] {
        let bytes = std::fs::read(root().join(format!("valid/{name}.bin"))).expect("bin");
        let want = read_json(&format!("valid/{name}.json"));
        let x = epochs::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(abi_validate(&bytes), 0, "{name}");
        assert_eq!(epochs::encode(&x), bytes, "{name}: re-encode differs");
        assert_eq!(to_hex(&epochs::epochs_hash(&bytes)), want["hash"], "{name}");
        assert_eq!(to_hex(&x.hash()), want["hash"], "{name}");
        assert_eq!(x.epoch_seconds(), int(&want["epoch_seconds"]), "{name}");
        let entries = want["entries"].as_array().expect("entries");
        assert_eq!(Some(x.entries().len() as u64), want["entry_count"].as_u64());
        assert_eq!(entries.len(), x.entries().len());
        for (e, w) in x.entries().iter().zip(entries) {
            assert_eq!(e.epoch, int(&w["epoch"]), "{name}");
            assert_eq!(to_hex(&e.manifest), w["manifest"], "{name}");
        }

        let cases = want["cases"].as_array().expect("cases");
        assert!(!cases.is_empty(), "{name}: no cases");
        for c in cases {
            let span_seconds = int(&c["span_seconds"]);
            let span_index = int(&c["span_index"]);
            let what = format!("{name}: span {span_seconds} s, index {span_index}");
            let got = epochs::manifest_for_span(&x, span_seconds, span_index);
            let (rc, out) = abi_manifest(&bytes, span_seconds, span_index);
            if let Some(code) = c.get("code") {
                let code = u16::try_from(code.as_u64().expect("code")).expect("u16");
                assert_eq!(got.expect_err(&what).code, code, "{what}");
                assert_eq!(
                    epochs::check_layer(&x, span_seconds).unwrap_err().code,
                    code
                );
                assert_eq!(rc, -i32::from(code), "{what}");
                assert_eq!(out, [0x7f; 32], "{what}: failure leaves out32");
                continue;
            }
            // The rule, recomputed here independently of the library.
            let epoch =
                u128::from(span_index) * u128::from(span_seconds) / u128::from(x.epoch_seconds());
            let listed = x
                .entries()
                .iter()
                .find(|e| u128::from(e.epoch) == epoch)
                .map(|e| e.manifest);
            let got = got.unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(got, listed, "{what}");
            match &c["manifest"] {
                Value::Null => {
                    assert_eq!(got, None, "{what}");
                    assert_eq!(rc, 1, "{what}");
                    assert_eq!(out, [0x7f; 32], "{what}: none leaves out32");
                }
                hex => {
                    let h = got.expect(&what);
                    assert_eq!(to_hex(&h), *hex, "{what}");
                    assert_eq!(rc, 0, "{what}");
                    assert_eq!(out, h, "{what}");
                }
            }
        }
    }
}

#[test]
fn every_rule_has_a_case() {
    let daily = read_json("valid/daily.json");
    let cases = daily["cases"].as_array().expect("cases");
    let codes_seen: BTreeSet<u64> = cases.iter().filter_map(|c| c["code"].as_u64()).collect();
    assert!(codes_seen.contains(&u64::from(codes::EPOCHS_SPAN_SECONDS_ZERO)));
    assert!(codes_seen.contains(&u64::from(codes::EPOCHS_SPAN_NOT_DIVISOR)));
    assert!(
        cases
            .iter()
            .any(|c| c["manifest"].is_null() && c.get("code").is_none()),
        "an unlisted epoch"
    );
    assert!(
        cases.iter().any(|c| int(&c["span_index"]) == u64::MAX),
        "a large span index"
    );
}

#[test]
fn null_pointers() {
    let bytes = std::fs::read(root().join("valid/daily.bin")).expect("bin");
    let mut out = [0u8; 32];
    unsafe {
        assert_eq!(
            gx_epochs_validate(core::ptr::null(), 3, core::ptr::null_mut(), 0),
            106
        );
        assert_eq!(
            gx_epochs_manifest(core::ptr::null(), 3, 3600, 0, out.as_mut_ptr()),
            -106
        );
        assert_eq!(
            gx_epochs_manifest(bytes.as_ptr(), bytes.len(), 3600, 0, core::ptr::null_mut()),
            -i32::from(codes::EPOCHS_POINTER_NULL)
        );
    }
}

#[test]
fn invalid_vectors_fail_with_their_code() {
    let index = read_json("invalid/index.json");
    let vectors = index["vectors"].as_object().expect("vectors");
    let mut seen = BTreeSet::new();
    for (file, code) in vectors {
        let code = u16::try_from(code.as_u64().expect("code")).expect("u16");
        assert!(file.starts_with(&format!("{code}-")), "{file}: name/code");
        let bytes = std::fs::read(root().join("invalid").join(file)).expect(file);
        let err = epochs::decode(&bytes).expect_err(file);
        assert_eq!(err.code, code, "{file}: {err}");
        assert_eq!(abi_validate(&bytes), i32::from(code), "{file}");
        assert_eq!(abi_manifest(&bytes, 3600, 0).0, -i32::from(code), "{file}");
        seen.insert(code);
    }
    let want: BTreeSet<u16> = [
        codes::EPOCHS_HEADER_TOO_SHORT,
        codes::EPOCHS_BAD_MAGIC,
        codes::EPOCHS_UNSUPPORTED_VERSION,
        codes::EPOCHS_RESERVED_NONZERO,
        codes::EPOCHS_ZERO_EPOCH_SECONDS,
        codes::EPOCHS_LENGTH_MISMATCH,
        codes::EPOCHS_NOT_ASCENDING,
    ]
    .into_iter()
    .collect();
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
