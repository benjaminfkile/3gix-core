//! A tiny example module that declares one frame, for tests.
//!
//! Implements `client-compute.md` section 4 (module ABI v1) for a layer that
//! declares a frame and holds no matter, so a frames slot (section 10) has
//! something to name:
//!
//! - For the registry key it returns a registry with the one frame
//!   [`fixture_frame`] (`matter-format.md` section 5).
//! - For the frame key `frame-{FIXTURE_FRAME_ID}` it returns that frame's
//!   record (`matter-format.md` section 11), and for any other frame key the
//!   empty record (section 11.2).
//! - For a cell key it returns the empty section of the cell under the
//!   frame's root extent, and for a surface key the empty surface tile.
//!
//! It reads no sources. The hub and the renderer use it as the layer a
//! frames slot names in their tests and the smoke. Build it for
//! `wasm32-unknown-unknown` to get the module.

use gx_core::frame_record::{self, FrameRecord};
use gx_core::key::ChunkKey;
use gx_core::matter::{self, Compression, Section};
use gx_core::registry::{self, Frame, Registry, ROOT_PARENT};
use gx_core::units::{Kilograms, Meters, Quat, Seconds, Vec3};
use gx_module_guest::{gx_module, Module, Request, Tiles};

/// The example module.
pub struct FrameModule;

/// The id of the one frame this module declares.
pub const FIXTURE_FRAME_ID: u64 = 7;

/// Edge of the fixture frame's root cube: 2.0e7 m.
pub const FIXTURE_ROOT_EXTENT: f64 = 2.0e7;

/// Mass of the fixture frame: 6.0e24 kg.
pub const FIXTURE_MASS: f64 = 6.0e24;

/// The one frame this module declares: a root frame at rest with a fixed
/// extent, mass, `j2`, and principal moments.
pub fn fixture_frame() -> Frame {
    Frame {
        frame_id: FIXTURE_FRAME_ID,
        parent_frame_id: ROOT_PARENT,
        root_extent: Meters::new(FIXTURE_ROOT_EXTENT),
        max_depth: 12,
        mass: Kilograms::new(FIXTURE_MASS),
        position: Vec3::new(0.0, 0.0, 0.0),
        velocity: Vec3::new(0.0, 0.0, 0.0),
        orientation: Quat::identity(),
        angular_velocity: Vec3::new(0.0, 0.0, 7.0e-5),
        j2: 1.0e-3,
        reference_radius: Meters::new(6.0e6),
        principal_moments: Vec3::new(8.0e37, 8.0e37, 8.1e37),
    }
}

/// The record of the fixture frame (`matter-format.md` section 11).
pub fn fixture_record() -> FrameRecord {
    FrameRecord::from_registry_frame(&fixture_frame())
}

impl Module for FrameModule {
    fn sources(_req: &Request) -> Vec<u64> {
        Vec::new()
    }

    fn compile(req: &Request, _tiles: &Tiles) -> Result<Vec<u8>, String> {
        match &req.key {
            ChunkKey::Registry => {
                let r = Registry::new(Seconds::new(0.0), vec![fixture_frame()])
                    .map_err(|e| e.to_string())?;
                Ok(registry::encode(&r))
            }
            ChunkKey::Frame(id) if *id == FIXTURE_FRAME_ID => {
                Ok(frame_record::encode(&fixture_record()))
            }
            ChunkKey::Frame(id) => Ok(frame_record::encode(&FrameRecord::empty(*id))),
            ChunkKey::Cell(cell) => {
                let g = cell.geometry(Meters::new(FIXTURE_ROOT_EXTENT));
                let s = Section::empty(*cell, g.origin, g.edge).map_err(|e| e.to_string())?;
                Ok(matter::encode(&s, Compression::None))
            }
            ChunkKey::Surface(tile) => Ok(gx_core::surface::encode_empty(tile)),
        }
    }
}

gx_module!(FrameModule);

#[cfg(test)]
mod tests {
    use super::*;
    use gx_core::module_io::{encode_request, encode_tiles};
    use gx_module_guest::run_compile;

    fn compile(key: &str) -> Vec<u8> {
        let req = Request {
            origin_utc_ms: 0,
            span_seconds: 0,
            span_index: 0,
            key: key.parse().unwrap(),
        };
        run_compile::<FrameModule>(&encode_request(&req), &encode_tiles(&[]).unwrap()).unwrap()
    }

    #[test]
    fn registry_record_and_cells() {
        let r = registry::decode(&compile("registry")).unwrap();
        assert_eq!(r.frames(), &[fixture_frame()]);
        let rec = frame_record::decode(&compile("frame-7")).unwrap();
        assert_eq!(rec, fixture_record());
        assert!(frame_record::agrees(&fixture_frame(), &rec).is_ok());
        assert!(frame_record::decode(&compile("frame-8")).unwrap().empty);
        let cell = gx_core::key::CellKey::new(7, 1, 0, 1, 0).unwrap();
        let s = matter::decode(&cell, &compile("7-1-0-1-0")).unwrap();
        assert_eq!(s.mass().value(), 0.0);
    }
}
