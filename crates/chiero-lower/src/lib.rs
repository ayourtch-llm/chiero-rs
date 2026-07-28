//! `chiero-lower` — the typed AST to CIR. See `docs/specs/015-lowering.md`.
//!
//! **Every construct lowers to a fixed shape** (015 §1). Two lowerings of one construct
//! must produce identical CIR, because golden `.cir` files are contracts (020 §6) and the
//! differential harness diffs them — so a choice left free here is a golden that changes
//! for no reason.
//!
//! **Lowering never infers a conversion.** 014 §5 already made every implicit conversion
//! an explicit `Cast` node; if this crate finds itself needing one, that is a
//! `chiero-sema` bug and not a lowering fix.

use chiero_ast::Ast;
use chiero_cir::Module;
use chiero_sema::{Analysis, SymbolText};
use chiero_span::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowerDiagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Debug)]
pub struct Lowered {
    pub module: Module,
    /// 015 §7: constructs lowering **refuses** rather than lowering wrongly. A function
    /// that cannot be represented is absent from the module and named here.
    pub diagnostics: Vec<LowerDiagnostic>,
}

/// Lower one translation unit (015 §§1–6).
pub fn lower_tu(ast: &Ast, analysis: &Analysis, names: &dyn SymbolText) -> Lowered {
    let _ = (ast, analysis, names);
    todo!("015 §§1-6: AST to CIR")
}
