//! Time scale conversion: UTC milliseconds to seconds of Barycentric
//! Dynamical Time (TDB) since J2000, and back.
//!
//! Implements `space-model.md` section 7 item 5. The hub stores UTC; every
//! computation in the library uses TDB seconds since J2000, the epoch
//! 2000-01-01 12:00:00 TDB. The chain is:
//!
//! 1. UTC to TAI with the built-in leap second table [`LEAP_SECONDS`]:
//!    `TAI = UTC + (TAI - UTC)`.
//! 2. TT = TAI + 32.184 s ([`TT_MINUS_TAI_MS`]).
//! 3. TDB = TT + `0.001657 sin(g) + 0.000014 sin(2g)` seconds, with
//!    `g = 357.53 + 0.98560028 (JD_TT - 2451545.0)` degrees.
//!
//! UTC milliseconds are counted the way Unix time counts them: milliseconds
//! since 1970-01-01T00:00:00 UTC with every day exactly 86 400 000 ms long,
//! so an inserted leap second has no UTC millisecond of its own. Converting
//! UTC to TDB therefore steps forward by one extra second across a leap
//! second, and TDB instants inside a leap second convert back to the first
//! millisecond of the following day.
//!
//! The table holds every leap second through the most recent one, at
//! 2017-01-01. **When the IERS announces a new leap second, add a row to
//! [`LEAP_SECONDS`]**; until then UTC after the last row is taken to keep
//! TAI - UTC = 37 s. Before 1972-01-01, when UTC did not yet step by whole
//! seconds, TAI - UTC is held at the 1972 value of 10 s: the variable rate
//! UTC of 1961 to 1971 is not modeled. Both are conventions of this library
//! and only matter for dates no space's origin can have.
//!
//! The sine is [`crate::detmath`]'s, and every other step is integer
//! arithmetic or a correctly rounded `f64` operation, so results are bitwise
//! identical natively and in WebAssembly.

use crate::detmath;

/// TT - TAI in milliseconds: 32.184 s, fixed by definition.
pub const TT_MINUS_TAI_MS: i64 = 32_184;

/// Seconds in one day.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Milliseconds in one day.
const MS_PER_DAY: i64 = 86_400_000;

/// Days from 1970-01-01 to the given proleptic Gregorian date.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// UTC milliseconds of midnight at the start of the given date.
pub const fn utc_ms_of_date(year: i64, month: i64, day: i64) -> i64 {
    days_from_civil(year, month, day) * MS_PER_DAY
}

/// UTC milliseconds of the J2000 epoch's calendar label, 2000-01-01
/// 12:00:00. J2000 is that label on the TT scale.
const J2000_LABEL_MS: i64 = utc_ms_of_date(2000, 1, 1) + MS_PER_DAY / 2;

/// Every step of TAI - UTC since 1972: `(year, month, seconds)` means that
/// from 00:00:00 UTC on the first day of that month, TAI - UTC is that many
/// seconds. Source: IERS Bulletin C. Extend this table when a new leap second
/// is announced.
pub const LEAP_SECONDS: [(i64, i64, i64); 28] = [
    (1972, 1, 10),
    (1972, 7, 11),
    (1973, 1, 12),
    (1974, 1, 13),
    (1975, 1, 14),
    (1976, 1, 15),
    (1977, 1, 16),
    (1978, 1, 17),
    (1979, 1, 18),
    (1980, 1, 19),
    (1981, 7, 20),
    (1982, 7, 21),
    (1983, 7, 22),
    (1985, 7, 23),
    (1988, 1, 24),
    (1990, 1, 25),
    (1991, 1, 26),
    (1992, 7, 27),
    (1993, 7, 28),
    (1994, 7, 29),
    (1996, 1, 30),
    (1997, 7, 31),
    (1999, 1, 32),
    (2006, 1, 33),
    (2009, 1, 34),
    (2012, 7, 35),
    (2015, 7, 36),
    (2017, 1, 37),
];

/// UTC milliseconds at which row `i` of [`LEAP_SECONDS`] takes effect, and
/// its TAI - UTC in milliseconds.
fn leap_row(i: usize) -> (i64, i64) {
    let (y, m, s) = LEAP_SECONDS[i];
    (utc_ms_of_date(y, m, 1), s * 1000)
}

/// TAI - UTC in milliseconds at the UTC instant `utc_ms`, from
/// [`LEAP_SECONDS`]: 10 000 before 1972 and 37 000 after the last row.
pub fn tai_minus_utc_ms(utc_ms: i64) -> i64 {
    (0..LEAP_SECONDS.len())
        .rev()
        .map(leap_row)
        .find(|&(start, _)| utc_ms >= start)
        .map_or(leap_row(0).1, |(_, offset)| offset)
}

