//! Surface tiles: a frame's outer surface as heights over a reference shape.
//!
//! Implements `matter-format.md` section 10. A surface is the boundary of a
//! frame's solid matter given as heights above a reference ellipsoid, cut
//! into tiles on the six faces of a cube projected onto that ellipsoid. A
//! compiler that owns a frame's matter may emit tiles; the renderer draws
//! them for any frame that has them. Nothing here knows what the frame is.
//!
//! # Geometry (section 10.1)
//!
//! Face `f` has a normal `n` and tangents `u`, `v` with `u x v = n`
//! ([`face_axes`]). Tile `(x, y)` at level `l` has [`TILE_QUADS`] quads per
//! side; its vertex `(i, j)` has the face coordinates
//! `s = 2 g / (TILE_QUADS 2^l) - 1` with `g = x TILE_QUADS + i` (and the same
//! for `t`, `y`, `j`), exact in `f64` ([`face_coord`]). The direction is
//! `n + s u + t v` normalized; the reference point is where that direction
//! meets the ellipsoid of radius `a` in frame x and y (`radius_xy`) and
//! radius `c` along frame z (`radius_z`); the vertex is that point moved `h` along the
//! ellipsoid's outward normal there ([`vertex_position`]). Only `+`, `-`,
//! `*`, `/`, and `sqrt` are used, so every machine computes the same bits,
//! and a vertex shared by two tiles (or by a tile and its parent) has the
//! same coordinates in both.

use crate::error::{codes, ValidationError};
use crate::key::SurfaceKey;
use crate::matter::{decompress, Compression, ZSTD_LEVEL};

/// Quads per tile side.
pub const TILE_QUADS: usize = 64;

/// Vertices per tile side.
pub const TILE_VERTICES: usize = TILE_QUADS + 1;

/// Heights per side including the one-vertex border all around.
pub const HEIGHT_SIDE: usize = TILE_VERTICES + 2;

/// Parent heights per side: the tile's even vertices (`TILE_QUADS / 2 + 1`)
/// plus a border of one parent vertex all around.
pub const PARENT_SIDE: usize = TILE_QUADS / 2 + 1 + 2;

/// Surface tile format version.
pub const VERSION: u16 = 2;

/// Magic bytes `3GSF`.
pub const MAGIC: [u8; 4] = *b"3GSF";

/// Header length, bytes.
pub const HEADER_LEN: usize = 64;

/// Flag: no surface in this tile.
pub const FLAG_EMPTY: u16 = 1;

/// Flag: the block is one zstd frame.
pub const FLAG_ZSTD: u16 = 2;

/// Vertices of a tile (no border).
pub const VERTEX_COUNT: usize = TILE_VERTICES * TILE_VERTICES;

/// Block length, bytes (section 10.3).
pub const BLOCK_LEN: usize = 4 * HEIGHT_SIDE * HEIGHT_SIDE
    + 4 * PARENT_SIDE * PARENT_SIDE
    + 3 * VERTEX_COUNT
    + VERTEX_COUNT
    + 4 * VERTEX_COUNT;

/// One tile with a surface.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceTile {
    /// The tile.
    pub key: SurfaceKey,
    /// Radius of the reference ellipsoid in the frame's x and y axes, meters.
    pub radius_xy: f64,
    /// Radius of the reference ellipsoid along the frame's z axis, meters.
    pub radius_z: f64,
    /// The lowest height, meters, of the frame's whole surface: no height
    /// of any tile of the frame, at any level, is below it (section 10.2).
    /// A renderer may treat the reference shape lowered by it as opaque.
    pub height_floor: f64,
    /// Heights above the reference ellipsoid, meters, at vertices
    /// `(i, j)` for `i, j` in `-1 ..= TILE_QUADS + 1`, row major (`j`
    /// outer): [`HEIGHT_SIDE`] squared.
    pub heights: Vec<f32>,
    /// The parent tile's heights at this tile's even vertices `(2k, 2m)`
    /// for `k, m` in `-1 ..= TILE_QUADS / 2 + 1`, row major: what the
    /// surface is one level coarser ([`PARENT_SIDE`] squared). At level 0,
    /// the tile's own heights at those vertices.
    pub parent_heights: Vec<f32>,
    /// Reflectance per vertex in three bands, long to short wavelength, as
    /// `value / 255`.
    pub albedo: Vec<[u8; 3]>,
    /// Microfacet roughness per vertex, as `value / 255`.
    pub roughness: Vec<u8>,
    /// Radiating temperature per vertex, kelvin.
    pub temperature: Vec<f32>,
}

