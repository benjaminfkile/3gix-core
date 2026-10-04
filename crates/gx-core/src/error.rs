//! Validation errors and the stable numeric code table.
//!
//! Every rule in `matter-format.md` section 4 (validation), and the field
//! rules of sections 3.2 (header), 3.3 (sample block), and 3.4 (empty
//! section) it refers to, has its own numeric code. A failing check reports
//! exactly one [`ValidationError`]: the code of the first rule that failed and
//! a human-readable reason naming the offending field.
//!
//! Codes are grouped in ranges:
//!
//! | Range | Rules |
//! |---|---|
//! | 100 to 199 | header: length, magic, version, flags, reserved fields |
//! | 200 to 299 | key: the header must match the chunk key |
//! | 300 to 399 | geometry: cell edge and origin |
//! | 400 to 499 | sample block: resolution, lengths, empty rules, zstd |
//! | 500 to 599 | channel values, including the vacuum rules |
//! | 600 to 699 | reserved for the frame registry (section 5) |
//! | 700 to 799 | reserved for the hub container (section 6) |
//! | 800 to 899 | compositing (section 3.5) |
//!
//! The full table with the check order lives in `docs/errors.md`. A code
//! never changes meaning once published. New rules get new codes.

use core::fmt;

/// A failed validation: a stable numeric code and a human-readable reason.
///
/// Compare on [`ValidationError::code`]. The reason is for people and may
/// change wording between releases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    /// Stable numeric code from the table in this module and `docs/errors.md`.
    pub code: u16,
    /// Human-readable reason naming the offending field and, for sample
    /// rules, the sample index and channel.
    pub reason: String,
}

impl ValidationError {
    /// Builds an error from a code and a reason.
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    /// Writes `code CODE: REASON`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "code {}: {}", self.code, self.reason)
    }
}

impl std::error::Error for ValidationError {}

/// The numeric codes. Each constant documents its rule and spec section.
pub mod codes {
    // 100 to 199: header (section 4 steps 1 and 2, section 3.2).

    /// Fewer than 72 bytes, so there is no complete header.
    pub const HEADER_TOO_SHORT: u16 = 101;
    /// The magic is not `0x33 0x47 0x4D 0x53`.
    pub const BAD_MAGIC: u16 = 102;
    /// `format_version` is not 1.
    pub const UNSUPPORTED_VERSION: u16 = 103;
    /// A `flags` bit other than bit 0 (EMPTY) and bit 1 (ZSTD) is set.
    pub const UNKNOWN_FLAGS: u16 = 104;
    /// The reserved `u16` at offset 18 is not 0.
    pub const RESERVED_NONZERO: u16 = 105;

    // 200 to 299: key (section 4 step 4, section 2).

    /// The chunk key given to the validator is not a valid cell key
    /// (depth above 31 or a coordinate not below `2^depth`). Reachable only
    /// through the Rust API, since a key string that parses is always valid.
    pub const KEY_INVALID: u16 = 200;
    /// Header `frame_id` differs from the key.
    pub const FRAME_ID_MISMATCH: u16 = 201;
    /// Header `depth` differs from the key.
    pub const DEPTH_MISMATCH: u16 = 202;
    /// Header `cell_x` differs from the key.
    pub const CELL_X_MISMATCH: u16 = 203;
    /// Header `cell_y` differs from the key.
    pub const CELL_Y_MISMATCH: u16 = 204;
    /// Header `cell_z` differs from the key.
    pub const CELL_Z_MISMATCH: u16 = 205;

    // 300 to 399: geometry (section 4 step 5, section 3.2).

    /// `cell_edge` is NaN or infinite.
    pub const EDGE_NOT_FINITE: u16 = 301;
    /// `cell_edge` is zero or negative.
    pub const EDGE_NOT_POSITIVE: u16 = 302;
    /// A component of `cell_origin` is NaN or infinite.
    pub const ORIGIN_NOT_FINITE: u16 = 303;

    // 400 to 499: sample block (section 4 steps 6 and 7, sections 3.3, 3.4).

