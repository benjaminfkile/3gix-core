# 3gix-core

The shared library of the 3GIX space runtime. It is the only place the renderer, the compilers, and the hub meet, and it is what makes the architecture's rules enforceable rather than aspirational.

## What lives here

- **Matter format.** The one wire format a compiler emits, the hub validates, and the renderer reads: a density field sampled on a regular grid over a cell of space, with physical material properties per sample. Format version 3: eight channels per sample (37 bytes), the last being `occupancy`, the share of the sample's sub-cube its matter fills; the level set `occupancy = 0.5` of solid and fluid samples is the one definition of a surface. Versions 1 and 2 are retired and rejected (code 107). Encoder, decoder, validator, compositing.
- **Units.** Branded SI types. Bare numbers do not enter the encoder.
- **Laws.** Implemented: reference frames with a floating origin (`frames::FrameSystem`, with `relative` as the camera-relative primitive and `nearest_frame` for re-parenting); gravity between every massive frame with the first post-Newtonian (Einstein-Infeld-Hoffmann) corrections, as in the DE ephemerides, plus the J2 force of oblate frames (`gravity`, `G = 6.67430e-11 m^3 kg^-1 s^-2`, `C = 299792458 m/s`, no softening, Newtonian gravity from coarse density grids too); rotation by Euler's equations under the torques other masses exert on a frame's principal moments, and constant spin for frames without moments (`rotation`); a fixed-step integrator (`integrate`) whose default scheme keeps the Newtonian part symplectic (fourth order Yoshida) and adds the relativistic, oblateness, and torque terms as a perturbation, with Newtonian-only schemes still selectable, O(n^2) per stage; checkpoints for jumping back in time and evaluating retarded states (`history`, bounded memory); and light time, so frames are drawn and lit from where they were when the light now arriving left them (`light`). Explicitly not implemented: collisions between frames, matter contributing to gravity (the registry `mass` is the only gravitational mass), fluid dynamics and heat transport, the translational force of a frame's triaxial shape (only its torque), and relativistic effects beyond first post-Newtonian order. Also implemented: blackbody radiance from temperature (`radiance`, Planck's law per wavelength and integrated over three fixed bands), the light that escapes matter, by radiative transfer through it (`emission`), extinction for volumetric matter (`extinction`), and octree cell selection for a camera (`lod`). Every law function, its units, and its determinism contract are listed in `docs/laws.md`.
- **Bands.** The matter format fixes three wavelength bands, `radiance::BAND_EDGES = [700e-9, 600e-9, 500e-9, 400e-9]` meters: band 0 is 600 to 700 nm, band 1 is 500 to 600 nm, band 2 is 400 to 500 nm. A sample's three albedo values map to these bands in this order, and every per-band law output is band 0 first. Band radiance is a 64-point midpoint rule per band, fixed as part of the deterministic contract.
- **Chunk keys and the frame registry.** Encoding, decoding, and validation. The registry is format version 2: 184-byte frame records with `j2`, `reference_radius`, and `principal_moments`; version 1 is rejected.
- **Frame records.** `matter-format.md` section 11: the 88-byte record under the key `frame-{frameId}` holding the constant part of a registry frame (parent, extent, depth, mass, `j2`, reference radius, moments), its empty form, validation (codes 1021 to 1029, also through `gx_validate`), and the section 11.4 agreement check against a registry (`frame_record::agrees`, `gx_frame_record_agrees`).
- **Section addresses, source manifests, module bytes.** The SHA-256 section address and span index of `client-compute.md` sections 2.4 and 3 (`address`), source manifests (`manifest`), epoch indexes and the epoch rule that picks a span's manifest (`epochs`, section 5.4), and the request, tile, and source id bytes that cross the module ABI (`module_io`), all on a dependency-free SHA-256 (`sha256`).
- **Time scales.** UTC milliseconds to seconds of Barycentric Dynamical Time since J2000 and back (`timescale`), with a built-in leap second table through 2017-01-01 that must be extended when a new leap second is announced.
- **Hub container.** Decoding of the container the hub stores per chunk, with every table entry bounds checked. The hub's layer ids are read past and dropped.
- **C ABI.** `gx_format_version`, `gx_validate`, `gx_error_name`, `gx_section_address`, `gx_manifest_validate`, `gx_epochs_validate`, `gx_epochs_manifest`, and `gx_frame_record_agrees`, exported from a native shared library so a .NET hub or a compiler in any language can call the validator and compute addresses.
- **WebAssembly build.** The same crate for the browser renderer and sandboxed compilers.
- **Conformance vectors.** Golden sections, registries, integrator states, and light-time solutions any second implementation must pass.
- **Vocabulary lint.** A CI script consumers run that fails on domain words in source.

