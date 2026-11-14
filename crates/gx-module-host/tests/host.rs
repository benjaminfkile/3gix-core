//! Admission and run-time behavior of the module host, with small modules
//! written in the WebAssembly text format.

use std::time::Duration;

use gx_core::key::ChunkKey;
use gx_core::module_io::ModuleRequest;
use gx_core::registry::{self, Registry};
use gx_core::sha256::sha256;
use gx_module_host::{HostConfig, HostError, LoadedModule, ModuleHost};

/// A minimal module with every required export. `SOURCES` and `COMPILE` are
/// replaced by the bodies of `gx_sources` and `gx_compile`, `EXTRA` by more
/// module fields, and `DATA` by the bytes at offset 1024.
const TEMPLATE: &str = r#"
(module
  EXTRA
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))
  (data (i32.const 1024) "DATA")
  (data (i32.const 2048) "nope")
  (func (export "gx_abi_version") (result i32) ABI)
  (func (export "gx_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $p))
  (func (export "gx_free") (param i32 i32))
  (func (export "gx_sources") (param i32 i32) (result i64) SOURCES)
  (func (export "gx_compile") (param i32 i32 i32 i32) (result i64) COMPILE)
  (func (export "gx_last_error") (result i64) (i64.const 0x0000080000000004)))
"#;

struct Wat {
    extra: &'static str,
    abi: &'static str,
    sources: &'static str,
    compile: &'static str,
}

const BASE: Wat = Wat {
    extra: "",
    abi: "(i32.const 1)",
    sources: "(i64.const 0)",
    compile: "(i64.const -1)",
};

fn escape(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

fn module(w: Wat) -> Vec<u8> {
    let registry = registry::encode(&Registry::empty());
    let text = TEMPLATE
        .replace("EXTRA", w.extra)
        .replace("ABI", w.abi)
        .replace("SOURCES", w.sources)
        .replace("COMPILE", w.compile)
        .replace("DATA", &escape(&registry));
    wat::parse_str(text).expect("test modules parse")
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

fn registry_len() -> i64 {
    registry::encode(&Registry::empty()).len() as i64
}

#[test]
fn runs_a_valid_module() {
    let compile = format!("(i64.const {})", (1024i64 << 32) | registry_len());
    let compile: &'static str = Box::leak(compile.into_boxed_str());
    let m = load(&module(Wat { compile, ..BASE })).unwrap();
    assert_eq!(m.sources(&request("registry")).unwrap(), Vec::<u64>::new());
    let out = m.compile(&request("registry"), &[]).unwrap();
    assert_eq!(out, registry::encode(&Registry::empty()));
    assert_eq!(m.hash(), sha256(&module(Wat { compile, ..BASE })));
}

#[test]
fn rejects_an_importing_module() {
    let bytes = module(Wat {
        extra: r#"(import "env" "clock" (func (result i64)))"#,
        ..BASE
    });
    let e = load(&bytes).err().unwrap();
    assert_eq!(
        e,
        HostError::Import {
            module: "env".into(),
            name: "clock".into()
        }
    );
    assert!(e.to_string().contains("imports env.clock"), "{e}");
}

#[test]
fn rejects_a_relaxed_simd_opcode() {
    let bytes = module(Wat {
        extra: r#"(func (export "f") (result v128)
            (i8x16.relaxed_swizzle (v128.const i64x2 0 0) (v128.const i64x2 0 0)))"#,
        ..BASE
    });
    match load(&bytes).err().unwrap() {
        HostError::ForbiddenOpcode(o) => {
            assert_eq!(o.proposal, "relaxed_simd");
            assert!(
                o.instruction.contains("RelaxedSwizzle"),
                "{}",
                o.instruction
            );
        }
        e => panic!("wrong error: {e}"),
    }
    let ops = gx_module_host::forbidden_opcodes(&bytes).unwrap();
    assert_eq!(ops.len(), 1);
}

#[test]
fn rejects_a_threads_opcode() {
    let bytes = module(Wat {
        extra: r#"(func (export "f") (atomic.fence))"#,
        ..BASE
    });
    match load(&bytes).err().unwrap() {
        HostError::ForbiddenOpcode(o) => assert_eq!(o.proposal, "threads"),
        e => panic!("wrong error: {e}"),
    }
}

#[test]
fn rejects_other_proposals() {
    let bytes = module(Wat {
        extra: r#"(func (export "f") (result funcref) (ref.null func))"#,
        ..BASE
    });
    assert!(matches!(
        load(&bytes).err().unwrap(),
        HostError::ForbiddenOpcode(_)
    ));
    let bytes = module(Wat {
        extra: r#"(func (export "f") (result i32 i32) (i32.const 1) (i32.const 2))"#,
        ..BASE
    });
    assert!(matches!(load(&bytes).err().unwrap(), HostError::Invalid(_)));
}

#[test]
fn allows_fixed_width_simd_and_the_listed_features() {
    let bytes = module(Wat {
        extra: r#"(func (export "f") (param i32 f64) (result i32)
            (drop (i32x4.add (v128.const i32x4 1 2 3 4) (i32x4.splat (local.get 0))))
            (drop (i32.extend8_s (local.get 0)))
            (memory.fill (i32.const 0) (i32.const 0) (i32.const 0))
            (i32.trunc_sat_f64_s (local.get 1)))"#,
        ..BASE
    });
    load(&bytes).unwrap();
}

#[test]
fn checks_the_hash() {
    let bytes = module(BASE);
    let e = ModuleHost::new()
        .unwrap()
        .load(&bytes, &[0; 32])
        .err()
        .unwrap();
    assert!(matches!(e, HostError::HashMismatch { .. }), "{e}");
    assert!(matches!(
        load(b"not wasm").err().unwrap(),
        HostError::Invalid(_)
    ));
}

#[test]
fn checks_exports_and_abi_version() {
    let text = TEMPLATE.replace(r#"(export "gx_last_error")"#, "");
    let text = text
        .replace("EXTRA", "")
        .replace("ABI", "(i32.const 1)")
        .replace("SOURCES", "(i64.const 0)")
        .replace("COMPILE", "(i64.const 0)")
        .replace("DATA", "");
    let e = load(&wat::parse_str(text).unwrap()).err().unwrap();
    assert_eq!(e, HostError::Export("gx_last_error is missing".into()));

    let e = load(&module(Wat {
        abi: "(i32.const 2)",
        ..BASE
    }))
    .err()
    .unwrap();
    assert_eq!(e, HostError::AbiVersion(2));
}

#[test]
fn reports_module_failure_reason() {
    let m = load(&module(Wat {
        sources: "(i64.const -1)",
        ..BASE
    }))
    .unwrap();
    assert_eq!(
        m.sources(&request("registry")),
        Err(HostError::ModuleFailed("nope".into()))
    );
    assert_eq!(
        m.compile(&request("registry"), &[]),
        Err(HostError::ModuleFailed("nope".into()))
    );
}

#[test]
fn validates_the_returned_section() {
    let m = load(&module(Wat {
        compile: "(i64.const 0x0000040000000004)",
        ..BASE
    }))
    .unwrap();
    assert!(matches!(
        m.compile(&request("registry"), &[]),
        Err(HostError::Section(_))
    ));
    assert!(matches!(
        m.compile(&request("registry"), &[(2, vec![]), (1, vec![])]),
        Err(HostError::Input(_))
    ));
}

#[test]
fn rejects_out_of_bounds_results() {
    let m = load(&module(Wat {
        sources: "(i64.const 0x0001000000000008)",
        ..BASE
    }))
    .unwrap();
    assert!(matches!(
        m.sources(&request("registry")),
        Err(HostError::Abi(_))
    ));
}

#[test]
fn enforces_the_time_limit() {
    let host = ModuleHost::with_config(HostConfig {
        time_limit: Duration::from_millis(200),
        ..HostConfig::default()
    })
    .unwrap();
    let m = load_with(
        &host,
        &module(Wat {
            sources: "(loop $l (br $l)) (i64.const 0)",
            ..BASE
        }),
    )
    .unwrap();
    assert_eq!(
        m.sources(&request("registry")),
        Err(HostError::TimeLimit(Duration::from_millis(200)))
    );
}

#[test]
fn enforces_the_memory_limit() {
    let m = load(&module(Wat {
        sources: "(if (i32.eq (memory.grow (i32.const 16384)) (i32.const -1)) \
                  (then unreachable)) (i64.const 0)",
        ..BASE
    }))
    .unwrap();
    assert_eq!(
        m.sources(&request("registry")),
        Err(HostError::MemoryLimit(1 << 30))
    );

    // Growing and then ignoring the failure still fails the call.
    let host = ModuleHost::with_config(HostConfig {
        memory_limit: 1 << 20,
        ..HostConfig::default()
    })
    .unwrap();
    let m = load_with(
        &host,
        &module(Wat {
            sources: "(drop (memory.grow (i32.const 16))) (i64.const 0)",
            ..BASE
        }),
    )
    .unwrap();
    assert_eq!(
        m.sources(&request("registry")),
        Err(HostError::MemoryLimit(1 << 20))
    );

    // A module whose initial memory exceeds the limit fails at load.
    let text = TEMPLATE
        .replace(
            "(memory (export \"memory\") 1)",
            "(memory (export \"memory\") 32)",
        )
        .replace("EXTRA", "")
        .replace("ABI", "(i32.const 1)")
        .replace("SOURCES", "(i64.const 0)")
        .replace("COMPILE", "(i64.const 0)")
        .replace("DATA", "");
    let e = load_with(&host, &wat::parse_str(text).unwrap())
        .err()
        .unwrap();
    assert_eq!(e, HostError::MemoryLimit(1 << 20));
}

#[test]
fn each_call_gets_a_fresh_instance() {
    // gx_sources bumps a global and fails on the second run in the same
    // instance; a fresh instance per call never sees the bump.
    let m = load(&module(Wat {
        extra: "(global $runs (mut i32) (i32.const 0))",
        sources: "(global.set $runs (i32.add (global.get $runs) (i32.const 1))) \
                  (if (i32.gt_u (global.get $runs) (i32.const 1)) (then unreachable)) \
                  (i64.const 0)",
        ..BASE
    }))
    .unwrap();
    for _ in 0..3 {
        assert_eq!(m.sources(&request("registry")).unwrap(), Vec::<u64>::new());
    }
}
