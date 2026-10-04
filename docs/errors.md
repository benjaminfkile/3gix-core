# Validation error codes

Every validation failure carries one stable numeric code and a human-readable reason. Callers compare codes; reasons are for people and may be reworded. A code never changes meaning once published. A new rule gets a new code, and a retired rule's code is never reused.

The codes are defined in `crates/gx-core/src/error.rs` (module `codes`). This table is the reference.

## Ranges

| Range | Group | Status |
|---|---|---|
| 100 to 199 | Header: length, magic, version, flags, reserved fields | in use |
| 200 to 299 | Key: header fields must match the chunk key | in use |
| 300 to 399 | Geometry: cell edge and origin | in use |
| 400 to 499 | Sample block: resolution, lengths, empty section rules, zstd framing | in use |
| 500 to 599 | Channel values, including the vacuum rules | in use |
| 600 to 649 | Frame registry, one registry (`matter-format.md` sections 5.1 to 5.3) | in use |
| 650 to 699 | Frame registry, union of a build's registries (`matter-format.md` section 5.2) | in use |
| 700 to 799 | Hub container (`matter-format.md` section 6) | in use |
| 800 to 899 | Compositing (`matter-format.md` section 3.5), not byte rules | in use |

## Check order

`decode` and `validate` run `matter-format.md` section 4 steps 1, 2, and 4 through 8 in that order and stop at the first failing rule. Step 3, the registry branch, belongs to the top-level validator that dispatches on the key. Within each step the checks run in the order of the table below. For channel values (step 8):

1. Each channel's own rule, channel by channel in wire order (density, state, temperature, albedo, roughness, attenuation). Within a channel, samples run in index order, and for each sample the finiteness rule comes before the range rule. Albedo checks bands 0, 1, 2 of a sample before moving to the next sample.
2. Then the vacuum rules, sample by sample in index order. For each sample: a non-zero density with the vacuum state (522), or a zero density with a non-vacuum state (521), then the zero rules for temperature, albedo bands 0 to 2, roughness, and attenuation (523 to 526).

A value of negative zero compares equal to zero, so it passes "greater than or equal to 0" and counts as density 0.

### Frame registry

`registry::decode` and `registry::validate` check one registry in this order and stop at the first failing rule:

1. Length at least 24 (601), magic (602), version (603), reserved `u16` at offset 6 (604), reserved `u32` at offset 20 (605), finite epoch (606), length exactly `24 + 144 * frame_count` (607).
2. Then record by record in file order. For each record: its `frame_id` against the previous record's (612 if lower, 613 if equal), the reserved bytes (611), then the field rules in table order, 614 to 625.

`Registry::new` runs the same rules on frames it has sorted by id: epoch (606), the frame count (608), then for each frame in id order a repeated id (613) and the field rules 614 to 625.

### Hub container

`container::decode_chunk` and `container::decode_registry_chunk` read the whole table before any section, in this order, and stop at the first failing rule:

1. At least 4 bytes (701), then `section_count` not negative (702).
2. Entry by entry in table order, field by field: id length (703 if cut off, 704 if negative), id bytes (703 if cut off, 705 if not UTF-8), `codec_len` (703 if cut off, 706 if negative, 707 if above 0), `data_offset` (703, 708), `data_len` (703, 709).
3. Then entry by entry against the data region that follows the table: an entry starting before the previous entry's end (710) or after it (711; the first entry must start at 0), an entry ending past the data region (712). After the last entry, any bytes left in the data region (713).
4. Then each section in table order, with `matter::decode` against the key, or `registry::decode`. A failing section keeps its own code (101 to 526, or 601 to 625), and its reason starts with `section INDEX:`.

A container with `section_count` 0 and no data is valid and holds no sections. Layer ids are read only to move past them. They are never returned and never appear in a reason.

### Top-level validator

`validate(key, bytes)` and `gx_validate` parse the key first (206). The key `registry` selects the registry rules; any cell key selects the matter rules. Through the C ABI, null pointer checks come first: `key` (207), then the key's UTF-8 (206), then `bytes` (106).

`FrameTree::from_registries` checks the union in this order: epochs (651), ids unique across registries (652), the number of roots (653, 654), every parent present (655), no cycle (656).

Negative zero counts as zero for the root position and velocity rules. Epochs are compared by bit pattern, so `0.0` and `-0.0` differ.

## Codes

### 100 to 199: header (section 4 steps 1 and 2, section 3.2)

| Code | Rule |
|---|---|
| 101 | The input is shorter than the 72-byte header. |
| 102 | The magic is not `0x33 0x47 0x4D 0x53`. |
| 103 | `format_version` is not 1. |
| 104 | A `flags` bit other than bit 0 (EMPTY) and bit 1 (ZSTD) is set. |
| 105 | The reserved `u16` at offset 18 is not 0. |
| 106 | The C ABI was given a null `bytes` pointer with a non-zero `bytes_len`. No byte vector. |

