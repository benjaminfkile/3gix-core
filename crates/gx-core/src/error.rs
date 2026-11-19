//! Validation errors and the stable numeric code table.
//!
//! Every rule in `matter-format.md` section 4 (validation), and the field
//! rules of sections 3.2 (header), 3.3 (sample block), and 3.4 (empty
//! section) it refers to, has its own numeric code. So does every rule of
//! section 5 (frame registry, format version 2), both the rules one registry can be checked
//! against and the rules across the union of a build's registries. A failing check reports
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
//! | 600 to 649 | frame registry, one registry (section 5.1 to 5.3) |
//! | 650 to 699 | frame registry, union of registries (section 5.2) |
//! | 700 to 799 | hub container (section 6) |
//! | 800 to 899 | compositing (section 3.5) |
//! | 900 to 929 | source manifest (`client-compute.md` section 5.1) |
//! | 930 to 959 | module request, tile, and source id bytes (`client-compute.md` section 4) |
//! | 960 to 979 | section addresses and span indices (`client-compute.md` sections 2.4 and 3) |
//! | 980 to 999 | surface tiles (`matter-format.md` section 10) |
//! | 1000 to 1019 | epoch indexes and the epoch rule (`client-compute.md` section 5.4) |
//! | 1020 to 1039 | frame records (`matter-format.md` section 11) |
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

