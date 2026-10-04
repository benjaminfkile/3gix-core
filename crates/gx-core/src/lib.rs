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
//! - [`error`]: [`error::ValidationError`] and the stable numeric code table
//!   for every section 4 rule (`docs/errors.md`).
//!
//! The C ABI validator is still a placeholder.

pub mod error;
pub mod key;
pub mod matter;
pub mod units;

/// Version of the matter format this library reads and writes.
pub const FORMAT_VERSION: u32 = 1;

const NOT_IMPLEMENTED: &[u8] = b"validator not implemented";

/// Returns the matter format version, [`FORMAT_VERSION`].
#[no_mangle]
pub extern "C" fn gx_format_version() -> u32 {
    FORMAT_VERSION
}

/// Validates a section of matter bytes stored under a chunk key.
///
/// Placeholder: the real validator replaces this body. For now it never reads
/// `key` or `bytes`, always returns `1`, and writes the NUL-terminated UTF-8
/// reason `validator not implemented` into `err_buf`, truncated to `err_cap`
/// bytes including the terminator.
///
/// # Safety
///
/// `key` and `bytes` are never dereferenced, so any value is accepted. If
/// `err_buf` is non-null and `err_cap` is non-zero, `err_buf` must point to at
/// least `err_cap` writable bytes. A null `err_buf` or a zero `err_cap` writes
/// nothing.
#[no_mangle]
pub unsafe extern "C" fn gx_validate(
    key: *const u8,
    key_len: usize,
    bytes: *const u8,
    bytes_len: usize,
    err_buf: *mut u8,
    err_cap: usize,
) -> i32 {
    let _ = (key, key_len, bytes, bytes_len);
    // SAFETY: the caller guarantees `err_buf` is valid for `err_cap` bytes
    // when it is non-null and `err_cap` is non-zero.
    unsafe { write_reason(err_buf, err_cap, NOT_IMPLEMENTED) };
    1
}

/// Copies `reason` into `buf`, truncated to leave room for a NUL terminator.
///
/// # Safety
///
/// If `buf` is non-null and `cap` is non-zero, `buf` must be valid for `cap`
/// writable bytes.
unsafe fn write_reason(buf: *mut u8, cap: usize, reason: &[u8]) {
    if buf.is_null() || cap == 0 {
        return;
    }
    let n = reason.len().min(cap - 1);
    // SAFETY: `n < cap` and `buf` is valid for `cap` bytes; `reason` is a
    // separate Rust slice so the regions cannot overlap.
    unsafe {
        core::ptr::copy_nonoverlapping(reason.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use wasm_bindgen::prelude::*;

    /// Returns the matter format version.
    #[wasm_bindgen]
    pub fn format_version() -> u32 {
        super::FORMAT_VERSION
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::format_version;

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr;

    #[test]
    fn format_version_is_one() {
        assert_eq!(gx_format_version(), 1);
    }

    #[test]
    fn validate_null_inputs_writes_reason() {
        let mut buf = [0xffu8; 64];
        let rc =
            unsafe { gx_validate(ptr::null(), 0, ptr::null(), 0, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(rc, 1);
        let end = buf.iter().position(|&b| b == 0).expect("NUL terminator");
        assert_eq!(&buf[..end], NOT_IMPLEMENTED);
    }

    #[test]
    fn validate_all_null_is_sound() {
        let rc = unsafe { gx_validate(ptr::null(), 0, ptr::null(), 0, ptr::null_mut(), 0) };
        assert_eq!(rc, 1);
    }

    #[test]
    fn validate_truncates_to_small_cap() {
        let mut buf = [0xffu8; 8];
        let rc = unsafe { gx_validate(ptr::null(), 0, ptr::null(), 0, buf.as_mut_ptr(), 4) };
        assert_eq!(rc, 1);
        assert_eq!(&buf[..4], b"val\0");
        assert!(buf[4..].iter().all(|&b| b == 0xff));
    }
}
