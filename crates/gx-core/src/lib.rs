//! Core library: matter format, units, laws, and the C ABI shim.
//!
//! Implemented so far:
//!
//! - [`units`]: branded SI quantities and the [`units::Vec3`] and
//!   [`units::Quat`] math types (`matter-format.md` section 1, principles).
//! - [`key`]: chunk key parsing and printing, and cell geometry
//!   (`matter-format.md` section 2, chunk keys, and section 3.1, cell
//!   geometry).
//! - [`matter`]: matter section format version 3 encode, decode, validate,
//!   and composite, with the occupancy channel that places surfaces
//!   (`matter-format.md` section 3, matter section, with 3.3 sample block,
//!   3.5 compositing, and section 4, validation, for matter sections).
//!   Versions 1 and 2 are retired and rejected.
//! - [`registry`]: frame registry format version 2 encode, decode, and
//!   validate, and the union of a build's registries as a
//!   [`registry::FrameTree`] (`matter-format.md` section 5, frame registry:
//!   header 5.1, frame records and union rules 5.2, empty registry 5.3).
//! - [`frame_record`]: frame records, the constant part of one frame's
//!   registry record under the key `frame-{frameId}`: encode, decode,
//!   validate, and agreement with the registry (`matter-format.md` section
//!   11).
//! - [`frames`]: reference frames with a floating origin, the current state
//!   of every frame, and the transforms between frames (`space-model.md`
//!   section 5, numerical devices, and section 6, time).
//! - [`gravity`]: first post-Newtonian (Einstein-Infeld-Hoffmann) gravity
//!   between massive frames, the oblateness force, and Newtonian gravity
//!   from point masses and coarse density grids (`space-model.md` section
//!   6.1).
//! - [`rotation`]: Euler's equations under the torques on a frame's shape,
//!   and constant spin for frames without moments (`space-model.md` section
//!   6.2).
//! - [`integrate`]: the fixed-step N-body and rotation integrator over the
//!   frame registry: symplectic Newtonian schemes, and the default scheme
//!   with the relativistic, oblateness, and torque terms as a perturbation
//!   (`space-model.md` sections 6.1, 6.2, and 7).
//! - [`history`]: checkpoints of the frame system for jumps back in time
//!   and retarded states (`space-model.md` sections 6.4 and 7 item 6).
//! - [`light`]: the light-time equation solved against a history: the
//!   retarded time and state a frame is drawn at, and the retarded position
//!   a frame is lit from (`space-model.md` section 6.4).
//! - [`radiance`]: blackbody radiance per wavelength and per band from
//!   temperature, with the three fixed bands of the matter format
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
//! - [`sha256`]: the SHA-256 every hash and address uses, with lowercase hex
//!   (`client-compute.md` sections 2.1, 3, and 5).
//! - [`address`]: section addresses and span indices (`client-compute.md`
//!   sections 2.4 and 3).
//! - [`manifest`]: source manifests, encode, decode, validate, hash, and
//!   look up (`client-compute.md` section 5.1).
//! - [`epochs`]: epoch indexes, encode, decode, validate, hash, and the
//!   epoch rule that picks a span's manifest (`client-compute.md` section
//!   5.4).
//! - [`module_io`]: module request bytes, tile bytes, and the source id
//!   list (`client-compute.md` section 4).
//! - [`timescale`]: UTC milliseconds to seconds of Barycentric Dynamical Time
//!   since J2000 and back (`space-model.md` section 7 item 5).
//! - The C ABI (`matter-format.md` sections 7 and 11.4, `client-compute.md`
//!   sections 3, 5.1, and 5.4): [`gx_format_version`], [`gx_validate`],
//!   [`gx_error_name`], [`gx_section_address`], [`gx_manifest_validate`],
//!   [`gx_epochs_validate`], [`gx_epochs_manifest`],
//!   [`gx_frame_record_agrees`], and the layer input functions of
//!   `client-compute.md` sections 3, 4.1, 10.3, 10.4, and 10.5:
//!   [`gx_layer_identity`], [`gx_section_address_v2`],
//!   [`gx_frames_slot_value`], [`gx_inputs_sha256`], [`gx_input_span_index`],
//!   [`gx_input_list_validate`], and [`gx_input_list_entry`], declared in
//!   `include/gx_core.h`.
//! - WebAssembly exports, on `wasm32` only: `format_version`, `validate`,
//!   `decode_chunk_mass`, `section_address`, `manifest_validate`,
//!   `epochs_validate`, `epochs_manifest`,
//!   `layer_identity`, `section_address_v2`, `frames_slot_value`,
//!   `inputs_sha256`, `input_span_index`, `input_list_validate`,
//!   `utc_ms_to_tdb_seconds`, `tdb_seconds_to_utc_ms`,
//!   `integrate_registry`, and `light_retarded_state`, built with
//!   `wasm-bindgen`.
//! - [`detmath`]: deterministic `exp`, `expm1`, `sin`, `cos`, and `tan`,
//!   public so compiler modules compute bitwise identical values on every
//!   platform (`space-model.md` section 1 rule 7).
//!
//! The C ABI symbols and the WebAssembly exports exist only with the
//! default `exports` feature. Without it the functions remain callable from
//! Rust but no unmangled symbol is emitted, which is how compiler modules
//! link the library (`client-compute.md` section 4: a module has no imports).

use core::ffi::c_char;

pub mod address;
pub mod container;
pub mod detmath;
pub mod emission;
pub mod epochs;
pub mod error;
pub mod extinction;
pub mod frame_record;
pub mod frames;
pub mod gravity;
pub mod history;
pub mod integrate;
pub mod key;
pub mod light;
pub mod lod;
pub mod manifest;
pub mod matter;
pub mod module_io;
pub mod radiance;
pub mod registry;
pub mod rotation;
pub mod sha256;
pub mod surface;
pub mod timescale;
pub mod units;
pub mod validate;

