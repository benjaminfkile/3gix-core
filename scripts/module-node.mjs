// Runs every module conformance vector in a directory under Node's own
// WebAssembly (V8, standing in for the browser) and compares each output
// with its expected SHA-256.
//
// Usage: node scripts/module-node.mjs MODULE.wasm VECTORS_DIR
//
// The module is instantiated with plain WebAssembly.instantiate and an empty
// import object, as the browser worker does (client-compute.md section 4).
// For every NAME.request.bin it reads NAME.tiles.bin and NAME.sha256, runs
// gx_sources and checks the ids equal the tile ids, runs gx_compile, and
// compares the SHA-256 of the section with the expected hash. Each vector
// runs in a fresh instance, as in the native host. Exits 1 if any fails.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";

const [wasmPath, dir] = process.argv.slice(2);
if (!wasmPath || !dir) {
    console.error("usage: node scripts/module-node.mjs MODULE.wasm VECTORS_DIR");
    process.exit(2);
}

const wasm = readFileSync(wasmPath);
const module = await WebAssembly.compile(wasm);
if (WebAssembly.Module.imports(module).length !== 0) {
    console.error("node: module has imports");
    process.exit(1);
}

function unpack(v) {
    const u = BigInt.asUintN(64, v);
    return [Number(u >> 32n), Number(u & 0xffffffffn)];
}

class Call {
    constructor(instance) {
        this.e = instance.exports;
    }
    bytes(ptr, len) {
        return new Uint8Array(this.e.memory.buffer, ptr, len).slice();
    }
    write(data) {
        const ptr = this.e.gx_alloc(data.length);
        if (ptr === 0 && data.length > 0) throw new Error("gx_alloc returned 0");
        new Uint8Array(this.e.memory.buffer, ptr >>> 0, data.length).set(data);
        return [ptr, data.length];
    }
    take(packed) {
        if (packed === -1n) {
            const [p, l] = unpack(this.e.gx_last_error());
            throw new Error("module failed: " + new TextDecoder().decode(this.bytes(p, l)));
        }
        const [p, l] = unpack(packed);
        const out = this.bytes(p, l);
        this.e.gx_free(p | 0, l | 0);
        return out;
    }
}

async function fresh() {
    const instance = await WebAssembly.instantiate(module, {});
    const call = new Call(instance);
    if (call.e.gx_abi_version() !== 1) throw new Error("gx_abi_version is not 1");
    return call;
}

function tileIds(tiles) {
    const v = new DataView(tiles.buffer, tiles.byteOffset, tiles.byteLength);
    const count = v.getUint32(0, true);
    const ids = [];
    let at = 4;
    for (let i = 0; i < count; i++) {
        ids.push(v.getBigUint64(at, true));
        at += 12 + v.getUint32(at + 8, true);
    }
    return ids;
}

function sourceIds(bytes) {
    const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const ids = [];
    for (let at = 0; at + 8 <= bytes.length; at += 8) ids.push(v.getBigUint64(at, true));
    return ids;
}

const names = readdirSync(dir)
    .filter((f) => f.endsWith(".request.bin"))
    .map((f) => f.slice(0, -".request.bin".length))
    .sort();
if (names.length === 0) {
    console.error(`${dir}: no vectors`);
    process.exit(1);
}

let failed = 0;
for (const name of names) {
    try {
        const request = new Uint8Array(readFileSync(join(dir, `${name}.request.bin`)));
        const tiles = new Uint8Array(readFileSync(join(dir, `${name}.tiles.bin`)));
        const expected = readFileSync(join(dir, `${name}.sha256`), "utf8").trim();

        let call = await fresh();
        const [rp, rl] = call.write(request);
        const ids = sourceIds(call.take(call.e.gx_sources(rp, rl)));
        const want = tileIds(tiles);
        if (ids.join(",") !== want.join(",")) {
            throw new Error(`sources [${ids}], tiles carry [${want}]`);
        }

        call = await fresh();
        const [qp, ql] = call.write(request);
        const [tp, tl] = call.write(tiles);
        const section = call.take(call.e.gx_compile(qp, ql, tp, tl));
        const got = createHash("sha256").update(section).digest("hex");
        if (got !== expected) throw new Error(`output ${got}, expected ${expected}`);
        console.log(`node   ok   ${name} ${got}`);
    } catch (e) {
        console.log(`node   FAIL ${name}: ${e.message}`);
        failed++;
    }
}
if (failed > 0) {
    console.error(`node: ${failed} of ${names.length} vectors failed`);
    process.exit(1);
}
console.log(`node: ${names.length} vectors ok`);
