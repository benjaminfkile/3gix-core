//! Emitter summaries: the light that escapes a section's matter, reduced to
//! one emitter.
//!
//! Implements the emission derived quantity of `matter-format.md` section 3.3
//! (blackbody radiance from `temperature`, scaled by `1 - albedo` per band)
//! and extinction (`density * attenuation`) for the renderer's step "derives
//! emission from temperature" in `space-model.md` section 2. Run on the
//! depth-0 section of a frame, which holds all of the frame's matter, it
//! gives the light that frame sends into space. It looks only at state,
//! density, temperature, albedo, attenuation, and position; it never knows
//! what the matter is.
//!
//! # Radiative transfer
//!
//! Light leaves the section only where the matter lets it out:
//!
//! - **Opaque matter.** Solid and fluid samples are surfaces (`matter-format.md`
//!   section 3.3: attenuation 0 means a hard surface). Light from inside
//!   them never escapes; an exposed one radiates from its surface.
//! - **Volumetric matter.** A gas or plasma sample of optical thickness
//!   `tau_s = density * attenuation * s` (sub-cube edge `s`) emits
//!   `B (1 - e^-tau_s)` along a ray through it (Kirchhoff's law: it emits
//!   what it absorbs) and transmits `e^-tau_s` of what comes from behind.
//! - **Vacuum** is transparent and emits nothing.
//!
//! The emergent radiance of a ray is the formal solution of the transfer
//! equation along it, `I = sum_i B_i (1 - e^-tau_i) e^-tau_out,i`, with
//! `tau_out,i` the optical depth between sample `i` and the section's
//! boundary; an opaque sample adds `B_i e^-tau_out,i` and hides everything
//! behind it. `B_i` is `emitted_band_radiance(temperature, albedo)`.
//!
//! The power is the radiance integrated over the boundary and over all
//! directions. The direction integral is a six point quadrature: rays
//! along `+x, -x, +y, -y, +z, -z`, one through every column of samples,
//! each standing for a solid angle of `4 pi / 6` and a cross section of
//! `s^2`, so `P_band = (2 pi / 3) s^2 sum over rays of I_band`. This is
//! exact for an optically thin cloud (`4 pi j V`) and, averaged over
//! orientations, for an opaque body of any convex shape (Cauchy's
//! projection formula: the six axis projections of a convex body sum on
//! average to 1.5 times its surface, so `(2 pi / 3)` times the projected
//! area is `pi` times the surface). A ray grazing a gas layer crosses it
//! at its real slant, so limb darkening is part of the result.
//!
//! Only samples at or above a minimum temperature count as emitters (a
//! cutoff for the renderer's lights; cold matter still absorbs).
//!
//! # Determinism
//!
//! Every sum runs in `f64` in a fixed order (directions in the order
//! above, columns and samples in index order) with plain operators and
//! [`crate::detmath::exp`].

use core::f64::consts::PI;

use crate::detmath::{exp, expm1};
use crate::matter::{Section, State};
use crate::radiance::emitted_band_radiance;
use crate::units::{Kelvin, Meters, Vec3};

/// The light escaping one section's matter, reduced to one emitter.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Emitter {
    /// Centroid of the contributing sample centers weighted by the power
    /// that escapes from each, in meters, in the section's frame
    /// coordinates.
    pub position: Vec3,
    /// Power escaping the section in each band of
    /// [`crate::radiance::BAND_EDGES`], in watts, band 0 (long wavelength)
    /// first.
    pub band_power: [f64; 3],
    /// Root mean square distance of the contributing sample centers from
    /// `position`, unweighted.
    pub radius: Meters,
}

