//! Universe time: seconds since the origin, converted from and to the hub's
//! wall-clock milliseconds.
//!
//! Implements `space-model.md` section 7 item 5. The hub stores the origin
//! and every instant as wall-clock milliseconds; every computation in the
//! library runs on universe time, `f64` seconds since the origin, and every
//! registry's epoch is 0, the origin (`matter-format.md` section 5).
//!
//! [`universe_seconds`] subtracts in `i64` and divides once; [`wall_ms`]
//! multiplies once, rounds with `f64::round`, and adds in `i64`. Each step
//! is integer arithmetic or a correctly rounded `f64` operation, so both
//! functions are exact in the sense of IEEE 754 and bitwise identical
//! natively and in WebAssembly.
//!
//! **Round trip.** `wall_ms(universe_seconds(w, o), o) == w` for every whole
//! millisecond `w` with `|w - o| <= 2^51` (about 71 000 years either side of
//! the origin). Beyond that an `f64` count of seconds no longer resolves one
//! millisecond: from `2^42` seconds on its spacing is at least `2^-10` s, so
//! a round trip can miss by one millisecond (it never misses by more within
//! `2^53` ms of the origin).

use crate::units::Seconds;

/// Universe time at a wall-clock instant: `(wall_ms - origin_ms) / 1000`
/// seconds, the subtraction in `i64` and the division in `f64`.
///
/// # Panics
///
/// Panics when `wall_ms - origin_ms` overflows `i64`, which no two instants
/// within about 146 million years of each other can do.
#[must_use]
pub fn universe_seconds(wall_ms: i64, origin_ms: i64) -> Seconds {
    let d = wall_ms
        .checked_sub(origin_ms)
        .expect("wall_ms - origin_ms overflows i64");
    Seconds::new(d as f64 / 1000.0)
}

/// The wall-clock instant, in milliseconds, of universe time `t`:
/// `origin_ms + round(t * 1000)`, rounded half away from zero by
/// `f64::round`. The conversion to `i64` and the addition saturate at the
/// `i64` range, and a NaN `t` counts as 0.
#[must_use]
pub fn wall_ms(t: Seconds, origin_ms: i64) -> i64 {
    let ms = (t.value() * 1000.0).round() as i64;
    origin_ms.saturating_add(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: i64 = 1_234_567_890_123;
    const B51: i64 = 1 << 51;
    const B53: i64 = 1 << 53;

    #[test]
    fn zero_at_the_origin() {
        assert_eq!(universe_seconds(ORIGIN, ORIGIN).value().to_bits(), 0);
        assert_eq!(wall_ms(Seconds::new(0.0), ORIGIN), ORIGIN);
        assert_eq!(wall_ms(Seconds::new(-0.0), ORIGIN), ORIGIN);
    }

    #[test]
    fn one_millisecond_either_side() {
        assert_eq!(universe_seconds(ORIGIN + 1, ORIGIN).value(), 0.001);
        assert_eq!(universe_seconds(ORIGIN - 1, ORIGIN).value(), -0.001);
        assert_eq!(wall_ms(Seconds::new(0.001), ORIGIN), ORIGIN + 1);
        assert_eq!(wall_ms(Seconds::new(-0.001), ORIGIN), ORIGIN - 1);
    }

    #[test]
    fn round_trip_on_edge_cases() {
        for d in [
            0,
            1,
            -1,
            999,
            -999,
            1000,
            -1000,
            86_399_999,
            -86_399_999,
            B51 - 1,
            -(B51 - 1),
            B51,
            -B51,
        ] {
            for origin in [0, ORIGIN, -ORIGIN] {
                let w = origin + d;
                assert_eq!(wall_ms(universe_seconds(w, origin), origin), w, "{d}");
            }
        }
    }

    #[test]
    fn round_trip_near_two_to_the_fifty_first() {
        for d in (B51 - 100_000..=B51).chain(-B51..-B51 + 100_000) {
            assert_eq!(wall_ms(universe_seconds(d, 0), 0), d, "{d}");
        }
    }

    #[test]
    fn within_one_millisecond_up_to_two_to_the_fifty_third() {
        for d in [B53 - 1, -(B53 - 1), B53 - 2, B53 - 3] {
            let t = universe_seconds(d, 0);
            assert_eq!(t.value(), d as f64 / 1000.0);
            let back = wall_ms(t, 0);
            assert!((back - d).abs() <= 1, "{d}: {back}");
        }
    }

    #[test]
    fn negative_times_and_origins() {
        assert_eq!(universe_seconds(-5_000, 0).value(), -5.0);
        assert_eq!(universe_seconds(0, -5_000).value(), 5.0);
        assert_eq!(wall_ms(Seconds::new(-5.0), 0), -5_000);
        assert_eq!(wall_ms(Seconds::new(-0.0005), 0), -1);
        assert_eq!(wall_ms(Seconds::new(0.0005), 0), 1);
    }

    #[test]
    fn bit_patterns_match_the_wasm_cases() {
        // The same cases and bit patterns are checked under Node by
        // `scripts/wasm-node-check.mjs`.
        for (wall, origin, bits) in BIT_CASES {
            let t = universe_seconds(*wall, *origin);
            assert_eq!(t.value().to_bits(), *bits, "{wall} {origin}");
            assert_eq!(wall_ms(t, *origin), *wall);
        }
    }

    /// `(wall_ms, origin_ms, bits of universe_seconds)`.
    const BIT_CASES: &[(i64, i64, u64)] = &[
        (ORIGIN, ORIGIN, 0x0000_0000_0000_0000),
        (ORIGIN + 1, ORIGIN, 0x3f50_624d_d2f1_a9fc),
        (ORIGIN - 1, ORIGIN, 0xbf50_624d_d2f1_a9fc),
        (ORIGIN + 86_399_999, ORIGIN, 0x40f5_17ff_fbe7_6c8b),
        (B51 - 1, 0, 0x4280_624d_d2f1_a9fa),
        (-(B51 - 1), 0, 0xc280_624d_d2f1_a9fa),
        (1_700_000_000_000, 1_600_000_000_000, 0x4197_d784_0000_0000),
    ];

    #[test]
    fn saturates() {
        assert_eq!(wall_ms(Seconds::new(f64::INFINITY), 0), i64::MAX);
        assert_eq!(wall_ms(Seconds::new(f64::NEG_INFINITY), 0), i64::MIN);
        assert_eq!(wall_ms(Seconds::new(f64::NAN), 7), 7);
        assert_eq!(wall_ms(Seconds::new(1.0), i64::MAX), i64::MAX);
    }

    #[test]
    #[should_panic(expected = "overflows")]
    fn overflow_panics() {
        let _ = universe_seconds(i64::MAX, -1);
    }
}
