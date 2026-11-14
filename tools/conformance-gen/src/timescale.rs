//! Time scale conversion conformance vectors, written under `timescale/`
//! (`space-model.md` section 7 item 5).
//!
//! - `cases.json` lists UTC instants (Unix-style milliseconds as decimal
//!   strings) with the TDB seconds since J2000 they convert to (decimal text
//!   and IEEE 754 bit pattern; compare the bits) and the UTC instant that TDB
//!   value converts back to.
//! - `inverse.json` lists TDB values that fall inside the 2017-01-01 leap
//!   second, each with the UTC instant it converts to.

use gx_core::timescale::{tdb_seconds_to_utc_ms, utc_ms_of_date, utc_ms_to_tdb_seconds};
use serde_json::{json, Value};

use crate::{number, pretty, File};

fn cases() -> Value {
    let leap = utc_ms_of_date(2017, 1, 1);
    let rows: [(&str, i64); 12] = [
        ("tt-equals-j2000", 946_727_935_816),
        ("j2000-label-utc", 946_728_000_000),
        ("unix-epoch", 0),
        ("before-1972", utc_ms_of_date(1960, 6, 15) + 123_456),
        ("first-leap-step", utc_ms_of_date(1972, 7, 1)),
        ("before-2017-leap", leap - 1),
        ("after-2017-leap", leap),
        ("second-after-2017-leap", leap + 1000),
        ("an-origin", 1_763_130_000_000),
        ("year-1900", utc_ms_of_date(1900, 1, 1)),
        ("year-2100", utc_ms_of_date(2100, 1, 1) + 86_399_999),
        ("negative-odd-ms", -2_000_000_000_001),
    ];
    let cases: Vec<Value> = rows
        .iter()
        .map(|&(name, utc)| {
            let tdb = utc_ms_to_tdb_seconds(utc);
            json!({
                "name": name,
                "utc_ms": utc.to_string(),
                "tdb_seconds": number(tdb),
                "back_utc_ms": tdb_seconds_to_utc_ms(tdb).to_string(),
            })
        })
        .collect();
    json!({ "cases": cases })
}

fn inverse() -> Value {
    let leap = utc_ms_of_date(2017, 1, 1);
    let start = utc_ms_to_tdb_seconds(leap - 1);
    let cases: Vec<Value> = [0.25, 0.5, 0.75]
        .iter()
        .map(|&frac| {
            let tdb = start + frac;
            json!({
                "tdb_seconds": number(tdb),
                "utc_ms": tdb_seconds_to_utc_ms(tdb).to_string(),
            })
        })
        .collect();
    json!({ "cases": cases })
}

/// Every time scale vector file.
pub fn files() -> Vec<File> {
    vec![
        ("timescale/cases.json".into(), pretty(&cases())),
        ("timescale/inverse.json".into(), pretty(&inverse())),
    ]
}
