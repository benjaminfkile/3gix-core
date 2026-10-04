//! Checks every entry of `conformance/keys.json` against the chunk key parser
//! (`matter-format.md` section 2).
//!
//! Valid entries must parse to the listed fields and print back to the exact
//! input. Invalid entries must be rejected, and when an entry names an
//! `error`, the [`KeyError`] variant must match it.

use gx_core::key::{CellKey, ChunkKey, KeyError, REGISTRY_KEY};
use serde_json::Value;

const KEYS_JSON: &str = include_str!("../../../conformance/keys.json");

/// Returns the variant name of a [`KeyError`], such as `LeadingZero`.
fn variant_name(e: &KeyError) -> String {
    let debug = format!("{e:?}");
    debug
        .split([' ', '{'])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn field(entry: &Value, name: &str) -> u64 {
    entry[name]
        .as_u64()
        .unwrap_or_else(|| panic!("{entry}: missing integer field {name}"))
}

#[test]
fn every_vector_matches_the_parser() {
    let entries: Vec<Value> = serde_json::from_str(KEYS_JSON).expect("keys.json is a JSON array");
    let mut valid = 0;
    let mut invalid = 0;
    let mut variants = std::collections::BTreeSet::new();
    for entry in &entries {
        let key = entry["key"].as_str().expect("key is a string");
        let expect_valid = entry["valid"].as_bool().expect("valid is a bool");
        let parsed = key.parse::<ChunkKey>();
        if expect_valid {
            valid += 1;
            let parsed = parsed.unwrap_or_else(|e| panic!("{key:?} should parse, got {e:?}"));
            assert_eq!(parsed.to_string(), key, "round trip of {key:?}");
            match parsed {
                ChunkKey::Registry => assert_eq!(key, REGISTRY_KEY),
                ChunkKey::Cell(c) => {
                    let expected = CellKey {
                        frame_id: field(entry, "frame_id"),
                        depth: u8::try_from(field(entry, "depth")).unwrap(),
                        x: u32::try_from(field(entry, "x")).unwrap(),
                        y: u32::try_from(field(entry, "y")).unwrap(),
                        z: u32::try_from(field(entry, "z")).unwrap(),
                    };
                    assert_eq!(c, expected, "fields of {key:?}");
                    assert!(c.is_valid());
                }
            }
        } else {
            invalid += 1;
            let err = match parsed {
                Ok(k) => panic!("{key:?} should be rejected, parsed as {k:?}"),
                Err(e) => e,
            };
            let name = variant_name(&err);
            if let Some(expected) = entry.get("error").and_then(Value::as_str) {
                assert_eq!(name, expected, "error variant for {key:?}");
            }
            variants.insert(name);
        }
    }
    assert!(valid >= 30, "need at least 30 valid vectors, have {valid}");
    assert!(
        invalid >= 25,
        "need at least 25 invalid vectors, have {invalid}"
    );
    let all = [
        "Empty",
        "InvalidCharacter",
        "PartCount",
        "EmptyPart",
        "LeadingZero",
        "Overflow",
        "DepthOutOfRange",
        "CoordinateOutOfRange",
    ];
    for v in all {
        assert!(variants.contains(v), "no invalid vector exercises {v}");
    }
}
