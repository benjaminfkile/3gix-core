//! Section addresses and span indices.
//!
//! Implements `client-compute.md` section 3 (section addresses) and the span
//! index rule of section 2.4, as `space-model.md` section 9 describes them: a
//! section is addressed by everything that went into it, so one address means
//! one byte sequence forever.
//!
//! The address is the SHA-256 of a fixed preimage:
//!
//! | Field | Bytes |
//! |---|---|
//! | `"3GXA"` | 4 |
//! | address version, 1 | `u16` LE |
//! | module hash | 32 |
//! | source manifest hash, all zero when none | 32 |
//! | origin UTC milliseconds, 0 when the layer's `usesOrigin` is false | `i64` LE |
//! | span seconds | `u64` LE |
//! | span index | `u64` LE |
//! | chunk key length | `u32` LE |
//! | chunk key | UTF-8 |
//!
//! It is written as lowercase hex wherever it appears in a route or a
//! storage path. The hub, the worker, and the conformance vectors all call
//! this code.

use crate::error::{codes, ValidationError};
use crate::key::ChunkKey;
use crate::sha256::{self, Sha256, DIGEST_LEN};

/// The four magic bytes that open an address preimage: `3GXA` in ASCII.
pub const MAGIC: [u8; 4] = *b"3GXA";

/// The address version this module writes.
pub const VERSION: u16 = 1;

/// Length of the preimage before the chunk key bytes.
pub const PREIMAGE_FIXED_LEN: usize = 4 + 2 + 32 + 32 + 8 + 8 + 8 + 4;

