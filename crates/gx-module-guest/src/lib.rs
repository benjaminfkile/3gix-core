//! The guest side of the module ABI: what a compiler author links to become
//! a module.
//!
//! Implements `client-compute.md` section 4 (module ABI version 1) inside the
//! module. The crate provides the memory and error exports every module
//! needs (`gx_alloc`, `gx_free`, `gx_abi_version`, `gx_last_error`), and the
//! [`gx_module!`] macro turns a type implementing [`Module`] into the two
//! remaining exports, `gx_sources` and `gx_compile`, with the exact ABI:
//!
//! ```
//! use gx_module_guest::{gx_module, Module, Request, Tiles};
//!
//! struct MyModule;
//!
//! impl Module for MyModule {
//!     fn sources(req: &Request) -> Vec<u64> {
//!         Vec::new()
//!     }
//!     fn compile(req: &Request, tiles: &Tiles) -> Result<Vec<u8>, String> {
//!         Err("nothing to compile".into())
//!     }
//! }
//!
//! gx_module!(MyModule);
//! ```
//!
//! The macro's exports decode the request and tile bytes with
//! [`gx_core::module_io`], check that the tiles are exactly the ones
//! [`Module::sources`] asked for, run the author's code, validate the
//! returned section with the library validator ([`gx_core::validate()`])
//! under the request's chunk key, and record any failure for
//! `gx_last_error`.
//!
//! The exports exist only on `wasm32`, where pointers are 32 bits. On other
//! targets the same logic is reachable through [`run_sources`] and
//! [`run_compile`], which is how tests and conformance generators call a
//! module natively.
//!
//! Build the module crate as a `cdylib` for `wasm32-unknown-unknown`. It
//! must not use threads or relaxed SIMD, and it has no imports: the
//! library is linked with `default-features = false` so none of its own
//! exports come along.

use gx_core::module_io::{self, ModuleRequest, Tile};

pub use gx_core;

/// The request a module receives: the chunk key, the origin, and the span
/// (`client-compute.md` section 4, request bytes).
pub type Request = ModuleRequest;

/// The source tiles passed to [`Module::compile`], in the order
/// [`Module::sources`] returned their ids (ascending, unique).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiles<'a> {
    tiles: Vec<Tile<'a>>,
}

impl<'a> Tiles<'a> {
    /// Wraps decoded tiles. The ids must ascend; [`module_io::decode_tiles`]
    /// guarantees that.
    pub fn new(tiles: Vec<Tile<'a>>) -> Tiles<'a> {
        Tiles { tiles }
    }

    /// The bytes of the tile with id `id`, or `None` if it was not passed.
    pub fn get(&self, id: u64) -> Option<&'a [u8]> {
        self.tiles
            .binary_search_by_key(&id, |t| t.id)
            .ok()
            .map(|i| self.tiles[i].bytes)
    }

    /// The tiles in ascending id order.
    pub fn as_slice(&self) -> &[Tile<'a>] {
        &self.tiles
    }

    /// The tile ids in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.tiles.iter().map(|t| t.id)
    }

    /// The number of tiles.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// True when no tiles were passed.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

/// A compiler module: a pure function from a request and source tiles to
/// one section.
///
/// Both functions must be deterministic: no clock, no randomness, no
/// iteration over a hash map into the output, and no platform-dependent
/// float paths (use [`gx_core::detmath`] for elementary functions).
pub trait Module {
    /// The source tile ids this request needs, ascending and unique.
    /// Empty when the module reads no sources.
    fn sources(req: &Request) -> Vec<u64>;

    /// The section for the request's chunk key: matter section bytes for a
    /// cell, frame registry bytes for `registry` (an empty registry when
    /// the module declares no frames). `Err` carries the failure reason.
    fn compile(req: &Request, tiles: &Tiles) -> Result<Vec<u8>, String>;
}

/// The `gx_sources` logic on safe inputs: decodes the request, asks the
/// module, and encodes the id list (`client-compute.md` section 4).
///
/// Fails if the request does not decode or the ids are not ascending and
/// unique.
pub fn run_sources<M: Module>(request: &[u8]) -> Result<Vec<u8>, String> {
    let req = module_io::decode_request(request).map_err(|e| format!("request: {e}"))?;
    let ids = M::sources(&req);
    module_io::encode_source_ids(&ids).map_err(|e| format!("sources: {e}"))
}

