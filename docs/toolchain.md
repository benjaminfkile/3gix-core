# Toolchain record

Result of `sh scripts/ci.sh` in the build container on 2026-10-04. Every value below is copied from that run.

## Result

`sh scripts/ci.sh` exited 0 and printed `ci ok`. No step is blocked.

## Versions (step 1)

```
rustc 1.99.0 (b940084d7 2026-09-28)
cargo 1.99.0 (5f94df478 2026-08-27)
wasm-pack 0.15.0
wasm-opt version 108
```

Host triple: `aarch64-unknown-linux-gnu`, 10 cores.

## GPU probe (step 8)

```
adapter: llvmpipe (LLVM 15.0.6, 128 bits)
backend: Vulkan
driver: llvmpipe (Mesa 22.3.6 (LLVM 15.0.6))
gpu-probe ok
```

The adapter is Mesa's software Vulkan driver (lavapipe), so compute runs on the CPU with no GPU device present. Mesa also prints `error: XDG_RUNTIME_DIR is invalid or not set in the environment.` twice to stderr before the adapter line. This is noise from its window system probing and does not affect the result.

## Wall-clock time

40 seconds for the whole script, measured with `date +%s` before and after, starting from `cargo clean`. The crate registry was already populated, so this excludes download time. Per-step cargo timings from the same run: clippy 8.05 s, test build 10.45 s, wasm32 build 2.99 s, wasm-pack build 2.85 s, gpu-probe release build 15.55 s.

## Steps

1. `rustc --version`, `cargo --version`, `wasm-pack --version`, `wasm-opt --version`
2. `cargo fmt --all -- --check`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo test --workspace` (4 tests in `gx-core`, all pass)
5. `cargo build -p gx-core --release --target wasm32-unknown-unknown`
6. `wasm-pack build crates/gx-core --release --target web --out-dir ../../target/wasm-pkg`
7. `wasm-opt -O2 target/wasm-pkg/gx_core_bg.wasm -o target/wasm-pkg/gx_core_bg.opt.wasm`
8. `cargo run -p gpu-probe --release`
9. `sh scripts/vocab-lint.sh` (printed `vocab-lint ok`)
10. `cargo build -p gx-core --release`, then `ls target/release/libgx_core.so`
