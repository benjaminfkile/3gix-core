//! Checks the time scale conformance vectors under `conformance/timescale/`
//! (`space-model.md` section 7 item 5): every UTC instant converts to the
//! recorded TDB bit pattern and back to the recorded UTC instant, and every
//! TDB value inside the 2017-01-01 leap second converts to the recorded UTC
//! instant.

use std::path::{Path, PathBuf};

use gx_core::timescale::{tdb_seconds_to_utc_ms, utc_ms_to_tdb_seconds};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/timescale")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

fn int(v: &Value) -> i64 {
    v.as_str()
        .expect("integers are strings")
        .parse()
        .expect("integer")
}

/// Reads the `bits` of a recorded number and checks `decimal` agrees.
fn number(v: &Value) -> f64 {
    let bits = v["bits"].as_str().expect("bits is a string");
    let bits = u64::from_str_radix(bits.trim_start_matches("0x"), 16).expect("hex bits");
    let decimal: f64 = v["decimal"].as_str().unwrap().parse().unwrap();
    assert_eq!(decimal.to_bits(), bits, "decimal and bits disagree in {v}");
    f64::from_bits(bits)
}

#[test]
fn cases_match() {
    let index = read_json("cases.json");
    let cases = index["cases"].as_array().expect("cases");
    assert!(cases.len() >= 10);
    for c in cases {
        let utc = int(&c["utc_ms"]);
        let tdb = utc_ms_to_tdb_seconds(utc);
        let want = number(&c["tdb_seconds"]);
        assert_eq!(tdb.to_bits(), want.to_bits(), "{}: {tdb:e}", c["name"]);
        assert_eq!(
            tdb_seconds_to_utc_ms(want),
            int(&c["back_utc_ms"]),
            "{}",
            c["name"]
        );
        assert_eq!(int(&c["back_utc_ms"]), utc, "{}: round trip", c["name"]);
    }
}

#[test]
fn inverse_inside_leap_second() {
    let index = read_json("inverse.json");
    for c in index["cases"].as_array().expect("cases") {
        let tdb = number(&c["tdb_seconds"]);
        assert_eq!(tdb_seconds_to_utc_ms(tdb), int(&c["utc_ms"]), "{c}");
    }
}
