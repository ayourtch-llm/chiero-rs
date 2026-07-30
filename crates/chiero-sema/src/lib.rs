//! `chiero-sema` — name resolution, types, layout and constant evaluation.
//! See `docs/specs/014-semantics-and-types.md`.
//!
//! **Layout correctness is the load-bearing part.** Every symbolic memory offset in 021
//! derives from a struct layout computed here, so a one-byte error produces confident,
//! wrong answers throughout the entire system rather than a visible failure. That is why
//! 014 §7 validates layout differentially against the real compiler instead of against
//! hand-written expectations: the expectations are exactly what a layout bug corrupts.

use chiero_ast::{
    Ast, BinOp, DeclId, DeclKind, ExprId, ExprKind, ForInit, StmtId, StmtKind, TypeId, TypeKind,
    UnOp,
};
pub mod strlit;

use chiero_span::{Span, Symbol};
use indexmap::IndexMap;

/// 014 §1. All target-dependent behaviour is **data, not code** — otherwise the aarch64
/// answer is a code path nobody runs and the x86-64 answer is "whatever the code does".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetConfig {
    pub pointer_width: u32,
    /// `true` on x86-64 Linux, `false` on aarch64. Flipping it changes the sign of
    /// comparisons in analysis results, so it belongs on every result.
    pub char_signed: bool,
    pub sizes: IntSizes,
    pub aligns: IntAligns,
    pub endian: Endian,
    pub long_double: LongDoubleKind,
    /// **No semantic effect** — caches are coherent and 021 §7 ignores it. It lives here
    /// because struct layout is the only place the number is knowable, and 041's locality
    /// analysis consumes it: cache-line straddling and false sharing are layout
    /// properties, and VPP tunes them deliberately.
    pub cache_line_bytes: u32,
    /// The largest alignment the ABI gives a vector type, in bytes.
    ///
    /// gcc aligns a vector to its own width **capped by this**: on baseline x86-64 a
    /// 32-byte vector still aligns to 16, and only `-mavx` raises the cap to 32 and
    /// `-mavx512f` to 64. Measured, not assumed — a 32-byte vector's alignment is 16, 32
    /// or 64 depending on flags chiero is not told about directly.
    ///
    /// This is exactly the 1:N mapping [060](060-vpp-integration.md) calls multiarch:
    /// VPP compiles the same source once per ISA variant, so one source file has several
    /// layouts and the `ConfigId` is what distinguishes them.
    pub max_vector_align: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IntSizes {
    pub short_: u64,
    pub int_: u64,
    pub long_: u64,
    pub long_long: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IntAligns {
    pub short_: u64,
    pub int_: u64,
    pub long_: u64,
    pub long_long: u64,
    pub double_: u64,
    pub long_double: u64,
    pub pointer: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LongDoubleKind {
    /// x86's 80-bit extended, stored in 16 bytes on x86-64.
    X87_80,
    /// IEEE binary128, as on aarch64.
    Binary128,
    /// `long double` is `double`.
    Double,
}

impl TargetConfig {
    pub fn x86_64_linux() -> TargetConfig {
        TargetConfig {
            pointer_width: 64,
            char_signed: true,
            sizes: IntSizes {
                short_: 2,
                int_: 4,
                long_: 8,
                long_long: 8,
            },
            aligns: IntAligns {
                short_: 2,
                int_: 4,
                long_: 8,
                long_long: 8,
                double_: 8,
                long_double: 16,
                pointer: 8,
            },
            endian: Endian::Little,
            long_double: LongDoubleKind::X87_80,
            cache_line_bytes: 64,
            // Baseline x86-64 is SSE2; `-mavx` would make this 32 and `-mavx512f` 64.
            max_vector_align: 16,
        }
    }

    /// VPP builds for this too, and `char_signed` flips.
    pub fn aarch64_linux() -> TargetConfig {
        let base = TargetConfig::x86_64_linux();
        TargetConfig {
            char_signed: false,
            long_double: LongDoubleKind::Binary128,
            cache_line_bytes: 128,
            ..base
        }
    }
}

/// 014 §2. Interned; `TyId` equality **is** type identity after canonicalization.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TyId(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Void,
    /// Includes `_Bool` at 1 bit and `__int128` at 128.
    Int {
        signed: bool,
        bits: u32,
    },
    Float(FloatKind),
    Ptr(TyId),
    Array {
        elem: TyId,
        len: ArrayLen,
    },
    Func {
        ret: TyId,
        params: Vec<TyId>,
        variadic: bool,
    },
    Record(RecordId),
    /// `__attribute__((vector_size(n)))`.
    ///
    /// **Alignment is part of the type**, not a property computed from it. gcc aligns a
    /// vector to its own width, but `__attribute__((vector_size(n), __aligned__(1)))` is a
    /// *different type* of the same width — VPP declares both for every shape (`u64x4` and
    /// `u64x4u`) and uses the unaligned one for unaligned loads. Deriving the alignment
    /// would make the two indistinguishable.
    Vector {
        elem: TyId,
        lanes: u32,
        /// The **placement** alignment: the width, or whatever `aligned` asked for.
        ///
        /// gcc has *two* alignments for a vector and they differ. `_Alignof(u64x4)` is
        /// **16** on baseline x86-64 — the psABI caps it — but a `u64x4` member is still
        /// placed at a multiple of **32**, so `struct { char c; u64x4 v; }` puts `v` at
        /// offset 32 while the struct itself aligns to 16. Storing the capped number and
        /// deriving placement from it gets every struct containing a wide vector wrong;
        /// storing the uncapped one and deriving `_Alignof` from it gets every
        /// `_Alignof` wrong. Both are needed, so the type keeps the placement value and
        /// [`align_of_ty`] applies the cap.
        align: u64,
    },
    /// Poison. Propagates so one bad declaration does not produce a thousand
    /// diagnostics (contract 20).
    Error,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FloatKind {
    Binary16,
    BFloat16,
    F32,
    F64,
    /// x87 80-bit extended.
    X87_80,
    Binary128,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ArrayLen {
    Fixed(u64),
    /// `int a[]` — contributes 0 to size but does affect alignment.
    Flexible,
    /// `int a[0]`, the GNU form. 1165 VPP files use it.
    Zero,
    /// A **variable-length array**: the bound is an expression evaluated where the
    /// declaration stands.
    ///
    /// Distinguished from `Flexible` because 015 contract 14 turns on it — a VLA emits
    /// `AllocaDyn` at its declaration point, and calling it flexible gave it extent zero,
    /// so every access to it was out of bounds.
    Vla(chiero_ast::ExprId),
}

/// 014 §3, computed per record and cached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordLayout {
    pub size: u64,
    pub align: u64,
    pub fields: Vec<FieldLayout>,
    pub is_union: bool,
    /// Index into `fields` of a flexible array member, if any.
    pub flexible_member: Option<usize>,
    pub packed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: Option<Symbol>,
    pub ty: TyId,
    /// Byte offset of the field. For a bit-field this is the byte the field's first bit
    /// falls in.
    pub offset: u64,
    /// `Some` for a bit-field: its offset **in bits from the start of the record**, and
    /// its width. Absolute rather than relative to `offset`, because gcc's straddling
    /// rules move the byte offset around and a relative number could not be read without
    /// it.
    pub bits: Option<BitField>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BitField {
    pub bit_offset: u64,
    pub width: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemaDiagnostic {
    pub span: Span,
    pub message: String,
}

/// 014 §5's typed AST: the syntactic tree with **every implicit conversion made
/// explicit**.
///
/// It is an overlay rather than a second copy of every expression kind. The syntactic
/// tree already records what was written and 013 §5 keeps it that way; what 014 adds is
/// the *conversions*, and those are the only new nodes here. A [`TypedNode::Value`]
/// points back at its syntactic expression and holds its operands **already converted**,
/// so a consumer that reads operand types never has to ask what C would have done.
///
/// That is the whole point, and 014 §5 states the reason: lowering must never infer a
/// conversion, because one it gets wrong is an invisible semantic bug rather than a
/// crash. Making them explicit here is what makes CIR unambiguous about bit-widths, which
/// is what the solver needs.
#[derive(Debug, Default)]
pub struct TypedAst {
    nodes: Vec<TypedNode>,
    by_expr: IndexMap<ExprId, TypedId>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypedId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedNode {
    /// A syntactic expression, typed, with its operands already converted.
    Value {
        expr: ExprId,
        ty: TyId,
        operands: Vec<TypedId>,
    },
    /// An inserted conversion — 014 §5's explicit `Cast`, with **its own span**.
    ///
    /// The span is the operand's, not a synthesized one: the conversion happened
    /// *because of* that operand, and a diagnostic about a bad implicit conversion has to
    /// point at the code that caused it rather than at nothing.
    Cast {
        operand: TypedId,
        ty: TyId,
        span: Span,
        why: Conversion,
    },
}

/// Why a conversion was inserted. Recorded rather than derived, because "this is a
/// widening" does not say whether C required it here — and 015 lowers an argument
/// conversion differently from an arithmetic one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Conversion {
    /// C11 §6.3.1.1: anything narrower than `int` becomes `int`.
    IntegerPromotion,
    /// C11 §6.3.1.8: the common type of a binary operation's operands.
    UsualArithmetic,
    /// An array becomes a pointer to its first element.
    ArrayDecay,
    /// A function becomes a pointer to itself.
    FunctionDecay,
    /// A null pointer constant becomes the target pointer type.
    NullPointer,
    /// Conversion to the type being assigned to or initialized.
    Assignment,
    /// Conversion to a parameter's declared type.
    Argument,
    /// Conversion to the function's return type.
    Return,
    /// The operand of a condition, converted to the scalar the branch tests.
    Condition,
}

impl TypedAst {
    pub fn nodes(&self) -> &[TypedNode] {
        &self.nodes
    }

    pub fn node(&self, id: TypedId) -> &TypedNode {
        &self.nodes[id.0 as usize]
    }

    pub fn ty_of(&self, id: TypedId) -> TyId {
        match self.node(id) {
            TypedNode::Value { ty, .. } | TypedNode::Cast { ty, .. } => *ty,
        }
    }

    /// The **outermost** node for a syntactic expression — after every conversion applied
    /// to it. Asking for the type of `c` in `i + c` gives `int`, not `char`, because that
    /// is what the addition actually consumes.
    pub fn top(&self, expr: ExprId) -> Option<TypedId> {
        self.by_expr.get(&expr).copied()
    }

    /// The conversions applied to a syntactic expression, outermost first.
    pub fn conversions_of(&self, expr: ExprId) -> Vec<Conversion> {
        let mut out = Vec::new();
        let mut cur = self.top(expr);
        while let Some(id) = cur {
            match self.node(id) {
                TypedNode::Cast { operand, why, .. } => {
                    out.push(*why);
                    cur = Some(*operand);
                }
                TypedNode::Value { .. } => break,
            }
        }
        out
    }
}

/// 014 §6. Integers plus **address constants**, which matter because `&arr[3]` and
/// `(char*)&s + offsetof(S, f)` are valid static initializers and appear throughout VPP's
/// node registration tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstVal {
    Int(i128),
    /// The address of a named object plus a byte offset.
    Addr {
        global: String,
        off: i64,
    },
}

/// One translation unit, as seen by the cross-TU table.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TuId(pub u32);

/// A single entity across every translation unit (014 §4).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalId(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Linkage {
    /// Visible to every TU: one entity however many TUs mention it.
    External,
    /// `static` at file scope: **one entity per TU**, even when the name repeats.
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalInfo {
    pub name: String,
    pub linkage: Linkage,
    /// `Some` only for internal linkage — an external entity belongs to no single TU.
    pub tu: Option<TuId>,
    /// Whether any TU provided a definition, as opposed to only declaring it.
    pub defined: bool,
    /// Whether every definition seen so far was tentative (`int x;` rather than
    /// `int x = 1;`).
    pub tentative_only: bool,
}

/// The cross-TU symbol table (014 §4): `(name, linkage)` → one `GlobalId`.
///
/// **Keyed by text, not by `Symbol`.** Each TU's interner is its own, so symbol 7 means
/// different things in different TUs — the hazard `Symbol`'s own doc comment in
/// `chiero-span` describes. A cross-TU table keyed on `Symbol` would merge unrelated
/// names and split identical ones, silently.
///
/// This is what lets [031](031-change-impact.md)'s call graph span TUs, and the rule that
/// makes it correct is the one 014 §4 calls out: `static` functions with the same name in
/// different TUs are **distinct entities and must not be merged**. VPP repeats short
/// static helper names across nodes constantly.
#[derive(Debug, Default)]
pub struct GlobalTable {
    globals: Vec<GlobalInfo>,
    by_external: IndexMap<String, GlobalId>,
    by_internal: IndexMap<(TuId, String), GlobalId>,
}

/// The result of analysing one translation unit.
#[derive(Debug, Default)]
pub struct Analysis {
    pub(crate) types: Vec<Ty>,
    pub(crate) interned: IndexMap<Ty, TyId>,
    pub(crate) records: Vec<RecordLayout>,
    pub(crate) by_tag: IndexMap<Symbol, RecordId>,
    pub(crate) decl_types: IndexMap<DeclId, TyId>,
    /// Syntactic type node → the type it resolved to, for consumers that hold an AST
    /// `TypeId` rather than an expression — an explicit cast's target, above all.
    pub(crate) syntactic_types: IndexMap<TypeId, TyId>,
    pub(crate) target: Option<TargetConfig>,
    pub(crate) typed: TypedAst,
    /// Enumerator name → value.
    ///
    /// **Kept on the output, not only on the context.** The values are computed while
    /// typing the enum and were dropped when `analyze` returned, so lowering had no way to
    /// ask what `A` is and every use of an enumeration constant lowered to `undef`.
    /// `const_eval` resolves them by rebuilding a whole context, which is right for one
    /// array bound and O(TU) per reference for a consumer that has one per expression.
    pub(crate) enumerators: IndexMap<Symbol, (i128, TyId)>,
    /// Each enumerator *reference* → the value and type it resolved to.
    ///
    /// **Keyed by the expression, not the name**, because a name is not enough: an
    /// `enum { K = 2 }` inside a function and an `enum { K = 1 }` at file scope are both
    /// legal and both called `K`. A by-name table keeps whichever was recorded last and
    /// hands the file-scope use the inner value. Scope is resolved here, while it is
    /// known; lowering asks what *this* reference is worth and needs no scope of its own.
    pub(crate) enum_refs: IndexMap<ExprId, (i128, TyId)>,
    pub diagnostics: Vec<SemaDiagnostic>,
}

impl Analysis {
    pub fn ty(&self, id: TyId) -> &Ty {
        &self.types[id.0 as usize]
    }

    /// The layout of a record. Panics on an unknown id, which cannot arise from a
    /// `Ty::Record` this analysis produced.
    pub fn layout(&self, id: RecordId) -> &RecordLayout {
        &self.records[id.0 as usize]
    }

    /// The record defined with this tag, if the TU defined one.
    pub fn record_by_tag(&self, tag: Symbol) -> Option<RecordId> {
        self.by_tag.get(&tag).copied()
    }

    /// The value of an enumeration constant, or `None` if the name is not one.
    ///
    /// 014 contract 10 gives an enum an underlying integer type; this is the other half —
    /// what each name in it *means*. C11 6.4.4.3 makes an enumeration constant an `int`
    /// with this value, so a consumer needs no further conversion.
    pub fn enumerator(&self, name: Symbol) -> Option<i128> {
        self.enumerators.get(&name).map(|&(v, _)| v)
    }

    /// The value of one enumerator *reference*, resolved in its own scope.
    pub fn enum_ref(&self, e: ExprId) -> Option<(i128, TyId)> {
        self.enum_refs.get(&e).copied()
    }

    /// The type an enumeration constant has.
    ///
    /// **Not always `int`.** C11 6.4.4.3 says `int`, but only because it also requires
    /// every value to fit in one; gcc widens the whole enumeration to `long` when one does
    /// not, and `sizeof(X)` is then 8. Typing the constant `int` regardless truncated it —
    /// `enum Big { X = 5000000000 }` lowered to `5000000000i32`. Stored in the same entry
    /// as the value so the two cannot disagree about which enum a name came from.
    pub fn enumerator_ty(&self, name: Symbol) -> Option<TyId> {
        self.enumerators.get(&name).map(|&(_, t)| t)
    }

    /// The tag a record was defined with, or `None` if it was anonymous.
    ///
    /// The reverse direction matters for 014 §7's generator: an anonymous record has a
    /// layout but **no spelling**, so it cannot appear in a generated `_Static_assert`,
    /// and a gate that silently skipped those would report a comfortable zero over a
    /// fraction of the records.
    pub fn tag_of(&self, id: RecordId) -> Option<Symbol> {
        self.by_tag.iter().find(|&(_, &r)| r == id).map(|(&s, _)| s)
    }

    pub fn ty_of_decl(&self, d: DeclId) -> Option<TyId> {
        self.decl_types.get(&d).copied()
    }

    /// The id of the interned `Ty::Error`, if this analysis produced one.
    ///
    /// A consumer that needs a poison type must ask for it rather than assuming an id:
    /// `TyId(0)` is whichever type was interned first, which is an arbitrary type wearing
    /// the name of an error.
    pub fn interned_error(&self) -> Option<TyId> {
        self.interned.get(&Ty::Error).copied()
    }

    /// Any valid id, for a caller that must return *something*. Prefer
    /// [`Self::interned_error`].
    pub fn any_ty(&self) -> TyId {
        TyId(0)
    }

    /// The semantic type of a **syntactic** type node, if one was resolved for it.
    ///
    /// Needed by an explicit cast: `(int)x` names its target type with an AST node rather
    /// than with an expression, so nothing in the typed AST carries it.
    pub fn ty_of_syntactic(&self, ty: chiero_ast::TypeId) -> Option<TyId> {
        self.syntactic_types.get(&ty).copied()
    }

    /// The target this analysis was built against. Every width and layout in it is
    /// relative to this, so a consumer computing its own must use the same one.
    pub fn target_config(&self) -> Option<&TargetConfig> {
        self.target.as_ref()
    }

    /// 014 §5's typed AST.
    pub fn typed(&self) -> &TypedAst {
        &self.typed
    }

    pub fn records(&self) -> &[RecordLayout] {
        &self.records
    }

    /// Size in bytes, or `None` for a type that has none — a function, or `Error`.
    pub fn size_of(&self, id: TyId) -> Option<u64> {
        let t = self.target.as_ref()?;
        size_of_ty(self, t, id)
    }

    pub fn align_of(&self, id: TyId) -> Option<u64> {
        let t = self.target.as_ref()?;
        align_of_ty(self, t, id)
    }
}

/// Everything `chiero-sema` needs to read a `Symbol`'s text.
///
/// The AST's symbols index an interner `chiero-parse` owns, and `chiero-sema` may not
/// depend on `chiero-parse` to reach it — so the caller supplies the lookup. Passing the
/// table rather than the crate also means a future front end can feed sema without
/// pretending to be the C parser.
pub trait SymbolText {
    fn text(&self, sym: Symbol) -> Option<&str>;
}

impl GlobalTable {
    pub fn new() -> GlobalTable {
        GlobalTable::default()
    }

    /// Fold one analysed TU's file-scope declarations into the table.
    pub fn add_tu(
        &mut self,
        tu: TuId,
        ast: &Ast,
        analysis: &Analysis,
        names: &dyn SymbolText,
    ) -> Vec<SemaDiagnostic> {
        let _ = analysis;
        let mut diags = Vec::new();
        for &item in ast.items() {
            let (name, storage, is_definition, tentative) = match &ast.decl(item).kind {
                DeclKind::Var {
                    name: Some(n),
                    storage,
                    init,
                    ..
                } => (
                    *n,
                    *storage,
                    // `extern int x;` declares; `int x;` and `int x = 1;` define. The
                    // second is tentative — C11 §6.9.2 — and repeating it is legal.
                    !storage.extern_,
                    init.is_none(),
                ),
                DeclKind::Func {
                    name,
                    body,
                    storage,
                    ..
                } => (*name, *storage, body.is_some(), false),
                _ => continue,
            };
            let Some(text) = names.text(name) else {
                continue;
            };
            let text = text.to_owned();
            let linkage = if storage.static_ {
                Linkage::Internal
            } else {
                Linkage::External
            };

            let id = match linkage {
                Linkage::Internal => {
                    // **One entity per TU**, even when the name repeats. This is the rule
                    // 014 §4 calls a real VPP hazard: short static helper names recur
                    // across nodes, and merging them would give 031 call-graph edges
                    // between functions that cannot see each other.
                    let key = (tu, text.clone());
                    match self.by_internal.get(&key) {
                        Some(&id) => id,
                        None => {
                            let id = self.fresh(text.clone(), linkage, Some(tu));
                            self.by_internal.insert(key, id);
                            id
                        }
                    }
                }
                Linkage::External => match self.by_external.get(&text) {
                    Some(&id) => id,
                    None => {
                        let id = self.fresh(text.clone(), linkage, None);
                        self.by_external.insert(text.clone(), id);
                        id
                    }
                },
            };

            let info = &mut self.globals[id.0 as usize];
            if is_definition {
                // Two *initialized* definitions of one external name are the error;
                // tentative ones may repeat freely (contract 14, across TUs this time).
                if info.defined && !info.tentative_only && !tentative {
                    diags.push(SemaDiagnostic {
                        span: ast.decl(item).span,
                        message: format!("`{text}` is defined more than once"),
                    });
                }
                info.defined = true;
                info.tentative_only = info.tentative_only && tentative;
            }
        }
        diags
    }

    fn fresh(&mut self, name: String, linkage: Linkage, tu: Option<TuId>) -> GlobalId {
        let id = GlobalId(self.globals.len() as u32);
        self.globals.push(GlobalInfo {
            name,
            linkage,
            tu,
            defined: false,
            tentative_only: true,
        });
        id
    }

    /// The entity a name refers to **from inside `tu`** — which is not the same question
    /// as "the entity with this name", because a `static` shadows an external one.
    pub fn resolve(&self, tu: TuId, name: &str) -> Option<GlobalId> {
        self.by_internal
            .get(&(tu, name.to_owned()))
            .or_else(|| self.by_external.get(name))
            .copied()
    }

    pub fn info(&self, id: GlobalId) -> &GlobalInfo {
        &self.globals[id.0 as usize]
    }

    pub fn globals(&self) -> &[GlobalInfo] {
        &self.globals
    }

    pub fn len(&self) -> usize {
        self.globals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.globals.is_empty()
    }
}

/// Analyse one TU's AST against a target (014 §§2–6).
pub fn analyze(ast: &Ast, target: &TargetConfig, names: &dyn SymbolText) -> Analysis {
    let mut cx = Cx {
        ast,
        target: target.clone(),
        names,
        out: Analysis {
            target: Some(target.clone()),
            ..Analysis::default()
        },
        typedefs: IndexMap::new(),
        tags: IndexMap::new(),
        enums: IndexMap::new(),
        enumerators: IndexMap::new(),
        in_progress: Vec::new(),
        current_ret: None,
        values: IndexMap::new(),
        unknown_names: Default::default(),
        defined_with_init: Default::default(),
    };
    for &item in ast.items() {
        cx.item(item);
    }
    cx.out
}

/// Evaluate an integer constant expression (014 §6).
///
/// Public because array bounds and bit-field widths need it *during* layout, and because
/// 013 deliberately leaves every literal unfolded — so this is the only place a written
/// `0x10` becomes the number 16.
pub fn const_eval(
    ast: &Ast,
    expr: ExprId,
    names: &dyn SymbolText,
    target: &TargetConfig,
    out: &mut Vec<SemaDiagnostic>,
) -> Option<ConstVal> {
    // A throwaway context, so `sizeof(int)` resolves standalone. `sizeof(struct S)` needs
    // the TU's tag table and therefore needs `analyze`; that is a real limit and is why
    // this takes a target rather than pretending sizes are universal.
    let mut cx = Cx {
        ast,
        target: target.clone(),
        names,
        out: Analysis::default(),
        typedefs: IndexMap::new(),
        tags: IndexMap::new(),
        enums: IndexMap::new(),
        enumerators: IndexMap::new(),
        in_progress: Vec::new(),
        current_ret: None,
        values: IndexMap::new(),
        unknown_names: Default::default(),
        defined_with_init: Default::default(),
    };
    // **The declarations are processed first.** An address constant is *about* a declared
    // object — `&arr[3]` needs `arr`'s element size to scale the offset — and `sizeof`
    // needs the tag table. Their diagnostics are then discarded, because the caller asked
    // about one expression and complaints about the surrounding file are not an answer.
    for &item in ast.items() {
        cx.item(item);
    }
    cx.out.diagnostics.clear();

    let v = cx.eval(expr);
    out.append(&mut cx.out.diagnostics);
    match v {
        Some(v) => Some(ConstVal::Int(v.v)),
        // Not an integer constant: it may still be an **address** constant, which 014 §6
        // requires because `&arr[3]` and `(char*)&s + offsetof(S, f)` are valid static
        // initializers and fill VPP's node registration tables.
        None => cx
            .addr_of(expr)
            .map(|(global, off, _)| ConstVal::Addr { global, off }),
    }
}

/// A constant's value **and its type**, because contract 19 is about the type.
///
/// `2147483647 + 1` overflows and `0x100000000` does not, and the only thing that
/// distinguishes them is that the first is `int` arithmetic and the second is a `long`
/// literal. Carrying the value alone makes the two indistinguishable, which is how the
/// first implementation of this silently accepted both.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct IntVal {
    v: i128,
    bits: u32,
    signed: bool,
}

// ---------------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------------

struct Cx<'a> {
    ast: &'a Ast,
    target: TargetConfig,
    names: &'a dyn SymbolText,
    out: Analysis,
    typedefs: IndexMap<Symbol, TyId>,
    tags: IndexMap<Symbol, RecordId>,
    /// Enum tag → its underlying integer type (014 contract 10).
    enums: IndexMap<Symbol, TyId>,
    /// Enumerator name → value, so `enum { A = 1, B = A + 1 }` and any later use of `A`
    /// in a constant expression resolve. 013 left every name unresolved on purpose.
    enumerators: IndexMap<Symbol, i128>,
    /// Records currently being laid out, so a struct containing itself by value is a
    /// diagnostic rather than a stack overflow.
    in_progress: Vec<Symbol>,
    /// The return type of the function whose body is being typed, if any.
    ///
    /// Saved and restored rather than set once: a nested `DeclKind::Func` is refused
    /// elsewhere, but a `None` left behind by a declaration would make the *next*
    /// function's returns unconverted, which is the failure mode nothing would notice.
    current_ret: Option<TyId>,
    /// Ordinary identifiers in scope → their type. C's five namespaces are separate
    /// (014 §4), and this is the one expressions read.
    values: IndexMap<Symbol, TyId>,
    /// Names already reported as undeclared, so the complaint is per name and not per
    /// use — contract 20.
    unknown_names: indexmap::IndexSet<Symbol>,
    /// File-scope names that already have an *initialized* definition — contract 14.
    defined_with_init: indexmap::IndexSet<Symbol>,
}

impl Cx<'_> {
    fn intern(&mut self, ty: Ty) -> TyId {
        if let Some(&id) = self.out.interned.get(&ty) {
            return id;
        }
        let id = TyId(self.out.types.len() as u32);
        self.out.types.push(ty.clone());
        self.out.interned.insert(ty, id);
        id
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.out.diagnostics.push(SemaDiagnostic {
            span,
            message: message.into(),
        });
    }

    fn text(&self, sym: Symbol) -> Option<&str> {
        self.names.text(sym)
    }

    fn item(&mut self, id: DeclId) {
        match self.ast.decl(id).kind.clone() {
            DeclKind::Var { name, ty, init, .. } => {
                let t = self.ty_of(ty);
                self.out.decl_types.insert(id, t);
                if let Some(n) = name {
                    self.values.insert(n, t);
                    // Contract 14. `int x; int x;` is two **tentative** definitions and is
                    // legal C11 §6.9.2 — it is how headers have always worked. Only a
                    // second *initialized* definition is an error, so the thing tracked
                    // is "has an initializer", not "has been seen".
                    if init.is_some() && !self.ast.decl(id).span.ctx.is_root() {
                        // Macro-produced definitions are not compared: a header expanded
                        // twice is the preprocessor's business, not a redefinition.
                    } else if init.is_some() && !self.defined_with_init.insert(n) {
                        let text = self.text(n).unwrap_or("?").to_owned();
                        let span = self.ast.decl(id).span;
                        self.error(span, format!("`{text}` is defined more than once"));
                    }
                }
                self.check_complete(id, t);
                if let Some(init) = init {
                    let node = self.type_expr(init);
                    // 014 §5: the initializer arrives **as the declared type**, so
                    // lowering never has to work out what the assignment did.
                    self.coerce(node, t, Conversion::Assignment, init);
                }
            }
            DeclKind::Typedef { name, ty } => {
                let t = self.ty_of(ty);
                self.typedefs.insert(name, t);
                self.out.decl_types.insert(id, t);
            }
            DeclKind::Func { name, ty, body, .. } => {
                let t = self.ty_of(ty);
                self.out.decl_types.insert(id, t);
                self.values.insert(name, t);
                // **A declaration's parameters are typed too.** Only definitions used to
                // get this, so `void f(void *p, size_t n);` left both parameters with no
                // recorded type — and a consumer asking `ty_of_decl` got `None` and
                // substituted something. The harness intrinsics in `chiero.h` are exactly
                // this shape: declarations with no body, whose parameter types are the
                // whole interface.
                let params = match &self.ast.ty(ty).kind {
                    chiero_ast::TypeKind::Func { params, .. } => params.clone(),
                    _ => Vec::new(),
                };
                for p in &params {
                    if let DeclKind::Var { ty: pty, .. } = self.ast.decl(*p).kind.clone() {
                        let t = self.ty_of(pty);
                        self.out.decl_types.insert(*p, t);
                    }
                }
                if let Some(body) = body {
                    // **Parameters are in scope in the body.** They are declared on the
                    // function's *type*, not as items, so nothing else brings them in —
                    // and until a fixture had a body that mentioned one, every parameter
                    // use in the project silently typed as `Ty::Error`. Found by 015's
                    // first fixture, three waves after the typing was written.
                    let params = match &self.ast.ty(ty).kind {
                        chiero_ast::TypeKind::Func { params, .. } => params.clone(),
                        _ => Vec::new(),
                    };
                    let saved = self.values.clone();
                    // An `enum` declared inside the body is local to it, exactly as a
                    // parameter is, and was leaking into the rest of the TU.
                    let saved_enums = self.enumerators.clone();
                    for p in params {
                        if let DeclKind::Var {
                            name: Some(pn),
                            ty: pty,
                            ..
                        } = self.ast.decl(p).kind.clone()
                        {
                            let t = self.ty_of(pty);
                            self.values.insert(pn, t);
                            self.out.decl_types.insert(p, t);
                        }
                    }
                    // The return type the body's `return` statements convert to.
                    let saved_ret = self.current_ret;
                    self.current_ret = match self.out.types[t.0 as usize].clone() {
                        Ty::Func { ret, .. } => Some(ret),
                        _ => None,
                    };
                    self.type_stmt(body);
                    self.current_ret = saved_ret;
                    // A parameter does not outlive its function; restoring rather than
                    // removing also undoes any shadowing the body introduced.
                    self.values = saved;
                    self.enumerators = saved_enums;
                }
            }
            DeclKind::TagDef { ty } => {
                let t = self.ty_of(ty);
                self.out.decl_types.insert(id, t);
            }
            DeclKind::StaticAssert { cond, msg } => self.static_assert(id, cond, msg),
            DeclKind::Error => {}
        }
    }

    /// 014 contract 13: a false `_Static_assert` is **exactly one** diagnostic, carrying
    /// the message the source gave it, and analysis continues.
    fn static_assert(&mut self, id: DeclId, cond: ExprId, msg: Option<Symbol>) {
        let span = self.ast.decl(id).span;
        let before = self.out.diagnostics.len();
        let v = self.eval(cond);
        let diags: Vec<SemaDiagnostic> = self.out.diagnostics.drain(before..).collect();
        // The condition's own diagnostics are **dropped when it evaluated anyway**: an
        // overflow inside an assertion that still comes out true is not a second
        // complaint about the assertion, and contract 13 asks for exactly one.
        match v.map(|v| v.v) {
            Some(0) => {
                let text = msg
                    .and_then(|m| self.text(m))
                    .map(str::to_owned)
                    .unwrap_or_default();
                self.error(span, format!("static assertion failed: {text}"));
            }
            Some(_) => {}
            None => {
                self.out.diagnostics.extend(diags);
                self.error(
                    span,
                    "static assertion is not an integer constant expression",
                );
            }
        }
    }

    /// Resolve a syntactic `TypeId` from 013 into a semantic `TyId`.
    fn ty_of(&mut self, ty: TypeId) -> TyId {
        let base = self.ty_of_inner(ty);
        let out = self.apply_vector_size(ty, base);
        self.out.syntactic_types.insert(ty, out);
        out
    }

    /// `__attribute__((vector_size(n)))` turns the type it is written on into a vector of
    /// `n` **bytes** — not `n` lanes, which is the reading that silently produces vectors
    /// several times too long.
    ///
    /// 013 §4 lists this as required because VPP's SIMD paths are built on it: every
    /// `u8x16`/`u64x4`/`f32x16` in `vppinfra/vector.h` is a `vector_size` typedef, and
    /// without it 258 of the corpus's 2909 layout assertions are wrong.
    fn apply_vector_size(&mut self, node: TypeId, base: TyId) -> TyId {
        let attrs = self.ast.ty(node).attrs.clone();
        let mut bytes: Option<u64> = None;
        let mut requested: Option<u64> = None;
        for a in &attrs {
            let name = self.text(a.name).unwrap_or("").to_owned();
            let arg = a.args.first().copied();
            match name.as_str() {
                "vector_size" | "__vector_size__" => {
                    if let Some(v) = arg
                        .and_then(|e| self.eval(e))
                        .map(|v| v.v)
                        .filter(|&v| v > 0)
                    {
                        bytes = Some(v as u64);
                    }
                }
                "aligned" | "__aligned__" => {
                    if let Some(v) = arg
                        .and_then(|e| self.eval(e))
                        .map(|v| v.v)
                        .filter(|&v| v > 0)
                    {
                        requested = Some(v as u64);
                    }
                }
                _ => {}
            }
        }
        let Some(bytes) = bytes else { return base };
        let elem_size = size_of_ty(&self.out, &self.target, base).unwrap_or(0);
        if elem_size == 0 || bytes % elem_size != 0 {
            return base;
        }
        let lanes = (bytes / elem_size) as u32;
        // gcc aligns a vector to its own width unless `aligned` says otherwise — and
        // `aligned(1)` really does lower it, which is what VPP's `u64x4u` relies on.
        // An explicit `aligned(n)` wins in **both** directions — VPP's `u64x4u` is
        // `aligned(1)` and really is byte-aligned. Otherwise the vector's own width; the
        // psABI cap is applied by `align_of_ty`, not here, because placement needs the
        // uncapped number.
        let align = requested.unwrap_or(bytes);
        self.intern(Ty::Vector {
            elem: base,
            lanes,
            align,
        })
    }

    fn ty_of_inner(&mut self, ty: TypeId) -> TyId {
        let node = self.ast.ty(ty).clone();
        match node.kind {
            TypeKind::Builtin(b) => {
                let t = self.builtin(b);
                self.intern(t)
            }
            TypeKind::Named(sym) => match self.typedefs.get(&sym).copied() {
                Some(t) => t,
                None => {
                    let name = self.text(sym).unwrap_or("?").to_owned();
                    self.error(node.span, format!("unknown type name `{name}`"));
                    self.intern(Ty::Error)
                }
            },
            TypeKind::Ptr(inner) => {
                let p = self.ty_of(inner);
                self.intern(Ty::Ptr(p))
            }
            TypeKind::Array { elem, len } => {
                let e = self.ty_of(elem);
                let l = match len {
                    chiero_ast::ArrayLen::Zero => ArrayLen::Zero,
                    chiero_ast::ArrayLen::Unspecified | chiero_ast::ArrayLen::Star => {
                        ArrayLen::Flexible
                    }
                    chiero_ast::ArrayLen::Fixed(expr) => {
                        let n = self.eval(expr).map(|v| v.v);
                        match n {
                            Some(n) if n >= 0 => ArrayLen::Fixed(n as u64),
                            Some(_) => {
                                self.error(node.span, "array length is negative");
                                ArrayLen::Fixed(0)
                            }
                            None => ArrayLen::Vla(expr),
                        }
                    }
                };
                self.intern(Ty::Array { elem: e, len: l })
            }
            TypeKind::Func {
                ret,
                params,
                variadic,
                ..
            } => {
                let r = self.ty_of(ret);
                let ps = params
                    .iter()
                    .map(|&p| match &self.ast.decl(p).kind {
                        DeclKind::Var { ty, .. } => {
                            let ty = *ty;
                            self.ty_of(ty)
                        }
                        _ => self.intern(Ty::Error),
                    })
                    .collect();
                self.intern(Ty::Func {
                    ret: r,
                    params: ps,
                    variadic,
                })
            }
            TypeKind::Tag { tag, name, members } => self.tag(ty, tag, name, members),
            TypeKind::TypeofExpr(_) | TypeKind::TypeofType(_) => {
                // `typeof` needs expression typing, which is contract 11's half of 014
                // and is not this slice. `Error` is the honest answer and it propagates
                // rather than producing a wrong size.
                self.intern(Ty::Error)
            }
            TypeKind::Error => self.intern(Ty::Error),
        }
    }

    fn builtin(&mut self, b: chiero_ast::Builtin) -> Ty {
        use chiero_ast::Builtin as B;
        let s = &self.target.sizes;
        let int = |signed, bits| Ty::Int { signed, bits };
        match b {
            B::Void => Ty::Void,
            B::Bool => int(false, 1),
            // **Plain `char` follows the target** (contract 9), and is a *third* type
            // distinct from both `signed char` and `unsigned char`. Only this one moves.
            B::Char => int(self.target.char_signed, 8),
            B::SChar => int(true, 8),
            B::UChar => int(false, 8),
            B::Short => int(true, (s.short_ * 8) as u32),
            B::UShort => int(false, (s.short_ * 8) as u32),
            B::Int => int(true, (s.int_ * 8) as u32),
            B::UInt => int(false, (s.int_ * 8) as u32),
            B::Long => int(true, (s.long_ * 8) as u32),
            B::ULong => int(false, (s.long_ * 8) as u32),
            B::LongLong => int(true, (s.long_long * 8) as u32),
            B::ULongLong => int(false, (s.long_long * 8) as u32),
            B::Int128 => int(true, 128),
            B::UInt128 => int(false, 128),
            B::VaList => {
                // On x86-64 `__builtin_va_list` is `struct __va_list_tag [1]`:
                //
                //     unsigned int gp_offset;    //  0
                //     unsigned int fp_offset;    //  4
                //     void *overflow_arg_area;   //  8
                //     void *reg_save_area;       // 16
                //
                // 24 bytes, aligned 8 because of the pointers. Modelled by its ABI *shape*
                // — three pointer-width words — rather than as a record, because nothing
                // declares the record and 021 needs only the extent and alignment.
                //
                // This arm used to build `Array { elem: <sentinel>, len: Fixed(0) }` under
                // a comment claiming the numbers above. `size_of` returned `None` for the
                // sentinel element, lowering's `.max(1)` made `va_list ap` one byte, and
                // every read of it was out of bounds.
                let word = self.intern(Ty::Int {
                    signed: false,
                    bits: self.target.pointer_width,
                });
                Ty::Array {
                    elem: word,
                    len: ArrayLen::Fixed(24 / u64::from(self.target.pointer_width / 8).max(1)),
                }
            }
            B::Float => Ty::Float(FloatKind::F32),
            B::Double => Ty::Float(FloatKind::F64),
            B::LongDouble => Ty::Float(match self.target.long_double {
                LongDoubleKind::X87_80 => FloatKind::X87_80,
                LongDoubleKind::Binary128 => FloatKind::Binary128,
                LongDoubleKind::Double => FloatKind::F64,
            }),
            B::ExtFloat { bits, fmt } => Ty::Float(match (bits, fmt) {
                (16, chiero_ast::FloatFmt::Brain) => FloatKind::BFloat16,
                (16, _) => FloatKind::Binary16,
                (32, _) => FloatKind::F32,
                (64, _) => FloatKind::F64,
                _ => FloatKind::Binary128,
            }),
        }
    }

    fn tag(
        &mut self,
        node: TypeId,
        tag: chiero_ast::TagKind,
        name: Option<Symbol>,
        members: Option<Vec<DeclId>>,
    ) -> TyId {
        let span = self.ast.ty(node).span;
        if tag == chiero_ast::TagKind::Enum {
            return self.enum_ty(span, name, members);
        }
        // A reference to an already-defined tag.
        if members.is_none() {
            if let Some(&rid) = name.and_then(|n| self.tags.get(&n)) {
                return self.intern(Ty::Record(rid));
            }
            // An incomplete type. Legal in a pointer, which is the only place it can
            // appear without a size being asked for.
            return self.intern(Ty::Error);
        }
        if let Some(name) = name {
            if self.in_progress.contains(&name) {
                let n = self.text(name).unwrap_or("?").to_owned();
                self.error(span, format!("`struct {n}` contains itself by value"));
                return self.intern(Ty::Error);
            }
            self.in_progress.push(name);
        }
        let layout = self.lay_out(node, tag == chiero_ast::TagKind::Union, &members.unwrap());
        if name.is_some() {
            self.in_progress.pop();
        }
        let rid = RecordId(self.out.records.len() as u32);
        self.out.records.push(layout);
        if let Some(name) = name {
            self.tags.insert(name, rid);
            self.out.by_tag.insert(name, rid);
        }
        self.intern(Ty::Record(rid))
    }

    /// 014 contract 10: the underlying type is `int` unless a value requires wider.
    fn enum_ty(&mut self, span: Span, name: Option<Symbol>, members: Option<Vec<DeclId>>) -> TyId {
        let Some(members) = members else {
            if let Some(&t) = name.and_then(|n| self.enums.get(&n)) {
                return t;
            }
            let _ = span;
            let bits = (self.target.sizes.int_ * 8) as u32;
            return self.intern(Ty::Int { signed: true, bits });
        };
        let mut next = 0i128;
        let mut lo = 0i128;
        let mut hi = 0i128;
        // The constants' *type* is the enumeration's, and that is not known until every
        // value has been seen — so they are collected here and recorded below.
        let mut pending: Vec<(Symbol, i128)> = Vec::new();
        for m in &members {
            let DeclKind::Var {
                name: Some(en),
                init,
                ..
            } = self.ast.decl(*m).kind.clone()
            else {
                continue;
            };
            let v = match init {
                Some(e) => self.eval(e).map(|v| v.v).unwrap_or(next),
                None => next,
            };
            next = v.wrapping_add(1);
            lo = lo.min(v);
            hi = hi.max(v);
            self.enumerators.insert(en, v);
            pending.push((en, v));
        }
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        let fits_int = lo >= -(1i128 << (int_bits - 1)) && hi < (1i128 << (int_bits - 1));
        let t = if fits_int {
            Ty::Int {
                signed: true,
                bits: int_bits,
            }
        } else {
            // gcc widens to `long` (64-bit here) rather than to an arbitrary width.
            Ty::Int {
                signed: lo < 0 || hi < (1i128 << 63),
                bits: (self.target.sizes.long_ * 8) as u32,
            }
        };
        let id = self.intern(t);
        // **On the output**, so a consumer that outlives this context can ask — lowering
        // had no way to reach `Cx::enumerators` and lowered every use to `undef`.
        for (en, v) in pending {
            self.out.enumerators.insert(en, (v, id));
        }
        if let Some(n) = name {
            self.enums.insert(n, id);
        }
        id
    }

    /// 014 §3. The gcc x86-64 rules, with bit-fields.
    fn lay_out(&mut self, node: TypeId, is_union: bool, members: &[DeclId]) -> RecordLayout {
        let packed = self
            .ast
            .ty(node)
            .attrs
            .iter()
            .any(|a| matches!(self.text(a.name), Some("packed" | "__packed__")));

        let mut fields: Vec<FieldLayout> = Vec::new();
        let mut bit_cursor: u64 = 0; // bits from the start of the record
        let mut size_bits: u64 = 0;
        let mut align: u64 = 1;
        let mut flexible_member = None;

        for &m in members {
            let DeclKind::Var { name, ty, .. } = self.ast.decl(m).kind.clone() else {
                continue;
            };
            let fty = self.ty_of(ty);
            let member_packed = packed
                || self
                    .ast
                    .ty(ty)
                    .attrs
                    .iter()
                    .any(|a| matches!(self.text(a.name), Some("packed" | "__packed__")));
            // `aligned(n)` raises alignment, and combined with `packed` it does **not**
            // re-introduce padding before the member — it only raises the record's own
            // alignment. 014 §3 calls this out because getting it backwards is the
            // common error.
            let requested = self.aligned_attr(ty);

            let bw = self.ast.bitfield(m);
            if let Some(bw) = bw {
                let w = self.eval(bw).map(|v| v.v).unwrap_or(0).max(0) as u64;
                let unit_bits = size_of_ty(&self.out, &self.target, fty).unwrap_or(4) * 8;
                let unit_align_bits = align_of_ty(&self.out, &self.target, fty).unwrap_or(4) * 8;

                if w == 0 {
                    // Contract 4: declares no member, and forces the next allocation to
                    // the next unit boundary.
                    if !member_packed {
                        bit_cursor = round_up(bit_cursor, unit_align_bits);
                        align = align.max(unit_align_bits / 8);
                    }
                    size_bits = size_bits.max(bit_cursor);
                    continue;
                }
                let mut start = if is_union { 0 } else { bit_cursor };
                if !member_packed {
                    // Straddling (contract 5): if the field would cross a boundary of its
                    // declared type's storage unit, move it to the next one.
                    if unit_bits > 0 && (start % unit_bits) + w > unit_bits {
                        start = round_up(start, unit_align_bits);
                    }
                    align = align.max(unit_align_bits / 8).max(requested.unwrap_or(1));
                } else if let Some(r) = requested {
                    align = align.max(r);
                }
                fields.push(FieldLayout {
                    name,
                    ty: fty,
                    offset: start / 8,
                    bits: Some(BitField {
                        bit_offset: start,
                        width: w,
                    }),
                });
                bit_cursor = start + w;
                size_bits = size_bits.max(start + w);
                continue;
            }

            // An ordinary member.
            let msize = size_of_ty(&self.out, &self.target, fty).unwrap_or(0);
            let malign = if member_packed {
                requested.unwrap_or(1)
            } else {
                field_align_of_ty(&self.out, &self.target, fty)
                    .unwrap_or(1)
                    .max(requested.unwrap_or(1))
            };
            let start = if is_union {
                0
            } else {
                round_up(bit_cursor, malign * 8)
            };
            let is_flexible = matches!(
                self.out.types[fty.0 as usize],
                Ty::Array {
                    len: ArrayLen::Flexible,
                    ..
                }
            );
            if is_flexible {
                flexible_member = Some(fields.len());
            }
            fields.push(FieldLayout {
                name,
                ty: fty,
                offset: start / 8,
                bits: None,
            });
            // A flexible or zero-length array contributes 0 to size but does affect
            // alignment (contract 7).
            //
            // The cursor advances even in a union, where it is then **never read** —
            // every union member starts at 0. Guarding it was dead code that read as
            // though it mattered, and two mutations that removed the guards changed
            // nothing, which is how it was found.
            bit_cursor = start + msize * 8;
            size_bits = size_bits.max(start + msize * 8);
            // The record's own alignment takes the **capped** value: gcc reports
            // `_Alignof(struct { char c; u64x4 v; })` as 16 even though `v` sits at 32.
            align = align.max(if member_packed {
                requested.unwrap_or(1)
            } else {
                align_of_ty(&self.out, &self.target, fty)
                    .unwrap_or(1)
                    .max(requested.unwrap_or(1))
            });
        }

        if packed {
            // The *record's* alignment is 1 unless a member asked for more.
            let requested_max = members
                .iter()
                .filter_map(|&m| match &self.ast.decl(m).kind {
                    DeclKind::Var { ty, .. } => self.aligned_attr(*ty),
                    _ => None,
                })
                .max()
                .unwrap_or(1);
            align = requested_max.max(1);
        }
        if let Some(r) = self.aligned_attr(node) {
            align = align.max(r);
        }
        let size = round_up(round_up(size_bits, 8) / 8, align);
        RecordLayout {
            size,
            align,
            fields,
            is_union,
            flexible_member,
            packed,
        }
    }

    /// The `n` of `__attribute__((aligned(n)))` on a syntactic type node.
    fn aligned_attr(&mut self, ty: TypeId) -> Option<u64> {
        let attrs = self.ast.ty(ty).attrs.clone();
        let mut best: Option<u64> = None;
        for a in attrs {
            if !matches!(self.text(a.name), Some("aligned" | "__aligned__")) {
                continue;
            }
            let Some(&arg) = a.args.first() else {
                // Bare `aligned` means the target's biggest useful alignment.
                best = Some(best.unwrap_or(0).max(16));
                continue;
            };
            if let Some(n) = self.eval(arg).map(|v| v.v).filter(|&n| n > 0) {
                best = Some(best.unwrap_or(0).max(n as u64));
            }
        }
        best
    }
}

