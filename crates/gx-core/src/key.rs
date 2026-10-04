//! Chunk keys and cell geometry.
//!
//! Implements `matter-format.md` section 2 (chunk keys) and section 3.1 (cell
//! geometry).
//!
//! A chunk key is either the literal `registry`, naming the frame registry, or
//! five decimal integers joined by `-`:
//! `frame_id-depth-x-y-z`. Each part is ASCII digits only with no sign, no
//! whitespace, and no leading zero (a bare `0` is allowed). `frame_id` is a
//! `u64`, `depth` is at most 31, and each coordinate is below `2^depth`. Any
//! other string is rejected with a specific [`KeyError`]. Parsing and printing
//! round trip exactly.
//!
//! A cell at `depth` divides its frame's root cube of edge `root_extent`,
//! centered on the frame origin, into `2^depth` cells per axis.

use core::fmt;
use core::str::FromStr;

use crate::units::{Meters, Vec3};

/// The reserved chunk key naming the frame registry.
pub const REGISTRY_KEY: &str = "registry";

/// Largest permitted cell depth.
pub const MAX_DEPTH: u8 = 31;

/// Number of `-` separated parts in a cell key.
const CELL_PARTS: usize = 5;

/// A parsed chunk key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChunkKey {
    /// The frame registry, written as [`REGISTRY_KEY`].
    Registry,
    /// One cell of one frame's octree.
    Cell(CellKey),
}

/// The address of one cell: a frame, an octree depth, and integer coordinates
/// at that depth.
///
/// The fields are public. Values built directly are not checked; use
/// [`CellKey::new`] or [`CellKey::is_valid`] to enforce section 2.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey {
    /// Integer id of the frame the cell belongs to.
    pub frame_id: u64,
    /// Octree depth, from 0 (the whole root cube) to [`MAX_DEPTH`].
    pub depth: u8,
    /// Cell index along x, below `2^depth`.
    pub x: u32,
    /// Cell index along y, below `2^depth`.
    pub y: u32,
    /// Cell index along z, below `2^depth`.
    pub z: u32,
}

/// The axis aligned cube a cell occupies, in its frame's coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CellGeometry {
    /// Minimum corner of the cube, in meters, frame coordinates.
    pub origin: Vec3,
    /// Edge length of the cube.
    pub edge: Meters,
}

/// Why a string is not a valid chunk key.
///
/// Part indices are zero based: 0 is `frame_id`, 1 is `depth`, 2 to 4 are
/// `x`, `y`, `z`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The input is the empty string.
    Empty,
    /// The input contains a character other than ASCII `0` to `9` and `-`
    /// (and is not exactly `registry`). Covers signs, whitespace, letters,
    /// non ASCII digits, and any other spelling of `registry`.
    InvalidCharacter {
        /// Byte offset of the first offending character.
        offset: usize,
    },
    /// The input does not have exactly five `-` separated parts.
    PartCount {
        /// Number of parts found.
        found: usize,
    },
    /// A part is empty, from a leading, trailing, or doubled `-`.
    EmptyPart {
        /// Index of the empty part.
        part: usize,
    },
    /// A part has more than one digit and starts with `0`.
    LeadingZero {
        /// Index of the offending part.
        part: usize,
    },
    /// A part does not fit its type: `u64` for `frame_id` and `depth`, `u32`
    /// for coordinates.
    Overflow {
        /// Index of the offending part.
        part: usize,
    },
    /// `depth` is greater than [`MAX_DEPTH`].
    DepthOutOfRange {
        /// The depth found.
        depth: u64,
    },
    /// A coordinate is not below `2^depth`.
    CoordinateOutOfRange {
        /// Index of the offending part.
        part: usize,
        /// The coordinate found.
        value: u32,
        /// The cell depth.
        depth: u8,
    },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            KeyError::Empty => write!(f, "empty key"),
            KeyError::InvalidCharacter { offset } => {
                write!(f, "invalid character at byte {offset}")
            }
            KeyError::PartCount { found } => {
                write!(f, "expected {CELL_PARTS} parts, found {found}")
            }
            KeyError::EmptyPart { part } => write!(f, "part {part} is empty"),
            KeyError::LeadingZero { part } => write!(f, "part {part} has a leading zero"),
            KeyError::Overflow { part } => write!(f, "part {part} overflows"),
            KeyError::DepthOutOfRange { depth } => {
                write!(f, "depth {depth} exceeds {MAX_DEPTH}")
            }
            KeyError::CoordinateOutOfRange { part, value, depth } => {
                write!(f, "part {part} value {value} not below 2^{depth}")
            }
        }
    }
}

