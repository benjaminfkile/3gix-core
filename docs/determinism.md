# Determinism

The hub's promise is that one fingerprint means one byte sequence. Every function in this library that produces bytes for the hub must therefore produce identical bytes for identical inputs on every run and every machine.

## How the matter encoder keeps the promise

- Fixed layout: little-endian, fixed-width fields, planar channels in the order of `matter-format.md` section 3.3, no padding.
- Values are rounded from the unit types to `f32` with Rust's `as` cast, which is round-to-nearest-even on every platform.
- No hash map iteration, no clock, no randomness, no environment input.
- Sums such as `Section::mass`, the weighted means in `composite`, and its occupancy sums (capped at 1 with `f64::min`) run in `f64` in a fixed order (sample index order, then input order) with plain operators: no fused multiply add, no SIMD reductions.

## How the registry encoder keeps the promise

- Fixed layout: the 24-byte header and 184-byte records of `matter-format.md` section 5 (registry format version 2), little-endian, reserved bytes written as 0. An empty registry always carries epoch 0.
- Records are always in `frame_id` order: `Registry::new` sorts, and `decode` rejects unsorted input rather than reordering it.
- Every `f64` is written with its exact bit pattern, including negative zero.
- The conformance generator builds orientations from rational unit quaternions, so no platform trigonometry enters the vectors.

## Hashes, addresses, manifests, epoch indexes, and module bytes

- SHA-256 is written out in `gx_core::sha256` with integer operations only, no platform intrinsics and no dependency, so every target hashes with the same code.
- A section address is SHA-256 over a fixed little-endian preimage (`client-compute.md` section 3), and is written as lowercase hex. The chunk key in it is the canonical text form, which parsing and printing round trip exactly.
- Manifests, epoch indexes, request bytes, tile bytes, and source id lists are fixed little-endian layouts. Epoch index entries are always in epoch order (`EpochIndex::new` sorts, `decode` rejects unsorted input), and the epoch rule divides in `u128`, so it never overflows or rounds differently. Manifest entries are always in id order (`Manifest::new` sorts, `decode` rejects unsorted input), and tiles and source ids must ascend, so there is one byte form per value.

## Universe time

`clock::universe_seconds` and `clock::wall_ms` produce no hub bytes, but they feed every computation (universe time is seconds since the origin, and every registry's epoch is 0, the origin), so they are held to the same rule:

- `universe_seconds` subtracts the origin from the wall-clock milliseconds in `i64` (panicking on overflow) and divides by 1000 once in `f64`.
- `wall_ms` multiplies by 1000 once, rounds with `f64::round`, and adds the origin in `i64`, saturating at the `i64` range.
- The round trip is exact for every whole millisecond within `2^51` ms of the origin.

`scripts/wasm-node-check.mjs` runs the WebAssembly build under Node against the bit patterns of `clock`'s tests and against `conformance/address/`, and requires the same bit patterns and addresses as the native tests.

## zstd is part of the format

A compressed section's bytes depend on the zstd implementation and level, not only on the sample block. These are pinned:

- Level: `gx_core::matter::ZSTD_LEVEL`, 9. One frame, no dictionary, crate default frame parameters (content size written, no checksum).
- Crate: `zstd = "=0.14.0"` in `crates/gx-core/Cargo.toml` (and in `tools/conformance-gen/Cargo.toml`, which builds frames with the wrong length for invalid vectors). `Cargo.lock` pins the bundled C library through `zstd-safe` and `zstd-sys` (`zstd-sys 2.1.0+zstd.1.5.7` at the time of writing).

**Changing the zstd crate version, the bundled C library version, or the level is a format-affecting change.** It can change the compressed bytes of every section, which breaks the fingerprint promise for cached chunks. Treat it like a layout change: regenerate the conformance vectors, review the diff, and coordinate a fingerprint change with the hub.

Decoding is not affected: any conforming zstd decoder reads any conforming frame. Only encoding is pinned.

## The proof

`tools/conformance-gen` regenerates `conformance/matter/`, `conformance/registry/`, `conformance/container/`, `conformance/address/`, `conformance/manifest/`, `conformance/epochs/`, `conformance/module-io/`, and `conformance/integrate/` from code with fixed values and a fixed-seed generator written in the tool. Two checks run it:

- `cargo test -p conformance-gen` runs the generator into a scratch directory and compares every file to the checked-in copy byte for byte.
- `scripts/ci.sh` runs it into `target/conformance-check` and fails on any `diff -r` difference against those directories.

To refresh the vectors after an intended change: `rm -rf conformance/matter conformance/registry conformance/container conformance/address conformance/manifest conformance/epochs conformance/module-io conformance/integrate && cargo run -p conformance-gen -- conformance`.

## How the laws keep the promise

The laws (`frames`, `gravity`, `rotation`, `integrate`, `history`, `light`, `radiance`, `emission`, `extinction`, `lod`) produce no hub bytes, but they are held to the same rule so that a renderer and a conformance check agree bit for bit on where every frame is at a given time.

- Frames are always visited by ascending `frame_id`, and gravity sources are summed in that order. No hash map is iterated.
- Only `+`, `-`, `*`, `/`, and `sqrt` are used, all correctly rounded in IEEE 754, plus the written-out `sin` and `cos` of `detmath`. No platform `sin`, `cos`, or `cbrt`: the Yoshida coefficients come from a written-out cube root of 2, and orientations advance by exact rotation quaternions `(sin(a/2) n, cos(a/2))` built with `detmath`, renormalized every step.
- The velocity-dependent kick of the default scheme iterates until an iterate repeats bit for bit or a fixed iteration cap; both stopping rules depend only on the values, so every platform stops at the same iteration.
- `conformance/integrate/` pins the state bytes of a six-frame registry (relativistic terms, oblateness, two rigid frames) after three integrations, and `scripts/wasm-node-check.mjs` checks that the WebAssembly build under Node produces the same bytes.
- `conformance/light/` pins four light-time solutions, one clamped to the epoch. `scripts/wasm-node-check.mjs` checks them against the WebAssembly build under Node. The light-time iteration stops by a fixed rule that depends only on the values.
- `advance` splits an interval into `ceil(|interval| / max_step)` equal steps and sets the time to exactly the target at the end.
- `radiance`, `extinction`, and `lod` need `exp` and `tan`, and `rotation` needs `sin` and `cos`. These are written out in `detmath` from correctly rounded operations, never taken from the platform math library. `band_radiance` uses a fixed 64-point midpoint rule per band. See `laws.md`.