impl Cx<'_> {
    /// 014 §6, the integer subset — array bounds, bit-field widths, enum values,
    /// `_Static_assert`, case labels.
    ///
    /// Types are tracked alongside values because contract 19 is a question about types:
    /// overflow **wraps and diagnoses once** rather than poisoning the expression, since
    /// an array bound that stopped resolving would cascade into every use of the type,
    /// which is the opposite of what §5's `Ty::Error` policy is for.
    fn eval(&mut self, expr: ExprId) -> Option<IntVal> {
        use chiero_ast::{BinOp, ExprKind, UnOp};
        let node = self.ast.expr(expr).clone();
        let span = node.span;
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        match node.kind {
            ExprKind::Number(sym) => {
                let text = self.text(sym)?.to_owned();
                parse_int_literal(&text, &self.target)
            }
            // A character constant has type `int` in C, not `char` — unless it carries a
            // prefix, which `strlit::char_element` is what knows. Its value comes from the
            // decoder string literals use, because the two share every escape rule and one
            // of them (a UCN becoming multiple UTF-8 bytes in a plain constant) is only
            // visible to a decoder that yields units.
            ExprKind::Char { spelling } => {
                let text = self.text(spelling)?.to_owned();
                let (signed, bits) = strlit::char_element(&text);
                Some(IntVal {
                    v: strlit::char_value(&text)?,
                    bits,
                    signed,
                })
            }
            ExprKind::Ident(sym) => self.enumerators.get(&sym).copied().map(|v| IntVal {
                v,
                bits: int_bits,
                signed: true,
            }),
            ExprKind::SizeofType(ty) => {
                let t = self.ty_of(ty);
                let n = size_of_ty(&self.out, &self.target, t)?;
                Some(self.size_t(n as i128))
            }
            ExprKind::AlignofType(ty) => {
                let t = self.ty_of(ty);
                let n = align_of_ty(&self.out, &self.target, t)?;
                Some(self.size_t(n as i128))
            }
            ExprKind::Unary { op, operand } => {
                let a = self.eval(operand)?;
                let a = promote(a, int_bits);
                Some(match op {
                    UnOp::Plus => a,
                    UnOp::Minus => self.wrap(a.v.wrapping_neg(), a, span),
                    UnOp::Not => IntVal {
                        v: (a.v == 0) as i128,
                        bits: int_bits,
                        signed: true,
                    },
                    UnOp::BitNot => self.wrap(!a.v, a, span),
                    _ => return None,
                })
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = promote(self.eval(lhs)?, int_bits);
                let b = promote(self.eval(rhs)?, int_bits);
                let r = usual_arithmetic(a, b);
                let bool_ = IntVal {
                    v: 0,
                    bits: int_bits,
                    signed: true,
                };
                let (raw, ty) = match op {
                    BinOp::Add => (a.v.checked_add(b.v)?, r),
                    BinOp::Sub => (a.v.checked_sub(b.v)?, r),
                    BinOp::Mul => (a.v.checked_mul(b.v)?, r),
                    BinOp::Div | BinOp::Rem => {
                        if b.v == 0 {
                            self.error(span, "division by zero in a constant expression");
                            return None;
                        }
                        let v = if matches!(op, BinOp::Div) {
                            a.v.checked_div(b.v)?
                        } else {
                            a.v.checked_rem(b.v)?
                        };
                        (v, r)
                    }
                    // Shifts take the *left* operand's type, not the usual conversions.
                    BinOp::Shl => (a.v.checked_shl(b.v.try_into().ok()?)?, a),
                    BinOp::Shr => (a.v.checked_shr(b.v.try_into().ok()?)?, a),
                    BinOp::Lt => ((a.v < b.v) as i128, bool_),
                    BinOp::Gt => ((a.v > b.v) as i128, bool_),
                    BinOp::Le => ((a.v <= b.v) as i128, bool_),
                    BinOp::Ge => ((a.v >= b.v) as i128, bool_),
                    BinOp::Eq => ((a.v == b.v) as i128, bool_),
                    BinOp::Ne => ((a.v != b.v) as i128, bool_),
                    BinOp::BitAnd => (a.v & b.v, r),
                    BinOp::BitXor => (a.v ^ b.v, r),
                    BinOp::BitOr => (a.v | b.v, r),
                    BinOp::LogAnd => (((a.v != 0) && (b.v != 0)) as i128, bool_),
                    BinOp::LogOr => (((a.v != 0) || (b.v != 0)) as i128, bool_),
                };
                Some(self.wrap(raw, ty, span))
            }
            ExprKind::Cond { cond, then, els } => {
                let c = self.eval(cond)?;
                if c.v != 0 {
                    match then {
                        Some(t) => self.eval(t),
                        None => Some(c),
                    }
                } else {
                    self.eval(els)
                }
            }
            ExprKind::Comma { rhs, .. } => self.eval(rhs),
            // **`__builtin_constant_p`** (contract 18). The answer is 1 exactly when the
            // argument folds — and it is a *constant* either way, which is the point:
            // VPP wraps it in a `?:` that must fold away so only one implementation is
            // compiled. Answering "does not fold" instead of a constant 0 would make
            // every such macro keep both branches.
            ExprKind::Call { callee, args } => {
                let ExprKind::Ident(name) = self.ast.expr(callee).kind else {
                    return None;
                };
                if self.text(name) != Some("__builtin_constant_p") {
                    return None;
                }
                let folds = args
                    .first()
                    .map(|&a| {
                        // The argument's own diagnostics are discarded: asking whether
                        // something is constant is not itself an error, however the
                        // answer comes out.
                        let before = self.out.diagnostics.len();
                        let v = self.eval(a).is_some() || self.addr_of(a).is_some();
                        self.out.diagnostics.truncate(before);
                        v
                    })
                    .unwrap_or(false);
                Some(IntVal {
                    v: folds as i128,
                    bits: int_bits,
                    signed: true,
                })
            }
            // A cast to an integer type truncates to that type, which is how
            // `(char)0xFF == -1` gets its answer (contract 9).
            ExprKind::Cast { ty, operand } => {
                let v = self.eval(operand)?;
                let t = self.ty_of(ty);
                match self.out.types.get(t.0 as usize) {
                    Some(Ty::Int { signed, bits }) => {
                        let (signed, bits) = (*signed, *bits);
                        Some(truncate(v.v, bits, signed))
                    }
                    _ => Some(v),
                }
            }
            _ => None,
        }
    }

    /// The expression of a compound statement's **last expression statement**, which is a
    /// statement expression's value (015 §2.4).
    fn last_value_of_block(&self, body: StmtId) -> Option<ExprId> {
        let StmtKind::Compound(ss) = &self.ast.stmt(body).kind else {
            return None;
        };
        ss.iter().rev().find_map(|&s| match self.ast.stmt(s).kind {
            StmtKind::Expr(e) => Some(e),
            _ => None,
        })
    }

    fn analysis_top_ty(&self, e: ExprId) -> Option<TyId> {
        let t = self.out.typed.top(e)?;
        Some(self.out.typed.ty_of(t))
    }

    fn size_t(&self, v: i128) -> IntVal {
        IntVal {
            v,
            bits: (self.target.sizes.long_ * 8) as u32,
            signed: false,
        }
    }

    /// Fit `raw` into `ty`, diagnosing **once** if it does not fit.
    fn wrap(&mut self, raw: i128, ty: IntVal, span: Span) -> IntVal {
        let fitted = truncate(raw, ty.bits, ty.signed);
        if fitted.v != raw && ty.signed {
            self.error(span, "signed overflow in a constant expression");
        }
        fitted
    }
}

