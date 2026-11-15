# 3gix-core

The shared library of the 3GIX space runtime. It is the only place the renderer, the compilers, and the hub meet, and it is what makes the architecture's rules enforceable rather than aspirational.

## What lives here

- **Matter format.** The one wire format a compiler emits, the hub validates, and the renderer reads: a density field sampled on a regular grid over a cell of space, with physical material properties per sample. Encoder, decoder, validator.
- **Units.** Branded SI types. Bare numbers do not enter the encoder.
- **Laws.** Implemented: reference frames with a floating origin (`frames::FrameSystem`, with `relative` as the camera-relative primitive and `nearest_frame` for re-parenting); gravity between every massive frame with the first post-Newtonian (Einstein-Infeld-Hoffmann) corrections, as in the DE ephemerides, plus the J2 force of oblate frames, and the plain Newtonian terms for callers such as the observer (`gravity`, `G = 6.67430e-11 m^3 kg^-1 s^-2`, `C = 299792458 m/s`, no softening, Newtonian gravity from coarse density grids too); rotation by Euler's equations under the torques other masses exert on a frame's principal moments, and constant spin for frames without moments (`rotation`); a fixed-step integrator (`integrate`) whose default scheme keeps the Newtonian part symplectic (fourth order Yoshida) and adds the relativistic, oblateness, and torque terms as a perturbation, with Newtonian-only schemes still selectable, O(n^2) per stage; checkpoints for jumping back in time and evaluating retarded states (`history`, bounded memory); light time, so frames are drawn and lit from where they were when the light now arriving left them (`light`); and the observer as a body in walk mode, with gravity, contact and friction on solid matter, drag, and buoyancy over the matter a renderer-supplied probe reports (`observer`). Explicitly not implemented: collisions between frames, matter contributing to gravity (the registry `mass` is the only gravitational mass), fluid dynamics and heat transport, the translational force of a frame's triaxial shape (only its torque), and relativistic effects beyond first post-Newtonian order. Also implemented: blackbody radiance from temperature (`radiance`, Planck's law per wavelength and integrated over three fixed bands), the light that escapes matter, by radiative transfer through it (`emission`), extinction for volumetric matter (`extinction`), and octree cell selection for a camera (`lod`). Every law function, its units, and its determinism contract are listed in `docs/laws.md`.
- **Bands.** Format version 1 fixes three wavelength bands, `radiance::BAND_EDGES = [700e-9, 600e-9, 500e-9, 400e-9]` meters: band 0 is 600 to 700 nm, band 1 is 500 to 600 nm, band 2 is 400 to 500 nm. A sample's three albedo values map to these bands in this order, and every per-band law output is band 0 first. Band radiance is a 64-point midpoint rule per band, fixed as part of the deterministic contract.
- **Chunk keys and the frame registry.** Encoding, decoding, and validation. The registry is format version 2: 184-byte frame records with `j2`, `reference_radius`, and `principal_moments`; version 1 is rejected.
- **Section addresses, source manifests, module bytes.** The SHA-256 section address and span index of `client-compute.md` sections 2.4 and 3 (`address`), source manifests (`manifest`), and the request, tile, and source id bytes that cross the module ABI (`module_io`), all on a dependency-free SHA-256 (`sha256`).
- **Time scales.** UTC milliseconds to seconds of Barycentric Dynamical Time since J2000 and back (`timescale`), with a built-in leap second table through 2017-01-01 that must be extended when a new leap second is announced.
- **Hub container.** Decoding of the container the hub stores per chunk, with every table entry bounds checked. The hub's layer ids are read past and dropped.
- **C ABI.** `gx_format_version`, `gx_validate`, `gx_error_name`, `gx_section_address`, and `gx_manifest_validate`, exported from a native shared library so a .NET hub or a compiler in any language can call the validator and compute addresses.
- **WebAssembly build.** The same crate for the browser renderer and sandboxed compilers.
- **Conformance vectors.** Golden sections, registries, integrator states, observer states, and light-time solutions any second implementation must pass.
- **Vocabulary lint.** A CI script consumers run that fails on domain words in source.

## The rule this library exists to enforce

The renderer may know physics. It may never know objects.

The format has no name, tag, label, or free-form string field. Frame ids are integers. There is physically no place for a compiler to write what a thing is and no field for a renderer to read it from.

## Specification

The architecture and the byte-level format are specified in the hub repository until the first implementation lands here, at which point the format spec moves to `docs/` in this repository:

- Architecture: `3GIXHub/docs/architecture/space-model.md` (v2)
- Hub, client, and module contract: `3GIXHub/docs/architecture/client-compute.md`
- Byte formats (matter sections version 1, registry version 2): `3GIXHub/docs/architecture/matter-format.md`

## Toolchain

Rust stable. Targets: the host triple for native builds and `wasm32-unknown-unknown` for the browser. `wasm-pack` drives WebAssembly builds. `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` must pass on every change.

