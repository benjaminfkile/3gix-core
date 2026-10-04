//! Matter sections: encode, decode, validate, and composite.
//!
//! Implements `matter-format.md` section 3 (matter section: header 3.2,
//! sample block 3.3, empty section 3.4, compositing 3.5) and section 4
//! (validation) for matter sections, steps 1, 2, and 4 through 8. Step 3, the
//! registry branch, belongs to the top-level validator that dispatches on the
//! chunk key.
//!
//! A [`Section`] is one compiler's contribution to one cell: a header that
//! repeats the cell key and geometry, and either nothing (an empty section)
//! or `n^3` samples. Each sample carries density, state, temperature, albedo
//! in three bands, roughness, and attenuation.
//!
//! The wire format stores `f32` channels and so do [`Samples`], but every
//! public constructor and accessor takes and returns the unit types from
//! [`crate::units`]. Values are rounded to the nearest `f32` when stored. A
//! [`Section`] cannot be built from bare floats, and it cannot be built with
//! values that break a rule: [`Section::new`] runs the same rules as
//! [`decode`].
//!
//! Every byte this module writes is a pure function of its inputs. The zstd
//! level and crate version are pinned (see `docs/determinism.md`).
//!
//! Validation order and the code for every rule are listed in
//! `docs/errors.md` and [`crate::error::codes`].

use std::io::Read;

use crate::error::{codes, ValidationError};
use crate::key::CellKey;
use crate::units::{Attenuation, CubicMeters, Density, Kelvin, Kilograms, Meters, Ratio, Vec3};

/// The four magic bytes that open a matter section: `3GMS` in ASCII.
pub const MAGIC: [u8; 4] = [0x33, 0x47, 0x4D, 0x53];

/// The matter section format version this module reads and writes.
pub const VERSION: u16 = 1;

/// Length of the fixed header in bytes.
pub const HEADER_LEN: usize = 72;

/// Largest permitted resolution of a non-empty section.
pub const MAX_RESOLUTION: u8 = 64;

/// Bytes per sample in the uncompressed sample block: density 4, state 1,
/// temperature 4, albedo 12, roughness 4, attenuation 4.
pub const BYTES_PER_SAMPLE: u64 = 29;

/// Header flag bit 0: the section is empty.
pub const FLAG_EMPTY: u16 = 1 << 0;

/// Header flag bit 1: the sample block is one zstd frame.
pub const FLAG_ZSTD: u16 = 1 << 1;

/// zstd compression level used by [`encode`]. Changing it changes the bytes
/// of every compressed section, so it is a format-affecting change.
pub const ZSTD_LEVEL: i32 = 9;

/// Physical state of the majority of the mass in a sample (section 3.3).
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum State {
    /// No matter. Exactly the samples with density 0.
    Vacuum = 0,
    /// Solid matter.
    Solid = 1,
    /// Fluid matter.
    Fluid = 2,
    /// Gas.
    Gas = 3,
    /// Plasma.
    Plasma = 4,
}

impl State {
    /// Every state, in wire value order.
    pub const ALL: [State; 5] = [
        State::Vacuum,
        State::Solid,
        State::Fluid,
        State::Gas,
        State::Plasma,
    ];

    /// Returns the state with wire value `v`, or `None` if `v` is above 4.
    pub fn from_u8(v: u8) -> Option<State> {
        Self::ALL.get(usize::from(v)).copied()
    }

    /// Returns the wire value.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns the lowercase name used in conformance files: `vacuum`,
    /// `solid`, `fluid`, `gas`, or `plasma`.
    pub fn name(self) -> &'static str {
        match self {
            State::Vacuum => "vacuum",
            State::Solid => "solid",
            State::Fluid => "fluid",
            State::Gas => "gas",
            State::Plasma => "plasma",
        }
    }
}

/// One sample's values in unit types, as passed to and returned from
/// [`Samples`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Sample {
    /// Mean density over the sub-cube.
    pub density: Density,
    /// State of the majority of the mass.
    pub state: State,
    /// Mass-weighted mean temperature.
    pub temperature: Kelvin,
    /// Reflectance in three bands, long to short wavelength.
    pub albedo: [Ratio; 3],
    /// Microfacet roughness.
    pub roughness: Ratio,
    /// Mass attenuation coefficient.
    pub attenuation: Attenuation,
}

impl Sample {
    /// A vacuum sample: density 0, state vacuum, every other channel 0.
    pub const VACUUM: Sample = Sample {
        density: Density::new(0.0),
        state: State::Vacuum,
        temperature: Kelvin::new(0.0),
        albedo: [Ratio::new(0.0); 3],
        roughness: Ratio::new(0.0),
        attenuation: Attenuation::new(0.0),
    };
}

/// The samples of a non-empty section: planar channels in x-fastest order,
/// `index = x + n * (y + n * z)`.
///
/// Values are stored as `f32`, as on the wire. Setting a value rounds it to
/// the nearest `f32`; reading returns that `f32` widened exactly to `f64`.
/// A [`Samples`] may hold any values; [`Section::new`] checks them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Samples {
    density: Vec<f32>,
    state: Vec<State>,
    temperature: Vec<f32>,
    albedo: Vec<[f32; 3]>,
    roughness: Vec<f32>,
    attenuation: Vec<f32>,
}

/// Converts a unit value to its stored `f32`, rounding to nearest.
fn narrow(v: f64) -> f32 {
    v as f32
}

impl Samples {
    /// Builds an empty sample list with room for `count` samples.
    fn with_capacity(count: usize) -> Self {
        Self {
            density: Vec::with_capacity(count),
            state: Vec::with_capacity(count),
            temperature: Vec::with_capacity(count),
            albedo: Vec::with_capacity(count),
            roughness: Vec::with_capacity(count),
            attenuation: Vec::with_capacity(count),
        }
    }

