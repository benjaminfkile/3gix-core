//! Core library: matter format, units, laws, and the C ABI shim.
//!
//! Implemented so far:
//!
//! - [`units`]: branded SI quantities and the [`units::Vec3`] and
//!   [`units::Quat`] math types (`matter-format.md` section 1, principles).
//! - [`key`]: chunk key parsing and printing, and cell geometry
//!   (`matter-format.md` section 2, chunk keys, and section 3.1, cell
//!   geometry).
//! - [`matter`]: matter section encode, decode, validate, and composite
//!   (`matter-format.md` section 3, matter section, with 3.5 compositing, and
//!   section 4, validation, for matter sections).
//! - [`registry`]: frame registry encode, decode, and validate, and the
//!   union of a build's registries as a [`registry::FrameTree`]
//!   (`matter-format.md` section 5, frame registry: header 5.1, frame
//!   records and union rules 5.2, empty registry 5.3).
//! - [`frames`]: reference frames with a floating origin, the current state
//!   of every frame, and the transforms between frames (`space-model.md`
//!   section 5, numerical devices, and section 6, time).
//! - [`gravity`]: Newtonian gravity from point masses and coarse density
//!   grids (`space-model.md` section 5, gravity between all frames, and
//!   section 6, integration of every massive frame).
//! - [`integrate`]: a fixed-step symplectic N-body integrator over the frame
//!   registry, velocity Verlet or fourth order Yoshida (`space-model.md`
//!   sections 5 and 6).
//! - [`radiance`]: blackbody radiance per wavelength and per band from
//!   temperature, with the three fixed bands of format version 1
//!   (`space-model.md` sections 1 and 2, `matter-format.md` section 3.3,
//!   derived quantities).
//! - [`emission`]: the hot matter of a section summarized as one emitter
//!   (`space-model.md` sections 2 and 8, `matter-format.md` section 3.3).
//! - [`extinction`]: extinction coefficient, transmittance, and optical
//!   depth through a section's grid (`space-model.md` sections 2 and 8,
//!   `matter-format.md` section 3.3).
//! - [`lod`]: octree cell selection for a camera (`space-model.md` sections
//!   2 and 5, `matter-format.md` section 3.1).
//! - [`container`]: the hub container that holds every section of one chunk,
//!   decoded and bounds checked (`matter-format.md` section 6).
//! - [`validate()`]: the top-level validator that dispatches between matter
//!   and registry by key (`matter-format.md` section 4, including step 3).
//! - [`error`]: [`error::ValidationError`] and the stable numeric code table
//!   for every section 4, 5, and 6 rule (`docs/errors.md`).
//! - The C ABI (`matter-format.md` section 7): [`gx_format_version`],
//!   [`gx_validate`], and [`gx_error_name`], declared in
//!   `include/gx_core.h`.
//! - WebAssembly exports, on `wasm32` only: `format_version`, `validate`,
//!   and `decode_chunk_mass`, built with `wasm-bindgen`.

use core::ffi::c_char;

pub mod container;
mod detmath;
pub mod emission;
pub mod error;
pub mod extinction;
pub mod frames;
pub mod gravity;
pub mod integrate;
pub mod key;
pub mod lod;
pub mod matter;
pub mod radiance;
pub mod registry;
pub mod units;
pub mod validate;

pub use validate::validate;

use error::{codes, ValidationError};

/// Version of the matter format this library reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Returns the matter format version, [`FORMAT_VERSION`].
#[no_mangle]
pub extern "C" fn gx_format_version() -> u32 {
    FORMAT_VERSION
}

/// Builds a slice from a C pointer and length. A null pointer with length 0
/// is the empty slice; a null pointer with a non-zero length is `None`.
///
/// # Safety
///
/// A non-null `ptr` must be valid for `len` readable bytes for `'a`.
unsafe fn slice<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    if ptr.is_null() {
        return (len == 0).then_some(&[][..]);
    }
    // SAFETY: the caller guarantees `ptr` is valid for `len` bytes.
    Some(unsafe { core::slice::from_raw_parts(ptr, len) })
}

/// The C ABI validation, on safe inputs: null pointer and UTF-8 checks, then
/// [`validate()`].
fn validate_ffi(key: Option<&[u8]>, bytes: Option<&[u8]>) -> Result<(), ValidationError> {
    let key = key.ok_or_else(|| {
        ValidationError::new(
            codes::KEY_POINTER_NULL,
            "key: null pointer with non-zero key_len",
        )
    })?;
    let key = core::str::from_utf8(key).map_err(|e| {
        ValidationError::new(
            codes::KEY_MALFORMED,
            format!("key: not UTF-8 at byte {}", e.valid_up_to()),
        )
    })?;
    let bytes = bytes.ok_or_else(|| {
        ValidationError::new(
            codes::BYTES_POINTER_NULL,
            "bytes: null pointer with non-zero bytes_len",
        )
    })?;
    validate(key, bytes)
}

