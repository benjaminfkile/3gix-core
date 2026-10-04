//! Checks the hub container conformance vectors under
//! `conformance/container/` (`matter-format.md` sections 6 and 8).
//!
//! The valid matter chunk must decode to its two sections, byte for byte the
//! matter vectors it names, with the recorded masses, mass sum, and
//! composite. The valid registry chunk must decode to the registries it
//! names and union to the recorded tree. Every file in `invalid/index.json`
//! must fail with exactly its listed code, and every container code from
//! 701 to 713 must have a file. The top-level validator must accept every
//! valid matter and registry vector under its key and reject a registry
//! under a cell key.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gx_core::container::{chunk_mass, decode_chunk, decode_registry_chunk, encode_chunk};
use gx_core::error::codes;
use gx_core::key::{CellKey, ChunkKey};
use gx_core::matter::{self, composite};
use gx_core::registry::{self, FrameTree};
use gx_core::validate;
use serde_json::Value;

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(conformance().join(rel)).expect(rel)
}

fn read_json(rel: &str) -> Value {
    serde_json::from_slice(&read(rel)).expect("JSON")
}

fn cell(key: &str) -> CellKey {
    match key.parse::<ChunkKey>().expect("key parses") {
        ChunkKey::Cell(c) => c,
        ChunkKey::Registry => panic!("expected a cell key"),
    }
}

/// Reads the `bits` of a recorded number and checks `decimal` agrees.
fn bits(v: &Value) -> u64 {
    let b = v["bits"].as_str().expect("bits");
    let b = u64::from_str_radix(b.trim_start_matches("0x"), 16).expect("hex");
    let d: f64 = v["decimal"].as_str().expect("decimal").parse().unwrap();
    assert_eq!(d.to_bits(), b, "decimal and bits disagree in {v}");
    b
}

#[test]
fn matter_chunk() {
    let want = read_json("container/valid/matter-chunk.json");
    let key_text = want["key"].as_str().unwrap();
    let key = cell(key_text);
    let bytes = read("container/valid/matter-chunk.bin");
    let sections = decode_chunk(&key, &bytes).unwrap();
    let listed = want["sections"].as_array().unwrap();
    assert_eq!(
        sections.len() as u64,
        want["section_count"].as_u64().unwrap()
    );
    assert_eq!(sections.len(), listed.len());
    assert_eq!(sections.len(), 2);

    let mut sources = Vec::new();
    let mut sum = 0.0f64;
    for (s, w) in sections.iter().zip(listed) {
        let src = read(w["source"].as_str().unwrap());
        assert_eq!(*s, matter::decode(&key, &src).unwrap());
        assert_eq!(u64::from(s.resolution()), w["resolution"].as_u64().unwrap());
        assert_eq!(s.mass().value().to_bits(), bits(&w["mass"]));
        sum += s.mass().value();
        sources.push(src);
    }
    let resolutions: Vec<u8> = sections.iter().map(|s| s.resolution()).collect();
    assert_eq!(resolutions, [2, 16]);
    assert_eq!(sum.to_bits(), bits(&want["mass_sum"]));
    assert_eq!(
        chunk_mass(key_text, &bytes).unwrap().value().to_bits(),
        bits(&want["mass_sum"])
    );

    let refs: Vec<&matter::Section> = sections.iter().collect();
    let c = composite(&refs).unwrap();
    assert_eq!(
        u64::from(c.resolution()),
        want["composite"]["resolution"].as_u64().unwrap()
    );
    assert_eq!(c.mass().value().to_bits(), bits(&want["composite"]["mass"]));

    // The data region is the source files, concatenated in table order.
    let data: Vec<u8> = sources.concat();
    assert!(bytes.ends_with(&data));

    // Re-encoding with any ids gives the same sections: ids are dropped.
    let refs: Vec<&[u8]> = sources.iter().map(Vec::as_slice).collect();
    let other = encode_chunk(&refs, &["x", "another-id"]);
    assert_eq!(decode_chunk(&key, &other).unwrap(), sections);
}

