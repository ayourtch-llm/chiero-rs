//! `chiero-parse` — C11 + GNU translation phases 5–7. See `docs/specs/013-parser.md`.
//!
//! Hand-written recursive descent with operator-precedence expressions. Not a generator:
//! C's grammar needs context feedback (§3), error recovery has to survive a 1M-line
//! codebase, and every node must carry a provenance-bearing `Span`.
//!
//! **The parser never returns `Err`** (§1). A malformed TU produces diagnostics and
//! `Error` nodes and keeps going.

use chiero_ast::Ast;
use chiero_pp::PreprocessedTu;
use chiero_span::{Span, Symbol};
use indexmap::IndexMap;
use std::sync::Arc;

/// 013 §3. `A * B;` is a declaration if `A` is a typedef name and a multiplication
/// otherwise, so the parser needs a symbol table *while parsing*. `chiero-sema` owns real
/// scoping; this is the minimal window into it.
///
/// This is the one place the phase separation is deliberately broken. C leaves no
/// alternative. The interface is kept to names, so the parser still knows nothing about
/// types.
pub trait TypedefOracle {
    fn is_typedef_name(&self, sym: Symbol) -> bool;
    fn enter_scope(&mut self);
    fn exit_scope(&mut self);
    fn declare(&mut self, sym: Symbol, is_typedef: bool);
}

/// A scope stack of names, sufficient for standalone parsing and for tests.
///
/// It lives here rather than in `chiero-sema` because `chiero-parse` is *below* sema in
/// the 001 §2 layering and cannot depend on it, so a parser with no oracle available
/// would be unusable on its own. Tracking which names are typedefs is not type
/// knowledge — sema still owns everything that follows from a name being a type.
#[derive(Debug, Default)]
pub struct ScopedTypedefs {
    scopes: Vec<IndexMap<Symbol, bool>>,
}

impl ScopedTypedefs {
    pub fn new() -> ScopedTypedefs {
        ScopedTypedefs {
            scopes: vec![IndexMap::new()],
        }
    }
}

impl TypedefOracle for ScopedTypedefs {
    fn is_typedef_name(&self, sym: Symbol) -> bool {
        // Innermost wins: a parameter named the same as a typedef shadows it (013 §3).
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(&sym))
            .copied()
            .unwrap_or(false)
    }

    fn enter_scope(&mut self) {
        self.scopes.push(IndexMap::new());
    }

    fn exit_scope(&mut self) {
        // Never pop the file scope: an unbalanced `}` in a malformed TU would otherwise
        // leave the parser with no scope at all, and every subsequent lookup would panic
        // rather than degrade.
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn declare(&mut self, sym: Symbol, is_typedef: bool) {
        if let Some(top) = self.scopes.last_mut() {
            top.insert(sym, is_typedef);
        }
    }
}

/// A parser diagnostic.
///
/// 010 §7's shared `Diagnostic` — with macro-backtrace rendering — is still owed, so this
/// is a local shape with the same fields rather than a premature abstraction over a type
/// that does not exist yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub span: Span,
    pub message: String,
}

/// §6's cap. Beyond it the parser continues silently and records that it truncated — a
/// cascade flood is not more informative than its first hundred lines.
pub const MAX_DIAGNOSTICS: usize = 100;

#[derive(Debug)]
pub struct ParsedTu {
    pub ast: Ast,
    pub diagnostics: Vec<ParseDiagnostic>,
    /// Set when diagnostics hit [`MAX_DIAGNOSTICS`] and were dropped after that
    /// (contract 16). Without it a truncated run is indistinguishable from a clean one
    /// that happened to find exactly a hundred problems.
    pub truncated: bool,
    /// **The only interner the AST's symbols index.**
    ///
    /// The parser re-interns identifiers rather than reusing the lexer's ids, so every
    /// `Symbol` in the tree — an identifier, a number's spelling, a member name — comes
    /// from one space. Mixing two structurally identical id spaces in one enum would let
    /// a lexer id be read against this table and produce a wrong *name* in a diagnostic,
    /// which no type error would catch.
    spellings: Vec<Arc<str>>,
}

impl ParsedTu {
    /// The text a `Symbol` in this tree stands for.
    pub fn text(&self, sym: Symbol) -> Option<&str> {
        self.spellings.get(sym.0 as usize).map(AsRef::as_ref)
    }
}

/// Parse a preprocessed TU (013 §§2, 5–7).
pub fn parse_tu(tu: &PreprocessedTu, oracle: &mut dyn TypedefOracle) -> ParsedTu {
    let _ = (tu, oracle);
    todo!("013 §§5-7: recursive descent")
}