## The rule this library exists to enforce

The renderer may know physics. It may never know objects.

The format has no name, tag, label, or free-form string field. Frame ids are integers. There is physically no place for a compiler to write what a thing is and no field for a renderer to read it from.

## Specification

The architecture and the byte-level format are specified in the hub repository until the first implementation lands here, at which point the format spec moves to `docs/` in this repository:

- Architecture: `3GIXHub/docs/architecture/space-model.md` (v2)
- Hub, client, and module contract: `3GIXHub/docs/architecture/client-compute.md`
- Byte formats (matter sections version 3, registry version 2): `3GIXHub/docs/architecture/matter-format.md`
- Rendering contract: `3GIXHub/docs/architecture/rendering.md`

## Toolchain

Rust stable. Targets: the host triple for native builds and `wasm32-unknown-unknown` for the browser. `wasm-pack` drives WebAssembly builds. `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` must pass on every change.

`sh scripts/ci.sh` runs the full check locally: toolchain versions, format, clippy, tests, regeneration of the conformance vectors with a byte-for-byte diff against every directory under `conformance/`, the WebAssembly build, a check that the generated TypeScript declarations carry every export, the WebAssembly build run under Node against the native conformance vectors (`scripts/wasm-node-check.mjs`), `wasm-opt`, the headless GPU compute probe, the vocabulary lint, the native shared library builds (`libgx_core` and `libgx_module_host`), the C ABI check program, and the module determinism harness (the example modules built for `wasm32-unknown-unknown` and run through `gx-module-host` natively and through Node, `scripts/module-determinism.sh`). See `docs/toolchain.md` for the last recorded run.

### Push hook

`scripts/hooks/pre-push` (POSIX `sh`) runs the fast part of CI before every push: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `sh scripts/vocab-lint.sh`, and `cargo test --workspace --lib`. If any of them fails, it prints which one and git refuses the push. Install it once per clone:

```
sh scripts/install-hooks.sh
```

This sets `git config core.hooksPath scripts/hooks` in this clone's local configuration only. The hook is the fast check; `sh scripts/ci.sh` remains the full check every commit passes first (`rendering.md` section 6).

### Validation timing

`cargo run -p gx-core --release --example validate_timing` prints the median of 100 runs of `matter::decode` (decode plus validate) of a resolution 32 section, raw and zstd.

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
  src/history.rs     checkpoints of the frame system
  src/light.rs       light-time solution: retarded times, states, emitters
  src/radiance.rs    blackbody radiance per wavelength and per band
  src/emission.rs    the light escaping a section, summarized as one emitter
  src/extinction.rs  extinction coefficient, transmittance, optical depth
  src/lod.rs         octree cell selection for a camera
  src/detmath.rs     deterministic exp, expm1, sin, cos, and tan, public for
                     modules too
  src/sha256.rs      SHA-256 and lowercase hex
  src/address.rs     section addresses and span indices
  src/manifest.rs    source manifests: encode, decode, validate, hash, lookup
  src/epochs.rs      epoch indexes: encode, decode, validate, hash, and the
                     epoch rule from a span to its manifest
  src/frame_record.rs frame records (matter-format.md section 11): encode,
                     decode, validate, and agreement with the registry
  src/module_io.rs   module request, tile, and source id bytes
  src/timescale.rs   UTC milliseconds to TDB seconds since J2000 and back
  src/container.rs   hub container: decode a chunk of matter sections or of
                     registries, and encode one for tests
  src/validate.rs    top-level validator: matter, registry, surface tile, or
                     frame record chosen by key
  src/error.rs       ValidationError, the stable numeric error codes, and
                     their short names
  src/lib.rs         C ABI (gx_format_version, gx_validate, gx_error_name,
                     gx_section_address, gx_manifest_validate,
                     gx_epochs_validate, gx_epochs_manifest,
                     gx_frame_record_agrees) and the
                     wasm32 exports
  examples/          validate_timing: decode plus validate of a resolution
                     32 section, median of 100 runs
  tests/             integration tests, including the conformance vector checks
                     the orbit tests (laws.rs), and the radiance,
                     emission, and extinction checks (radiance_laws.rs)