impl std::error::Error for KeyError {}

/// Parses one part as a decimal `u64`, enforcing the digit rules.
fn parse_part(s: &str, part: usize) -> Result<u64, KeyError> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Err(KeyError::EmptyPart { part });
    }
    if bytes.len() > 1 && bytes[0] == b'0' {
        return Err(KeyError::LeadingZero { part });
    }
    let mut v: u64 = 0;
    for &b in bytes {
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(u64::from(b - b'0')))
            .ok_or(KeyError::Overflow { part })?;
    }
    Ok(v)
}

impl FromStr for ChunkKey {
    type Err = KeyError;

    /// Parses a chunk key per section 2. Checks run in this order: empty
    /// input, the `registry` literal, characters, part count, then each part
    /// left to right (empty, leading zero, overflow, range).
    fn from_str(s: &str) -> Result<Self, KeyError> {
        if s.is_empty() {
            return Err(KeyError::Empty);
        }
        if s == REGISTRY_KEY {
            return Ok(ChunkKey::Registry);
        }
        if let Some(offset) = s.bytes().position(|b| !(b.is_ascii_digit() || b == b'-')) {
            return Err(KeyError::InvalidCharacter { offset });
        }
        let found = s.split('-').count();
        if found != CELL_PARTS {
            return Err(KeyError::PartCount { found });
        }
        let mut values = [0u64; CELL_PARTS];
        for (part, text) in s.split('-').enumerate() {
            values[part] = parse_part(text, part)?;
        }
        let depth = values[1];
        if depth > u64::from(MAX_DEPTH) {
            return Err(KeyError::DepthOutOfRange { depth });
        }
        let depth = depth as u8;
        let mut coords = [0u32; 3];
        for (i, coord) in coords.iter_mut().enumerate() {
            let part = i + 2;
            let v = u32::try_from(values[part]).map_err(|_| KeyError::Overflow { part })?;
            if u64::from(v) >= 1u64 << depth {
                return Err(KeyError::CoordinateOutOfRange {
                    part,
                    value: v,
                    depth,
                });
            }
            *coord = v;
        }
        Ok(ChunkKey::Cell(CellKey {
            frame_id: values[0],
            depth,
            x: coords[0],
            y: coords[1],
            z: coords[2],
        }))
    }
}

impl fmt::Display for ChunkKey {
    /// Writes the canonical form: `registry`, or `frame_id-depth-x-y-z`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkKey::Registry => f.write_str(REGISTRY_KEY),
            ChunkKey::Cell(c) => fmt::Display::fmt(c, f),
        }
    }
}

impl fmt::Display for CellKey {
    /// Writes `frame_id-depth-x-y-z` in decimal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{}-{}-{}-{}",
            self.frame_id, self.depth, self.x, self.y, self.z
        )
    }
}

impl From<CellKey> for ChunkKey {
    fn from(c: CellKey) -> Self {
        ChunkKey::Cell(c)
    }
}

impl CellKey {
    /// Builds a cell key, checking `depth <= 31` and each coordinate below
    /// `2^depth`.
    pub fn new(frame_id: u64, depth: u8, x: u32, y: u32, z: u32) -> Result<Self, KeyError> {
        if depth > MAX_DEPTH {
            return Err(KeyError::DepthOutOfRange {
                depth: u64::from(depth),
            });
        }
        for (i, &value) in [x, y, z].iter().enumerate() {
            if u64::from(value) >= 1u64 << depth {
                return Err(KeyError::CoordinateOutOfRange {
                    part: i + 2,
                    value,
                    depth,
                });
            }
        }
        Ok(Self {
            frame_id,
            depth,
            x,
            y,
            z,
        })
    }

