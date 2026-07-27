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
        self.src.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.src.is_empty()
    }

    /// Exclusive end of this file's range in the global space.
    pub fn end_pos(&self) -> BytePos {
        BytePos(self.start_pos.0 + self.byte_len())
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// 1-based line containing the file-relative byte offset `off`.
    fn line_of_offset(&self, off: u32) -> u32 {
        // The count of line starts at or before `off` is exactly the 1-based line.
        self.line_starts.partition_point(|s| s.0 <= off) as u32
    }

    /// Byte offset of the start of each line. An empty file has zero lines; a file
    /// with no trailing newline still has its final line recorded.
    fn compute_line_starts(src: &str) -> Vec<BytePos> {
        if src.is_empty() {
            return Vec::new();
        }
        let mut starts = vec![BytePos(0)];
        let bytes = src.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            // Only `\n` begins a line. `\r` belongs to the preceding line, so CRLF
            // does not create a phantom line. A trailing newline does not either.
            if b == b'\n' && i + 1 < bytes.len() {
                starts.push(BytePos(i as u32 + 1));
            }
        }
        starts
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

    /// Append a file to the global space. The same path may be added more than once —
    /// the preprocessor reads a header under several configurations, and merging them
    /// would make one configuration's spans point into another's text.
    pub fn add_file(&mut self, path: impl Into<PathBuf>, src: impl Into<Arc<str>>) -> FileId {
        let src: Arc<str> = src.into();
        let id = FileId(self.files.len() as u32);
        let start_pos = self.files.last().map_or(BytePos(0), |f| f.end_pos());
        let line_starts = SourceFile::compute_line_starts(&src);
        self.files.push(SourceFile {
            id,
            path: path.into(),
            src,
            start_pos,
            line_starts,
        });
        id
    }

    pub fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    pub fn files(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }

    /// O(log n) in the number of files.
    ///
    /// Empty files claim no positions. `partition_point` already lands on the *last*
    /// file whose start is at or before `pos`, and a run of empty files shares its
    /// start with the non-empty file that follows, so no backward walk is needed —
    /// the single containment check below is sufficient.
    pub fn lookup_file(&self, pos: BytePos) -> Option<FileId> {
        let i = self
            .files
            .partition_point(|f| f.start_pos <= pos)
            .checked_sub(1)?;
        let f = &self.files[i];
        (pos < f.end_pos()).then_some(f.id)
    }

    pub fn lookup_loc(&self, pos: BytePos) -> Option<Loc> {
        let id = self.lookup_file(pos)?;
        let f = self.file(id);
        let off = pos.0 - f.start_pos.0;
        let line = f.line_of_offset(off);
        let line_start = f.line_starts[line as usize - 1].0;
        Some(Loc {
            file: id,
            line,
            col: off - line_start + 1,
            pos,
        })
    }

    /// `None` when the span is out of range or straddles a file boundary — splicing
    /// bytes from two files would be worse than refusing.
    pub fn span_text(&self, sp: Span) -> Option<&str> {
        let id = self.lookup_file(sp.lo)?;
        let f = self.file(id);
        if sp.hi > f.end_pos() {
            return None;
        }
        let lo = (sp.lo.0 - f.start_pos.0) as usize;
        let hi = (sp.hi.0 - f.start_pos.0) as usize;
        f.src.get(lo..hi)
    }
}