crates/gx-module-guest/  what a compiler author links to become a module:
                     gx_alloc, gx_free, gx_last_error, the Module and
                     ModuleV2 traits, and the gx_module! and gx_module_v2!
                     macros (docs/modules.md)
crates/gx-module-host/   the native worker's module host on wasmtime:
                     admission checks, limits, sources and compile, the
                     module host C ABI cdylib (src/ffi.rs), and the
                     gx-module-run harness binary
crates/gx-module-test/   a tiny example module for tests and conformance:
                     layered gas whose occupancy is the filled share of
                     each sample
crates/gx-module-frame/  a tiny version 1 example module that declares one
                     frame, for frames slots in tests
crates/gx-module-reader/ a tiny version 2 example module reading a frame
                     record and gx-module-test's layer
conformance/         conformance vectors any implementation must pass
  keys.json          chunk key vectors
  matter/valid/      valid format 3 sections (.bin) at resolutions 1 to 64,
                     raw and zstd, with expected decode results (.json):
                     mass, per-channel sums including occupancy, and state
                     counts
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
  address/           section address cases and span index cases
  manifest/          valid manifests and one or more files per manifest code
  epochs/            valid epoch indexes with epoch rule cases (span seconds,
                     span index, manifest or none or code), and one or more
                     files per epoch index code
  frame-record/      valid frame records (moments, j2, a root, the empty
                     record) with their keys and agreement results against
                     a registry, and one or more files per frame record code
  module-io/         request, tile, and source id vectors, valid and invalid
  module/            example module cases: request bytes, tile bytes, and the
                     expected output SHA-256, index.json lists them
  module-v2/         version 2 example module cases: request, frame,
                     input list, tile, and input bytes, and the expected
                     output SHA-256
  module-frame/      example frame module cases: registry, its frame key,
                     and another frame key
  timescale/         UTC to TDB cases with bit patterns, and the inverse
                     inside a leap second
include/gx_core.h    hand-written C header for the C ABI
include/gx_module_host.h  hand-written C header for the module host's C ABI
tools/abi-check/     C program that links the shared library through the header
tools/gpu-probe/     headless GPU compute probe (wgpu over Vulkan)
tools/conformance-gen/  regenerates conformance/matter/,
                     conformance/registry/, and conformance/container/
                     deterministically
scripts/             ci.sh, the vocabulary lint, and its word list, the
                     module determinism harness (module-determinism.sh with
                     module-node.mjs), the pre-push hook (hooks/pre-push),
                     and its installer (install-hooks.sh)
docs/                toolchain record, error codes (errors.md), determinism
                     rules (determinism.md), law reference (laws.md), how to
                     write a module (modules.md);
                     format specification once it moves here
