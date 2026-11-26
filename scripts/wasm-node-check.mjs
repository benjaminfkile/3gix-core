// Runs the WebAssembly build under Node (V8, standing in for the browser)
// against the conformance vectors the native build is tested with, so the two
// must agree byte for byte: section addresses, layer identities and version
// 2 addresses, frames slot values, input span indexes, input lists and the
// inputs hash, universe time bit patterns from wall-clock milliseconds and
// back, source manifest validation, epoch index validation and the epoch
// rule cases, frame record validation under `frame-` keys, overlay tile
// validation and the layer-aware check (`validate_layer`), matter format version 3 validation
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

{
  const v = json("address-v2/cases.json");
  for (const f of v.frames) {
    const flat = new Uint8Array(32 * f.records.length);
    f.records.forEach((r, i) => flat.set(hex(r), 32 * i));
    check(gx.frames_slot_value(flat) === f.value, `frames_slot_value ${f.name}`);
  }
  for (const s of v.input_spans) {
    const args = [BigInt(s.reader_span_index), BigInt(s.reader_span_seconds), BigInt(s.input_span_seconds)];
    if (s.input_span_index !== undefined) {
      check(gx.input_span_index(...args) === BigInt(s.input_span_index), `input_span_index ${JSON.stringify(s)}`);
    } else {
      check(code(() => gx.input_span_index(...args)) === s.code, `input_span_index code ${JSON.stringify(s)}`);
    }
  }
  const slots = (c) => {
    const b = new Uint8Array(33 * c.slots.length);
    c.slots.forEach((s, i) => {
      b[33 * i] = s.kind === "layer" ? 0 : 1;
      b.set(hex(s.value), 33 * i + 1);
    });
    return b;
  };
  const head = (c) => [
    hex(c.module),
    c.manifest === null ? undefined : hex(c.manifest),
    BigInt(c.origin_utc_ms),
    BigInt(c.span_seconds),
    BigInt(c.span_index),
    slots(c),
  ];
  for (const c of v.identities) {
    check(gx.layer_identity(...head(c)) === c.identity, `layer_identity ${c.name}`);
  }
  for (const c of v.addresses) {
    if (c.address !== undefined) {
      check(gx.section_address_v2(...head(c), c.key) === c.address, `section_address_v2 ${c.name} ${c.key}`);
    } else {
      check(code(() => gx.section_address_v2(...head(c), c.key)) === c.code, `section_address_v2 ${c.name} code`);
    }
  }
  const lists = json("input-list/valid/index.json");
  for (const [file, entries] of Object.entries(lists)) {
    const b = bin(`input-list/valid/${file}`);
    check(gx.input_list_validate(b) === entries.length, `input_list_validate ${file}`);
  }
  const bad = json("input-list/invalid/index.json");
  for (const [file, want] of Object.entries(bad)) {
    const b = bin(`input-list/invalid/${file}`);
    check(code(() => gx.input_list_validate(b)) === want, `input_list_validate ${file}`);
  }
  const inputs = json("input-list/inputs.json");
  for (const c of inputs.valid) {
    check(gx.inputs_sha256(hex(c.bytes)) === c.inputs_sha256, `inputs_sha256 ${c.name}`);
  }
  for (const c of inputs.invalid) {
    check(code(() => gx.inputs_sha256(hex(c.bytes))) === c.code, `inputs_sha256 ${c.name}`);
  }
}

