#ifndef GX_CORE_H
#define GX_CORE_H

/*
 * C ABI of the 3gix core library (matter-format.md section 7, and
 * client-compute.md sections 3, 5.1, and 5.4 for section addresses, source
 * manifests, and epoch indexes).
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

/*
 * Validates an epoch index (client-compute.md section 5.4).
 *
 * Returns 0 on success, or an error code from 1001 to 1007 (docs/errors.md)
 * on failure, and writes the reason into `err_buf` exactly as gx_validate
 * does. A NULL `bytes` with a non-zero `len` returns 106; a NULL `bytes` with
 * length 0 is an empty input.
 */
int32_t gx_epochs_validate(const uint8_t* bytes, size_t len,
                           char* err_buf, size_t err_cap);

/*
 * Applies the epoch rule (client-compute.md section 5.4) to the epoch index
 * in `bytes`: the epoch of span `span_index` of a layer with span length
 * `span_seconds` is floor(span_index * span_seconds / epoch_seconds),
 * computed without overflow.
 *
 * Returns 0 and writes that epoch's 32-byte manifest hash to `out32` when the
 * index lists the epoch. Returns 1 when it does not: the section has no
 * manifest and its address takes 32 zero bytes. Otherwise returns a negative
 * error code (the code of docs/errors.md, negated) and leaves `out32`
 * untouched: -1011 for a NULL `out32`, -106 for a NULL `bytes` with a
 * non-zero `len`, -1001 to -1007 for an invalid index, -1009 for a zero
 * `span_seconds`, and -1010 when epoch_seconds is not a whole multiple of
 * `span_seconds`.
 */
int32_t gx_epochs_manifest(const uint8_t* bytes, size_t len,
                           uint64_t span_seconds, uint64_t span_index,
                           uint8_t* out32);

/* gx_frame_record_agrees results other than 0 and the negated error codes. */
#define GX_FRAME_ABSENT (-1)
#define GX_FRAME_DIFFERS_PARENT_FRAME_ID (-2)
#define GX_FRAME_DIFFERS_ROOT_EXTENT (-3)
#define GX_FRAME_DIFFERS_MAX_DEPTH (-4)
#define GX_FRAME_DIFFERS_MASS (-5)
#define GX_FRAME_DIFFERS_J2 (-6)
#define GX_FRAME_DIFFERS_REFERENCE_RADIUS (-7)
#define GX_FRAME_DIFFERS_PRINCIPAL_MOMENTS (-8)

/*
 * Checks a frame record (matter-format.md section 11) against a frame
 * registry (section 5): section 11.4, bitwise equality of parent_frame_id,
 * root_extent, max_depth, mass, j2, reference_radius, and principal_moments.
 * The record is decoded without a key; its own frame_id names the frame.
 *
 * Returns 0 when the registry declares the record's frame and every field
 * agrees. Returns GX_FRAME_ABSENT (-1) when the registry does not declare the
 * frame, and -2 to -8 (the GX_FRAME_DIFFERS_ constants above) for the first
 * field, in record order, that differs. An EMPTY record is compared like any
 * other and so disagrees. Invalid input returns a code of docs/errors.md
 * negated: -106 for a NULL pointer with a non-zero length, -601 to -633 for
 * an invalid registry (checked first), -1021 to -1029 for an invalid record.
 */
int32_t gx_frame_record_agrees(const uint8_t* registry, size_t registry_len,
                               const uint8_t* record, size_t record_len);

/*
 * Layer inputs (client-compute.md sections 3, 4.1, 10.3, 10.4, 10.5).
 *
 * Slots cross the ABI as `slots_len` bytes, `count * 33`: per slot, in slot
 * order, a kind byte (0 layer slot, 1 frames slot) and a 32-byte value. A
 * layer slot's value is the read layer's identity (gx_layer_identity) at
 * the input span (gx_input_span_index); a frames slot's value is
 * gx_frames_slot_value. Codes: 1041 no slots for a version 2 address, 1042
 * unknown kind byte, 1043 a length that is not a multiple of 33 (or 32 for
 * record hashes), 1044 an input span index that does not fit, 1045 a NULL
 * hash or output pointer, or a NULL slots or hashes pointer with a non-zero
 * length. On failure no output is touched.
 */

