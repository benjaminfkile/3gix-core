//! Hub container conformance vectors, written under `container/`
//! (`matter-format.md` sections 6 and 8).
//!
//! - `valid/matter-chunk.bin` holds two matter sections of one cell, the
//!   bytes of `matter/valid/res2-shared-cell.bin` and
//!   `matter/valid/res16-zstd.bin` in that order, under different layer ids.
//!   `valid/matter-chunk.json` gives the key, the section count, each
//!   section's source file, resolution, and mass, the sum of the masses in
//!   table order, and the resolution and mass of their composite.
//! - `valid/registry-chunk.bin` holds `registry/union/valid-a.bin` and
//!   `registry/union/valid-b.bin`. `valid/registry-chunk.json` gives the
//!   section count, each registry's source file and frame count, and the root
//!   and frame count of their union.
//! - `invalid/CODE-RULE.bin` breaks one container rule of
//!   `valid/matter-chunk.bin`, one file per code from 701 to 713, plus a
//!   container whose table is sound but whose section fails, to show the
//!   section's own code passes through. `invalid/index.json` gives the key
//!   every invalid file is decoded under and maps each file name to its code.

use std::collections::BTreeMap;

use gx_core::container::{decode_chunk, encode_chunk};
use gx_core::error::codes;
use gx_core::matter::{self, Compression, Section};
use gx_core::registry::{self, FrameTree};
use serde_json::json;

use crate::registry::{valid_a, valid_b};
use crate::{number, pretty, res16_ball, res2_shared_cell, File};

/// The layer ids the vectors carry. Decoders read past them and drop them.
const LAYER_IDS: [&str; 2] = ["layer-0001", "layer-0002"];