pub use validate::validate;

use error::{codes, ValidationError};

/// Version of the matter section format this library reads and writes, 3
/// ([`matter::VERSION`]). The frame registry has its own version,
/// [`registry::VERSION`] (2).
pub const FORMAT_VERSION: u32 = matter::VERSION as u32;

/// Returns the matter format version, [`FORMAT_VERSION`].
#[cfg_attr(feature = "exports", no_mangle)]
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
#[cfg_attr(feature = "exports", no_mangle)]
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

/// A chunk key from the C ABI: 207 for a null pointer with a non-zero
/// length, 206 for bytes that are not UTF-8 or not a chunk key.
fn parse_key_ffi(key: Option<&[u8]>) -> Result<key::ChunkKey, ValidationError> {
    let key = key.ok_or_else(|| {
        ValidationError::new(
            codes::KEY_POINTER_NULL,
            "key: null pointer with non-zero key_len",
        )
    })?;
    core::str::from_utf8(key)
        .map_err(|e| {
            ValidationError::new(
                codes::KEY_MALFORMED,
                format!("key: not UTF-8 at byte {}", e.valid_up_to()),
            )
        })?
        .parse::<key::ChunkKey>()
        .map_err(|e| ValidationError::new(codes::KEY_MALFORMED, format!("key: {e}")))
}

/// The C ABI section address, on safe inputs: null pointer and key checks,
/// then [`address::section_address`].
fn section_address_ffi(
    module: Option<&[u8; 32]>,
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: Option<&[u8]>,
) -> Result<[u8; 32], ValidationError> {
    let module = module.ok_or_else(|| {
        ValidationError::new(codes::ADDRESS_POINTER_NULL, "module32: null pointer")
    })?;
    let key = parse_key_ffi(key)?;
    Ok(address::section_address(
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
        &key,
    ))
}

/// Computes a section address (`client-compute.md` section 3) and writes its
/// 32 bytes to `out32`.
///
/// `module32` is the module's SHA-256, `manifest32_or_null` the source
/// manifest's SHA-256 or null for none, `origin_utc_ms` the space's origin or
/// 0 when the layer's `usesOrigin` is false, `span_seconds` and `span_index`
/// the layer's span length and the section's span, and `key` the chunk key as
/// UTF-8 of `key_len` bytes, not NUL terminated.
///
/// Returns 0 on success. Returns 962 for a null `module32` or `out32`, 207
/// for a null `key` with a non-zero `key_len`, and 206 for a key that is not
/// UTF-8 or not a chunk key. On failure `out32` is not touched.
///
/// # Safety
///
/// A non-null `module32` and `manifest32_or_null` must each be valid for 32
/// readable bytes, a non-null `key` for `key_len` readable bytes, and a
/// non-null `out32` for 32 writable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_section_address(
    module32: *const u8,
    manifest32_or_null: *const u8,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    key: *const u8,
    key_len: usize,
    out32: *mut u8,
) -> i32 {
    if out32.is_null() {
        return i32::from(codes::ADDRESS_POINTER_NULL);
    }
    // SAFETY: the caller guarantees each non-null pointer is valid for its
    // length, 32 bytes for the hashes.
    let (module, manifest, key) = unsafe {
        (
            module32.cast::<[u8; 32]>().as_ref(),
            manifest32_or_null.cast::<[u8; 32]>().as_ref(),
            slice(key, key_len),
        )
    };
    match section_address_ffi(
        module,
        manifest,
        origin_utc_ms,
        span_seconds,
        span_index,
        key,
    ) {
        Ok(a) => {
            // SAFETY: `out32` is non-null and the caller guarantees it is
            // valid for 32 writable bytes.
            unsafe { core::ptr::copy_nonoverlapping(a.as_ptr(), out32, 32) };
            0
        }
        Err(e) => i32::from(e.code),
    }
}

/// Validates a source manifest (`client-compute.md` section 5.1).
///
/// Returns 0 on success, or the error code from `docs/errors.md` (901 to
/// 907) on failure, and writes the reason into `err_buf` exactly as
/// [`gx_validate`] does. A null `bytes` with a non-zero `len` fails with 106;
/// a null `bytes` with length 0 is an empty input.
///
/// # Safety
///
/// A non-null `bytes` must be valid for `len` readable bytes. If `err_buf` is
/// non-null and `err_cap` is non-zero, `err_buf` must point to at least
/// `err_cap` writable bytes that do not overlap the input.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_manifest_validate(
    bytes: *const u8,
    len: usize,
    err_buf: *mut c_char,
    err_cap: usize,
) -> i32 {
    // SAFETY: the caller guarantees a non-null `bytes` is valid for `len`.
    let bytes = unsafe { slice(bytes, len) };
    let result = match bytes {
        None => Err(ValidationError::new(
            codes::BYTES_POINTER_NULL,
            "bytes: null pointer with non-zero len",
        )),
        Some(b) => manifest::validate(b),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            // SAFETY: the caller guarantees `err_buf` is valid for `err_cap`
            // bytes when it is non-null and `err_cap` is non-zero.
            unsafe { write_reason(err_buf.cast::<u8>(), err_cap, &e.reason) };
            i32::from(e.code)
        }
    }
}

