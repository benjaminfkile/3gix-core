//! C ABI of the module host, declared in `include/gx_module_host.h`.
//!
//! Lets the hub run a module natively with the host clients use, to settle
//! a dispute (`client-compute.md` section 2.7), to compute frame records
//! (section 10.2), and to read a module's input list (section 10.4). Each
//! call admits the module under its expected hash ([`ModuleHost::load`],
//! with the most recently compiled module reused when the hash matches) and
//! runs `gx_compile` or `gx_sources` on request and tile bytes laid out as
//! in section 4 ([`gx_module_compile`], [`gx_module_sources`]), or for
//! module ABI version 2 (section 4.1) reports the version
//! ([`gx_module_abi_version`]) and runs `gx_inputs`, `gx_sources`, and
//! `gx_compile` with frame and input bytes ([`gx_module_inputs`],
//! [`gx_module_sources2`], [`gx_module_compile2`]), with the
//! default limits ([`crate::DEFAULT_MEMORY_LIMIT`],
//! [`crate::DEFAULT_TIME_LIMIT`]). The
//! output is the module's own bytes, not validated here: the caller checks
//! a section with the core library's `gx_validate`.
//!
//! Every function catches panics at the boundary and reports them as
//! [`GX_MODULE_ERR_PANIC`].

use std::ffi::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};

use gx_core::sha256::sha256;

use crate::{HostError, LoadedModule, ModuleHost};

/// An argument is unusable: a null pointer with a non-zero length, or a
/// null `module_hash32`, `out_ptr`, or `out_len`.
pub const GX_MODULE_ERR_ARGUMENT: i32 = -1;
/// The module bytes do not hash to `module_hash32`.
pub const GX_MODULE_ERR_HASH: i32 = -2;
/// The module is not admissible: it does not parse, imports something, uses
/// a forbidden feature or instruction, lacks a required export, or reports
/// an ABI version other than 1 or 2, or exports that do not match the version
/// it reports.
pub const GX_MODULE_ERR_REJECTED: i32 = -3;
/// The engine could not be built, or the module could not be compiled or
/// instantiated.
pub const GX_MODULE_ERR_ENGINE: i32 = -4;
/// The call ran past the time limit.
pub const GX_MODULE_ERR_TIME_LIMIT: i32 = -5;
/// The module asked for more memory than the limit.
pub const GX_MODULE_ERR_MEMORY_LIMIT: i32 = -6;
/// The module trapped.
pub const GX_MODULE_ERR_TRAP: i32 = -7;
/// The module returned -1; the reason is what its `gx_last_error` reported.
pub const GX_MODULE_ERR_MODULE_FAILED: i32 = -8;
/// The module broke the ABI: an out-of-bounds block, a failed allocation,
/// or a malformed source id list.
pub const GX_MODULE_ERR_ABI: i32 = -9;
/// The host panicked. A bug in the host, never in the module.
pub const GX_MODULE_ERR_PANIC: i32 = -10;
/// The call does not fit the module's ABI version: a version 1 call
/// ([`gx_module_compile`], [`gx_module_sources`]) on a version 2 module, or
/// frame records or inputs given to a version 1 module.
pub const GX_MODULE_ERR_VERSION: i32 = -11;

/// The C ABI error code of a host error.
pub fn error_code(e: &HostError) -> i32 {
    match e {
        HostError::HashMismatch { .. } => GX_MODULE_ERR_HASH,
        HostError::Invalid(_)
        | HostError::Import { .. }
        | HostError::ForbiddenOpcode(_)
        | HostError::Export(_)
        | HostError::AbiVersion(_) => GX_MODULE_ERR_REJECTED,
        HostError::Engine(_) => GX_MODULE_ERR_ENGINE,
        HostError::TimeLimit(_) => GX_MODULE_ERR_TIME_LIMIT,
        HostError::MemoryLimit(_) => GX_MODULE_ERR_MEMORY_LIMIT,
        HostError::Trap(_) => GX_MODULE_ERR_TRAP,
        HostError::ModuleFailed(_) => GX_MODULE_ERR_MODULE_FAILED,
        HostError::Version(_) => GX_MODULE_ERR_VERSION,
        // The raw calls neither encode tiles nor validate the section, so
        // these two are not returned by the C ABI; they map for totality.
        HostError::Input(_) => GX_MODULE_ERR_ARGUMENT,
        HostError::Abi(_) | HostError::Section(_) => GX_MODULE_ERR_ABI,
    }
}

/// A failure at the boundary: its code and reason.
struct Failure {
    code: i32,
    reason: String,
}

