//! The hub container: several sections stored under one chunk key.
//!
//! Implements `matter-format.md` section 6 (hub container). The hub
//! concatenates the sections of one chunk behind a table. Every integer is a
//! little-endian `i32`:
//!
//! ```text
//! i32 section_count
//! repeat section_count times:
//!     i32 layer_id_len, then layer_id_len bytes of UTF-8 (the hub's layer id)
//!     i32 codec_len,    then codec_len bytes (always 0 bytes today)
//!     i32 data_offset   (relative to the start of the data region)
//!     i32 data_len
//! data region: the sections' bytes, concatenated in table order
//! ```
//!
//! The decoders parse the table, bounds check every entry (codes 701 to 713,
//! `docs/errors.md`), then decode and validate each section in table order.
//! A layer id is read only to move past it: it never appears in a return
//! value or an error reason. A section that fails keeps its own code, and
//! the reason names the section by its index in the table.
//!
//! The hub is the only producer of containers in practice. [`encode_chunk`]
//! writes the same bytes the hub does, for tests and conformance vectors.

use crate::error::{codes, ValidationError};
use crate::key::{CellKey, ChunkKey};
use crate::matter::{self, Section};
use crate::registry::{self, Registry};
use crate::units::Kilograms;

/// Byte length of every integer in the container.
const INT_LEN: usize = 4;

/// One table entry after bounds checking: where its bytes sit in the data
/// region.
struct Entry {
    offset: usize,
    len: usize,
}

/// Reads table integers from the front of the input.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    /// Reads one `i32`, or fails with 703 naming the entry and field.
    fn i32(&mut self, entry: usize, field: &str) -> Result<i32, ValidationError> {
        let b = self.take(INT_LEN, entry, field)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Takes `n` bytes, or fails with 703 naming the entry and field.
    fn take(&mut self, n: usize, entry: usize, field: &str) -> Result<&'a [u8], ValidationError> {
        let rest = self.bytes.len() - self.at;
        if n > rest {
            return Err(ValidationError::new(
                codes::CONTAINER_TABLE_TRUNCATED,
                format!("container: entry {entry}: {field} needs {n} bytes, {rest} remain"),
            ));
        }
        let out = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }

    /// Reads a length field and fails with `code` if it is negative.
    fn len(&mut self, entry: usize, field: &str, code: u16) -> Result<usize, ValidationError> {
        let v = self.i32(entry, field)?;
        usize::try_from(v).map_err(|_| {
            ValidationError::new(
                code,
                format!("container: entry {entry}: {field} {v} is negative"),
            )
        })
    }
}

/// Parses and bounds checks the table. Returns the data region and the
/// entries in table order. Layer ids are skipped here and go no further.
fn parse(bytes: &[u8]) -> Result<(&[u8], Vec<Entry>), ValidationError> {
    if bytes.len() < INT_LEN {
        return Err(ValidationError::new(
            codes::CONTAINER_TOO_SHORT,
            format!(
                "container: {} bytes, section_count needs {INT_LEN}",
                bytes.len()
            ),
        ));
    }
    let mut cur = Cursor { bytes, at: 0 };
    let count = cur.i32(0, "section_count")?;
    let count = usize::try_from(count).map_err(|_| {
        ValidationError::new(
            codes::CONTAINER_COUNT_NEGATIVE,
            format!("container: section_count {count} is negative"),
        )
    })?;

    // No capacity from `count`: it is untrusted until the table is read.
    let mut raw = Vec::new();
    for i in 0..count {
        let id_len = cur.len(i, "id length", codes::CONTAINER_ID_LEN_NEGATIVE)?;
        let id = cur.take(id_len, i, "id")?;
        if core::str::from_utf8(id).is_err() {
            return Err(ValidationError::new(
                codes::CONTAINER_ID_NOT_UTF8,
                format!("container: entry {i}: id is not UTF-8"),
            ));
        }
        let codec_len = cur.len(i, "codec_len", codes::CONTAINER_CODEC_LEN_NEGATIVE)?;
        if codec_len != 0 {
            return Err(ValidationError::new(
                codes::CONTAINER_CODEC_UNSUPPORTED,
                format!(
                    "container: entry {i}: codec_len {codec_len} is not 0, no codec is defined"
                ),
            ));
        }
        let offset = cur.len(i, "data_offset", codes::CONTAINER_OFFSET_NEGATIVE)?;
        let len = cur.len(i, "data_len", codes::CONTAINER_LENGTH_NEGATIVE)?;
        raw.push(Entry { offset, len });
    }

    let data = &bytes[cur.at..];
    let mut end = 0usize;
    for (i, e) in raw.iter().enumerate() {
        if e.offset < end {
            return Err(ValidationError::new(
                codes::CONTAINER_ENTRY_OVERLAP,
                format!(
                    "container: entry {i}: data_offset {} is before the end of the previous entry at {end}",
                    e.offset
                ),
            ));
        }
        if e.offset > end {
            return Err(ValidationError::new(
                codes::CONTAINER_ENTRY_GAP,
                format!(
                    "container: entry {i}: data_offset {} leaves a gap after the previous entry ending at {end}",
                    e.offset
                ),
            ));
        }
        // Both values came from non-negative `i32`s, so the sum fits `usize`.
        end = e.offset + e.len;
        if end > data.len() {
            return Err(ValidationError::new(
                codes::CONTAINER_ENTRY_PAST_END,
                format!(
                    "container: entry {i}: ends at {end}, data region is {} bytes",
                    data.len()
                ),
            ));
        }
    }
    if end != data.len() {
        return Err(ValidationError::new(
            codes::CONTAINER_TRAILING_BYTES,
            format!(
                "container: {} bytes follow the last entry in the data region",
                data.len() - end
            ),
        ));
    }
    Ok((data, raw))
}

