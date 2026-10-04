//! Emitter summaries: hot matter in a section reduced to one emitter.
//!
//! Implements the emission derived quantity of `matter-format.md` section 3.3
//! (blackbody radiance from `temperature`, scaled by `1 - albedo` per band)
//! for the renderer's step "derives emission from temperature" in
//! `space-model.md` section 2. Run on a composited section (section 8 of
//! `space-model.md`, section 3.5 of `matter-format.md`) it gives the light
//! the renderer uses for every other cell. It looks only at temperature,
//! albedo, and position; it never knows what the matter is.
//!
//! # The v1 power approximation
//!
//! Each qualifying sample is treated as an opaque cube of edge `s = e / n`
//! radiating from all six faces into the outward hemisphere: its power in a
//! band is `emitted_band_radiance * pi * 6 * s^2` watts. This ignores that
//! interior faces of neighbouring hot samples face each other and do not
//! escape, so a solid block of hot samples overestimates its power. That is
//! a deliberate v1 approximation, kept because it is local to one sample and
//! so needs no knowledge of the grid around it.
//!
//! # Determinism
//!
//! Every sum runs in `f64` in sample index order with plain operators.

use core::f64::consts::PI;

use crate::matter::{Section, State};
use crate::radiance::emitted_band_radiance;
use crate::units::{Kelvin, Meters, Vec3};

/// The hot matter of one section reduced to one emitter.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Emitter {
    /// Emission-weighted centroid of the contributing sample centers, in
    /// meters, in the section's frame coordinates.
    pub position: Vec3,
    /// Power emitted in each band of [`crate::radiance::BAND_EDGES`], in
    /// watts, band 0 (long wavelength) first.
    pub band_power: [f64; 3],
    /// Root mean square distance of the contributing sample centers from
    /// `position`, unweighted.
    pub radius: Meters,
}

