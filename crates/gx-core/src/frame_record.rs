//! Frame records: one frame's constant registry fields, addressed by the key
//! `frame-{frameId}`.
//!
//! Implements `matter-format.md` section 11: the 88-byte little-endian
//! layout (11.1), the empty record (11.2), validation with codes 1021 to
//! 1029 (11.3), and the agreement check against a registry frame (11.4). A
//! record carries nothing that depends on the universe's origin: no epoch,
//! position, velocity, orientation, or spin.
//!
//! Every byte [`encode`] writes is a pure function of the record.

use core::fmt;

use crate::error::{codes, ValidationError};
use crate::registry::{self, Frame};
use crate::units::{Kilograms, Meters, Quat, Vec3};

/// The record magic, `3GFR`.
pub const MAGIC: [u8; 4] = [0x33, 0x47, 0x46, 0x52];

/// The record format version this library reads and writes.
pub const VERSION: u16 = 1;

/// `flags` bit 0: the module does not declare the frame (section 11.2).
pub const FLAG_EMPTY: u16 = 1;

/// Length of every record in bytes, empty or not.
pub const RECORD_LEN: usize = 88;

/// One frame's record (section 11.1). Values built directly are not checked;
/// [`encode`] writes them as they are and [`decode`] checks section 11.3.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameRecord {
    /// `true` when the declaring module does not declare the frame: the
    /// EMPTY flag, with every other field 0 (section 11.2).
    pub empty: bool,
    /// The frame this record describes; equals the key.
    pub frame_id: u64,
    /// The parent frame, or [`registry::ROOT_PARENT`] for the root.
    pub parent_frame_id: u64,
    /// Edge of the frame's root cube.
    pub root_extent: Meters,
    /// Deepest cell the declaring compiler fills, 0 to 31.
    pub max_depth: u8,
    /// Gravitational mass of the frame.
    pub mass: Kilograms,
    /// Unnormalized second zonal harmonic about the frame's z axis, 0 for
    /// none.
    pub j2: f64,
    /// The radius `j2` is defined against; 0 when `j2` is 0.
    pub reference_radius: Meters,
    /// Moments of inertia about the frame's x, y, z axes, in kilogram
    /// square meters; all 0 for a frame whose spin is held constant.
    pub principal_moments: Vec3,
}

impl FrameRecord {
    /// The empty record for `frame_id` (section 11.2): EMPTY set, every
    /// other field 0.
    pub fn empty(frame_id: u64) -> FrameRecord {
        FrameRecord {
            empty: true,
            frame_id,
            parent_frame_id: 0,
            root_extent: Meters::new(0.0),
            max_depth: 0,
            mass: Kilograms::new(0.0),
            j2: 0.0,
            reference_radius: Meters::new(0.0),
            principal_moments: Vec3::new(0.0, 0.0, 0.0),
        }
    }

    /// The constant part of a registry frame record (section 11): its id,
    /// parent, extent, depth, mass, `j2`, reference radius, and moments.
    pub fn from_registry_frame(frame: &Frame) -> FrameRecord {
        FrameRecord {
            empty: false,
            frame_id: frame.frame_id,
            parent_frame_id: frame.parent_frame_id,
            root_extent: frame.root_extent,
            max_depth: frame.max_depth,
            mass: frame.mass,
            j2: frame.j2,
            reference_radius: frame.reference_radius,
            principal_moments: frame.principal_moments,
        }
    }
}

/// Encodes a record as the 88 bytes of section 11.1, little-endian.
pub fn encode(record: &FrameRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD_LEN);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let flags = if record.empty { FLAG_EMPTY } else { 0 };
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&record.frame_id.to_le_bytes());
    out.extend_from_slice(&record.parent_frame_id.to_le_bytes());
    out.extend_from_slice(&record.root_extent.value().to_le_bytes());
    out.push(record.max_depth);
    out.extend_from_slice(&[0u8; 7]);
    for v in [
        record.mass.value(),
        record.j2,
        record.reference_radius.value(),
        record.principal_moments.x,
        record.principal_moments.y,
        record.principal_moments.z,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(out.len(), RECORD_LEN);
    out
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_bits(u64_at(b, at))
}