/// A decoded tile: a surface, or none.
#[derive(Clone, Debug, PartialEq)]
pub enum Tile {
    /// No surface in this tile.
    Empty(SurfaceKey),
    /// A surface.
    Surface(SurfaceTile),
}

impl SurfaceTile {
    /// The height at vertex `(i, j)`, `i, j` in `-1 ..= TILE_QUADS + 1`.
    pub fn height(&self, i: i64, j: i64) -> f32 {
        self.heights[(i + 1) as usize + HEIGHT_SIDE * (j + 1) as usize]
    }

    /// The parent's height at even vertex `(2k, 2m)`, `k, m` in
    /// `-1 ..= TILE_QUADS / 2 + 1`.
    pub fn parent_height(&self, k: i64, m: i64) -> f32 {
        self.parent_heights[(k + 1) as usize + PARENT_SIDE * (m + 1) as usize]
    }
}

/// Normal, `u`, and `v` of face `f` (section 10.1), `u x v = n`.
pub fn face_axes(face: u8) -> ([f64; 3], [f64; 3], [f64; 3]) {
    match face {
        0 => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        1 => ([-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
        2 => ([0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        3 => ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        4 => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        _ => ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
    }
}

/// The face coordinate, `-1` to `1`, of tile index `t` (column or row) at
/// `level` and vertex offset `i` (may be `-1` or past [`TILE_QUADS`] for
/// the border). Exact: a power-of-two denominator.
pub fn face_coord(level: u8, t: u32, i: i64) -> f64 {
    let g = i64::from(t) * TILE_QUADS as i64 + i;
    (2 * g) as f64 / ((TILE_QUADS as u64) << level) as f64 - 1.0
}

/// The unit direction from the frame origin through vertex `(i, j)` of
/// `key` (section 10.1).
pub fn vertex_direction(key: &SurfaceKey, i: i64, j: i64) -> [f64; 3] {
    let (n, u, v) = face_axes(key.face);
    let s = face_coord(key.level, key.x, i);
    let t = face_coord(key.level, key.y, j);
    let p: [f64; 3] = core::array::from_fn(|a| n[a] + s * u[a] + t * v[a]);
    let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    p.map(|x| x / len)
}

/// Where the unit direction `d` meets the ellipsoid of radius `a` in frame
/// x and y and `c` along frame z, and the ellipsoid's outward unit normal
/// there.
pub fn ellipsoid_point(d: [f64; 3], a: f64, c: f64) -> ([f64; 3], [f64; 3]) {
    let q = (d[0] * d[0] + d[1] * d[1]) / (a * a) + d[2] * d[2] / (c * c);
    let s = 1.0 / q.sqrt();
    let p = d.map(|x| x * s);
    let g = [p[0] / (a * a), p[1] / (a * a), p[2] / (c * c)];
    let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
    (p, g.map(|x| x / len))
}

/// The position in frame coordinates, meters, of vertex `(i, j)` of `key`
/// at height `h` above the ellipsoid of radii `a` and `c`.
pub fn vertex_position(key: &SurfaceKey, i: i64, j: i64, h: f64, a: f64, c: f64) -> [f64; 3] {
    let (p, n) = ellipsoid_point(vertex_direction(key, i, j), a, c);
    core::array::from_fn(|k| p[k] + h * n[k])
}

fn le_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))
}
fn le_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}
fn le_f64(b: &[u8], o: usize) -> f64 {
    f64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}

