# Writing a module

A compiler module is a WebAssembly binary that turns one request (a chunk key, the universe's origin, and a time span) plus the source tiles it asked for into one section. The contract is `client-compute.md` section 4 (module ABI version 1); this page is how to meet it with `crates/gx-module-guest`.

## The crate

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
gx-core = { path = "...", default-features = false }
gx-module-guest = { path = "..." }
```

`default-features = false` on `gx-core` matters: the default `exports` feature emits the library's C ABI symbols and its `wasm-bindgen` exports, and the latter bring imports with them. A module must have none.

## The code

```rust
use gx_module_guest::{gx_module, Module, Request, Tiles};

pub struct MyModule;

impl Module for MyModule {
    fn sources(req: &Request) -> Vec<u64> {
        // Tile ids this request needs, ascending and unique. Empty if none.
        Vec::new()
    }

    fn compile(req: &Request, tiles: &Tiles) -> Result<Vec<u8>, String> {
        // Matter section bytes for a cell key, frame registry bytes for
        // `registry`. A module with no frames returns an empty registry.
        Err("not written yet".into())
    }
}

gx_module!(MyModule);
```

The crate provides `gx_alloc`, `gx_free`, `gx_abi_version` (returns 1), and `gx_last_error`. The macro exports `gx_sources` and `gx_compile`. Around your code they:

1. decode the request bytes and the tile bytes (`gx_core::module_io`),
2. check that the tiles passed are exactly the ids `sources` returns for the request,
3. call `compile`,
4. validate the section with the library validator (`gx_core::validate`) under the request's chunk key,
5. return `(ptr << 32) | len` of the bytes, or -1 with the reason stored for `gx_last_error`.

Use `Tiles::get(id)` to read a tile. Tile ids and tile contents are private to your module (`client-compute.md` section 5.1). `gx_sources` also returns -1 with a reason if the request does not decode or the ids are not ascending and unique; the ABI table does not list that case, and the host treats it as a module failure.

The same logic runs natively through `gx_module_guest::run_sources` and `run_compile`, which is how to unit test a module and generate its conformance vectors without a WebAssembly runtime.

## Determinism

The module must return the same bytes for the same request and tiles on every machine (`space-model.md` section 1 rule 7):

- No clock, no randomness, no environment. There are no imports, so these are unavailable anyway.
- No hash map iteration into the output. Use sorted vectors or `BTreeMap`.
- No platform math library: use `gx_core::detmath` for `exp`, `expm1`, `sin`, `cos`, and `tan`. Plain `+`, `-`, `*`, `/`, and `sqrt` are correctly rounded and safe.
- No threads and no relaxed SIMD. Fixed-width SIMD is allowed.
- Output never depends on the memory or time limit.

## Building

```sh
cargo build -p my-module --release --target wasm32-unknown-unknown
```

The Rust standard library for `wasm32-unknown-unknown` is built with the reference-types feature, so its `call_indirect` instructions carry a padded table index that a WebAssembly 1.0 validator rejects. Add a `build.rs` to the module crate that passes two linker arguments on `wasm32`, as `crates/gx-module-test/build.rs` does:

```rust
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-cdylib=--compress-relocations");
        println!("cargo:rustc-link-arg-cdylib=--strip-debug");
    }
}
```

## Checking

`gx_module_host::check_module(bytes)` runs the host's admission checks: no imports, no instruction from a forbidden proposal (relaxed SIMD, threads, reference types, and the rest), and validation with only the allowed features (mutable globals, sign extension, non-trapping float-to-int, bulk memory, fixed-width SIMD). `ModuleHost::load(bytes, hash)` additionally checks the SHA-256, compiles, instantiates with no imports, and checks every export and `gx_abi_version() == 1`. Each `sources` and `compile` call runs in a fresh instance with a 1 GiB memory limit and a 60 s time limit by default.

Pin the module's output with conformance vectors: a directory of `NAME.request.bin`, `NAME.tiles.bin`, and `NAME.sha256` (lowercase hex of the expected section). Then

```sh
sh scripts/module-determinism.sh path/to/module.wasm path/to/vectors
```

runs every vector through the native host (`gx-module-run`, wasmtime with Cranelift and NaN canonicalization) and through Node's WebAssembly (`scripts/module-node.mjs`, `WebAssembly.instantiate` with `{}` imports, standing in for the browser), and fails if either runtime gives a different hash. `crates/gx-module-test` with `conformance/module/` is a complete example, and `scripts/ci.sh` runs it.
