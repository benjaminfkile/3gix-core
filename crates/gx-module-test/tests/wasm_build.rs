//! Builds this crate for `wasm32-unknown-unknown` and checks the module the
//! guest kit produces: no imports, no relaxed SIMD or threads instruction,
//! admitted by the native host, and every conformance vector in
//! `conformance/module/` reproduced (`client-compute.md` section 4), both
//! through the Rust API and through the host's C ABI (section 2.7).

use std::ffi::{c_char, CStr};
use std::path::{Path, PathBuf};
use std::process::Command;

use gx_core::module_io;
use gx_core::sha256::{sha256, to_hex};
use gx_module_host::ffi::{
    self, GX_MODULE_ERR_ARGUMENT, GX_MODULE_ERR_HASH, GX_MODULE_ERR_MODULE_FAILED,
};
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

/// The conformance vector names in `conformance/module/`, sorted.
fn vector_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(workspace().join("conformance/module"))
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
    names
}

/// The C signature of `gx_module_compile`.
type CompileFn = unsafe extern "C" fn(
    *const u8,
    usize,
    *const u8,
    *const u8,
    usize,
    *const u8,
    usize,
    *mut *mut u8,
    *mut usize,
    *mut c_char,
    usize,
) -> i32;

/// The C signature of `gx_module_sources`.
type SourcesFn = unsafe extern "C" fn(
    *const u8,
    usize,
    *const u8,
    *const u8,
    usize,
    *mut *mut u8,
    *mut usize,
    *mut c_char,
    usize,
) -> i32;

/// The C signature of `gx_module_free`.
type FreeFn = unsafe extern "C" fn(*mut u8, usize);

const COMPILE: CompileFn = ffi::gx_module_compile;
const SOURCES: SourcesFn = ffi::gx_module_sources;
const FREE: FreeFn = ffi::gx_module_free;

/// Copies a returned buffer out and frees it with `gx_module_free`.
fn take(ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() {
        assert_eq!(len, 0);
        return Vec::new();
    }
    // SAFETY: the library returned `ptr` valid for `len` bytes.
    let out = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    // SAFETY: `ptr` and `len` are exactly what the library returned.
    unsafe { FREE(ptr, len) };
    out
}

/// Calls `gx_module_compile` and returns the bytes or the code and reason.
fn c_compile(
    module: &[u8],
    hash: &[u8; 32],
    request: &[u8],
    tiles: &[u8],
) -> Result<Vec<u8>, (i32, String)> {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    let mut err = [0 as c_char; 256];
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        COMPILE(
            module.as_ptr(),
            module.len(),
            hash.as_ptr(),
            request.as_ptr(),
            request.len(),
            tiles.as_ptr(),
            tiles.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    if rc != 0 {
        // SAFETY: the library NUL-terminates the reason inside the buffer.
        let reason = unsafe { CStr::from_ptr(err.as_ptr()) };
        return Err((rc, reason.to_string_lossy().into_owned()));
    }
    Ok(take(ptr, len))
}

/// Calls `gx_module_sources` and returns the id bytes or the code and
/// reason.
fn c_sources(module: &[u8], hash: &[u8; 32], request: &[u8]) -> Result<Vec<u8>, (i32, String)> {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    let mut err = [0 as c_char; 256];
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        SOURCES(
            module.as_ptr(),
            module.len(),
            hash.as_ptr(),
            request.as_ptr(),
            request.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    if rc != 0 {
        // SAFETY: the library NUL-terminates the reason inside the buffer.
        let reason = unsafe { CStr::from_ptr(err.as_ptr()) };
        return Err((rc, reason.to_string_lossy().into_owned()));
    }
    Ok(take(ptr, len))
}

#[test]
fn c_abi_reproduces_the_vectors_and_reports_failures() {
    let bytes = build_module();
    let hash = sha256(&bytes);
    let dir = workspace().join("conformance/module");
    let names = vector_names();
    assert_eq!(names.len(), 10);
    for name in &names {
        let read = |s: &str| std::fs::read(dir.join(format!("{name}.{s}"))).unwrap();
        let request = read("request.bin");
        let tiles = read("tiles.bin");
        let want: Vec<u8> = module_io::decode_tiles(&tiles)
            .unwrap()
            .iter()
            .flat_map(|t| t.id.to_le_bytes())
            .collect();
        assert_eq!(c_sources(&bytes, &hash, &request).unwrap(), want, "{name}");
        let section = c_compile(&bytes, &hash, &request, &tiles).unwrap();
        let expected = String::from_utf8(read("sha256")).unwrap();
        assert_eq!(to_hex(&sha256(&section)), expected.trim(), "{name}");
    }

    // A wrong hash is refused before the module runs.
    let request = std::fs::read(dir.join(format!("{}.request.bin", names[1]))).unwrap();
    let tiles = std::fs::read(dir.join(format!("{}.tiles.bin", names[1]))).unwrap();
    let mut wrong = hash;
    wrong[0] ^= 1;
    let (code, reason) = c_compile(&bytes, &wrong, &request, &tiles).unwrap_err();
    assert_eq!(code, GX_MODULE_ERR_HASH);
    assert!(reason.contains("hash mismatch"), "{reason}");
    let (code, _) = c_sources(&bytes, &wrong, &request).unwrap_err();
    assert_eq!(code, GX_MODULE_ERR_HASH);

    // A cell request without its tile fails in the module, with its reason.
    let no_tiles = 0u32.to_le_bytes();
    assert!(!module_io::decode_tiles(&tiles).unwrap().is_empty());
    let (code, reason) = c_compile(&bytes, &hash, &request, &no_tiles).unwrap_err();
    assert_eq!(code, GX_MODULE_ERR_MODULE_FAILED);
    assert!(reason.starts_with("module failed: "), "{reason}");

    // Null pointers with lengths and null outputs are argument errors.
    let mut err = [0 as c_char; 8];
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    // SAFETY: the null pointers are the case under test; the rest are valid.
    let rc = unsafe {
        SOURCES(
            std::ptr::null(),
            4,
            hash.as_ptr(),
            request.as_ptr(),
            request.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    assert_eq!(rc, GX_MODULE_ERR_ARGUMENT);
    // SAFETY: the reason is NUL-terminated within the eight bytes.
    let reason = unsafe { CStr::from_ptr(err.as_ptr()) };
    assert_eq!(reason.to_bytes(), b"module:");
    // SAFETY: as above.
    let rc = unsafe {
        SOURCES(
            bytes.as_ptr(),
            bytes.len(),
            hash.as_ptr(),
            request.as_ptr(),
            request.len(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(rc, GX_MODULE_ERR_ARGUMENT);
    // SAFETY: freeing null is a no-op.
    unsafe { FREE(std::ptr::null_mut(), 0) };
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
    let names = vector_names();
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