    /// Appends one sample, rounding each value to `f32`.
    fn push(&mut self, s: Sample) {
        self.density.push(narrow(s.density.value()));
        self.state.push(s.state);
        self.temperature.push(narrow(s.temperature.value()));
        self.albedo.push(s.albedo.map(|a| narrow(a.value())));
        self.roughness.push(narrow(s.roughness.value()));
        self.attenuation.push(narrow(s.attenuation.value()));
    }

    /// Builds `resolution^3` copies of `sample`.
    pub fn filled(resolution: u8, sample: Sample) -> Self {
        Self::from_fn(resolution, |_, _, _| sample)
    }

    /// Builds `resolution^3` samples by calling `f(x, y, z)` for every grid
    /// position, in index order (x fastest, then y, then z).
    pub fn from_fn(resolution: u8, mut f: impl FnMut(u32, u32, u32) -> Sample) -> Self {
        let n = u32::from(resolution);
        let mut out = Self::with_capacity(sample_count(resolution));
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    out.push(f(x, y, z));
                }
            }
        }
        out
    }

    /// Builds samples from a list already in index order.
    pub fn from_samples(samples: impl IntoIterator<Item = Sample>) -> Self {
        let iter = samples.into_iter();
        let mut out = Self::with_capacity(iter.size_hint().0);
        for s in iter {
            out.push(s);
        }
        out
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.density.len()
    }

    /// Returns `true` if there are no samples.
    pub fn is_empty(&self) -> bool {
        self.density.is_empty()
    }

    /// Returns sample `index`, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<Sample> {
        if index >= self.len() {
            return None;
        }
        Some(Sample {
            density: Density::new(f64::from(self.density[index])),
            state: self.state[index],
            temperature: Kelvin::new(f64::from(self.temperature[index])),
            albedo: self.albedo[index].map(|a| Ratio::new(f64::from(a))),
            roughness: Ratio::new(f64::from(self.roughness[index])),
            attenuation: Attenuation::new(f64::from(self.attenuation[index])),
        })
    }

    /// Replaces sample `index`, rounding each value to `f32`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn set(&mut self, index: usize, s: Sample) {
        assert!(index < self.len(), "sample index {index} out of bounds");
        self.density[index] = narrow(s.density.value());
        self.state[index] = s.state;
        self.temperature[index] = narrow(s.temperature.value());
        self.albedo[index] = s.albedo.map(|a| narrow(a.value()));
        self.roughness[index] = narrow(s.roughness.value());
        self.attenuation[index] = narrow(s.attenuation.value());
    }

    /// Iterates over every sample in index order.
    pub fn iter(&self) -> impl Iterator<Item = Sample> + '_ {
        (0..self.len()).filter_map(move |i| self.get(i))
    }

    /// Density of sample `index`, without building a whole [`Sample`].
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn density(&self, index: usize) -> Density {
        Density::new(f64::from(self.density[index]))
    }

    /// State of sample `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn state(&self, index: usize) -> State {
        self.state[index]
    }

    /// Checks every channel and vacuum rule of section 3.3.
    fn check(&self) -> Result<(), ValidationError> {
        let states: Vec<u8> = self.state.iter().map(|s| s.as_u8()).collect();
        check_channels(&Channels {
            density: &self.density,
            state: &states,
            temperature: &self.temperature,
            albedo: &self.albedo,
            roughness: &self.roughness,
            attenuation: &self.attenuation,
        })
    }
}

/// Number of samples at `resolution`: `n^3`.
fn sample_count(resolution: u8) -> usize {
    let n = usize::from(resolution);
    n * n * n
}

/// A whole cell's contribution from one compiler (section 3).
///
/// Always valid: every constructor runs the section 3.3 and 4 rules, and
/// the fields are private so they cannot be changed afterwards.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    key: CellKey,
    origin: Vec3,
    edge: Meters,
    resolution: u8,
    samples: Option<Samples>,
}

/// Compression applied to the sample block by [`encode`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    /// The sample block follows the header as is.
    None,
    /// The sample block is one zstd frame at [`ZSTD_LEVEL`], ZSTD flag set.
    Zstd,
}

/// Checks the key is a valid cell key (code 200).
fn check_key(key: &CellKey) -> Result<(), ValidationError> {
    if key.is_valid() {
        Ok(())
    } else {
        Err(ValidationError::new(
            codes::KEY_INVALID,
            format!("key: {key} is not a valid cell key"),
        ))
    }
}

/// Checks section 4 step 5: edge finite and positive, then origin finite.
fn check_geometry(origin: Vec3, edge: Meters) -> Result<(), ValidationError> {
    let e = edge.value();
    if !e.is_finite() {
        return Err(ValidationError::new(
            codes::EDGE_NOT_FINITE,
            format!("cell_edge: {e} is not finite"),
        ));
    }
    if e <= 0.0 {
        return Err(ValidationError::new(
            codes::EDGE_NOT_POSITIVE,
            format!("cell_edge: {e} is not greater than 0"),
        ));
    }
    for (axis, v) in [("x", origin.x), ("y", origin.y), ("z", origin.z)] {
        if !v.is_finite() {
            return Err(ValidationError::new(
                codes::ORIGIN_NOT_FINITE,
                format!("cell_origin.{axis}: {v} is not finite"),
            ));
        }
    }
    Ok(())
}

/// Checks the resolution of a non-empty section is 1 to 64 (code 411).
fn check_resolution(resolution: u8) -> Result<(), ValidationError> {
    if (1..=MAX_RESOLUTION).contains(&resolution) {
        Ok(())
    } else {
        Err(ValidationError::new(
            codes::RESOLUTION_OUT_OF_RANGE,
            format!("resolution: {resolution} is not 1 to {MAX_RESOLUTION}"),
        ))
    }
}

/// Borrowed channel arrays, states as raw bytes so the decoder can check
/// them before they become [`State`] values.
struct Channels<'a> {
    density: &'a [f32],
    state: &'a [u8],
    temperature: &'a [f32],
    albedo: &'a [[f32; 3]],
    roughness: &'a [f32],
    attenuation: &'a [f32],
}

