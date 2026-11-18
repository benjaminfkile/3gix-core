//! The module host's C ABI on a module that exceeds the default 1 GiB
//! memory limit (`client-compute.md` section 4, rules).

use std::ffi::{c_char, CStr};

use gx_core::sha256::sha256;
use gx_module_host::ffi::{gx_module_sources, GX_MODULE_ERR_MEMORY_LIMIT};

#[test]
fn reports_the_memory_limit() {
    let bytes = wat::parse_str(
        r#"(module
          (memory (export "memory") 1)
          (func (export "gx_abi_version") (result i32) (i32.const 1))
          (func (export "gx_alloc") (param i32) (result i32) (i32.const 4096))
          (func (export "gx_free") (param i32 i32))
          (func (export "gx_sources") (param i32 i32) (result i64)
            (if (i32.eq (memory.grow (i32.const 16384)) (i32.const -1))
              (then unreachable))
            (i64.const 0))
          (func (export "gx_compile") (param i32 i32 i32 i32) (result i64) (i64.const -1))
          (func (export "gx_last_error") (result i64) (i64.const 0)))"#,
    )
    .unwrap();
    let hash = sha256(&bytes);
    let request = b"any";
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    let mut err = [0 as c_char; 128];
    // SAFETY: every pointer is valid for its length.
    let rc = unsafe {
        gx_module_sources(
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
    assert_eq!(rc, GX_MODULE_ERR_MEMORY_LIMIT);
    assert!(ptr.is_null());
    // SAFETY: the library NUL-terminates the reason inside the buffer.
    let reason = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
    assert_eq!(
        reason,
        "module exceeded the memory limit of 1073741824 bytes"
    );
}
