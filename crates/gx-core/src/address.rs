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
//! A layer with at least one input uses address version 2
//! ([`section_address_v2`]), which writes, after the span index, a `u32` LE
//! input count and per input slot a kind byte ([`SlotKind`]) and a 32-byte
//! value. A layer slot's value is its layer's identity at the input span
//! ([`layer_identity`], section 10.5, magic `"3GXI"`, identity version 1),
//! read at the span [`input_span_index`] gives (section 10.3). A frames
//! slot's value is [`frames_slot_value`].
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

/// The address version of a layer with at least one input (section 3).
pub const VERSION_2: u16 = 2;

/// The four magic bytes that open an identity preimage: `3GXI` in ASCII
/// (`client-compute.md` section 10.5).
pub const IDENTITY_MAGIC: [u8; 4] = *b"3GXI";

/// The identity version this module writes.
pub const IDENTITY_VERSION: u16 = 1;

/// Length in bytes of one slot as it crosses the C ABI and appears in a
/// preimage: the kind byte and the 32-byte value.
pub const SLOT_LEN: usize = 33;

/// The kind of an input slot (`client-compute.md` section 10.5).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotKind {
    /// A layer slot, kind byte 0: the value is the read layer's identity.
    Layer,
    /// A frames slot, kind byte 1: the value is [`frames_slot_value`].
    Frames,
}

impl SlotKind {
    /// The kind byte written in a preimage: 0 for a layer slot, 1 for a
    /// frames slot.
    pub fn byte(self) -> u8 {
        match self {
            SlotKind::Layer => 0,
            SlotKind::Frames => 1,
        }
    }

    /// The kind of a kind byte. Fails with 1042 for a byte other than 0 or
    /// 1.
    pub fn from_byte(b: u8) -> Result<SlotKind, ValidationError> {
        match b {
            0 => Ok(SlotKind::Layer),
            1 => Ok(SlotKind::Frames),
            _ => Err(ValidationError::new(
                codes::SLOT_KIND_UNKNOWN,
                format!("slot kind: {b} is neither 0 (layer) nor 1 (frames)"),
            )),
        }
    }
}

/// One input slot's contribution to an identity or a version 2 address.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SlotValue {
    /// Whether the slot reads a layer or frame records.
    pub kind: SlotKind,
    /// The read layer's identity, or the frames slot's value.
    pub value: [u8; 32],
}

impl SlotValue {
    /// Decodes slots as they cross the C ABI: `count * 33` bytes, each a
    /// kind byte and a 32-byte value. Fails with 1043 for a length that is
    /// not a multiple of 33 and with 1042 for an unknown kind byte.
    pub fn decode_all(bytes: &[u8]) -> Result<Vec<SlotValue>, ValidationError> {
        let (slots, rest) = bytes.as_chunks::<SLOT_LEN>();
        if !rest.is_empty() {
            return Err(ValidationError::new(
                codes::SLOTS_LENGTH,
                format!(
                    "slots: {} bytes is not a multiple of {SLOT_LEN}",
                    bytes.len()
                ),
            ));
        }
        slots
            .iter()
            .map(|c| {
                Ok(SlotValue {
                    kind: SlotKind::from_byte(c[0])?,
                    value: c[1..].try_into().expect("32 bytes"),
                })
            })
            .collect()
    }
}

/// A frames slot's value (`client-compute.md` section 10.5): SHA-256 over the
/// 32-byte hashes of its frame records, in the order the config lists the
/// frames.
pub fn frames_slot_value(record_hashes: &[[u8; 32]]) -> [u8; DIGEST_LEN] {
    let mut h = Sha256::new();
    for r in record_hashes {
        h.update(r);
    }
    h.finish()
}

fn slots_bytes(out: &mut Vec<u8>, slots: &[SlotValue]) -> Result<(), ValidationError> {
    let count = u32::try_from(slots.len()).map_err(|_| {
        ValidationError::new(
            codes::SLOTS_LENGTH,
            format!("slots: {} do not fit a u32 count", slots.len()),
        )
    })?;
    out.extend_from_slice(&count.to_le_bytes());
    for s in slots {
        out.push(s.kind.byte());
        out.extend_from_slice(&s.value);
    }
    Ok(())
}