### 200 to 299: key (section 4 step 4, section 2)

| Code | Rule |
|---|---|
| 200 | The cell key passed to the validator is not valid (depth above 31 or a coordinate not below `2^depth`). Reachable only through the Rust API: a key string that parses is always valid. No byte vector. |
| 201 | Header `frame_id` differs from the key. |
| 202 | Header `depth` differs from the key. |
| 203 | Header `cell_x` differs from the key. |
| 204 | Header `cell_y` differs from the key. |
| 205 | Header `cell_z` differs from the key. |
| 206 | The key string given to the top-level validator does not parse as a chunk key (section 2), or through the C ABI its bytes are not UTF-8. |
| 207 | The C ABI was given a null `key` pointer with a non-zero `key_len`. No byte vector. |
| 208 | A cell key was required and `registry` was given. Reachable only through `container::chunk_mass` and the WebAssembly `decode_chunk_mass`. No byte vector. |

### 300 to 399: geometry (section 4 step 5, section 3.2)

| Code | Rule |
|---|---|
| 301 | `cell_edge` is NaN or infinite. |
| 302 | `cell_edge` is zero or negative. |
| 303 | A component of `cell_origin` is NaN or infinite (checked x, y, z). |

### 400 to 499: sample block (section 4 steps 6 and 7, sections 3.3 and 3.4)

| Code | Rule |
|---|---|
| 401 | EMPTY is set and `resolution` is not 0. |
| 402 | EMPTY is set and `sample_block_len` is not 0. |
| 403 | EMPTY and ZSTD are both set. |
| 404 | EMPTY is set and bytes follow the header. |
| 411 | EMPTY is clear and `resolution` is not 1 to 64. |
| 412 | `sample_block_len` is not `resolution^3 * 29`. |
| 413 | ZSTD is clear and the bytes after the header are not exactly `sample_block_len` long. |
| 414 | ZSTD is set and the bytes after the header do not start with a readable zstd frame, or the frame fails to decompress. |
| 415 | ZSTD is set and bytes follow the first zstd frame. |
| 416 | ZSTD is set and the frame decompresses to a length other than `sample_block_len`. |
| 417 | The sample arrays given to `Section::new` do not hold `resolution^3` samples. Reachable only through the Rust API. No byte vector. |

### 500 to 599: channel values (section 4 step 8, section 3.3)

| Code | Rule |
|---|---|
| 501 | A density is NaN or infinite. |
| 502 | A density is negative. |
| 503 | A state byte is above 4. |
| 504 | A temperature is NaN or infinite. |
| 505 | A temperature is negative. |
| 506 | An albedo band is NaN or infinite. |
| 507 | An albedo band is below 0 or above 1. |
| 508 | A roughness is NaN or infinite. |
| 509 | A roughness is below 0 or above 1. |
| 510 | An attenuation is NaN or infinite. |
| 511 | An attenuation is negative. |
| 521 | A sample with density 0 has a state other than vacuum. |
| 522 | A sample with density above 0 has the vacuum state. |
| 523 | A vacuum sample has a temperature other than 0. |
| 524 | A vacuum sample has an albedo band other than 0. |
| 525 | A vacuum sample has a roughness other than 0. |
| 526 | A vacuum sample has an attenuation other than 0. |

### 600 to 649: one frame registry (sections 5.1, 5.2, and 5.3)

Returned by `registry::decode`, `registry::validate`, and `Registry::new`.

| Code | Rule |
|---|---|
| 601 | The input is shorter than the 24-byte registry header. |
| 602 | The magic is not `0x33 0x47 0x52 0x47`. |
| 603 | `format_version` is not 1. |
| 604 | The reserved `u16` at offset 6 is not 0. |
| 605 | The reserved `u32` at offset 20 is not 0. |
| 606 | `epoch` is NaN or infinite. |
| 607 | The input length is not exactly `24 + 144 * frame_count`. |
| 608 | More frames than a `u32` counts were given to `Registry::new`. Reachable only through the Rust API. No byte vector. |
| 611 | A record's 7 reserved bytes at offset 25 are not all 0. |
| 612 | A record's `frame_id` is lower than the previous record's. Records must already be ascending; the decoder never sorts. |
| 613 | Two records in one registry have the same `frame_id`. |
| 614 | `root_extent` is NaN or infinite. |
| 615 | `root_extent` is zero or negative. |
| 616 | `max_depth` is above 31. |
| 617 | `mass` is NaN or infinite. |
| 618 | `mass` is negative. |
| 619 | A component of `position` is NaN or infinite. |
| 620 | A component of `velocity` is NaN or infinite. |
| 621 | A component of `orientation` is NaN or infinite. |
| 622 | `orientation` is not a unit quaternion: its norm differs from 1 by more than `1e-9`. |
| 623 | A component of `angular_velocity` is NaN or infinite. |
| 624 | A root frame (`parent_frame_id` is `0xFFFFFFFFFFFFFFFF`) has a `position` other than 0. |
| 625 | A root frame has a `velocity` other than 0. |

