//! `chiero-sema` — name resolution, types, layout and constant evaluation.
//! See `docs/specs/014-semantics-and-types.md`.
//!
//! **Layout correctness is the load-bearing part.** Every symbolic memory offset in 021
//! derives from a struct layout computed here, so a one-byte error produces confident,
//! wrong answers throughout the entire system rather than a visible failure. That is why
//! 014 §7 validates layout differentially against the real compiler instead of against
//! hand-written expectations: the expectations are exactly what a layout bug corrupts.

use chiero_ast::{Ast, DeclId, ExprId};
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
        variadic: bool,
    },
    Record(RecordId),
    /// `__attribute__((vector_size(n)))`.
    Vector {
        elem: TyId,
        lanes: u32,
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

/// 014 §6's integer subset — enough for array bounds, bit-field widths, enum values,
/// `_Static_assert` and case labels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConstVal {
    Int(i128),
}

/// The result of analysing one translation unit.
#[derive(Debug, Default)]
pub struct Analysis {
    pub(crate) types: Vec<Ty>,
    pub(crate) interned: IndexMap<Ty, TyId>,
    pub(crate) records: Vec<RecordLayout>,
    pub(crate) by_tag: IndexMap<Symbol, RecordId>,
    pub(crate) decl_types: IndexMap<DeclId, TyId>,
    pub(crate) target: Option<TargetConfig>,
    pub diagnostics: Vec<SemaDiagnostic>,
}

impl Analysis {
    pub fn ty(&self, id: TyId) -> &Ty {
        &self.types[id.0 as usize]
    }

    pub fn layout(&self, id: RecordId) -> &RecordLayout {
        &self.records[id.0 as usize]
    }

    /// The record defined with this tag, if the TU defined one.
    pub fn record_by_tag(&self, tag: Symbol) -> Option<RecordId> {
        self.by_tag.get(&tag).copied()
    }

    pub fn ty_of_decl(&self, d: DeclId) -> Option<TyId> {
        self.decl_types.get(&d).copied()
    }

    pub fn records(&self) -> &[RecordLayout] {
        &self.records
    }

    /// Size in bytes, or `None` for a type that has none — a function, or `Error`.
    pub fn size_of(&self, id: TyId) -> Option<u64> {
        let _ = id;
        todo!("014 §3")
    }

    pub fn align_of(&self, id: TyId) -> Option<u64> {
        let _ = id;
        todo!("014 §3")
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

/// Analyse one TU's AST against a target (014 §§2–6).
pub fn analyze(ast: &Ast, target: &TargetConfig, names: &dyn SymbolText) -> Analysis {
    let _ = (ast, target, names);
    todo!("014 §§2-6: types, layout and constant evaluation")
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
    out: &mut Vec<SemaDiagnostic>,
) -> Option<ConstVal> {
    let _ = (ast, expr, names, out);
    todo!("014 §6: constant evaluation")
}
