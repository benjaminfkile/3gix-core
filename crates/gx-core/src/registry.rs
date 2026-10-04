//! The frame registry: encode, decode, validate, and the union of a build's
//! registries.
//!
//! Implements `matter-format.md` section 5: the 24-byte header (5.1), the
//! 144-byte frame records sorted by `frame_id` (5.2), the empty registry
//! (5.3), and the rules across the union of all of a build's registries that the
//! renderer applies (5.2).
//!
//! A [`Registry`] is one compiler's list of reference frames with their mass
//! and state at the build epoch. It cannot hold a value that breaks a rule
//! one registry can be checked against: [`Registry::new`] runs the same
//! rules as [`decode`], and the fields are reachable only through accessors.
//!
//! A [`FrameTree`] is the union of every registry in a build. Building one with
//! [`FrameTree::from_registries`] checks the cross-registry rules: one epoch,
//! ids unique across the union, exactly one root, every parent present, and
//! no cycles.
//!
//! Every byte [`encode`] writes is a pure function of the registry. Validation
//! order and the code for every rule are listed in `docs/errors.md` and
//! [`crate::error::codes`].

use crate::error::{codes, ValidationError};
use crate::units::{Kilograms, Meters, Quat, Seconds, Vec3};

/// The `parent_frame_id` of a root frame: `0xFFFFFFFFFFFFFFFF`.
pub const ROOT_PARENT: u64 = u64::MAX;

/// The four magic bytes that open a registry: `3GRG` in ASCII.
pub const MAGIC: [u8; 4] = [0x33, 0x47, 0x52, 0x47];

/// The registry format version this module reads and writes.
pub const VERSION: u16 = 1;

/// Length of the registry header in bytes.
pub const HEADER_LEN: usize = 24;

/// Length of one frame record in bytes.
pub const RECORD_LEN: usize = 144;

/// Largest permitted `max_depth`.
pub const MAX_DEPTH: u8 = 31;

/// Largest permitted distance of an orientation's norm from 1.
pub const UNIT_TOLERANCE: f64 = 1e-9;

/// One reference frame: its place in the frame tree and its mass and state
/// relative to its parent at the registry epoch (section 5.2).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Frame {
    /// Identifier of this frame, unique across the union of a build's
    /// registries.
    pub frame_id: u64,
    /// Identifier of the parent frame, or [`ROOT_PARENT`] for the root.
    pub parent_frame_id: u64,
    /// Edge of the cube of space this frame's cells divide (section 3.1).
    pub root_extent: Meters,
    /// Deepest cell this frame's compilers fill with non-empty matter, 0 to
    /// 31.
    pub max_depth: u8,
    /// Authoritative gravitational mass of the frame.
    pub mass: Kilograms,
    /// Position of the frame origin in meters relative to the parent origin
    /// at the epoch, in root axes (translations add along the parent chain
    /// without rotation, see [`crate::frames`]). Zero for a root.
    pub position: Vec3,
    /// Velocity of the frame origin in meters per second relative to the
    /// parent at the epoch, in root axes. Zero for a root.
    pub velocity: Vec3,
    /// Unit quaternion giving the frame axes relative to the parent axes at
    /// the epoch.
    pub orientation: Quat,
    /// Angular velocity in radians per second about the frame axes.
    pub angular_velocity: Vec3,
}

impl Frame {
    /// Returns `true` if this frame is a root: its parent is [`ROOT_PARENT`].
    pub fn is_root(&self) -> bool {
        self.parent_frame_id == ROOT_PARENT
    }
}

/// One validated frame registry: an epoch and frames sorted by `frame_id`
/// with unique ids.
#[derive(Clone, Debug, PartialEq)]
pub struct Registry {
    epoch: Seconds,
    frames: Vec<Frame>,
}