/// The TT-scale label in milliseconds of a TAI-scale label, both counted
/// like UTC milliseconds.
fn utc_to_tt_label_ms(utc_ms: i64) -> i128 {
    i128::from(utc_ms) + i128::from(tai_minus_utc_ms(utc_ms)) + i128::from(TT_MINUS_TAI_MS)
}

/// The UTC millisecond of a TAI label in milliseconds. A TAI instant inside
/// an inserted leap second maps to the first millisecond after it.
fn tai_label_to_utc_ms(tai: i128) -> i128 {
    for i in (0..LEAP_SECONDS.len()).rev() {
        let (start, offset) = leap_row(i);
        let (start, offset) = (i128::from(start), i128::from(offset));
        if tai >= start + offset {
            return tai - offset;
        }
        if i > 0 {
            let before = i128::from(leap_row(i - 1).1);
            if tai >= start + before {
                return start;
            }
        }
    }
    tai - i128::from(leap_row(0).1)
}

/// TDB - TT in seconds at `tt` seconds of TT since J2000: the standard
/// periodic term `0.001657 sin(g) + 0.000014 sin(2g)`.
fn tdb_minus_tt(tt: f64) -> f64 {
    let days = tt / SECONDS_PER_DAY;
    let g_degrees = 357.53 + 0.985_600_28 * days;
    let g = g_degrees * (core::f64::consts::PI / 180.0);
    0.001_657 * detmath::sin(g) + 0.000_014 * detmath::sin(2.0 * g)
}

/// Seconds of TT since J2000 at the UTC instant `utc_ms`.
pub fn utc_ms_to_tt_seconds(utc_ms: i64) -> f64 {
    let rel_ms = utc_to_tt_label_ms(utc_ms) - i128::from(J2000_LABEL_MS);
    rel_ms as f64 / 1000.0
}

/// Seconds of Barycentric Dynamical Time since J2000 (2000-01-01 12:00:00
/// TDB) at the UTC instant `utc_ms`, Unix-style milliseconds.
///
/// UTC to TAI by [`LEAP_SECONDS`], TT = TAI + 32.184 s, then the periodic
/// TDB - TT term. Bitwise identical on every target.
pub fn utc_ms_to_tdb_seconds(utc_ms: i64) -> f64 {
    let tt = utc_ms_to_tt_seconds(utc_ms);
    tt + tdb_minus_tt(tt)
}