/// Builds a channel rule error naming the sample and channel.
fn sample_error(code: u16, index: usize, channel: &str, problem: String) -> ValidationError {
    ValidationError::new(code, format!("sample {index} channel {channel}: {problem}"))
}

/// Checks one `f32` channel with "finite, >= 0" or "finite, 0 to 1" rules.
fn check_scalar(
    values: &[f32],
    channel: &str,
    not_finite: u16,
    out_of_range: u16,
    unit_interval: bool,
) -> Result<(), ValidationError> {
    for (i, &v) in values.iter().enumerate() {
        check_value(v, i, channel, not_finite, out_of_range, unit_interval)?;
    }
    Ok(())
}

/// Checks one value: finiteness first, then the range.
fn check_value(
    v: f32,
    index: usize,
    channel: &str,
    not_finite: u16,
    out_of_range: u16,
    unit_interval: bool,
) -> Result<(), ValidationError> {
    if !v.is_finite() {
        return Err(sample_error(
            not_finite,
            index,
            channel,
            format!("{v} is not finite"),
        ));
    }
    if v < 0.0 {
        return Err(sample_error(
            out_of_range,
            index,
            channel,
            format!("{v} is negative"),
        ));
    }
    if unit_interval && v > 1.0 {
        return Err(sample_error(
            out_of_range,
            index,
            channel,
            format!("{v} is greater than 1"),
        ));
    }
    Ok(())
}

/// Checks section 4 step 8: each channel's rule in wire order, then the
/// vacuum rules sample by sample.
fn check_channels(c: &Channels<'_>) -> Result<(), ValidationError> {
    check_scalar(
        c.density,
        "density",
        codes::DENSITY_NOT_FINITE,
        codes::DENSITY_NEGATIVE,
        false,
    )?;
    for (i, &s) in c.state.iter().enumerate() {
        if State::from_u8(s).is_none() {
            return Err(sample_error(
                codes::STATE_OUT_OF_RANGE,
                i,
                "state",
                format!("{s} is not 0 to 4"),
            ));
        }
    }
    check_scalar(
        c.temperature,
        "temperature",
        codes::TEMPERATURE_NOT_FINITE,
        codes::TEMPERATURE_NEGATIVE,
        false,
    )?;
    for (i, bands) in c.albedo.iter().enumerate() {
        for (b, &v) in bands.iter().enumerate() {
            check_value(
                v,
                i,
                &format!("albedo[{b}]"),
                codes::ALBEDO_NOT_FINITE,
                codes::ALBEDO_OUT_OF_RANGE,
                true,
            )?;
        }
    }
    check_scalar(
        c.roughness,
        "roughness",
        codes::ROUGHNESS_NOT_FINITE,
        codes::ROUGHNESS_OUT_OF_RANGE,
        true,
    )?;
    check_scalar(
        c.attenuation,
        "attenuation",
        codes::ATTENUATION_NOT_FINITE,
        codes::ATTENUATION_NEGATIVE,
        false,
    )?;
    for i in 0..c.density.len() {
        let vacuum_state = c.state[i] == State::Vacuum.as_u8();
        if c.density[i] != 0.0 {
            if vacuum_state {
                return Err(sample_error(
                    codes::MATTER_STATE_VACUUM,
                    i,
                    "state",
                    format!("is vacuum but density is {}", c.density[i]),
                ));
            }
            continue;
        }
        if !vacuum_state {
            return Err(sample_error(
                codes::VACUUM_STATE_NOT_VACUUM,
                i,
                "state",
                format!("is {} but density is 0", c.state[i]),
            ));
        }
        let zero_rules: [(f32, String, u16); 6] = [
            (
                c.temperature[i],
                "temperature".into(),
                codes::VACUUM_TEMPERATURE_NONZERO,
            ),
            (
                c.albedo[i][0],
                "albedo[0]".into(),
                codes::VACUUM_ALBEDO_NONZERO,
            ),
            (
                c.albedo[i][1],
                "albedo[1]".into(),
                codes::VACUUM_ALBEDO_NONZERO,
            ),
            (
                c.albedo[i][2],
                "albedo[2]".into(),
                codes::VACUUM_ALBEDO_NONZERO,
            ),
            (
                c.roughness[i],
                "roughness".into(),
                codes::VACUUM_ROUGHNESS_NONZERO,
            ),
            (
                c.attenuation[i],
                "attenuation".into(),
                codes::VACUUM_ATTENUATION_NONZERO,
            ),
        ];
        for (v, channel, code) in zero_rules {
            if v != 0.0 {
                return Err(sample_error(
                    code,
                    i,
                    &channel,
                    format!("{v} is not 0 in a vacuum sample"),
                ));
            }
        }
    }
    Ok(())
}

impl Section {
    /// Builds an empty section (section 3.4) for a cell.
    ///
    /// Fails with the same codes as [`decode`] if `key` is not a valid cell
    /// key (200) or the geometry breaks a rule (301 to 303).
    pub fn empty(key: CellKey, origin: Vec3, edge: Meters) -> Result<Section, ValidationError> {
        check_key(&key)?;
        check_geometry(origin, edge)?;
        Ok(Section {
            key,
            origin,
            edge,
            resolution: 0,
            samples: None,
        })
    }

    /// Builds a non-empty section, running the same rules as [`decode`]:
    /// key (200), geometry (301 to 303), resolution 1 to 64 (411), exactly
    /// `resolution^3` samples (417), then every channel and vacuum rule
    /// (501 to 526) in decoder order.
    pub fn new(
        key: CellKey,
        origin: Vec3,
        edge: Meters,
        resolution: u8,
        samples: Samples,
    ) -> Result<Section, ValidationError> {
        check_key(&key)?;
        check_geometry(origin, edge)?;
        check_resolution(resolution)?;
        let expected = sample_count(resolution);
        if samples.len() != expected {
            return Err(ValidationError::new(
                codes::SAMPLE_COUNT_MISMATCH,
                format!(
                    "samples: {} given, resolution {resolution} needs {expected}",
                    samples.len()
                ),
            ));
        }
        samples.check()?;
        Ok(Section {
            key,
            origin,
            edge,
            resolution,
            samples: Some(samples),
        })
    }

