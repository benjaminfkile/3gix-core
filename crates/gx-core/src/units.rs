//! Branded SI quantities and the unitless vector and quaternion math types.
//!
//! Implements the units principle of `matter-format.md` section 1: every
//! physical value is carried in SI base or derived units (meters, kilograms,
//! seconds, kelvin) and is wrapped in a distinct type, so that no bare `f64`
//! for a physical value can reach an encoder or a law. Arithmetic is defined
//! only where the physics makes sense. Adding a length to a mass does not
//! compile:
//!
//! ```compile_fail,E0308
//! use gx_core::units::{Kilograms, Meters};
//! let _ = Meters::new(1.0) + Kilograms::new(1.0);
//! ```
//!
//! [`Vec3`] and [`Quat`] are plain math types with no unit. Callers document
//! which unit a [`Vec3`] carries. All arithmetic is written as straightforward
//! `f64` expressions with no fused multiply add and no SIMD, so results are
//! identical on every platform.

use core::ops::{Add, Div, Mul, Neg, Sub};

macro_rules! quantity {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[repr(transparent)]
        #[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Default)]
        pub struct $name(f64);

        impl $name {
            /// Wraps a raw `f64` already expressed in this type's SI unit.
            pub const fn new(value: f64) -> Self {
                Self(value)
            }

            /// Returns the raw `f64` in this type's SI unit.
            pub fn value(self) -> f64 {
                self.0
            }

            /// Returns `true` if the value is neither infinite nor NaN.
            pub fn is_finite(self) -> bool {
                self.0.is_finite()
            }
        }

        impl Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self(self.0 + rhs.0)
            }
        }

        impl Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self(self.0 - rhs.0)
            }
        }

        impl Mul<f64> for $name {
            type Output = Self;
            fn mul(self, rhs: f64) -> Self {
                Self(self.0 * rhs)
            }
        }

        impl Div<f64> for $name {
            type Output = Self;
            fn div(self, rhs: f64) -> Self {
                Self(self.0 / rhs)
            }
        }
    };
}

quantity! {
    /// A length in meters (m).
    Meters
}
quantity! {
    /// A volume in cubic meters (m^3).
    CubicMeters
}
quantity! {
    /// A mass in kilograms (kg).
    Kilograms
}
quantity! {
    /// A duration in seconds (s).
    Seconds
}
quantity! {
    /// A thermodynamic temperature in kelvin (K).
    Kelvin
}
quantity! {
    /// A mass density in kilograms per cubic meter (kg/m^3).
    Density
}
quantity! {
    /// A mass attenuation coefficient in square meters per kilogram (m^2/kg).
    Attenuation
}
quantity! {
    /// A speed or velocity component in meters per second (m/s).
    MetersPerSecond
}
quantity! {
    /// An angular rate in radians per second (rad/s).
    RadiansPerSecond
}
quantity! {
    /// A dimensionless quantity on the closed interval from 0 to 1, such as
    /// albedo or roughness.
    Ratio
}

impl Meters {
    /// Returns the volume of a cube with this edge length: `self * self * self`.
    pub fn cubed(self) -> CubicMeters {
        CubicMeters(self.0 * self.0 * self.0)
    }
}

impl Ratio {
    /// Returns `true` if the value lies in the closed interval from 0 to 1.
    pub fn is_in_unit_interval(self) -> bool {
        (0.0..=1.0).contains(&self.0)
    }
}

impl Mul<CubicMeters> for Density {
    type Output = Kilograms;
    fn mul(self, rhs: CubicMeters) -> Kilograms {
        Kilograms(self.0 * rhs.0)
    }
}

impl Div<Seconds> for Meters {
    type Output = MetersPerSecond;
    fn div(self, rhs: Seconds) -> MetersPerSecond {
        MetersPerSecond(self.0 / rhs.0)
    }
}

impl Mul<Seconds> for MetersPerSecond {
    type Output = Meters;
    fn mul(self, rhs: Seconds) -> Meters {
        Meters(self.0 * rhs.0)
    }
}

/// A three component vector of `f64` with no unit attached.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Vec3 {
    /// First component.
    pub x: f64,
    /// Second component.
    pub y: f64,
    /// Third component.
    pub z: f64,
}

impl Vec3 {
    /// Builds a vector from its components.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// The zero vector.
    pub const fn zero() -> Self {
        Self::new(0.0, 0.0, 0.0)
    }