fn header(key: &SurfaceKey, flags: u16, a: f64, c: f64, floor: f64, block_len: u64) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER_LEN);
    h.extend_from_slice(&MAGIC);
    h.extend_from_slice(&VERSION.to_le_bytes());
    h.extend_from_slice(&flags.to_le_bytes());
    h.extend_from_slice(&key.frame_id.to_le_bytes());
    h.push(key.face);
    h.push(key.level);
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&key.x.to_le_bytes());
    h.extend_from_slice(&key.y.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(&a.to_le_bytes());
    h.extend_from_slice(&c.to_le_bytes());
    h.extend_from_slice(&floor.to_le_bytes());
    h.extend_from_slice(&block_len.to_le_bytes());
    debug_assert_eq!(h.len(), HEADER_LEN);
    h
}

/// The bytes of an empty tile (section 10.4).
pub fn encode_empty(key: &SurfaceKey) -> Vec<u8> {
    header(key, FLAG_EMPTY, 0.0, 0.0, 0.0, 0)
}

/// The bytes of `tile` (section 10.2 and 10.3), the block compressed or not.
///
/// # Panics
///
/// Panics if a channel has the wrong length.
pub fn encode(tile: &SurfaceTile, compression: Compression) -> Vec<u8> {
    assert_eq!(tile.heights.len(), HEIGHT_SIDE * HEIGHT_SIDE);
    assert_eq!(tile.parent_heights.len(), PARENT_SIDE * PARENT_SIDE);
    assert_eq!(tile.albedo.len(), VERTEX_COUNT);
    assert_eq!(tile.roughness.len(), VERTEX_COUNT);
    assert_eq!(tile.temperature.len(), VERTEX_COUNT);
    let mut block = Vec::with_capacity(BLOCK_LEN);
    for h in &tile.heights {
        block.extend_from_slice(&h.to_le_bytes());
    }
    for h in &tile.parent_heights {
        block.extend_from_slice(&h.to_le_bytes());
    }
    for a in &tile.albedo {
        block.extend_from_slice(a);
    }
    block.extend_from_slice(&tile.roughness);
    for t in &tile.temperature {
        block.extend_from_slice(&t.to_le_bytes());
    }
    let (flags, payload) = match compression {
        Compression::None => (0, block),
        Compression::Zstd => (
            FLAG_ZSTD,
            zstd::bulk::compress(&block, ZSTD_LEVEL).expect("zstd compresses in memory"),
        ),
    };
    let mut out = header(
        &tile.key,
        flags,
        tile.radius_xy,
        tile.radius_z,
        tile.height_floor,
        BLOCK_LEN as u64,
    );
    out.extend_from_slice(&payload);
    out
}

/// Checks `bytes` against `key` (section 10.5); the first failing rule.
pub fn validate(key: &SurfaceKey, bytes: &[u8]) -> Result<(), ValidationError> {
    decode(key, bytes).map(|_| ())
}

