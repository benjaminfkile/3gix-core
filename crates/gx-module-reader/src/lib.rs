//! A tiny example module of the module ABI version 2, for tests and
//! conformance.
//!
//! Implements `client-compute.md` section 4.1 and section 10 with the
//! smallest module that reads both kinds of input. Its layer config has two
//! slots:
//!
//! - slot 0 ([`FRAMES_SLOT`]), a frames slot of one frame, whose record
//!   arrives decoded;
//! - slot 1 ([`LAYER_SLOT`]), a layer slot reading the layer of the version
//!   1 example module (`crates/gx-module-test`).
//!
//! For a cell key it asks slot 1 for the same key and, below depth 0, the
//! parent cell ([`input_keys`]). It checks that the record describes the
//! request key's frame (and fails on a mismatch), and returns a uniform gas
//! section whose density is [`density`]: a fixed function of the mean
//! densities of the input sections and the record's mass. For the registry
//! key it returns an empty registry, for a frame key the empty record, and
//! for a surface key the empty tile. It reads no sources.
//!
//! Build it for `wasm32-unknown-unknown` to get the module; the native
//! `rlib` is what the conformance generator and the tests call.

use gx_core::frame_record::{self, FrameRecord};
use gx_core::key::{CellKey, ChunkKey};
use gx_core::matter::{self, Compression, Sample, Samples, Section, State};
use gx_core::module_io::InputEntry;
use gx_core::registry::{self, Registry};
use gx_core::units::{Attenuation, Density, Kelvin, Ratio};
use gx_module_guest::{gx_module_v2, FramesSlot, Input, ModuleV2, Request, Tiles};

/// The example module.
pub struct ReaderModule;

/// The frames slot of the config: slot 0, one frame.
pub const FRAMES_SLOT: u32 = 0;

/// The layer slot of the config: slot 1, the version 1 example module's
/// layer.
pub const LAYER_SLOT: u32 = 1;

/// Samples per axis of every section this module returns.
pub const RESOLUTION: u8 = 4;

/// Temperature of the gas, kelvin.
pub const TEMPERATURE: f64 = 150.0;

/// Weight of the parent cell's mean density in [`density`].
pub const PARENT_WEIGHT: f64 = 0.5;

/// Density the frame's mass adds per kilogram, in kilograms per cubic meter
/// per kilogram.
pub const MASS_SCALE: f64 = 1.0e-27;

/// The keys a cell reads from slot 1: the cell itself and, below depth 0,
/// its parent, sorted by key bytes as the input list requires.
pub fn input_keys(cell: &CellKey) -> Vec<ChunkKey> {
    let mut keys = vec![ChunkKey::Cell(*cell)];
    if let Some(p) = cell.parent() {
        keys.push(ChunkKey::Cell(p));
    }
    keys.sort_by(|a, b| a.to_string().as_bytes().cmp(b.to_string().as_bytes()));
    keys
}

/// The gas density of a cell, kilograms per cubic meter: the cell's own
/// input mean density, plus [`PARENT_WEIGHT`] times the parent's (0 at depth
/// 0), plus [`MASS_SCALE`] times the frame's mass.
pub fn density(own: f64, parent: f64, mass: f64) -> f64 {
    own + PARENT_WEIGHT * parent + MASS_SCALE * mass
}

/// The mean density of an input section: its mass over its cube's volume,
/// 0 for an empty section.
fn mean_density(key: &CellKey, bytes: &[u8]) -> Result<f64, String> {
    let s = matter::decode(key, bytes).map_err(|e| format!("input {key}: {e}"))?;
    let edge = s.edge().value();
    Ok(s.mass().value() / (edge * edge * edge))
}

/// The section for one cell, given the frame record and the inputs.
pub fn cell_section(
    cell: &CellKey,
    record: &FrameRecord,
    inputs: &[Input],
) -> Result<Vec<u8>, String> {
    if record.empty || record.frame_id != cell.frame_id {
        return Err(format!(
            "frame mismatch: the record describes frame {}{}, the key is in frame {}",
            record.frame_id,
            if record.empty { " (empty)" } else { "" },
            cell.frame_id
        ));
    }
    let mean = |k: &CellKey| -> Result<f64, String> {
        let input = inputs
            .iter()
            .find(|i| i.slot == LAYER_SLOT && i.key == ChunkKey::Cell(*k))
            .ok_or_else(|| format!("input {k} missing"))?;
        mean_density(k, input.bytes)
    };
    let own = mean(cell)?;
    let parent = match cell.parent() {
        Some(p) => mean(&p)?,
        None => 0.0,
    };
    let d = density(own, parent, record.mass.value());
    let g = cell.geometry(record.root_extent);
    let sample = if d > 0.0 {
        Sample {
            density: Density::new(d),
            state: State::Gas,
            temperature: Kelvin::new(TEMPERATURE),
            albedo: [Ratio::new(0.5), Ratio::new(0.5), Ratio::new(0.5)],
            roughness: Ratio::new(0.5),
            attenuation: Attenuation::new(0.001),
            radiating_temperature: Kelvin::new(TEMPERATURE),
            occupancy: Ratio::new(1.0),
        }
    } else {
        Sample::VACUUM
    };
    let samples = Samples::from_fn(RESOLUTION, |_, _, _| sample);
    let section =
        Section::new(*cell, g.origin, g.edge, RESOLUTION, samples).map_err(|e| e.to_string())?;
    Ok(matter::encode(&section, Compression::None))
}