    /// The cell key this section belongs to.
    pub fn key(&self) -> CellKey {
        self.key
    }

    /// Minimum corner of the cell, in meters, frame coordinates.
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// Edge length of the cell.
    pub fn edge(&self) -> Meters {
        self.edge
    }

    /// Samples per axis, 0 for an empty section.
    pub fn resolution(&self) -> u8 {
        self.resolution
    }

    /// The samples, or `None` for an empty section.
    pub fn samples(&self) -> Option<&Samples> {
        self.samples.as_ref()
    }

    /// Returns `true` for an empty section (section 3.4).
    pub fn is_empty(&self) -> bool {
        self.samples.is_none()
    }

    /// Volume of one sample's sub-cube, `(edge / n)^3`, or 0 for an empty
    /// section.
    pub fn sample_volume(&self) -> CubicMeters {
        if self.resolution == 0 {
            return CubicMeters::new(0.0);
        }
        (self.edge / f64::from(self.resolution)).cubed()
    }

    /// Total mass: the sum over samples, in index order and in `f64`, of
    /// density times [`Section::sample_volume`]. 0 for an empty section.
    pub fn mass(&self) -> Kilograms {
        let vol = self.sample_volume();
        let mut total = Kilograms::new(0.0);
        if let Some(s) = &self.samples {
            for &d in &s.density {
                total = total + Density::new(f64::from(d)) * vol;
            }
        }
        total
    }
}

/// Writes the 72-byte header (section 3.2).
fn write_header(out: &mut Vec<u8>, s: &Section, flags: u16, block_len: u64) {
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&s.key.frame_id.to_le_bytes());
    out.push(s.key.depth);
    out.push(s.resolution);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&s.key.x.to_le_bytes());
    out.extend_from_slice(&s.key.y.to_le_bytes());
    out.extend_from_slice(&s.key.z.to_le_bytes());
    for v in [s.origin.x, s.origin.y, s.origin.z, s.edge.value()] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&block_len.to_le_bytes());
    debug_assert_eq!(out.len(), HEADER_LEN);
}

/// Writes the uncompressed sample block (section 3.3).
fn sample_block(s: &Samples) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * BYTES_PER_SAMPLE as usize);
    let put = |out: &mut Vec<u8>, vs: &[f32]| {
        for v in vs {
            out.extend_from_slice(&v.to_le_bytes());
        }
    };
    put(&mut out, &s.density);
    out.extend(s.state.iter().map(|st| st.as_u8()));
    put(&mut out, &s.temperature);
    for bands in &s.albedo {
        put(&mut out, bands);
    }
    put(&mut out, &s.roughness);
    put(&mut out, &s.attenuation);
    out
}

