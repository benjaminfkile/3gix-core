//! The observer as a body: gravity, contact with friction, drag, and
//! buoyancy acting on a small rigid body with no rotation dynamics.
//!
//! Implements `space-model.md` section 6.3, walk mode. The observer is a
//! mass, a height, a frontal area, a drag coefficient, a density, and a
//! friction coefficient ([`ObserverParams`]), constants of the observer
//! that are never derived from any matter it meets. The laws acting on it:
//!
//! - gravity from every massive frame, Newtonian point mass plus the J2
//!   force of oblate frames ([`crate::gravity::newtonian_field_at`]; the
//!   relativistic terms are negligible at this mass),
//! - contact with solid matter: a solid sample is impenetrable, the normal
//!   force holds the observer on it, and friction moves it with the matter
//!   it stands on, so it rotates and orbits with that matter,
//! - drag in fluid, gas, and plasma matter, from the local density and the
//!   observer's velocity relative to that matter,
//! - buoyancy from the local density.
//!
//! A body of gas therefore swallows the observer until the surrounding
//! density supports it. Temperature and pressure do nothing to it. Fly
//! mode, where nothing acts on the observer, needs no law and is not here.
//! Nothing here draws; the renderer calls [`step_observer`] once per frame.
//!
//! # Coordinates
//!
//! An [`ObserverState`] is relative to one frame, the one passed to
//! [`step_observer`] (normally the frame whose matter is nearest): its
//! `position` is the offset of the observer from that frame's origin and
//! its `velocity` the rate of change of that offset, both in root axes,
//! exactly like a frame's own state ([`crate::frames`]). These axes do not
//! turn with the frame, so no fictitious forces appear; the frame's spin
//! enters only through the velocity of the matter the [`MatterProbe`]
//! reports. A point is handed to the probe in the frame's axes. The frame's
//! origin falls freely under gravity, so the observer feels the gravity of
//! every massive frame minus the acceleration of that origin
//! ([`local_gravity`]). [`ObserverState::rebase`] moves a state to another
//! frame.
//!
//! The observer is a point at its lowest point: the probe is sampled there
//! for contact, drag, and buoyancy. Its height sets the size of the contact
//! patch ([`ObserverParams::contact_patch`]); a renderer places the eye at
//! `position + up * height` with [`up_direction`].
//!
//! # One step
//!
//! [`step_observer`] advances the observer by `dt` in equal substeps no
//! longer than [`MAX_SUBSTEP`] (1/240 s), holding the frame system at its
//! current time except for the frame's own turning, which it carries
//! forward at the frame's angular velocity. The caller advances the frame
//! system by the same `dt`. One substep of length `h`:
//!
//! 1. Drag for `h/2`. Quadratic drag `0.5 rho |v_rel|^2 Cd A` against the
//!    velocity relative to the local matter, for fluid, gas, and plasma
//!    samples, applied with its exact solution for a fixed medium,
//!    `v_rel <- v_rel / (1 + k |v_rel| h/2)` with `k = rho Cd A / (2 m)`,
//!    which never reverses the motion however dense the medium.
//! 2. Gravity and buoyancy by velocity Verlet (kick `h/2`, drift `h`, kick
//!    `h/2`): the acceleration is `g (1 - rho_local / rho_observer)`, that
//!    is gravity `m g` plus buoyancy `-rho_local V g` with
//!    `V = mass / density`. Solid samples contribute no buoyancy: contact
//!    takes over. Exact for uniform gravity.
//! 3. Contact (below).
//! 4. Drag for `h/2` at the new position.
//!
//! # Contact
//!
//! The observer is in contact when its point is in a solid sample, or a
//! solid sample lies within [`CONTACT_SKIN`] below it (along gravity). The
//! contact normal is the direction in which density falls, estimated from
//! the probe in two stages:
//!
//! 1. the central-difference density gradient over growing steps, from
//!    `patch / 8` up by factors of 4, until it is non-zero, with the
//!    negative of gravity as the fallback, and
//! 2. because solid matter ends in a step of density, whose difference
//!    gradient only points along a few lattice directions, a refinement: the
//!    boundary is found by bisection along that first estimate at four
//!    points of a cross of half-width `patch` around the contact, and the
//!    normal of the plane through them is the gradient of the boundary.
//!
//! A point inside a solid is moved out along the normal to the boundary,
//! found by bisection to [`BOUNDARY_TOLERANCE`]. Then, with `v_rel` the
//! velocity relative to the solid matter:
//!
//! - the normal impulse `J_n = m max(0, -v_rel . n)` removes any motion
//!   into the solid. Over a substep this is the normal force `J_n / h`,
//!   which for an observer at rest on the matter equals the inward part of
//!   gravity,
//! - the walking force is `intent` with its normal part removed, limited to
//!   `friction * J_n / h`,
//! - friction, an impulse limited by what is left of `friction * J_n`,
//!   drives the tangential relative velocity to zero, except for the part
//!   along the walking force, which friction lets the walk build up.
//!
//! `intent` is a force in newtons in root axes. It is the controls' to
//! choose; a controller that wants a walking speed sets it from the
//! difference between that speed and the relative velocity of
//! [`Contact::ground_velocity`]. Out of contact it does nothing.
//!
//! # Determinism
//!
//! Only `+`, `-`, `*`, `/`, `sqrt`, and the written-out sine and cosine of
//! [`crate::rotation::advance_orientation`] are used, with a fixed count of
//! substeps, fixed bisection rules, and frames in ascending `frame_id`, so
//! given a deterministic probe a step is bitwise identical on every
//! platform.

