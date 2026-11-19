#!/bin/sh
# Full local CI: toolchain versions, format, lint, tests, conformance vector
# regeneration, WebAssembly build and export check, the WebAssembly build run
# under Node against the native conformance vectors, headless GPU compute
# probe, vocabulary lint, native shared library builds (the core library and
# the module host), the C ABI check program, and the module determinism
# harness: the example module built for wasm32 and run natively through
# gx-module-host and under Node.
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
node --version

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
diff -r target/conformance-check/registry conformance/registry
diff -r target/conformance-check/container conformance/container
diff -r target/conformance-check/address conformance/address
diff -r target/conformance-check/manifest conformance/manifest
diff -r target/conformance-check/epochs conformance/epochs
diff -r target/conformance-check/frame-record conformance/frame-record
diff -r target/conformance-check/module-io conformance/module-io
diff -r target/conformance-check/timescale conformance/timescale
diff -r target/conformance-check/integrate conformance/integrate
diff -r target/conformance-check/light conformance/light
diff -r target/conformance-check/module conformance/module

step "5. cargo build wasm32"
cargo build -p gx-core --release --target wasm32-unknown-unknown

step "6. wasm-pack build"
wasm-pack build crates/gx-core --release --target web --out-dir ../../target/wasm-pkg

step "6b. wasm exports in the generated TypeScript declarations"
for f in format_version validate decode_chunk_mass section_address manifest_validate \
    epochs_validate epochs_manifest \
    utc_ms_to_tdb_seconds tdb_seconds_to_utc_ms integrate_registry \
    light_retarded_state; do
    grep -q "export function $f(" target/wasm-pkg/gx_core.d.ts || {
        echo "missing wasm export: $f" >&2
        exit 1
    }
done
echo "wasm exports ok"

step "6c. wasm under Node matches the native conformance vectors"
node scripts/wasm-node-check.mjs

step "7. wasm-opt"
wasm-opt -O2 target/wasm-pkg/gx_core_bg.wasm -o target/wasm-pkg/gx_core_bg.opt.wasm

step "8. gpu-probe"
cargo run -p gpu-probe --release

step "9. vocab-lint"
sh scripts/vocab-lint.sh

step "10. native shared library"
cargo build -p gx-core --release
ls target/release/libgx_core.so

step "10b. module host shared library"
cargo build -p gx-module-host --release
ls -l target/release/libgx_module_host.so

step "11. C ABI check program"
mkdir -p target/abi-check
cc -std=c99 -Wall -Wextra -Werror -pedantic -Iinclude \
    tools/abi-check/abi_check.c -Ltarget/release -lgx_core \
    -o target/abi-check/abi_check
LD_LIBRARY_PATH=target/release target/abi-check/abi_check \
    conformance/registry/valid/tree.bin \
    conformance/matter/valid/res16-zstd.bin 4-5-17-3-30 \
    conformance/registry/invalid/610-version-1-registry.bin \
    conformance/manifest/valid/three.bin \
    conformance/manifest/invalid/906-not-sorted.bin \
    conformance/epochs/valid/daily.bin \
    conformance/epochs/invalid/1007-not-ascending.bin \
    conformance/frame-record/valid/registry.bin \
    conformance/frame-record/valid/j2.bin \
    conformance/frame-record/valid/disagrees-mass.bin

step "12. example module for wasm32"
cargo build -p gx-module-test --release --target wasm32-unknown-unknown
module=target/wasm32-unknown-unknown/release/gx_module_test.wasm
ls -l "$module"

step "13. module determinism harness, native and Node"
sh scripts/module-determinism.sh "$module" conformance/module

step "13b. the harness fails on a wrong expected hash"
rm -rf target/module-vectors-broken
cp -r conformance/module target/module-vectors-broken
echo 0000000000000000000000000000000000000000000000000000000000000000 \
    > target/module-vectors-broken/05-span.sha256
if sh scripts/module-determinism.sh "$module" target/module-vectors-broken > /dev/null 2>&1; then
    echo "module-determinism accepted a wrong hash" >&2
    exit 1
fi
echo "harness rejects a wrong hash"

printf '\nci ok\n'