```

## Using from C, .NET, or WebAssembly

The native build (`cargo build -p gx-core --release`) produces `target/release/libgx_core.so`. Its C ABI is declared in `include/gx_core.h`: `gx_format_version`, `gx_validate` (one section under one chunk key, matter, registry, surface tile, or frame record chosen by the key), `gx_error_name`, `gx_section_address` (the 32-byte section address of `client-compute.md` section 3), `gx_manifest_validate` (a source manifest), `gx_epochs_validate` (an epoch index), and `gx_epochs_manifest` (the epoch rule of `client-compute.md` section 5.4: 0 with the manifest hash written, 1 when the epoch is not listed, a negative code otherwise), and `gx_frame_record_agrees` (`matter-format.md` section 11.4: 0 when a frame record agrees with the registry, -1 when the registry does not declare the frame, -2 to -8 naming the first differing field, a negated error code for invalid input). The buffer and return code contract is in `docs/errors.md`.

- **C and C++.** Include `gx_core.h` and link `-lgx_core`. `tools/abi-check/abi_check.c` is a complete caller; `scripts/ci.sh` builds it with the system `cc` and runs it against the conformance vectors.
- **.NET.** P/Invoke the same symbols from `libgx_core`: `size_t` maps to `nuint`, `uint8_t*` and `char*` to `byte*` or `byte[]`, and the return value to `int`.
- **WebAssembly.** `wasm-pack build crates/gx-core --target web` produces a package with these exports: `format_version()`, `validate(key, bytes)`, which throws an `Error` with message `CODE: REASON`, `decode_chunk_mass(key, bytes)`, the total mass in kilograms of every section in a hub container, `section_address(module, manifest, origin_utc_ms, span_seconds, span_index, key)`, the address as lowercase hex (the integers are `BigInt`, `manifest` may be `undefined`), `manifest_validate(bytes)`, `epochs_validate(bytes)`, `epochs_manifest(bytes, span_seconds, span_index)`, the manifest hash as lowercase hex or `undefined` when the epoch is not listed, `utc_ms_to_tdb_seconds(utc_ms)`, and `tdb_seconds_to_utc_ms(tdb)`. The C ABI symbols are exported from the WebAssembly module too.

### The module host's C ABI

`gx-core` stays free of wasmtime because it also builds for WebAssembly, so running a module natively is a separate library. `cargo build --release -p gx-module-host` produces `target/release/libgx_module_host.so` on Linux, `libgx_module_host.dylib` on macOS, and `gx_module_host.dll` on Windows, declared in `include/gx_module_host.h`. It is what the hub calls to settle a dispute with the host clients use (`client-compute.md` section 2.7):

- `gx_module_compile(module, module_len, module_hash32, request, request_len, tiles, tiles_len, out_ptr, out_len, err_buf, err_len)` loads the module under its expected SHA-256 (refused with `GX_MODULE_ERR_HASH` otherwise) and runs `gx_compile` on the request and tile bytes of `client-compute.md` section 4, under 1 GiB of memory and 60 s.
- `gx_module_sources(module, module_len, module_hash32, request, request_len, out_ptr, out_len, err_buf, err_len)` runs `gx_sources` and returns the raw `u64 LE` ids.
- `gx_module_abi_version(module, module_len, module_hash32, err_buf, err_len)` returns the module's ABI version, 1 or 2 (`client-compute.md` section 4.1), or a negative code.
- `gx_module_inputs(module, module_len, module_hash32, request, request_len, frames, frames_len, out_ptr, out_len, err_buf, err_len)` runs `gx_inputs` with the frame bytes and returns the raw input list, checked with the library's decoder.
- `gx_module_sources2(..., request, request_len, frames, frames_len, ...)` and `gx_module_compile2(..., request, request_len, tiles, tiles_len, inputs, inputs_len, ...)` run the version 2 `gx_sources` and `gx_compile`. They also run a version 1 module given no frame records or inputs; the version 1 functions refuse a version 2 module with `GX_MODULE_ERR_VERSION`.
- `gx_module_free(ptr, len)` frees a returned buffer.

Each returns 0 on success or a negative `GX_MODULE_ERR_*` code with the reason in `err_buf`, under the same buffer contract as `gx_core.h`; panics are caught at the boundary. The output is the module's own bytes: validate a section with `gx_validate` before using it. From .NET, P/Invoke the symbols from `gx_module_host` with the same type mapping, `uint8_t**` as `byte**` or `out nint`.

## Contributing rules

- No domain words anywhere in this repository: nothing from `scripts/vocab-banned.txt`, and no proper noun for any body. `scripts/vocab-lint.sh` enforces the list.
- No infrastructure identifiers, secrets, hostnames, or LAN addresses in code, history, docs, or CI output. This repository is public.
- No em or en dashes in any text.
