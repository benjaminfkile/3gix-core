//! The top-level validator: one submitted section under one chunk key.
//!
//! Implements `matter-format.md` section 4 as the hub runs it on a
//! submission, including step 3, the dispatch between a matter section and a
//! frame registry by key. The C ABI's `gx_validate` and the WebAssembly
//! `validate` export are thin wrappers over [`validate`].

use crate::error::{codes, ValidationError};
use crate::key::ChunkKey;
use crate::{matter, registry};

/// Validates `bytes` submitted under the chunk key string `key`.
///
/// Parses `key` per section 2; a key that does not parse fails with 206
/// (`KEY_MALFORMED`). If the key is `registry`, the bytes are checked with
/// [`registry::validate`] (codes 601 to 625). Otherwise they are checked with
/// [`matter::validate`] against the cell key (codes 101 to 526). Reports the
/// first failing rule.
pub fn validate(key: &str, bytes: &[u8]) -> Result<(), ValidationError> {
    match key.parse::<ChunkKey>() {
        Err(e) => Err(ValidationError::new(
            codes::KEY_MALFORMED,
            format!("key: {e}"),
        )),
        Ok(ChunkKey::Registry) => registry::validate(bytes),
        Ok(ChunkKey::Cell(cell)) => matter::validate(&cell, bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::CellKey;
    use crate::matter::{Compression, Section};
    use crate::registry::Registry;
    use crate::units::{Meters, Seconds};

    fn matter_bytes() -> Vec<u8> {
        let key = CellKey::new(7, 3, 1, 2, 5).unwrap();
        let g = key.geometry(Meters::new(8.0));
        matter::encode(
            &Section::empty(key, g.origin, g.edge).unwrap(),
            Compression::None,
        )
    }

    #[test]
    fn dispatches_by_key() {
        let m = matter_bytes();
        let r = registry::encode(&Registry::empty(Seconds::new(0.0)).unwrap());
        assert_eq!(validate("7-3-1-2-5", &m), Ok(()));
        assert_eq!(validate("registry", &r), Ok(()));
        assert_eq!(
            validate("registry", &m).unwrap_err().code,
            codes::REGISTRY_BAD_MAGIC
        );
        assert_eq!(
            validate("7-3-1-2-4", &m).unwrap_err().code,
            codes::CELL_Z_MISMATCH
        );
        // An empty registry is 24 bytes, shorter than a matter header.
        assert_eq!(
            validate("7-3-1-2-5", &r).unwrap_err().code,
            codes::HEADER_TOO_SHORT
        );
    }

    #[test]
    fn bad_keys_are_2xx() {
        for k in [
            "",
            "Registry",
            "1-2-3",
            "1-32-0-0-0",
            "1-1-2-0-0",
            "01-0-0-0-0",
        ] {
            let e = validate(k, &matter_bytes()).unwrap_err();
            assert_eq!(e.code, codes::KEY_MALFORMED, "{k:?}");
            assert!(e.reason.starts_with("key: "), "{}", e.reason);
        }
    }
}