/// Validates an epoch index (`client-compute.md` section 5.4).
///
/// Returns 0 on success, or the error code from `docs/errors.md` (1001 to
/// 1007) on failure, and writes the reason into `err_buf` exactly as
/// [`gx_validate`] does. A null `bytes` with a non-zero `len` fails with 106;
/// a null `bytes` with length 0 is an empty input.
///
/// # Safety
///
/// A non-null `bytes` must be valid for `len` readable bytes. If `err_buf` is
/// non-null and `err_cap` is non-zero, `err_buf` must point to at least
/// `err_cap` writable bytes that do not overlap the input.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_epochs_validate(
    bytes: *const u8,
    len: usize,
    err_buf: *mut c_char,
    err_cap: usize,
) -> i32 {
    // SAFETY: the caller guarantees a non-null `bytes` is valid for `len`.
    let bytes = unsafe { slice(bytes, len) };
    let result = match bytes {
        None => Err(ValidationError::new(
            codes::BYTES_POINTER_NULL,
            "bytes: null pointer with non-zero len",
        )),
        Some(b) => epochs::validate(b),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            // SAFETY: the caller guarantees `err_buf` is valid for `err_cap`
            // bytes when it is non-null and `err_cap` is non-zero.
            unsafe { write_reason(err_buf.cast::<u8>(), err_cap, &e.reason) };
            i32::from(e.code)
        }
    }
}

/// Applies the epoch rule (`client-compute.md` section 5.4): decodes the
/// epoch index in `bytes` and finds the manifest of span `span_index` of a
/// layer with span length `span_seconds`.
///
/// Returns 0 and writes the 32-byte manifest hash to `out32` when the epoch
/// is listed, and 1 when it is not (the section has no manifest; its address
/// takes all zero bytes). Otherwise returns a negative error code and leaves
/// `out32` untouched: minus 1011 for a null `out32`, minus 106 for a null
/// `bytes` with a non-zero `len`, minus 1001 to 1007 for an invalid index,
/// minus 1009 for a zero `span_seconds`, and minus 1010 when the epoch length
/// is not a whole multiple of `span_seconds`.
///
/// # Safety
///
/// A non-null `bytes` must be valid for `len` readable bytes and a non-null
/// `out32` for 32 writable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_epochs_manifest(
    bytes: *const u8,
    len: usize,
    span_seconds: u64,
    span_index: u64,
    out32: *mut u8,
) -> i32 {
    if out32.is_null() {
        return -i32::from(codes::EPOCHS_POINTER_NULL);
    }
    // SAFETY: the caller guarantees a non-null `bytes` is valid for `len`.
    let bytes = unsafe { slice(bytes, len) };
    match epochs_manifest_ffi(bytes, span_seconds, span_index) {
        Ok(Some(h)) => {
            // SAFETY: `out32` is non-null and the caller guarantees it is
            // valid for 32 writable bytes.
            unsafe { core::ptr::copy_nonoverlapping(h.as_ptr(), out32, 32) };
            0
        }
        Ok(None) => 1,
        Err(e) => -i32::from(e.code),
    }
}

/// The C ABI epoch rule, on safe inputs: the null pointer check, then
/// [`epochs::decode`] and [`epochs::manifest_for_span`].
fn epochs_manifest_ffi(
    bytes: Option<&[u8]>,
    span_seconds: u64,
    span_index: u64,
) -> Result<Option<[u8; 32]>, ValidationError> {
    let bytes = bytes.ok_or_else(|| {
        ValidationError::new(
            codes::BYTES_POINTER_NULL,
            "bytes: null pointer with non-zero len",
        )
    })?;
    let index = epochs::decode(bytes)?;
    epochs::manifest_for_span(&index, span_seconds, span_index)
}

/// Checks a frame record against a frame registry (`matter-format.md`
/// section 11.4).
///
/// Decodes the registry and the record (the record without a key, so its
/// own `frame_id` names the frame), finds the frame in the registry, and
/// compares the seven constant fields bitwise with [`frame_record::agrees`].
/// Returns 0 when the frame is in the registry and every field agrees, minus
/// 1 when the registry does not declare the record's frame, and minus 2 to
/// minus 8 for the first differing field in record order:
/// `parent_frame_id` (-2), `root_extent` (-3), `max_depth` (-4), `mass`
/// (-5), `j2` (-6), `reference_radius` (-7), `principal_moments` (-8). An
/// EMPTY record is compared like any other, so it disagrees with any frame.
/// Invalid input returns a code from `docs/errors.md` negated: -106 for a
/// null pointer with a non-zero length, -601 to -633 for an invalid registry
/// (checked first), -1021 to -1029 for an invalid record. Every code is at
/// least 101, so the two ranges never meet.
///
/// # Safety
///
/// A non-null `registry` must be valid for `registry_len` readable bytes and
/// a non-null `record` for `record_len`.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_frame_record_agrees(
    registry: *const u8,
    registry_len: usize,
    record: *const u8,
    record_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees a non-null pointer is valid for its
    // length.
    let (registry, record) = unsafe { (slice(registry, registry_len), slice(record, record_len)) };
    match (registry, record) {
        (Some(g), Some(r)) => frame_record_agrees_ffi(g, r),
        _ => -i32::from(codes::BYTES_POINTER_NULL),
    }
}

/// The C ABI agreement check on safe inputs: decode both, find the frame,
/// compare, and map the outcome to the codes of [`gx_frame_record_agrees`].
fn frame_record_agrees_ffi(registry_bytes: &[u8], record_bytes: &[u8]) -> i32 {
    use frame_record::Field;
    let registry = match registry::decode(registry_bytes) {
        Ok(g) => g,
        Err(e) => return -i32::from(e.code),
    };
    let record = match frame_record::decode(record_bytes) {
        Ok(r) => r,
        Err(e) => return -i32::from(e.code),
    };
    let Some(frame) = registry
        .frames()
        .iter()
        .find(|f| f.frame_id == record.frame_id)
    else {
        return -1;
    };
    match frame_record::agrees(frame, &record) {
        Ok(()) => 0,
        Err(Field::ParentFrameId) => -2,
        Err(Field::RootExtent) => -3,
        Err(Field::MaxDepth) => -4,
        Err(Field::Mass) => -5,
        Err(Field::J2) => -6,
        Err(Field::ReferenceRadius) => -7,
        Err(Field::PrincipalMoments) => -8,
    }
}