/// Integer promotion: anything narrower than `int` becomes `int`.
fn promote(v: IntVal, int_bits: u32) -> IntVal {
    if v.bits >= int_bits {
        return v;
    }
    IntVal {
        v: v.v,
        bits: int_bits,
        signed: true,
    }
}

/// C's usual arithmetic conversions for the integer cases.
fn usual_arithmetic(a: IntVal, b: IntVal) -> IntVal {
    if a.bits == b.bits {
        return IntVal {
            v: 0,
            bits: a.bits,
            // If either is unsigned at the same width, the result is unsigned.
            signed: a.signed && b.signed,
        };
    }
    let (w, n) = if a.bits > b.bits { (a, b) } else { (b, a) };
    IntVal {
        v: 0,
        bits: w.bits,
        // A wider signed type can represent every value of a narrower unsigned one.
        signed: if w.signed && !n.signed {
            true
        } else {
            w.signed
        },
    }
}

fn truncate(v: i128, bits: u32, signed: bool) -> IntVal {
    let bits = bits.clamp(1, 127);
    let mask = (1i128 << bits) - 1;
    let raw = v & mask;
    let out = if signed && (raw >> (bits - 1)) & 1 == 1 {
        raw - (1i128 << bits)
    } else {
        raw
    };
    IntVal {
        v: out,
        bits,
        signed,
    }
}