/// The two matter sections of the valid chunk with their source files and
/// encoded bytes.
fn matter_sections() -> [(&'static str, Section, Vec<u8>); 2] {
    let coarse = res2_shared_cell();
    let fine = res16_ball();
    let coarse_bytes = matter::encode(&coarse, Compression::None);
    let fine_bytes = matter::encode(&fine, Compression::Zstd);
    [
        ("matter/valid/res2-shared-cell.bin", coarse, coarse_bytes),
        ("matter/valid/res16-zstd.bin", fine, fine_bytes),
    ]
}

/// The valid matter chunk's bytes.
fn matter_chunk() -> Vec<u8> {
    let s = matter_sections();
    encode_chunk(&[&s[0].2, &s[1].2], &LAYER_IDS)
}

fn valid() -> Vec<File> {
    let s = matter_sections();
    let key = s[0].1.key();
    assert_eq!(key, s[1].1.key(), "both sections share one cell");
    let mass_sum: f64 = s.iter().map(|(_, sec, _)| sec.mass().value()).sum();
    let composite = matter::composite(&[&s[0].1, &s[1].1]).expect("sections composite");
    let matter_json = json!({
        "key": key.to_string(),
        "section_count": s.len(),
        "sections": s.iter().map(|(src, sec, _)| json!({
            "source": src,
            "resolution": sec.resolution(),
            "mass": number(sec.mass().value()),
        })).collect::<Vec<_>>(),
        "mass_sum": number(mass_sum),
        "composite": {
            "resolution": composite.resolution(),
            "mass": number(composite.mass().value()),
        },
    });

    let regs = [
        ("registry/union/valid-a.bin", valid_a()),
        ("registry/union/valid-b.bin", valid_b()),
    ];
    let encoded: Vec<Vec<u8>> = regs.iter().map(|(_, r)| registry::encode(r)).collect();
    let tree = FrameTree::from_registries(&[regs[0].1.clone(), regs[1].1.clone()])
        .expect("the pair forms a tree");
    let registry_json = json!({
        "key": "registry",
        "section_count": regs.len(),
        "sections": regs.iter().map(|(src, r)| json!({
            "source": src,
            "frame_count": r.frames().len(),
        })).collect::<Vec<_>>(),
        "union": {
            "root": tree.root().frame_id,
            "frame_count": tree.frames().len(),
        },
    });

    vec![
        ("container/valid/matter-chunk.bin".into(), matter_chunk()),
        (
            "container/valid/matter-chunk.json".into(),
            pretty(&matter_json),
        ),
        (
            "container/valid/registry-chunk.bin".into(),
            encode_chunk(&[&encoded[0], &encoded[1]], &LAYER_IDS),
        ),
        (
            "container/valid/registry-chunk.json".into(),
            pretty(&registry_json),
        ),
    ]
}

fn put_i32(b: &mut [u8], at: usize, v: i32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Byte offsets of the table fields of the valid matter chunk.
mod at {
    use super::LAYER_IDS;

    /// Start of entry `i`.
    fn entry(i: usize) -> usize {
        4 + LAYER_IDS[..i].iter().map(|id| 16 + id.len()).sum::<usize>()
    }
    pub fn id_len(i: usize) -> usize {
        entry(i)
    }
    pub fn id(i: usize) -> usize {
        entry(i) + 4
    }
    pub fn codec_len(i: usize) -> usize {
        id(i) + LAYER_IDS[i].len()
    }
    pub fn offset(i: usize) -> usize {
        codec_len(i) + 4
    }
    pub fn len(i: usize) -> usize {
        offset(i) + 4
    }
}

fn invalid() -> Vec<File> {
    let base = matter_chunk();
    let first_len = i32::try_from(matter_sections()[0].2.len()).expect("fits i32");

    let mut vectors: Vec<(&str, u16, Vec<u8>)> = Vec::new();
    let mut add = |name: &'static str, code: u16, f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = base.clone();
        f(&mut b);
        vectors.push((name, code, b));
    };

    add("too-short", codes::CONTAINER_TOO_SHORT, &|b| b.truncate(3));
    add("count-negative", codes::CONTAINER_COUNT_NEGATIVE, &|b| {
        put_i32(b, 0, -1)
    });
    add("table-truncated", codes::CONTAINER_TABLE_TRUNCATED, &|b| {
        b.truncate(at::offset(1) + 2)
    });
    add("id-len-negative", codes::CONTAINER_ID_LEN_NEGATIVE, &|b| {
        put_i32(b, at::id_len(0), -1)
    });
    add("id-not-utf8", codes::CONTAINER_ID_NOT_UTF8, &|b| {
        b[at::id(1)] = 0xff
    });
    add(
        "codec-len-negative",
        codes::CONTAINER_CODEC_LEN_NEGATIVE,
        &|b| put_i32(b, at::codec_len(0), -1),
    );
    add(
        "codec-unsupported",
        codes::CONTAINER_CODEC_UNSUPPORTED,
        &|b| {
            let c = at::codec_len(0);
            put_i32(b, c, 4);
            b.splice(c + 4..c + 4, *b"zstd");
        },
    );
    add("offset-negative", codes::CONTAINER_OFFSET_NEGATIVE, &|b| {
        put_i32(b, at::offset(1), -1)
    });
    add("length-negative", codes::CONTAINER_LENGTH_NEGATIVE, &|b| {
        put_i32(b, at::len(1), -1)
    });
    add("entry-overlap", codes::CONTAINER_ENTRY_OVERLAP, &|b| {
        put_i32(b, at::offset(1), first_len - 1)
    });
    add("entry-gap", codes::CONTAINER_ENTRY_GAP, &|b| {
        put_i32(b, at::offset(1), first_len + 1)
    });
    add("entry-past-end", codes::CONTAINER_ENTRY_PAST_END, &|b| {
        b.pop();
    });
    add("trailing-bytes", codes::CONTAINER_TRAILING_BYTES, &|b| {
        b.push(0)
    });
    // The table is sound; the first section's magic is not.
    let table_end = at::id_len(LAYER_IDS.len());
    add("section-bad-magic", codes::BAD_MAGIC, &|b| b[table_end] = 0);

    let key = matter_sections()[0].1.key();
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for (name, code, bytes) in vectors {
        let got = decode_chunk(&key, &bytes).map(|_| ()).map_err(|e| e.code);
        assert_eq!(got, Err(code), "container vector {name}");
        let file = format!("{code}-{name}.bin");
        assert!(
            index.insert(file.clone(), code).is_none(),
            "duplicate vector {file}"
        );
        out.push((format!("container/invalid/{file}"), bytes));
    }
    let index = json!({
        "key": key.to_string(),
        "vectors": index,
    });
    out.push(("container/invalid/index.json".into(), pretty(&index)));
    out
}

/// Every container vector file.
pub fn files() -> Vec<File> {
    let mut out = valid();
    out.extend(invalid());
    out
}
