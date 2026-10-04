//! Regenerates the matter section and frame registry conformance vectors.
//!
//! Usage: `conformance-gen OUT_DIR`. Writes `OUT_DIR/matter/valid/`,
//! `OUT_DIR/matter/invalid/`, and `OUT_DIR/registry/` (see the [`registry`]
//! module), the files checked in under `conformance/matter/` and
//! `conformance/registry/` (`matter-format.md` section 8). Run it with
//! `conformance` as `OUT_DIR` after removing `conformance/matter` and
//! `conformance/registry` to refresh the checked-in vectors, or into a
//! scratch directory and `diff -r` to prove the checked-in files are what the
//! code produces.
//!
//! Output is a pure function of this source and the `gx-core` version: fixed
//! values, a fixed-seed generator written here, no clock, no environment, no
//! hash map iteration.
//!
//! - `valid/NAME.bin` is a valid section and `valid/NAME.json` lists what any
//!   decoder must recover from it: key, origin, edge, resolution, flags,
//!   sample count, total mass, per-channel sums, and per-state counts. Every
//!   `f64` appears as a decimal string and as its IEEE 754 bit pattern in
//!   hex, and the bit pattern is the one to compare.
//! - `invalid/CODE-RULE.bin` breaks exactly one rule. `invalid/index.json`
//!   gives the chunk key every invalid file is validated under and maps each
//!   file name to the expected code from `docs/errors.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gx_core::error::codes;
use gx_core::key::CellKey;
use gx_core::matter::{self, Compression, Sample, Samples, Section, State, HEADER_LEN};
use gx_core::units::{Attenuation, Density, Kelvin, Meters, Ratio};
use serde_json::{json, Value};

mod registry;

/// SplitMix64: a small, fixed, portable pseudo-random sequence.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform on `[0, 1)` with 53 bits, exact in `f64`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform on `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// One output file: path relative to `OUT_DIR` and its bytes.
pub(crate) type File = (String, Vec<u8>);

fn cell(frame_id: u64, depth: u8, x: u32, y: u32, z: u32) -> CellKey {
    CellKey::new(frame_id, depth, x, y, z).expect("fixed keys are valid")
}

fn ratio3(a: f64, b: f64, c: f64) -> [Ratio; 3] {
    [Ratio::new(a), Ratio::new(b), Ratio::new(c)]
}

/// Builds a non-empty sample with the given values.
fn sample(
    density: f64,
    state: State,
    temperature: f64,
    albedo: [Ratio; 3],
    roughness: f64,
    attenuation: f64,
) -> Sample {
    Sample {
        density: Density::new(density),
        state,
        temperature: Kelvin::new(temperature),
        albedo,
        roughness: Ratio::new(roughness),
        attenuation: Attenuation::new(attenuation),
    }
}

/// Builds a section whose geometry is derived from the key and a root extent
/// in meters, as section 3.1 prescribes.
fn build(key: CellKey, root_extent: f64, resolution: u8, samples: Samples) -> Section {
    let g = key.geometry(Meters::new(root_extent));
    Section::new(key, g.origin, g.edge, resolution, samples).expect("fixed sections are valid")
}

/// Formats an `f64` as `{"decimal": shortest round-trip text, "bits": hex}`.
pub(crate) fn number(v: f64) -> Value {
    json!({
        "decimal": format!("{v:e}"),
        "bits": format!("0x{:016x}", v.to_bits()),
    })
}