/// Defines every code constant and the [`codes::ALL`] table from one list,
/// so a constant can never be missing from the table.
macro_rules! code_table {
    ($($(#[$meta:meta])* $name:ident = $code:literal;)*) => {
        $($(#[$meta])* pub const $name: u16 = $code;)*

        /// Every code with its short name (the constant's identifier), in
        /// ascending code order.
        pub const ALL: &[(u16, &str)] = &[$(($code, stringify!($name))),*];

        /// [`ALL`] with each name NUL terminated, for the C ABI.
        pub(crate) const ALL_C: &[(u16, &str)] =
            &[$(($code, concat!(stringify!($name), "\0"))),*];
    };
}

/// The numeric codes. Each constant documents its rule and spec section.
pub mod codes {
    code_table! {
        // 100 to 199: header (section 4 steps 1 and 2, section 3.2).

        /// Fewer than 72 bytes, so there is no complete header.
        HEADER_TOO_SHORT = 101;
        /// The magic is not `0x33 0x47 0x4D 0x53`.
        BAD_MAGIC = 102;
        /// `format_version` is neither 3, the version this library
        /// implements, nor a retired version (which has its own code, 107).
        UNSUPPORTED_VERSION = 103;
        /// A `flags` bit other than bit 0 (EMPTY) and bit 1 (ZSTD) is set.
        UNKNOWN_FLAGS = 104;
        /// The reserved `u16` at offset 18 is not 0.
        RESERVED_NONZERO = 105;
        /// The C ABI was given a null `bytes` pointer with a non-zero
        /// `bytes_len`.
        BYTES_POINTER_NULL = 106;
        /// `format_version` is 1 or 2, retired with format version 3
        /// (section 9): version 2 has no occupancy channel and version 1 no
        /// radiating temperature, so neither is read.
        VERSION_RETIRED = 107;

        // 200 to 299: key (section 4 step 4, section 2).

        /// The chunk key given to the validator is not a valid cell key
        /// (depth above 31 or a coordinate not below `2^depth`). Reachable only
        /// through the Rust API, since a key string that parses is always valid.
        KEY_INVALID = 200;
        /// Header `frame_id` differs from the key.
        FRAME_ID_MISMATCH = 201;
        /// Header `depth` differs from the key.
        DEPTH_MISMATCH = 202;
        /// Header `cell_x` differs from the key.
        CELL_X_MISMATCH = 203;
        /// Header `cell_y` differs from the key.
        CELL_Y_MISMATCH = 204;
        /// Header `cell_z` differs from the key.
        CELL_Z_MISMATCH = 205;
        /// The key string given to the top-level validator is not a chunk key:
        /// it does not parse under section 2, or (through the C ABI) its bytes
        /// are not UTF-8.
        KEY_MALFORMED = 206;
        /// The C ABI was given a null `key` pointer with a non-zero `key_len`.
        KEY_POINTER_NULL = 207;
        /// A cell key was required and the key is `registry`. Reachable only
        /// through `container::chunk_mass` and the WebAssembly export built on
        /// it.
        KEY_NOT_CELL = 208;

        // 300 to 399: geometry (section 4 step 5, section 3.2).

        /// `cell_edge` is NaN or infinite.
        EDGE_NOT_FINITE = 301;
        /// `cell_edge` is zero or negative.
        EDGE_NOT_POSITIVE = 302;
        /// A component of `cell_origin` is NaN or infinite.
        ORIGIN_NOT_FINITE = 303;

        // 400 to 499: sample block (section 4 steps 6 and 7, sections 3.3, 3.4).

        /// EMPTY is set and `resolution` is not 0.
        EMPTY_RESOLUTION_NONZERO = 401;
        /// EMPTY is set and `sample_block_len` is not 0.
        EMPTY_BLOCK_LEN_NONZERO = 402;
        /// EMPTY and ZSTD are both set.
        EMPTY_ZSTD_SET = 403;
        /// EMPTY is set and bytes follow the header.
        EMPTY_TRAILING_BYTES = 404;
        /// EMPTY is clear and `resolution` is not 1 to 64.
        RESOLUTION_OUT_OF_RANGE = 411;
        /// `sample_block_len` is not `n^3 * 29`.
        BLOCK_LEN_MISMATCH = 412;
        /// ZSTD is clear and the bytes after the header are not exactly
        /// `sample_block_len` long.
        RAW_LENGTH_MISMATCH = 413;
        /// ZSTD is set and the bytes after the header do not start with a
        /// readable zstd frame, or the frame fails to decompress.
        ZSTD_FRAME_INVALID = 414;
        /// ZSTD is set and bytes follow the first zstd frame.
        ZSTD_TRAILING_BYTES = 415;
        /// ZSTD is set and the frame decompresses to a length other than
        /// `sample_block_len`.
        ZSTD_LENGTH_MISMATCH = 416;
        /// The sample arrays given to the Rust constructor do not hold `n^3`
        /// samples. Reachable only through the Rust API, since the decoder sizes
        /// the arrays itself.
        SAMPLE_COUNT_MISMATCH = 417;

        // 500 to 599: channel values (section 4 step 8, section 3.3).

        /// A density is NaN or infinite.
        DENSITY_NOT_FINITE = 501;
        /// A density is negative.
        DENSITY_NEGATIVE = 502;
        /// A state byte is above 4.
        STATE_OUT_OF_RANGE = 503;
        /// A temperature is NaN or infinite.
        TEMPERATURE_NOT_FINITE = 504;
        /// A temperature is negative.
        TEMPERATURE_NEGATIVE = 505;
        /// An albedo band is NaN or infinite.
        ALBEDO_NOT_FINITE = 506;
        /// An albedo band is outside 0 to 1.
        ALBEDO_OUT_OF_RANGE = 507;
        /// A roughness is NaN or infinite.
        ROUGHNESS_NOT_FINITE = 508;
        /// A roughness is outside 0 to 1.
        ROUGHNESS_OUT_OF_RANGE = 509;
        /// An attenuation is NaN or infinite.
        ATTENUATION_NOT_FINITE = 510;
        /// An attenuation is negative.
        ATTENUATION_NEGATIVE = 511;
        /// A radiating temperature is NaN or infinite.
        RADIATING_TEMPERATURE_NOT_FINITE = 512;
        /// A radiating temperature is negative.
        RADIATING_TEMPERATURE_NEGATIVE = 513;
        /// An occupancy is NaN or infinite.
        OCCUPANCY_NOT_FINITE = 514;
        /// An occupancy is outside 0 to 1.
        OCCUPANCY_OUT_OF_RANGE = 515;
        /// A sample with density 0 has a state other than vacuum.
        VACUUM_STATE_NOT_VACUUM = 521;
        /// A sample with density above 0 has the vacuum state.
        MATTER_STATE_VACUUM = 522;
        /// A vacuum sample has a temperature other than 0.
        VACUUM_TEMPERATURE_NONZERO = 523;
        /// A vacuum sample has an albedo band other than 0.
        VACUUM_ALBEDO_NONZERO = 524;
        /// A vacuum sample has a roughness other than 0.
        VACUUM_ROUGHNESS_NONZERO = 525;
        /// A vacuum sample has an attenuation other than 0.
        VACUUM_ATTENUATION_NONZERO = 526;
        /// A vacuum sample has a radiating temperature other than 0.
        VACUUM_RADIATING_TEMPERATURE_NONZERO = 527;
        /// A vacuum sample has an occupancy other than 0.
        VACUUM_OCCUPANCY_NONZERO = 528;
        /// A sample with density above 0 has occupancy 0.
        MATTER_OCCUPANCY_ZERO = 529;

        // 600 to 649: one frame registry (sections 5.1, 5.2, and 5.3).

        /// Fewer than 24 bytes, so there is no complete registry header.
        REGISTRY_HEADER_TOO_SHORT = 601;
        /// The registry magic is not `0x33 0x47 0x52 0x47`.
        REGISTRY_BAD_MAGIC = 602;
        /// Registry `format_version` is neither 2, the version this library
        /// implements, nor the retired version 1 (which has its own code, 610).
        REGISTRY_UNSUPPORTED_VERSION = 603;
        /// The reserved `u16` at header offset 6 is not 0.
        REGISTRY_RESERVED_U16_NONZERO = 604;
        /// The reserved `u32` at header offset 20 is not 0.
        REGISTRY_RESERVED_U32_NONZERO = 605;
        /// `epoch` is NaN or infinite.
        REGISTRY_EPOCH_NOT_FINITE = 606;
        /// The input length is not exactly `24 + 184 * frame_count`.
        REGISTRY_LENGTH_MISMATCH = 607;
        /// More frames than a `u32` `frame_count` can hold were given to the Rust
        /// constructor. Reachable only through the Rust API.
        REGISTRY_TOO_MANY_FRAMES = 608;
        /// A registry with `frame_count` 0 has an `epoch` other than 0
        /// (section 5.3).
        REGISTRY_EMPTY_EPOCH_NONZERO = 609;
        /// Registry `format_version` is 1, retired with format version 2
        /// (section 9): its 144-byte records are no longer read.
        REGISTRY_VERSION_RETIRED = 610;
        /// A record's 7 reserved bytes at offset 25 are not all 0.
        FRAME_RESERVED_NONZERO = 611;
        /// A record's `frame_id` is lower than the previous record's: records are
        /// not sorted ascending.
        FRAME_NOT_SORTED = 612;
        /// Two records in one registry have the same `frame_id`.
        FRAME_DUPLICATE_ID = 613;
        /// `root_extent` is NaN or infinite.
        FRAME_EXTENT_NOT_FINITE = 614;
        /// `root_extent` is zero or negative.
        FRAME_EXTENT_NOT_POSITIVE = 615;
        /// `max_depth` is above 31.
        FRAME_MAX_DEPTH_OUT_OF_RANGE = 616;
        /// `mass` is NaN or infinite.
        FRAME_MASS_NOT_FINITE = 617;
        /// `mass` is negative.
        FRAME_MASS_NEGATIVE = 618;
        /// A component of `position` is NaN or infinite.
        FRAME_POSITION_NOT_FINITE = 619;
        /// A component of `velocity` is NaN or infinite.
        FRAME_VELOCITY_NOT_FINITE = 620;
        /// A component of `orientation` is NaN or infinite.
        FRAME_ORIENTATION_NOT_FINITE = 621;
        /// `orientation` is not a unit quaternion within `1e-9`.
        FRAME_ORIENTATION_NOT_UNIT = 622;
        /// A component of `angular_velocity` is NaN or infinite.
        FRAME_ANGULAR_VELOCITY_NOT_FINITE = 623;
        /// A root frame has a `position` other than 0.
        ROOT_POSITION_NONZERO = 624;
        /// A root frame has a `velocity` other than 0.
        ROOT_VELOCITY_NONZERO = 625;
        /// `j2` is NaN or infinite.
        FRAME_J2_NOT_FINITE = 626;
        /// `j2` is negative.
        FRAME_J2_NEGATIVE = 627;
        /// `reference_radius` is NaN or infinite.
        FRAME_REFERENCE_RADIUS_NOT_FINITE = 628;
        /// `j2` is above 0 and `reference_radius` is zero or negative.
        FRAME_REFERENCE_RADIUS_NOT_POSITIVE = 629;
        /// `j2` is 0 and `reference_radius` is not 0.
        FRAME_REFERENCE_RADIUS_NONZERO = 630;
        /// A component of `principal_moments` is NaN or infinite.
        FRAME_MOMENTS_NOT_FINITE = 631;
        /// `principal_moments` are neither all 0 nor all greater than 0.
        FRAME_MOMENTS_NOT_POSITIVE = 632;
        /// One of the `principal_moments` is greater than the sum of the other
        /// two.
        FRAME_MOMENTS_TRIANGLE = 633;

        // 650 to 699: the union of a build's registries (section 5.2). Not byte
        // rules of one registry.

        /// Two registries in the union that declare frames have different
        /// `epoch` bit patterns.
        UNION_EPOCH_MISMATCH = 651;
        /// A `frame_id` is declared by more than one registry in the union.
        UNION_DUPLICATE_ID = 652;
        /// The union has no root frame (this includes a union with no frames).
        UNION_NO_ROOT = 653;
        /// The union has more than one root frame.
        UNION_MULTIPLE_ROOTS = 654;
        /// A frame's `parent_frame_id` is not a frame in the union.
        UNION_PARENT_MISSING = 655;
        /// Following parents from some frame never reaches the root: the frames
        /// form a cycle.
        UNION_CYCLE = 656;

        // 700 to 799: hub container (section 6). Checked by
        // `container::decode_chunk` and `container::decode_registry_chunk`.

        /// Fewer than 4 bytes, so there is no complete `section_count`.
        CONTAINER_TOO_SHORT = 701;
        /// `section_count` is negative.
        CONTAINER_COUNT_NEGATIVE = 702;
        /// The section table ends inside an entry: a length field or the bytes
        /// it announces run past the end of the input.
        CONTAINER_TABLE_TRUNCATED = 703;
        /// An entry's id length is negative.
        CONTAINER_ID_LEN_NEGATIVE = 704;
        /// An entry's id bytes are not UTF-8.
        CONTAINER_ID_NOT_UTF8 = 705;
        /// An entry's `codec_len` is negative.
        CONTAINER_CODEC_LEN_NEGATIVE = 706;
        /// An entry's `codec_len` is above 0: no codec is defined, so the
        /// section bytes cannot be read.
        CONTAINER_CODEC_UNSUPPORTED = 707;
        /// An entry's `data_offset` is negative.
        CONTAINER_OFFSET_NEGATIVE = 708;
        /// An entry's `data_len` is negative.
        CONTAINER_LENGTH_NEGATIVE = 709;
        /// An entry starts before the end of the previous entry: entries overlap
        /// or are out of order.
        CONTAINER_ENTRY_OVERLAP = 710;
        /// An entry starts after the end of the previous entry, leaving bytes no
        /// entry owns.
        CONTAINER_ENTRY_GAP = 711;
        /// An entry ends past the end of the data region.
        CONTAINER_ENTRY_PAST_END = 712;
        /// Bytes follow the end of the last entry in the data region.
        CONTAINER_TRAILING_BYTES = 713;

        // 800 to 899: compositing (section 3.5). Not byte rules.

        /// `composite` was given no sections.
        COMPOSITE_NO_INPUT = 801;
        /// Sections given to `composite` have different keys.
        COMPOSITE_KEY_MISMATCH = 802;
        /// Sections given to `composite` have different origins.
        COMPOSITE_ORIGIN_MISMATCH = 803;
        /// Sections given to `composite` have different edges.
        COMPOSITE_EDGE_MISMATCH = 804;

        // 900 to 929: source manifest (`client-compute.md` section 5.1).
        // Checked by `manifest::decode`.

        /// Fewer than 12 bytes, so there is no complete manifest header.
        MANIFEST_HEADER_TOO_SHORT = 901;
        /// The manifest magic is not `3GXM` (`0x33 0x47 0x58 0x4D`).
        MANIFEST_BAD_MAGIC = 902;
        /// Manifest `version` is not 1.
        MANIFEST_UNSUPPORTED_VERSION = 903;
        /// The reserved `u16` at manifest offset 6 is not 0.
        MANIFEST_RESERVED_NONZERO = 904;
        /// The input length is not exactly `12 + 48 * entry_count`.
        MANIFEST_LENGTH_MISMATCH = 905;
        /// An entry's `id` is lower than the previous entry's: entries are not
        /// sorted ascending.
        MANIFEST_NOT_SORTED = 906;
        /// Two entries have the same `id`.
        MANIFEST_DUPLICATE_ID = 907;
        /// More entries than a `u32` `entry_count` can hold were given to the
        /// Rust constructor. Reachable only through the Rust API.
        MANIFEST_TOO_MANY_ENTRIES = 908;

        // 930 to 959: module request bytes, tile bytes, and the source id
        // list (`client-compute.md` section 4). Checked by `module_io`.

        /// Fewer than 36 bytes, so there is no complete request header.
        REQUEST_TOO_SHORT = 931;
        /// The request magic is not `3GXQ` (`0x33 0x47 0x58 0x51`).
        REQUEST_BAD_MAGIC = 932;
        /// Request `version` is not 1.
        REQUEST_UNSUPPORTED_VERSION = 933;
        /// The reserved `u16` at request offset 6 is not 0.
        REQUEST_RESERVED_NONZERO = 934;
        /// The input length is not exactly `36 + key_len`.
        REQUEST_LENGTH_MISMATCH = 935;
        /// The request's chunk key is not UTF-8 or not a chunk key.
        REQUEST_KEY_MALFORMED = 936;
        /// Fewer than 4 bytes, so there is no complete tile `count`.
        TILES_TOO_SHORT = 941;
        /// A tile's `id`, `length`, or bytes run past the end of the input.
        TILES_TRUNCATED = 942;
        /// Bytes follow the last tile.
        TILES_TRAILING_BYTES = 943;
        /// A tile's `id` is not greater than the previous tile's: tiles are
        /// not in the ascending, unique order `gx_sources` returns.
        TILES_NOT_ASCENDING = 944;
        /// More tiles than a `u32` counts, or a tile longer than a `u32`
        /// length holds, were given to the encoder. Reachable only through the
        /// Rust API.
        TILES_TOO_LARGE = 945;
        /// A source id list's length is not a multiple of 8.
        SOURCES_LENGTH_NOT_MULTIPLE = 951;
        /// A source id is not greater than the previous one: the list is not
        /// ascending and unique.
        SOURCES_NOT_ASCENDING = 952;

        // 960 to 979: section addresses and span indices
        // (`client-compute.md` sections 2.4 and 3).

        /// A span index was asked for a time before the space's origin.
        SPAN_TIME_BEFORE_ORIGIN = 961;
        /// The C ABI was given a null `module32` or `out32` pointer.
        ADDRESS_POINTER_NULL = 962;

        // 980 to 999: surface tiles (`matter-format.md` section 10).

        /// Fewer bytes than the 64-byte surface tile header.
        SURFACE_TOO_SHORT = 980;
        /// The first four bytes are not the surface tile magic `3GSF`.
        SURFACE_BAD_MAGIC = 981;
        /// `format_version` is not the surface tile version this library reads.
        SURFACE_BAD_VERSION = 982;
        /// A reserved field or flag bit is not 0.
        SURFACE_RESERVED_NONZERO = 983;
        /// The header's frame, face, level, or tile coordinates differ from
        /// the key.
        SURFACE_KEY_MISMATCH = 984;
        /// A reference radius is not finite and positive, or the polar radius
        /// exceeds the equatorial one by more than a factor of two either way.
        SURFACE_RADIUS = 985;
        /// An EMPTY tile has radii, a block length, ZSTD, or bytes after the
        /// header.
        SURFACE_EMPTY_INVALID = 986;
        /// `block_len` is not the block length of section 10.3, or the bytes
        /// after the header are not exactly that block (or one zstd frame of
        /// it).
        SURFACE_BLOCK_LENGTH = 987;
        /// A height is not finite, or a temperature is not finite and at
        /// least 0.
        SURFACE_VALUE = 988;
        /// `height_floor` is not finite, or reaches the reference shape's
        /// center (it must be above minus the smaller radius).
        SURFACE_FLOOR = 989;
        /// A height or parent height is below `height_floor`.
        SURFACE_BELOW_FLOOR = 990;

        // 1000 to 1019: epoch indexes and the epoch rule
        // (`client-compute.md` section 5.4). Checked by `epochs`.

        /// Fewer than 20 bytes, so there is no complete epoch index header.
        EPOCHS_HEADER_TOO_SHORT = 1001;
        /// The epoch index magic is not `3GXE` (`0x33 0x47 0x58 0x45`).
        EPOCHS_BAD_MAGIC = 1002;
        /// Epoch index `version` is not 1.
        EPOCHS_UNSUPPORTED_VERSION = 1003;
        /// The reserved `u16` at epoch index offset 6 is not 0.
        EPOCHS_RESERVED_NONZERO = 1004;
        /// `epoch seconds` is 0.
        EPOCHS_ZERO_EPOCH_SECONDS = 1005;
        /// The input length is not exactly `20 + 40 * entry_count`.
        EPOCHS_LENGTH_MISMATCH = 1006;
        /// An entry's epoch index is not greater than the previous entry's:
        /// entries are not strictly ascending.
        EPOCHS_NOT_ASCENDING = 1007;
        /// More entries than a `u32` `entry_count` can hold were given to the
        /// Rust constructor. Reachable only through the Rust API.
        EPOCHS_TOO_MANY_ENTRIES = 1008;
        /// The epoch rule was asked for a layer with `span_seconds` 0.
        EPOCHS_SPAN_SECONDS_ZERO = 1009;
        /// The epoch length is not a whole multiple of the layer's
        /// `span_seconds`, so a span could straddle two epochs.
        EPOCHS_SPAN_NOT_DIVISOR = 1010;
        /// The C ABI was given a null `out32` pointer.
        EPOCHS_POINTER_NULL = 1011;

        // 1020 to 1039: frame records (`matter-format.md` section 11.3).
        // Checked by `frame_record`.

        /// The record is not exactly 88 bytes.
        FRAME_RECORD_LENGTH = 1021;
        /// The magic is not `3GFR` (`0x33 0x47 0x46 0x52`).
        FRAME_RECORD_BAD_MAGIC = 1022;
        /// `format_version` is not 1.
        FRAME_RECORD_UNSUPPORTED_VERSION = 1023;
        /// A `flags` bit other than bit 0 (EMPTY) is set.
        FRAME_RECORD_UNKNOWN_FLAGS = 1024;
        /// A reserved byte at offsets 33 to 39 is not 0.
        FRAME_RECORD_RESERVED_NONZERO = 1025;
        /// `frame_id` differs from the key.
        FRAME_RECORD_ID_MISMATCH = 1026;
        /// EMPTY is set and a field other than `frame_id` is not 0.
        FRAME_RECORD_EMPTY_NONZERO = 1027;
        /// `root_extent` is not finite and greater than 0, `max_depth` is
        /// above 31, or `mass` is not finite and at least 0 (section 5.2).
        FRAME_RECORD_FIELD = 1028;
        /// `j2` is not finite and at least 0, `reference_radius` breaks its
        /// rule against `j2`, or `principal_moments` are not finite, not all 0
        /// or all greater than 0, or break the triangle rule (section 5.2).
        FRAME_RECORD_SHAPE = 1029;
    }
}

/// The short name of `code`, the identifier of its constant in [`codes`],
/// such as `BAD_MAGIC` for 102. `None` for a code that is not assigned.
pub fn code_name(code: u16) -> Option<&'static str> {
    codes::ALL
        .iter()
        .find(|&&(c, _)| c == code)
        .map(|&(_, n)| n)
}

/// [`code_name`] with a NUL terminator, or `"\0"` for an unknown code.
pub(crate) fn code_name_c(code: i64) -> &'static str {
    codes::ALL_C
        .iter()
        .find(|&&(c, _)| i64::from(c) == code)
        .map_or("\0", |&(_, n)| n)
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

    #[test]
    fn table_is_ascending_and_unique() {
        assert!(codes::ALL.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(codes::ALL.len(), codes::ALL_C.len());
        for (&(c, n), &(cc, nc)) in codes::ALL.iter().zip(codes::ALL_C) {
            assert_eq!(c, cc);
            assert_eq!(nc.strip_suffix('\0'), Some(n));
        }
    }

    #[test]
    fn names() {
        assert_eq!(code_name(codes::BAD_MAGIC), Some("BAD_MAGIC"));
        assert_eq!(
            code_name(codes::CONTAINER_TRAILING_BYTES),
            Some("CONTAINER_TRAILING_BYTES")
        );
        assert_eq!(code_name(0), None);
        assert_eq!(code_name(799), None);
        assert_eq!(code_name_c(102), "BAD_MAGIC\0");
        assert_eq!(code_name_c(-1), "\0");
        assert_eq!(code_name_c(70000), "\0");
    }
}
