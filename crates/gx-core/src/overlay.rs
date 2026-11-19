//! Overlay tiles: the material of part of a frame's ground.
//!
//! Implements `matter-format.md` section 12 (overlay tiles) and the
//! compositing formula of `rendering.md` section 7 item 6, which every
//! renderer build shares ([`composite`], `rendering.md` section 6 rule 5).
//! An overlay tile gives, per vertex, a coverage, an albedo, and a
//! roughness on the keys and vertex grid of surface tiles (section 10.1,
//! [`crate::surface`]). It changes no height and adds no matter. Overlay
//! layers (`client-compute.md` section 2.2) emit them under surface keys.
//!
//! # Layout (sections 12.2 to 12.4)
//!
//! A 32-byte header (magic `3GSO`, version 1, flags EMPTY and ZSTD, the
//! tile's key fields, `block_len`), then the block: coverage, albedo (three
//! bytes per vertex, long to short wavelength, as section 10.3), and
//! roughness at the [`TILE_VERTICES`] squared vertices, then the same three
//! at the parent's values at the tile's even vertices, [`PARENT_VERTICES`]
//! squared. Every value is a `u8` read as `value / 255`, and every `u8` is
//! valid. An empty tile is the header alone with EMPTY set.

use crate::error::{codes, ValidationError};
use crate::key::SurfaceKey;
use crate::matter::{decompress, Compression, ZSTD_LEVEL};

pub use crate::surface::{TILE_VERTICES, VERTEX_COUNT};

/// Parent values per side: the tile's even vertices `(2k, 2m)`, `k, m` in
/// 0 to 32.
pub const PARENT_VERTICES: usize = (TILE_VERTICES - 1) / 2 + 1;

/// Parent values per channel.
pub const PARENT_COUNT: usize = PARENT_VERTICES * PARENT_VERTICES;

/// Overlay tile format version.
pub const VERSION: u16 = 1;

/// Magic bytes `3GSO`.
pub const MAGIC: [u8; 4] = *b"3GSO";

/// Header length, bytes.
pub const HEADER_LEN: usize = 32;

/// Flag: no overlay in this tile.
pub const FLAG_EMPTY: u16 = 1;

/// Flag: the block is one zstd frame.
pub const FLAG_ZSTD: u16 = 2;

/// Block length, bytes (section 12.3).
pub const BLOCK_LEN: usize = 5 * VERTEX_COUNT + 5 * PARENT_COUNT;

/// One overlay tile with a block (section 12.3). Vertex lists are row
/// major, `j` outer.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayTile {
    /// The tile.
    pub key: SurfaceKey,
    /// Coverage per vertex, `value / 255`; 0 means no contribution.
    pub coverage: Vec<u8>,
    /// Albedo per vertex in three bands, long to short wavelength.
    pub albedo: Vec<[u8; 3]>,
    /// Roughness per vertex.
    pub roughness: Vec<u8>,
    /// The parent's coverage at this tile's even vertices (at level 0 the
    /// tile's own), [`PARENT_COUNT`] values.
    pub parent_coverage: Vec<u8>,
    /// The parent's albedo at this tile's even vertices.
    pub parent_albedo: Vec<[u8; 3]>,
    /// The parent's roughness at this tile's even vertices.
    pub parent_roughness: Vec<u8>,
}

/// A decoded overlay tile: an overlay, or none.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    /// No overlay in this tile (section 12.4).
    Empty(SurfaceKey),
    /// An overlay.
    Tile(OverlayTile),
}

/// True when `bytes` start with the overlay magic `3GSO`: the dispatch
/// between the two kinds of tile under a surface key.
pub fn is_overlay(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..4] == MAGIC
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

fn header(key: &SurfaceKey, flags: u16, block_len: u32) -> Vec<u8> {
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
    h.extend_from_slice(&block_len.to_le_bytes());
    debug_assert_eq!(h.len(), HEADER_LEN);
    h
}

/// The bytes of an empty overlay tile (section 12.4).
pub fn encode_empty(key: &SurfaceKey) -> Vec<u8> {
    header(key, FLAG_EMPTY, 0)
}