impl Registry {
    /// Builds a registry, sorting `frames` by `frame_id` ascending.
    ///
    /// Fails with the code of the first broken rule: a non-finite epoch
    /// (606), more frames than a `u32` counts (608), a repeated id (613), or
    /// a frame field rule (614 to 625), checked frame by frame in id order.
    /// `epoch` is seconds of Barycentric Dynamical Time since J2000.
    pub fn new(epoch: Seconds, mut frames: Vec<Frame>) -> Result<Registry, ValidationError> {
        check_epoch(epoch.value())?;
        if u32::try_from(frames.len()).is_err() {
            return Err(ValidationError::new(
                codes::REGISTRY_TOO_MANY_FRAMES,
                format!("frame_count: {} frames do not fit a u32", frames.len()),
            ));
        }
        frames.sort_by_key(|f| f.frame_id);
        for (i, f) in frames.iter().enumerate() {
            if i > 0 && frames[i - 1].frame_id == f.frame_id {
                return Err(duplicate(i, f.frame_id));
            }
            check_frame(i, f)?;
        }
        Ok(Registry { epoch, frames })
    }

    /// The empty registry of section 5.3: no frames.
    ///
    /// Fails with 606 if `epoch` is not finite.
    pub fn empty(epoch: Seconds) -> Result<Registry, ValidationError> {
        Registry::new(epoch, Vec::new())
    }

    /// Epoch in seconds of Barycentric Dynamical Time since J2000.
    pub fn epoch(&self) -> Seconds {
        self.epoch
    }

