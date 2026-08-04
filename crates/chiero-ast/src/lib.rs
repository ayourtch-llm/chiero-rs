//! `chiero-ast` — the syntactic AST. See `docs/specs/013-parser.md` §5.
//!
//! **The AST is syntactic** (013 §5): it records what was written, not what it means. No
//! implicit conversions, no resolved types, no folded constants — all of that is 014.
//! That is not minimalism for its own sake; change-impact analysis diffs *entities*, and
//! it can only do that if the tree is a faithful, printable record of source. A tree that
//! had already normalized `2+2` could not tell an edit that changed the source from one
//! that did not.
//!
//! Arena-allocated and id-indexed, no `Rc`: the tree is walked far more often than it is
//! built, whole-tree analysis holds many TUs at once, and a `Span` is a 12-byte `Copy`
//! value rather than a handle, so nodes stay small.

use chiero_span::{Span, Symbol};

macro_rules! node_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);
    };
}

node_id!(
    /// Index into the expression arena.
    ExprId
);
node_id!(
    /// Index into the statement arena.
    StmtId
);
node_id!(
    /// Index into the declaration arena.
    DeclId
);
node_id!(
    /// Index into the type arena.
    TypeId
);

/// Every node carries a `Span` with its `ExpnCtx` (013 §5). A node the parser
/// *synthesized* — an error node, an implicit piece — uses a zero-width span at the
/// relevant position, never a fabricated range over unrelated source (010 §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Ident(Symbol),
    /// The literal's **spelling**, interned. Not its value: `0x10`, `16` and `16u` are
    /// three different source texts and 014 owns turning any of them into a number.
    /// Folding here would make the AST lie about what was written.
    Number(Symbol),
    /// A character constant, spelling retained for the same reason.
    Char {
        spelling: Symbol,
    },
    /// A string literal, possibly the result of phase-6 concatenation.
    ///
    /// `fragments` is **per-constituent provenance** (013 §2, contract 18) and is
    /// non-empty even for an unconcatenated literal. VPP builds format strings out of
    /// macro-produced pieces, and a diagnostic that cannot say which fragment came from
    /// which macro is not actionable — so the fragments are retained rather than
    /// recomputed from the joined span, which is impossible once they are joined.
    Str {
        fragments: Vec<StrFragment>,
    },
    Unary {
        op: UnOp,
        operand: ExprId,
    },
    /// Post-increment and post-decrement. A separate variant rather than a flag on
    /// `Unary`, because prefix and postfix differ in value, not in spelling.
    Postfix {
        op: PostfixOp,
        operand: ExprId,
    },
    Binary {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    /// `=` and the compound assignments. `op: None` is plain `=`.
    Assign {
        op: Option<BinOp>,
        lhs: ExprId,
        rhs: ExprId,
    },
    Cond {
        cond: ExprId,
        /// `None` is the GNU elvis form `a ?: b`.
        then: Option<ExprId>,
        els: ExprId,
    },
    Comma {
        lhs: ExprId,
        rhs: ExprId,
    },
    Call {
        callee: ExprId,
        args: Vec<ExprId>,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
    Member {
        base: ExprId,
        field: Symbol,
        /// `true` for `->`, `false` for `.`. Kept syntactic: 014 decides whether either
        /// was legal.
        arrow: bool,
    },
    Cast {
        ty: TypeId,
        operand: ExprId,
    },
    /// A **type name in argument position**: `__builtin_types_compatible_p (T1, T2)`,
    /// `__builtin_va_arg (ap, T)`, `__builtin_offsetof (T, member)`.
    ///
    /// C has no such thing, which is why it needs its own node rather than being squeezed
    /// into `SizeofType`: these builtins take types where the grammar says expressions,
    /// and glibc and gcc's own headers use them, so every TU reaches one.
    TypeName(TypeId),
    SizeofExpr(ExprId),
    SizeofType(TypeId),
    AlignofType(TypeId),
    /// `_Alignof(expr)` — a GNU extension, also spelled `__alignof__`.
    ///
    /// **Its own variant, and it has to be.** The parser used to record it as a
    /// `SizeofExpr`, on the reasoning that a sizeof-shaped node beat inventing one nothing
    /// else produced — and that made `_Alignof` *compute a size*. The two agree for every
    /// scalar, which is why it stood: they differ for an array (`_Alignof(a)` is the
    /// element's alignment, `sizeof a` the whole thing) and for any struct whose size is
    /// rounded past its alignment.
    AlignofExpr(ExprId),
    /// GNU statement expression `({ ... })` (013 §4, contract 7). 217 VPP files use it.
    StmtExpr(StmtId),
    /// C11 6.5.1.1's `_Generic(e, T: x, default: y)`.
    ///
    /// **Every association is kept, and the selection is not made here.** Which one wins
    /// depends on the controlling expression's *type*, which the parser does not know — 013
    /// §2 puts type questions in sema. Storing the whole list means the choice is made once,
    /// where the types are, and lowering reads that answer rather than recomputing it.
    ///
    /// `ty` is `None` for the `default` association. C11 allows at most one, and allows it
    /// anywhere in the list rather than only at the end.
    Generic {
        controlling: ExprId,
        assocs: Vec<GenericAssoc>,
    },
    /// A braced initializer list. Syntactic: `{ .a = 1, [3] = 4 }` keeps its designators
    /// and its order, because 014 needs both to place the values.
    InitList(Vec<InitItem>),
    /// The parser could not make sense of this position. **It never returns `Err`**
    /// (013 §1) — a partial tree for a file chiero half-understands is worth more than a
    /// hard failure, because whole-tree analysis has to degrade rather than stop.
    Error,
}

/// One constituent of a (possibly concatenated) string literal, with its own span — and
/// therefore its own `ExpnCtx`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrFragment {
    pub spelling: Symbol,
    pub span: Span,
}

