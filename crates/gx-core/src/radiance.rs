//! Blackbody radiance from temperature, per wavelength and per band.
//!
//! Implements the blackbody law of `space-model.md` section 1 (rule 2) and
//! section 2 (the renderer derives emission from temperature), and the
//! emission derived quantity of `matter-format.md` section 3.3: blackbody
//! radiance from a sample's `temperature`, scaled by emissivity
//! `1 - albedo` per band. Any matter with a temperature above 0 K emits; the
//! law does not know what the matter is.
//!
//! # Bands
//!
//! Format version 1 fixes three wavelength bands, given by [`BAND_EDGES`] in
//! meters, from long to short wavelength:
//!
//! | Band | From | To |
//! |---|---|---|
//! | 0 | 600 nm | 700 nm |
//! | 1 | 500 nm | 600 nm |
//! | 2 | 400 nm | 500 nm |
//!
//! The three values of a sample's albedo channel map to these bands in this
//! order (`matter-format.md` section 3.3: long to short wavelength).
//!
//! # Determinism
//!
//! [`band_radiance`] integrates [`spectral_radiance`] over each band with a
//! fixed [`BAND_POINTS`]-point midpoint rule in `f64`, summed in ascending
//! wavelength order. The rule and the point count are part of the
//! deterministic contract: changing either changes every band value. The
//! exponential is evaluated by a written-out routine using only correctly
//! rounded operations, so results are bitwise identical on every platform.

use core::f64::consts::PI;

use crate::detmath;
use crate::units::{Kelvin, Ratio};

/// Planck constant `h`, J s (CODATA 2018, exact).
pub const PLANCK: f64 = 6.626_070_15e-34;

/// Speed of light in vacuum `c`, m/s (CODATA 2018, exact).
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;

/// Boltzmann constant `k_B`, J/K (CODATA 2018, exact).
pub const BOLTZMANN: f64 = 1.380_649e-23;

/// Stefan-Boltzmann constant `sigma`, W m^-2 K^-4 (CODATA 2018).
pub const STEFAN_BOLTZMANN: f64 = 5.670_374_419e-8;

/// Edges of the three bands in meters, long to short wavelength. Band `i`
/// runs from `BAND_EDGES[i + 1]` to `BAND_EDGES[i]`.
pub const BAND_EDGES: [f64; 4] = [700e-9, 600e-9, 500e-9, 400e-9];

/// Number of midpoint rule points per band in [`band_radiance`]. Part of the
/// deterministic contract.
pub const BAND_POINTS: u32 = 64;

/// Largest exponent `h c / (lambda k_B T)` evaluated. Above it the radiance
/// is below `1e-300` of its peak scale and is returned as 0, which keeps the
/// exponential finite.
const MAX_EXPONENT: f64 = 700.0;

/// Spectral radiance of a blackbody by Planck's law, in W m^-2 sr^-1 m^-1
/// (per meter of wavelength):
///
/// `B(lambda, T) = 2 h c^2 / lambda^5 / (exp(h c / (lambda k_B T)) - 1)`.
///
/// Returns 0 for a temperature or wavelength at or below 0, for NaN inputs,
/// and when the exponent exceeds 700 (cold matter at short wavelength), so
/// the exponential never overflows.
pub fn spectral_radiance(wavelength_m: f64, temperature: Kelvin) -> f64 {
    let t = temperature.value();
    let l = wavelength_m;
    // Written so NaN fails the test too.
    if !(t > 0.0 && l > 0.0) {
        return 0.0;
    }
    let x = PLANCK * SPEED_OF_LIGHT / (l * BOLTZMANN * t);
    if x > MAX_EXPONENT {
        return 0.0;
    }
    let l5 = l * l * l * l * l;
    2.0 * PLANCK * SPEED_OF_LIGHT * SPEED_OF_LIGHT / l5 / detmath::expm1(x)
}