impl ModuleV2 for ReaderModule {
    const FRAMES_SLOTS: &'static [FramesSlot] = &[FramesSlot {
        slot: FRAMES_SLOT,
        frames: 1,
    }];

    fn inputs(req: &Request, _frames: &[FrameRecord]) -> Vec<InputEntry> {
        match &req.key {
            ChunkKey::Cell(cell) => input_keys(cell)
                .into_iter()
                .map(|key| InputEntry {
                    slot: LAYER_SLOT,
                    key,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn sources(_req: &Request, _frames: &[FrameRecord]) -> Vec<u64> {
        Vec::new()
    }

    fn compile(
        req: &Request,
        _tiles: &Tiles,
        frames: &[FrameRecord],
        inputs: &[Input],
    ) -> Result<Vec<u8>, String> {
        match &req.key {
            ChunkKey::Registry => Ok(registry::encode(&Registry::empty())),
            // This module declares no frames (`matter-format.md` 11.2).
            ChunkKey::Frame(id) => Ok(frame_record::encode(&FrameRecord::empty(*id))),
            ChunkKey::Surface(tile) => Ok(gx_core::surface::encode_empty(tile)),
            ChunkKey::Cell(cell) => {
                let record = frames.first().ok_or("frame record missing")?;
                cell_section(cell, record, inputs)
            }
        }
    }
}

gx_module_v2!(ReaderModule);

#[cfg(test)]
mod tests {
    use super::*;
    use gx_core::module_io::{decode_input_list, encode_inputs, encode_request, encode_tiles};
    use gx_core::units::Meters;
    use gx_module_guest::{run_compile_v2, run_inputs};

    fn record() -> Vec<u8> {
        let mut r = FrameRecord::empty(7);
        r.empty = false;
        r.parent_frame_id = registry::ROOT_PARENT;
        r.root_extent = Meters::new(1.0e6);
        r.max_depth = 8;
        r.mass = gx_core::units::Kilograms::new(1.0e24);
        frame_record::encode(&r)
    }

    fn req(key: &str) -> Vec<u8> {
        encode_request(&Request {
            origin_utc_ms: 0,
            span_seconds: 0,
            span_index: 0,
            key: key.parse().unwrap(),
        })
    }

    fn empty_section(key: &CellKey) -> Vec<u8> {
        let g = key.geometry(Meters::new(1.0e6));
        matter::encode(
            &Section::empty(*key, g.origin, g.edge).unwrap(),
            Compression::None,
        )
    }

    #[test]
    fn reads_inputs_and_checks_the_frame() {
        let rec = record();
        let frames = encode_inputs(&[&rec]).unwrap();
        let list =
            decode_input_list(&run_inputs::<ReaderModule>(&req("7-1-1-0-1"), &frames).unwrap())
                .unwrap();
        assert_eq!(list.len(), 2);
        let cell = CellKey::new(7, 1, 1, 0, 1).unwrap();
        let parent = cell.parent().unwrap();
        let a = empty_section(&cell);
        let b = empty_section(&parent);
        let by_key = |k: &ChunkKey| if *k == ChunkKey::Cell(cell) { &a } else { &b };
        let mut all: Vec<&[u8]> = vec![&rec];
        all.extend(list.iter().map(|e| by_key(&e.key).as_slice()));
        let inputs = encode_inputs(&all).unwrap();
        let tiles = encode_tiles(&[]).unwrap();
        let out = run_compile_v2::<ReaderModule>(&req("7-1-1-0-1"), &tiles, &inputs).unwrap();
        let s = matter::decode(&cell, &out).unwrap();
        let edge = s.edge().value();
        let want = density(0.0, 0.0, 1.0e24) * edge * edge * edge;
        assert!(
            (s.mass().value() - want).abs() <= want * 1e-6,
            "{} {want}",
            s.mass().value()
        );

        // Another frame's key fails.
        let e = run_compile_v2::<ReaderModule>(
            &req("8-0-0-0-0"),
            &tiles,
            &encode_inputs(&[&rec, &a]).unwrap(),
        )
        .unwrap_err();
        assert!(e.contains("frame mismatch"), "{e}");

        // A frame key gets empty buffers.
        let out = run_compile_v2::<ReaderModule>(&req("frame-3"), &tiles, &[]).unwrap();
        assert!(frame_record::decode(&out).unwrap().empty);
    }
}
