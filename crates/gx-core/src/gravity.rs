//! Gravity: the first post-Newtonian equations of motion for every massive
//! frame, the extra force of an oblate frame, and Newtonian gravity from
//! point masses and coarse density grids.
//!
//! Implements `space-model.md` section 6.1. Every massive frame of the
//! registry is a point mass at its origin with its authoritative `mass`
//! (`matter-format.md` section 5.2). Gravity acts between all frames
//! regardless of the frame tree (`space-model.md` section 5). Frames with
//! zero mass feel gravity and exert none.
//!
//! # First post-Newtonian accelerations
//!
//! [`relativistic_corrections`] returns the difference between the
//! Einstein-Infeld-Hoffmann acceleration and the Newtonian one, in root
//! coordinates (harmonic, barycentric in the sense of the registry's root
//! axes). The equation is the one JPL's DE ephemerides integrate: Newhall,
//! Standish and Williams (1983), "DE 102", Astronomy and Astrophysics 125,
//! pages 150 to 167, equation 1, here with the parametrized
//! post-Newtonian parameters `beta = gamma = 1`:
//!
//! ```text
//! a_i = sum_{j != i} mu_j (r_j - r_i) / r_ij^3 * {
//!           1 - 4/c^2 sum_{k != i} mu_k / r_ik - 1/c^2 sum_{k != j} mu_k / r_jk
//!           + |v_i|^2 / c^2 + 2 |v_j|^2 / c^2 - 4/c^2 (v_i . v_j)
//!           - 3/(2 c^2) [ (r_i - r_j) . v_j / r_ij ]^2
//!           + 1/(2 c^2) (r_j - r_i) . a_j }
//!     + 1/c^2 sum_{j != i} mu_j / r_ij^3 [ (r_i - r_j) . (4 v_i - 3 v_j) ] (v_i - v_j)
//!     + 7/(2 c^2) sum_{j != i} mu_j a_j / r_ij
//! ```
//!
//! with `mu = G m`, `r_ij = |r_j - r_i|`, and `a_j` the Newtonian
//! point-mass acceleration of frame `j` (the equation is accurate to first
//! order in `1/c^2`, so the Newtonian `a_j` is the one that belongs in the
//! correction terms). The sums run over massive frames only, so a massless
//! frame contributes nothing and feels the full field. The Newtonian part is
//! the leading `1` of the first bracket; [`relativistic_corrections`]
//! returns everything else, so the integrator can keep the Newtonian part
//! symplectic and add the rest as a perturbation ([`crate::integrate`]).
//!
//! The figure terms of the DE equations (oblateness acting on the
//! translational motion) are [`oblateness_accelerations`]; the remaining DE
//! terms (small-body perturbations, tides, libration coupling) are out of
//! scope.
//!
//! # Oblateness
//!
//! A massive frame with `j2 > 0` and reference radius `R` has the potential
//! `-mu / r * (1 - j2 (R / r)^2 P2(sin phi))`, `P2(s) = (3 s^2 - 1) / 2`,
//! with `phi` the angle between `r` and the plane normal to its z axis (its
//! symmetry axis, taken from its current orientation). The extra
//! acceleration of a point at `r` from its origin is
//!
//! ```text
//! a = -3/2 j2 mu R^2 / |r|^4 * [ (1 - 5 s^2) r_hat + 2 s z_hat ],  s = r_hat . z_hat
//! ```
//!
//! and the oblate frame takes the equal and opposite force, so momentum is
//! conserved. Only the oblate frame's own field is expanded: the mutual
//! figure-figure terms between two oblate frames are left out (they are
//! smaller by another factor of `j2 (R / r)^2`).
//!
//! # Determinism
//!
//! There is no softening. A source at exactly the query point is skipped, so
//! a frame never pulls on itself. Sums run in a fixed order (slice order,
//! which callers keep in ascending `frame_id`, or sample index order) with
//! plain `f64` operators and `sqrt`, so results are bitwise identical on
//! every platform.

use crate::frames::FrameSystem;
use crate::matter::{Section, State};
use crate::units::{Kilograms, Vec3};

/// The Newtonian constant of gravitation, m^3 kg^-1 s^-2 (CODATA 2018).
pub const G: f64 = 6.674_30e-11;

/// The speed of light in vacuum, m/s (exact by definition of the meter).
pub const C: f64 = 299_792_458.0;