/// Returns the short name of an error code, such as `BAD_MAGIC` for 102, as
/// a pointer to a static NUL-terminated ASCII string. Unknown codes,
/// including 0, give the empty string. Never returns null.
#[cfg_attr(feature = "exports", no_mangle)]
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

fn identity_null(what: &str) -> ValidationError {
    ValidationError::new(
        codes::IDENTITY_POINTER_NULL,
        format!("{what}: null pointer"),
    )
}

/// The C ABI identity and version 2 address, on safe inputs: null pointer,
/// slot, and key checks, then [`address::layer_identity`] (no key) or
/// [`address::section_address_v2`].
fn identity_ffi(
    module: Option<&[u8; 32]>,
    manifest: Option<&[u8; 32]>,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: Option<&[u8]>,
    key: Option<Option<&[u8]>>,
) -> Result<[u8; 32], ValidationError> {
    let module = module.ok_or_else(|| identity_null("module32"))?;
    let slots = slots.ok_or_else(|| identity_null("slots with non-zero slots_len"))?;
    let slots = address::SlotValue::decode_all(slots)?;
    match key {
        None => address::layer_identity(
            module,
            manifest,
            origin_utc_ms,
            span_seconds,
            span_index,
            &slots,
        ),
        Some(key) => {
            let key = parse_key_ffi(key)?;
            address::section_address_v2(
                module,
                manifest,
                origin_utc_ms,
                span_seconds,
                span_index,
                &slots,
                &key,
            )
        }
    }
}

/// Writes 32 bytes to a non-null `out32`, or returns the error's code.
///
/// # Safety
///
/// A non-null `out32` must be valid for 32 writable bytes.
unsafe fn out_hash(out32: *mut u8, r: Result<[u8; 32], ValidationError>) -> i32 {
    match r {
        Ok(h) => {
            // SAFETY: the caller guarantees `out32` is valid for 32 bytes.
            unsafe { core::ptr::copy_nonoverlapping(h.as_ptr(), out32, 32) };
            0
        }
        Err(e) => i32::from(e.code),
    }
}

/// Computes a layer's identity at a span (`client-compute.md` section 10.5)
/// and writes its 32 bytes to `out32`.
///
/// The arguments are those of [`gx_section_address`] without the key, with
/// `manifest32_or_null` the manifest of span `span_index` under the epoch
/// rule, plus the layer's input slots in slot order: `slots_len` bytes,
/// `count * 33`, each a kind byte (0 layer slot, 1 frames slot) and a 32-byte
/// value. A layer with no slots has an identity too.
///
/// Returns 0 on success, 1045 for a null `module32` or `out32` or a null
/// `slots` with a non-zero `slots_len`, 1043 for a `slots_len` that is not a
/// multiple of 33, and 1042 for an unknown kind byte. On failure `out32` is
/// not touched.
///
/// # Safety
///
/// A non-null `module32` and `manifest32_or_null` must each be valid for 32
/// readable bytes, a non-null `slots` for `slots_len` readable bytes, and a
/// non-null `out32` for 32 writable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_layer_identity(
    module32: *const u8,
    manifest32_or_null: *const u8,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: *const u8,
    slots_len: usize,
    out32: *mut u8,
) -> i32 {
    if out32.is_null() {
        return i32::from(codes::IDENTITY_POINTER_NULL);
    }
    // SAFETY: the caller guarantees each non-null pointer is valid for its
    // length, 32 bytes for the hashes.
    let r = unsafe {
        identity_ffi(
            module32.cast::<[u8; 32]>().as_ref(),
            manifest32_or_null.cast::<[u8; 32]>().as_ref(),
            origin_utc_ms,
            span_seconds,
            span_index,
            slice(slots, slots_len),
            None,
        )
    };
    // SAFETY: `out32` is non-null and valid for 32 bytes.
    unsafe { out_hash(out32, r) }
}

/// Computes a version 2 section address (`client-compute.md` section 3) and
/// writes its 32 bytes to `out32`.
///
/// The arguments are those of [`gx_section_address`] with the slots of
/// [`gx_layer_identity`] before the key. Returns 0 on success, 1041 for no
/// slots (a layer without inputs uses [`gx_section_address`]), the codes of
/// [`gx_layer_identity`], 207 for a null `key` with a non-zero `key_len`, and
/// 206 for a key that is not UTF-8 or not a chunk key. On failure `out32` is
/// not touched.
///
/// # Safety
///
/// As [`gx_layer_identity`], and a non-null `key` must be valid for
/// `key_len` readable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_section_address_v2(
    module32: *const u8,
    manifest32_or_null: *const u8,
    origin_utc_ms: i64,
    span_seconds: u64,
    span_index: u64,
    slots: *const u8,
    slots_len: usize,
    key: *const u8,
    key_len: usize,
    out32: *mut u8,
) -> i32 {
    if out32.is_null() {
        return i32::from(codes::IDENTITY_POINTER_NULL);
    }
    // SAFETY: the caller guarantees each non-null pointer is valid for its
    // length, 32 bytes for the hashes.
    let r = unsafe {
        identity_ffi(
            module32.cast::<[u8; 32]>().as_ref(),
            manifest32_or_null.cast::<[u8; 32]>().as_ref(),
            origin_utc_ms,
            span_seconds,
            span_index,
            slice(slots, slots_len),
            Some(slice(key, key_len)),
        )
    };
    // SAFETY: `out32` is non-null and valid for 32 bytes.
    unsafe { out_hash(out32, r) }
}