    /// Dot product: `x*x' + y*y' + z*z'`, summed left to right.
    pub fn dot(self, rhs: Self) -> f64 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Right handed cross product.
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    /// Squared Euclidean length, `self.dot(self)`.
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    /// Euclidean length.
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    /// Returns the vector scaled to unit length, or `None` if its length is
    /// zero or not finite.
    pub fn normalized(self) -> Option<Self> {
        let len = self.length();
        if len == 0.0 || !len.is_finite() {
            return None;
        }
        Some(Self::new(self.x / len, self.y / len, self.z / len))
    }

    /// Multiplies every component by `s`.
    pub fn scale(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// A quaternion `w + xi + yj + zk` of `f64`, used for rotations when unit
/// length.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Quat {
    /// Coefficient of `i`.
    pub x: f64,
    /// Coefficient of `j`.
    pub y: f64,
    /// Coefficient of `k`.
    pub z: f64,
    /// Scalar part.
    pub w: f64,
}

impl Default for Quat {
    /// The identity rotation.
    fn default() -> Self {
        Self::identity()
    }
}

impl Quat {
    /// Builds a quaternion from its components, scalar part last.
    pub const fn new(x: f64, y: f64, z: f64, w: f64) -> Self {
        Self { x, y, z, w }
    }

    /// The identity rotation `(0, 0, 0, 1)`.
    pub const fn identity() -> Self {
        Self::new(0.0, 0.0, 0.0, 1.0)
    }

    /// Euclidean norm of the four components.
    pub fn norm(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt()
    }

    /// Returns the quaternion scaled to unit norm, or `None` if its norm is
    /// zero or not finite.
    pub fn normalized(self) -> Option<Self> {
        let n = self.norm();
        if n == 0.0 || !n.is_finite() {
            return None;
        }
        Some(Self::new(self.x / n, self.y / n, self.z / n, self.w / n))
    }

    /// Returns `true` if `|norm - 1| <= tolerance`.
    pub fn is_unit(self, tolerance: f64) -> bool {
        (self.norm() - 1.0).abs() <= tolerance
    }

    /// Conjugate `(-x, -y, -z, w)`, the inverse rotation for a unit quaternion.
    pub fn conjugate(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, self.w)
    }

    /// Rotates `v` by this quaternion, assumed unit length.
    ///
    /// Computes `v + w*t + u x t` with `u = (x, y, z)` and `t = 2 (u x v)`,
    /// which equals `q v q*` for a unit `q`.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(v).scale(2.0);
        v + t.scale(self.w) + u.cross(t)
    }
}