use crate::frames::FrameSystem;
use crate::gravity::{newtonian_field_at, Body};
use crate::matter::State;
use crate::rotation::advance_orientation;
use crate::units::{Density, Kilograms, Meters, Quat, Seconds, SquareMeters, Vec3};

/// The longest substep of [`step_observer`], seconds.
pub const MAX_SUBSTEP: f64 = 1.0 / 240.0;

/// How far below the observer, along gravity, a solid sample still counts
/// as contact, meters.
pub const CONTACT_SKIN: f64 = 2.0e-3;

/// The bisection tolerance for a solid boundary, meters.
pub const BOUNDARY_TOLERANCE: f64 = 1.0e-6;

/// The most doublings of a bracket around a solid boundary.
const MAX_DOUBLINGS: u32 = 40;

/// The most halvings of a bracket around a solid boundary.
const MAX_HALVINGS: u32 = 64;

/// The density step tries of the gradient estimate.
const GRADIENT_TRIES: u32 = 6;

/// The constants of the observer's body, all SI.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ObserverParams {
    /// Mass, kilograms.
    pub mass: Kilograms,
    /// Height, meters: the eye height and the size of the contact patch.
    pub height: Meters,
    /// Frontal area for drag, square meters.
    pub frontal_area: SquareMeters,
    /// Drag coefficient, dimensionless.
    pub drag_coefficient: f64,
    /// Mean density, kilograms per cubic meter. Sets the volume
    /// `mass / density` that buoyancy acts on.
    pub density: Density,
    /// Friction coefficient against solid matter, dimensionless.
    pub friction: f64,
}

impl Default for ObserverParams {
    /// A body of everyday size: 80 kg, 1.75 m tall, 0.7 m^2 of frontal
    /// area, drag coefficient 1.0, mean density 985 kg/m^3, and friction
    /// coefficient 0.8.
    fn default() -> Self {
        ObserverParams {
            mass: Kilograms::new(80.0),
            height: Meters::new(1.75),
            frontal_area: SquareMeters::new(0.7),
            drag_coefficient: 1.0,
            density: Density::new(985.0),
            friction: 0.8,
        }
    }
}

