//! `chiero-parse` — C11 + GNU translation phases 5–7. See `docs/specs/013-parser.md`.
//!
//! Hand-written recursive descent with operator-precedence expressions. Not a generator:
//! C's grammar needs context feedback (§3), error recovery has to survive a 1M-line
//! codebase, and every node must carry a provenance-bearing `Span`.
//!
//! **The parser never returns `Err`** (§1). A malformed TU produces diagnostics and
//! `Error` nodes and keeps going.

use chiero_ast::{
    ArrayLen, AsmOperand, AsmStmt, Ast, Attr, BinOp, Builtin, DeclId, DeclKind, Designator, ExprId,
    ExprKind, ForInit, InitItem, PostfixOp, Quals, StmtId, StmtKind, Storage, StrFragment, TagKind,
    TypeId, TypeKind, UnOp,
};
use chiero_lex::{PpTokenKind, Punct};
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
    let mut p = Parser::new(tu, oracle);
    p.translation_unit();
    p.finish()
}

/// Phase 7's tokens, as distinct from phase 3's pp-tokens.
///
/// The difference that matters is **keywords**: the lexer hands back `Ident` for `int`,
/// because at phase 3 `int` may still be a macro name. Recognizing keywords is therefore
/// the parser's job, done once in the pre-pass rather than by comparing spellings at every
/// decision point — the typedef test alone asks "is this a type?" of most identifiers in a
/// TU, and a string compare there would be the parser's hot loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TokKind {
    Kw(Kw),
    Ident(Symbol),
    /// A numeric literal's spelling. 013 §5: the AST records what was written.
    Number(Symbol),
    Char(Symbol),
    Str(Symbol),
    Punct(Punct),
    /// A pp-token that is not a C token — a stray `@`, `$` or backslash. The lexer keeps
    /// these rather than failing (011 §4), so the parser has to have somewhere to put
    /// them.
    Other,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Tok {
    kind: TokKind,
    span: Span,
}

/// C11 keywords plus the GNU spellings 013 §4 measured in VPP.
///
/// The `__`-prefixed aliases are not cosmetic: `__inline__`, `__restrict` and
/// `__typeof__` are what appears inside glibc headers, which every VPP TU includes, so a
/// parser that knew only the unprefixed spellings would fail on the standard library
/// before reaching any VPP code.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Kw {
    // type specifiers
    Void,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Signed,
    Unsigned,
    Bool,
    Int128,
    Complex,
    Struct,
    Union,
    Enum,
    Typeof,
    // storage-class specifiers and function specifiers
    Typedef,
    Extern,
    Static,
    Auto,
    Register,
    Inline,
    Noreturn,
    ThreadLocal,
    // qualifiers
    Const,
    Volatile,
    Restrict,
    Atomic,
    // operators that are keywords
    Sizeof,
    Alignof,
    Alignas,
    StaticAssert,
    // statements
    If,
    Else,
    While,
    Do,
    For,
    Switch,
    Case,
    Default,
    Break,
    Continue,
    Return,
    Goto,
    // GNU
    Attribute,
    Asm,
    Extension,
    Label,
    Generic,
}

fn keyword(text: &str) -> Option<Kw> {
    Some(match text {
        "void" => Kw::Void,
        "char" => Kw::Char,
        "short" => Kw::Short,
        "int" => Kw::Int,
        "long" => Kw::Long,
        "float" => Kw::Float,
        "double" => Kw::Double,
        "signed" | "__signed" | "__signed__" => Kw::Signed,
        "unsigned" => Kw::Unsigned,
        "_Bool" => Kw::Bool,
        "__int128" | "__int128_t" => Kw::Int128,
        "_Complex" | "__complex__" => Kw::Complex,
        "struct" => Kw::Struct,
        "union" => Kw::Union,
        "enum" => Kw::Enum,
        "typeof" | "__typeof" | "__typeof__" => Kw::Typeof,
        "typedef" => Kw::Typedef,
        "extern" => Kw::Extern,
        "static" => Kw::Static,
        "auto" => Kw::Auto,
        "register" => Kw::Register,
        "inline" | "__inline" | "__inline__" => Kw::Inline,
        "_Noreturn" => Kw::Noreturn,
        "_Thread_local" | "__thread" => Kw::ThreadLocal,
        "const" | "__const" | "__const__" => Kw::Const,
        "volatile" | "__volatile" | "__volatile__" => Kw::Volatile,
        "restrict" | "__restrict" | "__restrict__" => Kw::Restrict,
        "_Atomic" => Kw::Atomic,
        "sizeof" => Kw::Sizeof,
        "_Alignof" | "__alignof" | "__alignof__" => Kw::Alignof,
        "_Alignas" => Kw::Alignas,
        "_Static_assert" => Kw::StaticAssert,
        "if" => Kw::If,
        "else" => Kw::Else,
        "while" => Kw::While,
        "do" => Kw::Do,
        "for" => Kw::For,
        "switch" => Kw::Switch,
        "case" => Kw::Case,
        "default" => Kw::Default,
        "break" => Kw::Break,
        "continue" => Kw::Continue,
        "return" => Kw::Return,
        "goto" => Kw::Goto,
        "__attribute__" | "__attribute" => Kw::Attribute,
        "asm" | "__asm" | "__asm__" => Kw::Asm,
        "__extension__" => Kw::Extension,
        "__label__" => Kw::Label,
        "_Generic" => Kw::Generic,
        _ => return None,
    })
}

/// The declaration specifiers of one declaration, before any declarator is applied.
#[derive(Clone, Debug)]
struct Specs {
    ty: TypeId,
    storage: Storage,
    is_typedef: bool,
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    ast: Ast,
    diags: Vec<ParseDiagnostic>,
    truncated: bool,
    interner: IndexMap<Arc<str>, Symbol>,
    spellings: Vec<Arc<str>>,
    oracle: &'a mut dyn TypedefOracle,
    /// Guards the declarator double-scan (see [`Parser::declarator`]) and ordinary
    /// recursion. A 1M-line codebase contains generated files; a stack overflow is a
    /// `SIGABRT` that `catch_unwind` cannot contain, so depth is a diagnostic, never a
    /// crash — the same rule 012 had to learn twice.
    depth: u32,
}

const MAX_DEPTH: u32 = 200;

