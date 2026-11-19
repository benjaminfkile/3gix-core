//! Runs every module conformance vector in a directory through the native
//! host and compares each output with its expected SHA-256.
//!
//! Usage: `gx-module-run MODULE.wasm VECTORS_DIR`. For every
//! `NAME.request.bin` in `VECTORS_DIR` (in name order) it reads
//! `NAME.tiles.bin` and `NAME.sha256`, loads the module under the hash of
//! its own bytes, runs `gx_sources` and checks the ids equal the tile ids,
//! runs `gx_compile`, validates the section under the request's chunk key,
//! and compares the SHA-256 of the section with the expected hash. Exits 1
//! if any vector fails. This is the native half of
//! `scripts/module-determinism.sh` (`client-compute.md` section 4,
//! `space-model.md` section 4 item 6).
//!
//! For a version 2 module (`client-compute.md` section 4.1) every vector
//! also has `NAME.frames.bin`, `NAME.input-list.bin`, and `NAME.inputs.bin`:
//! it runs `gx_inputs` with the frame bytes and checks the list equals
//! `NAME.input-list.bin`, runs `gx_sources` with the frame bytes, and runs
//! `gx_compile` with the tile and input bytes.

use std::path::Path;
use std::process::ExitCode;

use gx_core::module_io;
use gx_core::sha256::{sha256, to_hex};
use gx_module_host::ModuleHost;

fn run_one(host: &gx_module_host::LoadedModule, dir: &Path, name: &str) -> Result<String, String> {
    let read = |suffix: &str| {
        let p = dir.join(format!("{name}.{suffix}"));
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let request = read("request.bin")?;
    let tiles = read("tiles.bin")?;
    let expected = String::from_utf8(read("sha256")?)
        .map_err(|_| "expected hash is not UTF-8".to_string())?
        .trim()
        .to_string();
    let req = module_io::decode_request(&request).map_err(|e| format!("request: {e}"))?;
    let want: Vec<u64> = module_io::decode_tiles(&tiles)
        .map_err(|e| format!("tiles: {e}"))?
        .iter()
        .map(|t| t.id)
        .collect();
    let v2 = host.abi_version() == module_io::ABI_VERSION_2;
    let (frames, inputs) = if v2 {
        (read("frames.bin")?, read("inputs.bin")?)
    } else {
        (Vec::new(), Vec::new())
    };
    if v2 {
        let list = host
            .inputs_raw(&request, &frames)
            .map_err(|e| e.to_string())?;
        if list != read("input-list.bin")? {
            return Err("input list differs from input-list.bin".into());
        }
    }
    let ids = if v2 {
        host.sources_v2_raw(&request, &frames)
    } else {
        host.sources_raw(&request)
    }
    .map_err(|e| e.to_string())?;
    let ids = module_io::decode_source_ids(&ids).map_err(|e| format!("sources: {e}"))?;
    if ids != want {
        return Err(format!("sources {ids:?}, tiles carry {want:?}"));
    }
    let section = if v2 {
        host.compile_v2_raw(&request, &tiles, &inputs)
    } else {
        host.compile_raw(&request, &tiles)
    }
    .map_err(|e| e.to_string())?;
    gx_core::validate(&req.key.to_string(), &section).map_err(|e| format!("section: {e}"))?;
    let got = to_hex(&sha256(&section));
    if got != expected {
        return Err(format!("output {got}, expected {expected}"));
    }
    Ok(got)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: gx-module-run MODULE.wasm VECTORS_DIR");
        return ExitCode::from(2);
    }
    let bytes = match std::fs::read(&args[1]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", args[1]);
            return ExitCode::from(2);
        }
    };
    let host = match ModuleHost::new().and_then(|h| h.load(&bytes, &sha256(&bytes))) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("load: {e}");
            return ExitCode::FAILURE;
        }
    };
    let dir = Path::new(&args[2]);
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".request.bin"))
                    .map(str::to_string)
            })
            .collect(),
        Err(e) => {
            eprintln!("{}: {e}", dir.display());
            return ExitCode::from(2);
        }
    };
    names.sort();
    if names.is_empty() {
        eprintln!("{}: no vectors", dir.display());
        return ExitCode::FAILURE;
    }
    let mut failed = 0;
    for name in &names {
        match run_one(&host, dir, name) {
            Ok(h) => println!("native ok   {name} {h}"),
            Err(e) => {
                println!("native FAIL {name}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        eprintln!("native: {failed} of {} vectors failed", names.len());
        return ExitCode::FAILURE;
    }
    println!("native: {} vectors ok", names.len());
    ExitCode::SUCCESS
}