### 650 to 699: union of a build's registries (section 5.2)

Returned by `FrameTree::from_registries`. These rules span several registries, so the per-section validator never reports them; the renderer does.

| Code | Rule |
|---|---|
| 651 | Two registries have different `epoch` values (compared by bit pattern). Empty registries take part. |
| 652 | A `frame_id` is declared by more than one registry. |
| 653 | The union has no root frame. This includes a union with no frames at all. |
| 654 | The union has more than one root frame. |
| 655 | A frame's `parent_frame_id` is not a frame in the union. |
| 656 | Following parents from some frame never reaches the root: a cycle, including a frame that is its own parent. |

### 700 to 799: hub container (section 6)

Returned by `container::decode_chunk` and `container::decode_registry_chunk`. Every integer in the container is a little-endian `i32`.

| Code | Rule |
|---|---|
| 701 | The input is shorter than the 4-byte `section_count`. |
| 702 | `section_count` is negative. |
| 703 | The table ends inside an entry: a field, or the id or codec bytes it announces, runs past the end of the input. |
| 704 | An entry's id length is negative. |
| 705 | An entry's id bytes are not UTF-8. |
| 706 | An entry's `codec_len` is negative. |
| 707 | An entry's `codec_len` is above 0. No codec is defined, so the bytes cannot be read. |
| 708 | An entry's `data_offset` is negative. |
| 709 | An entry's `data_len` is negative. |
| 710 | An entry starts before the end of the previous entry: entries overlap or are out of order. |
| 711 | An entry starts after the end of the previous entry (or the first entry after 0), leaving bytes no entry owns. |
| 712 | An entry ends past the end of the data region. |
| 713 | Bytes follow the end of the last entry in the data region. |

### 800 to 899: compositing (section 3.5)

Returned by `composite`. These are not byte rules and have no vectors.

| Code | Rule |
|---|---|
| 801 | No sections were given. |
| 802 | The sections do not all have the same key. |
| 803 | The sections do not all have the same origin. |
| 804 | The sections do not all have the same edge. |

A composite whose summed density overflows `f32` fails with 501, because the result is checked like any other section.

## Short names

Each code has a short name, the identifier of its constant in `error::codes`, such as `BAD_MAGIC` for 102. `error::code_name` returns it in Rust and `gx_error_name` through the C ABI.

## C ABI contract

Declared in `include/gx_core.h` (`matter-format.md` section 7):

- `uint32_t gx_format_version(void)` returns the format version, 1.
- `int32_t gx_validate(key, key_len, bytes, bytes_len, err_buf, err_cap)` returns 0 on success and the code from this table on failure. `key` is UTF-8 and need not be NUL terminated.
- On failure the reason is written to `err_buf` as NUL-terminated UTF-8, truncated to at most `err_cap - 1` bytes plus the NUL. Truncation never splits a UTF-8 sequence, so the written prefix may be shorter than `err_cap - 1` bytes. A reason that fits is written whole. With `err_cap` 1 only the NUL is written.
- With a null `err_buf` or a zero `err_cap`, nothing is written and the code is still returned. On success `err_buf` is never touched.
- A null pointer with length 0 is an empty input. A null `key` with a non-zero `key_len` returns 207 and a null `bytes` with a non-zero `bytes_len` returns 106: never undefined behavior.
- `const char* gx_error_name(int32_t code)` returns a static NUL-terminated short name, or the empty string for an unknown code (including 0). It never returns NULL and the string must not be freed.
- Every function is stateless and thread safe.

`tools/abi-check/abi_check.c` exercises this contract from C against the built shared library; `scripts/ci.sh` builds and runs it.

## Conformance vectors

`conformance/matter/invalid/` holds at least one file for every code above that bytes can trigger (every code from 101 to 526 except 106, 200, 206, 207, 208, and 417, which bytes alone cannot trigger). Files are named `CODE-RULE.bin`. `index.json` gives the key every file is validated under and maps each file name to its expected code.

`conformance/registry/invalid/` holds at least one file for every code from 601 to 625 except 608, named `CODE-RULE.bin`, with `index.json` mapping each file name to its expected code. `conformance/registry/union/` holds registries that are each valid alone, and its `index.json` lists cases: which files form the union, in order, and either the expected tree (root, frame count, every path to the root) or the expected code. There is at least one case for every code from 651 to 656.

`conformance/container/invalid/` holds one file for every code from 701 to 713, named `CODE-RULE.bin`, plus a container whose table is sound but whose first section fails with 102. `index.json` gives the key every file is decoded under and maps each file name to its code. `conformance/container/valid/` holds a matter chunk and a registry chunk with the expected decode results.