fn round_up(v: u64, to: u64) -> u64 {
    if to == 0 {
        return v;
    }
    v.div_ceil(to) * to
}

fn size_of_ty(a: &Analysis, t: &TargetConfig, id: TyId) -> Option<u64> {
    match a.types.get(id.0 as usize)? {
        Ty::Void => Some(1), // gcc's `sizeof(void) == 1`
        Ty::Int { bits, .. } => Some(((*bits).max(8) as u64).div_ceil(8)),
        Ty::Float(f) => Some(match f {
            FloatKind::Binary16 | FloatKind::BFloat16 => 2,
            FloatKind::F32 => 4,
            FloatKind::F64 => 8,
            FloatKind::X87_80 => 16,
            FloatKind::Binary128 => 16,
        }),
        Ty::Ptr(_) => Some((t.pointer_width / 8) as u64),
        Ty::Array { elem, len } => match len {
            ArrayLen::Fixed(n) => Some(size_of_ty(a, t, *elem)? * n),
            // Contributes 0 to size, but its alignment still counts. A **VLA** likewise
            // has no compile-time size — its extent is supplied by an `AllocaDyn` at the
            // declaration (020 §3), which is what `DYNAMIC_EXTENT` records.
            ArrayLen::Flexible | ArrayLen::Zero | ArrayLen::Vla(_) => Some(0),
        },
        Ty::Func { .. } => None,
        Ty::Record(r) => Some(a.records.get(r.0 as usize)?.size),
        Ty::Vector { elem, lanes, .. } => Some(size_of_ty(a, t, *elem)? * (*lanes as u64)),
        Ty::Error => None,
    }
}

