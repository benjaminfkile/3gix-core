//! Input list and input bytes conformance vectors, written under
//! `input-list/` (`client-compute.md` sections 4.1 and 10.4).
//!
//! - `valid/NAME.bin`: valid input lists, and `valid/index.json` mapping each
//!   file to its entries (`slot` and `key`).
//! - `invalid/CODE-RULE.bin`: one file or more for every code from 1031 to
//!   1036, and `invalid/index.json` mapping each file to its code.
//! - `inputs.json`: input bytes as hex with each input as hex and the
//!   `X-Inputs-Sha256` value, and malformed input bytes with their code
//!   (1037, 1038).

use std::collections::BTreeMap;

use gx_core::error::codes;
use gx_core::module_io::{
    decode_input_list, decode_inputs, encode_input_list, encode_inputs, inputs_sha256, InputEntry,
};
use gx_core::sha256::to_hex;
use serde_json::{json, Value};

use crate::{pretty, File};

fn entry(slot: u32, key: &str) -> InputEntry {
    InputEntry {
        slot,
        key: key.parse().expect("fixed keys are valid"),
    }
}

/// Raw list bytes from `(slot, key bytes)` pairs, with no checks.
fn raw(entries: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = (entries.len() as u32).to_le_bytes().to_vec();
    for (slot, key) in entries {
        out.extend_from_slice(&slot.to_le_bytes());
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key);
    }
    out
}

fn valid() -> Vec<File> {
    let lists: Vec<(&str, Vec<InputEntry>)> = vec![
        ("empty", vec![]),
        ("one", vec![entry(0, "1-0-0-0-0")]),
        (
            "sorted-by-slot-then-key",
            vec![
                entry(0, "2-0-0-0-0"),
                entry(0, "registry"),
                entry(1, "1-0-0-0-0"),
                entry(3, "frame-3"),
            ],
        ),
        (
            "sixty-four",
            (0..64).map(|s| entry(s, "1-0-0-0-0")).collect(),
        ),
    ];
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, entries) in lists {
        let b = encode_input_list(&entries).expect("fixed lists are valid");
        assert_eq!(decode_input_list(&b).expect("round trip"), entries);
        let file = format!("{name}.bin");
        let list: Vec<Value> = entries
            .iter()
            .map(|e| json!({ "slot": e.slot, "key": e.key.to_string() }))
            .collect();
        index.insert(file.clone(), json!(list));
        out.push((format!("input-list/valid/{file}"), b));
    }
    out.push(("input-list/valid/index.json".into(), pretty(&json!(index))));
    out
}

fn invalid() -> Vec<File> {
    let good = raw(&[(0, b"1-0-0-0-0"), (1, b"registry")]);
    let mut trailing = good.clone();
    trailing.push(0);
    let mut too_many = raw(&[]);
    too_many[0] = 65;
    let vectors: Vec<(&str, u16, Vec<u8>)> = vec![
        ("empty", codes::INPUT_LIST_TOO_SHORT, vec![]),
        ("three-bytes", codes::INPUT_LIST_TOO_SHORT, vec![0, 0, 0]),
        ("count-65", codes::INPUT_LIST_TOO_MANY, too_many),
        (
            "key-cut",
            codes::INPUT_LIST_TRUNCATED,
            good[..good.len() - 1].to_vec(),
        ),
        (
            "header-cut",
            codes::INPUT_LIST_TRUNCATED,
            good[..10].to_vec(),
        ),
        ("trailing", codes::INPUT_LIST_TRAILING_BYTES, trailing),
        (
            "key-not-a-key",
            codes::INPUT_LIST_KEY_MALFORMED,
            raw(&[(0, b"1-0-0-0-1")]),
        ),
        (
            "key-not-utf8",
            codes::INPUT_LIST_KEY_MALFORMED,
            raw(&[(0, b"\xff")]),
        ),
        (
            "slot-descending",
            codes::INPUT_LIST_NOT_SORTED,
            raw(&[(1, b"registry"), (0, b"registry")]),
        ),
        (
            "key-descending",
            codes::INPUT_LIST_NOT_SORTED,
            raw(&[(0, b"registry"), (0, b"1-0-0-0-0")]),
        ),
        (
            "duplicate",
            codes::INPUT_LIST_NOT_SORTED,
            raw(&[(2, b"frame-3"), (2, b"frame-3")]),
        ),
    ];
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        assert_eq!(
            decode_input_list(&bytes).expect_err(name).code,
            code,
            "{name}"
        );
        let file = format!("{code}-{name}.bin");
        assert!(index.insert(file.clone(), code).is_none(), "{file}");
        out.push((format!("input-list/invalid/{file}"), bytes));
    }
    out.push((
        "input-list/invalid/index.json".into(),
        pretty(&json!(index)),
    ));
    out
}

fn inputs() -> File {
    let sets: Vec<(&str, Vec<&[u8]>)> = vec![
        ("none", vec![]),
        ("one-empty", vec![b""]),
        ("three", vec![b"section a", b"", b"record b"]),
    ];
    let valid: Vec<Value> = sets
        .iter()
        .map(|(name, set)| {
            let b = encode_inputs(set).expect("small inputs");
            json!({
                "name": name,
                "bytes": to_hex(&b),
                "inputs": set.iter().map(|i| to_hex(i)).collect::<Vec<_>>(),
                "inputs_sha256": to_hex(&inputs_sha256(set)),
            })
        })
        .collect();
    let three = encode_inputs(&sets[2].1).expect("small inputs");
    let mut trailing = three.clone();
    trailing.push(0);
    let invalid: Vec<Value> = [
        ("short", three[..3].to_vec()),
        ("cut", three[..three.len() - 1].to_vec()),
        ("count-past-end", vec![2, 0, 0, 0, 0, 0, 0, 0]),
        ("trailing", trailing),
    ]
    .iter()
    .map(|(name, b)| {
        json!({
            "name": name,
            "bytes": to_hex(b),
            "code": decode_inputs(b).expect_err(name).code,
        })
    })
    .collect();
    (
        "input-list/inputs.json".into(),
        pretty(&json!({ "valid": valid, "invalid": invalid })),
    )
}

/// Every input list and input bytes vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out.push(inputs());
    out
}