/// The expected decode summary of a valid section, as JSON.
fn summary(section: &Section, bytes: &[u8]) -> Value {
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    let mut density = 0.0f64;
    let mut temperature = 0.0f64;
    let mut albedo = [0.0f64; 3];
    let mut roughness = 0.0f64;
    let mut attenuation = 0.0f64;
    let mut states = [0u64; 5];
    let mut count = 0u64;
    if let Some(samples) = section.samples() {
        for s in samples.iter() {
            count += 1;
            density += s.density.value();
            temperature += s.temperature.value();
            for (acc, a) in albedo.iter_mut().zip(s.albedo) {
                *acc += a.value();
            }
            roughness += s.roughness.value();
            attenuation += s.attenuation.value();
            states[usize::from(s.state.as_u8())] += 1;
        }
    }
    let mut state_counts = serde_json::Map::new();
    for st in State::ALL {
        state_counts.insert(st.name().into(), json!(states[usize::from(st.as_u8())]));
    }
    let o = section.origin();
    json!({
        "key": section.key().to_string(),
        "origin": [number(o.x), number(o.y), number(o.z)],
        "edge": number(section.edge().value()),
        "resolution": section.resolution(),
        "flags": flags,
        "sample_count": count,
        "mass": number(section.mass().value()),
        "sums": {
            "density": number(density),
            "temperature": number(temperature),
            "albedo": [number(albedo[0]), number(albedo[1]), number(albedo[2])],
            "roughness": number(roughness),
            "attenuation": number(attenuation),
        },
        "state_counts": state_counts,
    })
}

pub(crate) fn pretty(v: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(v).expect("JSON values serialize");
    s.push('\n');
    s.into_bytes()
}

/// One hot plasma sample filling the whole cell.
fn res1_hot_plasma() -> Section {
    let s = sample(
        1.5e-4,
        State::Plasma,
        15000.0,
        ratio3(0.0, 0.0, 0.0),
        0.0,
        0.04,
    );
    build(cell(1, 0, 0, 0, 0), 2.0e9, 1, Samples::filled(1, s))
}

/// Eight samples covering every state.
fn res2_every_state() -> Section {
    let list = [
        Sample::VACUUM,
        sample(
            2700.0,
            State::Solid,
            290.0,
            ratio3(0.3, 0.3, 0.25),
            0.8,
            0.0,
        ),
        sample(
            1000.0,
            State::Fluid,
            285.0,
            ratio3(0.02, 0.05, 0.1),
            0.05,
            0.002,
        ),
        sample(1.2, State::Gas, 250.0, ratio3(0.9, 0.9, 0.9), 1.0, 0.03),
        sample(
            1.0e-3,
            State::Plasma,
            8000.0,
            ratio3(0.0, 0.0, 0.0),
            0.0,
            0.5,
        ),
        sample(
            7800.0,
            State::Solid,
            1200.0,
            ratio3(0.5, 0.45, 0.4),
            0.3,
            0.0,
        ),
        sample(
            13500.0,
            State::Fluid,
            300.0,
            ratio3(0.7, 0.7, 0.7),
            0.0,
            0.0,
        ),
        sample(0.18, State::Gas, 20.0, ratio3(1.0, 1.0, 1.0), 0.5, 0.25),
    ];
    build(cell(2, 1, 1, 0, 1), 1.0e7, 2, Samples::from_samples(list))
}

/// A seeded mix of every state at an odd resolution.
fn res3_mixed() -> Section {
    let mut rng = SplitMix64(3);
    let samples = Samples::from_fn(3, |_, _, _| {
        let state = State::ALL[(rng.next_u64() % 5) as usize];
        let (lo, hi) = match state {
            State::Vacuum => return Sample::VACUUM,
            State::Solid => (1500.0, 9000.0),
            State::Fluid => (600.0, 1500.0),
            State::Gas => (0.01, 5.0),
            State::Plasma => (1.0e-6, 1.0e-2),
        };
        let a = rng.unit();
        sample(
            rng.range(lo, hi),
            state,
            rng.range(3.0, 20000.0),
            ratio3(a, rng.unit(), rng.unit()),
            rng.unit(),
            rng.range(0.0, 2.0),
        )
    });
    build(cell(3, 2, 3, 1, 0), 4096.0, 3, samples)
}

