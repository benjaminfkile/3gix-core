//! A tiny example module for an overlay layer, for tests.
//!
//! Implements `client-compute.md` section 4 (module ABI v1) for a layer with
//! `overlay` true (`client-compute.md` section 2.2), whose output is overlay
//! tiles (`matter-format.md` section 12):
//!
//! - For a surface key of frame [`OVERLAY_FRAME_ID`] (the frame the example
//!   frame module declares) it returns an overlay tile whose coverage,
//!   albedo, and roughness are a fixed pattern of the vertex direction
//!   ([`material`]): full coverage in a band about the frame's xy plane, partial at
//!   its edges, none elsewhere. For a surface key of any other frame it
//!   returns the empty overlay tile.
//! - For a cell key it returns the empty section of the cell under a root
//!   extent of [`ROOT_EXTENT`], for `registry` the empty registry, and for a
//!   frame key the empty frame record. It declares no frames.
//!
//! It reads no sources. Build it for `wasm32-unknown-unknown` to get the
//! module.

use gx_core::frame_record::{self, FrameRecord};
use gx_core::key::{ChunkKey, SurfaceKey};
use gx_core::matter::{self, Compression, Section};
use gx_core::overlay::{self, OverlayTile, PARENT_VERTICES, TILE_VERTICES};
use gx_core::registry::{self, Registry};
use gx_core::surface::vertex_direction;
use gx_core::units::Meters;
use gx_module_guest::{gx_module, Module, Request, Tiles};

/// The example overlay module.
pub struct OverlayModule;

/// The frame whose surface keys get overlay tiles: the example frame
/// module's frame.
pub const OVERLAY_FRAME_ID: u64 = 7;

/// Edge of the root cube the empty sections are placed under, meters.
pub const ROOT_EXTENT: f64 = 2.0e7;

/// The band's half width in the direction's `z`: full coverage below it.
pub const BAND_INNER: f64 = 0.25;

/// Where the coverage reaches 0, in the direction's `z`.
pub const BAND_OUTER: f64 = 0.35;

/// Rounds `v` in 0 to 1 to the nearest stored `u8`.
fn byte(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Coverage, albedo, and roughness at the unit direction `d` (frame axes):
/// coverage 1 where `|z|` is below [`BAND_INNER`], falling linearly to 0 at
/// [`BAND_OUTER`]; albedo bands that vary with `x` and `y`; roughness that
/// varies with `z`. Only `+`, `-`, `*`, and `/` on the direction, so every
/// machine computes the same bytes.
pub fn material(d: [f64; 3]) -> (u8, [u8; 3], u8) {
    let z = d[2].abs();
    let coverage = byte((BAND_OUTER - z) / (BAND_OUTER - BAND_INNER));
    let albedo = [
        byte(0.1 + 0.05 * (d[0] + 1.0)),
        byte(0.2 + 0.1 * (d[1] + 1.0)),
        byte(0.6),
    ];
    let roughness = byte(0.1 + z);
    (coverage, albedo, roughness)
}

/// The overlay tile of `key`: [`material`] at every vertex, and at the even
/// vertices for the parent values (a child's even vertices are its
/// parent's vertices, so the parent's values there are the same pattern).
pub fn overlay_tile(key: &SurfaceKey) -> OverlayTile {
    let at = |i: usize, j: usize| material(vertex_direction(key, i as i64, j as i64));
    let own: Vec<_> = (0..TILE_VERTICES)
        .flat_map(|j| (0..TILE_VERTICES).map(move |i| (i, j)))
        .map(|(i, j)| at(i, j))
        .collect();
    let parent: Vec<_> = (0..PARENT_VERTICES)
        .flat_map(|m| (0..PARENT_VERTICES).map(move |k| (k, m)))
        .map(|(k, m)| at(2 * k, 2 * m))
        .collect();
    OverlayTile {
        key: *key,
        coverage: own.iter().map(|v| v.0).collect(),
        albedo: own.iter().map(|v| v.1).collect(),
        roughness: own.iter().map(|v| v.2).collect(),
        parent_coverage: parent.iter().map(|v| v.0).collect(),
        parent_albedo: parent.iter().map(|v| v.1).collect(),
        parent_roughness: parent.iter().map(|v| v.2).collect(),
    }
}

impl Module for OverlayModule {
    fn sources(_req: &Request) -> Vec<u64> {
        Vec::new()
    }

    fn compile(req: &Request, _tiles: &Tiles) -> Result<Vec<u8>, String> {
        match &req.key {
            ChunkKey::Registry => Ok(registry::encode(&Registry::empty())),
            ChunkKey::Frame(id) => Ok(frame_record::encode(&FrameRecord::empty(*id))),
            ChunkKey::Cell(cell) => {
                let g = cell.geometry(Meters::new(ROOT_EXTENT));
                let s = Section::empty(*cell, g.origin, g.edge).map_err(|e| e.to_string())?;
                Ok(matter::encode(&s, Compression::None))
            }
            ChunkKey::Surface(tile) if tile.frame_id == OVERLAY_FRAME_ID => {
                Ok(overlay::encode(&overlay_tile(tile), Compression::Zstd))
            }
            ChunkKey::Surface(tile) => Ok(overlay::encode_empty(tile)),
        }
    }
}

gx_module!(OverlayModule);

#[cfg(test)]
mod tests {
    use super::*;
    use gx_core::module_io::{encode_request, encode_tiles};
    use gx_core::overlay::Overlay;
    use gx_core::validate_for_layer;
    use gx_module_guest::run_compile;

    fn compile(key: &str) -> Vec<u8> {
        let req = Request {
            origin_utc_ms: 0,
            span_seconds: 0,
            span_index: 0,
            key: key.parse().unwrap(),
        };
        run_compile::<OverlayModule>(&encode_request(&req), &encode_tiles(&[]).unwrap()).unwrap()
    }

    #[test]
    fn every_answer_passes_the_overlay_layer_check() {
        for key in [
            "registry",
            "frame-7",
            "7-1-0-1-0",
            "s-7-0-0-0-0",
            "s-7-4-1-1-0",
            "s-8-0-0-0-0",
        ] {
            assert_eq!(
                validate_for_layer(key, &compile(key), true),
                Ok(()),
                "{key}"
            );
        }
    }

    #[test]
    fn band_about_the_xy_plane() {
        let key = SurfaceKey::new(7, 0, 0, 0, 0).unwrap();
        let Overlay::Tile(t) = overlay::decode(&key, &compile("s-7-0-0-0-0")).unwrap() else {
            panic!("frame 7 has an overlay");
        };
        // The face center is in the xy plane; the face corner is far from it.
        assert_eq!(t.coverage[32 + 65 * 32], 255);
        assert_eq!(t.coverage[0], 0);
        assert!(t.coverage.iter().any(|&c| c > 0 && c < 255));
        assert_eq!(t.parent_coverage[16 + 33 * 16], 255);
        let z_axis = SurfaceKey::new(7, 4, 0, 0, 0).unwrap();
        let Overlay::Tile(p) = overlay::decode(&z_axis, &compile("s-7-4-0-0-0")).unwrap() else {
            panic!("frame 7 has an overlay");
        };
        assert!(p.coverage.iter().all(|&c| c == 0));
        let other = SurfaceKey::new(8, 0, 0, 0, 0).unwrap();
        assert_eq!(
            overlay::decode(&other, &compile("s-8-0-0-0-0")),
            Ok(Overlay::Empty(other))
        );
    }
}