/// The `gx_compile` logic on safe inputs: decodes the request and the
/// tiles, checks the tile ids are the ones [`Module::sources`] returns,
/// compiles, and validates the section under the request's chunk key with
/// the library validator before returning it (`client-compute.md`
/// section 4, rules).
pub fn run_compile<M: Module>(request: &[u8], tiles: &[u8]) -> Result<Vec<u8>, String> {
    let req = module_io::decode_request(request).map_err(|e| format!("request: {e}"))?;
    let tiles = module_io::decode_tiles(tiles).map_err(|e| format!("tiles: {e}"))?;
    let tiles = Tiles::new(tiles);
    let want = M::sources(&req);
    if !tiles.ids().eq(want.iter().copied()) {
        let got: Vec<u64> = tiles.ids().collect();
        return Err(format!(
            "tiles: ids {got:?} passed, the module asked for {want:?}"
        ));
    }
    let section = M::compile(&req, &tiles)?;
    let key = req.key.to_string();
    gx_core::validate(&key, &section)
        .map_err(|e| format!("section for {key} failed validation: {e}"))?;
    Ok(section)
}

/// Packs a pointer and a length as `(ptr << 32) | len`, the ABI's return
/// form.
pub fn pack(ptr: u32, len: u32) -> i64 {
    ((u64::from(ptr) << 32) | u64::from(len)) as i64
}

/// The guest exports and their state. Public only so [`gx_module!`] can
/// reach it; not part of the author's API.
#[doc(hidden)]
pub mod abi {
    use std::alloc::{alloc, dealloc, Layout};
    use std::sync::Mutex;

    /// The last failure reason, owned by the module until the next failure.
    pub(crate) static LAST_ERROR: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    /// Records `reason` for `gx_last_error`.
    pub fn set_last_error(reason: &str) {
        let mut e = LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner());
        e.clear();
        e.extend_from_slice(reason.as_bytes());
    }

    /// Allocates `len` bytes with alignment 1. Null for a negative length or
    /// when the allocator fails; a dangling non-null pointer for 0.
    pub fn alloc_bytes(len: i32) -> *mut u8 {
        let Ok(len) = usize::try_from(len) else {
            return core::ptr::null_mut();
        };
        if len == 0 {
            return core::ptr::NonNull::<u8>::dangling().as_ptr();
        }
        let layout = Layout::from_size_align(len, 1).expect("alignment 1");
        // SAFETY: `layout` has a non-zero size.
        unsafe { alloc(layout) }
    }

    /// Frees a block from [`alloc_bytes`] or from [`leak`].
    ///
    /// # Safety
    ///
    /// `ptr` and `len` must describe a live block from one of those two
    /// functions, freed at most once.
    pub unsafe fn free_bytes(ptr: *mut u8, len: i32) {
        let Ok(len) = usize::try_from(len) else {
            return;
        };
        if len == 0 || ptr.is_null() {
            return;
        }
        let layout = Layout::from_size_align(len, 1).expect("alignment 1");
        // SAFETY: the caller guarantees the block came from `alloc` with
        // this layout.
        unsafe { dealloc(ptr, layout) }
    }

    /// Hands `bytes` to the host as a block it frees with `gx_free`, and
    /// returns `(ptr, len)`.
    pub fn leak(bytes: Vec<u8>) -> (*mut u8, usize) {
        let b = bytes.into_boxed_slice();
        let len = b.len();
        let ptr = Box::into_raw(b).cast::<u8>();
        (ptr, len)
    }

    /// Reads `len` bytes the host wrote at `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for `len` readable bytes, or `len` must be 0.
    pub unsafe fn input<'a>(ptr: *const u8, len: i32) -> Result<&'a [u8], String> {
        let len = usize::try_from(len).map_err(|_| format!("negative input length {len}"))?;
        if len == 0 {
            return Ok(&[]);
        }
        if ptr.is_null() {
            return Err(format!("null input pointer with length {len}"));
        }
        // SAFETY: the caller guarantees validity.
        Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
    }

    /// Turns a result into the ABI return value: `(ptr << 32) | len` of the
    /// leaked bytes, or -1 with the reason recorded.
    pub fn finish(r: Result<Vec<u8>, String>) -> i64 {
        match r {
            Ok(bytes) => {
                let (ptr, len) = leak(bytes);
                match (u32::try_from(ptr as usize), u32::try_from(len)) {
                    (Ok(p), Ok(l)) => super::pack(p, l),
                    _ => {
                        // SAFETY: just leaked with this length; on wasm32
                        // the conversions never fail.
                        unsafe { free_bytes(ptr, len as i32) };
                        set_last_error("output does not fit 32-bit pointer and length");
                        -1
                    }
                }
            }
            Err(reason) => {
                set_last_error(&reason);
                -1
            }
        }
    }

    /// `gx_abi_version`: returns 1.
    #[cfg(target_arch = "wasm32")]
    #[no_mangle]
    pub extern "C" fn gx_abi_version() -> i32 {
        gx_core::module_io::ABI_VERSION
    }

    /// `gx_alloc`: allocates `len` bytes for the host to write into.
    #[cfg(target_arch = "wasm32")]
    #[no_mangle]
    pub extern "C" fn gx_alloc(len: i32) -> i32 {
        alloc_bytes(len) as usize as i32
    }

    /// `gx_free`: frees a block returned by the module or by `gx_alloc`.
    ///
    /// # Safety
    ///
    /// The block must be live and freed once, as the ABI requires.
    #[cfg(target_arch = "wasm32")]
    #[no_mangle]
    pub unsafe extern "C" fn gx_free(ptr: i32, len: i32) {
        // SAFETY: forwarded from the host under the ABI contract.
        unsafe { free_bytes(ptr as usize as *mut u8, len) }
    }

    /// `gx_last_error`: `(ptr << 32) | len` of the UTF-8 reason of the last
    /// failure. The bytes stay owned by the module; the host copies them and
    /// does not free them.
    #[cfg(target_arch = "wasm32")]
    #[no_mangle]
    pub extern "C" fn gx_last_error() -> i64 {
        let e = LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner());
        super::pack(e.as_ptr() as usize as u32, e.len() as u32)
    }
}

