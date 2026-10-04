//! Determinism proof: running the generator into a scratch directory
//! produces exactly the files checked in under `conformance/matter/`,
//! `conformance/registry/`, and `conformance/container/`, byte for byte, with
//! no file missing or extra.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Lists every file under `dir`, as paths relative to `dir`, sorted.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display())) {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).expect("under dir").to_path_buf());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn generator_reproduces_checked_in_vectors() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("conformance-determinism");
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch).expect("clear scratch directory");
    }
    let status = Command::new(env!("CARGO_BIN_EXE_conformance-gen"))
        .arg(&scratch)
        .status()
        .expect("run conformance-gen");
    assert!(status.success(), "conformance-gen failed: {status}");

    for dir in ["matter", "registry", "container"] {
        let fresh = scratch.join(dir);
        let checked_in = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance")
            .join(dir);
        let a = files(&fresh);
        let b = files(&checked_in);
        assert_eq!(a, b, "{dir}: generated and checked-in file lists differ");
        assert!(!a.is_empty(), "{dir}: no files");
        for rel in &a {
            let x = std::fs::read(fresh.join(rel)).unwrap();
            let y = std::fs::read(checked_in.join(rel)).unwrap();
            assert!(
                x == y,
                "{dir}/{} differs from the checked-in file",
                rel.display()
            );
        }
    }
}
