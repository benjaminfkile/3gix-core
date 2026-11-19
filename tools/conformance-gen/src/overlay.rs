//! Overlay tile conformance vectors, written under `overlay/`
//! (`matter-format.md` section 12), and the example overlay module's
//! vectors, written under `module-overlay/` (`client-compute.md` section 4).
//!
//! - `valid/NAME.bin` is a valid overlay tile, compressed, uncompressed, or
//!   empty. `valid/index.json` maps each file to the key it validates under.
//! - `invalid/CODE-RULE.bin` breaks exactly one rule of section 12.5
//!   (codes 1051 to 1057) under an overlay layer (`gx_validate_layer` with
//!   `overlay` 1; under `gx_validate`, bytes without the `3GSO` magic are
//!   checked as a surface tile). `invalid/index.json` gives the key every
//!   file is validated under and maps each file name to its code.
//! - `layer/NAME.bin` with `layer/index.json` are layer-aware cases
//!   (`gx_validate_layer`, `client-compute.md` section 2.2): each names its
//!   key, whether the layer is an overlay layer, and the expected code (0 for
//!   accepted, 1058 and 1059 for refused).
//! - `module-overlay/` holds version 1 module vectors (`NAME.request.bin`,
//!   `NAME.tiles.bin`, `NAME.sha256`, and `index.json`) for
//!   `crates/gx-module-overlay`.

use std::collections::BTreeMap;

use gx_core::error::codes;
use gx_core::frame_record::{self, FrameRecord};
use gx_core::key::{CellKey, ChunkKey, SurfaceKey};
use gx_core::matter::{self, Compression, Section};
use gx_core::module_io::{encode_request, encode_tiles, ModuleRequest};
use gx_core::overlay::{self, OverlayTile, PARENT_COUNT, VERTEX_COUNT};
use gx_core::registry::{self, Registry};
use gx_core::sha256::{sha256, to_hex};
use gx_core::surface;
use gx_core::units::Meters;
use gx_module_guest::run_compile;
use gx_module_overlay::OverlayModule;
use serde_json::json;

use crate::{pretty, File};

/// The key every invalid vector is validated under.
const KEY: &str = "s-7-2-3-5-6";

fn key() -> SurfaceKey {
    SurfaceKey::new(7, 2, 3, 5, 6).expect("fixed key")
}

fn tile() -> OverlayTile {
    OverlayTile {
        key: key(),
        coverage: (0..VERTEX_COUNT).map(|i| (i * 7 % 256) as u8).collect(),
        albedo: (0..VERTEX_COUNT)
            .map(|i| [(i % 251) as u8, (i % 241) as u8, (i % 239) as u8])
            .collect(),
        roughness: (0..VERTEX_COUNT).map(|i| (i % 200) as u8).collect(),
        parent_coverage: (0..PARENT_COUNT).map(|i| (i * 3 % 256) as u8).collect(),
        parent_albedo: vec![[30, 60, 90]; PARENT_COUNT],
        parent_roughness: vec![128; PARENT_COUNT],
    }
}

fn valid() -> Vec<File> {
    let level0 = SurfaceKey::new(7, 0, 0, 0, 0).expect("fixed key");
    let files: [(&str, SurfaceKey, Vec<u8>); 4] = [
        ("raw", key(), overlay::encode(&tile(), Compression::None)),
        ("zstd", key(), overlay::encode(&tile(), Compression::Zstd)),
        ("empty", key(), overlay::encode_empty(&key())),
        (
            "module-level0",
            level0,
            overlay::encode(&gx_module_overlay::overlay_tile(&level0), Compression::Zstd),
        ),
    ];
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, k, bytes) in files {
        let file = format!("{name}.bin");
        index.insert(
            file.clone(),
            json!({ "key": ChunkKey::Surface(k).to_string() }),
        );
        out.push((format!("overlay/valid/{file}"), bytes));
    }
    out.push((
        "overlay/valid/index.json".into(),
        pretty(&json!({ "tiles": index })),
    ));
    out
}

fn invalid() -> Vec<File> {
    let raw = overlay::encode(&tile(), Compression::None);
    let zstd = overlay::encode(&tile(), Compression::Zstd);
    let empty = overlay::encode_empty(&key());
    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, from: &[u8], f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = from.to_vec();
        f(&mut b);
        vectors.push((name, code, b));
    };
    add("too-short", codes::OVERLAY_TOO_SHORT, &empty, &|b| {
        b.truncate(31)
    });
    add("bad-magic", codes::OVERLAY_BAD_MAGIC, &raw, &|b| {
        b[3] = b'X'
    });
    add("version-2", codes::OVERLAY_BAD_VERSION, &raw, &|b| b[4] = 2);
    add("flag-bit", codes::OVERLAY_RESERVED_NONZERO, &raw, &|b| {
        b[6] |= 4
    });
    add("reserved", codes::OVERLAY_RESERVED_NONZERO, &raw, &|b| {
        b[19] = 1
    });
    add("frame-id", codes::OVERLAY_KEY_MISMATCH, &raw, &|b| b[8] = 8);
    add("level", codes::OVERLAY_KEY_MISMATCH, &empty, &|b| b[17] = 4);
    add(
        "empty-trailing",
        codes::OVERLAY_EMPTY_INVALID,
        &empty,
        &|b| b.push(0),
    );
    add("empty-zstd", codes::OVERLAY_EMPTY_INVALID, &empty, &|b| {
        b[6] |= 2
    });
    add("block-len", codes::OVERLAY_BLOCK_LENGTH, &raw, &|b| {
        b[28] ^= 1
    });
    add("block-short", codes::OVERLAY_BLOCK_LENGTH, &raw, &|b| {
        b.pop();
    });
    add("zstd-truncated", codes::OVERLAY_BLOCK_LENGTH, &zstd, &|b| {
        b.truncate(b.len() - 4)
    });

    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("overlay/invalid/{file}"), bytes));
    }
    out.push((
        "overlay/invalid/index.json".into(),
        pretty(&json!({ "key": KEY, "vectors": index })),
    ));
    out
}

