#!/bin/sh
# Module determinism harness (client-compute.md section 4, space-model.md
# section 4 item 6).
#
# Usage: sh scripts/module-determinism.sh MODULE.wasm VECTORS_DIR
#
# Runs every vector in VECTORS_DIR (NAME.request.bin, NAME.tiles.bin,
# NAME.sha256) through the native module host (gx-module-run, wasmtime with
# Cranelift) and through Node's own WebAssembly (scripts/module-node.mjs,
# plain WebAssembly.instantiate with an empty import object). Fails if any
# output differs from the expected hash in either runtime.
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: sh scripts/module-determinism.sh MODULE.wasm VECTORS_DIR" >&2
    exit 2
fi
wasm=$1
vectors=$2

cd "$(dirname "$0")/.."

cargo build -q -p gx-module-host --release --bin gx-module-run

status=0
target/release/gx-module-run "$wasm" "$vectors" || status=1
node scripts/module-node.mjs "$wasm" "$vectors" || status=1

if [ "$status" -ne 0 ]; then
    echo "module-determinism: FAILED" >&2
    exit 1
fi
echo "module-determinism ok"
