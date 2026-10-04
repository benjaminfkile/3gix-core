# Determinism

The hub's promise is that one fingerprint means one byte sequence. Every function in this library that produces bytes for the hub must therefore produce identical bytes for identical inputs on every run and every machine.

## How the matter encoder keeps the promise

- Fixed layout: little-endian, fixed-width fields, planar channels in the order of `matter-format.md` section 3.3, no padding.
- Values are rounded from the unit types to `f32` with Rust's `as` cast, which is round-to-nearest-even on every platform.
- No hash map iteration, no clock, no randomness, no environment input.
- Sums such as `Section::mass` and the weighted means in `composite` run in `f64` in a fixed order (sample index order, then input order) with plain operators: no fused multiply add, no SIMD reductions.

## How the registry encoder keeps the promise

- Fixed layout: the 24-byte header and 144-byte records of `matter-format.md` section 5, little-endian, reserved bytes written as 0.
- Records are always in `frame_id` order: `Registry::new` sorts, and `decode` rejects unsorted input rather than reordering it.
- Every `f64` is written with its exact bit pattern, including negative zero.
- The conformance generator builds orientations from rational unit quaternions, so no platform trigonometry enters the vectors.

## zstd is part of the format

A compressed section's bytes depend on the zstd implementation and level, not only on the sample block. These are pinned:

- Level: `gx_core::matter::ZSTD_LEVEL`, 9. One frame, no dictionary, crate default frame parameters (content size written, no checksum).
- Crate: `zstd = "=0.14.0"` in `crates/gx-core/Cargo.toml` (and in `tools/conformance-gen/Cargo.toml`, which builds frames with the wrong length for invalid vectors). `Cargo.lock` pins the bundled C library through `zstd-safe` and `zstd-sys` (`zstd-sys 2.1.0+zstd.1.5.7` at the time of writing).

**Changing the zstd crate version, the bundled C library version, or the level is a format-affecting change.** It can change the compressed bytes of every section, which breaks the fingerprint promise for cached chunks. Treat it like a layout change: regenerate the conformance vectors, review the diff, and coordinate a fingerprint change with the hub.

Decoding is not affected: any conforming zstd decoder reads any conforming frame. Only encoding is pinned.

## The proof

`tools/conformance-gen` regenerates `conformance/matter/` and `conformance/registry/` from code with fixed values and a fixed-seed generator written in the tool. Two checks run it:

- `cargo test -p conformance-gen` runs the generator into a scratch directory and compares every file to the checked-in copy byte for byte.
- `scripts/ci.sh` runs it into `target/conformance-check` and fails on any `diff -r` difference against `conformance/matter` or `conformance/registry`.

To refresh the vectors after an intended change: `rm -rf conformance/matter conformance/registry && cargo run -p conformance-gen -- conformance`.

## How the laws keep the promise

The laws (`frames`, `gravity`, `integrate`, `radiance`, `emission`, `extinction`, `lod`) produce no hub bytes, but they are held to the same rule so that a renderer and a conformance check agree bit for bit on where every frame is at a given time.

- Frames are always visited by ascending `frame_id`, and gravity sources are summed in that order. No hash map is iterated.
- Only `+`, `-`, `*`, `/`, and `sqrt` are used, all correctly rounded in IEEE 754. No platform `sin`, `cos`, or `cbrt`: the Yoshida coefficients come from a written-out cube root of 2, and orientations advance by `normalize(q + 0.5 * dt * q * (0, w))` with no trigonometry.
- `advance` splits an interval into `ceil(|interval| / max_step)` equal steps and sets the time to exactly the target at the end.
- `radiance`, `extinction`, and `lod` need `exp` and `tan`. These are written out in `detmath` from correctly rounded operations, never taken from the platform math library. `band_radiance` uses a fixed 64-point midpoint rule per band. See `laws.md`.