    /// The frames, sorted by `frame_id` ascending.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Returns `true` if the registry declares no frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

fn check_epoch(epoch: f64) -> Result<(), ValidationError> {
    if epoch.is_finite() {
        Ok(())
    } else {
        Err(ValidationError::new(
            codes::REGISTRY_EPOCH_NOT_FINITE,
            format!("epoch: {epoch} is not finite"),
        ))
    }
}

fn duplicate(i: usize, id: u64) -> ValidationError {
    ValidationError::new(
        codes::FRAME_DUPLICATE_ID,
        format!("frame {i}: frame_id {id} appears more than once"),
    )
}

fn vec_finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn vec_zero(v: Vec3) -> bool {
    v.x == 0.0 && v.y == 0.0 && v.z == 0.0
}

/// Checks the field rules of one record, in the order of `docs/errors.md`.
/// `i` is the record index, used in reasons.
fn check_frame(i: usize, f: &Frame) -> Result<(), ValidationError> {
    let id = f.frame_id;
    let fail = |code: u16, what: String| {
        Err(ValidationError::new(
            code,
            format!("frame {i} (frame_id {id}): {what}"),
        ))
    };
    let extent = f.root_extent.value();
    if !extent.is_finite() {
        return fail(
            codes::FRAME_EXTENT_NOT_FINITE,
            format!("root_extent {extent} is not finite"),
        );
    }
    if extent <= 0.0 {
        return fail(
            codes::FRAME_EXTENT_NOT_POSITIVE,
            format!("root_extent {extent:e} is not greater than 0"),
        );
    }
    if f.max_depth > MAX_DEPTH {
        return fail(
            codes::FRAME_MAX_DEPTH_OUT_OF_RANGE,
            format!("max_depth {} is above {MAX_DEPTH}", f.max_depth),
        );
    }
    let mass = f.mass.value();
    if !mass.is_finite() {
        return fail(
            codes::FRAME_MASS_NOT_FINITE,
            format!("mass {mass} is not finite"),
        );
    }
    if mass < 0.0 {
        return fail(
            codes::FRAME_MASS_NEGATIVE,
            format!("mass {mass:e} is negative"),
        );
    }
    if !vec_finite(f.position) {
        return fail(
            codes::FRAME_POSITION_NOT_FINITE,
            format!("position {:?} is not finite", f.position),
        );
    }
    if !vec_finite(f.velocity) {
        return fail(
            codes::FRAME_VELOCITY_NOT_FINITE,
            format!("velocity {:?} is not finite", f.velocity),
        );
    }
    let q = f.orientation;
    if !(q.x.is_finite() && q.y.is_finite() && q.z.is_finite() && q.w.is_finite()) {
        return fail(
            codes::FRAME_ORIENTATION_NOT_FINITE,
            format!("orientation {q:?} is not finite"),
        );
    }
    if !q.is_unit(UNIT_TOLERANCE) {
        return fail(
            codes::FRAME_ORIENTATION_NOT_UNIT,
            format!(
                "orientation norm {:e} is not 1 within {UNIT_TOLERANCE:e}",
                q.norm()
            ),
        );
    }
    if !vec_finite(f.angular_velocity) {
        return fail(
            codes::FRAME_ANGULAR_VELOCITY_NOT_FINITE,
            format!("angular_velocity {:?} is not finite", f.angular_velocity),
        );
    }
    if f.is_root() && !vec_zero(f.position) {
        return fail(
            codes::ROOT_POSITION_NONZERO,
            format!("root position {:?} is not 0", f.position),
        );
    }
    if f.is_root() && !vec_zero(f.velocity) {
        return fail(
            codes::ROOT_VELOCITY_NONZERO,
            format!("root velocity {:?} is not 0", f.velocity),
        );
    }
    Ok(())
}

fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_vec3(out: &mut Vec<u8>, v: Vec3) {
    for c in [v.x, v.y, v.z] {
        put_f64(out, c);
    }
}

/// Encodes a registry: the 24-byte header of section 5.1, then one 144-byte
/// record per frame (section 5.2) in `frame_id` order.
///
/// Every `f64` is written with its exact bit pattern, so the output is a pure
/// function of the registry.
pub fn encode(registry: &Registry) -> Vec<u8> {
    let frames = registry.frames();
    let count = u32::try_from(frames.len()).expect("Registry::new bounds the frame count");
    let mut out = Vec::with_capacity(HEADER_LEN + RECORD_LEN * frames.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    put_f64(&mut out, registry.epoch().value());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for f in frames {
        out.extend_from_slice(&f.frame_id.to_le_bytes());
        out.extend_from_slice(&f.parent_frame_id.to_le_bytes());
        put_f64(&mut out, f.root_extent.value());
        out.push(f.max_depth);
        out.extend_from_slice(&[0u8; 7]);
        put_f64(&mut out, f.mass.value());
        put_vec3(&mut out, f.position);
        put_vec3(&mut out, f.velocity);
        let q = f.orientation;
        for c in [q.x, q.y, q.z, q.w] {
            put_f64(&mut out, c);
        }
        put_vec3(&mut out, f.angular_velocity);
    }
    debug_assert_eq!(out.len(), HEADER_LEN + RECORD_LEN * frames.len());
    out
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_bits(u64_at(b, at))
}

fn vec3_at(b: &[u8], at: usize) -> Vec3 {
    Vec3::new(f64_at(b, at), f64_at(b, at + 8), f64_at(b, at + 16))
}

/// Reads one record. `r` is exactly [`RECORD_LEN`] bytes.
fn read_frame(r: &[u8]) -> Frame {
    Frame {
        frame_id: u64_at(r, 0),
        parent_frame_id: u64_at(r, 8),
        root_extent: Meters::new(f64_at(r, 16)),
        max_depth: r[24],
        mass: Kilograms::new(f64_at(r, 32)),
        position: vec3_at(r, 40),
        velocity: vec3_at(r, 64),
        orientation: Quat::new(f64_at(r, 88), f64_at(r, 96), f64_at(r, 104), f64_at(r, 112)),
        angular_velocity: vec3_at(r, 120),
    }
}

/// Decodes and validates a registry.
///
/// Checks, in order, and reports the first failing rule: length at least 24
/// (601), magic (602), version (603), the reserved header fields (604, 605),
/// a finite epoch (606), length exactly `24 + 144 * frame_count` (607). Then
/// record by record: ids strictly ascending (612 if lower than the previous,
/// 613 if equal), reserved bytes (611), and the field rules (614 to 625).
/// Unsorted input is an error; the decoder never reorders records. See
/// `docs/errors.md`.
pub fn decode(bytes: &[u8]) -> Result<Registry, ValidationError> {
    if bytes.len() < HEADER_LEN {
        return Err(ValidationError::new(
            codes::REGISTRY_HEADER_TOO_SHORT,
            format!("length: {} bytes, header needs {HEADER_LEN}", bytes.len()),
        ));
    }
    if bytes[0..4] != MAGIC {
        return Err(ValidationError::new(
            codes::REGISTRY_BAD_MAGIC,
            format!(
                "magic: {:02x} {:02x} {:02x} {:02x} is not 33 47 52 47",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
        ));
    }
    let version = u16_at(bytes, 4);
    if version != VERSION {
        return Err(ValidationError::new(
            codes::REGISTRY_UNSUPPORTED_VERSION,
            format!("format_version: {version} is not {VERSION}"),
        ));
    }
    let reserved = u16_at(bytes, 6);
    if reserved != 0 {
        return Err(ValidationError::new(
            codes::REGISTRY_RESERVED_U16_NONZERO,
            format!("reserved u16 at offset 6: {reserved} is not 0"),
        ));
    }
    let reserved = u32_at(bytes, 20);
    if reserved != 0 {
        return Err(ValidationError::new(
            codes::REGISTRY_RESERVED_U32_NONZERO,
            format!("reserved u32 at offset 20: {reserved} is not 0"),
        ));
    }
    let epoch = f64_at(bytes, 8);
    check_epoch(epoch)?;
    let count = u32_at(bytes, 16);
    // u64 arithmetic: cannot overflow, and is the same on 32-bit targets.
    let want = HEADER_LEN as u64 + RECORD_LEN as u64 * u64::from(count);
    if bytes.len() as u64 != want {
        return Err(ValidationError::new(
            codes::REGISTRY_LENGTH_MISMATCH,
            format!(
                "length: {} bytes, frame_count {count} needs {want}",
                bytes.len()
            ),
        ));
    }
    let mut frames: Vec<Frame> = Vec::with_capacity(count as usize);
    for (i, r) in bytes[HEADER_LEN..]
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .enumerate()
    {
        let f = read_frame(r);
        if let Some(prev) = frames.last() {
            if f.frame_id < prev.frame_id {
                return Err(ValidationError::new(
                    codes::FRAME_NOT_SORTED,
                    format!(
                        "frame {i}: frame_id {} follows {}, records must ascend",
                        f.frame_id, prev.frame_id
                    ),
                ));
            }
            if f.frame_id == prev.frame_id {
                return Err(duplicate(i, f.frame_id));
            }
        }
        if r[25..32].iter().any(|&b| b != 0) {
            return Err(ValidationError::new(
                codes::FRAME_RESERVED_NONZERO,
                format!(
                    "frame {i} (frame_id {}): reserved bytes at offset 25 are not 0",
                    f.frame_id
                ),
            ));
        }
        check_frame(i, &f)?;
        frames.push(f);
    }
    Ok(Registry {
        epoch: Seconds::new(epoch),
        frames,
    })
}

/// Validates a registry: [`decode`] without the result.
pub fn validate(bytes: &[u8]) -> Result<(), ValidationError> {
    decode(bytes).map(|_| ())
}

/// The union of a build's registries: one tree of frames under one root.
///
/// Built only by [`FrameTree::from_registries`], so it always satisfies the
/// cross-registry rules of section 5.2.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameTree {
    epoch: Seconds,
    /// Every frame of the union, sorted by `frame_id`.
    frames: Vec<Frame>,
    /// Index of the root in `frames`.
    root: usize,
    /// For each frame, the index of its parent in `frames`; `None` for the
    /// root.
    parents: Vec<Option<usize>>,
    /// For each frame, its children's ids, sorted ascending.
    children: Vec<Vec<u64>>,
    /// For each frame, its distance from the root in edges.
    depths: Vec<usize>,
}

impl FrameTree {
    /// Takes the union of `registries` and checks the rules of section 5.2
    /// across it.
    ///
    /// Checks, in order, and reports the first failing rule: every registry
    /// has the bitwise same epoch (651), no id is declared twice (652),
    /// exactly one root (653 if none, 654 if several), every parent is in the
    /// union (655), and every frame reaches the root by following parents,
    /// so there is no cycle (656). Empty registries are allowed and still
    /// take part in the epoch check. A union with no frames at all, including
    /// an empty slice, has no root and fails with 653.
    pub fn from_registries(registries: &[Registry]) -> Result<FrameTree, ValidationError> {
        let epoch = registries.first().map_or(Seconds::new(0.0), |r| r.epoch());
        for (i, r) in registries.iter().enumerate() {
            if r.epoch().value().to_bits() != epoch.value().to_bits() {
                return Err(ValidationError::new(
                    codes::UNION_EPOCH_MISMATCH,
                    format!(
                        "registry {i}: epoch {:e} differs from registry 0 epoch {:e}",
                        r.epoch().value(),
                        epoch.value()
                    ),
                ));
            }
        }

        let mut frames: Vec<Frame> = registries
            .iter()
            .flat_map(|r| r.frames().iter().copied())
            .collect();
        frames.sort_by_key(|f| f.frame_id);
        if let Some(w) = frames.windows(2).find(|w| w[0].frame_id == w[1].frame_id) {
            return Err(ValidationError::new(
                codes::UNION_DUPLICATE_ID,
                format!(
                    "frame_id {} is declared by more than one registry",
                    w[0].frame_id
                ),
            ));
        }

        let roots: Vec<usize> = (0..frames.len()).filter(|&i| frames[i].is_root()).collect();
        let root = match roots.as_slice() {
            [] => {
                return Err(ValidationError::new(
                    codes::UNION_NO_ROOT,
                    format!("no root frame among {} frames", frames.len()),
                ))
            }
            [only] => *only,
            [a, b, ..] => {
                return Err(ValidationError::new(
                    codes::UNION_MULTIPLE_ROOTS,
                    format!(
                        "{} root frames, the first two are frame_id {} and {}",
                        roots.len(),
                        frames[*a].frame_id,
                        frames[*b].frame_id
                    ),
                ))
            }
        };

        let find = |id: u64| frames.binary_search_by_key(&id, |f| f.frame_id).ok();
        let mut parents = Vec::with_capacity(frames.len());
        for f in &frames {
            if f.is_root() {
                parents.push(None);
                continue;
            }
            match find(f.parent_frame_id) {
                Some(p) => parents.push(Some(p)),
                None => {
                    return Err(ValidationError::new(
                        codes::UNION_PARENT_MISSING,
                        format!(
                            "frame_id {}: parent_frame_id {} is not in the union",
                            f.frame_id, f.parent_frame_id
                        ),
                    ))
                }
            }
        }

        // Children lists come out sorted because frames are visited in id
        // order.
        let mut children: Vec<Vec<u64>> = vec![Vec::new(); frames.len()];
        for (i, p) in parents.iter().enumerate() {
            if let Some(p) = *p {
                children[p].push(frames[i].frame_id);
            }
        }

        // Breadth first from the root. With one root and every parent
        // present, a frame the walk never reaches sits on, or hangs below, a
        // cycle of parents.
        let mut depths = vec![usize::MAX; frames.len()];
        depths[root] = 0;
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let at = queue[head];
            head += 1;
            for &c in &children[at] {
                let ci = find(c).expect("children are frames of the union");
                depths[ci] = depths[at] + 1;
                queue.push(ci);
            }
        }
        if let Some(i) = depths.iter().position(|&d| d == usize::MAX) {
            return Err(ValidationError::new(
                codes::UNION_CYCLE,
                format!(
                    "frame_id {}: following parents never reaches the root frame_id {}, the parents form a cycle",
                    frames[i].frame_id, frames[root].frame_id
                ),
            ));
        }

        Ok(FrameTree {
            epoch,
            frames,
            root,
            parents,
            children,
            depths,
        })
    }

    fn index(&self, frame_id: u64) -> Option<usize> {
        self.frames
            .binary_search_by_key(&frame_id, |f| f.frame_id)
            .ok()
    }

    /// The common epoch of every registry, seconds of Barycentric Dynamical
    /// Time since J2000.
    pub fn epoch(&self) -> Seconds {
        self.epoch
    }

    /// The one root frame.
    pub fn root(&self) -> &Frame {
        &self.frames[self.root]
    }

    /// The frame with this id, or `None` if it is not in the union.
    pub fn get(&self, frame_id: u64) -> Option<&Frame> {
        self.index(frame_id).map(|i| &self.frames[i])
    }

    /// The parent of the frame with this id, or `None` for the root and for
    /// an id not in the union.
    pub fn parent(&self, frame_id: u64) -> Option<&Frame> {
        let i = self.index(frame_id)?;
        self.parents[i].map(|p| &self.frames[p])
    }

    /// The ids of the frame's children, sorted ascending. Empty for a leaf
    /// and for an id not in the union.
    pub fn children(&self, frame_id: u64) -> &[u64] {
        self.index(frame_id)
            .map_or(&[], |i| self.children[i].as_slice())
    }

    /// The ids from `frame_id` up to the root, both included: `frame_id`
    /// first, the root last. Empty for an id not in the union.
    pub fn path_to_root(&self, frame_id: u64) -> Vec<u64> {
        let mut path = Vec::new();
        let mut at = self.index(frame_id);
        while let Some(i) = at {
            path.push(self.frames[i].frame_id);
            at = self.parents[i];
        }
        path
    }

    /// The number of parent steps from the frame to the root: 0 for the
    /// root. Also 0 for an id not in the union, so check with
    /// [`FrameTree::get`] when that matters.
    pub fn depth(&self, frame_id: u64) -> usize {
        self.index(frame_id).map_or(0, |i| self.depths[i])
    }

    /// Every frame in the union, sorted by `frame_id` ascending.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(id: u64) -> Frame {
        Frame {
            frame_id: id,
            parent_frame_id: ROOT_PARENT,
            root_extent: Meters::new(1.0e13),
            max_depth: 12,
            mass: Kilograms::new(2.0e30),
            position: Vec3::zero(),
            velocity: Vec3::zero(),
            orientation: Quat::new(0.0, 0.0, 0.6, 0.8),
            angular_velocity: Vec3::new(0.0, 0.0, 2.9e-6),
        }
    }