/// The alignment a **member of this type is placed at**, which is not always its
/// `_Alignof`.
///
/// They differ only for vectors, and gcc's own answers are the evidence:
/// `_Alignof(u64x4)` is 16 while `offsetof(struct { char c; u64x4 v; }, v)` is 32.
fn field_align_of_ty(a: &Analysis, t: &TargetConfig, id: TyId) -> Option<u64> {
    match a.types.get(id.0 as usize)? {
        Ty::Vector { align, .. } => Some(*align),
        Ty::Array { elem, .. } => field_align_of_ty(a, t, *elem),
        _ => align_of_ty(a, t, id),
    }
}

fn align_of_ty(a: &Analysis, t: &TargetConfig, id: TyId) -> Option<u64> {
    match a.types.get(id.0 as usize)? {
        Ty::Void => Some(1),
        Ty::Int { bits, .. } => Some(match bits {
            0..=8 => 1,
            9..=16 => t.aligns.short_,
            17..=32 => t.aligns.int_,
            33..=64 => t.aligns.long_,
            _ => 16,
        }),
        Ty::Float(f) => Some(match f {
            FloatKind::Binary16 | FloatKind::BFloat16 => 2,
            FloatKind::F32 => 4,
            FloatKind::F64 => t.aligns.double_,
            FloatKind::X87_80 | FloatKind::Binary128 => t.aligns.long_double,
        }),
        Ty::Ptr(_) => Some(t.aligns.pointer),
        Ty::Array { elem, .. } => align_of_ty(a, t, *elem),
        Ty::Func { .. } => None,
        Ty::Record(r) => Some(a.records.get(r.0 as usize)?.align),
        // The psABI cap, which `_Alignof` reports and which a record's own alignment is
        // computed from. Field *placement* uses the uncapped value — see
        // `field_align_of_ty`.
        Ty::Vector { align, .. } => Some((*align).min(t.max_vector_align)),
        Ty::Error => None,
    }
}