fn layer() -> Vec<File> {
    let cell = CellKey::new(7, 1, 0, 1, 0).expect("fixed key");
    let g = cell.geometry(Meters::new(2.0e7));
    let empty_cell = matter::encode(
        &Section::empty(cell, g.origin, g.edge).expect("empty section"),
        Compression::None,
    );
    let one_frame = Registry::new(
        gx_core::units::Seconds::new(0.0),
        vec![gx_module_frame::fixture_frame()],
    )
    .expect("valid registry");
    let cases: [(&str, String, Vec<u8>, bool, u16); 12] = [
        (
            "overlay-tile",
            KEY.into(),
            overlay::encode(&tile(), Compression::Zstd),
            true,
            0,
        ),
        (
            "overlay-empty",
            KEY.into(),
            overlay::encode_empty(&key()),
            true,
            0,
        ),
        ("empty-cell", cell.to_string(), empty_cell.clone(), true, 0),
        (
            "empty-registry",
            "registry".into(),
            registry::encode(&Registry::empty()),
            true,
            0,
        ),
        (
            "empty-frame-record",
            "frame-7".into(),
            frame_record::encode(&FrameRecord::empty(7)),
            true,
            0,
        ),
        (
            "surface-tile",
            KEY.into(),
            surface::encode_empty(&key()),
            false,
            0,
        ),
        (
            "surface-under-overlay",
            KEY.into(),
            surface::encode_empty(&key()),
            true,
            codes::OVERLAY_LAYER_REFUSED,
        ),
        (
            "registry-under-overlay",
            "registry".into(),
            registry::encode(&one_frame),
            true,
            codes::OVERLAY_LAYER_REFUSED,
        ),
        (
            "frame-record-under-overlay",
            "frame-7".into(),
            frame_record::encode(&gx_module_frame::fixture_record()),
            true,
            codes::OVERLAY_LAYER_REFUSED,
        ),
        (
            "overlay-under-surface",
            KEY.into(),
            overlay::encode(&tile(), Compression::Zstd),
            false,
            codes::OVERLAY_NOT_OVERLAY_LAYER,
        ),
        (
            "overlay-empty-under-surface",
            KEY.into(),
            overlay::encode_empty(&key()),
            false,
            codes::OVERLAY_NOT_OVERLAY_LAYER,
        ),
        ("cell-under-surface", cell.to_string(), empty_cell, false, 0),
    ];
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, key, bytes, overlay, code) in cases {
        let file = format!("{name}.bin");
        assert_eq!(
            gx_core::validate_for_layer(&key, &bytes, overlay).map_err(|e| e.code),
            if code == 0 { Ok(()) } else { Err(code) },
            "{name}"
        );
        index.insert(
            file.clone(),
            json!({ "key": key, "overlay": overlay, "code": code }),
        );
        out.push((format!("overlay/layer/{file}"), bytes));
    }
    out.push((
        "overlay/layer/index.json".into(),
        pretty(&json!({ "cases": index })),
    ));
    out
}

fn module_files() -> Vec<File> {
    let mut out = Vec::new();
    let mut index = BTreeMap::new();
    for (name, key) in [
        ("01-registry", "registry"),
        ("02-frame", "frame-7"),
        ("03-cell", "7-1-0-1-0"),
        ("04-equator", "s-7-0-0-0-0"),
        ("05-band-edge", "s-7-0-2-1-2"),
        ("06-pole", "s-7-4-1-0-1"),
        ("07-other-frame", "s-8-0-0-0-0"),
    ] {
        let req = ModuleRequest {
            origin_utc_ms: 0,
            span_seconds: 0,
            span_index: 0,
            key: key.parse::<ChunkKey>().expect("fixed keys are valid"),
        };
        let request = encode_request(&req);
        let tiles = encode_tiles(&[]).expect("no tiles");
        let section = run_compile::<OverlayModule>(&request, &tiles).expect("cases compile");
        assert_eq!(
            gx_core::validate_for_layer(key, &section, true),
            Ok(()),
            "{key}"
        );
        let hash = to_hex(&sha256(&section));
        index.insert(name, json!({ "key": key, "sha256": hash }));
        out.push((format!("module-overlay/{name}.request.bin"), request));
        out.push((format!("module-overlay/{name}.tiles.bin"), tiles));
        out.push((
            format!("module-overlay/{name}.sha256"),
            format!("{hash}\n").into_bytes(),
        ));
    }
    out.push((
        "module-overlay/index.json".to_string(),
        pretty(&json!({ "cases": index })),
    ));
    out
}

/// Every overlay and overlay module vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out.extend(layer());
    out.extend(module_files());
    out
}