impl From<HostError> for Failure {
    fn from(e: HostError) -> Failure {
        Failure {
            code: error_code(&e),
            reason: e.to_string(),
        }
    }
}

fn argument(reason: &str) -> Failure {
    Failure {
        code: GX_MODULE_ERR_ARGUMENT,
        reason: reason.to_string(),
    }
}

/// The process-wide host with the default limits, built on first use. Its
/// engine and epoch ticker thread are shared by every call.
fn host() -> Result<&'static ModuleHost, Failure> {
    static HOST: OnceLock<Result<ModuleHost, HostError>> = OnceLock::new();
    HOST.get_or_init(ModuleHost::new)
        .as_ref()
        .map_err(|e| Failure::from(e.clone()))
}

/// Admits `bytes` under `hash` with [`ModuleHost::load`], reusing the
/// compiled module of the previous call when its bytes hash to the same
/// value. The hash is checked on every call; reuse only skips compiling the
/// same bytes again, so it never changes the output.
fn load(bytes: &[u8], hash: &[u8; 32]) -> Result<Arc<LoadedModule>, Failure> {
    static LAST: Mutex<Option<Arc<LoadedModule>>> = Mutex::new(None);
    if sha256(bytes) == *hash {
        let last = LAST.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(m) = last.as_ref().filter(|m| m.hash() == *hash) {
            return Ok(Arc::clone(m));
        }
    }
    let loaded = Arc::new(host()?.load(bytes, hash)?);
    *LAST.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::clone(&loaded));
    Ok(loaded)
}

/// Builds a slice from a C pointer and length. A null pointer with length 0
/// is the empty slice.
///
/// # Safety
///
/// A non-null `ptr` must be valid for `len` readable bytes for `'a`.
unsafe fn slice<'a>(ptr: *const u8, len: usize, name: &str) -> Result<&'a [u8], Failure> {
    if ptr.is_null() {
        return if len == 0 {
            Ok(&[])
        } else {
            Err(argument(&format!(
                "{name}: null pointer with non-zero length"
            )))
        };
    }
    // SAFETY: the caller guarantees `ptr` is valid for `len` bytes.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Writes `reason` as NUL-terminated UTF-8 into `buf`, truncated to at most
/// `cap - 1` bytes without splitting a UTF-8 sequence. Writes nothing for a
/// null `buf` or a zero `cap`.
///
/// # Safety
///
/// A non-null `buf` must be valid for `cap` writable bytes.
unsafe fn write_reason(buf: *mut c_char, cap: usize, reason: &str) {
    if buf.is_null() || cap == 0 {
        return;
    }
    let mut n = reason.len().min(cap - 1);
    while !reason.is_char_boundary(n) {
        n -= 1;
    }
    // SAFETY: `n < cap` and `buf` is valid for `cap` bytes; `reason` is a
    // Rust string, so the regions cannot overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(reason.as_ptr(), buf.cast::<u8>(), n);
        *buf.add(n) = 0;
    }
}

/// Runs `body` with panics caught, hands its bytes to the caller through
/// `out_ptr` and `out_len`, and reports a failure through the return code
/// and `err_buf`.
///
/// # Safety
///
/// `out_ptr` and `out_len`, when non-null, must be valid for one write each;
/// `err_buf` as in [`write_reason`].
unsafe fn boundary(
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    err_buf: *mut c_char,
    err_len: usize,
    body: impl FnOnce() -> Result<Vec<u8>, Failure>,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return Err(argument("out_ptr or out_len is a null pointer"));
        }
        body()
    }))
    .unwrap_or_else(|_| {
        Err(Failure {
            code: GX_MODULE_ERR_PANIC,
            reason: "the module host panicked".to_string(),
        })
    });
    match result {
        Ok(bytes) => {
            let len = bytes.len();
            let ptr = if len == 0 {
                std::ptr::null_mut()
            } else {
                Box::into_raw(bytes.into_boxed_slice()).cast::<u8>()
            };
            // SAFETY: both were checked non-null above and the caller
            // guarantees they are valid for one write.
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            0
        }
        Err(f) => {
            // SAFETY: the caller guarantees `err_buf` is valid for `err_len`
            // bytes when it is non-null.
            unsafe { write_reason(err_buf, err_len, &f.reason) };
            f.code
        }
    }
}