impl<'a> Parser<'a> {
    fn new(tu: &PreprocessedTu, oracle: &'a mut dyn TypedefOracle) -> Parser<'a> {
        let mut p = Parser {
            toks: Vec::with_capacity(tu.tokens.len()),
            pos: 0,
            ast: Ast::new(),
            diags: Vec::new(),
            truncated: false,
            interner: IndexMap::new(),
            spellings: Vec::new(),
            oracle,
            depth: 0,
        };
        for (i, t) in tu.tokens.iter().enumerate() {
            let text = tu.text_at(i).unwrap_or("");
            let kind = match &t.kind {
                PpTokenKind::Ident(_) => match keyword(text) {
                    // `__extension__` is defer-able exactly because it is a no-op: GNU
                    // uses it only to silence pedantic warnings. Dropping the token here
                    // is the whole implementation.
                    Some(Kw::Extension) => continue,
                    Some(k) => TokKind::Kw(k),
                    // Re-interned rather than carrying the lexer's `Symbol`: see
                    // `ParsedTu::spellings`.
                    None => TokKind::Ident(p.intern(text)),
                },
                PpTokenKind::Number => TokKind::Number(p.intern(text)),
                PpTokenKind::CharLit { .. } => TokKind::Char(p.intern(text)),
                PpTokenKind::StringLit { .. } => TokKind::Str(p.intern(text)),
                PpTokenKind::Punct(punct) => TokKind::Punct(*punct),
                PpTokenKind::Other(_) => TokKind::Other,
                PpTokenKind::Eof => continue,
            };
            p.toks.push(Tok { kind, span: t.span });
        }
        p
    }

    fn finish(self) -> ParsedTu {
        ParsedTu {
            ast: self.ast,
            diagnostics: self.diags,
            truncated: self.truncated,
            spellings: self.spellings,
        }
    }

    fn intern(&mut self, text: &str) -> Symbol {
        if let Some(&s) = self.interner.get(text) {
            return s;
        }
        let arc: Arc<str> = Arc::from(text);
        let sym = Symbol(self.spellings.len() as u32);
        self.spellings.push(Arc::clone(&arc));
        self.interner.insert(arc, sym);
        sym
    }

    // ---- token access ----

    fn peek_at(&self, n: usize) -> Option<Tok> {
        self.toks.get(self.pos + n).copied()
    }

    fn peek(&self) -> Option<Tok> {
        self.peek_at(0)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    /// A zero-width span at the current position, for a node the parser synthesized.
    ///
    /// 013 §5 requires a real position rather than a fabricated range, and `Span::DUMMY`
    /// is not a position at all — a diagnostic carrying one cannot be rendered against
    /// source. At end of input this is the end of the last token, which is where a
    /// "missing `}`" belongs.
    fn here(&self) -> Span {
        match self.peek() {
            Some(t) => Span {
                lo: t.span.lo,
                hi: t.span.lo,
                ctx: t.span.ctx,
            },
            None => match self.toks.last() {
                Some(t) => Span {
                    lo: t.span.hi,
                    hi: t.span.hi,
                    ctx: t.span.ctx,
                },
                None => Span::DUMMY,
            },
        }
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.peek();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn is_punct(&self, n: usize, p: Punct) -> bool {
        matches!(self.peek_at(n), Some(t) if t.kind == TokKind::Punct(p))
    }

    fn is_kw(&self, n: usize, k: Kw) -> bool {
        matches!(self.peek_at(n), Some(t) if t.kind == TokKind::Kw(k))
    }

    fn eat_punct(&mut self, p: Punct) -> bool {
        if self.is_punct(0, p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.is_kw(0, k) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: Punct, what: &str) -> bool {
        if self.eat_punct(p) {
            return true;
        }
        let here = self.here();
        self.error(here, format!("expected `{}` {what}", punct_text(p)));
        false
    }

    /// Join two spans, or refuse to.
    ///
    /// **A node whose first and last token came from different expansions gets the first
    /// token's span alone.** Splicing `lo` from one `ExpnCtx` onto `hi` from another
    /// produces a range that exists in neither, which is exactly the fabrication 010 §4
    /// forbids — and it would map through `expansion_loc` to a plausible, wrong location
    /// rather than to an obvious failure. `a + MACRO(b)` is the ordinary case, not a rare
    /// one, so this is a hot path and not a corner.
    fn join(&self, a: Span, b: Span) -> Span {
        if a.ctx == b.ctx && b.hi >= a.lo {
            Span {
                lo: a.lo,
                hi: b.hi,
                ctx: a.ctx,
            }
        } else {
            a
        }
    }

    /// The span from `start` (a token index) through the token before the cursor.
    fn span_from(&self, start: usize) -> Span {
        let first = self.toks.get(start).map(|t| t.span);
        let last = self
            .pos
            .checked_sub(1)
            .and_then(|i| self.toks.get(i))
            .map(|t| t.span);
        match (first, last) {
            (Some(a), Some(b)) => self.join(a, b),
            (Some(a), None) => a,
            _ => self.here(),
        }
    }

    // ---- diagnostics and recovery (§6) ----

    fn error(&mut self, span: Span, message: impl Into<String>) {
        if self.diags.len() >= MAX_DIAGNOSTICS {
            // §6: continue silently, but *record* that we did. A run that hit the cap and
            // one that found exactly a hundred problems are different facts.
            self.truncated = true;
            return;
        }
        self.diags.push(ParseDiagnostic {
            span,
            message: message.into(),
        });
    }

    /// Resynchronize to the nearest enclosing boundary, in §6's order of preference:
    /// statement (`;`), then block (`}`).
    ///
    /// Brace depth is tracked so a `;` *inside* a nested block does not look like the end
    /// of the statement we are recovering from.
    fn recover_to_boundary(&mut self) {
        let mut brace = 0i32;
        while let Some(t) = self.peek() {
            match t.kind {
                TokKind::Punct(Punct::LBrace) => {
                    brace += 1;
                    self.pos += 1;
                }
                TokKind::Punct(Punct::RBrace) => {
                    if brace == 0 {
                        // Leave it for the caller: it closes a block we are inside.
                        return;
                    }
                    brace -= 1;
                    self.pos += 1;
                }
                TokKind::Punct(Punct::Semi) => {
                    self.pos += 1;
                    if brace <= 0 {
                        return;
                    }
                }
                _ => self.pos += 1,
            }
        }
    }

    // ---- translation unit ----

    fn translation_unit(&mut self) {
        while !self.at_end() {
            let before = self.pos;
            self.external_declaration();
            if self.pos == before {
                // Nothing consumed: an unexpected token at file scope. Report it and step
                // over exactly one token, so the loop cannot spin.
                let t = self.bump();
                let span = t.map(|t| t.span).unwrap_or_else(|| self.here());
                self.error(span, "expected a declaration");
            }
        }
    }

    fn external_declaration(&mut self) {
        if self.is_kw(0, Kw::StaticAssert) {
            if let Some(d) = self.static_assert() {
                self.ast.push_item(d);
            }
            return;
        }
        // A stray `;` at file scope is legal C and declares nothing.
        if self.eat_punct(Punct::Semi) {
            return;
        }
        // Top-level `asm ("...");` declares nothing either, but it is not an error and
        // must not resynchronize past whatever follows it.
        if self.is_kw(0, Kw::Asm) {
            self.asm_statement();
            return;
        }
        if !self.starts_declaration() {
            return;
        }
        let start = self.pos;
        let specs = self.declaration_specifiers();

        // `struct S { ... };` — specifiers and no declarator.
        if self.is_punct(0, Punct::Semi) {
            self.pos += 1;
            let span = self.span_from(start);
            let d = if specs.is_typedef {
                // `typedef struct {...};` declares no name; recorded as a tag definition
                // rather than dropped, because 031 must see the members.
                self.ast.add_decl(DeclKind::TagDef { ty: specs.ty }, span)
            } else {
                self.ast.add_decl(DeclKind::TagDef { ty: specs.ty }, span)
            };
            self.ast.push_item(d);
            return;
        }

        // First declarator decides whether this is a definition.
        //
        // The prototype scope is entered *before* the declarator, not after, because the
        // parameters have to be visible to each other as they are parsed: contract 2's
        // `void f(int T, T x)` turns on `T` being declared by the time `T x` is read.
        self.oracle.enter_scope();
        let (name, ty) = self.declarator(specs.ty, false);
        let is_func = matches!(self.ast.ty(ty).kind, TypeKind::Func { .. });

        // **K&R (contract 4).** An old-style declarator produced an identifier list with
        // no types; the types are in ordinary declarations between the `)` and the `{`.
        // They are read here, before the body, because the body's first statement may
        // shadow a parameter name and would then overwrite the type we are looking for.
        let kr = matches!(self.ast.ty(ty).kind, TypeKind::Func { kr: true, .. });
        if kr && !self.is_punct(0, Punct::LBrace) {
            self.kr_parameter_declarations(ty);
        }

        if is_func && self.is_punct(0, Punct::LBrace) {
            let Some(name) = name else {
                let span = self.span_from(start);
                self.error(span, "a function definition needs a name");
                self.oracle.exit_scope();
                self.recover_to_boundary();
                return;
            };
            // Declared in the *enclosing* scope, which is the one we are not in.
            self.declare_outer(name, false);
            let body = self.compound_statement(false);
            self.oracle.exit_scope();
            let span = self.span_from(start);
            let d = self.ast.add_decl(
                DeclKind::Func {
                    name,
                    ty,
                    body: Some(body),
                    storage: specs.storage,
                },
                span,
            );
            self.ast.push_item(d);
            return;
        }
        self.oracle.exit_scope();

        // An ordinary declaration, possibly with more declarators after a comma.
        let mut first = Some((name, ty));
        loop {
            let (name, ty) = match first.take() {
                Some(pair) => pair,
                None => {
                    self.oracle.enter_scope();
                    let pair = self.declarator(specs.ty, false);
                    self.oracle.exit_scope();
                    pair
                }
            };
            let init = if self.eat_punct(Punct::Eq) {
                Some(self.initializer())
            } else {
                None
            };
            let span = self.span_from(start);
            let d = self.finish_declarator(&specs, name, ty, init, span);
            self.ast.push_item(d);
            if self.eat_punct(Punct::Comma) {
                continue;
            }
            self.expect_punct(Punct::Semi, "after a declaration");
            return;
        }
    }

    /// The declarations between `)` and `{` in an old-style definition (contract 4).
    ///
    /// Each one types a name the identifier list already produced, so the parameter decls
    /// are **updated in place** rather than replaced: the identifier list fixed the
    /// parameter *order*, which these declarations do not have to follow, and rebuilding
    /// the list from them would silently reorder the arguments.
    ///
    /// A name declared here that is not a parameter is a diagnostic rather than a new
    /// local — that is what the construct means, and accepting it would let a typo
    /// disappear.
    fn kr_parameter_declarations(&mut self, func_ty: TypeId) {
        let params = match &self.ast.ty(func_ty).kind {
            TypeKind::Func { params, .. } => params.clone(),
            _ => return,
        };
        while !self.at_end() && !self.is_punct(0, Punct::LBrace) && self.starts_declaration() {
            let before = self.pos;
            let specs = self.declaration_specifiers();
            loop {
                let (name, ty) = self.declarator(specs.ty, false);
                let Some(name) = name else { break };
                let target = params.iter().copied().find(|&d| {
                    matches!(&self.ast.decl(d).kind,
                        DeclKind::Var { name: Some(n), .. } if *n == name)
                });
                match target {
                    Some(d) => {
                        if let DeclKind::Var { ty: slot, .. } = &mut self.ast.decl_mut(d).kind {
                            *slot = ty;
                        }
                        self.oracle.declare(name, false);
                    }
                    None => {
                        let span = self.span_from(before);
                        self.error(
                            span,
                            "this declaration names something that is not a parameter of \
                             the function it follows",
                        );
                    }
                }
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            self.expect_punct(Punct::Semi, "after an old-style parameter declaration");
            if self.pos == before {
                break;
            }
        }
    }

    /// Declare a name in the scope *outside* the prototype scope we are currently in.
    fn declare_outer(&mut self, name: Symbol, is_typedef: bool) {
        self.oracle.exit_scope();
        self.oracle.declare(name, is_typedef);
        self.oracle.enter_scope();
    }

    /// Build the declaration node and tell the oracle about the name.
    ///
    /// The `declare` call is what makes the typedef test work at all, and it has to happen
    /// **here** — after the declarator, before the next one — so that
    /// `typedef int T, *PT;` sees `T` while reading `PT`.
    fn finish_declarator(
        &mut self,
        specs: &Specs,
        name: Option<Symbol>,
        ty: TypeId,
        init: Option<ExprId>,
        span: Span,
    ) -> DeclId {
        // Specifier attributes already sit on `specs.ty`, which is either this type or is
        // reachable from it through the declarator's derivations.
        match (specs.is_typedef, name) {
            (true, Some(name)) => {
                self.oracle.declare(name, true);
                self.ast.add_decl(DeclKind::Typedef { name, ty }, span)
            }
            (true, None) => {
                self.error(span, "a typedef needs a name");
                self.ast.add_decl(DeclKind::Error, span)
            }
            (false, name) => {
                if let Some(n) = name {
                    self.oracle.declare(n, false);
                }
                if matches!(self.ast.ty(ty).kind, TypeKind::Func { .. }) {
                    match name {
                        Some(name) => self.ast.add_decl(
                            DeclKind::Func {
                                name,
                                ty,
                                body: None,
                                storage: specs.storage,
                            },
                            span,
                        ),
                        None => self.ast.add_decl(DeclKind::Error, span),
                    }
                } else {
                    self.ast.add_decl(
                        DeclKind::Var {
                            name,
                            ty,
                            init,
                            storage: specs.storage,
                        },
                        span,
                    )
                }
            }
        }
    }

    fn static_assert(&mut self) -> Option<DeclId> {
        let start = self.pos;
        self.pos += 1; // `_Static_assert`
        if !self.expect_punct(Punct::LParen, "after `_Static_assert`") {
            self.recover_to_boundary();
            return None;
        }
        let cond = self.assignment_expr();
        // C11 requires the message; C23 and GNU allow omitting it. Accepting both is
        // free, and rejecting the shorter form would fail on modern headers.
        let msg = if self.eat_punct(Punct::Comma) {
            match self.peek().map(|t| t.kind) {
                Some(TokKind::Str(s)) => {
                    self.pos += 1;
                    Some(s)
                }
                _ => {
                    let here = self.here();
                    self.error(here, "expected a string literal message");
                    None
                }
            }
        } else {
            None
        };
        self.expect_punct(Punct::RParen, "to close `_Static_assert`");
        self.expect_punct(Punct::Semi, "after `_Static_assert`");
        let span = self.span_from(start);
        Some(
            self.ast
                .add_decl(DeclKind::StaticAssert { cond, msg }, span),
        )
    }

    // ---- declaration specifiers ----

    /// Does the current position start a declaration rather than a statement?
    ///
    /// This is the typedef test (§3), and it is also where a wrong answer is invisible:
    /// treating `A * B;` as a declaration produces a plausible variable named `B` and no
    /// diagnostic at all.
    fn starts_declaration(&self) -> bool {
        match self.peek().map(|t| t.kind) {
            Some(TokKind::Kw(k)) => matches!(
                k,
                Kw::Void
                    | Kw::Char
                    | Kw::Short
                    | Kw::Int
                    | Kw::Long
                    | Kw::Float
                    | Kw::Double
                    | Kw::Signed
                    | Kw::Unsigned
                    | Kw::Bool
                    | Kw::Int128
                    | Kw::Complex
                    | Kw::Struct
                    | Kw::Union
                    | Kw::Enum
                    | Kw::Typeof
                    | Kw::Typedef
                    | Kw::Extern
                    | Kw::Static
                    | Kw::Auto
                    | Kw::Register
                    | Kw::Inline
                    | Kw::Noreturn
                    | Kw::ThreadLocal
                    | Kw::Const
                    | Kw::Volatile
                    | Kw::Restrict
                    | Kw::Atomic
                    | Kw::Alignas
                    | Kw::Attribute
                    | Kw::StaticAssert
            ),
            Some(TokKind::Ident(s)) => self.oracle.is_typedef_name(s),
            _ => false,
        }
    }

    fn declaration_specifiers(&mut self) -> Specs {
        let start = self.pos;
        let mut storage = Storage::default();
        let mut quals = Quals::default();
        let mut attrs = Vec::new();
        let mut is_typedef = false;
        // Longness and signedness accumulate; `unsigned long long int` is four tokens
        // naming one type, in any order.
        let mut sign: Option<bool> = None;
        let mut long_count = 0u32;
        let mut short_seen = false;
        let mut base: Option<Kw> = None;
        let mut tag_ty: Option<TypeId> = None;

        while let Some(t) = self.peek() {
            match t.kind {
                TokKind::Kw(Kw::Typedef) => {
                    is_typedef = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Extern) => {
                    storage.extern_ = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Static) => {
                    storage.static_ = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Auto) => {
                    storage.auto = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Register) => {
                    storage.register = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Inline) => {
                    storage.inline = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Noreturn) => {
                    storage.noreturn = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::ThreadLocal) => {
                    storage.thread_local = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Const) => {
                    quals.const_ = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Volatile) => {
                    quals.volatile_ = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Restrict) => {
                    quals.restrict_ = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Atomic) if !self.is_punct(1, Punct::LParen) => {
                    quals.atomic = true;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Attribute) => {
                    self.attribute_specifiers(&mut attrs);
                }
                TokKind::Kw(Kw::Alignas) => {
                    // Recorded as an attribute so 014 has one place to look for
                    // alignment, rather than two spellings of one fact.
                    self.pos += 1;
                    let name = self.intern("aligned");
                    let asp = self.here();
                    let mut args = Vec::new();
                    if self.eat_punct(Punct::LParen) {
                        args.push(self.assignment_expr());
                        self.expect_punct(Punct::RParen, "to close `_Alignas`");
                    }
                    attrs.push(Attr {
                        name,
                        args,
                        span: asp,
                    });
                }
                TokKind::Kw(Kw::Signed) => {
                    sign = Some(true);
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Unsigned) => {
                    sign = Some(false);
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Long) => {
                    long_count += 1;
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Short) => {
                    short_seen = true;
                    self.pos += 1;
                }
                TokKind::Kw(k @ (Kw::Void | Kw::Char | Kw::Int | Kw::Bool | Kw::Int128)) => {
                    base = Some(k);
                    self.pos += 1;
                }
                TokKind::Kw(k @ (Kw::Float | Kw::Double)) => {
                    base = Some(k);
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Complex) => {
                    // Parsed so a TU using it is not lost; 014 owns what it means.
                    self.pos += 1;
                }
                TokKind::Kw(k @ (Kw::Struct | Kw::Union | Kw::Enum)) if tag_ty.is_none() => {
                    tag_ty = Some(self.tag_specifier(k));
                }
                TokKind::Kw(Kw::Typeof) if tag_ty.is_none() && base.is_none() => {
                    tag_ty = Some(self.typeof_specifier());
                }
                // A typedef name is a type specifier — but only if we have not already
                // seen one. `T x` has `T` as the type; in `long T` the `T` is the
                // declarator's name, and consuming it here would silently rename the
                // declaration.
                TokKind::Ident(s)
                    if tag_ty.is_none()
                        && base.is_none()
                        && sign.is_none()
                        && long_count == 0
                        && !short_seen
                        && self.oracle.is_typedef_name(s) =>
                {
                    let span = t.span;
                    self.pos += 1;
                    tag_ty = Some(self.ast.add_type(TypeKind::Named(s), span));
                }
                _ => break,
            }
        }

        let span = self.span_from(start);
        let ty = match tag_ty {
            Some(t) => t,
            None => {
                let b = builtin_of(base, sign, long_count, short_seen);
                match b {
                    Some(b) => self.ast.add_type(TypeKind::Builtin(b), span),
                    None => {
                        // No type specifier at all. C89's implicit `int` is not accepted
                        // silently: in preprocessed VPP the overwhelmingly likelier cause
                        // is that a macro did not expand, and guessing `int` would turn
                        // that into a plausible declaration instead of a diagnostic.
                        self.error(span, "expected a type specifier");
                        self.ast.add_type(TypeKind::Error, span)
                    }
                }
            }
        };
        self.ast.ty_mut(ty).quals = quals;
        // **The specifier's attributes go on the specifier node, here and once.**
        // `__attribute__((packed)) struct S a, b;` really does apply to both declarators,
        // and they share this node — so attaching them per declarator would add them to
        // the shared node once per name instead of once.
        self.ast.ty_mut(ty).attrs.extend(attrs.iter().cloned());
        Specs {
            ty,
            storage,
            is_typedef,
        }
    }

    fn tag_specifier(&mut self, kw: Kw) -> TypeId {
        let start = self.pos;
        self.pos += 1; // struct/union/enum
        let tag = match kw {
            Kw::Struct => TagKind::Struct,
            Kw::Union => TagKind::Union,
            _ => TagKind::Enum,
        };
        let mut attrs = Vec::new();
        // `struct __attribute__((packed)) S {...}` — after the keyword (013 §4).
        self.attribute_specifiers(&mut attrs);
        let name = match self.peek().map(|t| t.kind) {
            Some(TokKind::Ident(s)) => {
                self.pos += 1;
                Some(s)
            }
            _ => None,
        };
        let members = if self.eat_punct(Punct::LBrace) {
            let mut out = Vec::new();
            while !self.at_end() && !self.is_punct(0, Punct::RBrace) {
                let before = self.pos;
                if tag == TagKind::Enum {
                    self.enumerator(&mut out);
                } else {
                    self.struct_member(&mut out);
                }
                if self.pos == before {
                    let t = self.bump();
                    let span = t.map(|t| t.span).unwrap_or_else(|| self.here());
                    self.error(span, "expected a member declaration");
                }
            }
            if !self.eat_punct(Punct::RBrace) {
                let here = self.here();
                self.error(here, "unclosed `{` in a struct, union or enum definition");
            }
            // `Some(vec![])` for `struct S {};` — an empty definition is not a reference,
            // and 014 has to be able to tell.
            Some(out)
        } else {
            None
        };
        if name.is_none() && members.is_none() {
            let here = self.here();
            self.error(here, "expected a tag name or a `{`");
        }
        let span = self.span_from(start);
        let ty = self
            .ast
            .add_type(TypeKind::Tag { tag, name, members }, span);
        // `struct S {...} __attribute__((packed))` — after the closing brace.
        self.attribute_specifiers(&mut attrs);
        self.ast.ty_mut(ty).attrs = attrs;
        ty
    }

    fn struct_member(&mut self, out: &mut Vec<DeclId>) {
        if self.is_kw(0, Kw::StaticAssert) {
            if let Some(d) = self.static_assert() {
                out.push(d);
            }
            return;
        }
        if self.eat_punct(Punct::Semi) {
            return;
        }
        let start = self.pos;
        let specs = self.declaration_specifiers();
        if self.eat_punct(Punct::Semi) {
            // An anonymous struct or union member.
            let span = self.span_from(start);
            let d = self.ast.add_decl(
                DeclKind::Var {
                    name: None,
                    ty: specs.ty,
                    init: None,
                    storage: specs.storage,
                },
                span,
            );
            out.push(d);
            return;
        }
        loop {
            let (name, ty) = self.declarator(specs.ty, true);
            // A bit-field. Its width is kept as an expression, unevaluated, because
            // `CLIB_CACHE_LINE_BYTES * 8` is the interesting case and 014 folds it.
            if self.eat_punct(Punct::Colon) {
                let _width = self.assignment_expr();
            }
            let mut attrs = Vec::new();
            self.attribute_specifiers(&mut attrs);
            if !attrs.is_empty() {
                self.ast.ty_mut(ty).attrs.extend(attrs);
            }
            let span = self.span_from(start);
            let d = self.finish_member(&specs, name, ty, span);
            out.push(d);
            if self.eat_punct(Punct::Comma) {
                continue;
            }
            self.expect_punct(Punct::Semi, "after a struct member");
            return;
        }
    }

    /// A member is never a typedef and never declares a name in the enclosing scope, so
    /// it deliberately does not go through `finish_declarator`.
    fn finish_member(
        &mut self,
        specs: &Specs,
        name: Option<Symbol>,
        ty: TypeId,
        span: Span,
    ) -> DeclId {
        // Specifier attributes already sit on `specs.ty`, which is either this type or is
        // reachable from it through the declarator's derivations.
        self.ast.add_decl(
            DeclKind::Var {
                name,
                ty,
                init: None,
                storage: specs.storage,
            },
            span,
        )
    }

    fn enumerator(&mut self, out: &mut Vec<DeclId>) {
        let start = self.pos;
        let Some(TokKind::Ident(name)) = self.peek().map(|t| t.kind) else {
            let here = self.here();
            self.error(here, "expected an enumerator name");
            return;
        };
        self.pos += 1;
        let init = if self.eat_punct(Punct::Eq) {
            Some(self.assignment_expr())
        } else {
            None
        };
        // An enumerator is an ordinary name for the typedef test's purposes.
        self.oracle.declare(name, false);
        let span = self.span_from(start);
        let ty = self.ast.add_type(TypeKind::Builtin(Builtin::Int), span);
        let d = self.ast.add_decl(
            DeclKind::Var {
                name: Some(name),
                ty,
                init,
                storage: Storage::default(),
            },
            span,
        );
        out.push(d);
        self.eat_punct(Punct::Comma);
    }

    fn typeof_specifier(&mut self) -> TypeId {
        let start = self.pos;
        self.pos += 1; // typeof
        if !self.expect_punct(Punct::LParen, "after `typeof`") {
            let span = self.span_from(start);
            return self.ast.add_type(TypeKind::Error, span);
        }
        let kind = if self.starts_type_name() {
            let inner = self.type_name();
            TypeKind::TypeofType(inner)
        } else {
            TypeKind::TypeofExpr(self.expression())
        };
        self.expect_punct(Punct::RParen, "to close `typeof`");
        let span = self.span_from(start);
        self.ast.add_type(kind, span)
    }

    /// `__attribute__ ((...))`, zero or more times.
    ///
    /// 013 §4: attributes appear in positions the C grammar does not anticipate, so this
    /// is called at each of them rather than at one canonical place. The argument list is
    /// parsed as expressions and left unevaluated — `aligned(CLIB_CACHE_LINE_BYTES)` is
    /// exactly the case 031 wants to see the macro in.
    fn attribute_specifiers(&mut self, out: &mut Vec<Attr>) {
        while self.is_kw(0, Kw::Attribute) {
            self.pos += 1;
            if !self.expect_punct(Punct::LParen, "after `__attribute__`") {
                return;
            }
            if !self.expect_punct(Punct::LParen, "after `__attribute__ (`") {
                return;
            }
            while !self.at_end() && !self.is_punct(0, Punct::RParen) {
                let start = self.pos;
                // An attribute name may be a keyword: `__attribute__((const))`.
                let name = match self.peek().map(|t| t.kind) {
                    Some(TokKind::Ident(s)) => {
                        self.pos += 1;
                        s
                    }
                    Some(TokKind::Kw(_)) => {
                        let idx = self.pos;
                        self.pos += 1;
                        let text = self.kw_text(idx);
                        self.intern(&text)
                    }
                    _ => {
                        let here = self.here();
                        self.error(here, "expected an attribute name");
                        break;
                    }
                };
                let mut args = Vec::new();
                if self.eat_punct(Punct::LParen) {
                    while !self.at_end() && !self.is_punct(0, Punct::RParen) {
                        args.push(self.assignment_expr());
                        if !self.eat_punct(Punct::Comma) {
                            break;
                        }
                    }
                    self.expect_punct(Punct::RParen, "to close an attribute's arguments");
                }
                let span = self.span_from(start);
                out.push(Attr { name, args, span });
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            self.expect_punct(Punct::RParen, "to close `__attribute__`");
            self.expect_punct(Punct::RParen, "to close `__attribute__`");
        }
    }

    /// The spelling of a keyword token, for the rare place a keyword is used as a name.
    fn kw_text(&self, idx: usize) -> String {
        match self.toks.get(idx).map(|t| t.kind) {
            Some(TokKind::Kw(k)) => format!("{k:?}").to_lowercase(),
            _ => String::new(),
        }
    }

    // ---- declarators ----

    fn declarator(&mut self, base: TypeId, abstract_ok: bool) -> (Option<Symbol>, TypeId) {
        if self.depth >= MAX_DEPTH {
            let here = self.here();
            self.error(here, "declarator nested too deeply");
            let ty = self.ast.add_type(TypeKind::Error, here);
            return (None, ty);
        }
        self.depth += 1;
        let r = self.declarator_inner(base, abstract_ok);
        self.depth -= 1;
        r
    }

    fn declarator_inner(&mut self, base: TypeId, abstract_ok: bool) -> (Option<Symbol>, TypeId) {
        let mut ty = base;
        // Pointers bind loosest: in `int *f(void)`, `f` is a function returning `int *`,
        // so the `*` must be applied *after* the suffixes.
        while self.is_punct(0, Punct::Star) {
            let sp = self.here();
            self.pos += 1;
            let mut quals = Quals::default();
            loop {
                if self.eat_kw(Kw::Const) {
                    quals.const_ = true;
                } else if self.eat_kw(Kw::Volatile) {
                    quals.volatile_ = true;
                } else if self.eat_kw(Kw::Restrict) {
                    quals.restrict_ = true;
                } else if self.eat_kw(Kw::Atomic) {
                    quals.atomic = true;
                } else {
                    break;
                }
            }
            let p = self.ast.add_type(TypeKind::Ptr(ty), sp);
            self.ast.ty_mut(p).quals = quals;
            ty = p;
        }
        let mut attrs = Vec::new();
        self.attribute_specifiers(&mut attrs);

        // A grouping paren, as in `int (*fp)(void)`.
        //
        // Resolved by scanning to the matching `)` **without building nodes**, applying
        // the suffixes that follow, and only then parsing the inner declarator with the
        // correct base. The textbook version re-parses the inner declarator twice; doing
        // that here would leave the first pass's abandoned types in the arena, which
        // inflates it and makes contract 20's memory bound measure garbage.
        if self.is_punct(0, Punct::LParen) && self.paren_starts_declarator() {
            let open = self.pos;
            self.pos += 1;
            self.skip_balanced(Punct::LParen, Punct::RParen);
            let ty = self.declarator_suffixes(ty);
            let after = self.pos;
            self.pos = open + 1;
            let (name, ty) = self.declarator(ty, abstract_ok);
            self.pos = after;
            if !attrs.is_empty() {
                self.ast.ty_mut(ty).attrs.extend(attrs);
            }
            return (name, ty);
        }

        let name = match self.peek().map(|t| t.kind) {
            Some(TokKind::Ident(s)) => {
                self.pos += 1;
                Some(s)
            }
            _ => {
                if !abstract_ok
                    && !self.is_punct(0, Punct::LParen)
                    && !self.is_punct(0, Punct::LBracket)
                {
                    let here = self.here();
                    self.error(here, "expected a declarator name");
                }
                None
            }
        };
        let mut ty = self.declarator_suffixes(ty);
        let mut post = Vec::new();
        self.attribute_specifiers(&mut post);
        attrs.extend(post);
        if !attrs.is_empty() {
            ty = self.unshare(ty, base);
            self.ast.ty_mut(ty).attrs.extend(attrs);
        }
        (name, ty)
    }

    /// Give this declarator a type node of its own before writing to it.
    ///
    /// A declarator with no derivations — no `*`, no `[]`, no `()` — has the *specifier's*
    /// `TypeId`, which every other declarator in the same declaration also has. Writing an
    /// attribute there put it on all of them: `int x __attribute__((aligned(64))), y;`
    /// aligned `y` too. `aligned` is one of the three attributes 013 §4 says changes
    /// analysis semantics, so the symptom was every offset 014 computes from the
    /// declaration being wrong, silently and with no diagnostic.
    fn unshare(&mut self, ty: TypeId, base: TypeId) -> TypeId {
        if ty != base {
            return ty;
        }
        let copy = self.ast.ty(ty).clone();
        let fresh = self.ast.add_type(copy.kind, copy.span);
        self.ast.ty_mut(fresh).quals = copy.quals;
        self.ast.ty_mut(fresh).attrs = copy.attrs;
        fresh
    }

    /// After `(`: is this a grouping paren or a parameter list?
    ///
    /// A parameter list starts with `)`, `...`, a type specifier keyword, or a typedef
    /// name. Anything else — `*`, an ordinary identifier, another `(` — groups. Getting
    /// this backwards on `int (x)` would produce a function type for a plain int.
    fn paren_starts_declarator(&self) -> bool {
        match self.peek_at(1).map(|t| t.kind) {
            None => false,
            Some(TokKind::Punct(Punct::RParen)) => false,
            Some(TokKind::Punct(Punct::Ellipsis)) => false,
            Some(TokKind::Kw(Kw::Attribute)) => true,
            Some(TokKind::Kw(_)) => false,
            Some(TokKind::Ident(s)) => !self.oracle.is_typedef_name(s),
            Some(TokKind::Punct(Punct::Star | Punct::LParen | Punct::LBracket)) => true,
            _ => false,
        }
    }

    fn skip_balanced(&mut self, open: Punct, close: Punct) {
        let mut depth = 1i32;
        while let Some(t) = self.peek() {
            if t.kind == TokKind::Punct(open) {
                depth += 1;
            } else if t.kind == TokKind::Punct(close) {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    return;
                }
            }
            self.pos += 1;
        }
    }

    fn declarator_suffixes(&mut self, mut ty: TypeId) -> TypeId {
        loop {
            if self.is_punct(0, Punct::LBracket) {
                // The **token index**, not `here()`. A zero-width span at the *next,
                // unconsumed* token ends the node between two tokens, which is not a
                // boundary of any token in the node — the fabricated range 010 §4
                // forbids, and it is invisible to any test that only asks whether a span
                // resolves.
                let open = self.pos;
                self.pos += 1;
                // `static` and qualifiers inside an array declarator's brackets are
                // legal C99 in a parameter and carry no meaning for us.
                while self.eat_kw(Kw::Static)
                    || self.eat_kw(Kw::Const)
                    || self.eat_kw(Kw::Volatile)
                    || self.eat_kw(Kw::Restrict)
                {}
                let len = if self.is_punct(0, Punct::RBracket) {
                    // `int a[]` — a flexible array member or an unsized parameter.
                    ArrayLen::Unspecified
                } else if self.is_punct(0, Punct::Star) && self.is_punct(1, Punct::RBracket) {
                    self.pos += 1;
                    ArrayLen::Star
                } else {
                    let e = self.assignment_expr();
                    // `int a[0]` is the GNU zero-length array, and 014 must not have to
                    // constant-fold to find out which idiom was written. Only a literal
                    // `0` counts: `a[N]` where `N` expands to 0 is `Fixed`, because the
                    // source said `N`.
                    match &self.ast.expr(e).kind {
                        ExprKind::Number(s) if self.spelling_is_zero(*s) => ArrayLen::Zero,
                        _ => ArrayLen::Fixed(e),
                    }
                };
                self.expect_punct(Punct::RBracket, "to close an array declarator");
                let span = self.span_from(open);
                ty = self.ast.add_type(TypeKind::Array { elem: ty, len }, span);
                continue;
            }
            if self.is_punct(0, Punct::LParen) {
                let open = self.pos;
                self.pos += 1;
                let (params, variadic, kr) = self.parameter_list();
                self.expect_punct(Punct::RParen, "to close a parameter list");
                let span = self.span_from(open);
                ty = self.ast.add_type(
                    TypeKind::Func {
                        ret: ty,
                        params,
                        variadic,
                        kr,
                    },
                    span,
                );
                continue;
            }
            return ty;
        }
    }

    fn spelling_is_zero(&self, s: Symbol) -> bool {
        matches!(self.spellings.get(s.0 as usize), Some(t) if &**t == "0")
    }

    fn parameter_list(&mut self) -> (Vec<DeclId>, bool, bool) {
        let mut params = Vec::new();
        let mut variadic = false;
        // `f(void)` is an empty parameter list, not one parameter of type void.
        if self.is_kw(0, Kw::Void) && self.is_punct(1, Punct::RParen) {
            self.pos += 1;
            return (params, false, false);
        }
        if self.is_punct(0, Punct::RParen) {
            return (params, false, false);
        }
        // An old-style identifier list: every entry is a bare name (contract 4). The
        // types arrive in declarations between the `)` and the `{`.
        if matches!(self.peek().map(|t| t.kind), Some(TokKind::Ident(s)) if !self.oracle.is_typedef_name(s))
            && (self.is_punct(1, Punct::Comma) || self.is_punct(1, Punct::RParen))
        {
            while let Some(TokKind::Ident(name)) = self.peek().map(|t| t.kind) {
                let sp = self.here();
                self.pos += 1;
                self.oracle.declare(name, false);
                let ty = self.ast.add_type(TypeKind::Error, sp);
                let d = self.ast.add_decl(
                    DeclKind::Var {
                        name: Some(name),
                        ty,
                        init: None,
                        storage: Storage::default(),
                    },
                    sp,
                );
                params.push(d);
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            return (params, false, true);
        }
        loop {
            if self.eat_punct(Punct::Ellipsis) {
                variadic = true;
                break;
            }
            if self.at_end() || self.is_punct(0, Punct::RParen) {
                break;
            }
            let start = self.pos;
            let specs = self.declaration_specifiers();
            // Abstract declarators are allowed: `void f(int)`.
            let (name, ty) = self.declarator(specs.ty, true);
            // **Declared as we go.** Contract 2 turns on this: by the time `T x` is read
            // in `void f(int T, T x)`, `T` is a parameter name and no longer a type. A
            // parser that declared the whole list afterwards would parse `T x` as a
            // declaration and report nothing.
            if let Some(n) = name {
                self.oracle.declare(n, false);
            }
            let span = self.span_from(start);
            let d = self.ast.add_decl(
                DeclKind::Var {
                    name,
                    ty,
                    init: None,
                    storage: specs.storage,
                },
                span,
            );
            params.push(d);
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        (params, variadic, false)
    }

    fn starts_type_name(&self) -> bool {
        match self.peek().map(|t| t.kind) {
            Some(TokKind::Kw(k)) => matches!(
                k,
                Kw::Void
                    | Kw::Char
                    | Kw::Short
                    | Kw::Int
                    | Kw::Long
                    | Kw::Float
                    | Kw::Double
                    | Kw::Signed
                    | Kw::Unsigned
                    | Kw::Bool
                    | Kw::Int128
                    | Kw::Complex
                    | Kw::Struct
                    | Kw::Union
                    | Kw::Enum
                    | Kw::Typeof
                    | Kw::Const
                    | Kw::Volatile
                    | Kw::Restrict
                    | Kw::Atomic
            ),
            Some(TokKind::Ident(s)) => self.oracle.is_typedef_name(s),
            _ => false,
        }
    }

    fn type_name(&mut self) -> TypeId {
        let specs = self.declaration_specifiers();
        let (_, ty) = self.declarator(specs.ty, true);
        ty
    }

    // ---- statements ----

    fn compound_statement(&mut self, own_scope: bool) -> StmtId {
        let start = self.pos;
        if own_scope {
            self.oracle.enter_scope();
        }
        self.expect_punct(Punct::LBrace, "to open a block");
        let mut stmts = Vec::new();
        while !self.at_end() && !self.is_punct(0, Punct::RBrace) {
            let before = self.pos;
            stmts.push(self.block_item());
            if self.pos == before {
                let t = self.bump();
                let span = t.map(|t| t.span).unwrap_or_else(|| self.here());
                self.error(span, "expected a statement");
            }
        }
        if !self.eat_punct(Punct::RBrace) {
            // Contract 15: report it, and return what we have. Every complete
            // declaration before the damage is already in the tree.
            let here = self.here();
            self.error(here, "unclosed `{`: expected `}` before end of file");
        }
        if own_scope {
            self.oracle.exit_scope();
        }
        let span = self.span_from(start);
        self.ast.add_stmt(StmtKind::Compound(stmts), span)
    }

    fn block_item(&mut self) -> StmtId {
        if self.is_kw(0, Kw::StaticAssert) {
            let start = self.pos;
            let d = self.static_assert();
            let span = self.span_from(start);
            return match d {
                Some(d) => self.ast.add_stmt(StmtKind::Decl(vec![d]), span),
                None => self.ast.add_stmt(StmtKind::Error, span),
            };
        }
        // A label looks like an expression statement until the `:`.
        if matches!(self.peek().map(|t| t.kind), Some(TokKind::Ident(_)))
            && self.is_punct(1, Punct::Colon)
        {
            return self.statement();
        }
        if self.starts_declaration() {
            return self.local_declaration();
        }
        self.statement()
    }

    fn local_declaration(&mut self) -> StmtId {
        let start = self.pos;
        let specs = self.declaration_specifiers();
        let mut decls = Vec::new();
        if self.eat_punct(Punct::Semi) {
            let span = self.span_from(start);
            let d = self.ast.add_decl(DeclKind::TagDef { ty: specs.ty }, span);
            decls.push(d);
            return self.ast.add_stmt(StmtKind::Decl(decls), span);
        }
        loop {
            let (name, ty) = self.declarator(specs.ty, false);
            let init = if self.eat_punct(Punct::Eq) {
                Some(self.initializer())
            } else {
                None
            };
            let span = self.span_from(start);
            let d = self.finish_declarator(&specs, name, ty, init, span);
            decls.push(d);
            if self.eat_punct(Punct::Comma) {
                continue;
            }
            self.expect_punct(Punct::Semi, "after a declaration");
            break;
        }
        let span = self.span_from(start);
        self.ast.add_stmt(StmtKind::Decl(decls), span)
    }

    fn statement(&mut self) -> StmtId {
        if self.depth >= MAX_DEPTH {
            let here = self.here();
            self.error(here, "statements nested too deeply");
            self.recover_to_boundary();
            return self.ast.add_stmt(StmtKind::Error, here);
        }
        self.depth += 1;
        let r = self.statement_inner();
        self.depth -= 1;
        r
    }

    /// `asm` / `__asm__`, basic and extended — 013 §4, 31 VPP files.
    ///
    /// **Parsed, never interpreted.** The template and every constraint keep their
    /// spelling; lowering turns the whole statement into an opaque effect that clobbers
    /// its outputs and marks the path `Approximated`. Modelling x86 semantics is out of
    /// scope, and treating asm as a no-op would be unsound in the direction that produces
    /// confident wrong answers.
    fn asm_statement(&mut self) -> StmtId {
        let start = self.pos;
        self.pos += 1; // asm
        let mut a = AsmStmt::default();
        loop {
            if self.eat_kw(Kw::Volatile) {
                a.volatile = true;
            } else if self.eat_kw(Kw::Goto) {
                a.goto = true;
            } else if self.eat_kw(Kw::Inline) || self.eat_kw(Kw::Const) {
                // GNU accepts `asm inline` and, historically, `asm const`. Neither says
                // anything we act on.
            } else {
                break;
            }
        }
        if !self.expect_punct(Punct::LParen, "after `asm`") {
            self.recover_to_boundary();
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Error, span);
        }
        // The template is one or more adjacent string literals — phase 6 applies here as
        // it does anywhere else, and each fragment keeps its own span.
        while let Some(TokKind::Str(s)) = self.peek().map(|t| t.kind) {
            let sp = self.peek().map(|t| t.span).unwrap_or_else(|| self.here());
            a.template.push(StrFragment {
                spelling: s,
                span: sp,
            });
            self.pos += 1;
        }
        if a.template.is_empty() {
            let here = self.here();
            self.error(here, "expected an assembly template string");
        }

        // Sections in fixed order, each introduced by `:`. **A missing section is not the
        // same as an empty one only in position**: `asm ("" ::: "memory")` has empty
        // outputs and inputs and a clobber list, so the colons have to be counted rather
        // than matched against content.
        let mut section = 0u8;
        while self.eat_punct(Punct::Colon) {
            match section {
                0 | 1 => {
                    while !self.at_end()
                        && !self.is_punct(0, Punct::RParen)
                        && !self.is_punct(0, Punct::Colon)
                    {
                        let Some(op) = self.asm_operand() else { break };
                        if section == 0 {
                            a.outputs.push(op);
                        } else {
                            a.inputs.push(op);
                        }
                        if !self.eat_punct(Punct::Comma) {
                            break;
                        }
                    }
                }
                2 => {
                    while let Some(TokKind::Str(s)) = self.peek().map(|t| t.kind) {
                        self.pos += 1;
                        a.clobbers.push(s);
                        if !self.eat_punct(Punct::Comma) {
                            break;
                        }
                    }
                }
                _ => {
                    while let Some(TokKind::Ident(s)) = self.peek().map(|t| t.kind) {
                        self.pos += 1;
                        a.labels.push(s);
                        if !self.eat_punct(Punct::Comma) {
                            break;
                        }
                    }
                }
            }
            section = section.saturating_add(1);
        }
        self.expect_punct(Punct::RParen, "to close `asm`");
        self.expect_punct(Punct::Semi, "after `asm`");
        let span = self.span_from(start);
        self.ast.add_stmt(StmtKind::Asm(Box::new(a)), span)
    }

    /// `[name] "constraint" (expr)` — the symbolic name is optional and rare, the
    /// constraint is mandatory.
    fn asm_operand(&mut self) -> Option<AsmOperand> {
        let symbolic_name = if self.eat_punct(Punct::LBracket) {
            let n = match self.peek().map(|t| t.kind) {
                Some(TokKind::Ident(s)) => {
                    self.pos += 1;
                    Some(s)
                }
                _ => {
                    let here = self.here();
                    self.error(here, "expected a symbolic operand name");
                    None
                }
            };
            self.expect_punct(Punct::RBracket, "to close a symbolic operand name");
            n
        } else {
            None
        };
        let Some(TokKind::Str(constraint)) = self.peek().map(|t| t.kind) else {
            let here = self.here();
            self.error(here, "expected an operand constraint string");
            return None;
        };
        self.pos += 1;
        if !self.expect_punct(Punct::LParen, "before an asm operand") {
            return None;
        }
        let expr = self.expression();
        self.expect_punct(Punct::RParen, "after an asm operand");
        Some(AsmOperand {
            symbolic_name,
            constraint,
            expr,
        })
    }

    fn statement_inner(&mut self) -> StmtId {
        let start = self.pos;
        if self.is_punct(0, Punct::LBrace) {
            return self.compound_statement(true);
        }
        if self.is_kw(0, Kw::Asm) {
            return self.asm_statement();
        }
        if self.eat_punct(Punct::Semi) {
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Empty, span);
        }
        if self.eat_kw(Kw::Return) {
            let value = if self.is_punct(0, Punct::Semi) {
                None
            } else {
                Some(self.expression())
            };
            self.expect_punct(Punct::Semi, "after `return`");
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Return(value), span);
        }
        if self.eat_kw(Kw::Break) {
            self.expect_punct(Punct::Semi, "after `break`");
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Break, span);
        }
        if self.eat_kw(Kw::Continue) {
            self.expect_punct(Punct::Semi, "after `continue`");
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Continue, span);
        }
        if self.eat_kw(Kw::Goto) {
            let kind = if self.eat_punct(Punct::Star) {
                // GNU computed goto, which is how VPP dispatches nodes.
                StmtKind::GotoIndirect(self.expression())
            } else {
                match self.peek().map(|t| t.kind) {
                    Some(TokKind::Ident(s)) => {
                        self.pos += 1;
                        StmtKind::Goto(s)
                    }
                    _ => {
                        let here = self.here();
                        self.error(here, "expected a label after `goto`");
                        StmtKind::Error
                    }
                }
            };
            self.expect_punct(Punct::Semi, "after `goto`");
            let span = self.span_from(start);
            return self.ast.add_stmt(kind, span);
        }
        if self.eat_kw(Kw::If) {
            self.expect_punct(Punct::LParen, "after `if`");
            let cond = self.expression();
            self.expect_punct(Punct::RParen, "to close an `if` condition");
            let then = self.statement();
            let els = if self.eat_kw(Kw::Else) {
                Some(self.statement())
            } else {
                None
            };
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::If { cond, then, els }, span);
        }
        if self.eat_kw(Kw::While) {
            self.expect_punct(Punct::LParen, "after `while`");
            let cond = self.expression();
            self.expect_punct(Punct::RParen, "to close a `while` condition");
            let body = self.statement();
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::While { cond, body }, span);
        }
        if self.eat_kw(Kw::Do) {
            let body = self.statement();
            let cond = if self.eat_kw(Kw::While) {
                self.expect_punct(Punct::LParen, "after `while`");
                let c = self.expression();
                self.expect_punct(Punct::RParen, "to close a `do`-`while` condition");
                c
            } else {
                let here = self.here();
                self.error(here, "expected `while` after a `do` body");
                self.ast.add_expr(ExprKind::Error, here)
            };
            self.expect_punct(Punct::Semi, "after `do`-`while`");
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::DoWhile { body, cond }, span);
        }
        if self.eat_kw(Kw::For) {
            self.expect_punct(Punct::LParen, "after `for`");
            // C99 lets the first clause declare, which means it opens a scope that the
            // body is inside — so the scope is entered here and not by the body.
            self.oracle.enter_scope();
            let init = if self.eat_punct(Punct::Semi) {
                None
            } else if self.starts_declaration() {
                let s = self.local_declaration();
                match &self.ast.stmt(s).kind {
                    StmtKind::Decl(d) => Some(ForInit::Decl(d.clone())),
                    _ => None,
                }
            } else {
                let e = self.expression();
                self.expect_punct(Punct::Semi, "after a `for` initializer");
                Some(ForInit::Expr(e))
            };
            let cond = if self.is_punct(0, Punct::Semi) {
                None
            } else {
                Some(self.expression())
            };
            self.expect_punct(Punct::Semi, "after a `for` condition");
            let step = if self.is_punct(0, Punct::RParen) {
                None
            } else {
                Some(self.expression())
            };
            self.expect_punct(Punct::RParen, "to close a `for` header");
            let body = self.statement();
            self.oracle.exit_scope();
            let span = self.span_from(start);
            return self.ast.add_stmt(
                StmtKind::For {
                    init,
                    cond,
                    step,
                    body,
                },
                span,
            );
        }
        if self.eat_kw(Kw::Switch) {
            self.expect_punct(Punct::LParen, "after `switch`");
            let cond = self.expression();
            self.expect_punct(Punct::RParen, "to close a `switch` condition");
            let body = self.statement();
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Switch { cond, body }, span);
        }
        if self.eat_kw(Kw::Case) {
            let lo = self.assignment_expr();
            // GNU case range `case 1 ... 5:` (contract 9).
            let hi = if self.eat_punct(Punct::Ellipsis) {
                Some(self.assignment_expr())
            } else {
                None
            };
            self.expect_punct(Punct::Colon, "after a `case` label");
            let body = self.statement();
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Case { lo, hi, body }, span);
        }
        if self.eat_kw(Kw::Default) {
            self.expect_punct(Punct::Colon, "after `default`");
            let body = self.statement();
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Default { body }, span);
        }
        if let (Some(TokKind::Ident(name)), true) =
            (self.peek().map(|t| t.kind), self.is_punct(1, Punct::Colon))
        {
            self.pos += 2;
            let body = self.statement();
            let span = self.span_from(start);
            return self.ast.add_stmt(StmtKind::Label { name, body }, span);
        }
        let e = self.expression();
        self.expect_punct(Punct::Semi, "after an expression statement");
        let span = self.span_from(start);
        self.ast.add_stmt(StmtKind::Expr(e), span)
    }

    // ---- expressions ----

    fn expression(&mut self) -> ExprId {
        let start = self.pos;
        let mut lhs = self.assignment_expr();
        while self.eat_punct(Punct::Comma) {
            let rhs = self.assignment_expr();
            let span = self.span_from(start);
            lhs = self.ast.add_expr(ExprKind::Comma { lhs, rhs }, span);
        }
        lhs
    }

    fn assignment_expr(&mut self) -> ExprId {
        let start = self.pos;
        let lhs = self.conditional_expr();
        let op = match self.peek().map(|t| t.kind) {
            Some(TokKind::Punct(Punct::Eq)) => Some(None),
            Some(TokKind::Punct(Punct::StarEq)) => Some(Some(BinOp::Mul)),
            Some(TokKind::Punct(Punct::SlashEq)) => Some(Some(BinOp::Div)),
            Some(TokKind::Punct(Punct::PercentEq)) => Some(Some(BinOp::Rem)),
            Some(TokKind::Punct(Punct::PlusEq)) => Some(Some(BinOp::Add)),
            Some(TokKind::Punct(Punct::MinusEq)) => Some(Some(BinOp::Sub)),
            Some(TokKind::Punct(Punct::ShlEq)) => Some(Some(BinOp::Shl)),
            Some(TokKind::Punct(Punct::ShrEq)) => Some(Some(BinOp::Shr)),
            Some(TokKind::Punct(Punct::AmpEq)) => Some(Some(BinOp::BitAnd)),
            Some(TokKind::Punct(Punct::CaretEq)) => Some(Some(BinOp::BitXor)),
            Some(TokKind::Punct(Punct::PipeEq)) => Some(Some(BinOp::BitOr)),
            _ => None,
        };
        let Some(op) = op else { return lhs };
        self.pos += 1;
        // Right-associative.
        let rhs = self.assignment_expr();
        let span = self.span_from(start);
        self.ast.add_expr(ExprKind::Assign { op, lhs, rhs }, span)
    }

    fn conditional_expr(&mut self) -> ExprId {
        let start = self.pos;
        let cond = self.binary_expr(0);
        if !self.eat_punct(Punct::Question) {
            return cond;
        }
        // GNU `a ?: b` omits the middle operand.
        let then = if self.is_punct(0, Punct::Colon) {
            None
        } else {
            Some(self.expression())
        };
        self.expect_punct(Punct::Colon, "in a conditional expression");
        let els = self.conditional_expr();
        let span = self.span_from(start);
        self.ast.add_expr(ExprKind::Cond { cond, then, els }, span)
    }

    /// Precedence climbing. The table is C's, and the order matters in a way that is
    /// invisible in tests that only use one operator: `a & b == c` parses as
    /// `a & (b == c)`, which is a C wart and not ours to fix.
    fn binary_expr(&mut self, min_prec: u8) -> ExprId {
        let start = self.pos;
        let mut lhs = self.unary_expr();
        loop {
            let Some(t) = self.peek() else { return lhs };
            let Some((op, prec)) = binop_of(t.kind) else {
                return lhs;
            };
            if prec < min_prec {
                return lhs;
            }
            self.pos += 1;
            // All C binary operators are left-associative.
            let rhs = self.binary_expr(prec + 1);
            let span = self.span_from(start);
            lhs = self.ast.add_expr(ExprKind::Binary { op, lhs, rhs }, span);
        }
    }

    fn unary_expr(&mut self) -> ExprId {
        let start = self.pos;
        let op = match self.peek().map(|t| t.kind) {
            Some(TokKind::Punct(Punct::Plus)) => Some(UnOp::Plus),
            Some(TokKind::Punct(Punct::Minus)) => Some(UnOp::Minus),
            Some(TokKind::Punct(Punct::Bang)) => Some(UnOp::Not),
            Some(TokKind::Punct(Punct::Tilde)) => Some(UnOp::BitNot),
            Some(TokKind::Punct(Punct::Star)) => Some(UnOp::Deref),
            Some(TokKind::Punct(Punct::Amp)) => Some(UnOp::AddrOf),
            Some(TokKind::Punct(Punct::PlusPlus)) => Some(UnOp::PreInc),
            Some(TokKind::Punct(Punct::MinusMinus)) => Some(UnOp::PreDec),
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            let operand = self.unary_expr();
            let span = self.span_from(start);
            return self.ast.add_expr(ExprKind::Unary { op, operand }, span);
        }
        if self.is_kw(0, Kw::Sizeof) || self.is_kw(0, Kw::Alignof) {
            let is_sizeof = self.is_kw(0, Kw::Sizeof);
            self.pos += 1;
            // `sizeof (T)` versus `sizeof (expr)`: the parenthesized form is a type name
            // only if what follows the `(` starts one. `sizeof (x)` is an expression.
            if self.is_punct(0, Punct::LParen) {
                let save = self.pos;
                self.pos += 1;
                if self.starts_type_name() {
                    let ty = self.type_name();
                    self.expect_punct(Punct::RParen, "to close a type name");
                    let span = self.span_from(start);
                    let kind = if is_sizeof {
                        ExprKind::SizeofType(ty)
                    } else {
                        ExprKind::AlignofType(ty)
                    };
                    return self.ast.add_expr(kind, span);
                }
                self.pos = save;
            }
            let operand = self.unary_expr();
            let span = self.span_from(start);
            let kind = if is_sizeof {
                ExprKind::SizeofExpr(operand)
            } else {
                // `_Alignof` requires a type; an expression operand is a GNU extension
                // we accept and record as a sizeof-shaped node rather than inventing a
                // variant nothing else produces.
                ExprKind::SizeofExpr(operand)
            };
            return self.ast.add_expr(kind, span);
        }
        // A cast: `(T) expr`.
        if self.is_punct(0, Punct::LParen) {
            let save = self.pos;
            self.pos += 1;
            if self.starts_type_name() {
                let ty = self.type_name();
                if self.eat_punct(Punct::RParen) {
                    // `(T){...}` is a compound literal, not a cast of a braced list.
                    if self.is_punct(0, Punct::LBrace) {
                        let operand = self.initializer();
                        let span = self.span_from(start);
                        return self.ast.add_expr(ExprKind::Cast { ty, operand }, span);
                    }
                    let operand = self.unary_expr();
                    let span = self.span_from(start);
                    return self.ast.add_expr(ExprKind::Cast { ty, operand }, span);
                }
            }
            self.pos = save;
        }
        self.postfix_expr()
    }

    fn postfix_expr(&mut self) -> ExprId {
        let start = self.pos;
        let mut e = self.primary_expr();
        loop {
            if self.eat_punct(Punct::LBracket) {
                let index = self.expression();
                self.expect_punct(Punct::RBracket, "to close a subscript");
                let span = self.span_from(start);
                e = self.ast.add_expr(ExprKind::Index { base: e, index }, span);
                continue;
            }
            if self.eat_punct(Punct::LParen) {
                let mut args = Vec::new();
                while !self.at_end() && !self.is_punct(0, Punct::RParen) {
                    args.push(self.assignment_expr());
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::RParen, "to close a call");
                let span = self.span_from(start);
                e = self.ast.add_expr(ExprKind::Call { callee: e, args }, span);
                continue;
            }
            let arrow = if self.is_punct(0, Punct::Dot) {
                false
            } else if self.is_punct(0, Punct::Arrow) {
                true
            } else if self.eat_punct(Punct::PlusPlus) {
                let span = self.span_from(start);
                e = self.ast.add_expr(
                    ExprKind::Postfix {
                        op: PostfixOp::Inc,
                        operand: e,
                    },
                    span,
                );
                continue;
            } else if self.eat_punct(Punct::MinusMinus) {
                let span = self.span_from(start);
                e = self.ast.add_expr(
                    ExprKind::Postfix {
                        op: PostfixOp::Dec,
                        operand: e,
                    },
                    span,
                );
                continue;
            } else {
                return e;
            };
            self.pos += 1;
            let field = match self.peek().map(|t| t.kind) {
                Some(TokKind::Ident(s)) => {
                    self.pos += 1;
                    s
                }
                // A member name may collide with a keyword after macro expansion; taking
                // the token's spelling is better than losing the access.
                Some(TokKind::Kw(_)) => {
                    let idx = self.pos;
                    self.pos += 1;
                    let text = self.kw_text(idx);
                    self.intern(&text)
                }
                _ => {
                    let here = self.here();
                    self.error(here, "expected a member name");
                    let span = self.span_from(start);
                    return self.ast.add_expr(ExprKind::Error, span);
                }
            };
            let span = self.span_from(start);
            e = self.ast.add_expr(
                ExprKind::Member {
                    base: e,
                    field,
                    arrow,
                },
                span,
            );
        }
    }

    fn primary_expr(&mut self) -> ExprId {
        let start = self.pos;
        match self.peek().map(|t| t.kind) {
            Some(TokKind::Ident(s)) => {
                self.pos += 1;
                let span = self.span_from(start);
                self.ast.add_expr(ExprKind::Ident(s), span)
            }
            Some(TokKind::Number(s)) => {
                self.pos += 1;
                let span = self.span_from(start);
                self.ast.add_expr(ExprKind::Number(s), span)
            }
            Some(TokKind::Char(s)) => {
                self.pos += 1;
                let span = self.span_from(start);
                self.ast.add_expr(ExprKind::Char { spelling: s }, span)
            }
            Some(TokKind::Str(_)) => {
                // Phase 6: adjacent string literals concatenate, and **each constituent
                // keeps its own span** (013 §2, contract 18). Joining first and
                // recovering the pieces later is not possible, so they are retained.
                let mut fragments = Vec::new();
                while let Some(TokKind::Str(s)) = self.peek().map(|t| t.kind) {
                    let sp = self.peek().map(|t| t.span).unwrap_or_else(|| self.here());
                    fragments.push(StrFragment {
                        spelling: s,
                        span: sp,
                    });
                    self.pos += 1;
                }
                let span = self.span_from(start);
                self.ast.add_expr(ExprKind::Str { fragments }, span)
            }
            Some(TokKind::Punct(Punct::LParen)) => {
                self.pos += 1;
                // GNU statement expression `({ ... })` (contract 7).
                if self.is_punct(0, Punct::LBrace) {
                    let body = self.compound_statement(true);
                    self.expect_punct(Punct::RParen, "to close a statement expression");
                    let span = self.span_from(start);
                    return self.ast.add_expr(ExprKind::StmtExpr(body), span);
                }
                let e = self.expression();
                self.expect_punct(Punct::RParen, "to close a parenthesized expression");
                e
            }
            Some(TokKind::Punct(Punct::LBrace)) => self.initializer(),
            _ => {
                let here = self.here();
                self.error(here, "expected an expression");
                // **No token is consumed here.** The caller's loop guard turns a
                // non-advancing parse into exactly one skipped token, so recovery is in
                // one place instead of scattered through every production.
                self.ast.add_expr(ExprKind::Error, here)
            }
        }
    }

    /// An initializer: an assignment expression, or a braced list with designators
    /// (contract 11).
    fn initializer(&mut self) -> ExprId {
        if !self.is_punct(0, Punct::LBrace) {
            return self.assignment_expr();
        }
        let start = self.pos;
        self.pos += 1;
        let mut items = Vec::new();
        while !self.at_end() && !self.is_punct(0, Punct::RBrace) {
            let before = self.pos;
            let mut designators = Vec::new();
            loop {
                if self.eat_punct(Punct::Dot) {
                    match self.peek().map(|t| t.kind) {
                        Some(TokKind::Ident(s)) => {
                            self.pos += 1;
                            designators.push(Designator::Field(s));
                        }
                        _ => {
                            let here = self.here();
                            self.error(here, "expected a field name after `.`");
                            break;
                        }
                    }
                    continue;
                }
                if self.eat_punct(Punct::LBracket) {
                    let lo = self.assignment_expr();
                    // GNU range designator `[1 ... 2] =`.
                    if self.eat_punct(Punct::Ellipsis) {
                        let hi = self.assignment_expr();
                        designators.push(Designator::Range(lo, hi));
                    } else {
                        designators.push(Designator::Index(lo));
                    }
                    self.expect_punct(Punct::RBracket, "to close a designator");
                    continue;
                }
                break;
            }
            if !designators.is_empty() {
                self.expect_punct(Punct::Eq, "after a designator");
            }
            let value = self.initializer();
            items.push(InitItem { designators, value });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }
        self.expect_punct(Punct::RBrace, "to close an initializer list");
        let span = self.span_from(start);
        self.ast.add_expr(ExprKind::InitList(items), span)
    }
}

