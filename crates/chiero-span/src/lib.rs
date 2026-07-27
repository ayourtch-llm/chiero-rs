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
        self == Self::ROOT
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
    pub const DUMMY: Span = Span {
        lo: BytePos(0),
        hi: BytePos(0),
        ctx: ExpnCtx::ROOT,
    };

    /// `lo` is inclusive, `hi` exclusive. Debug-asserts `lo <= hi`; an inverted span
    /// is a frontend bug, and silently normalizing one would hide it.
    pub fn new(lo: BytePos, hi: BytePos, ctx: ExpnCtx) -> Span {
        debug_assert!(lo <= hi, "inverted span: {lo:?}..{hi:?}");
        Span { lo, hi, ctx }
    }

    pub fn len(self) -> u32 {
        self.hi.0.saturating_sub(self.lo.0)
    }

    pub fn is_empty(self) -> bool {
        self.lo >= self.hi
    }

    /// Half-open: `lo` is contained, `hi` is not.
    pub fn contains(self, pos: BytePos) -> bool {
        self.lo <= pos && pos < self.hi
    }

    pub fn is_dummy(self) -> bool {
        self == Self::DUMMY
    }
}

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Index into `SourceMap::files`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

/// A resolved position: file, 1-based line and column, and the global offset.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Loc {
    pub file: FileId,
    pub line: u32,
    pub col: u32,
    pub pos: BytePos,
}

/// One source file, occupying `[start_pos, start_pos + len)` of the global space.
#[derive(Debug)]
pub struct SourceFile {
    id: FileId,
    path: PathBuf,
    src: Arc<str>,
    /// Global-space range this file occupies (010 §3).
    pub start_pos: BytePos,
    /// Byte offset of the start of each line, for O(log n) offset→line.
    line_starts: Vec<BytePos>,
}

impl SourceFile {
    pub fn id(&self) -> FileId {
        self.id
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn src(&self) -> &str {
        &self.src
    }
    pub fn byte_len(&self) -> u32 {
        todo!("green")
    }
    pub fn is_empty(&self) -> bool {
        todo!("green")
    }
    pub fn end_pos(&self) -> BytePos {
        todo!("green")
    }
    pub fn line_count(&self) -> u32 {
        todo!("green")
    }
}

/// Owns every source file in one global `BytePos` space, so a `Span` needs no
/// `FileId` field (010 §3).
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, _path: impl Into<PathBuf>, _src: impl Into<Arc<str>>) -> FileId {
        todo!("green")
    }

    pub fn file(&self, _id: FileId) -> &SourceFile {
        todo!("green")
    }
    pub fn files(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }
    pub fn lookup_file(&self, _pos: BytePos) -> Option<FileId> {
        todo!("green")
    }
    pub fn lookup_loc(&self, _pos: BytePos) -> Option<Loc> {
        todo!("green")
    }
    pub fn span_text(&self, _sp: Span) -> Option<&str> {
        todo!("green")
    }
}
