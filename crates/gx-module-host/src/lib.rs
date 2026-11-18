//! The native module host: what the client's worker uses to run compiler
//! modules.
//!
//! Implements the host side of `client-compute.md` section 4 (module ABI
//! version 1). A module is admitted by [`ModuleHost::load`], which checks
//! the SHA-256 the module is addressed by, runs the static checks in
//! [`check`] (no imports, no relaxed SIMD or threads instruction, only the
//! allowed features), compiles it with Cranelift, instantiates it with an
//! empty linker, and checks every required export and
//! `gx_abi_version() == 1`. [`LoadedModule::sources`] and
//! [`LoadedModule::compile`] then run the module, each call in a fresh
//! instance, under a memory limit (1 GiB by default) and an epoch
//! interruption time limit (60 s by default). A compiled section is checked
//! with the library validator before it is returned, as the worker must
//! before upload (`client-compute.md` section 1 step 4).
//!
//! The engine is configured for determinism (`space-model.md` section 1
//! rule 7, section 4 item 6): every WebAssembly proposal outside the allowed
//! set is disabled, including relaxed SIMD and threads, and Cranelift
//! canonicalizes NaNs so no NaN bit pattern depends on the machine.
//!
//! The same host is exported through a C ABI in [`ffi`] (`gx_module_compile`,
//! `gx_module_sources`, `gx_module_free`) so the hub can settle a dispute by
//! running the module with the host clients use (`client-compute.md`
//! section 2.7).
//!
//! Native only. The browser build uses the browser's own WebAssembly.

pub mod check;
pub mod ffi;

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use gx_core::error::ValidationError;
use gx_core::module_io::{self, ModuleRequest, Tile, ABI_VERSION};
use gx_core::sha256::{sha256, to_hex};
use wasmtime::{
    Config, Engine, ExternType, Instance, InstancePre, Linker, Memory, Module, ResourceLimiter,
    Store, Strategy, Trap, ValType, WasmFeatures,
};

pub use check::{check_module, forbidden_opcodes, ForbiddenOpcode, ALLOWED_FEATURES};

/// The default memory limit of one module call: 1 GiB
/// (`client-compute.md` section 4, rules).
pub const DEFAULT_MEMORY_LIMIT: usize = 1 << 30;

/// The default time limit of one module call: 60 seconds.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(60);

/// How often the engine's epoch advances. The time limit is enforced to
/// within one tick.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// The exports every module must have, with their signatures written as
/// parameter and result types (`i` for `i32`, `I` for `i64`).
pub const REQUIRED_FUNCTIONS: [(&str, &str, &str); 6] = [
    ("gx_abi_version", "", "i"),
    ("gx_alloc", "i", "i"),
    ("gx_free", "ii", ""),
    ("gx_sources", "ii", "I"),
    ("gx_compile", "iiii", "I"),
    ("gx_last_error", "", "I"),
];