/// The C precedence table, lowest binding first. Returns `None` for a token that is not
/// a binary operator.
fn binop_of(k: TokKind) -> Option<(BinOp, u8)> {
    let TokKind::Punct(p) = k else { return None };
    Some(match p {
        Punct::OrOr => (BinOp::LogOr, 1),
        Punct::AndAnd => (BinOp::LogAnd, 2),
        Punct::Pipe => (BinOp::BitOr, 3),
        Punct::Caret => (BinOp::BitXor, 4),
        Punct::Amp => (BinOp::BitAnd, 5),
        Punct::EqEq => (BinOp::Eq, 6),
        Punct::Ne => (BinOp::Ne, 6),
        Punct::Lt => (BinOp::Lt, 7),
        Punct::Gt => (BinOp::Gt, 7),
        Punct::Le => (BinOp::Le, 7),
        Punct::Ge => (BinOp::Ge, 7),
        Punct::Shl => (BinOp::Shl, 8),
        Punct::Shr => (BinOp::Shr, 8),
        Punct::Plus => (BinOp::Add, 9),
        Punct::Minus => (BinOp::Sub, 9),
        Punct::Star => (BinOp::Mul, 10),
        Punct::Slash => (BinOp::Div, 10),
        Punct::Percent => (BinOp::Rem, 10),
        _ => return None,
    })
}

