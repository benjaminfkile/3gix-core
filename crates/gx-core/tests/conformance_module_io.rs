//! Checks the module request, tile, and source id conformance vectors under
//! `conformance/module-io/` (`client-compute.md` section 4).
//!
//! Every valid file must decode to its recorded values and re-encode to the
//! same bytes. Every invalid file must fail with its code, and every byte
//! rule must have a file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::error::{codes, ValidationError};
use gx_core::module_io::{
    decode_request, decode_source_ids, decode_tiles, encode_request, encode_source_ids,
    encode_tiles,
};
use gx_core::sha256::to_hex;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/module-io")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(root().join(rel)).expect(rel)
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
fn valid_requests() {
    for name in ["cell", "registry", "static-cell"] {
        let bytes = read(&format!("request/valid/{name}.bin"));
        let want = read_json(&format!("request/valid/{name}.json"));
        let r = decode_request(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            r.origin_utc_ms,
            int::<i64>(&want["origin_utc_ms"]),
            "{name}"
        );
        assert_eq!(r.span_seconds, int::<u64>(&want["span_seconds"]), "{name}");
        assert_eq!(r.span_index, int::<u64>(&want["span_index"]), "{name}");
        assert_eq!(r.key.to_string(), want["key"], "{name}");
        assert_eq!(encode_request(&r), bytes, "{name}");
    }
}

#[test]
fn valid_tiles() {
    for name in ["empty", "three"] {
        let bytes = read(&format!("tiles/valid/{name}.bin"));
        let want = read_json(&format!("tiles/valid/{name}.json"));
        let tiles = decode_tiles(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let list = want["tiles"].as_array().expect("tiles");
        assert_eq!(Some(tiles.len() as u64), want["count"].as_u64());
        assert_eq!(tiles.len(), list.len());
        for (t, w) in tiles.iter().zip(list) {
            assert_eq!(t.id, int::<u64>(&w["id"]), "{name}");
            assert_eq!(Some(t.bytes.len() as u64), w["length"].as_u64(), "{name}");
            assert_eq!(to_hex(t.bytes), w["bytes"], "{name}");
        }
        assert_eq!(encode_tiles(&tiles).unwrap(), bytes, "{name}");
    }
}

#[test]
fn valid_sources() {
    for name in ["empty", "three"] {
        let bytes = read(&format!("sources/valid/{name}.bin"));
        let want = read_json(&format!("sources/valid/{name}.json"));
        let ids = decode_source_ids(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let list: Vec<u64> = want["ids"].as_array().unwrap().iter().map(int).collect();
        assert_eq!(ids, list, "{name}");
        assert_eq!(encode_source_ids(&ids).unwrap(), bytes, "{name}");
    }
}

/// Checks `DIR/invalid/` against `decode` and returns the codes seen.
fn check_invalid(
    dir: &str,
    decode: &dyn Fn(&[u8]) -> Result<(), ValidationError>,
) -> BTreeSet<u16> {
    let index = read_json(&format!("{dir}/invalid/index.json"));
    let vectors = index["vectors"].as_object().expect("vectors");
    let mut seen = BTreeSet::new();
    for (file, code) in vectors {
        let code = u16::try_from(code.as_u64().expect("code")).expect("u16");
        assert!(file.starts_with(&format!("{code}-")), "{file}: name/code");
        let bytes = read(&format!("{dir}/invalid/{file}"));
        let err = decode(&bytes).expect_err(file);
        assert_eq!(err.code, code, "{dir}/{file}: {err}");
        seen.insert(code);
    }
    let mut on_disk: Vec<String> = std::fs::read_dir(root().join(dir).join("invalid"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    on_disk.sort();
    let listed: Vec<String> = vectors.keys().cloned().collect();
    assert_eq!(on_disk, listed, "{dir}: index and directory agree");
    seen
}

#[test]
fn invalid_vectors_fail_with_their_code() {
    let seen = check_invalid("request", &|b| decode_request(b).map(|_| ()));
    let want: BTreeSet<u16> = [
        codes::REQUEST_TOO_SHORT,
        codes::REQUEST_BAD_MAGIC,
        codes::REQUEST_UNSUPPORTED_VERSION,
        codes::REQUEST_RESERVED_NONZERO,
        codes::REQUEST_LENGTH_MISMATCH,
        codes::REQUEST_KEY_MALFORMED,
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, want, "every request rule has a vector");

    let seen = check_invalid("tiles", &|b| decode_tiles(b).map(|_| ()));
    let want: BTreeSet<u16> = [
        codes::TILES_TOO_SHORT,
        codes::TILES_TRUNCATED,
        codes::TILES_TRAILING_BYTES,
        codes::TILES_NOT_ASCENDING,
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, want, "every tile rule has a vector");

    let seen = check_invalid("sources", &|b| decode_source_ids(b).map(|_| ()));
    let want: BTreeSet<u16> = [
        codes::SOURCES_LENGTH_NOT_MULTIPLE,
        codes::SOURCES_NOT_ASCENDING,
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, want, "every source id rule has a vector");
}
