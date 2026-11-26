# Writing a module

A compiler module is a WebAssembly binary that turns one request (a chunk key, the universe's origin, and a time span) plus the source tiles it asked for into one section. The contract is `client-compute.md` section 4 (module ABI version 1) and section 4.1 (module ABI version 2, for a module that reads inputs); this page is how to meet it with `crates/gx-module-guest`.

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
        // Matter section bytes (format version 3) for a cell key, frame
        // registry bytes for `registry`. A module with no frames returns
        // an empty registry.
        Err("not written yet".into())
    }
}

gx_module!(MyModule);
```

The crate provides `gx_alloc`, `gx_free`, and `gx_last_error`. The macro exports `gx_abi_version` (returns 1), `gx_sources`, and `gx_compile`. Around your code they:

1. decode the request bytes and the tile bytes (`gx_core::module_io`),
2. check that the tiles passed are exactly the ids `sources` returns for the request,
3. call `compile`,
4. validate the section with the library validator (`gx_core::validate`) under the request's chunk key,
5. return `(ptr << 32) | len` of the bytes, or -1 with the reason stored for `gx_last_error`.

A matter section is format version 3 (`matter-format.md` section 3.3): every sample carries an `occupancy`, the share of its sub-cube that its matter fills. Compute it from your finer structure, as you compute the mean density: 1 inside matter, the filled share where the boundary between matter and vacuum cuts the sample, exactly 0 for vacuum and above 0 for any sample with density above 0. The level set `occupancy = 0.5` of solid and fluid samples is where the renderer draws the surface, so a linear ramp across the boundary, 0.5 at the boundary, keeps its position exact between sample centers. The density stays the mass in the sub-cube over the whole sub-cube's volume. `crates/gx-module-test` is a worked example: gas in layers whose occupancy is the exact filled share of each sample.

Use `Tiles::get(id)` to read a tile. Tile ids and tile contents are private to your module (`client-compute.md` section 5.1). `gx_sources` also returns -1 with a reason if the request does not decode or the ids are not ascending and unique; the ABI table does not list that case, and the host treats it as a module failure.

The same logic runs natively through `gx_module_guest::run_sources` and `run_compile`, which is how to unit test a module and generate its conformance vectors without a WebAssembly runtime.

## Version 2: modules that read inputs

A layer may read other layers' sections and frame records (`client-compute.md` section 10). Its module implements `ModuleV2` and uses `gx_module_v2!`, which exports `gx_abi_version` (returns 2), `gx_inputs(req, frames)`, `gx_sources(req, frames)`, and `gx_compile(req, tiles, inputs)`:

```rust
use gx_core::frame_record::FrameRecord;
use gx_core::module_io::InputEntry;
use gx_module_guest::{gx_module_v2, FramesSlot, Input, ModuleV2, Request, Tiles};

pub struct MyReader;

impl ModuleV2 for MyReader {
    // The frames slots of the layer's config: slot 0 lists one frame.
    const FRAMES_SLOTS: &'static [FramesSlot] = &[FramesSlot { slot: 0, frames: 1 }];

    fn inputs(req: &Request, frames: &[FrameRecord]) -> Vec<InputEntry> {
        // (slot, key) pairs from the layer slots, sorted by slot then key
        // bytes, unique, at most 64. A function of the request and the
        // records alone.
        Vec::new()
    }
    fn sources(req: &Request, frames: &[FrameRecord]) -> Vec<u64> {
        Vec::new()
    }
    fn compile(
        req: &Request,
        tiles: &Tiles,
        frames: &[FrameRecord],
        inputs: &[Input],
    ) -> Result<Vec<u8>, String> {
        Err("not written yet".into())
    }
}

