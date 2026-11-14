//! Checks the source manifest conformance vectors under
//! `conformance/manifest/` (`client-compute.md` section 5.1).
//!
//! Every `valid/NAME.bin` must decode to its recorded entries and hash and
//! re-encode to the same bytes. Every file in `invalid/index.json` must fail
//! with its code, through the Rust API and the C ABI, and every byte rule
//! must have a file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::error::codes;
use gx_core::gx_manifest_validate;
use gx_core::manifest;
use gx_core::sha256::to_hex;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/manifest")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

fn abi(bytes: &[u8]) -> i32 {
    let mut buf = [0u8; 256];
    unsafe {
        gx_manifest_validate(
            bytes.as_ptr(),
            bytes.len(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    }
}

#[test]
fn valid_vectors_decode_and_match() {
    for name in ["empty", "one", "three"] {
        let bytes = std::fs::read(root().join(format!("valid/{name}.bin"))).expect("bin");
        let want = read_json(&format!("valid/{name}.json"));
        let m = manifest::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(abi(&bytes), 0, "{name}");
        assert_eq!(manifest::encode(&m), bytes, "{name}: re-encode differs");
        assert_eq!(
            to_hex(&manifest::manifest_hash(&bytes)),
            want["hash"],
            "{name}"
        );
        assert_eq!(to_hex(&m.hash()), want["hash"], "{name}");
        let entries = want["entries"].as_array().expect("entries");
        assert_eq!(Some(m.entries().len() as u64), want["entry_count"].as_u64());
        assert_eq!(entries.len(), m.entries().len());
        for (e, w) in m.entries().iter().zip(entries) {
            let id: u64 = w["id"].as_str().unwrap().parse().unwrap();
            let length: u64 = w["length"].as_str().unwrap().parse().unwrap();
            assert_eq!(e.id, id, "{name}");
            assert_eq!(e.length, length, "{name}");
            assert_eq!(to_hex(&e.sha256), w["sha256"], "{name}");
            assert_eq!(m.lookup(id), Some((e.sha256, length)), "{name}");
        }
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
        let err = manifest::decode(&bytes).expect_err(file);
        assert_eq!(err.code, code, "{file}: {err}");
        assert_eq!(abi(&bytes), i32::from(code), "{file}");
        seen.insert(code);
    }
    let want: BTreeSet<u16> = [
        codes::MANIFEST_HEADER_TOO_SHORT,
        codes::MANIFEST_BAD_MAGIC,
        codes::MANIFEST_UNSUPPORTED_VERSION,
        codes::MANIFEST_RESERVED_NONZERO,
        codes::MANIFEST_LENGTH_MISMATCH,
        codes::MANIFEST_NOT_SORTED,
        codes::MANIFEST_DUPLICATE_ID,
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