/// The bytes of `tile` (sections 12.2 and 12.3), the block compressed at
/// [`ZSTD_LEVEL`] or not.
///
/// # Panics
///
/// Panics if a channel has the wrong length.
pub fn encode(tile: &OverlayTile, compression: Compression) -> Vec<u8> {
    assert_eq!(tile.coverage.len(), VERTEX_COUNT);
    assert_eq!(tile.albedo.len(), VERTEX_COUNT);
    assert_eq!(tile.roughness.len(), VERTEX_COUNT);
    assert_eq!(tile.parent_coverage.len(), PARENT_COUNT);
    assert_eq!(tile.parent_albedo.len(), PARENT_COUNT);
    assert_eq!(tile.parent_roughness.len(), PARENT_COUNT);
    let mut block = Vec::with_capacity(BLOCK_LEN);
    block.extend_from_slice(&tile.coverage);
    block.extend(tile.albedo.iter().flatten());
    block.extend_from_slice(&tile.roughness);
    block.extend_from_slice(&tile.parent_coverage);
    block.extend(tile.parent_albedo.iter().flatten());
    block.extend_from_slice(&tile.parent_roughness);
    debug_assert_eq!(block.len(), BLOCK_LEN);
    let (flags, payload) = match compression {
        Compression::None => (0, block),
        Compression::Zstd => (
            FLAG_ZSTD,
            zstd::bulk::compress(&block, ZSTD_LEVEL).expect("zstd compresses in memory"),
        ),
    };
    let mut out = header(&tile.key, flags, BLOCK_LEN as u32);
    out.extend_from_slice(&payload);
    out
}

/// Checks `bytes` against `key` (section 12.5); the first failing rule.
pub fn validate(key: &SurfaceKey, bytes: &[u8]) -> Result<(), ValidationError> {
    decode(key, bytes).map(|_| ())
}

/// Decodes and validates `bytes` submitted under `key` (section 12.5):
/// length, magic, version, reserved fields and flag bits, key fields, then
/// the EMPTY rules or the block length and the zstd frame (codes 1051 to
/// 1057).
pub fn decode(key: &SurfaceKey, bytes: &[u8]) -> Result<Overlay, ValidationError> {
    let err = |code, reason: String| Err(ValidationError::new(code, reason));
    if bytes.len() < HEADER_LEN {
        return err(
            codes::OVERLAY_TOO_SHORT,
            format!("{} bytes, the header alone is {HEADER_LEN}", bytes.len()),
        );
    }
    if bytes[0..4] != MAGIC {
        return err(codes::OVERLAY_BAD_MAGIC, "magic is not 3GSO".into());
    }
    let version = le_u16(bytes, 4);
    if version != VERSION {
        return err(
            codes::OVERLAY_BAD_VERSION,
            format!("format_version: {version}, this library reads {VERSION}"),
        );
    }
    let flags = le_u16(bytes, 6);
    if flags & !(FLAG_EMPTY | FLAG_ZSTD) != 0 || le_u16(bytes, 18) != 0 {
        return err(
            codes::OVERLAY_RESERVED_NONZERO,
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
            codes::OVERLAY_KEY_MISMATCH,
            format!("header names {found:?}, the key is {key}"),
        );
    }
    let block_len = le_u32(bytes, 28);
    if flags & FLAG_EMPTY != 0 {
        if flags & FLAG_ZSTD != 0 || block_len != 0 || bytes.len() != HEADER_LEN {
            return err(
                codes::OVERLAY_EMPTY_INVALID,
                "an empty overlay tile has no ZSTD flag, block_len 0, and nothing after the header"
                    .into(),
            );
        }
        return Ok(Overlay::Empty(*key));
    }
    if block_len as usize != BLOCK_LEN {
        return err(
            codes::OVERLAY_BLOCK_LENGTH,
            format!("block_len {block_len}, the block is {BLOCK_LEN}"),
        );
    }
    let payload = &bytes[HEADER_LEN..];
    let block = if flags & FLAG_ZSTD != 0 {
        decompress(payload, BLOCK_LEN)
            .map_err(|e| ValidationError::new(codes::OVERLAY_BLOCK_LENGTH, e.reason))?
    } else {
        if payload.len() != BLOCK_LEN {
            return err(
                codes::OVERLAY_BLOCK_LENGTH,
                format!(
                    "{} bytes after the header, the block is {BLOCK_LEN}",
                    payload.len()
                ),
            );
        }
        payload.to_vec()
    };
    let mut o = 0usize;
    let mut take = |n: usize| -> &[u8] {
        let s = &block[o..o + n];
        o += n;
        s
    };
    let coverage = take(VERTEX_COUNT).to_vec();
    let albedo = take(3 * VERTEX_COUNT).as_chunks::<3>().0.to_vec();
    let roughness = take(VERTEX_COUNT).to_vec();
    let parent_coverage = take(PARENT_COUNT).to_vec();
    let parent_albedo = take(3 * PARENT_COUNT).as_chunks::<3>().0.to_vec();
    let parent_roughness = take(PARENT_COUNT).to_vec();
    Ok(Overlay::Tile(OverlayTile {
        key: *key,
        coverage,
        albedo,
        roughness,
        parent_coverage,
        parent_albedo,
        parent_roughness,
    }))
}

/// One overlay's values at one vertex for [`composite`]: coverage, albedo
/// in three bands, and roughness, each `value / 255` or any value in 0 to 1.
pub type OverlaySample = (f32, [f32; 3], f32);

