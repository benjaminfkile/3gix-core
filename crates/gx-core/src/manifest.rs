//! Source manifests: encode, decode, validate, hash, and look up.
//!
//! Implements `client-compute.md` section 5.1. A manifest lists the source
//! tiles one layer's module may read, each as an integer id with the SHA-256
//! and length of its bytes:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `3GXM` |
//! | 4 | 2 | version, `u16` LE, 1 |
//! | 6 | 2 | reserved, 0 |
//! | 8 | 4 | entry count, `u32` LE |
//! | 12 | 48 each | entries ascending by id: `id` `u64` LE, `sha256` 32 bytes, `length` `u64` LE |
//!
//! The manifest's hash is the SHA-256 of these bytes ([`manifest_hash`]).
//! Tile ids and tile contents are private to the module that reads them; the
//! manifest gives them no other meaning. A manifest with no entries is well
//! formed. Every byte [`encode`] writes is a pure function of the manifest.

use crate::error::{codes, ValidationError};
use crate::sha256::{sha256, DIGEST_LEN};

/// The four magic bytes that open a manifest: `3GXM` in ASCII.
pub const MAGIC: [u8; 4] = *b"3GXM";

/// The manifest version this module reads and writes.
pub const VERSION: u16 = 1;

/// Length of the manifest header in bytes.
pub const HEADER_LEN: usize = 12;

/// Length of one entry in bytes.
pub const ENTRY_LEN: usize = 48;

/// One source tile: its id, the SHA-256 of its bytes, and its length in
/// bytes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// The tile id, private to the module that reads it.
    pub id: u64,
    /// SHA-256 of the tile bytes.
    pub sha256: [u8; DIGEST_LEN],
    /// Length of the tile bytes.
    pub length: u64,
}

/// One validated source manifest: entries sorted by id with unique ids.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Manifest {
    entries: Vec<Entry>,
}

impl Manifest {
    /// Builds a manifest, sorting `entries` by id ascending.
    ///
    /// Fails with 908 if there are more entries than a `u32` counts, or with
    /// 907 if an id appears more than once.
    pub fn new(mut entries: Vec<Entry>) -> Result<Manifest, ValidationError> {
        if u32::try_from(entries.len()).is_err() {
            return Err(ValidationError::new(
                codes::MANIFEST_TOO_MANY_ENTRIES,
                format!("entry_count: {} entries do not fit a u32", entries.len()),
            ));
        }
        entries.sort_by_key(|e| e.id);
        if let Some(i) = (1..entries.len()).find(|&i| entries[i - 1].id == entries[i].id) {
            return Err(duplicate(i, entries[i].id));
        }
        Ok(Manifest { entries })
    }

    /// The entries, sorted by id ascending.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The SHA-256 and length of the tile with this id, or `None` if the
    /// manifest does not list it. A module asking for an unlisted id has
    /// failed (`client-compute.md` section 5.2).
    pub fn lookup(&self, id: u64) -> Option<([u8; DIGEST_LEN], u64)> {
        self.entries
            .binary_search_by_key(&id, |e| e.id)
            .ok()
            .map(|i| (self.entries[i].sha256, self.entries[i].length))
    }

    /// The SHA-256 of this manifest's encoded bytes.
    pub fn hash(&self) -> [u8; DIGEST_LEN] {
        manifest_hash(&encode(self))
    }
}

fn duplicate(i: usize, id: u64) -> ValidationError {
    ValidationError::new(
        codes::MANIFEST_DUPLICATE_ID,
        format!("entry {i}: id {id} appears more than once"),
    )
}

/// The manifest hash: SHA-256 of the manifest bytes exactly as stored
/// (`client-compute.md` section 5.1). Does not validate.
pub fn manifest_hash(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    sha256(bytes)
}

/// Encodes a manifest: the 12-byte header, then one 48-byte entry per tile in
/// id order.
pub fn encode(manifest: &Manifest) -> Vec<u8> {
    let entries = manifest.entries();
    let count = u32::try_from(entries.len()).expect("Manifest::new bounds the entry count");
    let mut out = Vec::with_capacity(HEADER_LEN + ENTRY_LEN * entries.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.id.to_le_bytes());
        out.extend_from_slice(&e.sha256);
        out.extend_from_slice(&e.length.to_le_bytes());
    }
    out
}

