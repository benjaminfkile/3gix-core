//! Module request bytes, tile bytes, and the source id list.
//!
//! Implements the data formats of `client-compute.md` section 4 (module ABI
//! version 1) that cross the boundary between the worker and a compiler
//! module. Pure data: no WebAssembly runtime lives here.
//!
//! Request bytes, passed to `gx_sources` and `gx_compile`:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `3GXQ` |
//! | 4 | 2 | request version, `u16` LE, 1 |
//! | 6 | 2 | reserved, 0 |
//! | 8 | 8 | origin UTC milliseconds, `i64` LE, 0 when `usesOrigin` is false |
//! | 16 | 8 | span seconds, `u64` LE |
//! | 24 | 8 | span index, `u64` LE |
//! | 32 | 4 | chunk key length, `u32` LE |
//! | 36 | key length | chunk key, UTF-8 |
//!
//! Tile bytes, passed to `gx_compile`: a `u32` LE count, then per tile in the
//! order `gx_sources` returned (ascending, unique ids): `u64` LE id, `u32` LE
//! length, and the bytes.
//!
//! Source id list, returned by `gx_sources`: `u64` LE ids, ascending and
//! unique, possibly none.
//!
//! Module ABI version 2 (section 4.1) adds two formats.
//!
//! Input list, returned by `gx_inputs`: a `u32` LE count, then per entry a
//! `u32` LE slot, a `u32` LE key length, and the chunk key (UTF-8). Entries
//! are sorted by slot, then by key bytes, unique, and at most
//! [`MAX_INPUT_ENTRIES`]. Whether each slot is a layer slot of the config is
//! the host's check, not this module's.
//!
//! Input bytes, passed to `gx_compile`, and frame bytes, passed to
//! `gx_inputs` and `gx_sources`: a `u32` LE count, then per input a `u32` LE
//! length and the bytes. [`inputs_sha256`] is the `X-Inputs-Sha256` hash
//! over them (section 10.4).
//!
//! Every encoder output is a pure function of its input.

use crate::error::{codes, ValidationError};
use crate::key::ChunkKey;
use crate::sha256::{Sha256, DIGEST_LEN};

/// The four magic bytes that open a request: `3GXQ` in ASCII.
pub const REQUEST_MAGIC: [u8; 4] = *b"3GXQ";

/// The request version this module reads and writes.
pub const REQUEST_VERSION: u16 = 1;

/// Length of the request before the chunk key bytes.
pub const REQUEST_HEADER_LEN: usize = 36;

/// The module ABI version `gx_abi_version` returns.
pub const ABI_VERSION: i32 = 1;

/// One module request: the cell (or the registry), the universe's origin,
/// and the time span the section covers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ModuleRequest {
    /// The space's origin in UTC milliseconds, or 0 when the layer's
    /// `usesOrigin` is false.
    pub origin_utc_ms: i64,
    /// The layer's span length in seconds; 0 for matter that never changes.
    pub span_seconds: u64,
    /// The span the section covers; 0 when `span_seconds` is 0.
    pub span_index: u64,
    /// The chunk key: a cell, or the registry.
    pub key: ChunkKey,
}