/// Why a module could not be loaded or a call failed.
#[derive(Clone, Debug, PartialEq)]
pub enum HostError {
    /// The SHA-256 of the bytes is not the hash the module was requested by.
    HashMismatch {
        /// The hash the caller expected, lowercase hex.
        expected: String,
        /// The hash of the bytes, lowercase hex.
        actual: String,
    },
    /// The bytes are not an admissible WebAssembly module: they do not
    /// parse, or they use a feature outside the allowed set.
    Invalid(String),
    /// The module imports something. Modules are instantiated with an empty
    /// import object, so any import fails.
    Import {
        /// The import's module name.
        module: String,
        /// The import's field name.
        name: String,
    },
    /// The module contains an instruction from a forbidden proposal, such as
    /// relaxed SIMD or threads.
    ForbiddenOpcode(ForbiddenOpcode),
    /// A required export is missing or has the wrong type.
    Export(String),
    /// `gx_abi_version` returned something other than 1.
    AbiVersion(i32),
    /// The engine could not be built or the module could not be compiled or
    /// instantiated.
    Engine(String),
    /// The call ran past the time limit.
    TimeLimit(Duration),
    /// The module asked for more memory than the limit.
    MemoryLimit(usize),
    /// The module trapped.
    Trap(String),
    /// The module returned -1; the reason is what `gx_last_error` reported.
    ModuleFailed(String),
    /// The module broke the ABI: an out-of-bounds pointer, a failed
    /// allocation, or a malformed source id list.
    Abi(String),
    /// The caller's input could not be encoded, for example tile ids that do
    /// not ascend.
    Input(ValidationError),
    /// The section the module returned fails the library validator.
    Section(ValidationError),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::HashMismatch { expected, actual } => write!(
                f,
                "module hash mismatch: expected {expected}, bytes hash to {actual}"
            ),
            HostError::Invalid(r) => write!(f, "invalid module: {r}"),
            HostError::Import { module, name } => write!(
                f,
                "module imports {module}.{name}; modules are instantiated with no imports"
            ),
            HostError::ForbiddenOpcode(o) => write!(
                f,
                "module uses a forbidden {} instruction {} at offset {}",
                o.proposal, o.instruction, o.offset
            ),
            HostError::Export(r) => write!(f, "module export: {r}"),
            HostError::AbiVersion(v) => {
                write!(f, "gx_abi_version returned {v}, expected {ABI_VERSION}")
            }
            HostError::Engine(r) => write!(f, "engine: {r}"),
            HostError::TimeLimit(d) => {
                write!(f, "module exceeded the time limit of {} ms", d.as_millis())
            }
            HostError::MemoryLimit(n) => {
                write!(f, "module exceeded the memory limit of {n} bytes")
            }
            HostError::Trap(r) => write!(f, "module trapped: {r}"),
            HostError::ModuleFailed(r) => write!(f, "module failed: {r}"),
            HostError::Abi(r) => write!(f, "module broke the ABI: {r}"),
            HostError::Input(e) => write!(f, "input: {e}"),
            HostError::Section(e) => write!(f, "section fails validation: {e}"),
        }
    }
}

impl std::error::Error for HostError {}

/// Result of a host operation.
pub type Result<T> = std::result::Result<T, HostError>;

/// Limits applied to every module call.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HostConfig {
    /// The largest linear memory a module may grow to, in bytes.
    pub memory_limit: usize,
    /// The longest one call may run.
    pub time_limit: Duration,
}

impl Default for HostConfig {
    fn default() -> HostConfig {
        HostConfig {
            memory_limit: DEFAULT_MEMORY_LIMIT,
            time_limit: DEFAULT_TIME_LIMIT,
        }
    }
}

