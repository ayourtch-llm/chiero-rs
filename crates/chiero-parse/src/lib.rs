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
    ExprKind, FloatFmt, ForInit, GenericAssoc, InitItem, PostfixOp, Quals, StmtId, StmtKind,
    Storage, StrFragment, TagKind, TypeId, TypeKind, UnOp,
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

    /// Declare in the scope *enclosing* the current one, without disturbing the current one.
    ///
    /// A function definition needs this: its name belongs to the file scope while the parser
    /// is inside the prototype scope holding the parameters, and the parameters must survive
    /// into the body. Popping and re-pushing loses them.
    fn declare_enclosing(&mut self, sym: Symbol, is_typedef: bool);
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

    fn declare_enclosing(&mut self, sym: Symbol, is_typedef: bool) {
        // Second from the top, or the file scope when there is nothing above it.
        //
        // **`scopes.len() - 2` and `0` are the same thing for every input C can produce**, and
        // mutation confirms it: a function definition is only ever at file scope, so the scope
        // enclosing its prototype scope *is* the file scope. That mutant is equivalent, not
        // untested. It stops being equivalent the day GNU nested functions are supported —
        // 013's construct table lists them as unsupported, at one VPP file — so this is
        // written relative to the top of the stack rather than to its bottom.
        let at = self.scopes.len().saturating_sub(2);
        if let Some(scope) = self.scopes.get_mut(at) {
            scope.insert(sym, is_typedef);
        }
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

/// Parse a preprocessed TU (013 §§2, 5–7), enforcing `-pedantic-errors` (wave 314).
pub fn parse_tu(tu: &PreprocessedTu, oracle: &mut dyn TypedefOracle) -> ParsedTu {
    parse_tu_with(tu, oracle, chiero_ast::Dialect::pedantic())
}

/// As [`parse_tu`], in a chosen dialect.
///
/// **Only rules measured to differ between `gnu11` and `-pedantic-errors` may consult it.** A
/// syntax error is an error in both; a flag that widened into "report less" would inflate the
/// parser-coverage figure a reader uses to judge the parser.
pub fn parse_tu_with(
    tu: &PreprocessedTu,
    oracle: &mut dyn TypedefOracle,
    dialect: chiero_ast::Dialect,
) -> ParsedTu {
    let mut p = Parser::new(tu, oracle);
    p.dialect = dialect;
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
    VaList,
    F16,
    BF16,
    F32,
    F32x,
    F64,
    F64x,
    F128,
    F128x,
    Ibm128,
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
        // **Only the builtin.** `__gnuc_va_list` is not a gcc builtin type — it is a *typedef*
        // that gcc's own `stdarg.h` writes, `typedef __builtin_va_list __gnuc_va_list;`, so that
        // headers can name the type without claiming the name `va_list`. Treating it as a keyword
        // turned that line into two type specifiers and no declarator: the typedef declared
        // nothing, and every use of `__gnuc_va_list` as an identifier resolved to the keyword
        // instead of to the declaration.
        //
        // Nothing noticed for three hundred waves because the alias made the *type* come out
        // right anyway. Wave 331's "a declaration declares something" rule is what reported it,
        // which is the case for adding a constraint even where a wrong answer is not yet visible.
        "__builtin_va_list" => Kw::VaList,
        "_Float16" => Kw::F16,
        "__bf16" => Kw::BF16,
        "_Float32" => Kw::F32,
        "_Float32x" => Kw::F32x,
        "_Float64" => Kw::F64,
        "_Float64x" => Kw::F64x,
        "_Float128" | "__float128" => Kw::F128,
        "_Float128x" => Kw::F128x,
        "__ibm128" => Kw::Ibm128,
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

/// One `[...]` or `(...)` suffix of a declarator, held until the whole run is read.
///
/// They are applied in reverse, so the type has to be buildable after the fact rather than as
/// each bracket is consumed — see [`Parser::declarator_suffixes`].
enum Suffix {
    Arr(ArrayLen, Span, chiero_ast::Quals),
    Fun(Vec<DeclId>, bool, bool, bool, Span),
}

struct Parser<'a> {
    dialect: chiero_ast::Dialect,
    /// Inside a `_Generic` association's type, where a following `:` separates the association
    /// and is **not** an enum's underlying type. Three constructs spell that token: an
    /// underlying type, a bit-field width, and this. Only a flag distinguishes the third,
    /// because the text either side is identical.
    in_generic_assoc: bool,
    toks: Vec<Tok>,
    pos: usize,
    ast: Ast,
    diags: Vec<ParseDiagnostic>,
    truncated: bool,
    interner: IndexMap<Arc<str>, Symbol>,
    spellings: Vec<Arc<str>>,
    oracle: &'a mut dyn TypedefOracle,
    /// How many parameter lists enclose the declarator being read.
    ///
    /// **C 6.7.6.2p1 and p4 are the only rules in the grammar that depend on this**: `static` and
    /// qualifiers inside `[]`, and `[*]`, are legal syntax everywhere and *mean* something only
    /// in a parameter — `int a[static 3]` promises the caller passes three elements, and an
    /// object declaration has no caller. 013 discarded them as meaningless; they are meaningless
    /// to 014, which is a different thing from being unconstrained.
    ///
    /// A counter rather than a flag, because a parameter's own declarator may contain another
    /// parameter list: `int f(int g(int a[static 3]))` is legal, and leaving on the way out of
    /// the inner one would make the outer look like file scope.
    param_depth: u32,
    /// An asm label read by the declarator, waiting for the `DeclId` it belongs to.
    ///
    /// The declarator returns `(name, type)` and the declaration node does not exist yet,
    /// so the label is parked here for the few lines between. Cleared on read, so a label
    /// can never attach to a later declaration that had none.
    pending_asm_label: Option<Symbol>,
    /// GNU local labels (`__label__ d;`), innermost block last.
    ///
    /// **Renaming, not a scope check.** Lowering keys labels by `Symbol` in one map per
    /// *function*, so two blocks each declaring `d` would collide there however carefully the
    /// parser tracked scopes — and two `hash_foreach_pair` loops in one function is precisely
    /// what the construct exists for. Giving each declaration its own minted symbol makes the
    /// blocks independent without lowering needing to know local labels exist at all.
    ///
    /// The minted spelling contains a `$`, which no C identifier can, so a renamed label can
    /// never collide with a written one.
    local_labels: Vec<Vec<(Symbol, Symbol)>>,
    /// Distinguishes two declarations of the same name; only ever increments.
    local_label_seq: u32,
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
            dialect: chiero_ast::Dialect::pedantic(),
            in_generic_assoc: false,
            toks: Vec::with_capacity(tu.tokens.len()),
            pos: 0,
            ast: Ast::new(),
            diags: Vec::new(),
            truncated: false,
            interner: IndexMap::new(),
            spellings: Vec::new(),
            oracle,
            param_depth: 0,
            pending_asm_label: None,
            local_labels: Vec::new(),
            local_label_seq: 0,
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

    /// The symbol a label name refers to here: a local label's minted one, or the name itself.
    ///
    /// Innermost first, so an inner `__label__ d;` shadows an outer one, and a name no block
    /// declared falls through to function scope unchanged — which is what lets a `goto` inside a
    /// local-label block still reach an ordinary label outside it.
    fn label_symbol(&self, name: Symbol) -> Symbol {
        for frame in self.local_labels.iter().rev() {
            if let Some(&(_, renamed)) = frame.iter().find(|(orig, _)| *orig == name) {
                return renamed;
            }
        }
        name
    }

    /// `__label__ a, b;` — GNU local label declarations, which may only open a block.
    fn local_label_decl(&mut self, start: usize) -> StmtId {
        loop {
            match self.peek().map(|t| t.kind) {
                Some(TokKind::Ident(name)) => {
                    self.pos += 1;
                    let text = self.spellings[name.0 as usize].to_string();
                    self.local_label_seq += 1;
                    // `$` is not an identifier character in C, so this cannot collide with a
                    // label the program wrote.
                    let renamed = self.intern(&format!("{text}${}", self.local_label_seq));
                    if let Some(frame) = self.local_labels.last_mut() {
                        frame.push((name, renamed));
                    }
                }
                _ => {
                    let here = self.here();
                    self.error(here, "expected a label name after `__label__`");
                    break;
                }
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Semi, "after `__label__`");
        let span = self.span_from(start);
        // **The declaration itself lowers to nothing.** It introduces a name, and the name has
        // already been substituted into every `goto` and label in the block by the time anyone
        // reads the tree — so there is no node for lowering to handle and no new `StmtKind`.
        self.ast.add_stmt(StmtKind::Compound(Vec::new()), span)
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

    /// Widen a **positional** span to the token it sits at, so no report covers nothing.
    ///
    /// `here()` returns a zero-width span between two tokens, which is the right answer for "an
    /// insertion point" and the wrong one for a *report*: an editor highlights nothing, and 023
    /// §9 asks for something a reader can act on. gcc points at the token that is actually there
    /// — "expected `;` before `}` token" — so that is what this finds.
    ///
    /// Done **here rather than at the thirty-one call sites**, because `here()` is available,
    /// plausible and exactly what anyone writing a parser diagnostic reaches for: waves 365 and
    /// 366 each did, after the warning against it was already written. Fixing the call sites
    /// would leave the thirty-second to make the same mistake.
    fn visible(&self, span: Span) -> Span {
        if !span.is_empty() {
            return span;
        }
        // The token starting here — the one the parser stopped at.
        if let Some(t) = self
            .toks
            .iter()
            .find(|t| t.span.lo == span.lo && !t.span.is_empty())
        {
            return t.span;
        }
        // At end of input there is no such token, so name the last one instead: "unclosed `{`"
        // then highlights the final token rather than the void after it.
        self.toks
            .iter()
            .rev()
            .find(|t| t.span.hi == span.lo && !t.span.is_empty())
            .map_or(span, |t| t.span)
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        let span = self.visible(span);
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
        // **Called for every old-style definition, not only those with declarations.** The
        // `{`-guard was an optimisation and it hid the case the rule is about: `int f(a) { … }`
        // has *no* declarations, so its parameter is the one that certainly defaults. The loop
        // inside runs zero times there, which is what the guard was avoiding.
        if kr {
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
        // Which parameters a declaration actually typed, so the rest can be named below.
        let mut declared: indexmap::IndexSet<Symbol> = Default::default();
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
                        declared.insert(name);
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
        // **A parameter no declaration typed defaults to `int`** (C89 3.7.1), which this project
        // reports because it calibrates to `-pedantic-errors` — gcc's `-Wimplicit-int` is an
        // error there and a warning under `-std=gnu11`. Saying so is the whole fix: the type is
        // `int` above rather than poison, so 014 no longer calls the parameter incomplete, which
        // was a sentence about a type C specifies.
        for &d in &params {
            if let DeclKind::Var {
                name: Some(n), ty, ..
            } = self.ast.decl(d).kind
                && !declared.contains(&n)
            {
                let text = self.spellings[n.0 as usize].to_string();
                let span = self.ast.ty(ty).span;
                self.error(span, format!("type of `{text}` defaults to `int`"));
            }
        }
    }

    /// Declare a name in the scope *outside* the prototype scope we are currently in.
    ///
    /// **Not by popping and re-pushing.** That discarded every parameter the declarator had
    /// just declared and handed the body a fresh empty scope, so a parameter shadowing a
    /// typedef stopped shadowing it the moment the body began: `void f(u64 word)` with
    /// `typedef i64 word;` in scope read `word >>= 1` as a declaration. It was the only parse
    /// failure in a sweep of all 1552 VPP files.
    fn declare_outer(&mut self, name: Symbol, is_typedef: bool) {
        self.oracle.declare_enclosing(name, is_typedef);
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
        let label = self.pending_asm_label.take();
        let d = self.finish_declarator_inner(specs, name, ty, init, span);
        if let Some(label) = label {
            self.ast.set_asm_label(d, label);
        }
        d
    }

    fn finish_declarator_inner(
        &mut self,
        specs: &Specs,
        name: Option<Symbol>,
        ty: TypeId,
        init: Option<ExprId>,
        span: Span,
    ) -> DeclId {
        match (specs.is_typedef, name) {
            (true, Some(name)) => {
                // **A `typedef` is not initialized** (C 6.7p2): the declaration declares a type
                // and no object, so there is nothing for `= 1` to initialize. `DeclKind::Typedef`
                // has no room for an initializer, so without this the expression was parsed and
                // then silently discarded — the same wrong-answer-not-missing-diagnostic that the
                // function arm below already guards against, in the arm beside it.
                if init.is_some() {
                    self.error(span, "a `typedef` cannot be initialized");
                }
                self.oracle.declare(name, true);
                self.ast.add_decl(
                    DeclKind::Typedef {
                        name,
                        ty,
                        storage: specs.storage,
                    },
                    span,
                )
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
                    // **A function is not initialized** (C 6.9.1p2). `DeclKind::Func` has no room
                    // for an initializer, so without this the `= 1` was parsed and then silently
                    // discarded — a wrong answer rather than a missing diagnostic, since the
                    // program compiled as an ordinary declaration of `x`.
                    if init.is_some() {
                        self.error(span, "a function cannot be initialized");
                    }
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
            // **Adjacent literals are one literal** (C 6.4.5p5, phase 6). This read a single
            // `Str` token, so `_Static_assert(1, "a" "b")` failed on the `)` that was never
            // reached. Three other sites in this file already loop — the linker-name label, and
            // two in the `asm` operands — and this one was written as a `match` beside them.
            //
            // VPP reaches it through `STATIC_ASSERT_SIZEOF`, whose message is two stringized
            // arguments concatenated with three literals, so every use of it was refused;
            // `_Static_assert` is at 140 uses in that tree.
            //
            // **The joined text is interned**, not the first token's symbol. sema prints this
            // on a failed assertion — "static assertion failed: {text}" — and gcc prints the
            // concatenation, `"ab"` for `"a" "b"`. Keeping the first symbol was the first version
            // of this fix and a mutant caught it: the program was accepted and the message
            // silently truncated, which is a wrong answer rather than a missing one.
            //
            // Interned as a quoted spelling, because that is what a `Str` symbol holds
            // everywhere else and what `unquote` expects of it.
            let mut parts: Option<String> = None;
            while let Some(TokKind::Str(s)) = self.peek().map(|t| t.kind) {
                self.pos += 1;
                let text = unquote(self.spelling_of(s)).to_owned();
                parts.get_or_insert_with(String::new).push_str(&text);
            }
            match parts {
                Some(joined) => Some(self.intern(&format!("\"{joined}\""))),
                None => {
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
                    | Kw::VaList
                    | Kw::F16
                    | Kw::BF16
                    | Kw::F32
                    | Kw::F32x
                    | Kw::F64
                    | Kw::F64x
                    | Kw::F128
                    | Kw::F128x
                    | Kw::Ibm128
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
        let mut two_types = false;
        let mut two_signs = false;
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
                    // Recorded as an attribute so 014 has one place to look for alignment,
                    // rather than two spellings of one fact — but under its **own name**, because
                    // C and GNU disagree about where each spelling may appear. `_Alignas` is
                    // refused on a `typedef` and may not weaken a type's alignment;
                    // `__attribute__((aligned))` is legal in both positions and VPP writes it on
                    // typedefs throughout `vppinfra`. One name for both would have to choose which
                    // of those to get wrong.
                    // **The token index, not `here()`.** A zero-width span at the next,
                    // unconsumed token is the fabricated range 010 §4 forbids: it lies between
                    // two tokens and is a boundary of neither, so an editor highlights nothing
                    // and a reader is told an alignment is wrong without being told which. The
                    // array-suffix code two hundred lines below carries the same warning; this
                    // wave is what noticed the warning had not travelled.
                    let astart = self.pos;
                    self.pos += 1;
                    let name = self.intern("_Alignas");
                    let mut args = Vec::new();
                    if self.eat_punct(Punct::LParen) {
                        // **`_Alignas` takes a type name as well as a constant** (C11
                        // 6.7.5p1): `_Alignas(double)` means "align me like a `double`". The
                        // expression parser cannot read a type name, so this used to be a
                        // parse error on entirely ordinary C. It becomes `_Alignof(T)`, which
                        // 014 already folds, so the attribute still carries one constant
                        // expression whichever spelling was written.
                        if self.starts_type_name() {
                            let tstart = self.pos;
                            let ty = self.type_name();
                            let span = self.span_from(tstart);
                            args.push(self.ast.add_expr(ExprKind::AlignofType(ty), span));
                        } else {
                            args.push(self.assignment_expr());
                        }
                        self.expect_punct(Punct::RParen, "to close `_Alignas`");
                    }
                    attrs.push(Attr {
                        name,
                        args,
                        // Covers `_Alignas(…)` entire, which is what the alignment rules point at.
                        span: self.span_from(astart),
                        // Set by `declarator`, which is the only place that knows.
                        from_declarator: false,
                    });
                }
                TokKind::Kw(Kw::Signed) => {
                    // `signed signed` is a repeat; `signed unsigned` is a contradiction. Both are
                    // violations, and only the second changes what the type would have been.
                    two_signs |= sign.is_some();
                    sign = Some(true);
                    self.pos += 1;
                }
                TokKind::Kw(Kw::Unsigned) => {
                    two_signs |= sign.is_some();
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
                TokKind::Kw(
                    k @ (Kw::Void | Kw::Char | Kw::Int | Kw::Bool | Kw::Int128 | Kw::VaList),
                ) => {
                    // **A second data type is a violation, not a replacement** (C 6.7.2p2).
                    // Overwriting `base` is what made `int int` and `void int` name a type at
                    // all: the last specifier won and the earlier one vanished.
                    if base.is_some() {
                        two_types = true;
                    }
                    base = Some(k);
                    self.pos += 1;
                }
                TokKind::Kw(
                    k @ (Kw::Float
                    | Kw::Double
                    | Kw::F16
                    | Kw::BF16
                    | Kw::F32
                    | Kw::F32x
                    | Kw::F64
                    | Kw::F64x
                    | Kw::F128
                    | Kw::F128x
                    | Kw::Ibm128),
                ) => {
                    if base.is_some() {
                        two_types = true;
                    }
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
                self.check_specifier_set(
                    base, sign, long_count, short_seen, two_types, two_signs, span,
                );
                let b = builtin_of(base, sign, long_count, short_seen);
                match b {
                    Some(b) => self.ast.add_type(TypeKind::Builtin(b), span),
                    None => {
                        // No type specifier at all. C89's implicit `int` is not accepted
                        // silently: in preprocessed VPP the overwhelmingly likelier cause
                        // is that a macro did not expand, and guessing `int` would turn
                        // that into a plausible declaration instead of a diagnostic.
                        //
                        // **Name the identifier when there is one**, which is gcc's message and
                        // therefore this project's (013's tiebreak). Nearly every real instance
                        // of this is a type name the oracle does not know, and the reader's next
                        // question is which — answering it turns a reduction into a read. Where
                        // the specifier position holds a punctuator instead there is no name to
                        // give, and the generic wording is the honest one.
                        let msg = match self.peek().map(|t| t.kind) {
                            Some(TokKind::Ident(sym)) => {
                                format!("unknown type name `{}`", self.spelling_of(sym))
                            }
                            _ => "expected a type specifier".to_string(),
                        };
                        self.error(span, msg);
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
        // **An enum may name its underlying type**: `enum E : u8 { … }` (C23; gcc takes it
        // under `gnu11` and refuses it under `-pedantic-errors`). 35 VPP translation units
        // reach `plugins/http/http.h`'s `typedef enum http_version_ : u8 { … }`.
        //
        // **Only for `enum`, and only here.** The same `:` after a declarator inside a record
        // is a bit-field width, and reading the two alike would break every enum-typed
        // bit-field in the tree. The type is parsed and discarded for now: it decides the
        // enumeration's representation, which 014 owns, and inventing that here would be a
        // layout claim this crate cannot make.
        if tag == TagKind::Enum && self.is_punct(0, Punct::Colon) && !self.in_generic_assoc {
            self.pos += 1;
            let start = self.pos;
            let specs = self.declaration_specifiers();
            if self.pos == start {
                let here = self.here();
                self.error(here, "expected a type after `:` in an enum specifier");
            }
            let _ = specs;
        }
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
        // **A member declaration declares a member** (C 6.7.2.1p1). A bare `;` inside a struct
        // declares nothing at all — distinct from an unnamed *bit-field*, `int : 5;`, which does
        // declare a member and merely gives it no name, and from an anonymous struct or union
        // member, which declares its own members into the enclosing one. Both of those go through
        // the specifier path below.
        if self.is_punct(0, Punct::Semi) {
            let here = self.here();
            // **Pedantic-only, measured.** `struct S { ; };`, `struct S { int; };` and a
            // nested tag definition are all accepted by `gcc -std=gnu11` and refused by
            // `-pedantic-errors`; none is refused by both.
            if self.dialect.pedantic {
                self.error(here, "a member declaration must declare a member");
            }
            self.pos += 1;
            return;
        }
        let start = self.pos;
        // **Whether the specifier itself failed**, which the member rule below must not add a
        // second sentence to. Counted rather than inspected, because the specifier reports
        // through several paths and the question here is only "did it already speak" — the same
        // reading `ty_of` uses in sema for an explained array length.
        let before_specs = self.diags.len();
        let specs = self.declaration_specifiers();
        let specs_failed = self.diags.len() > before_specs;
        if self.eat_punct(Punct::Semi) {
            // **Specifiers then `;` is two different declarations** (C 6.7.2.1p1). An *anonymous*
            // struct or union — no tag, its own member list — declares those members into the
            // enclosing record and is legal. Anything else declares nothing at all: `int;`,
            // `const int;`, and `struct T;`, which is a tag declaration that puts no member here.
            //
            // Both used to take this branch, so `struct S { int; };` was accepted silently while
            // the sentence for it, three lines above, fired only on a bare `;`. The rule was
            // written for this case and reached its neighbour instead.
            //
            // The test is on the *specifier*: a `Tag` with no name and a member list. A check for
            // "no declarator" would swallow the anonymous record too, and an unnamed bit-field —
            // `int : 5;` — never reaches here, since it has a declarator-less colon and goes
            // through the loop below.
            // **A struct or a union, not an enum.** An anonymous *enum* member —
            // `struct S { enum { A }; };` — puts its enumerators in the enclosing scope and
            // declares no member, so gcc refuses it exactly as it refuses `int;`. The first
            // version of this predicate omitted the kind and accepted it; a surviving mutant
            // ("members ignored") is what sent me to measure the case.
            // **No `members` clause here, deliberately.** An unnamed tag with no member list is
            // the one input it would exclude, and the specifier reports "expected a tag name or a
            // `{`" for exactly that condition — so `specs_failed` is set and the branch below
            // returns either way. Mutation says so: dropping the clause survives, which makes it
            // an *equivalent* mutant and the clause dead code (wave 391's rule — ask whether a
            // survivor is equivalent before hunting for a row).
            let anonymous_record = matches!(
                self.ast.ty(specs.ty).kind,
                TypeKind::Tag {
                    tag: TagKind::Struct | TagKind::Union,
                    name: None,
                    ..
                }
            );
            // **Poison declares nothing by construction**, so the rule stands down when the
            // specifier has already spoken: `struct S { struct; };` is one mistake and gcc gives
            // it one sentence. Contract 20, and the pattern this crate uses elsewhere — "poison
            // is not a non-integer type", "an explained length is poison rather than a VLA".
            if !anonymous_record && !specs_failed {
                let span = self.span_from(start);
                if self.dialect.pedantic {
                    self.error(span, "a member declaration must declare a member");
                }
                return;
            }
            if specs_failed {
                return;
            }
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
            let bit_width = self
                .eat_punct(Punct::Colon)
                .then(|| self.conditional_expr());
            let mut attrs = Vec::new();
            self.attribute_specifiers(&mut attrs);
            if !attrs.is_empty() {
                self.ast.ty_mut(ty).attrs.extend(attrs);
            }
            let span = self.span_from(start);
            let d = self.finish_member(&specs, name, ty, span);
            if let Some(w) = bit_width {
                self.ast.set_bitfield(d, w);
            }
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
                out.push(Attr {
                    name,
                    args,
                    span,
                    // Set by `declarator`, which is the only place that knows.
                    from_declarator: false,
                });
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
                self.ast
                    .ty_mut(ty)
                    .attrs
                    .extend(attrs.into_iter().map(mark_declarator));
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
        // A GNU **asm label**, not an asm statement: `int f (void) __asm__ ("real");`
        // renames the symbol. glibc's `__REDIRECT` is built on it, so `<string.h>` alone
        // reaches it and every TU includes that.
        if self.is_kw(0, Kw::Asm) && self.is_punct(1, Punct::LParen) {
            self.pos += 2;
            // **One or more** adjacent literals, because phase 6 applies here too and
            // glibc's `__ASMNAME` is literally `__STRING (prefix) cname` — two of them,
            // the first usually empty. Reading only the first gives every redirected
            // symbol the label `""`.
            let mut label = String::new();
            let mut any = false;
            while let Some(TokKind::Str(sym)) = self.peek().map(|t| t.kind) {
                self.pos += 1;
                any = true;
                label.push_str(unquote(self.spelling_of(sym)));
            }
            if any {
                // Stored as **content**, not spelling — unlike `ExprKind::Str`, whose
                // quotes 014 needs. A linker name is a name; every consumer of this
                // (030 matching gcov records, 060 resolving multiarch aliases) wants the
                // symbol, and dropping the delimiters is phase 6's own work, not the
                // escape evaluation 013 §2 defers.
                self.pending_asm_label = Some(self.intern(&label));
            } else {
                let here = self.here();
                self.error(here, "expected a string literal in an asm label");
            }
            self.expect_punct(Punct::RParen, "to close an asm label");
        }
        let mut post = Vec::new();
        self.attribute_specifiers(&mut post);
        attrs.extend(post);
        if !attrs.is_empty() {
            ty = self.unshare(ty, base);
            // **Marked as the declarator's**, because `unshare` may have cloned a node that
            // *defines* a record and 014 must not read these as the definition's. See
            // `Attr::from_declarator`: `typedef struct S {…} T __attribute__((aligned(16)))`
            // aligns `T`, and chiero was aligning `struct S` and rounding its size up too.
            self.ast
                .ty_mut(ty)
                .attrs
                .extend(attrs.into_iter().map(mark_declarator));
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

    /// The `[...]` and `(...)` suffixes of one declarator level.
    ///
    /// **Collected left to right and applied right to left**, which is the whole of C's rule and
    /// was the bug. `int a[2][3]` binds as `(a[2])[3]`, so `a` is an array of **2** rows of 3 —
    /// the *leftmost* bracket is the outermost type. Folding as the suffixes are read builds the
    /// inverse, `int[3][2]`, and almost nothing notices: `sizeof(a)` is 2·3·4 either way, a
    /// square array is its own reverse, and `a[1][0]` reads the same element under both layouts.
    /// `sizeof(a[0])` is the shape that separates them.
    ///
    /// The suffixes are still *parsed* in source order — the length expressions have to be, for
    /// their diagnostics and their spans — and only the type construction runs backwards. Each
    /// node keeps the span of its own bracket, so a reversed fold does not move any of them.
    fn declarator_suffixes(&mut self, ty: TypeId) -> TypeId {
        let mut suffixes: Vec<Suffix> = Vec::new();
        self.collect_declarator_suffixes(&mut suffixes);
        let mut ty = ty;
        for sfx in suffixes.into_iter().rev() {
            ty = match sfx {
                Suffix::Arr(len, span, bracket_quals) => self.ast.add_type(
                    TypeKind::Array {
                        elem: ty,
                        len,
                        bracket_quals,
                    },
                    span,
                ),
                Suffix::Fun(params, variadic, kr, prototyped, span) => {
                    // **From the return type through the parameter list.** The suffix's own span
                    // is `(…)` alone, so a diagnostic pointing at a function *type* named the
                    // parameters — the one part of `int(void)` that is never what is wrong with
                    // it. A `_Generic` association of function type said `(void)`.
                    let whole = self.join(self.ast.ty(ty).span, span);
                    self.ast.add_type(
                        TypeKind::Func {
                            ret: ty,
                            params,
                            variadic,
                            kr,
                            prototyped,
                        },
                        whole,
                    )
                }
            };
        }
        ty
    }

    fn collect_declarator_suffixes(&mut self, out: &mut Vec<Suffix>) {
        loop {
            if self.is_punct(0, Punct::LBracket) {
                // The **token index**, not `here()`. A zero-width span at the *next,
                // unconsumed* token ends the node between two tokens, which is not a
                // boundary of any token in the node — the fabricated range 010 §4
                // forbids, and it is invisible to any test that only asks whether a span
                // resolves.
                let open = self.pos;
                self.pos += 1;
                // `static` and qualifiers inside an array declarator's brackets are legal C99
                // **in a parameter** and carry no meaning for us beyond that. Outside one they
                // are a constraint violation (C 6.7.6.2p1) rather than merely useless, so the
                // spelling is reported here and still discarded.
                let deco = self.here();
                let mut saw_static = false;
                let mut bracket_quals = chiero_ast::Quals::default();
                loop {
                    if self.eat_kw(Kw::Static) {
                        saw_static = true;
                    } else if self.eat_kw(Kw::Const) {
                        bracket_quals.const_ = true;
                    } else if self.eat_kw(Kw::Volatile) {
                        bracket_quals.volatile_ = true;
                    } else if self.eat_kw(Kw::Restrict) {
                        bracket_quals.restrict_ = true;
                    } else {
                        break;
                    }
                }
                // **Which qualifier it was now matters.** C 6.7.6.3p7 moves these onto the
                // *adjusted pointer*, so `int p[const 4]` is `int *const p` and `p = 0` is a
                // write to a read-only object — measured, a gcc error in both modes. The
                // comment that stood here said they "carry no meaning for us beyond that",
                // which was true only for as long as the adjustment itself was missing.
                let saw_qual = bracket_quals != chiero_ast::Quals::default();
                if self.param_depth == 0 && (saw_static || saw_qual) {
                    let what = if saw_static {
                        "`static` in an array size belongs to a parameter"
                    } else {
                        "a qualifier in an array size belongs to a parameter"
                    };
                    self.error(deco, what);
                }
                let len = if self.is_punct(0, Punct::RBracket) {
                    // `int a[]` — a flexible array member or an unsized parameter.
                    ArrayLen::Unspecified
                } else if self.is_punct(0, Punct::Star) && self.is_punct(1, Punct::RBracket) {
                    // **`[*]` names an unspecified VLA size and exists only in a prototype**
                    // (C 6.7.6.2p4). Kept as `Star` either way so 014 sees one shape.
                    if self.param_depth == 0 {
                        let sp = self.here();
                        self.error(sp, "`[*]` belongs to a function prototype");
                    }
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
                out.push(Suffix::Arr(len, span, bracket_quals));
                continue;
            }
            if self.is_punct(0, Punct::LParen) {
                let open = self.pos;
                self.pos += 1;
                let (params, variadic, kr, prototyped) = self.parameter_list();
                self.expect_punct(Punct::RParen, "to close a parameter list");
                let span = self.span_from(open);
                out.push(Suffix::Fun(params, variadic, kr, prototyped, span));
                continue;
            }
            return;
        }
    }

    fn spelling_of(&self, s: Symbol) -> &str {
        self.spellings.get(s.0 as usize).map(|a| &**a).unwrap_or("")
    }

    fn spelling_is_zero(&self, s: Symbol) -> bool {
        matches!(self.spellings.get(s.0 as usize), Some(t) if &**t == "0")
    }

    /// The parameters, whether variadic, whether K&R, and **whether specified at all**.
    ///
    /// The last is not `params.is_empty()`: `f()` and `f(void)` both yield an empty list and mean
    /// opposite things. A K&R identifier list is not a prototype either — it names parameters
    /// without typing them — which is why `static int g(){...}` still accepts `g(1)`.
    /// **The type specifiers name one of C 6.7.2p2's sets** — checked as a multiset, because that
    /// is what C specifies and because the parser has already reduced the *order* away.
    ///
    /// This cannot be "at most one of each": `long long int` and `unsigned long int` are three
    /// specifiers naming one type, and C fixes no order, so `int long unsigned` is the same
    /// declaration written backwards. What the standard actually lists is a set of legal
    /// multisets, and the four questions below are what distinguish them.
    ///
    /// **`builtin_of` cannot report this itself**, which is why the check is here rather than
    /// there: it answers for every combination it is given — that is what made `int int` a type —
    /// and it is called from abstract declarators and type names where a second diagnostic would
    /// be a duplicate.
    #[allow(clippy::too_many_arguments)]
    fn check_specifier_set(
        &mut self,
        base: Option<Kw>,
        sign: Option<bool>,
        long_count: u32,
        short_seen: bool,
        two_types: bool,
        two_signs: bool,
        span: Span,
    ) {
        if two_types {
            self.error(span, "two or more data types in one declaration");
            return;
        }
        if two_signs {
            self.error(span, "both `signed` and `unsigned` in one declaration");
            return;
        }
        // **`long` counts, `short` does not.** C has `long long` and no `short short`, so the two
        // modifiers need different questions rather than one shared counter.
        if long_count > 2 {
            self.error(span, "`long long long` is too long");
            return;
        }
        if short_seen && long_count > 0 {
            self.error(span, "both `long` and `short` in one declaration");
            return;
        }
        // **Which bases take which modifiers.** `long double` is the one floating combination C
        // allows; every other float takes neither a length nor a signedness, and `void`, `_Bool`
        // and the extended floats take nothing at all.
        let modified = long_count > 0 || short_seen || sign.is_some();
        match base {
            Some(Kw::Double) => {
                if short_seen || sign.is_some() || long_count > 1 {
                    self.error(span, "`double` takes only `long`");
                }
            }
            Some(Kw::Char) => {
                if long_count > 0 || short_seen {
                    self.error(span, "`char` takes only a signedness");
                }
            }
            Some(Kw::Void | Kw::Bool | Kw::VaList) if modified => {
                self.error(span, "this type takes no length or signedness specifier");
            }
            Some(Kw::Float) if modified => {
                self.error(span, "`float` takes no length or signedness specifier");
            }
            // The extended floating types, which take nothing either.
            Some(
                Kw::F16
                | Kw::BF16
                | Kw::F32
                | Kw::F32x
                | Kw::F64
                | Kw::F64x
                | Kw::F128
                | Kw::F128x
                | Kw::Ibm128,
            ) if modified => {
                self.error(span, "this type takes no length or signedness specifier");
            }
            _ => {}
        }
    }

    fn parameter_list(&mut self) -> (Vec<DeclId>, bool, bool, bool) {
        self.param_depth += 1;
        let r = self.parameter_list_inner();
        self.param_depth -= 1;
        r
    }

    fn parameter_list_inner(&mut self) -> (Vec<DeclId>, bool, bool, bool) {
        let mut params = Vec::new();
        let mut variadic = false;
        // `f(void)` is an empty parameter list, not one parameter of type void — and it is a
        // *prototype*, which is what distinguishes it from `f()` two lines below. The two produce
        // the same empty list and mean opposite things.
        if self.is_kw(0, Kw::Void) && self.is_punct(1, Punct::RParen) {
            self.pos += 1;
            return (params, false, false, true);
        }
        if self.is_punct(0, Punct::RParen) {
            return (params, false, false, false);
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
                // **An undeclared K&R parameter is an `int`** (C89 3.7.1, and what gcc
                // implements). `TypeKind::Error` was a placeholder for "a later declaration will
                // say", and when none came 014 read the poison as an incomplete type and said
                // so — of a parameter whose type C specifies. The placeholder is replaced in
                // `kr_parameter_declarations` when a declaration does arrive.
                let ty = self.ast.add_type(TypeKind::Builtin(Builtin::Int), sp);
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
            return (params, false, true, false);
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
            // **`typedef` is a storage-class specifier and a parameter takes only `register`**
            // (C 6.7.6.3p2). Reported here because a parameter is built as a `DeclKind::Var`
            // whatever its specifiers said, so `is_typedef` is gone by the time sema looks —
            // the same shape as wave 331's `typedef static` and wave 339's initialized function.
            if specs.is_typedef {
                self.error(span, "`typedef` is not allowed in a parameter");
            }
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
        (params, variadic, false, true)
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
                    | Kw::VaList
                    | Kw::F16
                    | Kw::BF16
                    | Kw::F32
                    | Kw::F32x
                    | Kw::F64
                    | Kw::F64x
                    | Kw::F128
                    | Kw::F128x
                    | Kw::Ibm128
                    | Kw::Struct
                    | Kw::Union
                    | Kw::Enum
                    | Kw::Typeof
                    | Kw::Const
                    | Kw::Volatile
                    | Kw::Restrict
                    | Kw::Atomic
                    // A type name may *start* with an attribute:
                    // `(__attribute__((__vector_size__ (16))) int) {4,1,2,3}` is a
                    // compound literal in gcc's own `xmmintrin.h`, which every VPP TU
                    // reaches through `x86intrin.h`. Without this the `(` is read as a
                    // parenthesized expression and the whole SIMD header derails.
                    | Kw::Attribute
                    | Kw::Alignas
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
        // A frame for this block's local labels, whatever the typedef scope is doing —
        // `__label__` is scoped to the *block*, and `own_scope` is false for a function body
        // whose parameters were already scoped by the declarator.
        self.local_labels.push(Vec::new());
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
        self.local_labels.pop();
        let span = self.span_from(start);
        self.ast.add_stmt(StmtKind::Compound(stmts), span)
    }

    fn block_item(&mut self) -> StmtId {
        if self.is_kw(0, Kw::Label) {
            let start = self.pos;
            self.pos += 1;
            return self.local_label_decl(start);
        }
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
        if let Some(st) = self.attribute_statement() {
            return st;
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
            // **A nested function definition** — 013 §4 puts it in the "no" column, and
            // contract 14 wants exactly one diagnostic with the enclosing function still
            // parsing. The body is skipped as a balanced brace group and the declaration
            // becomes `Error`, so a consumer sees that something was refused rather than a
            // plausible local *declaration* followed by a stray compound statement.
            if matches!(self.ast.ty(ty).kind, TypeKind::Func { .. })
                && self.is_punct(0, Punct::LBrace)
            {
                let span = self.span_from(start);
                self.error(span, "a nested function definition is not supported");
                self.pos += 1;
                self.skip_balanced(Punct::LBrace, Punct::RBrace);
                let d = self.ast.add_decl(DeclKind::Error, span);
                decls.push(d);
                break;
            }
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

    /// **`__attribute__ ((fallthrough));` — an attribute specifier where a statement belongs.**
    ///
    /// 013 §4's position list with no grammar production behind it: C has none, and gcc accepts
    /// it in both `gnu11` and `-pedantic-errors`. VPP marks every deliberate switch fallthrough
    /// this way, and all 3 of the corpus's remaining `expected a type specifier` findings were
    /// this construct reaching the declaration path and failing there.
    ///
    /// **Decided by trying it and backtracking**, the same way `call_argument` decides a type
    /// name, and for the same reason: `__attribute__((unused)) int y = 1;` also starts here and
    /// is a declaration. Only the `;` tells the two apart, and it sits past the attributes. The
    /// diagnostics roll back with the cursor — a speculative parse that leaves its complaints
    /// behind reports errors for a reading the parser itself rejected.
    ///
    /// **Called from `block_item` as well as from `statement_inner`**, because those are two
    /// different arrivals and the corpus uses the first: `starts_declaration` is true of
    /// `__attribute__`, so a fallthrough marker standing among a case's statements is claimed by
    /// the declaration path before `statement_inner` ever sees it. Only a marker written
    /// directly after a label reaches the statement path. Both spellings appear in VPP.
    fn attribute_statement(&mut self) -> Option<StmtId> {
        if !self.is_kw(0, Kw::Attribute) {
            return None;
        }
        let start = self.pos;
        let before = self.diags.len();
        let mut attrs = Vec::new();
        self.attribute_specifiers(&mut attrs);
        if self.eat_punct(Punct::Semi) {
            let span = self.span_from(start);
            return Some(self.ast.add_stmt(StmtKind::Attr(attrs), span));
        }
        self.pos = start;
        self.diags.truncate(before);
        None
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
        if let Some(st) = self.attribute_statement() {
            return st;
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
                        StmtKind::Goto(self.label_symbol(s))
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
            let name = self.label_symbol(name);
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
                // `_Alignof` requires a type; an expression operand is a GNU extension.
                // **It gets its own node.** Recording it as a `SizeofExpr` — which this did,
                // to avoid a variant nothing else produced — made `_Alignof` answer a *size*,
                // and the two agree for every scalar, so nothing noticed until an array.
                ExprKind::AlignofExpr(operand)
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
                        let lit = self.ast.add_expr(ExprKind::Cast { ty, operand }, span);
                        // **A compound literal is a postfix expression**, so `.a`, `[i]`
                        // and `->f` may follow it. Returning here consumed the braces and
                        // left the `.a` for the statement parser, which reported "expected
                        // `;` after `return`" — a parse failure with no hint of the cause.
                        return self.postfix_suffixes(start, lit);
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
        let e = self.primary_expr();
        self.postfix_suffixes(start, e)
    }

    /// The `[]`, `()`, `.`, `->`, `++` and `--` that may follow a postfix expression.
    ///
    /// Split out so a **compound literal** can use it. `(struct S){9, 1}.a` is valid C99 —
    /// 6.5.2.5p4 makes the literal an lvalue, and a postfix operator applies to it like any
    /// other — but the literal was built in `unary_expr` and returned straight to the
    /// caller, so the `.a` was never consumed and the statement failed to parse at all.
    fn postfix_suffixes(&mut self, start: usize, e: ExprId) -> ExprId {
        let mut e = e;
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
                    args.push(self.call_argument());
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

    /// One call argument, which is *usually* an assignment expression.
    ///
    /// gcc's type-taking builtins are the exception, and they are unavoidable: `<string.h>`
    /// reaches `__builtin_types_compatible_p (__typeof__ (x), void)` and VPP's `vec_add1`
    /// is built on it, so the construct appears in every TU that includes either.
    ///
    /// **Decided by trying the type name and backtracking**, not by matching the callee's
    /// name. Keying on names would need a list that is wrong the moment gcc adds a
    /// builtin, and would not cover the `__typeof__` form at all. A type name is only
    /// accepted when it runs exactly to the `,` or `)`, so an ordinary expression that
    /// merely starts with a typedef name — which is not valid C anyway — cannot be
    /// swallowed halfway.
    fn call_argument(&mut self) -> ExprId {
        if self.starts_type_name() {
            let save = self.pos;
            let start = self.pos;
            let before = self.diags.len();
            let ty = self.type_name();
            if self.is_punct(0, Punct::Comma) || self.is_punct(0, Punct::RParen) {
                let span = self.span_from(start);
                return self.ast.add_expr(ExprKind::TypeName(ty), span);
            }
            // Not a type argument after all. Roll the cursor back **and the diagnostics
            // with it** — a speculative parse that leaves its complaints behind reports
            // errors for a reading the parser itself rejected.
            self.pos = save;
            self.diags.truncate(before);
        }
        self.assignment_expr()
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
            // **C11 6.5.1.1.** The keyword has been in the table since the lexer was written
            // and nothing consumed it, so `_Generic` fell out of here as an unexpected token
            // and took the rest of the statement with it.
            //
            // No selection happens here. Which association wins is a question about the
            // controlling expression's *type*, and 013 §2 puts type questions in sema — so the
            // parser keeps every arm and lets one answer be computed once, where the types are.
            Some(TokKind::Kw(Kw::Generic)) => {
                self.pos += 1;
                self.expect_punct(Punct::LParen, "to open a `_Generic` selection");
                let controlling = self.assignment_expr();
                let mut assocs: Vec<GenericAssoc> = Vec::new();
                while self.eat_punct(Punct::Comma) {
                    // `default` is a keyword, so it cannot be mistaken for a type name.
                    let ty = if self.is_kw(0, Kw::Default) {
                        self.pos += 1;
                        None
                    } else {
                        // `_Generic(x, enum E: 1, …)`: the `:` ends the association, so an
                        // `enum E` here must not swallow it as an underlying type.
                        let outer = std::mem::replace(&mut self.in_generic_assoc, true);
                        let t = self.type_name();
                        self.in_generic_assoc = outer;
                        Some(t)
                    };
                    self.expect_punct(Punct::Colon, "after a `_Generic` association's type");
                    let value = self.assignment_expr();
                    assocs.push(GenericAssoc { ty, value });
                }
                self.expect_punct(Punct::RParen, "to close a `_Generic` selection");
                let span = self.span_from(start);
                self.ast.add_expr(
                    ExprKind::Generic {
                        controlling,
                        assocs,
                    },
                    span,
                )
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
        Some(Kw::VaList) => Builtin::VaList,
        Some(Kw::Float) => Builtin::Float,
        Some(Kw::F16) => Builtin::ExtFloat {
            bits: 16,
            fmt: FloatFmt::Binary,
        },
        Some(Kw::BF16) => Builtin::ExtFloat {
            bits: 16,
            fmt: FloatFmt::Brain,
        },
        Some(Kw::F32) => Builtin::ExtFloat {
            bits: 32,
            fmt: FloatFmt::Binary,
        },
        Some(Kw::F32x) => Builtin::ExtFloat {
            bits: 32,
            fmt: FloatFmt::Extended,
        },
        Some(Kw::F64) => Builtin::ExtFloat {
            bits: 64,
            fmt: FloatFmt::Binary,
        },
        Some(Kw::F64x) => Builtin::ExtFloat {
            bits: 64,
            fmt: FloatFmt::Extended,
        },
        Some(Kw::F128) => Builtin::ExtFloat {
            bits: 128,
            fmt: FloatFmt::Binary,
        },
        Some(Kw::F128x) => Builtin::ExtFloat {
            bits: 128,
            fmt: FloatFmt::Extended,
        },
        Some(Kw::Ibm128) => Builtin::ExtFloat {
            bits: 128,
            fmt: FloatFmt::Ibm,
        },
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

/// The content of a string literal's spelling: everything between the first and last
/// `"`. Encoding prefixes are dropped with it. Escapes are **not** processed — that is
/// 014's job, and an asm label containing one is not a thing that occurs.
fn unquote(spelling: &str) -> &str {
    match (spelling.find('"'), spelling.rfind('"')) {
        (Some(a), Some(b)) if b > a => &spelling[a + 1..b],
        _ => spelling,
    }
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

/// An attribute written on a declarator rather than on the type specifier.
///
/// See [`Attr::from_declarator`]: a declarator with no derivations shares the specifier's
/// node, so the parser clones it before writing — and when the specifier *defines* a record,
/// that clone is the definition. Without this mark 014 read
/// `typedef struct S {…} T __attribute__((aligned(16)))` as an attribute of `struct S`.
fn mark_declarator(a: Attr) -> Attr {
    Attr {
        from_declarator: true,
        ..a
    }
}