    /// Returns `true` if the fields satisfy section 2.
    pub fn is_valid(&self) -> bool {
        Self::new(self.frame_id, self.depth, self.x, self.y, self.z).is_ok()
    }

    /// The cube this cell occupies, per section 3.1.
    ///
    /// `edge = root_extent / 2^depth` and, per axis,
    /// `origin = -root_extent / 2 + edge * index`, evaluated in `f64` in
    /// exactly that order so every implementation agrees bit for bit.
    pub fn geometry(&self, root_extent: Meters) -> CellGeometry {
        let r = root_extent.value();
        let edge = r / ((1u64 << self.depth) as f64);
        let half = -r / 2.0;
        CellGeometry {
            origin: Vec3::new(
                half + edge * f64::from(self.x),
                half + edge * f64::from(self.y),
                half + edge * f64::from(self.z),
            ),
            edge: Meters::new(edge),
        }
    }

    /// The cell one depth up that contains this one, or `None` at depth 0.
    pub fn parent(&self) -> Option<CellKey> {
        if self.depth == 0 {
            return None;
        }
        Some(CellKey {
            frame_id: self.frame_id,
            depth: self.depth - 1,
            x: self.x >> 1,
            y: self.y >> 1,
            z: self.z >> 1,
        })
    }

    /// The eight cells one depth down, or `None` at depth 31.
    ///
    /// Child `i` has offset `(i & 1, (i >> 1) & 1, (i >> 2) & 1)` from
    /// `2 * (x, y, z)`, so x varies fastest, then y, then z.
    pub fn children(&self) -> Option<[CellKey; 8]> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        Some(core::array::from_fn(|i| {
            let i = i as u32;
            CellKey {
                frame_id: self.frame_id,
                depth: self.depth + 1,
                x: (self.x << 1) | (i & 1),
                y: (self.y << 1) | ((i >> 1) & 1),
                z: (self.z << 1) | ((i >> 2) & 1),
            }
        }))
    }

    /// Returns `true` if `p` (meters, frame coordinates) lies in this cell,
    /// using the half open interval `[origin, origin + edge)` on each axis.
    pub fn contains_point(&self, root_extent: Meters, p: Vec3) -> bool {
        let g = self.geometry(root_extent);
        let e = g.edge.value();
        let inside = |o: f64, v: f64| o <= v && v < o + e;
        inside(g.origin.x, p.x) && inside(g.origin.y, p.y) && inside(g.origin.z, p.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<ChunkKey, KeyError> {
        s.parse()
    }

    fn cell(frame_id: u64, depth: u8, x: u32, y: u32, z: u32) -> CellKey {
        CellKey {
            frame_id,
            depth,
            x,
            y,
            z,
        }
    }

    #[test]
    fn registry_literal() {
        assert_eq!(parse("registry"), Ok(ChunkKey::Registry));
        assert_eq!(ChunkKey::Registry.to_string(), REGISTRY_KEY);
        assert_eq!(
            parse("Registry"),
            Err(KeyError::InvalidCharacter { offset: 0 })
        );
        assert_eq!(
            parse("registry "),
            Err(KeyError::InvalidCharacter { offset: 0 })
        );
    }

    #[test]
    fn valid_cells_round_trip() {
        for s in [
            "0-0-0-0-0",
            "1-1-1-0-1",
            "18446744073709551615-31-2147483647-2147483647-2147483647",
            "42-3-5-2-7",
            "7-31-0-0-0",
        ] {
            let k = parse(s).unwrap();
            assert_eq!(k.to_string(), s);
        }
        assert_eq!(
            parse("42-3-5-2-7"),
            Ok(ChunkKey::Cell(cell(42, 3, 5, 2, 7)))
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse(""), Err(KeyError::Empty));
    }

    #[test]
    fn invalid_characters() {
        assert_eq!(
            parse("+1-0-0-0-0"),
            Err(KeyError::InvalidCharacter { offset: 0 })
        );
        assert_eq!(
            parse("1-0-0-0-0 "),
            Err(KeyError::InvalidCharacter { offset: 9 })
        );
        assert_eq!(
            parse("1 -0-0-0-0"),
            Err(KeyError::InvalidCharacter { offset: 1 })
        );
        assert_eq!(
            parse("1-0-0-0-\u{0663}"),
            Err(KeyError::InvalidCharacter { offset: 8 })
        );
        assert_eq!(
            parse("0x1-0-0-0-0"),
            Err(KeyError::InvalidCharacter { offset: 1 })
        );
        assert_eq!(
            parse("1.0-0-0-0-0"),
            Err(KeyError::InvalidCharacter { offset: 1 })
        );
    }

    #[test]
    fn part_count() {
        assert_eq!(parse("1-0-0-0"), Err(KeyError::PartCount { found: 4 }));
        assert_eq!(parse("1-0-0-0-0-0"), Err(KeyError::PartCount { found: 6 }));
        assert_eq!(parse("7"), Err(KeyError::PartCount { found: 1 }));
        assert_eq!(parse("-"), Err(KeyError::PartCount { found: 2 }));
    }

    #[test]
    fn empty_parts() {
        assert_eq!(parse("1-0--0-0"), Err(KeyError::EmptyPart { part: 2 }));
        assert_eq!(parse("-1-0-0-0"), Err(KeyError::EmptyPart { part: 0 }));
        assert_eq!(parse("1-0-0-0-"), Err(KeyError::EmptyPart { part: 4 }));
        assert_eq!(parse("----"), Err(KeyError::EmptyPart { part: 0 }));
    }

    #[test]
    fn leading_zeros() {
        assert_eq!(parse("01-0-0-0-0"), Err(KeyError::LeadingZero { part: 0 }));
        assert_eq!(parse("1-00-0-0-0"), Err(KeyError::LeadingZero { part: 1 }));
        assert_eq!(parse("1-2-01-0-0"), Err(KeyError::LeadingZero { part: 2 }));
    }

    #[test]
    fn overflow() {
        assert_eq!(
            parse("18446744073709551616-0-0-0-0"),
            Err(KeyError::Overflow { part: 0 })
        );
        assert_eq!(
            parse("1-99999999999999999999-0-0-0"),
            Err(KeyError::Overflow { part: 1 })
        );
        assert_eq!(
            parse("1-31-4294967296-0-0"),
            Err(KeyError::Overflow { part: 2 })
        );
        assert_eq!(
            parse("1-31-0-0-18446744073709551616"),
            Err(KeyError::Overflow { part: 4 })
        );
    }

    #[test]
    fn depth_range() {
        assert_eq!(
            parse("1-32-0-0-0"),
            Err(KeyError::DepthOutOfRange { depth: 32 })
        );
        assert_eq!(
            parse("1-255-0-0-0"),
            Err(KeyError::DepthOutOfRange { depth: 255 })
        );
    }

    #[test]
    fn coordinate_range() {
        assert_eq!(
            parse("1-0-1-0-0"),
            Err(KeyError::CoordinateOutOfRange {
                part: 2,
                value: 1,
                depth: 0
            })
        );
        assert_eq!(
            parse("1-3-0-8-0"),
            Err(KeyError::CoordinateOutOfRange {
                part: 3,
                value: 8,
                depth: 3
            })
        );
        assert_eq!(
            parse("1-31-0-0-2147483648"),
            Err(KeyError::CoordinateOutOfRange {
                part: 4,
                value: 2147483648,
                depth: 31
            })
        );
    }

    #[test]
    fn new_and_is_valid() {
        assert_eq!(CellKey::new(1, 3, 7, 7, 7), Ok(cell(1, 3, 7, 7, 7)));
        assert_eq!(
            CellKey::new(1, 32, 0, 0, 0),
            Err(KeyError::DepthOutOfRange { depth: 32 })
        );
        assert_eq!(
            CellKey::new(1, 2, 0, 4, 0),
            Err(KeyError::CoordinateOutOfRange {
                part: 3,
                value: 4,
                depth: 2
            })
        );
        assert!(cell(0, 0, 0, 0, 0).is_valid());
        assert!(!cell(0, 0, 1, 0, 0).is_valid());
    }

    #[test]
    fn errors_display() {
        assert_eq!(KeyError::Empty.to_string(), "empty key");
        assert_eq!(
            KeyError::PartCount { found: 4 }.to_string(),
            "expected 5 parts, found 4"
        );
    }

    #[test]
    fn geometry_example() {
        let g = cell(9, 3, 5, 2, 7).geometry(Meters::new(16.0));
        assert_eq!(g.edge, Meters::new(2.0));
        assert_eq!(g.origin, Vec3::new(2.0, -4.0, 6.0));
    }

    #[test]
    fn geometry_root_and_deep() {
        let g = cell(0, 0, 0, 0, 0).geometry(Meters::new(10.0));
        assert_eq!(g.edge, Meters::new(10.0));
        assert_eq!(g.origin, Vec3::new(-5.0, -5.0, -5.0));
        let m = (1u32 << 31) - 1;
        let g = cell(0, 31, m, 0, m).geometry(Meters::new(4294967296.0));
        assert_eq!(g.edge, Meters::new(2.0));
        assert_eq!(
            g.origin,
            Vec3::new(2147483646.0, -2147483648.0, 2147483646.0)
        );
    }

    #[test]
    fn parent_and_children() {
        assert_eq!(cell(4, 0, 0, 0, 0).parent(), None);
        assert_eq!(cell(4, 3, 5, 2, 7).parent(), Some(cell(4, 2, 2, 1, 3)));
        let m = (1u32 << 31) - 1;
        assert_eq!(cell(4, 31, m, 0, m).children(), None);
        let kids = cell(4, 1, 1, 0, 1).children().unwrap();
        let expect = [
            cell(4, 2, 2, 0, 2),
            cell(4, 2, 3, 0, 2),
            cell(4, 2, 2, 1, 2),
            cell(4, 2, 3, 1, 2),
            cell(4, 2, 2, 0, 3),
            cell(4, 2, 3, 0, 3),
            cell(4, 2, 2, 1, 3),
            cell(4, 2, 3, 1, 3),
        ];
        assert_eq!(kids, expect);
        for k in kids {
            assert!(k.is_valid());
            assert_eq!(k.parent(), Some(cell(4, 1, 1, 0, 1)));
        }
        let deepest = cell(4, 30, (1 << 30) - 1, 0, 0).children().unwrap();
        assert!(deepest.iter().all(CellKey::is_valid));
    }

    #[test]
    fn contains_point_half_open() {
        let r = Meters::new(16.0);
        let c = cell(1, 3, 5, 2, 7);
        assert!(c.contains_point(r, Vec3::new(2.0, -4.0, 6.0)));
        assert!(c.contains_point(r, Vec3::new(3.9, -2.1, 7.5)));
        assert!(!c.contains_point(r, Vec3::new(4.0, -3.0, 7.0)));
        assert!(!c.contains_point(r, Vec3::new(3.0, -2.0, 7.0)));
        assert!(!c.contains_point(r, Vec3::new(3.0, -3.0, 8.0)));
        assert!(!c.contains_point(r, Vec3::new(1.999, -3.0, 7.0)));
        assert!(!c.contains_point(r, Vec3::new(f64::NAN, -3.0, 7.0)));
        // Every point lies in exactly one child of the root.
        let root = cell(1, 0, 0, 0, 0);
        let p = Vec3::new(0.0, -0.5, 7.9);
        let hits = root
            .children()
            .unwrap()
            .iter()
            .filter(|k| k.contains_point(r, p))
            .count();
        assert_eq!(hits, 1);
    }
}