/// One `T: expr` arm of a `_Generic` selection, or `default: expr` when `ty` is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericAssoc {
    pub ty: Option<TypeId>,
    pub value: ExprId,
}

/// One element of a braced initializer, with its designator chain (contract 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitItem {
    /// Empty for a positional element; `.a.b` is two entries, in written order.
    pub designators: Vec<Designator>,
    pub value: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Designator {
    Field(Symbol),
    Index(ExprId),
    /// GNU range designator `[1 ... 2] =`.
    Range(ExprId, ExprId),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnOp {
    Plus,
    Minus,
    Not,
    BitNot,
    Deref,
    AddrOf,
    PreInc,
    PreDec,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PostfixOp {
    Inc,
    Dec,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    LogAnd,
    LogOr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StmtKind {
    /// An expression statement. `A * B;` inside a function is this when `A` is not a
    /// typedef name, and a `Decl` when it is — 013 §3, and the whole reason the parser
    /// needs a symbol table.
    Expr(ExprId),
    /// A declaration appearing where a statement may (C99 block-scope declarations).
    /// One per declared name, matching [`DeclKind`]'s split.
    Decl(Vec<DeclId>),
    Compound(Vec<StmtId>),
    If {
        cond: ExprId,
        then: StmtId,
        els: Option<StmtId>,
    },
    While {
        cond: ExprId,
        body: StmtId,
    },
    DoWhile {
        body: StmtId,
        cond: ExprId,
    },
    For {
        init: Option<ForInit>,
        cond: Option<ExprId>,
        step: Option<ExprId>,
        body: StmtId,
    },
    Switch {
        cond: ExprId,
        body: StmtId,
    },
    /// `hi` is `Some` for a GNU case range `case 1 ... 5:` (contract 9).
    Case {
        lo: ExprId,
        hi: Option<ExprId>,
        body: StmtId,
    },
    Default {
        body: StmtId,
    },
    Label {
        name: Symbol,
        body: StmtId,
    },
    Goto(Symbol),
    /// Computed goto `goto *p;` — GNU, and VPP's node-dispatch shape.
    GotoIndirect(ExprId),
    Break,
    Continue,
    Return(Option<ExprId>),
    Empty,
    /// `__attribute__ ((fallthrough));` — a bare attribute specifier standing where a statement
    /// belongs, which the C grammar has no production for and gcc accepts in both modes.
    ///
    /// **Distinct from `Empty` so the attribute survives.** It executes nothing, so folding it
    /// into `Empty` would lower identically and pass any test written about behaviour — but gcc
    /// refuses a *misplaced* `fallthrough` ("not preceding a case label or default label", and
    /// "invalid use of attribute" outside a switch), and a checker for that has to know which
    /// attribute was written and where. Discarding it here would make that rule unimplementable
    /// without re-parsing.
    Attr(Vec<Attr>),
    /// `asm`/`__asm__`, **parsed but not modeled** (013 §4). Lowering turns it into an
    /// opaque effect that clobbers its outputs and marks the path `Approximated`.
    /// Treating asm as a no-op would be unsound in the direction that produces confident
    /// wrong answers.
    Asm(Box<AsmStmt>),
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForInit {
    Decl(Vec<DeclId>),
    Expr(ExprId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AsmStmt {
    /// The template string's spelling. Never interpreted.
    pub template: Vec<StrFragment>,
    pub volatile: bool,
    pub goto: bool,
    pub outputs: Vec<AsmOperand>,
    pub inputs: Vec<AsmOperand>,
    pub clobbers: Vec<Symbol>,
    pub labels: Vec<Symbol>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsmOperand {
    /// `[name]` in `[name] "=r" (x)`, if written.
    pub symbolic_name: Option<Symbol>,
    pub constraint: Symbol,
    pub expr: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub kind: DeclKind,
    pub span: Span,
}

/// **One `Decl` per declared name.** `int a, b;` is two `Decl`s, not one node with a
/// declarator list.
///
/// The C grammar groups them, so this is a deliberate deviation, recorded here: 031's
/// unit of change is an *entity*, and a grouped node makes "did `b` change?" a question
/// about a subrange of a node rather than about a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclKind {
    Var {
        /// `None` for an abstract declarator — a parameter or a cast with no name.
        name: Option<Symbol>,
        ty: TypeId,
        init: Option<ExprId>,
        storage: Storage,
    },
    Typedef {
        name: Symbol,
        ty: TypeId,
        /// The other storage-class specifiers written beside `typedef`.
        ///
        /// **Kept because `typedef` is itself one** (C 6.7.1p1), so any of these being set is a
        /// violation. The node dropped them until wave 333, which is why the multiple-storage-class
        /// rule could not see `typedef static int T;` — the `static` was gone before sema looked.
        storage: Storage,
    },
    Func {
        name: Symbol,
        /// The function type, so the return type and parameters have one home.
        ty: TypeId,
        /// `None` for a declaration, `Some` for a definition.
        body: Option<StmtId>,
        storage: Storage,
    },
    /// `_Static_assert(cond, "msg")` (contract 12) — 140 VPP files reach it through
    /// `STATIC_ASSERT`.
    StaticAssert {
        cond: ExprId,
        msg: Option<Symbol>,
    },
    /// A struct/union/enum definition with no declarator: `struct S { int a; };`
    TagDef {
        ty: TypeId,
    },
    Error,
}

/// Which C dialect a stage enforces.
///
/// chiero calibrates constraint violations to `-pedantic-errors` (wave 314), which is the
/// strictest reading and the right default for a checker. Real projects do not build that way:
/// VPP compiles under `-std=gnu11`, where an extra `;` in a struct and an enumerator wider than
/// `int` are accepted. `Dialect::gnu()` follows gcc's default so a sweep reports what a
/// project's own compiler would.
///
/// **This is not a verbosity knob.** Only diagnostics measured to differ between
/// `gcc -std=gnu11` and `gcc -std=gnu11 -pedantic-errors` may consult it; a syntax error is an
/// error in both.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Dialect {
    pub pedantic: bool,
}

impl Dialect {
    /// `-pedantic-errors`: the project default since wave 314.
    pub fn pedantic() -> Self {
        Dialect { pedantic: true }
    }

    /// `-std=gnu11` with no `-pedantic`, which is how VPP builds.
    pub fn gnu() -> Self {
        Dialect { pedantic: false }
    }
}

impl Default for Dialect {
    fn default() -> Self {
        Self::pedantic()
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Storage {
    pub extern_: bool,
    pub static_: bool,
    pub thread_local: bool,
    pub auto: bool,
    pub register: bool,
    pub inline: bool,
    pub noreturn: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
    pub quals: Quals,
    /// Attributes attach to the entity the *parser* saw them on (contract 6).
    /// Only `packed`, `aligned` and `may_alias` change analysis semantics and are
    /// interpreted by 014; the rest are recorded and ignored, which is not the same as
    /// dropped — 031 must see an attribute edit as a change.
    pub attrs: Vec<Attr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Builtin(Builtin),
    /// A typedef name, resolved to a name and nothing more. 014 resolves it to a type.
    Named(Symbol),
    Tag {
        tag: TagKind,
        name: Option<Symbol>,
        /// `None` for a reference (`struct S *p`), `Some` for a definition — including
        /// `Some(vec![])` for `struct S {};`, which is not the same thing as a reference.
        members: Option<Vec<DeclId>>,
    },
    Ptr(TypeId),
    Array {
        elem: TypeId,
        len: ArrayLen,
        /// Qualifiers written **inside the brackets** of a parameter: `int p[const 4]`.
        ///
        /// **Not the element's qualifiers.** `const int p[4]` makes the *element* const;
        /// `int p[const 4]` makes the adjusted *pointer* const (C 6.7.6.3p7), which is a
        /// different object and a different diagnostic. They are kept apart because the
        /// adjustment moves this set onto the pointer and leaves `TypeExpr::quals` on the
        /// pointee.
        ///
        /// Empty everywhere but a parameter — outside one these are a constraint violation the
        /// parser already reports.
        bracket_quals: Quals,
    },
    Func {
        ret: TypeId,
        params: Vec<DeclId>,
        variadic: bool,
        /// Old-style K&R parameter list (contract 4): the names appeared in the
        /// declarator and their types in declarations before the body.
        kr: bool,
        /// Whether the parameters were **specified** — `(void)` and `(int, char)` yes, `()` no.
        ///
        /// **`params.is_empty()` cannot answer this**, which is the whole reason the flag exists:
        /// `f()` and `f(void)` both produce an empty list, and C treats them as opposites.
        /// `f(void)` promises there are no parameters, so `f(1)` is an error; `f()` says nothing
        /// at all, so no call to it can be wrong and no later declaration conflicts with it.
        prototyped: bool,
    },
    /// `typeof(x)` / `__typeof__(*p)` (contract 8). 52 VPP files.
    TypeofExpr(ExprId),
    TypeofType(TypeId),
    Error,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ArrayLen {
    /// `int a[4]`
    Fixed(ExprId),
    /// `int a[]` — a flexible array member in a struct, or an unsized parameter
    /// (contract 5). Distinguished from `Zero`, because C and GNU treat them
    /// differently and 1165 VPP files use one or the other.
    Unspecified,
    /// `int a[0]` — the GNU zero-length array. Kept apart from `Fixed(0)` so 014 does not
    /// have to constant-fold to find out which idiom was written.
    Zero,
    /// `int a[*]` in a prototype.
    Star,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TagKind {
    Struct,
    Union,
    Enum,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Quals {
    pub const_: bool,
    pub volatile_: bool,
    pub restrict_: bool,
    pub atomic: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Builtin {
    Void,
    Bool,
    Char,
    SChar,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    /// `__int128` / `unsigned __int128` (contract 13).
    Int128,
    UInt128,
    Float,
    Double,
    LongDouble,
    /// gcc's `__builtin_va_list`. A *type*, not a typedef: nothing declares it, so a
    /// parser without it reports "expected a type specifier" on `<stdarg.h>` and
    /// therefore on every TU in existence. 014 gives it the target's layout.
    VaList,
    /// gcc's extended floating types — `_Float16`, `__bf16`, `_Float128`, `__ibm128` and
    /// the `x` forms.
    ///
    /// They appear only in intrinsic headers, but they appear in **every** TU that
    /// includes one, so a parser that did not know them would fail on all of VPP.
    /// Modelled as width plus format rather than as nine opaque names because the two
    /// facts 014 needs are exactly those: `_Float16` and `__bf16` are both 16 bits and
    /// are different formats, so neither the width nor the name alone identifies one.
    ExtFloat {
        bits: u16,
        fmt: FloatFmt,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FloatFmt {
    /// IEEE 754 binary interchange format of the stated width.
    Binary,
    /// The `x` forms: "at least this wide", mapped to a machine format by the target.
    Extended,
    /// bfloat16 — 16 bits, but `float`'s exponent range and 8 bits of mantissa.
    Brain,
    /// IBM double-double, which is not an IEEE format at all.
    Ibm,
}

/// An attribute as written, with its arguments left as expressions.
///
/// The arguments are *unevaluated*: `aligned(64)` and `aligned(CLIB_CACHE_LINE_BYTES)`
/// are both a name and one argument expression, and which macro produced the second is
/// exactly what 031 wants to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: Symbol,
    pub args: Vec<ExprId>,
    pub span: Span,
}

/// The arena. Ids index these vectors directly; nothing is ever removed, so an id stays
/// valid for the life of the `Ast`.
#[derive(Clone, Debug, Default)]
pub struct Ast {
    exprs: Vec<Expr>,
    stmts: Vec<Stmt>,
    decls: Vec<Decl>,
    types: Vec<TypeExpr>,
    /// Top-level declarations in written order.
    ///
    /// 013 §5's sketch lists only the four arenas, which leaves the TU's own contents
    /// with nowhere to live; a parser that returned a bag of nodes and no roots would be
    /// unusable. Recorded as a deviation rather than smuggled in.
    items: Vec<DeclId>,
    /// Bit-field widths: the `3` in `struct { int a:3; };`
    ///
    /// A side table for the same reason as [`Self::asm_labels`] — most declarations are
    /// not members and no member outside a struct can have one. Kept as an **`ExprId`,
    /// unevaluated**, because the width is routinely written as
    /// `CLIB_CACHE_LINE_BYTES * 8` and folding it here would lose which macro produced
    /// the number, which is exactly what 031 diffs.
    bitfields: indexmap::IndexMap<DeclId, ExprId>,
    /// GNU asm labels: `extern int f (void) __asm__ ("real_name");`
    ///
    /// **A side table, not a field**, because fewer than one declaration in a thousand
    /// has one and every `DeclKind` variant would otherwise carry a `None`. It cannot be
    /// dropped, though: the label *is* the symbol's linker name, so 030 matching gcov
    /// records and 060 resolving VPP's multiarch aliases both need the name the object
    /// file will actually contain rather than the one in the source.
    asm_labels: indexmap::IndexMap<DeclId, Symbol>,
}

impl Ast {
    pub fn new() -> Ast {
        Ast::default()
    }

    pub fn add_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        self.exprs.push(Expr { kind, span });
        ExprId((self.exprs.len() - 1) as u32)
    }

    pub fn add_stmt(&mut self, kind: StmtKind, span: Span) -> StmtId {
        self.stmts.push(Stmt { kind, span });
        StmtId((self.stmts.len() - 1) as u32)
    }

    pub fn add_decl(&mut self, kind: DeclKind, span: Span) -> DeclId {
        self.decls.push(Decl { kind, span });
        DeclId((self.decls.len() - 1) as u32)
    }

    pub fn add_type(&mut self, kind: TypeKind, span: Span) -> TypeId {
        self.types.push(TypeExpr {
            kind,
            span,
            quals: Quals::default(),
            attrs: Vec::new(),
        });
        TypeId((self.types.len() - 1) as u32)
    }

    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id.0 as usize]
    }

    pub fn stmt(&self, id: StmtId) -> &Stmt {
        &self.stmts[id.0 as usize]
    }

    pub fn decl(&self, id: DeclId) -> &Decl {
        &self.decls[id.0 as usize]
    }

    pub fn ty(&self, id: TypeId) -> &TypeExpr {
        &self.types[id.0 as usize]
    }

    pub fn ty_mut(&mut self, id: TypeId) -> &mut TypeExpr {
        &mut self.types[id.0 as usize]
    }

    pub fn decl_mut(&mut self, id: DeclId) -> &mut Decl {
        &mut self.decls[id.0 as usize]
    }

    pub fn items(&self) -> &[DeclId] {
        &self.items
    }

    pub fn push_item(&mut self, id: DeclId) {
        self.items.push(id);
    }

    pub fn set_bitfield(&mut self, id: DeclId, width: ExprId) {
        self.bitfields.insert(id, width);
    }

    /// The declared bit width, if this declaration is a bit-field.
    ///
    /// `Some` even for a **zero**-width field, which declares no member but forces the
    /// next one to a fresh allocation unit (014 contract 4) — so "is a bit-field" and
    /// "has a nonzero width" are different questions and the type has to keep them apart.
    pub fn bitfield(&self, id: DeclId) -> Option<ExprId> {
        self.bitfields.get(&id).copied()
    }

    pub fn set_asm_label(&mut self, id: DeclId, label: Symbol) {
        self.asm_labels.insert(id, label);
    }

    pub fn asm_label(&self, id: DeclId) -> Option<Symbol> {
        self.asm_labels.get(&id).copied()
    }

    pub fn exprs(&self) -> &[Expr] {
        &self.exprs
    }

    pub fn stmts(&self) -> &[Stmt] {
        &self.stmts
    }

    pub fn decls(&self) -> &[Decl] {
        &self.decls
    }

    pub fn types(&self) -> &[TypeExpr] {
        &self.types
    }

    /// Every node's span, in one iterator — what contract 17 quantifies over.
    ///
    /// A caller that had to know the four arenas in order to check a property of "every
    /// node" would silently stop covering a fifth if one were ever added.
    pub fn all_spans(&self) -> impl Iterator<Item = Span> + '_ {
        self.exprs
            .iter()
            .map(|n| n.span)
            .chain(self.stmts.iter().map(|n| n.span))
            .chain(self.decls.iter().map(|n| n.span))
            .chain(self.types.iter().map(|n| n.span))
    }

    /// Total node count across every arena — the denominator for contract 20's memory
    /// bound, and a cheap guard against a test asserting a property of an empty tree.
    pub fn node_count(&self) -> usize {
        self.exprs.len() + self.stmts.len() + self.decls.len() + self.types.len()
    }
}