/// A seeded noisy ball at resolution 16, shared by the raw and zstd twins.
fn res16_ball() -> Section {
    let mut rng = SplitMix64(16);
    let n = 16.0;
    let samples = Samples::from_fn(16, |x, y, z| {
        let c = |i: u32| f64::from(i) + 0.5 - n / 2.0;
        let r = (c(x) * c(x) + c(y) * c(y) + c(z) * c(z)).sqrt() / n;
        let noise = rng.unit();
        if r < 0.3 {
            sample(
                2500.0 + 3000.0 * noise,
                State::Solid,
                250.0 + 100.0 * rng.unit(),
                ratio3(rng.unit(), rng.unit(), rng.unit()),
                rng.unit(),
                0.0,
            )
        } else if r < 0.4 {
            sample(
                900.0 + 200.0 * noise,
                State::Fluid,
                270.0 + 30.0 * rng.unit(),
                ratio3(0.05, 0.1, 0.2),
                0.1,
                0.001 * rng.unit(),
            )
        } else if r < 0.5 {
            sample(
                0.1 + noise,
                State::Gas,
                200.0 + 50.0 * rng.unit(),
                ratio3(0.8, 0.85, 0.9),
                1.0,
                0.01 + 0.02 * rng.unit(),
            )
        } else {
            Sample::VACUUM
        }
    });
    build(cell(4, 5, 17, 3, 30), 6.4e6, 16, samples)
}

/// Concentric shells at resolution 64, piecewise constant so it compresses.
fn res64_shells() -> Section {
    let n = 64.0;
    let samples = Samples::from_fn(64, |x, y, z| {
        let c = |i: u32| f64::from(i) + 0.5 - n / 2.0;
        let r = (c(x) * c(x) + c(y) * c(y) + c(z) * c(z)).sqrt();
        if r < 10.0 {
            sample(
                5000.0,
                State::Solid,
                1500.0,
                ratio3(0.2, 0.2, 0.2),
                0.9,
                0.0,
            )
        } else if r < 20.0 {
            sample(
                3300.0,
                State::Solid,
                800.0,
                ratio3(0.35, 0.3, 0.25),
                0.7,
                0.0,
            )
        } else if r < 24.0 {
            sample(
                1000.0,
                State::Fluid,
                280.0,
                ratio3(0.03, 0.06, 0.12),
                0.05,
                0.001,
            )
        } else if r < 28.0 {
            sample(1.2, State::Gas, 250.0, ratio3(0.9, 0.92, 0.95), 1.0, 0.03)
        } else {
            Sample::VACUUM
        }
    });
    build(cell(5, 4, 8, 8, 8), 1.0e5, 64, samples)
}

fn empty() -> Section {
    let key = cell(6, 10, 1000, 0, 1023);
    let g = key.geometry(Meters::new(1.0e4));
    Section::empty(key, g.origin, g.edge).expect("fixed section is valid")
}

fn valid() -> Vec<File> {
    let ball = res16_ball();
    let sections = [
        ("res1-hot-plasma", res1_hot_plasma(), Compression::None),
        ("res2-every-state", res2_every_state(), Compression::None),
        ("res3-mixed", res3_mixed(), Compression::None),
        ("res16-raw", ball.clone(), Compression::None),
        ("res16-zstd", ball, Compression::Zstd),
        ("res64-shells-zstd", res64_shells(), Compression::Zstd),
        ("empty", empty(), Compression::None),
    ];
    let mut out = Vec::new();
    for (name, section, compression) in sections {
        let bytes = matter::encode(&section, compression);
        let summary = summary(&section, &bytes);
        out.push((format!("matter/valid/{name}.bin"), bytes));
        out.push((format!("matter/valid/{name}.json"), pretty(&summary)));
    }
    out
}

/// The key every invalid vector is validated under.
fn invalid_key() -> CellKey {
    cell(7, 3, 5, 2, 1)
}

