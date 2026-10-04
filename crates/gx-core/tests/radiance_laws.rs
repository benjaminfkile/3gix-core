//! Checks of the radiance, emission, and extinction laws
//! (`matter-format.md` section 3.3, derived quantities) with the tolerances
//! the task sets.

use core::f64::consts::PI;

use gx_core::emission::summarize;
use gx_core::extinction::transmittance;
use gx_core::key::CellKey;
use gx_core::matter::{Sample, Samples, Section, State};
use gx_core::radiance::{band_radiance, emitted_band_radiance, spectral_radiance, total_radiance};
use gx_core::units::{Attenuation, Density, Kelvin, Meters, Ratio, Vec3};

/// Integral of `spectral_radiance` from 10 nm to 100 um: a midpoint rule in
/// `ln(lambda)` with 200000 points, `d lambda = lambda d(ln lambda)`.
fn integrate(t: Kelvin) -> f64 {
    let (a, b) = (10e-9f64.ln(), 100e-6f64.ln());
    let n = 200_000;
    let h = (b - a) / f64::from(n);
    let mut sum = 0.0;
    for i in 0..n {
        let l = (a + (f64::from(i) + 0.5) * h).exp();
        sum += spectral_radiance(l, t) * l * h;
    }
    sum
}

#[test]
fn spectral_integral_matches_total() {
    for t in [300.0, 5800.0, 30000.0] {
        let t = Kelvin::new(t);
        let got = integrate(t);
        let want = total_radiance(t);
        let rel = (got - want).abs() / want;
        assert!(rel < 0.005, "{} K: relative error {rel}", t.value());
    }
}

#[test]
fn band_shape() {
    let b = band_radiance(Kelvin::new(5800.0));
    assert!(b[2] > b[0], "{b:?}");
    assert!(b[2] / b[0] < 1.5, "{b:?}");
    assert!((b[2] - b[0]).abs() < 0.2 * b[0].max(b[2]), "{b:?}");
    let c = band_radiance(Kelvin::new(3000.0));
    assert!(c[0] > 4.0 * c[2], "{c:?}");
}

#[test]
fn white_albedo_emits_nothing() {
    for t in [0.0, 300.0, 5800.0, 30000.0] {
        let e = emitted_band_radiance(Kelvin::new(t), [Ratio::new(1.0); 3]);
        assert_eq!(e, [0.0; 3]);
    }
}

fn hot_or_cold(hot: bool) -> Sample {
    Sample {
        density: Density::new(1.0e-3),
        state: State::Plasma,
        temperature: Kelvin::new(if hot { 8000.0 } else { 50.0 }),
        albedo: [Ratio::new(0.0); 3],
        roughness: Ratio::new(0.0),
        attenuation: Attenuation::new(0.1),
    }
}

#[test]
fn summarize_one_hot_sample_and_cold_section() {
    let key = CellKey::new(2, 1, 1, 0, 1).unwrap();
    let origin = Vec3::new(0.0, -100.0, 0.0);
    let edge = Meters::new(100.0);
    let section = |f: fn(u32, u32, u32) -> bool| {
        Section::new(
            key,
            origin,
            edge,
            5,
            Samples::from_fn(5, |x, y, z| hot_or_cold(f(x, y, z))),
        )
        .unwrap()
    };

    let one = section(|x, y, z| (x, y, z) == (4, 0, 2));
    let e = summarize(&one, Kelvin::new(1000.0)).unwrap();
    // Sub-cube edge 20 m: center of (4, 0, 2) is origin + (90, 10, 50).
    assert_eq!(e.position, Vec3::new(90.0, -90.0, 50.0));
    assert_eq!(e.radius, Meters::new(0.0));
    let b = band_radiance(Kelvin::new(8000.0));
    for (got, bi) in e.band_power.iter().zip(b) {
        let want = bi * PI * 6.0 * 400.0;
        assert!((got - want).abs() <= 1e-12 * want);
    }

    let cold = section(|_, _, _| false);
    assert_eq!(summarize(&cold, Kelvin::new(1000.0)), None);
}

#[test]
fn transmittance_limits() {
    let rho = Density::new(4.0);
    let a = Attenuation::new(0.125);
    assert_eq!(transmittance(rho, a, Meters::new(0.0)), 1.0);
    // k = 0.5 per meter, so a 2 m path has k * path = 1.
    let t = transmittance(rho, a, Meters::new(2.0));
    assert!((t - (-1.0f64).exp()).abs() < 1e-12, "{t}");
}
