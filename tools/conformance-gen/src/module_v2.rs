//! Example module conformance vectors for module ABI version 2, written
//! under `module-v2/`, and for the example frame module, written under
//! `module-frame/` (`client-compute.md` sections 4, 4.1, and 10).
//!
//! For each `module-v2/` case `NAME`, run against the version 2 example
//! module (`crates/gx-module-reader`):
//!
//! - `NAME.request.bin`: the request bytes passed to every export.
//! - `NAME.frames.bin`: the frame bytes passed to `gx_inputs` and
//!   `gx_sources`: the record of the example frame module's frame
//!   (`crates/gx-module-frame`), or no records for a frame key.
//! - `NAME.input-list.bin`: the input list `gx_inputs` returns.
//! - `NAME.tiles.bin`: the tile bytes passed to `gx_compile` (no tiles).
//! - `NAME.inputs.bin`: the input bytes passed to `gx_compile`: the frame
//!   record (slot 0), then the sections the version 1 example module
//!   (`crates/gx-module-test`) returns for the listed keys (slot 1).
//! - `NAME.sha256`: the lowercase hex SHA-256 of the section, and a newline.
//!
//! `module-frame/` holds version 1 vectors (`NAME.request.bin`,
//! `NAME.tiles.bin`, `NAME.sha256`, as in `module/`) for the example frame
//! module: its registry, its frame key, and another frame key. Each
//! directory has an `index.json`. The outputs come from the native builds;
//! `scripts/module-determinism.sh` checks the WebAssembly builds return the
//! same bytes natively and under Node.

use std::collections::BTreeMap;

use gx_core::key::ChunkKey;
use gx_core::module_io::{
    decode_input_list, encode_inputs, encode_request, encode_tiles, ModuleRequest, Tile,
};
use gx_core::sha256::{sha256, to_hex};
use gx_module_frame::{fixture_record, FrameModule};
use gx_module_guest::{run_compile, run_compile_v2, run_inputs, Module};
use gx_module_reader::ReaderModule;
use gx_module_test::{TestModule, TileParams};
use serde_json::json;

use crate::{pretty, File};

/// The tile parameters of the version 1 example module's layer: the root
/// extent of the example frame module's frame.
const LAYER_PARAMS: TileParams = TileParams {
    root_extent: gx_module_frame::FIXTURE_ROOT_EXTENT,
    base_density: 2.5,
    temperature: 300.0,
};

/// One case: name and request fields.
struct Case {
    name: &'static str,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: &'static str,
}

const CASES: [Case; 5] = [
    Case {
        name: "01-registry",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "registry",
    },
    Case {
        name: "02-frame-key",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "frame-7",
    },
    Case {
        name: "03-root-cell",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "7-0-0-0-0",
    },
    Case {
        name: "04-depth-one",
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: "7-1-1-0-1",
    },
    Case {
        name: "05-span-origin",
        origin_utc_ms: 1_763_130_000_000,
        span_seconds: 3600,
        span_index: 12_345,
        key: "7-3-5-2-6",
    },
];

/// The section the version 1 example module returns for `req`.
fn layer_section(req: &ModuleRequest) -> Vec<u8> {
    let tile = LAYER_PARAMS.encode();
    let ids = TestModule::sources(req);
    let tiles: Vec<Tile<'_>> = ids.iter().map(|&id| Tile { id, bytes: &tile }).collect();
    let tiles = encode_tiles(&tiles).expect("one tile ascends");
    run_compile::<TestModule>(&encode_request(req), &tiles).expect("layer cases compile")
}

fn v2_files() -> Vec<File> {
    let mut out = Vec::new();
    let mut index = BTreeMap::new();
    let record = gx_core::frame_record::encode(&fixture_record());
    for case in &CASES {
        let req = ModuleRequest {
            origin_utc_ms: case.origin_utc_ms,
            span_seconds: case.span_seconds,
            span_index: case.span_index,
            key: case.key.parse::<ChunkKey>().expect("fixed keys are valid"),
        };
        let frame_key = matches!(req.key, ChunkKey::Frame(_));
        let records: Vec<&[u8]> = if frame_key { vec![] } else { vec![&record] };
        let frames = encode_inputs(&records).expect("one record");
        let request = encode_request(&req);
        let list = run_inputs::<ReaderModule>(&request, &frames).expect("inputs run");
        let entries = decode_input_list(&list).expect("the list decodes");
        let sections: Vec<Vec<u8>> = entries
            .iter()
            .map(|e| layer_section(&ModuleRequest { key: e.key, ..req }))
            .collect();
        let mut all = records.clone();
        all.extend(sections.iter().map(Vec::as_slice));
        let inputs = encode_inputs(&all).expect("inputs encode");
        let tiles = encode_tiles(&[]).expect("no tiles");
        let section =
            run_compile_v2::<ReaderModule>(&request, &tiles, &inputs).expect("cases compile");
        let hash = to_hex(&sha256(&section));
        let keys: Vec<String> = entries.iter().map(|e| e.key.to_string()).collect();
        index.insert(
            case.name,
            json!({ "key": case.key, "inputs": keys, "sha256": hash }),
        );
        let base = format!("module-v2/{}", case.name);
        out.push((format!("{base}.request.bin"), request));
        out.push((format!("{base}.frames.bin"), frames));
        out.push((format!("{base}.input-list.bin"), list));
        out.push((format!("{base}.tiles.bin"), tiles));
        out.push((format!("{base}.inputs.bin"), inputs));
        out.push((format!("{base}.sha256"), format!("{hash}\n").into_bytes()));
    }
    out.push((
        "module-v2/index.json".to_string(),
        pretty(&json!({ "cases": index })),
    ));
    out
}

fn frame_files() -> Vec<File> {
    let mut out = Vec::new();
    let mut index = BTreeMap::new();
    for (name, key) in [
        ("01-registry", "registry"),
        ("02-declared-frame", "frame-7"),
        ("03-other-frame", "frame-8"),
    ] {
        let req = ModuleRequest {
            origin_utc_ms: 0,
            span_seconds: 0,
            span_index: 0,
            key: key.parse::<ChunkKey>().expect("fixed keys are valid"),
        };
        let request = encode_request(&req);
        let tiles = encode_tiles(&[]).expect("no tiles");
        let section = run_compile::<FrameModule>(&request, &tiles).expect("cases compile");
        let hash = to_hex(&sha256(&section));
        index.insert(name, json!({ "key": key, "sha256": hash }));
        out.push((format!("module-frame/{name}.request.bin"), request));
        out.push((format!("module-frame/{name}.tiles.bin"), tiles));
        out.push((
            format!("module-frame/{name}.sha256"),
            format!("{hash}\n").into_bytes(),
        ));
    }
    out.push((
        "module-frame/index.json".to_string(),
        pretty(&json!({ "cases": index })),
    ));
    out
}

/// The `module-v2/` and `module-frame/` vector files.
pub(crate) fn files() -> Vec<File> {
    let mut out = v2_files();
    out.extend(frame_files());
    out
}