    fn child(id: u64, parent: u64) -> Frame {
        Frame {
            frame_id: id,
            parent_frame_id: parent,
            root_extent: Meters::new(1.0e8),
            max_depth: 20,
            mass: Kilograms::new(6.0e24),
            position: Vec3::new(1.5e11, -2.0e9, 0.0),
            velocity: Vec3::new(0.0, 2.98e4, 1.0),
            orientation: Quat::new(0.48, 0.6, 0.0, 0.64),
            angular_velocity: Vec3::new(0.0, 0.0, 7.3e-5),
        }
    }

    fn epoch() -> Seconds {
        Seconds::new(8.0e8)
    }

    fn reg(frames: Vec<Frame>) -> Registry {
        Registry::new(epoch(), frames).unwrap()
    }

    fn code<T: core::fmt::Debug>(r: Result<T, ValidationError>) -> u16 {
        r.unwrap_err().code
    }

    #[test]
    fn new_sorts_and_round_trips() {
        let r = reg(vec![child(30, 1), root(1), child(10, 1)]);
        let ids: Vec<u64> = r.frames().iter().map(|f| f.frame_id).collect();
        assert_eq!(ids, [1, 10, 30]);
        let b = encode(&r);
        assert_eq!(b.len(), HEADER_LEN + 3 * RECORD_LEN);
        assert_eq!(&b[0..4], &MAGIC);
        assert_eq!(u32_at(&b, 16), 3);
        assert_eq!(decode(&b).unwrap(), r);
        assert_eq!(encode(&decode(&b).unwrap()), b);
    }

