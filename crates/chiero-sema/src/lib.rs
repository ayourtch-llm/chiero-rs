//! `chiero-sema` — name resolution, types, layout and constant evaluation.
//! See `docs/specs/014-semantics-and-types.md`.
//!
//! **Layout correctness is the load-bearing part.** Every symbolic memory offset in 021
//! derives from a struct layout computed here, so a one-byte error produces confident,
//! wrong answers throughout the entire system rather than a visible failure. That is why
//! 014 §7 validates layout differentially against the real compiler instead of against
//! hand-written expectations: the expectations are exactly what a layout bug corrupts.

use chiero_ast::{
    Ast, BinOp, DeclId, DeclKind, ExprId, ExprKind, ForInit, StmtId, StmtKind, Storage, TypeId,
    TypeKind, UnOp,
};
pub mod strlit;

mod builtins;

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
        /// Whether the parameters were **specified** — `(void)` and `(int)` yes, `()` no.
        ///
        /// Part of the type, so `int f()` and `int f(void)` intern to different `TyId`s. C treats
        /// them as opposites: `(void)` promises there are no parameters, so a call with one is an
        /// error, while `()` says nothing at all and no call to it can be wrong.
        ///
        /// A K&R identifier list is **not** prototyped. The names are there and the types are
        /// not, which is why `static int g(){...}` still accepts `g(1)`.
        prototyped: bool,
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

/// The qualifiers on a type: `const`, `volatile`, `restrict` (C 6.7.3).
///
/// **Held beside the type rather than inside it.** `Ty` could have gained a `Qualified { of, q }`
/// variant, and §9 budgeted 436 match sites for the audit that would need; the audit found the
/// number is 274 across two crates, and that *none of them want to see the qualifier* — a
/// qualified `int` is laid out, promoted, converted and lowered exactly like an `int`. So the
/// qualifier goes in a table parallel to `types`, the interning key becomes `(Ty, Qual)`, and
/// every existing `match self.out.types[t]` keeps seeing the unqualified shape it was written
/// for. What changes meaning is `TyId` equality, and that is **four sites**.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Qual {
    pub const_: bool,
    pub volatile_: bool,
    pub restrict_: bool,
}

impl Qual {
    pub const NONE: Qual = Qual {
        const_: false,
        volatile_: false,
        restrict_: false,
    };

    /// Whether `self` has every qualifier `other` has — C 6.5.16.1's "all the qualifiers of".
    ///
    /// `restrict` is **not** included. C 6.7.3.1 makes it a promise about aliasing rather than a
    /// property the assignment must preserve, and gcc accepts `int *p = rp;` from a
    /// `int *restrict rp`. Including it would reject correct code, which wave 303's rule ranks
    /// worse than missing incorrect code.
    fn covers(self, other: Qual) -> bool {
        (self.const_ || !other.const_) && (self.volatile_ || !other.volatile_)
    }

    fn is_none(self) -> bool {
        self == Qual::NONE
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FloatKind {
    /// gcc's `_Float32` and `_Float64` — **distinct types from `float` and `double`** though
    /// identical in width and representation (C 6.2.5, F.10).
    ///
    /// They exist as separate variants for one reason: type *identity*. `_Generic(0.0f32,
    /// float: …)` selects `default` in gcc, and the interning key is the kind, so `float` and
    /// `_Float32` must be different kinds to be different `TyId`s. Everything else about them —
    /// size, alignment, the value a literal denotes, the CIR they lower to — is the standard
    /// type's, and `chiero-cir` has no matching variants because it does not need any: it
    /// describes *representations*, and these two share theirs.
    Float32Ext,
    Float64Ext,
    /// gcc's `_Float32x` and `_Float64x` — the "at least this wide, and wider than the unsuffixed
    /// one" forms, so `_Float32x` has `double`'s width and `_Float64x` has `long double`'s.
    ///
    /// Distinct from *everything*, which is why they need variants of their own rather than
    /// sharing `Float64Ext` and `X87_80`: gcc's `_Generic` separates `_Float32x` from `double`
    /// **and** from `_Float64`.
    Float32xExt,
    Float64xExt,
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
    /// **False while the tag has been named but not yet defined.** A reference to such a tag
    /// still gets a real `RecordId`, so the definition — which may come later in the file, or be
    /// the very definition whose members are being laid out — completes *this* record rather than
    /// a second one. Without it, a tag referenced before its definition was frozen as `Ty::Error`
    /// and `struct Node { struct Node *next; }` could not be walked past the first hop.
    pub complete: bool,
    /// Index into `fields` of a flexible array member, if any.
    pub flexible_member: Option<usize>,
    pub packed: bool,
    /// `__attribute__((transparent_union))`: a parameter of this union type accepts any
    /// member's type (gcc's extension). glibc declares `bind`/`connect`/`sendto` this way, so
    /// every socket-calling translation unit depends on it.
    pub transparent: bool,
    /// Whether the record declares a **zero-width bit-field**, whose effect on the layout
    /// `fields` cannot show.
    ///
    /// A `:0` declares no member and forces the *next* allocation to a unit boundary, so it
    /// pushes no `FieldLayout` — it cannot, because C 6.7.9 has initializers skip unnamed
    /// bit-fields and the initializer check indexes `fields` positionally. Its effect
    /// therefore survives only as a gap in its neighbours' offsets, indistinguishable from
    /// ordinary alignment padding.
    ///
    /// That distinction is the whole question for a consumer proposing a reorder
    /// ([041 §3.1](../../../docs/specs/041-optimization-analysis.md)): ordinary padding comes
    /// back and this gap does not, because the boundary follows the run wherever it is moved.
    /// So the layout says the record has one rather than leaving a reader to infer it from an
    /// absence.
    pub has_zero_width_bitfield: bool,
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
    /// Whether this diagnostic means chiero **could not**, or merely has something to say.
    ///
    /// **The distinction is the difference between refusing a file and annotating it.** Every
    /// consumer used to see one undifferentiated list and treat any entry as a refusal, which
    /// is right for "this type is unknown" and wrong for "this constant expression overflows
    /// a signed type" — chiero folds that one to the same value gcc does, and then the CLI
    /// threw the whole translation unit away. gcc compiles that file with exit 0.
    ///
    /// Defaults to [`Severity::Error`] at every construction site, so adding this changed no
    /// existing behaviour; only the sites explicitly marked [`Severity::Advisory`] moved.
    pub severity: Severity,
}

/// What a [`SemaDiagnostic`] means for the analysis that produced it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Severity {
    /// chiero could not do something. The result, if any, is not to be trusted.
    #[default]
    Error,
    /// chiero did the thing and has a concern about the program. **A usable result exists**,
    /// and a consumer that discards it is discarding an analysis it asked for.
    ///
    /// Reserved for diagnostics emitted *beside a value*, and each site says why it qualifies.
    ///
    /// Two classes carry it today. **Wrapped arithmetic** — `Cx::wrap` returns the value gcc
    /// and clang return and remarks that the program relied on undefined behaviour. And
    /// **ISO conformance remarks**, every one of which is a rule where `gnu11` is silent and
    /// `-pedantic-errors` speaks: the construct is supported unconditionally and only the
    /// sentence follows the dialect, so the analysis around it is complete. `is_error()` said
    /// otherwise for all six, and `is_error()` is what a consumer asks before discarding an
    /// analysis — `chiero-diff` runs the strict dialect today.
    Advisory,
}

impl SemaDiagnostic {
    /// Whether this diagnostic invalidates the analysis around it.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
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
    /// The common type of a conditional operator's two arms (C 6.5.15p6).
    ///
    /// A separate context because the arm is otherwise shared with assignment, and borrowing its
    /// words made `1 ? a : b` report "initializing or assigning from an incompatible pointer
    /// type" — which is neither. gcc names the construct.
    Conditional,
    /// Conversion to a parameter's declared type.
    Argument,
    /// Conversion to the function's return type.
    Return,
}

impl Conversion {
    /// Every variant, so a test can ask whether the engine ever produces each one.
    ///
    /// `Conversion` is stored in the typed AST and handed out by `conversions_of`, which makes
    /// each variant a promise to a consumer that this engine can tell that case apart. Written
    /// out here rather than derived because the list is the point: it has to change when the
    /// enum does, and a gate that enumerated only what it expected would pass by agreeing with
    /// itself.
    pub const ALL: &'static [Conversion] = &[
        Conversion::IntegerPromotion,
        Conversion::UsualArithmetic,
        Conversion::ArrayDecay,
        Conversion::FunctionDecay,
        Conversion::NullPointer,
        Conversion::Assignment,
        Conversion::Conditional,
        Conversion::Argument,
        Conversion::Return,
    ];
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
    /// The qualifiers of each `TyId`, parallel to `types`. See [`Qual`].
    pub(crate) quals: Vec<Qual>,
    /// Each `TyId` → the same type with its qualifiers removed, parallel to `types`.
    ///
    /// **Precomputed rather than re-interned on demand**, because the places that need it —
    /// assignment, comparison, the usual conversions — hold `&self` and interning needs `&mut`.
    /// A qualified type is always interned *after* its unqualified form, so the entry always
    /// exists by the time anything can ask.
    pub(crate) unqual: Vec<TyId>,
    /// Which **enumeration** each `TyId` is, parallel to `types`; 0 for a type that is not one.
    ///
    /// An enumeration is its integer type for every question but one: it is laid out, promoted,
    /// converted, compared and lowered as that type, so nothing here wants to *see* this number.
    /// What it changes is `TyId` equality, and C 6.7.2.3p5 needs it changed — each enumerator
    /// list declares a **distinct** type, so `enum E` and `enum F` are two types even though the
    /// sign that picks their integer type gives both the same one.
    ///
    /// Numbered per *definition* rather than per tag name, because two anonymous enumerations are
    /// two types and have no name to be numbered by.
    ///
    /// This is [`Qual`]'s trade, made a second time for the same reason and measured the same
    /// way: with the tag in the key and nothing else changed, one test in the workspace fails.
    pub(crate) enum_tags: Vec<u32>,
    /// Each `TyId` → the same type with no enumeration identity: its plain integer type.
    ///
    /// Precomputed for the same reason as `unqual` — compatibility holds `&self`.
    pub(crate) untagged: Vec<TyId>,
    pub(crate) interned: IndexMap<(Ty, Qual, u32), TyId>,
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
    /// Each `_Generic` expression → the association's value expression that its controlling
    /// type selected.
    ///
    /// **Keyed by the expression, like `enum_refs` above and for the same reason.** The
    /// choice is a question about types, so it is answered here, once, where the types are;
    /// lowering asks which arm *this* selection took rather than repeating the rule. C11
    /// 6.5.1.1 also says the unselected arms are not evaluated, and a recorded answer makes
    /// that automatic — lowering never sees them.
    pub(crate) generic_selections: IndexMap<ExprId, ExprId>,
    /// Each argument widened by `__attribute__((transparent_union))` → the member it became:
    /// `(index, name)`.
    ///
    /// **Recorded, not merely permitted.** gcc passes such an argument as the union's *first*
    /// member while the callee sees the union, so a later stage told only "this was allowed"
    /// knows neither which member the value is nor that a conversion happened at all. Keyed by
    /// the argument expression, like `enum_refs` and `generic_selections` beside it, and for
    /// the same reason: the question is about types, so it is answered once, here.
    pub(crate) transparent_union_args: IndexMap<ExprId, (usize, Symbol)>,
    /// Each conversion whose pointee alignment changed → `(from, to)`.
    ///
    /// **Compatibility ignores the attribute; the change is still information.** An increase
    /// is the hazard — a 1-aligned object reached through a pointer promising 16 — and the
    /// direction cannot be recovered downstream once compatibility has been decided. Not a
    /// diagnostic: gcc reports nothing here in any mode, so it belongs to a checker rather
    /// than to the dialect.
    pub(crate) pointee_alignment_changes: IndexMap<ExprId, (u64, u64)>,
    /// Each declaration → the alignment its declarator asked for, when it asked for more than
    /// the type's own.
    ///
    /// **Per *declaration*, not per type, because that is where C puts it.** `_Alignas(16) int
    /// x;` does not make a new type — `x` is still an `int` and still compatible with every
    /// other `int` — it makes *this object* over-aligned. `Ty` carries no alignment for a
    /// scalar and should not grow one: two `int`s that differ only in a declarator's request
    /// would become incompatible types.
    ///
    /// Keyed by `DeclId` like `decl_types` beside it, so lowering asks the same question of the
    /// same key when it sizes the slot.
    pub(crate) decl_aligns: IndexMap<DeclId, u64>,
    /// Each typedef name → the alignment it carries, for `typedef int A __attribute__((aligned
    /// (16)))`.
    ///
    /// A typedef *does* attach the alignment to the name, so `A x;` is over-aligned although
    /// `x`'s own declarator asks for nothing. Kept by name rather than by `TyId` because the
    /// interned type is plain `int` and must stay that way.
    pub(crate) typedef_aligns: IndexMap<Symbol, u64>,
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

    /// A member by name, **searching through anonymous members** (C11 6.7.2.1p13).
    ///
    /// An unnamed member whose type is a struct or union has *its* members treated as members
    /// of the containing type, so `s.a` may name something several records down. The returned
    /// `FieldLayout` is rebased onto `rec`: its `offset` is from the start of `rec`, not of the
    /// record that declared it, which is what makes it usable exactly like a direct member.
    ///
    /// **`bits.bit_offset` is rebased too, and that is the part worth stating.** It is
    /// documented as absolute — bits from the start of *the record* — so promoting a bit-field
    /// out of an anonymous struct has to add the anonymous member's byte offset in bits.
    /// Adjusting `offset` alone leaves a `BitRange` pointing into the wrong storage unit, and
    /// only a bit-field inside an anonymous struct that is not at offset zero can see it.
    ///
    /// **Named members win over anonymous ones**, and the search is depth-first in declaration
    /// order. C forbids the ambiguity that would make the order matter — two members of the same
    /// name reachable at one level — so this does not have to detect it to be right about every
    /// program C accepts.
    pub fn find_field(&self, rec: RecordId, name: Symbol) -> Option<FieldLayout> {
        let l = self.records.get(rec.0 as usize)?;
        if let Some(f) = l.fields.iter().find(|f| f.name == Some(name)) {
            return Some(f.clone());
        }
        for f in &l.fields {
            if f.name.is_some() {
                continue;
            }
            let Some(Ty::Record(inner)) = self.types.get(f.ty.0 as usize) else {
                continue;
            };
            if let Some(mut got) = self.find_field(*inner, name) {
                got.offset += f.offset;
                if let Some(b) = got.bits.as_mut() {
                    b.bit_offset += f.offset * 8;
                }
                return Some(got);
            }
        }
        None
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

    /// Conversions whose pointee alignment changed: `(expression, from, to)`. An increase is
    /// the hazardous direction.
    pub fn pointee_alignment_changes(&self) -> impl Iterator<Item = (ExprId, u64, u64)> + '_ {
        self.pointee_alignment_changes
            .iter()
            .map(|(e, (f, t))| (*e, *f, *t))
    }

    /// The transparent-union member an argument was widened to, if it was.
    pub fn transparent_union_arg(&self, e: ExprId) -> Option<(usize, Symbol)> {
        self.transparent_union_args.get(&e).copied()
    }

    /// Every widened argument: `(expression, member index, member symbol)`.
    pub fn transparent_union_args(&self) -> impl Iterator<Item = (ExprId, usize, Symbol)> + '_ {
        self.transparent_union_args
            .iter()
            .map(|(e, (i, n))| (*e, *i, *n))
    }

    /// The alignment `d`'s declarator asked for, if it asked for more than its type's own.
    /// The alignment a **typedef name** carries, for `typedef struct {…} T
    /// __attribute__((aligned(16)));`.
    ///
    /// **Not the record's** — C puts a post-declarator attribute on the name, so `_Alignof(T)`
    /// is 16 while the struct it names stays 8-aligned, and `sizeof(T)` is the struct's either
    /// way. A consumer comparing chiero's answer against `_Alignof` of the *name* must ask this
    /// as well as `RecordLayout::align`; conflating them is what the contract-12 gate did, and
    /// it reported a defect in a layout that was right.
    pub fn typedef_align(&self, name: Symbol) -> Option<u64> {
        self.typedef_aligns.get(&name).copied()
    }

    pub fn decl_align(&self, d: DeclId) -> Option<u64> {
        self.decl_aligns.get(&d).copied()
    }

    /// The association a `_Generic` selected, as the expression to lower in its place.
    pub fn generic_selection(&self, e: ExprId) -> Option<ExprId> {
        self.generic_selections.get(&e).copied()
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

    /// This type without its qualifiers (C 6.7.3), for consumers asking about representation.
    ///
    /// A qualifier changes what a program may *do* with an object, never how the object is laid
    /// out or converted — so anything reasoning about representation should ask this, and
    /// anything reasoning about assignment should not.
    pub fn unqualified(&self, t: TyId) -> TyId {
        self.unqual[t.0 as usize]
    }

    /// The qualifiers on this type.
    pub fn qualifiers(&self, t: TyId) -> Qual {
        self.quals[t.0 as usize]
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
        self.interned.get(&(Ty::Error, Qual::NONE, 0)).copied()
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
                        severity: Severity::Error,
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
    analyze_with(ast, target, names, chiero_ast::Dialect::pedantic())
}

/// As [`analyze`], in a chosen dialect.
///
/// **Only rules measured to differ between `gnu11` and `-pedantic-errors` may consult it.** A
/// constraint gcc refuses in both modes stays refused, or the dialect becomes a way to hide
/// defects rather than to match a project's compiler.
pub fn analyze_with(
    ast: &Ast,
    target: &TargetConfig,
    names: &dyn SymbolText,
    dialect: chiero_ast::Dialect,
) -> Analysis {
    let mut cx = Cx {
        next_severity: Severity::Error,
        dialect,
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
        quiet: 0,
        in_typeof: 0,
        next_enum_tag: 0,
        enumerators: IndexMap::new(),
        in_progress: Vec::new(),
        current_ret: None,
        current_fn: None,
        read_only: Default::default(),
        read_only_pointee: Default::default(),
        register_objects: Default::default(),
        meanings: Default::default(),
        body_scope_open: false,
        automatic_objects: Default::default(),
        loop_depth: 0,
        breakable_depth: 0,
        switches: Vec::new(),
        labels_defined: Default::default(),
        labels_used: Vec::new(),
        declaring: None,
        defined_tags: Default::default(),
        declared_enumerators: Default::default(),
        open_vla_scopes: Vec::new(),
        switch_vla_depth: Vec::new(),
        next_vla_scope: 0,
        label_scopes: Default::default(),
        prior: Default::default(),
        values: ScopedTypes::default(),
        unknown_names: Default::default(),
        defined_with_init: Default::default(),
    };
    // **C 6.9p1: a translation unit contains at least one external declaration.** `gnu11`
    // accepts an empty one and `-pedantic-errors` refuses it, so this follows the dialect. The
    // last 4 of the first strict sweep's 104 misses, all `vppinfra/test/` files whose whole
    // body sits behind an `#ifdef` that configuration leaves off.
    //
    // `items()` is the test, not "declares an object or a function": a `typedef` or a bare
    // `struct S { … };` *is* an external declaration and gcc accepts a unit holding only one.
    if cx.dialect.pedantic && ast.items().is_empty() {
        cx.advisory(Span::DUMMY, "ISO C forbids an empty translation unit");
    }
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
///
/// **Each call prepares a fresh context, which costs one walk of the whole translation
/// unit.** That is the right price for a caller asking about one expression and the wrong
/// one for a caller asking about thousands — lowering asks once per integer literal, and
/// paying F declarations for each of F bodies is the O(F²) that made a VPP translation
/// unit take 673 seconds. Such a caller wants [`ConstEvaluator`], which pays for the walk
/// once; this entry point stays because a `.cir` fixture or any caller with no
/// [`Analysis`] must still be able to fold a constant with nothing else in hand.
pub fn const_eval(
    ast: &Ast,
    expr: ExprId,
    names: &dyn SymbolText,
    target: &TargetConfig,
    out: &mut Vec<SemaDiagnostic>,
) -> Option<ConstVal> {
    ConstEvaluator::new(ast, names, target).eval(expr, out)
}

/// One translation unit, prepared once for **repeated** constant evaluation (014 §6).
///
/// The preparation [`const_eval`] does per call — a context in which `sizeof(int)`
/// resolves, and a walk of every declaration so that an address constant can be *about* a
/// declared object — depends only on the translation unit. Holding it across expressions
/// is what makes a caller with thousands of constants linear rather than quadratic in the
/// size of the file.
///
/// Two `eval` calls on the same expression answer the same thing, in either order, and
/// interleaving expressions from different functions is fine — in **values and
/// diagnostics** both. That is a property the type has to maintain rather than one it gets
/// for free: a context outliving the expression it was built for turns everything it
/// accumulates into a channel between evaluations, and `eval` rewinds the two that are
/// histories (the diagnostic list and the once-per-name report dedup). What remains is
/// declaration knowledge and resolved lookups, which are a cache.
pub struct ConstEvaluator<'a> {
    cx: Cx<'a>,
    /// How many names the *preparation* walk had already reported as undeclared.
    ///
    /// `Cx::unknown_names` reports each name once per context, which is right for one
    /// `const_eval` and wrong for a context that outlives the expression: the first `eval`
    /// would report `loc` and every later one — even for another expression in another
    /// function — would be silent. Rewinding to this mark before each `eval` is what makes a
    /// reused evaluator answer what a fresh one would. A length, not a copy of the set,
    /// because `IndexSet` keeps insertion order and `eval` only ever appends.
    prepared_unknown: usize,
}

// Hand-written because `Cx` holds a `&dyn SymbolText`, which cannot be derived through —
// and because a prepared context's interesting content is the tables, not the tree.
impl std::fmt::Debug for ConstEvaluator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConstEvaluator")
            .field("items", &self.cx.ast.items().len())
            .field("typedefs", &self.cx.typedefs.len())
            .field("tags", &self.cx.tags.len())
            .finish_non_exhaustive()
    }
}

impl<'a> ConstEvaluator<'a> {
    /// Prepare the translation unit. **This is the expensive call** — one pass over every
    /// declaration in `ast`.
    pub fn new(ast: &'a Ast, names: &'a dyn SymbolText, target: &TargetConfig) -> Self {
        let mut cx = Self::context(ast, names, target);
        // **The declarations are processed first.** An address constant is *about* a
        // declared object — `&arr[3]` needs `arr`'s element size to scale the offset —
        // and `sizeof` needs the tag table. Their diagnostics are then discarded, because
        // the caller asked about one expression and complaints about the surrounding file
        // are not an answer.
        for &item in ast.items() {
            cx.item(item);
        }
        cx.out.diagnostics.clear();
        let prepared_unknown = cx.unknown_names.len();
        ConstEvaluator {
            cx,
            prepared_unknown,
        }
    }

    /// Fold one expression, appending any diagnostics *it* produced to `out`.
    pub fn eval(&mut self, expr: ExprId, out: &mut Vec<SemaDiagnostic>) -> Option<ConstVal> {
        // Cleared, not drained: anything still here belongs to an earlier expression, and
        // reporting it again against this one would make a caller's diagnostics depend on
        // how many constants it had already folded.
        self.cx.out.diagnostics.clear();
        // And the once-per-name dedup is rewound for the same reason, in the other
        // direction: without this an earlier `eval` *suppresses* a later one's diagnostic.
        self.cx.unknown_names.truncate(self.prepared_unknown);
        let v = self.cx.eval(expr);
        out.append(&mut self.cx.out.diagnostics);
        match v {
            Some(v) => Some(ConstVal::Int(v.v)),
            // Not an integer constant: it may still be an **address** constant, which 014
            // §6 requires because `&arr[3]` and `(char*)&s + offsetof(S, f)` are valid
            // static initializers and fill VPP's node registration tables.
            None => self
                .cx
                .addr_of(expr)
                .map(|(global, off, _)| ConstVal::Addr { global, off }),
        }
    }

    fn context(ast: &'a Ast, names: &'a dyn SymbolText, target: &TargetConfig) -> Cx<'a> {
        // A throwaway context, so `sizeof(int)` resolves standalone. `sizeof(struct S)`
        // needs the TU's tag table and therefore needs `analyze`; that is a real limit and
        // is why this takes a target rather than pretending sizes are universal.
        Cx {
            next_severity: Severity::Error,
            // A const-eval helper: no dialect-gated rule is reachable from here, so the strict
            // default is the safe one.
            dialect: chiero_ast::Dialect::pedantic(),
            ast,
            target: target.clone(),
            names,
            out: Analysis::default(),
            typedefs: IndexMap::new(),
            tags: IndexMap::new(),
            enums: IndexMap::new(),
            quiet: 0,
            in_typeof: 0,
            next_enum_tag: 0,
            enumerators: IndexMap::new(),
            in_progress: Vec::new(),
            current_ret: None,
            current_fn: None,
            read_only: Default::default(),
            read_only_pointee: Default::default(),
            register_objects: Default::default(),
            meanings: Default::default(),
            body_scope_open: false,
            automatic_objects: Default::default(),
            loop_depth: 0,
            breakable_depth: 0,
            switches: Vec::new(),
            labels_defined: Default::default(),
            labels_used: Vec::new(),
            declaring: None,
            defined_tags: Default::default(),
            declared_enumerators: Default::default(),
            open_vla_scopes: Vec::new(),
            switch_vla_depth: Vec::new(),
            next_vla_scope: 0,
            label_scopes: Default::default(),
            prior: Default::default(),
            values: ScopedTypes::default(),
            unknown_names: Default::default(),
            defined_with_init: Default::default(),
        }
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
    /// Severity for the **next** diagnostic only; reset to `Error` after every emission.
    /// See [`Cx::advisory`].
    next_severity: Severity,
    ast: &'a Ast,
    target: TargetConfig,
    names: &'a dyn SymbolText,
    out: Analysis,
    typedefs: IndexMap<Symbol, TyId>,
    tags: IndexMap<Symbol, RecordId>,
    /// Enum tag → its underlying integer type (014 contract 10).
    enums: IndexMap<Symbol, TyId>,
    /// Nesting depth of a `typeof` operand, which C does not evaluate.
    in_typeof: u32,
    /// Nesting depth of `re_resolving`; diagnostics are dropped while it is non-zero.
    dialect: chiero_ast::Dialect,
    quiet: u32,
    /// Numbered per enumeration *definition*, so two anonymous ones differ. 0 means "none".
    next_enum_tag: u32,
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
    /// The function whose body is being walked, for `__func__`.
    current_fn: Option<Symbol>,
    /// Objects declared `const` at their outermost level, so assigning to the *name* is an
    /// error. Scoped exactly like `values`: a block may shadow a `const` with a mutable object
    /// of the same name, which is why a non-const declaration **removes** as well as a const one
    /// inserting.
    read_only: indexmap::IndexSet<Symbol>,
    /// Objects whose *pointee* is `const`, so writing through them is a write to a read-only
    /// object. A **separate** set from `read_only`, because C separates the two: `int *const p`
    /// is in `read_only` and not here, `const int *p` is here and not in `read_only`, and one
    /// set for both gets one of them wrong whichever way it is written.
    read_only_pointee: indexmap::IndexSet<Symbol>,
    /// Objects declared `register`, whose address may not be taken. Scoped like `read_only`, and
    /// for the same reason: a block may shadow one with an ordinary object.
    register_objects: indexmap::IndexSet<Symbol>,
    /// What each ordinary identifier in scope means (C 6.2.3), for the rule that a name denotes
    /// one thing per scope.
    meanings: ScopedMeanings,
    /// Set by the function-definition arm just before it walks the body, so the body's own
    /// `Compound` does not open a *second* meanings scope over the parameters' one.
    body_scope_open: bool,
    /// Objects with **automatic** storage duration, so `&x` in a static initializer is not an
    /// address constant. Scoped like `read_only` and `register_objects`, and for the same
    /// reason — a block-scope `static y` may shadow an automatic `y`, and `&y` then *is*
    /// constant. Recording the automatic ones rather than the static ones is what makes an
    /// unknown name (a typo, a poisoned declaration) default to *not* reported.
    automatic_objects: indexmap::IndexSet<Symbol>,
    /// How many *loops* enclose the statement being walked, and how many loops-or-switches.
    ///
    /// Two counters rather than one, because `break` and `continue` do not agree about what a
    /// `switch` is: `break` leaves it, `continue` looks past it to the enclosing loop. A single
    /// depth would accept `continue` in a switch that no loop encloses, and there is no way to
    /// recover the distinction afterwards.
    loop_depth: usize,
    breakable_depth: usize,
    /// One frame per `switch` currently open: the case values seen, and whether a `default` has
    /// been. A stack rather than a set, because a nested switch starts a fresh set and a sibling
    /// switch may legally repeat every value of the one before it.
    /// Per open `switch`: the **closed intervals** its labels occupy, and whether a `default` has
    /// been seen.
    ///
    /// **Intervals, not a set of values.** `case 1 ... 3` is a GNU range this engine supports, and
    /// a set can only record one number for it — the lower bound, which is what wave 319 did, so
    /// `case 2:` beside it collided with nothing. A range may span more values than are worth
    /// enumerating (`case 0 ... 1000000`), so the interval is stored and intersection is asked
    /// rather than membership.
    ///
    /// A `Vec` rather than a map: switches are short, the test is against every entry anyway, and
    /// source order makes the diagnostics deterministic (001 §5).
    switches: Vec<(Vec<(i128, i128)>, bool)>,
    /// The labels this function defines, and the `goto`s that named one. Collected for the whole
    /// function and checked at the end: a forward `goto` names a label declared later, so nothing
    /// can be decided at the point of use.
    labels_defined: indexmap::IndexSet<Symbol>,
    labels_used: Vec<(Symbol, Span, Vec<u32>)>,
    /// The variably-modified scopes open at the point being walked, innermost last.
    ///
    /// A scope opens at a VLA's declaration and closes with its block, which is why this is a
    /// stack truncated on the way out of a compound statement rather than a set: C 6.8.6.1 is
    /// about whether a jump *crosses* a declaration, and the same block can open one after a
    /// label and not before it.
    /// The declarator a type is currently being built for, when there is one.
    ///
    /// **A side channel, and deliberately so.** `ty_of` walks a type node and structurally does
    /// not know the declarator — the same node serves a `sizeof`, a cast and a parameter, none of
    /// which has a name. Threading an `Option<Symbol>` through every `ty_of` call site to reach
    /// two diagnostics would cost more than it buys; setting it around the one call that *does*
    /// have a name is one line, and it names the array in every type-level message rather than
    /// only in the one this wave was reading.
    declaring: Option<Symbol>,
    /// Tags *defined* — not merely declared — in each open scope. `struct S;` after a definition
    /// is how a forward declaration is written, so only a definition registers here.
    defined_tags: ScopedNames,
    /// Enumeration constants declared in each open scope. **Not per enum**: C 6.7.2.2 makes an
    /// enumerator an ordinary identifier, so two *different* enums in one scope may not share a
    /// name any more than one enum may repeat one.
    declared_enumerators: ScopedNames,
    open_vla_scopes: Vec<u32>,
    /// How many variably-modified scopes were open when each enclosing `switch` began.
    ///
    /// **A `case` label is a jump** (C 6.8.6.1p1): control reaches it from the `switch`, skipping
    /// whatever lies between. Wave 341 built this check for `goto`, where the destination is
    /// named and the comparison is between two label positions; here the origin is the `switch`
    /// itself, so what has to be remembered is the depth *there*.
    switch_vla_depth: Vec<usize>,
    next_vla_scope: u32,
    /// The scopes open where each label sits. A `goto` is illegal when this is not contained in
    /// the scopes open at the `goto`.
    label_scopes: indexmap::IndexMap<Symbol, Vec<u32>>,
    /// Every file-scope declaration seen so far, for comparing the next one against it.
    prior: indexmap::IndexMap<Symbol, Prior>,
    /// Ordinary identifiers in scope → their type. C's five namespaces are separate
    /// (014 §4), and this is the one expressions read.
    values: ScopedTypes,
    /// Names already reported as undeclared, so the complaint is per name and not per
    /// use — contract 20.
    unknown_names: indexmap::IndexSet<Symbol>,
    /// File-scope names that already have an *initialized* definition — contract 14.
    defined_with_init: indexmap::IndexSet<Symbol>,
}

/// Whether a type has no size — **including `void`**, which [`is_incomplete`] deliberately
/// excludes.
///
/// That exclusion is right for the callers it was written for: `void *p` is an ordinary pointer,
/// `void f(void)` an ordinary function, and this engine defines `sizeof(void)` as 1 the way GNU C
/// does. But three contexts need a size and do not care *why* one is missing — an array's element,
/// a record's member, and a definition's return type — and each of those had to decide about
/// `void` separately. Two of them had not, so `struct I a[3];` was caught and `void a[3];` was not.
///
/// One predicate for "needs a size", so a fourth such context inherits the answer instead of
/// repeating the omission.
fn has_no_size(out: &Analysis, ty: TyId) -> bool {
    is_incomplete(out, ty) || matches!(out.types[ty.0 as usize], Ty::Void)
}

/// The linkage a redeclaration resolves to, given the one already established.
///
/// C 6.2.2p4: `static` is internal; a plain file-scope declaration is external; and `extern`
/// takes whatever the prior declaration had. Only the last of those needs `was` at all, and it
/// is the whole reason this is a function rather than a field.
/// What one of the atomic builtins returns (014 §6, measured against gcc 13.3.0).
///
/// These are 46 of the 82 names `builtins.rs` deliberately omits: a row there records a constant
/// return type, and "the pointee of operand 1" is not one. They are answered here, per call,
/// beside `__builtin_shuffle` — including the members whose result *is* constant, because
/// splitting one family across two mechanisms is how the next reader gets it wrong.
///
/// Measured by passing the call to `void take(struct Z)` and reading the type back out of gcc's
/// diagnostic, exactly as `builtins.rs` documents; the `void` group is the one that answers
/// "invalid use of void expression" instead.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum AtomicResult {
    /// The unqualified pointee of operand 1.
    Pointee,
    Bool,
    Void,
}

fn atomic_result(name: &str) -> Option<AtomicResult> {
    use AtomicResult::*;
    Some(match name {
        "__atomic_load_n"
        | "__atomic_exchange_n"
        | "__atomic_add_fetch"
        | "__atomic_sub_fetch"
        | "__atomic_and_fetch"
        | "__atomic_or_fetch"
        | "__atomic_xor_fetch"
        | "__atomic_nand_fetch"
        | "__atomic_fetch_add"
        | "__atomic_fetch_sub"
        | "__atomic_fetch_and"
        | "__atomic_fetch_or"
        | "__atomic_fetch_xor"
        | "__atomic_fetch_nand"
        | "__sync_lock_test_and_set"
        | "__sync_val_compare_and_swap"
        | "__sync_add_and_fetch"
        | "__sync_sub_and_fetch"
        | "__sync_and_and_fetch"
        | "__sync_or_and_fetch"
        | "__sync_xor_and_fetch"
        | "__sync_nand_and_fetch"
        | "__sync_fetch_and_add"
        | "__sync_fetch_and_sub"
        | "__sync_fetch_and_and"
        | "__sync_fetch_and_or"
        | "__sync_fetch_and_xor"
        | "__sync_fetch_and_nand" => Pointee,
        "__atomic_compare_exchange"
        | "__atomic_compare_exchange_n"
        | "__atomic_test_and_set"
        | "__atomic_is_lock_free"
        | "__sync_bool_compare_and_swap" => Bool,
        "__atomic_store"
        | "__atomic_store_n"
        | "__atomic_load"
        | "__atomic_exchange"
        | "__atomic_clear"
        | "__atomic_thread_fence"
        | "__atomic_signal_fence"
        | "__sync_lock_release"
        | "__sync_synchronize" => Void,
        _ => return None,
    })
}

/// Whether a name is one gcc declares intrinsically, with no header.
///
/// **Three families, because gcc has three.** The exemption began as `__builtin_` alone, for the
/// reason recorded at its only caller: `stdarg.h` is `#define va_start(v,l) __builtin_va_start(v,l)`
/// and nothing declares the target, so reporting it undeclared made every variadic function a
/// sema error. `__atomic_*` and `__sync_*` are declared the same way — gcc compiles
/// `__atomic_load_n` and `__sync_fetch_and_add` with no header — and were simply not named.
///
/// **Prefix match, deliberately exact.** `atomic_load_n` without the leading underscores and
/// `__atomi_load` are ordinary undeclared names, and a looser test would swallow the typo along
/// with the builtin — turning a missing diagnostic into a wrong answer.
fn is_compiler_builtin(name: &str) -> bool {
    name.starts_with("__builtin_") || name.starts_with("__atomic_") || name.starts_with("__sync_")
}

fn resolved_linkage(was: Prior, now: Prior) -> bool {
    if now.internal {
        true
    } else if now.deferring {
        was.internal
    } else {
        false
    }
}

/// What a previous file-scope declaration of a name committed to.
#[derive(Copy, Clone)]
struct Prior {
    ty: TyId,
    /// A function with a body, or an object without `extern` — the things C calls definitions.
    defined: bool,
    /// `static`. Internal linkage; the mirror of it is *explicitly* external.
    internal: bool,
    /// Written `extern`, which claims nothing about linkage and so conflicts with nothing.
    deferring: bool,
}

/// Where a declaration appears. Contract 14's redefinition rule is a *file-scope* rule, and the
/// two scopes reach the same handler, so the distinction has to be carried rather than deduced.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Scope {
    File,
    Block,
}

/// Where a declaration sits, for the storage-class table (C 6.7.1p3 and its three neighbours).
///
/// **Finer than [`Scope`], and deliberately separate from it.** `Scope` answers contract 14's
/// redefinition question and has exactly two answers; this one has five, because a `for`
/// initializer and a parameter are block-ish for linkage and not for storage. Widening `Scope`
/// would have made every existing `scope == Scope::Block` a question about the wrong thing.
#[derive(Copy, Clone, PartialEq, Eq)]
enum StorageContext {
    File,
    Block,
    ForInit,
    Parameter,
    Function,
    /// A `typedef`, and a member of a structure or union. Neither takes a storage class *or* a
    /// function specifier; the storage side of a `typedef` is counted separately (6.7.1p2 lets
    /// `_Thread_local` accompany `static` there and not here), so this context exists for the
    /// specifier half, which is the same in both places.
    NotAnObject,
}

/// Names declared in the scope being walked, for the rules that ask "again, *here*?".
///
/// **A stack with marks, not a depth counter.** Two sibling blocks are two different scopes at the
/// same depth — `{ struct S {int a;}; } { struct S {int b;}; }` is legal C — so a rule keyed on
/// depth reports a redefinition for code gcc accepts. The mark is where the current scope's names
/// begin; leaving the scope truncates back to it, which is what makes a sibling start empty.
///
/// Wave 326's rule applies and was written into the fixture in the same edit: a scoped set's
/// *removal* is unfalsifiable until something reuses the name, so the sibling case exists here
/// before the code did.
#[derive(Default)]
struct ScopedNames {
    names: Vec<(Symbol, u8)>,
    marks: Vec<usize>,
}

impl ScopedNames {
    fn enter(&mut self) {
        self.marks.push(self.names.len());
    }

    fn leave(&mut self) {
        let mark = self.marks.pop().unwrap_or(0);
        self.names.truncate(mark);
    }

    /// Whether `name` is already declared **in the innermost scope**, and record it either way.
    fn redeclares(&mut self, name: Symbol) -> bool {
        self.redeclares_as(name, 0).is_some()
    }

    /// The same, remembering **what kind** the name was declared as.
    ///
    /// A tag reused as a different kind is not a redefinition — `union U { … }; struct U { … };`
    /// says "redefinition of `struct U`" of something that was never a struct. Telling the two
    /// apart needs the previous kind, and returning it is the smallest way to have it: the
    /// caller compares and chooses its sentence.
    fn redeclares_as(&mut self, name: Symbol, kind: u8) -> Option<u8> {
        let start = self.marks.last().copied().unwrap_or(0);
        let before = self.names[start..]
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, k)| *k);
        self.names.push((name, kind));
        before
    }
}

/// What an **ordinary identifier** denotes (C 6.2.3): the namespace holding objects, functions,
/// typedef names and enumeration constants — but not tags, and not labels.
///
/// A function and an object share `Ordinary` on purpose. C separates them, and so does gcc's
/// message, but the *disagreement* between `int x;` and `int x(void);` is already reported by the
/// type-conflict check; adding a second sentence here would be contract 20's cascade for no gain.
/// What this enum has to tell apart is the cases nothing else asks about.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Meaning {
    /// A `typedef` name, carrying the type it names — repeating one with the *same* type is
    /// legal C and with a different type is not, so the kind alone is not enough.
    Typedef(TyId),
    Ordinary,
    Enumerator,
}

/// Ordinary identifiers in the scopes being walked, with what each one means.
///
/// The same shape as [`ScopedNames`] and deliberately not merged with it: that one answers
/// "again, here?" for tags, which have their own namespace, and a shared table would make
/// `struct S { int a; }; int S;` a redeclaration. Four census rows turn on exactly that.
/// Object names to types, scoped like `ScopedMeanings` beside it.
///
/// **A flat map was the bug.** An inner block's declaration overwrote the outer entry and
/// nothing restored it, so `uword r; if (..) { vlib_process_restore_t r = {..}; } return r;`
/// resolved the `return` to the struct. C 6.2.1p4 ends the inner declaration with its block.
/// That shape reached 867 of 879 findings in a full VPP sweep through one header.
#[derive(Debug, Default)]
struct ScopedTypes {
    entries: Vec<(Symbol, TyId)>,
    marks: Vec<usize>,
}

impl ScopedTypes {
    fn enter(&mut self) {
        self.marks.push(self.entries.len());
    }

    fn leave(&mut self) {
        let mark = self.marks.pop().unwrap_or(0);
        self.entries.truncate(mark);
    }

    fn insert(&mut self, name: Symbol, ty: TyId) {
        self.entries.push((name, ty));
    }

    /// Innermost wins, so a shadow is found before the name it shadows.
    fn get(&self, name: &Symbol) -> Option<&TyId> {
        self.entries
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t)
    }
}

#[derive(Default)]
struct ScopedMeanings {
    names: Vec<(Symbol, Meaning)>,
    marks: Vec<usize>,
}

impl ScopedMeanings {
    fn enter(&mut self) {
        self.marks.push(self.names.len());
    }

    fn leave(&mut self) {
        let mark = self.marks.pop().unwrap_or(0);
        self.names.truncate(mark);
    }

    /// Whether this is the outermost scope. File scope admits repeated declarations of an object
    /// — `int x; int x;` is two tentative definitions (C 6.9.2p2) and is how every header is
    /// written — and a block does not.
    fn at_file_scope(&self) -> bool {
        self.marks.is_empty()
    }

    /// What `name` already means **in the innermost scope**, and record the new meaning.
    ///
    /// Records unconditionally, including on a collision: the declaration exists whatever this
    /// says about it, and leaving it out would make a third declaration of the same name report
    /// against the first rather than the second.
    fn declare(&mut self, name: Symbol, meaning: Meaning) -> Option<Meaning> {
        let start = self.marks.last().copied().unwrap_or(0);
        let was = self.names[start..]
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, m)| *m);
        self.names.push((name, meaning));
        was
    }
}

impl Cx<'_> {
    /// **An ordinary identifier means one thing per scope** (C 6.7p3).
    ///
    /// The four answers, in the order they are asked:
    ///
    /// - Nothing was declared: nothing to say.
    /// - Two `typedef`s naming the same type: legal, and common — a header included twice through
    ///   different paths lands here.
    /// - Two `Ordinary` declarations: legal at **file scope** and not in a block. The type
    ///   question is somebody else's, and answering it here would double the report.
    /// - Anything else: the name means two different things, which is the case nothing else in
    ///   this file was asking about.
    fn declare_ordinary(&mut self, name: Symbol, meaning: Meaning, span: Span) {
        self.declare_ordinary_linked(name, meaning, span, false);
    }

    /// The same, told whether this declaration's identifier has **linkage**.
    ///
    /// C 6.7p3 restricts "no more than one declaration ... with the same scope" to identifiers
    /// with **no** linkage. A block-scope function declaration has external linkage (6.2.2p5)
    /// and so does an `extern` object, so repeating either is legal — `l2_api.c` writes
    /// `extern int vnet_l2_patch_add_del (…);` and then the same declaration without `extern`,
    /// which gcc accepts silently and this rule was refusing.
    fn declare_ordinary_linked(
        &mut self,
        name: Symbol,
        meaning: Meaning,
        span: Span,
        has_linkage: bool,
    ) {
        let file_scope = self.meanings.at_file_scope() || has_linkage;
        let Some(was) = self.meanings.declare(name, meaning) else {
            return;
        };
        let text = self.text(name).unwrap_or("?").to_owned();
        match (was, meaning) {
            (Meaning::Typedef(a), Meaning::Typedef(b)) if a == b => {}
            (Meaning::Typedef(_), Meaning::Typedef(_)) => self.error(
                span,
                format!("`{text}` is redefined as a `typedef` for a different type"),
            ),
            (Meaning::Ordinary, Meaning::Ordinary) => {
                if !file_scope {
                    self.error(span, format!("`{text}` is declared twice in one block"));
                }
            }
            // Two enumerators are already `declared_enumerators`' business, and it names them
            // better than this would.
            (Meaning::Enumerator, Meaning::Enumerator) => {}
            _ => self.error(
                span,
                format!("`{text}` is declared as a different kind of thing"),
            ),
        }
    }

    fn intern(&mut self, ty: Ty) -> TyId {
        self.intern_qual(ty, Qual::NONE)
    }

    /// Intern a type carrying qualifiers.
    ///
    /// The unqualified form is interned **first and unconditionally**, so `unqual` is populated
    /// before anything holding only `&self` can look the qualified id up.
    fn intern_qual(&mut self, ty: Ty, q: Qual) -> TyId {
        self.intern_tagged(ty, q, 0)
    }

    /// Intern a type carrying qualifiers and an enumeration identity (`tag`, 0 for none).
    ///
    /// Both side tables follow `unqual`'s discipline: the simpler form is interned **first and
    /// unconditionally**, so an entry always exists before anything holding `&self` looks it up.
    fn intern_tagged(&mut self, ty: Ty, q: Qual, tag: u32) -> TyId {
        if let Some(&id) = self.out.interned.get(&(ty.clone(), q, tag)) {
            return id;
        }
        // **Both simpler forms are interned before this id is allocated**, and neither may be
        // written as `TyId(types.len())` computed early. That shorthand was correct while
        // `unqual` was the only side table — nothing could intern between reading the length and
        // pushing — and interning `plain` here is exactly such a something. Read early, `bare`
        // named the slot `plain` then took, so `bare(enum E)` answered `unsigned int` and every
        // comparison that goes through `bare` stopped seeing the enumeration: the parameter path,
        // which is how the function arm reaches a parameter's type.
        let bare = if q.is_none() {
            None
        } else {
            Some(self.intern_tagged(ty.clone(), Qual::NONE, tag))
        };
        let plain = if tag == 0 {
            None
        } else {
            Some(self.intern_qual(ty.clone(), q))
        };
        let id = TyId(self.out.types.len() as u32);
        let (bare, plain) = (bare.unwrap_or(id), plain.unwrap_or(id));
        self.out.types.push(ty.clone());
        self.out.quals.push(q);
        self.out.unqual.push(bare);
        self.out.enum_tags.push(tag);
        self.out.untagged.push(plain);
        self.out.interned.insert((ty, q, tag), id);
        id
    }

    /// Which enumeration `t` is, or 0.
    fn enum_tag(&self, t: TyId) -> u32 {
        self.out.enum_tags[t.0 as usize]
    }

    /// The same type with no enumeration identity — its plain integer type.
    fn untagged(&self, t: TyId) -> TyId {
        self.out.untagged[t.0 as usize]
    }

    /// The same type without its qualifiers.
    fn bare(&self, t: TyId) -> TyId {
        self.out.unqual[t.0 as usize]
    }

    fn qual_of(&self, t: TyId) -> Qual {
        self.out.quals[t.0 as usize]
    }

    /// `t` with `q` added to whatever it already carries.
    fn add_quals(&mut self, t: TyId, q: Qual) -> TyId {
        if q.is_none() {
            return t;
        }
        let had = self.qual_of(t);
        let merged = Qual {
            const_: had.const_ || q.const_,
            volatile_: had.volatile_ || q.volatile_,
            restrict_: had.restrict_ || q.restrict_,
        };
        if merged == had {
            return t;
        }
        let ty = self.out.types[t.0 as usize].clone();
        self.intern_qual(ty, merged)
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        // **Silent while re-resolving something already resolved.** See `re_resolving`.
        if self.quiet > 0 {
            return;
        }
        self.out.diagnostics.push(SemaDiagnostic {
            span,
            message: message.into(),
            severity: self.next_severity,
        });
        self.next_severity = Severity::Error;
    }

    /// Emit an **advisory**: a concern about the program, beside a value chiero did produce.
    ///
    /// Threaded through a one-shot field rather than a second `error`-shaped method because
    /// `error` is called from 160 places and every one of them would have had to choose.
    /// The default is [`Severity::Error`], so a site that says nothing keeps saying what it
    /// always said.
    fn advisory(&mut self, span: Span, message: impl Into<String>) {
        self.next_severity = Severity::Advisory;
        self.error(span, message);
    }

    /// Run `f` with diagnostics suppressed, for a **second** resolution of a node the first pass
    /// already reported on.
    ///
    /// A parameter list is resolved up to three times — once for the function's type, once to
    /// record `decl_types`, and once more when a body is walked — and every diagnostic raised
    /// inside `ty_of` came out once per pass. The passes cannot simply be removed: a tag or an
    /// enumerator declared in the list is visible in the body (C 6.9.1p9), and that visibility is
    /// established by re-resolving. So the side effects still run and only the reporting stops.
    ///
    /// **Suppression, not de-duplication.** Two parameters wrong the same way are two mistakes
    /// and must still produce two sentences, which a filter on message text would destroy.
    fn re_resolving<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.quiet += 1;
        let out = f(self);
        self.quiet -= 1;
        out
    }

    fn text(&self, sym: Symbol) -> Option<&str> {
        self.names.text(sym)
    }

    /// Whether `callee` names `__builtin_offsetof`.
    /// Type the **subscript expressions** inside an `offsetof` member designator, and nothing
    /// else.
    ///
    /// `offsetof(T, a.b[i].c)` names members with `a`, `b` and `c` — identifiers that are not
    /// objects and must not be typed — while `i` is an ordinary expression that C evaluates.
    /// Typing the whole thing reports "`b` was not declared"; typing none of it leaves `i`
    /// without a typed node, which lowering reads as `undef`.
    fn type_designator_indices(&mut self, path: ExprId) {
        match self.ast.expr(path).kind.clone() {
            ExprKind::Member { base, .. } => self.type_designator_indices(base),
            ExprKind::Index { base, index } => {
                self.type_designator_indices(base);
                self.type_expr(index);
            }
            _ => {}
        }
    }

    fn is_offsetof(&self, callee: ExprId) -> bool {
        let ExprKind::Ident(n) = self.ast.expr(callee).kind else {
            return false;
        };
        self.text(n) == Some("__builtin_offsetof")
    }

    /// One step of a **member designator**, as `(byte offset from `root`, the type reached)`.
    ///
    /// C11 7.19 allows a member name, a `.` chain and `[...]` subscripts. The parser already
    /// builds exactly that shape — `n.y` is a `Member`, `v[2]` an `Index`, both rooted at an
    /// `Ident` — so this reads the tree it produced rather than adding a grammar. The root
    /// `Ident` is a *member of `root`*, not a name in scope, which is the whole reason typing
    /// the argument as an expression reported it undeclared.
    ///
    /// **Field lookup goes through `find_field`**, so a designator naming something inside an
    /// anonymous member resolves and is rebased for free (wave 279). A scan of `l.fields` here
    /// would work for every fixture that does not use one and quietly fail for the header this
    /// is for.
    fn offsetof_step(&mut self, root: TyId, e: ExprId) -> Option<(u64, TyId)> {
        match self.ast.expr(e).kind.clone() {
            ExprKind::Ident(name) => {
                let Ty::Record(r) = *self.out.ty(root) else {
                    return None;
                };
                let f = self.out.find_field(r, name)?;
                Some((f.offset, f.ty))
            }
            ExprKind::Member {
                base,
                field,
                arrow: false,
            } => {
                let (off, ty) = self.offsetof_step(root, base)?;
                let Ty::Record(r) = *self.out.ty(ty) else {
                    return None;
                };
                let f = self.out.find_field(r, field)?;
                Some((off.checked_add(f.offset)?, f.ty))
            }
            ExprKind::Index { base, index } => {
                let (off, ty) = self.offsetof_step(root, base)?;
                let elem = match *self.out.ty(ty) {
                    Ty::Array { elem, .. } => elem,
                    // A designator may subscript an array member and nothing else; `->` and a
                    // pointer are not designators at all.
                    _ => return None,
                };
                let k = self.eval(index)?.v;
                if k < 0 {
                    return None;
                }
                let esz = size_of_ty(&self.out, &self.target, elem)?;
                Some((off.checked_add((k as u64).checked_mul(esz)?)?, elem))
            }
            _ => None,
        }
    }

    /// Whether `callee` names one of 7.12.14's floating classification builtins.
    ///
    /// Only the ones lowering can express as a single CIR comparison. `isinf`, `isfinite`,
    /// `isnormal`, `signbit` and `fpclassify` are deliberately absent: they need more than a
    /// comparison, and 015 §7's refusal — loud, and naming the function it skipped — is the
    /// honest answer for a capability that is not there. Typing them `int` here while lowering
    /// still could not represent them would replace a declared limit with a wrong answer.
    fn is_fp_classify_builtin(&self, callee: ExprId) -> bool {
        let ExprKind::Ident(n) = self.ast.expr(callee).kind else {
            return false;
        };
        matches!(
            self.text(n),
            Some(
                "__builtin_isnan"
                    | "__builtin_isunordered"
                    | "__builtin_isless"
                    | "__builtin_islessequal"
                    | "__builtin_isgreater"
                    | "__builtin_isgreaterequal"
                    | "__builtin_islessgreater"
            )
        )
    }

    /// The result type of a **type-generic** builtin, resolved against this call's arguments.
    ///
    /// `builtins.rs` records a measured return type per name, which cannot express "the type of
    /// operand 1" — so the ten front-end special forms keep `Ty::Error`, and a value with no type
    /// lowers to a scalar `Int(32)` fallback. For a vector-returning one that is fatal rather
    /// than approximate: initializing a vector object from a scalar is a `Copy` with no address
    /// to copy from, the verifier rejects it, and 015 §7 discards the function. Every
    /// `_mm512_reduce_*` intrinsic is written with `__builtin_shuffle`, so this was 26 of the
    /// first 27 VPP translation units.
    ///
    /// **Measured against gcc 13.3.0**, by the method `builtins.rs` documents: the call is passed
    /// to `void take(struct Z)` and the return type is read out of gcc's own diagnostic.
    ///
    /// - `__builtin_shuffle(v, mask)` and `(a, b, mask)` are **the first argument's type**. The
    ///   mask contributes nothing: gcc rejects one whose lane count differs, and `v4sf` data with
    ///   a `v4si` mask is `v4sf`.
    /// - `__builtin_shufflevector(a, b, idx…)` is the first argument's **element** type with as
    ///   many lanes as there are indices — `(v4si, v4si, 0, 1)` is a two-lane vector.
    ///
    /// The 46 `__atomic_*`/`__sync_*` names are the same shape with a different rule — the
    /// *pointee* of operand 1, qualifiers stripped — and are the larger remaining gap. They are
    /// not here because they have not been measured to the standard this file requires.
    fn type_generic_builtin(
        &mut self,
        callee: ExprId,
        args: &[ExprId],
        first_arg: Option<TyId>,
        argc: usize,
    ) -> Option<TyId> {
        let ExprKind::Ident(n) = self.ast.expr(callee).kind else {
            return None;
        };
        // A *declared* function of that name is an ordinary call and must stay one, exactly as
        // the unmodeled-builtin arm in lowering requires.
        if self.values.get(&n).is_some() {
            return None;
        }
        // **The atomics are answered before `first_arg` is required**, because the `void` and
        // `_Bool` members of those families have a constant result and no operand to read it
        // from — `__sync_synchronize()` takes none at all.
        if let Some(kind) = atomic_result(self.text(n)?) {
            return match kind {
                AtomicResult::Void => Some(self.intern(Ty::Void)),
                AtomicResult::Bool => Some(self.intern(Ty::Int {
                    signed: false,
                    bits: 1,
                })),
                // **Unqualified**, which is the half a plain "pointee" rule gets wrong:
                // `__atomic_load_n` of a `const volatile unsigned long *` is an `unsigned long`,
                // and VPP's counters are declared exactly that way. An array argument is the
                // same pointer after decay, which is why it is answered here rather than left to
                // a caller that may not have decayed yet.
                AtomicResult::Pointee => match self.out.ty(first_arg?).clone() {
                    Ty::Ptr(inner) | Ty::Array { elem: inner, .. } => {
                        Some(self.out.unqualified(inner))
                    }
                    _ => None,
                },
            };
        }
        // **`va_arg`'s type is written down**, in an `ExprKind::TypeName` operand that denotes
        // no value and so types as `Ty::Error` — right for the node, and fatal for the call,
        // which then took `Error` from the undeclared builtin and lowered to a 32-bit fallback.
        // `va_arg(ap, int)` was the only correct case and it was correct by accident.
        if self.text(n)? == "__builtin_va_arg" {
            let &[_, tyarg] = args else { return None };
            let ExprKind::TypeName(t) = self.ast.expr(tyarg).kind else {
                return None;
            };
            return Some(self.ty_of(t));
        }
        let first = first_arg?;
        match self.text(n)? {
            "__builtin_shuffle" => Some(first),
            "__builtin_shufflevector" => {
                let Ty::Vector { elem, align, .. } = self.out.ty(first).clone() else {
                    return None;
                };
                // Two vector operands then one index each. Fewer than three arguments is not a
                // call gcc accepts, and inventing a zero-lane vector for it would be worse than
                // leaving the poison in place.
                let lanes = u32::try_from(argc.checked_sub(2)?).ok()?;
                if lanes == 0 {
                    return None;
                }
                // **The alignment is not the first argument's.** A narrower result is a narrower
                // type: `__builtin_shufflevector(v4si, v4si, 0, 1)` is eight bytes, and carrying
                // sixteen into it would misplace it in a struct. Width, as `apply_vector_size`
                // computes it for a declaration carrying no `aligned` attribute — and this one
                // carries none, because there is no declaration to write it on.
                let _ = align;
                let width = size_of_ty(&self.out, &self.target, elem)? * u64::from(lanes);
                Some(self.intern(Ty::Vector {
                    elem,
                    lanes,
                    align: width,
                }))
            }
            _ => None,
        }
    }

    /// A declaration at **file scope**. Contract 14's redefinition rule applies here and only
    /// here, which is why the scope is a parameter rather than something inferred.
    fn item(&mut self, id: DeclId) {
        self.decl(id, Scope::File);
    }

    /// A declaration inside a block. Nothing it names can redefine anything: a block-scope
    /// identifier has no linkage, so it is a different object from every other declaration of
    /// that name, in an enclosing scope or in another function.
    fn block_decl(&mut self, id: DeclId) {
        self.decl(id, Scope::Block);
    }

    fn decl(&mut self, id: DeclId, scope: Scope) {
        match self.ast.decl(id).kind.clone() {
            DeclKind::Var {
                name,
                ty,
                init,
                storage,
            } => {
                // **Saved and restored, not just set.** An initializer can contain a `sizeof`
                // over another type, and a nested declaration would otherwise inherit this name.
                let outer_declaring = std::mem::replace(&mut self.declaring, name);
                let t = self.ty_of(ty);
                self.declaring = outer_declaring;
                self.out.decl_types.insert(id, t);
                let span = self.ast.decl(id).span;
                self.check_storage_classes(storage, span);
                self.check_storage_context(
                    storage,
                    match scope {
                        Scope::File => StorageContext::File,
                        Scope::Block => StorageContext::Block,
                    },
                    span,
                );
                // **A variably-modified declarator needs automatic storage duration**
                // (C 6.7.6.2p2). Not a rule about *where* the declaration is: `int a[k]` with a
                // `const int k` is a VLA in a function body and at file scope alike — `const`
                // does not make a constant expression in C — and what decides it is the storage
                // duration. So file scope, `static` and `extern` are all illegal and a plain
                // block-scope local is not.
                //
                // A **parameter** never reaches here as an array: `int f(int a[k])` adjusts to a
                // pointer, so the length is evaluated and discarded and there is no object of
                // variably-modified type to place. That is why the rule needs no exemption for
                // one.
                if matches!(
                    self.out.types[t.0 as usize],
                    Ty::Array {
                        len: ArrayLen::Vla(_),
                        ..
                    }
                ) && Cx::has_static_storage(scope, &storage)
                {
                    let where_ = if scope == Scope::File {
                        "at file scope"
                    } else {
                        "with static storage duration"
                    };
                    self.error(span, format!("variably modified type {where_}"));
                }
                if let Some(a) = self.declared_align(ty) {
                    self.out.decl_aligns.insert(id, a);
                }
                if let Some(n) = name {
                    // **The *outermost* qualifier is the object's.** `const int k` and
                    // `int *const p` both make the named object read-only; `const int *p` makes
                    // the *pointee* read-only and leaves `p` assignable, and its const sits on an
                    // inner node where this does not see it. That asymmetry is the whole rule.
                    if self.ast.ty(ty).quals.const_ {
                        self.read_only.insert(n);
                    } else {
                        self.read_only.swap_remove(&n);
                    }
                    if self.points_to_const(ty) {
                        self.read_only_pointee.insert(n);
                    } else {
                        self.read_only_pointee.swap_remove(&n);
                    }
                    // **A variably-modified declaration opens a scope** that runs to the end of
                    // its block.
                    //
                    // Only at block scope, because only a block closes a scope. A parameter
                    // reaches here too — `int f(int n, int a[n])` is in the corpus — and its
                    // scope has no compound statement to end it, so an entry pushed for one
                    // would outlive the function and every function after it.
                    //
                    // **That guard is measured-unobserved**, and the reason is worth stating
                    // rather than papering over: a parameter's scope is open at *every* label
                    // and every `goto` in the body, so the containment below holds with or
                    // without the leak. Dropping it survives the suite. It is kept because the
                    // stack is only meaningful if it tracks block structure, not because any
                    // test can currently tell.
                    if scope == Scope::Block
                        && matches!(
                            self.out.types[t.0 as usize],
                            Ty::Array {
                                len: ArrayLen::Vla(_),
                                ..
                            }
                        )
                    {
                        let id = self.next_vla_scope;
                        self.next_vla_scope += 1;
                        self.open_vla_scopes.push(id);
                    }
                    if storage.register {
                        self.register_objects.insert(n);
                    } else {
                        self.register_objects.swap_remove(&n);
                    }
                    if Cx::has_static_storage(scope, &storage) {
                        self.automatic_objects.swap_remove(&n);
                    } else {
                        self.automatic_objects.insert(n);
                    }
                    self.declare_ordinary_linked(n, Meaning::Ordinary, span, storage.extern_);
                    self.check_alignment(
                        ty,
                        t,
                        if scope == Scope::File {
                            StorageContext::File
                        } else {
                            StorageContext::Block
                        },
                    );
                    self.values.insert(n, t);
                    // Contract 14. `int x; int x;` is two **tentative** definitions and is
                    // legal C11 §6.9.2 — it is how headers have always worked. Only a
                    // second *initialized* definition is an error, so the thing tracked
                    // is "has an initializer", not "has been seen".
                    if scope == Scope::File {
                        // An object is *defined* unless it says `extern`: `int x;` is a tentative
                        // definition and `extern int x;` is only a declaration (C 6.9.2). The
                        // repeat-tentative case is handled by the branch below, which is about
                        // initializers rather than linkage, so `defined` here is about linkage
                        // alone and an initializer is not consulted.
                        // **A function with no storage class defers, however it is spelled**
                        // (C 6.2.2p5: it is as if `extern`, so 6.2.2p4's adoption applies).
                        // `typedef int F(void); F f;` is a *function* declaration that reaches
                        // this arm because its declarator has no `()` — the function path below
                        // already says `deferring: !storage.static_` and this one said only
                        // `storage.extern_`, so the same program written through a typedef
                        // conflicted with a prior `static` definition. VPP writes exactly that:
                        // `format_function_t format_bihash_kvp_8_8;` after a `static inline` one.
                        //
                        // **Asked of the resolved type, not the declarator's shape** — the second
                        // time this project has been caught reading a spelling where C reads a
                        // type (wave 389's `void` parameter was the first).
                        //
                        // **No `&& !storage.static_` here**, though the first version had one.
                        // `resolved_linkage` reads `deferring` only when `!now.internal`, so the
                        // flag on a `static` declaration is never consulted and the extra clause
                        // could not change an answer. A mutant dropping it survived, and the
                        // reason is plain equivalence — not, as wave 401 recorded, a neighbouring
                        // missing rule. That claim rested on a misreading of the sweep and the
                        // rules concerned are all present and correct.
                        //
                        // An *object* is unchanged: 6.2.2p5 is about functions, so
                        // `static int f(void){…} int f;` stays a conflict.
                        let is_function = matches!(self.out.types[t.0 as usize], Ty::Func { .. });
                        let now = Prior {
                            ty: t,
                            defined: false,
                            internal: storage.static_,
                            deferring: storage.extern_ || is_function,
                        };
                        self.check_redeclaration(n, now, self.ast.decl(id).span);
                    }
                    if scope == Scope::Block {
                        // **A block-scope name redefines nothing.** It has no linkage, so it is a
                        // distinct object from every other declaration of that name — in an
                        // enclosing block, in another function, or at file scope. Applying the
                        // file-scope rule here made `int a = 0;` in two different functions a
                        // redefinition, which is most of C.
                    } else if init.is_some() && !self.ast.decl(id).span.ctx.is_root() {
                        // Macro-produced definitions are not compared: a header expanded
                        // twice is the preprocessor's business, not a redefinition.
                    } else if init.is_some() && !self.defined_with_init.insert(n) {
                        let text = self.text(n).unwrap_or("?").to_owned();
                        let span = self.ast.decl(id).span;
                        self.error(span, format!("`{text}` is defined more than once"));
                    }
                }
                self.check_complete(id, t);
                // **`extern` and an initializer do not go together in a block** (C 6.7.9p5). At
                // *file* scope `extern int x = 1;` is a definition and perfectly legal, so the
                // rule is about block scope rather than about `extern` — both halves are rows in
                // the fixture. Inside a function the declaration refers to an object defined
                // elsewhere, and there is nothing here to initialize.
                if scope == Scope::Block && storage.extern_ && init.is_some() {
                    let text = name.and_then(|n| self.text(n)).unwrap_or("?").to_owned();
                    let span = self.ast.decl(id).span;
                    self.error(
                        span,
                        format!("`{text}` has both `extern` and an initializer"),
                    );
                }
                // **C 6.7.9p22: an array declared without a length takes its initializer's.**
                // The length is not knowable where the *type* is built — `ty_of` sees the
                // declarator and not the initializer — so the type is rebuilt here, at the one
                // place that has both. Doing it by re-interning rather than by mutating keeps
                // types immutable, which every other consumer relies on.
                let t = match (init, self.out.types[t.0 as usize].clone()) {
                    (
                        Some(init),
                        Ty::Array {
                            elem,
                            len: ArrayLen::Flexible,
                        },
                    ) => match self.inferred_len(init) {
                        Some(n) => {
                            let t = self.intern(Ty::Array {
                                elem,
                                len: ArrayLen::Fixed(n),
                            });
                            self.out.decl_types.insert(id, t);
                            if let Some(n) = name {
                                self.values.insert(n, t);
                            }
                            t
                        }
                        None => t,
                    },
                    _ => t,
                };
                if let Some(init) = init {
                    self.check_init(t, Some(ty), init);
                    // **Static storage duration, not file scope.** A local initializer may be
                    // any expression; only an object the linker writes down needs a constant.
                    // Keying this on `Scope::File` — which it was until wave 358 — left every
                    // block-scope `static` unchecked, and the identical initializer that was
                    // refused one line outside a function was taken inside it.
                    if Cx::has_static_storage(scope, &storage) {
                        self.check_static_init(init);
                    }
                    let node = self.type_expr(init);
                    // 014 §5: the initializer arrives **as the declared type**, so
                    // lowering never has to work out what the assignment did.
                    //
                    // **A string initialising an array is not an assignment** (C 6.7.9p14 is its
                    // own rule), so it does not go through `assignable`: doing so compared the
                    // literal's decayed `char *` against the array and produced a complaint about
                    // a pointer nobody wrote. `check_init` above owns that case entirely.
                    let string_into_array =
                        matches!(self.ast.expr(init).kind, ExprKind::Str { .. })
                            && matches!(self.out.types[t.0 as usize], Ty::Array { .. });
                    if !string_into_array {
                        self.coerce(node, t, Conversion::Assignment, init);
                    }
                }
            }
            DeclKind::Typedef { name, ty, storage } => {
                // **`typedef` is a storage-class specifier**, so any *other* one beside it is a
                // violation (C 6.7.1p1) — including `_Thread_local`, which the object-side rule
                // exempts. 6.7.1p2 lets `_Thread_local` accompany `static` or `extern` and
                // nothing else, so `_Thread_local static int x;` is legal and
                // `typedef _Thread_local int T;` is not. Reusing `check_storage_classes`
                // unchanged would get exactly that case wrong, which is why this counts its own.
                if storage.extern_
                    || storage.static_
                    || storage.auto
                    || storage.register
                    || storage.thread_local
                {
                    self.error(
                        self.ast.decl(id).span,
                        "`typedef` cannot be combined with another storage class",
                    );
                } else {
                    // **The specifier half**, which the counting above deliberately does not
                    // cover. `else` because a `typedef` carrying both a stray storage class and
                    // an `inline` is one bad declaration (contract 20).
                    self.check_storage_context(
                        storage,
                        StorageContext::NotAnObject,
                        self.ast.decl(id).span,
                    );
                }
                // **A `typedef` names itself too**, and gcc agrees: `typedef int A[-1];` is
                // "size of array `A` is negative". Set for the same reason as on a parameter —
                // a typedef is often used far from where it was written, so the name is the
                // whole of what makes the report actionable.
                let outer = self.declaring.replace(name);
                let t = self.ty_of(ty);
                self.declaring = outer;
                self.check_alignment(ty, t, StorageContext::NotAnObject);
                self.declare_ordinary(name, Meaning::Typedef(t), self.ast.decl(id).span);
                // **A variably modified `typedef` opens a scope too** (C 6.8.6.1p1). `typedef int
                // T[n]` evaluates `n` once, where the declaration stands, so a jump past it skips
                // that evaluation exactly as a jump past `int a[n]` does — and this declares no
                // object, which is why the scope tracking beside the object arm never saw it.
                if scope == Scope::Block
                    && matches!(
                        self.out.types[t.0 as usize],
                        Ty::Array {
                            len: ArrayLen::Vla(_),
                            ..
                        }
                    )
                {
                    let id = self.next_vla_scope;
                    self.next_vla_scope += 1;
                    self.open_vla_scopes.push(id);
                }
                self.typedefs.insert(name, t);
                self.out.decl_types.insert(id, t);
                if let Some(a) = self.declared_align(ty) {
                    self.out.typedef_aligns.insert(name, a);
                }
            }
            DeclKind::Func {
                name,
                ty,
                body,
                storage,
            } => {
                // **A function declarator names itself too.** `int (*f(void))[-1];` is
                // "size of array `f` is negative" in gcc, and this arm was the third path — after
                // parameters and typedefs — that resolved a type without saying which declarator
                // it belonged to. Saved and restored like the others, because a parameter list
                // inside sets and clears its own.
                let outer = self.declaring.replace(name);
                let t = self.ty_of(ty);
                self.declaring = outer;
                self.out.decl_types.insert(id, t);
                // **A function is an ordinary identifier**, so `int f(void); typedef int f;`
                // collides. Two *declarations* of the same function do not — they land in the
                // `Ordinary`/`Ordinary` arm, which file scope permits, and any disagreement about
                // the type is `conflicting types for` f``'s business rather than this rule's.
                self.declare_ordinary_linked(name, Meaning::Ordinary, self.ast.decl(id).span, true);
                self.values.insert(name, t);
                // A function declaration carries storage classes too, and `static extern` is a
                // violation there for the same reason. `inline` is not counted, which is what
                // keeps `static inline` — the most common spelling in the corpus — legal.
                self.check_storage_classes(storage, self.ast.decl(id).span);
                self.check_storage_context(
                    storage,
                    StorageContext::Function,
                    self.ast.decl(id).span,
                );
                // **A *definition* returns a complete type** (C 6.9.1p3); a declaration need not.
                // `struct I f(void);` is legal — the type may be completed before anything calls
                // it — so this asks about `body` rather than about the type alone, and it is the
                // reason the check cannot live in `ty_of` beside the array and member ones.
                //
                // `void` is a return type and not a size question here, so `is_incomplete` is
                // the right predicate rather than `has_no_size`.
                if body.is_some()
                    && let Ty::Func { ret, .. } = self.out.types[t.0 as usize].clone()
                    && is_incomplete(&self.out, ret)
                {
                    self.error(
                        self.ast.decl(id).span,
                        "a function definition returns an incomplete type",
                    );
                }
                if scope == Scope::File {
                    let now = Prior {
                        ty: t,
                        defined: body.is_some(),
                        internal: storage.static_,
                        // **A function with no storage-class specifier is `extern`**
                        // (C 6.2.2p5), which an *object* is not: a plain `int x;` at file scope is
                        // a tentative definition with external linkage, so `static int x; int x;`
                        // conflicts while `static int f(void); int f(void);` does not — the
                        // second declaration adopts the internal linkage rather than contradicting
                        // it. This arm shared the object's `storage.extern_` and so reported
                        // three legal shapes.
                        //
                        // **The adoption runs one way only.** `int f(void); static int f(void);`
                        // is still an error: `static` states internal linkage outright and never
                        // defers, which is what `internal` above already says.
                        deferring: !storage.static_,
                    };
                    self.check_redeclaration(name, now, self.ast.decl(id).span);
                }
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
                // **In the list's own scope**, as in the `TypeKind::Func` arm: this loop
                // re-resolves the same parameter types, and a tag defined in the list would
                // otherwise be installed a second time in the *enclosing* scope and reported as
                // a redefinition of itself.
                self.defined_tags.enter();
                self.declared_enumerators.enter();
                for p in &params {
                    if let DeclKind::Var { ty: pty, .. } = self.ast.decl(*p).kind.clone() {
                        let t = self.re_resolving(|cx| cx.ty_of(pty));
                        let t = self.adjusted_param_ty(pty, t);
                        self.out.decl_types.insert(*p, t);
                    }
                }
                self.declared_enumerators.leave();
                self.defined_tags.leave();
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
                    self.values.enter();
                    // An `enum` declared inside the body is local to it, exactly as a
                    // parameter is, and was leaking into the rest of the TU.
                    let saved_enums = self.enumerators.clone();
                    let saved_ro = self.read_only.clone();
                    let saved_rop = self.read_only_pointee.clone();
                    let saved_reg = self.register_objects.clone();
                    let saved_auto = self.automatic_objects.clone();
                    // **One scope for the parameters and the body**, per C 6.9.1p9. Opened here
                    // rather than by the body's `Compound`, which is told to reuse it.
                    self.meanings.enter();
                    self.values.enter();
                    self.values.enter();
                    for p in params {
                        if let DeclKind::Var {
                            name: Some(pn),
                            ty: pty,
                            ..
                        } = self.ast.decl(p).kind.clone()
                        {
                            let t = self.re_resolving(|cx| cx.ty_of(pty));
                            let t = self.adjusted_param_ty(pty, t);
                            // **A parameter of a definition needs a size**: the function has to
                            // receive the object. A *declaration* may name an incomplete
                            // parameter type — `struct T; int s(struct T t);` is legal, since
                            // nothing is passed until there is a call — so this is checked here,
                            // in the arm that walks a body, and not where prototypes are typed.
                            if is_incomplete(&self.out, t)
                                || matches!(self.out.types[t.0 as usize], Ty::Void)
                            {
                                let n = self.text(pn).unwrap_or("?").to_owned();
                                let span = self.ast.decl(p).span;
                                self.error(span, format!("parameter `{n}` has an incomplete type"));
                            }
                            // The uniqueness of parameter names moved to the `TypeKind::Func`
                            // arm in wave 359, which every declarator passes through — here it
                            // only ever saw definitions. Reporting it in both places would be
                            // two sentences for one mistake.
                            if self.ast.ty(pty).quals.const_ {
                                self.read_only.insert(pn);
                            } else {
                                self.read_only.swap_remove(&pn);
                            }
                            if self.points_to_const(pty) {
                                self.read_only_pointee.insert(pn);
                            } else {
                                self.read_only_pointee.swap_remove(&pn);
                            }
                            // **A parameter has automatic storage duration**, so `&n` is not
                            // an address constant either. It reaches the set here rather than
                            // through the declaration arm because a parameter is part of the
                            // function's *type* and is never walked as an item.
                            self.automatic_objects.insert(pn);
                            // **Recorded, not checked.** Collisions *among* parameters are
                            // wave 359's rule, in the `TypeKind::Func` arm, and it names them
                            // better ("duplicate parameter `a`"). What this entry is for is a
                            // parameter colliding with a declaration in the *body*, which
                            // nothing else sees. Wave 353's contract-20 channel caught the
                            // double report on its own new row.
                            self.meanings.declare(pn, Meaning::Ordinary);
                            self.values.insert(pn, t);
                            self.out.decl_types.insert(p, t);
                        }
                    }
                    // The return type the body's `return` statements convert to, and the name
                    // `__func__` denotes inside it.
                    self.body_scope_open = true;
                    let saved_fn = self.current_fn;
                    self.current_fn = Some(name);
                    let saved_ret = self.current_ret;
                    self.current_ret = match self.out.types[t.0 as usize].clone() {
                        Ty::Func { ret, .. } => Some(ret),
                        _ => None,
                    };
                    let saved_defined = std::mem::take(&mut self.labels_defined);
                    let saved_used = std::mem::take(&mut self.labels_used);
                    let mut saved_label_scopes = std::mem::take(&mut self.label_scopes);
                    self.type_stmt(body);
                    // **Checked here, once the whole body has been walked.** A `goto` may name a
                    // label declared later — that is what a forward jump is — so nothing can be
                    // decided at the point of use. Labels are function-scoped, so the sets are
                    // per function, and the *restore* below is what enforces that: two functions
                    // may each define `lab`, and only the second may not `goto` the first's.
                    //
                    // `std::mem::take` rather than `clone` on the way in is **measured
                    // equivalent** — the restore already leaves the set empty for the next
                    // function, so nothing observes the clearing. It is kept because taking says
                    // what the code means, and because a stale set would be a silent wrong answer
                    // if a nested body ever reached here.
                    for (name, span, from) in std::mem::take(&mut self.labels_used) {
                        if !self.labels_defined.contains(&name) {
                            let n = self.text(name).unwrap_or("?").to_owned();
                            self.error(span, format!("label `{n}` used but not defined"));
                            continue;
                        }
                        // **A jump may not enter the scope of a variably-modified identifier**
                        // (C 6.8.6.1p1). The label's open scopes must all be open at the `goto`
                        // too; anything open there and not here is a scope the jump *enters*,
                        // skipping the declaration that gives the array its length.
                        //
                        // Checked at function end for the same reason the undefined-label check
                        // is: the label may be declared after the `goto` that names it.
                        if let Some(at) = self.label_scopes.get(&name)
                            && at.iter().any(|s| !from.contains(s))
                        {
                            let n = self.text(name).unwrap_or("?").to_owned();
                            self.error(
                                span,
                                format!(
                                    "jump to label `{n}` enters the scope of a \
                                     variably-modified declaration"
                                ),
                            );
                        }
                    }
                    self.labels_defined = saved_defined;
                    self.labels_used = saved_used;
                    self.label_scopes = std::mem::take(&mut saved_label_scopes);
                    self.current_ret = saved_ret;
                    self.current_fn = saved_fn;
                    // A parameter does not outlive its function; restoring rather than
                    // removing also undoes any shadowing the body introduced.
                    self.values.leave();
                    self.enumerators = saved_enums;
                    self.read_only = saved_ro;
                    self.read_only_pointee = saved_rop;
                    self.register_objects = saved_reg;
                    self.automatic_objects = saved_auto;
                    self.meanings.leave();
                    self.values.leave();
                    self.values.leave();
                }
            }
            DeclKind::TagDef { ty } => {
                let t = self.ty_of(ty);
                self.out.decl_types.insert(id, t);
                // **A declaration declares something** (C 6.7p2): a declarator, a tag, or the
                // members of an enumeration. The parser routes every declarator-less declaration
                // here, so `int;` and `struct { int m; };` arrive alongside the legitimate
                // `struct S { int m; };`.
                //
                // **The rule cannot be "no declarator", and it cannot be "anonymous" either.**
                // `enum { A = 1 };` has neither a declarator nor a tag and is perfectly legal,
                // because it declares its enumerators. So the question is asked of the *syntactic
                // form*: a tag with a name declares that tag, an enumeration declares its
                // constants, and everything else — a bare type, a nameless structure — declares
                // nothing at all.
                //
                // An anonymous struct or union *member* is not this. It shares the spelling
                // exactly and is legal C11, and it stays legal because members are laid out by
                // `lay_out` and never reach this arm.
                let declares = match self.ast.ty(ty).kind {
                    TypeKind::Tag { tag, name, .. } => {
                        tag == chiero_ast::TagKind::Enum || name.is_some()
                    }
                    _ => false,
                };
                if !declares {
                    self.error(self.ast.decl(id).span, "this declaration declares nothing");
                }
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
        let out = self.qualify(ty, out);
        self.out.syntactic_types.insert(ty, out);
        out
    }

    /// Attach a syntactic node's qualifiers to the type it produced.
    ///
    /// Applied **here rather than in `ty_of_inner`'s arms** so that it reaches every spelling with
    /// one rule, including a typedef: `typedef int *ip; const ip p;` qualifies the whole typedef'd
    /// type, making `p` an `int *const` and not a pointer to `const int`, which is the reading
    /// people expect and C does not use.
    ///
    /// **Qualifying an array qualifies its element** (C 6.7.3p9), and it must, because the array
    /// is only ever seen through its decay: `const int a[3]` has to give `&a[0]` the type
    /// `const int *` or `int *p = a;` looks fine. A qualifier left on the `Ty::Array` itself would
    /// be dropped by the decay and the rule would silently do nothing.
    fn qualify(&mut self, node: TypeId, t: TyId) -> TyId {
        let q = self.ast.ty(node).quals;
        let q = Qual {
            const_: q.const_,
            volatile_: q.volatile_,
            restrict_: q.restrict_,
        };
        if q.is_none() {
            return t;
        }
        // **C 6.7.3p2: `restrict` qualifies a pointer**, and nothing else. Asked here for the
        // same reason the qualifiers are applied here: this is the one place every spelling
        // passes, typedefs included, so `typedef int *P; P restrict p;` is legal and
        // `typedef int T; T restrict x;` is not without either being special-cased.
        //
        // Asked of the type the qualifier **actually attaches to**, which for an array is its
        // element — the line immediately below is what puts it there. That is what makes
        // `int *restrict a[2]` an array of restricted pointers and legal, while
        // `int restrict a[2]` is not. Poison is excused, as everywhere (contract 20).
        if q.restrict_ {
            let target = match self.out.types[t.0 as usize].clone() {
                Ty::Array { elem, .. } => elem,
                _ => t,
            };
            if !matches!(
                self.out.types[target.0 as usize],
                Ty::Ptr { .. } | Ty::Error
            ) {
                self.error(
                    self.ast.ty(node).span,
                    "`restrict` qualifies a pointer, and this is not one",
                );
            }
        }
        if let Ty::Array { elem, len } = self.out.types[t.0 as usize].clone() {
            let e = self.add_quals(elem, q);
            return self.intern(Ty::Array { elem: e, len });
        }
        self.add_quals(t, q)
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
                // **A GNU extension is supported *and* reported under the strict dialect.**
                // `gcc -pedantic-errors` refuses `__int128` ("ISO C does not support"), and
                // wave 314 calibrates the default dialect to that; 013's construct table calls
                // the type required at VPP scale, so support is unconditional and only the
                // sentence follows the dialect. 100 of the first strict sweep's 104 misses.
                if self.dialect.pedantic
                    && matches!(
                        b,
                        chiero_ast::Builtin::Int128 | chiero_ast::Builtin::UInt128
                    )
                {
                    self.advisory(node.span, "ISO C does not support `__int128` types");
                }
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
            TypeKind::Array { elem, len, .. } => {
                let e = self.ty_of(elem);
                // **An array needs its element's size, whatever its own length is.** Without one
                // there is no stride, so `a[1]` has no address — which is why this is an error
                // even for `extern struct I arr[];`, where the *array's* length is legitimately
                // unknown. The two unknowns are not the same unknown.
                if has_no_size(&self.out, e) {
                    self.error(node.span, "array has an incomplete element type");
                }
                // **An array's element may not be a function** (C 6.7.6.2p1). `int f[3](void);`
                // is nearly always a typo for `int (*f[3])(void);`, an array of function
                // *pointers*, which is legal and unaffected — the pointer is the element there.
                //
                // Asked about the *resolved* element type rather than the syntax, so it also
                // catches `typedef int F(void); F a[3];`, where nothing in the declarator looks
                // like a function.
                //
                // Separate from `has_no_size` above: a function type has no size either, but
                // "incomplete element type" sends a reader looking for a missing definition.
                if matches!(self.out.types[e.0 as usize], Ty::Func { .. }) {
                    let n = self
                        .declaring
                        .and_then(|n| self.text(n))
                        .map(|t| format!(" `{t}`"))
                        .unwrap_or_default();
                    self.error(
                        node.span,
                        format!("declaration of{n} as an array of functions"),
                    );
                }
                let l = match len {
                    chiero_ast::ArrayLen::Zero => {
                        // **A zero-size array is a GNU extension, a flexible member is not.**
                        // Measured: `gnu11` accepts `char d[0]` and `-pedantic-errors` refuses
                        // it ("ISO C forbids zero-size array"), while `char d[]` — C99's
                        // flexible array member, `ArrayLen::Unspecified` below — is accepted by
                        // both. The AST already keeps them apart, which is what lets the rule
                        // name the extension without taking every VPP struct that ends in one.
                        //
                        // 013 puts `[0]` arrays in 1165 VPP files and calls them required, so
                        // support is unconditional and only the sentence follows the dialect.
                        if self.dialect.pedantic {
                            // Named the way the neighbouring array rules name a declarator, so
                            // a reader gets `data` rather than a bare sentence about arrays.
                            let n = self
                                .declaring
                                .and_then(|n| self.text(n))
                                .map(|t| format!(" `{t}`"))
                                .unwrap_or_default();
                            self.advisory(node.span, format!("ISO C forbids zero-size array{n}"));
                        }
                        ArrayLen::Zero
                    }
                    chiero_ast::ArrayLen::Unspecified | chiero_ast::ArrayLen::Star => {
                        ArrayLen::Flexible
                    }
                    chiero_ast::ArrayLen::Fixed(expr) => {
                        let before = self.out.diagnostics.len();
                        let n = self.eval(expr).map(|v| v.v);
                        // **A length that failed *and explained* is poison, not a VLA.** `1/0`
                        // does not fold, and calling the result variably modified made
                        // `int a[1/0];` report twice: the division, then "variably modified type
                        // at file scope" — a consequence of the first, which is contract 20's
                        // cascade. A genuine `int a[n]` folds to `None` with nothing said and is
                        // still a VLA.
                        // The same rule: an explained length is poison whether the fold failed
                        // outright (`1/0`) or produced a wrapped value (`2147483647 + 1`, which
                        // comes out negative and would draw a second complaint about that).
                        if self.out.diagnostics.len() > before {
                            return self.intern(Ty::Array {
                                elem: e,
                                len: ArrayLen::Fixed(0),
                            });
                        }
                        match n {
                            Some(n) if n >= 0 => ArrayLen::Fixed(n as u64),
                            Some(_) => {
                                let n = self
                                    .declaring
                                    .and_then(|n| self.text(n))
                                    .map(|t| format!(" of `{t}`"))
                                    .unwrap_or_default();
                                self.error(node.span, format!("array length{n} is negative"));
                                ArrayLen::Fixed(0)
                            }
                            None => {
                                // **C 6.7.6.2p1: the size has integer type**, and that is a
                                // question about the *type*, not about whether the size folded.
                                // Everything that failed to fold used to become a VLA, so
                                // `int a[1.5];` was silent as a local and a parameter, and at
                                // file scope and in a member it was called variably modified.
                                //
                                // Asked here, in the arm that was about to say "VLA", so only an
                                // expression that already failed to fold is typed at all — and
                                // **quietly**, because these expressions were never typed before
                                // and reporting from them now would be a new and much wider
                                // change than this rule. The type is all that is wanted.
                                //
                                // `Ty::Error` counts as integer: something else already spoke.
                                let t = self.re_resolving(|cx| {
                                    let n = cx.type_expr(expr);
                                    cx.out.typed.ty_of(n)
                                });
                                // **No `bare` here on purpose.** A qualifier lives in a table
                                // beside `types`, not inside `Ty`, so `self.out.types[t]` already
                                // yields the unqualified shape — `const double` matches
                                // `Ty::Float` exactly as `double` does. A `bare` call was written
                                // here first and mutation exposed it as an *equivalent* mutant:
                                // replacing it with `t` changed nothing any test could see.
                                // An equivalent mutant is dead code, not a missing row.
                                if matches!(
                                    self.out.types[t.0 as usize],
                                    Ty::Int { .. } | Ty::Error
                                ) {
                                    ArrayLen::Vla(expr)
                                } else {
                                    let n = self
                                        .declaring
                                        .and_then(|n| self.text(n))
                                        .map(|t| format!(" of `{t}`"))
                                        .unwrap_or_default();
                                    self.error(
                                        node.span,
                                        format!("array length{n} has a non-integer type"),
                                    );
                                    ArrayLen::Fixed(0)
                                }
                            }
                        }
                    }
                };
                self.intern(Ty::Array { elem: e, len: l })
            }
            TypeKind::Func {
                ret,
                params,
                variadic,
                prototyped,
                ..
            } => {
                let r = self.ty_of(ret);
                // **A function may not return an array or another function** (C 6.7.6.3p1):
                // neither can be copied out by value, and both are what pointers exist for.
                // `int (*f(void))[3]` is unaffected — its return type is a *pointer*, and the
                // array lives behind it.
                if matches!(
                    self.out.types[r.0 as usize],
                    Ty::Array { .. } | Ty::Func { .. }
                ) {
                    // **At the return type, not at the function.** The fault is the `[3]` in
                    // `int f(void)[3]`, and the function node covers the whole declarator — true
                    // of the mistake and useless for finding it.
                    self.error(
                        self.ast.ty(ret).span,
                        "a function may not return an array or a function",
                    );
                }
                // **A parameter takes only `register`** (C 6.7.6.3p2). Asked here because a
                // parameter never reaches `decl` — it is a `DeclId` walked for its *type* only,
                // which is why the storage on it had nothing looking at it at all.
                for &p in &params {
                    match self.ast.decl(p).kind.clone() {
                        DeclKind::Var { storage, .. } => self.check_storage_context(
                            storage,
                            StorageContext::Parameter,
                            self.ast.decl(p).span,
                        ),
                        DeclKind::Typedef { .. } => self.error(
                            self.ast.decl(p).span,
                            "`typedef` is not allowed in a parameter",
                        ),
                        _ => {}
                    }
                }
                // **A parameter list is its own scope** (C 6.7.6.3p1), so a tag defined in one
                // is not a redefinition of anything and does not outlive the declarator. Without
                // this, `int f(struct S { int a; } s);` was refused outright — and a *definition*
                // was refused twice, since the list is resolved once for the function's type and
                // again when the body is walked, and the second pass found what the first
                // installed. The parameter's *type* is already a `TyId` by then, so a body that
                // says `s.a` is unaffected by the tag name going out of scope here.
                self.defined_tags.enter();
                self.declared_enumerators.enter();
                let ps: Vec<TyId> = params
                    .iter()
                    .map(|&p| match &self.ast.decl(p).kind {
                        DeclKind::Var { name, ty, .. } => {
                            let (name, ty) = (*name, *ty);
                            // **A parameter is a declarator and names itself in a diagnostic.**
                            // `declaring` is what "array length of `a`" reads, and only the
                            // object and member paths were setting it — so a bad bound in a
                            // parameter, which is the position a reader can least easily locate,
                            // was the one that would not say which. Left `None` for an unnamed
                            // parameter, where the unnamed wording is correct.
                            let outer = std::mem::replace(&mut self.declaring, name);
                            let t = self.ty_of(ty);
                            self.declaring = outer;
                            self.check_alignment(ty, t, StorageContext::Parameter);
                            self.adjusted_param_ty(ty, t)
                        }
                        _ => self.intern(Ty::Error),
                    })
                    .collect();
                // **A parameter name is unique within its list** (C 6.7.6.3p2). This lives here,
                // where every declarator passes, rather than on the definition path where it was
                // until wave 359 — keyed there, `int f(int x, int x);` was refused with a body
                // and accepted without one. Two *functions* may each have an `x`, which is why
                // the set is built per list and never consulted across them.
                let mut seen: indexmap::IndexSet<Symbol> = Default::default();
                for &p in &params {
                    if let DeclKind::Var { name: Some(pn), .. } = self.ast.decl(p).kind
                        && !seen.insert(pn)
                    {
                        let text = self.text(pn).unwrap_or("?").to_owned();
                        self.error(
                            self.ast.decl(p).span,
                            format!("duplicate parameter `{text}`"),
                        );
                    }
                }
                // **`...` follows at least one named parameter** (C 6.7.6.3p4): `va_start` needs
                // a parameter to start from. Calibrated to `-pedantic-errors`, as wave 314 set —
                // `-std=gnu11` takes a bare `f(...)`.
                if variadic && params.is_empty() {
                    self.error(node.span, "`...` needs a named parameter before it");
                }
                self.declared_enumerators.leave();
                self.defined_tags.leave();
                // **`void` may be the whole parameter list, but not one of several**
                // (C 6.7.6.3p10). `f(void)` never reaches here — the parser recognises it and
                // returns an empty list.
                //
                // **A surviving `Ty::Void` is not by itself a mistake**, which is what this used
                // to assume. Two legal shapes also survive the parser's folding: a *named* one,
                // `int f(void v);`, which gcc accepts with a warning; and a *typedef'd* one,
                // `typedef void V; int f(V);`, which the parser cannot fold because it does not
                // know `V` is `void`. Both were refused, the second being ordinary C.
                //
                // So the special case is recognised here on the **type**, and the rule fires only
                // on an **unnamed** `void`: gcc takes `int f(int a, void v);` too, so the
                // spelling and not the count is what distinguishes the constraint from the
                // warning. A named `void` parameter is left to the incomplete-parameter rule,
                // which already refuses it in a *definition* and correctly permits it in a
                // declaration.
                //
                // Reported once per list rather than once per offending parameter, because
                // `f(void, void)` is one mistake about one list. Contract 20's spirit: one bad
                // declaration, one diagnostic.
                let unnamed_void = |cx: &Self, i: usize| {
                    matches!(cx.out.types[ps[i].0 as usize], Ty::Void)
                        && params.get(i).is_some_and(|&p| {
                            matches!(cx.ast.decl(p).kind, DeclKind::Var { name: None, .. })
                        })
                };
                let offending = (0..ps.len()).any(|i| unnamed_void(self, i));
                if offending {
                    // **Two faults reached one sentence.** `f(void, int)` has a `void` among
                    // others; `f(const void)` has `void` *as* the only parameter and qualifies
                    // it — so the old message told a reader the opposite of what was wrong. The
                    // parser folds `(void)` into an empty list, so a single `Ty::Void` still
                    // standing means either shape, and the count tells them apart.
                    // A *sole* unnamed `void` is the special case itself and legal — unless it
                    // is qualified, which is its own sentence.
                    let qualified_only = ps.len() == 1
                        && params.first().is_some_and(|&p| {
                            matches!(self.ast.decl(p).kind, DeclKind::Var { ty, .. }
                            if {
                                let q = self.ast.ty(ty).quals;
                                q.const_ || q.volatile_ || q.restrict_ || q.atomic
                            })
                        });
                    if qualified_only {
                        self.error(
                            node.span,
                            "`void` as the only parameter may not be qualified",
                        );
                    } else if ps.len() > 1 {
                        self.error(node.span, "`void` must be the only parameter");
                    }
                }
                self.intern(Ty::Func {
                    ret: r,
                    params: ps,
                    variadic,
                    prototyped,
                })
            }
            TypeKind::Tag {
                tag, name, members, ..
            } => self.tag(ty, tag, name, members),
            // **`typeof` is the operand's type, and the operand is not evaluated.** The arm
            // used to return `Ty::Error` with a note that it "needs expression typing, which
            // is contract 11's half of 014 and is not this slice" — a declared gap from
            // before expression typing existed. It does now, so the gap is just a gap.
            //
            // **No decay.** `__typeof__(a)` for `int a[4]` is `int[4]`, not `int *`: gcc keeps
            // the array type, and `sizeof` of it is 16. Every other place that reads an
            // expression's type here reaches for `decay` first, which would be wrong in
            // exactly the case `typeof` exists for — copying an object's type.
            TypeKind::TypeofExpr(e) => {
                // **The operand is not evaluated**, so a dereference inside it designates no
                // object and needs no size: `typeof (**v)` on a `struct I **` is the type
                // `struct I`, which C is perfectly able to name while it is incomplete. gcc is
                // silent on it and errors on a real `*p`, and VPP's `vec_foreach_pointer` is
                // written exactly this way — every walk over a vector of opaque pointers.
                //
                // **A counter, not a flag, and not `quiet`.** Nesting is possible, and dropping
                // *every* diagnostic here would hide the ones that are still faults in an
                // unevaluated operand — an undeclared name in it is an error in gcc too.
                self.in_typeof += 1;
                let node = self.type_expr(e);
                self.in_typeof -= 1;
                self.out.typed.ty_of(node)
            }
            TypeKind::TypeofType(inner) => self.ty_of(inner),
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
            // **`bits` alone does not identify the type**, and reading it alone was a defect this
            // wave introduced and its own fixture caught: `_Float64x` says 64, so it landed on
            // `Float64Ext` and `sizeof` answered 8 where gcc says 16. The `x` forms mean "wider
            // than the unsuffixed one" — `FloatFmt::Extended` is what the parser records them as,
            // and it has to be read.
            B::ExtFloat { bits, fmt } => Ty::Float(match (bits, fmt) {
                (32, chiero_ast::FloatFmt::Extended) => FloatKind::Float32xExt,
                (64, chiero_ast::FloatFmt::Extended) => FloatKind::Float64xExt,
                (16, chiero_ast::FloatFmt::Brain) => FloatKind::BFloat16,
                (16, _) => FloatKind::Binary16,
                // **`_Float32` is not `float`.** Same width, same representation, different type
                // — which is the whole of what these two variants are for.
                (32, _) => FloatKind::Float32Ext,
                (64, _) => FloatKind::Float64Ext,
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
        // **A definition is resolved once per syntactic node, not once per declarator.**
        // C 6.7p1: a declaration is specifiers followed by an init-declarator-*list*, so
        // `struct S { int x; } a, b;` has one specifier and two declarators — and sema resolves
        // the specifier for each of them. The second call found the tag the first installed and
        // called it a redefinition; the tagless form was worse, minting a *fresh* anonymous
        // record per declarator so that `struct { int x; } a, b; a = b;` compared two unrelated
        // types.
        //
        // Keyed on the AST node, so it memoises exactly "this definition, already done" and not
        // "this type, seen before" — two identical anonymous records written separately are
        // still two types, which is a row in the fixture.
        //
        // **Only definitions, and that restriction is deliberately unproven.** A mutant that
        // memoises *references* too survives the entire workspace — 1608 tests — and no case
        // distinguishing the two could be constructed: a reference resolves through the scoped
        // `tags` map, but the same AST node is only re-resolved on paths (a parameter list, wave
        // 388) where the scope state is the same both times.
        //
        // It is kept narrow anyway, because absence of evidence is not equivalence. Wave 391
        // removed a redundant `bare` call on the strength of the crate's own `Qual` documentation
        // *proving* it could not matter; there is no such proof here, and a memo that caches an
        // incomplete record for a tag defined later would be a wrong answer rather than a noisy
        // one. Memoising less is the conservative direction.
        //
        // Recorded rather than silently left: this guard has no killing test.
        if members.is_some()
            && let Some(&already) = self.out.syntactic_types.get(&node)
        {
            return already;
        }
        if tag == chiero_ast::TagKind::Enum {
            return self.enum_ty(node, span, name, members);
        }
        // A reference to a tag, defined or not.
        if members.is_none() {
            if let Some(&rid) = name.and_then(|n| self.tags.get(&n)) {
                return self.intern(Ty::Record(rid));
            }
            // **A named but undefined tag still gets a `RecordId`**, marked incomplete. This is
            // what lets the definition — later in the file, or the one currently being laid out —
            // fill in *this* record, so that every reference written before it sees the finished
            // type. Returning `Ty::Error` here instead froze the reference, which is why
            // `struct Node { struct Node *next; }` could not be walked.
            //
            // An *anonymous* undefined tag is a different thing: there is no name for a later
            // definition to match, so nothing can ever complete it and `Ty::Error` is honest.
            let Some(name) = name else {
                return self.intern(Ty::Error);
            };
            let rid = self.declare_tag(name, tag == chiero_ast::TagKind::Union);
            return self.intern(Ty::Record(rid));
        }
        if let Some(name) = name {
            if self.in_progress.contains(&name) {
                let n = self.text(name).unwrap_or("?").to_owned();
                self.error(span, format!("`struct {n}` contains itself by value"));
                return self.intern(Ty::Error);
            }
            self.in_progress.push(name);
        }
        // **Reserve the record before laying out the members**, so a member that mentions this
        // tag finds it in the table and resolves to the record being built rather than to a
        // second, permanently incomplete one. It is still marked incomplete while the members are
        // walked, which is exactly right: `struct S { struct S s; }` by value must fail, and it
        // fails because an incomplete record has no size.
        let is_union = tag == chiero_ast::TagKind::Union;
        let rid = match name {
            Some(name) => self.declare_tag(name, is_union),
            None => {
                let rid = RecordId(self.out.records.len() as u32);
                self.out.records.push(incomplete_layout(is_union));
                rid
            }
        };
        // **A tag is defined once per scope** (C 6.7.2.3p1). Registered on the *definition*
        // only: `struct S;` after `struct S { ... };` is how a forward declaration is written, so
        // a rule keyed on "seen this tag before" rejects the idiom it exists to permit.
        // **Wrong kind and second definition are different faults** (C 6.7.2.3p1). `union U { …
        // }; struct U { … };` is not a redefinition of `struct U` — there was never one — and
        // saying so sends a reader looking for it. gcc keeps "redefinition" for a second
        // definition of the *same* kind, and this now does too. The kinds are 0/1/2 for
        // struct/union/enum, which is the whole of what the table has to remember.
        let kind = u8::from(is_union);
        if let Some(name) = name
            && let Some(before) = self.defined_tags.redeclares_as(name, kind)
        {
            let n = self.text(name).unwrap_or("?").to_owned();
            if before == kind {
                let kw = if is_union { "union" } else { "struct" };
                self.error(span, format!("redefinition of `{kw} {n}`"));
            } else {
                self.error(span, format!("`{n}` is defined as the wrong kind of tag"));
            }
        }
        let members = members.unwrap();
        let layout = self.lay_out(node, is_union, &members);
        // **C 6.7.2.1p8: a record needs a named member.** gcc separates two cases and so does
        // this, because they are two different fixes: nothing at all is "has no members", while
        // only unnamed bit-fields is "has no *named* members" — add a member, or name one.
        //
        // **An anonymous record member supplies names**, so `struct S { struct { int x; }; };` is
        // fine. It is recognised as a member with no name and no bit-field, which is exactly the
        // shape: an unnamed *bit-field* is the only other way to write a member without a name.
        //
        // Unlike the `_Alignas` rule beside it this is a `-Wpedantic` promotion, reportable here
        // only because wave 314 calibrated to `-pedantic-errors`. VPP has none of either.
        let kw = if is_union { "union" } else { "struct" };
        let tag = name.and_then(|n| self.text(n)).map(str::to_owned);
        let names_something = |cx: &Self, m: DeclId| {
            matches!(&cx.ast.decl(m).kind, DeclKind::Var { name, .. } if name.is_some())
                || (matches!(&cx.ast.decl(m).kind, DeclKind::Var { name: None, .. })
                    && cx.ast.bitfield(m).is_none())
        };
        // **A tagless record is a record too.** gcc reports `struct { } x;` and
        // `struct S { struct { }; int a; };` exactly as it reports the tagged forms, and guarding
        // this on the tag left the whole untagged half silent — six programs, found by carrying
        // the census one question further than the rule that prompted it. The tag only decides
        // whether the message can name anything.
        let named = tag.map_or_else(|| kw.to_owned(), |t| format!("{kw} `{t}`"));
        if members.is_empty() {
            // Pedantic-only, measured: `struct S { union { struct { } inner; } u; };` is
            // accepted by `gcc -std=gnu11`, refused by `-pedantic-errors`.
            if self.dialect.pedantic {
                self.advisory(span, format!("{named} has no members"));
            }
        } else if !members.iter().any(|&m| names_something(self, m)) {
            self.advisory(span, format!("{named} has no named members"));
        }
        if name.is_some() {
            self.in_progress.pop();
        }
        self.out.records[rid.0 as usize] = layout;
        if let Some(name) = name {
            self.tags.insert(name, rid);
            self.out.by_tag.insert(name, rid);
        }
        self.intern(Ty::Record(rid))
    }

    /// The `RecordId` a tag name denotes, creating an incomplete one if the name is new.
    ///
    /// Registered in `tags` immediately, which is the whole point: everything that mentions the
    /// name afterwards — including the members of the definition currently being laid out — must
    /// reach the same record, so that completing it completes them all.
    fn declare_tag(&mut self, name: Symbol, is_union: bool) -> RecordId {
        if let Some(&rid) = self.tags.get(&name) {
            return rid;
        }
        let rid = RecordId(self.out.records.len() as u32);
        self.out.records.push(incomplete_layout(is_union));
        self.tags.insert(name, rid);
        self.out.by_tag.insert(name, rid);
        rid
    }

    /// 014 contract 10: the underlying type is `int` unless a value requires wider.
    fn enum_ty(
        &mut self,
        node: TypeId,
        span: Span,
        name: Option<Symbol>,
        members: Option<Vec<DeclId>>,
    ) -> TyId {
        let Some(members) = members else {
            if let Some(&t) = name.and_then(|n| self.enums.get(&n)) {
                return t;
            }
            // **An enum tag whose enumerators have not been seen is incomplete**, and answering
            // `int` made it indistinguishable from a complete one — so an object of that type,
            // and a struct member of it, were accepted at four bytes.
            //
            // The lookup above is what keeps a *re*declaration after the definition legal:
            // `enum E { A }; enum E; enum E e;` finds `E` in `enums` and never reaches here. Only
            // a tag that has genuinely never been defined does.
            //
            // Unlike a struct tag this does not get a completable placeholder. An enum's complete
            // form is `Ty::Int`, so there is no record to fill in later, and a reference written
            // before the definition stays poisoned. That is a real limit, recorded in §9 — it
            // costs nothing in standard C, where an enum cannot be forward-declared at all.
            let _ = span;
            return self.intern(Ty::Error);
        };
        // **An enum tag is defined once per scope** (C 6.7.2.3p1), the same rule the struct and
        // union path enforces through the same scoped set — and it was missing here, so
        // `enum E { A }; enum E { B };` was accepted. Found by a fixture written for the
        // parameter-list scope beside it: the row was there to prove the new scope had not made
        // genuine redefinitions legal, and it turned out they always had been.
        //
        // Registered on the **definition** only, exactly as for a struct: this arm is past the
        // `members.is_none()` return, so `enum E;` after a definition never reaches it.
        if let Some(name) = name
            && let Some(before) = self.defined_tags.redeclares_as(name, 2)
        {
            let n = self.text(name).unwrap_or("?").to_owned();
            if before == 2 {
                self.error(span, format!("redefinition of `enum {n}`"));
            } else {
                self.error(span, format!("`{n}` is defined as the wrong kind of tag"));
            }
        }
        // **C 6.7.2.2p1: an enumeration has an enumerator list.** Unlike the range rule below,
        // gcc refuses `enum E { };` in GNU mode too — an empty enumeration has no values, so
        // there is no extension to be had.
        if members.is_empty() {
            self.error(span, "an enumeration needs at least one enumerator");
        }
        let mut next = 0i128;
        let mut lo = 0i128;
        let mut hi = 0i128;
        // Each enumerator's value with the span that should be blamed for it.
        let mut values: Vec<(i128, Span)> = Vec::new();
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
                Some(e) => {
                    let before = self.out.diagnostics.len();
                    let folded = self.eval(e);
                    // **If the fold already explained itself, do not add a second sentence.**
                    // `enum { X = sizeof(struct I) }` has one cause and had two messages: the
                    // `sizeof` complaint and this one. The second tells a reader nothing the
                    // first did not, and 023 §9 asks for reports a person can act on rather than
                    // every true sentence about a program.
                    let explained = self.out.diagnostics.len() > before;
                    match folded {
                        Some(v) => v.v,
                        // **020 §5: a gap is a diagnostic rather than a licence.** Falling back
                        // to `next` silently gave the enumerator exactly the value it would have
                        // had with no initializer written, so a constant expression this engine
                        // cannot fold was indistinguishable from one it folded correctly. That is
                        // how a missing `sizeof` arm survived to be found by accident rather than
                        // by the suite. The fallback is kept — an enumeration that stopped
                        // resolving would cascade into every use of its type, exactly as for an
                        // array bound — but it is now announced, unless something already has.
                        None => match self.fold_arith(e) {
                            // **Foldable as an *arithmetic* constant expression (6.6p8) but
                            // not an integer one (6.6p6).** Measured: `gnu11` folds
                            // `(u32)(1/0.01)` silently, `-pedantic-errors` reports exactly
                            // this sentence tagged `[-Wpedantic]`. 11 findings through
                            // `plugins/wireguard/wireguard_messages.h`.
                            //
                            // The value is folded in **both** dialects: silencing the message
                            // and leaving `next` behind would hand back 6 where gcc says 33,
                            // with nothing said at all.
                            Some(f) => {
                                if !explained && self.dialect.pedantic {
                                    self.error(
                                        self.ast.expr(e).span,
                                        "enumerator value is not an integer constant expression",
                                    );
                                }
                                f.trunc() as i128
                            }
                            // **Not a constant at all** — a variable, a call. gcc refuses
                            // that under `gnu11` too, so the dialect does not enter. One
                            // sentence, two causes; only the foldable one is a calibration
                            // question.
                            None => {
                                if !explained {
                                    self.error(
                                        self.ast.expr(e).span,
                                        "enumerator value is not an integer constant expression",
                                    );
                                }
                                next
                            }
                        },
                    }
                }
                None => next,
            };
            next = v.wrapping_add(1);
            lo = lo.min(v);
            hi = hi.max(v);
            // **Remember where each value came from**, because the range check below cannot
            // recover it. Wave 357 put that check after the loop so the *implicit successor* is
            // caught — `{A = 2147483647, B}` overflows on a `B` that names no value — and that is
            // exactly the case with no initializer to point at, so the fallback is the
            // enumerator's own declaration. gcc draws the same distinction: the value when one is
            // written, the name when it is not.
            values.push((
                v,
                match init {
                    Some(e) => self.ast.expr(e).span,
                    None => self.ast.decl(*m).span,
                },
            ));
            // **An enumerator is an ordinary identifier in its scope** (C 6.7.2.2p3), not a
            // member of its enumeration. So two *different* enums in one scope may not share a
            // constant's name any more than one enum may repeat one, and shadowing in an inner
            // scope stays legal — which is why this is a scoped set rather than a check against
            // the flat `enumerators` table, whose second write simply wins.
            if self.declared_enumerators.redeclares(en) {
                let n = self.text(en).unwrap_or("?").to_owned();
                self.error(
                    self.ast.decl(*m).span,
                    format!("redeclaration of enumerator `{n}`"),
                );
            }
            self.declare_ordinary(en, Meaning::Enumerator, self.ast.decl(*m).span);
            self.enumerators.insert(en, v);
            pending.push((en, v));
        }
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        let long_bits = (self.target.sizes.long_ * 8) as u32;
        // **One rule, not two** (C 6.7.2.2p4 leaves the choice to the implementation; this is
        // gcc's). The sign comes from whether any enumerator is negative, and the width is the
        // narrowest of `int`/`long` that holds the range at that sign. The fitting and widened
        // cases used to be written as separate branches with separate signedness rules, and the
        // widened one was wrong twice over: it always chose `long`, so `{ A = 4294967295u }` was
        // 64 bits where gcc uses `unsigned int`, and its sign test `lo < 0 || hi < (1 << 63)`
        // made a non-negative `{ A = LONG_MAX }` *signed* where gcc uses `unsigned long`.
        //
        // The enumerator constant is separate and stays `int` (6.4.4.3p2), which is why `-1 < A`
        // and `-1 < e` do not have to agree.
        let signed = lo < 0;
        let holds = |bits: u32| {
            if signed {
                lo >= -(1i128 << (bits - 1)) && hi < (1i128 << (bits - 1))
            } else {
                hi < (1i128 << bits)
            }
        };
        // **`packed` narrows an enumeration to the smallest width that holds its range** (the
        // GNU attribute). gcc gives `{ A = 0, B = 3 }` one byte, `{ 0..300 }` two, and keeps the
        // sign: `{ -1, 3 }` is a signed byte. Ignoring the attribute gave four bytes for all of
        // them — a wrong *size*, which every consumer downstream believes, rather than a wrong
        // message. VPP's `ip_ecn_t` asserts its own width, so the error surfaced as a failing
        // `_Static_assert` on legal code.
        //
        // Read from the AST node, like `packed` on a record: it is a specifier attribute and
        // never reaches the interned type.
        // **Declarator attributes are not the definition's** — see `Attr::from_declarator`.
        // gcc is explicit about this one: `typedef struct S {…} T __attribute__((packed));`
        // compiles with `warning: 'packed' attribute ignored` and leaves `struct S` alone.
        let packed = self.ast.ty(node).attrs.iter().any(|a| {
            !a.from_declarator
                && !a.from_specifier
                && matches!(self.text(a.name), Some("packed" | "__packed__"))
        });
        let bits = if packed {
            // Smallest first; `long_bits` is the last resort and the existing answer for a range
            // that needs it, so an over-wide enumeration is unaffected by the attribute.
            [8, 16, 32, long_bits]
                .into_iter()
                .find(|&b| holds(b))
                .unwrap_or(long_bits)
        } else if holds(int_bits) {
            int_bits
        } else {
            long_bits
        };
        let t = Ty::Int { signed, bits };
        // **C 6.7.2.2p2: every enumerator is representable as `int`.** Anything wider is a GNU
        // extension, and this project calibrates constraint violations to `-pedantic-errors`
        // (wave 314), so the widening happens *and* is reported — the type stays usable so
        // nothing downstream cascades, exactly as an out-of-range array bound is reported and
        // then clamped.
        //
        // **This asks about `int`, not about the type chosen above**, and the two differ: an
        // enumeration of `{ A = 4294967295u }` is `unsigned int` here and is still refused,
        // because the constraint is on each value's representability as `int` and not on how
        // wide the implementation's choice turned out to be.
        //
        // Reported from the *range* rather than from each initializer, which is what catches the
        // implicit successor: `{A = 2147483647, B}` names no value for `B` and overflows on it.
        // **The enumerator, not the enumeration.** Naming the whole `enum { … }` is true and
        // useless once there is more than one constant in it; the first value outside the range
        // is the one a reader has to change.
        let int_min = -(1i128 << (int_bits - 1));
        let int_max = 1i128 << (int_bits - 1);
        if lo < int_min || hi >= int_max {
            let blame = values
                .iter()
                .find(|&&(v, _)| v < int_min || v >= int_max)
                .map_or(span, |&(_, s)| s);
            // **Pedantic-only, measured**: `enum { A = 0xffffffffu }` is accepted by
            // `gcc -std=gnu11` and refused by `-pedantic-errors` ("ISO C restricts
            // enumerator values to range of `int`"). VPP relies on it in 336 of
            // `vnet`'s 348 findings.
            if self.dialect.pedantic {
                self.advisory(
                    blame,
                    "an enumerator's value is not representable as an `int`",
                );
            }
        }
        // **A fresh number per definition** (C 6.7.2.3p5), which is what makes `enum E` and
        // `enum F` two types even though the sign gives both the same integer type. Per
        // definition and not per tag name: two anonymous enumerations are two types and have no
        // name to be numbered by. A *reference* to an existing tag never reaches here — it
        // returns the cached id from `enums` above — so `enum E a; enum E b;` is one number.
        // **A declared underlying type wins over the fitted one** — `enum E : unsigned char`
        // is one byte whatever its enumerators would have fitted in. The fitting above still
        // runs: it is what produces the pedantic diagnostics, and silencing those because a
        // base type was written would hide a real ISO complaint about the enumerators.
        //
        // Read off the AST node rather than threaded through the signature, because a
        // *reference* to the tag returns from `enums` long before here and never has one.
        let declared = match &self.ast.ty(node).kind {
            chiero_ast::TypeKind::Tag { underlying, .. } => *underlying,
            _ => None,
        };
        let t = match declared {
            Some(u) => {
                let id = self.ty_of(u);
                self.out.ty(id).clone()
            }
            None => t,
        };
        self.next_enum_tag += 1;
        let id = self.intern_tagged(t, Qual::NONE, self.next_enum_tag);
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
        // ⚠️ **Filtered by position, and it was not.** This is the second place that asks
        // whether a record is packed — the other is `record_is_packed`, ninety lines up, which
        // has always filtered `from_declarator`. Two sites reading one fact differently is how
        // a rule ends up half-applied: this one honoured an attribute written *before* the
        // `struct` keyword, which gcc and clang both ignore, so
        // `__attribute__((packed)) struct S { char a; int b; }` came out 5 bytes with `b` at
        // offset 1 against gcc's 8 and 4. See [`chiero_ast::Attr::from_specifier`].
        let definition_attr = |a: &chiero_ast::Attr| !a.from_declarator && !a.from_specifier;
        let packed = self.ast.ty(node).attrs.iter().any(|a| {
            definition_attr(a) && matches!(self.text(a.name), Some("packed" | "__packed__"))
        });
        // **Only a union can be transparent** (gcc rejects the attribute on a struct), and the
        // flag is read here beside `packed` because both are attributes of the definition.
        let transparent = is_union
            && self.ast.ty(node).attrs.iter().any(|a| {
                definition_attr(a)
                    && matches!(
                        self.text(a.name),
                        Some("transparent_union" | "__transparent_union__")
                    )
            });

        let mut fields: Vec<FieldLayout> = Vec::new();
        let mut bit_cursor: u64 = 0; // bits from the start of the record
        let mut size_bits: u64 = 0;
        let mut align: u64 = 1;
        let mut flexible_member = None;
        // Set where the `w == 0` case is handled, which is the only place a `:0` is seen.
        let mut has_zero_width_bitfield = false;

        for &m in members {
            let DeclKind::Var {
                name, ty, storage, ..
            } = self.ast.decl(m).kind.clone()
            else {
                continue;
            };
            // **A member is not an object declaration either** (C 6.7.4p2): `struct S { inline
            // int a; }` is refused by gcc in *both* modes, unlike the `-pedantic-errors`-only
            // rows on objects. Storage classes on a member are the parser's business; this is
            // the specifier half only, which is why the shared context is named for what a
            // member and a `typedef` have in common rather than for either one.
            self.check_storage_context(storage, StorageContext::NotAnObject, self.ast.decl(m).span);
            // **A member's type is built for the *member*.** Without this the enclosing object's
            // name was still in `declaring`, so `struct S { int bad[-1]; } x;` reported "array
            // length of `x` is negative" — naming a declarator that is not the one at fault,
            // which is exactly the class of defect this audit exists to remove. Introduced by the
            // mechanism two commits earlier and caught by a mutant on its own restore.
            let outer_declaring = std::mem::replace(&mut self.declaring, name);
            // **Whether resolving the member's type said anything**, which is the only reliable
            // reading of "already explained" here. `Ty::Error` is not one: chiero uses it both
            // for a type that failed to resolve *and* for an `enum E;` whose tag has no
            // definition, and the second is an incompleteness this record must still report.
            // Keying on the diagnostic count separates them without inventing a distinction the
            // type table does not carry.
            let before_member_ty = self.out.diagnostics.len();
            let fty = self.ty_of(ty);
            let member_ty_explained = self.out.diagnostics.len() > before_member_ty;
            // A member is an object, so `_Alignas` is legal on it and only the value is checked.
            // **Using the type already resolved**, never resolving it again: a second `ty_of` on
            // the same node re-emits whatever the first said — an inline `struct T { … }` in the
            // member's own declaration was reported as a redefinition of itself. Wave 359's
            // parameter-list defect, in a new caller.
            // **C 6.7.5p2: an alignment specifier may not be applied to a bit-field.** gcc
            // refuses this in *both* modes, unlike most of what this project reports, and it
            // refuses **only the `_Alignas` spelling** — `__attribute__((aligned(8))) int a : 2`
            // is accepted. That is the same split `check_alignment` already draws for typedefs
            // and for weakening an alignment, so the attribute spelling is deliberately not
            // included here; measuring it is what kept this rule from being one construct wider
            // than gcc's.
            //
            // Reported instead of, not as well as, the value checks: `_Alignas(3)` on a
            // bit-field has one thing wrong with it that a reader can act on (contract 20).
            let alignas_attr = self.ast.bitfield(m).is_some().then(|| {
                self.ast
                    .ty(ty)
                    .attrs
                    .iter()
                    .find(|a| matches!(self.text(a.name), Some("_Alignas")))
                    .map(|a| a.span)
            });
            if let Some(Some(aspan)) = alignas_attr {
                match name.and_then(|n| self.text(n)) {
                    Some(text) => {
                        let text = text.to_owned();
                        self.error(aspan, format!("alignment specified for bit-field `{text}`"));
                    }
                    None => self.error(aspan, "alignment specified for an unnamed bit-field"),
                }
            } else {
                self.check_alignment(ty, fty, StorageContext::File);
            }
            self.declaring = outer_declaring;
            // **A flexible array member is the last one** (C 6.7.2.1p18). Checked by looking
            // *back*: an `ArrayLen::Flexible` already in `fields` means a member follows one, and
            // that is the violation. Asking "is this the last member" instead would need the
            // count up front, and this way the diagnostic lands on the member that should not be
            // there rather than on the array.
            if fields.iter().any(|f: &FieldLayout| {
                matches!(
                    self.out.types[f.ty.0 as usize],
                    Ty::Array {
                        len: ArrayLen::Flexible,
                        ..
                    }
                )
            }) {
                let span = self.ast.decl(m).span;
                self.error(span, "a flexible array member must be the last member");
            }
            // **...and it needs a member before it, and never appears in a union** (C 6.7.2.1p18).
            // The three halves of one paragraph, and only the "last" one existed. Asked here,
            // where `fields` holds the members seen so far, which is what makes "before it" a
            // question this loop can answer without a second pass.
            if matches!(
                self.out.types[fty.0 as usize],
                Ty::Array {
                    len: ArrayLen::Flexible,
                    ..
                }
            ) {
                let span = self.ast.decl(m).span;
                if is_union {
                    self.error(span, "a union may not have a flexible array member");
                }
            }
            // **A member is never variably modified** (C 6.7.2.1p9), anywhere — not only at file
            // scope, which is merely where gcc's message says so. A record has one layout, and a
            // length that is not known until the declaration is reached has nowhere in it to
            // live. `struct S { int a[n]; }` inside a function with a runtime `n` is rejected for
            // the same reason as one at file scope.
            //
            // Distinct from the *flexible* array member `int a[]`, which is `ArrayLen::Unknown`
            // and legal in the last position: that has no length at all rather than a length
            // computed at run time.
            if matches!(
                self.out.types[fty.0 as usize],
                Ty::Array {
                    len: ArrayLen::Vla(_),
                    ..
                }
            ) {
                let span = self.ast.decl(m).span;
                self.error(span, "a member cannot have a variably modified type");
            }
            // **A member name is unique within its record** (C 6.7.2.1p2), and only within it:
            // two structs may each have an `m`, so the set is built per `lay_out` call rather
            // than kept across them.
            // **An anonymous member's names are the record's own** (6.7.2.1p13), which is what
            // makes `s.a` resolve through one — `find_field` has always recursed into unnamed
            // record members for lookup. The uniqueness check looked only at the top level, so
            // the names it promotes for *use* were never checked for *collision*.
            //
            // Both directions matter and both are this one comparison: a named member colliding
            // with something an earlier anonymous member promoted, and an anonymous member whose
            // promotions collide with what is already there. `visible_names` answers for either
            // side, and a **named** nested member contributes nothing — which is the row that
            // stops this from being "walk into every record".
            let contributed = self.visible_names(name, fty);
            if let Some(&n) = contributed
                .iter()
                .find(|&&n| fields.iter().any(|f| self.field_shows(f, n)))
            {
                let text = self.text(n).unwrap_or("?").to_owned();
                let span = self.ast.decl(m).span;
                self.error(span, format!("duplicate member `{text}`"));
            }
            // **A member must have a size, because the record has to place it.** This is the
            // check that makes reserving the record before laying out its members safe: the
            // reservation means `struct S { struct S s; }` now finds `S` in the tag table, and
            // what stops it is that the record it finds is still marked incomplete. A pointer to
            // the same tag is fine and is the whole point of the reservation.
            //
            // **A bit-field of a non-integer type is not also reported as incomplete.** `struct
            // S { struct I a:3; }` is one mistake, and gcc says one thing about it — "bit-field
            // `a` has invalid type" — because a bit-field could not have taken that type
            // complete either. Contract 20: the more specific sentence wins and the general one
            // stands down. Poison is excused here for the same reason it is below: an
            // unresolvable type already reported itself.
            let bit_field_of_a_bad_type = self.ast.bitfield(m).is_some()
                && !matches!(self.out.types[fty.0 as usize], Ty::Int { .. } | Ty::Error);
            //
            // **A type that already reported is not also an incomplete one** (contract 20).
            // `struct S { __typeof__(nope) a; }` said `nope` was not declared *and* that `a` was
            // incomplete — the second sentence about a type this code invented. Wave 358 found
            // it while mutating the bit-field rule beside it, and the first attempt at the guard
            // asked `Ty::Error`, which also suppressed `enum E; struct S { enum E m; };` — a
            // genuinely incomplete member that nothing else reports.
            // **A member has an object type** (C 6.7.2.1p3), and a function type is not an
            // incomplete one — it is not an object type at all, so `has_no_size` below never saw
            // it. Wave 339 drew that distinction for `sizeof` and this site never got it. The
            // pointer spelling `int (*f)(void)` is a pointer and unaffected, which is the whole
            // reason C can say this.
            if matches!(self.out.types[fty.0 as usize], Ty::Func { .. }) {
                let span = self.ast.decl(m).span;
                self.error(span, "a member may not have a function type");
            } else if has_no_size(&self.out, fty)
                && !bit_field_of_a_bad_type
                && !member_ty_explained
            {
                let what = match name {
                    Some(n) => format!("`{}`", self.text(n).unwrap_or("?")),
                    None => "an unnamed field".to_owned(),
                };
                let span = self.ast.decl(m).span;
                self.error(span, format!("field {what} has an incomplete type"));
            }
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
            // **`declared_align`, not `aligned_attr`: the typedef the member names may carry
            // one.** `typedef struct {…} clib_longjmp_t __attribute__((aligned(16)));` puts the
            // alignment on the *name*, so a member declared `clib_longjmp_t j;` is 16-aligned
            // although its own declarator asks for nothing and the record it names is 8-aligned.
            //
            // This was masked by the defect above it: while the attribute was wrongly applied to
            // the record, the member inherited the alignment through the record's own, and the
            // enclosing struct came out right by two cancelling errors. Moving the attribute to
            // the name — which is where C puts it — is only correct if the name is then honoured
            // where it is used, and the contract-12 gate said so immediately: one rejection
            // became eleven, VPP's `serialize_main_t` among them.
            let requested = self.declared_align(ty);

            let bw = self.ast.bitfield(m);
            if let Some(bw) = bw {
                // **The bits *in the type*, not the bits it is stored in** (C 6.7.2.1p4). The two
                // agree for every type anyone writes a bit-field on by hand, which is why
                // `size_of_ty * 8` stood — and they differ for exactly one: a `_Bool` occupies a
                // byte and holds a single bit, so `_Bool b : 2;` was accepted. `Ty::Int`'s `bits`
                // is the number C means; the storage size is what `sizeof` reports.
                // **C 6.7.2.1p5: a bit-field's type is an integer type.** The constraint's letter
                // names `_Bool`, `signed int` and `unsigned int` and then permits
                // implementation-defined types; gcc's implementation takes every integer type,
                // including `char`, `short`, `long long`, an enumeration, and typedef and
                // qualified spellings of those. Enforcing the letter would reject nine rows gcc
                // compiles, so the line drawn here is the one gcc draws — integer or not.
                //
                // **Poison is not a non-integer type** (contract 20): a member whose type already
                // failed to resolve is `Ty::Error`, and reporting it again would be a second
                // sentence for one mistake.
                let n = name
                    .and_then(|n| self.text(n))
                    .map(|t| format!(" `{t}`"))
                    .unwrap_or_default();
                if !matches!(self.out.types[fty.0 as usize], Ty::Int { .. } | Ty::Error) {
                    let span = self.ast.decl(m).span;
                    self.error(span, format!("bit-field{n} has a non-integer type"));
                } else if self.ast.ty(ty).quals.atomic {
                    // **And not an atomic one** (C 6.7.2.1p5): a bit-field's type is a qualified
                    // or unqualified *integer* type, and `_Atomic` is not a qualifier — an atomic
                    // type is a different type, which a bit-field's addressless storage cannot
                    // provide. gcc refuses it in both modes, so this is not calibration.
                    //
                    // **In the `else`, so the type rule wins**: `_Atomic float a : 2` is refused
                    // for being a float, which is what gcc says too. Asking about `_Atomic` first
                    // would pass every other row here and change that one sentence.
                    //
                    // On the *member*, not the specifier: `struct S { _Atomic int a; };` is an
                    // ordinary atomic member and stays legal.
                    //
                    // Read from the **AST** node's qualifiers rather than from `qual_of`, because
                    // sema's `Qual` carries only `const`/`volatile`/`restrict` — `_Atomic` is a
                    // specifier that never reaches the interned type. That is also why an atomic
                    // type is not distinguishable here by its `TyId`.
                    let span = self.ast.decl(m).span;
                    self.error(span, format!("bit-field{n} has an atomic type"));
                }
                let unit_bits = match self.out.types[fty.0 as usize] {
                    Ty::Int { bits, .. } => u64::from(bits),
                    // **Reached only after the diagnostic above**, as of wave 358. Every type a
                    // bit-field may *legally* take is `Ty::Int`, and until that wave a
                    // non-integer one was accepted silently and landed here, which is why the
                    // comment used to read "unreached, and measured so". It now carries the
                    // rejected declaration far enough to keep the member — the same choice the
                    // width fallback makes, and for the same reason: one diagnostic about the
                    // type beats a second about a field that vanished because of it.
                    _ => size_of_ty(&self.out, &self.target, fty).unwrap_or(4) * 8,
                };
                let unit_align_bits = align_of_ty(&self.out, &self.target, fty).unwrap_or(4) * 8;
                let span = self.ast.expr(bw).span;

                // **C 6.7.2.1p4's constraints on a bit-field width.** Every one of them used to be
                // absorbed by `.unwrap_or(0).max(0)`, which is the worst possible place for a
                // fallback: zero is not a neutral value here, it is the *legal* unnamed
                // zero-width field handled immediately below. A width that could not be folded
                // and a negative width therefore produced a valid but different declaration —
                // member gone, next field bumped to a unit boundary — and said nothing.
                //
                // The fallback of one bit is chosen so the member still exists. Dropping it would
                // cascade into every `s.f` that references it, which is the same reason wave 301
                // kept the enumerator's fallback; the program is rejected either way, and a
                // reader is better served by one diagnostic about the width than by a second
                // about a field that vanished because of it.
                let before = self.out.diagnostics.len();
                let folded_bw = self.eval(bw);
                // **The fold's own sentence is the better one**, so the generic complaint is added
                // only when nothing has been said — the rule wave 301 gave enumerator values, and
                // the same cascade `case 1/0:` and `int a[1/0];` had.
                let explained = self.out.diagnostics.len() > before;
                let w = match folded_bw {
                    None if explained => 1,
                    None => {
                        self.error(
                            span,
                            "bit-field width is not an integer constant expression",
                        );
                        1
                    }
                    Some(v) if v.v < 0 => {
                        self.error(span, "negative width in a bit-field");
                        1
                    }
                    // The comparison is against the field's *own* type, not against `int`:
                    // `int f : 32` and `long f : 33` are both legal, and a check phrased the
                    // other way would accept every violation here and reject those.
                    Some(v) if (v.v as u64) > unit_bits => {
                        // **Name the field.** In a struct of twenty bit-fields the nameless
                        // sentence says only that one of them is wrong; gcc prints
                        // `width of 'b' exceeds its type`.
                        let n = name
                            .and_then(|n| self.text(n))
                            .map(|t| format!(" of `{t}`"))
                            .unwrap_or_default();
                        self.error(
                            span,
                            format!("bit-field width{n} exceeds the width of its type"),
                        );
                        unit_bits
                    }
                    // Zero width declares no member, so there is nothing for a name to name.
                    Some(v) if v.v == 0 && name.is_some() => {
                        self.error(span, "a named bit-field cannot have zero width");
                        1
                    }
                    Some(v) => v.v as u64,
                };

                if w == 0 {
                    // Contract 4: declares no member, and forces the next allocation to
                    // the next unit boundary.
                    //
                    // **Recorded, because `fields` cannot hold it.** See
                    // `RecordLayout::has_zero_width_bitfield`: the boundary survives only as a
                    // gap in the neighbours' offsets, and a consumer proposing a reorder must
                    // be able to tell that gap from alignment padding.
                    has_zero_width_bitfield = true;
                    // ⚠️ **In a union there is no allocation unit to flush.** Every member
                    // starts at offset 0, so "force the next allocation to the next unit
                    // boundary" names nothing, and a zero-width bit-field declares no member
                    // — it can contribute neither size nor alignment.
                    //
                    // Without this guard the bit cursor carried across union members:
                    // `union U { short a:14; int :0; }` left the cursor at 14, rounded it to
                    // the `int`'s 32, and `size_bits.max` made the union **4 bytes where gcc
                    // and clang both say 2**. A leading `:0` was already right (the cursor is
                    // still 0), which is why the ordering matters and both orders are pinned
                    // in `a_zero_width_bitfield_in_a_union_contributes_nothing`.
                    //
                    // `has_zero_width_bitfield` is still recorded: a consumer proposing a
                    // reorder needs to know the record declared one, in a union as much as in
                    // a struct.
                    // ⚠️ **`packed` does not switch the flush off**, and the guard here used
                    // to say it did. `packed` removes padding *between members* and drops
                    // member alignment to 1; a zero-width bit-field is neither. gcc and clang
                    // both still round the next allocation, and so the record's size, up to
                    // the boundary of the `:0`'s declared type:
                    //
                    //     struct { char a; int :0; }        __attribute__((packed))  is 4
                    //     struct { char a; int :0; char b; } __attribute__((packed)) is 5, b at 4
                    //
                    // chiero said 1 and 2. Found by `generated_layout.rs` on the run that
                    // added 16-byte-aligned members — a pre-existing defect in an unrelated
                    // shape, surfaced because new scalars reshuffled which records the seeds
                    // produce. A widening pays sideways as well as forwards.
                    if !is_union {
                        bit_cursor = round_up(bit_cursor, unit_align_bits);
                        size_bits = size_bits.max(bit_cursor);
                    }
                    continue;
                }
                let mut start = if is_union { 0 } else { bit_cursor };
                if !member_packed {
                    // Straddling (contract 5): if the field would cross a boundary of its
                    // declared type's storage unit, move it to the next one.
                    if unit_bits > 0 && (start % unit_bits) + w > unit_bits {
                        start = round_up(start, unit_align_bits);
                    }
                    // **Only a *named* bit-field aligns the record** — the psABI's rule, and
                    // gcc's. The unit still governs where the bits go either way, which is why
                    // the straddling computation above is unconditional and this is not.
                    //
                    // Applying it to both inflated every record with an unnamed bit-field:
                    // `struct { char c; unsigned :0; char d; }` was 8 with alignment 4 where
                    // gcc says 5 and 1. It reached the surface as a wrong `chiero layout`
                    // number, which is how a reviewer found it, but the error was here.
                    if name.is_some() {
                        align = align.max(unit_align_bits / 8);
                    }
                    align = align.max(requested.unwrap_or(1));
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
        // **The definition's, not the declarator's.** `typedef struct S {…} T
        // __attribute__((aligned(16)))` aligns `T` and leaves `struct S` alone — and gcc does
        // not round even `T`'s size up to it. Reading both here made `struct S` 112/16 where
        // gcc says 104/8, glibc's `__pthread_unwind_buf_t` among them.
        if let Some(r) = self.definition_aligned_attr(node) {
            align = align.max(r);
        }
        // **A flexible array member needs a member before it** (C 6.7.2.1p18), asked *after* the
        // loop because "before it" and "after it" are the same paragraph and only one of them
        // may speak. `struct S { int a[]; int b; }` has nothing before the flexible member and
        // something after it; gcc says one thing, "not at end of struct", so that complaint —
        // raised inside the loop — wins and this one is only reached when the flexible member is
        // the record's *sole* field. Wave 361's rule: gcc's choice of message is the tiebreak.
        if flexible_member == Some(0)
            && fields.len() == 1
            && let Some(&m) = members.first()
        {
            let span = self.ast.decl(m).span;
            self.error(span, "a flexible array member needs a member before it");
        }
        let size = round_up(round_up(size_bits, 8) / 8, align);
        RecordLayout {
            size,
            align,
            fields,
            is_union,
            flexible_member,
            packed,
            transparent,
            complete: true,
            has_zero_width_bitfield,
        }
    }

    /// The alignment a *declarator* asks for: its own `aligned` attribute, or the one a typedef
    /// it names carries.
    ///
    /// **Both spellings arrive as the same attribute.** The parser rewrites `_Alignas(N)` into
    /// `aligned(N)`, so `aligned_attr` sees them alike — which is why the two spellings had
    /// identical symptoms and why one fix covers both.
    ///
    /// The typedef case is why this is not just `aligned_attr`: in `typedef int A
    /// __attribute__((aligned(16))); A x;` the *declarator* `A x` carries no attribute at all,
    /// and the alignment has to come from the name it uses.
    fn declared_align(&mut self, ty: TypeId) -> Option<u64> {
        // **Walk down the array wrappers.** An alignment specifier sits in the declaration
        // *specifiers*, so for `_Alignas(32) int a[4]` the attribute is on the `int` node while
        // the declaration's type node is the array `declarator_suffixes` built around it.
        // Asking only the outermost node found nothing and left the array at 4.
        //
        // Arrays only: an array of over-aligned elements is itself over-aligned, and its
        // element type is the same declaration. A pointer is not — `int *p` is a pointer
        // whatever `int`'s alignment is — so the walk stops at anything else.
        let mut best: Option<u64> = None;
        let mut node = ty;
        loop {
            if let Some(a) = self.aligned_attr(node) {
                best = Some(best.unwrap_or(0).max(a));
            }
            if let TypeKind::Named(sym) = self.ast.ty(node).kind
                && let Some(&a) = self.out.typedef_aligns.get(&sym)
            {
                best = Some(best.unwrap_or(0).max(a));
            }
            match self.ast.ty(node).kind {
                TypeKind::Array { elem, .. } => node = elem,
                _ => break,
            }
        }
        best
    }

    /// Fold an **arithmetic** constant expression to a double (C 6.6p8).
    ///
    /// Deliberately *not* part of [`Self::eval`], which answers "is this an **integer**
    /// constant expression" (6.6p6) — there, a floating operand counts only as a cast's
    /// immediate operand, and recursing would wrongly accept `case (int)(1.5 + 2.5):`. 6.6p8
    /// is the weaker rule an enumerator and an initializer may use, and it is what `gnu11`
    /// applies when it folds `(u32)(1/0.01)` to 100.
    fn fold_arith(&mut self, expr: ExprId) -> Option<f64> {
        match self.ast.expr(expr).kind.clone() {
            ExprKind::Number(sym) => {
                let text = self.text(sym)?.to_owned();
                match float_literal(&text) {
                    Some((_, f)) => Some(f),
                    None => parse_int_literal(&text, &self.target).map(|v| v.v as f64),
                }
            }
            ExprKind::Unary { op, operand } => {
                let v = self.fold_arith(operand)?;
                match op {
                    UnOp::Minus => Some(-v),
                    UnOp::Plus => Some(v),
                    _ => None,
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (self.fold_arith(lhs)?, self.fold_arith(rhs)?);
                match op {
                    BinOp::Add => Some(a + b),
                    BinOp::Sub => Some(a - b),
                    BinOp::Mul => Some(a * b),
                    // A division by zero is undefined, not zero: refuse rather than invent.
                    BinOp::Div if b != 0.0 => Some(a / b),
                    _ => None,
                }
            }
            // A cast to an integer type truncates toward zero, as C requires, and the result
            // stays in the double channel so an enclosing expression keeps folding.
            ExprKind::Cast { ty, operand } => {
                let t = self.ty_of(ty);
                let v = self.fold_arith(operand)?;
                match self.out.types.get(t.0 as usize) {
                    Some(Ty::Int { .. }) => Some(v.trunc()),
                    Some(Ty::Float(_)) => Some(v),
                    _ => None,
                }
            }
            ExprKind::Ident(sym) => self.enumerators.get(&sym).copied().map(|v| v as f64),
            _ => None,
        }
    }

    /// The `n` of `__attribute__((aligned(n)))` on a syntactic type node.
    fn aligned_attr(&mut self, ty: TypeId) -> Option<u64> {
        self.aligned_attr_where(ty, false)
    }

    /// The same, restricted to attributes the **type specifier** carries.
    ///
    /// A record's own layout may only read these: `typedef struct S {…} T
    /// __attribute__((aligned(16)))` aligns `T`, not `struct S`, and the two attributes reach
    /// the same node. See `Attr::from_declarator`. Every other caller wants both, because
    /// `struct S x __attribute__((aligned(32)));` really does over-align `x`.
    fn definition_aligned_attr(&mut self, ty: TypeId) -> Option<u64> {
        self.aligned_attr_where(ty, true)
    }

    fn aligned_attr_where(&mut self, ty: TypeId, definition_only: bool) -> Option<u64> {
        let attrs = self.ast.ty(ty).attrs.clone();
        let mut best: Option<u64> = None;
        for a in attrs {
            if definition_only && (a.from_declarator || a.from_specifier) {
                continue;
            }
            if !matches!(
                self.text(a.name),
                Some("aligned" | "__aligned__" | "_Alignas")
            ) {
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
            // **At the enumeration's own width, not at `int`.** `type_expr` resolves an
            // enumerator through `enumerator_ty` — it was taught to when `enum Big { X =
            // 5000000000 }` lowered truncated — and this path kept its own answer, so a fold of
            // `A_BIT | B_BIT` on `1ULL << 32` overflowed 32 bits and reported a defect in code
            // gcc compiles silently. VPP's virtio feature bits are exactly that shape.
            //
            // The fallback stays `int` for an enumerator whose enumeration this context never
            // saw defined: 6.4.4.3p2's rule is the right guess when there is nothing better.
            ExprKind::Ident(sym) => self.enumerators.get(&sym).copied().map(|v| {
                let (bits, signed) = self
                    .out
                    .enumerator_ty(sym)
                    .and_then(|t| match self.out.types[t.0 as usize] {
                        Ty::Int { signed, bits } => Some((bits, signed)),
                        _ => None,
                    })
                    .unwrap_or((int_bits, true));
                IntVal { v, bits, signed }
            }),
            ExprKind::SizeofType(ty) => {
                let t = self.ty_of(ty);
                // **The constant-folding path needs its own check**, because an array bound never
                // reaches `type_expr`: `ty_of` folds the length by calling `eval` directly. So
                // `int a[sizeof(enum E)]` returned `None` here and the length became
                // `ArrayLen::Vla` — a file-scope VLA, silently, from an expression that is not
                // variable at all. Diagnosing at the incompleteness rather than at the missing
                // size keeps the message about the cause.
                if is_incomplete(&self.out, t) {
                    self.error(span, "`sizeof` applied to an incomplete type");
                    return None;
                }
                let n = size_of_ty(&self.out, &self.target, t)?;
                Some(self.size_t(n as i128))
            }
            ExprKind::AlignofType(ty) => {
                let t = self.ty_of(ty);
                let n = align_of_ty(&self.out, &self.target, t)?;
                Some(self.size_t(n as i128))
            }
            // **`sizeof` and `_Alignof` of an *expression* are constant expressions too**
            // (C 6.5.3.4), whenever the operand is not a variable-length array. Only the
            // `…Type` spellings had arms here, so `sizeof(int)` folded and `sizeof(1)` did not,
            // which made `enum { N = sizeof buf };` — ordinary C — silently wrong.
            //
            // **The operand is not evaluated**, only typed: `sizeof(1 / 0)` is a size, not a
            // division. So this asks `type_expr` rather than `eval`, and discards any complaint
            // typing produced, because a diagnostic about computing the operand is not about an
            // expression that never computes it.
            //
            // The typing is reused when the main pass has already done it, which avoids pushing a
            // duplicate node for every `sizeof` in the program and overwriting the expression's
            // `by_expr` entry that `top` and `conversions_of` answer from.
            //
            // **Measured, not assumed:** both of these choices are unfalsifiable today. Forcing
            // the lookup to `None` so the operand is always retyped, and separately keeping the
            // diagnostics instead of truncating them, each leave the whole 1473-test suite green.
            // So neither is protecting a right answer — the first saves duplicate work and the
            // second guards a report that no input currently produces. They are kept as the
            // cheaper and quieter of two equivalent behaviours, and that is the whole claim.
            ExprKind::SizeofExpr(inner) | ExprKind::AlignofExpr(inner) => {
                let want_size = matches!(node.kind, ExprKind::SizeofExpr(_));
                let id = match self.out.typed.top(inner) {
                    Some(id) => id,
                    None => {
                        let before = self.out.diagnostics.len();
                        let id = self.type_expr(inner);
                        self.out.diagnostics.truncate(before);
                        id
                    }
                };
                let t = self.out.typed.ty_of(id);
                let n = if want_size {
                    size_of_ty(&self.out, &self.target, t)?
                } else {
                    align_of_ty(&self.out, &self.target, t)?
                };
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
                    //
                    // **Truncated here, not left for `wrap`.** `1 << 31` does not fit a signed
                    // `int`, and C 6.5.7p4 does make it undefined — but `gcc -std=gnu11` and
                    // `gcc -std=gnu11 -pedantic-errors` both accept it silently, and every
                    // bit-flag enum in C is written this way. The first full-tree sweep found
                    // this one construct behind **871 of 884 findings** across VPP, reached
                    // through `vppinfra/elf.h`. Truncating matches gcc and keeps the value
                    // gcc computes; `wrap` would report a defect the project's own compiler
                    // does not have. An overflowing *addition* is still diagnosed.
                    BinOp::Shl => (
                        truncate(a.v.checked_shl(b.v.try_into().ok()?)?, a.bits, a.signed).v,
                        a,
                    ),
                    BinOp::Shr => (a.v.checked_shr(b.v.try_into().ok()?)?, a),
                    // **Comparisons ask about the converted operands, not the written ones.**
                    // Values are carried as mathematical `i128`, so `-1` stays −1 even once the
                    // usual arithmetic conversions have made the common type unsigned. C 6.3.1.8
                    // converts both operands to that type first, which turns `-1 < 1u` into
                    // `4294967295u < 1u`. `truncate` rather than `wrap`, because this conversion
                    // is defined and silent — `wrap` would report it as a signed overflow.
                    BinOp::Lt => ((cmp(a, r) < cmp(b, r)) as i128, bool_),
                    BinOp::Gt => ((cmp(a, r) > cmp(b, r)) as i128, bool_),
                    BinOp::Le => ((cmp(a, r) <= cmp(b, r)) as i128, bool_),
                    BinOp::Ge => ((cmp(a, r) >= cmp(b, r)) as i128, bool_),
                    BinOp::Eq => ((cmp(a, r) == cmp(b, r)) as i128, bool_),
                    BinOp::Ne => ((cmp(a, r) != cmp(b, r)) as i128, bool_),
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
                let taken = c.v != 0;
                let value = if taken {
                    match then {
                        // GNU `a ?: b` reuses the condition as the second operand.
                        Some(t) => self.eval(t)?,
                        None => c,
                    }
                } else {
                    self.eval(els)?
                };
                // **C 6.5.15p5: the result type is the usual arithmetic conversions of *both*
                // arms**, so the arm that was not taken still decides whether the result is
                // unsigned. It is evaluated only for its type.
                //
                // Two things that evaluation must not do. It must not contribute diagnostics —
                // `1 ? 7 : 1/0` is a constant expression gcc accepts, and the dead arm's division
                // by zero is not the expression's problem. And it must not be *required* to
                // succeed: when it cannot be evaluated, the taken arm's type is kept, which is no
                // worse than the answer given before this rule existed.
                let mark = self.out.diagnostics.len();
                let other = if taken {
                    self.eval(els)
                } else {
                    match then {
                        Some(t) => self.eval(t),
                        None => Some(c),
                    }
                };
                self.out.diagnostics.truncate(mark);
                Some(match other {
                    Some(other) => {
                        let ty =
                            usual_arithmetic(promote(value, int_bits), promote(other, int_bits));
                        truncate(value.v, ty.bits, ty.signed)
                    }
                    None => value,
                })
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
                // **`__builtin_offsetof` folds to a byte count**, which is all `offsetof` is:
                // on gcc `<stddef.h>`'s macro *is* this builtin. Lowering asks `const_of` for
                // it exactly as it does for `sizeof`, so folding here is the whole of the
                // value half.
                if self.text(name) == Some("__builtin_offsetof")
                    && let [tyarg, path] = args[..]
                    && let ExprKind::TypeName(t) = self.ast.expr(tyarg).kind
                {
                    let root = self.ty_of(t);
                    let (off, _) = self.offsetof_step(root, path)?;
                    let bits = (self.target.sizes.long_ * 8) as u32;
                    return Some(IntVal {
                        v: off as i128,
                        bits,
                        signed: false,
                    });
                }
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
                let t = self.ty_of(ty);
                // **A floating constant is an integer constant expression when it is the cast's
                // *immediate* operand** (C 6.6p6), and only then. `eval` answers in integers, so
                // a floating operand had made the whole expression unfoldable and
                // `case (int)1.5:` — legal C — was rejected as "not an integer constant
                // expression".
                //
                // **Immediate is the whole restriction.** `(int)-1.5` puts a unary operator
                // between the cast and the constant and is *not* an integer constant expression;
                // gcc agrees, and accepts it in an *initializer*, which needs only an
                // *arithmetic* constant expression (6.6p8). Reading the operand's spelling rather
                // than folding it is what keeps those apart — a recursive float folder here would
                // accept `(int)(1.5 + 2.5)` in a `case`, which C does not.
                if let Some(Ty::Int { signed, bits }) = self.out.types.get(t.0 as usize).cloned()
                    && let ExprKind::Number(sym) = self.ast.expr(operand).kind
                    && let Some(text) = self.text(sym).map(str::to_owned)
                    // **No "is it an integer?" guard, because the two are disjoint.**
                    // `float_literal` answers `None` for every spelling `parse_int_literal`
                    // accepts — it needs a `.`, an exponent marker or an `f`, and none of those
                    // survive an integer parse. A guard here was written first and mutation
                    // could not falsify it; it is gone rather than left as a condition no test
                    // can reach.
                    && let Some((_, f)) = float_literal(&text)
                {
                    return Some(truncate(f as i128, bits, signed));
                }
                let v = self.eval(operand)?;
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
    ///
    /// ⚠️ **Advisory, because a value comes back either way.** The wrapped result is byte for
    /// byte what gcc and clang fold the same expression to — measured on
    /// `0x7fffffff + 65535`, which all three answer `-2147418114` and both compilers accept
    /// with a `-Woverflow` warning and exit 0. Emitting this at `Error` meant `chiero cir`
    /// refused an entire translation unit it had understood completely, so a file the
    /// project's own compiler builds could not be analysed at all.
    ///
    /// It stays a diagnostic: the program relies on undefined behaviour and a reader should
    /// be told. What changed is that being told no longer costs them the analysis.
    fn wrap(&mut self, raw: i128, ty: IntVal, span: Span) -> IntVal {
        let fitted = truncate(raw, ty.bits, ty.signed);
        if fitted.v != raw && ty.signed {
            self.advisory(span, "signed overflow in a constant expression");
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

/// An operand as the common type represents it, for comparison.
///
/// Separate from `wrap` because this conversion is defined and silent: converting `-1` to an
/// unsigned type is C 6.3.1.3p2 arithmetic, not the signed overflow `wrap` would report.
fn cmp(v: IntVal, common: IntVal) -> i128 {
    truncate(v.v, common.bits, common.signed).v
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
    // **At 128 bits there is nothing to mask, and the mask does not exist.** `i128` holds exactly
    // that range already, while `(1 << 127) - 1` overflows and panics — the clamp below turned
    // `__int128`'s 128 into 127 and walked straight into it.
    //
    // Latent until wave 349: `is_null_constant` used to short-circuit on the expression's *kind*
    // before calling `eval`, so no 128-bit expression was ever folded here. Widening that
    // predicate to C's actual rule made every `__int128` expression reach this function, and the
    // whole `__int128` fixture panicked at once.
    if bits >= 128 {
        return IntVal { v, bits, signed };
    }
    let bits = bits.clamp(1, 127);
    // Built in `u128` so a 127-bit mask is representable; `1i128 << 127` is the sign bit.
    //
    // **This and the early return above are alternatives, and mutation says so**: reverting either
    // one alone survives, and reverting both panics. The early return is what states the rule —
    // `i128` already *is* 128 bits — and the `u128` mask is what makes the clamped 127 case safe
    // if a later change reaches it. Neither is redundant with a caller; they are redundant with
    // each other, deliberately.
    let mask = ((1u128 << bits) - 1) as i128;
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
            FloatKind::F32 | FloatKind::Float32Ext => 4,
            FloatKind::F64 | FloatKind::Float64Ext | FloatKind::Float32xExt => 8,
            FloatKind::X87_80 | FloatKind::Float64xExt => 16,
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
        // An incomplete record has no size, which is what makes it incomplete. Callers that want
        // to *diagnose* incompleteness ask `is_incomplete` instead: a function type and a VLA
        // also have no size here and neither is an incomplete object type.
        //
        // **Also measured unobserved:** letting this answer `Some(0)` for an incomplete record
        // leaves the whole suite green, because every diagnostic is raised through
        // `is_incomplete` before a size is ever asked for. It is kept for the same reason as the
        // `Ty::Error` arm there — refusing to invent a number costs nothing and a placeholder's
        // zero would otherwise look like a real answer to a future caller that forgets to check.
        Ty::Record(r) => match a.records.get(r.0 as usize) {
            Some(rec) if rec.complete => Some(rec.size),
            _ => None,
        },
        Ty::Vector { elem, lanes, .. } => Some(size_of_ty(a, t, *elem)? * (*lanes as u64)),
        Ty::Error => None,
    }
}

/// Whether a type is incomplete in the sense C's constraints mean.
///
/// Deliberately **not** "has no size". A function type and a variable-length array both fail that
/// test and neither is an incomplete object type, so phrasing the checks that way would reject
/// `sizeof(vla)` and every function declaration. The two things that are incomplete here are a
/// named tag whose definition has not been seen, and `Ty::Error` — which is what an *anonymous*
/// undefined tag and a genuinely unresolvable type both become.
fn is_incomplete(a: &Analysis, ty: TyId) -> bool {
    match &a.types[ty.0 as usize] {
        // **Measured unreachable, and kept anyway.** Since a named undefined tag became a record,
        // nothing in the suite produces a `Ty::Error` that reaches a completeness check — forcing
        // this arm to `false` leaves all 1476 tests green. It stays because it *forwards* rather
        // than asserts: a poisoned type has no size either, and if one ever arrives here the cost
        // of this arm being absent is a missing diagnostic, which is the failure mode this whole
        // wave was about.
        Ty::Error => true,
        Ty::Record(r) => !a.records.get(r.0 as usize).is_some_and(|rec| rec.complete),
        _ => false,
    }
}

/// The placeholder a named tag gets before its definition is seen.
///
/// Zero-sized and byte-aligned so that any use which slips past a completeness check produces an
/// obviously wrong number rather than a plausible one. `is_union` is carried because it is known
/// from the spelling of the reference, and a union completed as a struct would be worse than
/// either.
fn incomplete_layout(is_union: bool) -> RecordLayout {
    RecordLayout {
        size: 0,
        align: 1,
        fields: Vec::new(),
        is_union,
        // **False for a placeholder.** The attribute lives on the definition, and a reference
        // seen before it must not claim transparency the definition may not grant.
        transparent: false,
        flexible_member: None,
        packed: false,
        complete: false,
        has_zero_width_bitfield: false,
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
            FloatKind::F32 | FloatKind::Float32Ext => 4,
            FloatKind::F64 | FloatKind::Float64Ext | FloatKind::Float32xExt => t.aligns.double_,
            FloatKind::X87_80 | FloatKind::Binary128 | FloatKind::Float64xExt => {
                t.aligns.long_double
            }
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
/// The value of a decimal float literal that is **integral**, when it is one.
///
/// `float_literal` answers in `f64`, which is eleven bits short of x87's significand — so
/// `4611686018427387905.0L` (2^62 + 1) would arrive already rounded and the `+ 1` gone before
/// anything could see it. This answers in the integer, so no precision is lost on the way out of
/// the front end.
///
/// **Integers only, and the restriction is what makes it exact rather than approximate.** General
/// decimal-to-binary rounding needs arbitrary precision and a tie-breaking rule; an integer needs
/// neither, because the digits *are* the value. `None` for anything else — a fraction, an exponent,
/// a value past `u64` — and the caller falls back to the `f64` path it already had. Returning `None`
/// rather than an approximation is what keeps the two paths honest about which one answered.
///
/// **Deliberately not an encoding.** This used to return x87 bits, and `chiero-sema` cannot reach
/// `chiero-cir` — sema runs *before* CIR exists, so 001 §4 puts the crate below it. That is the
/// layering saying what this function is: the question "is this literal an integer" is syntactic and
/// belongs here, and "what does 2^62 + 1 look like in eighty bits" is a target format and belongs
/// beside `FloatKind`.
pub fn integral_float_literal(text: &str) -> Option<u64> {
    let t = text.replace('\'', "");
    let lower = t.to_ascii_lowercase();
    if !lower.ends_with('l') || lower.starts_with("0x") {
        return None;
    }
    // `123.0L` is integral; `1.5L`, `1e3L` and `0x1p1L` are not. A trailing `.0` (or a bare `.`) is
    // the only fraction this accepts, because it contributes nothing.
    let digits = lower.trim_end_matches('l');
    let digits = match digits.split_once('.') {
        None => digits,
        Some((int, frac)) if frac.chars().all(|c| c == '0') => int,
        Some(_) => return None,
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut v: u64 = 0;
    for c in digits.chars() {
        v = v.checked_mul(10)?.checked_add(u64::from(c.to_digit(10)?))?;
    }
    Some(v)
}

pub fn float_literal(text: &str) -> Option<(FloatKind, f64)> {
    let t = text.replace('\'', ""); // C23 digit separators
    let lower = t.to_ascii_lowercase();
    // A hexadecimal *integer* is not a float, and `0x1p3` is. The exponent marker is `p`
    // for hex and `e` for decimal, so the presence of a dot or the right marker decides.
    let hex = lower.starts_with("0x");
    let looks_float = t.contains('.')
        || (hex && lower.contains('p'))
        || (!hex && lower.contains('e') && !lower.starts_with("0b"));
    // **The suffix decides the kind, and `ends_with` cannot read it.** `bf16`, `f32x` and `q` all
    // end in something other than `f` or `l`, so the old test called every one of them `double`.
    // `number_split` is the same scan `number_defect` uses, so a spelling the one accepts is a
    // spelling the other can classify.
    let suffix_kind =
        number_split(&t).and_then(|(_, at)| float_suffix_kind(&t[at..], LongDoubleKind::X87_80));
    let kind = if let Some(k) = suffix_kind {
        k
    } else if lower.ends_with('f') {
        FloatKind::F32
    } else if lower.ends_with('l') {
        // x87 long double. **The value this function returns is still an `f64`, and for an
        // `f80` literal that is not the value.** The comment here used to call that "a
        // narrowing this records rather than hides", which was false in both halves: nothing
        // recorded it, and the excuse attached — "arithmetic on it is a gap anyway" — stopped
        // being true the day wave 239 shipped multiplication, at which point the rounding
        // started producing wrong answers instead of imprecise ones.
        //
        // Callers that need the value take the digits instead, through
        // `decimal_float_parts` or `hex_float_parts`, and convert in `chiero_cir::fp` where
        // sixty-four significand bits and x87's exponent range both fit. What is left here is
        // the *kind*, which is what this function is for.
        FloatKind::X87_80
    } else {
        FloatKind::F64
    };
    if !looks_float && !(lower.ends_with('f') && !hex) {
        return None;
    }
    // **The digits are what precedes the suffix**, whatever the suffix turned out to be —
    // `trim_end_matches` of `f`/`l` leaves `16` on the end of `0.0f16` and parses it as 0.016.
    let digits = match number_split(&t) {
        Some((_, at)) => &t[..at],
        None => t.trim_end_matches(['f', 'F', 'l', 'L']),
    };
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
/// A hexadecimal float literal's mantissa and binary scale, exactly.
///
/// `hex_float` composes these into an `f64`, which is fifty-three bits — so a literal with more than
/// that lost the rest before anything could use it, and `FpTrunc`'s tie fixtures were measuring the
/// loss rather than the conversion (§9). The parts have no such limit: the digits are binary and the
/// `p` exponent is a power of two, so `mant × 2^scale` *is* the value.
///
/// **Not an encoding**, for the reason wave 233 established: `chiero-sema` runs before CIR exists, so
/// what the bits look like belongs beside `FloatKind` and what the literal *says* belongs here.
/// A decimal floating literal's significant digits and its power of ten, with nothing rounded.
///
/// **The decimal counterpart of [`hex_float_parts`], and it exists for the same reason.** A hex
/// literal's digits are already binary, so `hex_parts` can hand back a `u64` mantissa and a binary
/// scale that reconstruct the value exactly. A decimal literal cannot: `0.1` has no finite binary
/// expansion, and the rounding to sixty-four bits depends on digits a fixed-width mantissa has
/// already thrown away. So this returns the *digits themselves* and lets `chiero_cir::fp::from_decimal`
/// do the conversion, where the big-integer arithmetic it needs is allowed to live (001 §4 keeps
/// `chiero-sema` below `chiero-cir`, so the conversion cannot come the other way).
///
/// `1.25e-3` comes back as `("125", -5)`: the fraction digits join the integer ones and each one
/// costs a power of ten, which is the same accounting `hex_parts` does four bits at a time.
///
/// Returns `None` for a hex literal — those have their own function — and for anything whose
/// mantissa is not digits and a dot.
pub fn decimal_float_parts(text: &str) -> Option<(String, i32)> {
    let t = text.replace('\'', ""); // C23 digit separators
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("0x") {
        return None;
    }
    // **The suffix comes off with the shared scan, not with `trim_end_matches`.** This was the
    // third copy of the suffix grammar in this crate — wave 336 merged the other two after
    // `float_literal` parsed `0.0f16` as `0.016` — and it failed the same way one step later:
    // `1e-18f64x` kept `64x` on the exponent, `parse::<i32>` refused it, and the exact-digits
    // path silently fell back to `f64`'s fifty-three bits for a type that has sixty-four.
    let body = match number_split(&lower) {
        Some((_, at)) => &lower[..at],
        None => lower.trim_end_matches(['f', 'l']),
    };
    let (mant, exp) = match body.split_once('e') {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (body, 0),
    };
    let (int_part, frac_part) = mant.split_once('.').unwrap_or((mant, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    // Every fraction digit is one decimal place below the point, exactly as every hex fraction
    // digit is four binary places below it.
    let e = exp.checked_sub(i32::try_from(frac_part.len()).ok()?)?;
    Some((format!("{int_part}{frac_part}"), e))
}

pub fn hex_float_parts(text: &str) -> Option<(u64, i32)> {
    let t = text.replace('\'', "");
    let lower = t.to_ascii_lowercase();
    if !lower.starts_with("0x") {
        return None;
    }
    hex_parts(&lower)
}

fn hex_float(lower: &str) -> Option<f64> {
    let (m, scale) = hex_parts(lower)?;
    Some(m as f64 * 2f64.powi(scale))
}

fn hex_parts(lower: &str) -> Option<(u64, i32)> {
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
    // **Accumulated in a `u128`, because sixty-four significant bits need more than sixty-four
    // digits' worth of room.** `0x1.fffffffffffffffep0` is seventeen hex digits — sixty-eight bits as
    // an integer — and its value fits x87's significand exactly, because the low bit is a zero. A
    // `u64` accumulator overflowed and returned `None`, which turned the widest legal literal into a
    // refused function.
    let mut m: u128 = 0;
    for c in int_part.chars().chain(frac_part.chars()) {
        m = m
            .checked_mul(16)?
            .checked_add(u128::from(c.to_digit(16)?))?;
    }
    // C allows a sign on the exponent and `i32::from_str` accepts both, `+3` included — so there
    // is nothing to strip. I wrote a `strip_prefix('+')` here on the assumption that it did not,
    // and mutation showed the line was dead: removing it changed nothing, including for `0x1p+3`.
    let e: i32 = exp.parse().ok()?;
    // Every fraction digit is four binary places below the point.
    let mut scale = e.checked_sub(4i32.checked_mul(i32::try_from(frac_part.len()).ok()?)?)?;
    // **Trailing zeros are scale, not significance.** Shifting them out is exact — the value is
    // unchanged — and it is what lets a sixty-eight-bit integer with four trailing zeros be a
    // sixty-four-bit significand. A literal that still does not fit has more than sixty-four
    // significant bits and would need *rounding*, so it returns `None` and the caller declares the
    // gap rather than inventing a rule here.
    while m > u128::from(u64::MAX) && m.is_multiple_of(2) {
        m >>= 1;
        scale = scale.checked_add(1)?;
    }
    Some((u64::try_from(m).ok()?, scale))
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
    // **Nothing standard holds it.** 6.4.4.1p5 makes a constant with no representable type a
    // constraint violation, and gcc says so even where `__int128` exists — the extended type is
    // not among those an unsuffixed constant may take. Answering with a 128-bit value keeps the
    // rest of typing working; `number_too_large` is what makes it a report rather than a licence.
    Some(IntVal {
        v,
        bits: 128,
        signed: true,
    })
}

/// Whether this integer constant fits no standard integer type (C 6.4.4.1p5).
///
/// Asked of the *parsed value*, not of the spelling, because the answer depends on the suffix:
/// `18446744073709551615u` fits `unsigned long long` and the same digits without the `u` do not.
fn number_too_large(text: &str, target: &TargetConfig) -> bool {
    let Some(v) = parse_int_literal(text, target) else {
        return false;
    };
    v.bits > (target.sizes.long_ * 8) as u32
}

/// **What is wrong with a preprocessing number, if anything** (C 6.4.4).
///
/// Returns the message rather than reporting it, so this stays a pure function of the spelling —
/// every case below is decidable from the text alone — and so the same rules can be asked from
/// more than one place.
///
/// **A pp-number is not a constant.** `1z`, `018` and `0x` are all well-formed preprocessing
/// tokens; the grammar that rejects them is the one for *constants*, which is why this is asked
/// here, where a literal is given a value, and not in the lexer.
/// Where a preprocessing number's *suffix* begins, and whether what precedes it is floating.
///
/// **One scan, shared with [`number_defect`].** The two questions — "is this spelling legal" and
/// "what type does it name" — need exactly the same walk, and writing it twice is how `0.0bf16`
/// came to be diagnosed correctly by one and typed `double` by the other: `float_literal` decided
/// the kind with `lower.ends_with('f')`, which is false for `bf16`, `f32x` and `q` alike.
///
/// Returns `None` for a spelling the scan cannot make sense of, which the caller treats as "not a
/// number I can classify" rather than as a diagnosis — `number_defect` is what reports.
fn number_split(t: &str) -> Option<(bool, usize)> {
    let b = t.as_bytes();
    let lower = t.to_ascii_lowercase();
    let hex = lower.starts_with("0x");
    let bin = lower.starts_with("0b");
    let mut i = if hex || bin { 2 } else { 0 };
    let is_digit = |c: u8| {
        if hex {
            c.is_ascii_hexdigit()
        } else if bin {
            matches!(c, b'0' | b'1')
        } else {
            c.is_ascii_digit()
        }
    };
    let exp_marker: &[u8] = if hex { b"pP" } else { b"eE" };
    let start = i;
    let mut dotted = false;
    while i < b.len() && (is_digit(b[i]) || b[i] == b'.') {
        dotted |= b[i] == b'.';
        i += 1;
    }
    if i == start || (dotted && i == start + 1) {
        return None;
    }
    let mut floating = dotted;
    if i < b.len() && exp_marker.contains(&b[i]) {
        floating = true;
        i += 1;
        if i < b.len() && matches!(b[i], b'+' | b'-') {
            i += 1;
        }
        let e = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return None;
        }
    }
    Some((floating, i))
}

/// The `FloatKind` a floating suffix names (C 6.4.4.2p1, plus gcc's extended set).
///
/// **Two of these contradict their spelling**, which is why the mapping is probed rather than
/// derived: `f32x` is *eight* bytes — the `x` forms mean "wider than the unsuffixed one", not "as
/// wide as the number says" — and `f64x` is sixteen.
///
/// **`f32` and `f64` land on `F32` and `F64`, which conflates `_Float32` with `float`.** gcc keeps
/// them distinct, and so does C: `_Generic(0.0f32, float: …)` selects `default` there and `float`
/// here. That is not a defect this introduces — `B::ExtFloat` has mapped a declared `_Float32` to
/// `FloatKind::F32` since the type existed — and separating them needs a `FloatKind` variant that
/// reaches CIR and lowering. Recorded in §9; the sizes and alignments, which is what the corpus
/// depends on, are right either way.
fn float_suffix_kind(suffix: &str, long_double: LongDoubleKind) -> Option<FloatKind> {
    let x87 = match long_double {
        LongDoubleKind::X87_80 => FloatKind::X87_80,
        LongDoubleKind::Binary128 => FloatKind::Binary128,
        LongDoubleKind::Double => FloatKind::F64,
    };
    Some(match suffix.to_ascii_lowercase().as_str() {
        "" => FloatKind::F64,
        "f" => FloatKind::F32,
        "l" => x87,
        "f16" => FloatKind::Binary16,
        "bf16" => FloatKind::BFloat16,
        "f32" => FloatKind::Float32Ext,
        "f64" => FloatKind::Float64Ext,
        "f128" | "q" => FloatKind::Binary128,
        // `_Float32x` is `double`; `_Float64x` and `__ibm128` are whatever `long double` is here.
        // `_Float32x` is `double`'s width; gcc gives it `double`'s *type* too, unlike `_Float32`.
        "f32x" => FloatKind::Float32xExt,
        "f64x" => FloatKind::Float64xExt,
        "w" => x87,
        _ => return None,
    })
}

fn number_defect(text: &str) -> Option<String> {
    // C23 digit separators are stripped rather than judged — a declared divergence, see the
    // fixture. `parse_int_literal` does the same.
    let t: String = text.chars().filter(|c| *c != '\'').collect();
    let b = t.as_bytes();
    let lower = t.to_ascii_lowercase();
    let hex = lower.starts_with("0x");
    let bin = lower.starts_with("0b");
    let mut i = if hex || bin { 2 } else { 0 };

    // **The digit set and the exponent marker go together.** In a hexadecimal constant `e` is a
    // digit and `p` introduces the exponent; everywhere else `e` does. Splitting on the wrong one
    // makes `0x1e` look like an exponent with no digits.
    let is_digit = |c: u8| {
        if hex {
            c.is_ascii_hexdigit()
        } else if bin {
            matches!(c, b'0' | b'1')
        } else {
            c.is_ascii_digit()
        }
    };
    let exp_marker: &[u8] = if hex { b"pP" } else { b"eE" };

    let start = i;
    let mut dotted = false;
    while i < b.len() && (is_digit(b[i]) || b[i] == b'.') {
        dotted |= b[i] == b'.';
        i += 1;
    }
    if i == start || (dotted && i == start + 1) {
        return Some(format!("`{text}` has no digits"));
    }

    let mut floating = dotted;
    if i < b.len() && exp_marker.contains(&b[i]) {
        floating = true;
        i += 1;
        if i < b.len() && matches!(b[i], b'+' | b'-') {
            i += 1;
        }
        let e = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return Some(format!("`{text}` has an exponent with no digits"));
        }
    } else if hex && dotted {
        // **6.4.4.2p1: a hexadecimal floating constant has an exponent.** `0x1.8` is not a number
        // in C, because `p` is the only thing that says where the binary point went.
        return Some(format!("`{text}` needs a `p` exponent"));
    }

    // **An octal constant is `0` followed by *octal* digits.** Checked after the shape is known,
    // because `018.5` and `018e1` are decimal floating constants and perfectly legal — the digit
    // `8` is only a mistake when the leading `0` really is an octal prefix.
    if !hex
        && !bin
        && !floating
        && t.len() > 1
        && b[0] == b'0'
        && let Some(c) = t[1..i].chars().find(|c| !('0'..='7').contains(c))
    {
        return Some(format!(
            "invalid digit `{c}` in the octal constant `{text}`"
        ));
    }

    // **6.4.4.1p1 and 6.4.4.2p1: the suffix sets, and they are not the same.** An integer takes a
    // `u` and one of `l`/`ll` in either order; a float takes `f` or `l` alone. `ll` may not mix
    // case, which is the part a shorter rule gets wrong.
    let suffix = &t[i..];
    let valid = if floating {
        // C11's two, plus **gcc's extended floating suffixes, which the corpus uses**: VPP writes
        // `0.0f16` and every one of its headers reaches it. They are refused under
        // `-pedantic-errors` and accepted in the GNU mode the corpus is compiled with, so they
        // belong with `0b101` among this project's declared extensions.
        //
        // **Recognising a suffix is not the same as typing it.** `float_literal` still knows only
        // `f`/`l`, so `0.0f16` is typed `double` rather than `_Float16` — a real gap this rule
        // surfaced and did not create, recorded in §9. Rejecting the literal here would replace a
        // wrong type with a wrong diagnostic on code that compiles.
        matches!(
            suffix.to_ascii_lowercase().as_str(),
            "" | "f"
                | "l"
                | "f16"
                | "f32"
                | "f64"
                | "f128"
                | "f32x"
                | "f64x"
                | "bf16"
                | "q"
                | "w"
                | "df"
                | "dd"
                | "dl"
        )
    } else {
        let rest = match suffix.find(['u', 'U']) {
            Some(k) => format!("{}{}", &suffix[..k], &suffix[k + 1..]),
            None => suffix.to_string(),
        };
        matches!(rest.as_str(), "" | "l" | "L" | "ll" | "LL")
    };
    if !valid {
        return Some(format!("invalid suffix `{suffix}` on `{text}`"));
    }
    None
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
    ///
    /// **Compared bare, because a qualifier has no representation.** Reading an object yields an
    /// unqualified value (C 6.3.2.1p2), so `const int` reaching an `int` is not a conversion at
    /// all — there is nothing to convert. Wave 328 made qualifiers part of type identity and this
    /// comparison started saying otherwise: `return table[i];` from a `const int[]` grew a
    /// `bitcast i32 to i32`, which is precisely the vacuous cast the paragraph above says must
    /// not exist. The golden caught it.
    fn convert(&mut self, node: TypedId, to: TyId, why: Conversion, span: Span) -> TypedId {
        if self.bare(self.out.typed.ty_of(node)) == self.bare(to) {
            return node;
        }
        // **Poison converts to nothing** (contract 20). A conversion *from* `Ty::Error` claims a
        // width change no operand has: the `Cast` pushed here declares a source type the value
        // does not carry, and 015 lowers it as a real instruction on top of whatever the
        // expression itself already emitted.
        //
        // `(long) __builtin_ctz(x)` produced `zext i32 -> i64` **twice** — once from lowering's
        // explicit-cast arm and once from this node — and the verifier rejected the function,
        // which discarded it. That was 29 of the first 35 VPP translation units, reached through
        // gcc's `avx512fintrin.h`, whose every masked intrinsic is
        // `(__mmask16) __builtin_ia32_ptestmd512 (…)`.
        //
        // **Typing the builtins instead was tried and is wrong.** A blanket implicit-`int` makes
        // `__builtin_alloca` return an integer and the `__builtin_ia32_*` family return scalars,
        // so `vppinfra`'s own headers began reporting "returning a value makes a pointer from an
        // integer". Real signatures are Tier 1/2 of the plan in HANDOFF §9; until then the type
        // is honestly unknown, and the fix is to stop *converting* the unknown.
        if matches!(
            self.out.types[self.out.typed.ty_of(node).0 as usize],
            Ty::Error
        ) {
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
                // **A malformed constant is reported here, where a pp-number is asked for a
                // value.** Without it the fall-through below is a wrong answer rather than a
                // missing diagnostic: neither parser accepts `018`, so it became a `double`.
                if let Some(why) = number_defect(&text) {
                    self.error(span, why);
                } else if number_too_large(&text, &self.target) {
                    self.error(
                        span,
                        format!("the integer constant `{text}` is too large for any integer type"),
                    );
                }
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
                if let Some(text) = self.text(*spelling).map(str::to_owned) {
                    // **A character constant's escapes are bounded by its *element*, which is
                    // not its type.** A plain `'x'` has type `int`, so `char_element` answers 32
                    // — correctly, for the type — while the constant is a sequence of *bytes* and
                    // `'\400'` is a violation. `string_element` is what knows the element width,
                    // and it agrees with `char_element` for every prefixed form; the two differ
                    // only in the plain case, which is exactly the case that matters here.
                    let ebits = strlit::string_element(&text).1;
                    //
                    // **And its quotes are `'`, not `"`.** `unquote` strips only double quotes,
                    // so asking it about `''` returns `''` — not empty, and the check said
                    // nothing. The two literal kinds need their own unquoting and share
                    // everything after it.
                    let content = strlit::unquote_char(&text);
                    if content.is_empty() {
                        self.error(span, "empty character constant");
                    } else {
                        self.check_literal_content(content, ebits, span);
                    }
                }
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
                // **C 6.4.5p5: two literals with different prefixes do not concatenate.** One
                // prefixed and one plain is fine — the result takes the prefix — so the question
                // is whether two *non-empty* prefixes disagree, and it is asked about the prefix
                // rather than the element type because `u8` and plain share an element and do
                // not share this rule.
                //
                // Reported once for the whole literal: `L"a" u"b" U"c"` is one bad literal, not
                // two bad joins (contract 20's spirit).
                let mut seen_prefix: Option<&'static str> = None;
                let mut prefix_clash = false;
                for f in fragments.iter() {
                    let Some(text) = self.text(f.spelling) else {
                        continue;
                    };
                    let p = strlit::string_prefix(text);
                    if p.is_empty() {
                        continue;
                    }
                    match seen_prefix {
                        Some(q) if q != p => prefix_clash = true,
                        _ => seen_prefix = Some(p),
                    }
                }
                if prefix_clash {
                    self.error(
                        span,
                        "string literals with different prefixes do not concatenate",
                    );
                }
                let (esign, ebits) = fragments
                    .first()
                    .and_then(|f| self.text(f.spelling))
                    .map(strlit::string_element)
                    .unwrap_or((self.target.char_signed, 8));
                let elem = self.intern(Ty::Int {
                    signed: esign,
                    bits: ebits,
                });
                // **Every fragment, at the element width the *first* one set.** Concatenated
                // literals share one element type, so `"\x1FF" L"x"` is checked at the width the
                // whole object ends up with — which is the width its escapes have to fit.
                for f in fragments.iter() {
                    if let Some(text) = self.text(f.spelling).map(str::to_owned) {
                        self.check_literal(&text, ebits, self.ast.expr(expr).span);
                    }
                }
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
                    // **`__func__` is declared by the language** (C99 6.4.2.2): the compiler
                    // behaves as if `static const char __func__[] = "name";` opened every
                    // function body. gcc's `__FUNCTION__` is the same object under an older
                    // spelling. It is resolved *after* the ordinary lookups, not before, so a
                    // program that declares its own `__func__` keeps it — the predefined one is
                    // what the name means when nothing else has claimed it.
                    .or_else(|| {
                        // **Three spellings, because gcc has three.** `__PRETTY_FUNCTION__` was
                        // left out and reported undeclared. In C++ it names the signature; in C
                        // gcc makes all three the unqualified function name, so only the spelling
                        // was ever at stake. Found by grepping for the shape §9 records — a guard
                        // naming one case of several — rather than by hitting it.
                        if !matches!(
                            self.text(*sym),
                            Some("__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__")
                        ) {
                            return None;
                        }
                        let n = self.text(self.current_fn?)?.len() as u64;
                        let ch = self.intern(Ty::Int {
                            signed: self.target.char_signed,
                            bits: 8,
                        });
                        // The length includes the terminator, which is what makes
                        // `sizeof(__func__)` the name's length plus one rather than a pointer
                        // width. It is an array, and only an array answers that correctly.
                        Some(self.intern(Ty::Array {
                            elem: ch,
                            len: ArrayLen::Fixed(n + 1),
                        }))
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
                        if !is_compiler_builtin(&n) && self.unknown_names.insert(*sym) {
                            self.error(span, format!("`{n}` was not declared"));
                        }
                        match self.builtin_signature(&n) {
                            Some(t) => t,
                            None => self.intern(Ty::Error),
                        }
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
                // **Typed before the operand is judged**, so it is typed exactly once — see the
                // note on `check_writable`. Both checks want the type as written, which is what
                // `type_expr` gives; neither decays.
                let inner = self.type_expr(*operand);
                let ity = self.out.typed.ty_of(inner);
                if matches!(op, UnOp::PreInc | UnOp::PreDec) {
                    // `++x` modifies its operand exactly as `x += 1` does.
                    self.check_writable(*operand, ity, "increment or decrement of");
                    let op = if matches!(op, UnOp::PreInc) {
                        "++"
                    } else {
                        "--"
                    };
                    self.check_incdec_pointee(ity, span, op);
                }
                let (ty, inner) = match op {
                    UnOp::AddrOf => {
                        // **A `register` object has no address** (C 6.7.1p6). Only the pair is an
                        // error: `register int x` used by value is ordinary, and `&x` on a
                        // non-`register` local is too.
                        if let ExprKind::Ident(n) = self.ast.expr(*operand).kind
                            && self.register_objects.contains(&n)
                        {
                            let text = self.text(n).unwrap_or("?").to_owned();
                            self.error(span, format!("address of `register` object `{text}`"));
                        }
                        // **`&` needs an lvalue, a function, or a `*`/`[]` result** (C 6.5.3.2p1).
                        // `not_an_lvalue` is wave 329's predicate, asked as a disqualification,
                        // and it already answers this: `*p` and `a[i]` are lvalues there, and a
                        // function designator is an `Ident`. Writing a second predicate is what
                        // wave 336 earned a rule against.
                        if self.not_an_lvalue(*operand) {
                            self.error(span, "cannot take the address of a value");
                        }
                        // **...and not of a bit-field**, which has no address to take: it shares
                        // a storage unit with its neighbours. Asked of the *member*, so a named
                        // member beside a bit-field stays addressable.
                        if let ExprKind::Member { base, field, .. } = self.ast.expr(*operand).kind
                            && self.is_bit_field(base, field)
                        {
                            let n = self.text(field).unwrap_or("?").to_owned();
                            self.error(span, format!("cannot take the address of bit-field `{n}`"));
                        }
                        (self.intern(Ty::Ptr(ity)), inner)
                    }
                    UnOp::Deref => {
                        let decayed = self.decay(inner, *operand);
                        let dty = self.out.typed.ty_of(decayed);
                        // **Say which mistake it is.** A non-pointer operand used to be given a
                        // poisoned pointee and then reported by the incompleteness check below as
                        // "dereference of a pointer to an incomplete type" — true of the pointee
                        // this code invented, false of the program, and it sends a reader looking
                        // for a missing `struct` definition. `Error` stays silent, per contract 20.
                        let pointee = match self.out.types[dty.0 as usize].clone() {
                            Ty::Ptr(p) => p,
                            Ty::Error => self.intern(Ty::Error),
                            _ => {
                                self.error(span, "the operand of `*` is not a pointer");
                                self.intern(Ty::Error)
                            }
                        };
                        // **`*p` needs the pointee to be a complete object type** (C 6.5.3.2p4):
                        // the result designates an object, and an object of unknown size is not
                        // one. `*p = *q` on an opaque pointer copies something whose extent
                        // nobody knows, which is the case that matters here rather than the
                        // diagnostic.
                        //
                        // Checked on the *pointee*, not on `p`: copying, comparing and converting
                        // the pointer itself never touches it, and those are what an opaque handle
                        // is for. `struct I **p` is unaffected — its pointee is a pointer, which
                        // is complete.
                        //
                        // A `void *` deref is left to the arm above: `Ty::Void` is not incomplete
                        // by `is_incomplete`'s reckoning, deliberately, since `void *p` and
                        // `sizeof(void)` are both legal and that predicate has other callers.
                        // **Poison is not an incomplete type**, and saying so was a cascade:
                        // `*nope` on an undeclared name reported the undeclared name *and*
                        // "dereference of a pointer to an incomplete type", which is contract 20's
                        // one-bad-thing-one-report broken by a message that was wrong anyway.
                        //
                        // **Not inside a `typeof` operand**, which is unevaluated: there is no
                        // result to designate an object and no size to want. See the note there.
                        if self.in_typeof == 0
                            && !matches!(self.out.types[pointee.0 as usize], Ty::Error)
                            && is_incomplete(&self.out, pointee)
                        {
                            self.error(span, "dereference of a pointer to an incomplete type");
                        }
                        (pointee, decayed)
                    }
                    // **`++x` and `--x` keep the operand's type** (C 6.5.3.1p2: the result is
                    // the value of the operand after incrementing, and `x += 1` gives that the
                    // operand's type back). Measured: gcc 13.3.0 answers `unsigned char` to
                    // `_Generic(--c, unsigned char: 1, int: 2)` for an `unsigned char c`, and
                    // the same for `c--`, which the `Postfix` arm already models.
                    //
                    // The catch-all below promotes, which is right for `+`, `-` and `~` and
                    // wrong here: it made `--f->refcnt` an `int` in the typed tree while
                    // lowering produced the object's eight bits, and the comparison in
                    // `svm_fifo_free` then declared `Int(32)` for an `Int(8)` operand. The
                    // promotion a *use* needs happens at the use.
                    UnOp::PreInc | UnOp::PreDec => (ity, inner),
                    UnOp::Not => {
                        // **Decayed before the question**, unlike the other unary operators
                        // here, which promote instead. `!a` on an array asks about the pointer
                        // it decays to — which is what makes it legal and always false.
                        let decayed = self.decay(inner, *operand);
                        self.require_scalar(decayed, *operand, "the operand of `!`");
                        (
                            self.intern(Ty::Int {
                                signed: true,
                                bits: int_bits,
                            }),
                            decayed,
                        )
                    }
                    _ => {
                        let promoted = self.promote_node(inner, *operand, span);
                        let pty = self.out.typed.ty_of(promoted);
                        // **C 6.5.3.3: `+` and `-` take an arithmetic operand, `~` an integer
                        // one.**
                        //
                        // Written against the *promoted* type, and that is **measured
                        // equivalent** to asking the operand's own: promotion maps `Int` to
                        // `Int`, and the decay inside it maps `Array` and `Func` to `Ptr`, so
                        // every operand lands on the same side of this test either way. Mutation
                        // says so — swapping `promoted` for `inner` survives the suite.
                        //
                        // The comment here first claimed the promotion was what made `~c` legal
                        // on a `char`. It is not: `char` is an integer type before promotion as
                        // well as after. `~c` is legal because the rule is about the *category*,
                        // and no conversion C applies here changes one.
                        //
                        // `Error` is exempt, per contract 20 — poison means something upstream
                        // has already reported.
                        let bad = match op {
                            UnOp::Minus | UnOp::Plus => !matches!(
                                self.out.types[pty.0 as usize],
                                Ty::Int { .. } | Ty::Float(_) | Ty::Vector { .. } | Ty::Error
                            ),
                            // **An *integer* vector**, as wave 371 requires for `&`, `^` and
                            // `|` — the same paragraph and the same element question. This arm
                            // was written before that one and took any vector, so `~` on a
                            // `float` vector passed where `&` on one did not.
                            UnOp::BitNot => match self.out.types[pty.0 as usize] {
                                Ty::Int { .. } | Ty::Error => false,
                                Ty::Vector { elem, .. } => {
                                    !matches!(self.out.types[elem.0 as usize], Ty::Int { .. })
                                }
                                _ => true,
                            },
                            _ => false,
                        };
                        if bad {
                            let what = match op {
                                UnOp::Minus => "unary `-`",
                                UnOp::Plus => "unary `+`",
                                _ => "`~`",
                            };
                            let needs = if matches!(op, UnOp::BitNot) {
                                "an integer"
                            } else {
                                "an arithmetic"
                            };
                            self.error(span, format!("{what} needs {needs} operand"));
                        }
                        // **A refused operand yields poison**, as the binary arms have done since
                        // wave 364. `(int)-s` on a record reported the operand *and* drew a cast
                        // complaint, because the result kept the record's type — one mistake,
                        // two sentences, and the second about a conversion that was never the
                        // fault.
                        let pty = if bad { self.intern(Ty::Error) } else { pty };
                        (pty, promoted)
                    }
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![inner],
                })
            }
            ExprKind::Postfix { operand, op } => {
                // Typed once, before the checks — see the note on `check_writable`.
                let inner = self.type_expr(*operand);
                let ty = self.out.typed.ty_of(inner);
                // `x++` and `x--` modify their operand exactly as `x += 1` does.
                self.check_writable(*operand, ty, "increment or decrement of");
                let name = if matches!(op, chiero_ast::PostfixOp::Inc) {
                    "++"
                } else {
                    "--"
                };
                self.check_incdec_pointee(ty, span, name);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![inner],
                })
            }
            ExprKind::Assign { op, lhs, rhs } => {
                // Typed once, before the checks — see the note on `check_writable`.
                let l = self.type_expr(*lhs);
                let lty = self.out.typed.ty_of(l);
                self.check_writable(*lhs, lty, "assignment to");
                // **An array is not assignable** (C 6.5.16p2: the left operand must be a
                // modifiable lvalue, and an array type is not one). Checked on the *left* operand
                // only: `a[0] = b[0]` is an element, and a `struct` holding an array assigns
                // whole — which is how one copies an array in C, so rejecting arrays wherever
                // they appear in an assignment would remove the only way to do it.
                let lhs_is_array = matches!(self.out.types[lty.0 as usize], Ty::Array { .. });
                if lhs_is_array {
                    self.error(span, "assignment to an array");
                }
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
                // **`p += n` is `p = p + n`**, so it needs the stride the same way. The binary arm
                // never sees a compound assignment — that is what the flag above exists for — so
                // the question is asked again here rather than shared with `+`.
                if pointer_displacement && self.incomplete_pointee(lty) {
                    self.error(span, "arithmetic on a pointer to an incomplete type");
                }
                // **And the offset is an integer** (C 6.5.6p2), for the same reason: `p += d` is
                // `p = p + d`, and wave 364 refuses `p + d`. The compound form spells the same
                // fault and never asked — the binary arm does not see a compound assignment,
                // which is exactly what the flag above exists to say.
                if pointer_displacement {
                    let rty = self.out.typed.ty_of(r);
                    if !matches!(self.out.types[rty.0 as usize], Ty::Int { .. } | Ty::Error)
                        && !is_incomplete(&self.out, rty)
                    {
                        self.error(span, "a pointer may only be offset by an integer");
                    }
                }
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
                // **`%=` is an integer operation** (C 6.5.5p2 via 6.5.16.2p1), unlike every other
                // arithmetic compound assignment: `d *= 2` on a `double` is fine and `d %= 2` is
                // not. Asked of the *left* operand's type and of the right one, since either
                // being floating is the violation.
                if matches!(op, Some(BinOp::Rem)) {
                    let rty = self.out.typed.ty_of(r);
                    if matches!(self.out.ty(lty), Ty::Float(_))
                        || matches!(self.out.types[rty.0 as usize], Ty::Float(_))
                    {
                        self.error(span, "`%` needs integer operands");
                    }
                }
                let bool_lvalue =
                    op.is_some() && matches!(self.out.ty(lty), Ty::Int { bits: 1, .. });
                // **`v *= 2` does not convert `2` to a vector.** The third instance of the
                // shape the two comments above describe, and the same tell each time: an
                // lvalue whose type the right operand must *not* be dragged to.
                //
                // gcc converts a scalar operand to the vector's **element** type and
                // broadcasts it, and there is no scalar-to-vector conversion for sema to
                // insert — the broadcast is a lowering step, one value used once per lane.
                // Coercing produced a node whose lowered form is an address, so the
                // elementwise multiply met a `Ptr` where its declared type said `Int(32)`.
                //
                // Unlike the `_Bool` case this needs no promotion either: `vec_arith`
                // converts the scalar to the element type itself, which is the conversion C
                // actually specifies here and is narrower than `int` for a `v8qi`.
                let vector_lvalue = op.is_some() && matches!(self.out.ty(lty), Ty::Vector { .. });
                let r = if pointer_displacement || vector_lvalue {
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
                } else if lhs_is_array {
                    // **Already reported, so do not convert to it** (contract 20). `int a[2]; a
                    // = 0;` drew "assignment to an array" and then "makes a pointer from an
                    // integer without a cast", the second describing a conversion to a target
                    // that cannot be assigned at all. gcc says one thing and stops.
                    r
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
                // **C11 6.3.2.1 before 6.5.15.** Both arms decay — an array to a pointer to its
                // first element, a function to a pointer to itself — and only then does the
                // conditional pick their common type. Skipping it made `sizeof(c ? a : b)` for two
                // `int[4]` report *sixteen* rather than eight, and `sizeof(c ? f : g)` refuse the
                // whole function because a `Ty::Func` has no size to report.
                //
                // The condition decays too, and for a different reason: `a ? x : y` on an array
                // tests the decayed pointer, which is what makes it always true.
                //
                // **That last one is unkillable by the current suite, and it stays anyway.**
                // Deleting `decay(c, ..)` passes all 1391 tests, because an array or a function
                // designator decays to a pointer that is never null — so the branch goes the same
                // way with or without it, and no test reads the typed node's *type*. It is not a
                // dead line in wave 241's sense: it executes and it changes what the typed AST
                // says. 001 §1 puts a reader at the other end of that AST, and telling them the
                // condition is an array when C says it is a pointer is wrong information whether
                // or not a test currently asks.
                let c = self.type_expr(*cond);
                let c = self.decay(c, *cond);
                self.require_scalar(c, *cond, "the condition of `?:`");
                let t = then.map(|t| {
                    let n = self.type_expr(t);
                    self.decay(n, t)
                });
                let e = self.type_expr(*els);
                let e = self.decay(e, *els);
                let ety = self.out.typed.ty_of(e);
                let ty = match t {
                    Some(t) => {
                        let tty = self.out.typed.ty_of(t);
                        self.common_type(tty, ety)
                    }
                    // **The elvis form's first operand is also its true arm** (GNU: `a ?: b` is
                    // `a ? a : b`), so the usual arithmetic conversions run across the *condition*
                    // and the else arm. Answering `ety` alone made `unsigned long ?: 1` an `int`,
                    // and lowering then allocated four bytes for a value that needs eight.
                    // `_Generic` against gcc 13.3.0 says `unsigned long`, and says `long` for
                    // both `int ?: long` and `unsigned ?: long`.
                    None => {
                        let cty = self.out.typed.ty_of(c);
                        self.common_type(cty, ety)
                    }
                };
                // **One `void` side is a GNU extension, not a constraint violation.** C 6.5.15p3
                // wants both operands `void` or neither; gcc accepts the mix — silently under
                // `gnu11`, as "ISO C forbids conditional expr with only one void side" under
                // `-pedantic-errors` — and VPP's `vtep.c` writes exactly it, an assignment in one
                // arm and a `void` call in the other.
                //
                // The result is `void`, which is what makes such an expression usable only as a
                // statement. Coercing instead produced "a `void` value is used where a value is
                // required" from `coerce`, which reads as a missing return type rather than as a
                // divergence — so the arms are left alone and the sentence is gcc's.
                let arm_ty = |cx: &Self, n: Option<TypedId>| {
                    n.map(|n| cx.out.typed.ty_of(n))
                        .map(|t| matches!(cx.out.types[t.0 as usize], Ty::Void))
                };
                let one_void = t.is_some()
                    && arm_ty(self, t) != arm_ty(self, Some(e))
                    && (arm_ty(self, t) == Some(true) || arm_ty(self, Some(e)) == Some(true));
                if one_void {
                    if self.dialect.pedantic {
                        self.advisory(
                            span,
                            "ISO C forbids a conditional expression with only one void side",
                        );
                    }
                    let ty = self.intern(Ty::Void);
                    let mut ops = vec![c];
                    if let Some(t) = t {
                        ops.push(t);
                    }
                    ops.push(e);
                    return self.push_typed(TypedNode::Value {
                        expr,
                        ty,
                        operands: ops,
                    });
                }
                let mut ops = vec![c];
                if let (Some(t), Some(te)) = (t, then.as_ref()) {
                    let t = self.coerce(t, ty, Conversion::Conditional, *te);
                    ops.push(t);
                }
                let e = self.coerce(e, ty, Conversion::Conditional, *els);
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
                // **`p[i]` is `*(p + i)`**, so the stride has to exist. Reported as *arithmetic*
                // rather than as a dereference: both fail here and only one is the reason —
                // blaming the deref sends a reader to the pointee's use instead of to its missing
                // definition.
                let bty0 = self.out.typed.ty_of(b);
                if self.incomplete_pointee(bty0) {
                    self.error(span, "arithmetic on a pointer to an incomplete type");
                }
                let i = self.type_expr(*index);
                let i = self.promote_node(i, *index, span);
                let bty = self.out.typed.ty_of(b);
                let ity = self.out.typed.ty_of(i);
                // **`a[b]` is `*(a + b)`, so either operand may be the pointer.** `0[p]` is
                // legal C and is written on purpose often enough to matter; a check that looked
                // only at the base rejected it.
                let elem_of = |cx: &Cx, t: TyId| match cx.out.types[t.0 as usize].clone() {
                    Ty::Ptr(p) => Some(p),
                    Ty::Vector { elem, .. } => Some(elem),
                    _ => None,
                };
                // **C 6.5.2.1p1: one operand is a pointer and the other an integer.** chiero
                // asked only about the subscripted value, so `a[d]` on a `double` and `a[p]` on a
                // pointer both passed — and the second is `*(a + p)`, which wave 364 made a
                // violation when written that way round, so the same mistake was reported or not
                // depending on spelling.
                //
                // Asked of the operand that is **not** the pointer, which is what keeps `0[a]`
                // legal: C lets either side be the pointer, and the arm below already commutes.
                // The non-pointer side is the *promoted* one, so every integer spelling —
                // `char`, `_Bool`, an enumeration — has already become `Ty::Int` and needs no arm.
                // `(None, None)` is the base arm's case below — "subscripted value is not an
                // array, pointer or vector" — and saying anything here would double it.
                let non_pointer_side = match (elem_of(self, bty), elem_of(self, ity)) {
                    (Some(_), None) => Some(ity),
                    (None, Some(_)) => Some(bty),
                    // **Both sides pointers**, which `a[p]` is: `*(a + p)` adds two pointers.
                    // Reported here rather than left to wave 364's additive rule, because that
                    // rule never sees a subscript.
                    (Some(_), Some(_)) => Some(bty),
                    (None, None) => None,
                };
                if let Some(other) = non_pointer_side
                    && !matches!(self.out.types[other.0 as usize], Ty::Int { .. } | Ty::Error)
                    && !is_incomplete(&self.out, other)
                {
                    self.error(span, "a subscript is an integer");
                }
                if let Some(elem) = elem_of(self, bty).or_else(|| elem_of(self, ity)) {
                    let id = self.push_typed(TypedNode::Value {
                        expr,
                        ty: elem,
                        operands: vec![b, i],
                    });
                    return self.set_top(expr, id);
                }
                let ty = match self.out.types[bty.0 as usize].clone() {
                    Ty::Ptr(p) => p,
                    // **A vector subscripts like an array without decaying like one.** gcc's
                    // `vector_size` allows `v[i]` on the vector itself, and C has no
                    // vector-to-pointer conversion, so `decay` above leaves it alone and this
                    // match fell to `Ty::Error`. Lowering reads an `Error` as a 32-bit integer,
                    // which is why the defect was invisible for `int` lanes and *nearly*
                    // invisible for `long` ones — a small `long` and its low four bytes are the
                    // same number. A `float` lane came back as its bit pattern.
                    Ty::Vector { elem, .. } => elem,
                    // **Subscripting something that is neither.** `int x = 5; x[0]` returned 5,
                    // because lowering reads an `Error` type as a 32-bit integer — so the engine
                    // answered a question that has no meaning. `Ty::Error` is excluded from the
                    // complaint, not from the poison: it means the base's type is already unknown
                    // and already reported, and contract 20 says one bad declaration is one
                    // diagnostic.
                    _ => {
                        if !is_incomplete(&self.out, bty) {
                            self.error(
                                span,
                                "subscripted value is not an array, pointer or vector",
                            );
                        }
                        self.intern(Ty::Error)
                    }
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: vec![b, i],
                })
            }
            ExprKind::Member { base, field, arrow } => {
                let arrow = *arrow;
                let b = self.type_expr(*base);
                let b = self.decay(b, *base);
                let bty = self.out.typed.ty_of(b);
                let base_ty = self.out.types[bty.0 as usize].clone();
                // **`.` takes a structure, `->` takes a pointer to one** (C 6.5.2.3p1–p2). 013
                // kept `arrow` syntactic and left the question here; until wave 361 nothing
                // asked it, so the two operators were interchangeable.
                //
                // Asked of the **decayed** base, which is what makes `struct S a[2]; a->a`
                // legal — the array is a pointer by the time the question is put. A rule
                // written against the base as spelled would reject it, and would also get both
                // typedef spellings backwards, since `typedef struct S *SP;` puts no `*` in
                // `SP p;`.
                let (rec, wrong_operator) = match &base_ty {
                    Ty::Record(r) => (Some(*r), arrow),
                    Ty::Ptr(p) => match self.out.types[p.0 as usize].clone() {
                        Ty::Record(r) => (Some(r), !arrow),
                        _ => (None, false),
                    },
                    _ => (None, false),
                };
                // **Only when the base is a record either way** (contract 20): `int x; x->a` is
                // one mistake and the complaint below names it. This arm is for the base that is
                // a structure, or a pointer to one, and reached with the other operator.
                if wrong_operator {
                    let (used, want) = if arrow {
                        ("`->`", "a pointer to a structure or union")
                    } else {
                        ("`.`", "a structure or union")
                    };
                    self.error(span, format!("{used} needs {want}"));
                }
                let found = rec.and_then(|r| self.out.find_field(r, *field).map(|f| f.ty));
                // **Two different mistakes, told apart.** A base that is not a structure at all
                // and a structure without that member are separate errors, and saying which is
                // most of a useful report. `Ty::Error` stays silent for the reason above — the
                // base's type is unknown and something else has already said so.
                if found.is_none() && !is_incomplete(&self.out, bty) {
                    let name = self.text(*field).unwrap_or("?").to_owned();
                    self.error(
                        span,
                        match rec {
                            Some(_) => format!("no member named `{name}`"),
                            None => format!(
                                "request for member `{name}` in something that is not a \
                                 structure or union"
                            ),
                        },
                    );
                }
                let ty = found.unwrap_or_else(|| self.intern(Ty::Error));
                // **A member of a qualified aggregate is qualified** (C 6.5.2.3p3). `s->m` where
                // `s` is a `const struct S *` has type `const int` even where `m` is declared
                // plain `int`, so `&s->m` is a `const int *` and cannot initialize an `int *`.
                // The qualifiers come from the *record* — from the pointee for `->`, from the
                // base itself for `.` — and not from the pointer, which is why this reads a
                // different type in each arm.
                let from_base = match &base_ty {
                    Ty::Ptr(p) => self.qual_of(*p),
                    _ => self.qual_of(bty),
                };
                let ty = self.add_quals(ty, from_base);
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
                let callee_ty = self.out.types[cty.0 as usize].clone();
                let (ret, params) = match callee_ty.clone() {
                    Ty::Func { ret, params, .. } => (ret, params),
                    Ty::Ptr(p) => match self.out.types[p.0 as usize].clone() {
                        Ty::Func { ret, params, .. } => (ret, params),
                        _ => (self.intern(Ty::Error), Vec::new()),
                    },
                    _ => (self.intern(Ty::Error), Vec::new()),
                };
                // **Only a callee whose type is concretely known to be uncallable.** This is the
                // arm where keying on `Ty::Error` would be a disaster: an undeclared callee types
                // as `Error`, and `__builtin_isnan` and the rest of 7.12.14 *are* undeclared —
                // nothing declares them and gcc knows them intrinsically. Complaining about the
                // poison would reject the float corpus.
                let callable = is_incomplete(&self.out, cty)
                    || matches!(callee_ty, Ty::Func { .. })
                    || matches!(&callee_ty, Ty::Ptr(p)
                        if matches!(self.out.types[p.0 as usize], Ty::Func { .. } | Ty::Error));
                if !callable {
                    // **Name it when it has a name.** gcc prints `called object 'q' is not a
                    // function`, and the name is the whole of what a reader needs — a call
                    // expression can have several operands and only one of them is the callee.
                    // `(*fp)()` and `a[i]()` have no name to give, so the sentence stands alone.
                    let named = match self.ast.expr(*callee).kind {
                        ExprKind::Ident(n) => self.text(n).map(|t| format!(" `{t}`")),
                        _ => None,
                    };
                    self.error(
                        span,
                        format!(
                            "called object{} is not a function or function pointer",
                            named.unwrap_or_default()
                        ),
                    );
                }
                // **C 6.5.2.2p1: a call produces a value, so its type must be one an object can
                // have.** A *declaration* returning an incomplete type is legal — nothing is
                // produced until there is a call — which is the same distinction wave 359 drew
                // for parameters, and it is why this is asked here and not at the declarator.
                //
                // **`void` needs no arm here.** `is_incomplete` deliberately excludes it — wave
                // 347 split `has_no_size` into `is_incomplete || Void` for exactly this reason —
                // so a `void` function is already silent. Writing `Ty::Void` beside `Ty::Error`
                // looked like the careful thing and was dead code: the mutant that removes it
                // survives, which is the signal to delete rather than to test.
                if callable
                    && !matches!(self.out.types[ret.0 as usize], Ty::Error)
                    && is_incomplete(&self.out, ret)
                {
                    let named = match self.ast.expr(*callee).kind {
                        ExprKind::Ident(n) => self.text(n).map(|t| format!(" `{t}`")),
                        _ => None,
                    };
                    self.error(
                        span,
                        format!(
                            "calling{} produces an incomplete type",
                            named.unwrap_or_default()
                        ),
                    );
                }
                // **A floating classification macro returns `int`.** 7.12.14's `isless`,
                // `isunordered` and the rest are macros over `__builtin_*`, which nothing
                // declares — gcc knows them intrinsically — so the generic path above types
                // the callee `Ty::Error` and the result with it. That is harmless for the
                // varargs builtins, whose type 020 §4.4.1 never consults, but these are
                // *values*: `__builtin_isnan(x) + 2` has to add an `int` to an `int`, and an
                // `Error` operand poisons the usual arithmetic conversions for the whole
                // expression.
                // **The arguments are not expressions and must not be typed.** The first is a
                // type name and the second a member designator whose identifiers name members,
                // not objects — typing it is what reported "`b` was not declared" and refused
                // every function that used `offsetof`.
                if self.is_offsetof(*callee) && args.len() == 2 {
                    // **The type name is still resolved, even though nothing here is typed.**
                    // `ty_of` records it in `syntactic_types`, which is where lowering reads the
                    // root of the designator from when it has to compute a runtime subscript.
                    // Without this the only writer was whichever *fold* got there first — and a
                    // fold inside `ConstEvaluator` writes to that evaluator's own analysis, so
                    // `return offsetof(T, m[i])` was resolvable and
                    // `s = { .x = offsetof(T, m[i]) }` was not. The same expression, answerable
                    // or not depending on the context that folded it.
                    //
                    // **Quietly**: the fold reports a bad type name where a reader expects it,
                    // and a second report here is contract 20 broken by bookkeeping.
                    if let ExprKind::TypeName(t) = self.ast.expr(args[0]).kind {
                        self.re_resolving(|cx| cx.ty_of(t));
                    }
                    // **A subscript's index *is* an expression, and must be typed.** The
                    // designator's identifiers name members and are rightly left alone, but
                    // `offsetof(T, key[(req->n) + 1])` evaluates `(req->n) + 1` — gcc does, and
                    // it is the whole reason the result is not a constant. Untyped, it reached
                    // lowering with no typed node, the member read became `undef`, and the
                    // offset was computed from nothing at all: CIR that verifies, runs, and
                    // answers with an unknown where C has a number.
                    self.type_designator_indices(args[1]);
                    let bits = (self.target.sizes.long_ * 8) as u32;
                    let ty = self.intern(Ty::Int {
                        signed: false,
                        bits,
                    });
                    // **`set_top` as well as `push_typed`.** Every other arm of `type_expr`
                    // reaches the `set_top` at its end by falling out of the match; an early
                    // `return` skips it, and a node that is pushed but not registered is
                    // invisible to `type_of`. `sizeof(__builtin_offsetof(...))` reads the
                    // operand's type from exactly there, so it was the one shape that noticed.
                    let id = self.push_typed(TypedNode::Value {
                        expr,
                        ty,
                        operands: Vec::new(),
                    });
                    return self.set_top(expr, id);
                }
                let ret = if self.is_fp_classify_builtin(*callee) {
                    let bits = (self.target.sizes.int_ * 8) as u32;
                    self.intern(Ty::Int { signed: true, bits })
                } else {
                    ret
                };
                // **The argument count must match the parameter list** (C 6.5.2.2p2), with two
                // exemptions that are the difference between a rule and a nuisance:
                //
                //   - a **variadic** function takes *at least* its named parameters, so `...`
                //     turns the equality into a minimum;
                //   - an **empty** parameter list is "unspecified", not "none" — `int g();`
                //     admits any call, and the parser cannot tell it from `int g(void)` anyway
                //     (wave 313 met the same limit from the declaration side).
                //
                // For this engine a short call is not just a diagnostic: the missing parameter
                // reads whatever the frame held, which the memory model then reports against the
                // *callee*.
                // **Through the pointer, as the arm above does.** A callee decays to a pointer
                // to function, so matching `Ty::Func` alone sees nothing — which is exactly what
                // the first version of this check did, silently.
                let signature = match callee_ty.clone() {
                    Ty::Func {
                        params,
                        variadic,
                        prototyped,
                        ..
                    } => Some((params, variadic, prototyped)),
                    Ty::Ptr(p) => match self.out.types[p.0 as usize].clone() {
                        Ty::Func {
                            params,
                            variadic,
                            prototyped,
                            ..
                        } => Some((params, variadic, prototyped)),
                        _ => None,
                    },
                    _ => None,
                };
                // **Checked when the callee is prototyped, not when it has parameters.** The
                // guard used to be `!formals.is_empty()`, which had to stand in for "specified"
                // while `f()` and `f(void)` produced the same empty list — and the cost was that
                // `int g(void); g(1);` went unreported, since the promise of *zero* parameters
                // was indistinguishable from no promise at all.
                //
                // `int g(); g(1,2,3);` is still legal and still silent: an unprototyped
                // declaration specifies nothing, so no call to it can have the wrong count.
                if let Some((formals, variadic, prototyped)) = signature
                    && prototyped
                {
                    let n = args.len();
                    let want = formals.len();
                    if n < want || (n > want && !variadic) {
                        let how = if n < want { "few" } else { "many" };
                        self.error(
                            span,
                            format!("too {how} arguments: expected {want}, got {n}"),
                        );
                    }
                }
                let mut ops = vec![c];
                // **As written, before the promotions.** A type-generic builtin's result is one
                // of its arguments' types, and the argument it names is the one in the source.
                let mut first_arg = None;
                for (i, a) in args.iter().enumerate() {
                    let node = self.type_expr(*a);
                    if i == 0 {
                        first_arg = Some(self.out.typed.ty_of(node));
                    }
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
                // **Resolved here because a table row cannot say "the type of operand 1".**
                let ret = self
                    .type_generic_builtin(*callee, args, first_arg, args.len())
                    .unwrap_or(ret);
                self.push_typed(TypedNode::Value {
                    expr,
                    ty: ret,
                    operands: ops,
                })
            }
            ExprKind::Cast { ty, operand } => {
                // **A compound literal wears a cast's clothes** (C 6.5.2.5). `(int){1}` and
                // `(int)x` are one `ExprKind` here, told apart only by whether the operand is an
                // initializer list — the same distinction wave 329 needed to make `(int){1}++`
                // an lvalue. That is why the initializer rules had never run on one: they are
                // driven from a *declaration*, and this is an expression.
                if matches!(self.ast.expr(*operand).kind, ExprKind::InitList(_)) {
                    // **The ordinary path still runs.** An earlier draft returned from here with
                    // its own typed node and lowering stopped answering `(struct S){9,1}.a` — the
                    // node a compound literal produces is what 015 reads to build the object, and
                    // it is built below. These checks are additions, not a replacement.
                    let t = self.ty_of(*ty);
                    // 6.5.2.5p1: the type name is a **complete object type**. `void` and a
                    // function type are refused by the cast path already; an incomplete record
                    // and a variably modified array are not.
                    let span = self.ast.ty(*ty).span;
                    let usable = if has_no_size(&self.out, t) {
                        self.error(span, "a compound literal needs a complete object type");
                        false
                    } else if matches!(
                        self.out.types[t.0 as usize],
                        Ty::Array {
                            len: ArrayLen::Vla(_),
                            ..
                        }
                    ) {
                        self.error(span, "a compound literal may not be variably modified");
                        false
                    } else {
                        true
                    };
                    // 6.5.2.5p3: the braced list initializes the object, so every rule that
                    // applies to `T x = {…};` applies here — excess elements, designators,
                    // string length, the lot.
                    //
                    // **Only when the type is usable.** An incomplete record has no members, so
                    // `check_init` calls every element excess and the reader gets two sentences
                    // for one fault. Contract 20, and wave 353's channel caught it on this
                    // wave's own new row — its first live catch.
                    if usable {
                        self.check_init(t, None, *operand);
                    }
                }
                let inner = self.type_expr(*operand);
                let inner = self.decay(inner, *operand);
                let t = self.ty_of(*ty);
                // **C 6.5.4p2/p4**, skipped for a compound literal — `(struct S){1}` shares this
                // arm and is a record target on purpose.
                // **A refused cast yields poison, not its written type.** `(int)(struct S)s.a`
                // is one mistake; typing the inner cast as `struct S` made the *outer* one report
                // that its operand was not scalar — contract 20's cascade. Poison propagates
                // instead, and every check in this file already stays silent on it.
                let t = if matches!(self.ast.expr(*operand).kind, ExprKind::InitList(_))
                    || self.check_cast(t, self.out.typed.ty_of(inner), span)
                {
                    t
                } else {
                    self.intern(Ty::Error)
                };
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
                let node = self.type_expr(inner);
                // **`sizeof` of an incomplete type has no answer** (C 6.5.3.4p1). Checked on the
                // operand's type rather than on its spelling, so `sizeof(*p)` is caught as well
                // as `sizeof(struct I)` — the dereference is where the size is actually asked
                // for, and it is the spelling that appears in real code.
                let operand_ty = self.out.typed.ty_of(node);
                // **Poison is not an incomplete type** (contract 20). `sizeof(nope)` on an
                // undeclared name reported the name *and* claimed its type was incomplete — two
                // sentences for one mistake, and the second about a type this code invented. The
                // same guard wave 339 put on `*`; this was the last site with it missing.
                if !matches!(self.out.types[operand_ty.0 as usize], Ty::Error)
                    && is_incomplete(&self.out, operand_ty)
                {
                    self.error(span, "`sizeof` applied to an incomplete type");
                }
                // **A function type has no size** (C 6.5.3.4p1), and this is a separate rule from
                // incompleteness rather than a case of it: `is_incomplete` deliberately answers
                // "may this be an object *yet*", and a function type never can be — it is not an
                // incomplete object type, it is not an object type at all.
                //
                // Asked without the decay that `sizeof(&g)` relies on, which is why `sizeof(g)`
                // is rejected while `sizeof(&g)` and `sizeof(void(*)(void))` stay legal: the
                // operand of `sizeof` is the one place C does *not* decay a function designator.
                if matches!(self.out.types[operand_ty.0 as usize], Ty::Func { .. }) {
                    self.error(span, "`sizeof` applied to a function type");
                }
                self.check_not_a_bit_field(inner, span, "sizeof");
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
            // **Alignment, not size.** The operand is typed and never evaluated, exactly as for
            // `sizeof`; only the number taken from its type differs.
            ExprKind::AlignofExpr(inner) => {
                let inner = *inner;
                self.type_expr(inner);
                self.check_not_a_bit_field(inner, span, "_Alignof");
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
            ExprKind::SizeofType(t) | ExprKind::AlignofType(t) => {
                // **Resolve the operand type, do not just intern the result's.** This arm used
                // to skip `ty_of` entirely and leave the answer to `const_eval`, which rebuilds
                // a throwaway context that sees only file-scope declarations — so
                // `sizeof(__typeof__(x))` for a local `x` had nothing that could answer it.
                // Resolving here records the node in `syntactic_types`, which is what lets a
                // consumer holding the AST `TypeId` ask what it became.
                let operand_ty = self.ty_of(*t);
                // **Poison is not an incomplete type** (contract 20). `sizeof(nope)` on an
                // undeclared name reported the name *and* claimed its type was incomplete — two
                // sentences for one mistake, and the second about a type this code invented. The
                // same guard wave 339 put on `*`; this was the last site with it missing.
                if !matches!(self.out.types[operand_ty.0 as usize], Ty::Error)
                    && is_incomplete(&self.out, operand_ty)
                {
                    self.error(span, "`sizeof` applied to an incomplete type");
                }
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
            // **C11 6.5.1.1.** The controlling expression is typed and never evaluated; the
            // selection is by type and is recorded for lowering.
            ExprKind::Generic {
                controlling,
                assocs,
            } => {
                let (controlling, assocs) = (*controlling, assocs.clone());
                // **Decayed, and that is the whole of "lvalue conversion" here.** C11 says
                // the controlling expression's type is taken as if it had undergone lvalue
                // conversion, so an array selects `int *` and a string literal `char *`.
                //
                // **Lvalue conversion also drops qualifiers, and now that has to be done.**
                // This comment used to say the opposite — that `const int` and `int` were
                // already one interned id, so `const` matching `int` was free. Wave 328 made
                // qualifiers part of type identity and that sentence became a wrong answer:
                // `_Generic(c, int: 1, default: 2)` with a `const int c` startedselecting
                // `default`. The differential channel caught it on the same run.
                //
                // **Stripped at the outermost level only**, which is what `bare` does and what
                // gcc agrees with: `const int *p` still selects a `const int *` association,
                // because the qualifier there is on the pointee and no conversion touches it.
                // An association naming `const int` therefore matches nothing at all.
                //
                // **And no promotion.** Every other context here would call `promote_node`;
                // doing so would make `(unsigned char)1` select `int`, which is the single
                // easiest thing to get wrong in this construct.
                let c = self.type_expr(controlling);
                let c = self.decay(c, controlling);
                let cty = self.out.typed.ty_of(c);

                // **No two associations may name compatible types** (C 6.5.1.1p2) — a rule
                // about the associations themselves, with nothing to do with the selector.
                // Detecting duplicates by "a second one matched" only ever finds the pairs the
                // selector happens to land on, so `_Generic(x, int: 1, int: 2)` with a `double`
                // selector went unreported.
                //
                // **Reported once**, and reported *here* rather than inside the loop, so a
                // program with a duplicate pair gets one sentence about the pair instead of one
                // per association (contract 20). gcc emits this *and* "matches multiple
                // associations" when both apply; the pair is the version that explains the
                // program without reference to what was selected, so it is the one kept.
                let named: Vec<(TyId, Span)> = assocs
                    .iter()
                    .filter_map(|a| a.ty.map(|t| (self.ty_of(t), self.ast.ty(t).span)))
                    .collect();
                let mut duplicated = false;
                'pairs: for (i, &(ti, _)) in named.iter().enumerate() {
                    for &(tj, sj) in &named[i + 1..] {
                        if self.compatible(ti, tj) {
                            self.error(sj, "two `_Generic` associations name compatible types");
                            duplicated = true;
                            break 'pairs;
                        }
                    }
                }

                let mut chosen: Option<(ExprId, TyId)> = None;
                let mut fallback: Option<(ExprId, TyId)> = None;
                let mut ops = vec![c];
                for a in &assocs {
                    // **Every arm is typed, selected or not.** They are not evaluated, but
                    // they must be valid expressions and gcc says so; typing them is what
                    // reports a nonsense arm that this selection happens to skip.
                    let node = self.type_expr(a.value);
                    let node = self.decay(node, a.value);
                    ops.push(node);
                    let vty = self.out.typed.ty_of(node);
                    match a.ty {
                        None => {
                            // C11 6.5.1.1p2: at most one `default`. Two is a constraint
                            // violation, and reporting it is the difference between refusing
                            // the program and quietly preferring one of the two.
                            if fallback.is_some() {
                                self.error(span, "a `_Generic` selection has two `default`s");
                            } else {
                                fallback = Some((a.value, vty));
                            }
                        }
                        Some(t) => {
                            let at = self.ty_of(t);
                            // **6.5.1.1p2: the association names a complete object type, and not
                            // a variably modified one.** The selection rules above are about
                            // *matching*; this is about whether the type named is one an object
                            // could have at all, and nothing was asking. `void`, an incomplete
                            // tag and a function type are the three shapes that reach here.
                            //
                            // Reported and then matched anyway: a rejected association that
                            // happens to match should not also produce "no association matches",
                            // which is contract 20's cascade wearing a different hat.
                            let ats = self.ast.ty(t).span;
                            if matches!(self.out.types[at.0 as usize], Ty::Void | Ty::Func { .. })
                                || (is_incomplete(&self.out, at)
                                    && !matches!(self.out.types[at.0 as usize], Ty::Error))
                            {
                                self.error(
                                    ats,
                                    "a `_Generic` association needs a complete object type",
                                );
                            } else if matches!(
                                self.out.types[at.0 as usize],
                                Ty::Array {
                                    len: ArrayLen::Vla(_),
                                    ..
                                }
                            ) {
                                self.error(
                                    ats,
                                    "a `_Generic` association may not be variably modified",
                                );
                            }
                            // **Compatibility, not identity** (C 6.5.1.1p2). This read
                            // `at == self.bare(cty)` under a comment saying interned ids *were*
                            // the compatibility test for everything `_Generic` can name — true
                            // until an enumeration tag joined the interning key, after which an
                            // `unsigned` association stopped matching an `enum E` selector and
                            // the `default` was taken silently.
                            if self.compatible(at, self.bare(cty)) {
                                // **Also 6.5.1.1p2: no two associations may name compatible
                                // types.** Both of these guards survived mutation until they
                                // were made to report, because only an *invalid* program can
                                // tell "first match wins" from "last match wins" — gcc
                                // rejects it, so the differential oracle can never grade it.
                                // The choice is between silently picking one and saying the
                                // program is wrong, and 020 §5 settles that.
                                //
                                // **Which arm the `else` keeps is still unobservable, and now
                                // permanently so**: the diagnostic makes 015 §7 refuse the
                                // function, so no answer is produced for either choice.
                                // Keeping the first is written as an `else` rather than an
                                // `.or()` so the code states a rule it cannot be tested on,
                                // instead of implying one that mutation would contradict.
                                if chosen.is_some() {
                                    if !duplicated {
                                        self.error(
                                            span,
                                            "two `_Generic` associations match the controlling \
                                             expression's type",
                                        );
                                    }
                                } else {
                                    chosen = Some((a.value, vty));
                                }
                            }
                        }
                    }
                }
                // `default` only when nothing else matched, wherever it was written — so a
                // `default` first in the list cannot shadow a later exact match.
                let sel = chosen.or(fallback);
                let ty = match sel {
                    Some((value, vty)) => {
                        self.out.generic_selections.insert(expr, value);
                        vty
                    }
                    None => {
                        self.error(
                            span,
                            "no `_Generic` association matches the controlling expression, and \
                             there is no `default`",
                        );
                        self.intern(Ty::Error)
                    }
                };
                self.push_typed(TypedNode::Value {
                    expr,
                    ty,
                    operands: ops,
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
        // **A `void` value cannot be an operand** (C 6.3.2.2: a `void` expression's value does
        // not exist). `*p != 0` on a `void *` and `v() != 0` on a `void` function both arrive
        // here, and neither goes through `coerce` — where the same rule already lived for
        // assignment, arguments and `return` — because binary operands keep their own types.
        //
        // **Producing a `void` value is fine; only using it is not.** `*p;` as a statement and
        // `(void)*p` are both legal C, and both stay legal because nothing asks this question of
        // them. That is the distinction the accepted half of wave 329's fixture pins, and it is
        // why this is checked at the operand rather than wherever a `void` value is made.
        for (node, e) in [(a, ae), (b, be)] {
            if matches!(
                self.out.types[self.out.typed.ty_of(node).0 as usize],
                Ty::Void
            ) {
                let span = self.ast.expr(e).span;
                self.error(span, "a `void` value is used where a value is required");
            }
        }
        let int_bits = (self.target.sizes.int_ * 8) as u32;
        let int_ty = self.intern(Ty::Int {
            signed: true,
            bits: int_bits,
        });
        // Named once, because two arms below key on it: the record complaint and the integer one.
        let bitwise_op = matches!(op, BinOp::BitAnd | BinOp::BitXor | BinOp::BitOr);
        // Shifts do **not** take the usual arithmetic conversions: each operand is
        // promoted on its own and the result has the left operand's type. Applying the
        // common type here would silently widen `x << 1` to whatever the shift count is.
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            let a = self.promote_node(a, ae, span);
            let b = self.promote_node(b, be, span);
            let ty = self.out.typed.ty_of(a);
            let bty = self.out.typed.ty_of(b);
            // **A record is named as one**, before the integer question: `s << 1` reported "a
            // structure or union is copied only from its own type" plus a cast complaint, and
            // neither said `<<`. The record arm below never sees a shift, because this branch
            // returns first — so the check comes here rather than the key being widened.
            if self.record_operand(ty, bty) {
                self.error(
                    span,
                    "a structure or union is not an operand of `<<`, `>>`, `&`, `^` or `|`",
                );
                let poison = self.intern(Ty::Error);
                return self.push_typed(TypedNode::Value {
                    expr,
                    ty: poison,
                    operands: vec![a, b],
                });
            }
            let what = if matches!(op, BinOp::Shl) {
                "`<<`"
            } else {
                "`>>`"
            };
            self.check_integer_operands(what, ty, bty, span);
            return self.push_typed(TypedNode::Value {
                expr,
                ty,
                operands: vec![a, b],
            });
        }
        if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
            let a = self.decay(a, ae);
            let b = self.decay(b, be);
            // **Both operands, not just the left.** `1 || s` is as wrong as `s || 1`, and a
            // short-circuit is a run-time notion that says nothing about the constraint.
            let what = if matches!(op, BinOp::LogAnd) {
                "an operand of `&&`"
            } else {
                "an operand of `||`"
            };
            self.require_scalar(a, ae, what);
            self.require_scalar(b, be, what);
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

        // **C 6.5.5p2: `*` and `/` take arithmetic operands, `%` takes integer ones.** Two rules
        // in one paragraph, and `double` is what separates them — it is arithmetic and not
        // integer, so `d / 2` is right and `d % 2` is wrong. Written as one rule either way, one
        // of those comes out backwards.
        //
        // Asked of the **promoted** operands, the same place the arm above asks about `~`: the
        // promotion maps every integer spelling — `char`, `_Bool`, an enumeration — onto `Int`,
        // so the rule needs no arm for any of them. A vector is arithmetic here, matching the
        // `-`/`+`/`~` rule beside it and gcc's `vector_size`. Poison stays silent (contract 20).
        // **The same question as `%`**, for the three bitwise operators (C 6.5.10–6.5.12p2).
        // Beside wave 362's arm rather than inside it, because the vector answers differ: any
        // vector may be multiplied and only an *integer* vector may be masked.
        // **Not when a record is involved**, which the arm below names specifically and better.
        // This check runs first because it lives beside wave 362's multiplicative one, so the
        // guard is here rather than an ordering change — `s ^ 1` would otherwise draw both.
        if bitwise_op && !self.record_operand(aty, bty) {
            let what = match op {
                BinOp::BitAnd => "`&`",
                BinOp::BitXor => "`^`",
                _ => "`|`",
            };
            self.check_integer_operands(what, aty, bty, span);
        }
        if matches!(op, BinOp::Mul | BinOp::Div | BinOp::Rem) {
            let integer_only = matches!(op, BinOp::Rem);
            // **An operand whose type is incomplete has already been reported** (contract 20).
            // `struct I; struct I x; x * 2;` said the declaration was unusable and then said `*`
            // needed arithmetic — the second sentence about a type the reader was already told
            // to fix. Wave 358's lesson, in a new rule one wave later.
            let ok = |cx: &Self, t: TyId| match cx.out.types[t.0 as usize] {
                Ty::Int { .. } | Ty::Vector { .. } | Ty::Error => true,
                Ty::Float(_) => !integer_only,
                _ => is_incomplete(&cx.out, t),
            };
            if !ok(self, aty) || !ok(self, bty) {
                let what = match op {
                    BinOp::Mul => "`*`",
                    BinOp::Div => "`/`",
                    _ => "`%`",
                };
                let needs = if integer_only {
                    "integer operands"
                } else {
                    "arithmetic operands"
                };
                self.error(span, format!("{what} needs {needs}"));
                // **A refused operand yields poison**, as the additive, comparison, bitwise and
                // unary arms have since waves 364 and 377. This one is the oldest of the family
                // and never got it, so `(int)(s * 2)` said three things: this, an initializer
                // complaint about a structure copy nobody wrote, and a cast complaint about a
                // conversion that was never the fault.
                let poison = self.intern(Ty::Error);
                return self.push_typed(TypedNode::Value {
                    expr,
                    ty: poison,
                    operands: vec![a, b],
                });
            }
        }

        // **A vector and a scalar keep their operands too**, for the reason the pointer branch
        // below gives. gcc's `vector_size` converts the scalar to the *element* type and
        // broadcasts it, and there is no scalar-to-vector conversion for sema to insert — the
        // broadcast is a lowering step, one value used once per lane.
        //
        // Coercing here did real damage rather than nothing: the literal in `x + 1` became a
        // conversion node whose lowered form is an *address*, so the elementwise `Add` had a
        // `Ptr` operand where its declared type said `Int(32)` and the verifier refused the
        // function. `x << 1` was correct throughout only because shifts return above without
        // ever asking for a common type.
        //
        // The result is the vector's type in either operand order. `1 + x` is the same
        // operation as `x + 1`, and `common_type`'s catch-all returns its *first* argument, so
        // without this branch the two spellings disagreed.
        let is_vec = |cx: &Cx, t: TyId| matches!(cx.out.types[t.0 as usize], Ty::Vector { .. });
        if is_vec(self, aty) || is_vec(self, bty) {
            let vty = if is_vec(self, aty) { aty } else { bty };
            // **A comparison is the one vector operator whose result is not its operand type.**
            // gcc gives it the same total size and lane count with the element replaced by a
            // *signed integer of the lane's width*, so `v4sf == v4sf` is a `v4si` and
            // `v2df == v2df` is a vector of two `long`. Every other operator returns `vty`
            // unchanged, which is why this branch could stop at that until now.
            //
            // The signedness is the result's, not the comparison's: the operands are still
            // compared at their own element type, so an `unsigned char` lane compares unsigned
            // and *yields* a signed lane holding 0 or -1.
            let ty = if matches!(
                op,
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge
            ) {
                self.vector_mask_ty(vty)
            } else {
                vty
            };
            return self.push_typed(TypedNode::Value {
                expr,
                ty,
                operands: vec![a, b],
            });
        }

        // **C 6.5.6p2–p3 and 6.5.9p2: a record is not an operand of `+`, `-` or a comparison.**
        // Asked before the pointer branch because neither operand is a pointer, so nothing below
        // would look at it — `s + 1` used to reach the conversion path and report "a structure or
        // union is copied only from its own type", which is true of nothing the program wrote.
        // Poison and incomplete types stay silent (contract 20).
        let comparison_op = matches!(
            op,
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
        );
        let bitwise_op = matches!(op, BinOp::BitAnd | BinOp::BitXor | BinOp::BitOr);
        if (matches!(op, BinOp::Add | BinOp::Sub) || comparison_op || bitwise_op)
            && self.record_operand(aty, bty)
        {
            {
                let what = if comparison_op {
                    "a structure or union is not comparable"
                } else if bitwise_op {
                    "a structure or union is not an operand of `<<`, `>>`, `&`, `^` or `|`"
                } else {
                    "a structure or union is not an operand of `+` or `-`"
                };
                self.error(span, what);
                // Poison, so an enclosing cast or condition does not add a second sentence
                // about the type this operator could not produce (contract 20).
                let poison = self.intern(Ty::Error);
                return self.push_typed(TypedNode::Value {
                    expr,
                    ty: poison,
                    operands: vec![a, b],
                });
            }
        }

        // Pointer arithmetic and comparisons keep their operands as they are: `p + n` has
        // no common type, and forcing one would turn a pointer into an integer.
        let is_ptr = |cx: &Cx, t: TyId| matches!(cx.out.types[t.0 as usize], Ty::Ptr(_));
        if is_ptr(self, aty) || is_ptr(self, bty) {
            // **Arithmetic on a pointer scales by the pointee's size, so the pointee must have
            // one.** Comparisons are excluded deliberately: `p == q` and `p < q` need no stride,
            // and two opaque handles are compared all the time. This is the check the
            // `size_of_ty(..).unwrap_or(1)` in `addr_of` was standing in for — badly, since one
            // byte is exactly the stride of a `char` and so the wrong answer looked like a right
            // one for any code that happened to use byte offsets.
            if matches!(op, BinOp::Add | BinOp::Sub) {
                for t in [aty, bty] {
                    if self.incomplete_pointee(t) {
                        self.error(span, "arithmetic on a pointer to an incomplete type");
                        break;
                    }
                }
                // **C 6.5.6p2–p3: what may sit beside a pointer.** `+` takes a pointer and an
                // integer in either order; `-` takes a pointer and an integer *in that order*,
                // or two pointers to compatible types.
                //
                // The other operand must be an **integer**, which is why `p + d` is refused —
                // the pointee's size scales an integer count, and there is no meaning for a
                // fractional one. `Ty::Error` is silent, and an incomplete pointee has already
                // been reported just above.
                let integer = |cx: &Self, t: TyId| {
                    matches!(cx.out.types[t.0 as usize], Ty::Int { .. } | Ty::Error)
                };
                let both = is_ptr(self, aty) && is_ptr(self, bty);
                let bad = if both {
                    // Two pointers: legal only for `-`, and only when the pointees agree. The
                    // compatibility question is `assignable`'s, exactly as for `p == q` below —
                    // a second copy of it would drift, and it is what makes qualifiers and
                    // typedefs not count.
                    matches!(op, BinOp::Add)
                        || (!self.assignable(aty, bty, false) && !self.assignable(bty, aty, false))
                } else if is_ptr(self, aty) {
                    !integer(self, bty)
                } else {
                    // The pointer is on the right, so `-` has an integer on the left: `1 - p`
                    // is not pointer arithmetic in any direction.
                    matches!(op, BinOp::Sub) || !integer(self, aty)
                };
                if bad {
                    let what = match (both, op) {
                        (true, BinOp::Add) => "two pointers cannot be added",
                        (true, _) => "subtracting pointers to incompatible types",
                        (_, BinOp::Sub) if !is_ptr(self, aty) => {
                            "an integer minus a pointer is not pointer arithmetic"
                        }
                        _ => "a pointer may only be offset by an integer",
                    };
                    self.error(span, what);
                }
            }
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
            // **Comparing two pointers needs them compatible** (C 6.5.9p2), and this does not go
            // through `coerce` — pointer operands keep their own types, which is what the arm
            // above exists to do. So the same question is asked here, in the one place where two
            // pointers meet without either being converted to the other.
            //
            // A null constant on either side is exempt, and so is `void *`, both by way of
            // `assignable` rather than a second copy of those rules.
            if comparison && is_ptr(self, aty) && is_ptr(self, bty) {
                let an = self.is_null_constant(ae);
                let bn = self.is_null_constant(be);
                if !self.assignable(aty, bty, bn) && !self.assignable(bty, aty, an) {
                    self.error(span, "comparison between incompatible pointer types");
                }
            }
            // **A pointer against a *non*-pointer** (C 6.5.9p2, 6.5.8p2). The guard above needs
            // both operands to be pointers, so this half was never examined at all: `p == i`,
            // `p == 1` and `p == c` were silent, and `p == b` on a `_Bool` was the entry that
            // made it visible — `assignable` exempts `_Bool` because a *conversion* to it is a
            // test against zero, and a comparison converts nothing.
            //
            // **Equality admits a null pointer constant and a relational operator does not.**
            // `p == 0` is legal and `p > 0` is a constraint violation; that pair is the whole
            // difference between the two paragraphs, which is why `equality` is asked separately
            // rather than folded into `comparison`.
            //
            // **But gcc only enforces the relational half pedantically**, and this arm reported
            // it flatly. Measured: `p > 0` is silent under `-std=gnu11` and is "ordered
            // comparison of pointer with integer zero [-Wpedantic]" under `-pedantic-errors`,
            // while `p > 1` is refused in *both*. So the null constant decides the dialect here,
            // not the legality — an earlier comment said "`p > 0` is not legal" and stopped,
            // which is right about the standard and wrong about the compiler the corpus is
            // measured against. Both of VPP's findings of this kind were the zero form.
            if comparison && is_ptr(self, aty) != is_ptr(self, bty) {
                let equality = matches!(op, BinOp::Eq | BinOp::Ne);
                let null = if is_ptr(self, aty) {
                    self.is_null_constant(be)
                } else {
                    self.is_null_constant(ae)
                };
                if !equality && null {
                    if self.dialect.pedantic {
                        self.advisory(span, "ordered comparison of a pointer with integer zero");
                    }
                } else if !(equality && null) {
                    // **Name the operand that is actually there.** This arm fires whenever
                    // exactly one side is a pointer, and said "an integer" of whatever the other
                    // side was — including a `double`, which is not one. One arm, two cases, one
                    // message; splitting the message is the fix, and widening it to "a
                    // non-pointer" would lose the one it already got right.
                    let other = if is_ptr(self, aty) { bty } else { aty };
                    let what = if matches!(self.out.types[other.0 as usize], Ty::Float(_)) {
                        "comparison between a pointer and a floating value"
                    } else {
                        "comparison between a pointer and an integer"
                    };
                    self.error(span, what);
                }
            }
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
    /// If `to` is a transparent union and `from` matches one of its members, which member.
    ///
    /// Only widens a *union* carrying the attribute, and only to a member it actually matches:
    /// the extension widens a parameter to its members, not to anything at all.
    fn transparent_member_for(
        &mut self,
        from: TyId,
        to: TyId,
        null_constant: bool,
    ) -> Option<(usize, Symbol)> {
        let Ty::Record(r) = self.out.types[to.0 as usize] else {
            return None;
        };
        let layout = self.out.layout(r);
        if !layout.transparent {
            return None;
        }
        // Cloned because `assignable` takes `&mut self`: the fields are needed after the
        // borrow ends, and a union's member list is short.
        let fields: Vec<(Option<Symbol>, TyId)> =
            layout.fields.iter().map(|f| (f.name, f.ty)).collect();
        // **No `from == to` guard: the member loop already excludes it.** A union type is
        // never `assignable` to one of its own member types, so passing the union itself finds
        // no member and is not recorded. A guard for it was written here and mutation could
        // not falsify it — dead, not defensive.
        fields.into_iter().enumerate().find_map(|(i, (name, ty))| {
            (self.assignable(from, ty, null_constant)).then_some((i, name?))
        })
    }

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
        // **A `void` call has no value to convert.** This sits in `coerce` rather than at each
        // use because `coerce` is precisely the set of places C *wants a value*: an initializer,
        // an argument, a return, an assignment. The places that do not want one — `v();` as a
        // statement, `(void)v();` — never call it, so they need no exemption, and adding one
        // would have been the way to get this wrong.
        if matches!(
            self.out.types[self.out.typed.ty_of(node).0 as usize],
            Ty::Void
        ) && !matches!(self.out.types[to.0 as usize], Ty::Void)
        {
            self.error(span, "a `void` value is used where a value is required");
        }
        // **One place for four rules.** Assignment, argument passing, `return` and initialization
        // all arrive here, and C states the same constraint for all four (6.5.16.1) — so the
        // check lives where they meet rather than at each of them.
        let from = self.out.typed.ty_of(node);
        let null_constant = self.is_null_constant(expr);
        // **`__attribute__((transparent_union))`**: a parameter of such a union takes any
        // member's type (gcc's extension). glibc declares `bind`/`connect`/`sendto` this way,
        // which is 67 of VPP's translation units — the largest single category of the first
        // full build.
        //
        // **The selected member is recorded, not just permitted.** gcc passes the argument as
        // the union's *first* member while the callee sees the union, so a later stage told
        // only "this was allowed" knows neither which member the value is nor that a
        // conversion happened. Answered here, once, where the types are.
        // **Record a pointee alignment change** (see `pointee_alignment_changes`). Done before
        // the compatibility verdict, because a compatible conversion is exactly the case that
        // produces no diagnostic and would otherwise leave no trace at all.
        let pointee = |cx: &Self, t: TyId| match cx.out.types[t.0 as usize] {
            Ty::Ptr(p) => Some(p),
            _ => None,
        };
        if let (Some(fp), Some(tp)) = (pointee(self, from), pointee(self, to))
            && let (Some(fa), Some(ta)) = (
                align_of_ty(&self.out, &self.target, fp),
                align_of_ty(&self.out, &self.target, tp),
            )
            && fa != ta
        {
            self.out.pointee_alignment_changes.insert(expr, (fa, ta));
        }
        if let Some((idx, name)) = self.transparent_member_for(from, to, null_constant) {
            self.out.transparent_union_args.insert(expr, (idx, name));
            return node;
        }
        if !self.assignable(from, to, null_constant) {
            let why = self.conversion_defect(from, to, why);
            self.error(span, why);
        }
        let id = self.convert(node, to, why, span);
        self.set_top(expr, id)
    }

    /// C11 §6.3.2.3: an integer constant expression with value 0 is a null pointer
    /// constant. Checked on the **written** expression, because `0` and a variable that
    /// happens to hold zero are different things.
    fn is_null_constant(&mut self, expr: ExprId) -> bool {
        // **Any integer constant expression worth zero** (C 6.3.2.3p3), which is what `eval`
        // already answers — it folds constant expressions and returns `None` for anything else.
        //
        // A guard on the expression's *kind* stood here, admitting only a `Number` or a `Cast`.
        // Its comment was half right: the question must be about the written expression rather
        // than about a variable that happens to hold zero, and `eval` is precisely that
        // distinction. What the guard added was a second, coarser idea of "written", and it
        // refused `1 - 1`, `'\0'`, `(1 ? 0 : 0)`, `sizeof(int) - 4` and an enumerator worth zero.
        //
        // The case the comment protects is still protected, and by the same call: `eval` answers
        // `None` for a variable, for a `const int` — C does not call that a constant expression —
        // and for `i - i` on a parameter.
        //
        // **Asked speculatively, so its diagnostics are discarded.** `eval` reports as it folds —
        // signed overflow, division by zero — and this predicate is a *question*, put to every
        // operand of every comparison and assignment. With the kind guard gone it began folding
        // expressions nobody asked it to evaluate, and a generated program was refused for
        // "signed overflow in a constant expression" that no constant context contained. The
        // same save-and-truncate the other speculative folds use.
        let before = self.out.diagnostics.len();
        let v = self.eval(expr).map(|v| v.v);
        self.out.diagnostics.truncate(before);
        v == Some(0)
    }

    /// **Which mistake this conversion is**, in gcc's terms rather than one sentence for all of
    /// them (023 §9).
    ///
    /// `assignable` has already said *no*; this says *why*, and the five answers are genuinely
    /// different things to fix. A discarded qualifier is the one worth separating hardest: the
    /// pointee types are compatible, so "incompatible types" sends a reader looking for a mismatch
    /// that is not there.
    ///
    /// **The context comes from `Conversion`, which `coerce` already carries.** Naming it costs
    /// nothing and is what tells a reader *which* of four things in a line is wrong — a call with
    /// three bad arguments used to produce three identical sentences.
    fn conversion_defect(&self, from: TyId, to: TyId, why: Conversion) -> String {
        let ctx = match why {
            Conversion::Argument => "passing an argument",
            Conversion::Return => "returning a value",

            _ => "initializing or assigning",
        };
        let f = self.out.types[from.0 as usize].clone();
        let t = self.out.types[to.0 as usize].clone();

        let pointee = |x: &Ty| match x {
            Ty::Ptr(p) => Some(*p),
            Ty::Array { elem, .. } => Some(*elem),
            _ => None,
        };
        let ptr_like = |x: &Ty| matches!(x, Ty::Ptr(_) | Ty::Array { .. } | Ty::Func { .. });
        let arith = |x: &Ty| matches!(x, Ty::Int { .. } | Ty::Float(_));

        // **A conditional gets its own sentence in every shape**, because the prefixes below are
        // verb phrases — "initializing or assigning", "passing an argument" — and there is no
        // verb here: the arms simply differ. Prefixing produced "a conditional expression's arms
        // differ makes a pointer from an integer without a cast", which a mutant found by
        // surviving every row that only exercised the pointer pair. gcc names the mismatch and
        // the construct, and says nothing about a conversion nobody asked for.
        if matches!(why, Conversion::Conditional) {
            return match (ptr_like(&f), ptr_like(&t)) {
                (true, true) => "pointer type mismatch in a conditional expression",
                (true, false) | (false, true) => {
                    "pointer/integer type mismatch in a conditional expression"
                }
                (false, false) => "type mismatch in a conditional expression",
            }
            .into();
        }

        // **The qualifier case first**, because it is the one the generic sentence describes
        // worst: everything about the types agrees except a `const` or a `volatile`.
        if let (Some(a), Some(b)) = (pointee(&f), pointee(&t))
            && self.bare(a) == self.bare(b)
        {
            let (qa, qb) = (self.qual_of(a), self.qual_of(b));
            let lost = if qa.const_ && !qb.const_ {
                "const"
            } else if qa.volatile_ && !qb.volatile_ {
                "volatile"
            } else {
                ""
            };
            if !lost.is_empty() {
                return format!("{ctx} discards the `{lost}` qualifier from the pointer's target");
            }
        }
        if matches!(f, Ty::Record(_)) || matches!(t, Ty::Record(_)) {
            return "invalid initializer: a structure or union is copied only from its own type"
                .into();
        }
        if ptr_like(&t) && arith(&f) {
            return format!("{ctx} makes a pointer from an integer without a cast");
        }
        if arith(&t) && ptr_like(&f) {
            return format!("{ctx} makes an integer from a pointer without a cast");
        }
        if ptr_like(&f) && ptr_like(&t) {
            return format!("{ctx} from an incompatible pointer type");
        }
        format!("{ctx} from an incompatible type")
    }

    /// C11 §6.3.1.8's usual arithmetic conversions.
    /// The type a comparison on `vty` yields: the same shape with signed integer lanes.
    ///
    /// **The lane's *width*, not its type.** A `v4sf` compares to a vector of four 32-bit signed
    /// integers, because the mask has to occupy the lane it describes — that is what lets
    /// `x & (x == y)` work at all, and it is why the result of comparing two `double`s is a
    /// `long` lane rather than an `int` one.
    ///
    /// The alignment travels unchanged. It is the *placement* alignment (see `Ty::Vector`), a
    /// property of the vector's size rather than of its element, and the result has the same
    /// size.
    fn vector_mask_ty(&mut self, vty: TyId) -> TyId {
        let Ty::Vector { elem, lanes, align } = self.out.types[vty.0 as usize].clone() else {
            return vty;
        };
        // **The lane's storage size, not `Ty::Int`'s `bits` field.** One rule for both integer
        // and floating elements, and it is the right one for each: the mask has to *occupy* the
        // lane, so what matters is how many bytes the lane takes and not how many bits its type
        // nominally has. A `_Bool` lane is not something gcc accepts anyway, and a size this
        // cannot compute leaves the type alone rather than inventing a width — lowering's own
        // guard then reports the gap instead of guessing.
        let Some(bytes) = size_of_ty(&self.out, &self.target, elem) else {
            return vty;
        };
        let m = self.intern(Ty::Int {
            signed: true,
            bits: (bytes * 8) as u32,
        });
        self.intern(Ty::Vector {
            elem: m,
            lanes,
            align,
        })
    }

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
            // **Two pointers meet at a pointee qualified with the qualifiers of both**
            // (C 6.5.15p6). This is the conditional operator's rule, and it is the reason the
            // rule exists: `src < dest ? src : dest` with a `const void *` and a `void *` is
            // ordinary C — VPP's `clib_memcpy` writes it — and there is no conversion for either
            // arm to undergo, because the answer is a *third* type that both convert to.
            //
            // Without it the two arms have no common type, `coerce` asks `assignable`, and the
            // `void *` arm is reported as discarding `const`. Nine corpus headers said so at
            // once, which is what a rule stated only in the negative looks like from the far
            // side: the wave added "may not discard a qualifier" and forgot "and here is where
            // qualifiers combine".
            (Ty::Ptr(pa), Ty::Ptr(pb)) if self.bare(*pa) == self.bare(*pb) => {
                let (qa, qb) = (self.qual_of(*pa), self.qual_of(*pb));
                let both = Qual {
                    const_: qa.const_ || qb.const_,
                    volatile_: qa.volatile_ || qb.volatile_,
                    restrict_: qa.restrict_ && qb.restrict_,
                };
                let bare = self.bare(*pa);
                let pointee = self.add_quals(bare, both);
                self.intern(Ty::Ptr(pointee))
            }
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
            // **C11 6.5.15p6, which only the conditional reaches.** A pointer never takes part in
            // the usual arithmetic conversions — those are defined on arithmetic types — so any
            // caller arriving here with one is the conditional, and these are its rules.
            //
            // A null pointer constant beside a pointer yields the *pointer* type. Chiero used to
            // fall into the catch-all below and answer with the integer, which made
            // `sizeof(c ? 0 : a)` four rather than eight — a wrong number rather than a refusal.
            //
            // Whether the integer really is a null pointer constant is not checked, and it does not
            // need to be: if it is not, the program is a constraint violation that a front end
            // rejects, so there is no correct answer being displaced.
            (Ty::Ptr(_), Ty::Int { .. }) => a,
            (Ty::Int { .. }, Ty::Ptr(_)) => b,
            // Two pointers: `void *` wins, because 6.5.15p6 says the result is a pointer to void
            // when either operand is one. Otherwise they must be compatible and either will do.
            //
            // **Only the `b` direction needs stating**, and mutation is what established that: the
            // catch-all below already returns `a`, so an arm returning `a` when the *first* operand
            // is the `void *` cannot change an answer. It was written, it survived the sweep, and
            // it is gone. Two arms would read as a symmetric rule and one of them would be a lie
            // about which line is doing the work.
            (Ty::Ptr(_), Ty::Ptr(y)) if matches!(self.out.types[y.0 as usize], Ty::Void) => b,
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

/// The order the usual arithmetic conversions use (C 6.3.1.8, F.10.11).
///
/// **An extended type outranks the standard type of its own width**, and is outranked by the next
/// wider standard one — `_Float32 + float` is `_Float32`, `_Float32 + double` is `double`. So the
/// ranks interleave rather than sitting beside each other, and a fix that only made the kinds
/// *distinct* would get every mixed expression wrong in one direction or the other.
fn float_rank(k: FloatKind) -> u8 {
    match k {
        FloatKind::Binary16 | FloatKind::BFloat16 => 0,
        FloatKind::F32 => 1,
        FloatKind::Float32Ext => 2,
        FloatKind::F64 => 3,
        FloatKind::Float32xExt => 4,
        FloatKind::Float64Ext => 5,
        FloatKind::X87_80 => 6,
        FloatKind::Float64xExt => 7,
        FloatKind::Binary128 => 8,
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
    /// How many scalars this type can absorb from a brace-elided initializer.
    ///
    /// `None` when the answer is not a fixed number — a flexible or unsized array, an incomplete
    /// record — in which case nothing is counted rather than something being guessed.
    ///
    /// **The unsized-array arm is measured unreachable**, and is kept rather than deleted. An
    /// array written `[]` *with* an initializer has its length inferred before this runs, so the
    /// target is always `Fixed` here; the arm exists for the shapes that reach `scalar_capacity`
    /// by recursion — a flexible array member — and those are caught by the record rule before
    /// capacity is consulted. Mutating it to `Some(1)` leaves the fixture green, so a reader
    /// changing it will not be caught.
    ///
    /// The **union** arm is not in that category, though wave 317 said it was: the case written
    /// for it never reached the file, and once it did the mutant died. Summing a union's members
    /// instead of taking the largest doubles its capacity.
    fn scalar_capacity(&self, ty: TyId) -> Option<u64> {
        match self.out.types[ty.0 as usize].clone() {
            Ty::Array {
                elem,
                len: ArrayLen::Fixed(n),
            } => self.scalar_capacity(elem)?.checked_mul(n),
            Ty::Array { .. } => None,
            Ty::Record(r) => {
                let rec = self.out.records.get(r.0 as usize)?;
                if !rec.complete {
                    return None;
                }
                let (fields, is_union) = (rec.fields.clone(), rec.is_union);
                let each: Option<Vec<u64>> =
                    fields.iter().map(|f| self.scalar_capacity(f.ty)).collect();
                let each = each?;
                // **A union holds one member at a time**, so its capacity is the largest of them
                // rather than their sum — and a flat list may only ever fill the first.
                if is_union {
                    each.into_iter().max().or(Some(0))
                } else {
                    each.into_iter().try_fold(0u64, |a, b| a.checked_add(b))
                }
            }
            Ty::Vector { lanes, .. } => Some(lanes as u64),
            Ty::Void | Ty::Func { .. } | Ty::Error => None,
            _ => Some(1),
        }
    }

    /// The length C 6.7.9p22 gives an array declared without one, from its initializer.
    ///
    /// **A string initializer counts its elements plus the terminator**, so `char s[] = "hi"` is
    /// three; a braced list counts its items, and a designator can push the length past the item
    /// count — `int a[] = {[4] = 1}` is five long, not one.
    ///
    /// `None` when the initializer says nothing about the length, which leaves the array flexible
    /// exactly as before rather than guessing a size.
    fn inferred_len(&mut self, init: ExprId) -> Option<u64> {
        match self.ast.expr(init).kind.clone() {
            ExprKind::Str { fragments } => {
                let n: usize = fragments
                    .iter()
                    .filter_map(|f| self.text(f.spelling).map(str::to_owned))
                    .map(|t| {
                        let (_, bits) = strlit::string_element(&t);
                        strlit::string_elements(&t, bits).len()
                    })
                    .sum();
                Some(n as u64 + 1)
            }
            ExprKind::InitList(items) => {
                // **The cursor, not the count.** A designator moves it, so the length is one past
                // the highest position written rather than the number of items — the same
                // distinction the excess-element rule needed in wave 314.
                let mut at = 0u64;
                let mut high = 0u64;
                for item in &items {
                    for d in &item.designators {
                        if let chiero_ast::Designator::Index(e) = d
                            && let Some(v) = self.eval(*e).map(|v| v.v)
                        {
                            at = v.max(0) as u64;
                        }
                    }
                    at += 1;
                    high = high.max(at);
                }
                Some(high)
            }
            _ => None,
        }
    }

    /// Follow a designator list into the object, reporting any component that names nothing
    /// (C 6.7.9p6, p7).
    ///
    /// Returns the position the **first** component selects — the outer cursor — and the type the
    /// **last** one reaches, which is what the item's value initializes. `{[1].y = 3}` selects
    /// element 1 of the array and hands back the type of `y`.
    ///
    /// **The descent is why this is a function and not a longer loop.** The existing walks looped
    /// over the components and kept the last index, so `[0][5]` set the *outer* cursor to 5 and
    /// asked the outer bound about it. Each component has to be asked of the type the previous one
    /// reached, and only the first one moves the cursor the caller keeps.
    fn resolve_designators(
        &mut self,
        ty: TyId,
        item: &chiero_ast::InitItem,
    ) -> Option<(Option<u64>, TyId)> {
        let span = self.ast.expr(item.value).span;
        let mut here = ty;
        let mut first: Option<u64> = None;
        for d in &item.designators {
            match d {
                chiero_ast::Designator::Index(e) => {
                    let Ty::Array { elem, len } = self.out.types[here.0 as usize].clone() else {
                        self.error(
                            span,
                            "an array designator names something that is not an array",
                        );
                        return None;
                    };
                    // **C 6.7.9p6: the index is non-negative**, asked here as well as on the
                    // single-designator path — the two clamps were written separately and both
                    // turned `[-1]` into `[0]`, a silently different program rather than a
                    // rejected one.
                    let raw = self.eval(*e).map(|v| v.v).unwrap_or(0);
                    if raw < 0 {
                        self.error(span, "initializer index is negative");
                        return None;
                    }
                    let v = raw.max(0) as u64;
                    if let ArrayLen::Fixed(n) = len
                        && v >= n
                    {
                        self.error(span, "initializer index is outside the array");
                        return None;
                    }
                    first.get_or_insert(v);
                    here = elem;
                }
                // gcc's `[a ... b] =` range designator. Its *lower* bound moves the cursor, the
                // same reading wave 350 gave a `case` range; whether the upper bound is in range
                // is a rule this wave does not add, so it is left alone rather than half-checked.
                chiero_ast::Designator::Range(lo, _) => {
                    let Ty::Array { elem, .. } = self.out.types[here.0 as usize].clone() else {
                        return None;
                    };
                    let v = self.eval(*lo).map(|v| v.v).unwrap_or(0).max(0) as u64;
                    first.get_or_insert(v);
                    here = elem;
                }
                chiero_ast::Designator::Field(f) => {
                    let Ty::Record(r) = self.out.types[here.0 as usize] else {
                        self.error(
                            span,
                            "a field designator names something that is not a struct or union",
                        );
                        return None;
                    };
                    let Some(fl) = self.out.find_field(r, *f) else {
                        let n = self.text(*f).unwrap_or("?").to_owned();
                        self.error(span, format!("no member named `{n}` to initialize"));
                        return None;
                    };
                    // **The index of the member that *shows* the name**, which for a promoted
                    // one is the anonymous member holding it. `position(name == f)` fell to
                    // `unwrap_or(0)` for those, putting the cursor on the first field — right by
                    // accident when the anonymous member came first, and wrong otherwise.
                    let pos = self.out.records[r.0 as usize]
                        .fields
                        .clone()
                        .iter()
                        .position(|x| self.field_shows(x, *f))
                        .unwrap_or(0) as u64;
                    first.get_or_insert(pos);
                    here = fl.ty;
                }
            }
        }
        Some((first, here))
    }

    /// Whether this list leaves out braces around an aggregate member, as C 6.7.9p20 permits.
    ///
    /// The signal is an aggregate element initialised by something that is not itself a list.
    /// When that happens the flat sequence is distributed across the sub-objects and positions no
    /// longer line up with items, so the counting rules above stop applying.
    /// Whether this item begins a **brace-elided run** for a slot of type `slot`.
    ///
    /// `elides_braces` asks the question of the whole list and of its *first* slot only, which is
    /// the shape wave 356 needed. C11 6.7.9p20 lets a run start anywhere: `struct { int a;
    /// struct { int b, c; } in; int d; } s = {1,2,3,4}` elides at the *second* member, and
    /// `int m[2][3] = {[1]=5,6}` elides inside the row a designator just selected. Both were
    /// reported as "excess elements" — valid C refused, and the refusal kept lowering's own
    /// (correct) walk from ever running on them.
    ///
    /// **A string is not a run.** `char s[4] = "ab"` initializes the array from the literal, and
    /// treating it as a run would skip the too-long check that belongs to it.
    ///
    /// **A compatible aggregate value is not a run either.** `{s, 3}` with `s` a whole
    /// `struct V`, and `{ .s = (U)a }` with `U` a union, initialize the slot directly — C11
    /// 6.7.9p13. The type is what says so, so the value is typed **quietly** to ask: the real
    /// typing still happens on the path taken, and doing it here without `re_resolving` reported
    /// every pedantic sentence in those operands twice.
    fn starts_elided_run(&mut self, slot: TyId, item: &chiero_ast::InitItem) -> bool {
        if !matches!(
            self.out.types[slot.0 as usize],
            Ty::Array { .. } | Ty::Record(_) | Ty::Vector { .. }
        ) {
            return false;
        }
        if matches!(
            self.ast.expr(item.value).kind,
            ExprKind::InitList(_) | ExprKind::Str { .. }
        ) {
            return false;
        }
        let node = self.re_resolving(|cx| cx.type_expr(item.value));
        let vty = self.out.typed.ty_of(node);
        self.out.unqualified(vty) != self.out.unqualified(slot)
    }

    fn elides_braces(&self, elem: TyId, items: &[chiero_ast::InitItem]) -> bool {
        // **A designator that descends is not elision, it is the opposite.** `{[0][5] = 1}` writes
        // a scalar into an aggregate element, which is exactly the signal below — and it is not
        // brace elision at all, because the item says *precisely* which sub-object it means. The
        // rule was swallowing every nested designator, which is why four of them went unchecked.
        // Wave 356 wrote this for `len() > 1`, and **one designator is the same argument**:
        // `{.c = 1}` says precisely which sub-object it means, so nothing is elided. Keyed on the
        // longer form, a record whose first member is an anonymous aggregate made every
        // single-designator initializer skip the check entirely — which is how `.c = 1` on a
        // record with no `c` went unreported. The same guard, found short twice.
        if items.iter().any(|i| !i.designators.is_empty()) {
            return false;
        }
        let aggregate = matches!(
            self.out.types[elem.0 as usize],
            Ty::Array { .. } | Ty::Record(_)
        );
        aggregate
            && items
                .iter()
                .any(|i| !matches!(self.ast.expr(i.value).kind, ExprKind::InitList(_)))
    }

    /// Check an initializer against the type it initializes (C 6.7.9).
    ///
    /// **Recursive, because C's rules are.** The target type drives the walk: an array
    /// distributes its list across elements, a record across members, and a scalar takes one
    /// value. Counting anything at the top level cannot work — `int a[2][2] = {1,2,3,4}` is legal
    /// with braces elided, so four items against an outer dimension of two is not an error.
    ///
    /// **Positions, not counts.** A designator moves the cursor, so `{[0]=1,[2]=3}` has two items
    /// and a highest index of 2. The cursor is what both the range check and the excess check
    /// read.
    /// `syn` is the *written* type, when the caller still has it. Sema's interned `TyId`
    /// cannot name a type — `short` and `int` share a width where `int` is 16-bit, and a
    /// typedef resolves away entirely — so a diagnostic that names one reads it from the AST
    /// instead of guessing from the width.
    fn check_init(&mut self, target: TyId, syn: Option<chiero_ast::TypeId>, init: ExprId) {
        let span = self.ast.expr(init).span;
        let ty = self.out.types[target.0 as usize].clone();
        // The written element type, for the array recursions below and for naming.
        let syn_elem = syn.and_then(|t| match self.ast.ty(t).kind {
            chiero_ast::TypeKind::Array { elem, .. } => Some(elem),
            _ => None,
        });

        // A string may initialise a character array directly, with the terminator dropped when it
        // is the only thing that does not fit: `char s[3] = "abc"` is legal and `"abcd"` is not.
        if let ExprKind::Str { fragments } = self.ast.expr(init).kind.clone()
            && let Ty::Array {
                len: ArrayLen::Fixed(n),
                elem,
            } = ty
        {
            // **C 6.7.9p14: the element and the literal must agree in width**, and that is the
            // whole test — signedness does not enter, which is why `char`, `signed char` and
            // `unsigned char` all take a plain literal. Refusing the unsigned spelling was this
            // rule written once for `char` with the sign forgotten, and it was reaching the
            // reader as "initializing or assigning from an incompatible pointer type": of an
            // array, from a literal, with no pointer written anywhere.
            let lit_bits = fragments
                .first()
                .and_then(|f| self.text(f.spelling))
                .map(|t| strlit::string_element(t).1)
                .unwrap_or(8);
            if let Ty::Int { bits, .. } = self.out.types[elem.0 as usize]
                && bits != lit_bits
            {
                // **Named by width and sign**, because there is no type printer in this crate
                // and gcc's sentence needs the element: `int`, `char`, `unsigned char`. The
                // three the rule is about are the ones a reader will meet.
                // **Named against the target's widths, not fixed ones.** `int` is 16 bits on
                // some targets and `long` is 32 on others, so a table keyed on 32 = `int`
                // reports a type the source does not contain. Widest-first, because several
                // C types share a width on any given target (`long` and `long long` on LP64)
                // and the narrowest match is the one a reader will have written.
                // **The written spelling first.** The AST keeps it — `TypeKind::Builtin`
                // and `TypeKind::Named` — and it is the only thing that can tell `short` from
                // `int` where they share a width, or report a typedef as the name the source
                // used. The width-based fallback below is for callers that no longer hold the
                // syntactic type (a nested member, a compound literal).
                let written = syn_elem.and_then(|e| match self.ast.ty(e).kind {
                    chiero_ast::TypeKind::Builtin(b) => Some(builtin_spelling(b).to_owned()),
                    chiero_ast::TypeKind::Named(sym) => self.text(sym).map(str::to_owned),
                    _ => None,
                });
                let sizes = &self.target.sizes;
                let name = written.unwrap_or_else(|| {
                    let by_width = match self.out.types[elem.0 as usize] {
                        Ty::Int { signed, bits } => {
                            let bytes = u64::from(bits / 8);
                            // Exact match, `int` first: several C types share a width on any given
                            // target, and a width cannot tell them apart. Only reached when the
                            // written type was not available.
                            let base = if bytes == sizes.int_ {
                                "int"
                            } else if bytes == 1 {
                                "char"
                            } else if bytes == sizes.short_ {
                                "short"
                            } else if bytes == sizes.long_ {
                                "long"
                            } else if bytes == sizes.long_long {
                                "long long"
                            } else if bytes < sizes.int_ {
                                "short"
                            } else {
                                "long long"
                            };
                            match (signed, base) {
                                (true, b) => b,
                                (false, "char") => "unsigned char",
                                (false, "short") => "unsigned short",
                                (false, "int") => "unsigned int",
                                (false, "long") => "unsigned long",
                                (false, _) => "unsigned long long",
                            }
                        }
                        _ => "that type",
                    };
                    by_width.to_owned()
                });
                self.error(
                    span,
                    format!("cannot initialise an array of `{name}` from a string literal"),
                );
                return;
            }
            let chars: usize = fragments
                .iter()
                .filter_map(|f| self.text(f.spelling).map(|t| t.to_owned()))
                .map(|t| {
                    let (_, bits) = strlit::string_element(&t);
                    strlit::string_elements(&t, bits).len()
                })
                .sum();
            if chars as u64 > n {
                self.error(span, "initializer-string is longer than the array");
            }
            return;
        }

        let ExprKind::InitList(items) = self.ast.expr(init).kind.clone() else {
            return;
        };

        match ty {
            Ty::Array { elem, len } => {
                // **Brace elision defeats counting, so counting stops.** `int a[2][2] =
                // {1,2,3,4}` is legal: the braces around each row may be omitted and the flat
                // list is distributed across them. Treating each top-level item as one element
                // then reports items 3 and 4 as out of range, which is the first thing this did.
                //
                // Detecting elision is easy — an aggregate element initialised by something that
                // is not a list — and distributing correctly is not, so this declines to answer
                // rather than answering wrongly. `int a[2][2] = {1,2,3,4,5}` is a declared miss.
                if self.elides_braces(elem, &items) {
                    // **Count scalars, not items.** Distributing a flat list across sub-objects is
                    // the hard part and is not needed to answer "is there one too many": the
                    // aggregate's total scalar capacity against the list's length does it.
                    //
                    // Only when the list is *entirely* flat. `{{1,2},3,4}` is legal and a scalar
                    // count cannot see where the braced item stops, so a mixed list is left
                    // unchecked — a narrower limit than wave 314's, not a different one.
                    // **Measured equivalent, and kept for what it says.** Every item fills at
                    // least one scalar, so a *legal* mixed list can never have more items than
                    // capacity — forcing this to `true` changes no answer. It is written because
                    // the count means something only for a flat list, and a reader should not
                    // have to rediscover that.
                    let flat = items
                        .iter()
                        .all(|i| !matches!(self.ast.expr(i.value).kind, ExprKind::InitList(_)));
                    if flat
                        && let Some(cap) = self.scalar_capacity(target)
                        && items.len() as u64 > cap
                    {
                        // **Which aggregate overflowed**, since the same walk serves all of
                        // them and the fix differs: an array takes a shorter list, a struct takes
                        // fewer members.
                        let what = match self.out.types[target.0 as usize] {
                            Ty::Array { .. } => "an array",
                            Ty::Record(r) if self.out.records[r.0 as usize].is_union => "a union",
                            Ty::Record(_) => "a struct",
                            Ty::Vector { .. } => "a vector",
                            _ => "a scalar",
                        };
                        self.error(span, format!("excess elements in {what} initializer"));
                    }
                    for item in &items {
                        self.type_expr(item.value);
                    }
                    return;
                }
                let bound = match len {
                    ArrayLen::Fixed(n) => Some(n),
                    _ => None,
                };
                let mut at = 0u64;
                for (i, item) in items.iter().enumerate() {
                    // **Whether an index was *written* is the difference between two mistakes.**
                    // The walk uses `at` as a cursor, and reporting the cursor told a reader of
                    // `int a[2] = {1,2,3};` to look for a designator the program does not
                    // contain. A designator that is out of range and a list that is simply too
                    // long are different things to fix, and gcc words them differently.
                    let mut designated = false;
                    // **A multi-component list is followed into the object**; a single one keeps
                    // the cheaper path, which also keeps the cursor semantics the excess-element
                    // rule depends on. `resolve_designators` reports its own faults and returns
                    // `None`, and one bad designator is one diagnostic (contract 20), so the item
                    // is skipped rather than checked again against the wrong type.
                    if item.designators.len() > 1 {
                        let Some((pos, inner)) = self.resolve_designators(target, item) else {
                            continue;
                        };
                        at = pos.unwrap_or(at);
                        // The same rule as the record arm: `{[0][1] = 1, 2}` designates a
                        // sub-object and then fills it from the enclosing list.
                        if self.starts_elided_run(inner, item) {
                            for rest in &items[i..] {
                                self.type_expr(rest.value);
                            }
                            return;
                        }
                        self.check_init(inner, None, item.value);
                        at += 1;
                        continue;
                    }
                    for d in &item.designators {
                        if let chiero_ast::Designator::Index(e) = d
                            && let Some(v) = self.eval(*e).map(|v| v.v)
                        {
                            // **C 6.7.9p6: the index is non-negative.** The upper bound was
                            // checked below and the lower was clamped by `max(0)` — which turned
                            // `[-1] = 1` into `[0] = 1`, a silently *different* program rather
                            // than a rejected one. The clamp stays so the cursor is usable, and
                            // now it is announced.
                            if v < 0 {
                                let dspan = self.ast.expr(item.value).span;
                                self.error(dspan, "initializer index is negative");
                            }
                            at = v.max(0) as u64;
                            designated = true;
                        }
                    }
                    if let Some(n) = bound
                        && at >= n
                    {
                        let dspan = self.ast.expr(item.value).span;
                        let msg = if designated {
                            "initializer index is outside the array"
                        } else {
                            "excess elements in an array initializer"
                        };
                        self.error(dspan, msg);
                        break;
                    }
                    // **A run may start here** — including inside the row a designator just
                    // selected, which is what `{[1]=5,6}` means. The rest of the list belongs to
                    // it, so the items are typed and the walk ends: distributing a flat list
                    // across sub-objects is the hard part `elides_braces` declines to do, and
                    // this declines it in the same direction rather than reporting an excess
                    // that is not there.
                    if self.starts_elided_run(elem, item) {
                        for rest in &items[i..] {
                            self.type_expr(rest.value);
                        }
                        return;
                    }
                    self.check_init(elem, syn_elem, item.value);
                    at += 1;
                }
            }
            Ty::Record(r) => {
                let fields = self.out.records[r.0 as usize].fields.clone();
                if let Some(f) = fields.first()
                    && self.elides_braces(f.ty, &items)
                {
                    for item in &items {
                        self.type_expr(item.value);
                    }
                    return;
                }
                let is_union = self.out.records[r.0 as usize].is_union;
                let mut at = 0usize;
                for (i, item) in items.iter().enumerate() {
                    let mut named = None;
                    // The same descent the array arm takes, for the same reason: this loop keeps
                    // the *last* component, so `.p.x` looked for `x` in the outer record.
                    //
                    // **Every designator takes it, not just the nested ones**: a single `.a` may
                    // name a member an *anonymous* member promotes, which the top-level scan
                    // below cannot see. `resolve_designators` asks `find_field`, which has
                    // recursed into unnamed records since long before this wave.
                    if !item.designators.is_empty() {
                        let Some((pos, inner)) = self.resolve_designators(target, item) else {
                            continue;
                        };
                        at = pos.unwrap_or(at as u64) as usize;
                        // **The designated object can itself be elided into**: `{.in = 1, 2}`
                        // names the member and then fills it from the enclosing list, so the run
                        // starts *at* the designator rather than after it.
                        if self.starts_elided_run(inner, item) {
                            for rest in &items[i..] {
                                self.type_expr(rest.value);
                            }
                            return;
                        }
                        self.check_init(inner, None, item.value);
                        at += 1;
                        continue;
                    }
                    for d in &item.designators {
                        if let chiero_ast::Designator::Field(f) = d {
                            match fields.iter().position(|x| x.name == Some(*f)) {
                                Some(i) => {
                                    at = i;
                                    named = Some(i);
                                }
                                None => {
                                    let n = self.text(*f).unwrap_or("?").to_owned();
                                    let dspan = self.ast.expr(item.value).span;
                                    self.error(
                                        dspan,
                                        format!("no member named `{n}` to initialize"),
                                    );
                                    named = Some(usize::MAX);
                                }
                            }
                        }
                    }
                    if named == Some(usize::MAX) {
                        continue;
                    }
                    // **A union takes one member, so a second positional item is excess** — but a
                    // designated one names its own member and only ever writes that.
                    if at >= fields.len() || (is_union && at > 0 && named.is_none()) {
                        let dspan = self.ast.expr(item.value).span;
                        let what = if is_union { "a union" } else { "a struct" };
                        self.error(dspan, format!("excess elements in {what} initializer"));
                        break;
                    }
                    if self.starts_elided_run(fields[at].ty, item) {
                        for rest in &items[i..] {
                            self.type_expr(rest.value);
                        }
                        return;
                    }
                    self.check_init(fields[at].ty, None, item.value);
                    at += 1;
                }
            }
            // **A vector initialises elementwise**, like an array of its lanes — `v4 v = {1,2,3,4}`
            // is four values, not four excess ones. It reaches the scalar arm otherwise, which is
            // where the whole vector corpus landed.
            Ty::Vector { elem, lanes, .. } => {
                for (i, item) in items.iter().enumerate() {
                    if i >= lanes as usize {
                        let dspan = self.ast.expr(item.value).span;
                        self.error(dspan, "excess elements in a vector initializer");
                        break;
                    }
                    self.check_init(elem, syn_elem, item.value);
                }
            }
            // **A scalar takes one value, braced at most once.** `int x = {1}` is legal and
            // `int x = {1,2}` is not; the inner value is checked against the scalar again, which
            // is what rejects `int x = {{1},{2}}` for the same reason.
            _ => {
                if items.len() > 1 {
                    self.error(span, "excess elements in a scalar initializer");
                }
            }
        }
    }

    /// Whether an expression *reads the value of an object*, which is the one thing a file-scope
    /// initializer may not do (C 6.7.9p4).
    ///
    /// **Asked as a disqualification rather than as a qualification.** The positive question — "is
    /// this a constant expression" — needs a complete account of address constants and gets it
    /// wrong by omission, which is why wave 314 narrowed it to "contains a call". The negative
    /// question has one answer: a name denoting an object whose value is read, or a call.
    ///
    /// Four things that look like reads and are not. An **array** or **function** name is an
    /// address. An **enumerator** is a constant. A **`const` object** is one gcc folds —
    /// `static const int c = 5; int g = c;` compiles even under `-pedantic-errors`, so rejecting
    /// it would reject real code, and wave 311's `read_only` set is what knows. And an object of
    /// **incomplete type** is skipped for contract 20's reason: its declaration has already been
    /// reported, and one bad declaration is one diagnostic however often the name appears.
    /// Whether `e` is certainly **not** an lvalue (C 6.3.2.1p1).
    ///
    /// **Asked as a disqualification**, like `reads_an_object` above and for the same reason: the
    /// positive question — "does this designate an object" — has to be right about every
    /// expression kind, and being wrong about one rejects correct code. This lists the kinds that
    /// cannot designate an object and treats everything else as an lvalue, so a shape nobody
    /// thought of is accepted rather than refused. Wave 303's rule.
    ///
    /// **A statement expression is not listed, and used to be.** `({ x; }) = 1` and
    /// `({ s; }).m = 1` both compile under `gcc -std=gnu11`; the only thing `-pedantic-errors`
    /// says about either is "ISO C forbids braced-groups within expressions", which is about the
    /// construct and not about the write. The entry was never measured and nothing pinned it.
    ///
    /// It went unnoticed while it was reachable only by writing to a braced group directly.
    /// Adding the `Member` arm below made `({ s; }).m = 1` recurse into it, which turned one
    /// unlikely over-rejection into a shape real code writes — the arm widened the divergence
    /// rather than creating it, and that is what made it visible.
    ///
    /// Two entries are not what they look like:
    ///
    ///   - **`Cast` is only disqualifying when its operand is not an initializer list.** A
    ///     compound literal `(int){1}` is an lvalue and this AST spells it as a cast, so the
    ///     blanket rule rejects `(int){1}++`, which gcc accepts.
    ///   - **An `Ident` naming an enumeration constant is not an lvalue.** `A++` is spelled
    ///     exactly like `x++` and no test of the expression's *kind* can tell them apart; the
    ///     enumerator table can.
    ///
    /// Parentheses need no entry because the parser discards them, which is what makes `(x)++`
    /// legal here for free — and is the reason the accepted half of wave 329's fixture pins it,
    /// since an AST that kept them would need this to see through.
    fn not_an_lvalue(&self, e: ExprId) -> bool {
        match self.ast.expr(e).kind.clone() {
            ExprKind::Number(_)
            | ExprKind::Char { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Postfix { .. }
            | ExprKind::Call { .. }
            | ExprKind::Cond { .. }
            | ExprKind::Assign { .. }
            | ExprKind::Comma { .. }
            | ExprKind::SizeofExpr(_)
            | ExprKind::SizeofType(_)
            | ExprKind::AlignofExpr(_)
            | ExprKind::AlignofType(_)
            | ExprKind::TypeName(_)
            | ExprKind::InitList(_) => true,
            // `*p` designates an object; every other unary operator produces a value.
            ExprKind::Unary { op, .. } => !matches!(op, UnOp::Deref),
            ExprKind::Cast { operand, .. } => {
                !matches!(self.ast.expr(operand).kind, ExprKind::InitList(_))
            }
            ExprKind::Ident(n) => self.enumerators.contains_key(&n),
            // **`s.m` is an lvalue exactly when `s` is; `p->m` always is** (C 6.5.2.3p3–4).
            // `->` dereferences, and a dereference designates an object however the pointer was
            // computed, which is why only the dot arm asks about its base.
            //
            // Added when cast-to-union began accepting `(U)x`: gcc refuses `((U)x).u = 1` under
            // `gnu11` too, and without this arm accepting the cast would also have made its
            // result assignable. The arm is not specific to that extension — `f().m = 1` on a
            // struct-returning call is the same rule, and was accepted here before.
            ExprKind::Member { base, arrow, .. } => !arrow && self.not_an_lvalue(base),
            _ => false,
        }
    }

    /// Whether **forming the address** of `e` requires reading an object (C 6.6p9).
    ///
    /// Distinct from [`reads_an_object`] because `&` inverts the question: `&x` reads nothing
    /// though `x` alone would, and `&*p` reads `p` though neither `&` nor `*` reads on its own.
    /// Each arm is one of 6.6p9's constructors:
    ///
    ///   - a **name** is an address constant outright;
    ///   - a **subscript** reads whatever its index reads, and its base only if that base is a
    ///     pointer *object* rather than an array;
    ///   - `.` keeps the question, `->` is a dereference and so hands the base to the ordinary
    ///     walk. **The `.` arm is measured unreached**: every `&s.m` this walk could see has
    ///     already been folded by `addr_of`, so the check short-circuits before asking, and
    ///     mutating that arm survives. It is written the correct way rather than the convenient
    ///     one because the `->` arm beside it is *not* unreached, and a reader comparing the two
    ///     needs the distinction to be the one C makes;
    ///   - **`&*E` cancels**: `*` yields the object `E` points at and `&` takes its address back,
    ///     so the pair reads exactly what `E` does. This is why `&*&x` and `&*(a+1)` are constants
    ///     and `&*p` is not.
    fn address_reads_an_object(&self, e: ExprId) -> bool {
        match self.ast.expr(e).kind.clone() {
            ExprKind::Ident(_) => false,
            ExprKind::Index { base, index } => {
                self.reads_an_object(index) || self.address_reads_an_object(base)
            }
            ExprKind::Member { base, arrow, .. } => {
                if arrow {
                    self.reads_an_object(base)
                } else {
                    self.address_reads_an_object(base)
                }
            }
            ExprKind::Unary {
                op: UnOp::Deref,
                operand,
            } => self.reads_an_object(operand),
            _ => self.reads_an_object(e),
        }
    }

    fn reads_an_object(&self, e: ExprId) -> bool {
        match self.ast.expr(e).kind.clone() {
            ExprKind::Call { .. } => true,
            ExprKind::Ident(n) => {
                if self.enumerators.contains_key(&n) || self.read_only.contains(&n) {
                    return false;
                }
                match self.values.get(&n) {
                    Some(t) => {
                        !is_incomplete(&self.out, *t)
                            && !matches!(
                                self.out.types[t.0 as usize],
                                Ty::Array { .. } | Ty::Func { .. }
                            )
                    }
                    // An unknown name has been reported already; saying so twice helps nobody.
                    None => false,
                }
            }
            // **`&E` asks a different question, so it gets its own walk.** The operand of `&` is
            // not read — that much was right — but *forming* the address can still require
            // reading something: `&p->m` reads `p`, `&a[i]` reads `i`. Returning `false` here
            // outright accepted both.
            ExprKind::Unary {
                op: UnOp::AddrOf,
                operand,
            } => self.address_reads_an_object(operand),
            // **A dereference is a read, whatever it wraps.** `*&x` descended into `&x` and hit
            // the arm above, so the walk answered "reads nothing" for an expression whose whole
            // purpose is to read. The `&*E` case does *not* come through here — it is handled in
            // `address_reads_an_object`, where `*` and `&` cancel.
            ExprKind::Unary {
                op: UnOp::Deref, ..
            } => true,
            ExprKind::Unary { operand, .. } | ExprKind::Cast { operand, .. } => {
                self.reads_an_object(operand)
            }
            ExprKind::Postfix { operand, .. } => self.reads_an_object(operand),
            ExprKind::Binary { lhs, rhs, .. } | ExprKind::Assign { lhs, rhs, .. } => {
                self.reads_an_object(lhs) || self.reads_an_object(rhs)
            }
            ExprKind::Comma { lhs, rhs } => self.reads_an_object(lhs) || self.reads_an_object(rhs),
            ExprKind::Index { base, index } => {
                self.reads_an_object(base) || self.reads_an_object(index)
            }
            ExprKind::Member { base, .. } => self.reads_an_object(base),
            ExprKind::Cond { cond, then, els } => {
                self.reads_an_object(cond)
                    || then.is_some_and(|t| self.reads_an_object(t))
                    || self.reads_an_object(els)
            }
            ExprKind::InitList(items) => items.iter().any(|i| self.reads_an_object(i.value)),
            _ => false,
        }
    }

    /// **`sizeof` and `_Alignof` do not apply to a bit-field** (C 6.5.3.4p1). It has no size of
    /// its own — it shares a storage unit with its neighbours — which is the same reason `&s.a`
    /// is refused, and the check is asked of the same `is_bit_field`.
    ///
    /// The operator is named in the message because the two arms are otherwise identical and a
    /// reader with both in one expression would have to guess which one was meant.
    fn check_not_a_bit_field(&mut self, operand: ExprId, span: Span, op: &str) {
        if let ExprKind::Member { base, field, .. } = self.ast.expr(operand).kind
            && self.is_bit_field(base, field)
        {
            let n = self.text(field).unwrap_or("?").to_owned();
            self.error(span, format!("`{op}` applied to bit-field `{n}`"));
        }
    }

    /// Whether an expression takes the address of an object with automatic storage duration.
    ///
    /// Walks the same shapes `addr_of` folds — `&x`, an array name, pointer arithmetic on
    /// either, and casts — because a rule that only looked at a bare `&x` would take
    /// `&a[1] + 0` and `(int *)&x` instead.
    ///
    /// **No initializer-list arm**, deliberately: `check_static_init` recurses into a list and
    /// asks this of each element, so a list never reaches here. One was written and a mutant
    /// that deleted it survived, which is what unreachable code looks like from the outside.
    fn addresses_an_automatic(&mut self, e: ExprId) -> bool {
        match self.ast.expr(e).kind.clone() {
            ExprKind::Unary {
                op: UnOp::AddrOf,
                operand,
            } => self
                .root_object(operand)
                .is_some_and(|n| self.automatic_objects.contains(&n)),
            ExprKind::Ident(sym) => self.automatic_objects.contains(&sym),
            ExprKind::Binary {
                op: BinOp::Add | BinOp::Sub,
                lhs,
                rhs,
            } => self.addresses_an_automatic(lhs) || self.addresses_an_automatic(rhs),
            ExprKind::Cast { operand, .. } => self.addresses_an_automatic(operand),
            _ => false,
        }
    }

    /// The name an lvalue designates, through members and indices: `a[2].b` roots at `a`.
    fn root_object(&mut self, e: ExprId) -> Option<Symbol> {
        match self.ast.expr(e).kind.clone() {
            ExprKind::Ident(sym) => Some(sym),
            ExprKind::Member { base, .. } => self.root_object(base),
            ExprKind::Index { base, .. } => self.root_object(base),
            _ => None,
        }
    }

    /// **C 6.7.5p2–p5: where `_Alignas` may appear and what it may name.**
    ///
    /// Two of the four rules apply to `__attribute__((aligned))` as well — a non-power-of-two and
    /// a parameter are refused for both spellings — and two do not: gcc takes the attribute on a
    /// `typedef` and takes it weakening a type's alignment, and VPP writes it on typedefs
    /// throughout `vppinfra`. That split is why the parser gives `_Alignas` its own attribute
    /// name, and why this function asks which spelling it is looking at.
    ///
    /// `_Alignas(0)` is **explicitly no effect** (p3), not an error, which is why the value is
    /// read from the attribute rather than from `declared_align` — the latter has already
    /// discarded a zero.
    fn check_alignment(&mut self, node: TypeId, resolved: TyId, ctx: StorageContext) {
        for a in self.ast.ty(node).attrs.clone() {
            let alignas = matches!(self.text(a.name), Some("_Alignas"));
            if !alignas && !matches!(self.text(a.name), Some("aligned" | "__aligned__")) {
                continue;
            }
            let span = a.span;
            // A parameter has no alignment to specify — both spellings, both modes.
            if matches!(ctx, StorageContext::Parameter) {
                let what = if alignas { "`_Alignas`" } else { "`aligned`" };
                self.error(span, format!("{what} is not allowed on a parameter"));
                continue;
            }
            // ...and `_Alignas` alone is refused on a `typedef`, where the attribute is legal.
            if alignas && matches!(ctx, StorageContext::NotAnObject) {
                self.error(span, "`_Alignas` is not allowed on a `typedef`");
                continue;
            }
            let Some(arg) = a.args.first().copied() else {
                continue;
            };
            let Some(n) = self.eval(arg).map(|v| v.v) else {
                continue;
            };
            // Zero is no effect; anything else must be a power of two.
            if n == 0 {
                continue;
            }
            if n < 0 || (n & (n - 1)) != 0 {
                self.error(span, "an alignment must be a power of two");
                continue;
            }
            // **And `_Alignas` may not weaken.** The attribute may — `__attribute__((aligned(1)))
            // int` is legal GNU C — so this arm is `_Alignas`-only too.
            if alignas
                && let Some(natural) = align_of_ty(&self.out, &self.target, resolved)
                && (n as u64) < natural
            {
                self.error(span, "an alignment may not be weaker than the type's own");
            }
        }
    }

    /// The names a member makes visible in its enclosing record: its own if it has one, and
    /// otherwise — an *anonymous* struct or union — the names of everything inside it, in turn.
    fn visible_names(&self, name: Option<Symbol>, ty: TyId) -> Vec<Symbol> {
        if let Some(n) = name {
            return vec![n];
        }
        let Ty::Record(r) = self.out.types[ty.0 as usize] else {
            return Vec::new();
        };
        let Some(layout) = self.out.records.get(r.0 as usize) else {
            return Vec::new();
        };
        layout
            .fields
            .clone()
            .iter()
            .flat_map(|f| self.visible_names(f.name, f.ty))
            .collect()
    }

    /// Whether an already-laid-out field makes `n` visible, directly or by promotion.
    fn field_shows(&self, f: &FieldLayout, n: Symbol) -> bool {
        self.visible_names(f.name, f.ty).contains(&n)
    }

    /// **A `switch` label is reached by a jump** (C 6.8.6.1p1), so it may not land inside the
    /// scope of a variably-modified declaration the jump did not run.
    ///
    /// The comparison is against the depth recorded when the `switch` began, not against another
    /// label's: the origin of this jump is the `switch` itself. That is the whole difference from
    /// wave 341's `goto` check, which compares two label positions — and it is why a *braced*
    /// case is legal: `case 1: { int a[n]; }` closes the scope before the next label, so the depth
    /// is back where it started by the time that label is reached.
    fn check_label_vla_scope(&mut self, stmt: StmtId, what: &str) {
        if let Some(&depth) = self.switch_vla_depth.last()
            && self.open_vla_scopes.len() > depth
        {
            let span = self.ast.stmt(stmt).span;
            self.error(
                span,
                format!("{what} enters the scope of a variably-modified declaration"),
            );
        }
    }

    /// Whether a declaration names an object with **static storage duration** (C 6.2.4).
    ///
    /// Everything at file scope has it, and inside a block only `static` and `extern` do. There
    /// is deliberately no `thread_local` arm: C 6.7.1p3 requires block-scope `_Thread_local` to
    /// be written with `static` or `extern` anyway, so every legal spelling already sets one of
    /// these — and a bare one is a different diagnostic, not a case for this predicate to guess at.
    fn has_static_storage(scope: Scope, storage: &Storage) -> bool {
        scope == Scope::File || storage.static_ || storage.extern_
    }

    /// Whether an expression is a constant expression, for a static initializer (C 6.7.9p4).
    ///
    /// Asked of the *whole* initializer including its list elements, because `{f()}` is as
    /// non-constant as `f()` is. Both `eval` and `addr_of` count: `&y` and `"s"` are address
    /// constants, which arithmetic folding alone cannot answer for.
    fn check_static_init(&mut self, init: ExprId) {
        match self.ast.expr(init).kind.clone() {
            ExprKind::InitList(items) => {
                for item in items {
                    self.check_static_init(item.value);
                }
            }
            ExprKind::Str { .. } => {}
            _ => {
                let before = self.out.diagnostics.len();
                let constant = self.eval(init).is_some() || self.addr_of(init).is_some();
                // **Discard the fold's diagnostics only when it succeeded.** Asking whether
                // something is constant is not itself an error — that is why the truncate is
                // here — but when the answer is *no because the expression is malformed*, those
                // diagnostics are the reason, and this arm has nothing better to say. Discarding
                // them unconditionally accepted `int g = 1/0;` outright: `eval` refused it, said
                // why, the explanation went in the bin, and `reads_an_object` found nothing to
                // object to.
                //
                // **Only where a constant expression is required.** `int f(void){ return 1/0; }`
                // is runtime undefined behaviour rather than a constraint violation, and this
                // walk only runs on static initializers, so the distinction costs nothing here.
                // **A fold can report and still succeed.** `2147483647 + 1` yields a wrapped
                // value *and* complains, so keying the rescue on "the fold failed" discarded the
                // overflow and accepted the program. What the truncate is for is the *silent*
                // successful fold — an ordinary constant — so it runs only when nothing was said.
                let explained = self.out.diagnostics.len() > before;
                if !explained {
                    self.out.diagnostics.truncate(before);
                } else {
                    return;
                }
                // **Not constant is not the same as "we could not fold it".** `eval` answers about
                // arithmetic and `addr_of` about object addresses, and between them they miss
                // several things C *does* call constant expressions — a function designator such
                // as `{add1, dbl}` in a table of function pointers is the one that broke the
                // corpus, and `&arr[1]` and string literals are others.
                //
                // So the complaint is narrowed to what cannot be constant under any reading: an
                // initializer containing a **call**. That is the census case, `int g = f();`, and
                // it is sound where the general question is not. The declared miss is
                // `int x; int g = x;` — a non-constant that this accepts.
                if !constant && self.reads_an_object(init) {
                    let span = self.ast.expr(init).span;
                    self.error(span, "initializer element is not a constant expression");
                } else if constant && self.addresses_an_automatic(init) {
                    // **`addr_of` answers "is this an address", not "is this a constant".** An
                    // address constant is the address of an object with static storage duration
                    // (C 6.6p9); `&x` for an automatic `x` is an address that does not exist yet
                    // when the initializer is written down. This arm is reached *because*
                    // `addr_of` succeeded, which is why it is an `else` rather than another
                    // condition on the first branch.
                    let span = self.ast.expr(init).span;
                    self.error(
                        span,
                        "initializer element is not a constant: the address of an object with \
                         automatic storage duration",
                    );
                }
            }
        }
    }

    /// The type of an expression as *written*, without the array-to-pointer decay a use applies.
    ///
    /// `type_expr` is what records it, and it must run first — the assignment arm calls this
    /// before typing the operands only because typing them is what fills the table this reads.
    /// Reject a write to an object declared `const`.
    ///
    /// **Only when the target is the name itself.** `*p = 1` and `a[i] = 1` are writes through a
    /// pointer, and whether *those* are allowed depends on the qualifiers of the pointee — which
    /// sema does not model, so it says nothing rather than guessing. `int *const p; *p = 1;` is
    /// legal and must stay so, and it is legal precisely because the write is not to `p`.
    /// Walk `body` with one more enclosing loop, which is also one more breakable statement.
    /// Whether a value of type `from` may become `to` without a cast (C 6.5.16.1, 6.5.9).
    ///
    /// Only *pointer* mixing is judged. C's arithmetic conversions are unrestricted — `long` to
    /// `int` and `double` to `int` narrow silently — so a rule based on type identity would
    /// reject every one of them, and the whole question here is which pointers may meet.
    fn assignable(&self, from: TyId, to: TyId, null_constant: bool) -> bool {
        let f = self.out.types[from.0 as usize].clone();
        let t = self.out.types[to.0 as usize].clone();
        // **A structure or union is copied only from its own type** (C 6.7.9p13, 6.5.16.1p1).
        // Asked before the pointer question below, because neither side is a pointer and the
        // early return therefore let `struct S s = 1;` through — the whole rule for aggregates
        // was on the wrong side of a test that exists to skip *arithmetic* conversions.
        //
        // A braced initializer never reaches here: `check_init` handles a list against the
        // record's members, so what arrives is an assignment of one value to another, and for a
        // record that means the types must match. Compared bare, since a `const struct S` copies
        // into a `struct S` perfectly well.
        let record = |x: &Ty| matches!(x, Ty::Record(_));
        if record(&f) || record(&t) {
            // **An *incomplete* record is already-reported poison**, exactly as `Ty::Error` is,
            // and contract 20 is why: `typedef struct Undefined u_t; u_t x;` reports once for the
            // declaration, and every later `x + 1` must stay silent. An incomplete record is not
            // `Ty::Error` — it is a perfectly well-formed `Ty::Record` whose layout is unknown —
            // so the poison escape above does not cover it, and without this the rule turned one
            // diagnostic into eight.
            if matches!(f, Ty::Error)
                || matches!(t, Ty::Error)
                || is_incomplete(&self.out, from)
                || is_incomplete(&self.out, to)
            {
                return true;
            }
            return self.bare(from) == self.bare(to);
        }
        let ptr_like = |x: &Ty| matches!(x, Ty::Ptr(_) | Ty::Array { .. } | Ty::Func { .. });
        if !ptr_like(&f) && !ptr_like(&t) {
            return true;
        }
        // **Poison is compatible with everything.** An `Error` operand means something else has
        // already been reported, and contract 20 keeps one bad declaration to one diagnostic.
        if matches!(f, Ty::Error) || matches!(t, Ty::Error) {
            return true;
        }
        // **`_Bool` takes any scalar** (C 6.3.1.2): `_Bool b = p;` is a test against zero, not a
        // truncation, and it is legal for every pointer. This is checked before anything else
        // because it is the one destination that accepts everything.
        if matches!(t, Ty::Int { bits: 1, .. }) {
            return true;
        }
        // **A pointee, whichever way the type was spelled.** A parameter declared `int a[2][3]`
        // keeps its array type in sema while the argument passed to it has decayed to a pointer,
        // so the two sides of one legal call arrive here spelled differently. Normalising both is
        // what lets the comparison be about the pointee rather than about the spelling.
        let pointee = |x: &Ty| match x {
            Ty::Ptr(p) => Some(*p),
            Ty::Array { elem, .. } => Some(*elem),
            _ => None,
        };
        match (&f, &t) {
            // **`0` is a null pointer constant, `1` is not** (C 6.3.2.3p3). The distinction is the
            // value rather than the type, which is why this is a parameter and not a type test.
            //
            // **`Ty::Array` used to be named here too, and no longer needs to be.** A parameter
            // declared "array of T" was the only destination that could reach this arm as an
            // array, and C 6.7.6.3p7 now adjusts it to a pointer at declaration (b5e163d), so it
            // arrives as `Ty::Ptr` like everything else. Assigning `0` to an array *object* never
            // reached here: `a = 0` is refused as "assignment to an array" and contract 20
            // suppresses the conversion behind it.
            (_, Ty::Ptr(_)) if null_constant => true,
            // A function converts to a pointer to itself.
            (Ty::Func { .. }, Ty::Ptr(b)) => self.compatible(from, *b),
            _ => match (pointee(&f), pointee(&t)) {
                (Some(a), Some(b)) => {
                    let av = matches!(self.out.types[a.0 as usize], Ty::Void);
                    let bv = matches!(self.out.types[b.0 as usize], Ty::Void);
                    // **The destination pointee must have every qualifier the source has**
                    // (C 6.5.16.1p1). Checked for `void *` too: `void *p = cp;` from a
                    // `const void *` discards `const` exactly as any other pointer would, and
                    // exempting `void *` from the qualifier rule is how a permissive `void *`
                    // case swallows it.
                    if !self.qual_of(b).covers(self.qual_of(a)) {
                        return false;
                    }
                    // `void *` converts to and from any object pointer without a cast, in both
                    // directions — that is what makes it `void *`.
                    //
                    // **Compared bare.** C ignores qualifiers only at *this* level: the check
                    // above has already accounted for them here, and anything deeper is part of
                    // the type. That is why `const int **cpp = pp;` is illegal in C though the
                    // analogue is legal in C++ — one level down, `int *` and `const int *` are
                    // simply different types.
                    av || bv || self.compatible(self.bare(a), self.bare(b))
                }
                _ => false,
            },
        }
    }

    /// Whether two pointee types are the same type, structurally.
    ///
    /// Types are interned, so this is mostly identity — the exception is a function type reached
    /// through a pointer, where the *parameters* must agree and an empty list still means
    /// "unspecified" for the reason recorded on `types_conflict`.
    fn compatible(&self, a: TyId, b: TyId) -> bool {
        a == b || !self.types_conflict(a, b)
    }

    /// Whether two declarations of one name commit to incompatible types.
    ///
    /// **Return types are compared always; parameter lists only when both are non-empty.** The
    /// parser gives `f()` and `f(void)` the *same* empty list — `parameter_list` returns
    /// `(vec![], false, false)` for both — so sema cannot tell "unspecified parameters" from "no
    /// parameters", and comparing an empty list against a prototype would reject
    /// `int f(); int f(int x){...}`, which is legal C.
    ///
    /// A separate guard for old-style declarations was written first and **deleted as
    /// subsumed**: a K&R *declaration* has an empty list and is skipped by the rule above, and a
    /// K&R *definition* has its parameter types filled in by then and is legitimately comparable.
    /// Mutation could not falsify the guard because there was nothing left for it to do.
    ///
    /// That is a declared limit in one direction only: `int f(void); int f(int);` is a conflict
    /// this misses. It is the right way round — wave 303's rule is that rejecting a correct
    /// program is worse than missing an incorrect one — and comparing return types regardless
    /// keeps `int f(); long f();` caught, since even an old-style declaration commits to what it
    /// returns.
    /// A parameter's type as the function actually has it, for comparison only.
    ///
    /// **An array parameter is adjusted to a pointer** (C 6.7.6.3p7), so `f(int a[2])`,
    /// `f(int a[3])` and `f(int *a)` are one declaration and the written length is not part of
    /// the type. Returned as a *shape* — "points at this, or is this" — rather than as an
    /// interned pointer type, because `&self` cannot intern and the pointer form may never have
    /// been built in the program being compiled.
    fn param_shape(&self, t: TyId) -> (bool, TyId) {
        match self.out.types[t.0 as usize] {
            // **No `Ty::Array` arm.** A parameter is the only place two declarations could differ
            // by array-versus-pointer, and 6.7.6.3p7 adjusts it before this is ever asked
            // (b5e163d), so `f(int p[4])` and `f(int *p)` now agree by interned identity rather
            // than by being normalised here.
            Ty::Ptr(p) => (true, p),
            _ => (false, t),
        }
    }

    fn types_conflict(&self, a: TyId, b: TyId) -> bool {
        if a == b {
            return false;
        }
        // **An enumeration is a distinct type against another enumeration, and its integer type
        // against everything else** (C 6.7.2.3p5 for the first, 6.7.2.2p4 for the second).
        //
        // gcc draws it exactly here: `enum E a; enum F a;` conflicts while `enum E a; unsigned
        // a;` does not, even though that makes compatibility non-transitive. Following the
        // compiler rather than the standard's transitivity is this project's calibration.
        //
        // Dropping the tag on one side is what keeps wave 383's rows legal, and it has to happen
        // before the match below, whose `_` arm would call any two differing ids a conflict.
        if self.enum_tag(a) != self.enum_tag(b) {
            if self.enum_tag(a) != 0 && self.enum_tag(b) != 0 {
                return true;
            }
            return self.types_conflict(self.untagged(a), self.untagged(b));
        }
        match (
            self.out.types[a.0 as usize].clone(),
            self.out.types[b.0 as usize].clone(),
        ) {
            (
                Ty::Func {
                    ret: r1,
                    params: p1,
                    variadic: v1,
                    prototyped: q1,
                },
                Ty::Func {
                    ret: r2,
                    params: p2,
                    variadic: v2,
                    prototyped: q2,
                },
            ) => {
                // **A top-level qualifier is not part of the type** (C 6.7.6.3p15). Interned ids
                // carry one and C drops it: `const int f(void)` and `int f(void)` are one
                // declaration, and so are `f(const int)` and `f(int)`. `bare` strips only the
                // outermost, which is the whole distinction — `f(const int *)` against
                // `f(int *)` is a real conflict, because that `const` is on the pointee.
                //
                // **Compared for compatibility, not identity.** `!=` on the two bare ids was the
                // same habit waves 379-381 kept finding: it makes `enum E f(void)` and
                // `unsigned f(void)` two declarations, though the enumeration *is* that integer
                // type, and it would answer identity for a returned pointer-to-unsized-array too.
                if self.types_conflict(self.bare(r1), self.bare(r2)) {
                    return true;
                }
                // **Parameters are compared when both declarations specified them.** This used to
                // read `p1.is_empty() || p2.is_empty()`, and the comment above recorded why: the
                // parser gave `f()` and `f(void)` the same empty list, so the only safe reading of
                // an empty list was "unspecified" and `int f(void); int f(int);` went unreported.
                //
                // With the flag the guard says what it always meant. `f()` composes with anything
                // — C 6.7.6.3p15 — so it still returns early; `f(void)` is a prototype with zero
                // parameters and now conflicts with `f(int)` exactly as `f(char)` would.
                if !q1 || !q2 {
                    return false;
                }
                // **A parameter's array length is not part of the type** (C 6.7.6.3p7): an array
                // parameter is adjusted to a pointer, so `f(int a[2])` and `f(int a[3])` and
                // `f(int *a)` are one declaration. Comparing the interned ids called them three.
                if p1.len() != p2.len() {
                    return true;
                }
                p1.iter().zip(&p2).any(|(&x, &y)| {
                    let (px, ex) = self.param_shape(self.bare(x));
                    let (py, ey) = self.param_shape(self.bare(y));
                    px != py || self.types_conflict(ex, ey)
                }) || v1 != v2
            }
            // **An array of unspecified length is compatible with any length** (C 6.2.7p3), which
            // is how a header declares what a translation unit sizes: `extern int a[];` then
            // `int a[3];`. Refusing it broke the idiom the construct exists for, in both orders.
            (Ty::Array { elem: e1, len: l1 }, Ty::Array { elem: e2, len: l2 }) => {
                if self.types_conflict(e1, e2) {
                    return true;
                }
                // `Flexible` is `int a[]` — the unspecified length. `Zero` is the GNU `int a[0]`,
                // which *is* a length and does conflict with a different one.
                !matches!(l1, ArrayLen::Flexible) && !matches!(l2, ArrayLen::Flexible) && l1 != l2
            }
            // **A pointer is compatible when its pointee is** (C 6.7.6.1p2). Without this arm two
            // pointers that are not the same id fall through to `_ => true`, which is the right
            // answer for every pointee whose own rule is identity and the wrong one for the
            // single pointee rule that is weaker: the array arm just above, where an unspecified
            // length is compatible with any length. `extern int (*a)[]; int (*a)[3];` is that
            // idiom one `*` deeper than the plain form wave 380 fixed.
            //
            // Qualifiers are *not* stripped on the way down: a top-level qualifier is not part of
            // a type (6.7.6.3p15) but a pointee's is, so `const int *` and `int *` reach the
            // fallthrough below and conflict, exactly as before.
            (Ty::Ptr(x), Ty::Ptr(y)) => self.types_conflict(x, y),
            // **`aligned` and `may_alias` do not make a distinct vector for compatibility.**
            // gcc's `__m128i_u` is `__m128i` with `__may_alias__` and `__aligned__(1)`, and its
            // own `emmintrin.h` passes one where the other is wanted — silently, in every mode
            // including `-pedantic-errors` and `-Wcast-align=strict`. The attributes change
            // placement and aliasing, not type identity.
            //
            // `align` stays in the interning key because layout genuinely differs; only
            // *compatibility* ignores it. 1849 of 1871 VPP translation units depend on this,
            // and they became visible only once `-march` reached the predefines.
            (
                Ty::Vector {
                    elem: ex,
                    lanes: lx,
                    ..
                },
                Ty::Vector {
                    elem: ey,
                    lanes: ly,
                    ..
                },
            ) => lx != ly || self.types_conflict(ex, ey),
            // Interned types, so anything else differing is a real difference.
            _ => true,
        }
    }

    /// Compare a file-scope declaration with any earlier one of the same name (C 6.7p4, 6.2.2p7).
    fn check_redeclaration(&mut self, name: Symbol, now: Prior, span: Span) {
        let Some(&was) = self.prior.get(&name) else {
            self.prior.insert(name, now);
            return;
        };
        let text = self.text(name).unwrap_or("?").to_owned();

        if was.defined && now.defined {
            self.error(span, format!("`{text}` is defined more than once"));
        } else if self.types_conflict(was.ty, now.ty) {
            self.error(span, format!("conflicting types for `{text}`"));
        } else if resolved_linkage(was, now) != was.internal {
            // **`extern` adopts, it does not defer.** C 6.2.2p4: an `extern` declaration takes
            // the linkage of a prior visible declaration, and external only when there is none.
            // That is what makes the rule asymmetric — `static int n; extern int n;` is legal
            // because the second adopts internal linkage, while `extern int n; static int n;` is
            // not, because the first already resolved to external. A model where `extern` simply
            // never conflicts accepts both, and was the first thing written here.
            let (a, b) = if now.internal {
                ("static", "non-static")
            } else {
                ("non-static", "static")
            };
            self.error(
                span,
                format!("{a} declaration of `{text}` follows {b} declaration"),
            );
        }

        // The record keeps whichever facts are the stronger claim: once defined, always defined,
        // and the linkage the pair resolved to governs the rest.
        self.prior.insert(
            name,
            Prior {
                ty: now.ty,
                defined: was.defined || now.defined,
                internal: resolved_linkage(was, now),
                deferring: was.deferring && now.deferring,
            },
        );
    }

    fn in_loop(&mut self, body: impl FnOnce(&mut Self)) {
        self.loop_depth += 1;
        self.breakable_depth += 1;
        body(self);
        self.breakable_depth -= 1;
        self.loop_depth -= 1;
    }

    /// `p++`, `p--`, `++p`, `--p` — pointer arithmetic by another name (C 6.5.2.4p1, 6.5.3.1p1).
    ///
    /// Reported where the operand is *typed*, so the one predicate serves this and `p + 1` alike.
    fn check_incdec_pointee(&mut self, t: TyId, span: Span, op: &str) {
        if self.incomplete_pointee(t) {
            self.error(span, "arithmetic on a pointer to an incomplete type");
            return;
        }
        // **C 6.5.2.4p1 / 6.5.3.1p1: the operand is a real or pointer type.** A structure, a
        // union, an array and a function designator are none of those, and `check_writable`
        // above does not ask — it asks whether the object may be *written*, which is a different
        // question that `s++` passes.
        //
        // Asked of the type **as written**, deliberately not decayed: an array is not a
        // modifiable lvalue, and decaying it first would make `a++` look like pointer
        // arithmetic. That is the opposite of waves 360–364, where the decay was the load-bearing
        // step — here it would hide the mistake, and the difference is that `++` writes back.
        //
        // `void *` keeps its GNU arithmetic: the question is the operand's kind, not its
        // pointee's size.
        if !matches!(
            self.out.types[t.0 as usize],
            Ty::Int { .. } | Ty::Float(_) | Ty::Ptr(_) | Ty::Vector { .. } | Ty::Error
        ) {
            self.error(span, format!("`{op}` needs a scalar operand"));
        }
    }

    fn check_writable(&mut self, target: ExprId, t: TyId, what: &str) {
        // **One question about the type, not four questions about the syntax.**
        //
        // This was three scoped name-sets and a syntactic walk: `read_only` for a `const` object,
        // `read_only_pointee` for a pointer parameter whose element was `const`, and an arm each
        // for `*p`, `p[i]` and `p->m` to route a write through a pointer to the second set. Every
        // one of them was reconstructing, from the spelling of the declaration, what the type
        // now says outright. `s->m` where `m` is declared `const` was the case no amount of
        // spelling could reach, because the qualifier is on the *member*.
        //
        // **The target's type is passed in, not typed here.** This used to call
        // `type_of_written`, on a comment claiming `type_expr` was memoized so the caller's
        // own typing would be a lookup. `type_expr` is **not** memoized — `by_expr` records the
        // outermost node but is never consulted to short-circuit — so the target was typed twice
        // for an assignment and twice for `++`, and every diagnostic inside it was reported once
        // per typing. Invisible until a *cast* could carry one: `((U)x).u = 1` said "ISO C
        // forbids casts to union type" twice where gcc says it once. Typing at the call site and
        // passing the type down is what makes it once, and it also puts the operand's own
        // diagnostics before this one — gcc's order.
        //
        // **Not an lvalue at all** (C 6.5.2.4p1, 6.5.3.1p1): `x++++` increments a value. Asked
        // before the qualifier question because it is the more basic failure and because the type
        // of `x++` is a perfectly ordinary `int` — nothing about the *type* is wrong.
        if self.not_an_lvalue(target) {
            let span = self.ast.expr(target).span;
            self.error(span, format!("{what} something that is not an lvalue"));
            return;
        }
        if !self.qual_of(t).const_ {
            return;
        }
        let span = self.ast.expr(target).span;
        // The name is worth naming when there is one, and there usually is: a write is to an
        // object, through a pointer, or to a member, and all three have something to call it.
        let named = match self.ast.expr(target).kind.clone() {
            ExprKind::Ident(n) => self.text(n).map(|t| format!(" `{t}`")),
            ExprKind::Member { field, .. } => self.text(field).map(|t| format!(" `{t}`")),
            _ => None,
        };
        self.error(
            span,
            format!("{what} read-only object{}", named.unwrap_or_default()),
        );
    }

    /// Whether this declared type is a pointer — or an array, which a parameter's is — whose
    /// element is `const`.
    ///
    /// **The outermost pointee only.** `const int **p` is not: its immediate pointee is
    /// `const int *`, which is a perfectly writable pointer, and only the level below that is
    /// read-only. One level of indirection is one level.
    fn points_to_const(&self, ty: TypeId) -> bool {
        let inner = match self.ast.ty(ty).kind {
            TypeKind::Ptr(inner) => inner,
            TypeKind::Array { elem, .. } => elem,
            _ => return false,
        };
        self.ast.ty(inner).quals.const_
    }

    /// **At most one of `extern`, `static`, `auto`, `register`** (C 6.7.1p2).
    ///
    /// Three things are deliberately *not* counted:
    ///
    ///   - **`_Thread_local`**, which 6.7.1p2 exempts by name — it may accompany `static` or
    ///     `extern`, in either order, and both spellings appear in real code.
    ///   - **`inline` and `_Noreturn`**, which are *function* specifiers (6.7.4) and combine with
    ///     anything. They share this struct only because the parser collects them together.
    ///   - **`typedef`**, which C does count as a storage-class specifier — so
    ///     `typedef static int T;` is a violation this cannot see. `DeclKind::Typedef` carries no
    ///     `Storage` in this AST, so the `static` is gone before sema looks. A declared limit,
    ///     and a parser change rather than an oversight.
    fn check_storage_classes(&mut self, storage: chiero_ast::Storage, span: Span) {
        let n = [
            storage.extern_,
            storage.static_,
            storage.auto,
            storage.register,
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if n > 1 {
            self.error(span, "multiple storage classes in one declaration");
        }
    }

    /// **Which storage classes the context admits** (C 6.7.1p3, 6.8.5p3, 6.7.6.3p2, 6.9.1p4).
    ///
    /// Four paragraphs, one table. Written as a table because that is what they are: read singly
    /// they look like four unrelated rules, which is how all four came to be unimplemented.
    ///
    /// **`inline` and `_Noreturn` are absent on purpose.** They are *function specifiers*, not
    /// storage classes, and folding them in would reject `static inline` — the corpus's commonest
    /// spelling, and a case wave 330 already had to protect once.
    fn check_storage_context(
        &mut self,
        storage: chiero_ast::Storage,
        ctx: StorageContext,
        span: Span,
    ) {
        let offender = match ctx {
            // No automatic storage exists to refer to.
            StorageContext::File => [("auto", storage.auto), ("register", storage.register)],
            // A block admits all four.
            StorageContext::Block => [("", false), ("", false)],
            // 6.8.5p3: only `auto` and `register`. A `static` here would outlive its loop.
            StorageContext::ForInit => [("extern", storage.extern_), ("static", storage.static_)],
            // 6.7.6.3p2: only `register`.
            StorageContext::Parameter => [
                ("extern", storage.extern_ || storage.auto),
                ("static", storage.static_),
            ],
            // 6.9.1p4: only `extern` and `static`.
            StorageContext::Function => [("auto", storage.auto), ("register", storage.register)],
            StorageContext::NotAnObject => [("", false), ("", false)],
        }
        .into_iter()
        .find(|&(_, hit)| hit)
        .map(|(name, _)| name);
        if let Some(name) = offender {
            let where_ = match ctx {
                StorageContext::File => "a file-scope declaration",
                StorageContext::Block => "a block-scope declaration",
                StorageContext::ForInit => "a `for` initializer",
                StorageContext::Parameter => "a parameter",
                StorageContext::Function => "a function",
                StorageContext::NotAnObject => "a `typedef` or a member",
            };
            self.error(span, format!("`{name}` is not allowed in {where_}"));
            return;
        }
        // **A function specifier declares a function** (C 6.7.4p2). Asked here rather than in a
        // rule of its own because it is the same question in the same five places: `inline` and
        // `_Noreturn` belong on a function and nowhere else, so every context except `Function`
        // refuses both.
        //
        // Below the storage complaint and behind its `return`, so `inline register int x;` is
        // one sentence rather than two — contract 20, and the storage class is the more useful
        // half to name first.
        if !matches!(ctx, StorageContext::Function)
            && let Some(name) = [("inline", storage.inline), ("_Noreturn", storage.noreturn)]
                .into_iter()
                .find(|&(_, hit)| hit)
                .map(|(name, _)| name)
        {
            let where_ = match ctx {
                StorageContext::File => "a file-scope object",
                StorageContext::Block => "a block-scope object",
                StorageContext::ForInit => "a `for` initializer",
                StorageContext::Parameter => "a parameter",
                StorageContext::NotAnObject => "a `typedef` or a member",
                StorageContext::Function => unreachable!("excluded above"),
            };
            self.error(
                span,
                format!("`{name}` declares a function, and this is {where_}"),
            );
        }
    }

    /// **A cast names a scalar type and takes a scalar operand** (C 6.5.4p2), and a pointer
    /// converts to neither direction of a floating type (p4).
    ///
    /// **The `void` target is the exception the rule is built around**, not a footnote: a cast
    /// to `void` discards its operand rather than converting it, so `(void)s` on a structure is
    /// legal — and it is the only cast anyone writes with a struct. Asking "are both sides
    /// scalar" would reject that and catch nothing anyone writes.
    ///
    /// The operand arrives **decayed**, which is why an array and a function designator are
    /// scalar here and no arm is needed for either. Poison stays silent on both sides
    /// (contract 20).
    /// Answers whether the cast was accepted, so the caller can poison a refused one.
    fn check_cast(&mut self, target: TyId, operand: TyId, span: Span) -> bool {
        let scalar = |cx: &Self, t: TyId| {
            matches!(
                cx.out.types[t.0 as usize],
                Ty::Int { .. } | Ty::Float(_) | Ty::Ptr(_) | Ty::Error
            )
        };
        if matches!(self.out.types[target.0 as usize], Ty::Void) {
            return true;
        }
        // **A cast involving a vector keeps its size, and the other side is a vector or an
        // integer.** A GNU extension, and one the corpus needs — VPP casts a two-lane vector to
        // `uword` all over `vppinfra`, and the twenty-header gate caught the first draft of this
        // rule ("a vector converts only to a vector") rejecting it in six headers.
        //
        // The rule as measured against gcc, not as guessed: `(long)v` from an 8-byte vector is
        // taken and `(int)v` is not; `(double)v` is refused at the same size, and so is
        // `(v2)p` from a pointer; two vector types convert exactly when their widths match. One
        // size test and one integer test cover all seven rows.
        let vector = |cx: &Self, t: TyId| matches!(cx.out.types[t.0 as usize], Ty::Vector { .. });
        if vector(self, target) || vector(self, operand) {
            if matches!(self.out.types[target.0 as usize], Ty::Error)
                || matches!(self.out.types[operand.0 as usize], Ty::Error)
            {
                return true;
            }
            let sizes = size_of_ty(&self.out, &self.target, target).zip(size_of_ty(
                &self.out,
                &self.target,
                operand,
            ));
            let other_ok = |cx: &Self, t: TyId| {
                matches!(
                    cx.out.types[t.0 as usize],
                    Ty::Vector { .. } | Ty::Int { .. }
                )
            };
            if sizes.is_some_and(|(a, b)| a == b)
                && other_ok(self, target)
                && other_ok(self, operand)
            {
                return true;
            }
            self.error(
                span,
                "a cast involving a vector type needs an integer or a vector of the same size",
            );
            return false;
        }
        if !scalar(self, target) {
            // **Two GNU extensions meet here**, both measured accepted by `gcc -std=gnu11` and
            // refused by `-pedantic-errors` — which is what makes them dialect rules rather than
            // over-rejections, on wave 314's calibration. Cast-to-union is the largest kind left
            // in the VPP queue: `(ip4_address_t) la`, `((iavf_rx_desc_qw1_t) qw1).length` and
            // two more, 11 findings over four sites.
            //
            // **An already-reported operand ends the question here** (contract 20). `Ty::Error`
            // is `compatible` with everything, so letting it reach either arm would launder a
            // reported fault into a silently accepted extension — but falling *through* to the
            // sentence below is no better, and that is what the first draft did: `(U)undeclared`
            // said "a cast names a scalar type or `void`" after the undeclared-identifier
            // message, a second sentence that is also false, since `U` is a union one may cast
            // to. gcc says one thing and stops.
            //
            // An **incomplete** operand is excused the same way and refused the same way — false
            // rather than true, matching the scalar path below, so the caller poisons the result
            // and the member access on it stays quiet too.
            if matches!(self.out.types[operand.0 as usize], Ty::Error) {
                return true;
            }
            if is_incomplete(&self.out, operand) {
                return false;
            }
            // **A cast to the operand's own record type**: `(struct S)s`. gcc says "ISO C
            // forbids casting nonscalar to the same type", and it says *that* rather than the
            // union sentence for `(union U)u` — which is why this arm is tested first and not
            // folded into the member search below.
            if self.compatible(self.bare(target), self.bare(operand)) {
                if self.dialect.pedantic {
                    self.advisory(span, "ISO C forbids casting nonscalar to the same type");
                }
                return true;
            }
            if let Ty::Record(r) = self.out.types[target.0 as usize]
                && self.out.layout(r).is_union
            {
                // **The member match is by compatibility, not by conversion**, and that is the
                // half that keeps the extension from swallowing defects: gcc refuses `(U)x` for
                // an `int` x against an `unsigned int` member, in *both* modes.
                // `transparent_member_for` rightly asks `assignable` — the attribute widens a
                // parameter — and a cast written the same way would go quiet on a real mismatch.
                //
                // `compatible` rather than `TyId` identity because C's compatibility is what gcc
                // applies: an `enum e` operand matches an `unsigned int` member, and identity
                // would refuse it.
                //
                // **`layout.fields` is the right list precisely because it is not flattened.**
                // gcc does not search inside an anonymous member — `union { struct { u32 a; };
                // u64 l; }` refuses `(U)some_u32` in both modes — while the member *access* on
                // the result does see through it. Two of the three real VPP unions are that
                // shape, so a search that reused the access path would be wrong on the corpus.
                //
                // Cloned because `compatible` borrows `self` while the layout is still held, and
                // a union's member list is short.
                // **A bit-field is not a member of its declared type for this search.**
                // `FieldLayout.ty` records what the bit-field was declared with, so an
                // unfiltered list matches `unsigned int whole : 8` against an `unsigned int`
                // operand — a false acceptance of a cast gcc refuses in both modes, which is the
                // exact failure this whole rule is written to avoid.
                let members: Vec<TyId> = self
                    .out
                    .layout(r)
                    .fields
                    .iter()
                    .filter(|f| f.bits.is_none())
                    .map(|f| f.ty)
                    .collect();
                let from = self.bare(operand);
                if members
                    .iter()
                    .any(|&m| self.compatible(self.bare(m), from) && self.prototypes_agree(m, from))
                {
                    if self.dialect.pedantic {
                        self.advisory(span, "ISO C forbids casts to union type");
                    }
                    return true;
                }
                // **An incomplete union arrives here too, and wants this sentence**: it has no
                // members, so gcc's complaint is "from type not present in union" rather than
                // anything about completeness. Measured.
                self.error(span, "a cast to a union names a type no member has");
                return false;
            }
            self.error(span, "a cast names a scalar type or `void`");
            return false;
        }
        // **The operand side excuses an incomplete type; the target side does not.** An operand
        // of incomplete type was already reported at its declaration, so complaining here is a
        // second sentence about a fault the reader has been told about (contract 20). An
        // incomplete *target* is a fresh mistake made by the cast itself, and keeps its own.
        if !scalar(self, operand) && !is_incomplete(&self.out, operand) {
            self.error(span, "a cast takes a scalar operand");
            return false;
        }
        if is_incomplete(&self.out, operand) {
            return false;
        }
        let float_and_pointer = |a: TyId, b: TyId| {
            matches!(self.out.types[a.0 as usize], Ty::Float(_))
                && matches!(self.out.types[b.0 as usize], Ty::Ptr(_))
        };
        if float_and_pointer(target, operand) || float_and_pointer(operand, target) {
            self.error(
                span,
                "a pointer does not convert to or from a floating type",
            );
            return false;
        }
        true
    }

    /// **A parameter's declared type, adjusted** — C 6.7.6.3p8 for now, p7 to follow.
    ///
    /// A parameter declared as a function becomes a pointer to it. VPP writes callback
    /// parameters this way — `int options (u32, ip6_hop_by_hop_option_t *, u16)` in
    /// `ip6_ioam_analyse_register_hbh_handler` — and without the adjustment the argument, which
    /// *has* decayed to a pointer, is compared against a function type and called incompatible.
    ///
    /// **It is a function because there are three places a parameter's type is recorded, and
    /// they must agree.** The interned `Ty::Func`'s parameter list is what a call's arguments are
    /// checked against and what two declarations are compared through; `decl_types` and `values`
    /// are what the body and 015's lowering read. This adjustment lived inline in the first of
    /// those and was absent from the other two, so a function-typed parameter was a pointer to
    /// its caller and a function to its own body — `sizeof g` was refused where gcc says 8.
    /// One helper at all three call sites is the whole fix, and it is also what makes the p7
    /// array adjustment a one-arm change rather than a third place to get out of step.
    ///
    /// Applied **after** `ty_of` returns, so every diagnostic the declared form owns — an
    /// incomplete element type, a negative or zero bound, qualifier redistribution through a
    /// typedef — has already fired on the type as written.
    ///
    /// Not `param_shape`'s job: that answers "do two declarations agree", while this is the
    /// parameter's actual type.
    fn adjusted_param_ty(&mut self, pty: TypeId, t: TyId) -> TyId {
        match self.out.types[t.0 as usize] {
            Ty::Func { .. } => self.intern(Ty::Ptr(t)),
            // **p7: "array of T" becomes "pointer to T".** Only the outermost dimension — the
            // element keeps its own type, so a parameter `int q[3][4]` is `int (*)[4]` and
            // `sizeof q[0]` is still 16. Recursing would be a different and wrong adjustment.
            //
            // **The bracket qualifiers land on the pointer, not the pointee.** `int p[const 4]`
            // is `int *const p`: the *parameter* is read-only, while `const int p[4]` makes the
            // element read-only and is a different diagnostic. They are read from the AST rather
            // than from `t`, because the array `TyId` deliberately does not carry them — putting
            // them there would have made `qualify` push them down onto the element, which is the
            // wrong object.
            Ty::Array { elem, .. } => {
                let bracket = match self.ast.ty(pty).kind {
                    TypeKind::Array { bracket_quals, .. } => bracket_quals,
                    // A typedef'd array parameter — `typedef int A[4]; f(A p)` — reaches here
                    // with an AST node that is not an `Array`. gcc allows no bracket qualifiers
                    // in that spelling, so there are none to move.
                    _ => chiero_ast::Quals::default(),
                };
                self.intern_qual(
                    Ty::Ptr(elem),
                    Qual {
                        const_: bracket.const_,
                        volatile_: bracket.volatile_,
                        restrict_: bracket.restrict_,
                    },
                )
            }
            _ => t,
        }
    }

    /// The type of a compiler builtin chiero has **measured**, or `None` to leave it poison.
    ///
    /// gcc declares these itself and no header names them, so sema exempts them from
    /// "was not declared" — but an exemption is not a type. Without one the call's value has no
    /// width, and `unsigned long long __rdpmc (int) { return __builtin_ia32_rdpmc (__S); }` from
    /// gcc's own `ia32intrin.h` produced a 32-bit result where 64 was declared, so the verifier
    /// rejected the function and 015 §7 discarded it.
    ///
    /// **Only the return type is claimed.** The signature is interned *unprototyped*, which is C's
    /// way of saying the parameters are unspecified — and that is exactly what is known here.
    /// Asserting parameter types nobody measured would turn every call into a false diagnostic.
    ///
    /// **Every row is measured against gcc 13.3.0 on this machine**, with `_Generic` over the call
    /// expression, not read from documentation. A wrong signature is worse than none: chiero
    /// trusts this. A blanket implicit-`int` fallback was tried and refuted — `__builtin_alloca`
    /// returns a pointer and the `__builtin_ia32_*` family return vectors, so it made `vppinfra`'s
    /// own headers report "returning a value makes a pointer from an integer".
    ///
    /// A name absent from this table keeps `Ty::Error` and lowers to an opaque effect. That floor
    /// works, and is the right answer until someone measures the name.
    fn builtin_signature(&mut self, name: &str) -> Option<TyId> {
        use builtins::{B, Ret};
        let scalar = |cx: &mut Self, b: B| -> TyId {
            let int_ = |signed, bits| Ty::Int { signed, bits };
            let w = |n: u64| (n * 8) as u32;
            let t = match b {
                B::Void => Ty::Void,
                B::Bool => int_(false, 1),
                B::Char => int_(cx.target.char_signed, 8),
                B::I8 => int_(true, 8),
                B::U8 => int_(false, 8),
                B::I16 => int_(true, 16),
                B::U16 => int_(false, 16),
                B::I32 => int_(true, 32),
                B::U32 => int_(false, 32),
                B::ILong => int_(true, w(cx.target.sizes.long_)),
                B::ULong => int_(false, w(cx.target.sizes.long_)),
                B::I64 => int_(true, 64),
                B::U64 => int_(false, 64),
                B::F16 => Ty::Float(FloatKind::Binary16),
                B::F32 => Ty::Float(FloatKind::F32),
                B::F64 => Ty::Float(FloatKind::F64),
                B::F80 => Ty::Float(FloatKind::X87_80),
                B::F128 => Ty::Float(FloatKind::Binary128),
                B::BF16 => Ty::Float(FloatKind::BFloat16),
                B::F32Ext => Ty::Float(FloatKind::Float32Ext),
                B::F64Ext => Ty::Float(FloatKind::Float64Ext),
                B::F32xExt => Ty::Float(FloatKind::Float32xExt),
                B::F64xExt => Ty::Float(FloatKind::Float64xExt),
            };
            cx.intern(t)
        };
        let ret = match builtins::measured_return(name)? {
            Ret::Scalar(b) => scalar(self, b),
            Ret::Ptr(b) => {
                let e = scalar(self, b);
                self.intern(Ty::Ptr(e))
            }
            // **A vector return is what the `__builtin_ia32_*` bulk is** — 2499 of the measured
            // names — and it is why this table exists rather than a handful of hand-written rows.
            Ret::Vector { elem, lanes } => {
                let e = scalar(self, elem);
                let bytes = size_of_ty(&self.out, &self.target, e).unwrap_or(0) * u64::from(lanes);
                self.intern(Ty::Vector {
                    elem: e,
                    lanes,
                    align: bytes.max(1),
                })
            }
        };
        Some(self.intern(Ty::Func {
            ret,
            params: Vec::new(),
            variadic: false,
            prototyped: false,
        }))
    }

    /// C 6.2.7p3's extra condition when an **unprototyped** function type meets a prototyped one:
    /// compatible only if the prototype has no ellipsis and every parameter is unchanged by the
    /// default argument promotions.
    ///
    /// **`types_conflict` deliberately does not ask this**, and is right not to: it answers about
    /// *redeclarations*, where an unspecified parameter list means "says nothing" and
    /// over-rejection is the worse error. The cast-to-union member search inverts the risk — there,
    /// leniency turns a gcc error into silence — so it asks the paragraph directly.
    ///
    /// Measured, both directions: gcc takes `int (*)(double)`, `int (*)(_Float16)` and
    /// `int (*)(_Float32)` against an `int (*)()` member, and refuses `char`, `short` and `float`.
    /// Answers `true` for every type that is not a pair of function types disagreeing about
    /// prototypedness, so it is a filter on `compatible` rather than a second opinion.
    fn prototypes_agree(&self, a: TyId, b: TyId) -> bool {
        let peel = |cx: &Self, t: TyId| match cx.out.types[cx.bare(t).0 as usize] {
            Ty::Ptr(p) => cx.bare(p),
            _ => cx.bare(t),
        };
        let (
            Ty::Func {
                params: pa,
                variadic: va,
                prototyped: qa,
                ..
            },
            Ty::Func {
                params: pb,
                variadic: vb,
                prototyped: qb,
                ..
            },
        ) = (
            &self.out.types[peel(self, a).0 as usize],
            &self.out.types[peel(self, b).0 as usize],
        )
        else {
            return true;
        };
        if qa == qb {
            return true;
        }
        let (params, variadic) = if *qa { (pa, va) } else { (pb, vb) };
        !variadic && params.iter().all(|&p| self.promotion_stable(p))
    }

    /// Whether the **default argument promotions** leave `t` alone (C 6.5.2.2p6).
    ///
    /// Narrower-than-`int` integers become `int` and `float` becomes `double`; everything else,
    /// `_Float16` and `_Float32` included, arrives as itself. The float list is measured against
    /// gcc rather than reasoned from width — `_Float16` is narrower than `double` and is still
    /// not promoted.
    fn promotion_stable(&self, t: TyId) -> bool {
        match self.out.types[self.bare(t).0 as usize] {
            Ty::Int { bits, .. } => bits >= (self.target.sizes.int_ * 8) as u32,
            Ty::Float(k) => !matches!(k, FloatKind::F32),
            _ => true,
        }
    }

    /// **`<<`, `>>`, `&`, `^` and `|` take integer operands** (C 6.5.7p2, 6.5.10–6.5.12p2).
    ///
    /// The same question `%` asks — wave 362 wrote that arm for the multiplicative operators and
    /// these five never joined it. Kept as one predicate rather than two arms because the answer
    /// is identical and the *vector* rule is not: an integer vector counts here, a floating one
    /// does not, where wave 362's arm takes any vector because a float vector may be multiplied.
    ///
    /// Asked of the **promoted** operands, where `char`, `_Bool` and an enumeration have already
    /// become `Ty::Int`. Poison and incomplete types stay silent (contract 20).
    /// Whether either operand is a **complete** record, which no operator in 6.5 accepts but
    /// assignment and the member operators. Incomplete ones have already been reported.
    fn record_operand(&self, aty: TyId, bty: TyId) -> bool {
        let record = |t: TyId| {
            matches!(self.out.types[t.0 as usize], Ty::Record(_)) && !is_incomplete(&self.out, t)
        };
        record(aty) || record(bty)
    }

    fn check_integer_operands(&mut self, what: &str, aty: TyId, bty: TyId, span: Span) {
        let integer = |cx: &Self, t: TyId| match cx.out.types[t.0 as usize] {
            Ty::Int { .. } | Ty::Error => true,
            Ty::Vector { elem, .. } => matches!(cx.out.types[elem.0 as usize], Ty::Int { .. }),
            _ => is_incomplete(&cx.out, t),
        };
        if !integer(self, aty) || !integer(self, bty) {
            self.error(span, format!("{what} needs integer operands"));
        }
    }

    /// Whether `t` is a pointer whose pointee has no size (C 6.5.6p2).
    ///
    /// **One question for all seven spellings of pointer arithmetic.** `p + n`, `p - q`, `p++`,
    /// `p--`, `++p`, `--p`, `p += n`, `p -= n` and `p[i]` all scale by the pointee's size, and
    /// only the first two were asking — the rest are the same operation written shorter, so they
    /// share the predicate rather than repeating the match.
    ///
    /// **`void *` is deliberately not caught here.** `is_incomplete` excludes `Ty::Void` on
    /// purpose, because gcc defines `sizeof(void)` as 1 in the GNU mode the corpus uses and this
    /// engine implements that; an incomplete *record* has an unknown size rather than a defined
    /// one, which is what makes one an extension and the other a violation.
    fn incomplete_pointee(&self, t: TyId) -> bool {
        matches!(self.out.types[t.0 as usize], Ty::Ptr(p) if is_incomplete(&self.out, p))
    }

    /// Whether `base.field` names a **bit-field** (C 6.5.3.2p1).
    ///
    /// The base's type is read after decay, so `s->b` and `s.b` reach the same record — the two
    /// spellings differ only in whether a pointer is in the way, and the rule is about the member.
    fn is_bit_field(&mut self, base: ExprId, field: Symbol) -> bool {
        let node = self.type_expr(base);
        let bty = self.out.typed.ty_of(node);
        let rec = match self.out.types[bty.0 as usize].clone() {
            Ty::Record(r) => Some(r),
            Ty::Ptr(p) => match self.out.types[p.0 as usize] {
                Ty::Record(r) => Some(r),
                _ => None,
            },
            _ => None,
        };
        rec.and_then(|r| self.out.find_field(r, field))
            .is_some_and(|f| f.bits.is_some())
    }

    /// Report what is wrong with a literal's escapes (C 6.4.4.4), shape first and range second.
    ///
    /// At most **one diagnostic per literal**: three bad escapes in one string are one mistake
    /// about one string, which is contract 20's spirit applied below the declaration level.
    /// Shape is asked before range because a malformed escape has no value to be out of range.
    fn check_literal(&mut self, spelling: &str, bits: u32, span: Span) {
        self.check_literal_content(strlit::unquote(spelling), bits, span);
    }

    /// The same, given content the caller has already unquoted — which the two literal kinds must
    /// do differently.
    fn check_literal_content(&mut self, content: &str, bits: u32, span: Span) {
        let mut bad = Vec::new();
        let mut gnu_only = Vec::new();
        strlit::string_units_split(content, &mut bad, &mut gnu_only);
        // **Only the escapes gcc accepts *silently* follow the dialect.** `\%` and `\e` are
        // silent under `gnu11`; `\q` and `\8` warn, so going quiet on those would say
        // nothing where gcc speaks. 5 findings from `perfmon/arm/bundle/branch_pred.c`.
        if self.dialect.pedantic {
            bad.extend(gnu_only);
        }
        if let Some(first) = bad.first() {
            self.error(span, first.clone());
            return;
        }
        if let Some(why) = strlit::escape_range_defect(content, bits) {
            self.error(span, why);
        }
    }

    fn check_complete(&mut self, id: DeclId, ty: TyId) {
        // **`void` is the incomplete type that can never be completed.** It is deliberately not
        // part of `is_incomplete`: that predicate answers "may this be an object *yet*", and
        // every other caller — array elements, pointer arithmetic, `sizeof` — has its own reason
        // to treat `void` differently. `void *p` and `sizeof(void)` are both legal here, and
        // making `void` incomplete in general would reject them.
        let void_object = matches!(self.out.types[ty.0 as usize], Ty::Void);
        if !is_incomplete(&self.out, ty) && !void_object {
            return;
        }
        // **C 6.9.2p3: an `extern` declaration with no initializer is not a definition**, so no
        // size is needed here — the object is defined in another translation unit, and that is
        // where its type is completed. This is the opaque-handle idiom one indirection down, and
        // rejecting it turns a correct program into a broken one.
        if let DeclKind::Var {
            ref storage,
            init: None,
            ..
        } = self.ast.decl(id).kind
            && storage.extern_
        {
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
                    self.block_decl(d);
                }
            }
            StmtKind::Compound(ss) => {
                // Scopes opened inside this block end with it. Truncating rather than popping one
                // per declaration keeps the two in step even when a block opens several.
                let outer = self.open_vla_scopes.len();
                self.defined_tags.enter();
                self.declared_enumerators.enter();
                // **The function body's outermost block is the parameters' scope** (C 6.9.1p9),
                // so the body arm has already opened one and set this flag; opening a second
                // here would let `int f(int T){ typedef int T; }` shadow rather than collide.
                let own_scope = !std::mem::take(&mut self.body_scope_open);
                if own_scope {
                    self.meanings.enter();
                    self.values.enter();
                    self.values.enter();
                }
                for s in ss {
                    self.type_stmt(s);
                }
                if own_scope {
                    self.meanings.leave();
                    self.values.leave();
                    self.values.leave();
                }
                self.declared_enumerators.leave();
                self.defined_tags.leave();
                self.open_vla_scopes.truncate(outer);
            }
            StmtKind::If { cond, then, els } => {
                let c = self.type_expr(cond);
                self.condition(c, cond, "the condition of `if`");
                self.type_stmt(then);
                if let Some(e) = els {
                    self.type_stmt(e);
                }
            }
            StmtKind::While { cond, body } => {
                let c = self.type_expr(cond);
                self.condition(c, cond, "the condition of `while`");
                self.in_loop(|cx| cx.type_stmt(body));
            }
            StmtKind::DoWhile { body, cond } => {
                self.in_loop(|cx| cx.type_stmt(body));
                let c = self.type_expr(cond);
                self.condition(c, cond, "the condition of `do`");
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                // **A `for` initializer is its own scope** (C 6.8.5p5), which is why two
                // `for (int i = 0; ...)` loops in one block are legal C — and why the first
                // draft of the meanings table called the second `i` a redeclaration. Found by
                // the corpus and by an existing fixture in the same run.
                self.meanings.enter();
                self.values.enter();
                match init {
                    Some(ForInit::Decl(ds)) => {
                        for d in ds {
                            // **A `for` initializer is block-ish for linkage and not for
                            // storage** (C 6.8.5p3), so it takes the ordinary block walk *and*
                            // its own storage question. A `typedef` is not a storage class in
                            // this engine's AST — it is a `DeclKind` — so it is asked separately.
                            match self.ast.decl(d).kind.clone() {
                                DeclKind::Var { storage, .. } => self.check_storage_context(
                                    storage,
                                    StorageContext::ForInit,
                                    self.ast.decl(d).span,
                                ),
                                DeclKind::Typedef { .. } => self.error(
                                    self.ast.decl(d).span,
                                    "`typedef` is not allowed in a `for` initializer",
                                ),
                                _ => {}
                            }
                            self.block_decl(d);
                        }
                    }
                    Some(ForInit::Expr(e)) => {
                        self.type_expr(e);
                    }
                    None => {}
                }
                if let Some(c) = cond {
                    let n = self.type_expr(c);
                    self.condition(n, c, "the condition of `for`");
                }
                if let Some(s) = step {
                    self.type_expr(s);
                }
                self.in_loop(|cx| cx.type_stmt(body));
                self.meanings.leave();
                self.values.leave();
            }
            StmtKind::Switch { cond, body } => {
                let c = self.type_expr(cond);
                // **Any integer type, not `int`** (C 6.8.4.2p1). `switch(c)` on a `char`,
                // `switch(u)` on an `unsigned` and `switch(n)` on a `long` are all legal, so the
                // test is on the type's *category* — writing it against `int` rejects all three.
                //
                // Reading the operand's own type rather than the promoted one is **measured
                // equivalent**: promotion only widens narrow integers, which are `Ty::Int` before
                // and after, and it does not turn a `double` or a pointer into one. It is written
                // this way because the rule is about what the program said, but a reader moving it
                // after `promote_node` will not be caught.
                //
                // `Ty::Error` and incomplete types stay silent — contract 20.
                let cty = self.out.typed.ty_of(c);
                if !matches!(self.out.types[cty.0 as usize], Ty::Int { .. })
                    && !is_incomplete(&self.out, cty)
                {
                    let span = self.ast.expr(cond).span;
                    self.error(span, "switch quantity is not an integer");
                }
                self.promote_node(c, cond, self.ast.expr(cond).span);
                self.switches.push((Default::default(), false));
                self.switch_vla_depth.push(self.open_vla_scopes.len());
                self.breakable_depth += 1;
                self.type_stmt(body);
                self.breakable_depth -= 1;
                self.switch_vla_depth.pop();
                self.switches.pop();
            }
            StmtKind::Case { lo, hi, body } => {
                self.check_label_vla_scope(stmt, "a `case` label");
                self.type_expr(lo);
                if let Some(h) = hi {
                    self.type_expr(h);
                }
                // **The folded value, not the written expression.** `case 2-1` and `case 1` are
                // the same label. A case whose value will not fold is left alone: something else
                // has already complained, and inventing a value here would invent a duplicate.
                // **A case label is an integer constant expression** (C 6.8.4.2p3), judged by
                // what it folds to rather than how it is spelled: `case 'a':` and `case 1+1:` are
                // fine, `case m:` and `case 1.5:` are not. `eval` answers integers only, so a
                // failure to fold is both conditions at once — which is why one complaint covers
                // "not constant" and "not an integer" without having to tell them apart.
                //
                // A range's *bounds* are both constant expressions; the lower one is asked here
                // and the upper one below, where the interval is built.
                // **Folded once, and once only.** `eval` reports as it folds, so asking it three
                // times about `case 1/0:` — for this check, for the interval's lower bound and
                // for its upper — said "division by zero" twice and then added a third sentence
                // about the label. Contract 20 wants one report for one mistake, and the second
                // and third folds could tell a reader nothing the first had not.
                //
                // **And the first fold's own diagnostic is the better one.** "Division by zero in
                // a constant expression" says what is wrong; "case label is not an integer
                // constant expression" only says that something is. So the generic sentence is
                // added *unless* the fold already explained itself, the same rule wave 301 gave
                // enumerator values.
                let before = self.out.diagnostics.len();
                let folded_lo = self.eval(lo).map(|v| v.v);
                let explained = self.out.diagnostics.len() > before;
                if folded_lo.is_none() && !explained {
                    let span = self.ast.expr(lo).span;
                    self.error(span, "case label is not an integer constant expression");
                }
                // **A label needs a switch to belong to** — the same question `break` asks of
                // the loop stack, asked of the switch stack that wave 312 already built.
                if self.switches.is_empty() {
                    let span = self.ast.stmt(stmt).span;
                    self.error(span, "`case` label not within a switch");
                }
                // **The interval a label occupies.** A plain `case v:` is `[v, v]`; a range is
                // `[lo, hi]`. An **empty** range — `case 3 ... 1` — occupies its *lower bound*
                // and nothing else, which is gcc's rule and not an invention: `3 ... 1` collides
                // with `case 3:` and with a second `3 ... 1`, and not with `1` or `2`. That is
                // what `max` encodes, and it is why the old lower-bound-only rule looked right —
                // it was correct for exactly the empty case and wrong for every other range.
                let interval = match (folded_lo, hi) {
                    (Some(l), None) => Some((l, l)),
                    (Some(l), Some(h)) => {
                        // The upper bound is a second expression and folds on its own; its
                        // diagnostics are its own and are not suppressed.
                        self.eval(h).map(|v| (l, v.v.max(l)))
                    }
                    (None, _) => None,
                };
                if let Some((l, h)) = interval
                    && let Some((seen, _)) = self.switches.last_mut()
                {
                    // Two closed intervals meet when neither ends before the other begins.
                    let clash = seen.iter().any(|&(a, b)| l <= b && a <= h);
                    if clash {
                        let span = self.ast.expr(lo).span;
                        let what = if l == h {
                            format!("duplicate case value `{l}`")
                        } else {
                            format!("case range `{l} ... {h}` overlaps an earlier label")
                        };
                        self.error(span, what);
                    } else {
                        seen.push((l, h));
                    }
                }
                self.type_stmt(body);
            }
            StmtKind::Default { body } => {
                self.check_label_vla_scope(stmt, "a `default` label");
                if self.switches.is_empty() {
                    let span = self.ast.stmt(stmt).span;
                    self.error(span, "`default` label not within a switch");
                }
                let repeated = match self.switches.last_mut() {
                    Some((_, seen)) => std::mem::replace(seen, true),
                    None => false,
                };
                if repeated {
                    let span = self.ast.stmt(stmt).span;
                    self.error(span, "multiple `default` labels in one switch");
                }
                self.type_stmt(body);
            }
            StmtKind::Label { name, body } => {
                // **A label is defined once per function** (C 6.2.1p4). Its scope is the whole
                // function, which is what makes this its own rule rather than a case of ordinary
                // redeclaration: `a: { a: ; }` and two *sibling* blocks each defining `a` both
                // collide, where every other identifier in those positions would not. That is
                // also why nothing had to be built for it — `labels_defined` is already
                // function-wide and cleared per function, so the collision is what `insert`
                // returns and the block structure never enters into it.
                if !self.labels_defined.insert(name) {
                    let n = self.text(name).unwrap_or("?").to_owned();
                    let span = self.ast.stmt(stmt).span;
                    self.error(span, format!("duplicate label `{n}`"));
                }
                // **Where the label sits, in variably-modified terms.** Sampled at the label and
                // not at the block, because `{ skip: ; int a[n]; }` puts the label outside the
                // scope and `{ int a[n]; skip: ; }` puts it inside — the same block either way.
                self.label_scopes.insert(name, self.open_vla_scopes.clone());
                self.type_stmt(body);
            }
            StmtKind::Return(None) => {
                // **`return;` needs a `void` function** (C 6.8.6.4p1) — the mirror of the rule
                // beside it, and the reason both live in one paragraph. `current_ret` is `None`
                // outside a function body, where the parser has already reported.
                if let Some(ret) = self.current_ret
                    && !matches!(self.out.types[ret.0 as usize], Ty::Void | Ty::Error)
                {
                    let span = self.ast.stmt(stmt).span;
                    self.error(
                        span,
                        "`return` with no value in a function returning non-`void`",
                    );
                }
            }
            StmtKind::Return(Some(e)) => {
                let node = self.type_expr(e);
                // **A `return` with a value in a `void` function** (C 6.8.6.4p1). Checked here
                // rather than through `coerce`, which is never reached: there is no return type
                // to convert *to*, so the conversion that would have complained never happens.
                if self
                    .current_ret
                    .is_some_and(|r| matches!(self.out.types[r.0 as usize], Ty::Void))
                {
                    // **The two halves differ and only one is a dialect question.** Measured:
                    // `return void_expr;` is accepted *silently* by `gcc -std=gnu11` and
                    // refused by `-pedantic-errors`; `return 5;` is **warned** about by
                    // `gnu11`. Chiero has no warning level, so silencing the second would say
                    // nothing where gcc says something — a `Miss`, not agreement. VPP wraps
                    // void calls this way 735 times through `vnet/interface_funcs.h`.
                    let expr_ty = self.out.typed.ty_of(node);
                    let returns_void = matches!(self.out.types[expr_ty.0 as usize], Ty::Void);
                    if self.dialect.pedantic || !returns_void {
                        let span = self.ast.expr(e).span;
                        self.error(span, "`return` with a value in a function returning `void`");
                    }
                }
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
            StmtKind::Goto(name) => {
                self.labels_used.push((
                    name,
                    self.ast.stmt(stmt).span,
                    self.open_vla_scopes.clone(),
                ));
            }
            StmtKind::Break => {
                if self.breakable_depth == 0 {
                    let span = self.ast.stmt(stmt).span;
                    self.error(span, "`break` outside a loop or switch");
                }
            }
            StmtKind::Continue => {
                // **Loops only.** A `switch` is breakable but not continuable, so `continue`
                // inside a switch inside a loop continues the loop and is legal.
                if self.loop_depth == 0 {
                    let span = self.ast.stmt(stmt).span;
                    self.error(span, "`continue` outside a loop");
                }
            }
            // An attribute statement executes nothing; the misplaced-`fallthrough` rule that
            // would read its attributes is a 040 checker, not a 014 constraint (HANDOFF §9).
            StmtKind::Empty | StmtKind::Attr(_) | StmtKind::Error => {}
        }
    }

    /// A controlling expression is compared against zero, so it decays but is not
    /// promoted to a common type with anything.
    fn condition(&mut self, node: TypedId, expr: ExprId, what: &str) {
        let node = self.decay(node, expr);
        self.require_scalar(node, expr, what);
        self.set_top(expr, node);
    }

    /// **Where C asks a value "is it true", the value must be scalar** — C 6.8.4.1p1 (`if`),
    /// 6.8.5p2 (`while`, `do`, `for`), 6.5.15p2 (`?:`), 6.5.3.3p1 (`!`), 6.5.13/14p2 (`&&` and
    /// `||`). A structure or a union has no zero to compare against, and `void` has no value.
    ///
    /// **Asked of the decayed node**, which is the whole reason `if(a)` on an array is legal:
    /// the array has become a pointer by the time the question is put, so this needs no array
    /// arm and a rule phrased "integer or pointer" over the *written* type would have rejected
    /// it. Every one of the eight callers decays first for its own reasons; this one depends on
    /// it, so the two are done together in `condition` and immediately after the decay elsewhere.
    ///
    /// A vector is **not** scalar here, matching gcc, which refuses `if(v)` in GNU mode too.
    fn require_scalar(&mut self, node: TypedId, expr: ExprId, what: &str) {
        let t = self.out.typed.ty_of(node);
        if !matches!(
            self.out.types[t.0 as usize],
            Ty::Int { .. } | Ty::Float(_) | Ty::Ptr(_) | Ty::Error
        ) {
            let span = self.ast.expr(expr).span;
            self.error(span, format!("{what} needs a scalar, and this is not one"));
        }
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
                // **No vector arm here, and that is not an oversight.** The obvious symmetry
                // with the `Index` typing above is wrong: this walk folds *constant addresses*,
                // and C does not allow a vector subscript in one. `v4si g = {1,2,3,4};
                // int *gp = &g[2];` at file scope is `error: initializer element is not
                // constant` in gcc, where the same line over an array is fine. So a vector
                // cannot reach this match, and an arm for it would be unreachable code that
                // reads like coverage. Measured, wave 272: adding one changed no fixture's
                // answer either way.
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
                let f = self.out.find_field(rec, *field)?;
                Some((name, off.checked_add(f.offset as i64)?, f.ty))
            }
            _ => None,
        }
    }
}

/// The C spelling of a builtin, for diagnostics that name a written type.
fn builtin_spelling(b: chiero_ast::Builtin) -> &'static str {
    use chiero_ast::Builtin as B;
    match b {
        B::Void => "void",
        B::Bool => "_Bool",
        B::Char => "char",
        B::SChar => "signed char",
        B::UChar => "unsigned char",
        B::Short => "short",
        B::UShort => "unsigned short",
        B::Int => "int",
        B::UInt => "unsigned int",
        B::Long => "long",
        B::ULong => "unsigned long",
        B::LongLong => "long long",
        B::ULongLong => "unsigned long long",
        B::Int128 => "__int128",
        B::UInt128 => "unsigned __int128",
        B::Float => "float",
        B::Double => "double",
        B::LongDouble => "long double",
        _ => "that type",
    }
}