/// Decodes and validates `bytes` submitted under `key` (section 10.5).
pub fn decode(key: &SurfaceKey, bytes: &[u8]) -> Result<Tile, ValidationError> {
    let err = |code, reason: String| Err(ValidationError::new(code, reason));
    if bytes.len() < HEADER_LEN {
        return err(
            codes::SURFACE_TOO_SHORT,
            format!("{} bytes, the header alone is {HEADER_LEN}", bytes.len()),
        );
    }
    if bytes[0..4] != MAGIC {
        return err(codes::SURFACE_BAD_MAGIC, "magic is not 3GSF".into());
    }
    let version = le_u16(bytes, 4);
    if version != VERSION {
        return err(
            codes::SURFACE_BAD_VERSION,
            format!("format_version: {version}, this library reads {VERSION}"),
        );
    }
    let flags = le_u16(bytes, 6);
    if flags & !(FLAG_EMPTY | FLAG_ZSTD) != 0 || le_u16(bytes, 18) != 0 || le_u32(bytes, 28) != 0 {
        return err(
            codes::SURFACE_RESERVED_NONZERO,
            "reserved field or flag bit set".into(),
        );
    }
    let found = (
        le_u64(bytes, 8),
        bytes[16],
        bytes[17],
        le_u32(bytes, 20),
        le_u32(bytes, 24),
    );
    if found != (key.frame_id, key.face, key.level, key.x, key.y) {
        return err(
            codes::SURFACE_KEY_MISMATCH,
            format!("header names {found:?}, the key is {key}"),
        );
    }
    let (a, c, floor) = (le_f64(bytes, 32), le_f64(bytes, 40), le_f64(bytes, 48));
    let block_len = le_u64(bytes, 56);
    if flags & FLAG_EMPTY != 0 {
        if flags & FLAG_ZSTD != 0
            || a != 0.0
            || c != 0.0
            || floor.to_bits() != 0
            || block_len != 0
            || bytes.len() != HEADER_LEN
        {
            return err(
                codes::SURFACE_EMPTY_INVALID,
                "an empty tile has zero radii and floor, no block, and nothing after the header"
                    .into(),
            );
        }
        return Ok(Tile::Empty(*key));
    }
    let radius_ok = |r: f64| r.is_finite() && r > 0.0;
    if !(radius_ok(a) && radius_ok(c)) || c > 2.0 * a || a > 2.0 * c {
        return err(codes::SURFACE_RADIUS, format!("radii {a} m and {c} m"));
    }
    if !(floor.is_finite() && floor > -(a.min(c))) {
        return err(
            codes::SURFACE_FLOOR,
            format!("height_floor {floor} m with radii {a} m and {c} m"),
        );
    }
    if block_len != BLOCK_LEN as u64 {
        return err(
            codes::SURFACE_BLOCK_LENGTH,
            format!("block_len {block_len}, the block is {BLOCK_LEN}"),
        );
    }
    let payload = &bytes[HEADER_LEN..];
    let block = if flags & FLAG_ZSTD != 0 {
        decompress(payload, BLOCK_LEN)
            .map_err(|e| ValidationError::new(codes::SURFACE_BLOCK_LENGTH, e.reason))?
    } else {
        if payload.len() != BLOCK_LEN {
            return err(
                codes::SURFACE_BLOCK_LENGTH,
                format!(
                    "{} bytes after the header, the block is {BLOCK_LEN}",
                    payload.len()
                ),
            );
        }
        payload.to_vec()
    };
    let mut o = 0usize;
    let mut f32s = |count: usize| -> Vec<f32> {
        let v = block[o..o + 4 * count]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        o += 4 * count;
        v
    };
    let heights = f32s(HEIGHT_SIDE * HEIGHT_SIDE);
    let parent_heights = f32s(PARENT_SIDE * PARENT_SIDE);
    let albedo: Vec<[u8; 3]> = block[o..o + 3 * VERTEX_COUNT].as_chunks::<3>().0.to_vec();
    o += 3 * VERTEX_COUNT;
    let roughness = block[o..o + VERTEX_COUNT].to_vec();
    o += VERTEX_COUNT;
    let temperature: Vec<f32> = block[o..o + 4 * VERTEX_COUNT]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    for (name, list) in [("height", &heights), ("parent height", &parent_heights)] {
        if let Some(i) = list.iter().position(|&h| !h.is_finite()) {
            return err(codes::SURFACE_VALUE, format!("{name} {i} is {}", list[i]));
        }
        if let Some(i) = list.iter().position(|&h| f64::from(h) < floor) {
            return err(
                codes::SURFACE_BELOW_FLOOR,
                format!("{name} {i} is {}, below height_floor {floor}", list[i]),
            );
        }
    }
    if let Some(i) = temperature
        .iter()
        .position(|&t| !(t.is_finite() && t >= 0.0))
    {
        return err(
            codes::SURFACE_VALUE,
            format!("temperature {i} is {}", temperature[i]),
        );
    }
    Ok(Tile::Surface(SurfaceTile {
        key: *key,
        radius_xy: a,
        radius_z: c,
        height_floor: floor,
        heights,
        parent_heights,
        albedo,
        roughness,
        temperature,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::ChunkKey;

    fn tile(key: SurfaceKey) -> SurfaceTile {
        SurfaceTile {
            key,
            radius_xy: 5_712_400.0,
            radius_z: 5_689_100.0,
            height_floor: -10.0,
            heights: (0..HEIGHT_SIDE * HEIGHT_SIDE)
                .map(|i| (i % 97) as f32)
                .collect(),
            parent_heights: (0..PARENT_SIDE * PARENT_SIDE)
                .map(|i| (i % 31) as f32)
                .collect(),
            albedo: vec![[76, 71, 64]; VERTEX_COUNT],
            roughness: vec![230; VERTEX_COUNT],
            temperature: vec![251.0; VERTEX_COUNT],
        }
    }

    #[test]
    fn keys_round_trip_and_check() {
        let k: ChunkKey = "s-4-2-3-5-1".parse().unwrap();
        assert_eq!(k.to_string(), "s-4-2-3-5-1");
        assert!("s-4-6-0-0-0".parse::<ChunkKey>().is_err());
        assert!("s-4-0-1-2-0".parse::<ChunkKey>().is_err());
        assert!("s-4-0-01-0-0".parse::<ChunkKey>().is_err());
        assert!("s-4-0-1-0".parse::<ChunkKey>().is_err());
        let t = SurfaceKey::new(4, 2, 3, 5, 1).unwrap();
        assert_eq!(t.children().unwrap()[3].parent(), Some(t));
    }

    #[test]
    fn encode_decode_round_trip() {
        let key = SurfaceKey::new(4, 0, 2, 1, 3).unwrap();
        let t = tile(key);
        for c in [Compression::None, Compression::Zstd] {
            let bytes = encode(&t, c);
            assert_eq!(decode(&key, &bytes), Ok(Tile::Surface(t.clone())));
            assert!(crate::validate(&key.to_string(), &bytes).is_ok());
        }
        let e = encode_empty(&key);
        assert_eq!(decode(&key, &e), Ok(Tile::Empty(key)));
        let other = SurfaceKey::new(4, 0, 2, 1, 2).unwrap();
        assert_eq!(
            decode(&other, &e).unwrap_err().code,
            codes::SURFACE_KEY_MISMATCH
        );
    }

    #[test]
    fn renamed_radii_keep_the_committed_bytes() {
        // The committed empty tile vector is what `encode_empty` gives.
        let key = SurfaceKey::new(7, 2, 3, 5, 6).unwrap();
        assert_eq!(
            encode_empty(&key),
            include_bytes!("../../../conformance/overlay/layer/surface-tile.bin")
        );
        // A full tile set through `radius_xy` and `radius_z` gives the bytes
        // the same tile gave before the rename: their FNV-1a 64 hash and
        // length, taken from the encoder as it was then (with this test's
        // made-up radii and temperature).
        let key = SurfaceKey::new(4, 0, 2, 1, 3).unwrap();
        let t = tile(key);
        let bytes = encode(&t, Compression::None);
        assert_eq!(le_f64(&bytes, 32), t.radius_xy);
        assert_eq!(le_f64(&bytes, 40), t.radius_z);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in &bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        assert_eq!((h, bytes.len()), (0xebbf_5f9c_472a_e7c7, 56_720));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let key = SurfaceKey::new(4, 5, 1, 0, 1).unwrap();
        let mut t = tile(key);
        t.heights[7] = f32::NAN;
        assert_eq!(
            decode(&key, &encode(&t, Compression::None))
                .unwrap_err()
                .code,
            codes::SURFACE_VALUE
        );
        let mut t = tile(key);
        t.radius_z = 0.0;
        assert_eq!(
            decode(&key, &encode(&t, Compression::None))
                .unwrap_err()
                .code,
            codes::SURFACE_RADIUS
        );
        let mut b = encode(&tile(key), Compression::None);
        b.pop();
        assert_eq!(
            decode(&key, &b).unwrap_err().code,
            codes::SURFACE_BLOCK_LENGTH
        );
        // The floor: finite, above minus the smaller radius, and no height
        // or parent height below it.
        for bad in [f64::NAN, f64::NEG_INFINITY, -5_689_100.0, -7.0e6] {
            let mut t = tile(key);
            t.height_floor = bad;
            assert_eq!(
                decode(&key, &encode(&t, Compression::None))
                    .unwrap_err()
                    .code,
                codes::SURFACE_FLOOR,
                "{bad}"
            );
        }
        let mut t = tile(key);
        t.heights[100] = -10.5;
        assert_eq!(
            decode(&key, &encode(&t, Compression::None))
                .unwrap_err()
                .code,
            codes::SURFACE_BELOW_FLOOR
        );
        let mut t = tile(key);
        t.parent_heights[3] = -11.0;
        assert_eq!(
            decode(&key, &encode(&t, Compression::None))
                .unwrap_err()
                .code,
            codes::SURFACE_BELOW_FLOOR
        );
        // A height exactly at the floor is allowed; the floor round-trips.
        let mut t = tile(key);
        t.heights[100] = -10.0;
        match decode(&key, &encode(&t, Compression::None)).unwrap() {
            Tile::Surface(d) => assert_eq!(d.height_floor, -10.0),
            Tile::Empty(_) => panic!("empty"),
        }
        // An empty tile carries no floor.
        let mut e = encode_empty(&key);
        e[48..56].copy_from_slice(&(-1.0f64).to_le_bytes());
        assert_eq!(
            decode(&key, &e).unwrap_err().code,
            codes::SURFACE_EMPTY_INVALID
        );
    }

    #[test]
    fn shared_vertices_have_the_same_bits() {
        // Neighbors at one level share an edge; a child's even vertices are
        // its parent's vertices.
        let (a, c) = (5_712_400.0, 5_689_100.0);
        let left = SurfaceKey::new(4, 3, 5, 6, 9).unwrap();
        let right = SurfaceKey::new(4, 3, 5, 7, 9).unwrap();
        for j in 0..=TILE_QUADS as i64 {
            assert_eq!(
                vertex_position(&left, TILE_QUADS as i64, j, 12.5, a, c),
                vertex_position(&right, 0, j, 12.5, a, c)
            );
        }
        let child = left.children().unwrap()[3];
        for k in 0..=(TILE_QUADS as i64 / 2) {
            assert_eq!(
                vertex_position(&child, 2 * k, 2 * k, 3.0, a, c),
                vertex_position(
                    &left,
                    TILE_QUADS as i64 / 2 + k,
                    TILE_QUADS as i64 / 2 + k,
                    3.0,
                    a,
                    c
                )
            );
        }
    }

    #[test]
    fn faces_meet_at_their_edges() {
        // The +x face's right edge (s = 1) is the +y face's left edge.
        let (a, c) = (1000.0, 900.0);
        let px = SurfaceKey::new(1, 0, 0, 0, 0).unwrap();
        let py = SurfaceKey::new(1, 2, 0, 0, 0).unwrap();
        for j in 0..=TILE_QUADS as i64 {
            let p = vertex_position(&px, TILE_QUADS as i64, j, 0.0, a, c);
            let q = vertex_position(&py, 0, j, 0.0, a, c);
            for k in 0..3 {
                assert!((p[k] - q[k]).abs() < 1e-9, "{p:?} {q:?}");
            }
        }
    }
}
