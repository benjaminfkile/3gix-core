#ifndef GX_CORE_H
#define GX_CORE_H

/*
 * C ABI of the 3gix core library (matter-format.md section 7, and
 * client-compute.md sections 3 and 5.1 for section addresses and source
 * manifests).
 *
 * Link against the native shared library (libgx_core.so on Linux). Every
 * function is thread safe and keeps no state between calls. Error codes and
 * their meaning are listed in docs/errors.md; a code never changes meaning.
 */

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Returns the matter section format version this library implements (3:
 * eight channels per sample, occupancy last, 37 bytes per sample). Versions
 * 1 and 2 are retired: a section carrying either fails validation with
 * code 107 (VERSION_RETIRED). The frame registry it reads and writes is
 * registry format version 2.
 */
uint32_t gx_format_version(void);

/*
 * Validates one submitted section stored under chunk `key`: a frame registry
 * when the key is "registry", a matter section otherwise (matter-format.md
 * section 4). `key` is UTF-8 and need not be NUL terminated.
 *
 * Returns 0 on success, or the error code from docs/errors.md on failure.
 *
 * On failure, writes the reason as NUL-terminated UTF-8 into `err_buf`,
 * truncated to at most `err_cap - 1` bytes plus the NUL, never splitting a
 * UTF-8 sequence. With a null `err_buf` or a zero `err_cap`, writes nothing
 * and still returns the code. On success `err_buf` is not touched.
 *
 * A null pointer with length 0 is an empty input. A null `key` with a
 * non-zero `key_len` returns 207, a null `bytes` with a non-zero `bytes_len`
 * returns 106, and a `key` that is not UTF-8 or not a chunk key returns 206.
 *
 * Non-null `key` and `bytes` must be readable for their lengths, and a
 * non-null `err_buf` writable for `err_cap` bytes.
 */
int32_t gx_validate(const uint8_t* key, size_t key_len,
                    const uint8_t* bytes, size_t bytes_len,
                    char* err_buf, size_t err_cap);

/*
 * Returns the short name of error code `code`, such as "BAD_MAGIC" for 102,
 * as a static NUL-terminated ASCII string the caller must not free. Unknown
 * codes, including 0, give the empty string. Never returns NULL.
 */
const char* gx_error_name(int32_t code);

/*
 * Computes a section address (client-compute.md section 3) and writes its 32
 * bytes to `out32`. Format it as lowercase hex for routes and storage paths.
 *
 * `module32` is the module's SHA-256 (32 bytes). `manifest32_or_null` is the
 * source manifest's SHA-256, or NULL when the layer has none (written as 32
 * zero bytes). `origin_utc_ms` is the space's origin in UTC milliseconds, or
 * 0 when the layer's usesOrigin is false. `span_seconds` and `span_index` are
 * the layer's span length and the section's span (both 0 for matter that
 * never changes). `key` is the chunk key as UTF-8 of `key_len` bytes, not
 * NUL terminated.
 *
 * Returns 0 on success. Returns 962 for a NULL `module32` or `out32`, 207 for
 * a NULL `key` with a non-zero `key_len`, and 206 for a key that is not UTF-8
 * or not a chunk key. On failure `out32` is not touched.
 */
int32_t gx_section_address(const uint8_t* module32,
                           const uint8_t* manifest32_or_null,
                           int64_t origin_utc_ms, uint64_t span_seconds,
                           uint64_t span_index, const uint8_t* key,
                           size_t key_len, uint8_t* out32);

/*
 * Validates a source manifest (client-compute.md section 5.1).
 *
 * Returns 0 on success, or an error code from 901 to 907 (docs/errors.md) on
 * failure, and writes the reason into `err_buf` exactly as gx_validate does.
 * A NULL `bytes` with a non-zero `len` returns 106; a NULL `bytes` with
 * length 0 is an empty input.
 */
int32_t gx_manifest_validate(const uint8_t* bytes, size_t len,
                             char* err_buf, size_t err_cap);

#ifdef __cplusplus
}
#endif

#endif /* GX_CORE_H */
