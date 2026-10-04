# 3gix-core

The shared library of the 3GIX space runtime. It is the only place the renderer, the compilers, and the hub meet, and it is what makes the architecture's rules enforceable rather than aspirational.

## What lives here

- **Matter format.** The one wire format a compiler emits, the hub validates, and the renderer reads: a density field sampled on a regular grid over a cell of space, with physical material properties per sample. Encoder, decoder, validator.
- **Units.** Branded SI types. Bare numbers do not enter the encoder.
- **Laws.** Newtonian gravity from point masses and coarse density grids, a symplectic N-body integrator, reference frame transforms with a floating origin, equipotential relaxation for fluid matter, blackbody radiance from temperature.
- **Chunk keys and the frame registry.** Encoding, decoding, and validation.
- **C ABI.** `gx_format_version` and `gx_validate`, exported from a native shared library so a .NET hub or a compiler in any language can call the validator.
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

`sh scripts/ci.sh` runs the full check locally: toolchain versions, format, clippy, tests, regeneration of the conformance vectors with a byte-for-byte diff against `conformance/matter`, the WebAssembly build and `wasm-opt`, the headless GPU compute probe, the vocabulary lint, and the native shared library build. See `docs/toolchain.md` for the last recorded run.

## Layout

```
Cargo.toml           virtual workspace (members crates/* and tools/*)
rust-toolchain.toml  pinned Rust toolchain, components, and wasm32 target
crates/gx-core/      the core library: rlib, C ABI cdylib, and WebAssembly exports
  src/units.rs       branded SI quantities, Vec3, Quat
  src/key.rs         chunk keys and cell geometry
  src/matter.rs      matter sections: encode, decode, validate, composite
  src/error.rs       ValidationError and the stable numeric error codes
  tests/             integration tests, including the conformance vector checks
conformance/         conformance vectors any implementation must pass
  keys.json          chunk key vectors
  matter/valid/      valid sections (.bin) with expected decode results (.json)
  matter/invalid/    one or more files per validation code, index.json maps
                     file name to expected code
include/gx_core.h    hand-written C header for the C ABI
tools/gpu-probe/     headless GPU compute probe (wgpu over Vulkan)
tools/conformance-gen/  regenerates conformance/matter/ deterministically
scripts/             ci.sh, the vocabulary lint, and its word list
docs/                toolchain record, error codes (errors.md), determinism
                     rules (determinism.md); format specification once it
                     moves here
```

## Contributing rules

- No domain words anywhere in this repository: nothing from `scripts/vocab-banned.txt`, and no proper noun for any body. `scripts/vocab-lint.sh` enforces the list.
- No infrastructure identifiers, secrets, hostnames, or LAN addresses in code, history, docs, or CI output. This repository is public.
- No em or en dashes in any text.