/// Reads the 32-byte expected hash.
///
/// # Safety
///
/// A non-null `hash` must be valid for 32 readable bytes.
unsafe fn hash32(hash: *const u8) -> Result<[u8; 32], Failure> {
    if hash.is_null() {
        return Err(argument("module_hash32: null pointer"));
    }
    let mut out = [0u8; 32];
    // SAFETY: the caller guarantees `hash` is valid for 32 bytes.
    unsafe { std::ptr::copy_nonoverlapping(hash, out.as_mut_ptr(), 32) };
    Ok(out)
}

/// Loads `module` under `module_hash32` and runs `gx_compile` with the
/// request and tile bytes exactly as given (`client-compute.md` section 4),
/// under the default limits: 1 GiB of memory and 60 s.
///
/// Returns 0 and the section bytes in `*out_ptr` and `*out_len`, which the
/// caller frees with [`gx_module_free`]; an empty output is a null pointer
/// with length 0. On failure returns a negative `GX_MODULE_ERR_*` code,
/// leaves the outputs untouched, and writes the reason into `err_buf` as
/// NUL-terminated UTF-8 truncated to `err_len - 1` bytes. The bytes are the
/// module's own: validate them with `gx_validate`.
///
/// # Safety
///
/// Each non-null input pointer must be valid for its length in readable
/// bytes (`module_hash32` for 32), `out_ptr` and `out_len` for one write
/// each, and a non-null `err_buf` for `err_len` writable bytes.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn gx_module_compile(
    module: *const u8,
    module_len: usize,
    module_hash32: *const u8,
    request: *const u8,
    request_len: usize,
    tiles: *const u8,
    tiles_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    err_buf: *mut c_char,
    err_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees every pointer as documented.
    unsafe {
        boundary(out_ptr, out_len, err_buf, err_len, || {
            let module = slice(module, module_len, "module")?;
            let hash = hash32(module_hash32)?;
            let request = slice(request, request_len, "request")?;
            let tiles = slice(tiles, tiles_len, "tiles")?;
            let loaded = load(module, &hash)?;
            Ok(loaded.compile_raw(request, tiles)?)
        })
    }
}

/// Loads `module` under `module_hash32` and runs `gx_sources` with the
/// request bytes exactly as given (`client-compute.md` section 4), under the
/// default limits.
///
/// Returns 0 and the source id list in `*out_ptr` and `*out_len`: `u64 LE`
/// ids, eight bytes each, ascending and unique (checked here), empty as a
/// null pointer with length 0. The caller frees it with [`gx_module_free`].
/// Failures as for [`gx_module_compile`].
///
/// # Safety
///
/// As for [`gx_module_compile`].
#[no_mangle]
pub unsafe extern "C" fn gx_module_sources(
    module: *const u8,
    module_len: usize,
    module_hash32: *const u8,
    request: *const u8,
    request_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    err_buf: *mut c_char,
    err_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees every pointer as documented.
    unsafe {
        boundary(out_ptr, out_len, err_buf, err_len, || {
            let module = slice(module, module_len, "module")?;
            let hash = hash32(module_hash32)?;
            let request = slice(request, request_len, "request")?;
            let loaded = load(module, &hash)?;
            let ids = loaded.sources_raw(request)?;
            gx_core::module_io::decode_source_ids(&ids)
                .map_err(|e| HostError::Abi(e.to_string()))?;
            Ok(ids)
        })
    }
}

/// Loads `module` under `module_hash32` and returns its ABI version, what
/// its `gx_abi_version` returned: 1, or 2 for a module that reads inputs
/// (`client-compute.md` section 4.1). On failure returns a negative
/// `GX_MODULE_ERR_*` code and writes the reason into `err_buf` as for
/// [`gx_module_compile`].
///
/// # Safety
///
/// Each non-null pointer must be valid for its length (`module_hash32` for
/// 32 readable bytes), and a non-null `err_buf` for `err_len` writable
/// bytes.
#[no_mangle]
pub unsafe extern "C" fn gx_module_abi_version(
    module: *const u8,
    module_len: usize,
    module_hash32: *const u8,
    err_buf: *mut c_char,
    err_len: usize,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller guarantees every pointer as documented.
        let module = unsafe { slice(module, module_len, "module") }?;
        // SAFETY: as above.
        let hash = unsafe { hash32(module_hash32) }?;
        Ok(load(module, &hash)?.abi_version())
    }))
    .unwrap_or_else(|_| {
        Err(Failure {
            code: GX_MODULE_ERR_PANIC,
            reason: "the module host panicked".to_string(),
        })
    });
    match result {
        Ok(v) => v,
        Err(f) => {
            // SAFETY: the caller guarantees `err_buf` is valid for `err_len`
            // bytes when it is non-null.
            unsafe { write_reason(err_buf, err_len, &f.reason) };
            f.code
        }
    }
}