impl Mul for Quat {
    type Output = Self;
    /// Hamilton product. `(a * b).rotate(v) == a.rotate(b.rotate(v))`.
    fn mul(self, r: Self) -> Self {
        Self::new(
            self.w * r.x + self.x * r.w + self.y * r.z - self.z * r.y,
            self.w * r.y - self.x * r.z + self.y * r.w + self.z * r.x,
            self.w * r.z + self.x * r.y - self.y * r.x + self.z * r.w,
            self.w * r.w - self.x * r.x - self.y * r.y - self.z * r.z,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-12
    }

    #[test]
    fn quantity_basics() {
        let m = Meters::new(2.5);
        assert_eq!(m.value(), 2.5);
        assert!(m.is_finite());
        assert!(!Meters::new(f64::NAN).is_finite());
        assert!(!Kelvin::new(f64::INFINITY).is_finite());
        assert_eq!(Seconds::default().value(), 0.0);
        assert!(Meters::new(1.0) < Meters::new(2.0));
        const C: Kilograms = Kilograms::new(3.0);
        assert_eq!(C.value(), 3.0);
    }

    #[test]
    fn same_type_add_sub_scale() {
        assert_eq!(Meters::new(1.0) + Meters::new(2.0), Meters::new(3.0));
        assert_eq!(Meters::new(1.0) - Meters::new(2.0), Meters::new(-1.0));
        assert_eq!(Meters::new(3.0) * 2.0, Meters::new(6.0));
        assert_eq!(Meters::new(3.0) / 2.0, Meters::new(1.5));
        assert_eq!(
            CubicMeters::new(1.0) + CubicMeters::new(1.0),
            CubicMeters::new(2.0)
        );
        assert_eq!(
            Kilograms::new(5.0) - Kilograms::new(1.0),
            Kilograms::new(4.0)
        );
        assert_eq!(Seconds::new(2.0) * 3.0, Seconds::new(6.0));
        assert_eq!(Kelvin::new(300.0) / 3.0, Kelvin::new(100.0));
        assert_eq!(Density::new(1.0) + Density::new(2.0), Density::new(3.0));
        assert_eq!(
            Attenuation::new(4.0) - Attenuation::new(1.0),
            Attenuation::new(3.0)
        );
        assert_eq!(MetersPerSecond::new(4.0) * 0.5, MetersPerSecond::new(2.0));
        assert_eq!(RadiansPerSecond::new(4.0) / 4.0, RadiansPerSecond::new(1.0));
        assert_eq!(Ratio::new(0.25) + Ratio::new(0.5), Ratio::new(0.75));
    }

    #[test]
    fn cross_type_operators() {
        assert_eq!(Meters::new(3.0).cubed(), CubicMeters::new(27.0));
        assert_eq!(
            Density::new(2.0) * CubicMeters::new(8.0),
            Kilograms::new(16.0)
        );
        assert_eq!(
            Meters::new(10.0) / Seconds::new(4.0),
            MetersPerSecond::new(2.5)
        );
        assert_eq!(
            MetersPerSecond::new(2.5) * Seconds::new(4.0),
            Meters::new(10.0)
        );
    }

    #[test]
    fn ratio_interval() {
        assert!(Ratio::new(0.0).is_in_unit_interval());
        assert!(Ratio::new(1.0).is_in_unit_interval());
        assert!(!Ratio::new(1.5).is_in_unit_interval());
        assert!(!Ratio::new(-0.1).is_in_unit_interval());
        assert!(!Ratio::new(f64::NAN).is_in_unit_interval());
    }

    #[test]
    fn vec3_ops() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(Vec3::zero(), Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(Vec3::default(), Vec3::zero());
        assert_eq!(a.dot(b), 32.0);
        assert_eq!(a.cross(b), Vec3::new(-3.0, 6.0, -3.0));
        assert_eq!(
            Vec3::new(1.0, 0.0, 0.0).cross(Vec3::new(0.0, 1.0, 0.0)),
            Vec3::new(0.0, 0.0, 1.0)
        );
        assert_eq!(a.length_squared(), 14.0);
        assert_eq!(Vec3::new(3.0, 4.0, 0.0).length(), 5.0);
        assert_eq!(
            Vec3::new(3.0, 4.0, 0.0).normalized(),
            Some(Vec3::new(0.6, 0.8, 0.0))
        );
        assert_eq!(Vec3::zero().normalized(), None);
        assert_eq!(Vec3::new(f64::INFINITY, 0.0, 0.0).normalized(), None);
        assert_eq!(a.scale(2.0), Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(a + b, Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(b - a, Vec3::new(3.0, 3.0, 3.0));
        assert_eq!(-a, Vec3::new(-1.0, -2.0, -3.0));
    }

    #[test]
    fn quat_basics() {
        let id = Quat::identity();
        assert_eq!(id, Quat::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(Quat::default(), id);
        assert_eq!(id.norm(), 1.0);
        assert!(id.is_unit(0.0));
        let q = Quat::new(0.0, 0.0, 3.0, 4.0);
        assert_eq!(q.norm(), 5.0);
        assert!(!q.is_unit(1e-9));
        assert_eq!(q.normalized(), Some(Quat::new(0.0, 0.0, 0.6, 0.8)));
        assert_eq!(Quat::new(0.0, 0.0, 0.0, 0.0).normalized(), None);
        assert_eq!(
            Quat::new(1.0, 2.0, 3.0, 4.0).conjugate(),
            Quat::new(-1.0, -2.0, -3.0, 4.0)
        );
    }

    #[test]
    fn quat_rotate_and_mul() {
        let h = core::f64::consts::FRAC_1_SQRT_2;
        // Quarter turn about +z.
        let qz = Quat::new(0.0, 0.0, h, h);
        assert!(qz.is_unit(1e-15));
        assert!(close(
            qz.rotate(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(0.0, 1.0, 0.0)
        ));
        // Quarter turn about +x.
        let qx = Quat::new(h, 0.0, 0.0, h);
        assert!(close(
            qx.rotate(Vec3::new(0.0, 1.0, 0.0)),
            Vec3::new(0.0, 0.0, 1.0)
        ));
        // Composition applies the right operand first.
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert!(close((qz * qx).rotate(v), qz.rotate(qx.rotate(v))));
        // Conjugate undoes the rotation.
        assert!(close(qz.conjugate().rotate(qz.rotate(v)), v));
        // Identity is neutral.
        assert_eq!(Quat::identity() * qz, qz);
        assert_eq!(qz * Quat::identity(), qz);
        assert_eq!(Quat::identity().rotate(v), v);
        // i * j = k.
        assert_eq!(
            Quat::new(1.0, 0.0, 0.0, 0.0) * Quat::new(0.0, 1.0, 0.0, 0.0),
            Quat::new(0.0, 0.0, 1.0, 0.0)
        );
    }
}
