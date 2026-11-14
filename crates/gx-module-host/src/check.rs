//! Static checks on module bytes before anything is compiled.
//!
//! Implements the admission rules of `client-compute.md` section 4: a module
//! is a WebAssembly 1.0 binary using only the mutable-globals,
//! sign-extension, non-trapping float-to-int, bulk-memory, and fixed-width
//! SIMD features, with no imports. Relaxed SIMD and threads, the two
//! features whose results may differ between machines (`space-model.md`
//! section 4 item 6), are named explicitly in the error so the author knows
//! what to remove.

use wasmparser::{
    for_each_visit_operator, for_each_visit_simd_operator, Parser, Payload, TypeRef, Validator,
    VisitOperator, VisitSimdOperator, WasmFeatures,
};

use crate::HostError;

/// The WebAssembly features a module may use: WebAssembly 1.0 plus
/// mutable globals, sign extension, non-trapping float-to-int, bulk memory,
/// and fixed-width SIMD (`client-compute.md` section 4).
pub const ALLOWED_FEATURES: WasmFeatures = WasmFeatures::FLOATS
    .union(WasmFeatures::MUTABLE_GLOBAL)
    .union(WasmFeatures::SIGN_EXTENSION)
    .union(WasmFeatures::SATURATING_FLOAT_TO_INT)
    .union(WasmFeatures::BULK_MEMORY)
    .union(WasmFeatures::SIMD);

/// The proposals whose instructions a module may contain, as named by
/// `wasmparser`'s operator table.
const ALLOWED_PROPOSALS: [&str; 5] = [
    "mvp",
    "sign_extension",
    "saturating_float_to_int",
    "bulk_memory",
    "simd",
];

/// One instruction a module is not allowed to contain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForbiddenOpcode {
    /// Byte offset of the instruction in the module.
    pub offset: u64,
    /// The proposal it belongs to, for example `relaxed_simd` or `threads`.
    pub proposal: &'static str,
    /// The instruction, as `wasmparser` prints it.
    pub instruction: String,
}

/// Visitor that answers which proposal an instruction belongs to.
struct Proposal;

macro_rules! define_proposal_visit {
    ($( @$proposal:ident $op:ident $({ $($arg:ident: $argty:ty),* })? => $visit:ident ($($ann:tt)*))*) => {
        $(
            fn $visit(&mut self $($(, $arg: $argty)*)?) -> &'static str {
                $($(let _ = $arg;)*)?
                stringify!($proposal)
            }
        )*
    };
}

impl<'a> VisitOperator<'a> for Proposal {
    type Output = &'static str;

    fn simd_visitor(&mut self) -> Option<&mut dyn VisitSimdOperator<'a, Output = Self::Output>> {
        Some(self)
    }

    for_each_visit_operator!(define_proposal_visit);
}

impl<'a> VisitSimdOperator<'a> for Proposal {
    for_each_visit_simd_operator!(define_proposal_visit);
}

/// Every instruction in the module's function bodies whose proposal is not
/// allowed, in byte order. Parses with every feature enabled so a relaxed
/// SIMD or threads instruction is found and named rather than reported as a
/// generic validation failure.
pub fn forbidden_opcodes(bytes: &[u8]) -> Result<Vec<ForbiddenOpcode>, HostError> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| HostError::Invalid(e.to_string()))?;
        if let Payload::CodeSectionEntry(body) = payload {
            let mut ops = body
                .get_operators_reader()
                .map_err(|e| HostError::Invalid(e.to_string()))?;
            while !ops.eof() {
                let offset = ops.original_position();
                let proposal = ops
                    .clone()
                    .visit_operator(&mut Proposal)
                    .map_err(|e| HostError::Invalid(e.to_string()))?;
                let op = ops.read().map_err(|e| HostError::Invalid(e.to_string()))?;
                if !ALLOWED_PROPOSALS.contains(&proposal) {
                    out.push(ForbiddenOpcode {
                        offset,
                        proposal,
                        instruction: format!("{op:?}"),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// The module's imports as `(module, name)` pairs, in order.
pub fn imports(bytes: &[u8]) -> Result<Vec<(String, String)>, HostError> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| HostError::Invalid(e.to_string()))?;
        if let Payload::ImportSection(reader) = payload {
            for import in reader.into_imports() {
                let import = import.map_err(|e| HostError::Invalid(e.to_string()))?;
                let _: TypeRef = import.ty;
                out.push((import.module.to_string(), import.name.to_string()));
            }
        }
    }
    Ok(out)
}

/// Runs every static admission check, in order: the bytes parse, the module
/// has no imports, no instruction belongs to a forbidden proposal, and the
/// module validates with only [`ALLOWED_FEATURES`] enabled.
pub fn check_module(bytes: &[u8]) -> Result<(), HostError> {
    if bytes.len() < 8 || bytes[0..4] != *b"\0asm" {
        return Err(HostError::Invalid(
            "not a WebAssembly binary: bad magic".into(),
        ));
    }
    if bytes[4..8] != [1, 0, 0, 0] {
        return Err(HostError::Invalid(
            "not a WebAssembly 1.0 core module: bad version".into(),
        ));
    }
    if let Some((module, name)) = imports(bytes)?.into_iter().next() {
        return Err(HostError::Import { module, name });
    }
    if let Some(f) = forbidden_opcodes(bytes)?.into_iter().next() {
        return Err(HostError::ForbiddenOpcode(f));
    }
    Validator::new_with_features(ALLOWED_FEATURES)
        .validate_all(bytes)
        .map_err(|e| HostError::Invalid(e.to_string()))?;
    Ok(())
}