/// The C ABI frames slot value, on safe inputs.
fn frames_slot_value_ffi(hashes: Option<&[u8]>) -> Result<[u8; 32], ValidationError> {
    let hashes = hashes.ok_or_else(|| identity_null("hashes with non-zero hashes_len"))?;
    let (records, rest) = hashes.as_chunks::<32>();
    if !rest.is_empty() {
        return Err(ValidationError::new(
            codes::SLOTS_LENGTH,
            format!("hashes: {} bytes is not a multiple of 32", hashes.len()),
        ));
    }
    Ok(address::frames_slot_value(records))
}

/// Computes a frames slot's value (`client-compute.md` section 10.5) from
/// `hashes_len` bytes holding the 32-byte hashes of its frame records, in
/// the order the config lists the frames, and writes it to `out32`.
///
/// Returns 0 on success, 1045 for a null `out32` or a null `hashes` with a
/// non-zero `hashes_len`, and 1043 for a length that is not a multiple of
/// 32. On failure `out32` is not touched.
///
/// # Safety
///
/// A non-null `hashes` must be valid for `hashes_len` readable bytes and a
/// non-null `out32` for 32 writable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_frames_slot_value(
    hashes: *const u8,
    hashes_len: usize,
    out32: *mut u8,
) -> i32 {
    if out32.is_null() {
        return i32::from(codes::IDENTITY_POINTER_NULL);
    }
    // SAFETY: the caller guarantees `hashes` is valid for its length and
    // `out32` for 32 bytes.
    unsafe { out_hash(out32, frames_slot_value_ffi(slice(hashes, hashes_len))) }
}

/// The C ABI inputs hash, on safe inputs.
fn inputs_sha256_ffi(inputs: Option<&[u8]>) -> Result<[u8; 32], ValidationError> {
    let inputs = inputs.ok_or_else(|| {
        ValidationError::new(
            codes::INPUT_ARGUMENT,
            "inputs: null pointer with non-zero inputs_len",
        )
    })?;
    Ok(module_io::inputs_sha256(&module_io::decode_inputs(inputs)?))
}

/// Computes the `X-Inputs-Sha256` hash (`client-compute.md` section 10.4)
/// of input bytes as passed to `gx_compile` (`u32` count, then per input a
/// `u32` length and the bytes) and writes its 32 bytes to `out32`.
///
/// Returns 0 on success, 1039 for a null `out32` or a null `inputs` with a
/// non-zero `inputs_len`, and 1037 or 1038 for malformed input bytes. On
/// failure `out32` is not touched.
///
/// # Safety
///
/// A non-null `inputs` must be valid for `inputs_len` readable bytes and a
/// non-null `out32` for 32 writable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_inputs_sha256(
    inputs: *const u8,
    inputs_len: usize,
    out32: *mut u8,
) -> i32 {
    if out32.is_null() {
        return i32::from(codes::INPUT_ARGUMENT);
    }
    // SAFETY: the caller guarantees `inputs` is valid for its length and
    // `out32` for 32 bytes.
    unsafe { out_hash(out32, inputs_sha256_ffi(slice(inputs, inputs_len))) }
}

/// The span at which a layer slot is read (`client-compute.md` section
/// 10.3), written to `out`: `floor(reader_span_index * reader_span_seconds /
/// input_span_seconds)`, 0 when `input_span_seconds` is 0.
///
/// Returns 0 on success, 1045 for a null `out`, and 1044 when the result
/// does not fit a `u64`. On failure `out` is not touched.
///
/// # Safety
///
/// A non-null `out` must be valid for one writable, aligned `u64`.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_input_span_index(
    reader_span_index: u64,
    reader_span_seconds: u64,
    input_span_seconds: u64,
    out: *mut u64,
) -> i32 {
    if out.is_null() {
        return i32::from(codes::IDENTITY_POINTER_NULL);
    }
    match address::input_span_index(reader_span_index, reader_span_seconds, input_span_seconds) {
        Ok(i) => {
            // SAFETY: `out` is non-null and the caller guarantees it is valid.
            unsafe { out.write(i) };
            0
        }
        Err(e) => i32::from(e.code),
    }
}

/// The C ABI input list decode, on safe inputs.
fn input_list_ffi(list: Option<&[u8]>) -> Result<Vec<module_io::InputEntry>, ValidationError> {
    let list = list.ok_or_else(|| {
        ValidationError::new(
            codes::INPUT_ARGUMENT,
            "list: null pointer with non-zero list_len",
        )
    })?;
    module_io::decode_input_list(list)
}

/// Validates an input list as `gx_inputs` returns it (`client-compute.md`
/// section 4.1) and, when `count_out_or_null` is non-null, writes its entry
/// count there.
///
/// Returns 0 on success, or 1031 to 1036 for an invalid list and 1039 for a
/// null `list` with a non-zero `list_len`, writing the reason to `err_buf`
/// under the rules of [`gx_validate`]. Whether each slot is a layer slot of
/// the config is the caller's check.
///
/// # Safety
///
/// A non-null `list` must be valid for `list_len` readable bytes, a non-null
/// `count_out_or_null` for one writable, aligned `u32`, and `err_buf` as in
/// [`gx_validate`].
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_input_list_validate(
    list: *const u8,
    list_len: usize,
    count_out_or_null: *mut u32,
    err_buf: *mut c_char,
    err_cap: usize,
) -> i32 {
    // SAFETY: the caller guarantees `list` is valid for its length.
    match input_list_ffi(unsafe { slice(list, list_len) }) {
        Ok(entries) => {
            if !count_out_or_null.is_null() {
                // SAFETY: non-null and the caller guarantees it is valid.
                unsafe { count_out_or_null.write(entries.len() as u32) };
            }
            0
        }
        Err(e) => {
            // SAFETY: the caller guarantees `err_buf` is valid for `err_cap`.
            unsafe { write_reason(err_buf.cast::<u8>(), err_cap, &e.reason) };
            i32::from(e.code)
        }
    }
}

