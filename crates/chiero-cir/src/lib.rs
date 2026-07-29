//! CIR — chiero's intermediate representation. See `docs/specs/020-cir.md`.
//!
//! **This crate depends on no frontend crate** (001 §3). That is the single most
//! important structural rule in the project: it is what lets the entire symbolic core be
//! built and tested against hand-written `.cir` before a line of C is parsed.

use chiero_span::Span;
use indexmap::IndexMap;
use smallvec::SmallVec;
use std::sync::Arc;

pub mod text;
pub mod verify;

pub use verify::{VerifyError, VerifyErrorKind, verify};

/// A name. Not interned yet — CIR names are cold, and an interner belongs in
/// `chiero-span` once more than one crate needs it.
pub type Symbol = Arc<str>;

/// Lowered types. No typedefs, no qualifiers, no field names (020 §2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CTy {
    Void,
    /// Bit width. `_Bool` is `Int(1)`, `__int128` is `Int(128)`.
    Int(u32),
    Float(FloatKind),
    /// Opaque: pointers are address-sized and untyped. The pointee type lives on the
    /// access, because C's type punning makes a typed-pointer IR a source of lies.
    Ptr,
    Vector {
        elem: Box<CTy>,
        lanes: u32,
    },
}

impl CTy {
    /// Total width in bits, or `None` for `Void` and `Ptr` — a pointer's width is
    /// target-dependent and so is not a property of the type alone.
    pub fn bit_width(&self) -> Option<u32> {
        match self {
            CTy::Int(b) => Some(*b),
            CTy::Float(f) => Some(f.bits()),
            CTy::Vector { elem, lanes } => elem.bit_width().map(|w| w * lanes),
            CTy::Void | CTy::Ptr => None,
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, CTy::Int(_))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FloatKind {
    F32,
    F64,
    X87_80,
}

impl FloatKind {
    pub fn bits(self) -> u32 {
        match self {
            FloatKind::F32 => 32,
            FloatKind::F64 => 64,
            FloatKind::X87_80 => 80,
        }
    }
}

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);
    };
}

id_type!(/// Index into `Function::blocks`.
    BlockId);
id_type!(/// Single-assignment temporary within a function.
    ValueId);
id_type!(/// Index into `Function::allocas`.
    AllocaId);
id_type!(/// Index into `Module::funcs`.
    FuncId);
id_type!(/// Index into `Module::globals`.
    GlobalId);
id_type!(/// Lexical scope, for stack-object lifetime (020 §4.4).
    ScopeId);

