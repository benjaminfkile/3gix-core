//! Module request, tile, and source id list conformance vectors, written
//! under `module-io/` (`client-compute.md` section 4).
//!
//! - `request/valid/NAME.bin` is a valid request and `NAME.json` its fields
//!   (integers as decimal strings).
//! - `tiles/valid/NAME.bin` is valid tile bytes and `NAME.json` lists each
//!   tile's id (decimal string), length, and bytes as hex.
//! - `sources/valid/NAME.bin` is a valid source id list and `NAME.json` its
//!   ids as decimal strings.
//! - `request/invalid/`, `tiles/invalid/`, and `sources/invalid/` hold one or
//!   more files per rule, named `CODE-RULE.bin`, each with an `index.json`
//!   mapping file name to code.

use std::collections::BTreeMap;

use gx_core::error::codes;
use gx_core::key::ChunkKey;
use gx_core::module_io::{
    encode_request, encode_source_ids, encode_tiles, ModuleRequest, Tile, REQUEST_HEADER_LEN,
};
use gx_core::sha256::to_hex;
use serde_json::{json, Value};

use crate::{pretty, File};

fn request(origin_utc_ms: i64, span_seconds: u64, span_index: u64, key: &str) -> ModuleRequest {
    ModuleRequest {
        origin_utc_ms,
        span_seconds,
        span_index,
        key: key.parse::<ChunkKey>().expect("fixed keys are valid"),
    }
}

fn request_json(r: &ModuleRequest) -> Value {
    json!({
        "origin_utc_ms": r.origin_utc_ms.to_string(),
        "span_seconds": r.span_seconds.to_string(),
        "span_index": r.span_index.to_string(),
        "key": r.key.to_string(),
    })
}

/// Writes `vectors` as `DIR/invalid/CODE-NAME.bin` plus an index.
fn invalid_files(dir: &str, vectors: Vec<(&str, u16, Vec<u8>)>) -> Vec<File> {
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("module-io/{dir}/invalid/{file}"), bytes));
    }
    out.push((
        format!("module-io/{dir}/invalid/index.json"),
        pretty(&json!({ "vectors": index })),
    ));
    out
}

