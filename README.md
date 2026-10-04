# 3gix-core

The shared library of the 3GIX space runtime. It is the only place the renderer, the compilers, and the hub meet, and it is what makes the architecture's rules enforceable rather than aspirational.

## What lives here

- **Matter format.** The one wire format a compiler emits, the hub validates, and the renderer reads: a density field sampled on a regular grid over a cell of space, with physical material properties per sample. Encoder, decoder, validator.
- **Units.** Branded SI types. Bare numbers do not enter the encoder.
- **Laws.** Implemented: reference frames with a floating origin (`frames::FrameSystem`, with `relative` as the camera-relative primitive and `nearest_frame` for re-parenting), Newtonian gravity from point masses and from coarse density grids (`gravity`, `G = 6.67430e-11 m^3 kg^-1 s^-2`, no softening), and a fixed-step symplectic N-body integrator (`integrate`, velocity Verlet or fourth order Yoshida, the default) that moves every frame under the gravity of every massive frame, O(n^2) per stage, and turns every frame at its constant angular velocity. Explicitly not implemented: relativity (Newtonian gravity is the law), torques (angular velocity stays constant), collisions between frames, and fluid relaxation to the equipotential surface. Also implemented: blackbody radiance from temperature (`radiance`, Planck's law per wavelength and integrated over three fixed bands), emitter summaries of hot matter (`emission`), extinction for volumetric matter (`extinction`), and octree cell selection for a camera (`lod`). Every law function, its units, and its determinism contract are listed in `docs/laws.md`.
- **Bands.** Format version 1 fixes three wavelength bands, `radiance::BAND_EDGES = [700e-9, 600e-9, 500e-9, 400e-9]` meters: band 0 is 600 to 700 nm, band 1 is 500 to 600 nm, band 2 is 400 to 500 nm. A sample's three albedo values map to these bands in this order, and every per-band law output is band 0 first. Band radiance is a 64-point midpoint rule per band, fixed as part of the deterministic contract.
- **Chunk keys and the frame registry.** Encoding, decoding, and validation.
- **Hub container.** Decoding of the container the hub stores per chunk, with every table entry bounds checked. The hub's layer ids are read past and dropped.
- **C ABI.** `gx_format_version`, `gx_validate`, and `gx_error_name`, exported from a native shared library so a .NET hub or a compiler in any language can call the validator.
- **WebAssembly build.** The same crate for the browser renderer and sandboxed compilers.
- **Conformance vectors.** Golden sections and registries any second implementation must pass.
- **Vocabulary lint.** A CI script consumers run that fails on domain words in source.

## The rule this library exists to enforce

The renderer may know physics. It may never know objects.

The format has no name, tag, label, or free-form string field. Frame ids are integers. There is physically no place for a compiler to write what a thing is and no field for a renderer to read it from.

## Specification

The architecture and the byte-level format are specified in the hub repository until the first implementation lands here, at which point the format spec moves to `docs/` in this repository:

- Architecture: `3GIXHub/docs/architecture/space-model.md`
- Matter format v1: `3GIXHub/docs/architecture/matter-format.md`

## Toolchain

Rust stable. Targets: the host triple for native builds and `wasm32-unknown-unknown` for the browser. `wasm-pack` drives WebAssembly builds. `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` must pass on every change.

`sh scripts/ci.sh` runs the full check locally: toolchain versions, format, clippy, tests, regeneration of the conformance vectors with a byte-for-byte diff against `conformance/matter`, `conformance/registry`, and `conformance/container`, the WebAssembly build, a check that the generated TypeScript declarations carry the three exports, `wasm-opt`, the headless GPU compute probe, the vocabulary lint, the native shared library build, and the C ABI check program. See `docs/toolchain.md` for the last recorded run.

## Layout

```
Cargo.toml           virtual workspace (members crates/* and tools/*)
rust-toolchain.toml  pinned Rust toolchain, components, and wasm32 target
crates/gx-core/      the core library: rlib, C ABI cdylib, and WebAssembly exports
  src/units.rs       branded SI quantities, Vec3, Quat
  src/key.rs         chunk keys and cell geometry
  src/matter.rs      matter sections: encode, decode, validate, composite
  src/registry.rs    frame registry: encode, decode, validate, and the union
                     of a build's registries as a frame tree
  src/frames.rs      frame system: current state of every frame, transforms,
                     floating origin, nearest frame
  src/gravity.rs     Newtonian gravity from point masses and coarse grids
  src/integrate.rs   symplectic N-body integrator: Verlet and Yoshida4
  src/radiance.rs    blackbody radiance per wavelength and per band
  src/emission.rs    hot matter of a section summarized as one emitter
  src/extinction.rs  extinction coefficient, transmittance, optical depth
  src/lod.rs         octree cell selection for a camera
  src/detmath.rs     deterministic exp, expm1, and tan for the laws
  src/container.rs   hub container: decode a chunk of matter sections or of
                     registries, and encode one for tests
  src/validate.rs    top-level validator: matter or registry chosen by key
  src/error.rs       ValidationError, the stable numeric error codes, and
                     their short names
  src/lib.rs         C ABI (gx_format_version, gx_validate, gx_error_name)
                     and the wasm32 exports
  tests/             integration tests, including the conformance vector checks
                     the orbit tests (laws.rs), and the radiance,
                     emission, and extinction checks (radiance_laws.rs)
conformance/         conformance vectors any implementation must pass
  keys.json          chunk key vectors
  matter/valid/      valid sections (.bin) with expected decode results (.json)
  matter/invalid/    one or more files per validation code, index.json maps
                     file name to expected code
  registry/valid/    valid registries (.bin) with every decoded field (.json)
  registry/invalid/  one or more files per registry code, index.json maps
                     file name to expected code
  registry/union/    registries and index.json listing union cases: which
                     files merge and the expected tree or code
  container/valid/   a matter chunk and a registry chunk with expected results
  container/invalid/ one file per container code, index.json maps file name
                     to expected code
include/gx_core.h    hand-written C header for the C ABI
tools/abi-check/     C program that links the shared library through the header
tools/gpu-probe/     headless GPU compute probe (wgpu over Vulkan)
tools/conformance-gen/  regenerates conformance/matter/,
                     conformance/registry/, and conformance/container/
                     deterministically
scripts/             ci.sh, the vocabulary lint, and its word list
docs/                toolchain record, error codes (errors.md), determinism
                     rules (determinism.md), law reference (laws.md);
                     format specification once it moves here
```

## Using from C, .NET, or WebAssembly

The native build (`cargo build -p gx-core --release`) produces `target/release/libgx_core.so`. Its C ABI is declared in `include/gx_core.h`: `gx_format_version`, `gx_validate` (one section under one chunk key, matter or registry chosen by the key), and `gx_error_name`. The buffer and return code contract is in `docs/errors.md`.

- **C and C++.** Include `gx_core.h` and link `-lgx_core`. `tools/abi-check/abi_check.c` is a complete caller; `scripts/ci.sh` builds it with the system `cc` and runs it against the conformance vectors.
- **.NET.** P/Invoke the same symbols from `libgx_core`: `size_t` maps to `nuint`, `uint8_t*` and `char*` to `byte*` or `byte[]`, and the return value to `int`.
- **WebAssembly.** `wasm-pack build crates/gx-core --target web` produces a package with three exports: `format_version()`, `validate(key, bytes)`, which throws an `Error` with message `CODE: REASON`, and `decode_chunk_mass(key, bytes)`, the total mass in kilograms of every section in a hub container. The C ABI symbols are exported from the WebAssembly module too.

## Contributing rules

- No domain words anywhere in this repository: nothing from `scripts/vocab-banned.txt`, and no proper noun for any body. `scripts/vocab-lint.sh` enforces the list.
- No infrastructure identifiers, secrets, hostnames, or LAN addresses in code, history, docs, or CI output. This repository is public.
- No em or en dashes in any text.