impl ObserverParams {
    /// The volume buoyancy acts on, `mass / density`, cubic meters.
    pub fn volume(&self) -> f64 {
        self.mass.value() / self.density.value()
    }

    /// The half-width of the cross the contact normal is fitted over,
    /// `height / 8`, meters.
    pub fn contact_patch(&self) -> f64 {
        self.height.value() / 8.0
    }

    /// Panics unless every constant is finite and the mass, height,
    /// density, and contact patch are above zero and the rest at or above
    /// zero.
    fn check(&self) {
        let positive = [
            ("mass", self.mass.value()),
            ("height", self.height.value()),
            ("density", self.density.value()),
        ];
        for (what, v) in positive {
            assert!(
                v.is_finite() && v > 0.0,
                "{what} must be finite and above zero, got {v}"
            );
        }
        let non_negative = [
            ("frontal_area", self.frontal_area.value()),
            ("drag_coefficient", self.drag_coefficient),
            ("friction", self.friction),
        ];
        for (what, v) in non_negative {
            assert!(
                v.is_finite() && v >= 0.0,
                "{what} must be finite and at or above zero, got {v}"
            );
        }
    }
}

/// What the probe reports of the matter at one point.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Sample {
    /// Density, kilograms per cubic meter (0 for vacuum).
    pub density: Density,
    /// State of the matter.
    pub state: State,
    /// Velocity of the matter relative to the frame origin, meters per
    /// second, as seen from axes that do not turn, with its components in
    /// the frame's axes. For matter at rest in the frame this is
    /// `w x p`, with `w` the frame's angular velocity (about its own axes)
    /// and `p` the point.
    pub velocity: Vec3,
}

/// The matter around the observer, as the renderer has it loaded.
pub trait MatterProbe {
    /// The matter at a point in the frame (relative to its origin, in its
    /// axes), or `None` where nothing is known, which counts as vacuum. Must
    /// return the same answer for the same point every time.
    fn sample(&self, point_in_frame: Vec3) -> Option<Sample>;
}

/// The solid contact of the last substep of a step.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Contact {
    /// Unit normal of the solid surface, pointing out of it, root axes.
    pub normal: Vec3,
    /// The normal force over the substep, newtons.
    pub normal_force: f64,
    /// Velocity of the solid matter at the contact relative to the frame
    /// origin, meters per second, root axes.
    pub ground_velocity: Vec3,
}

/// The observer's state relative to one frame (module docs).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ObserverState {
    /// Offset of the observer's lowest point from the frame origin, meters,
    /// root axes.
    pub position: Vec3,
    /// Rate of change of `position`, meters per second, root axes.
    pub velocity: Vec3,
    /// The contact at the end of the last step, if any.
    pub contact: Option<Contact>,
}

impl ObserverState {
    /// A state at `position` with `velocity`, relative to some frame, with
    /// no contact yet.
    pub fn new(position: Vec3, velocity: Vec3) -> ObserverState {
        ObserverState {
            position,
            velocity,
            contact: None,
        }
    }

    /// Moves the state from frame `from` to frame `to`: the same point and
    /// velocity, now relative to `to`. The offset between the origins is
    /// taken with [`FrameSystem::relative`], so large offsets above the two
    /// frames' common ancestor never enter the arithmetic. The contact, in
    /// root axes, is kept with its ground velocity rebased too.
    ///
    /// # Panics
    ///
    /// Panics if either frame is not in the system.
    pub fn rebase(&mut self, from: u64, to: u64, system: &FrameSystem) {
        let offset = system.relative(Vec3::zero(), from, Vec3::zero(), to);
        let dv = system.root_velocity(from) - system.root_velocity(to);
        self.position = self.position + offset;
        self.velocity = self.velocity + dv;
        if let Some(c) = &mut self.contact {
            c.ground_velocity = c.ground_velocity + dv;
        }
    }