// The same cases and bit patterns as `BIT_CASES` in crates/gx-core/src/clock.rs:
// [wall_ms, origin_ms, bits of universe_seconds].
{
  const origin = 1234567890123n;
  const b51 = 1n << 51n;
  const cases = [
    [origin, origin, "0x0000000000000000"],
    [origin + 1n, origin, "0x3f50624dd2f1a9fc"],
    [origin - 1n, origin, "0xbf50624dd2f1a9fc"],
    [origin + 86399999n, origin, "0x40f517fffbe76c8b"],
    [b51 - 1n, 0n, "0x4280624dd2f1a9fa"],
    [-(b51 - 1n), 0n, "0xc280624dd2f1a9fa"],
    [1700000000000n, 1600000000000n, "0x4197d78400000000"],
  ];
  for (const [wall, o, bits] of cases) {
    const t = gx.universe_seconds(wall, o);
    check(f64Bits(t) === bits, `universe_seconds ${wall} ${o}: ${f64Bits(t)}`);
    const back = gx.wall_ms(t, o);
    check(back === wall, `wall_ms ${wall} ${o}: ${back}`);
  }
  check(gx.wall_ms(-0.0005, 0n) === -1n, "wall_ms rounds half away from zero");
  check(gx.wall_ms(Infinity, 0n) === (1n << 63n) - 1n, "wall_ms saturates");
  let threw = false;
  try {
    gx.universe_seconds((1n << 63n) - 1n, -1n);
  } catch {
    threw = true;
  }
  check(threw, "universe_seconds throws on overflow");
}

for (const name of ["empty", "one", "three"]) {
  check(code(() => gx.manifest_validate(bin(`manifest/valid/${name}.bin`))) === 0, `manifest ${name}`);
}
for (const [file, want] of Object.entries(json("manifest/invalid/index.json").vectors)) {
  const got = code(() => gx.manifest_validate(bin(`manifest/invalid/${file}`)));
  check(got === want, `manifest ${file}: ${got}`);
}

for (const name of ["regular", "empty", "widest"]) {
  const bytes = bin(`epochs/valid/${name}.bin`);
  check(code(() => gx.epochs_validate(bytes)) === 0, `epochs ${name}`);
  for (const c of json(`epochs/valid/${name}.json`).cases) {
    const what = `epochs ${name}: span ${c.span_seconds} s, index ${c.span_index}`;
    const run = () => gx.epochs_manifest(bytes, BigInt(c.span_seconds), BigInt(c.span_index));
    if (c.code !== undefined) {
      const got = code(run);
      check(got === c.code, `${what}: ${got}`);
    } else {
      const got = run();
      check((got ?? null) === c.manifest, `${what}: ${got}`);
    }
  }
}
for (const [file, want] of Object.entries(json("epochs/invalid/index.json").vectors)) {
  const got = code(() => gx.epochs_validate(bin(`epochs/invalid/${file}`)));
  check(got === want, `epochs ${file}: ${got}`);
}

const frValid = json("frame-record/valid/index.json");
for (const [file, want] of Object.entries(frValid.records)) {
  const got = code(() => gx.validate(want.key, bin(`frame-record/valid/${file}`)));
  check(got === 0, `frame record ${file}: ${got}`);
}
const frInvalid = json("frame-record/invalid/index.json");
for (const [file, want] of Object.entries(frInvalid.vectors)) {
  const got = code(() => gx.validate(frInvalid.key, bin(`frame-record/invalid/${file}`)));
  check(got === want, `frame record ${file}: ${got}`);
}
for (let c = 1021; c <= 1029; c++) {
  check(Object.values(frInvalid.vectors).includes(c), `frame record vector for code ${c}`);
}

const ovValid = json("overlay/valid/index.json");
for (const [file, want] of Object.entries(ovValid.tiles)) {
  const got = code(() => gx.validate(want.key, bin(`overlay/valid/${file}`)));
  check(got === 0, `overlay ${file}: ${got}`);
}
const ovInvalid = json("overlay/invalid/index.json");
for (const [file, want] of Object.entries(ovInvalid.vectors)) {
  const got = code(() => gx.validate_layer(ovInvalid.key, bin(`overlay/invalid/${file}`), true));
  check(got === want, `overlay ${file}: ${got}`);
}
for (let c = 1051; c <= 1057; c++) {
  check(Object.values(ovInvalid.vectors).includes(c), `overlay vector for code ${c}`);
}
for (const [file, c] of Object.entries(json("overlay/layer/index.json").cases)) {
  const got = code(() => gx.validate_layer(c.key, bin(`overlay/layer/${file}`), c.overlay));
  check(got === c.code, `overlay layer ${file}: ${got}, want ${c.code}`);
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
