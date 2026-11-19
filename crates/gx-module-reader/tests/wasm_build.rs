//! Builds the version 2 example module and the example frame module for
//! `wasm32-unknown-unknown` and checks both: admitted by the native host
//! with the right ABI version, and every conformance vector in
//! `conformance/module-v2/` and `conformance/module-frame/` reproduced
//! (`client-compute.md` sections 4, 4.1, and 10), through the Rust API and
//! through the host's C ABI (`gx_module_abi_version`, `gx_module_inputs`,
//! `gx_module_sources2`, `gx_module_compile2`).

use std::ffi::{c_char, CStr};
use std::path::{Path, PathBuf};
use std::process::Command;

use gx_core::module_io;
use gx_core::sha256::{sha256, to_hex};
use gx_module_host::ffi::{self, GX_MODULE_ERR_MODULE_FAILED, GX_MODULE_ERR_VERSION};
use gx_module_host::ModuleHost;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds a module crate into its own target directory and returns its
/// bytes.
fn build_module(package: &str) -> Vec<u8> {
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
            package,
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(&target)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "wasm32 build of {package} failed");
    let file = package.replace('-', "_");
    std::fs::read(target.join(format!("wasm32-unknown-unknown/release/{file}.wasm")))
        .expect("the module was built")
}

fn vector_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
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

/// Copies a returned buffer out and frees it with `gx_module_free`.
fn take(ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() {
        assert_eq!(len, 0);
        return Vec::new();
    }
    // SAFETY: the library returned `ptr` valid for `len` bytes.
    let out = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    // SAFETY: exactly what the library returned.
    unsafe { ffi::gx_module_free(ptr, len) };
    out
}

/// Turns a return code and buffers into a result.
fn result(rc: i32, ptr: *mut u8, len: usize, err: &[c_char]) -> Result<Vec<u8>, (i32, String)> {
    if rc != 0 {
        // SAFETY: the library NUL-terminates the reason inside the buffer.
        let reason = unsafe { CStr::from_ptr(err.as_ptr()) };
        return Err((rc, reason.to_string_lossy().into_owned()));
    }
    Ok(take(ptr, len))
}