/// Loads `module` under `module_hash32` and runs `gx_inputs` with the
/// request and frame bytes exactly as given (`client-compute.md` section
/// 4.1), under the default limits.
///
/// Returns 0 and the raw input list in `*out_ptr` and `*out_len`, checked
/// with the library's input list decoder (sorted by slot, then key bytes,
/// unique, at most 64 entries, every key a chunk key). The caller frees it
/// with [`gx_module_free`]. A version 1 module reads no inputs: its list is
/// the four bytes of a zero count, and it must be given no frame records
/// (a zero count or no bytes at all). Failures as for
/// [`gx_module_compile`].
///
/// # Safety
///
/// As for [`gx_module_compile`].
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn gx_module_inputs(
    module: *const u8,
    module_len: usize,
    module_hash32: *const u8,
    request: *const u8,
    request_len: usize,
    frames: *const u8,
    frames_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    err_buf: *mut c_char,
    err_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees every pointer as documented.
    unsafe {
        boundary(out_ptr, out_len, err_buf, err_len, || {
            let module = slice(module, module_len, "module")?;
            let hash = hash32(module_hash32)?;
            let request = slice(request, request_len, "request")?;
            let frames = slice(frames, frames_len, "frames")?;
            let loaded = load(module, &hash)?;
            Ok(loaded.inputs_raw(request, frames)?)
        })
    }
}

/// Loads `module` under `module_hash32` and runs the version 2 `gx_sources`
/// with the request and frame bytes exactly as given (`client-compute.md`
/// section 4.1), under the default limits. A version 1 module runs its
/// version 1 `gx_sources` and must be given no frame records.
///
/// Output as for [`gx_module_sources`]; failures as for
/// [`gx_module_compile`].
///
/// # Safety
///
/// As for [`gx_module_compile`].
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn gx_module_sources2(
    module: *const u8,
    module_len: usize,
    module_hash32: *const u8,
    request: *const u8,
    request_len: usize,
    frames: *const u8,
    frames_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    err_buf: *mut c_char,
    err_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees every pointer as documented.
    unsafe {
        boundary(out_ptr, out_len, err_buf, err_len, || {
            let module = slice(module, module_len, "module")?;
            let hash = hash32(module_hash32)?;
            let request = slice(request, request_len, "request")?;
            let frames = slice(frames, frames_len, "frames")?;
            let loaded = load(module, &hash)?;
            let ids = loaded.sources_v2_raw(request, frames)?;
            gx_core::module_io::decode_source_ids(&ids)
                .map_err(|e| HostError::Abi(e.to_string()))?;
            Ok(ids)
        })
    }
}

/// Loads `module` under `module_hash32` and runs the version 2 `gx_compile`
/// with the request, tile, and input bytes exactly as given
/// (`client-compute.md` section 4.1), under the default limits; the tiles
/// and inputs together count against the memory limit. A version 1 module
/// runs its version 1 `gx_compile` and must be given no inputs.
///
/// Output and failures as for [`gx_module_compile`].
///
/// # Safety
///
/// As for [`gx_module_compile`].
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn gx_module_compile2(
    module: *const u8,
    module_len: usize,
    module_hash32: *const u8,
    request: *const u8,
    request_len: usize,
    tiles: *const u8,
    tiles_len: usize,
    inputs: *const u8,
    inputs_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    err_buf: *mut c_char,
    err_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees every pointer as documented.
    unsafe {
        boundary(out_ptr, out_len, err_buf, err_len, || {
            let module = slice(module, module_len, "module")?;
            let hash = hash32(module_hash32)?;
            let request = slice(request, request_len, "request")?;
            let tiles = slice(tiles, tiles_len, "tiles")?;
            let inputs = slice(inputs, inputs_len, "inputs")?;
            let loaded = load(module, &hash)?;
            Ok(loaded.compile_v2_raw(request, tiles, inputs)?)
        })
    }
}

/// Frees a buffer returned by [`gx_module_compile`], [`gx_module_sources`],
/// [`gx_module_inputs`], [`gx_module_sources2`], or [`gx_module_compile2`].
/// A null `ptr` is ignored.
///
/// # Safety
///
/// A non-null `ptr` must be a buffer this library returned with exactly
/// this `len`, not already freed.
#[no_mangle]
pub unsafe extern "C" fn gx_module_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let _ = catch_unwind(|| {
        // SAFETY: the caller guarantees `ptr` and `len` came from a boxed
        // slice this library leaked.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
    });
}