/// One frame as the gravity law sees it: everything in root coordinates.
///
/// Slices of bodies are kept in ascending `frame_id` order by
/// [`bodies_from_system`] and by the integrator, which fixes the order of
/// every sum.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Body {
    /// Position of the frame origin, meters, root coordinates.
    pub position: Vec3,
    /// Velocity of the frame origin, meters per second, root axes.
    pub velocity: Vec3,
    /// Gravitational mass, kilograms. Zero for a frame that exerts no
    /// gravity.
    pub mass: Kilograms,
    /// Unnormalized second zonal harmonic about `axis`; 0 for none.
    pub j2: f64,
    /// The radius `j2` is defined against, meters; 0 when `j2` is 0.
    pub reference_radius: f64,
    /// Unit symmetry axis (the frame's z axis) in root axes.
    pub axis: Vec3,
}

impl Body {
    /// `G m`, m^3/s^2.
    fn mu(&self) -> f64 {
        G * self.mass.value()
    }
}

/// Every frame of the system as a [`Body`] in root coordinates, in ascending
/// `frame_id` order.
pub fn bodies_from_system(system: &FrameSystem) -> Vec<Body> {
    system
        .tree()
        .frames()
        .iter()
        .map(|f| Body {
            position: system.root_position(f.frame_id),
            velocity: system.root_velocity(f.frame_id),
            mass: f.mass,
            j2: f.j2,
            reference_radius: f.reference_radius.value(),
            axis: system
                .root_orientation(f.frame_id)
                .rotate(Vec3::new(0.0, 0.0, 1.0)),
        })
        .collect()
}

/// The Newtonian point-mass acceleration of every body from every massive
/// body, m/s^2, in slice order. Coincident bodies do not pull on each other.
pub fn newtonian_accelerations(bodies: &[Body]) -> Vec<Vec3> {
    bodies
        .iter()
        .map(|b| {
            let mut a = Vec3::zero();
            for s in bodies {
                if s.mass.value() > 0.0 {
                    a = a + pull(b.position, s.position, s.mass.value());
                }
            }
            a
        })
        .collect()
}

/// The first post-Newtonian correction to the acceleration of every body,
/// m/s^2, in slice order: the Einstein-Infeld-Hoffmann acceleration of the
/// module docs minus the Newtonian point-mass acceleration.
///
/// Velocities enter, so the result is only meaningful with velocities and
/// positions taken at the same instant.
pub fn relativistic_corrections(bodies: &[Body]) -> Vec<Vec3> {
    let c2 = C * C;
    let newtonian = newtonian_accelerations(bodies);
    // phi[i] = sum over massive k != i of mu_k / r_ik.
    let phi: Vec<f64> = bodies
        .iter()
        .map(|b| {
            let mut sum = 0.0;
            for s in bodies {
                let r = (s.position - b.position).length();
                if s.mass.value() > 0.0 && r > 0.0 {
                    sum += s.mu() / r;
                }
            }
            sum
        })
        .collect();
    let mut out = Vec::with_capacity(bodies.len());
    for (i, bi) in bodies.iter().enumerate() {
        let vi = bi.velocity;
        let vi2 = vi.length_squared();
        let mut a = Vec3::zero();
        for (j, bj) in bodies.iter().enumerate() {
            let mu = bj.mu();
            if j == i || mu <= 0.0 {
                continue;
            }
            let d = bj.position - bi.position;
            let r2 = d.length_squared();
            if r2 == 0.0 {
                continue;
            }
            let r = r2.sqrt();
            let r3 = r2 * r;
            let vj = bj.velocity;
            let aj = newtonian[j];
            let radial_vj = d.dot(vj) / r;
            let bracket = -4.0 * phi[i] - phi[j] + vi2 + 2.0 * vj.length_squared()
                - 4.0 * vi.dot(vj)
                - 1.5 * radial_vj * radial_vj
                + 0.5 * d.dot(aj);
            a = a + d.scale(mu / r3 * bracket / c2);
            let w = -d.dot(vi.scale(4.0) - vj.scale(3.0));
            a = a + (vi - vj).scale(mu / r3 * w / c2);
            a = a + aj.scale(3.5 * mu / (r * c2));
        }
        out.push(a);
    }
    out
}

/// The extra acceleration of every body from the oblateness of every
/// massive body with `j2 > 0`, and the equal and opposite reaction on the
/// oblate body, m/s^2, in slice order (formula in the module docs).
pub fn oblateness_accelerations(bodies: &[Body]) -> Vec<Vec3> {
    let mut out = vec![Vec3::zero(); bodies.len()];
    for (j, oblate) in bodies.iter().enumerate() {
        let mu = oblate.mu();
        if oblate.j2 <= 0.0 || mu <= 0.0 {
            continue;
        }
        for (i, b) in bodies.iter().enumerate() {
            if i == j {
                continue;
            }
            let a = oblate_pull(b.position - oblate.position, oblate, mu);
            out[i] = out[i] + a;
            let m = b.mass.value();
            if m > 0.0 {
                out[j] = out[j] - a.scale(m / oblate.mass.value());
            }
        }
    }
    out
}