/// A literal's value **and the type C gives it**, which is what decides whether a later
/// operation overflows: `2147483647 + 1` is `int` arithmetic and overflows,
/// `0x100000000 + 1` is `long` arithmetic and does not.
///
/// The rule is C11 §6.4.4.1: take the first type in the literal's rank list that can
/// represent the value. A decimal literal without a suffix never becomes unsigned; a hex
/// or octal one may, which is why `0xffffffff` is `unsigned int` and `4294967295` is
/// `long`.
/// A **floating literal**: its type from the suffix, and its value.
///
/// One decoder, exported, for the reason waves 151 and 152 established: sema picks the
/// literal's type and lowering needs its bits, and two readings of one spelling are free to
/// disagree. `FloatKind` comes from the suffix (C11 6.4.4.2 — `f`/`F` is `float`, `l`/`L`
/// is `long double`, otherwise `double`), and the value from Rust's `f64` parser, which
/// accepts the same decimal and hexadecimal forms C does.
///
/// `None` for anything that is not a floating literal, which is how the caller tells this
/// from an integer one without parsing twice.
pub fn float_literal(text: &str) -> Option<(FloatKind, f64)> {
    let t = text.replace('\'', ""); // C23 digit separators
    let lower = t.to_ascii_lowercase();
    // A hexadecimal *integer* is not a float, and `0x1p3` is. The exponent marker is `p`
    // for hex and `e` for decimal, so the presence of a dot or the right marker decides.
    let hex = lower.starts_with("0x");
    let looks_float = t.contains('.')
        || (hex && lower.contains('p'))
        || (!hex && lower.contains('e') && !lower.starts_with("0b"));
    let kind = if lower.ends_with('f') {
        FloatKind::F32
    } else if lower.ends_with('l') {
        // x87 long double. Its *value* is parsed at f64 and the extra range is lost, which
        // is a narrowing this records rather than hides; arithmetic on it is a gap anyway.
        FloatKind::X87_80
    } else {
        FloatKind::F64
    };
    if !looks_float && !(lower.ends_with('f') && !hex) {
        return None;
    }
    let digits = t.trim_end_matches(['f', 'F', 'l', 'L']);
    // **Rust's parser does not accept hex float syntax**, and `looks_float` above already said
    // this is one. Falling through returned `None`, lowering took the integer path, and the
    // verifier refused the function for emitting `Const::Int` where a float was declared — a
    // whole function lost to a literal C99 6.4.4.2 has had since 1999.
    if hex {
        return hex_float(&lower).map(|v| (kind, v));
    }
    digits.parse::<f64>().ok().map(|v| (kind, v))
}

