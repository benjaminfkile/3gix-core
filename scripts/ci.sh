#!/bin/sh
# Full local CI: toolchain versions, format, lint, tests, conformance vector
# regeneration, WebAssembly build, headless GPU compute probe, vocabulary lint,
# native shared library check.
set -eu

cd "$(dirname "$0")/.."

step() {
    printf '\n==> %s\n' "$*"
}

step "1. toolchain versions"
rustc --version
cargo --version
wasm-pack --version
wasm-opt --version

step "2. cargo fmt"
cargo fmt --all -- --check

step "3. cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "4. cargo test"
cargo test --workspace

step "4b. conformance vectors regenerate byte for byte"
rm -rf target/conformance-check
cargo run -p conformance-gen --release -- target/conformance-check
diff -r target/conformance-check/matter conformance/matter

step "5. cargo build wasm32"
cargo build -p gx-core --release --target wasm32-unknown-unknown

step "6. wasm-pack build"
wasm-pack build crates/gx-core --release --target web --out-dir ../../target/wasm-pkg

step "7. wasm-opt"
wasm-opt -O2 target/wasm-pkg/gx_core_bg.wasm -o target/wasm-pkg/gx_core_bg.opt.wasm

step "8. gpu-probe"
cargo run -p gpu-probe --release

step "9. vocab-lint"
sh scripts/vocab-lint.sh

step "10. native shared library"
cargo build -p gx-core --release
ls target/release/libgx_core.so

printf '\nci ok\n'
