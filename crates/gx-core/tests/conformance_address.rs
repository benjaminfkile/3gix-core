//! Checks the section address and span index conformance vectors under
//! `conformance/address/` (`client-compute.md` sections 2.4 and 3).
//!
//! Every case in `cases.json` must give its recorded preimage and address,
//! and the C ABI must agree. Every case in `spans.json` must give its span
//! index or code.

use std::path::{Path, PathBuf};

use gx_core::address::{address_preimage, section_address, span_index, to_hex};
use gx_core::gx_section_address;
use gx_core::key::ChunkKey;
use gx_core::sha256::{self, from_hex};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/address")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

fn int<T: std::str::FromStr>(v: &Value) -> T
where
    T::Err: std::fmt::Debug,
{
    v.as_str()
        .expect("integers are strings")
        .parse()
        .expect("integer")
}

#[test]
fn address_cases_match() {
    let index = read_json("cases.json");
    let cases = index["cases"].as_array().expect("cases");
    assert!(cases.len() >= 12);
    let (mut registry, mut with_manifest, mut origin_on, mut spans) = (0, 0, 0, 0);
    let mut depths = std::collections::BTreeSet::new();
    for c in cases {
        let name = c["name"].as_str().expect("name");
        let module = from_hex(c["module"].as_str().expect("module")).expect("hex");
        let manifest = c["manifest"].as_str().map(|m| from_hex(m).expect("hex"));
        let origin: i64 = int(&c["origin_utc_ms"]);
        let span_seconds: u64 = int(&c["span_seconds"]);
        let span: u64 = int(&c["span_index"]);
        let key_str = c["key"].as_str().expect("key");
        let key: ChunkKey = key_str.parse().expect("key parses");
        let uses_origin = c["uses_origin"].as_bool().expect("uses_origin");
        if !uses_origin {
            assert_eq!(origin, 0, "{name}: usesOrigin off writes origin 0");
        }

        let pre = address_preimage(&module, manifest.as_ref(), origin, span_seconds, span, &key);
        assert_eq!(sha256::to_hex(&pre), c["preimage"], "{name}: preimage");
        let a = section_address(&module, manifest.as_ref(), origin, span_seconds, span, &key);
        assert_eq!(to_hex(&a), c["address"], "{name}: address");

        let mut out = [0u8; 32];
        let rc = unsafe {
            gx_section_address(
                module.as_ptr(),
                manifest.as_ref().map_or(core::ptr::null(), |m| m.as_ptr()),
                origin,
                span_seconds,
                span,
                key_str.as_ptr(),
                key_str.len(),
                out.as_mut_ptr(),
            )
        };
        assert_eq!(rc, 0, "{name}");
        assert_eq!(out, a, "{name}: C ABI");

        match key {
            ChunkKey::Registry => registry += 1,
            ChunkKey::Cell(cell) => {
                depths.insert(cell.depth);
            }
            ChunkKey::Surface(_) | ChunkKey::Frame(_) => {}
        }
        with_manifest += usize::from(manifest.is_some());
        origin_on += usize::from(uses_origin);
        spans += usize::from(span_seconds > 0);
    }
    assert!(registry >= 1 && depths.len() >= 3);
    assert!(with_manifest >= 1 && with_manifest < cases.len());
    assert!(origin_on >= 1 && origin_on < cases.len());
    assert!(spans >= 1);
}

#[test]
fn span_cases_match() {
    let index = read_json("spans.json");
    for c in index["cases"].as_array().expect("cases") {
        let t: i64 = int(&c["t_utc_ms"]);
        let origin: i64 = int(&c["origin_utc_ms"]);
        let span: u64 = int(&c["span_seconds"]);
        let got = span_index(t, origin, span);
        match c.get("span_index") {
            Some(want) => assert_eq!(got, Ok(int::<u64>(want)), "{c}"),
            None => assert_eq!(
                Some(u64::from(got.expect_err("expected an error").code)),
                c["code"].as_u64(),
                "{c}"
            ),
        }
    }
}
