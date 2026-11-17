//! A tiny example module for tests and conformance.
//!
//! Exercises every part of `client-compute.md` section 4 (module ABI v1)
//! with the smallest module that still reads a source tile and depends on
//! every request field:
//!
//! - For the registry key it reads no sources and returns an empty registry
//!   (`matter-format.md` section 5.3).
//! - For a cell key it asks for one tile whose id is derived from the key
//!   ([`tile_id`]), reads the cube's root extent, a base density, and a
//!   temperature from it ([`TileParams`]), and returns a gas section in
//!   matter format version 3. The gas lies in layers ([`fill`]): in every
//!   column of samples, it fills the lowest part of each period of
//!   [`LAYER_PERIOD`] samples along z, a share that varies from column to
//!   column. Each sample's occupancy is the exact share of its sub-cube the
//!   gas fills, so a cell holds full, partly filled, and vacuum samples. The
//!   gas density varies with the global sample coordinates, the span index,
//!   and the origin through [`gx_core::detmath`], so the bytes are identical
//!   on every machine.
//!
//! Build it for `wasm32-unknown-unknown` to get the module; the native
//! `rlib` is what the conformance generator and the tests call.

use gx_core::detmath;
use gx_core::key::{CellKey, ChunkKey};
use gx_core::matter::{self, Compression, Sample, Samples, Section, State};
use gx_core::registry::{self, Registry};
use gx_core::units::{Attenuation, Density, Kelvin, Meters, Ratio};
use gx_module_guest::{gx_module, Module, Request, Tiles};

/// The example module.
pub struct TestModule;

/// Length of a tile: three `f64` LE values.
pub const TILE_LEN: usize = 24;

/// The parameters one tile carries, private to this module: three `f64` LE
/// values in this order.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TileParams {
    /// Edge of the frame's root cube, meters. Finite and greater than 0.
    pub root_extent: f64,
    /// Mean density of the matter, kilograms per cubic meter. Finite and
    /// greater than 0.
    pub base_density: f64,
    /// Temperature of the matter, kelvin. Finite and at least 0.
    pub temperature: f64,
}

impl TileParams {
    /// Encodes the parameters as tile bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(TILE_LEN);
        out.extend_from_slice(&self.root_extent.to_le_bytes());
        out.extend_from_slice(&self.base_density.to_le_bytes());
        out.extend_from_slice(&self.temperature.to_le_bytes());
        out
    }

    /// Decodes and checks tile bytes.
    pub fn decode(bytes: &[u8]) -> Result<TileParams, String> {
        if bytes.len() != TILE_LEN {
            return Err(format!("tile: {} bytes, expected {TILE_LEN}", bytes.len()));
        }
        let f = |i: usize| f64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().expect("8"));
        let p = TileParams {
            root_extent: f(0),
            base_density: f(1),
            temperature: f(2),
        };
        if !(p.root_extent.is_finite() && p.root_extent > 0.0) {
            return Err(format!("tile: root extent {} m", p.root_extent));
        }
        if !(p.base_density.is_finite() && p.base_density > 0.0) {
            return Err(format!("tile: base density {} kg/m^3", p.base_density));
        }
        if !(p.temperature.is_finite() && p.temperature >= 0.0) {
            return Err(format!("tile: temperature {} K", p.temperature));
        }
        Ok(p)
    }
}

/// The tile id a cell reads: the frame id shifted left by 5 bits (wrapping),
/// with the depth in the low 5 bits.
pub fn tile_id(cell: &CellKey) -> u64 {
    cell.frame_id.wrapping_shl(5) | u64::from(cell.depth)
}

/// Samples per axis for a depth: 4, 6, or 8.
pub fn resolution(depth: u8) -> u8 {
    4 + (depth % 3) * 2
}

/// Global sample coordinate along one axis, reduced modulo 4096 so the
/// trigonometric argument stays small.
fn global(cell_index: u32, n: u8, i: u32) -> f64 {
    ((u64::from(cell_index) * u64::from(n) + u64::from(i)) % 4096) as f64
}

/// Samples along z per gas layer period.
pub const LAYER_PERIOD: u64 = 4;

/// Height of the gas in one period of a column, in samples: between 1.25
/// and 2.75 of the [`LAYER_PERIOD`], set by a wave over the column's global
/// coordinates `gx`, `gy` and the request `phase`.
pub fn fill(gx: f64, gy: f64, phase: f64) -> f64 {
    let wave = detmath::sin(0.23 * gx + 0.31 * gy + phase) * detmath::cos(0.17 * gy);
    2.0 + 0.75 * wave
}

/// Share of a sample's sub-cube the gas fills: the part of the sample's
/// height, `layer` to `layer + 1` within its period, that lies below
/// `fill`. Exactly 0 above the gas and exactly 1 inside it.
pub fn occupancy(fill: f64, layer: u64) -> f64 {
    (fill - layer as f64).clamp(0.0, 1.0)
}