/// A bit range within a storage unit, for bitfields (020 §4.5.1).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BitRange {
    pub off: u32,
    pub width: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Volatility {
    Normal,
    Volatile,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    /// Stored sign-extended; interpretation is per-operation.
    Int {
        bits: u32,
        val: i128,
    },
    /// Widths above 128 bits — AVX-512 vectors and masks. Little-endian limbs.
    Wide {
        bits: u32,
        words: Vec<u64>,
    },
    /// Raw bits, so NaN payloads survive a round trip.
    Float(FloatKind, u64),
    Null,
    GlobalAddr {
        g: GlobalId,
        off: i64,
    },
    FuncAddr(FuncId),
    Undef(CTy),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Value(ValueId),
    Const(Const),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    UDiv,
    SDiv,
    URem,
    SRem,
    And,
    Or,
    Xor,
    Shl,
    LShr,
    AShr,
    FAdd,
    FSub,
    FMul,
    FDiv,
    FRem,
    PtrDiff { elem_size: u64 },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    FNeg,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    ULt,
    ULe,
    UGt,
    UGe,
    SLt,
    SLe,
    SGt,
    SGe,
    /// Ordered: false if either operand is NaN.
    FOEq,
    FONe,
    FOLt,
    FOLe,
    /// Unordered: true if either operand is NaN. **C's `isnan` idiom is `x != x`**,
    /// which is an *unordered* not-equal — `FONe` is false for NaN, the opposite of
    /// what C means, so the idiom has no correct lowering without these.
    FUEq,
    FUNe,
    FULt,
    FULe,
    /// "Neither is NaN".
    FOrd,
    /// "At least one is NaN".
    FUno,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CastKind {
    Trunc,
    ZExt,
    SExt,
    FpTrunc,
    FpExt,
    FpToUi,
    FpToSi,
    UiToFp,
    SiToFp,
    PtrToInt,
    IntToPtr,
    Bitcast,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RValue {
    Use(Operand),
    Load {
        addr: Operand,
        ty: CTy,
        align: u64,
        vol: Volatility,
    },
    LoadBits {
        addr: Operand,
        unit: CTy,
        bits: BitRange,
        signed: bool,
        align: u64,
    },
    Bin {
        op: BinOp,
        a: Operand,
        b: Operand,
        ty: CTy,
        /// Whether the C operands were **signed**, which is what C's arithmetic UB rules
        /// turn on and what nothing else here records.
        ///
        /// `SDiv`/`UDiv`, `SRem`/`URem` and `AShr`/`LShr` are separate opcodes because the
        /// *machine* operation differs. For `Add`, `Sub`, `Mul` and `Shl` it does not —
        /// the same instruction computes both — so splitting the opcode would name a
        /// distinction the hardware does not make. The distinction C makes is in the
        /// undefinedness, not the result: signed overflow and a signed left shift past the
        /// sign bit are undefined, and the unsigned forms are defined to wrap.
        ///
        /// So the bit rides on the instruction, as LLVM's `nsw` does, rather than on the
        /// opcode or on `CTy::Int(w)` — which carries a width and no signedness, and would
        /// have to grow one everywhere to answer a question only arithmetic asks.
        ///
        /// Ignored by the float opcodes: IEEE overflow is defined (it yields an infinity),
        /// so there is no rule for this bit to select.
        signed: bool,
    },
    Un {
        op: UnOp,
        a: Operand,
        ty: CTy,
    },
    Cmp {
        op: CmpOp,
        a: Operand,
        b: Operand,
        ty: CTy,
    },
    Cast {
        kind: CastKind,
        a: Operand,
        from: CTy,
        to: CTy,
    },
    Select {
        cond: Operand,
        t: Operand,
        f: Operand,
    },
    /// Distinct from integer `Add` so pointer provenance survives arithmetic
    /// (020 §4.1, 021 §2).
    PtrAdd {
        base: Operand,
        off: Operand,
    },
    AddrOfLocal {
        alloca: AllocaId,
    },
    AddrOfGlobal {
        g: GlobalId,
    },
    AddrOfFunc(FuncId),
    Shuffle {
        a: Operand,
        b: Operand,
        mask: Vec<u32>,
    },
    InsertLane {
        v: Operand,
        lane: u32,
        val: Operand,
    },
    ExtractLane {
        v: Operand,
        lane: u32,
    },
    Splat {
        elem: Operand,
        lanes: u32,
    },
    Fresh {
        ty: CTy,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Line(u32),
    SeqPoint,
    Scope(ScopeEvent),
    Label(Symbol),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScopeEvent {
    pub scope: ScopeId,
    pub kind: ScopeKind,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Enter,
    Exit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstKind {
    Assign {
        dst: ValueId,
        rv: RValue,
    },
    Store {
        addr: Operand,
        val: Operand,
        ty: CTy,
        align: u64,
        vol: Volatility,
    },
    StoreBits {
        addr: Operand,
        val: Operand,
        unit: CTy,
        bits: BitRange,
        align: u64,
    },
    CopyMem {
        dst: Operand,
        src: Operand,
        size: Operand,
        align: u64,
    },
    SetMem {
        dst: Operand,
        byte: Operand,
        size: Operand,
    },
    Call {
        dst: Option<ValueId>,
        callee: Callee,
        args: Vec<Operand>,
    },
    /// VLA / `alloca()`: a stack allocation with a runtime size, at a real program point
    /// so ordinary dominance applies to `count` (020 §3).
    AllocaDyn {
        dst: ValueId,
        alloca: AllocaId,
        elem: CTy,
        count: Operand,
        align: u64,
    },
    /// An instruction, not an `RValue`: it advances the list (020 §4.4.1).
    VaArg {
        dst: ValueId,
        list: Operand,
        ty: CTy,
    },
    VaStart {
        list: Operand,
    },
    VaCopy {
        dst: Operand,
        src: Operand,
    },
    VaEnd {
        list: Operand,
    },
    /// **Inline `asm`, and anything chiero parses but refuses to model** (020 §4.3).
    ///
    /// `dsts` exists because most inline asm produces values in *registers*, not memory:
    /// `vppinfra/time.h` alone has six sites shaped like
    /// `asm volatile ("rdtsc":"=a"(a),"=d"(d))`, and `clib_cpu_time_now()` is called from
    /// the dispatch loop. With only `writes` there would be no way to express them, and
    /// lowering's options would be to drop the asm, to invent an unattached `Fresh` that
    /// a CSE pass could then merge across two `rdtsc` calls, or to refuse the function.
    ///
    /// Each `dst` is a fresh symbol, distinct per instruction, never CSE'd, never cached,
    /// never reordered. Two textually identical `rdtsc` instructions yield two different
    /// values, which is the entire point of reading a clock.
    Opaque {
        dsts: Vec<(ValueId, CTy)>,
        writes: Vec<OpaqueWrite>,
        reads: Vec<Operand>,
        why: OpaqueReason,
    },
    /// **The one non-SSA exception, and only `mem2reg` may emit it** (020 §9).
    ///
    /// `dst` takes the incoming value belonging to the edge actually taken into this
    /// block. Its operands are therefore *not* evaluated where they appear, which makes
    /// `Phi` the only instruction ordinary dominance is the wrong rule for: an incoming
    /// from `bb1` is defined in `bb1`, and `bb1` does not dominate the join.
    ///
    /// `incomings` carries one entry per predecessor, in the block order of those
    /// predecessors — sorted rather than insertion-ordered so two runs print identically
    /// (020 contract 21).
    Phi {
        dst: ValueId,
        ty: CTy,
        incomings: Vec<(BlockId, Operand)>,
    },
    Marker(MarkerKind),
}

/// A region an `Opaque` clobbers.
#[derive(Clone, Debug, PartialEq)]
pub struct OpaqueWrite {
    pub addr: Operand,
    pub size: Operand,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpaqueReason {
    InlineAsm,
    UnmodeledBuiltin(Symbol),
    /// 020 §4.3 writes this as `&'static str`, which lowering can satisfy but the
    /// textual parser cannot: a `.cir` fixture's text is owned, not static. `Symbol` is
    /// an `Arc<str>`, so the representation costs the same and round-trips.
    UnsupportedConstruct(Symbol),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    Direct(FuncId),
    Indirect(Operand),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inst {
    pub kind: InstKind,
    pub span: Span,
    /// **Introduced by lowering, not written in the source** — the `&&` shape's slot
    /// store, an implicit `Scope(Exit)`, a widening the C program did not name.
    ///
    /// 020 contract 15 requires this to be a *recorded* property rather than a guess.
    /// "It had no source span" is a different fact: a lowering bug that lost a span looks
    /// identical, and 015 §5's `gcov_lines` rule turns on this flag — a block of only
    /// generated instructions has no lines, because gcov has no counter for it either.
    pub generated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    Goto(BlockId),
    Br {
        cond: Operand,
        t: BlockId,
        f: BlockId,
    },
    Switch {
        scrut: Operand,
        ty: CTy,
        cases: Vec<(i128, BlockId)>,
        default: BlockId,
    },
    Return(Option<Operand>),
    IndirectGoto {
        addr: Operand,
        targets: Vec<BlockId>,
    },
    Unreachable(UnreachableReason),
}

impl Terminator {
    /// Successor blocks, in the deterministic order exploration must follow.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Terminator::Goto(b) => vec![*b],
            Terminator::Br { t, f, .. } => vec![*t, *f],
            Terminator::Switch { cases, default, .. } => {
                let mut v: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
                v.push(*default);
                v
            }
            Terminator::IndirectGoto { targets, .. } => targets.clone(),
            Terminator::Return(_) | Terminator::Unreachable(_) => Vec::new(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnreachableReason {
    AfterNoreturn,
    ExhaustiveSwitch,
    BuiltinUnreachable,
    /// Reaching this is `Fidelity::Unknown` and a diagnostic — never a licence to treat
    /// the path as infeasible (020 §5).
    LoweringGap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: BlockId,
    pub insts: Vec<Inst>,
    pub term: Terminator,
    /// Lines gcov attributes to this block. Computed via `expansion_loc`, never
    /// `spelling_loc` (015 §5).
    pub gcov_lines: SmallVec<[u32; 4]>,
    pub span: Span,
}

/// Sentinel for `AllocaDecl::count` meaning "extent supplied by an `AllocaDyn`".
pub const DYNAMIC_EXTENT: u64 = u64::MAX;

#[derive(Clone, Debug, PartialEq)]
pub struct AllocaDecl {
    pub id: AllocaId,
    pub ty: CTy,
    /// Static element count, or `DYNAMIC_EXTENT` when an `AllocaDyn` supplies it.
    pub count: u64,
    pub align: u64,
    pub scope: ScopeId,
    pub lifetime: Lifetime,
    pub name: Option<Symbol>,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lifetime {
    Scope,
    /// `alloca()` lives until *function* return, unlike a VLA (020 §3).
    Function,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub value: ValueId,
    pub ty: CTy,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FnAttrs {
    pub noreturn: bool,
    pub no_side_effects: bool,
    pub order_sensitive: bool,
    pub march_variant: Option<Symbol>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Defined,
    Declared,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub id: FuncId,
    pub name: Symbol,
    pub params: Vec<Param>,
    pub ret: CTy,
    pub variadic: bool,
    pub allocas: Vec<AllocaDecl>,
    pub blocks: Vec<Block>,
    pub entry: BlockId,
    pub attrs: FnAttrs,
    pub body: Body,
    /// `Internal` for a `static` function, `External` otherwise.
    ///
    /// Carried because it changes what an analysis may *assume about the caller*. Every
    /// call site of an internal function is in this module, so a property of its arguments
    /// is something chiero can look up rather than guess; an external function can be
    /// called from a translation unit chiero will never see, and there guessing is all
    /// there is. 021 §6's "start at each exported function in turn" is the same
    /// distinction from the other side.
    ///
    /// `Global` has carried this since 020 §3. A function had nowhere to put it, so the
    /// engine could not tell the two cases apart and applied the external assumption to
    /// both.
    pub linkage: Linkage,
    /// Reporting-only access paths, keyed by the address `ValueId` they describe
    /// (020 §4.4). Absent for most values; a finding without one simply says less.
    pub access_paths: IndexMap<ValueId, AccessPath>,
    pub span: Span,
}

impl Function {
    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }
}

/// Where an access started, for reporting (020 §4.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathRoot {
    Local {
        alloca: AllocaId,
        /// `None` for a compiler-generated temporary, which has no name to print.
        name: Option<Symbol>,
    },
    Global {
        g: GlobalId,
        name: Symbol,
    },
    /// A pointer that arrived from somewhere this function cannot see.
    Value(ValueId),
}

/// One step from an access's root to the bytes it touched (020 §4.4).
///
/// No `Eq`: `Index` carries an `Operand`, which can be a float constant.
#[derive(Clone, Debug, PartialEq)]
pub enum PathStep {
    Field {
        name: Symbol,
        off: u64,
    },
    /// Which member of a union this access viewed the bytes through, and the record it
    /// belongs to. Purely for reporting — see 020 §4.5.
    UnionMember {
        name: Symbol,
        off: u64,
        view: Symbol,
    },
    Bits {
        name: Symbol,
        bits: BitRange,
    },
    Index(Operand),
    Deref,
}

/// How an access reached the bytes it touched — **reporting only** (020 §4.4).
///
/// "It makes a finding read `p->adj[3].counter`, or `b->opaque as
/// ip4_rewrite_t.adj_index`, instead of `*(i64*)(%7 + 24)`. No analysis may branch on it."
///
/// Which is why these live in a side table on [`Function`] keyed by the address's
/// `ValueId`, rather than on the `Inst`. An instruction that carried one would put it in
/// front of every pass and every checker, and "no analysis may branch on it" would be a
/// rule nobody could see they were breaking.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessPath {
    pub root: PathRoot,
    pub steps: SmallVec<[PathStep; 4]>,
}

impl AccessPath {
    /// The path as C would write it.
    ///
    /// Never fails and never returns nothing: a path that panicked on an unnamed root
    /// would take the whole finding with it, and a finding that says less is better than
    /// no finding at all.
    pub fn render(&self) -> String {
        let mut out = match &self.root {
            PathRoot::Local { name: Some(n), .. } => n.to_string(),
            PathRoot::Local { alloca, name: None } => format!("%alloca{}", alloca.0),
            PathRoot::Global { name, .. } => name.to_string(),
            PathRoot::Value(v) => format!("%{}", v.0),
        };
        // **A `Deref` immediately followed by a member renders as `->`**, which is what
        // 020 §4.4's own example writes (`b->opaque as ip4_rewrite_t.adj_index`). Rendering
        // it as `(*b).opaque` is equivalent C and is not what anybody typed, so a reader
        // matching a finding against their source has to translate it back.
        let mut pending_deref = false;
        for st in &self.steps {
            let sep = if std::mem::take(&mut pending_deref) {
                "->"
            } else {
                "."
            };
            match st {
                PathStep::Deref => {
                    // A trailing deref with no member after it is a plain dereference.
                    if sep == "->" {
                        out = format!("(*{out})");
                    }
                    pending_deref = true;
                }
                PathStep::Field { name, .. } => out = format!("{out}{sep}{name}"),
                // **`as` rather than `.`**, because the whole value of this step is saying
                // the bytes were viewed *through* something that may not be what wrote
                // them. `.adj_index` alone reads like an ordinary field access.
                // **The deref binds to the base, not to the view.** §4.4 writes
                // `b->opaque as ip4_rewrite_t.adj_index`: the `->` reaches the bytes and
                // the `.` selects within the view. `… as ip4_rewrite_t->adj_index` would
                // read as a pointer hop through the view's *name*, which is not a thing.
                PathStep::UnionMember { name, view, .. } => {
                    if sep == "->" {
                        out = format!("(*{out})");
                    }
                    out = format!("{out} as {view}.{name}");
                }
                PathStep::Bits { name, bits } => {
                    out = format!("{out}{sep}{name}:{}..{}", bits.off, bits.off + bits.width)
                }
                PathStep::Index(o) => {
                    if sep == "->" {
                        out = format!("(*{out})");
                    }
                    let idx = match o {
                        Operand::Const(Const::Int { val, .. }) => val.to_string(),
                        Operand::Value(v) => format!("%{}", v.0),
                        other => format!("{other:?}"),
                    };
                    out = format!("{out}[{idx}]");
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Global {
    pub id: GlobalId,
    pub name: Symbol,
    pub size: u64,
    pub align: u64,
    pub is_const: bool,
    /// 020 §3. Recorded here rather than left implicit: a global with no initializer is
    /// not the same fact as one initialized to zero, and a string literal's *bytes* are
    /// the whole reason its address is worth having.
    pub init: GlobalInit,
    pub linkage: Linkage,
    pub span: Span,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GlobalInit {
    /// C's default for static storage: all bytes zero.
    #[default]
    Zero,
    /// Literal bytes — a string literal, or a fully constant aggregate.
    Bytes(Vec<u8>),
    /// Defined in another translation unit; the bytes are not this module's to state.
    Extern,
    /// **The address of another global**, plus a byte offset: `int *gp = &g;` or
    /// `int *gp = ga;` or `int *gp = &ga[1];`.
    ///
    /// Its own variant rather than `Bytes`, because an address is not a byte pattern. 021
    /// gives every pointer an *object*, and the bytes of an address carry no provenance —
    /// so encoding one as `Bytes` would produce a pointer that dereferences to nothing and
    /// compares equal to null. Lowering used to fall back to `Zero` here, which made
    /// `gp == 0` answer *true* for a pointer that is definitely not null.
    Addr { g: GlobalId, off: i64 },
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Linkage {
    #[default]
    External,
    Internal,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Module {
    pub funcs: Vec<Function>,
    pub globals: Vec<Global>,
    /// `None` for hand-written `.cir`, which legitimately has no build configuration.
    pub config: Option<u64>,
    /// `IndexMap`, not `HashMap`: 001 §5 makes determinism a hard requirement.
    pub metadata: IndexMap<String, String>,
}
