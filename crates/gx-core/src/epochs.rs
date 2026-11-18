//! Epoch indexes: encode, decode, validate, hash, and the epoch rule.
//!
//! Implements `client-compute.md` section 5.4. A layer whose records grow
//! over time names an epoch index instead of a single source manifest. The
//! index cuts time into epochs of a fixed length and lists, for each epoch
//! that has records, the hash of that epoch's manifest:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `3GXE` |
//! | 4 | 2 | version, `u16` LE, 1 |
//! | 6 | 2 | reserved, 0 |
//! | 8 | 8 | epoch seconds, `u64` LE, greater than 0 |
//! | 16 | 4 | entry count, `u32` LE |
//! | 20 | 40 each | entries ascending by epoch index, unique: `epoch index` `u64` LE, `manifest hash` 32 bytes |
//!
//! The index's hash is the SHA-256 of these bytes ([`epochs_hash`]). The
//! epoch rule ([`manifest_for_span`]) maps span index `s` of a layer with
//! span length `span_seconds` to epoch `floor(s * span_seconds /
//! epoch_seconds)` and returns that epoch's manifest hash, or `None` when the
//! epoch is not listed. The section address (section 3) takes that manifest
//! hash, all zero when none, never the index's hash. Every byte [`encode`]
//! writes is a pure function of the index.

use crate::error::{codes, ValidationError};
use crate::sha256::{sha256, DIGEST_LEN};

/// The four magic bytes that open an epoch index: `3GXE` in ASCII.
pub const MAGIC: [u8; 4] = *b"3GXE";

/// The epoch index version this module reads and writes.
pub const VERSION: u16 = 1;

/// Length of the epoch index header in bytes.
pub const HEADER_LEN: usize = 20;

/// Length of one entry in bytes.
pub const ENTRY_LEN: usize = 40;

/// One epoch: its index and the hash of its source manifest.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// The epoch index, `floor(time since the span origin / epoch seconds)`.
    pub epoch: u64,
    /// SHA-256 of the epoch's source manifest bytes.
    pub manifest: [u8; DIGEST_LEN],
}

/// One validated epoch index: a positive epoch length and entries strictly
/// ascending by epoch index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochIndex {
    epoch_seconds: u64,
    entries: Vec<Entry>,
}

impl EpochIndex {
    /// Builds an epoch index, sorting `entries` by epoch index ascending.
    ///
    /// Fails with 1005 if `epoch_seconds` is 0, with 1008 if there are more
    /// entries than a `u32` counts, or with 1007 if an epoch index appears
    /// more than once.
    pub fn new(epoch_seconds: u64, mut entries: Vec<Entry>) -> Result<EpochIndex, ValidationError> {
        if epoch_seconds == 0 {
            return Err(zero_epoch());
        }
        if u32::try_from(entries.len()).is_err() {
            return Err(ValidationError::new(
                codes::EPOCHS_TOO_MANY_ENTRIES,
                format!("entry_count: {} entries do not fit a u32", entries.len()),
            ));
        }
        entries.sort_by_key(|e| e.epoch);
        if let Some(i) = (1..entries.len()).find(|&i| entries[i - 1].epoch == entries[i].epoch) {
            return Err(not_ascending(i, entries[i].epoch, entries[i - 1].epoch));
        }
        Ok(EpochIndex {
            epoch_seconds,
            entries,
        })
    }

    /// The epoch length in seconds, greater than 0.
    pub fn epoch_seconds(&self) -> u64 {
        self.epoch_seconds
    }

    /// The entries, sorted by epoch index ascending.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The manifest hash of epoch `epoch`, or `None` if the index does not
    /// list it.
    pub fn lookup(&self, epoch: u64) -> Option<[u8; DIGEST_LEN]> {
        self.entries
            .binary_search_by_key(&epoch, |e| e.epoch)
            .ok()
            .map(|i| self.entries[i].manifest)
    }

    /// The SHA-256 of this index's encoded bytes.
    pub fn hash(&self) -> [u8; DIGEST_LEN] {
        epochs_hash(&encode(self))
    }
}

fn zero_epoch() -> ValidationError {
    ValidationError::new(
        codes::EPOCHS_ZERO_EPOCH_SECONDS,
        "epoch_seconds: 0, must be greater than 0",
    )
}

fn not_ascending(i: usize, epoch: u64, prev: u64) -> ValidationError {
    ValidationError::new(
        codes::EPOCHS_NOT_ASCENDING,
        format!("entry {i}: epoch index {epoch} follows {prev}, entries must strictly ascend"),
    )
}

/// The epoch index hash: SHA-256 of the index bytes exactly as stored
/// (`client-compute.md` section 5.4). Does not validate.
pub fn epochs_hash(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    sha256(bytes)
}

