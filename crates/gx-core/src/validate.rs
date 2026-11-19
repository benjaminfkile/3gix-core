//! The top-level validator: one submitted section under one chunk key.
//!
//! Implements `matter-format.md` section 4 as the hub runs it on a
//! submission, including step 3, the dispatch by key between a matter
//! section, a frame registry, a surface tile (section 10) or an overlay tile
//! (section 12) chosen by magic, and a frame record (section 11). The C ABI's
//! `gx_validate` and the WebAssembly `validate` export are thin wrappers over
//! [`validate`].
//!
//! [`validate_for_layer`] adds the rule of `matter-format.md` section 12.5
//! and `client-compute.md` section 2.2 that depends on the layer: an overlay
//! layer answers surface keys with overlay tiles and every other key with
//! nothing, and no other layer emits overlay tiles. The C ABI's
//! `gx_validate_layer` and the WebAssembly `validate_layer` export wrap it.

use crate::error::{codes, ValidationError};
use crate::key::ChunkKey;
use crate::{frame_record, matter, overlay, registry, surface};

/// Validates `bytes` submitted under the chunk key string `key`.
///
/// Parses `key` per section 2; a key that does not parse fails with 206
/// (`KEY_MALFORMED`). If the key is `registry`, the bytes are checked with
/// [`registry::validate`] (codes 601 to 625). Otherwise they are checked with
/// [`matter::validate`] against the cell key (codes 101 to 526). A surface
/// tile key checks bytes that start with `3GSO` with [`overlay::validate`]
/// (1051 to 1057) and any other bytes with [`surface::validate`] (980 to
/// 990), and a `frame-` key checks them with [`frame_record::validate`]
/// (1021 to 1029, `matter-format.md` section 11.3). Reports the first
/// failing rule.
pub fn validate(key: &str, bytes: &[u8]) -> Result<(), ValidationError> {
    match parse(key)? {
        ChunkKey::Registry => registry::validate(bytes),
        ChunkKey::Cell(cell) => matter::validate(&cell, bytes),
        ChunkKey::Surface(tile) if overlay::is_overlay(bytes) => overlay::validate(&tile, bytes),
        ChunkKey::Surface(tile) => surface::validate(&tile, bytes),
        ChunkKey::Frame(id) => frame_record::validate(id, bytes),
    }
}

fn parse(key: &str) -> Result<ChunkKey, ValidationError> {
    key.parse::<ChunkKey>()
        .map_err(|e| ValidationError::new(codes::KEY_MALFORMED, format!("key: {e}")))
}