/// Decodes and validates a record without a key: every rule of section 11.3
/// except `frame_id` against the key. Checks, in order, and reports the
/// first failing rule: length 88 (1021), magic (1022), version (1023), flag
/// bits (1024), reserved bytes (1025), then for an EMPTY record every other
/// field 0 (1027), else `root_extent`, `max_depth`, and `mass` (1028) and
/// `j2`, `reference_radius`, and `principal_moments` (1029).
pub fn decode(bytes: &[u8]) -> Result<FrameRecord, ValidationError> {
    decode_checked(bytes, None)
}

/// Validates `bytes` as the record for the key `frame-{frame_id}`: every
/// rule of section 11.3, in the order of [`decode`], with `frame_id` against
/// the key (1026) after the reserved bytes and before the field rules.
pub fn validate(frame_id: u64, bytes: &[u8]) -> Result<(), ValidationError> {
    decode_checked(bytes, Some(frame_id)).map(|_| ())
}

/// [`decode`], checking `frame_id` against `key` when one is given.
fn decode_checked(bytes: &[u8], key: Option<u64>) -> Result<FrameRecord, ValidationError> {
    let fail = |code: u16, what: String| Err(ValidationError::new(code, what));
    if bytes.len() != RECORD_LEN {
        return fail(
            codes::FRAME_RECORD_LENGTH,
            format!("length {} is not {RECORD_LEN}", bytes.len()),
        );
    }
    if bytes[0..4] != MAGIC {
        return fail(
            codes::FRAME_RECORD_BAD_MAGIC,
            format!("magic {:02x?} is not 3GFR", &bytes[0..4]),
        );
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return fail(
            codes::FRAME_RECORD_UNSUPPORTED_VERSION,
            format!("format_version {version} is not {VERSION}"),
        );
    }
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    if flags & !FLAG_EMPTY != 0 {
        return fail(
            codes::FRAME_RECORD_UNKNOWN_FLAGS,
            format!("flags {flags:#06x} has a bit other than EMPTY set"),
        );
    }
    if let Some(i) = bytes[33..40].iter().position(|&b| b != 0) {
        return fail(
            codes::FRAME_RECORD_RESERVED_NONZERO,
            format!("reserved byte at offset {} is not 0", 33 + i),
        );
    }
    let found = u64_at(bytes, 8);
    if let Some(key) = key.filter(|&k| k != found) {
        return fail(
            codes::FRAME_RECORD_ID_MISMATCH,
            format!("frame_id {found} differs from the key's {key}"),
        );
    }
    let record = FrameRecord {
        empty: flags & FLAG_EMPTY != 0,
        frame_id: found,
        parent_frame_id: u64_at(bytes, 16),
        root_extent: Meters::new(f64_at(bytes, 24)),
        max_depth: bytes[32],
        mass: Kilograms::new(f64_at(bytes, 40)),
        j2: f64_at(bytes, 48),
        reference_radius: Meters::new(f64_at(bytes, 56)),
        principal_moments: Vec3::new(f64_at(bytes, 64), f64_at(bytes, 72), f64_at(bytes, 80)),
    };
    if record.empty {
        // Every byte after `frame_id` is 0: bitwise, so -0.0 is refused.
        if let Some(i) = bytes[16..].iter().position(|&b| b != 0) {
            return fail(
                codes::FRAME_RECORD_EMPTY_NONZERO,
                format!("EMPTY record has a nonzero byte at offset {}", 16 + i),
            );
        }
        return Ok(record);
    }
    check_fields(&record)?;
    Ok(record)
}

/// Checks the section 5.2 field rules through the registry's own checker, so
/// a record and its registry frame are judged by the same code, and maps the
/// registry codes to 1028 and 1029.
fn check_fields(r: &FrameRecord) -> Result<(), ValidationError> {
    let frame = Frame {
        frame_id: r.frame_id,
        parent_frame_id: r.parent_frame_id,
        root_extent: r.root_extent,
        max_depth: r.max_depth,
        mass: r.mass,
        position: Vec3::new(0.0, 0.0, 0.0),
        velocity: Vec3::new(0.0, 0.0, 0.0),
        orientation: Quat::new(0.0, 0.0, 0.0, 1.0),
        angular_velocity: Vec3::new(0.0, 0.0, 0.0),
        j2: r.j2,
        reference_radius: r.reference_radius,
        principal_moments: r.principal_moments,
    };
    registry::check_frame(0, &frame).map_err(|e| {
        let code = match e.code {
            codes::FRAME_EXTENT_NOT_FINITE
            | codes::FRAME_EXTENT_NOT_POSITIVE
            | codes::FRAME_MAX_DEPTH_OUT_OF_RANGE
            | codes::FRAME_MASS_NOT_FINITE
            | codes::FRAME_MASS_NEGATIVE => codes::FRAME_RECORD_FIELD,
            _ => codes::FRAME_RECORD_SHAPE,
        };
        // Drop the registry's `frame 0 (frame_id N): ` prefix.
        let reason = e.reason.split_once("): ").map_or(&*e.reason, |(_, r)| r);
        ValidationError::new(code, format!("frame_id {}: {reason}", r.frame_id))
    })
}