/// The exact bytes [`section_address`] hashes (section 3).
///
/// `manifest` `None` writes 32 zero bytes. `origin_utc_ms` is written as
/// given: the caller passes 0 for a layer whose `usesOrigin` is false.
pub fn address_preimage(
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: &ChunkKey,
) -> Vec<u8> {
    let key = key.to_string();
    let key_len = u32::try_from(key.len()).expect("a chunk key is at most 56 bytes");
    let mut out = Vec::with_capacity(PREIMAGE_FIXED_LEN + key.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(module);
    out.extend_from_slice(manifest.unwrap_or(&[0u8; 32]));
    out.extend_from_slice(&origin_utc_ms.to_le_bytes());
    out.extend_from_slice(&span_seconds.to_le_bytes());
    out.extend_from_slice(&span_index.to_le_bytes());
    out.extend_from_slice(&key_len.to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    out
}

/// The address of one section: SHA-256 over [`address_preimage`]
/// (`client-compute.md` section 3).
///
/// `module` is the SHA-256 of the module's WebAssembly bytes, `manifest` the
/// SHA-256 of the layer's source manifest or `None`, `origin_utc_ms` the
/// space's origin in UTC milliseconds or 0 when the layer's `usesOrigin` is
/// false, and `span_seconds` and `span_index` the layer's span length and the
/// span the section covers (both 0 for matter that never changes).
pub fn section_address(
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: &ChunkKey,
) -> [u8; DIGEST_LEN] {
    let mut h = Sha256::new();
    h.update(&address_preimage(
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
        key,
    ));
    h.finish()
}

/// Lowercase hex of an address, the form used in routes and storage paths.
pub fn to_hex(address: &[u8; DIGEST_LEN]) -> String {
    sha256::to_hex(address)
}

/// The span a time falls in (`client-compute.md` section 2.4):
/// `floor((t_utc_ms - origin_utc_ms) / (span_seconds * 1000))`, and 0 when
/// `span_seconds` is 0.
///
/// Fails with 961 (`SPAN_TIME_BEFORE_ORIGIN`) when `t_utc_ms` is before
/// `origin_utc_ms`, whatever the span length. Exact integer arithmetic over
/// the whole input range, so it never overflows.
pub fn span_index(
    t_utc_ms: i64,
    origin_utc_ms: i64,
    span_seconds: u64,
) -> Result<u64, ValidationError> {
    if t_utc_ms < origin_utc_ms {
        return Err(ValidationError::new(
            codes::SPAN_TIME_BEFORE_ORIGIN,
            format!("t_utc_ms {t_utc_ms} is before origin_utc_ms {origin_utc_ms}"),
        ));
    }
    if span_seconds == 0 {
        return Ok(0);
    }
    // Both differences fit: t - origin is below 2^64 and the span below 2^74.
    let elapsed = (i128::from(t_utc_ms) - i128::from(origin_utc_ms)) as u128;
    let span_ms = u128::from(span_seconds) * 1000;
    Ok(u64::try_from(elapsed / span_ms).expect("the quotient is at most the elapsed time"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ChunkKey {
        s.parse().unwrap()
    }

    #[test]
    fn preimage_layout() {
        let m = [0x11u8; 32];
        let s = [0x22u8; 32];
        let k = key("4-12-1024-2048-3071");
        let p = address_preimage(&m, Some(&s), -5, 3600, 7, &k);
        assert_eq!(p.len(), PREIMAGE_FIXED_LEN + 19);
        assert_eq!(&p[0..4], b"3GXA");
        assert_eq!(&p[4..6], &[1, 0]);
        assert_eq!(&p[6..38], &m);
        assert_eq!(&p[38..70], &s);
        assert_eq!(&p[70..78], &(-5i64).to_le_bytes());
        assert_eq!(&p[78..86], &3600u64.to_le_bytes());
        assert_eq!(&p[86..94], &7u64.to_le_bytes());
        assert_eq!(&p[94..98], &19u32.to_le_bytes());
        assert_eq!(&p[98..], b"4-12-1024-2048-3071");

        let none = address_preimage(&m, None, 0, 0, 0, &ChunkKey::Registry);
        assert!(none[38..70].iter().all(|&b| b == 0));
        assert_eq!(&none[98..], b"registry");
    }

    #[test]
    fn address_is_hash_of_preimage() {
        let m = [7u8; 32];
        let k = key("1-0-0-0-0");
        let a = section_address(&m, None, 0, 0, 0, &k);
        assert_eq!(a, sha256::sha256(&address_preimage(&m, None, 0, 0, 0, &k)));
        assert_eq!(to_hex(&a).len(), 64);
        assert!(to_hex(&a)
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        // Every input changes the address.
        let others = [
            section_address(&[8u8; 32], None, 0, 0, 0, &k),
            section_address(&m, Some(&[0u8; 32]), 0, 0, 0, &k),
            section_address(&m, None, 1, 0, 0, &k),
            section_address(&m, None, 0, 1, 0, &k),
            section_address(&m, None, 0, 0, 1, &k),
            section_address(&m, None, 0, 0, 0, &key("2-0-0-0-0")),
        ];
        // An all-zero manifest hash is the same preimage as no manifest.
        assert_eq!(others[1], a);
        for (i, o) in others.iter().enumerate() {
            if i != 1 {
                assert_ne!(*o, a, "input {i}");
            }
        }
    }

    #[test]
    fn spans() {
        let o = 1_763_130_000_000;
        assert_eq!(span_index(o, o, 0), Ok(0));
        assert_eq!(span_index(o + 10_000_000, o, 0), Ok(0));
        assert_eq!(span_index(o, o, 60), Ok(0));
        assert_eq!(span_index(o + 59_999, o, 60), Ok(0));
        assert_eq!(span_index(o + 60_000, o, 60), Ok(1));
        assert_eq!(span_index(o + 3_600_000 * 25 + 1, o, 3600), Ok(25));
        assert_eq!(
            span_index(o - 1, o, 60).unwrap_err().code,
            codes::SPAN_TIME_BEFORE_ORIGIN
        );
        assert_eq!(
            span_index(o - 1, o, 0).unwrap_err().code,
            codes::SPAN_TIME_BEFORE_ORIGIN
        );
        assert_eq!(span_index(i64::MAX, i64::MIN, 1), Ok(u64::MAX / 1000));
        assert_eq!(span_index(i64::MAX, i64::MIN, u64::MAX), Ok(0));
    }
}
