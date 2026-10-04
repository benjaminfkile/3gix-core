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

## Layout

```
crates/            Rust crates (the core library, the C ABI shim, tooling probes)
conformance/       golden byte files and expected results
docs/              format specification once it moves here
scripts/           the vocabulary lint and other CI helpers
```

## Contributing rules

- No domain words anywhere in this repository: no earth, planet, moon, sun, star, solar, terrain, ocean, sea, water, land, sky, atmosphere, sphere, globe, and no proper noun for any body. The lint enforces this.
- No infrastructure identifiers, secrets, hostnames, or LAN addresses in code, history, docs, or CI output. This repository is public.
- No em or en dashes in any text.