    #[test]
    fn empty_is_header_only() {
        let r = Registry::empty(epoch()).unwrap();
        let b = encode(&r);
        assert_eq!(b.len(), HEADER_LEN);
        assert!(decode(&b).unwrap().is_empty());
    }

    #[test]
    fn record_layout() {
        let b = encode(&reg(vec![child(10, 1)]));
        let r = &b[HEADER_LEN..];
        assert_eq!(u64_at(r, 0), 10);
        assert_eq!(u64_at(r, 8), 1);
        assert_eq!(f64_at(r, 16), 1.0e8);
        assert_eq!(r[24], 20);
        assert!(r[25..32].iter().all(|&x| x == 0));
        assert_eq!(f64_at(r, 32), 6.0e24);
        assert_eq!(f64_at(r, 40), 1.5e11);
        assert_eq!(f64_at(r, 72), 2.98e4);
        assert_eq!(f64_at(r, 88), 0.48);
        assert_eq!(f64_at(r, 112), 0.64);
        assert_eq!(f64_at(r, 136), 7.3e-5);
    }

    type Edit = fn(&mut Frame);

    #[test]
    fn new_rejects_each_field_rule() {
        let cases: [(u16, Edit); 12] = [
            (614, |f| f.root_extent = Meters::new(f64::NAN)),
            (615, |f| f.root_extent = Meters::new(0.0)),
            (616, |f| f.max_depth = 32),
            (617, |f| f.mass = Kilograms::new(f64::INFINITY)),
            (618, |f| f.mass = Kilograms::new(-1.0)),
            (619, |f| f.position.y = f64::NAN),
            (620, |f| f.velocity.z = f64::NEG_INFINITY),
            (621, |f| f.orientation.w = f64::NAN),
            (622, |f| {
                f.orientation = Quat::new(0.0, 0.0, 0.0, 1.0 + 2e-9)
            }),
            (623, |f| f.angular_velocity.x = f64::NAN),
            (624, |f| {
                f.parent_frame_id = ROOT_PARENT;
                f.velocity = Vec3::zero();
            }),
            (625, |f| {
                f.parent_frame_id = ROOT_PARENT;
                f.position = Vec3::zero();
            }),
        ];
        for (want, edit) in cases {
            let mut f = child(10, 1);
            edit(&mut f);
            assert_eq!(code(Registry::new(epoch(), vec![f])), want);
        }
        assert_eq!(code(Registry::new(epoch(), vec![root(1), root(1)])), 613);
        assert_eq!(code(Registry::empty(Seconds::new(f64::NAN))), 606);
    }

