//! Spans, source maps and macro provenance. See `docs/specs/010-source-and-provenance.md`.
//!
//! This crate depends on no other `chiero-*` crate (001 §4 rule 5) and is depended on by
//! everything, so it must stay small and stable.

/// Byte offset into the global concatenated source space owned by the `SourceMap`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BytePos(pub u32);

/// Index into `SourceMap::expansions`. `ROOT` means "written literally in a source
/// file, not produced by any macro".
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpnCtx(pub u32);

impl ExpnCtx {
    pub const ROOT: ExpnCtx = ExpnCtx(0);

    pub fn is_root(self) -> bool {
        todo!("green")
    }
}

/// A byte range plus the expansion context it was produced in.
///
/// **12 bytes, `Copy`** — stored on every token, AST node and CIR instruction
/// (010 §2, contract 1). Do not grow it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub lo: BytePos,
    pub hi: BytePos,
    pub ctx: ExpnCtx,
}

impl Span {
    /// The span of something with no source location — hand-written `.cir` fixtures,
    /// synthesized nodes (020 §6).
    pub const DUMMY: Span = Span { lo: BytePos(0), hi: BytePos(0), ctx: ExpnCtx::ROOT };

    pub fn new(_lo: BytePos, _hi: BytePos, _ctx: ExpnCtx) -> Span {
        todo!("green")
    }

    pub fn len(self) -> u32 {
        todo!("green")
    }

    pub fn is_empty(self) -> bool {
        todo!("green")
    }

    pub fn contains(self, _pos: BytePos) -> bool {
        todo!("green")
    }

    pub fn is_dummy(self) -> bool {
        todo!("green")
    }
}
