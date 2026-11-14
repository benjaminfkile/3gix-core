//! Builds this crate for `wasm32-unknown-unknown` and checks the module the
//! guest kit produces: no imports, no relaxed SIMD or threads instruction,
//! admitted by the native host, and every conformance vector in
//! `conformance/module/` reproduced (`client-compute.md` section 4).

use std::path::{Path, PathBuf};
use std::process::Command;

use gx_core::module_io;
use gx_core::sha256::{sha256, to_hex};
use gx_module_host::ModuleHost;
use wasmparser::{Parser, Payload};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds the module into its own target directory, so the outer `cargo
/// test` build is not disturbed, and returns its bytes.
fn build_module() -> Vec<u8> {
    let root = workspace();
    let target = root.join("target/module-wasm-test");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(&root)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .args([
            "build",
            "-q",
            "-p",
            "gx-module-test",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(&target)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "wasm32 build failed");
    std::fs::read(target.join("wasm32-unknown-unknown/release/gx_module_test.wasm"))
        .expect("the module was built")
}

#[test]
fn built_module_is_admissible_and_reproduces_the_vectors() {
    let bytes = build_module();

    // The import section is absent or empty.
    let mut imports = 0;
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::ImportSection(r) = payload.unwrap() {
            imports += r.count();
        }
    }
    assert_eq!(imports, 0, "the module has imports");

    // No relaxed SIMD, threads, or other forbidden instruction.
    let forbidden = gx_module_host::forbidden_opcodes(&bytes).unwrap();
    assert!(
        forbidden.is_empty(),
        "forbidden instructions: {forbidden:?}"
    );
    gx_module_host::check_module(&bytes).unwrap();

    let host = ModuleHost::new().unwrap();
    let module = host.load(&bytes, &sha256(&bytes)).unwrap();
    let dir = workspace().join("conformance/module");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            e.unwrap()
                .file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".request.bin"))
                .map(str::to_string)
        })
        .collect();
    names.sort();
    assert_eq!(names.len(), 10);
    for name in names {
        let read = |s: &str| std::fs::read(dir.join(format!("{name}.{s}"))).unwrap();
        let request = module_io::decode_request(&read("request.bin")).unwrap();
        let tiles: Vec<(u64, Vec<u8>)> = module_io::decode_tiles(&read("tiles.bin"))
            .unwrap()
            .iter()
            .map(|t| (t.id, t.bytes.to_vec()))
            .collect();
        let ids: Vec<u64> = tiles.iter().map(|t| t.0).collect();
        assert_eq!(module.sources(&request).unwrap(), ids, "{name}");
        let section = module.compile(&request, &tiles).unwrap();
        let want = String::from_utf8(read("sha256")).unwrap();
        assert_eq!(to_hex(&sha256(&section)), want.trim(), "{name}");
    }
}