/// Advances the engine's epoch every [`EPOCH_TICK`] on a background thread
/// until dropped.
struct Ticker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(engine: Engine) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        });
        Ticker {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Builds the engine configuration: Cranelift, only the allowed features,
/// relaxed SIMD and threads off, NaN canonicalization on, epoch
/// interruption on.
pub fn engine_config() -> Config {
    let mut c = Config::new();
    c.strategy(Strategy::Cranelift);
    c.wasm_features(WasmFeatures::all(), false);
    c.wasm_features(ALLOWED_FEATURES, true);
    c.wasm_relaxed_simd(false);
    c.wasm_features(
        WasmFeatures::THREADS | WasmFeatures::SHARED_EVERYTHING_THREADS,
        false,
    );
    c.cranelift_nan_canonicalization(true);
    c.epoch_interruption(true);
    c
}

/// The engine shared by every module a worker runs.
pub struct ModuleHost {
    engine: Engine,
    config: HostConfig,
    ticker: Arc<Ticker>,
}

impl ModuleHost {
    /// A host with the default limits: 1 GiB of memory and 60 s per call.
    pub fn new() -> Result<ModuleHost> {
        ModuleHost::with_config(HostConfig::default())
    }

    /// A host with the given limits.
    pub fn with_config(config: HostConfig) -> Result<ModuleHost> {
        let engine = Engine::new(&engine_config()).map_err(|e| HostError::Engine(e.to_string()))?;
        let ticker = Arc::new(Ticker::start(engine.clone()));
        Ok(ModuleHost {
            engine,
            config,
            ticker,
        })
    }

    /// The limits applied to every call.
    pub fn config(&self) -> HostConfig {
        self.config
    }

    /// Admits a module: checks its SHA-256 equals `expected_hash`, runs
    /// [`check_module`], compiles it, instantiates it with an empty linker,
    /// and checks the required exports and `gx_abi_version() == 1`.
    pub fn load(&self, bytes: &[u8], expected_hash: &[u8; 32]) -> Result<LoadedModule> {
        let actual = sha256(bytes);
        if actual != *expected_hash {
            return Err(HostError::HashMismatch {
                expected: to_hex(expected_hash),
                actual: to_hex(&actual),
            });
        }
        check_module(bytes)?;
        let module =
            Module::new(&self.engine, bytes).map_err(|e| HostError::Engine(format!("{e:#}")))?;
        check_exports(&module)?;
        let linker: Linker<Limits> = Linker::new(&self.engine);
        let pre = linker
            .instantiate_pre(&module)
            .map_err(|e| HostError::Engine(format!("{e:#}")))?;
        let loaded = LoadedModule {
            hash: actual,
            pre,
            config: self.config,
            _ticker: Arc::clone(&self.ticker),
        };
        let mut call = loaded.instantiate()?;
        let version = call.abi_version()?;
        if version != ABI_VERSION {
            return Err(HostError::AbiVersion(version));
        }
        Ok(loaded)
    }
}

fn type_code(v: &ValType) -> char {
    match v {
        ValType::I32 => 'i',
        ValType::I64 => 'I',
        ValType::F32 => 'f',
        ValType::F64 => 'F',
        ValType::V128 => 'v',
        ValType::Ref(_) => 'r',
    }
}

fn check_exports(module: &Module) -> Result<()> {
    match module.get_export("memory") {
        Some(ExternType::Memory(m)) if !m.is_64() && !m.is_shared() => {}
        Some(_) => {
            return Err(HostError::Export(
                "memory is not a 32-bit unshared memory".into(),
            ))
        }
        None => return Err(HostError::Export("memory is missing".into())),
    }
    for (name, params, results) in REQUIRED_FUNCTIONS {
        let Some(ext) = module.get_export(name) else {
            return Err(HostError::Export(format!("{name} is missing")));
        };
        let ExternType::Func(f) = ext else {
            return Err(HostError::Export(format!("{name} is not a function")));
        };
        let p: String = f.params().map(|t| type_code(&t)).collect();
        let r: String = f.results().map(|t| type_code(&t)).collect();
        if p != params || r != results {
            return Err(HostError::Export(format!(
                "{name} has signature ({p}) -> ({r}), expected ({params}) -> ({results})"
            )));
        }
    }
    Ok(())
}

/// The store's data: the memory limit and whether the module hit it.
struct Limits {
    memory_limit: usize,
    memory_exceeded: bool,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_limit {
            self.memory_exceeded = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 1 << 20)
    }

    fn instances(&self) -> usize {
        1
    }

    fn memories(&self) -> usize {
        1
    }
}

/// A module admitted by [`ModuleHost::load`], ready to run.
pub struct LoadedModule {
    hash: [u8; 32],
    pre: InstancePre<Limits>,
    config: HostConfig,
    _ticker: Arc<Ticker>,
}

/// One fresh instance of a module, for one call.
struct Call {
    store: Store<Limits>,
    instance: Instance,
    memory: Memory,
    config: HostConfig,
}