/// Decodes every entry with `f`, prefixing a failure with its table index.
fn decode_each<T>(
    bytes: &[u8],
    mut f: impl FnMut(&[u8]) -> Result<T, ValidationError>,
) -> Result<Vec<T>, ValidationError> {
    let (data, entries) = parse(bytes)?;
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            f(&data[e.offset..e.offset + e.len]).map_err(|err| {
                ValidationError::new(err.code, format!("section {i}: {}", err.reason))
            })
        })
        .collect()
}

/// Decodes a hub container of matter sections stored under `key`.
///
/// Parses and bounds checks the whole table first (701 to 713), then
/// decodes and validates each section with [`matter::decode`] against `key`,
/// in table order, and stops at the first failure. A failing section keeps
/// its own code and its reason starts with `section INDEX:`. Returns the
/// sections in table order. A container with no sections decodes to an empty
/// list.
pub fn decode_chunk(key: &CellKey, bytes: &[u8]) -> Result<Vec<Section>, ValidationError> {
    decode_each(bytes, |b| matter::decode(key, b))
}

/// Decodes a hub container of frame registries, the chunk stored under the
/// `registry` key.
///
/// Same container rules as [`decode_chunk`], with each entry decoded by
/// [`registry::decode`]. The registries are returned in table order and are
/// not merged; pass them to [`registry::FrameTree::from_registries`] for the
/// union.
pub fn decode_registry_chunk(bytes: &[u8]) -> Result<Vec<Registry>, ValidationError> {
    decode_each(bytes, registry::decode)
}

/// Writes a hub container exactly as the hub does: the sections in the
/// given order, entry `i` carrying `layer_ids[i]`, an empty codec, and
/// contiguous offsets.
///
/// For tests and conformance vectors; the hub stays the only producer in
/// practice.
///
/// # Panics
///
/// If the two slices differ in length, or a length or offset does not fit
/// an `i32`, which the hub cannot write either.
pub fn encode_chunk(sections: &[&[u8]], layer_ids: &[&str]) -> Vec<u8> {
    assert_eq!(sections.len(), layer_ids.len(), "one layer id per section");
    let int = |v: usize| {
        i32::try_from(v)
            .expect("container fields fit i32")
            .to_le_bytes()
    };
    let mut out = Vec::new();
    out.extend_from_slice(&int(sections.len()));
    let mut offset = 0usize;
    for (s, id) in sections.iter().zip(layer_ids) {
        out.extend_from_slice(&int(id.len()));
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&int(0));
        out.extend_from_slice(&int(offset));
        out.extend_from_slice(&int(s.len()));
        offset += s.len();
    }
    for s in sections {
        out.extend_from_slice(s);
    }
    out
}