/// Decodes and validates a manifest.
///
/// Checks, in order, and reports the first failing rule: length at least 12
/// (901), magic (902), version (903), the reserved field (904), length
/// exactly `12 + 48 * entry_count` (905). Then entry by entry: ids strictly
/// ascending (906 if lower than the previous, 907 if equal). Unsorted input is
/// an error; the decoder never reorders entries.
pub fn decode(bytes: &[u8]) -> Result<Manifest, ValidationError> {
    if bytes.len() < HEADER_LEN {
        return Err(ValidationError::new(
            codes::MANIFEST_HEADER_TOO_SHORT,
            format!("length: {} bytes, header needs {HEADER_LEN}", bytes.len()),
        ));
    }
    if bytes[0..4] != MAGIC {
        return Err(ValidationError::new(
            codes::MANIFEST_BAD_MAGIC,
            format!(
                "magic: {:02x} {:02x} {:02x} {:02x} is not 33 47 58 4d",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
        ));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(ValidationError::new(
            codes::MANIFEST_UNSUPPORTED_VERSION,
            format!("version: {version} is not {VERSION}"),
        ));
    }
    let reserved = u16::from_le_bytes([bytes[6], bytes[7]]);
    if reserved != 0 {
        return Err(ValidationError::new(
            codes::MANIFEST_RESERVED_NONZERO,
            format!("reserved u16 at offset 6: {reserved} is not 0"),
        ));
    }
    let count = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
    // u64 arithmetic: cannot overflow, and is the same on 32-bit targets.
    let want = HEADER_LEN as u64 + ENTRY_LEN as u64 * u64::from(count);
    if bytes.len() as u64 != want {
        return Err(ValidationError::new(
            codes::MANIFEST_LENGTH_MISMATCH,
            format!(
                "length: {} bytes, entry_count {count} needs {want}",
                bytes.len()
            ),
        ));
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(count as usize);
    for (i, r) in bytes[HEADER_LEN..]
        .as_chunks::<ENTRY_LEN>()
        .0
        .iter()
        .enumerate()
    {
        let e = Entry {
            id: u64::from_le_bytes(r[0..8].try_into().expect("8 bytes")),
            sha256: r[8..40].try_into().expect("32 bytes"),
            length: u64::from_le_bytes(r[40..48].try_into().expect("8 bytes")),
        };
        if let Some(prev) = entries.last() {
            if e.id < prev.id {
                return Err(ValidationError::new(
                    codes::MANIFEST_NOT_SORTED,
                    format!(
                        "entry {i}: id {} follows {}, entries must ascend",
                        e.id, prev.id
                    ),
                ));
            }
            if e.id == prev.id {
                return Err(duplicate(i, e.id));
            }
        }
        entries.push(e);
    }
    Ok(Manifest { entries })
}

/// Validates a manifest: [`decode`] without the result.
pub fn validate(bytes: &[u8]) -> Result<(), ValidationError> {
    decode(bytes).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, fill: u8, length: u64) -> Entry {
        Entry {
            id,
            sha256: [fill; 32],
            length,
        }
    }

    fn code<T: core::fmt::Debug>(r: Result<T, ValidationError>) -> u16 {
        r.unwrap_err().code
    }

    #[test]
    fn round_trip_and_lookup() {
        let m = Manifest::new(vec![entry(30, 3, 300), entry(1, 1, 10), entry(7, 2, 0)]).unwrap();
        let ids: Vec<u64> = m.entries().iter().map(|e| e.id).collect();
        assert_eq!(ids, [1, 7, 30]);
        let b = encode(&m);
        assert_eq!(b.len(), HEADER_LEN + 3 * ENTRY_LEN);
        assert_eq!(&b[0..4], b"3GXM");
        assert_eq!(&b[4..8], &[1, 0, 0, 0]);
        assert_eq!(&b[8..12], &3u32.to_le_bytes());
        assert_eq!(&b[12..20], &1u64.to_le_bytes());
        assert_eq!(&b[20..52], &[1u8; 32]);
        assert_eq!(&b[52..60], &10u64.to_le_bytes());
        assert_eq!(decode(&b).unwrap(), m);
        assert_eq!(m.lookup(7), Some(([2u8; 32], 0)));
        assert_eq!(m.lookup(30), Some(([3u8; 32], 300)));
        assert_eq!(m.lookup(8), None);
        assert_eq!(m.hash(), sha256(&b));
        assert_eq!(manifest_hash(&b), m.hash());

        let empty = Manifest::new(vec![]).unwrap();
        assert_eq!(encode(&empty).len(), HEADER_LEN);
        assert_eq!(decode(&encode(&empty)).unwrap(), empty);
        assert_eq!(empty.lookup(0), None);
    }

    #[test]
    fn rejects_each_rule() {
        assert_eq!(
            code(Manifest::new(vec![entry(1, 0, 0), entry(1, 1, 1)])),
            907
        );
        let good = encode(&Manifest::new(vec![entry(1, 1, 1), entry(2, 2, 2)]).unwrap());
        let edit = |at: usize, v: u8| {
            let mut b = good.clone();
            b[at] = v;
            code(decode(&b))
        };
        assert_eq!(code(decode(&good[..11])), 901);
        assert_eq!(code(decode(&[])), 901);
        assert_eq!(edit(3, b'Q'), 902);
        assert_eq!(edit(4, 2), 903);
        assert_eq!(edit(7, 1), 904);
        assert_eq!(edit(8, 3), 905);
        assert_eq!(code(decode(&good[..good.len() - 1])), 905);
        assert_eq!(edit(12, 3), 906);
        assert_eq!(edit(12 + ENTRY_LEN, 1), 907);
    }
}