impl LoadedModule {
    /// The SHA-256 of the module's bytes.
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }

    fn instantiate(&self) -> Result<Call> {
        let engine = self.pre.module().engine();
        let mut store = Store::new(
            engine,
            Limits {
                memory_limit: self.config.memory_limit,
                memory_exceeded: false,
            },
        );
        store.limiter(|l| l as &mut dyn ResourceLimiter);
        let ticks = self
            .config
            .time_limit
            .as_nanos()
            .div_ceil(EPOCH_TICK.as_nanos())
            + 1;
        store.set_epoch_deadline(u64::try_from(ticks).unwrap_or(u64::MAX));
        store.epoch_deadline_trap();
        let config = self.config;
        let instance = match self.pre.instantiate(&mut store) {
            Ok(i) => i,
            Err(e) => {
                return Err(classify(&store, config, e));
            }
        };
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| HostError::Export("memory is missing".into()))?;
        Ok(Call {
            store,
            instance,
            memory,
            config,
        })
    }

    /// Runs `gx_sources`: the source tile ids the request needs, ascending
    /// and unique.
    pub fn sources(&self, request: &ModuleRequest) -> Result<Vec<u64>> {
        let mut call = self.instantiate()?;
        let req = module_io::encode_request(request);
        let (rp, rl) = call.write(&req)?;
        let packed = call.call_i64("gx_sources", &[rp, rl])?;
        if packed == -1 {
            return Err(HostError::ModuleFailed(call.last_error()?));
        }
        let bytes = call.take(packed)?;
        module_io::decode_source_ids(&bytes).map_err(|e| HostError::Abi(e.to_string()))
    }

    /// Runs `gx_compile` with the given tiles (ids ascending, in the order
    /// [`LoadedModule::sources`] returned them) and checks the section with
    /// the library validator under the request's chunk key.
    pub fn compile(&self, request: &ModuleRequest, tiles: &[(u64, Vec<u8>)]) -> Result<Vec<u8>> {
        let list: Vec<Tile<'_>> = tiles
            .iter()
            .map(|(id, bytes)| Tile { id: *id, bytes })
            .collect();
        let tile_bytes = module_io::encode_tiles(&list).map_err(HostError::Input)?;
        self.compile_raw(&module_io::encode_request(request), &tile_bytes)
            .and_then(|section| {
                gx_core::validate(&request.key.to_string(), &section)
                    .map_err(HostError::Section)?;
                Ok(section)
            })
    }

    /// Runs `gx_compile` on request and tile bytes exactly as given, without
    /// validating the result. For conformance harnesses that replay stored
    /// vectors; the worker uses [`LoadedModule::compile`].
    pub fn compile_raw(&self, request: &[u8], tiles: &[u8]) -> Result<Vec<u8>> {
        let mut call = self.instantiate()?;
        let (rp, rl) = call.write(request)?;
        let (tp, tl) = call.write(tiles)?;
        let packed = call.call_i64("gx_compile", &[rp, rl, tp, tl])?;
        if packed == -1 {
            return Err(HostError::ModuleFailed(call.last_error()?));
        }
        call.take(packed)
    }

    /// Runs `gx_sources` on request bytes exactly as given and returns the
    /// raw id list bytes. For conformance harnesses.
    pub fn sources_raw(&self, request: &[u8]) -> Result<Vec<u8>> {
        let mut call = self.instantiate()?;
        let (rp, rl) = call.write(request)?;
        let packed = call.call_i64("gx_sources", &[rp, rl])?;
        if packed == -1 {
            return Err(HostError::ModuleFailed(call.last_error()?));
        }
        call.take(packed)
    }
}

/// Turns a wasmtime error into a [`HostError`], giving the limits
/// precedence: a module that hit either limit has failed for that reason,
/// whatever trap followed.
fn classify(store: &Store<Limits>, config: HostConfig, e: wasmtime::Error) -> HostError {
    if store.data().memory_exceeded {
        return HostError::MemoryLimit(config.memory_limit);
    }
    match e.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => HostError::TimeLimit(config.time_limit),
        Some(t) => HostError::Trap(t.to_string()),
        None => HostError::Engine(format!("{e:#}")),
    }
}

fn unpack(packed: i64) -> (u32, u32) {
    let v = packed as u64;
    ((v >> 32) as u32, v as u32)
}