/// Fold the accumulated arithmetic specifiers into one builtin.
///
/// `unsigned __int128` and `__int128` are **separate** builtins (contract 13), as are the
/// signed and unsigned forms of every integer type: collapsing them would make every
/// wraparound check in 021 ask the wrong question.
fn builtin_of(
    base: Option<Kw>,
    sign: Option<bool>,
    long_count: u32,
    short_seen: bool,
) -> Option<Builtin> {
    let unsigned = sign == Some(false);
    Some(match base {
        Some(Kw::Void) => Builtin::Void,
        Some(Kw::Bool) => Builtin::Bool,
        Some(Kw::Float) => Builtin::Float,
        Some(Kw::Double) if long_count > 0 => Builtin::LongDouble,
        Some(Kw::Double) => Builtin::Double,
        Some(Kw::Int128) => {
            if unsigned {
                Builtin::UInt128
            } else {
                Builtin::Int128
            }
        }
        Some(Kw::Char) => match sign {
            Some(true) => Builtin::SChar,
            Some(false) => Builtin::UChar,
            // Plain `char` is a third type, distinct from both: its signedness is
            // target-defined, and 014 decides. Folding it into `SChar` here would hide
            // that decision in the parser.
            None => Builtin::Char,
        },
        Some(Kw::Int) | None => {
            if base.is_none() && sign.is_none() && long_count == 0 && !short_seen {
                return None;
            }
            if short_seen {
                if unsigned {
                    Builtin::UShort
                } else {
                    Builtin::Short
                }
            } else {
                match (long_count, unsigned) {
                    (0, false) => Builtin::Int,
                    (0, true) => Builtin::UInt,
                    (1, false) => Builtin::Long,
                    (1, true) => Builtin::ULong,
                    (_, false) => Builtin::LongLong,
                    (_, true) => Builtin::ULongLong,
                }
            }
        }
        _ => return None,
    })
}

fn punct_text(p: Punct) -> &'static str {
    match p {
        Punct::LBracket => "[",
        Punct::RBracket => "]",
        Punct::LParen => "(",
        Punct::RParen => ")",
        Punct::LBrace => "{",
        Punct::RBrace => "}",
        Punct::Semi => ";",
        Punct::Comma => ",",
        Punct::Colon => ":",
        Punct::Eq => "=",
        Punct::Star => "*",
        Punct::Ellipsis => "...",
        _ => "token",
    }
}