    #[test]
    fn unit_tolerance_is_inclusive_enough() {
        let mut f = child(10, 1);
        f.orientation = Quat::new(0.0, 0.0, 0.0, 1.0 + 5e-10);
        assert!(Registry::new(epoch(), vec![f]).is_ok());
    }

    #[test]
    fn root_allows_negative_zero() {
        let mut f = root(1);
        f.position = Vec3::new(-0.0, 0.0, -0.0);
        assert!(Registry::new(epoch(), vec![f]).is_ok());
    }

    #[test]
    fn decode_rejects_header_and_order() {
        let good = encode(&reg(vec![root(1), child(10, 1)]));
        assert_eq!(code(decode(&good[..23])), 601);
        let edit = |at: usize, v: u8| {
            let mut b = good.clone();
            b[at] = v;
            code(decode(&b))
        };
        assert_eq!(edit(3, 0x53), 602);
        assert_eq!(edit(4, 2), 603);
        assert_eq!(edit(7, 1), 604);
        assert_eq!(edit(23, 1), 605);
        assert_eq!(edit(16, 3), 607);
        assert_eq!(edit(HEADER_LEN + 31, 1), 611);
        let mut nan = good.clone();
        nan[8..16].copy_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(code(decode(&nan)), 606);

        let mut swapped = good[..HEADER_LEN].to_vec();
        swapped.extend_from_slice(&good[HEADER_LEN + RECORD_LEN..]);
        swapped.extend_from_slice(&good[HEADER_LEN..HEADER_LEN + RECORD_LEN]);
        assert_eq!(code(decode(&swapped)), 612);
        let mut twice = good[..HEADER_LEN + RECORD_LEN].to_vec();
        twice.extend_from_slice(&good[HEADER_LEN..HEADER_LEN + RECORD_LEN]);
        assert_eq!(code(decode(&twice)), 613);
    }