/// Encodes a section to bytes (sections 3.2 to 3.4).
///
/// An empty section is the header alone with EMPTY set, whatever
/// `compression` says. Otherwise the sample block follows the header, raw
/// for [`Compression::None`] or as one zstd frame at [`ZSTD_LEVEL`] with the
/// ZSTD flag set for [`Compression::Zstd`]. Identical inputs give identical
/// bytes.
pub fn encode(section: &Section, compression: Compression) -> Vec<u8> {
    let Some(samples) = &section.samples else {
        let mut out = Vec::with_capacity(HEADER_LEN);
        write_header(&mut out, section, FLAG_EMPTY, 0);
        return out;
    };
    let block = sample_block(samples);
    let block_len = block.len() as u64;
    match compression {
        Compression::None => {
            let mut out = Vec::with_capacity(HEADER_LEN + block.len());
            write_header(&mut out, section, 0, block_len);
            out.extend_from_slice(&block);
            out
        }
        Compression::Zstd => {
            let frame = zstd::bulk::compress(&block, ZSTD_LEVEL)
                .expect("in-memory zstd compression of a bounded block does not fail");
            let mut out = Vec::with_capacity(HEADER_LEN + frame.len());
            write_header(&mut out, section, FLAG_ZSTD, block_len);
            out.extend_from_slice(&frame);
            out
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_bits(u64_at(b, at))
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|&c| f32::from_le_bytes(c))
        .collect()
}

/// Decompresses the payload after the header when ZSTD is set: exactly one
/// zstd frame that yields exactly `block_len` bytes.
fn decompress(payload: &[u8], block_len: usize) -> Result<Vec<u8>, ValidationError> {
    let frame_len = zstd::zstd_safe::find_frame_compressed_size(payload).map_err(|e| {
        ValidationError::new(
            codes::ZSTD_FRAME_INVALID,
            format!(
                "sample block: no readable zstd frame ({})",
                zstd::zstd_safe::get_error_name(e)
            ),
        )
    })?;
    if frame_len != payload.len() {
        return Err(ValidationError::new(
            codes::ZSTD_TRAILING_BYTES,
            format!(
                "sample block: {} bytes follow the {frame_len} byte zstd frame",
                payload.len() - frame_len
            ),
        ));
    }
    let invalid = |e: std::io::Error| {
        ValidationError::new(
            codes::ZSTD_FRAME_INVALID,
            format!("sample block: zstd frame does not decompress ({e})"),
        )
    };
    let decoder = zstd::stream::read::Decoder::with_buffer(payload)
        .map_err(invalid)?
        .single_frame();
    let mut out = Vec::with_capacity(block_len);
    decoder
        .take(block_len as u64 + 1)
        .read_to_end(&mut out)
        .map_err(invalid)?;
    if out.len() != block_len {
        let got = if out.len() > block_len {
            "more than".to_string()
        } else {
            out.len().to_string()
        };
        return Err(ValidationError::new(
            codes::ZSTD_LENGTH_MISMATCH,
            format!("sample block: zstd frame decompresses to {got} bytes, sample_block_len is {block_len}"),
        ));
    }
    Ok(out)
}

/// Decodes and validates a matter section submitted under `key`.
///
/// Runs `matter-format.md` section 4 steps 1, 2, and 4 through 8 in order and
/// reports the first failing rule. See `docs/errors.md` for the exact order
/// and codes.
pub fn decode(key: &CellKey, bytes: &[u8]) -> Result<Section, ValidationError> {
    // Step 1: length, magic, version.
    if bytes.len() < HEADER_LEN {
        return Err(ValidationError::new(
            codes::HEADER_TOO_SHORT,
            format!("length: {} bytes, header needs {HEADER_LEN}", bytes.len()),
        ));
    }
    if bytes[0..4] != MAGIC {
        return Err(ValidationError::new(
            codes::BAD_MAGIC,
            format!(
                "magic: {:02x} {:02x} {:02x} {:02x} is not 33 47 4d 53",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
        ));
    }
    let version = u16_at(bytes, 4);
    if version != VERSION {
        return Err(ValidationError::new(
            codes::UNSUPPORTED_VERSION,
            format!("format_version: {version} is not {VERSION}"),
        ));
    }

    // Step 2: reserved bits and fields.
    let flags = u16_at(bytes, 6);
    if flags & !(FLAG_EMPTY | FLAG_ZSTD) != 0 {
        return Err(ValidationError::new(
            codes::UNKNOWN_FLAGS,
            format!("flags: {flags:#06x} sets bits other than EMPTY and ZSTD"),
        ));
    }
    let reserved = u16_at(bytes, 18);
    if reserved != 0 {
        return Err(ValidationError::new(
            codes::RESERVED_NONZERO,
            format!("reserved: {reserved} at offset 18 is not 0"),
        ));
    }

    // Step 4: the header matches the key.
    check_key(key)?;
    let frame_id = u64_at(bytes, 8);
    if frame_id != key.frame_id {
        return Err(ValidationError::new(
            codes::FRAME_ID_MISMATCH,
            format!("frame_id: {frame_id} differs from key {}", key.frame_id),
        ));
    }
    let depth = bytes[16];
    if depth != key.depth {
        return Err(ValidationError::new(
            codes::DEPTH_MISMATCH,
            format!("depth: {depth} differs from key {}", key.depth),
        ));
    }
    for (field, at, want, code) in [
        ("cell_x", 20, key.x, codes::CELL_X_MISMATCH),
        ("cell_y", 24, key.y, codes::CELL_Y_MISMATCH),
        ("cell_z", 28, key.z, codes::CELL_Z_MISMATCH),
    ] {
        let got = u32_at(bytes, at);
        if got != want {
            return Err(ValidationError::new(
                code,
                format!("{field}: {got} differs from key {want}"),
            ));
        }
    }

    // Step 5: geometry.
    let origin = Vec3::new(f64_at(bytes, 32), f64_at(bytes, 40), f64_at(bytes, 48));
    let edge = Meters::new(f64_at(bytes, 56));
    check_geometry(origin, edge)?;

    let resolution = bytes[17];
    let block_len = u64_at(bytes, 64);
    let payload = &bytes[HEADER_LEN..];

    // Step 6: empty section.
    if flags & FLAG_EMPTY != 0 {
        if resolution != 0 {
            return Err(ValidationError::new(
                codes::EMPTY_RESOLUTION_NONZERO,
                format!("resolution: {resolution} is not 0 in an empty section"),
            ));
        }
        if block_len != 0 {
            return Err(ValidationError::new(
                codes::EMPTY_BLOCK_LEN_NONZERO,
                format!("sample_block_len: {block_len} is not 0 in an empty section"),
            ));
        }
        if flags & FLAG_ZSTD != 0 {
            return Err(ValidationError::new(
                codes::EMPTY_ZSTD_SET,
                "flags: ZSTD is set in an empty section",
            ));
        }
        if !payload.is_empty() {
            return Err(ValidationError::new(
                codes::EMPTY_TRAILING_BYTES,
                format!(
                    "length: {} bytes follow the header of an empty section",
                    payload.len()
                ),
            ));
        }
        return Ok(Section {
            key: *key,
            origin,
            edge,
            resolution: 0,
            samples: None,
        });
    }

    // Step 7: sample block framing.
    check_resolution(resolution)?;
    let count = sample_count(resolution);
    let expected = count as u64 * BYTES_PER_SAMPLE;
    if block_len != expected {
        return Err(ValidationError::new(
            codes::BLOCK_LEN_MISMATCH,
            format!("sample_block_len: {block_len} is not {expected} for resolution {resolution}"),
        ));
    }
    let expected = expected as usize;
    let owned;
    let block: &[u8] = if flags & FLAG_ZSTD != 0 {
        owned = decompress(payload, expected)?;
        &owned
    } else {
        if payload.len() != expected {
            return Err(ValidationError::new(
                codes::RAW_LENGTH_MISMATCH,
                format!(
                    "length: {} bytes follow the header, sample_block_len is {expected}",
                    payload.len()
                ),
            ));
        }
        payload
    };

    // Step 8: channel values.
    let (density, rest) = block.split_at(4 * count);
    let (state, rest) = rest.split_at(count);
    let (temperature, rest) = rest.split_at(4 * count);
    let (albedo, rest) = rest.split_at(12 * count);
    let (roughness, attenuation) = rest.split_at(4 * count);
    let density = f32s(density);
    let temperature = f32s(temperature);
    let albedo_flat = f32s(albedo);
    let albedo: Vec<[f32; 3]> = albedo_flat.as_chunks::<3>().0.to_vec();
    let roughness = f32s(roughness);
    let attenuation = f32s(attenuation);
    check_channels(&Channels {
        density: &density,
        state,
        temperature: &temperature,
        albedo: &albedo,
        roughness: &roughness,
        attenuation: &attenuation,
    })?;
    let state = state
        .iter()
        .map(|&s| State::from_u8(s).expect("checked by check_channels"))
        .collect();
    Ok(Section {
        key: *key,
        origin,
        edge,
        resolution,
        samples: Some(Samples {
            density,
            state,
            temperature,
            albedo,
            roughness,
            attenuation,
        }),
    })
}

/// Validates a matter section submitted under `key`: [`decode`] without the
/// result.
pub fn validate(key: &CellKey, bytes: &[u8]) -> Result<(), ValidationError> {
    decode(key, bytes).map(|_| ())
}

/// Maps fine grid index `i` at resolution `fine` to the nearest index at
/// resolution `coarse`: the coarse sample containing the fine sample's
/// center, `floor((i + 1/2) * coarse / fine)`, in exact integer arithmetic.
fn nearest(i: usize, coarse: usize, fine: usize) -> usize {
    ((2 * i + 1) * coarse) / (2 * fine)
}

/// Composites sections from several layers into one (section 3.5).
///
/// All inputs must share key (802), origin (803), and edge (804); an empty
/// list fails with 801. Every non-empty input is resampled to the finest
/// resolution present by nearest (block) resampling. Then, per sample:
/// densities add; the state is that of the densest contributor, the first in
/// input order on a tie; temperature, albedo, roughness, and attenuation are
/// density-weighted means. Sums run in `f64` in input order and the results
/// are rounded to `f32`. A sample with total density 0 is vacuum with every
/// channel 0. Empty inputs contribute nothing, and if every input is empty
/// the result is an empty section.
///
/// The result is checked like any other section, so a density sum that
/// overflows `f32` fails with 501.
pub fn composite(sections: &[&Section]) -> Result<Section, ValidationError> {
    let Some(first) = sections.first() else {
        return Err(ValidationError::new(
            codes::COMPOSITE_NO_INPUT,
            "composite: no sections given",
        ));
    };
    for (i, s) in sections.iter().enumerate().skip(1) {
        if s.key != first.key {
            return Err(ValidationError::new(
                codes::COMPOSITE_KEY_MISMATCH,
                format!("composite: section {i} key {} is not {}", s.key, first.key),
            ));
        }
        if s.origin != first.origin {
            return Err(ValidationError::new(
                codes::COMPOSITE_ORIGIN_MISMATCH,
                format!("composite: section {i} origin differs from section 0"),
            ));
        }
        if s.edge != first.edge {
            return Err(ValidationError::new(
                codes::COMPOSITE_EDGE_MISMATCH,
                format!("composite: section {i} edge differs from section 0"),
            ));
        }
    }
    let inputs: Vec<(usize, &Samples)> = sections
        .iter()
        .filter_map(|s| s.samples.as_ref().map(|m| (usize::from(s.resolution), m)))
        .collect();
    let Some(fine) = inputs.iter().map(|&(n, _)| n).max() else {
        return Section::empty(first.key, first.origin, first.edge);
    };
    let mut out = Samples::with_capacity(fine * fine * fine);
    for z in 0..fine {
        for y in 0..fine {
            for x in 0..fine {
                let mut total = 0.0f64;
                let mut best: Option<(f64, State)> = None;
                let mut temperature = 0.0f64;
                let mut albedo = [0.0f64; 3];
                let mut roughness = 0.0f64;
                let mut attenuation = 0.0f64;
                for &(n, m) in &inputs {
                    let (cx, cy, cz) = (
                        nearest(x, n, fine),
                        nearest(y, n, fine),
                        nearest(z, n, fine),
                    );
                    let ci = cx + n * (cy + n * cz);
                    let d = f64::from(m.density[ci]);
                    if d == 0.0 {
                        continue;
                    }
                    total += d;
                    if best.is_none_or(|(bd, _)| d > bd) {
                        best = Some((d, m.state[ci]));
                    }
                    temperature += d * f64::from(m.temperature[ci]);
                    for (acc, &a) in albedo.iter_mut().zip(&m.albedo[ci]) {
                        *acc += d * f64::from(a);
                    }
                    roughness += d * f64::from(m.roughness[ci]);
                    attenuation += d * f64::from(m.attenuation[ci]);
                }
                match best {
                    None => out.push(Sample::VACUUM),
                    Some((_, state)) => {
                        out.density.push(narrow(total));
                        out.state.push(state);
                        out.temperature.push(narrow(temperature / total));
                        out.albedo.push(albedo.map(|a| narrow(a / total)));
                        out.roughness.push(narrow(roughness / total));
                        out.attenuation.push(narrow(attenuation / total));
                    }
                }
            }
        }
    }
    let resolution = u8::try_from(fine).expect("resolution of a valid section fits u8");
    Section::new(first.key, first.origin, first.edge, resolution, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> CellKey {
        CellKey::new(7, 3, 5, 2, 1).unwrap()
    }

    fn origin() -> Vec3 {
        Vec3::new(2.0, -4.0, -6.0)
    }

    fn edge() -> Meters {
        Meters::new(2.0)
    }

    fn matter(density: f64, state: State, temperature: f64) -> Sample {
        Sample {
            density: Density::new(density),
            state,
            temperature: Kelvin::new(temperature),
            albedo: [Ratio::new(0.25), Ratio::new(0.5), Ratio::new(0.75)],
            roughness: Ratio::new(0.5),
            attenuation: Attenuation::new(0.125),
        }
    }

    fn section(resolution: u8, f: impl FnMut(u32, u32, u32) -> Sample) -> Section {
        Section::new(
            key(),
            origin(),
            edge(),
            resolution,
            Samples::from_fn(resolution, f),
        )
        .unwrap()
    }

    #[test]
    fn state_values() {
        for (i, s) in State::ALL.iter().enumerate() {
            assert_eq!(usize::from(s.as_u8()), i);
            assert_eq!(State::from_u8(i as u8), Some(*s));
        }
        assert_eq!(State::from_u8(5), None);
        assert_eq!(State::Plasma.name(), "plasma");
    }

    #[test]
    fn round_trip_raw_and_zstd() {
        let s = section(4, |x, y, z| {
            if (x + y + z) % 3 == 0 {
                Sample::VACUUM
            } else {
                matter(f64::from(x + 10 * y + 100 * z), State::Solid, 300.0)
            }
        });
        let raw = encode(&s, Compression::None);
        let packed = encode(&s, Compression::Zstd);
        assert_eq!(raw.len(), HEADER_LEN + 64 * 29);
        assert_ne!(raw, packed);
        assert_eq!(u16_at(&packed, 6), FLAG_ZSTD);
        assert_eq!(decode(&key(), &raw).unwrap(), s);
        assert_eq!(decode(&key(), &packed).unwrap(), s);
        assert_eq!(encode(&s, Compression::Zstd), packed);
        assert!(validate(&key(), &raw).is_ok());
    }

    #[test]
    fn empty_round_trip() {
        let s = Section::empty(key(), origin(), edge()).unwrap();
        for c in [Compression::None, Compression::Zstd] {
            let b = encode(&s, c);
            assert_eq!(b.len(), HEADER_LEN);
            assert_eq!(u16_at(&b, 6), FLAG_EMPTY);
            assert_eq!(decode(&key(), &b).unwrap(), s);
        }
        assert!(s.is_empty());
        assert_eq!(s.mass(), Kilograms::new(0.0));
        assert_eq!(s.sample_volume(), CubicMeters::new(0.0));
    }

    #[test]
    fn header_layout() {
        let s = section(1, |_, _, _| matter(1.0, State::Gas, 10.0));
        let b = encode(&s, Compression::None);
        assert_eq!(&b[0..4], b"3GMS");
        assert_eq!(u16_at(&b, 4), 1);
        assert_eq!(u64_at(&b, 8), 7);
        assert_eq!(b[16], 3);
        assert_eq!(b[17], 1);
        assert_eq!((u32_at(&b, 20), u32_at(&b, 24), u32_at(&b, 28)), (5, 2, 1));
        assert_eq!(f64_at(&b, 32), 2.0);
        assert_eq!(f64_at(&b, 56), 2.0);
        assert_eq!(u64_at(&b, 64), 29);
        // Channel order: density, state, temperature, albedo, roughness,
        // attenuation.
        assert_eq!(&b[72..76], &1.0f32.to_le_bytes());
        assert_eq!(b[76], 3);
        assert_eq!(&b[77..81], &10.0f32.to_le_bytes());
        assert_eq!(&b[81..85], &0.25f32.to_le_bytes());
        assert_eq!(&b[89..93], &0.75f32.to_le_bytes());
        assert_eq!(&b[93..97], &0.5f32.to_le_bytes());
        assert_eq!(&b[97..101], &0.125f32.to_le_bytes());
    }

    #[test]
    fn mass_and_volume() {
        let s = section(2, |x, _, _| {
            if x == 0 {
                matter(3.0, State::Solid, 1.0)
            } else {
                Sample::VACUUM
            }
        });
        assert_eq!(s.sample_volume(), CubicMeters::new(1.0));
        assert_eq!(s.mass(), Kilograms::new(12.0));
    }

    #[test]
    fn new_rejects_bad_values() {
        let err = |r: u8, f: Samples| {
            Section::new(key(), origin(), edge(), r, f)
                .unwrap_err()
                .code
        };
        let one = |s: Sample| Samples::filled(1, s);
        assert_eq!(err(0, Samples::default()), codes::RESOLUTION_OUT_OF_RANGE);
        assert_eq!(err(65, one(Sample::VACUUM)), codes::RESOLUTION_OUT_OF_RANGE);
        assert_eq!(err(2, one(Sample::VACUUM)), codes::SAMPLE_COUNT_MISMATCH);
        assert_eq!(
            err(1, one(matter(-1.0, State::Solid, 1.0))),
            codes::DENSITY_NEGATIVE
        );
        assert_eq!(
            err(1, one(matter(1e39, State::Solid, 1.0))),
            codes::DENSITY_NOT_FINITE
        );
        assert_eq!(
            err(1, one(matter(1.0, State::Vacuum, 1.0))),
            codes::MATTER_STATE_VACUUM
        );
        assert_eq!(
            err(1, one(matter(0.0, State::Solid, 0.0))),
            codes::VACUUM_STATE_NOT_VACUUM
        );
        let mut s = matter(1.0, State::Solid, 1.0);
        s.albedo[2] = Ratio::new(1.5);
        assert_eq!(err(1, one(s)), codes::ALBEDO_OUT_OF_RANGE);
        let mut s = Sample::VACUUM;
        s.attenuation = Attenuation::new(1.0);
        assert_eq!(err(1, one(s)), codes::VACUUM_ATTENUATION_NONZERO);
        let bad = CellKey {
            frame_id: 1,
            depth: 1,
            x: 2,
            y: 0,
            z: 0,
        };
        assert_eq!(
            Section::empty(bad, origin(), edge()).unwrap_err().code,
            codes::KEY_INVALID
        );
        assert_eq!(
            Section::empty(key(), origin(), Meters::new(0.0))
                .unwrap_err()
                .code,
            codes::EDGE_NOT_POSITIVE
        );
        assert_eq!(
            Section::empty(key(), Vec3::new(0.0, f64::NAN, 0.0), edge())
                .unwrap_err()
                .code,
            codes::ORIGIN_NOT_FINITE
        );
    }

    #[test]
    fn reason_names_sample_and_channel() {
        let mut samples = Samples::filled(2, matter(1.0, State::Fluid, 5.0));
        let mut s = matter(1.0, State::Fluid, 5.0);
        s.roughness = Ratio::new(-0.5);
        samples.set(6, s);
        let e = Section::new(key(), origin(), edge(), 2, samples).unwrap_err();
        assert_eq!(e.code, codes::ROUGHNESS_OUT_OF_RANGE);
        assert_eq!(e.reason, "sample 6 channel roughness: -0.5 is negative");
    }

    #[test]
    fn samples_accessors() {
        let mut s = Samples::filled(2, Sample::VACUUM);
        assert_eq!(s.len(), 8);
        assert!(!s.is_empty());
        assert_eq!(s.get(8), None);
        let m = matter(0.1, State::Gas, 2.0);
        s.set(3, m);
        let got = s.get(3).unwrap();
        assert_eq!(got.density.value(), f64::from(0.1f32));
        assert_eq!(got.state, State::Gas);
        assert_eq!(s.density(3), got.density);
        assert_eq!(s.state(3), State::Gas);
        assert_eq!(s.iter().count(), 8);
        let t = Samples::from_samples(s.iter());
        assert_eq!(t, s);
    }

    #[test]
    fn nearest_resampling() {
        assert_eq!(
            (0..4).map(|i| nearest(i, 2, 4)).collect::<Vec<_>>(),
            [0, 0, 1, 1]
        );
        assert_eq!(
            (0..4).map(|i| nearest(i, 1, 4)).collect::<Vec<_>>(),
            [0, 0, 0, 0]
        );
        assert_eq!(
            (0..3).map(|i| nearest(i, 3, 3)).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(
            (0..4).map(|i| nearest(i, 3, 4)).collect::<Vec<_>>(),
            [0, 1, 1, 2]
        );
    }

    #[test]
    fn composite_disjoint_solid_and_fluid() {
        // Solid fills the low-x half at resolution 2; fluid fills the high-x
        // half at resolution 8.
        let solid = section(2, |x, _, _| {
            if x == 0 {
                matter(3000.0, State::Solid, 280.0)
            } else {
                Sample::VACUUM
            }
        });
        let fluid = section(8, |x, _, _| {
            if x >= 4 {
                matter(1000.0, State::Fluid, 290.0)
            } else {
                Sample::VACUUM
            }
        });
        let c = composite(&[&solid, &fluid]).unwrap();
        assert_eq!(c.resolution(), 8);
        let want = solid.mass().value() + fluid.mass().value();
        let got = c.mass().value();
        assert!((got - want).abs() <= want * 1e-12, "{got} vs {want}");
        let m = c.samples().unwrap();
        assert_eq!(m.state(0), State::Solid);
        assert_eq!(m.state(7), State::Fluid);
        assert_eq!(m.get(0).unwrap().temperature, Kelvin::new(280.0));
        // Order does not matter for disjoint inputs.
        assert_eq!(composite(&[&fluid, &solid]).unwrap(), c);
    }

    #[test]
    fn composite_overlap_rules() {
        let a = section(1, |_, _, _| Sample {
            density: Density::new(1.0),
            state: State::Gas,
            temperature: Kelvin::new(100.0),
            albedo: [Ratio::new(0.0); 3],
            roughness: Ratio::new(0.0),
            attenuation: Attenuation::new(0.0),
        });
        let b = section(1, |_, _, _| Sample {
            density: Density::new(3.0),
            state: State::Solid,
            temperature: Kelvin::new(500.0),
            albedo: [Ratio::new(1.0); 3],
            roughness: Ratio::new(1.0),
            attenuation: Attenuation::new(4.0),
        });
        let e = Section::empty(key(), origin(), edge()).unwrap();
        let c = composite(&[&a, &e, &b]).unwrap();
        let s = c.samples().unwrap().get(0).unwrap();
        assert_eq!(s.density, Density::new(4.0));
        assert_eq!(s.state, State::Solid);
        assert_eq!(s.temperature, Kelvin::new(400.0));
        assert_eq!(s.albedo, [Ratio::new(0.75); 3]);
        assert_eq!(s.roughness, Ratio::new(0.75));
        assert_eq!(s.attenuation, Attenuation::new(3.0));
        // Tie: the first densest contributor wins.
        let g = section(1, |_, _, _| matter(3.0, State::Gas, 1.0));
        let c = composite(&[&g, &b]).unwrap();
        assert_eq!(c.samples().unwrap().state(0), State::Gas);
        let c = composite(&[&b, &g]).unwrap();
        assert_eq!(c.samples().unwrap().state(0), State::Solid);
    }

    #[test]
    fn composite_empty_and_mismatch() {
        let e = Section::empty(key(), origin(), edge()).unwrap();
        assert_eq!(composite(&[&e, &e]).unwrap(), e);
        assert_eq!(composite(&[]).unwrap_err().code, codes::COMPOSITE_NO_INPUT);
        let vac = section(2, |_, _, _| Sample::VACUUM);
        let c = composite(&[&vac, &e]).unwrap();
        assert_eq!(c.resolution(), 2);
        assert!(c.samples().unwrap().iter().all(|s| s == Sample::VACUUM));
        let other_key =
            Section::empty(CellKey::new(8, 3, 5, 2, 1).unwrap(), origin(), edge()).unwrap();
        assert_eq!(
            composite(&[&e, &other_key]).unwrap_err().code,
            codes::COMPOSITE_KEY_MISMATCH
        );
        let other_origin = Section::empty(key(), Vec3::zero(), edge()).unwrap();
        assert_eq!(
            composite(&[&e, &other_origin]).unwrap_err().code,
            codes::COMPOSITE_ORIGIN_MISMATCH
        );
        let other_edge = Section::empty(key(), origin(), Meters::new(3.0)).unwrap();
        assert_eq!(
            composite(&[&e, &other_edge]).unwrap_err().code,
            codes::COMPOSITE_EDGE_MISMATCH
        );
    }

    #[test]
    fn composite_overflow_is_rejected() {
        let big = section(1, |_, _, _| matter(f64::from(f32::MAX), State::Solid, 1.0));
        assert_eq!(
            composite(&[&big, &big]).unwrap_err().code,
            codes::DENSITY_NOT_FINITE
        );
    }
}
