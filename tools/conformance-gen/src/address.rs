//! Section address and span index conformance vectors, written under
//! `address/` (`client-compute.md` sections 2.4 and 3).
//!
//! - `cases.json` lists address cases. Each gives the inputs exactly as
//!   `section_address` takes them (module and manifest hashes as lowercase
//!   hex, the manifest `null` for none; the origin, span seconds, and span
//!   index as decimal strings so no reader loses precision; the chunk key),
//!   whether the case stands for a layer with `usesOrigin` on (when off the
//!   origin is 0), the preimage as hex, and the expected address as hex.
//! - `spans.json` lists span index cases: a time, an origin, and a span
//!   length, all decimal strings, with the expected span index or code.
//!
//! Module and manifest hashes are SHA-256 digests of fixed short strings.

use gx_core::address::{address_preimage, section_address, span_index, to_hex};
use gx_core::error::codes;
use gx_core::key::ChunkKey;
use gx_core::sha256::{self, sha256};
use serde_json::{json, Value};

use crate::{pretty, File};

/// An origin in UTC milliseconds used by the cases with `usesOrigin` on.
const ORIGIN: i64 = 1_763_130_000_000;

struct Case {
    name: &'static str,
    module: [u8; 32],
    manifest: Option<[u8; 32]>,
    uses_origin: bool,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: &'static str,
}

fn cases() -> Vec<Case> {
    let module_a = sha256(b"module a");
    let module_b = sha256(b"module b");
    let manifest_1 = sha256(b"source manifest 1");
    let manifest_2 = sha256(b"source manifest 2");
    let case = |name, module, manifest, origin: Option<i64>, span_seconds, span_index, key| Case {
        name,
        module,
        manifest,
        uses_origin: origin.is_some(),
        origin_utc_ms: origin.unwrap_or(0),
        span_seconds,
        span_index,
        key,
    };
    vec![
        case("registry-static", module_a, None, None, 0, 0, "registry"),
        case(
            "registry-uses-origin",
            module_a,
            None,
            Some(ORIGIN),
            0,
            0,
            "registry",
        ),
        case(
            "registry-with-manifest",
            module_b,
            Some(manifest_1),
            None,
            0,
            0,
            "registry",
        ),
        case("depth-0", module_a, None, None, 0, 0, "1-0-0-0-0"),
        case(
            "depth-1-with-manifest",
            module_a,
            Some(manifest_1),
            None,
            0,
            0,
            "30-1-1-0-1",
        ),
        case(
            "depth-12-with-manifest",
            module_b,
            Some(manifest_1),
            None,
            0,
            0,
            "23-12-1024-2048-3071",
        ),
        case(
            "depth-12-with-manifest-uses-origin",
            module_b,
            Some(manifest_1),
            Some(ORIGIN),
            0,
            0,
            "23-12-1024-2048-3071",
        ),
        case(
            "depth-31-largest-key",
            module_b,
            Some(manifest_2),
            None,
            0,
            0,
            "18446744073709551615-31-2147483647-0-1073741824",
        ),
        case(
            "depth-20-span-0",
            module_a,
            None,
            Some(ORIGIN),
            3600,
            0,
            "30-20-524288-1-1048575",
        ),
        case(
            "depth-20-span-1",
            module_a,
            None,
            Some(ORIGIN),
            3600,
            1,
            "30-20-524288-1-1048575",
        ),
        case(
            "depth-8-long-span-uses-origin",
            module_b,
            Some(manifest_2),
            Some(ORIGIN),
            72_000,
            12_345,
            "31-8-255-0-128",
        ),
        case(
            "depth-8-long-span-static",
            module_b,
            Some(manifest_2),
            None,
            72_000,
            12_345,
            "31-8-255-0-128",
        ),
        case(
            "negative-origin",
            module_a,
            None,
            Some(-72_000_000),
            0,
            0,
            "1-3-7-0-5",
        ),
        case(
            "largest-span-index",
            module_a,
            Some(manifest_1),
            None,
            1,
            u64::MAX,
            "2-2-3-3-3",
        ),
    ]
}

fn spans() -> Value {
    let rows: [(i64, i64, u64); 10] = [
        (ORIGIN, ORIGIN, 0),
        (ORIGIN + 999_999_999, ORIGIN, 0),
        (ORIGIN, ORIGIN, 60),
        (ORIGIN + 59_999, ORIGIN, 60),
        (ORIGIN + 60_000, ORIGIN, 60),
        (ORIGIN + 3_600_000 * 25 + 1, ORIGIN, 3600),
        (ORIGIN - 1, ORIGIN, 60),
        (ORIGIN - 1, ORIGIN, 0),
        (i64::MAX, i64::MIN, 1),
        (i64::MAX, i64::MIN, u64::MAX),
    ];
    let cases: Vec<Value> = rows
        .iter()
        .map(|&(t, origin, span)| {
            let mut v = json!({
                "t_utc_ms": t.to_string(),
                "origin_utc_ms": origin.to_string(),
                "span_seconds": span.to_string(),
            });
            match span_index(t, origin, span) {
                Ok(i) => v["span_index"] = json!(i.to_string()),
                Err(e) => {
                    assert_eq!(e.code, codes::SPAN_TIME_BEFORE_ORIGIN);
                    v["code"] = json!(e.code);
                }
            }
            v
        })
        .collect();
    json!({ "cases": cases })
}

/// Every address vector file.
pub fn files() -> Vec<File> {
    let cases: Vec<Value> = cases()
        .iter()
        .map(|c| {
            let key: ChunkKey = c.key.parse().expect("fixed keys are valid");
            let pre = address_preimage(
                &c.module,
                c.manifest.as_ref(),
                c.origin_utc_ms,
                c.span_seconds,
                c.span_index,
                &key,
            );
            let addr = section_address(
                &c.module,
                c.manifest.as_ref(),
                c.origin_utc_ms,
                c.span_seconds,
                c.span_index,
                &key,
            );
            json!({
                "name": c.name,
                "module": to_hex(&c.module),
                "manifest": c.manifest.map(|m| to_hex(&m)),
                "uses_origin": c.uses_origin,
                "origin_utc_ms": c.origin_utc_ms.to_string(),
                "span_seconds": c.span_seconds.to_string(),
                "span_index": c.span_index.to_string(),
                "key": c.key,
                "preimage": sha256::to_hex(&pre),
                "address": to_hex(&addr),
            })
        })
        .collect();
    vec![
        (
            "address/cases.json".into(),
            pretty(&json!({ "cases": cases })),
        ),
        ("address/spans.json".into(), pretty(&spans())),
    ]
}
