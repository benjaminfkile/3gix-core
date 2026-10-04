//! Newtonian gravity from point masses and from coarse density grids.
//!
//! Implements the gravity law of `space-model.md` section 1 (rule 2) and the
//! gravity sources of section 12: the frames of the registry, each a point
//! mass at its origin with its authoritative `mass` (`matter-format.md`
//! section 5.2), and optionally a frame's coarsest density grid. Gravity acts
//! between all frames regardless of the frame tree (`space-model.md` section
//! 5).
//!
//! There is no softening. A source at exactly the query point is skipped, so
//! a frame never pulls on itself. Sums run in a fixed order (slice order, or
//! sample index order) with plain `f64` operators and `sqrt`, so results are
//! bitwise identical on every platform.

use crate::frames::FrameSystem;
use crate::matter::{Section, State};
use crate::units::{Kilograms, Vec3};

/// The Newtonian constant of gravitation, m^3 kg^-1 s^-2 (CODATA 2018).
pub const G: f64 = 6.674_30e-11;

/// The acceleration at `point`, in m/s^2, from point masses at the given
/// positions, summed in slice order. A source at exactly `point` is skipped.
///
/// Positions in `sources` and `point` must share one coordinate system; the
/// result is in its axes.
pub fn acceleration_at(point: Vec3, sources: &[(Vec3, Kilograms)]) -> Vec3 {
    let mut a = Vec3::zero();
    for &(p, m) in sources {
        a = a + pull(point, p, m.value());
    }
    a
}

/// The gravitational potential at `point`, in J/kg, from point masses at the
/// given positions, summed in slice order: `-G * sum(m / r)`. Negative, or
/// zero with no sources. A source at exactly `point` is skipped.
pub fn potential_at(point: Vec3, sources: &[(Vec3, Kilograms)]) -> f64 {
    let mut phi = 0.0;
    for &(p, m) in sources {
        let r = (p - point).length();
        if r == 0.0 {
            continue;
        }
        phi -= G * m.value() / r;
    }
    phi
}

/// Every frame with a mass above zero as a point source: its root position
/// and its mass, in ascending `frame_id` order.
pub fn sources_from_system(system: &FrameSystem) -> Vec<(Vec3, Kilograms)> {
    system
        .tree()
        .frames()
        .iter()
        .filter(|f| f.mass.value() > 0.0)
        .map(|f| (system.root_position(f.frame_id), f.mass))
        .collect()
}

/// The acceleration at `point_in_frame`, in m/s^2 and in the section's frame
/// axes, from a matter section treated as point masses: every non-vacuum
/// sample is a mass of `density * (edge / n)^3` at the center of its
/// sub-cube. Samples are summed in index order. An empty section pulls with
/// zero.
///
/// This is for coarse grids only. A fine grid has too many samples to sum
/// per query, and close to a grid the point mass approximation breaks down at
/// the scale of one sub-cube. Which grid to use, and whether to use one at
/// all instead of the frame's point mass, is the renderer's decision
/// (`space-model.md` section 12: fine matter is rendered, not weighed
/// individually).
pub fn acceleration_from_section(point_in_frame: Vec3, section: &Section) -> Vec3 {
    let Some(samples) = section.samples() else {
        return Vec3::zero();
    };
    let n = usize::from(section.resolution());
    let step = section.edge().value() / n as f64;
    let volume = section.sample_volume();
    let origin = section.origin();
    let mut a = Vec3::zero();
    for i in 0..samples.len() {
        if samples.state(i) == State::Vacuum {
            continue;
        }
        let (x, y, z) = (i % n, (i / n) % n, i / (n * n));
        let center = origin
            + Vec3::new(
                (x as f64 + 0.5) * step,
                (y as f64 + 0.5) * step,
                (z as f64 + 0.5) * step,
            );
        let m = samples.density(i) * volume;
        a = a + pull(point_in_frame, center, m.value());
    }
    a
}