/// Reads entry `index` of a valid input list: writes its slot to `slot_out`,
/// its key's length in bytes to `key_len_out`, and the key (UTF-8, not NUL
/// terminated) to `key_buf`.
///
/// Returns 0 on success; the codes of [`gx_input_list_validate`] for an
/// invalid list; 1039 for a null `slot_out` or `key_len_out`, an `index` at
/// or past the entry count, or a `key_cap` smaller than the key, in which
/// last case only `key_len_out` is written so the caller can size the
/// buffer. A chunk key is at most 56 bytes.
///
/// # Safety
///
/// A non-null `list` must be valid for `list_len` readable bytes, `slot_out`
/// and `key_len_out` for one writable, aligned value each, and a non-null
/// `key_buf` for `key_cap` writable bytes.
#[cfg_attr(feature = "exports", no_mangle)]
pub unsafe extern "C" fn gx_input_list_entry(
    list: *const u8,
    list_len: usize,
    index: u32,
    slot_out: *mut u32,
    key_buf: *mut u8,
    key_cap: usize,
    key_len_out: *mut usize,
) -> i32 {
    let arg = i32::from(codes::INPUT_ARGUMENT);
    if slot_out.is_null() || key_len_out.is_null() {
        return arg;
    }
    // SAFETY: the caller guarantees `list` is valid for its length.
    let entries = match input_list_ffi(unsafe { slice(list, list_len) }) {
        Ok(e) => e,
        Err(e) => return i32::from(e.code),
    };
    let Some(entry) = entries.get(index as usize) else {
        return arg;
    };
    let key = entry.key.to_string();
    // SAFETY: `key_len_out` is non-null and the caller guarantees it is valid.
    unsafe { key_len_out.write(key.len()) };
    if key_cap < key.len() || (key_buf.is_null() && !key.is_empty()) {
        return arg;
    }
    // SAFETY: `key_buf` holds at least `key.len()` writable bytes and
    // `slot_out` is non-null and valid.
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_buf, key.len());
        slot_out.write(entry.slot);
    }
    0
}