gx_module_v2!(MyReader);
```

The module knows its own config by construction and declares its frames slots in `FRAMES_SLOTS`. The kit decodes the frame records with `gx_core::frame_record::decode` and passes the input sections as `Input { slot, key, bytes }`, paired with the entries `inputs` returned, in that order. `gx_compile` receives the records as its first inputs, so this kit requires every frames slot to have a lower slot index than every layer slot the module reads; an input list that breaks this fails. A frame key (`frame-{frameId}`) reaches a version 2 module with empty frame and input buffers, and `inputs` must return nothing for it. Around your code the exports check the record count, the input count, and the tiles, and validate the section as for version 1. `run_inputs`, `run_sources_v2`, and `run_compile_v2` run the same logic natively.

The host (`gx_module_host`) reads `gx_abi_version` at admission and refuses a module whose exports are not exactly that version's set: a version 2 module must export `gx_inputs (iiii) -> I`, `gx_sources (iiii) -> I`, and `gx_compile (iiiiii) -> I`, and a version 1 module must not export `gx_inputs`. `LoadedModule::abi_version` reports it. A version 2 module runs with `inputs`, `sources_v2`, and `compile_v2` (the input list is checked with `gx_core::module_io::decode_input_list`; tiles and inputs count against the same memory limit); the version 1 calls refuse it. The version 2 calls also run a version 1 module given no frame records and no inputs. The C ABI (`include/gx_module_host.h`) adds `gx_module_abi_version`, `gx_module_inputs`, `gx_module_sources2`, and `gx_module_compile2`, and the error `GX_MODULE_ERR_VERSION` for a call that does not fit the module's version.

`crates/gx-module-reader` is a worked example: slot 0 is a frames slot of one frame, slot 1 a layer slot reading `crates/gx-module-test`'s layer. For a cell key it reads the same key and, below depth 0, the parent, checks the record describes the key's frame, and returns gas whose density is a fixed function of the inputs' mean densities and the frame's mass. `crates/gx-module-frame` is the version 1 module that declares the frame (id 7) the frames slot names: its registry holds the frame, its frame key answers the record, any other frame key the empty record, and its cells are empty. `crates/gx-module-overlay` is the version 1 module of an overlay layer (`client-compute.md` section 2.2): for surface keys of frame 7 it answers an overlay tile (`matter-format.md` section 12) whose coverage is 1 in a band about the frame's xy plane and partial at the band's edges, for other frames the empty overlay tile, and for every other key the empty section, registry, or frame record; every answer passes `validate_for_layer` with `overlay` true. It declares no frames.

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

`gx_module_host::check_module(bytes)` runs the host's admission checks: no imports, no instruction from a forbidden proposal (relaxed SIMD, threads, reference types, and the rest), and validation with only the allowed features (mutable globals, sign extension, non-trapping float-to-int, bulk memory, fixed-width SIMD). `ModuleHost::load(bytes, hash)` additionally checks the SHA-256, compiles, instantiates with no imports, and checks `gx_abi_version()` is 1 or 2 with exactly that version's exports. Each `sources` and `compile` call runs in a fresh instance with a 1 GiB memory limit and a 60 s time limit by default.

Pin the module's output with conformance vectors: a directory of `NAME.request.bin`, `NAME.tiles.bin`, and `NAME.sha256` (lowercase hex of the expected section). A version 2 module's vectors also carry `NAME.frames.bin` (the frame bytes for `gx_inputs` and `gx_sources`), `NAME.input-list.bin` (the input list `gx_inputs` must return), and `NAME.inputs.bin` (the input bytes for `gx_compile`). Then

```sh
sh scripts/module-determinism.sh path/to/module.wasm path/to/vectors
```

runs every vector through the native host (`gx-module-run`, wasmtime with Cranelift and NaN canonicalization) and through Node's WebAssembly (`scripts/module-node.mjs`, `WebAssembly.instantiate` with `{}` imports, standing in for the browser), and fails if either runtime gives a different hash. `crates/gx-module-test` with `conformance/module/`, `crates/gx-module-frame` with `conformance/module-frame/`, and `crates/gx-module-reader` with `conformance/module-v2/`, and `crates/gx-module-overlay` with `conformance/module-overlay/` are complete examples, and `scripts/ci.sh` runs all four.