/// The pull at `point` of a mass `m` at `source`, or zero if they coincide.
fn pull(point: Vec3, source: Vec3, m: f64) -> Vec3 {
    let d = source - point;
    let r2 = d.length_squared();
    if r2 == 0.0 {
        return Vec3::zero();
    }
    let r = r2.sqrt();
    d.scale(G * m / (r2 * r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::CellKey;
    use crate::matter::{Sample, Samples};
    use crate::units::{Attenuation, Density, Kelvin, Meters, Ratio};

    #[test]
    fn single_source() {
        let src = [(Vec3::new(10.0, 0.0, 0.0), Kilograms::new(1.0e12))];
        let a = acceleration_at(Vec3::zero(), &src);
        assert_eq!(a, Vec3::new(G * 1.0e12 / 100.0, 0.0, 0.0));
        assert_eq!(potential_at(Vec3::zero(), &src), -G * 1.0e12 / 10.0);
    }

    #[test]
    fn coincident_source_is_skipped() {
        let src = [
            (Vec3::zero(), Kilograms::new(5.0)),
            (Vec3::new(0.0, 2.0, 0.0), Kilograms::new(4.0)),
        ];
        assert_eq!(
            acceleration_at(Vec3::zero(), &src),
            Vec3::new(0.0, G * 4.0 / 4.0, 0.0)
        );
        assert_eq!(potential_at(Vec3::zero(), &src), -G * 4.0 / 2.0);
        assert_eq!(acceleration_at(Vec3::zero(), &[]), Vec3::zero());
        assert_eq!(potential_at(Vec3::zero(), &[]), 0.0);
    }

    #[test]
    fn symmetric_sources_cancel() {
        let m = Kilograms::new(3.0e20);
        let src = [
            (Vec3::new(1.0e3, 0.0, 0.0), m),
            (Vec3::new(-1.0e3, 0.0, 0.0), m),
        ];
        assert_eq!(acceleration_at(Vec3::zero(), &src), Vec3::zero());
    }

    fn solid(density: f64) -> Sample {
        Sample {
            density: Density::new(density),
            state: if density > 0.0 {
                State::Solid
            } else {
                State::Vacuum
            },
            temperature: Kelvin::new(0.0),
            albedo: [Ratio::new(0.0); 3],
            roughness: Ratio::new(0.0),
            attenuation: Attenuation::new(0.0),
        }
    }

    #[test]
    fn section_as_point_masses() {
        let key = CellKey::new(1, 0, 0, 0, 0).unwrap();
        let edge = 8.0;
        let origin = Vec3::new(-4.0, -4.0, -4.0);
        // Only sample (1, 0, 0) of a 2 x 2 x 2 grid holds matter; its center
        // is (2, -2, -2) and its mass is 1000 * 4^3.
        let samples = Samples::from_fn(2, |x, y, z| {
            solid(if (x, y, z) == (1, 0, 0) { 1000.0 } else { 0.0 })
        });
        let section = Section::new(key, origin, Meters::new(edge), 2, samples).unwrap();
        let p = Vec3::new(10.0, 3.0, -1.0);
        let expect = acceleration_at(p, &[(Vec3::new(2.0, -2.0, -2.0), Kilograms::new(64_000.0))]);
        assert_eq!(acceleration_from_section(p, &section), expect);

        let empty = Section::empty(key, origin, Meters::new(edge)).unwrap();
        assert_eq!(acceleration_from_section(p, &empty), Vec3::zero());
    }

    #[test]
    fn uniform_grid_far_field_matches_total_mass() {
        let key = CellKey::new(1, 0, 0, 0, 0).unwrap();
        let section = Section::new(
            key,
            Vec3::new(-0.5, -0.5, -0.5),
            Meters::new(1.0),
            4,
            Samples::filled(4, solid(2000.0)),
        )
        .unwrap();
        let p = Vec3::new(0.0, 0.0, 1.0e4);
        let a = acceleration_from_section(p, &section);
        let point = acceleration_at(p, &[(Vec3::zero(), Kilograms::new(2000.0))]);
        assert!((a - point).length() < 1e-9 * point.length());
    }
}