/*
 * Computes a layer's identity at span `span_index` (section 10.5, "3GXI",
 * identity version 1) and writes its 32 bytes to `out32`. The arguments are
 * those of gx_section_address without the key, with `manifest32_or_null`
 * the manifest of that span under the epoch rule, plus the slots. A layer
 * with no slots has an identity too. Returns 0, 1042, 1043, or 1045.
 */
int32_t gx_layer_identity(const uint8_t* module32,
                          const uint8_t* manifest32_or_null,
                          int64_t origin_utc_ms, uint64_t span_seconds,
                          uint64_t span_index, const uint8_t* slots,
                          size_t slots_len, uint8_t* out32);

/*
 * Computes a version 2 section address (section 3) of a layer with at least
 * one input and writes its 32 bytes to `out32`. Returns 0, 1041 for no slots
 * (use gx_section_address), 1042, 1043, 1045, 207 for a NULL `key` with a
 * non-zero `key_len`, or 206 for a key that is not UTF-8 or not a chunk key.
 */
int32_t gx_section_address_v2(const uint8_t* module32,
                              const uint8_t* manifest32_or_null,
                              int64_t origin_utc_ms, uint64_t span_seconds,
                              uint64_t span_index, const uint8_t* slots,
                              size_t slots_len, const uint8_t* key,
                              size_t key_len, uint8_t* out32);

/*
 * Computes a frames slot's value: SHA-256 over the 32-byte hashes of its
 * frame records in config order, given as `hashes_len` concatenated bytes.
 * Returns 0, 1043, or 1045.
 */
int32_t gx_frames_slot_value(const uint8_t* hashes, size_t hashes_len,
                             uint8_t* out32);

/*
 * Computes X-Inputs-Sha256 (section 10.4): SHA-256 over the SHA-256 of each
 * input in order, from input bytes as passed to gx_compile (u32 LE count,
 * then per input a u32 LE length and the bytes). Returns 0, 1037 or 1038 for
 * malformed input bytes, or 1039 for a NULL `out32` or a NULL `inputs` with
 * a non-zero `inputs_len`.
 */
int32_t gx_inputs_sha256(const uint8_t* inputs, size_t inputs_len,
                         uint8_t* out32);

/*
 * The span at which a layer slot is read (section 10.3):
 * floor(reader_span_index * reader_span_seconds / input_span_seconds),
 * computed exactly, and 0 when `input_span_seconds` is 0. Writes it to `out`
 * and returns 0, or returns 1044 when it does not fit a uint64_t and 1045 for
 * a NULL `out`.
 */
int32_t gx_input_span_index(uint64_t reader_span_index,
                            uint64_t reader_span_seconds,
                            uint64_t input_span_seconds, uint64_t* out);

/*
 * Validates an input list as gx_inputs returns it (section 4.1): u32 LE
 * count, then per entry a u32 LE slot, a u32 LE key length, and the key;
 * sorted by slot then key bytes, unique, at most 64, keys that parse.
 * Writes the entry count to `count_out_or_null` when non-NULL. Returns 0, or
 * 1031 to 1036 for an invalid list and 1039 for a NULL `list` with a non-zero
 * `list_len`, writing the reason to `err_buf` as gx_validate does.
 */
int32_t gx_input_list_validate(const uint8_t* list, size_t list_len,
                               uint32_t* count_out_or_null, char* err_buf,
                               size_t err_cap);

/*
 * Reads entry `index` of a valid input list: its slot to `slot_out`, its key
 * length to `key_len_out`, and the key (UTF-8, not NUL terminated, at most 56
 * bytes) to `key_buf`. Returns 0, the codes of gx_input_list_validate for an
 * invalid list, or 1039 for a NULL `slot_out` or `key_len_out`, an `index`
 * past the end, or a `key_cap` smaller than the key (then only `key_len_out`
 * is written).
 */
int32_t gx_input_list_entry(const uint8_t* list, size_t list_len,
                            uint32_t index, uint32_t* slot_out,
                            uint8_t* key_buf, size_t key_cap,
                            size_t* key_len_out);

#ifdef __cplusplus
}
#endif

#endif /* GX_CORE_H */