/// Validates `bytes` submitted under the chunk key string `key` by a layer
/// that is an overlay layer when `overlay` is true (`matter-format.md`
/// section 12.5, `client-compute.md` section 2.2).
///
/// For an overlay layer, a surface key takes only an overlay tile, empty or
/// not ([`overlay::validate`]; a `3GSF` surface tile fails with 1058), a cell
/// key only the empty matter section, `registry` only the empty registry,
/// and a `frame-` key only the empty frame record: bytes that fail their
/// format report its code, and valid bytes that are not empty fail with 1058
/// (`OVERLAY_LAYER_REFUSED`). For any other layer, an overlay tile under a
/// surface key fails with 1059 (`OVERLAY_NOT_OVERLAY_LAYER`), and every
/// other submission is checked as [`validate`] does.
pub fn validate_for_layer(key: &str, bytes: &[u8], overlay: bool) -> Result<(), ValidationError> {
    let key = parse(key)?;
    let refused = |what: &str| {
        Err(ValidationError::new(
            codes::OVERLAY_LAYER_REFUSED,
            format!("an overlay layer emits {what}"),
        ))
    };
    if !overlay {
        return match key {
            ChunkKey::Surface(_) if overlay::is_overlay(bytes) => Err(ValidationError::new(
                codes::OVERLAY_NOT_OVERLAY_LAYER,
                "an overlay tile from a layer that is not an overlay layer",
            )),
            ChunkKey::Surface(tile) => surface::validate(&tile, bytes),
            ChunkKey::Registry => registry::validate(bytes),
            ChunkKey::Cell(cell) => matter::validate(&cell, bytes),
            ChunkKey::Frame(id) => frame_record::validate(id, bytes),
        };
    }
    match key {
        ChunkKey::Surface(_) if bytes.len() >= 4 && bytes[0..4] == surface::MAGIC => {
            refused("only overlay tiles under surface keys, not surface tiles")
        }
        ChunkKey::Surface(tile) => overlay::validate(&tile, bytes),
        ChunkKey::Registry => {
            if registry::decode(bytes)?.is_empty() {
                Ok(())
            } else {
                refused("only the empty registry")
            }
        }
        ChunkKey::Cell(cell) => {
            if matter::decode(&cell, bytes)?.is_empty() {
                Ok(())
            } else {
                refused("only empty matter sections under cell keys")
            }
        }
        ChunkKey::Frame(id) => {
            frame_record::validate(id, bytes)?;
            if frame_record::decode(bytes)?.empty {
                Ok(())
            } else {
                refused("only empty frame records")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::CellKey;
    use crate::matter::{Compression, Section};
    use crate::registry::Registry;
    use crate::units::Meters;

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
        let r = registry::encode(&Registry::empty());
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
    fn frame_keys_validate_frame_records() {
        let b = frame_record::encode(&frame_record::FrameRecord::empty(3));
        assert_eq!(validate("frame-3", &b), Ok(()));
        assert_eq!(
            validate("frame-4", &b).unwrap_err().code,
            codes::FRAME_RECORD_ID_MISMATCH
        );
        assert_eq!(
            validate("frame-3", &matter_bytes()).unwrap_err().code,
            codes::FRAME_RECORD_LENGTH
        );
        assert_eq!(
            validate("frame-03", &b).unwrap_err().code,
            codes::KEY_MALFORMED
        );
    }

    #[test]
    fn surface_keys_dispatch_on_magic() {
        use crate::key::SurfaceKey;
        let k = SurfaceKey::new(7, 1, 0, 0, 0).unwrap();
        let o = overlay::encode_empty(&k);
        let s = surface::encode_empty(&k);
        assert_eq!(validate("s-7-1-0-0-0", &o), Ok(()));
        assert_eq!(validate("s-7-1-0-0-0", &s), Ok(()));
        assert_eq!(
            validate("s-7-2-0-0-0", &o).unwrap_err().code,
            codes::OVERLAY_KEY_MISMATCH
        );
        let layer = |b: &[u8], ov| validate_for_layer("s-7-1-0-0-0", b, ov);
        assert_eq!(layer(&o, true), Ok(()));
        assert_eq!(layer(&s, false), Ok(()));
        assert_eq!(
            layer(&s, true).unwrap_err().code,
            codes::OVERLAY_LAYER_REFUSED
        );
        assert_eq!(
            layer(&o, false).unwrap_err().code,
            codes::OVERLAY_NOT_OVERLAY_LAYER
        );
        assert_eq!(
            layer(b"xx", true).unwrap_err().code,
            codes::OVERLAY_TOO_SHORT
        );
    }

    #[test]
    fn overlay_layers_emit_only_empty_sections() {
        let empty = matter_bytes();
        assert_eq!(validate_for_layer("7-3-1-2-5", &empty, true), Ok(()));
        assert_eq!(
            validate_for_layer("registry", &registry::encode(&Registry::empty()), true),
            Ok(())
        );
        let rec = frame_record::encode(&frame_record::FrameRecord::empty(3));
        assert_eq!(validate_for_layer("frame-3", &rec, true), Ok(()));
        assert_eq!(
            validate_for_layer("frame-4", &rec, true).unwrap_err().code,
            codes::FRAME_RECORD_ID_MISMATCH
        );
        assert_eq!(
            validate_for_layer("bad", &rec, true).unwrap_err().code,
            codes::KEY_MALFORMED
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
