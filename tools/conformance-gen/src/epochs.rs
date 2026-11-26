//! Epoch index conformance vectors, written under `epochs/`
//! (`client-compute.md` section 5.4).
//!
//! - `valid/NAME.bin` is a valid epoch index and `valid/NAME.json` gives its
//!   epoch seconds, its entries (epoch index as a decimal string, manifest
//!   hash as lowercase hex), the index hash, and a table of epoch rule cases:
//!   `span_seconds` and `span_index` as decimal strings, and either
//!   `manifest` (lowercase hex, or `null` when the epoch is not listed) or
//!   the expected error `code`.
//! - `invalid/CODE-RULE.bin` breaks exactly one rule. `invalid/index.json`
//!   maps each file name to its code.
//!
//! Manifest hashes are SHA-256 digests of fixed short strings.

use std::collections::BTreeMap;

use gx_core::epochs::{self, Entry, EpochIndex, ENTRY_LEN, HEADER_LEN};
use gx_core::error::codes;
use gx_core::sha256::{sha256, to_hex};
use serde_json::{json, Value};

use crate::{pretty, File};

/// Epoch length of the `regular` index: twenty spans of `HOUR`, a made-up
/// length that is no body's day.
const EPOCH_SPAN: u64 = 72_000;
const HOUR: u64 = 3_600;
/// Spans of `HOUR` in one epoch of the `regular` index.
const PER_EPOCH: u64 = EPOCH_SPAN / HOUR;

fn entry(epoch: u64) -> Entry {
    Entry {
        epoch,
        manifest: sha256(format!("manifest of epoch {epoch}").as_bytes()),
    }
}

/// Epochs of `EPOCH_SPAN` seconds with a gap at epoch 2 and an entry at the
/// epoch of the largest hourly span index, deliberately given out of order.
fn regular() -> EpochIndex {
    EpochIndex::new(
        EPOCH_SPAN,
        vec![entry(3), entry(0), entry(u64::MAX / PER_EPOCH), entry(1)],
    )
    .expect("fixed indexes are valid")
}

/// The epoch rule cases each valid index is checked against.
fn cases(name: &str) -> Vec<(u64, u64)> {
    match name {
        "regular" => vec![
            // Hourly spans, 20 per epoch: both sides of each boundary.
            (HOUR, 0),
            (HOUR, 19),
            (HOUR, 20),
            (HOUR, 39),
            (HOUR, 40),
            (HOUR, 59),
            (HOUR, 60),
            (HOUR, 79),
            (HOUR, 80),
            // Spans as long as the epoch: one per epoch.
            (EPOCH_SPAN, 1),
            (EPOCH_SPAN, 2),
            (EPOCH_SPAN, 3),
            // One-second spans across the first boundary.
            (1, EPOCH_SPAN - 1),
            (1, EPOCH_SPAN),
            // Large span indexes: the product needs more than 64 bits.
            (HOUR, u64::MAX),
            (HOUR, u64::MAX / PER_EPOCH * PER_EPOCH),
            (HOUR, u64::MAX / PER_EPOCH * PER_EPOCH - 1),
            (EPOCH_SPAN, u64::MAX),
            // Errors: a zero span and a span that does not divide the epoch.
            (0, 5),
            (7, 5),
            (EPOCH_SPAN * 2, 0),
        ],
        "empty" => vec![(HOUR, 0), (HOUR, u64::MAX), (0, 0), (7_200, 1)],
        "widest" => vec![
            (u64::MAX, 0),
            (u64::MAX, 1),
            (u64::MAX, u64::MAX),
            (1, u64::MAX),
            (3, u64::MAX),
            (2, 1),
        ],
        _ => unreachable!("no cases for {name}"),
    }
}

fn summary(name: &str, x: &EpochIndex, bytes: &[u8]) -> Value {
    let entries: Vec<Value> = x
        .entries()
        .iter()
        .map(|e| {
            json!({
                "epoch": e.epoch.to_string(),
                "manifest": to_hex(&e.manifest),
            })
        })
        .collect();
    let cases: Vec<Value> = cases(name)
        .into_iter()
        .map(|(span_seconds, span_index)| {
            let mut c = json!({
                "span_seconds": span_seconds.to_string(),
                "span_index": span_index.to_string(),
            });
            match epochs::manifest_for_span(x, span_seconds, span_index) {
                Ok(m) => c["manifest"] = m.map_or(Value::Null, |h| to_hex(&h).into()),
                Err(e) => c["code"] = e.code.into(),
            }
            c
        })
        .collect();
    json!({
        "epoch_seconds": x.epoch_seconds().to_string(),
        "entry_count": x.entries().len(),
        "entries": entries,
        "hash": to_hex(&epochs::epochs_hash(bytes)),
        "cases": cases,
    })
}

fn valid() -> Vec<File> {
    let sets = [
        ("regular", regular()),
        ("empty", EpochIndex::new(HOUR, vec![]).expect("valid")),
        (
            "widest",
            EpochIndex::new(u64::MAX, vec![entry(u64::MAX), entry(0)]).expect("valid"),
        ),
    ];
    let mut out = Vec::new();
    for (name, x) in sets {
        let b = epochs::encode(&x);
        out.push((
            format!("epochs/valid/{name}.json"),
            pretty(&summary(name, &x, &b)),
        ));
        out.push((format!("epochs/valid/{name}.bin"), b));
    }
    out
}

fn entry_at(i: usize, field: usize) -> usize {
    HEADER_LEN + ENTRY_LEN * i + field
}

fn invalid() -> Vec<File> {
    let base = epochs::encode(&regular());
    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = base.clone();
        f(&mut b);
        vectors.push((name, code, b));
    };
    add("header-too-short", codes::EPOCHS_HEADER_TOO_SHORT, &|b| {
        b.truncate(HEADER_LEN - 1)
    });
    add("bad-magic", codes::EPOCHS_BAD_MAGIC, &|b| b[3] = b'M');
    add(
        "unsupported-version",
        codes::EPOCHS_UNSUPPORTED_VERSION,
        &|b| b[4] = 2,
    );
    add("reserved-nonzero", codes::EPOCHS_RESERVED_NONZERO, &|b| {
        b[6] = 1
    });
    add(
        "zero-epoch-seconds",
        codes::EPOCHS_ZERO_EPOCH_SECONDS,
        &|b| b[8..16].fill(0),
    );
    add("length-short", codes::EPOCHS_LENGTH_MISMATCH, &|b| {
        b.pop();
    });
    add("length-long", codes::EPOCHS_LENGTH_MISMATCH, &|b| b.push(0));
    add("count-high", codes::EPOCHS_LENGTH_MISMATCH, &|b| b[16] = 5);
    add("not-ascending", codes::EPOCHS_NOT_ASCENDING, &|b| {
        let one = b[entry_at(1, 0)..entry_at(2, 0)].to_vec();
        let two = b[entry_at(2, 0)..entry_at(3, 0)].to_vec();
        b[entry_at(1, 0)..entry_at(2, 0)].copy_from_slice(&two);
        b[entry_at(2, 0)..entry_at(3, 0)].copy_from_slice(&one);
    });
    add("duplicate-epoch", codes::EPOCHS_NOT_ASCENDING, &|b| {
        let epoch = b[entry_at(0, 0)..entry_at(0, 8)].to_vec();
        b[entry_at(1, 0)..entry_at(1, 8)].copy_from_slice(&epoch);
    });

    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("epochs/invalid/{file}"), bytes));
    }
    out.push((
        "epochs/invalid/index.json".into(),
        pretty(&json!({ "vectors": index })),
    ));
    out
}

/// Every epoch index vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out
}
