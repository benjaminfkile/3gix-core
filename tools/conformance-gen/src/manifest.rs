//! Source manifest conformance vectors, written under `manifest/`
//! (`client-compute.md` section 5.1).
//!
//! - `valid/NAME.bin` is a valid manifest and `valid/NAME.json` gives its
//!   entries (id and length as decimal strings, SHA-256 as lowercase hex) and
//!   the manifest hash.
//! - `invalid/CODE-RULE.bin` breaks exactly one rule. `invalid/index.json`
//!   maps each file name to its code.
//!
//! Tile hashes are SHA-256 digests of fixed short strings.

use std::collections::BTreeMap;

use gx_core::error::codes;
use gx_core::manifest::{self, Entry, Manifest, ENTRY_LEN, HEADER_LEN};
use gx_core::sha256::{sha256, to_hex};
use serde_json::{json, Value};

use crate::{pretty, File};

fn entry(id: u64, content: &str) -> Entry {
    Entry {
        id,
        sha256: sha256(content.as_bytes()),
        length: content.len() as u64,
    }
}

/// Three tiles of a pyramid, deliberately given out of order.
fn three() -> Manifest {
    Manifest::new(vec![
        entry(0x0002_0000_0000_0003, "tile at level 2, index 3"),
        entry(1, "tile at level 0, index 1"),
        entry(0x0001_0000_0000_0000, "tile at level 1, index 0"),
    ])
    .expect("fixed manifests are valid")
}

fn summary(m: &Manifest, bytes: &[u8]) -> Value {
    let entries: Vec<Value> = m
        .entries()
        .iter()
        .map(|e| {
            json!({
                "id": e.id.to_string(),
                "sha256": to_hex(&e.sha256),
                "length": e.length.to_string(),
            })
        })
        .collect();
    json!({
        "entry_count": m.entries().len(),
        "entries": entries,
        "hash": to_hex(&manifest::manifest_hash(bytes)),
    })
}

fn valid() -> Vec<File> {
    let sets = [
        ("empty", Manifest::default()),
        (
            "one",
            Manifest::new(vec![entry(u64::MAX, "the only tile")]).expect("valid"),
        ),
        ("three", three()),
    ];
    let mut out = Vec::new();
    for (name, m) in sets {
        let b = manifest::encode(&m);
        out.push((
            format!("manifest/valid/{name}.json"),
            pretty(&summary(&m, &b)),
        ));
        out.push((format!("manifest/valid/{name}.bin"), b));
    }
    out
}

fn entry_at(i: usize, field: usize) -> usize {
    HEADER_LEN + ENTRY_LEN * i + field
}

fn invalid() -> Vec<File> {
    let base = manifest::encode(&three());
    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = base.clone();
        f(&mut b);
        vectors.push((name, code, b));
    };
    add("header-too-short", codes::MANIFEST_HEADER_TOO_SHORT, &|b| {
        b.truncate(HEADER_LEN - 1)
    });
    add("bad-magic", codes::MANIFEST_BAD_MAGIC, &|b| b[3] = b'G');
    add(
        "unsupported-version",
        codes::MANIFEST_UNSUPPORTED_VERSION,
        &|b| b[4] = 2,
    );
    add("reserved-nonzero", codes::MANIFEST_RESERVED_NONZERO, &|b| {
        b[6] = 1
    });
    add("length-short", codes::MANIFEST_LENGTH_MISMATCH, &|b| {
        b.pop();
    });
    add("length-long", codes::MANIFEST_LENGTH_MISMATCH, &|b| {
        b.push(0)
    });
    add("count-high", codes::MANIFEST_LENGTH_MISMATCH, &|b| b[8] = 4);
    add("not-sorted", codes::MANIFEST_NOT_SORTED, &|b| {
        let one = b[entry_at(1, 0)..entry_at(2, 0)].to_vec();
        let two = b[entry_at(2, 0)..entry_at(3, 0)].to_vec();
        b[entry_at(1, 0)..entry_at(2, 0)].copy_from_slice(&two);
        b[entry_at(2, 0)..entry_at(3, 0)].copy_from_slice(&one);
    });
    add("duplicate-id", codes::MANIFEST_DUPLICATE_ID, &|b| {
        let id = b[entry_at(0, 0)..entry_at(0, 8)].to_vec();
        b[entry_at(1, 0)..entry_at(1, 8)].copy_from_slice(&id);
    });

    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("manifest/invalid/{file}"), bytes));
    }
    out.push((
        "manifest/invalid/index.json".into(),
        pretty(&json!({ "vectors": index })),
    ));
    out
}

/// Every manifest vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out
}
