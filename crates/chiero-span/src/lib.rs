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
    expansions: Vec<Expansion>,
    macros: Vec<MacroInfo>,
    /// Reverse index: macro → every expansion of it. The test-selection primitive.
    by_macro: indexmap::IndexMap<MacroId, Vec<ExpnCtx>>,
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

/// Index into `SourceMap::macros`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacroId(pub u32);

/// The minimum a macro needs for *provenance*. `chiero-pp` owns the full definition
/// (012 §1); this is the slice `chiero-span` needs to answer 010 §3.1, and keeping it
/// here is what lets `chiero-span` depend on nothing (001 §4 rule 5).
#[derive(Debug, Clone)]
pub struct MacroInfo {
    pub name: Arc<str>,
    /// Where the macro's name appears in its `#define`.
    pub def_span: Span,
    /// Extent of the replacement list, used to tell a body token from an argument
    /// token (010 §2.2).
    pub body_extent: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExpnKind {
    ObjectLike,
    FunctionLike,
    Builtin,
    Pragma,
    Paste,
    Stringize,
}

/// One macro invocation (010 §2.1).
#[derive(Debug, Clone)]
pub struct Expansion {
    pub parent: ExpnCtx,
    pub macro_id: Option<MacroId>,
    pub call_site: Span,
    pub call_extent: Span,
    pub arg_spans: Vec<Span>,
    pub kind: ExpnKind,
}

/// One frame of an expansion backtrace, outermost-first.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExpnFrame {
    pub ctx: ExpnCtx,
    pub macro_id: Option<MacroId>,
    pub call_site: Span,
}

/// Where a token's text actually came from (010 §2.2).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TokenOrigin {
    MacroBody(MacroId),
    MacroArg { expn: ExpnCtx, arg_index: usize },
    Verbatim(FileId),
    Synthesized,
}

impl SourceMap {
    pub fn add_macro(&mut self, name: &str, def_span: Span, body_extent: Span) -> MacroId {
        let id = MacroId(self.macros.len() as u32);
        self.macros.push(MacroInfo {
            name: Arc::from(name),
            def_span,
            body_extent,
        });
        id
    }

    pub fn macro_info(&self, m: MacroId) -> Option<&MacroInfo> {
        self.macros.get(m.0 as usize)
    }

    pub fn expansion(&self, ctx: ExpnCtx) -> Option<&Expansion> {
        if ctx.is_root() {
            return None;
        }
        self.expansions.get(ctx.0 as usize - 1)
    }

    /// `ExpnCtx(0)` is reserved for ROOT, so expansions are stored from index 0 but
    /// numbered from 1.
    pub fn add_expansion(
        &mut self,
        parent: ExpnCtx,
        macro_id: Option<MacroId>,
        call_site: Span,
        call_extent: Span,
        arg_spans: Vec<Span>,
        kind: ExpnKind,
    ) -> ExpnCtx {
        let ctx = ExpnCtx(self.expansions.len() as u32 + 1);
        self.expansions.push(Expansion {
            parent,
            macro_id,
            call_site,
            call_extent,
            arg_spans,
            kind,
        });
        if let Some(m) = macro_id {
            self.by_macro.entry(m).or_default().push(ctx);
        }
        ctx
    }

    /// Where the token's text literally appears — possibly inside a macro definition.
    pub fn spelling_loc(&self, sp: Span) -> Option<Loc> {
        self.lookup_loc(sp.lo)
    }

    /// Walk ctx → parent → … → ROOT and resolve the outermost call site.
    ///
    /// **This is what gcov sees** (030 §1, measured). Coverage correlation uses this
    /// and nothing else. Must not allocate (010 contract 9).
    pub fn expansion_loc(&self, sp: Span) -> Option<Loc> {
        let mut pos = sp.lo;
        let mut ctx = sp.ctx;
        // Bounded by the number of expansions: a malformed cycle terminates with a
        // wrong answer rather than hanging (contract: cyclic_parent_chain_terminates).
        for _ in 0..=self.expansions.len() {
            if ctx.is_root() {
                return self.lookup_loc(pos);
            }
            let e = self.expansion(ctx)?;
            pos = e.call_site.lo;
            ctx = e.call_site.ctx;
        }
        self.lookup_loc(pos)
    }