`sh scripts/ci.sh` runs the full check locally: toolchain versions, format, clippy, tests, regeneration of the conformance vectors with a byte-for-byte diff against every directory under `conformance/`, the WebAssembly build, a check that the generated TypeScript declarations carry every export, the WebAssembly build run under Node against the native conformance vectors (`scripts/wasm-node-check.mjs`), `wasm-opt`, the headless GPU compute probe, the vocabulary lint, the native shared library build, the C ABI check program, and the module determinism harness (the example module built for `wasm32-unknown-unknown` and run through `gx-module-host` natively and through Node, `scripts/module-determinism.sh`). See `docs/toolchain.md` for the last recorded run.

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
  src/observer.rs    the observer body: gravity, contact, drag, buoyancy
  src/radiance.rs    blackbody radiance per wavelength and per band
  src/emission.rs    the light escaping a section, summarized as one emitter
  src/extinction.rs  extinction coefficient, transmittance, optical depth
  src/lod.rs         octree cell selection for a camera
  src/detmath.rs     deterministic exp, expm1, sin, cos, and tan, public for
                     modules too
  src/sha256.rs      SHA-256 and lowercase hex
  src/address.rs     section addresses and span indices
  src/manifest.rs    source manifests: encode, decode, validate, hash, lookup
  src/module_io.rs   module request, tile, and source id bytes
  src/timescale.rs   UTC milliseconds to TDB seconds since J2000 and back
  src/container.rs   hub container: decode a chunk of matter sections or of
                     registries, and encode one for tests
  src/validate.rs    top-level validator: matter or registry chosen by key
  src/error.rs       ValidationError, the stable numeric error codes, and
                     their short names
  src/lib.rs         C ABI (gx_format_version, gx_validate, gx_error_name,
                     gx_section_address, gx_manifest_validate) and the
                     wasm32 exports
  tests/             integration tests, including the conformance vector checks
                     the orbit tests (laws.rs), and the radiance,
                     emission, and extinction checks (radiance_laws.rs)
crates/gx-module-guest/  what a compiler author links to become a module:
                     gx_alloc, gx_free, gx_abi_version, gx_last_error, the
                     Module trait, and the gx_module! macro (docs/modules.md)
crates/gx-module-host/   the native worker's module host on wasmtime:
                     admission checks, limits, sources and compile; also the
                     gx-module-run harness binary
crates/gx-module-test/   a tiny example module for tests and conformance
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
  address/           section address cases and span index cases
  manifest/          valid manifests and one or more files per manifest code
  module-io/         request, tile, and source id vectors, valid and invalid
  module/            example module cases: request bytes, tile bytes, and the
                     expected output SHA-256, index.json lists them
  timescale/         UTC to TDB cases with bit patterns, and the inverse
                     inside a leap second
include/gx_core.h    hand-written C header for the C ABI
tools/abi-check/     C program that links the shared library through the header
tools/gpu-probe/     headless GPU compute probe (wgpu over Vulkan)
tools/conformance-gen/  regenerates conformance/matter/,
                     conformance/registry/, and conformance/container/
                     deterministically
scripts/             ci.sh, the vocabulary lint, and its word list, and the
                     module determinism harness (module-determinism.sh with
                     module-node.mjs)
docs/                toolchain record, error codes (errors.md), determinism
                     rules (determinism.md), law reference (laws.md), how to
                     write a module (modules.md);
                     format specification once it moves here
```

## Using from C, .NET, or WebAssembly

The native build (`cargo build -p gx-core --release`) produces `target/release/libgx_core.so`. Its C ABI is declared in `include/gx_core.h`: `gx_format_version`, `gx_validate` (one section under one chunk key, matter or registry chosen by the key), `gx_error_name`, `gx_section_address` (the 32-byte section address of `client-compute.md` section 3), and `gx_manifest_validate` (a source manifest). The buffer and return code contract is in `docs/errors.md`.

- **C and C++.** Include `gx_core.h` and link `-lgx_core`. `tools/abi-check/abi_check.c` is a complete caller; `scripts/ci.sh` builds it with the system `cc` and runs it against the conformance vectors.
- **.NET.** P/Invoke the same symbols from `libgx_core`: `size_t` maps to `nuint`, `uint8_t*` and `char*` to `byte*` or `byte[]`, and the return value to `int`.
- **WebAssembly.** `wasm-pack build crates/gx-core --target web` produces a package with these exports: `format_version()`, `validate(key, bytes)`, which throws an `Error` with message `CODE: REASON`, `decode_chunk_mass(key, bytes)`, the total mass in kilograms of every section in a hub container, `section_address(module, manifest, origin_utc_ms, span_seconds, span_index, key)`, the address as lowercase hex (the integers are `BigInt`, `manifest` may be `undefined`), `manifest_validate(bytes)`, `utc_ms_to_tdb_seconds(utc_ms)`, and `tdb_seconds_to_utc_ms(tdb)`. The C ABI symbols are exported from the WebAssembly module too.

## Contributing rules

- No domain words anywhere in this repository: nothing from `scripts/vocab-banned.txt`, and no proper noun for any body. `scripts/vocab-lint.sh` enforces the list.
- No infrastructure identifiers, secrets, hostnames, or LAN addresses in code, history, docs, or CI output. This repository is public.
- No em or en dashes in any text.