/// `0x1.8p1` and friends, C99 6.4.4.2.
///
/// **Exact, because a hex literal's digits are already binary.** Each digit is four bits and the
/// `p` exponent is a power of two, so the value is `mantissa × 2^(exp - 4 × fraction_digits)` with
/// no decimal-to-binary rounding to get wrong. That is the whole reason C has this syntax, and it
/// is why an `f80` value can be written exactly in source even though a *decimal* `long double`
/// literal is still rounded through `f64` (§9).
///
/// The mantissa is accumulated in a `u64` and the scaling is left to `f64` arithmetic on a power of
/// two, which is exact for any mantissa `f64` can hold. A literal needing more than 53 significant
/// bits rounds here, which is the same narrowing decimal literals have and not a new one.
fn hex_float(lower: &str) -> Option<f64> {
    // `0x` first, then the suffix. The order is not load-bearing — for any *valid* hex float the
    // mandatory `p` exponent puts a decimal digit between the mantissa and the suffix, so trimming
    // `f`/`l` can never eat a hex digit — and mutation says so: swapping the two changes nothing.
    // It reads better this way and the comment is here so the next reader does not go looking for
    // the trap I thought I had avoided.
    let body = lower.strip_prefix("0x")?.trim_end_matches(['f', 'l']);
    // The exponent is mandatory in C for a hex float; without it the literal is not one, which
    // `looks_float` has already decided.
    let (mant, exp) = body.split_once('p')?;
    let (int_part, frac_part) = mant.split_once('.').unwrap_or((mant, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    let mut m: u64 = 0;
    for c in int_part.chars().chain(frac_part.chars()) {
        m = m.checked_mul(16)?.checked_add(u64::from(c.to_digit(16)?))?;
    }
    // C allows a sign on the exponent and `i32::from_str` accepts both, `+3` included — so there
    // is nothing to strip. I wrote a `strip_prefix('+')` here on the assumption that it did not,
    // and mutation showed the line was dead: removing it changed nothing, including for `0x1p+3`.
    let e: i32 = exp.parse().ok()?;
    // Every fraction digit is four binary places below the point.
    let scale = e.checked_sub(4i32.checked_mul(i32::try_from(frac_part.len()).ok()?)?)?;
    Some(m as f64 * 2f64.powi(scale))
}

fn parse_int_literal(text: &str, target: &TargetConfig) -> Option<IntVal> {
    let lower = text.to_ascii_lowercase();
    let unsigned_suffix = lower.contains('u');
    let long_suffix = lower.contains('l');
    let t: String = text
        .trim_end_matches(['u', 'U', 'l', 'L', 'z', 'Z'])
        .replace('\'', ""); // C23 digit separators
    let (v, decimal) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (i128::from_str_radix(h, 16).ok()?, false)
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (i128::from_str_radix(b, 2).ok()?, false)
    } else if t.len() > 1 && t.starts_with('0') && t.bytes().all(|c| c.is_ascii_digit()) {
        (i128::from_str_radix(&t[1..], 8).ok()?, false)
    } else {
        (t.parse::<i128>().ok()?, true)
    };

    let int_bits = (target.sizes.int_ * 8) as u32;
    let long_bits = (target.sizes.long_ * 8) as u32;
    let mut candidates: Vec<(u32, bool)> = Vec::new();
    if !long_suffix {
        if !unsigned_suffix {
            candidates.push((int_bits, true));
        }
        if unsigned_suffix || !decimal {
            candidates.push((int_bits, false));
        }
    }
    if !unsigned_suffix {
        candidates.push((long_bits, true));
    }
    if unsigned_suffix || !decimal {
        candidates.push((long_bits, false));
    }
    candidates.push((128, !unsigned_suffix));
    for (bits, signed) in candidates {
        if fits(v, bits, signed) {
            return Some(IntVal { v, bits, signed });
        }
    }
    Some(IntVal {
        v,
        bits: 128,
        signed: true,
    })
}

fn fits(v: i128, bits: u32, signed: bool) -> bool {
    if bits >= 127 {
        return true;
    }
    if signed {
        v >= -(1i128 << (bits - 1)) && v < (1i128 << (bits - 1))
    } else {
        v >= 0 && v < (1i128 << bits)
    }
}

// ---------------------------------------------------------------------------------
// 014 §5 — expression typing, with every conversion made explicit
// ---------------------------------------------------------------------------------

impl Cx<'_> {
    fn push_typed(&mut self, node: TypedNode) -> TypedId {
        let id = TypedId(self.out.typed.nodes.len() as u32);
        self.out.typed.nodes.push(node);
        id
    }

    /// Record `id` as the outermost node for `expr`, so `top` and `conversions_of`
    /// answer about the value the *consumer* sees rather than the one written.
    fn set_top(&mut self, expr: ExprId, id: TypedId) -> TypedId {
        self.out.typed.by_expr.insert(expr, id);
        id
    }

    /// Insert an explicit conversion, or return the operand unchanged when the types
    /// already agree.
    ///
    /// **Not inserting a no-op cast is the point**, not an optimization: contract 11's
    /// corpus check reads operand types, and a tree full of `int -> int` casts would
    /// satisfy it while saying nothing. A `Cast` here means a real conversion happened.
    fn convert(&mut self, node: TypedId, to: TyId, why: Conversion, span: Span) -> TypedId {
        if self.out.typed.ty_of(node) == to {
            return node;
        }
        self.push_typed(TypedNode::Cast {
            operand: node,
            ty: to,
            span,
            why,
        })
    }

    /// The type an expression has **before** conversion, plus its typed node.
    fn type_expr(&mut self, expr: ExprId) -> TypedId {
        let node = self.ast.expr(expr).clone();
        let span = node.span;
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        let id = match &node.kind {
            ExprKind::Number(sym) => {
                let text = self.text(*sym).unwrap_or("").to_owned();
                let v = parse_int_literal(&text, &self.target);
                let ty = match v {
                    Some(v) => self.intern(Ty::Int {
                        signed: v.signed,
                        bits: v.bits,
                    }),
                    // A floating literal; 014 §2's `FloatKind` from the suffix, read by
                    // the same decoder lowering uses for the value.
                    None => {
                        let k = float_literal(&text).map_or(FloatKind::F64, |(k, _)| k);
                        self.intern(Ty::Float(k))
                    }
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::Char { spelling } => {
                // The prefix decides the type: `u'a'` is `char16_t`, whose size of 2 is the
                // only one `sizeof` can tell from `int`'s. Every constant used to be `int`.
                let (signed, bits) = self
                    .text(*spelling)
                    .map(strlit::char_element)
                    .unwrap_or((true, int_bits));
                let ty = self.intern(Ty::Int { signed, bits });
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::Str { fragments } => {
                // A string literal is an array of its **element type**, and it decays like
                // any other array. C11 6.4.5p6: `L"…"` has element `wchar_t`, `u"…"`
                // `char16_t`, `U"…"` `char32_t`; plain and `u8"…"` are `char`.
                //
                // Every literal used to be `char[n]` whatever prefix it carried, so
                // `sizeof(L"AB")` answered 3 instead of 12 — a silent wrong answer, and one
                // §9 attributed to `unquote`, which strips the prefix perfectly well and
                // never had the type to lose.
                let (esign, ebits) = fragments
                    .first()
                    .and_then(|f| self.text(f.spelling))
                    .map(strlit::string_element)
                    .unwrap_or((self.target.char_signed, 8));
                let elem = self.intern(Ty::Int {
                    signed: esign,
                    bits: ebits,
                });
                // **Phase 5 decides the length, not the source text.** Counting source
                // characters made `"a\nb"` five elements and `u"\uFFFF"` five where C has
                // four and one. The count and the contents now come from one decoder
                // (`strlit`), so the array sema sizes is the array lowering fills.
                let n: u64 = fragments
                    .iter()
                    .filter_map(|f| self.text(f.spelling))
                    .map(|t| strlit::string_elements(t, ebits).len() as u64)
                    .sum::<u64>()
                    + 1;
                let ty = self.intern(Ty::Array {
                    elem,
                    len: ArrayLen::Fixed(n),
                });
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::Ident(sym) => {
                let ty = self
                    .values
                    .get(sym)
                    .copied()
                    // **An enumeration constant has its enumeration's type**, which is
                    // `int` only when every value in it fits one. Hardcoding `int_bits`
                    // here typed `enum Big { X = 5000000000 }`'s constant 32 bits wide and
                    // lowering truncated it to `5000000000i32`.
                    //
                    // `self.enumerators` is the *scoped* map, so this resolves the name the
                    // way C does; the reference and its answer are recorded together so
                    // lowering inherits that resolution rather than repeating it by name.
                    .or_else(|| {
                        let v = self.enumerators.get(sym).copied()?;
                        let t = self.out.enumerator_ty(*sym)?;
                        self.out.enum_refs.insert(expr, (v, t));
                        Some(t)
                    })
                    .unwrap_or_else(|| {
                        // An undeclared name is reported **once per name**, not once per
                        // use. Contract 20's rule is about the ratio: a name used forty
                        // times is one mistake, and forty copies of the same complaint
                        // bury the thirty-nine other things that went wrong. The type is
                        // poison either way, so every use after the first says nothing.
                        let n = self.text(*sym).unwrap_or("?").to_owned();
                        // **A compiler builtin is declared by the compiler.** `stdarg.h`
                        // is `#define va_start(v,l) __builtin_va_start(v,l)` and nothing
                        // declares the target — gcc knows it intrinsically. Reporting it
                        // undeclared made **every variadic function in C** a sema error,
                        // which 015 §7 then turns into refusing the whole function.
                        //
                        // Typed as `Ty::Error` still: 020 §4.4.1 lowers these to `VaStart`
                        // / `VaArg` / `VaEnd` instructions from the *call*, not from the
                        // callee's type, so the type is never consulted. What matters is
                        // that it is not a diagnostic.
                        if !n.starts_with("__builtin_") && self.unknown_names.insert(*sym) {
                            self.error(span, format!("`{n}` was not declared"));
                        }
                        self.intern(Ty::Error)
                    });
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.type_expr(*lhs);
                let b = self.type_expr(*rhs);
                self.type_binary(
                    expr,
                    *op,
                    BinSides {
                        a,
                        b,
                        ae: *lhs,
                        be: *rhs,
                    },
                    span,
                )
            }
            ExprKind::Unary { op, operand } => {
                let inner = self.type_expr(*operand);
                let ity = self.out.typed.ty_of(inner);
                let (ty, inner) = match op {
                    UnOp::AddrOf => (self.intern(Ty::Ptr(ity)), inner),
                    UnOp::Deref => {
                        let decayed = self.decay(inner, *operand);
                        let dty = self.out.typed.ty_of(decayed);
                        let pointee = match self.out.types[dty.0 as usize].clone() {
                            Ty::Ptr(p) => p,
                            _ => self.intern(Ty::Error),
                        };
                        (pointee, decayed)
                    }
                    UnOp::Not => (
                        self.intern(Ty::Int {
                            signed: true,
                            bits: int_bits,
                        }),
                        inner,
                    ),
                    _ => {
                        let promoted = self.promote_node(inner, *operand, span);
                        (self.out.typed.ty_of(promoted), promoted)
                    }
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![inner],
                })
            }
            ExprKind::Postfix { operand, .. } => {
                let inner = self.type_expr(*operand);
                let ty = self.out.typed.ty_of(inner);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![inner],
                })
            }
            ExprKind::Assign { op, lhs, rhs } => {
                let l = self.type_expr(*lhs);
                let lty = self.out.typed.ty_of(l);
                let r = self.type_expr(*rhs);
                // **`p += n` does not convert `n` to a pointer.** C11 6.5.16.2p1: for `+=`
                // and `-=` with a pointer lvalue, the right operand stays an integer and
                // the whole thing means `p = p + n`, which counts in elements.
                //
                // Coercing it produced an `IntToPtr` on the literal, so lowering received a
                // pointer where the element count belonged and had no way to recover the
                // count. Every other compound operator is arithmetic on both sides and does
                // want the coercion.
                let pointer_displacement = matches!(op, Some(BinOp::Add) | Some(BinOp::Sub))
                    && matches!(self.out.ty(lty), Ty::Ptr(_) | Ty::Array { .. });
                // **`b += e` with `b` a `_Bool` does not convert `e` to `_Bool`.** C11
                // 6.5.16.2p3 makes it mean `b = b + e`, and 6.5p4 promotes both operands — so
                // the addition happens in `int` and only the *result* is converted back.
                //
                // Coercing the right operand first is invisible for every other integer type,
                // which is why it stood for a hundred waves: conversion to `char` is a
                // truncation, and truncation commutes with `+`, `-` and `*`, so
                // `(char)(1 + 300)` and `1 + (char)300` are both 45. Conversion to `_Bool` is
                // `!= 0`, which commutes with nothing — `-1` became `1` here and `b += -1`
                // stopped depending on `b` at all.
                //
                // Same shape as the pointer case above and wave 133's fix: sema coercing a
                // compound assignment's right operand to the lvalue's type when the operation
                // does not call for it. The *result* is still converted, by the store.
                let bool_lvalue =
                    op.is_some() && matches!(self.out.ty(lty), Ty::Int { bits: 1, .. });
                let r = if pointer_displacement {
                    r
                } else if bool_lvalue {
                    // **Promoted, not coerced.** The operation happens in `int`, so the right
                    // operand takes the integer promotions like any other arithmetic operand —
                    // which is also what makes the widths match: lowering widens the loaded
                    // `_Bool` to `int`, and a right operand left at one bit meets it as
                    // "Add operand is Int(1), declared Int(32)". Dropping the conversion
                    // altogether was the first attempt and produced exactly that, on seven of
                    // the two hundred generated programs.
                    self.promote_node(r, *rhs, span)
                } else {
                    self.coerce(r, lty, Conversion::Assignment, *rhs)
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty: lty,
                    operands: vec![l, r],
                })
            }
            ExprKind::Cond { cond, then, els } => {
                let c = self.type_expr(*cond);
                let t = then.map(|t| self.type_expr(t));
                let e = self.type_expr(*els);
                let ety = self.out.typed.ty_of(e);
                let ty = match t {
                    Some(t) => {
                        let tty = self.out.typed.ty_of(t);
                        self.common_type(tty, ety)
                    }
                    None => ety,
                };
                let mut ops = vec![c];
                if let (Some(t), Some(te)) = (t, then.as_ref()) {
                    let t = self.coerce(t, ty, Conversion::UsualArithmetic, *te);
                    ops.push(t);
                }
                let e = self.coerce(e, ty, Conversion::UsualArithmetic, *els);
                ops.push(e);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: ops,
                })
            }
            ExprKind::Comma { lhs, rhs } => {
                let a = self.type_expr(*lhs);
                let b = self.type_expr(*rhs);
                let ty = self.out.typed.ty_of(b);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![a, b],
                })
            }
            ExprKind::Index { base, index } => {
                let b = self.type_expr(*base);
                let b = self.decay(b, *base);
                let i = self.type_expr(*index);
                let i = self.promote_node(i, *index, span);
                let bty = self.out.typed.ty_of(b);
                let ty = match self.out.types[bty.0 as usize].clone() {
                    Ty::Ptr(p) => p,
                    _ => self.intern(Ty::Error),
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![b, i],
                })
            }
            ExprKind::Member { base, field, .. } => {
                let b = self.type_expr(*base);
                let b = self.decay(b, *base);
                let bty = self.out.typed.ty_of(b);
                let rec = match self.out.types[bty.0 as usize].clone() {
                    Ty::Record(r) => Some(r),
                    Ty::Ptr(p) => match self.out.types[p.0 as usize].clone() {
                        Ty::Record(r) => Some(r),
                        _ => None,
                    },
                    _ => None,
                };
                let ty = rec
                    .and_then(|r| {
                        self.out.records.get(r.0 as usize).and_then(|l| {
                            l.fields
                                .iter()
                                .find(|f| f.name == Some(*field))
                                .map(|f| f.ty)
                        })
                    })
                    .unwrap_or_else(|| self.intern(Ty::Error));
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![b],
                })
            }
            ExprKind::Call { callee, args } => {
                let c = self.type_expr(*callee);
                let c = self.decay(c, *callee);
                let cty = self.out.typed.ty_of(c);
                let (ret, params) = match self.out.types[cty.0 as usize].clone() {
                    Ty::Func { ret, params, .. } => (ret, params),
                    Ty::Ptr(p) => match self.out.types[p.0 as usize].clone() {
                        Ty::Func { ret, params, .. } => (ret, params),
                        _ => (self.intern(Ty::Error), Vec::new()),
                    },
                    _ => (self.intern(Ty::Error), Vec::new()),
                };
                let mut ops = vec![c];
                for (i, a) in args.iter().enumerate() {
                    let node = self.type_expr(*a);
                    let node = match params.get(i) {
                        // A declared parameter: convert to **its** type, not to the
                        // promoted one. `f(long)` called with a `char` must receive a
                        // `long`, and stopping at `int` is the bug this pins.
                        Some(&p) => self.coerce(node, p, Conversion::Argument, *a),
                        // Variadic or unprototyped: the default argument promotions.
                        None => self.promote_node(node, *a, span),
                    };
                    ops.push(node);
                }
                self.push_typed(TypedNode::Value {
                    expr,
                    ty: ret,
                    operands: ops,
                })
            }
            ExprKind::Cast { ty, operand } => {
                let inner = self.type_expr(*operand);
                let inner = self.decay(inner, *operand);
                let t = self.ty_of(*ty);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty: t,
                    operands: vec![inner],
                })
            }
            // **The operand of `sizeof` is typed, though it is not evaluated.** C's
            // "unevaluated" is about side effects; its *type* is the whole answer. Leaving
            // it untyped meant lowering had no type to take a size from, so `sizeof x`
            // lowered to `Undef` and every `chiero_make_symbolic(&x, sizeof x, …)` in the
            // corpus handed the intrinsic an unknown byte count.
            ExprKind::SizeofExpr(inner) => {
                let inner = *inner;
                self.type_expr(inner);
                let ty = self.intern(Ty::Int {
                    signed: false,
                    bits: (self.target.sizes.long_ * 8) as u32,
                });
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::SizeofType(_) | ExprKind::AlignofType(_) => {
                let ty = self.intern(Ty::Int {
                    signed: false,
                    bits: (self.target.sizes.long_ * 8) as u32,
                });
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::InitList(items) => {
                let ops = items
                    .iter()
                    .map(|i| self.type_expr(i.value))
                    .collect::<Vec<_>>();
                let ty = self.intern(Ty::Error);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: ops,
                })
            }
            // **A statement expression's type is its last expression statement's.** 013 §5
            // keeps the block as a `StmtId`, so the type is found by walking to the last
            // statement rather than by any rule of its own — and the statements inside are
            // typed here too, or every expression in 217 VPP files' worth of `({ ... })`
            // would be absent from the typed AST.
            ExprKind::StmtExpr(body) => {
                let body = *body;
                self.type_stmt(body);
                let ty = self
                    .last_value_of_block(body)
                    .and_then(|e| self.analysis_top_ty(e))
                    .unwrap_or_else(|| self.intern(Ty::Void));
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
            ExprKind::TypeName(_) | ExprKind::Error => {
                let ty = self.intern(Ty::Error);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: Vec::new(),
                })
            }
        };
        self.set_top(expr, id)
    }

    /// C11 §6.3.2.1: an array becomes a pointer to its first element and a function
    /// becomes a pointer to itself. Both are explicit `Cast`s, and they are **different**
    /// conversions because 015 emits different code for them.
    fn decay(&mut self, node: TypedId, expr: ExprId) -> TypedId {
        let ty = self.out.typed.ty_of(node);
        let span = self.ast.expr(expr).span;
        match self.out.types[ty.0 as usize].clone() {
            Ty::Array { elem, .. } => {
                let p = self.intern(Ty::Ptr(elem));
                let id = self.convert(node, p, Conversion::ArrayDecay, span);
                self.set_top(expr, id)
            }
            Ty::Func { .. } => {
                let p = self.intern(Ty::Ptr(ty));
                let id = self.convert(node, p, Conversion::FunctionDecay, span);
                self.set_top(expr, id)
            }
            _ => node,
        }
    }

    /// C11 §6.3.1.1's integer promotions, as an explicit conversion.
    fn promote_node(&mut self, node: TypedId, expr: ExprId, span: Span) -> TypedId {
        let node = self.decay(node, expr);
        let ty = self.out.typed.ty_of(node);
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        let Ty::Int { bits, .. } = self.out.types[ty.0 as usize] else {
            return node;
        };
        if bits >= int_bits {
            return node;
        }
        let to = self.intern(Ty::Int {
            signed: true,
            bits: int_bits,
        });
        let id = self.convert(node, to, Conversion::IntegerPromotion, span);
        self.set_top(expr, id)
    }

    /// The operands of one binary expression: their typed nodes and the syntactic ids
    /// they came from, which conversions have to be recorded against.
    fn type_binary(&mut self, expr: ExprId, op: BinOp, sides: BinSides, span: Span) -> TypedId {
        let BinSides { a, b, ae, be } = sides;
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        let int_ty = self.intern(Ty::Int {
            signed: true,
            bits: int_bits,
        });
        // Shifts do **not** take the usual arithmetic conversions: each operand is
        // promoted on its own and the result has the left operand's type. Applying the
        // common type here would silently widen `x << 1` to whatever the shift count is.
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            let a = self.promote_node(a, ae, span);
            let b = self.promote_node(b, be, span);
            let ty = self.out.typed.ty_of(a);
            return self.push_typed(TypedNode::Value {
                expr,
                ty,
                operands: vec![a, b],
            });
        }
        if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
            let a = self.decay(a, ae);
            let b = self.decay(b, be);
            return self.push_typed(TypedNode::Value {
                expr,
                ty: int_ty,
                operands: vec![a, b],
            });
        }

        let a = self.promote_node(a, ae, span);
        let b = self.promote_node(b, be, span);
        let aty = self.out.typed.ty_of(a);
        let bty = self.out.typed.ty_of(b);

        // Pointer arithmetic and comparisons keep their operands as they are: `p + n` has
        // no common type, and forcing one would turn a pointer into an integer.
        let is_ptr = |cx: &Cx, t: TyId| matches!(cx.out.types[t.0 as usize], Ty::Ptr(_));
        if is_ptr(self, aty) || is_ptr(self, bty) {
            let ty = match op {
                BinOp::Sub if is_ptr(self, aty) && is_ptr(self, bty) => self.intern(Ty::Int {
                    signed: true,
                    bits: (self.target.sizes.long_ * 8) as u32,
                }),
                BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne => int_ty,
                _ if is_ptr(self, aty) => aty,
                _ => bty,
            };
            // A null constant **compared** against a pointer becomes that pointer type, so
            // `p == 0` does not look like a pointer/integer mismatch downstream.
            //
            // **Comparisons only**, which is what this comment always said and what the
            // code did not do. C11 6.5.6 makes the other operand of `+` or `-` on a pointer
            // an *integer*, and 6.3.2.3's null-constant rule is about assignment and
            // comparison contexts — `p + 0` is neither. Converting there turned the `0`
            // into a pointer, and lowering then tried to sign-extend a `Ptr` to 64 bits for
            // the scaled offset: `inttoptr i32 0 to ptr`, then `zext i32 %v to i64`. The
            // verifier rejects that and nothing runs it at lowering time, so the function
            // was emitted and every `*(&a[i] + 0)` produced no state at all.
            let comparison = matches!(
                op,
                BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
            );
            let (a, b) = if !comparison {
                (a, b)
            } else if is_ptr(self, aty) && !is_ptr(self, bty) && self.is_null_constant(be) {
                let b = self.convert(b, aty, Conversion::NullPointer, span);
                let b = self.set_top(be, b);
                (a, b)
            } else if is_ptr(self, bty) && !is_ptr(self, aty) && self.is_null_constant(ae) {
                let a = self.convert(a, bty, Conversion::NullPointer, span);
                let a = self.set_top(ae, a);
                (a, b)
            } else {
                (a, b)
            };
            return self.push_typed(TypedNode::Value {
                expr,
                ty,
                operands: vec![a, b],
            });
        }

        let common = self.common_type(aty, bty);
        let a = self.coerce(a, common, Conversion::UsualArithmetic, ae);
        let b = self.coerce(b, common, Conversion::UsualArithmetic, be);
        let ty = match op {
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne => int_ty,
            _ => common,
        };
        self.push_typed(TypedNode::Value {
            expr,
            ty,
            operands: vec![a, b],
        })
    }

    /// Convert `node` to `to`, decaying first and recording the outermost node for
    /// `expr` so `conversions_of` sees the whole chain.
    fn coerce(&mut self, node: TypedId, to: TyId, why: Conversion, expr: ExprId) -> TypedId {
        let node = self.decay(node, expr);
        let span = self.ast.expr(expr).span;
        // A null pointer constant assigned to a pointer is `NullPointer`, not
        // `Assignment` — 015 lowers it to a constant rather than to a conversion, and
        // `(void*)0` and `(int)0` are different things to lower.
        let why =
            if matches!(self.out.types[to.0 as usize], Ty::Ptr(_)) && self.is_null_constant(expr) {
                Conversion::NullPointer
            } else {
                why
            };
        let id = self.convert(node, to, why, span);
        self.set_top(expr, id)
    }

    /// C11 §6.3.2.3: an integer constant expression with value 0 is a null pointer
    /// constant. Checked on the **written** expression, because `0` and a variable that
    /// happens to hold zero are different things.
    fn is_null_constant(&mut self, expr: ExprId) -> bool {
        matches!(
            self.ast.expr(expr).kind,
            ExprKind::Number(_) | ExprKind::Cast { .. }
        ) && self.eval(expr).map(|v| v.v) == Some(0)
    }

    /// C11 §6.3.1.8's usual arithmetic conversions.
    fn common_type(&mut self, a: TyId, b: TyId) -> TyId {
        if a == b {
            return a;
        }
        let (ta, tb) = (
            self.out.types[a.0 as usize].clone(),
            self.out.types[b.0 as usize].clone(),
        );
        match (&ta, &tb) {
            (Ty::Error, _) | (_, Ty::Error) => self.intern(Ty::Error),
            // Floating wins over integer, and the wider float wins.
            (Ty::Float(x), Ty::Float(y)) => {
                let k = if float_rank(*x) >= float_rank(*y) {
                    *x
                } else {
                    *y
                };
                self.intern(Ty::Float(k))
            }
            (Ty::Float(_), _) => a,
            (_, Ty::Float(_)) => b,
            (
                Ty::Int {
                    signed: sa,
                    bits: ba,
                },
                Ty::Int {
                    signed: sb,
                    bits: bb,
                },
            ) => {
                let (sa, ba, sb, bb) = (*sa, *ba, *sb, *bb);
                if ba == bb {
                    // Equal width: unsigned wins.
                    self.intern(Ty::Int {
                        signed: sa && sb,
                        bits: ba,
                    })
                } else {
                    let (ws, wb, ns) = if ba > bb { (sa, ba, sb) } else { (sb, bb, sa) };
                    // A wider signed type represents every value of a narrower unsigned
                    // one, so it stays signed.
                    let _ = ns;
                    self.intern(Ty::Int {
                        signed: ws,
                        bits: wb,
                    })
                }
            }
            _ => a,
        }
    }
}

