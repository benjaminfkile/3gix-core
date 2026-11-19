//! Admission and run-time behavior of the module host for module ABI
//! version 2 (`client-compute.md` section 4.1), with small modules written
//! in the WebAssembly text format.

use gx_core::key::ChunkKey;
use gx_core::module_io::{encode_input_list, InputEntry, ModuleRequest};
use gx_core::registry::{self, Registry};
use gx_core::sha256::sha256;
use gx_module_host::{HostConfig, HostError, LoadedModule, ModuleHost};

/// A minimal version 2 module. `INPUTS` is the body of `gx_inputs`, `DATA`
/// the bytes at offset 1024 (an empty registry) and `LIST` those at 3072.
/// `gx_compile` returns the registry; it records its input length at 8.
const TEMPLATE: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))
  (data (i32.const 1024) "DATA")
  (data (i32.const 2048) "nope")
  (data (i32.const 3072) "LIST")
  (func (export "gx_abi_version") (result i32) (i32.const 2))
  (func (export "gx_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $p))
  (func (export "gx_free") (param i32 i32))
  EXTRA
  (func (export "gx_sources") (param i32 i32 i32 i32) (result i64) (i64.const 0))
  (func (export "gx_compile") (param i32 i32 i32 i32 i32 i32) (result i64) COMPILE)
  (func (export "gx_last_error") (result i64) (i64.const 0x0000080000000004)))
"#;

fn escape(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

fn list() -> Vec<u8> {
    encode_input_list(&[
        InputEntry {
            slot: 1,
            key: "0-0-0-0-0".parse().unwrap(),
        },
        InputEntry {
            slot: 1,
            key: "0-1-0-0-0".parse().unwrap(),
        },
    ])
    .unwrap()
}

fn module_with(extra: &str, list_bytes: &[u8], compile: &str) -> Vec<u8> {
    let text = TEMPLATE
        .replace("EXTRA", extra)
        .replace("COMPILE", compile)
        .replace("LIST", &escape(list_bytes))
        .replace("DATA", &escape(&registry::encode(&Registry::empty())));
    wat::parse_str(text).expect("test modules parse")
}

fn inputs_export(len: usize) -> String {
    format!(
        r#"(func (export "gx_inputs") (param i32 i32 i32 i32) (result i64) (i64.const {}))"#,
        (3072i64 << 32) | len as i64
    )
}

fn returns_registry() -> String {
    format!(
        "(i64.const {})",
        (1024i64 << 32) | registry::encode(&Registry::empty()).len() as i64
    )
}

fn module(list_bytes: &[u8]) -> Vec<u8> {
    module_with(
        &inputs_export(list_bytes.len()),
        list_bytes,
        &returns_registry(),
    )
}

fn load_with(host: &ModuleHost, bytes: &[u8]) -> Result<LoadedModule, HostError> {
    host.load(bytes, &sha256(bytes))
}

fn load(bytes: &[u8]) -> Result<LoadedModule, HostError> {
    load_with(&ModuleHost::new().unwrap(), bytes)
}

fn request(key: &str) -> ModuleRequest {
    ModuleRequest {
        origin_utc_ms: 0,
        span_seconds: 0,
        span_index: 0,
        key: key.parse::<ChunkKey>().unwrap(),
    }
}

#[test]
fn runs_a_version_2_module() {
    let m = load(&module(&list())).unwrap();
    assert_eq!(m.abi_version(), 2);
    let record = [0u8; 88];
    let got = m.inputs(&request("registry"), &[&record]).unwrap();
    assert_eq!(
        got,
        vec![
            (1, "0-0-0-0-0".parse().unwrap()),
            (1, "0-1-0-0-0".parse().unwrap())
        ]
    );
    assert_eq!(m.inputs_raw(&[], &[]).unwrap(), list());
    assert!(m
        .sources_v2(&request("registry"), &[&record])
        .unwrap()
        .is_empty());
    let out = m
        .compile_v2(&request("registry"), &[], &[&record, b"ab"])
        .unwrap();
    assert_eq!(out, registry::encode(&Registry::empty()));

    // The version 1 calls refuse a version 2 module.
    let e = m.sources(&request("registry")).unwrap_err();
    assert!(matches!(e, HostError::Version(_)), "{e:?}");
    let e = m.compile_raw(&[], &[]).unwrap_err();
    assert!(matches!(e, HostError::Version(_)), "{e:?}");
}

#[test]
fn checks_the_input_list() {
    // Unsorted.
    let mut bad = list();
    let tail = bad.split_off(4);
    let (a, b) = tail.split_at(tail.len() / 2);
    bad.extend_from_slice(b);
    bad.extend_from_slice(a);
    let m = load(&module(&bad)).unwrap();
    let e = m.inputs(&request("registry"), &[]).unwrap_err();
    assert!(
        matches!(&e, HostError::Abi(r) if r.contains("1036")),
        "{e:?}"
    );
    // Trailing bytes.
    let mut bad = list();
    bad.push(0);
    let m = load(&module(&bad)).unwrap();
    let e = m.inputs_raw(&[], &[]).unwrap_err();
    assert!(
        matches!(&e, HostError::Abi(r) if r.contains("1034")),
        "{e:?}"
    );
}

#[test]
fn refuses_exports_that_do_not_match_the_version() {
    // Version 2 without gx_inputs.
    let e = load(&module_with("", &[], &returns_registry()))
        .err()
        .unwrap();
    assert_eq!(e, HostError::Export("gx_inputs is missing".into()));
    // gx_inputs with the wrong signature.
    let e = load(&module_with(
        r#"(func (export "gx_inputs") (param i32 i32) (result i64) (i64.const 0))"#,
        &[],
        &returns_registry(),
    ))
    .err()
    .unwrap();
    assert!(matches!(e, HostError::Export(_)), "{e:?}");
    // A version 1 module that exports gx_inputs.
    let v1 = r#"
(module
  (memory (export "memory") 1)
  (func (export "gx_abi_version") (result i32) (i32.const 1))
  (func (export "gx_alloc") (param i32) (result i32) (i32.const 4096))
  (func (export "gx_free") (param i32 i32))
  (func (export "gx_inputs") (param i32 i32 i32 i32) (result i64) (i64.const 0))
  (func (export "gx_sources") (param i32 i32) (result i64) (i64.const 0))
  (func (export "gx_compile") (param i32 i32 i32 i32) (result i64) (i64.const -1))
  (func (export "gx_last_error") (result i64) (i64.const 0)))
"#;
    let e = load(&wat::parse_str(v1).unwrap()).err().unwrap();
    assert!(
        matches!(&e, HostError::Export(r) if r.contains("gx_inputs")),
        "{e:?}"
    );
}

#[test]
fn version_1_modules_run_through_the_version_2_calls() {
    let v1 = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))
  (func (export "gx_abi_version") (result i32) (i32.const 1))
  (func (export "gx_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $p))
  (func (export "gx_free") (param i32 i32))
  (func (export "gx_sources") (param i32 i32) (result i64) (i64.const 0))
  (func (export "gx_compile") (param i32 i32 i32 i32) (result i64) (i64.const -1))
  (func (export "gx_last_error") (result i64) (i64.const 0)))
"#;
    let m = load(&wat::parse_str(v1).unwrap()).unwrap();
    assert_eq!(m.abi_version(), 1);
    assert!(m.inputs(&request("registry"), &[]).unwrap().is_empty());
    assert_eq!(m.inputs_raw(&[], &[]).unwrap(), 0u32.to_le_bytes());
    assert!(m.sources_v2(&request("registry"), &[]).unwrap().is_empty());
    let e = m.sources_v2(&request("registry"), &[b"x"]).unwrap_err();
    assert!(matches!(e, HostError::Version(_)), "{e:?}");
    let e = m
        .compile_v2(&request("registry"), &[], &[b"x"])
        .unwrap_err();
    assert!(matches!(e, HostError::Version(_)), "{e:?}");
}

#[test]
fn tiles_and_inputs_share_the_memory_limit() {
    let host = ModuleHost::with_config(HostConfig {
        memory_limit: 1 << 20,
        ..HostConfig::default()
    })
    .unwrap();
    let m = load_with(&host, &module(&list())).unwrap();
    let half = vec![0u8; 600 << 10];
    let tiles = vec![(1u64, half.clone())];
    // Each fits alone; together they pass the limit.
    let e = m
        .compile_v2(&request("registry"), &tiles, &[&half])
        .unwrap_err();
    assert_eq!(e, HostError::MemoryLimit(1 << 20));
}
