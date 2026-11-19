//! Checks the identity and version 2 address vectors under
//! `conformance/address-v2/` (`client-compute.md` sections 3, 10.3, and
//! 10.5) and the input list and input bytes vectors under
//! `conformance/input-list/` (sections 4.1 and 10.4), through the Rust API
//! and the C ABI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gx_core::address::{
    frames_slot_value, identity_preimage, input_span_index, layer_identity, section_address_v2,
    SlotKind, SlotValue,
};
use gx_core::epochs;
use gx_core::key::ChunkKey;
use gx_core::module_io::{decode_input_list, decode_inputs, inputs_sha256};
use gx_core::sha256::{from_hex, to_hex};
use gx_core::{
    gx_frames_slot_value, gx_input_list_entry, gx_input_list_validate, gx_input_span_index,
    gx_inputs_sha256, gx_layer_identity, gx_section_address_v2,
};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn read_json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect(rel);
    serde_json::from_str(&text).expect("JSON")
}

fn int<T: std::str::FromStr>(v: &Value) -> T
where
    T::Err: std::fmt::Debug,
{
    v.as_str()
        .expect("integers are strings")
        .parse()
        .expect("integer")
}

fn bytes(v: &Value) -> Vec<u8> {
    let s = v.as_str().expect("hex");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

struct Case {
    name: String,
    module: [u8; 32],
    manifest: Option<[u8; 32]>,
    origin: i64,
    span_seconds: u64,
    span_index: u64,
    slots: Vec<SlotValue>,
    slot_bytes: Vec<u8>,
}

/// Parses a case, checking each slot's value against the case it names.
fn case(c: &Value, known: &BTreeMap<String, String>) -> Case {
    let name = c["name"].as_str().expect("name").to_owned();
    let origin: i64 = int(&c["origin_utc_ms"]);
    if !c["uses_origin"].as_bool().expect("uses_origin") {
        assert_eq!(origin, 0, "{name}");
    }
    let mut slots = Vec::new();
    let mut slot_bytes = Vec::new();
    for s in c["slots"].as_array().expect("slots") {
        let kind = match s["kind"].as_str() {
            Some("layer") => SlotKind::Layer,
            Some("frames") => SlotKind::Frames,
            k => panic!("{name}: kind {k:?}"),
        };
        let value = from_hex(s["value"].as_str().expect("value")).expect("hex");
        let of = s["of"].as_str().expect("of");
        assert_eq!(known.get(of), Some(&to_hex(&value)), "{name}: slot of {of}");
        slot_bytes.push(kind.byte());
        slot_bytes.extend_from_slice(&value);
        slots.push(SlotValue { kind, value });
    }
    Case {
        module: from_hex(c["module"].as_str().expect("module")).expect("hex"),
        manifest: c["manifest"].as_str().map(|m| from_hex(m).expect("hex")),
        origin,
        span_seconds: int(&c["span_seconds"]),
        span_index: int(&c["span_index"]),
        slots,
        slot_bytes,
        name,
    }
}

#[test]
fn identities_and_addresses_match() {
    let v = read_json("address-v2/cases.json");
    let mut known = BTreeMap::new();
    for f in v["frames"].as_array().expect("frames") {
        let hashes: Vec<[u8; 32]> = f["records"]
            .as_array()
            .expect("records")
            .iter()
            .map(|r| from_hex(r.as_str().expect("hex")).expect("hex"))
            .collect();
        let got = to_hex(&frames_slot_value(&hashes));
        assert_eq!(got, f["value"], "{}", f["name"]);
        let flat = hashes.concat();
        let mut out = [0u8; 32];
        assert_eq!(
            unsafe { gx_frames_slot_value(flat.as_ptr(), flat.len(), out.as_mut_ptr()) },
            0
        );
        assert_eq!(to_hex(&out), got);
        known.insert(f["name"].as_str().expect("name").to_owned(), got);
    }

    for s in v["input_spans"].as_array().expect("input_spans") {
        let (i, r, x) = (
            int(&s["reader_span_index"]),
            int(&s["reader_span_seconds"]),
            int(&s["input_span_seconds"]),
        );
        let mut out = 0u64;
        let rc = unsafe { gx_input_span_index(i, r, x, &mut out) };
        match s.get("input_span_index") {
            Some(want) => {
                assert_eq!(input_span_index(i, r, x), Ok(int::<u64>(want)), "{s}");
                assert_eq!((rc, out), (0, int::<u64>(want)), "{s}");
            }
            None => {
                let code = s["code"].as_u64().expect("code");
                assert_eq!(u64::from(input_span_index(i, r, x).unwrap_err().code), code);
                assert_eq!(rc as u64, code);
            }
        }
    }

    let (mut epoch_cases, mut nested) = (0, 0);
    for c in v["identities"].as_array().expect("identities") {
        let k = case(c, &known);
        if let Some(index) = c.get("epoch_index") {
            let index = epochs::decode(&bytes(index)).expect("epoch index");
            let m = epochs::manifest_for_span(&index, k.span_seconds, k.span_index).expect("rule");
            assert_eq!(m, k.manifest, "{}: epoch manifest", k.name);
            epoch_cases += 1;
        }
        nested += usize::from(
            k.slots.iter().any(|s| s.kind == SlotKind::Layer)
                && c["slots"][0]["of"]
                    .as_str()
                    .is_some_and(|o| !o.starts_with("static") && !o.starts_with("epoch")),
        );
        let pre = identity_preimage(
            &k.module,
            k.manifest.as_ref(),
            k.origin,
            k.span_seconds,
            k.span_index,
            &k.slots,
        )
        .unwrap();
        assert_eq!(pre, bytes(&c["preimage"]), "{}: preimage", k.name);
        let id = layer_identity(
            &k.module,
            k.manifest.as_ref(),
            k.origin,
            k.span_seconds,
            k.span_index,
            &k.slots,
        )
        .unwrap();
        assert_eq!(to_hex(&id), c["identity"], "{}", k.name);
        let mut out = [0u8; 32];
        let rc = unsafe {
            gx_layer_identity(
                k.module.as_ptr(),
                k.manifest
                    .as_ref()
                    .map_or(core::ptr::null(), |m| m.as_ptr()),
                k.origin,
                k.span_seconds,
                k.span_index,
                k.slot_bytes.as_ptr(),
                k.slot_bytes.len(),
                out.as_mut_ptr(),
            )
        };
        assert_eq!((rc, out), (0, id), "{}: C ABI", k.name);
        known.insert(k.name, to_hex(&id));
    }
    assert!(epoch_cases >= 1 && nested >= 1);

    let (mut refused, mut ok) = (0, 0);
    for c in v["addresses"].as_array().expect("addresses") {
        let k = case(c, &known);
        let key_str = c["key"].as_str().expect("key");
        let key: ChunkKey = key_str.parse().expect("key");
        let got = section_address_v2(
            &k.module,
            k.manifest.as_ref(),
            k.origin,
            k.span_seconds,
            k.span_index,
            &k.slots,
            &key,
        );
        let mut out = [0u8; 32];
        let rc = unsafe {
            gx_section_address_v2(
                k.module.as_ptr(),
                k.manifest
                    .as_ref()
                    .map_or(core::ptr::null(), |m| m.as_ptr()),
                k.origin,
                k.span_seconds,
                k.span_index,
                k.slot_bytes.as_ptr(),
                k.slot_bytes.len(),
                key_str.as_ptr(),
                key_str.len(),
                out.as_mut_ptr(),
            )
        };
        match c.get("address") {
            Some(want) => {
                let a = got.expect("address");
                assert_eq!(to_hex(&a), *want, "{}", k.name);
                assert_eq!((rc, out), (0, a), "{}: C ABI", k.name);
                assert_eq!(
                    gx_core::sha256::sha256(&bytes(&c["preimage"])),
                    a,
                    "{}: preimage",
                    k.name
                );
                ok += 1;
            }
            None => {
                let code = c["code"].as_u64().expect("code");
                assert_eq!(u64::from(got.unwrap_err().code), code, "{}", k.name);
                assert_eq!(rc as u64, code, "{}: C ABI", k.name);
                refused += 1;
            }
        }
    }
    assert!(refused >= 1 && ok >= 4);
}

#[test]
fn input_lists_match() {
    let valid = read_json("input-list/valid/index.json");
    for (file, entries) in valid.as_object().expect("index") {
        let b = std::fs::read(root().join("input-list/valid").join(file)).expect("file");
        let got = decode_input_list(&b).expect(file);
        let want = entries.as_array().expect("entries");
        assert_eq!(got.len(), want.len(), "{file}");
        let mut count = u32::MAX;
        let rc = unsafe {
            gx_input_list_validate(b.as_ptr(), b.len(), &mut count, core::ptr::null_mut(), 0)
        };
        assert_eq!((rc, count as usize), (0, want.len()), "{file}");
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert_eq!(u64::from(g.slot), w["slot"].as_u64().unwrap(), "{file}");
            assert_eq!(g.key.to_string(), w["key"].as_str().unwrap(), "{file}");
            let (mut slot, mut len, mut buf) = (u32::MAX, 0usize, [0u8; 56]);
            let rc = unsafe {
                gx_input_list_entry(
                    b.as_ptr(),
                    b.len(),
                    i as u32,
                    &mut slot,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut len,
                )
            };
            assert_eq!(rc, 0);
            assert_eq!(slot, g.slot);
            assert_eq!(&buf[..len], w["key"].as_str().unwrap().as_bytes());
        }
        let (mut slot, mut len) = (0u32, 0usize);
        let past = unsafe {
            gx_input_list_entry(
                b.as_ptr(),
                b.len(),
                want.len() as u32,
                &mut slot,
                core::ptr::null_mut(),
                0,
                &mut len,
            )
        };
        assert_eq!(past, 1039, "{file}: index past the end");
    }

    let invalid = read_json("input-list/invalid/index.json");
    let mut codes = std::collections::BTreeSet::new();
    for (file, code) in invalid.as_object().expect("index") {
        let b = std::fs::read(root().join("input-list/invalid").join(file)).expect("file");
        let code = code.as_u64().expect("code");
        assert_eq!(
            u64::from(decode_input_list(&b).unwrap_err().code),
            code,
            "{file}"
        );
        let mut err = [0i8; 64];
        let rc = unsafe {
            gx_input_list_validate(
                b.as_ptr(),
                b.len(),
                core::ptr::null_mut(),
                err.as_mut_ptr().cast(),
                64,
            )
        };
        assert_eq!(rc as u64, code, "{file}: C ABI");
        assert_ne!(err[0], 0, "{file}: reason written");
        codes.insert(code);
    }
    assert_eq!(
        codes.into_iter().collect::<Vec<_>>(),
        (1031..=1036).collect::<Vec<_>>()
    );
}