/// Validates one submitted section, matter or registry chosen by key
/// (`matter-format.md` sections 4 and 7).
///
/// Returns 0 on success, or the error code from `docs/errors.md` on
/// failure. On failure, writes the reason as NUL-terminated UTF-8 into
/// `err_buf`, truncated to at most `err_cap - 1` bytes plus the NUL, never
/// splitting a UTF-8 sequence. With a null `err_buf` or a zero `err_cap`,
/// writes nothing and still returns the code. On success `err_buf` is not
/// touched.
///
/// A null `key` with a non-zero `key_len` fails with 207, a `key` that is
/// not UTF-8 or not a chunk key with 206, and a null `bytes` with a non-zero
/// `bytes_len` with 106. A null pointer with length 0 is an empty input.
///
/// # Safety
///
/// A non-null `key` must be valid for `key_len` readable bytes and a
/// non-null `bytes` for `bytes_len` readable bytes. If `err_buf` is non-null
/// and `err_cap` is non-zero, `err_buf` must point to at least `err_cap`
/// writable bytes that do not overlap the inputs.
#[no_mangle]
pub unsafe extern "C" fn gx_validate(
    key: *const u8,
    key_len: usize,
    bytes: *const u8,
    bytes_len: usize,
    err_buf: *mut c_char,
    err_cap: usize,
) -> i32 {
    // SAFETY: the caller guarantees each non-null pointer is valid for its
    // length.
    let (key, bytes) = unsafe { (slice(key, key_len), slice(bytes, bytes_len)) };
    match validate_ffi(key, bytes) {
        Ok(()) => 0,
        Err(e) => {
            // SAFETY: the caller guarantees `err_buf` is valid for `err_cap`
            // bytes when it is non-null and `err_cap` is non-zero.
            unsafe { write_reason(err_buf.cast::<u8>(), err_cap, &e.reason) };
            i32::from(e.code)
        }
    }
}

/// Returns the short name of an error code, such as `BAD_MAGIC` for 102, as
/// a pointer to a static NUL-terminated ASCII string. Unknown codes,
/// including 0, give the empty string. Never returns null.
#[no_mangle]
pub extern "C" fn gx_error_name(code: i32) -> *const c_char {
    error::code_name_c(i64::from(code))
        .as_ptr()
        .cast::<c_char>()
}

/// The longest prefix of `s` that fits in `max` bytes without splitting a
/// UTF-8 sequence.
fn utf8_prefix(s: &str, max: usize) -> &[u8] {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s.as_bytes()[..n]
}