    /// The state as bytes, for comparing two runs bit for bit.
    /// Little-endian, no padding:
    ///
    /// ```text
    /// position         f64 x3
    /// velocity         f64 x3
    /// in contact       u8 (0 or 1)
    /// when in contact:
    ///   normal         f64 x3
    ///   normal_force   f64
    ///   ground_velocity f64 x3
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 + 1 + 56);
        let put = |v: Vec3, out: &mut Vec<u8>| {
            for x in [v.x, v.y, v.z] {
                out.extend_from_slice(&x.to_le_bytes());
            }
        };
        put(self.position, &mut out);
        put(self.velocity, &mut out);
        match &self.contact {
            None => out.push(0),
            Some(c) => {
                out.push(1);
                put(c.normal, &mut out);
                out.extend_from_slice(&c.normal_force.to_le_bytes());
                put(c.ground_velocity, &mut out);
            }
        }
        out
    }
}

/// Every frame as a [`Body`] with its position relative to `frame`'s
/// origin, root axes, in ascending `frame_id` order.
fn bodies_around(frame: u64, system: &FrameSystem) -> Vec<Body> {
    system
        .tree()
        .frames()
        .iter()
        .map(|f| Body {
            position: system.relative(Vec3::zero(), f.frame_id, Vec3::zero(), frame),
            velocity: Vec3::zero(),
            mass: f.mass,
            j2: f.j2,
            reference_radius: f.reference_radius.value(),
            axis: system
                .root_orientation(f.frame_id)
                .rotate(Vec3::new(0.0, 0.0, 1.0)),
        })
        .collect()
}

/// The gravity field around one frame, relative to its freely falling
/// origin.
struct Field {
    bodies: Vec<Body>,
    origin: Vec3,
}

impl Field {
    fn new(frame: u64, system: &FrameSystem) -> Field {
        let bodies = bodies_around(frame, system);
        // A body at exactly the origin, the frame itself, exerts nothing
        // there.
        let origin = newtonian_field_at(Vec3::zero(), &bodies);
        Field { bodies, origin }
    }

    fn at(&self, position: Vec3) -> Vec3 {
        newtonian_field_at(position, &self.bodies) - self.origin
    }
}

/// The gravity the observer feels at `position` (offset from `frame`'s
/// origin, root axes), m/s^2, root axes: the Newtonian point-mass and J2
/// field of every massive frame minus the acceleration the same field gives
/// the frame's origin.
///
/// # Panics
///
/// Panics if `frame` is not in the system.
pub fn local_gravity(position: Vec3, frame: u64, system: &FrameSystem) -> Vec3 {
    Field::new(frame, system).at(position)
}

/// The unit vector against [`local_gravity`] at `position`, root axes, or
/// `None` where gravity is exactly zero.
///
/// # Panics
///
/// Panics if `frame` is not in the system.
pub fn up_direction(position: Vec3, frame: u64, system: &FrameSystem) -> Option<Vec3> {
    (-local_gravity(position, frame, system)).normalized()
}

/// A probe answer with `None` read as vacuum at rest.
fn sample_at(probe: &dyn MatterProbe, p: Vec3) -> Sample {
    probe.sample(p).unwrap_or(Sample {
        density: Density::new(0.0),
        state: State::Vacuum,
        velocity: Vec3::zero(),
    })
}

fn is_solid(probe: &dyn MatterProbe, p: Vec3) -> bool {
    sample_at(probe, p).state == State::Solid
}

/// The density of a sample that drag and buoyancy act with: fluid, gas, and
/// plasma only.
fn medium_density(s: &Sample) -> f64 {
    match s.state {
        State::Fluid | State::Gas | State::Plasma => s.density.value(),
        State::Vacuum | State::Solid => 0.0,
    }
}

/// The outward normal of the solid at `inside` (a solid point), frame axes.
fn surface_normal(probe: &dyn MatterProbe, inside: Vec3, patch: f64, up: Vec3) -> Vec3 {
    let first = density_gradient_normal(probe, inside, patch).unwrap_or(up);
    boundary_plane_normal(probe, inside, first, patch).unwrap_or(first)
}