/// The valid base for invalid vectors: resolution 2, sample 0 vacuum, the
/// rest matter. Edge 2 m, origin (2, -4, -6) m.
fn invalid_base() -> Section {
    let list = [
        Sample::VACUUM,
        sample(2700.0, State::Solid, 290.0, ratio3(0.3, 0.3, 0.3), 0.5, 0.0),
        sample(
            1000.0,
            State::Fluid,
            280.0,
            ratio3(0.1, 0.1, 0.1),
            0.1,
            0.01,
        ),
        sample(1.0, State::Gas, 250.0, ratio3(0.9, 0.9, 0.9), 1.0, 0.05),
        sample(
            0.001,
            State::Plasma,
            9000.0,
            ratio3(0.0, 0.0, 0.0),
            0.0,
            0.5,
        ),
        sample(3000.0, State::Solid, 300.0, ratio3(0.4, 0.4, 0.4), 0.6, 0.0),
        sample(
            1100.0,
            State::Fluid,
            275.0,
            ratio3(0.2, 0.2, 0.2),
            0.2,
            0.02,
        ),
        sample(0.5, State::Gas, 240.0, ratio3(0.8, 0.8, 0.8), 0.9, 0.04),
    ];
    build(invalid_key(), 16.0, 2, Samples::from_samples(list))
}

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn put_f64(b: &mut [u8], at: usize, v: f64) {
    put_u64(b, at, v.to_bits());
}