/// Copies `reason` into `buf`, truncated at a UTF-8 boundary to leave room
/// for a NUL terminator.
///
/// # Safety
///
/// If `buf` is non-null and `cap` is non-zero, `buf` must be valid for `cap`
/// writable bytes.
unsafe fn write_reason(buf: *mut u8, cap: usize, reason: &str) {
    if buf.is_null() || cap == 0 {
        return;
    }
    let src = utf8_prefix(reason, cap - 1);
    let n = src.len();
    // SAFETY: `n < cap` and `buf` is valid for `cap` bytes; `src` is a
    // separate Rust string so the regions cannot overlap.
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use wasm_bindgen::prelude::*;

    use crate::error::ValidationError;

    /// Formats an error as `CODE: REASON` for JavaScript.
    fn js(e: ValidationError) -> JsError {
        JsError::new(&format!("{}: {}", e.code, e.reason))
    }

    /// Returns the matter format version.
    #[wasm_bindgen]
    pub fn format_version() -> u32 {
        super::FORMAT_VERSION
    }

    /// Validates one submitted section under a chunk key, matter or registry
    /// chosen by key. Throws an `Error` with message `CODE: REASON` on
    /// failure.
    #[wasm_bindgen]
    pub fn validate(key: &str, bytes: &[u8]) -> Result<(), JsError> {
        crate::validate(key, bytes).map_err(js)
    }

    /// Total mass in kilograms of every matter section in a hub container
    /// stored under a cell key. Throws an `Error` with message
    /// `CODE: REASON` on failure.
    #[wasm_bindgen]
    pub fn decode_chunk_mass(key: &str, bytes: &[u8]) -> Result<f64, JsError> {
        crate::container::chunk_mass(key, bytes)
            .map(|m| m.value())
            .map_err(js)
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{decode_chunk_mass, format_version};

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr;
    use std::ffi::CStr;

    fn call(key: &[u8], bytes: &[u8], buf: &mut [u8], cap: usize) -> i32 {
        assert!(cap <= buf.len());
        unsafe {
            gx_validate(
                key.as_ptr(),
                key.len(),
                bytes.as_ptr(),
                bytes.len(),
                buf.as_mut_ptr().cast(),
                cap,
            )
        }
    }

    fn registry_bytes() -> Vec<u8> {
        registry::encode(&registry::Registry::empty(units::Seconds::new(1.0)).unwrap())
    }

    fn reason(buf: &[u8]) -> &str {
        CStr::from_bytes_until_nul(buf).unwrap().to_str().unwrap()
    }

    #[test]
    fn format_version_is_one() {
        assert_eq!(gx_format_version(), 1);
    }

    #[test]
    fn success_returns_zero_and_leaves_buffer() {
        let mut buf = [0xffu8; 16];
        assert_eq!(call(b"registry", &registry_bytes(), &mut buf, 16), 0);
        assert!(buf.iter().all(|&b| b == 0xff));
    }

    #[test]
    fn failure_returns_code_and_reason() {
        let mut buf = [0xffu8; 128];
        let rc = call(b"registry", b"3GMS", &mut buf, 128);
        assert_eq!(rc, i32::from(codes::REGISTRY_HEADER_TOO_SHORT));
        let want = validate("registry", b"3GMS").unwrap_err().reason;
        assert_eq!(reason(&buf), want);
    }

    #[test]
    fn null_inputs() {
        let mut buf = [0u8; 128];
        let p = buf.as_mut_ptr().cast();
        let rc = unsafe { gx_validate(ptr::null(), 0, ptr::null(), 0, p, 128) };
        assert_eq!(rc, i32::from(codes::KEY_MALFORMED));
        let rc = unsafe { gx_validate(ptr::null(), 3, ptr::null(), 0, p, 128) };
        assert_eq!(rc, i32::from(codes::KEY_POINTER_NULL));
        let k = b"registry";
        let rc = unsafe { gx_validate(k.as_ptr(), k.len(), ptr::null(), 9, p, 128) };
        assert_eq!(rc, i32::from(codes::BYTES_POINTER_NULL));
        let rc = unsafe { gx_validate(k.as_ptr(), k.len(), ptr::null(), 0, p, 128) };
        assert_eq!(rc, i32::from(codes::REGISTRY_HEADER_TOO_SHORT));
        let rc = unsafe { gx_validate(ptr::null(), 0, ptr::null(), 0, ptr::null_mut(), 0) };
        assert_eq!(rc, i32::from(codes::KEY_MALFORMED));
        let rc = unsafe { gx_validate(ptr::null(), 0, ptr::null(), 0, ptr::null_mut(), 64) };
        assert_eq!(rc, i32::from(codes::KEY_MALFORMED));
    }

    #[test]
    fn non_utf8_key() {
        let mut buf = [0u8; 64];
        let rc = call(b"1-0-0-0-\xff", b"", &mut buf, 64);
        assert_eq!(rc, i32::from(codes::KEY_MALFORMED));
        assert_eq!(reason(&buf), "key: not UTF-8 at byte 8");
    }

    #[test]
    fn zero_cap_writes_nothing() {
        let mut buf = [0xffu8; 8];
        let rc = call(b"registry", b"", &mut buf, 0);
        assert_eq!(rc, i32::from(codes::REGISTRY_HEADER_TOO_SHORT));
        assert!(buf.iter().all(|&b| b == 0xff));
    }

    #[test]
    fn truncates_to_cap() {
        let full = validate("registry", b"").unwrap_err().reason;
        assert!(full.is_ascii() && full.len() > 4);
        let mut buf = [0xffu8; 8];
        call(b"registry", b"", &mut buf, 4);
        assert_eq!(
            &buf[..4],
            &[
                full.as_bytes()[0],
                full.as_bytes()[1],
                full.as_bytes()[2],
                0
            ]
        );
        assert!(buf[4..].iter().all(|&b| b == 0xff));
        call(b"registry", b"", &mut buf, 1);
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn truncation_keeps_utf8_whole() {
        // Two-byte, three-byte, and four-byte sequences.
        let s = "ab\u{e9}\u{20ac}\u{1f600}";
        for cap in 1..=s.len() + 2 {
            let mut buf = vec![0xffu8; cap];
            unsafe { write_reason(buf.as_mut_ptr(), cap, s) };
            let got = reason(&buf);
            assert!(s.starts_with(got));
            assert!(got.len() < cap);
            // The longest whole prefix that fits.
            let next = s[got.len()..].chars().next();
            if let Some(c) = next {
                assert!(got.len() + c.len_utf8() > cap - 1, "cap {cap}");
            }
        }
    }

    #[test]
    fn error_names() {
        let name = |c| {
            unsafe { CStr::from_ptr(gx_error_name(c)) }
                .to_str()
                .unwrap()
        };
        assert_eq!(name(102), "BAD_MAGIC");
        assert_eq!(name(713), "CONTAINER_TRAILING_BYTES");
        assert_eq!(name(0), "");
        assert_eq!(name(-102), "");
        assert_eq!(name(1_000_000), "");
        for &(c, n) in codes::ALL {
            assert_eq!(name(i32::from(c)), n);
        }
    }
}