impl Call {
    fn abi_version(&mut self) -> Result<i32> {
        let f = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, "gx_abi_version")
            .map_err(|e| HostError::Export(format!("{e:#}")))?;
        let r = f.call(&mut self.store, ());
        self.finish(r)
    }

    /// Maps a call result, failing if the module hit the memory limit even
    /// when the call itself returned.
    fn finish<T>(&mut self, r: wasmtime::Result<T>) -> Result<T> {
        match r {
            Ok(_) if self.store.data().memory_exceeded => {
                Err(HostError::MemoryLimit(self.config.memory_limit))
            }
            Ok(v) => Ok(v),
            Err(e) => Err(classify(&self.store, self.config, e)),
        }
    }

    fn call_i64(&mut self, name: &str, args: &[i32]) -> Result<i64> {
        let r = match args {
            [a, b] => {
                let f = self
                    .instance
                    .get_typed_func::<(i32, i32), i64>(&mut self.store, name)
                    .map_err(|e| HostError::Export(format!("{e:#}")))?;
                f.call(&mut self.store, (*a, *b))
            }
            [a, b, c, d] => {
                let f = self
                    .instance
                    .get_typed_func::<(i32, i32, i32, i32), i64>(&mut self.store, name)
                    .map_err(|e| HostError::Export(format!("{e:#}")))?;
                f.call(&mut self.store, (*a, *b, *c, *d))
            }
            _ => unreachable!("only the two and four argument exports are called"),
        };
        self.finish(r)
    }

    /// Copies `bytes` into a block from `gx_alloc` and returns its pointer
    /// and length as the ABI's `i32` pair.
    fn write(&mut self, bytes: &[u8]) -> Result<(i32, i32)> {
        let len = i32::try_from(bytes.len())
            .map_err(|_| HostError::Abi(format!("{} bytes do not fit an i32", bytes.len())))?;
        let alloc = self
            .instance
            .get_typed_func::<i32, i32>(&mut self.store, "gx_alloc")
            .map_err(|e| HostError::Export(format!("{e:#}")))?;
        let r = alloc.call(&mut self.store, len);
        let ptr = self.finish(r)?;
        if ptr == 0 && len > 0 {
            return Err(HostError::Abi(format!("gx_alloc({len}) returned 0")));
        }
        if len > 0 {
            self.memory
                .write(&mut self.store, ptr as u32 as usize, bytes)
                .map_err(|_| {
                    HostError::Abi(format!(
                        "gx_alloc({len}) returned {} outside memory",
                        ptr as u32
                    ))
                })?;
        }
        Ok((ptr, len))
    }

    /// Copies the block `(ptr << 32) | len` out of the module's memory.
    fn read(&mut self, packed: i64) -> Result<Vec<u8>> {
        let (ptr, len) = unpack(packed);
        let mut out = vec![0u8; len as usize];
        self.memory
            .read(&self.store, ptr as usize, &mut out)
            .map_err(|_| {
                HostError::Abi(format!("block at {ptr} of {len} bytes is outside memory"))
            })?;
        Ok(out)
    }

    /// Copies a returned block out and frees it with `gx_free`.
    fn take(&mut self, packed: i64) -> Result<Vec<u8>> {
        let out = self.read(packed)?;
        let (ptr, len) = unpack(packed);
        let free = self
            .instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, "gx_free")
            .map_err(|e| HostError::Export(format!("{e:#}")))?;
        let r = free.call(&mut self.store, (ptr as i32, len as i32));
        self.finish(r)?;
        Ok(out)
    }

    /// The reason `gx_last_error` reports, as UTF-8 with invalid sequences
    /// replaced.
    fn last_error(&mut self) -> Result<String> {
        let f = self
            .instance
            .get_typed_func::<(), i64>(&mut self.store, "gx_last_error")
            .map_err(|e| HostError::Export(format!("{e:#}")))?;
        let r = f.call(&mut self.store, ());
        let packed = self.finish(r)?;
        let bytes = self.read(packed)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine refuses relaxed SIMD and threads on its own, without the
    /// static checks in front of it.
    #[test]
    fn engine_rejects_forbidden_proposals() {
        let engine = Engine::new(&engine_config()).unwrap();
        let relaxed = wat::parse_str(
            r#"(module (func (result v128)
                (i8x16.relaxed_swizzle (v128.const i64x2 0 0) (v128.const i64x2 0 0))))"#,
        )
        .unwrap();
        let e = Module::new(&engine, &relaxed).err().unwrap();
        assert!(format!("{e:#}").contains("relaxed SIMD"), "{e:#}");
        let threads = wat::parse_str("(module (func (atomic.fence)))").unwrap();
        assert!(Module::new(&engine, &threads).is_err());
        let simd = wat::parse_str(
            "(module (func (result v128) (i32x4.add (v128.const i64x2 0 0) (v128.const i64x2 0 0))))",
        )
        .unwrap();
        assert!(Module::new(&engine, &simd).is_ok());
    }
}