/// Summarizes the light that escapes `section` from its samples at or
/// above `min_temperature` (module docs).
///
/// - `band_power` is the escaping power per band.
/// - A sample contributes when it is at or above `min_temperature` and
///   some of its light escapes. `position` is the centroid of the
///   contributing sub-cube centers weighted by their escaping power over
///   the three bands; if that is 0 for all of them (albedo 1, or 0 K) it
///   is their unweighted centroid.
/// - `radius` is the root mean square distance of their centers from
///   `position`.
///
/// Returns `None` for an empty section or when no sample contributes.
/// Offsets are summed relative to the first contributing center, so a
/// single sample gives exactly its center and a radius of exactly 0.
pub fn summarize(section: &Section, min_temperature: Kelvin) -> Option<Emitter> {
    let samples = section.samples()?;
    let n = usize::from(section.resolution());
    let step = section.edge().value() / n as f64;
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

    // Per sample: opaque, optical thickness of its sub-cube, and whether it
    // emits. Written so a NaN threshold admits nothing.
    let opaque: Vec<bool> = samples
        .iter()
        .map(|s| matches!(s.state, State::Solid | State::Fluid))
        .collect();
    let tau: Vec<f64> = samples
        .iter()
        .map(|s| match s.state {
            State::Gas | State::Plasma => s.density.value() * s.attenuation.value() * step,
            _ => 0.0,
        })
        .collect();
    let hot: Vec<bool> = samples
        .iter()
        .map(|s| {
            s.state != State::Vacuum && s.radiating_temperature.value() >= min_temperature.value()
        })
        .collect();

    // Escape weight per sample: the sum over the six ray directions of the
    // fraction of its radiance that leaves the section along the ray.
    let mut escape = vec![0.0f64; samples.len()];
    let stride = [1usize, n, n * n];
    for axis in 0..3 {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        for forward in [true, false] {
            for v in 0..n {
                for u in 0..n {
                    let base = u * stride[a] + v * stride[b];
                    // March from the exit face inward.
                    let mut depth = 0.0;
                    for k in 0..n {
                        let along = if forward { n - 1 - k } else { k };
                        let i = base + along * stride[axis];
                        let through = exp(-depth);
                        if through == 0.0 {
                            break;
                        }
                        if opaque[i] {
                            if hot[i] {
                                escape[i] += through;
                            }
                            break;
                        }
                        if hot[i] && tau[i] > 0.0 {
                            escape[i] += -expm1(-tau[i]) * through;
                        }
                        depth += tau[i];
                    }
                }
            }
        }
    }

    // Contributing samples, in index order, with their center and power.
    let area = 2.0 * PI / 3.0 * step * step;
    let mut hits: Vec<(Vec3, [f64; 3])> = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        if !hot[i] || escape[i] <= 0.0 {
            continue;
        }
        let l = emitted_band_radiance(s.radiating_temperature, s.albedo);
        let w = area * escape[i];
        hits.push((center(i), [l[0] * w, l[1] * w, l[2] * w]));
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

    fn sample(state: State, density: f64, temperature: f64, albedo: f64, att: f64) -> Sample {
        if density == 0.0 {
            return Sample::VACUUM;
        }
        Sample {
            density: Density::new(density),
            state,
            temperature: Kelvin::new(temperature),
            albedo: [Ratio::new(albedo); 3],
            roughness: Ratio::new(0.0),
            attenuation: Attenuation::new(att),
            radiating_temperature: Kelvin::new(temperature),
        }
    }

    fn solid(temperature: f64, albedo: f64) -> Sample {
        sample(State::Solid, 1.0, temperature, albedo, 0.0)
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

    fn close(got: f64, want: f64, rel: f64) {
        assert!((got - want).abs() <= rel * want.abs(), "{got} vs {want}");
    }

    #[test]
    fn one_exposed_opaque_sample() {
        let s = section(4, |x, y, z| {
            if (x, y, z) == (2, 1, 3) {
                solid(6000.0, 0.25)
            } else {
                Sample::VACUUM
            }
        });
        let e = summarize(&s, Kelvin::new(1000.0)).unwrap();
        // Sub-cube edge 1, so the center of (2, 1, 3) is (0.5, -0.5, 1.5).
        assert_eq!(e.position, Vec3::new(0.5, -0.5, 1.5));
        assert_eq!(e.radius, Meters::new(0.0));
        // Six rays, each 2 pi / 3 s^2: 4 pi s^2 B.
        let b = band_radiance(Kelvin::new(6000.0));
        for (got, bi) in e.band_power.iter().zip(b) {
            close(*got, bi * 0.75 * 4.0 * PI, 1e-12);
        }
    }

    #[test]
    fn buried_matter_does_not_shine() {
        // A hot sample inside a shell of cold solid matter: nothing escapes.
        let s = section(4, |x, y, z| {
            if (x, y, z) == (1, 1, 1) {
                solid(6000.0, 0.0)
            } else if x < 3 && y < 3 && z < 3 {
                solid(300.0, 0.0)
            } else {
                Sample::VACUUM
            }
        });
        assert_eq!(summarize(&s, Kelvin::new(1000.0)), None);
        // The same with the shell hot: only its 26 exposed samples shine.
        let s = section(4, |x, y, z| {
            if x < 3 && y < 3 && z < 3 {
                solid(6000.0, 0.0)
            } else {
                Sample::VACUUM
            }
        });
        let e = summarize(&s, Kelvin::new(1000.0)).unwrap();
        // A 3 x 3 x 3 cube: 9 rays per face, 54 rays in all.
        let b = band_radiance(Kelvin::new(6000.0));
        for (got, bi) in e.band_power.iter().zip(b) {
            close(*got, bi * 54.0 * 2.0 * PI / 3.0, 1e-12);
        }
        assert!((e.position - Vec3::new(-0.5, -0.5, -0.5)).length() < 1e-12);
    }

    #[test]
    fn thin_gas_emits_four_pi_j_v() {
        // Optically thin hot gas: power 4 pi kappa rho V B.
        let (rho, kappa) = (1e-6, 1e-3);
        let s = section(8, |_, _, _| sample(State::Gas, rho, 5000.0, 0.0, kappa));
        let e = summarize(&s, Kelvin::new(1000.0)).unwrap();
        let b = band_radiance(Kelvin::new(5000.0));
        for (got, bi) in e.band_power.iter().zip(b) {
            close(*got, 4.0 * PI * kappa * rho * 64.0 * bi, 1e-6);
        }
    }

    #[test]
    fn thick_gas_shines_from_its_surface() {
        // Optically thick hot gas filling the section radiates like an
        // opaque cube of the same size, and a cold transparent gas in front
        // changes nothing.
        let s = section(8, |_, _, _| sample(State::Plasma, 1.0, 5000.0, 0.0, 100.0));
        let e = summarize(&s, Kelvin::new(1000.0)).unwrap();
        let b = band_radiance(Kelvin::new(5000.0));
        for (got, bi) in e.band_power.iter().zip(b) {
            close(*got, 6.0 * 16.0 * 2.0 * PI / 3.0 * bi, 1e-12);
        }
        // Cold gas of optical thickness 1 per sample over the x = 7 slab
        // dims the +x rays by 1/e and leaves the other faces alone.
        let s = section(8, |x, _, _| {
            if x == 7 {
                sample(State::Gas, 1.0, 10.0, 0.0, 2.0)
            } else {
                sample(State::Plasma, 1.0, 5000.0, 0.0, 100.0)
            }
        });
        let dimmed = summarize(&s, Kelvin::new(1000.0)).unwrap();
        assert!(dimmed.band_power[2] < e.band_power[2]);
    }

    #[test]
    fn transparent_matter_emits_nothing() {
        let s = section(2, |_, _, _| sample(State::Gas, 5.0, 8000.0, 0.0, 0.0));
        assert_eq!(summarize(&s, Kelvin::new(1000.0)), None);
    }

    #[test]
    fn cold_section_is_none() {
        let s = section(2, |_, _, _| solid(300.0, 0.0));
        assert_eq!(summarize(&s, Kelvin::new(1000.0)), None);
        let key = CellKey::new(3, 0, 0, 0, 0).unwrap();
        let empty = Section::empty(key, Vec3::zero(), Meters::new(1.0)).unwrap();
        assert_eq!(summarize(&empty, Kelvin::new(0.0)), None);
    }

    #[test]
    fn vacuum_is_skipped_and_threshold_is_inclusive() {
        let s = section(2, |x, _, _| {
            if x == 1 {
                solid(2000.0, 0.0)
            } else {
                Sample::VACUUM
            }
        });
        let e = summarize(&s, Kelvin::new(2000.0)).unwrap();
        // Only x = 1 samples hold matter: centers at x = 1, y and z at +-1.
        assert_eq!(e.position, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(e.radius, Meters::new(2.0f64.sqrt()));
    }

    #[test]
    fn centroid_follows_power() {
        // Two exposed hot samples at x centers -1 and 1; the hotter one
        // pulls the centroid toward it by its share of the total power.
        let s = section(2, |x, y, z| match (x, y, z) {
            (0, 0, 0) => solid(3000.0, 0.0),
            (1, 0, 0) => solid(6000.0, 0.0),
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
                solid(5000.0, 1.0)
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
