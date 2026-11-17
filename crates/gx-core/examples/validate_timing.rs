//! Times `matter::decode` (decode plus validate, `matter-format.md`
//! section 4) of one resolution 32 section, raw and zstd, and prints the
//! median of 100 runs of each.
//!
//! Run with `cargo run -p gx-core --release --example validate_timing`.
//! The section is a solid ball inside a gas shell inside vacuum, so every
//! rule of section 3.3 runs on matter and on vacuum samples. Timing is the
//! only output that varies between runs; the section bytes do not.

use std::hint::black_box;
use std::time::Instant;

use gx_core::key::CellKey;
use gx_core::matter::{self, Compression, Sample, Samples, Section, State};
use gx_core::units::{Attenuation, Density, Kelvin, Meters, Ratio};

/// Runs per measurement.
const RUNS: usize = 100;

/// Samples per axis.
const RESOLUTION: u8 = 32;

fn matter_sample(density: f64, state: State, occupancy: f64) -> Sample {
    Sample {
        density: Density::new(density),
        state,
        temperature: Kelvin::new(280.0),
        albedo: [Ratio::new(0.3), Ratio::new(0.4), Ratio::new(0.5)],
        roughness: Ratio::new(0.6),
        attenuation: Attenuation::new(0.01),
        radiating_temperature: Kelvin::new(280.0),
        occupancy: Ratio::new(occupancy),
    }
}

fn section() -> (CellKey, Section) {
    let key = CellKey::new(1, 2, 1, 2, 3).expect("fixed key is valid");
    let g = key.geometry(Meters::new(1.0e6));
    let n = f64::from(RESOLUTION);
    let samples = Samples::from_fn(RESOLUTION, |x, y, z| {
        let c = |i: u32| (f64::from(i) + 0.5) / n - 0.5;
        let r = (c(x) * c(x) + c(y) * c(y) + c(z) * c(z)).sqrt();
        if r < 0.3 {
            matter_sample(3000.0, State::Solid, 1.0)
        } else if r < 0.45 {
            matter_sample(1.0, State::Gas, 1.0)
        } else {
            Sample::VACUUM
        }
    });
    let s = Section::new(key, g.origin, g.edge, RESOLUTION, samples).expect("valid section");
    (key, s)
}

/// Median wall time of `RUNS` decodes of `bytes`, in seconds.
fn median_decode(key: &CellKey, bytes: &[u8]) -> f64 {
    let mut times = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let t = Instant::now();
        let s = matter::decode(black_box(key), black_box(bytes)).expect("valid bytes");
        black_box(&s);
        times.push(t.elapsed().as_secs_f64());
    }
    times.sort_by(f64::total_cmp);
    (times[RUNS / 2 - 1] + times[RUNS / 2]) / 2.0
}

fn main() {
    let (key, s) = section();
    for (name, c) in [("raw", Compression::None), ("zstd", Compression::Zstd)] {
        let bytes = matter::encode(&s, c);
        let t = median_decode(&key, &bytes);
        println!(
            "decode+validate res{RESOLUTION} {name}: {} bytes, median of {RUNS}: {:.1} us",
            bytes.len(),
            t * 1e6
        );
    }
}