fn put_f32(b: &mut [u8], at: usize, v: f32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Byte offsets of each channel in an uncompressed resolution 2 section.
mod at {
    use super::HEADER_LEN;

    const N: usize = 8;

    pub fn density(i: usize) -> usize {
        HEADER_LEN + 4 * i
    }
    pub fn state(i: usize) -> usize {
        HEADER_LEN + 4 * N + i
    }
    pub fn temperature(i: usize) -> usize {
        HEADER_LEN + 5 * N + 4 * i
    }
    pub fn albedo(i: usize, band: usize) -> usize {
        HEADER_LEN + 9 * N + 12 * i + 4 * band
    }
    pub fn roughness(i: usize) -> usize {
        HEADER_LEN + 21 * N + 4 * i
    }
    pub fn attenuation(i: usize) -> usize {
        HEADER_LEN + 25 * N + 4 * i
    }
}

fn invalid() -> Vec<File> {
    let base = invalid_base();
    let raw = matter::encode(&base, Compression::None);
    let packed = matter::encode(&base, Compression::Zstd);
    let g = invalid_key().geometry(Meters::new(16.0));
    let empty = matter::encode(
        &Section::empty(invalid_key(), g.origin, g.edge).expect("valid"),
        Compression::None,
    );
    let block = &raw[HEADER_LEN..];

    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, from: &[u8], f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = from.to_vec();
        f(&mut b);
        vectors.push((name, code, b));
    };

    // 100: header.
    add("header-too-short", codes::HEADER_TOO_SHORT, &empty, &|b| {
        b.truncate(HEADER_LEN - 1)
    });
    add(
        "header-zero-length",
        codes::HEADER_TOO_SHORT,
        &empty,
        &|b| b.clear(),
    );
    add("bad-magic", codes::BAD_MAGIC, &raw, &|b| b[3] = 0x47);
    add(
        "unsupported-version",
        codes::UNSUPPORTED_VERSION,
        &raw,
        &|b| put_u16(b, 4, 2),
    );
    add("unknown-flags", codes::UNKNOWN_FLAGS, &raw, &|b| {
        put_u16(b, 6, 0x0004)
    });
    add("reserved-nonzero", codes::RESERVED_NONZERO, &raw, &|b| {
        put_u16(b, 18, 1)
    });

    // 200: key.
    add("frame-id-mismatch", codes::FRAME_ID_MISMATCH, &raw, &|b| {
        put_u64(b, 8, 8)
    });
    add("depth-mismatch", codes::DEPTH_MISMATCH, &raw, &|b| {
        b[16] = 4
    });
    add("cell-x-mismatch", codes::CELL_X_MISMATCH, &raw, &|b| {
        put_u32(b, 20, 6)
    });
    add("cell-y-mismatch", codes::CELL_Y_MISMATCH, &raw, &|b| {
        put_u32(b, 24, 3)
    });
    add("cell-z-mismatch", codes::CELL_Z_MISMATCH, &raw, &|b| {
        put_u32(b, 28, 0)
    });

    // 300: geometry.
    add("edge-infinite", codes::EDGE_NOT_FINITE, &raw, &|b| {
        put_f64(b, 56, f64::INFINITY)
    });
    add("edge-nan", codes::EDGE_NOT_FINITE, &raw, &|b| {
        put_f64(b, 56, f64::NAN)
    });
    add("edge-zero", codes::EDGE_NOT_POSITIVE, &raw, &|b| {
        put_f64(b, 56, 0.0)
    });
    add("edge-negative", codes::EDGE_NOT_POSITIVE, &raw, &|b| {
        put_f64(b, 56, -2.0)
    });
    add("origin-not-finite", codes::ORIGIN_NOT_FINITE, &raw, &|b| {
        put_f64(b, 40, f64::NAN)
    });

    // 400: sample block.
    add(
        "empty-resolution-nonzero",
        codes::EMPTY_RESOLUTION_NONZERO,
        &empty,
        &|b| b[17] = 2,
    );
    add(
        "empty-block-len-nonzero",
        codes::EMPTY_BLOCK_LEN_NONZERO,
        &empty,
        &|b| put_u64(b, 64, 232),
    );
    add("empty-zstd-set", codes::EMPTY_ZSTD_SET, &empty, &|b| {
        put_u16(b, 6, 0x0003)
    });
    add(
        "empty-trailing-bytes",
        codes::EMPTY_TRAILING_BYTES,
        &empty,
        &|b| b.push(0),
    );
    add(
        "resolution-zero",
        codes::RESOLUTION_OUT_OF_RANGE,
        &raw,
        &|b| b[17] = 0,
    );
    add(
        "resolution-above-64",
        codes::RESOLUTION_OUT_OF_RANGE,
        &raw,
        &|b| b[17] = 65,
    );
    add(
        "block-len-mismatch",
        codes::BLOCK_LEN_MISMATCH,
        &raw,
        &|b| put_u64(b, 64, 231),
    );
    add("raw-block-short", codes::RAW_LENGTH_MISMATCH, &raw, &|b| {
        b.pop();
    });
    add("raw-block-long", codes::RAW_LENGTH_MISMATCH, &raw, &|b| {
        b.push(0)
    });
    add("zstd-garbage", codes::ZSTD_FRAME_INVALID, &packed, &|b| {
        b.truncate(HEADER_LEN);
        b.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03]);
    });
    add(
        "zstd-flag-on-raw-block",
        codes::ZSTD_FRAME_INVALID,
        &raw,
        &|b| put_u16(b, 6, 0x0002),
    );
    add("zstd-truncated", codes::ZSTD_FRAME_INVALID, &packed, &|b| {
        b.pop();
    });
    add(
        "zstd-trailing-bytes",
        codes::ZSTD_TRAILING_BYTES,
        &packed,
        &|b| b.push(0),
    );
    let short_frame = zstd_frame(&block[..block.len() - 1]);
    add(
        "zstd-decompresses-short",
        codes::ZSTD_LENGTH_MISMATCH,
        &packed,
        &|b| {
            b.truncate(HEADER_LEN);
            b.extend_from_slice(&short_frame);
        },
    );
    let mut long_block = block.to_vec();
    long_block.push(0);
    let long_frame = zstd_frame(&long_block);
    add(
        "zstd-decompresses-long",
        codes::ZSTD_LENGTH_MISMATCH,
        &packed,
        &|b| {
            b.truncate(HEADER_LEN);
            b.extend_from_slice(&long_frame);
        },
    );

    // 500: channel values. Sample 0 is vacuum; samples 1 to 7 are matter.
    add("density-nan", codes::DENSITY_NOT_FINITE, &raw, &|b| {
        put_f32(b, at::density(3), f32::NAN)
    });
    add("density-negative", codes::DENSITY_NEGATIVE, &raw, &|b| {
        put_f32(b, at::density(3), -1.0)
    });
    add(
        "state-out-of-range",
        codes::STATE_OUT_OF_RANGE,
        &raw,
        &|b| b[at::state(5)] = 5,
    );
    add(
        "temperature-infinite",
        codes::TEMPERATURE_NOT_FINITE,
        &raw,
        &|b| put_f32(b, at::temperature(2), f32::INFINITY),
    );
    add(
        "temperature-negative",
        codes::TEMPERATURE_NEGATIVE,
        &raw,
        &|b| put_f32(b, at::temperature(2), -1.0),
    );
    add("albedo-nan", codes::ALBEDO_NOT_FINITE, &raw, &|b| {
        put_f32(b, at::albedo(4, 1), f32::NAN)
    });
    add("albedo-above-one", codes::ALBEDO_OUT_OF_RANGE, &raw, &|b| {
        put_f32(b, at::albedo(4, 2), 1.5)
    });
    add("albedo-negative", codes::ALBEDO_OUT_OF_RANGE, &raw, &|b| {
        put_f32(b, at::albedo(1, 0), -0.25)
    });
    add("roughness-nan", codes::ROUGHNESS_NOT_FINITE, &raw, &|b| {
        put_f32(b, at::roughness(6), f32::NAN)
    });
    add(
        "roughness-above-one",
        codes::ROUGHNESS_OUT_OF_RANGE,
        &raw,
        &|b| put_f32(b, at::roughness(6), 2.0),
    );
    add(
        "attenuation-infinite",
        codes::ATTENUATION_NOT_FINITE,
        &raw,
        &|b| put_f32(b, at::attenuation(7), f32::INFINITY),
    );
    add(
        "attenuation-negative",
        codes::ATTENUATION_NEGATIVE,
        &raw,
        &|b| put_f32(b, at::attenuation(7), -0.5),
    );
    add(
        "vacuum-state-not-vacuum",
        codes::VACUUM_STATE_NOT_VACUUM,
        &raw,
        &|b| b[at::state(0)] = 1,
    );
    add(
        "matter-state-vacuum",
        codes::MATTER_STATE_VACUUM,
        &raw,
        &|b| b[at::state(1)] = 0,
    );
    add(
        "zero-density-matter-state",
        codes::VACUUM_STATE_NOT_VACUUM,
        &raw,
        &|b| put_f32(b, at::density(1), 0.0),
    );
    add(
        "vacuum-temperature-nonzero",
        codes::VACUUM_TEMPERATURE_NONZERO,
        &raw,
        &|b| put_f32(b, at::temperature(0), 3.0),
    );
    add(
        "vacuum-albedo-nonzero",
        codes::VACUUM_ALBEDO_NONZERO,
        &raw,
        &|b| put_f32(b, at::albedo(0, 1), 0.5),
    );
    add(
        "vacuum-roughness-nonzero",
        codes::VACUUM_ROUGHNESS_NONZERO,
        &raw,
        &|b| put_f32(b, at::roughness(0), 0.5),
    );
    add(
        "vacuum-attenuation-nonzero",
        codes::VACUUM_ATTENUATION_NONZERO,
        &raw,
        &|b| put_f32(b, at::attenuation(0), 1.0),
    );

    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("matter/invalid/{file}"), bytes));
    }
    let index = json!({
        "key": invalid_key().to_string(),
        "vectors": index,
    });
    out.push(("matter/invalid/index.json".into(), pretty(&index)));
    out
}

/// Compresses `block` exactly as the encoder does: one zstd frame at
/// [`matter::ZSTD_LEVEL`]. Used for vectors whose frame is well formed but
/// carries the wrong length.
fn zstd_frame(block: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(block, matter::ZSTD_LEVEL).expect("compressing a fixed block")
}

/// Every file the generator writes, sorted by path.
fn generate() -> Vec<File> {
    let mut files = valid();
    files.extend(invalid());
    files.extend(registry::files());
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn write_all(root: &Path, files: &[File]) -> std::io::Result<()> {
    for (rel, bytes) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
    }
    Ok(())
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(out), None) = (args.next(), args.next()) else {
        eprintln!("usage: conformance-gen OUT_DIR");
        std::process::exit(2);
    };
    let root = PathBuf::from(out);
    let files = generate();
    if let Err(e) = write_all(&root, &files) {
        eprintln!("conformance-gen: {e}");
        std::process::exit(1);
    }
    println!(
        "conformance-gen: wrote {} files under {}",
        files.len(),
        root.display()
    );
}