/// Encodes a request (`client-compute.md` section 4).
pub fn encode_request(request: &ModuleRequest) -> Vec<u8> {
    let key = request.key.to_string();
    let key_len = u32::try_from(key.len()).expect("a chunk key is at most 56 bytes");
    let mut out = Vec::with_capacity(REQUEST_HEADER_LEN + key.len());
    out.extend_from_slice(&REQUEST_MAGIC);
    out.extend_from_slice(&REQUEST_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&request.origin_utc_ms.to_le_bytes());
    out.extend_from_slice(&request.span_seconds.to_le_bytes());
    out.extend_from_slice(&request.span_index.to_le_bytes());
    out.extend_from_slice(&key_len.to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    out
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

/// Decodes and validates a request.
///
/// Checks, in order, and reports the first failing rule: length at least 36
/// (931), magic (932), version (933), the reserved field (934), length
/// exactly `36 + key_len` (935), and a key that is UTF-8 and parses as a
/// chunk key under `matter-format.md` section 2 (936).
pub fn decode_request(bytes: &[u8]) -> Result<ModuleRequest, ValidationError> {
    if bytes.len() < REQUEST_HEADER_LEN {
        return Err(ValidationError::new(
            codes::REQUEST_TOO_SHORT,
            format!(
                "length: {} bytes, header needs {REQUEST_HEADER_LEN}",
                bytes.len()
            ),
        ));
    }
    if bytes[0..4] != REQUEST_MAGIC {
        return Err(ValidationError::new(
            codes::REQUEST_BAD_MAGIC,
            format!(
                "magic: {:02x} {:02x} {:02x} {:02x} is not 33 47 58 51",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
        ));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != REQUEST_VERSION {
        return Err(ValidationError::new(
            codes::REQUEST_UNSUPPORTED_VERSION,
            format!("version: {version} is not {REQUEST_VERSION}"),
        ));
    }
    let reserved = u16::from_le_bytes([bytes[6], bytes[7]]);
    if reserved != 0 {
        return Err(ValidationError::new(
            codes::REQUEST_RESERVED_NONZERO,
            format!("reserved u16 at offset 6: {reserved} is not 0"),
        ));
    }
    let key_len = u32_at(bytes, 32);
    let want = REQUEST_HEADER_LEN as u64 + u64::from(key_len);
    if bytes.len() as u64 != want {
        return Err(ValidationError::new(
            codes::REQUEST_LENGTH_MISMATCH,
            format!(
                "length: {} bytes, key length {key_len} needs {want}",
                bytes.len()
            ),
        ));
    }
    let key_bytes = &bytes[REQUEST_HEADER_LEN..];
    let key = core::str::from_utf8(key_bytes)
        .map_err(|e| {
            ValidationError::new(
                codes::REQUEST_KEY_MALFORMED,
                format!("key: not UTF-8 at byte {}", e.valid_up_to()),
            )
        })?
        .parse::<ChunkKey>()
        .map_err(|e| ValidationError::new(codes::REQUEST_KEY_MALFORMED, format!("key: {e}")))?;
    Ok(ModuleRequest {
        origin_utc_ms: i64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes")),
        span_seconds: u64_at(bytes, 16),
        span_index: u64_at(bytes, 24),
        key,
    })
}

/// One source tile passed to a module: its id and its bytes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Tile<'a> {
    /// The tile id, as listed in the layer's source manifest.
    pub id: u64,
    /// The tile bytes.
    pub bytes: &'a [u8],
}

fn tiles_error(code: u16, what: String) -> ValidationError {
    ValidationError::new(code, what)
}

/// Encodes tiles for `gx_compile`: count, then id, length, and bytes per
/// tile.
///
/// Fails with 944 if the ids are not strictly ascending, or with 945 if there
/// are more tiles than a `u32` counts or a tile is longer than a `u32` holds.
pub fn encode_tiles(tiles: &[Tile<'_>]) -> Result<Vec<u8>, ValidationError> {
    let count = u32::try_from(tiles.len()).map_err(|_| {
        tiles_error(
            codes::TILES_TOO_LARGE,
            format!("count: {} tiles do not fit a u32", tiles.len()),
        )
    })?;
    let total: usize = tiles.iter().map(|t| 12 + t.bytes.len()).sum();
    let mut out = Vec::with_capacity(4 + total);
    out.extend_from_slice(&count.to_le_bytes());
    for (i, t) in tiles.iter().enumerate() {
        if i > 0 && t.id <= tiles[i - 1].id {
            return Err(not_ascending(i, t.id, tiles[i - 1].id));
        }
        let len = u32::try_from(t.bytes.len()).map_err(|_| {
            tiles_error(
                codes::TILES_TOO_LARGE,
                format!(
                    "tile {i} (id {}): {} bytes do not fit a u32 length",
                    t.id,
                    t.bytes.len()
                ),
            )
        })?;
        out.extend_from_slice(&t.id.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(t.bytes);
    }
    Ok(out)
}

fn not_ascending(i: usize, id: u64, prev: u64) -> ValidationError {
    tiles_error(
        codes::TILES_NOT_ASCENDING,
        format!("tile {i}: id {id} follows {prev}, ids must ascend and be unique"),
    )
}

/// Decodes tile bytes, borrowing each tile's bytes from the input.
///
/// Checks, in order, and reports the first failing rule: length at least 4
/// (941), then tile by tile: the id, length, and bytes are within the input
/// (942) and the id is greater than the previous one (944), and finally no
/// bytes follow the last tile (943).
pub fn decode_tiles(bytes: &[u8]) -> Result<Vec<Tile<'_>>, ValidationError> {
    if bytes.len() < 4 {
        return Err(tiles_error(
            codes::TILES_TOO_SHORT,
            format!("length: {} bytes, count needs 4", bytes.len()),
        ));
    }
    let count = u32_at(bytes, 0);
    let mut at = 4usize;
    // Never trust `count` for an allocation: each tile takes at least 12
    // bytes.
    let mut tiles = Vec::with_capacity((count as usize).min(bytes.len() / 12));
    for i in 0..count as usize {
        if bytes.len() - at < 12 {
            return Err(tiles_error(
                codes::TILES_TRUNCATED,
                format!(
                    "tile {i}: id and length need 12 bytes at offset {at}, {} remain",
                    bytes.len() - at
                ),
            ));
        }
        let id = u64_at(bytes, at);
        let len = u32_at(bytes, at + 8) as usize;
        at += 12;
        if bytes.len() - at < len {
            return Err(tiles_error(
                codes::TILES_TRUNCATED,
                format!(
                    "tile {i} (id {id}): {len} bytes at offset {at}, {} remain",
                    bytes.len() - at
                ),
            ));
        }
        if let Some(prev) = tiles.last().map(|t: &Tile<'_>| t.id) {
            if id <= prev {
                return Err(not_ascending(i, id, prev));
            }
        }
        tiles.push(Tile {
            id,
            bytes: &bytes[at..at + len],
        });
        at += len;
    }
    if at != bytes.len() {
        return Err(tiles_error(
            codes::TILES_TRAILING_BYTES,
            format!(
                "{} bytes follow the last tile at offset {at}",
                bytes.len() - at
            ),
        ));
    }
    Ok(tiles)
}

/// Encodes a source id list as `gx_sources` returns it.
///
/// Fails with 952 if the ids are not strictly ascending.
pub fn encode_source_ids(ids: &[u64]) -> Result<Vec<u8>, ValidationError> {
    let mut out = Vec::with_capacity(8 * ids.len());
    for (i, &id) in ids.iter().enumerate() {
        if i > 0 && id <= ids[i - 1] {
            return Err(sources_not_ascending(i, id, ids[i - 1]));
        }
        out.extend_from_slice(&id.to_le_bytes());
    }
    Ok(out)
}

fn sources_not_ascending(i: usize, id: u64, prev: u64) -> ValidationError {
    ValidationError::new(
        codes::SOURCES_NOT_ASCENDING,
        format!("source {i}: id {id} follows {prev}, ids must ascend and be unique"),
    )
}

/// Decodes a source id list returned by `gx_sources`.
///
/// Fails with 951 if the length is not a multiple of 8, or with 952 if the
/// ids are not strictly ascending.
pub fn decode_source_ids(bytes: &[u8]) -> Result<Vec<u64>, ValidationError> {
    let (ids, rest) = bytes.as_chunks::<8>();
    if !rest.is_empty() {
        return Err(ValidationError::new(
            codes::SOURCES_LENGTH_NOT_MULTIPLE,
            format!("length: {} bytes is not a multiple of 8", bytes.len()),
        ));
    }
    let mut out: Vec<u64> = Vec::with_capacity(ids.len());
    for (i, b) in ids.iter().enumerate() {
        let id = u64::from_le_bytes(*b);
        if let Some(&prev) = out.last() {
            if id <= prev {
                return Err(sources_not_ascending(i, id, prev));
            }
        }
        out.push(id);
    }
    Ok(out)
}

/// The most entries an input list may hold (`client-compute.md` section
/// 4.1).
pub const MAX_INPUT_ENTRIES: usize = 64;

/// One input list entry: a slot of the reading layer's config and a chunk
/// key of the layer in that slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputEntry {
    /// The slot index in the reading layer's input list.
    pub slot: u32,
    /// The key of the input section.
    pub key: ChunkKey,
}

fn entry_order(a: &(u32, String), b: &(u32, String)) -> core::cmp::Ordering {
    a.0.cmp(&b.0)
        .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
}

fn not_sorted(i: usize) -> ValidationError {
    ValidationError::new(
        codes::INPUT_LIST_NOT_SORTED,
        format!("entry {i}: not after the previous entry by slot, then key bytes, or equal to it"),
    )
}

fn too_many(n: u64) -> ValidationError {
    ValidationError::new(
        codes::INPUT_LIST_TOO_MANY,
        format!("count: {n} entries, at most {MAX_INPUT_ENTRIES}"),
    )
}

/// Encodes an input list as `gx_inputs` returns it (`client-compute.md`
/// section 4.1).
///
/// Fails with 1032 for more than 64 entries, or with 1036 if the entries are
/// not strictly ascending by slot, then by key bytes.
pub fn encode_input_list(entries: &[InputEntry]) -> Result<Vec<u8>, ValidationError> {
    if entries.len() > MAX_INPUT_ENTRIES {
        return Err(too_many(entries.len() as u64));
    }
    let keyed: Vec<(u32, String)> = entries
        .iter()
        .map(|e| (e.slot, e.key.to_string()))
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (i, (slot, key)) in keyed.iter().enumerate() {
        if i > 0 && entry_order(&keyed[i - 1], &keyed[i]) != core::cmp::Ordering::Less {
            return Err(not_sorted(i));
        }
        out.extend_from_slice(&slot.to_le_bytes());
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
    }
    Ok(out)
}

/// Decodes and validates an input list returned by `gx_inputs`.
///
/// Checks, in order, and reports the first failing rule: length at least 4
/// (1031), count at most 64 (1032), then entry by entry: slot, key length,
/// and key within the input (1033), a key that is UTF-8 and a chunk key
/// (1035), and the entry after the previous one by slot, then key bytes
/// (1036), and finally no bytes after the last entry (1034).
pub fn decode_input_list(bytes: &[u8]) -> Result<Vec<InputEntry>, ValidationError> {
    if bytes.len() < 4 {
        return Err(ValidationError::new(
            codes::INPUT_LIST_TOO_SHORT,
            format!("length: {} bytes, count needs 4", bytes.len()),
        ));
    }
    let count = u32_at(bytes, 0);
    if count as usize > MAX_INPUT_ENTRIES {
        return Err(too_many(u64::from(count)));
    }
    let mut at = 4usize;
    let mut prev: Option<(u32, String)> = None;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        if bytes.len() - at < 8 {
            return Err(ValidationError::new(
                codes::INPUT_LIST_TRUNCATED,
                format!(
                    "entry {i}: slot and key length need 8 bytes at offset {at}, {} remain",
                    bytes.len() - at
                ),
            ));
        }
        let slot = u32_at(bytes, at);
        let len = u32_at(bytes, at + 4) as usize;
        at += 8;
        if bytes.len() - at < len {
            return Err(ValidationError::new(
                codes::INPUT_LIST_TRUNCATED,
                format!(
                    "entry {i}: key of {len} bytes at offset {at}, {} remain",
                    bytes.len() - at
                ),
            ));
        }
        let raw = &bytes[at..at + len];
        at += len;
        let text = core::str::from_utf8(raw).map_err(|e| {
            ValidationError::new(
                codes::INPUT_LIST_KEY_MALFORMED,
                format!("entry {i}: key not UTF-8 at byte {}", e.valid_up_to()),
            )
        })?;
        let key = text.parse::<ChunkKey>().map_err(|e| {
            ValidationError::new(
                codes::INPUT_LIST_KEY_MALFORMED,
                format!("entry {i}: key: {e}"),
            )
        })?;
        let this = (slot, text.to_owned());
        if let Some(p) = &prev {
            if entry_order(p, &this) != core::cmp::Ordering::Less {
                return Err(not_sorted(i));
            }
        }
        prev = Some(this);
        out.push(InputEntry { slot, key });
    }
    if at != bytes.len() {
        return Err(ValidationError::new(
            codes::INPUT_LIST_TRAILING_BYTES,
            format!(
                "{} bytes follow the last entry at offset {at}",
                bytes.len() - at
            ),
        ));
    }
    Ok(out)
}

/// Encodes input bytes for `gx_compile`, or frame bytes for `gx_inputs` and
/// `gx_sources` (`client-compute.md` section 4.1): count, then length and
/// bytes per input, in the order given.
///
/// Fails with 1037 if there are more inputs than a `u32` counts or an input
/// is longer than a `u32` holds.
pub fn encode_inputs(inputs: &[&[u8]]) -> Result<Vec<u8>, ValidationError> {
    let too_large = |what: String| ValidationError::new(codes::INPUTS_TRUNCATED, what);
    let count = u32::try_from(inputs.len())
        .map_err(|_| too_large(format!("count: {} inputs do not fit a u32", inputs.len())))?;
    let total: usize = inputs.iter().map(|b| 4 + b.len()).sum();
    let mut out = Vec::with_capacity(4 + total);
    out.extend_from_slice(&count.to_le_bytes());
    for (i, b) in inputs.iter().enumerate() {
        let len = u32::try_from(b.len()).map_err(|_| {
            too_large(format!(
                "input {i}: {} bytes do not fit a u32 length",
                b.len()
            ))
        })?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(b);
    }
    Ok(out)
}

/// Decodes input bytes or frame bytes, borrowing each input from `bytes`.
///
/// Checks, in order, and reports the first failing rule: length at least 4
/// and every length and input within the bytes (1037), and no bytes after
/// the last input (1038).
pub fn decode_inputs(bytes: &[u8]) -> Result<Vec<&[u8]>, ValidationError> {
    if bytes.len() < 4 {
        return Err(ValidationError::new(
            codes::INPUTS_TRUNCATED,
            format!("length: {} bytes, count needs 4", bytes.len()),
        ));
    }
    let count = u32_at(bytes, 0);
    let mut at = 4usize;
    // Never trust `count` for an allocation: each input takes at least 4
    // bytes.
    let mut out = Vec::with_capacity((count as usize).min(bytes.len() / 4));
    for i in 0..count as usize {
        if bytes.len() - at < 4 {
            return Err(ValidationError::new(
                codes::INPUTS_TRUNCATED,
                format!(
                    "input {i}: length needs 4 bytes at offset {at}, {} remain",
                    bytes.len() - at
                ),
            ));
        }
        let len = u32_at(bytes, at) as usize;
        at += 4;
        if bytes.len() - at < len {
            return Err(ValidationError::new(
                codes::INPUTS_TRUNCATED,
                format!(
                    "input {i}: {len} bytes at offset {at}, {} remain",
                    bytes.len() - at
                ),
            ));
        }
        out.push(&bytes[at..at + len]);
        at += len;
    }
    if at != bytes.len() {
        return Err(ValidationError::new(
            codes::INPUTS_TRAILING_BYTES,
            format!(
                "{} bytes follow the last input at offset {at}",
                bytes.len() - at
            ),
        ));
    }
    Ok(out)
}

/// The `X-Inputs-Sha256` hash (`client-compute.md` section 10.4): SHA-256
/// over the 32-byte SHA-256 of each input, in the order passed to
/// `gx_compile`. Write it as lowercase hex with [`crate::sha256::to_hex`].
pub fn inputs_sha256(inputs: &[&[u8]]) -> [u8; DIGEST_LEN] {
    let mut h = Sha256::new();
    for b in inputs {
        h.update(&crate::sha256::sha256(b));
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code<T: core::fmt::Debug>(r: Result<T, ValidationError>) -> u16 {
        r.unwrap_err().code
    }

    fn request() -> ModuleRequest {
        ModuleRequest {
            origin_utc_ms: 1_763_130_000_000,
            span_seconds: 3600,
            span_index: 2,
            key: "4-12-1024-2048-3071".parse().unwrap(),
        }
    }

    #[test]
    fn request_round_trip_and_layout() {
        let r = request();
        let b = encode_request(&r);
        assert_eq!(b.len(), REQUEST_HEADER_LEN + 19);
        assert_eq!(&b[0..8], b"3GXQ\x01\x00\x00\x00");
        assert_eq!(&b[8..16], &1_763_130_000_000i64.to_le_bytes());
        assert_eq!(&b[16..24], &3600u64.to_le_bytes());
        assert_eq!(&b[24..32], &2u64.to_le_bytes());
        assert_eq!(&b[32..36], &19u32.to_le_bytes());
        assert_eq!(&b[36..], b"4-12-1024-2048-3071");
        assert_eq!(decode_request(&b), Ok(r));

        let reg = ModuleRequest {
            origin_utc_ms: -1,
            span_seconds: 0,
            span_index: 0,
            key: ChunkKey::Registry,
        };
        assert_eq!(decode_request(&encode_request(&reg)), Ok(reg));
    }

    #[test]
    fn request_rejects_each_rule() {
        let good = encode_request(&request());
        let edit = |at: usize, v: u8| {
            let mut b = good.clone();
            b[at] = v;
            code(decode_request(&b))
        };
        assert_eq!(code(decode_request(&good[..35])), 931);
        assert_eq!(edit(3, b'M'), 932);
        assert_eq!(edit(4, 2), 933);
        assert_eq!(edit(6, 1), 934);
        assert_eq!(edit(32, 18), 935);
        assert_eq!(code(decode_request(&good[..good.len() - 1])), 935);
        assert_eq!(edit(36, 0xff), 936);
        assert_eq!(edit(37, b'x'), 936);
    }

    #[test]
    fn tiles_round_trip_and_layout() {
        let a = [1u8, 2, 3];
        let tiles = [
            Tile { id: 5, bytes: &a },
            Tile { id: 9, bytes: &[] },
            Tile {
                id: u64::MAX,
                bytes: b"z",
            },
        ];
        let b = encode_tiles(&tiles).unwrap();
        assert_eq!(b.len(), 4 + 3 * 12 + 4);
        assert_eq!(&b[0..4], &3u32.to_le_bytes());
        assert_eq!(&b[4..12], &5u64.to_le_bytes());
        assert_eq!(&b[12..16], &3u32.to_le_bytes());
        assert_eq!(&b[16..19], &a);
        assert_eq!(decode_tiles(&b).unwrap(), tiles);
        assert_eq!(encode_tiles(&[]).unwrap(), [0, 0, 0, 0]);
        assert!(decode_tiles(&[0, 0, 0, 0]).unwrap().is_empty());
    }

    #[test]
    fn tiles_reject_each_rule() {
        let good = encode_tiles(&[
            Tile {
                id: 1,
                bytes: b"ab",
            },
            Tile { id: 2, bytes: b"c" },
        ])
        .unwrap();
        assert_eq!(code(decode_tiles(&good[..3])), 941);
        assert_eq!(code(decode_tiles(&good[..good.len() - 1])), 942);
        assert_eq!(code(decode_tiles(&good[..10])), 942);
        let mut b = good.clone();
        b[0] = 3;
        assert_eq!(code(decode_tiles(&b)), 942);
        let mut b = good.clone();
        b[0] = 1;
        assert_eq!(code(decode_tiles(&b)), 943);
        let mut b = good.clone();
        b.push(0);
        assert_eq!(code(decode_tiles(&b)), 943);
        let mut b = good.clone();
        b[4 + 12 + 2] = 1;
        assert_eq!(code(decode_tiles(&b)), 944);
        // A huge count with no tiles never allocates for the count.
        assert_eq!(code(decode_tiles(&[0xff, 0xff, 0xff, 0xff])), 942);
        assert_eq!(
            code(encode_tiles(&[
                Tile { id: 2, bytes: b"" },
                Tile { id: 2, bytes: b"" }
            ])),
            944
        );
    }

    #[test]
    fn source_ids() {
        let b = encode_source_ids(&[1, 2, 40]).unwrap();
        assert_eq!(b.len(), 24);
        assert_eq!(decode_source_ids(&b).unwrap(), [1, 2, 40]);
        assert!(decode_source_ids(&[]).unwrap().is_empty());
        assert_eq!(code(decode_source_ids(&b[..23])), 951);
        assert_eq!(code(encode_source_ids(&[3, 3])), 952);
        let mut swapped = b[8..16].to_vec();
        swapped.extend_from_slice(&b[0..8]);
        assert_eq!(code(decode_source_ids(&swapped)), 952);
    }

    fn entry(slot: u32, key: &str) -> InputEntry {
        InputEntry {
            slot,
            key: key.parse().unwrap(),
        }
    }

    #[test]
    fn input_list_round_trip_and_rules() {
        let entries = [
            entry(0, "1-0-0-0-0"),
            entry(0, "registry"),
            entry(2, "frame-3"),
        ];
        let b = encode_input_list(&entries).unwrap();
        assert_eq!(&b[0..4], &3u32.to_le_bytes());
        assert_eq!(&b[4..8], &0u32.to_le_bytes());
        assert_eq!(&b[8..12], &9u32.to_le_bytes());
        assert_eq!(&b[12..21], b"1-0-0-0-0");
        assert_eq!(decode_input_list(&b).unwrap(), entries);
        assert!(decode_input_list(&[0, 0, 0, 0]).unwrap().is_empty());

        assert_eq!(code(decode_input_list(&b[..3])), 1031);
        assert_eq!(code(decode_input_list(&[65, 0, 0, 0])), 1032);
        assert_eq!(code(decode_input_list(&b[..b.len() - 1])), 1033);
        let mut t = b.clone();
        t.push(0);
        assert_eq!(code(decode_input_list(&t)), 1034);
        let mut k = b.clone();
        k[12] = b'x';
        assert_eq!(code(decode_input_list(&k)), 1035);
        let mut k = b.clone();
        k[12] = 0xff;
        assert_eq!(code(decode_input_list(&k)), 1035);
        let swapped = encode_input_list(&entries[..1])
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();
        let mut dup = vec![2, 0, 0, 0];
        dup.extend_from_slice(&swapped[4..]);
        dup.extend_from_slice(&swapped[4..]);
        assert_eq!(code(decode_input_list(&dup)), 1036);
        assert_eq!(
            code(encode_input_list(&[
                entry(1, "registry"),
                entry(0, "registry")
            ])),
            1036
        );
        let many: Vec<InputEntry> = (0..65).map(|s| entry(s, "registry")).collect();
        assert_eq!(code(encode_input_list(&many)), 1032);
        assert!(encode_input_list(&many[..64]).is_ok());
    }

    #[test]
    fn inputs_round_trip_and_hash() {
        let a: &[u8] = b"abc";
        let inputs = [a, &[][..], b"z"];
        let b = encode_inputs(&inputs).unwrap();
        assert_eq!(b.len(), 4 + 12 + 4);
        assert_eq!(decode_inputs(&b).unwrap(), inputs);
        assert_eq!(code(decode_inputs(&b[..3])), 1037);
        assert_eq!(code(decode_inputs(&b[..b.len() - 1])), 1037);
        assert_eq!(code(decode_inputs(&[0xff, 0xff, 0xff, 0xff])), 1037);
        let mut t = b.clone();
        t.push(1);
        assert_eq!(code(decode_inputs(&t)), 1038);

        let mut pre = Vec::new();
        for i in inputs {
            pre.extend_from_slice(&crate::sha256::sha256(i));
        }
        assert_eq!(inputs_sha256(&inputs), crate::sha256::sha256(&pre));
        assert_eq!(inputs_sha256(&[]), crate::sha256::sha256(b""));
    }
}