/// Encodes an epoch index: the 20-byte header, then one 40-byte entry per
/// epoch in epoch order.
pub fn encode(index: &EpochIndex) -> Vec<u8> {
    let entries = index.entries();
    let count = u32::try_from(entries.len()).expect("EpochIndex::new bounds the entry count");
    let mut out = Vec::with_capacity(HEADER_LEN + ENTRY_LEN * entries.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&index.epoch_seconds.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.epoch.to_le_bytes());
        out.extend_from_slice(&e.manifest);
    }
    out
}

/// Decodes and validates an epoch index.
///
/// Checks, in order, and reports the first failing rule: length at least 20
/// (1001), magic (1002), version (1003), the reserved field (1004), epoch
/// seconds greater than 0 (1005), length exactly `20 + 40 * entry_count`
/// (1006). Then entry by entry: epoch indexes strictly ascending (1007).
/// Unsorted input is an error; the decoder never reorders entries.
pub fn decode(bytes: &[u8]) -> Result<EpochIndex, ValidationError> {
    if bytes.len() < HEADER_LEN {
        return Err(ValidationError::new(
            codes::EPOCHS_HEADER_TOO_SHORT,
            format!("length: {} bytes, header needs {HEADER_LEN}", bytes.len()),
        ));
    }
    if bytes[0..4] != MAGIC {
        return Err(ValidationError::new(
            codes::EPOCHS_BAD_MAGIC,
            format!(
                "magic: {:02x} {:02x} {:02x} {:02x} is not 33 47 58 45",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
        ));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(ValidationError::new(
            codes::EPOCHS_UNSUPPORTED_VERSION,
            format!("version: {version} is not {VERSION}"),
        ));
    }
    let reserved = u16::from_le_bytes([bytes[6], bytes[7]]);
    if reserved != 0 {
        return Err(ValidationError::new(
            codes::EPOCHS_RESERVED_NONZERO,
            format!("reserved u16 at offset 6: {reserved} is not 0"),
        ));
    }
    let epoch_seconds = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
    if epoch_seconds == 0 {
        return Err(zero_epoch());
    }
    let count = u32::from_le_bytes(bytes[16..20].try_into().expect("4 bytes"));
    // u64 arithmetic: cannot overflow, and is the same on 32-bit targets.
    let want = HEADER_LEN as u64 + ENTRY_LEN as u64 * u64::from(count);
    if bytes.len() as u64 != want {
        return Err(ValidationError::new(
            codes::EPOCHS_LENGTH_MISMATCH,
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
            epoch: u64::from_le_bytes(r[0..8].try_into().expect("8 bytes")),
            manifest: r[8..40].try_into().expect("32 bytes"),
        };
        if let Some(prev) = entries.last() {
            if e.epoch <= prev.epoch {
                return Err(not_ascending(i, e.epoch, prev.epoch));
            }
        }
        entries.push(e);
    }
    Ok(EpochIndex {
        epoch_seconds,
        entries,
    })
}

/// Validates an epoch index: [`decode`] without the result.
pub fn validate(bytes: &[u8]) -> Result<(), ValidationError> {
    decode(bytes).map(|_| ())
}

/// Checks that a layer with span length `span_seconds` can use `index`:
/// `span_seconds` is greater than 0 (1009) and the index's epoch length is a
/// whole multiple of it (1010), so every span lies inside one epoch.
pub fn check_layer(index: &EpochIndex, span_seconds: u64) -> Result<(), ValidationError> {
    if span_seconds == 0 {
        return Err(ValidationError::new(
            codes::EPOCHS_SPAN_SECONDS_ZERO,
            "span_seconds: 0, must be greater than 0",
        ));
    }
    if !index.epoch_seconds.is_multiple_of(span_seconds) {
        return Err(ValidationError::new(
            codes::EPOCHS_SPAN_NOT_DIVISOR,
            format!(
                "epoch_seconds {} is not a whole multiple of span_seconds {span_seconds}",
                index.epoch_seconds
            ),
        ));
    }
    Ok(())
}

/// The epoch of span `span_index` of a layer with span length
/// `span_seconds`: `floor(span_index * span_seconds / epoch_seconds)`,
/// computed in `u128` so it never overflows. Fails as [`check_layer`] does.
pub fn epoch_for_span(
    index: &EpochIndex,
    span_seconds: u64,
    span_index: u64,
) -> Result<u64, ValidationError> {
    check_layer(index, span_seconds)?;
    let e = u128::from(span_index) * u128::from(span_seconds) / u128::from(index.epoch_seconds);
    // span_seconds <= epoch_seconds after check_layer, so e <= span_index.
    Ok(u64::try_from(e).expect("the epoch is at most the span index"))
}

/// The epoch rule (`client-compute.md` section 5.4): the manifest hash of
/// the epoch holding span `span_index` of a layer with span length
/// `span_seconds`, or `None` when the index does not list that epoch. Fails
/// with 1009 when `span_seconds` is 0 and 1010 when the epoch length is not
/// a whole multiple of `span_seconds`.
pub fn manifest_for_span(
    index: &EpochIndex,
    span_seconds: u64,
    span_index: u64,
) -> Result<Option<[u8; DIGEST_LEN]>, ValidationError> {
    epoch_for_span(index, span_seconds, span_index).map(|e| index.lookup(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(epoch: u64, fill: u8) -> Entry {
        Entry {
            epoch,
            manifest: [fill; 32],
        }
    }

    fn code<T: core::fmt::Debug>(r: Result<T, ValidationError>) -> u16 {
        r.unwrap_err().code
    }

    #[test]
    fn round_trip_and_lookup() {
        let x = EpochIndex::new(86_400, vec![entry(9, 3), entry(0, 1), entry(4, 2)]).unwrap();
        let epochs: Vec<u64> = x.entries().iter().map(|e| e.epoch).collect();
        assert_eq!(epochs, [0, 4, 9]);
        let b = encode(&x);
        assert_eq!(b.len(), HEADER_LEN + 3 * ENTRY_LEN);
        assert_eq!(&b[0..4], b"3GXE");
        assert_eq!(&b[4..8], &[1, 0, 0, 0]);
        assert_eq!(&b[8..16], &86_400u64.to_le_bytes());
        assert_eq!(&b[16..20], &3u32.to_le_bytes());
        assert_eq!(&b[20..28], &0u64.to_le_bytes());
        assert_eq!(&b[28..60], &[1u8; 32]);
        assert_eq!(decode(&b).unwrap(), x);
        assert_eq!(x.lookup(4), Some([2u8; 32]));
        assert_eq!(x.lookup(5), None);
        assert_eq!(x.hash(), sha256(&b));

        let empty = EpochIndex::new(1, vec![]).unwrap();
        assert_eq!(decode(&encode(&empty)).unwrap(), empty);
    }

    #[test]
    fn epoch_rule() {
        let x = EpochIndex::new(86_400, vec![entry(0, 1), entry(1, 2)]).unwrap();
        // One-hour spans: 24 per epoch.
        assert_eq!(manifest_for_span(&x, 3600, 0).unwrap(), Some([1u8; 32]));
        assert_eq!(manifest_for_span(&x, 3600, 23).unwrap(), Some([1u8; 32]));
        assert_eq!(manifest_for_span(&x, 3600, 24).unwrap(), Some([2u8; 32]));
        assert_eq!(manifest_for_span(&x, 3600, 48).unwrap(), None);
        assert_eq!(manifest_for_span(&x, 86_400, 1).unwrap(), Some([2u8; 32]));
        // No overflow at the top of the range.
        let big = EpochIndex::new(u64::MAX, vec![entry(u64::MAX - 1, 7)]).unwrap();
        assert_eq!(epoch_for_span(&big, u64::MAX, u64::MAX).unwrap(), u64::MAX);
        assert_eq!(epoch_for_span(&x, 3600, u64::MAX).unwrap(), u64::MAX / 24);
        assert_eq!(code(manifest_for_span(&x, 0, 0)), 1009);
        assert_eq!(code(manifest_for_span(&x, 7, 0)), 1010);
        assert_eq!(code(check_layer(&x, 86_401)), 1010);
        assert!(check_layer(&x, 86_400).is_ok());
    }

    #[test]
    fn rejects_each_rule() {
        assert_eq!(code(EpochIndex::new(0, vec![])), 1005);
        assert_eq!(
            code(EpochIndex::new(1, vec![entry(1, 0), entry(1, 1)])),
            1007
        );
        let good = encode(&EpochIndex::new(60, vec![entry(1, 1), entry(2, 2)]).unwrap());
        let edit = |at: usize, v: u8| {
            let mut b = good.clone();
            b[at] = v;
            code(decode(&b))
        };
        assert_eq!(code(decode(&good[..19])), 1001);
        assert_eq!(code(decode(&[])), 1001);
        assert_eq!(edit(3, b'M'), 1002);
        assert_eq!(edit(4, 2), 1003);
        assert_eq!(edit(7, 1), 1004);
        assert_eq!(edit(8, 0), 1005);
        assert_eq!(edit(16, 3), 1006);
        assert_eq!(code(decode(&good[..good.len() - 1])), 1006);
        assert_eq!(edit(20, 2), 1007);
        assert_eq!(edit(20, 3), 1007);
    }
}
