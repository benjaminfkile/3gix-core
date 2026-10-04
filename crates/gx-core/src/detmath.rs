//! Deterministic elementary functions for the laws.
//!
//! The platform `exp` and `tan` come from the system math library, whose
//! last bit differs between platforms. The laws in [`crate::radiance`],
//! [`crate::extinction`], and [`crate::lod`] need those functions and are held
//! to the determinism rule of `docs/determinism.md`, so this module writes
//! them out with `+`, `-`, `*`, `/`, `round`, and exact power of two scaling
//! only. Every operation used is correctly rounded in IEEE 754, so the result
//! is bitwise identical on every platform. Accuracy is within a few units in
//! the last place over the ranges the laws use.

/// High part of `ln 2`, with trailing zero bits so `k * LN2_HI` is exact for
/// `|k| < 2^11`.
const LN2_HI: f64 = f64::from_bits(0x3FE6_2E42_FEE0_0000);
/// `ln 2 - LN2_HI`.
const LN2_LO: f64 = f64::from_bits(0x3DEA_39EF_3579_3C76);
/// `1 / ln 2`.
const LOG2_E: f64 = core::f64::consts::LOG2_E;

/// `2^n` for `n` in `-1022..=1023`, built from its bit pattern.
fn pow2(n: i64) -> f64 {
    debug_assert!((-1022..=1023).contains(&n));
    f64::from_bits(((n + 1023) as u64) << 52)
}

/// `p * 2^k` for `k` in `-1075..=1024`, in at most two exact steps.
fn scale(p: f64, k: i64) -> f64 {
    if k > 1023 {
        p * pow2(1023) * pow2(k - 1023)
    } else if k < -1022 {
        p * pow2(-1022) * pow2(k + 1022)
    } else {
        p * pow2(k)
    }
}

/// Taylor series of `e^r - 1` to `terms` terms, in Horner form.
fn taylor_expm1(r: f64, terms: u32) -> f64 {
    let mut acc = 1.0;
    for n in (2..=terms).rev() {
        acc = 1.0 + r / f64::from(n) * acc;
    }
    r * acc
}

/// `e^x`.
///
/// Reduces `x = k ln 2 + r` with `|r| <= ln 2 / 2`, sums the Taylor series of
/// `e^r` to 16 terms, and scales by `2^k`. Returns 0 below -745.2,
/// infinity above 709.8, and NaN for NaN.
pub(crate) fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.8 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    let k = (x * LOG2_E).round();
    let r = (x - k * LN2_HI) - k * LN2_LO;
    scale(1.0 + taylor_expm1(r, 16), k as i64)
}

/// `e^x - 1`, accurate near 0 where `exp(x) - 1` would cancel.
pub(crate) fn expm1(x: f64) -> f64 {
    if x.abs() < 0.5 {
        taylor_expm1(x, 22)
    } else {
        exp(x) - 1.0
    }
}

/// `tan(x)` for `|x| < pi / 2`, from the Taylor series of sine and cosine
/// to the `x^35` and `x^34` terms.
pub(crate) fn tan(x: f64) -> f64 {
    let x2 = x * x;
    let mut sin = 1.0;
    let mut cos = 1.0;
    for n in (1..=17u32).rev() {
        let n = f64::from(n);
        sin = 1.0 - x2 / ((2.0 * n) * (2.0 * n + 1.0)) * sin;
        cos = 1.0 - x2 / ((2.0 * n - 1.0) * (2.0 * n)) * cos;
    }
    x * sin / cos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs().max(f64::MIN_POSITIVE)
    }

    #[test]
    fn exp_matches_std() {
        let mut x = -740.0;
        while x < 709.0 {
            assert!(close(exp(x), x.exp(), 4e-15), "exp({x})");
            x += 0.371;
        }
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(1000.0), f64::INFINITY);
        assert_eq!(exp(-1000.0), 0.0);
        assert!(exp(f64::NAN).is_nan());
        assert!(close(exp(-1.0), (-1.0f64).exp(), 1e-15));
    }

    #[test]
    fn expm1_matches_std() {
        for x in [
            -3.0, -0.49, -1e-3, -1e-12, 1e-300, 1e-9, 0.2, 0.499, 0.5, 4.0,
        ] {
            assert!(close(expm1(x), f64::exp_m1(x), 4e-15), "expm1({x})");
        }
        assert_eq!(expm1(0.0), 0.0);
    }

    #[test]
    fn tan_matches_std() {
        let mut x = -1.5;
        while x < 1.5 {
            assert!(close(tan(x), x.tan(), 1e-14), "tan({x})");
            x += 0.0137;
        }
        assert_eq!(tan(0.0), 0.0);
    }
}
