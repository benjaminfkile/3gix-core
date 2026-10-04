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
| 600 to 699 | Frame registry (`matter-format.md` section 5) | reserved |
| 700 to 799 | Hub container (`matter-format.md` section 6) | reserved |
| 800 to 899 | Compositing (`matter-format.md` section 3.5), not byte rules | in use |

## Check order

`decode` and `validate` run `matter-format.md` section 4 steps 1, 2, and 4 through 8 in that order and stop at the first failing rule. Step 3, the registry branch, belongs to the top-level validator that dispatches on the key. Within each step the checks run in the order of the table below. For channel values (step 8):

1. Each channel's own rule, channel by channel in wire order (density, state, temperature, albedo, roughness, attenuation). Within a channel, samples run in index order, and for each sample the finiteness rule comes before the range rule. Albedo checks bands 0, 1, 2 of a sample before moving to the next sample.
2. Then the vacuum rules, sample by sample in index order. For each sample: a non-zero density with the vacuum state (522), or a zero density with a non-vacuum state (521), then the zero rules for temperature, albedo bands 0 to 2, roughness, and attenuation (523 to 526).

A value of negative zero compares equal to zero, so it passes "greater than or equal to 0" and counts as density 0.

## Codes

### 100 to 199: header (section 4 steps 1 and 2, section 3.2)

| Code | Rule |
|---|---|
| 101 | The input is shorter than the 72-byte header. |
| 102 | The magic is not `0x33 0x47 0x4D 0x53`. |
| 103 | `format_version` is not 1. |
| 104 | A `flags` bit other than bit 0 (EMPTY) and bit 1 (ZSTD) is set. |
| 105 | The reserved `u16` at offset 18 is not 0. |

### 200 to 299: key (section 4 step 4, section 2)

| Code | Rule |
|---|---|
| 200 | The cell key passed to the validator is not valid (depth above 31 or a coordinate not below `2^depth`). Reachable only through the Rust API: a key string that parses is always valid. No byte vector. |
| 201 | Header `frame_id` differs from the key. |
| 202 | Header `depth` differs from the key. |
| 203 | Header `cell_x` differs from the key. |
| 204 | Header `cell_y` differs from the key. |
| 205 | Header `cell_z` differs from the key. |

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

### 600 to 699: frame registry

Reserved for section 5. Not yet assigned.

### 700 to 799: hub container

Reserved for section 6. Not yet assigned.

### 800 to 899: compositing (section 3.5)

Returned by `composite`. These are not byte rules and have no vectors.

| Code | Rule |
|---|---|
| 801 | No sections were given. |
| 802 | The sections do not all have the same key. |
| 803 | The sections do not all have the same origin. |
| 804 | The sections do not all have the same edge. |

A composite whose summed density overflows `f32` fails with 501, because the result is checked like any other section.

## Conformance vectors

`conformance/matter/invalid/` holds at least one file for every code above that bytes can trigger (every code from 101 to 526 except 200 and 417). Files are named `CODE-RULE.bin`. `index.json` gives the key every file is validated under and maps each file name to its expected code.