/// Total mass of every section in a container of matter sections stored
/// under the chunk key string `key`: the sum of each section's
/// [`Section::mass`] in table order.
///
/// A key that does not parse fails with 206 and the `registry` key with 208.
/// Otherwise fails as [`decode_chunk`] does.
pub fn chunk_mass(key: &str, bytes: &[u8]) -> Result<Kilograms, ValidationError> {
    let cell = match key.parse::<ChunkKey>() {
        Err(e) => {
            return Err(ValidationError::new(
                codes::KEY_MALFORMED,
                format!("key: {e}"),
            ))
        }
        Ok(ChunkKey::Registry) => {
            return Err(ValidationError::new(
                codes::KEY_NOT_CELL,
                "key: a cell key is required, not registry",
            ))
        }
        Ok(ChunkKey::Cell(c)) => c,
    };
    let sections = decode_chunk(&cell, bytes)?;
    Ok(Kilograms::new(
        sections.iter().map(|s| s.mass().value()).sum(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matter::{encode, Compression, Sample, Samples};
    use crate::units::{Meters, Seconds};

    const LAYER_A: &str = "layer-a";
    const LAYER_B: &str = "layer-b-\u{e9}";

    fn key() -> CellKey {
        CellKey::new(9, 2, 1, 2, 3).unwrap()
    }

    fn section(density: f64) -> Vec<u8> {
        let g = key().geometry(Meters::new(64.0));
        let mut s = Sample::VACUUM;
        if density > 0.0 {
            s.density = crate::units::Density::new(density);
            s.state = crate::matter::State::Solid;
            s.temperature = crate::units::Kelvin::new(300.0);
        }
        let section = Section::new(key(), g.origin, g.edge, 1, Samples::filled(1, s)).unwrap();
        encode(&section, Compression::None)
    }

    fn two() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let a = section(1000.0);
        let b = section(500.0);
        let c = encode_chunk(&[&a, &b], &[LAYER_A, LAYER_B]);
        (a, b, c)
    }

    fn table_len(ids: &[&str]) -> usize {
        4 + ids.iter().map(|id| 16 + id.len()).sum::<usize>()
    }

    fn put(b: &mut [u8], at: usize, v: i32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn code(bytes: &[u8]) -> u16 {
        decode_chunk(&key(), bytes).unwrap_err().code
    }

    #[test]
    fn layout_matches_the_hub() {
        let (a, b, c) = two();
        let t = table_len(&[LAYER_A, LAYER_B]);
        assert_eq!(c.len(), t + a.len() + b.len());
        assert_eq!(&c[0..4], &2i32.to_le_bytes());
        assert_eq!(&c[4..8], &(LAYER_A.len() as i32).to_le_bytes());
        assert_eq!(&c[8..8 + LAYER_A.len()], LAYER_A.as_bytes());
        let e = 8 + LAYER_A.len();
        assert_eq!(&c[e..e + 4], &0i32.to_le_bytes());
        assert_eq!(&c[e + 4..e + 8], &0i32.to_le_bytes());
        assert_eq!(&c[e + 8..e + 12], &(a.len() as i32).to_le_bytes());
        let e2 = e + 12 + 4 + LAYER_B.len() + 4;
        assert_eq!(&c[e2..e2 + 4], &(a.len() as i32).to_le_bytes());
        assert_eq!(&c[t..t + a.len()], &a[..]);
        assert_eq!(&c[t + a.len()..], &b[..]);
    }

    #[test]
    fn round_trip_in_table_order() {
        let (a, b, c) = two();
        let got = decode_chunk(&key(), &c).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], matter::decode(&key(), &a).unwrap());
        assert_eq!(got[1], matter::decode(&key(), &b).unwrap());
    }

    #[test]
    fn empty_container() {
        let c = encode_chunk(&[], &[]);
        assert_eq!(c, 0i32.to_le_bytes());
        assert!(decode_chunk(&key(), &c).unwrap().is_empty());
        assert!(decode_registry_chunk(&c).unwrap().is_empty());
    }

    #[test]
    fn mass_is_the_sum() {
        let (a, b, c) = two();
        let want = matter::decode(&key(), &a).unwrap().mass().value()
            + matter::decode(&key(), &b).unwrap().mass().value();
        let got = chunk_mass(&key().to_string(), &c).unwrap();
        assert_eq!(got.value().to_bits(), want.to_bits());
        assert_eq!(
            chunk_mass("registry", &c).unwrap_err().code,
            codes::KEY_NOT_CELL
        );
        assert_eq!(
            chunk_mass("9-2", &c).unwrap_err().code,
            codes::KEY_MALFORMED
        );
    }

    #[test]
    fn registries() {
        let r1 = Registry::empty(Seconds::new(5.0)).unwrap();
        let r2 = Registry::empty(Seconds::new(5.0)).unwrap();
        let (e1, e2) = (registry::encode(&r1), registry::encode(&r2));
        let c = encode_chunk(&[&e1, &e2], &[LAYER_A, LAYER_B]);
        assert_eq!(decode_registry_chunk(&c).unwrap(), vec![r1, r2]);
        // A matter container is not a registry container.
        let (_, _, m) = two();
        assert_eq!(
            decode_registry_chunk(&m).unwrap_err().code,
            codes::REGISTRY_BAD_MAGIC
        );
    }

    #[test]
    fn table_errors() {
        let (a, _, c) = two();
        let e0 = 4 + 4 + LAYER_A.len();
        let e1 = e0 + 12 + 4 + LAYER_B.len();
        let t = table_len(&[LAYER_A, LAYER_B]);

        assert_eq!(code(&c[..3]), codes::CONTAINER_TOO_SHORT);
        assert_eq!(code(&[]), codes::CONTAINER_TOO_SHORT);

        let mut b = c.clone();
        put(&mut b, 0, -1);
        assert_eq!(code(&b), codes::CONTAINER_COUNT_NEGATIVE);

        let mut b = c.clone();
        put(&mut b, 0, i32::MAX);
        assert_eq!(code(&b), codes::CONTAINER_TABLE_TRUNCATED);
        for cut in [5, 8, e0 + 2, e0 + 6, e0 + 10, t - 1] {
            assert_eq!(
                code(&c[..cut]),
                codes::CONTAINER_TABLE_TRUNCATED,
                "cut at {cut}"
            );
        }

        let mut b = c.clone();
        put(&mut b, 4, -2);
        assert_eq!(code(&b), codes::CONTAINER_ID_LEN_NEGATIVE);

        let mut b = c.clone();
        b[8] = 0xff;
        assert_eq!(code(&b), codes::CONTAINER_ID_NOT_UTF8);

        let mut b = c.clone();
        put(&mut b, e0, -1);
        assert_eq!(code(&b), codes::CONTAINER_CODEC_LEN_NEGATIVE);

        let mut b = c.clone();
        put(&mut b, e0, 1);
        assert_eq!(code(&b), codes::CONTAINER_CODEC_UNSUPPORTED);

        let mut b = c.clone();
        put(&mut b, e0 + 4, -1);
        assert_eq!(code(&b), codes::CONTAINER_OFFSET_NEGATIVE);

        let mut b = c.clone();
        put(&mut b, e0 + 8, i32::MIN);
        assert_eq!(code(&b), codes::CONTAINER_LENGTH_NEGATIVE);

        // Second entry starts inside the first.
        let mut b = c.clone();
        put(&mut b, e1 + 4, a.len() as i32 - 1);
        assert_eq!(code(&b), codes::CONTAINER_ENTRY_OVERLAP);

        // Entries swapped: the first no longer starts at 0.
        let mut b = c.clone();
        put(&mut b, e0 + 4, a.len() as i32);
        put(&mut b, e1 + 4, 0);
        assert_eq!(code(&b), codes::CONTAINER_ENTRY_GAP);

        let mut b = c.clone();
        put(&mut b, e0 + 4, 1);
        assert_eq!(code(&b), codes::CONTAINER_ENTRY_GAP);

        let mut b = c.clone();
        put(&mut b, e1 + 8, i32::MAX);
        assert_eq!(code(&b), codes::CONTAINER_ENTRY_PAST_END);

        let mut b = c.clone();
        b.pop();
        assert_eq!(code(&b), codes::CONTAINER_ENTRY_PAST_END);

        let mut b = c.clone();
        b.push(0);
        assert_eq!(code(&b), codes::CONTAINER_TRAILING_BYTES);
    }

    #[test]
    fn section_errors_keep_their_code_and_index() {
        let (a, mut bad, _) = two();
        bad[0] = 0;
        let c = encode_chunk(&[&a, &bad], &[LAYER_A, LAYER_B]);
        let e = decode_chunk(&key(), &c).unwrap_err();
        assert_eq!(e.code, codes::BAD_MAGIC);
        assert!(e.reason.starts_with("section 1: "), "{}", e.reason);

        let other = CellKey::new(9, 2, 1, 2, 2).unwrap();
        let (_, _, c) = two();
        let e = decode_chunk(&other, &c).unwrap_err();
        assert_eq!(e.code, codes::CELL_Z_MISMATCH);
        assert!(e.reason.starts_with("section 0: "), "{}", e.reason);
    }

    #[test]
    fn layer_ids_never_reach_a_reason() {
        let (a, mut bad, _) = two();
        bad[0] = 0;
        let ids = ["unique-id-one", "unique-id-two"];
        let mut cases = vec![encode_chunk(&[&a, &bad], &ids)];
        let mut b = encode_chunk(&[&a, &a], &ids);
        b.push(1);
        cases.push(b);
        for c in cases {
            let e = decode_chunk(&key(), &c).unwrap_err();
            for id in ids {
                assert!(!e.reason.contains(id), "{}", e.reason);
            }
        }
    }
}