/// The extra acceleration at `r` from an oblate body's origin, or zero at
/// its origin.
fn oblate_pull(r: Vec3, oblate: &Body, mu: f64) -> Vec3 {
    let r2 = r.length_squared();
    if r2 == 0.0 {
        return Vec3::zero();
    }
    let rn = r2.sqrt();
    let rhat = r.scale(1.0 / rn);
    let s = rhat.dot(oblate.axis);
    let rr = oblate.reference_radius;
    let k = -1.5 * oblate.j2 * mu * rr * rr / (r2 * r2);
    (rhat.scale(1.0 - 5.0 * s * s) + oblate.axis.scale(2.0 * s)).scale(k)
}

/// The full acceleration of every body, m/s^2, in slice order: Newtonian
/// point masses, plus the first post-Newtonian corrections, plus
/// oblateness.
pub fn accelerations(bodies: &[Body]) -> Vec<Vec3> {
    let n = newtonian_accelerations(bodies);
    let r = relativistic_corrections(bodies);
    let o = oblateness_accelerations(bodies);
    (0..bodies.len()).map(|i| n[i] + r[i] + o[i]).collect()
}

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

    fn body(position: Vec3, velocity: Vec3, mass: f64) -> Body {
        Body {
            position,
            velocity,
            mass: Kilograms::new(mass),
            j2: 0.0,
            reference_radius: 0.0,
            axis: Vec3::new(0.0, 0.0, 1.0),
        }
    }

    #[test]
    fn test_particle_limit_of_the_relativistic_terms() {
        // A massless frame around a mass at rest: the correction is the
        // harmonic-gauge test-particle form
        // mu / (r^3 c^2) * [ (4 mu / r - v^2) r + 4 (r . v) v ].
        let m = 2.7e30;
        let mu = G * m;
        let r = Vec3::new(4.0e10, 3.0e10, 1.0e9);
        let v = Vec3::new(-2.0e4, 3.5e4, 1.0e3);
        let bodies = [body(Vec3::zero(), Vec3::zero(), m), body(r, v, 0.0)];
        let got = relativistic_corrections(&bodies);
        assert_eq!(got[0], Vec3::zero());
        let rl = r.length();
        let expect = (r.scale(4.0 * mu / rl - v.length_squared()) + v.scale(4.0 * r.dot(v)))
            .scale(mu / (rl * rl * rl * C * C));
        assert!((got[1] - expect).length() < 1e-12 * expect.length());
        let total = accelerations(&bodies);
        let newton = newtonian_accelerations(&bodies);
        assert_eq!(total[1], newton[1] + got[1] + Vec3::zero());
    }

    #[test]
    fn oblateness_force_and_reaction() {
        let m = 3.4e24;
        let mut oblate = body(Vec3::zero(), Vec3::zero(), m);
        oblate.j2 = 1.0e-3;
        oblate.reference_radius = 8.3e6;
        // In the xy plane the extra pull is inward and equals
        // 3/2 j2 mu R^2 / r^4; on the z axis it is outward with twice that.
        let r = 9.1e6;
        let k = 1.5 * 1.0e-3 * G * m * 8.3e6 * 8.3e6 / (r * r * r * r);
        let in_xy =
            oblateness_accelerations(&[oblate, body(Vec3::new(r, 0.0, 0.0), Vec3::zero(), 0.0)]);
        assert!((in_xy[1] - Vec3::new(-k, 0.0, 0.0)).length() < 1e-12 * k);
        assert_eq!(in_xy[0], Vec3::zero());
        let on_z =
            oblateness_accelerations(&[oblate, body(Vec3::new(0.0, 0.0, r), Vec3::zero(), 0.0)]);
        assert!((on_z[1] - Vec3::new(0.0, 0.0, 2.0 * k)).length() < 1e-12 * k);
        // A massive frame off the axis: total momentum change is zero.
        let other = body(Vec3::new(3.0e7, -1.0e7, 2.0e7), Vec3::zero(), 7.0e22);
        let a = oblateness_accelerations(&[oblate, other]);
        let p = a[0].scale(m) + a[1].scale(7.0e22);
        assert!(p.length() < 1e-12 * a[1].length() * 7.0e22);
    }

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
            radiating_temperature: Kelvin::new(0.0),
            occupancy: Ratio::new(if density > 0.0 { 1.0 } else { 0.0 }),
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