/// The section for one cell.
pub fn cell_section(req: &Request, cell: &CellKey, p: &TileParams) -> Result<Vec<u8>, String> {
    let g = cell.geometry(Meters::new(p.root_extent));
    let n = resolution(cell.depth);
    let phase = (req.span_index % 1024) as f64 * 0.1
        + (req.origin_utc_ms.rem_euclid(86_400_000) as f64) * 1.0e-6;
    let samples = Samples::from_fn(n, |x, y, z| {
        let gx = global(cell.x, n, x);
        let gy = global(cell.y, n, y);
        let gz = global(cell.z, n, z);
        let wave = detmath::sin(0.37 * gx + 0.53 * gy + 0.71 * gz + phase)
            * detmath::cos(0.29 * gx - 0.41 * gz);
        let layer = (u64::from(cell.z) * u64::from(n) + u64::from(z)) % LAYER_PERIOD;
        let occupancy = occupancy(fill(gx, gy, phase), layer);
        if occupancy == 0.0 {
            return Sample::VACUUM;
        }
        // Mean density over the whole sub-cube: the gas's own density times
        // the share it fills.
        let density = p.base_density * (1.25 + 0.75 * wave) * occupancy;
        Sample {
            density: Density::new(density),
            state: State::Gas,
            temperature: Kelvin::new(p.temperature),
            albedo: [Ratio::new(0.5), Ratio::new(0.25), Ratio::new(0.125)],
            roughness: Ratio::new(0.75),
            attenuation: Attenuation::new(0.001),
            radiating_temperature: Kelvin::new(p.temperature),
            occupancy: Ratio::new(occupancy),
        }
    });
    let section = Section::new(*cell, g.origin, g.edge, n, samples).map_err(|e| e.to_string())?;
    let compression = if cell.depth.is_multiple_of(2) {
        Compression::Zstd
    } else {
        Compression::None
    };
    Ok(matter::encode(&section, compression))
}

impl Module for TestModule {
    fn sources(req: &Request) -> Vec<u64> {
        match &req.key {
            ChunkKey::Registry => Vec::new(),
            ChunkKey::Cell(cell) => vec![tile_id(cell)],
        }
    }

    fn compile(req: &Request, tiles: &Tiles) -> Result<Vec<u8>, String> {
        match &req.key {
            ChunkKey::Registry => Ok(registry::encode(&Registry::empty())),
            ChunkKey::Cell(cell) => {
                let id = tile_id(cell);
                let bytes = tiles.get(id).ok_or_else(|| format!("tile {id} missing"))?;
                cell_section(req, cell, &TileParams::decode(bytes)?)
            }
        }
    }
}

gx_module!(TestModule);

#[cfg(test)]
mod tests {
    use super::*;
    use gx_core::module_io::{encode_request, encode_tiles, Tile};
    use gx_module_guest::run_compile;

    fn params() -> TileParams {
        TileParams {
            root_extent: 1.0e6,
            base_density: 2.0,
            temperature: 300.0,
        }
    }

    #[test]
    fn occupancy_is_the_filled_share() {
        assert_eq!(occupancy(2.5, 0), 1.0);
        assert_eq!(occupancy(2.5, 1), 1.0);
        assert_eq!(occupancy(2.5, 2), 0.5);
        assert_eq!(occupancy(2.5, 3), 0.0);
        // Summed over a period, the occupancies are the fill height.
        for f in [1.25, 1.5, 2.0, 2.75] {
            let total: f64 = (0..LAYER_PERIOD).map(|k| occupancy(f, k)).sum();
            assert_eq!(total, f);
        }
        for i in 0..64 {
            let f = fill(f64::from(i), f64::from(3 * i), 0.7);
            assert!((1.25..=2.75).contains(&f), "{f}");
        }
    }

    #[test]
    fn tile_round_trip_and_checks() {
        let p = params();
        assert_eq!(TileParams::decode(&p.encode()), Ok(p));
        assert!(TileParams::decode(&[0; 23]).is_err());
        let bad = TileParams {
            base_density: 0.0,
            ..p
        };
        assert!(TileParams::decode(&bad.encode()).is_err());
    }

    #[test]
    fn compiles_valid_sections_for_cells_and_registry() {
        let cell = CellKey::new(9, 3, 1, 2, 7).unwrap();
        let req = Request {
            origin_utc_ms: 1_763_130_000_000,
            span_seconds: 3600,
            span_index: 5,
            key: ChunkKey::Cell(cell),
        };
        let tile = params().encode();
        let tiles = encode_tiles(&[Tile {
            id: tile_id(&cell),
            bytes: &tile,
        }])
        .unwrap();
        let out = run_compile::<TestModule>(&encode_request(&req), &tiles).unwrap();
        let section = matter::decode(&cell, &out).unwrap();
        assert_eq!(section.resolution(), resolution(3));
        assert!(section.mass().value() > 0.0);
        assert_eq!(&out[4..6], &3u16.to_le_bytes());
        // Full, partly filled, and vacuum samples all occur.
        let samples = section.samples().unwrap();
        let occ: Vec<f64> = (0..samples.len())
            .map(|i| samples.occupancy(i).value())
            .collect();
        assert!(occ.contains(&1.0));
        assert!(occ.contains(&0.0));
        assert!(occ.iter().any(|&o| o > 0.0 && o < 1.0));

        let other = Request {
            span_index: 6,
            ..req
        };
        let out2 = run_compile::<TestModule>(&encode_request(&other), &tiles).unwrap();
        assert_ne!(out, out2, "the span index changes the section");

        let reg = Request {
            key: ChunkKey::Registry,
            ..req
        };
        let out =
            run_compile::<TestModule>(&encode_request(&reg), &encode_tiles(&[]).unwrap()).unwrap();
        assert_eq!(out, registry::encode(&Registry::empty()));
    }
}