/// The UTC instant, Unix-style milliseconds rounded to the nearest, of `tdb`
/// seconds of Barycentric Dynamical Time since J2000. The inverse of
/// [`utc_ms_to_tdb_seconds`].
///
/// Solves `tt + tdb_minus_tt(tt) = tdb` by fixed-point iteration from
/// `tt = tdb` until `tt` repeats bitwise, at most 8 steps (it settles in two
/// or three, since the term's slope is about `3e-10`). An instant inside an
/// inserted leap second gives the first millisecond of the following day.
/// Results beyond the `i64` range saturate, and NaN gives the UTC instant of
/// J2000's label minus 64.184 s (a NaN converts to integer 0, Rust's
/// saturating rule).
pub fn tdb_seconds_to_utc_ms(tdb: f64) -> i64 {
    if tdb.is_infinite() {
        return if tdb > 0.0 { i64::MAX } else { i64::MIN };
    }
    let mut tt = tdb;
    for _ in 0..8 {
        let next = tdb - tdb_minus_tt(tt);
        if next.to_bits() == tt.to_bits() {
            break;
        }
        tt = next;
    }
    let rel_ms = (tt * 1000.0).round() as i128;
    let tai = rel_ms + i128::from(J2000_LABEL_MS) - i128::from(TT_MINUS_TAI_MS);
    let utc = tai_label_to_utc_ms(tai);
    utc.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;

    #[test]
    fn calendar() {
        assert_eq!(utc_ms_of_date(1970, 1, 1), 0);
        assert_eq!(utc_ms_of_date(2000, 1, 1), 946_684_800_000);
        assert_eq!(utc_ms_of_date(2017, 1, 1), 1_483_228_800_000);
        assert_eq!(utc_ms_of_date(1900, 1, 1), -2_208_988_800_000);
        assert_eq!(utc_ms_of_date(2100, 3, 1), 4_107_542_400_000);
        assert_eq!(J2000_LABEL_MS, 946_728_000_000);
    }

    #[test]
    fn j2000() {
        // 2000-01-01T11:58:55.816Z: TAI - UTC was 32 s, so TT is exactly
        // 12:00:00 and TDB differs from it only by the periodic term.
        let utc = J2000_LABEL_MS - 32_000 - TT_MINUS_TAI_MS;
        assert_eq!(utc, 946_727_935_816);
        assert_eq!(utc_ms_to_tt_seconds(utc), 0.0);
        let tdb = utc_ms_to_tdb_seconds(utc);
        assert!(tdb.abs() < 1e-3, "{tdb}");
        // The periodic term at g = 357.53 degrees:
        // 0.001657 sin(357.53) + 0.000014 sin(715.06) = -7.2616e-5 s.
        assert!((tdb - -7.2616e-5).abs() < 1e-9, "{tdb}");
        assert_eq!(tdb_seconds_to_utc_ms(tdb), utc);
        assert_eq!(tdb_seconds_to_utc_ms(0.0), utc);
    }

    #[test]
    fn leap_second_2017() {
        let t = utc_ms_of_date(2017, 1, 1);
        assert_eq!(tai_minus_utc_ms(t - 1), 36_000);
        assert_eq!(tai_minus_utc_ms(t), 37_000);
        // One UTC millisecond spans the inserted second.
        let before = utc_ms_to_tdb_seconds(t - 1);
        let after = utc_ms_to_tdb_seconds(t);
        assert!((after - before - 1.001).abs() < 1e-6, "{}", after - before);
        // TT at 2017-01-01T00:00:00Z is 69.184 s past the UTC label.
        let tt = utc_ms_to_tt_seconds(t);
        assert_eq!(tt, (t - J2000_LABEL_MS) as f64 / 1000.0 + 69.184);
        // Both sides round trip, and the leap second itself maps forward.
        assert_eq!(tdb_seconds_to_utc_ms(before), t - 1);
        assert_eq!(tdb_seconds_to_utc_ms(after), t);
        for frac in [0.1, 0.5, 0.9] {
            assert_eq!(tdb_seconds_to_utc_ms(before + frac), t, "{frac}");
        }
        // A second before the leap second is still on the old offset.
        assert_eq!(
            tdb_seconds_to_utc_ms(utc_ms_to_tdb_seconds(t - 1000)),
            t - 1000
        );
    }

    #[test]
    fn every_leap_second_steps_by_one() {
        for i in 1..LEAP_SECONDS.len() {
            let (start, offset) = leap_row(i);
            assert_eq!(offset - leap_row(i - 1).1, 1000);
            let tt0 = utc_ms_to_tt_seconds(start - 1);
            let tt1 = utc_ms_to_tt_seconds(start);
            assert!((tt1 - tt0 - 1.001).abs() < 1e-6);
            assert_eq!(tdb_seconds_to_utc_ms(utc_ms_to_tdb_seconds(start)), start);
        }
        // Before 1972 and after the last row the offset is held.
        assert_eq!(tai_minus_utc_ms(utc_ms_of_date(1950, 1, 1)), 10_000);
        assert_eq!(tai_minus_utc_ms(utc_ms_of_date(2090, 1, 1)), 37_000);
    }

    #[test]
    fn round_trip_1900_to_2100() {
        let start = utc_ms_of_date(1900, 1, 1);
        let end = utc_ms_of_date(2100, 1, 1);
        let step = (end - start) / 1000;
        for i in 0..1000 {
            // An odd millisecond offset so samples fall between whole seconds.
            let utc = start + i * step + (i * 7919) % 86_400_000;
            let tdb = utc_ms_to_tdb_seconds(utc);
            let back = tdb_seconds_to_utc_ms(tdb);
            assert!((back - utc).abs() <= 1, "{utc}: {back}");
            assert_eq!(back, utc, "exact round trip at {utc}");
            let periodic = tdb - utc_ms_to_tt_seconds(utc);
            assert!(periodic.abs() <= 0.001_672, "{periodic}");
        }
    }

    #[test]
    fn monotonic_and_extremes() {
        let t = utc_ms_of_date(2026, 11, 14);
        let mut prev = utc_ms_to_tdb_seconds(t);
        for k in 1..48 {
            let x = utc_ms_to_tdb_seconds(t + k * HOUR);
            assert!(x > prev);
            prev = x;
        }
        let _ = utc_ms_to_tdb_seconds(i64::MIN);
        let _ = utc_ms_to_tdb_seconds(i64::MAX);
        assert_eq!(tdb_seconds_to_utc_ms(f64::INFINITY), i64::MAX);
        assert_eq!(tdb_seconds_to_utc_ms(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(
            tdb_seconds_to_utc_ms(f64::NAN),
            J2000_LABEL_MS - 32_000 - TT_MINUS_TAI_MS
        );
    }
}