/// The unit vector against the central-difference density gradient at `p`,
/// over steps from `patch / 8` growing by factors of 4, or `None` if every
/// try gives zero.
fn density_gradient_normal(probe: &dyn MatterProbe, p: Vec3, patch: f64) -> Option<Vec3> {
    let rho = |q: Vec3| sample_at(probe, q).density.value();
    let mut d = patch / 8.0;
    for _ in 0..GRADIENT_TRIES {
        let gx = rho(p + Vec3::new(d, 0.0, 0.0)) - rho(p - Vec3::new(d, 0.0, 0.0));
        let gy = rho(p + Vec3::new(0.0, d, 0.0)) - rho(p - Vec3::new(0.0, d, 0.0));
        let gz = rho(p + Vec3::new(0.0, 0.0, d)) - rho(p - Vec3::new(0.0, 0.0, d));
        if let Some(n) = Vec3::new(-gx, -gy, -gz).normalized() {
            return Some(n);
        }
        d *= 4.0;
    }
    None
}

/// Two unit vectors that complete `n` (a unit vector) to a right-handed
/// basis `t1, t2, n`.
fn tangents(n: Vec3) -> (Vec3, Vec3) {
    let (ax, ay, az) = (n.x.abs(), n.y.abs(), n.z.abs());
    let e = if ax <= ay && ax <= az {
        Vec3::new(1.0, 0.0, 0.0)
    } else if ay <= az {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let t1 = e.cross(n).normalized().expect("e is not parallel to n");
    (t1, n.cross(t1))
}

/// The point where the line `base + s dir` leaves the solid, searched on
/// both sides of `base`: `s` with a solid point at most
/// [`BOUNDARY_TOLERANCE`] before it and a non-solid point at it. `None` if
/// no bracket is found within [`MAX_DOUBLINGS`] doublings of `reach`.
fn crossing(probe: &dyn MatterProbe, base: Vec3, dir: Vec3, reach: f64) -> Option<f64> {
    let mut r = reach;
    for _ in 0..MAX_DOUBLINGS {
        if is_solid(probe, base - dir.scale(r)) && !is_solid(probe, base + dir.scale(r)) {
            return Some(bisect(probe, base, dir, -r, r));
        }
        r *= 2.0;
    }
    None
}

/// Bisects `[lo, hi]` on the line `base + s dir`, solid at `lo` and not at
/// `hi`, to [`BOUNDARY_TOLERANCE`]; returns the non-solid end.
fn bisect(probe: &dyn MatterProbe, base: Vec3, dir: Vec3, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..MAX_HALVINGS {
        if hi - lo <= BOUNDARY_TOLERANCE {
            break;
        }
        let mid = 0.5 * (lo + hi);
        if is_solid(probe, base + dir.scale(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// The normal of the plane through the solid boundary at four points of a
/// cross of half-width `patch` around `p`, each found along `first`, or
/// `None` if a point is not found or the plane faces against `first`.
fn boundary_plane_normal(
    probe: &dyn MatterProbe,
    p: Vec3,
    first: Vec3,
    patch: f64,
) -> Option<Vec3> {
    let (t1, t2) = tangents(first);
    let at = |t: Vec3| -> Option<Vec3> {
        let base = p + t.scale(patch);
        crossing(probe, base, first, patch).map(|s| base + first.scale(s))
    };
    let a = at(t1)?;
    let b = at(-t1)?;
    let c = at(t2)?;
    let d = at(-t2)?;
    let n = (a - b).cross(c - d).normalized()?;
    (n.dot(first) > 0.0).then_some(n)
}

/// The distance along `dir` from the solid point `p` to where the solid
/// ends, to [`BOUNDARY_TOLERANCE`], on the non-solid side. `None` if the
/// solid does not end within [`MAX_DOUBLINGS`] doublings of `start`.
fn exit_distance(probe: &dyn MatterProbe, p: Vec3, dir: Vec3, start: f64) -> Option<f64> {
    let mut lo = 0.0;
    let mut hi = start;
    for _ in 0..MAX_DOUBLINGS {
        if !is_solid(probe, p + dir.scale(hi)) {
            return Some(bisect(probe, p, dir, lo, hi));
        }
        lo = hi;
        hi *= 2.0;
    }
    None
}

/// Applies drag for `h` seconds against the medium at the observer, with
/// the medium velocity in root axes.
fn drag(v: Vec3, medium: &Sample, q: Quat, h: f64, params: &ObserverParams) -> Vec3 {
    let rho = medium_density(medium);
    if rho <= 0.0 {
        return v;
    }
    let k = 0.5 * rho * params.drag_coefficient * params.frontal_area.value() / params.mass.value();
    let vm = q.rotate(medium.velocity);
    let rel = v - vm;
    vm + rel.scale(1.0 / (1.0 + k * rel.length() * h))
}

/// Advances the observer by `dt` seconds near `frame`, under the laws of
/// the module docs, with the matter `probe` reports and the walking force
/// `intent` (newtons, root axes). The system is read at its current time
/// and is not advanced; the caller advances it by the same `dt`.
///
/// # Panics
///
/// Panics if `dt` is negative or not finite, if `intent` or the state is not
/// finite, if a constant of `params` is out of range, or if `frame` is not
/// in the system.
pub fn step_observer(
    state: &mut ObserverState,
    frame: u64,
    system: &FrameSystem,
    probe: &dyn MatterProbe,
    intent: Vec3,
    dt: Seconds,
    params: &ObserverParams,
) {
    params.check();
    let total = dt.value();
    assert!(
        total.is_finite() && total >= 0.0,
        "dt must be finite and at or above zero, got {total}"
    );
    for v in [state.position, state.velocity, intent] {
        assert!(
            v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
            "observer state and intent must be finite, got {v:?}"
        );
    }
    if total == 0.0 {
        return;
    }
    let field = Field::new(frame, system);
    let q0 = system.root_orientation(frame);
    let w = system
        .state(frame)
        .expect("root_orientation checked the frame")
        .angular_velocity;
    let count = (total / MAX_SUBSTEP).ceil();
    let h = total / count;
    let count = count as u64;
    let m = params.mass.value();
    let rho_body = params.density.value();
    let patch = params.contact_patch();

    let orientation = |k: u64| advance_orientation(q0, w, k as f64 * h);
    let mut p = state.position;
    let mut v = state.velocity;
    let mut q = orientation(0);
    let mut here = sample_at(probe, q.conjugate().rotate(p));
    let mut contact = None;
    for k in 0..count {
        // 1. Drag for h/2.
        v = drag(v, &here, q, 0.5 * h, params);
        // 2. Gravity and buoyancy, kick-drift-kick.
        let a0 = field.at(p).scale(1.0 - medium_density(&here) / rho_body);
        v = v + a0.scale(0.5 * h);
        p = p + v.scale(h);
        q = orientation(k + 1);
        let mut pf = q.conjugate().rotate(p);
        here = sample_at(probe, pf);
        let g = field.at(p);
        let a1 = g.scale(1.0 - medium_density(&here) / rho_body);
        v = v + a1.scale(0.5 * h);
        // 3. Contact.
        let up_f = q
            .conjugate()
            .rotate(-g)
            .normalized()
            .or_else(|| pf.normalized())
            .unwrap_or(Vec3::new(0.0, 0.0, 1.0));
        let solid_here = here.state == State::Solid;
        let touching = if solid_here {
            Some((pf, here))
        } else {
            let below = pf - up_f.scale(CONTACT_SKIN);
            let s = sample_at(probe, below);
            (s.state == State::Solid).then_some((below, s))
        };
        contact = None;
        if let Some((inside, ground)) = touching {
            let n_f = surface_normal(probe, inside, patch, up_f);
            if solid_here {
                if let Some(d) = exit_distance(probe, pf, n_f, patch / 8.0) {
                    pf = pf + n_f.scale(d);
                    p = q.rotate(pf);
                    here = sample_at(probe, pf);
                }
            }
            let n = q.rotate(n_f);
            let vg = q.rotate(ground.velocity);
            let vn = (v - vg).dot(n);
            let mut jn = 0.0;
            if vn < 0.0 {
                v = v - n.scale(vn);
                jn = -m * vn;
            }
            let budget = params.friction * jn;
            let mut walk = intent - n.scale(intent.dot(n));
            let wl = walk.length();
            if wl * h > budget {
                walk = walk.scale(budget / (h * wl));
            }
            v = v + walk.scale(h / m);
            let rel = v - vg;
            let mut slide = rel - n.scale(rel.dot(n));
            if let Some(u) = walk.normalized() {
                let along = slide.dot(u);
                if along > 0.0 {
                    slide = slide - u.scale(along);
                }
            }
            let left = budget - walk.length() * h;
            let sl = slide.length();
            if sl > 0.0 && left > 0.0 {
                let dv = sl.min(left / m);
                v = v - slide.scale(dv / sl);
            }
            contact = Some(Contact {
                normal: n,
                normal_force: jn / h,
                ground_velocity: vg,
            });
        }
        // 4. Drag for h/2.
        v = drag(v, &here, q, 0.5 * h, params);
    }
    state.position = p;
    state.velocity = v;
    state.contact = contact;
}

/// The renderer's loop in its simplest form: `steps` times, advances the
/// observer by `dt` with [`step_observer`], then the frame system by the
/// same `dt` with [`crate::integrate::advance`] in one step of the default
/// scheme. The observer conformance vectors and the WebAssembly export that
/// runs them use this.
///
/// # Panics
///
/// As [`step_observer`], and if `dt` is zero while `steps` is not.
#[allow(clippy::too_many_arguments)]
pub fn simulate(
    state: &mut ObserverState,
    frame: u64,
    system: &mut FrameSystem,
    probe: &dyn MatterProbe,
    intent: Vec3,
    dt: Seconds,
    steps: u32,
    params: &ObserverParams,
) {
    for _ in 0..steps {
        step_observer(state, frame, system, probe, intent, dt, params);
        let to = system.time() + dt;
        crate::integrate::advance(system, to, dt, crate::integrate::Scheme::default());
    }
}

/// A probe for matter whose density depends only on the distance from the
/// frame origin, at rest in the frame: density interpolated linearly
/// between tabulated radii, one state inside the outermost radius, vacuum
/// outside. Used by the observer conformance vectors and the WebAssembly
/// export that runs them.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialProbe {
    /// Radii in meters, ascending, at least one.
    pub radii: Vec<f64>,
    /// Density at each radius, kilograms per cubic meter.
    pub densities: Vec<f64>,
    /// The state of the matter inside the outermost radius.
    pub state: State,
    /// The frame's angular velocity, radians per second about its own axes;
    /// the matter turns with it.
    pub angular_velocity: Vec3,
}

impl MatterProbe for RadialProbe {
    fn sample(&self, p: Vec3) -> Option<Sample> {
        let r = p.length();
        let n = self.radii.len().min(self.densities.len());
        let velocity = self.angular_velocity.cross(p);
        if n == 0 || r > self.radii[n - 1] {
            return Some(Sample {
                density: Density::new(0.0),
                state: State::Vacuum,
                velocity,
            });
        }
        let i = self.radii[..n].partition_point(|&x| x < r);
        let rho = if i == 0 {
            self.densities[0]
        } else {
            let (r0, r1) = (self.radii[i - 1], self.radii[i]);
            let (d0, d1) = (self.densities[i - 1], self.densities[i]);
            d0 + (d1 - d0) * (r - r0) / (r1 - r0)
        };
        Some(Sample {
            density: Density::new(rho),
            state: if rho > 0.0 { self.state } else { State::Vacuum },
            velocity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Frame, FrameTree, Registry, ROOT_PARENT};

    fn system(mass: f64, w: Vec3) -> FrameSystem {
        let f = Frame {
            frame_id: 1,
            parent_frame_id: ROOT_PARENT,
            root_extent: Meters::new(1.0e8),
            max_depth: 4,
            mass: Kilograms::new(mass),
            position: Vec3::zero(),
            velocity: Vec3::zero(),
            orientation: Quat::identity(),
            angular_velocity: w,
            j2: 0.0,
            reference_radius: Meters::new(0.0),
            principal_moments: Vec3::zero(),
        };
        let reg = Registry::new(Seconds::new(0.0), vec![f]).unwrap();
        FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap())
    }

    #[test]
    fn up_points_away_from_mass() {
        let s = system(6.0e24, Vec3::zero());
        let up = up_direction(Vec3::new(0.0, 7.0e6, 0.0), 1, &s).unwrap();
        assert!((up - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-12);
        assert!(up_direction(Vec3::zero(), 1, &s).is_none());
    }

    #[test]
    fn radial_probe_interpolates() {
        let p = RadialProbe {
            radii: vec![0.0, 10.0],
            densities: vec![100.0, 0.0],
            state: State::Gas,
            angular_velocity: Vec3::new(0.0, 0.0, 2.0),
        };
        let s = p.sample(Vec3::new(5.0, 0.0, 0.0)).unwrap();
        assert_eq!(s.density, Density::new(50.0));
        assert_eq!(s.state, State::Gas);
        assert_eq!(s.velocity, Vec3::new(0.0, 10.0, 0.0));
        assert_eq!(
            p.sample(Vec3::new(11.0, 0.0, 0.0)).unwrap().state,
            State::Vacuum
        );
        assert_eq!(
            p.sample(Vec3::new(10.0, 0.0, 0.0)).unwrap().state,
            State::Vacuum
        );
    }

    #[test]
    fn walking_moves_along_the_ground() {
        let s = system(6.0e24, Vec3::zero());
        let probe = RadialProbe {
            radii: vec![0.0, 6.4e6],
            densities: vec![5500.0, 5500.0],
            state: State::Solid,
            angular_velocity: Vec3::zero(),
        };
        let params = ObserverParams::default();
        let mut o = ObserverState::new(Vec3::new(0.0, 0.0, 6.4e6), Vec3::zero());
        for _ in 0..60 {
            step_observer(
                &mut o,
                1,
                &s,
                &probe,
                Vec3::zero(),
                Seconds::new(1.0 / 60.0),
                &params,
            );
        }
        assert!(o.contact.is_some());
        let start = o.position;
        for _ in 0..60 {
            step_observer(
                &mut o,
                1,
                &s,
                &probe,
                Vec3::new(100.0, 0.0, 0.0),
                Seconds::new(1.0 / 60.0),
                &params,
            );
        }
        // 100 N on 80 kg for 1 s: 0.625 m along x, still on the ground.
        let moved = o.position - start;
        assert!((moved.x - 0.625).abs() < 0.01, "{moved:?}");
        assert!((o.position.length() - 6.4e6).abs() < 1e-3);
        let c = o.contact.unwrap();
        assert!(
            (c.normal_force - 80.0 * 9.77).abs() < 2.0,
            "{}",
            c.normal_force
        );
    }

    #[test]
    fn rebase_keeps_the_point() {
        let s = system(1.0, Vec3::zero());
        let mut o = ObserverState::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let before = o;
        o.rebase(1, 1, &s);
        assert_eq!(o, before);
    }
}