#[test]
fn registry_chunk() {
    let want = read_json("container/valid/registry-chunk.json");
    assert_eq!(want["key"], "registry");
    let bytes = read("container/valid/registry-chunk.bin");
    let regs = decode_registry_chunk(&bytes).unwrap();
    let listed = want["sections"].as_array().unwrap();
    assert_eq!(regs.len() as u64, want["section_count"].as_u64().unwrap());
    assert_eq!(regs.len(), listed.len());
    for (r, w) in regs.iter().zip(listed) {
        let src = read(w["source"].as_str().unwrap());
        assert_eq!(*r, registry::decode(&src).unwrap());
        assert_eq!(r.frames().len() as u64, w["frame_count"].as_u64().unwrap());
    }
    let tree = FrameTree::from_registries(&regs).unwrap();
    assert_eq!(
        tree.root().frame_id,
        want["union"]["root"].as_u64().unwrap()
    );
    assert_eq!(
        tree.frames().len() as u64,
        want["union"]["frame_count"].as_u64().unwrap()
    );

    // A registry chunk is not a matter chunk, and the reverse.
    let key = cell(
        read_json("container/valid/matter-chunk.json")["key"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        decode_chunk(&key, &bytes).unwrap_err().code,
        codes::BAD_MAGIC
    );
    let m = read("container/valid/matter-chunk.bin");
    assert_eq!(
        decode_registry_chunk(&m).unwrap_err().code,
        codes::REGISTRY_BAD_MAGIC
    );
}

#[test]
fn invalid_vectors_fail_with_their_code() {
    let index = read_json("container/invalid/index.json");
    let key = cell(index["key"].as_str().unwrap());
    let vectors = index["vectors"].as_object().unwrap();
    let mut seen = BTreeSet::new();
    for (file, code) in vectors {
        let code = u16::try_from(code.as_u64().unwrap()).unwrap();
        let bytes = read(&format!("container/invalid/{file}"));
        let e = decode_chunk(&key, &bytes).expect_err(file);
        assert_eq!(e.code, code, "{file}: {e}");
        seen.insert(code);
    }
    for code in 701..=713 {
        assert!(seen.contains(&code), "no container vector for code {code}");
    }
    // Every file on disk is listed.
    let on_disk = std::fs::read_dir(conformance().join("container/invalid"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != "index.json")
        .count();
    assert_eq!(on_disk, vectors.len());
}

#[test]
fn top_level_validator_dispatches_by_key() {
    for name in ["empty", "single-root", "tree"] {
        let bytes = read(&format!("registry/valid/{name}.bin"));
        assert_eq!(validate("registry", &bytes), Ok(()), "{name}");
    }
    let tree = read("registry/valid/tree.bin");
    let e = validate("7-3-1-2-5", &tree).unwrap_err();
    assert_eq!(e.code, codes::BAD_MAGIC);
    assert!(e.reason.contains("magic"), "{}", e.reason);

    let dir = conformance().join("matter/valid");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let want: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let bytes = std::fs::read(path.with_extension("bin")).unwrap();
            let key = want["key"].as_str().unwrap();
            assert_eq!(validate(key, &bytes), Ok(()), "{}", path.display());
            assert_eq!(
                validate("registry", &bytes).unwrap_err().code,
                codes::REGISTRY_BAD_MAGIC
            );
            n += 1;
        }
    }
    assert!(n >= 7);

    let index = read_json("matter/invalid/index.json");
    let key = index["key"].as_str().unwrap();
    for (file, code) in index["vectors"].as_object().unwrap() {
        let bytes = read(&format!("matter/invalid/{file}"));
        let got = validate(key, &bytes).unwrap_err().code;
        assert_eq!(u64::from(got), code.as_u64().unwrap(), "{file}");
    }
    let index = read_json("registry/invalid/index.json");
    for (file, code) in index["vectors"].as_object().unwrap() {
        let bytes = read(&format!("registry/invalid/{file}"));
        let got = validate("registry", &bytes).unwrap_err().code;
        assert_eq!(u64::from(got), code.as_u64().unwrap(), "{file}");
    }
}