/// Blackbody radiance integrated over each band of [`BAND_EDGES`], in
/// W m^-2 sr^-1, band 0 (long wavelength) first.
///
/// Each band is integrated with the [`BAND_POINTS`]-point midpoint rule:
/// `width = (hi - lo) / 64`, and the sum over `j` from 0 to 63 in that order
/// of `spectral_radiance(lo + (j + 0.5) * width) * width`. 0 K gives zeros.
pub fn band_radiance(temperature: Kelvin) -> [f64; 3] {
    core::array::from_fn(|band| {
        let hi = BAND_EDGES[band];
        let lo = BAND_EDGES[band + 1];
        let width = (hi - lo) / f64::from(BAND_POINTS);
        let mut sum = 0.0;
        for j in 0..BAND_POINTS {
            let l = lo + (f64::from(j) + 0.5) * width;
            sum += spectral_radiance(l, temperature) * width;
        }
        sum
    })
}

/// Blackbody radiance over all wavelengths, in W m^-2 sr^-1:
/// `sigma T^4 / pi`. The analytic value the band integration is checked
/// against.
pub fn total_radiance(temperature: Kelvin) -> f64 {
    let t = temperature.value();
    STEFAN_BOLTZMANN * t * t * t * t / PI
}

/// Radiance emitted by opaque matter in each band, in W m^-2 sr^-1:
/// [`band_radiance`] times the emissivity `1 - albedo` of that band
/// (Kirchhoff's law). Albedo 1 in a band emits nothing in it.
pub fn emitted_band_radiance(temperature: Kelvin, albedo: [Ratio; 3]) -> [f64; 3] {
    let b = band_radiance(temperature);
    core::array::from_fn(|i| b[i] * (1.0 - albedo[i].value()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_invalid_inputs() {
        assert_eq!(spectral_radiance(500e-9, Kelvin::new(0.0)), 0.0);
        assert_eq!(spectral_radiance(500e-9, Kelvin::new(-5.0)), 0.0);
        assert_eq!(spectral_radiance(0.0, Kelvin::new(300.0)), 0.0);
        assert_eq!(spectral_radiance(f64::NAN, Kelvin::new(300.0)), 0.0);
        assert_eq!(spectral_radiance(500e-9, Kelvin::new(f64::NAN)), 0.0);
        assert_eq!(band_radiance(Kelvin::new(0.0)), [0.0; 3]);
    }

    #[test]
    fn cold_short_wavelength_does_not_overflow() {
        for t in [1e-6, 1e-3, 1.0, 3.0, 20.0] {
            let b = spectral_radiance(10e-9, Kelvin::new(t));
            assert!(b.is_finite() && b >= 0.0, "{t} K gives {b}");
            assert!(band_radiance(Kelvin::new(t)).iter().all(|v| v.is_finite()));
        }
        assert_eq!(spectral_radiance(400e-9, Kelvin::new(1.0)), 0.0);
    }

    #[test]
    fn long_wavelength_limit() {
        // Rayleigh-Jeans: B -> 2 c k_B T / lambda^4 when h c << lambda k_B T.
        let l = 1.0;
        let t = Kelvin::new(1000.0);
        let rj = 2.0 * SPEED_OF_LIGHT * BOLTZMANN * 1000.0 / (l * l * l * l);
        let b = spectral_radiance(l, t);
        assert!((b - rj).abs() < 1e-5 * rj);
    }

    #[test]
    fn band_radiance_is_bitwise_stable() {
        let t = Kelvin::new(5800.0);
        assert_eq!(band_radiance(t), band_radiance(t));
        assert!(band_radiance(t).iter().all(|&v| v > 0.0));
    }

    #[test]
    fn albedo_scales_emission() {
        let t = Kelvin::new(4000.0);
        let b = band_radiance(t);
        let half = emitted_band_radiance(t, [Ratio::new(0.5); 3]);
        let none = emitted_band_radiance(t, [Ratio::new(0.0); 3]);
        assert_eq!(none, b);
        for i in 0..3 {
            assert_eq!(half[i], b[i] * 0.5);
        }
    }
}