    /// Full chain, outermost-first.
    pub fn expansion_backtrace(&self, sp: Span) -> Vec<ExpnFrame> {
        let mut frames = Vec::new();
        let mut ctx = sp.ctx;
        for _ in 0..=self.expansions.len() {
            if ctx.is_root() {
                break;
            }
            let Some(e) = self.expansion(ctx) else { break };
            frames.push(ExpnFrame {
                ctx,
                macro_id: e.macro_id,
                call_site: e.call_site,
            });
            ctx = e.parent;
        }
        frames.reverse(); // innermost-first while walking; callers want outermost-first
        frames
    }

    /// Did this span come from expanding `m`, at any nesting depth?
    pub fn involves_macro(&self, sp: Span, m: MacroId) -> bool {
        let mut ctx = sp.ctx;
        for _ in 0..=self.expansions.len() {
            if ctx.is_root() {
                return false;
            }
            let Some(e) = self.expansion(ctx) else {
                return false;
            };
            if e.macro_id == Some(m) {
                return true;
            }
            ctx = e.parent;
        }
        false
    }

    /// Every expansion of `m`, including those reached because a macro whose body
    /// expands `m` was itself expanded. The core of change-impact analysis (031 §3.2).
    ///
    /// Direct sites come from the reverse index; they are already transitive in the
    /// sense that matters, because an expansion of `m` nested inside another macro is
    /// still recorded against `m`.
    pub fn expansion_sites(&self, m: MacroId) -> impl Iterator<Item = ExpnCtx> + '_ {
        self.by_macro.get(&m).into_iter().flatten().copied()
    }

    pub fn origin(&self, sp: Span) -> TokenOrigin {
        if sp.is_dummy() {
            return TokenOrigin::Synthesized;
        }
        let Some(e) = self.expansion(sp.ctx) else {
            return match self.lookup_file(sp.lo) {
                Some(f) => TokenOrigin::Verbatim(f),
                None => TokenOrigin::Synthesized,
            };
        };
        // An argument token's bytes lie inside one of the recorded argument spans at
        // the call site; a body token's lie inside the macro's replacement list.
        for (i, arg) in e.arg_spans.iter().enumerate() {
            if arg.contains(sp.lo) {
                return TokenOrigin::MacroArg {
                    expn: sp.ctx,
                    arg_index: i,
                };
            }
        }
        match e.macro_id {
            Some(m) => TokenOrigin::MacroBody(m),
            None => TokenOrigin::Synthesized,
        }
    }

    #[doc(hidden)]
    pub fn force_parent_for_test(&mut self, child: ExpnCtx, parent: ExpnCtx) {
        if let Some(e) = self.expansions.get_mut(child.0 as usize - 1) {
            e.parent = parent;
            e.call_site.ctx = parent;
        }
    }
}

/// Cross-TU identity for a macro: `(defining file, name, definition line)`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacroEntity(pub u32);

/// A globally interned file, valid after the owning `SourceMap` is dropped.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalFileId(pub u32);

/// Cross-TU interners, owned by the driver (010 §6.2).
#[derive(Debug, Default)]
pub struct GlobalInterner {
    files: indexmap::IndexMap<PathBuf, GlobalFileId>,
    paths: Vec<PathBuf>,
    macros: indexmap::IndexMap<(GlobalFileId, Arc<str>, u32), MacroEntity>,
}

/// One resolved expansion site. Self-contained: no `ExpnCtx`, no per-TU ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedSite {
    pub file: GlobalFileId,
    pub line: u32,
    pub depth: u32,
}

/// The retained whole-tree artifact (010 §6.2).
#[derive(Debug, Default)]
pub struct CookedExpansionIndex {
    sites: indexmap::IndexMap<MacroEntity, Vec<CookedSite>>,
}

impl GlobalInterner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern_file(&mut self, _path: &Path) -> GlobalFileId {
        todo!("green")
    }

    pub fn path(&self, _id: GlobalFileId) -> &Path {
        todo!("green")
    }

    pub fn lookup_macro(&self, _file: &str, _name: &str) -> Option<MacroEntity> {
        todo!("green")
    }

    pub fn macro_count(&self) -> usize {
        self.macros.len()
    }
}

impl CookedExpansionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve every expansion site in `sm` to `(global file, line)` **before** the
    /// per-TU tables are dropped, and merge them into the whole-tree index.
    pub fn cook_tu(&mut self, _interner: &mut GlobalInterner, _sm: &SourceMap) {
        todo!("green")
    }

    pub fn sites(&self, _m: MacroEntity) -> impl Iterator<Item = &CookedSite> + '_ {
        std::iter::empty()
    }
}