/// Exports `gx_sources` and `gx_compile` for a type implementing
/// [`Module`], with the exact ABI of `client-compute.md` section 4.
///
/// `gx_sources(req_ptr, req_len) -> i64` returns `(ptr << 32) | len` of the
/// `u64` LE id list, or -1 with a reason for `gx_last_error` if the request
/// does not decode or the ids are not ascending and unique.
/// `gx_compile(req_ptr, req_len, tiles_ptr, tiles_len) -> i64` returns
/// `(ptr << 32) | len` of the validated section, or -1 with a reason. The
/// host frees returned blocks with `gx_free`. Use it once per module crate,
/// at the crate root.
#[macro_export]
macro_rules! gx_module {
    ($module:ty) => {
        /// `gx_sources`: the source tile ids the request needs.
        ///
        /// # Safety
        ///
        /// The host passes a block it wrote with `gx_alloc`.
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn gx_sources(req_ptr: i32, req_len: i32) -> i64 {
            // SAFETY: the host wrote `req_len` bytes at `req_ptr`.
            let r = unsafe { $crate::abi::input(req_ptr as usize as *const u8, req_len) }
                .and_then($crate::run_sources::<$module>);
            $crate::abi::finish(r)
        }

        /// `gx_compile`: the validated section for the request.
        ///
        /// # Safety
        ///
        /// The host passes blocks it wrote with `gx_alloc`.
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn gx_compile(
            req_ptr: i32,
            req_len: i32,
            tiles_ptr: i32,
            tiles_len: i32,
        ) -> i64 {
            // SAFETY: the host wrote the two blocks.
            let r = unsafe {
                $crate::abi::input(req_ptr as usize as *const u8, req_len).and_then(|req| {
                    $crate::abi::input(tiles_ptr as usize as *const u8, tiles_len)
                        .and_then(|tiles| $crate::run_compile::<$module>(req, tiles))
                })
            };
            $crate::abi::finish(r)
        }

        /// Keeps the library's exports linked into the module.
        #[cfg(target_arch = "wasm32")]
        #[used]
        static GX_MODULE_GUEST_EXPORTS: [unsafe extern "C" fn(i32, i32); 1] =
            [$crate::abi::gx_free];

        /// Checks at compile time that the type implements the trait.
        const _: fn() = || {
            fn is_module<M: $crate::Module>() {}
            is_module::<$module>();
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use gx_core::key::{CellKey, ChunkKey};
    use gx_core::matter::{self, Compression, Section};
    use gx_core::module_io::{encode_request, encode_tiles};
    use gx_core::registry::{self, Registry};
    use gx_core::units::Meters;

    struct Echo;

    impl Module for Echo {
        fn sources(req: &Request) -> Vec<u64> {
            match req.key {
                ChunkKey::Registry => Vec::new(),
                ChunkKey::Cell(c) => vec![c.frame_id, c.frame_id + 1],
                ChunkKey::Surface(_) => Vec::new(),
                ChunkKey::Frame(_) => Vec::new(),
            }
        }
        fn compile(req: &Request, tiles: &Tiles) -> Result<Vec<u8>, String> {
            match req.key {
                ChunkKey::Registry => Ok(registry::encode(&Registry::empty())),
                ChunkKey::Cell(c) => {
                    if tiles.get(c.frame_id) == Some(b"bad") {
                        return Ok(b"3GMS".to_vec());
                    }
                    let g = c.geometry(Meters::new(64.0));
                    let s = Section::empty(c, g.origin, g.edge).map_err(|e| e.to_string())?;
                    Ok(matter::encode(&s, Compression::None))
                }
                ChunkKey::Surface(t) => Ok(gx_core::surface::encode_empty(&t)),
                ChunkKey::Frame(id) => Ok(gx_core::frame_record::encode(
                    &gx_core::frame_record::FrameRecord::empty(id),
                )),
            }
        }
    }

    gx_module!(Echo);

    struct Unsorted;

    impl Module for Unsorted {
        fn sources(_: &Request) -> Vec<u64> {
            vec![3, 3]
        }
        fn compile(_: &Request, _: &Tiles) -> Result<Vec<u8>, String> {
            Err("never".into())
        }
    }

    fn req(key: &str) -> Vec<u8> {
        encode_request(&Request {
            origin_utc_ms: 0,
            span_seconds: 0,
            span_index: 0,
            key: key.parse().unwrap(),
        })
    }

    fn tiles(list: &[(u64, &[u8])]) -> Vec<u8> {
        let t: Vec<Tile<'_>> = list.iter().map(|&(id, bytes)| Tile { id, bytes }).collect();
        encode_tiles(&t).unwrap()
    }

    #[test]
    fn sources_encode_ids() {
        let ids = run_sources::<Echo>(&req("7-1-0-1-0")).unwrap();
        assert_eq!(module_io::decode_source_ids(&ids).unwrap(), vec![7, 8]);
        assert!(run_sources::<Echo>(&req("registry")).unwrap().is_empty());
        assert!(run_sources::<Echo>(b"3GXQ")
            .unwrap_err()
            .starts_with("request: code 931"));
        assert!(run_sources::<Unsorted>(&req("registry"))
            .unwrap_err()
            .starts_with("sources: code 952"));
    }

    #[test]
    fn compile_validates_and_checks_tiles() {
        let key = CellKey::new(7, 1, 0, 1, 0).unwrap();
        let out = run_compile::<Echo>(&req("7-1-0-1-0"), &tiles(&[(7, b"a"), (8, b"")])).unwrap();
        assert!(matter::validate(&key, &out).is_ok());
        let out = run_compile::<Echo>(&req("registry"), &tiles(&[])).unwrap();
        assert!(registry::validate(&out).is_ok());

        let e = run_compile::<Echo>(&req("7-1-0-1-0"), &tiles(&[(7, b"a")])).unwrap_err();
        assert!(e.contains("asked for [7, 8]"), "{e}");
        let e =
            run_compile::<Echo>(&req("7-1-0-1-0"), &tiles(&[(7, b"bad"), (8, b"")])).unwrap_err();
        assert!(e.contains("failed validation"), "{e}");
        let e = run_compile::<Echo>(&req("7-1-0-1-0"), &[1, 0]).unwrap_err();
        assert!(e.starts_with("tiles: code 941"), "{e}");
    }

    #[test]
    fn finish_records_errors_and_packs() {
        assert_eq!(abi::finish(Err("broken".into())), -1);
        assert_eq!(*abi::LAST_ERROR.lock().unwrap(), b"broken");
        assert_eq!(pack(2, 3), (2 << 32) | 3);
        assert_eq!(pack(u32::MAX, u32::MAX), -1);
    }

    #[test]
    fn alloc_and_free() {
        assert!(abi::alloc_bytes(-1).is_null());
        let p = abi::alloc_bytes(0);
        assert!(!p.is_null());
        unsafe { abi::free_bytes(p, 0) };
        let p = abi::alloc_bytes(16);
        assert!(!p.is_null());
        unsafe { abi::free_bytes(p, 16) };
    }
}