#[cfg(all(target_arch = "wasm32", feature = "exports"))]
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

    /// The section address (`client-compute.md` section 3) as lowercase
    /// hex. `module` and `manifest` are 32-byte SHA-256 digests; pass
    /// `undefined` or `null` for no manifest. Throws an `Error` with message
    /// `CODE: REASON` for a hash of the wrong length (962) or a key that is
    /// not a chunk key (206).
    #[wasm_bindgen]
    pub fn section_address(
        module: &[u8],
        manifest: Option<Vec<u8>>,
        origin_utc_ms: i64,
        span_seconds: u64,
        span_index: u64,
        key: &str,
    ) -> Result<String, JsError> {
        let hash = |what: &str, b: &[u8]| -> Result<[u8; 32], JsError> {
            b.try_into().map_err(|_| {
                js(ValidationError::new(
                    crate::error::codes::ADDRESS_POINTER_NULL,
                    format!("{what}: {} bytes, a SHA-256 is 32", b.len()),
                ))
            })
        };
        let module = hash("module", module)?;
        let manifest = manifest.map(|m| hash("manifest", &m)).transpose()?;
        let a = super::section_address_ffi(
            Some(&module),
            manifest.as_ref(),
            origin_utc_ms,
            span_seconds,
            span_index,
            Some(key.as_bytes()),
        )
        .map_err(js)?;
        Ok(crate::address::to_hex(&a))
    }

    fn hash32(what: &str, b: &[u8]) -> Result<[u8; 32], JsError> {
        b.try_into().map_err(|_| {
            js(ValidationError::new(
                crate::error::codes::IDENTITY_POINTER_NULL,
                format!("{what}: {} bytes, a SHA-256 is 32", b.len()),
            ))
        })
    }

    /// A layer's identity at a span (`client-compute.md` section 10.5) as
    /// lowercase hex. `slots` is `count * 33` bytes: per slot a kind byte (0
    /// layer, 1 frames) and a 32-byte value. Throws an `Error` with message
    /// `CODE: REASON` on failure (1042, 1043, 1045).
    #[wasm_bindgen]
    pub fn layer_identity(
        module: &[u8],
        manifest: Option<Vec<u8>>,
        origin_utc_ms: i64,
        span_seconds: u64,
        span_index: u64,
        slots: &[u8],
    ) -> Result<String, JsError> {
        let module = hash32("module", module)?;
        let manifest = manifest.map(|m| hash32("manifest", &m)).transpose()?;
        super::identity_ffi(
            Some(&module),
            manifest.as_ref(),
            origin_utc_ms,
            span_seconds,
            span_index,
            Some(slots),
            None,
        )
        .map(|h| crate::sha256::to_hex(&h))
        .map_err(js)
    }

    /// A version 2 section address (`client-compute.md` section 3) as
    /// lowercase hex, with `slots` as in `layer_identity`. Throws an `Error`
    /// with message `CODE: REASON` on failure (1041 for no slots).
    #[wasm_bindgen]
    pub fn section_address_v2(
        module: &[u8],
        manifest: Option<Vec<u8>>,
        origin_utc_ms: i64,
        span_seconds: u64,
        span_index: u64,
        slots: &[u8],
        key: &str,
    ) -> Result<String, JsError> {
        let module = hash32("module", module)?;
        let manifest = manifest.map(|m| hash32("manifest", &m)).transpose()?;
        super::identity_ffi(
            Some(&module),
            manifest.as_ref(),
            origin_utc_ms,
            span_seconds,
            span_index,
            Some(slots),
            Some(Some(key.as_bytes())),
        )
        .map(|h| crate::sha256::to_hex(&h))
        .map_err(js)
    }

    /// A frames slot's value (`client-compute.md` section 10.5) as lowercase
    /// hex, from the concatenated 32-byte record hashes. Throws an `Error`
    /// with message `CODE: REASON` (1043) for a length that is not a
    /// multiple of 32.
    #[wasm_bindgen]
    pub fn frames_slot_value(hashes: &[u8]) -> Result<String, JsError> {
        super::frames_slot_value_ffi(Some(hashes))
            .map(|h| crate::sha256::to_hex(&h))
            .map_err(js)
    }

    /// The `X-Inputs-Sha256` hash (`client-compute.md` section 10.4) of
    /// input bytes, as lowercase hex. Throws an `Error` with message
    /// `CODE: REASON` (1037, 1038) for malformed input bytes.
    #[wasm_bindgen]
    pub fn inputs_sha256(inputs: &[u8]) -> Result<String, JsError> {
        super::inputs_sha256_ffi(Some(inputs))
            .map(|h| crate::sha256::to_hex(&h))
            .map_err(js)
    }

    /// The span at which a layer slot is read (`client-compute.md` section
    /// 10.3). Throws an `Error` with message `CODE: REASON` (1044) when it
    /// does not fit a `u64`.
    #[wasm_bindgen]
    pub fn input_span_index(
        reader_span_index: u64,
        reader_span_seconds: u64,
        input_span_seconds: u64,
    ) -> Result<u64, JsError> {
        crate::address::input_span_index(reader_span_index, reader_span_seconds, input_span_seconds)
            .map_err(js)
    }

    /// Validates an input list (`client-compute.md` section 4.1) and returns
    /// its entry count. Throws an `Error` with message `CODE: REASON` (1031
    /// to 1036) on failure.
    #[wasm_bindgen]
    pub fn input_list_validate(bytes: &[u8]) -> Result<u32, JsError> {
        crate::module_io::decode_input_list(bytes)
            .map(|e| e.len() as u32)
            .map_err(js)
    }

    /// Validates a source manifest. Throws an `Error` with message
    /// `CODE: REASON` on failure.
    #[wasm_bindgen]
    pub fn manifest_validate(bytes: &[u8]) -> Result<(), JsError> {
        crate::manifest::validate(bytes).map_err(js)
    }

    /// Validates an epoch index. Throws an `Error` with message
    /// `CODE: REASON` on failure.
    #[wasm_bindgen]
    pub fn epochs_validate(bytes: &[u8]) -> Result<(), JsError> {
        crate::epochs::validate(bytes).map_err(js)
    }

    /// The epoch rule: the manifest hash, as lowercase hex, of span
    /// `span_index` of a layer with span length `span_seconds` under the
    /// epoch index `bytes`, or `undefined` when the epoch is not listed.
    /// Throws an `Error` with message `CODE: REASON` for an invalid index or
    /// span length.
    #[wasm_bindgen]
    pub fn epochs_manifest(
        bytes: &[u8],
        span_seconds: u64,
        span_index: u64,
    ) -> Result<Option<String>, JsError> {
        super::epochs_manifest_ffi(Some(bytes), span_seconds, span_index)
            .map(|h| h.map(|h| crate::sha256::to_hex(&h)))
            .map_err(js)
    }

    /// Seconds of Barycentric Dynamical Time since J2000 at a UTC instant in
    /// Unix-style milliseconds.
    #[wasm_bindgen]
    pub fn utc_ms_to_tdb_seconds(utc_ms: i64) -> f64 {
        crate::timescale::utc_ms_to_tdb_seconds(utc_ms)
    }

    /// The UTC instant in Unix-style milliseconds of a TDB time in seconds
    /// since J2000.
    #[wasm_bindgen]
    pub fn tdb_seconds_to_utc_ms(tdb: f64) -> i64 {
        crate::timescale::tdb_seconds_to_utc_ms(tdb)
    }

    /// Integrates a frame registry (format version 2) from its epoch to
    /// `to_seconds` (TDB seconds since J2000) in equal steps no longer than
    /// `max_step_seconds` with the default scheme, and returns
    /// `FrameSystem::state_bytes` of the result. Throws an `Error` with
    /// message `CODE: REASON` for an invalid registry, and with code 0 for a
    /// step limit or time that is not finite and positive where required.
    #[wasm_bindgen]
    pub fn integrate_registry(
        bytes: &[u8],
        to_seconds: f64,
        max_step_seconds: f64,
    ) -> Result<Vec<u8>, JsError> {
        if !(max_step_seconds.is_finite() && max_step_seconds > 0.0) || !to_seconds.is_finite() {
            return Err(JsError::new(
                "0: time and step limit must be finite, step above zero",
            ));
        }
        let reg = crate::registry::decode(bytes).map_err(js)?;
        let tree = crate::registry::FrameTree::from_registries(&[reg]).map_err(js)?;
        let mut s = crate::frames::FrameSystem::from_tree(tree);
        crate::integrate::advance(
            &mut s,
            crate::units::Seconds::new(to_seconds),
            crate::units::Seconds::new(max_step_seconds),
            crate::integrate::Scheme::default(),
        );
        Ok(s.state_bytes())
    }

    /// Solves the light-time equation for frame `frame_id` seen from
    /// `observer` (three numbers, root coordinates) at `t` (TDB seconds
    /// since J2000), against a history of the frame system built from
    /// `registry` (format version 2) with checkpoints every `interval`
    /// seconds, steps no longer than `max_step`, the default scheme, and a
    /// capacity of 64, extended to `t` first. Returns
    /// `light::RetardedState::to_bytes`. Throws an `Error` with message
    /// `CODE: REASON` for an invalid registry, and with code 0 for any other
    /// bad argument.
    #[wasm_bindgen]
    pub fn light_retarded_state(
        registry: &[u8],
        interval: f64,
        max_step: f64,
        frame_id: u64,
        observer: &[f64],
        t: f64,
    ) -> Result<Vec<u8>, JsError> {
        use crate::units::{Seconds, Vec3};
        let bad = |what: &str| JsError::new(&format!("0: {what}"));
        if observer.len() != 3 || !observer.iter().all(|x| x.is_finite()) || !t.is_finite() {
            return Err(bad(
                "observer needs three finite numbers and t must be finite",
            ));
        }
        for v in [interval, max_step] {
            if !(v.is_finite() && v > 0.0) {
                return Err(bad("interval and max_step must be finite and above zero"));
            }
        }
        let reg = crate::registry::decode(registry).map_err(js)?;
        let tree = crate::registry::FrameTree::from_registries(&[reg]).map_err(js)?;
        let system = crate::frames::FrameSystem::from_tree(tree);
        if system.state(frame_id).is_none() {
            return Err(bad("frame_id is not in the registry"));
        }
        let mut h = crate::history::History::new(
            system,
            Seconds::new(interval),
            Seconds::new(max_step),
            crate::integrate::Scheme::default(),
            64,
        );
        h.extend_to(Seconds::new(t));
        let r = crate::light::retarded_state(
            Vec3::new(observer[0], observer[1], observer[2]),
            Seconds::new(t),
            &h,
            frame_id,
        );
        Ok(r.to_bytes())
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

#[cfg(all(target_arch = "wasm32", feature = "exports"))]
pub use wasm::{
    decode_chunk_mass, epochs_manifest, epochs_validate, format_version, frames_slot_value,
    input_list_validate, input_span_index, inputs_sha256, integrate_registry, layer_identity,
    light_retarded_state, manifest_validate, section_address, section_address_v2,
};

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
        registry::encode(&registry::Registry::empty())
    }

    fn reason(buf: &[u8]) -> &str {
        CStr::from_bytes_until_nul(buf).unwrap().to_str().unwrap()
    }

    #[test]
    fn format_version_is_three() {
        assert_eq!(gx_format_version(), 3);
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
    fn section_address_abi() {
        let m = [3u8; 32];
        let f = [4u8; 32];
        let k = b"9-2-1-3-0";
        let mut out = [0u8; 32];
        let rc = unsafe {
            gx_section_address(
                m.as_ptr(),
                f.as_ptr(),
                -7,
                60,
                2,
                k.as_ptr(),
                k.len(),
                out.as_mut_ptr(),
            )
        };
        assert_eq!(rc, 0);
        let key: key::ChunkKey = "9-2-1-3-0".parse().unwrap();
        assert_eq!(out, address::section_address(&m, Some(&f), -7, 60, 2, &key));
        let rc = unsafe {
            gx_section_address(
                m.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                b"registry".as_ptr(),
                8,
                out.as_mut_ptr(),
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(
            out,
            address::section_address(&m, None, 0, 0, 0, &key::ChunkKey::Registry)
        );

        let mut untouched = [0xeeu8; 32];
        let call = |module: *const u8, key: *const u8, key_len: usize, out: *mut u8| unsafe {
            gx_section_address(module, ptr::null(), 0, 0, 0, key, key_len, out)
        };
        let o = untouched.as_mut_ptr();
        assert_eq!(call(ptr::null(), k.as_ptr(), k.len(), o), 962);
        assert_eq!(call(m.as_ptr(), k.as_ptr(), k.len(), ptr::null_mut()), 962);
        assert_eq!(call(m.as_ptr(), ptr::null(), 3, o), 207);
        assert_eq!(call(m.as_ptr(), ptr::null(), 0, o), 206);
        assert_eq!(call(m.as_ptr(), b"1-0-0-0-1".as_ptr(), 9, o), 206);
        assert_eq!(call(m.as_ptr(), b"\xff".as_ptr(), 1, o), 206);
        assert!(untouched.iter().all(|&b| b == 0xee));
    }

    #[test]
    fn manifest_validate_abi() {
        let good = manifest::encode(
            &manifest::Manifest::new(vec![manifest::Entry {
                id: 1,
                sha256: [9; 32],
                length: 5,
            }])
            .unwrap(),
        );
        let mut buf = [0xffu8; 64];
        let call = |b: &[u8], buf: &mut [u8]| unsafe {
            gx_manifest_validate(b.as_ptr(), b.len(), buf.as_mut_ptr().cast(), buf.len())
        };
        assert_eq!(call(&good, &mut buf), 0);
        assert!(buf.iter().all(|&b| b == 0xff));
        assert_eq!(call(&good[..5], &mut buf), 901);
        assert_eq!(
            reason(&buf),
            manifest::validate(&good[..5]).unwrap_err().reason
        );
        let rc = unsafe { gx_manifest_validate(ptr::null(), 4, buf.as_mut_ptr().cast(), 64) };
        assert_eq!(rc, 106);
        let rc = unsafe { gx_manifest_validate(ptr::null(), 0, ptr::null_mut(), 0) };
        assert_eq!(rc, 901);
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