/// One binary expression's operands, grouped so `type_binary` takes four things rather
/// than eight positional ones — the syntactic id always travels with its typed node.
struct BinSides {
    a: TypedId,
    b: TypedId,
    ae: ExprId,
    be: ExprId,
}

fn float_rank(k: FloatKind) -> u8 {
    match k {
        FloatKind::Binary16 | FloatKind::BFloat16 => 0,
        FloatKind::F32 => 1,
        FloatKind::F64 => 2,
        FloatKind::X87_80 => 3,
        FloatKind::Binary128 => 4,
    }
}

impl Cx<'_> {
    /// **Contract 20's single diagnostic**, emitted at the *declaration*.
    ///
    /// An object of incomplete type has no size, which is a type error. Reporting it here
    /// and giving the name `Ty::Error` means every later use reads poison and says
    /// nothing — one bad declaration is one diagnostic no matter how many times the name
    /// appears, which is what keeps a single missing header from burying the real problem
    /// under a hundred copies.
    fn check_complete(&mut self, id: DeclId, ty: TyId) {
        if !matches!(self.out.types[ty.0 as usize], Ty::Error) {
            return;
        }
        let span = self.ast.decl(id).span;
        let name = match &self.ast.decl(id).kind {
            DeclKind::Var { name: Some(n), .. } => self.text(*n).unwrap_or("?").to_owned(),
            _ => "?".to_owned(),
        };
        self.error(
            span,
            format!("`{name}` has an incomplete or unknown type; its uses are not checked"),
        );
    }

    /// Walk a statement, typing every expression in it.
    fn type_stmt(&mut self, stmt: StmtId) {
        let kind = self.ast.stmt(stmt).kind.clone();
        match kind {
            StmtKind::Expr(e) => {
                self.type_expr(e);
            }
            StmtKind::Decl(ds) => {
                for d in ds {
                    self.item(d);
                }
            }
            StmtKind::Compound(ss) => {
                for s in ss {
                    self.type_stmt(s);
                }
            }
            StmtKind::If { cond, then, els } => {
                let c = self.type_expr(cond);
                self.condition(c, cond);
                self.type_stmt(then);
                if let Some(e) = els {
                    self.type_stmt(e);
                }
            }
            StmtKind::While { cond, body } => {
                let c = self.type_expr(cond);
                self.condition(c, cond);
                self.type_stmt(body);
            }
            StmtKind::DoWhile { body, cond } => {
                self.type_stmt(body);
                let c = self.type_expr(cond);
                self.condition(c, cond);
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                match init {
                    Some(ForInit::Decl(ds)) => {
                        for d in ds {
                            self.item(d);
                        }
                    }
                    Some(ForInit::Expr(e)) => {
                        self.type_expr(e);
                    }
                    None => {}
                }
                if let Some(c) = cond {
                    let n = self.type_expr(c);
                    self.condition(n, c);
                }
                if let Some(s) = step {
                    self.type_expr(s);
                }
                self.type_stmt(body);
            }
            StmtKind::Switch { cond, body } => {
                let c = self.type_expr(cond);
                self.promote_node(c, cond, self.ast.expr(cond).span);
                self.type_stmt(body);
            }
            StmtKind::Case { lo, hi, body } => {
                self.type_expr(lo);
                if let Some(h) = hi {
                    self.type_expr(h);
                }
                self.type_stmt(body);
            }
            StmtKind::Default { body } | StmtKind::Label { body, .. } => self.type_stmt(body),
            StmtKind::Return(Some(e)) => {
                let node = self.type_expr(e);
                // **A returned value is converted to the function's return type.**
                // `Conversion::Return` existed in the enum and nothing produced it, so
                // `char f(void) { return 300; }` returned 300 — the truncation C requires
                // happened nowhere, and 021 would report a value the program cannot
                // produce.
                if let Some(ret) = self.current_ret {
                    self.coerce(node, ret, Conversion::Return, e);
                }
            }
            StmtKind::GotoIndirect(e) => {
                self.type_expr(e);
            }
            StmtKind::Asm(a) => {
                for op in a.outputs.iter().chain(a.inputs.iter()) {
                    self.type_expr(op.expr);
                }
            }
            StmtKind::Return(None)
            | StmtKind::Goto(_)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Empty
            | StmtKind::Error => {}
        }
    }

    /// A controlling expression is compared against zero, so it decays but is not
    /// promoted to a common type with anything.
    fn condition(&mut self, node: TypedId, expr: ExprId) {
        let node = self.decay(node, expr);
        self.set_top(expr, node);
    }
}

impl Cx<'_> {
    /// An **address constant**: a named object plus a byte offset (014 §6, contract 17).
    ///
    /// Returns the designated object's type as well, because the offset has to be
    /// *scaled* by the element size at each step — `&arr[3]` is byte offset 12, and an
    /// unscaled 3 points three bytes into the first element, which no type check catches.
    fn addr_of(&mut self, expr: ExprId) -> Option<(String, i64, TyId)> {
        let node = self.ast.expr(expr).clone();
        match &node.kind {
            // `&x` — the address of an lvalue.
            ExprKind::Unary {
                op: UnOp::AddrOf,
                operand,
            } => self.designator(*operand),
            // An array name used as a value already *is* an address (it decays).
            ExprKind::Ident(sym) => {
                let ty = self.values.get(sym).copied()?;
                match self.out.types[ty.0 as usize].clone() {
                    Ty::Array { elem, .. } => Some((self.text(*sym)?.to_owned(), 0, elem)),
                    _ => None,
                }
            }
            // `addr + n` and `addr - n`, scaled by the pointee.
            ExprKind::Binary {
                op: op @ (BinOp::Add | BinOp::Sub),
                lhs,
                rhs,
            } => {
                let (base, off, elem) = self.addr_of(*lhs).or_else(|| self.addr_of(*rhs))?;
                let other = if self.addr_of(*lhs).is_some() {
                    *rhs
                } else {
                    *lhs
                };
                let n = self.eval(other)?.v as i64;
                let scale = size_of_ty(&self.out, &self.target, elem).unwrap_or(1) as i64;
                let delta = n.checked_mul(scale)?;
                Some((
                    base,
                    if matches!(op, BinOp::Add) {
                        off.checked_add(delta)?
                    } else {
                        off.checked_sub(delta)?
                    },
                    elem,
                ))
            }
            ExprKind::Cast { operand, .. } => self.addr_of(*operand),
            _ => None,
        }
    }

    /// The object an lvalue designates, as `(name, byte offset, type of the designated
    /// object)`.
    fn designator(&mut self, expr: ExprId) -> Option<(String, i64, TyId)> {
        let node = self.ast.expr(expr).clone();
        match &node.kind {
            ExprKind::Ident(sym) => {
                let ty = self.values.get(sym).copied()?;
                Some((self.text(*sym)?.to_owned(), 0, ty))
            }
            ExprKind::Index { base, index } => {
                let (name, off, ty) = self.designator(*base).or_else(|| self.addr_of(*base))?;
                let elem = match self.out.types[ty.0 as usize].clone() {
                    Ty::Array { elem, .. } | Ty::Ptr(elem) => elem,
                    _ => ty,
                };
                let n = self.eval(*index)?.v as i64;
                let scale = size_of_ty(&self.out, &self.target, elem).unwrap_or(1) as i64;
                Some((name, off.checked_add(n.checked_mul(scale)?)?, elem))
            }
            ExprKind::Member { base, field, arrow } => {
                let (name, off, ty) = if *arrow {
                    self.addr_of(*base)?
                } else {
                    self.designator(*base)?
                };
                let rec = match self.out.types[ty.0 as usize].clone() {
                    Ty::Record(r) => r,
                    _ => return None,
                };
                let f = self
                    .out
                    .records
                    .get(rec.0 as usize)?
                    .fields
                    .iter()
                    .find(|f| f.name == Some(*field))?;
                Some((name, off.checked_add(f.offset as i64)?, f.ty))
            }
            _ => None,
        }
    }
}
