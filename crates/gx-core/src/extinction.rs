//! Extinction of light passing through volumetric matter.
//!
//! Implements the extinction derived quantity of `matter-format.md` section
//! 3.3: extinction per meter is `density * attenuation`, and light crossing a
//! path of length `L` through uniform matter keeps the fraction
//! `exp(-k L)` (Beer-Lambert). [`optical_depth_along`] integrates `k` along a
//! segment through one section's grid, which the renderer uses when it ray
//! marches the field (`space-model.md` section 2). Run it on a composited
//! section (`space-model.md` section 8, `matter-format.md` section 3.5) to
//! see every layer's matter.
//!
//! # Determinism
//!
//! The exponential is a written-out routine using only correctly rounded
//! operations, and [`optical_depth_along`] samples a caller-fixed number of
//! points in a fixed order, so results are bitwise identical on every
//! platform.

use crate::detmath;
use crate::matter::Section;
use crate::units::{Attenuation, Density, Meters, Vec3};

/// Extinction coefficient in 1/m: `density * attenuation`.
pub fn extinction_coefficient(density: Density, attenuation: Attenuation) -> f64 {
    density.value() * attenuation.value()
}

/// Fraction of light transmitted through `path` of uniform matter:
/// `exp(-k * path)` with `k` from [`extinction_coefficient`]. 1 for a zero
/// path, between 0 and 1 for non-negative inputs.
pub fn transmittance(density: Density, attenuation: Attenuation, path: Meters) -> f64 {
    detmath::exp(-(extinction_coefficient(density, attenuation) * path.value()))
}

/// Optical depth (dimensionless) of the segment from `start` to `end`, both
/// in meters in the section's frame coordinates, through the section's
/// grid.
///
/// The segment is split into `steps` equal parts. At the midpoint of part
/// `i`, `start + (end - start) * ((i + 0.5) / steps)`, the sample whose
/// sub-cube contains the point is looked up (nearest sample), and its
/// extinction coefficient times the part's length is added, for `i` from 0
/// up in order. A midpoint outside the cell `[origin, origin + edge]` adds
/// nothing, since matter outside the cell belongs to other sections; a
/// midpoint on the cell's far faces is clamped to the last sample.
///
/// Returns 0 for an empty section, zero `steps`, or a zero-length segment.
/// The caller fixes `steps`; the result is a deterministic function of the
/// inputs.
pub fn optical_depth_along(section: &Section, start: Vec3, end: Vec3, steps: u32) -> f64 {
    let Some(samples) = section.samples() else {
        return 0.0;
    };
    if steps == 0 {
        return 0.0;
    }
    let n = usize::from(section.resolution());
    let edge = section.edge().value();
    let step = edge / n as f64;
    let origin = section.origin();
    let d = end - start;
    let ds = d.length() / f64::from(steps);
    // Sub-cube index along one axis, or `None` outside the cell.
    let index = |p: f64, o: f64| {
        let rel = p - o;
        if !(0.0..=edge).contains(&rel) {
            return None;
        }
        Some(((rel / step) as usize).min(n - 1))
    };
    let mut depth = 0.0;
    for i in 0..steps {
        let t = (f64::from(i) + 0.5) / f64::from(steps);
        let p = start + d.scale(t);
        let (Some(x), Some(y), Some(z)) = (
            index(p.x, origin.x),
            index(p.y, origin.y),
            index(p.z, origin.z),
        ) else {
            continue;
        };
        let s = x + n * (y + n * z);
        let k = samples
            .get(s)
            .map_or(0.0, |s| extinction_coefficient(s.density, s.attenuation));
        depth += k * ds;
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::CellKey;
    use crate::matter::{Sample, Samples, State};
    use crate::units::{Kelvin, Ratio};

    fn gas(density: f64, attenuation: f64) -> Sample {
        if density == 0.0 {
            return Sample::VACUUM;
        }
        Sample {
            density: Density::new(density),
            state: State::Gas,
            temperature: Kelvin::new(200.0),
            albedo: [Ratio::new(0.5); 3],
            roughness: Ratio::new(0.0),
            attenuation: Attenuation::new(attenuation),
        }
    }

    fn section(res: u8, f: impl FnMut(u32, u32, u32) -> Sample) -> Section {
        let key = CellKey::new(1, 0, 0, 0, 0).unwrap();
        Section::new(
            key,
            Vec3::new(0.0, 0.0, 0.0),
            Meters::new(8.0),
            res,
            Samples::from_fn(res, f),
        )
        .unwrap()
    }

    #[test]
    fn coefficient_and_transmittance() {
        let (rho, a) = (Density::new(2.0), Attenuation::new(0.25));
        assert_eq!(extinction_coefficient(rho, a), 0.5);
        assert_eq!(transmittance(rho, a, Meters::new(0.0)), 1.0);
        let e = transmittance(rho, a, Meters::new(2.0));
        assert!((e - (-1.0f64).exp()).abs() < 1e-12);
        let v = transmittance(rho, a, Meters::new(7.0));
        assert!(v > 0.0 && v < e);
    }

    #[test]
    fn uniform_section_matches_transmittance() {
        let s = section(4, |_, _, _| gas(2.0, 0.25));
        // A diagonal path fully inside the cell: depth is k * length.
        let a = Vec3::new(1.0, 1.0, 1.0);
        let b = Vec3::new(7.0, 5.0, 3.0);
        let len = (b - a).length();
        let tau = optical_depth_along(&s, a, b, 100);
        assert!((tau - 0.5 * len).abs() < 1e-12 * len);
        let t = transmittance(Density::new(2.0), Attenuation::new(0.25), Meters::new(len));
        assert!(((-tau).exp() - t).abs() < 1e-12);
    }

    #[test]
    fn outside_the_cell_adds_nothing() {
        let s = section(2, |_, _, _| gas(1.0, 1.0));
        // Half the segment lies outside the cell on the x axis.
        let tau = optical_depth_along(&s, Vec3::new(-8.0, 1.0, 1.0), Vec3::new(8.0, 1.0, 1.0), 64);
        assert!((tau - 8.0).abs() < 1e-12);
        let empty = Section::empty(s.key(), s.origin(), s.edge()).unwrap();
        assert_eq!(
            optical_depth_along(&empty, Vec3::zero(), Vec3::new(8.0, 8.0, 8.0), 8),
            0.0
        );
        assert_eq!(
            optical_depth_along(&s, Vec3::zero(), Vec3::new(8.0, 8.0, 8.0), 0),
            0.0
        );
    }

    #[test]
    fn nearest_sample_lookup() {
        // Only the x = 1 half holds matter.
        let s = section(2, |x, _, _| gas(f64::from(x) * 3.0, 0.5));
        let tau = optical_depth_along(&s, Vec3::new(0.0, 2.0, 6.0), Vec3::new(8.0, 2.0, 6.0), 10);
        assert!((tau - 1.5 * 4.0).abs() < 1e-12);
        // Deterministic: identical on a second call.
        let again = optical_depth_along(&s, Vec3::new(0.0, 2.0, 6.0), Vec3::new(8.0, 2.0, 6.0), 10);
        assert_eq!(tau.to_bits(), again.to_bits());
    }
}