    #[test]
    fn tree_queries() {
        let a = reg(vec![root(1), child(10, 1), child(20, 1)]);
        let b = reg(vec![child(11, 10), child(5, 1)]);
        let empty = Registry::empty(epoch()).unwrap();
        let t = FrameTree::from_registries(&[a, empty, b]).unwrap();
        assert_eq!(t.root().frame_id, 1);
        assert_eq!(t.epoch(), epoch());
        assert_eq!(t.children(1), &[5, 10, 20]);
        assert_eq!(t.children(10), &[11]);
        assert!(t.children(99).is_empty());
        assert_eq!(t.path_to_root(11), [11, 10, 1]);
        assert_eq!(t.path_to_root(1), [1]);
        assert!(t.path_to_root(99).is_empty());
        assert_eq!(t.depth(11), 2);
        assert_eq!(t.depth(1), 0);
        assert_eq!(t.parent(11).unwrap().frame_id, 10);
        assert!(t.parent(1).is_none());
        assert_eq!(t.get(20).unwrap().frame_id, 20);
        let ids: Vec<u64> = t.frames().iter().map(|f| f.frame_id).collect();
        assert_eq!(ids, [1, 5, 10, 11, 20]);
    }

    #[test]
    fn tree_rejects_each_union_rule() {
        let other = Registry::new(Seconds::new(8.0e8 + 1.0), vec![]).unwrap();
        assert_eq!(
            code(FrameTree::from_registries(&[reg(vec![root(1)]), other])),
            651
        );
        let pos = Registry::empty(Seconds::new(0.0)).unwrap();
        let neg = Registry::empty(Seconds::new(-0.0)).unwrap();
        assert_eq!(code(FrameTree::from_registries(&[pos, neg])), 651);
        assert_eq!(
            code(FrameTree::from_registries(&[
                reg(vec![root(1), child(10, 1)]),
                reg(vec![child(10, 1)])
            ])),
            652
        );
        assert_eq!(code(FrameTree::from_registries(&[])), 653);
        assert_eq!(code(FrameTree::from_registries(&[reg(vec![])])), 653);
        assert_eq!(
            code(FrameTree::from_registries(&[
                reg(vec![root(1)]),
                reg(vec![root(2)])
            ])),
            654
        );
        assert_eq!(
            code(FrameTree::from_registries(&[reg(vec![
                root(1),
                child(10, 7)
            ])])),
            655
        );
        assert_eq!(
            code(FrameTree::from_registries(&[
                reg(vec![root(1)]),
                reg(vec![child(20, 21), child(21, 20), child(22, 21)])
            ])),
            656
        );
        assert_eq!(
            code(FrameTree::from_registries(&[reg(vec![
                root(1),
                child(9, 9)
            ])])),
            656
        );
    }
}