fn c_inputs(m: &[u8], h: &[u8; 32], req: &[u8], frames: &[u8]) -> Result<Vec<u8>, (i32, String)> {
    let (mut ptr, mut len, mut err) = (std::ptr::null_mut(), 0usize, [0 as c_char; 256]);
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        ffi::gx_module_inputs(
            m.as_ptr(),
            m.len(),
            h.as_ptr(),
            req.as_ptr(),
            req.len(),
            frames.as_ptr(),
            frames.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    result(rc, ptr, len, &err)
}

fn c_sources2(m: &[u8], h: &[u8; 32], req: &[u8], frames: &[u8]) -> Result<Vec<u8>, (i32, String)> {
    let (mut ptr, mut len, mut err) = (std::ptr::null_mut(), 0usize, [0 as c_char; 256]);
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        ffi::gx_module_sources2(
            m.as_ptr(),
            m.len(),
            h.as_ptr(),
            req.as_ptr(),
            req.len(),
            frames.as_ptr(),
            frames.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    result(rc, ptr, len, &err)
}

fn c_compile2(
    m: &[u8],
    h: &[u8; 32],
    req: &[u8],
    tiles: &[u8],
    inputs: &[u8],
) -> Result<Vec<u8>, (i32, String)> {
    let (mut ptr, mut len, mut err) = (std::ptr::null_mut(), 0usize, [0 as c_char; 256]);
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        ffi::gx_module_compile2(
            m.as_ptr(),
            m.len(),
            h.as_ptr(),
            req.as_ptr(),
            req.len(),
            tiles.as_ptr(),
            tiles.len(),
            inputs.as_ptr(),
            inputs.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    result(rc, ptr, len, &err)
}

fn c_abi_version(m: &[u8], h: &[u8; 32]) -> i32 {
    // SAFETY: every pointer is valid for its length.
    unsafe { ffi::gx_module_abi_version(m.as_ptr(), m.len(), h.as_ptr(), std::ptr::null_mut(), 0) }
}

#[test]
fn version_2_module_reproduces_the_vectors() {
    let bytes = build_module("gx-module-reader");
    let hash = sha256(&bytes);
    gx_module_host::check_module(&bytes).unwrap();
    let host = ModuleHost::new().unwrap();
    let module = host.load(&bytes, &hash).unwrap();
    assert_eq!(module.abi_version(), 2);
    assert_eq!(c_abi_version(&bytes, &hash), 2);

    let dir = workspace().join("conformance/module-v2");
    let names = vector_names(&dir);
    assert_eq!(names.len(), 5);
    for name in &names {
        let read = |s: &str| std::fs::read(dir.join(format!("{name}.{s}"))).unwrap();
        let (request, frames) = (read("request.bin"), read("frames.bin"));
        let (tiles, inputs) = (read("tiles.bin"), read("inputs.bin"));
        let want = String::from_utf8(read("sha256")).unwrap();

        // Rust API, typed.
        let req = module_io::decode_request(&request).unwrap();
        let records = module_io::decode_inputs(&frames).unwrap();
        let list: Vec<(u32, gx_core::key::ChunkKey)> =
            module_io::decode_input_list(&read("input-list.bin"))
                .unwrap()
                .into_iter()
                .map(|e| (e.slot, e.key))
                .collect();
        assert_eq!(module.inputs(&req, &records).unwrap(), list, "{name}");
        assert!(module.sources_v2(&req, &records).unwrap().is_empty());
        let ins = module_io::decode_inputs(&inputs).unwrap();
        let section = module.compile_v2(&req, &[], &ins).unwrap();
        assert_eq!(to_hex(&sha256(&section)), want.trim(), "{name}");

        // C ABI, raw.
        let got = c_inputs(&bytes, &hash, &request, &frames).unwrap();
        assert_eq!(got, read("input-list.bin"), "{name}");
        assert!(c_sources2(&bytes, &hash, &request, &frames)
            .unwrap()
            .is_empty());
        let section = c_compile2(&bytes, &hash, &request, &tiles, &inputs).unwrap();
        assert_eq!(to_hex(&sha256(&section)), want.trim(), "{name}");
    }

    // A frame record of another frame fails in the module.
    let read = |s: &str| std::fs::read(dir.join(format!("04-depth-one.{s}"))).unwrap();
    let mut inputs = read("inputs.bin");
    // The record's frame id sits at bytes 8..16 of the record, after the
    // count and the record's length.
    inputs[4 + 4 + 8] ^= 1;
    let (code, reason) = c_compile2(
        &bytes,
        &hash,
        &read("request.bin"),
        &read("tiles.bin"),
        &inputs,
    )
    .unwrap_err();
    assert_eq!(code, GX_MODULE_ERR_MODULE_FAILED);
    assert!(reason.contains("frame mismatch"), "{reason}");

    // The version 1 calls refuse a version 2 module.
    let (mut ptr, mut len, mut err) = (std::ptr::null_mut(), 0usize, [0 as c_char; 64]);
    let request = read("request.bin");
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        ffi::gx_module_sources(
            bytes.as_ptr(),
            bytes.len(),
            hash.as_ptr(),
            request.as_ptr(),
            request.len(),
            &mut ptr,
            &mut len,
            err.as_mut_ptr(),
            err.len(),
        )
    };
    assert_eq!(rc, GX_MODULE_ERR_VERSION);
}

#[test]
fn frame_module_reproduces_the_vectors() {
    let bytes = build_module("gx-module-frame");
    let hash = sha256(&bytes);
    let host = ModuleHost::new().unwrap();
    let module = host.load(&bytes, &hash).unwrap();
    assert_eq!(module.abi_version(), 1);
    assert_eq!(c_abi_version(&bytes, &hash), 1);
    let dir = workspace().join("conformance/module-frame");
    let names = vector_names(&dir);
    assert_eq!(names.len(), 3);
    for name in &names {
        let read = |s: &str| std::fs::read(dir.join(format!("{name}.{s}"))).unwrap();
        let req = module_io::decode_request(&read("request.bin")).unwrap();
        let want = String::from_utf8(read("sha256")).unwrap();
        let section = module.compile(&req, &[]).unwrap();
        assert_eq!(to_hex(&sha256(&section)), want.trim(), "{name}");
        // The version 2 calls run it as version 1 with empty buffers.
        assert_eq!(
            c_inputs(&bytes, &hash, &read("request.bin"), &[]).unwrap(),
            0u32.to_le_bytes()
        );
        let section =
            c_compile2(&bytes, &hash, &read("request.bin"), &read("tiles.bin"), &[]).unwrap();
        assert_eq!(to_hex(&sha256(&section)), want.trim(), "{name}");
    }
}
