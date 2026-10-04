//! Checks the matter section conformance vectors under `conformance/matter/`
//! (`matter-format.md` sections 3, 4, and 8).
//!
//! Every `valid/NAME.bin` must decode under the key in `NAME.json` and match
//! every recorded value bit for bit. Every file in `invalid/index.json` must
//! fail with exactly its listed code, and every code a matter section can
//! trigger must have at least one file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::error::codes;
use gx_core::key::{CellKey, ChunkKey};
use gx_core::matter::{self, State};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/matter")
}

fn parse_cell(s: &str) -> CellKey {
    match s.parse::<ChunkKey>() {
        Ok(ChunkKey::Cell(c)) => c,
        other => panic!("{s}: not a cell key: {other:?}"),
    }
}

/// Reads the `bits` of a recorded number and checks `decimal` agrees.
fn number(v: &Value) -> f64 {
    let bits = v["bits"].as_str().expect("bits is a string");
    let bits = u64::from_str_radix(bits.trim_start_matches("0x"), 16).expect("hex bits");
    let x = f64::from_bits(bits);
    let decimal: f64 = v["decimal"]
        .as_str()
        .expect("decimal is a string")
        .parse()
        .expect("decimal parses");
    assert_eq!(decimal.to_bits(), bits, "decimal and bits disagree in {v}");
    x
}

fn assert_bits(name: &str, field: &str, want: &Value, got: f64) {
    assert_eq!(
        number(want).to_bits(),
        got.to_bits(),
        "{name}: {field} is {got:e}, expected {}",
        want["decimal"]
    );
}

fn valid_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root().join("valid"))
        .expect("valid directory")
        .map(|e| e.expect("entry").file_name().into_string().expect("UTF-8"))
        .filter_map(|n| n.strip_suffix(".bin").map(str::to_string))
        .collect();
    names.sort();
    names
}