fn common_preimage(
    magic: &[u8; 4],
    version: u16,
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(PREIMAGE_FIXED_LEN + 64 * SLOT_LEN);
    out.extend_from_slice(magic);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(module);
    out.extend_from_slice(manifest.unwrap_or(&[0u8; 32]));
    out.extend_from_slice(&origin_utc_ms.to_le_bytes());
    out.extend_from_slice(&span_seconds.to_le_bytes());
    out.extend_from_slice(&span_index.to_le_bytes());
    out
}

/// The exact bytes [`layer_identity`] hashes (`client-compute.md` section
/// 10.5). `manifest` is the manifest of span `span_index` under the epoch
/// rule, `None` for none; `origin_utc_ms` is 0 when the layer's `usesOrigin`
/// is false. A layer with no slots has an identity too.
///
/// Fails with 1043 only for more slots than a `u32` counts.
pub fn identity_preimage(
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: &[SlotValue],
) -> Result<Vec<u8>, ValidationError> {
    let mut out = common_preimage(
        &IDENTITY_MAGIC,
        IDENTITY_VERSION,
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
    );
    slots_bytes(&mut out, slots)?;
    Ok(out)
}

/// A layer's identity at span `span_index` (`client-compute.md` section
/// 10.5): SHA-256 over [`identity_preimage`]. It is the value of a layer
/// slot that reads this layer.
pub fn layer_identity(
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: &[SlotValue],
) -> Result<[u8; DIGEST_LEN], ValidationError> {
    let pre = identity_preimage(
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
        slots,
    )?;
    Ok(sha256::sha256(&pre))
}

