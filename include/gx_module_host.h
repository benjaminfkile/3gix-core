#ifndef GX_MODULE_HOST_H
#define GX_MODULE_HOST_H

/*
 * C ABI of the native module host (client-compute.md sections 2.7 and 4).
 *
 * Runs a compiler module natively with the host clients use (wasmtime with
 * Cranelift, the module ABI version 1), so the hub can settle a dispute by
 * running the module itself. Link against the native shared library built
 * by `cargo build --release -p gx-module-host`: libgx_module_host.so on
 * Linux, libgx_module_host.dylib on macOS, gx_module_host.dll on Windows.
 *
 * Every call checks the module against its expected hash and runs it in
 * a fresh instance with the default limits: 1 GiB of linear memory and 60 s
 * per call. The most recently compiled module is kept and reused when the
 * next call passes the same hash; this never changes the output. Limits are
 * per call. Every function is thread safe; panics are caught at the
 * boundary and reported as GX_MODULE_ERR_PANIC.
 *
 * Output is the module's own bytes, not validated by this library. The
 * caller validates a section with gx_validate (gx_core.h) under the
 * request's chunk key before using it.
 *
 * Errors: a negative code below, with the reason written into err_buf as
 * NUL-terminated UTF-8, truncated to at most err_len - 1 bytes without
 * splitting a UTF-8 sequence. With a null err_buf or a zero err_len the
 * reason is not written. On failure *out_ptr and *out_len are untouched.
 */

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* An argument is unusable: a null pointer with a non-zero length, or a null
 * module_hash32, out_ptr, or out_len. */
#define GX_MODULE_ERR_ARGUMENT (-1)
/* The module bytes do not hash (SHA-256) to module_hash32. */
#define GX_MODULE_ERR_HASH (-2)
/* The module is not admissible: it does not parse, imports something, uses a
 * forbidden feature or instruction, lacks a required export, or reports an
 * ABI version other than 1. */
#define GX_MODULE_ERR_REJECTED (-3)
/* The engine could not be built, or the module could not be compiled or
 * instantiated. */
#define GX_MODULE_ERR_ENGINE (-4)
/* The call ran past the time limit. */
#define GX_MODULE_ERR_TIME_LIMIT (-5)
/* The module asked for more memory than the limit. */
#define GX_MODULE_ERR_MEMORY_LIMIT (-6)
/* The module trapped. */
#define GX_MODULE_ERR_TRAP (-7)
/* The module returned -1; err_buf holds what its gx_last_error reported. */
#define GX_MODULE_ERR_MODULE_FAILED (-8)
/* The module broke the ABI: an out-of-bounds block, a failed allocation, or
 * a source id list that is not ascending, unique u64 LE values. */
#define GX_MODULE_ERR_ABI (-9)
/* The host panicked: a bug in the host, never in the module. */
#define GX_MODULE_ERR_PANIC (-10)

/*
 * Loads the module (module_len bytes) under module_hash32, its expected
 * 32-byte SHA-256, and runs gx_compile with the request bytes and tile bytes
 * exactly as given (client-compute.md section 4: "3GXQ" request; u32 LE tile
 * count, then per tile u64 LE id, u32 LE length, bytes, in gx_sources order).
 *
 * Returns 0 and the section bytes in *out_ptr and *out_len; free them with
 * gx_module_free. An empty output is a null pointer with length 0. On
 * failure returns a GX_MODULE_ERR_* code.
 */
int32_t gx_module_compile(const uint8_t* module, size_t module_len,
                          const uint8_t* module_hash32,
                          const uint8_t* request, size_t request_len,
                          const uint8_t* tiles, size_t tiles_len,
                          uint8_t** out_ptr, size_t* out_len,
                          char* err_buf, size_t err_len);

/*
 * Loads the module as gx_module_compile does and runs gx_sources with the
 * request bytes exactly as given.
 *
 * Returns 0 and the source tile id list in *out_ptr and *out_len: raw u64 LE
 * ids, eight bytes each, ascending and unique; empty (null, 0) when the
 * module reads no sources. Free it with gx_module_free. On failure returns a
 * GX_MODULE_ERR_* code.
 */
int32_t gx_module_sources(const uint8_t* module, size_t module_len,
                          const uint8_t* module_hash32,
                          const uint8_t* request, size_t request_len,
                          uint8_t** out_ptr, size_t* out_len,
                          char* err_buf, size_t err_len);

/*
 * Frees a buffer returned by gx_module_compile or gx_module_sources, with
 * the exact length returned. A null ptr is ignored.
 */
void gx_module_free(uint8_t* ptr, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* GX_MODULE_HOST_H */