    /// EMPTY is set and `resolution` is not 0.
    pub const EMPTY_RESOLUTION_NONZERO: u16 = 401;
    /// EMPTY is set and `sample_block_len` is not 0.
    pub const EMPTY_BLOCK_LEN_NONZERO: u16 = 402;
    /// EMPTY and ZSTD are both set.
    pub const EMPTY_ZSTD_SET: u16 = 403;
    /// EMPTY is set and bytes follow the header.
    pub const EMPTY_TRAILING_BYTES: u16 = 404;
    /// EMPTY is clear and `resolution` is not 1 to 64.
    pub const RESOLUTION_OUT_OF_RANGE: u16 = 411;
    /// `sample_block_len` is not `n^3 * 29`.
    pub const BLOCK_LEN_MISMATCH: u16 = 412;
    /// ZSTD is clear and the bytes after the header are not exactly
    /// `sample_block_len` long.
    pub const RAW_LENGTH_MISMATCH: u16 = 413;
    /// ZSTD is set and the bytes after the header do not start with a
    /// readable zstd frame, or the frame fails to decompress.
    pub const ZSTD_FRAME_INVALID: u16 = 414;
    /// ZSTD is set and bytes follow the first zstd frame.
    pub const ZSTD_TRAILING_BYTES: u16 = 415;
    /// ZSTD is set and the frame decompresses to a length other than
    /// `sample_block_len`.
    pub const ZSTD_LENGTH_MISMATCH: u16 = 416;
    /// The sample arrays given to the Rust constructor do not hold `n^3`
    /// samples. Reachable only through the Rust API, since the decoder sizes
    /// the arrays itself.
    pub const SAMPLE_COUNT_MISMATCH: u16 = 417;

    // 500 to 599: channel values (section 4 step 8, section 3.3).

    /// A density is NaN or infinite.
    pub const DENSITY_NOT_FINITE: u16 = 501;
    /// A density is negative.
    pub const DENSITY_NEGATIVE: u16 = 502;
    /// A state byte is above 4.
    pub const STATE_OUT_OF_RANGE: u16 = 503;
    /// A temperature is NaN or infinite.
    pub const TEMPERATURE_NOT_FINITE: u16 = 504;
    /// A temperature is negative.
    pub const TEMPERATURE_NEGATIVE: u16 = 505;
    /// An albedo band is NaN or infinite.
    pub const ALBEDO_NOT_FINITE: u16 = 506;
    /// An albedo band is outside 0 to 1.
    pub const ALBEDO_OUT_OF_RANGE: u16 = 507;
    /// A roughness is NaN or infinite.
    pub const ROUGHNESS_NOT_FINITE: u16 = 508;
    /// A roughness is outside 0 to 1.
    pub const ROUGHNESS_OUT_OF_RANGE: u16 = 509;
    /// An attenuation is NaN or infinite.
    pub const ATTENUATION_NOT_FINITE: u16 = 510;
    /// An attenuation is negative.
    pub const ATTENUATION_NEGATIVE: u16 = 511;
    /// A sample with density 0 has a state other than vacuum.
    pub const VACUUM_STATE_NOT_VACUUM: u16 = 521;
    /// A sample with density above 0 has the vacuum state.
    pub const MATTER_STATE_VACUUM: u16 = 522;
    /// A vacuum sample has a temperature other than 0.
    pub const VACUUM_TEMPERATURE_NONZERO: u16 = 523;
    /// A vacuum sample has an albedo band other than 0.
    pub const VACUUM_ALBEDO_NONZERO: u16 = 524;
    /// A vacuum sample has a roughness other than 0.
    pub const VACUUM_ROUGHNESS_NONZERO: u16 = 525;
    /// A vacuum sample has an attenuation other than 0.
    pub const VACUUM_ATTENUATION_NONZERO: u16 = 526;

    // 600 to 699: reserved for the frame registry (section 5).
    // 700 to 799: reserved for the hub container (section 6).

    // 800 to 899: compositing (section 3.5). Not byte rules.

    /// `composite` was given no sections.
    pub const COMPOSITE_NO_INPUT: u16 = 801;
    /// Sections given to `composite` have different keys.
    pub const COMPOSITE_KEY_MISMATCH: u16 = 802;
    /// Sections given to `composite` have different origins.
    pub const COMPOSITE_ORIGIN_MISMATCH: u16 = 803;
    /// Sections given to `composite` have different edges.
    pub const COMPOSITE_EDGE_MISMATCH: u16 = 804;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        let e = ValidationError::new(codes::BAD_MAGIC, "magic is 00 00 00 00");
        assert_eq!(e.to_string(), "code 102: magic is 00 00 00 00");
        assert_eq!(e.code, 102);
    }
}