/// One invalid vector: a name, its code, and the edit that breaks the base.
type Edit = (&'static str, u16, Box<dyn Fn(&mut Vec<u8>)>);

/// Applies each edit to a copy of `base`.
fn edits(base: &[u8], list: Vec<Edit>) -> Vec<(&'static str, u16, Vec<u8>)> {
    list.into_iter()
        .map(|(name, code, f)| {
            let mut b = base.to_vec();
            f(&mut b);
            (name, code, b)
        })
        .collect()
}

fn requests() -> Vec<File> {
    let valid = [
        (
            "cell",
            request(1_763_130_000_000, 3600, 2, "23-12-1024-2048-3071"),
        ),
        ("registry", request(1_763_130_000_000, 0, 0, "registry")),
        ("static-cell", request(0, 0, 0, "30-1-1-0-1")),
    ];
    let mut out = Vec::new();
    for (name, r) in &valid {
        out.push((
            format!("module-io/request/valid/{name}.bin"),
            encode_request(r),
        ));
        out.push((
            format!("module-io/request/valid/{name}.json"),
            pretty(&request_json(r)),
        ));
    }
    let base = encode_request(&valid[0].1);
    let key_at = REQUEST_HEADER_LEN;
    let vectors = edits(
        &base,
        vec![
            (
                "too-short",
                codes::REQUEST_TOO_SHORT,
                Box::new(|b| b.truncate(REQUEST_HEADER_LEN - 1)),
            ),
            (
                "bad-magic",
                codes::REQUEST_BAD_MAGIC,
                Box::new(|b| b[3] = b'M'),
            ),
            (
                "unsupported-version",
                codes::REQUEST_UNSUPPORTED_VERSION,
                Box::new(|b| b[4] = 2),
            ),
            (
                "reserved-nonzero",
                codes::REQUEST_RESERVED_NONZERO,
                Box::new(|b| b[7] = 1),
            ),
            (
                "key-length-high",
                codes::REQUEST_LENGTH_MISMATCH,
                Box::new(|b| b[32] += 1),
            ),
            (
                "trailing-byte",
                codes::REQUEST_LENGTH_MISMATCH,
                Box::new(|b| b.push(b'0')),
            ),
            (
                "key-not-utf8",
                codes::REQUEST_KEY_MALFORMED,
                Box::new(move |b| b[key_at] = 0xff),
            ),
            (
                "key-coordinate-out-of-range",
                codes::REQUEST_KEY_MALFORMED,
                // 23-12-4096-2048-3071: x is not below 2^12.
                Box::new(move |b| b[key_at + 6..key_at + 10].copy_from_slice(b"4096")),
            ),
            (
                "key-empty",
                codes::REQUEST_KEY_MALFORMED,
                Box::new(|b| {
                    b.truncate(REQUEST_HEADER_LEN);
                    b[32] = 0;
                }),
            ),
        ],
    );
    out.extend(invalid_files("request", vectors));
    out
}

fn tiles() -> Vec<File> {
    let a: Vec<u8> = (0..=255u8).collect();
    let sets: [(&str, Vec<Tile<'_>>); 2] = [
        ("empty", vec![]),
        (
            "three",
            vec![
                Tile { id: 1, bytes: &a },
                Tile {
                    id: 0x0001_0000_0000_0000,
                    bytes: b"",
                },
                Tile {
                    id: u64::MAX,
                    bytes: b"tile bytes are opaque",
                },
            ],
        ),
    ];
    let mut out = Vec::new();
    let mut base = Vec::new();
    for (name, tiles) in &sets {
        let b = encode_tiles(tiles).expect("fixed tiles are valid");
        let list: Vec<Value> = tiles
            .iter()
            .map(|t| {
                json!({
                    "id": t.id.to_string(),
                    "length": t.bytes.len(),
                    "bytes": to_hex(t.bytes),
                })
            })
            .collect();
        out.push((
            format!("module-io/tiles/valid/{name}.json"),
            pretty(&json!({ "count": tiles.len(), "tiles": list })),
        ));
        base = b.clone();
        out.push((format!("module-io/tiles/valid/{name}.bin"), b));
    }
    // `base` is now the three-tile set: tile 0 at offset 4 with 256 bytes,
    // tile 1 at offset 272 with none, tile 2 at offset 284.
    let vectors = edits(
        &base,
        vec![
            (
                "too-short",
                codes::TILES_TOO_SHORT,
                Box::new(|b| b.truncate(3)),
            ),
            ("count-high", codes::TILES_TRUNCATED, Box::new(|b| b[0] = 4)),
            (
                "bytes-cut",
                codes::TILES_TRUNCATED,
                Box::new(|b| {
                    b.pop();
                }),
            ),
            (
                "header-cut",
                codes::TILES_TRUNCATED,
                Box::new(|b| b.truncate(272 + 6)),
            ),
            (
                "count-low",
                codes::TILES_TRAILING_BYTES,
                Box::new(|b| b[0] = 2),
            ),
            (
                "trailing-byte",
                codes::TILES_TRAILING_BYTES,
                Box::new(|b| b.push(0)),
            ),
            (
                "ids-repeat",
                codes::TILES_NOT_ASCENDING,
                Box::new(|b| b[272..280].copy_from_slice(&1u64.to_le_bytes())),
            ),
            (
                "ids-descend",
                codes::TILES_NOT_ASCENDING,
                Box::new(|b| b[272..280].copy_from_slice(&0u64.to_le_bytes())),
            ),
        ],
    );
    out.extend(invalid_files("tiles", vectors));
    out
}

fn sources() -> Vec<File> {
    let sets: [(&str, &[u64]); 2] = [
        ("empty", &[]),
        ("three", &[1, 0x0001_0000_0000_0000, u64::MAX]),
    ];
    let mut out = Vec::new();
    for (name, ids) in sets {
        let b = encode_source_ids(ids).expect("fixed ids ascend");
        let list: Vec<String> = ids.iter().map(u64::to_string).collect();
        out.push((
            format!("module-io/sources/valid/{name}.json"),
            pretty(&json!({ "ids": list })),
        ));
        out.push((format!("module-io/sources/valid/{name}.bin"), b));
    }
    let base = encode_source_ids(sets[1].1).expect("fixed ids ascend");
    let vectors = edits(
        &base,
        vec![
            (
                "length-not-multiple",
                codes::SOURCES_LENGTH_NOT_MULTIPLE,
                Box::new(|b| {
                    b.pop();
                }),
            ),
            (
                "ids-repeat",
                codes::SOURCES_NOT_ASCENDING,
                Box::new(|b| b[8..16].copy_from_slice(&1u64.to_le_bytes())),
            ),
            (
                "ids-descend",
                codes::SOURCES_NOT_ASCENDING,
                Box::new(|b| b[16..24].copy_from_slice(&2u64.to_le_bytes())),
            ),
        ],
    );
    out.extend(invalid_files("sources", vectors));
    out
}

/// Every module request, tile, and source id vector file.
pub fn files() -> Vec<File> {
    let mut out = requests();
    out.extend(tiles());
    out.extend(sources());
    out
}