/// The exact bytes [`section_address_v2`] hashes (`client-compute.md`
/// section 3, address version 2).
///
/// Fails with 1041 when `slots` is empty (such a layer uses version 1,
/// [`address_preimage`]) and with 1043 for more slots than a `u32` counts.
pub fn address_preimage_v2(
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: &[SlotValue],
    key: &ChunkKey,
) -> Result<Vec<u8>, ValidationError> {
    if slots.is_empty() {
        return Err(ValidationError::new(
            codes::ADDRESS_V2_NO_INPUTS,
            "slots: none, a layer without inputs uses address version 1",
        ));
    }
    let key = key.to_string();
    let key_len = u32::try_from(key.len()).expect("a chunk key is at most 56 bytes");
    let mut out = common_preimage(
        &MAGIC,
        VERSION_2,
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
    );
    slots_bytes(&mut out, slots)?;
    out.extend_from_slice(&key_len.to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    Ok(out)
}

/// The version 2 address of one section of a layer with inputs
/// (`client-compute.md` section 3): SHA-256 over [`address_preimage_v2`].
/// The arguments are those of [`section_address`] plus the slots, in slot
/// order. Fails with 1041 for no slots.
pub fn section_address_v2(
    module: &[u8; 32],
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: &[SlotValue],
    key: &ChunkKey,
) -> Result<[u8; DIGEST_LEN], ValidationError> {
    let pre = address_preimage_v2(
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
        slots,
        key,
    )?;
    Ok(sha256::sha256(&pre))
}

/// The span at which a layer slot is read (`client-compute.md` section
/// 10.3): `floor(reader_span_index * reader_span_seconds /
/// input_span_seconds)`, and 0 when `input_span_seconds` is 0.
///
/// Computed exactly in 128 bits. Fails with 1044 when the result does not
/// fit a `u64`, which only an input span shorter than the reader's can give
/// (config rule 2 of section 10.1 refuses such a slot).
pub fn input_span_index(
    reader_span_index: u64,
    reader_span_seconds: u64,
    input_span_seconds: u64,
) -> Result<u64, ValidationError> {
    if input_span_seconds == 0 {
        return Ok(0);
    }
    let start = u128::from(reader_span_index) * u128::from(reader_span_seconds);
    u64::try_from(start / u128::from(input_span_seconds)).map_err(|_| {
        ValidationError::new(
            codes::INPUT_SPAN_OVERFLOW,
            format!(
                "input span index of reader span {reader_span_index} at {reader_span_seconds} s \
                 over {input_span_seconds} s does not fit a u64"
            ),
        )
    })
}

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
        let k = key("23-12-1024-2048-3071");
        let p = address_preimage(&m, Some(&s), -5, 3600, 7, &k);
        assert_eq!(p.len(), PREIMAGE_FIXED_LEN + 20);
        assert_eq!(&p[0..4], b"3GXA");
        assert_eq!(&p[4..6], &[1, 0]);
        assert_eq!(&p[6..38], &m);
        assert_eq!(&p[38..70], &s);
        assert_eq!(&p[70..78], &(-5i64).to_le_bytes());
        assert_eq!(&p[78..86], &3600u64.to_le_bytes());
        assert_eq!(&p[86..94], &7u64.to_le_bytes());
        assert_eq!(&p[94..98], &20u32.to_le_bytes());
        assert_eq!(&p[98..], b"23-12-1024-2048-3071");

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

    #[test]
    fn identity_and_v2_layout() {
        let m = [0x11u8; 32];
        let s = [0x22u8; 32];
        let slots = [
            SlotValue {
                kind: SlotKind::Layer,
                value: [0x33; 32],
            },
            SlotValue {
                kind: SlotKind::Frames,
                value: [0x44; 32],
            },
        ];
        let p = identity_preimage(&m, Some(&s), -5, 3600, 7, &slots).unwrap();
        assert_eq!(p.len(), 94 + 4 + 66);
        assert_eq!(&p[0..6], b"3GXI\x01\x00");
        assert_eq!(&p[6..38], &m);
        assert_eq!(&p[94..98], &2u32.to_le_bytes());
        assert_eq!(p[98], 0);
        assert_eq!(p[131], 1);
        assert_eq!(
            layer_identity(&m, Some(&s), -5, 3600, 7, &slots).unwrap(),
            sha256::sha256(&p)
        );
        // An identity without slots exists.
        assert_eq!(identity_preimage(&m, None, 0, 0, 0, &[]).unwrap().len(), 98);

        let k = key("1-0-0-0-0");
        let a = address_preimage_v2(&m, Some(&s), -5, 3600, 7, &slots, &k).unwrap();
        assert_eq!(&a[0..6], b"3GXA\x02\x00");
        assert_eq!(&a[6..94], &p[6..94]);
        assert_eq!(&a[94..164], &p[94..164]);
        assert_eq!(&a[164..168], &9u32.to_le_bytes());
        assert_eq!(&a[168..], b"1-0-0-0-0");
        assert_eq!(
            section_address_v2(&m, Some(&s), -5, 3600, 7, &slots, &k).unwrap(),
            sha256::sha256(&a)
        );
        assert_eq!(
            section_address_v2(&m, None, 0, 0, 0, &[], &k)
                .unwrap_err()
                .code,
            codes::ADDRESS_V2_NO_INPUTS
        );
        // The order and kind of slots matter.
        let swapped = [slots[1], slots[0]];
        assert_ne!(
            section_address_v2(&m, None, 0, 0, 0, &swapped, &k),
            section_address_v2(&m, None, 0, 0, 0, &slots, &k)
        );
    }

    #[test]
    fn slots_and_frames() {
        let mut b = vec![0u8; 33];
        b.extend_from_slice(&[1u8; 33]);
        let s = SlotValue::decode_all(&b).unwrap();
        assert_eq!(s[0].kind, SlotKind::Layer);
        assert_eq!(s[1].kind, SlotKind::Frames);
        assert_eq!(s[1].value, [1u8; 32]);
        assert_eq!(SlotValue::decode_all(&b[..65]).unwrap_err().code, 1043);
        b[33] = 2;
        assert_eq!(SlotValue::decode_all(&b).unwrap_err().code, 1042);
        let h = [[1u8; 32], [2u8; 32]];
        assert_eq!(frames_slot_value(&h), sha256::sha256(&h.concat()));
        assert_eq!(frames_slot_value(&[]), sha256::sha256(b""));
    }

    #[test]
    fn input_spans() {
        assert_eq!(input_span_index(5, 60, 0), Ok(0));
        assert_eq!(input_span_index(59, 60, 3600), Ok(0));
        assert_eq!(input_span_index(60, 60, 3600), Ok(1));
        assert_eq!(input_span_index(7, 3600, 3600), Ok(7));
        assert_eq!(input_span_index(u64::MAX, u64::MAX, u64::MAX), Ok(u64::MAX));
        assert_eq!(input_span_index(u64::MAX, 2, 1).unwrap_err().code, 1044);
    }
}
