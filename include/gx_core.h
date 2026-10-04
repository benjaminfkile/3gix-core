#ifndef GX_CORE_H
#define GX_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Returns the matter format version. */
uint32_t gx_format_version(void);

/*
 * Validates `bytes` stored under chunk `key`. Returns 0 on success and a
 * non-zero code on failure, writing a NUL-terminated UTF-8 reason into
 * `err_buf` truncated to `err_cap` bytes. Currently a placeholder that always
 * returns 1 with the reason "validator not implemented".
 */
int32_t gx_validate(const uint8_t* key, size_t key_len,
                    const uint8_t* bytes, size_t bytes_len,
                    uint8_t* err_buf, size_t err_cap);

#ifdef __cplusplus
}
#endif

#endif /* GX_CORE_H */