/// The material at one vertex after the shown overlays, in layer order
/// (`rendering.md` section 7 item 6): starting from the surface tile's
/// albedo `a` and roughness `r`, each overlay with coverage `c`, albedo
/// `a_o`, and roughness `r_o` gives `a = (1 - c) a + c a_o` and
/// `r = (1 - c) r + c r_o`. The same applies at the parent values. Every
/// renderer build calls this one function (`rendering.md` section 6 rule 5).
pub fn composite(
    base_albedo: [f32; 3],
    base_roughness: f32,
    overlays: &[OverlaySample],
) -> ([f32; 3], f32) {
    let mut a = base_albedo;
    let mut r = base_roughness;
    for &(c, a_o, r_o) in overlays {
        for k in 0..3 {
            a[k] = (1.0 - c) * a[k] + c * a_o[k];
        }
        r = (1.0 - c) * r + c * r_o;
    }
    (a, r)
}

/// A stored `u8` value as `value / 255`.
pub fn unit(v: u8) -> f32 {
    f32::from(v) / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SurfaceKey {
        SurfaceKey::new(7, 2, 3, 5, 6).unwrap()
    }

    fn tile() -> OverlayTile {
        OverlayTile {
            key: key(),
            coverage: (0..VERTEX_COUNT).map(|i| (i % 256) as u8).collect(),
            albedo: (0..VERTEX_COUNT)
                .map(|i| [i as u8, (i / 3) as u8, (i / 7) as u8])
                .collect(),
            roughness: vec![200; VERTEX_COUNT],
            parent_coverage: vec![255; PARENT_COUNT],
            parent_albedo: vec![[10, 20, 30]; PARENT_COUNT],
            parent_roughness: (0..PARENT_COUNT).map(|i| (i % 7) as u8).collect(),
        }
    }

    #[test]
    fn round_trips() {
        let t = tile();
        for c in [Compression::None, Compression::Zstd] {
            let b = encode(&t, c);
            assert_eq!(decode(&key(), &b), Ok(Overlay::Tile(t.clone())));
        }
        assert_eq!(encode(&t, Compression::None).len(), HEADER_LEN + BLOCK_LEN);
        let e = encode_empty(&key());
        assert_eq!(e.len(), HEADER_LEN);
        assert_eq!(decode(&key(), &e), Ok(Overlay::Empty(key())));
        assert!(is_overlay(&e));
    }

    #[test]
    fn rules() {
        let b = encode(&tile(), Compression::None);
        let code = |b: &[u8]| decode(&key(), b).unwrap_err().code;
        assert_eq!(code(&b[..31]), codes::OVERLAY_TOO_SHORT);
        let mut m = b.clone();
        m[3] = b'F';
        assert_eq!(code(&m), codes::OVERLAY_BAD_MAGIC);
        let mut m = b.clone();
        m[4] = 2;
        assert_eq!(code(&m), codes::OVERLAY_BAD_VERSION);
        let mut m = b.clone();
        m[18] = 1;
        assert_eq!(code(&m), codes::OVERLAY_RESERVED_NONZERO);
        let mut m = b.clone();
        m[20] = 9;
        assert_eq!(code(&m), codes::OVERLAY_KEY_MISMATCH);
        let mut m = encode_empty(&key());
        m.push(0);
        assert_eq!(code(&m), codes::OVERLAY_EMPTY_INVALID);
        assert_eq!(code(&b[..b.len() - 1]), codes::OVERLAY_BLOCK_LENGTH);
    }

    #[test]
    fn coverage_zero_changes_nothing() {
        let base = ([0.3, 0.4, 0.5], 0.6);
        let got = composite(base.0, base.1, &[(0.0, [1.0, 1.0, 1.0], 1.0)]);
        assert_eq!(got, base);
    }

    #[test]
    fn coverage_one_replaces() {
        let got = composite([0.3, 0.4, 0.5], 0.6, &[(1.0, [0.1, 0.2, 0.7], 0.25)]);
        assert_eq!(got, ([0.1, 0.2, 0.7], 0.25));
    }

    #[test]
    fn order_matters() {
        let x = (1.0, [0.2, 0.2, 0.2], 0.2);
        let y = (0.5, [0.8, 0.8, 0.8], 0.8);
        let xy = composite([0.0; 3], 0.0, &[x, y]);
        let yx = composite([0.0; 3], 0.0, &[y, x]);
        assert_eq!(xy, ([0.5; 3], 0.5));
        assert_eq!(yx, ([0.2; 3], 0.2));
    }

    #[test]
    fn two_partial_overlays() {
        let got = composite(
            [1.0, 0.0, 0.5],
            1.0,
            &[(0.5, [0.0, 1.0, 0.5], 0.0), (0.25, [1.0, 1.0, 1.0], 1.0)],
        );
        // First: a = (0.5, 0.5, 0.5), r = 0.5. Second: 0.75 a + 0.25.
        assert_eq!(got, ([0.625, 0.625, 0.625], 0.625));
        assert_eq!(unit(255), 1.0);
        assert_eq!(unit(0), 0.0);
    }
}