/// The seven fields section 11.4 compares, in record order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// `parent_frame_id`.
    ParentFrameId,
    /// `root_extent`.
    RootExtent,
    /// `max_depth`.
    MaxDepth,
    /// `mass`.
    Mass,
    /// `j2`.
    J2,
    /// `reference_radius`.
    ReferenceRadius,
    /// `principal_moments`.
    PrincipalMoments,
}

impl Field {
    /// The field's name as the spec writes it.
    pub fn name(self) -> &'static str {
        match self {
            Field::ParentFrameId => "parent_frame_id",
            Field::RootExtent => "root_extent",
            Field::MaxDepth => "max_depth",
            Field::Mass => "mass",
            Field::J2 => "j2",
            Field::ReferenceRadius => "reference_radius",
            Field::PrincipalMoments => "principal_moments",
        }
    }
}

impl fmt::Display for Field {
    /// Writes [`Field::name`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Section 11.4: `Ok` when the seven constant fields of `registry_frame` and
/// `record` are bitwise equal, else the first field, in record order, that
/// differs. Floats compare by bit pattern, so `0.0` and `-0.0` differ. The
/// frame ids and the EMPTY flag are not compared: the caller pairs a record
/// with the registry frame of the same id, and an empty record's zero fields
/// differ from any valid frame's `root_extent`.
pub fn agrees(registry_frame: &Frame, record: &FrameRecord) -> Result<(), Field> {
    let f = registry_frame;
    let r = record;
    let bits3 = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
    if f.parent_frame_id != r.parent_frame_id {
        return Err(Field::ParentFrameId);
    }
    if f.root_extent.value().to_bits() != r.root_extent.value().to_bits() {
        return Err(Field::RootExtent);
    }
    if f.max_depth != r.max_depth {
        return Err(Field::MaxDepth);
    }
    if f.mass.value().to_bits() != r.mass.value().to_bits() {
        return Err(Field::Mass);
    }
    if f.j2.to_bits() != r.j2.to_bits() {
        return Err(Field::J2);
    }
    if f.reference_radius.value().to_bits() != r.reference_radius.value().to_bits() {
        return Err(Field::ReferenceRadius);
    }
    if bits3(f.principal_moments) != bits3(r.principal_moments) {
        return Err(Field::PrincipalMoments);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(parent: u64, j2: f64, radius: f64, moments: Vec3) -> Frame {
        Frame {
            frame_id: 5,
            parent_frame_id: parent,
            root_extent: Meters::new(2.0e7),
            max_depth: 12,
            mass: Kilograms::new(3.4e24),
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(4.0, 5.0, 6.0),
            orientation: Quat::new(0.0, 0.0, 0.0, 1.0),
            angular_velocity: Vec3::new(0.0, 0.0, 2.2e-4),
            j2,
            reference_radius: Meters::new(radius),
            principal_moments: moments,
        }
    }

    fn sample() -> Frame {
        frame(1, 1.0e-3, 8.3e6, Vec3::new(8.0e37, 8.0e37, 8.1e37))
    }

    #[test]
    fn layout_and_round_trip() {
        let r = FrameRecord::from_registry_frame(&sample());
        let b = encode(&r);
        assert_eq!(b.len(), RECORD_LEN);
        assert_eq!(&b[0..4], b"3GFR");
        assert_eq!(&b[4..8], &[1, 0, 0, 0]);
        assert_eq!(u64_at(&b, 8), 5);
        assert_eq!(u64_at(&b, 16), 1);
        assert_eq!(f64_at(&b, 24), 2.0e7);
        assert_eq!(b[32], 12);
        assert_eq!(f64_at(&b, 40), 3.4e24);
        assert_eq!(f64_at(&b, 48), 1.0e-3);
        assert_eq!(f64_at(&b, 56), 8.3e6);
        assert_eq!(f64_at(&b, 80), 8.1e37);
        assert_eq!(decode(&b), Ok(r));
        assert_eq!(validate(5, &b), Ok(()));
        assert_eq!(agrees(&sample(), &r), Ok(()));
    }

    #[test]
    fn empty_record() {
        let b = encode(&FrameRecord::empty(9));
        assert_eq!(b.len(), RECORD_LEN);
        assert_eq!(&b[6..8], &[1, 0]);
        assert!(b[16..].iter().all(|&x| x == 0));
        assert_eq!(validate(9, &b), Ok(()));
        assert_eq!(decode(&b), Ok(FrameRecord::empty(9)));
        let mut neg = b.clone();
        neg[47] = 0x80;
        assert_eq!(
            validate(9, &neg).unwrap_err().code,
            codes::FRAME_RECORD_EMPTY_NONZERO
        );
    }

    #[test]
    fn check_order() {
        let b = encode(&FrameRecord::from_registry_frame(&sample()));
        let code = |id: u64, f: &dyn Fn(&mut Vec<u8>)| {
            let mut c = b.clone();
            f(&mut c);
            validate(id, &c).unwrap_err().code
        };
        assert_eq!(code(5, &|c| c.truncate(87)), codes::FRAME_RECORD_LENGTH);
        assert_eq!(code(5, &|c| c[0] = 0), codes::FRAME_RECORD_BAD_MAGIC);
        assert_eq!(
            code(5, &|c| c[4] = 2),
            codes::FRAME_RECORD_UNSUPPORTED_VERSION
        );
        assert_eq!(code(5, &|c| c[6] = 2), codes::FRAME_RECORD_UNKNOWN_FLAGS);
        assert_eq!(
            code(5, &|c| c[39] = 1),
            codes::FRAME_RECORD_RESERVED_NONZERO
        );
        // The reserved bytes come before the key; the key before the fields.
        assert_eq!(
            code(6, &|c| c[39] = 1),
            codes::FRAME_RECORD_RESERVED_NONZERO
        );
        assert_eq!(
            code(6, &|c| c[24..32].fill(0)),
            codes::FRAME_RECORD_ID_MISMATCH
        );
        assert_eq!(code(5, &|c| c[24..32].fill(0)), codes::FRAME_RECORD_FIELD);
        assert_eq!(code(5, &|c| c[32] = 32), codes::FRAME_RECORD_FIELD);
        assert_eq!(code(5, &|c| c[56..64].fill(0)), codes::FRAME_RECORD_SHAPE);
        assert_eq!(code(5, &|c| c[64..72].fill(0)), codes::FRAME_RECORD_SHAPE);
    }

    #[test]
    fn agreement_names_the_field() {
        let base = sample();
        let r = FrameRecord::from_registry_frame(&base);
        type Change = &'static dyn Fn(&mut FrameRecord);
        let cases: [(Field, Change); 7] = [
            (Field::ParentFrameId, &|r| r.parent_frame_id = 2),
            (Field::RootExtent, &|r| r.root_extent = Meters::new(2.1e7)),
            (Field::MaxDepth, &|r| r.max_depth = 11),
            (Field::Mass, &|r| r.mass = Kilograms::new(3.5e24)),
            (Field::J2, &|r| r.j2 = 2.0e-3),
            (Field::ReferenceRadius, &|r| {
                r.reference_radius = Meters::new(8.4e6)
            }),
            (Field::PrincipalMoments, &|r| r.principal_moments.z = 8.2e37),
        ];
        for (field, f) in cases {
            let mut c = r;
            f(&mut c);
            assert_eq!(agrees(&base, &c), Err(field));
        }
        let mut zero = frame(1, 0.0, 0.0, Vec3::new(0.0, 0.0, 0.0));
        let rz = FrameRecord::from_registry_frame(&zero);
        zero.j2 = -0.0;
        assert_eq!(agrees(&zero, &rz), Err(Field::J2));
        assert_eq!(
            agrees(&base, &FrameRecord::empty(5)),
            Err(Field::ParentFrameId)
        );
    }
}
