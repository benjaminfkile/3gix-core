#ifndef GX_CORE_H
#define GX_CORE_H

/*
 * C ABI of the 3gix core library (matter-format.md section 7).
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

/* Returns the matter format version this library implements (1). */
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

#ifdef __cplusplus
}
#endif

#endif /* GX_CORE_H */
