//! Example module conformance vectors, written under `module/`
//! (`client-compute.md` section 4, `space-model.md` section 4 item 6).
//!
//! For each case `NAME`:
//!
//! - `NAME.request.bin`: the request bytes passed to `gx_sources` and
//!   `gx_compile`.
//! - `NAME.tiles.bin`: the tile bytes passed to `gx_compile`, holding exactly
//!   the tiles `gx_sources` asks for.
//! - `NAME.sha256`: the lowercase hex SHA-256 of the section the example
//!   module (`crates/gx-module-test`) returns, and a newline.
//!
//! `index.json` lists every case with its chunk key, source ids (decimal
//! strings), and expected hash. The outputs come from the module's native
//! build; `scripts/module-determinism.sh` checks that its WebAssembly build
//! returns the same bytes natively and under Node.

use std::collections::BTreeMap;

use gx_core::key::ChunkKey;
use gx_core::module_io::{encode_request, encode_tiles, ModuleRequest, Tile};
use gx_core::sha256::{sha256, to_hex};
use gx_module_guest::{run_compile, Module};
use gx_module_test::{TestModule, TileParams};
use serde_json::{json, Value};

use crate::{pretty, File};

/// One case: name, request fields, and the parameters in its tile.
struct Case {
    name: &'static str,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: &'static str,
    params: TileParams,
}

const fn params(root_extent: f64, base_density: f64, temperature: f64) -> TileParams {
    TileParams {
        root_extent,
        base_density,
        temperature,
    }
}

const CASES: [Case; 10] = [
    Case {
        name: "01-registry",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "registry",
        params: params(1.0, 1.0, 0.0),
    },
    Case {
        name: "02-root-cell",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "0-0-0-0-0",
        params: params(1.0e6, 2.5, 300.0),
    },
    Case {
        name: "03-depth-one",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "1-1-1-0-1",
        params: params(4096.0, 1000.0, 273.15),
    },
    Case {
        name: "04-depth-two",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "7-2-3-1-2",
        params: params(8.0e7, 4.6e3, 1500.0),
    },
    Case {
        name: "05-span",
        origin_utc_ms: 0,
        span_seconds: 3600,
        span_index: 12_345,
        key: "42-5-17-3-30",
        params: params(1.7e9, 0.1, 6300.0),
    },
    Case {
        name: "06-origin",
        origin_utc_ms: 1_763_130_000_000,
        span_seconds: 72_000,
        span_index: 3,
        key: "3-10-1023-0-512",
        params: params(1.1e7, 2.7e3, 250.0),
    },
    Case {
        name: "07-deepest-largest-frame",
        origin_utc_ms: 1_763_130_000_000,
        span_seconds: 60,
        span_index: 29_385_500,
        key: "18446744073709551615-31-2147483647-0-1073741824",
        params: params(1.0e13, 1.0e-20, 3.0),
    },
    Case {
        name: "08-cold",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "1000-12-4095-4095-4095",
        params: params(8.3e6, 1.2, 0.0),
    },
    Case {
        name: "09-small-cube",
        origin_utc_ms: -1,
        span_seconds: 1,
        span_index: 1,
        key: "5-3-7-7-7",
        params: params(1.0, 7.8e3, 1800.0),
    },
    Case {
        name: "10-dense-hot",
        origin_utc_ms: 946_728_000_000,
        span_seconds: 0,
        span_index: 0,
        key: "9-20-524288-1-1048575",
        params: params(7.0e8, 1.5e5, 1.5e7),
    },
];

/// The module vector files.
pub(crate) fn files() -> Vec<File> {
    let mut out = Vec::new();
    let mut index = BTreeMap::new();
    for case in &CASES {
        let req = ModuleRequest {
            origin_utc_ms: case.origin_utc_ms,
            span_seconds: case.span_seconds,
            span_index: case.span_index,
            key: case.key.parse::<ChunkKey>().expect("fixed keys are valid"),
        };
        let tile = case.params.encode();
        let ids = TestModule::sources(&req);
        let tiles: Vec<Tile<'_>> = ids.iter().map(|&id| Tile { id, bytes: &tile }).collect();
        let request = encode_request(&req);
        let tiles = encode_tiles(&tiles).expect("one tile ascends");
        let section = run_compile::<TestModule>(&request, &tiles).expect("cases compile");
        let hash = to_hex(&sha256(&section));
        let ids: Vec<Value> = ids.iter().map(|id| Value::from(id.to_string())).collect();
        index.insert(
            case.name,
            json!({ "key": case.key, "sources": ids, "sha256": hash }),
        );
        out.push((format!("module/{}.request.bin", case.name), request));
        out.push((format!("module/{}.tiles.bin", case.name), tiles));
        out.push((
            format!("module/{}.sha256", case.name),
            format!("{hash}\n").into_bytes(),
        ));
    }
    out.push((
        "module/index.json".to_string(),
        pretty(&json!({ "cases": index })),
    ));
    out
}
