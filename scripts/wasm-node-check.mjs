// Runs the WebAssembly build under Node (V8, standing in for the browser)
// against the conformance vectors the native build is tested with, so the two
// must agree byte for byte: section addresses, UTC to TDB bit patterns and
// back, source manifest validation, matter format version 3 validation
// (every valid and invalid matter vector, and the mass of the matter
// container), registry format version 2 validation, the integrator's state bytes for the integrate vectors, and
// the retarded state bytes for the light vectors.
//
// Usage: node scripts/wasm-node-check.mjs [PKG_DIR]
// PKG_DIR defaults to target/wasm-pkg, the output of the wasm-pack step in
// scripts/ci.sh. Exits 0 when every check passes.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const root = new URL("..", import.meta.url).pathname;
const pkg = process.argv[2] ?? join(root, "target/wasm-pkg");
const gx = await import(pathToFileURL(join(pkg, "gx_core.js")).href);
gx.initSync({ module: readFileSync(join(pkg, "gx_core_bg.wasm")) });

const conformance = join(root, "conformance");
const json = (rel) => JSON.parse(readFileSync(join(conformance, rel), "utf8"));
const bin = (rel) => new Uint8Array(readFileSync(join(conformance, rel)));
const hex = (s) => Uint8Array.from(s.match(/../g).map((b) => parseInt(b, 16)));

let failures = 0;
let checks = 0;
function check(ok, what) {
  checks++;
  if (!ok) {
    failures++;
    console.log(`FAIL: ${what}`);
  }
}

// Returns the numeric code an export threw with, or 0 if it did not throw.
function code(f) {
  try {
    f();
    return 0;
  } catch (e) {
    return Number(String(e.message).split(":")[0]);
  }
}

function f64Bits(x) {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, x, true);
  return "0x" + view.getBigUint64(0, true).toString(16).padStart(16, "0");
}

for (const c of json("address/cases.json").cases) {
  const got = gx.section_address(
    hex(c.module),
    c.manifest === null ? undefined : hex(c.manifest),
    BigInt(c.origin_utc_ms),
    BigInt(c.span_seconds),
    BigInt(c.span_index),
    c.key,
  );
  check(got === c.address, `address ${c.name}: ${got}`);
}
check(code(() => gx.section_address(new Uint8Array(31), undefined, 0n, 0n, 0n, "registry")) === 962,
  "short module hash throws 962");
check(code(() => gx.section_address(new Uint8Array(32), undefined, 0n, 0n, 0n, "1-0-0-0-1")) === 206,
  "malformed key throws 206");

for (const c of json("timescale/cases.json").cases) {
  const tdb = gx.utc_ms_to_tdb_seconds(BigInt(c.utc_ms));
  check(f64Bits(tdb) === c.tdb_seconds.bits, `utc_ms_to_tdb_seconds ${c.name}: ${f64Bits(tdb)}`);
  const back = gx.tdb_seconds_to_utc_ms(tdb);
  check(back === BigInt(c.back_utc_ms), `tdb_seconds_to_utc_ms ${c.name}: ${back}`);
}
for (const c of json("timescale/inverse.json").cases) {
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, BigInt(c.tdb_seconds.bits), true);
  const back = gx.tdb_seconds_to_utc_ms(view.getFloat64(0, true));
  check(back === BigInt(c.utc_ms), `inverse inside the leap second: ${back}`);
}

for (const name of ["empty", "one", "three"]) {
  check(code(() => gx.manifest_validate(bin(`manifest/valid/${name}.bin`))) === 0, `manifest ${name}`);
}
for (const [file, want] of Object.entries(json("manifest/invalid/index.json").vectors)) {
  const got = code(() => gx.manifest_validate(bin(`manifest/invalid/${file}`)));
  check(got === want, `manifest ${file}: ${got}`);
}

check(gx.format_version() === 3, `format_version is ${gx.format_version()}`);
const matterNames = readdirSync(join(conformance, "matter/valid"))
  .filter((f) => f.endsWith(".bin"))
  .map((f) => f.slice(0, -4));
for (const name of matterNames) {
  const want = json(`matter/valid/${name}.json`);
  const got = code(() => gx.validate(want.key, bin(`matter/valid/${name}.bin`)));
  check(got === 0, `matter ${name}: ${got}`);
}
check(matterNames.length >= 10, "matter valid vectors present");
const matterInvalid = json("matter/invalid/index.json");
for (const [file, want] of Object.entries(matterInvalid.vectors)) {
  const got = code(() => gx.validate(matterInvalid.key, bin(`matter/invalid/${file}`)));
  check(got === want, `matter ${file}: ${got}`);
}
for (const c of [107, 514, 515, 528, 529]) {
  check(Object.values(matterInvalid.vectors).includes(c), `matter vector for code ${c}`);
}
const chunk = json("container/valid/matter-chunk.json");
const chunkMass = gx.decode_chunk_mass(chunk.key, bin("container/valid/matter-chunk.bin"));
check(f64Bits(chunkMass) === chunk.mass_sum.bits, `matter chunk mass: ${f64Bits(chunkMass)}`);
const containerInvalid = json("container/invalid/index.json");
for (const [file, want] of Object.entries(containerInvalid.vectors)) {
  const got = code(() => gx.decode_chunk_mass(containerInvalid.key, bin(`container/invalid/${file}`)));
  check(got === want, `container ${file}: ${got}`);
}

for (const name of ["empty", "single-root", "tree"]) {
  check(code(() => gx.validate("registry", bin(`registry/valid/${name}.bin`))) === 0, `registry ${name}`);
}
for (const [file, want] of Object.entries(json("registry/invalid/index.json").vectors)) {
  const got = code(() => gx.validate("registry", bin(`registry/invalid/${file}`)));
  check(got === want, `registry ${file}: ${got}`);
}
check(readdirSync(join(conformance, "registry/invalid")).length > 40, "registry vectors present");

function bitsToF64(bits) {
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, BigInt(bits), true);
  return view.getFloat64(0, true);
}

const integrate = json("integrate/cases.json");
const integrateRegistry = bin(`integrate/${integrate.registry}`);
for (const c of integrate.cases) {
  const got = gx.integrate_registry(
    integrateRegistry,
    bitsToF64(c.to_seconds.bits),
    bitsToF64(c.max_step_seconds.bits),
  );
  const want = bin(`integrate/${c.state}`);
  const same = got.length === want.length && got.every((b, i) => b === want[i]);
  check(same, `integrate ${c.name}: state bytes differ from the native build`);
}
check(integrate.cases.length === 3, "integrate cases present");

const f64s = (list) => Float64Array.from(list.map((n) => bitsToF64(n.bits)));
const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

const light = json("light/cases.json");
const lightRegistry = bin(`light/${light.registry}`);
for (const c of light.cases) {
  const got = gx.light_retarded_state(
    lightRegistry,
    bitsToF64(c.interval_seconds.bits),
    bitsToF64(c.max_step_seconds.bits),
    BigInt(c.frame_id),
    f64s(c.observer),
    bitsToF64(c.t_seconds.bits),
  );
  check(same(got, bin(`light/${c.state_file}`)), `light ${c.name}: retarded state bytes differ from the native build`);
}
check(light.cases.length === 4, "light cases present");

if (failures !== 0) {
  console.log(`wasm-node-check: ${failures} of ${checks} failed`);
  process.exit(1);
}
console.log(`wasm-node-check ok (${checks} checks)`);