/// Summarizes the emission of every non-vacuum sample of `section` whose
/// temperature is at or above `min_temperature`.
///
/// - `band_power` is the sum over those samples of
///   `emitted_band_radiance(temperature, albedo) * pi * 6 * (e / n)^2` (see
///   the module docs for why six faces).
/// - `position` is the centroid of their sub-cube centers weighted by total
///   power (the sum of the three bands). If every contributing sample emits
///   nothing (albedo 1, or 0 K), it is the unweighted centroid.
/// - `radius` is the root mean square distance of their centers from
///   `position`.
///
/// Returns `None` for an empty section or when no sample qualifies. Offsets
/// are summed relative to the first contributing center, so a single sample
/// gives exactly its center and a radius of exactly 0.
pub fn summarize(section: &Section, min_temperature: Kelvin) -> Option<Emitter> {
    let samples = section.samples()?;
    let n = usize::from(section.resolution());
    let step = section.edge().value() / n as f64;
    let face_area = 6.0 * step * step;
    let origin = section.origin();
    let center = |i: usize| {
        let (x, y, z) = (i % n, (i / n) % n, i / (n * n));
        origin
            + Vec3::new(
                (x as f64 + 0.5) * step,
                (y as f64 + 0.5) * step,
                (z as f64 + 0.5) * step,
            )
    };

    // Contributing samples, in index order, with their center and power.
    let mut hits: Vec<(Vec3, [f64; 3])> = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        // Written so a NaN threshold admits nothing.
        let hot = s.temperature.value() >= min_temperature.value();
        if s.state == State::Vacuum || !hot {
            continue;
        }
        let l = emitted_band_radiance(s.temperature, s.albedo);
        let p = [
            l[0] * PI * face_area,
            l[1] * PI * face_area,
            l[2] * PI * face_area,
        ];
        hits.push((center(i), p));
    }
    let reference = hits.first()?.0;

    let mut band_power = [0.0; 3];
    let mut weight = 0.0;
    let mut weighted = Vec3::zero();
    let mut plain = Vec3::zero();
    for &(c, p) in &hits {
        let w = p[0] + p[1] + p[2];
        let d = c - reference;
        for b in 0..3 {
            band_power[b] += p[b];
        }
        weight += w;
        weighted = weighted + d.scale(w);
        plain = plain + d;
    }
    let count = hits.len() as f64;
    let offset = if weight > 0.0 {
        weighted.scale(1.0 / weight)
    } else {
        plain.scale(1.0 / count)
    };
    let position = reference + offset;

    let mut sum_sq = 0.0;
    for &(c, _) in &hits {
        sum_sq += ((c - reference) - offset).length_squared();
    }
    Some(Emitter {
        position,
        band_power,
        radius: Meters::new((sum_sq / count).sqrt()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::CellKey;
    use crate::matter::{Sample, Samples};
    use crate::radiance::band_radiance;
    use crate::units::{Attenuation, Density, Ratio};

    fn sample(density: f64, temperature: f64, albedo: f64) -> Sample {
        if density == 0.0 {
            return Sample::VACUUM;
        }
        Sample {
            density: Density::new(density),
            state: State::Gas,
            temperature: Kelvin::new(temperature),
            albedo: [Ratio::new(albedo); 3],
            roughness: Ratio::new(0.0),
            attenuation: Attenuation::new(0.0),
        }
    }

    fn section(res: u8, f: impl FnMut(u32, u32, u32) -> Sample) -> Section {
        let key = CellKey::new(3, 0, 0, 0, 0).unwrap();
        Section::new(
            key,
            Vec3::new(-2.0, -2.0, -2.0),
            Meters::new(4.0),
            res,
            Samples::from_fn(res, f),
        )
        .unwrap()
    }

    #[test]
    fn one_hot_sample() {
        let s = section(4, |x, y, z| {
            if (x, y, z) == (2, 1, 3) {
                sample(1.0, 6000.0, 0.25)
            } else {
                sample(1.0, 100.0, 0.0)
            }
        });
        let e = summarize(&s, Kelvin::new(1000.0)).unwrap();
        // Sub-cube edge 1, so the center of (2, 1, 3) is (0.5, -0.5, 1.5).
        assert_eq!(e.position, Vec3::new(0.5, -0.5, 1.5));
        assert_eq!(e.radius, Meters::new(0.0));
        let b = band_radiance(Kelvin::new(6000.0));
        for (got, bi) in e.band_power.iter().zip(b) {
            let want = bi * 0.75 * PI * 6.0;
            assert!((got - want).abs() <= 1e-12 * want);
        }
    }

    #[test]
    fn cold_section_is_none() {
        let s = section(2, |_, _, _| sample(5.0, 300.0, 0.0));
        assert_eq!(summarize(&s, Kelvin::new(1000.0)), None);
        let key = CellKey::new(3, 0, 0, 0, 0).unwrap();
        let empty = Section::empty(key, Vec3::zero(), Meters::new(1.0)).unwrap();
        assert_eq!(summarize(&empty, Kelvin::new(0.0)), None);
    }

    #[test]
    fn vacuum_is_skipped_and_threshold_is_inclusive() {
        let s = section(2, |x, _, _| sample(f64::from(x), 2000.0, 0.0));
        let e = summarize(&s, Kelvin::new(2000.0)).unwrap();
        // Only x = 1 samples hold matter: centers at x = 1, y and z at +-1.
        assert_eq!(e.position, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(e.radius, Meters::new(2.0f64.sqrt()));
    }

    #[test]
    fn centroid_follows_power() {
        // Two hot samples at x centers -1 and 1; the hotter one pulls the
        // centroid toward it by its share of the total power.
        let s = section(2, |x, y, z| match (x, y, z) {
            (0, 0, 0) => sample(1.0, 3000.0, 0.0),
            (1, 0, 0) => sample(1.0, 6000.0, 0.0),
            _ => Sample::VACUUM,
        });
        let e = summarize(&s, Kelvin::new(1.0)).unwrap();
        let w = |t| band_radiance(Kelvin::new(t)).iter().sum::<f64>();
        let (w0, w1) = (w(3000.0), w(6000.0));
        let want = -1.0 + 2.0 * w1 / (w0 + w1);
        assert!((e.position.x - want).abs() < 1e-12);
        assert_eq!((e.position.y, e.position.z), (-1.0, -1.0));
        assert!(e.position.x > 0.5);
    }

    #[test]
    fn zero_power_uses_plain_centroid() {
        let s = section(2, |_, y, z| {
            if y == 0 && z == 0 {
                sample(1.0, 5000.0, 1.0)
            } else {
                Sample::VACUUM
            }
        });
        let e = summarize(&s, Kelvin::new(0.0)).unwrap();
        assert_eq!(e.band_power, [0.0; 3]);
        assert_eq!(e.position, Vec3::new(0.0, -1.0, -1.0));
        assert_eq!(e.radius, Meters::new(1.0));
    }
}