#[test]
fn valid_vectors_decode_and_match() {
    let names = valid_names();
    assert!(names.len() >= 6, "expected at least six valid vectors");
    let mut resolutions = BTreeSet::new();
    let mut every_state = false;
    let mut hot_plasma = false;
    let mut empty = false;
    for name in &names {
        let bytes = std::fs::read(root().join(format!("valid/{name}.bin"))).expect("bin");
        let text =
            std::fs::read_to_string(root().join(format!("valid/{name}.json"))).expect("json");
        let want: Value = serde_json::from_str(&text).expect("JSON");
        let key = parse_cell(want["key"].as_str().expect("key"));
        let s = matter::decode(&key, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(s.key(), key);
        let o = s.origin();
        for (i, got) in [o.x, o.y, o.z].into_iter().enumerate() {
            assert_bits(name, "origin", &want["origin"][i], got);
        }
        assert_bits(name, "edge", &want["edge"], s.edge().value());
        assert_eq!(
            u64::from(s.resolution()),
            want["resolution"].as_u64().unwrap()
        );
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        assert_eq!(u64::from(flags), want["flags"].as_u64().unwrap(), "{name}");
        assert_bits(name, "mass", &want["mass"], s.mass().value());

        let mut count = 0u64;
        let mut sums = [0.0f64; 7];
        let mut states = [0u64; 5];
        let mut hot = false;
        if let Some(samples) = s.samples() {
            for x in samples.iter() {
                count += 1;
                sums[0] += x.density.value();
                sums[1] += x.temperature.value();
                for b in 0..3 {
                    sums[2 + b] += x.albedo[b].value();
                }
                sums[5] += x.roughness.value();
                sums[6] += x.attenuation.value();
                states[usize::from(x.state.as_u8())] += 1;
                hot |= x.state == State::Plasma
                    && x.temperature.value() > 5000.0
                    && x.attenuation.value() > 0.0;
            }
        }
        assert_eq!(count, want["sample_count"].as_u64().unwrap(), "{name}");
        let w = &want["sums"];
        assert_bits(name, "density sum", &w["density"], sums[0]);
        assert_bits(name, "temperature sum", &w["temperature"], sums[1]);
        for b in 0..3 {
            assert_bits(name, "albedo sum", &w["albedo"][b], sums[2 + b]);
        }
        assert_bits(name, "roughness sum", &w["roughness"], sums[5]);
        assert_bits(name, "attenuation sum", &w["attenuation"], sums[6]);
        for st in State::ALL {
            assert_eq!(
                states[usize::from(st.as_u8())],
                want["state_counts"][st.name()].as_u64().unwrap(),
                "{name}: count of {}",
                st.name()
            );
        }

        resolutions.insert(s.resolution());
        every_state |= states.iter().all(|&c| c > 0);
        hot_plasma |= hot;
        empty |= s.is_empty();
    }
    for r in [1, 2, 16, 64] {
        assert!(
            resolutions.contains(&r),
            "no valid vector at resolution {r}"
        );
    }
    assert!(every_state, "no valid vector with every state");
    assert!(hot_plasma, "no hot plasma vector with attenuation");
    assert!(empty, "no empty vector");
}

#[test]
fn zstd_twin_matches_raw() {
    let key = parse_cell("4-5-17-3-30");
    let raw = std::fs::read(root().join("valid/res16-raw.bin")).unwrap();
    let packed = std::fs::read(root().join("valid/res16-zstd.bin")).unwrap();
    assert_ne!(raw, packed);
    let a = matter::decode(&key, &raw).unwrap();
    let b = matter::decode(&key, &packed).unwrap();
    assert_eq!(a, b);
    // Encoding the decoded section reproduces both files exactly.
    assert_eq!(matter::encode(&a, matter::Compression::None), raw);
    assert_eq!(matter::encode(&a, matter::Compression::Zstd), packed);
}

#[test]
fn valid_vectors_reencode_exactly() {
    for name in valid_names() {
        let bytes = std::fs::read(root().join(format!("valid/{name}.bin"))).unwrap();
        let text = std::fs::read_to_string(root().join(format!("valid/{name}.json"))).unwrap();
        let want: Value = serde_json::from_str(&text).unwrap();
        let key = parse_cell(want["key"].as_str().unwrap());
        let s = matter::decode(&key, &bytes).unwrap();
        let compression = if u16::from_le_bytes([bytes[6], bytes[7]]) & matter::FLAG_ZSTD != 0 {
            matter::Compression::Zstd
        } else {
            matter::Compression::None
        };
        assert_eq!(matter::encode(&s, compression), bytes, "{name}");
    }
}

#[test]
fn invalid_vectors_fail_with_their_code() {
    let text = std::fs::read_to_string(root().join("invalid/index.json")).expect("index");
    let index: Value = serde_json::from_str(&text).expect("JSON");
    let key = parse_cell(index["key"].as_str().expect("key"));
    let vectors = index["vectors"].as_object().expect("vectors object");

    let mut on_disk: Vec<String> = std::fs::read_dir(root().join("invalid"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    on_disk.sort();
    let listed: Vec<String> = vectors.keys().cloned().collect();
    assert_eq!(on_disk, listed, "index.json and the .bin files differ");

    let mut seen = BTreeSet::new();
    for (file, code) in vectors {
        let code = u16::try_from(code.as_u64().expect("code")).unwrap();
        assert!(
            file.starts_with(&format!("{code}-")),
            "{file} is not named by its code {code}"
        );
        let bytes = std::fs::read(root().join("invalid").join(file)).unwrap();
        let err = matter::validate(&key, &bytes).expect_err(file);
        assert_eq!(err.code, code, "{file}: {err}");
        assert!(!err.reason.is_empty());
        seen.insert(code);
    }

    // Every code in 100 to 599 that bytes can trigger has a vector. Codes 200
    // and 417 are reachable only through the Rust API.
    let triggerable = [
        codes::HEADER_TOO_SHORT,
        codes::BAD_MAGIC,
        codes::UNSUPPORTED_VERSION,
        codes::UNKNOWN_FLAGS,
        codes::RESERVED_NONZERO,
        codes::FRAME_ID_MISMATCH,
        codes::DEPTH_MISMATCH,
        codes::CELL_X_MISMATCH,
        codes::CELL_Y_MISMATCH,
        codes::CELL_Z_MISMATCH,
        codes::EDGE_NOT_FINITE,
        codes::EDGE_NOT_POSITIVE,
        codes::ORIGIN_NOT_FINITE,
        codes::EMPTY_RESOLUTION_NONZERO,
        codes::EMPTY_BLOCK_LEN_NONZERO,
        codes::EMPTY_ZSTD_SET,
        codes::EMPTY_TRAILING_BYTES,
        codes::RESOLUTION_OUT_OF_RANGE,
        codes::BLOCK_LEN_MISMATCH,
        codes::RAW_LENGTH_MISMATCH,
        codes::ZSTD_FRAME_INVALID,
        codes::ZSTD_TRAILING_BYTES,
        codes::ZSTD_LENGTH_MISMATCH,
        codes::DENSITY_NOT_FINITE,
        codes::DENSITY_NEGATIVE,
        codes::STATE_OUT_OF_RANGE,
        codes::TEMPERATURE_NOT_FINITE,
        codes::TEMPERATURE_NEGATIVE,
        codes::ALBEDO_NOT_FINITE,
        codes::ALBEDO_OUT_OF_RANGE,
        codes::ROUGHNESS_NOT_FINITE,
        codes::ROUGHNESS_OUT_OF_RANGE,
        codes::ATTENUATION_NOT_FINITE,
        codes::ATTENUATION_NEGATIVE,
        codes::VACUUM_STATE_NOT_VACUUM,
        codes::MATTER_STATE_VACUUM,
        codes::VACUUM_TEMPERATURE_NONZERO,
        codes::VACUUM_ALBEDO_NONZERO,
        codes::VACUUM_ROUGHNESS_NONZERO,
        codes::VACUUM_ATTENUATION_NONZERO,
    ];
    let want: BTreeSet<u16> = triggerable.into_iter().collect();
    assert_eq!(
        seen, want,
        "invalid vectors do not cover exactly the byte rules"
    );
}