#[test]
fn inputs_match() {
    let v = read_json("input-list/inputs.json");
    for c in v["valid"].as_array().expect("valid") {
        let b = bytes(&c["bytes"]);
        let got = decode_inputs(&b).expect("valid");
        let want: Vec<Vec<u8>> = c["inputs"].as_array().unwrap().iter().map(bytes).collect();
        assert_eq!(got, want.iter().map(Vec::as_slice).collect::<Vec<_>>());
        assert_eq!(to_hex(&inputs_sha256(&got)), c["inputs_sha256"]);
        let mut out = [0u8; 32];
        assert_eq!(
            unsafe { gx_inputs_sha256(b.as_ptr(), b.len(), out.as_mut_ptr()) },
            0
        );
        assert_eq!(to_hex(&out), c["inputs_sha256"]);
    }
    for c in v["invalid"].as_array().expect("invalid") {
        let b = bytes(&c["bytes"]);
        let code = c["code"].as_u64().expect("code");
        assert_eq!(
            u64::from(decode_inputs(&b).unwrap_err().code),
            code,
            "{}",
            c["name"]
        );
        let mut out = [0u8; 32];
        let rc = unsafe { gx_inputs_sha256(b.as_ptr(), b.len(), out.as_mut_ptr()) };
        assert_eq!(rc as u64, code);
    }
}
