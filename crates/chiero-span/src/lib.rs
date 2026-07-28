//! Spans, source maps and macro provenance. See `docs/specs/010-source-and-provenance.md`.
//!
//! This crate depends on no other `chiero-*` crate (001 §4 rule 5) and is depended on by
//! everything, so it must stay small and stable.

/// Byte offset into the global concatenated source space owned by the `SourceMap`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BytePos(pub u32);

/// An interned identifier or spelling. Which interner it indexes is the holder's
/// business; this crate only owns the *type*.
///
/// **It lives here because a second crate needs it.** `chiero-lex` defined it and
/// `chiero-ast` now needs the same one: an AST node holds an identifier, and
/// `chiero-ast` may not depend on `chiero-lex` — the 001 §2 graph hangs both off
/// `chiero-span` and the arrows between them are pp-token *data flow*, not dependencies.
/// Two structurally identical `Symbol(u32)` types would be worse than one: the compiler
/// would not stop `lex::Symbol(3)` from being read against the AST's interner, and the
/// symptom would be a wrong identifier name in a diagnostic, not a type error.
/// `chiero-lex` re-exports this, so its public API is unchanged.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(pub u32);

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
        let len = u32::try_from(src.len()).expect("source file exceeds 4 GiB");
        let id = FileId(self.files.len() as u32);
        let start_pos = self.files.last().map_or(BytePos(0), |f| f.end_pos());
        // The global space is u32. Wrapping here would silently turn every subsequent
        // span into garbage with no signal — `[profile.release]` sets only `debug = 1`,
        // so arithmetic wraps rather than panicking.
        start_pos
            .0
            .checked_add(len)
            .expect("global source space exceeds u32; split the analysis");
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

    /// Try-variant. `FileId` is per-TU (010 §6.2), so being handed one minted by a
    /// different `SourceMap` is the anticipated caller error rather than a bug.
    pub fn try_file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
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
    /// Defining file and line, recorded explicitly rather than recovered from
    /// `def_span.lo`. A `-D` macro or a builtin has `def_file: None` and a `DUMMY`
    /// span, and `DUMMY.lo` is `BytePos(0)` — which resolves to whichever file happens
    /// to occupy offset 0, silently attributing `__FILE__` to an unrelated source file.
    /// 010 §4 forbids exactly that.
    pub def_file: Option<FileId>,
    pub def_line: u32,
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
    /// Record a macro. Identity comes from `def_span`'s location when it has one.
    ///
    /// A `DUMMY` span means "no source location" — a `-D` macro or a builtin — and must
    /// **not** be resolved: `DUMMY.lo` is `BytePos(0)`, which lands in whichever file
    /// occupies offset 0 and fabricates a definition site. This is the default
    /// constructor every caller uses, so guarding only `add_macro_at` would leave the
    /// forbidden behaviour on the path everyone takes.
    pub fn add_macro(&mut self, name: &str, def_span: Span, body_extent: Span) -> MacroId {
        if def_span.is_dummy() {
            return self.add_macro_at(name, def_span, body_extent, None, 0);
        }
        let loc = self.lookup_loc(def_span.lo);
        self.add_macro_at(
            name,
            def_span,
            body_extent,
            loc.map(|l| l.file),
            loc.map_or(0, |l| l.line),
        )
    }

    /// Record a macro with an explicit identity. `def_file: None` means "not from a
    /// source file" — a `-D` on the command line, or a builtin.
    pub fn add_macro_at(
        &mut self,
        name: &str,
        def_span: Span,
        body_extent: Span,
        def_file: Option<FileId>,
        def_line: u32,
    ) -> MacroId {
        let id = MacroId(self.macros.len() as u32);
        self.macros.push(MacroInfo {
            name: Arc::from(name),
            def_span,
            def_file,
            def_line,
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
        // `expansion_loc` walks `call_site.ctx` while `expansion_backtrace` walks
        // `parent`. Both are legal per the types, and `cook_tu` uses one for the line
        // and the other for the depth — so a map where they disagree yields a site whose
        // line and depth describe different chains. They are one chain, by invariant.
        // Exempt synthesized call sites: a `##` paste or `_Pragma` nested in a macro
        // body has no written location, so its span is DUMMY at ROOT while its parent
        // is the enclosing expansion. `chiero-pp` will construct exactly that.
        debug_assert!(
            call_site.is_dummy() || call_site.ctx == parent,
            "an expansion's call site must live in its parent context: {:?} vs {:?}",
            call_site.ctx,
            parent
        );
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
        // 010 §2.2: the discriminator is whether the token's bytes fall inside the
        // macro's *replacement list*. An argument token's bytes lie at the call site
        // instead. Checking arguments first is not sufficient on its own — a token in
        // neither range (a `##` paste, a `#` stringize, anything synthesized) must not
        // be reported as a body token just because no argument matched.
        for (i, arg) in e.arg_spans.iter().enumerate() {
            // A zero-width argument (`M()`, or `M(a,,c)`) can never `contain` anything,
            // so match it by position.
            let hit = if arg.is_empty() {
                arg.lo == sp.lo && sp.is_empty()
            } else {
                arg.contains(sp.lo)
            };
            if hit {
                return TokenOrigin::MacroArg {
                    expn: sp.ctx,
                    arg_index: i,
                };
            }
        }
        match e.macro_id {
            Some(m) => match self.macro_info(m) {
                Some(info) if info.body_extent.contains(sp.lo) => TokenOrigin::MacroBody(m),
                // In neither the body nor an argument: pasted, stringized or invented.
                _ => TokenOrigin::Synthesized,
            },
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

/// Lexically normalize a path: drop `.`, resolve `..` against the preceding component,
/// and collapse duplicate separators. Does not touch the filesystem.
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    let mut prefix = PathBuf::new();
    let mut rooted = false;
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.is_empty() {
                    out.pop();
                } else if !rooted {
                    // Leading `..` in a *relative* path cannot be resolved lexically.
                    out.push("..".into());
                }
                // `/..` is `/`: discard.
            }
            std::path::Component::Normal(n) => out.push(n.to_os_string()),
            other => {
                rooted = true;
                prefix.push(other.as_os_str());
            }
        }
    }
    let mut result = prefix;
    for c in out {
        result.push(c);
    }
    result
}

/// A macro's cross-TU identity: defining file, name, definition line.
type MacroKey = (GlobalFileId, Arc<str>, u32);

/// Cross-TU interners, owned by the driver (010 §6.2).
#[derive(Debug, Default)]
pub struct GlobalInterner {
    canonicalized: bool,
    files: indexmap::IndexMap<PathBuf, GlobalFileId>,
    paths: Vec<PathBuf>,
    macros: indexmap::IndexMap<MacroKey, MacroEntity>,
}

/// One resolved expansion site. Self-contained: no `ExpnCtx`, no per-TU ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedSite {
    pub file: GlobalFileId,
    pub line: u32,
    pub depth: u32,
    /// Which build configuration this expansion happened under (010 contract 19).
    ///
    /// **A plain `u64`, not `chiero_pp::ConfigId`.** `chiero-span` is the foundation and
    /// depends on nothing (001 §4); the preprocessor's id is an opaque number here for the
    /// same reason `Module::config` is one in CIR.
    pub config: u64,
}

/// The retained whole-tree artifact (010 §6.2).
#[derive(Debug, Default)]
pub struct CookedExpansionIndex {
    sites: indexmap::IndexMap<MacroEntity, Vec<CookedSite>>,
    /// `(entity, file, line, config) -> index into that entity's site vector`.
    ///
    /// Without it, deduplication is a linear scan of a vector that keeps growing across
    /// all 1552 TUs — quadratic in total sites, measured at 305 ms for 32k sites and
    /// rising ~3.5x per doubling. 010 §6.3 budgets *millions* of sites, so the fix that
    /// made the index bounded by sites would have made building it unaffordable.
    /// **The config is part of the key.** VPP builds the same headers several ways, and
    /// one source line is a different expansion in each — `CLIB_DEBUG` changes what
    /// `ASSERT` becomes. Keyed without it, the second configuration's site is deduplicated
    /// away against the first's and the index answers "where is this used?" with a list
    /// true of no single build.
    by_site: indexmap::IndexMap<(MacroEntity, GlobalFileId, u32, u64), usize>,
    dropped: u32,
}

impl GlobalInterner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern by **normalized** path, so one header is one id across all 1552 TUs.
    ///
    /// Normalization is lexical (`.`, `..` and duplicate separators), not
    /// `fs::canonicalize`: interning must work for paths that no longer exist on disk —
    /// the index outlives the build — and must not do IO per include. Real include
    /// resolution produces `vec.h`, `./vec.h` and `a/../vec.h` for one file, and without
    /// this they would be three ids and contract 14 would be false in practice while
    /// passing a test that spells the path identically twice.
    pub fn intern_file(&mut self, path: &Path) -> GlobalFileId {
        let key = normalize_path(path);
        if let Some(&id) = self.files.get(&key) {
            return id;
        }
        let id = GlobalFileId(self.paths.len() as u32);
        self.paths.push(key.clone());
        self.files.insert(key, id);
        id
    }

    pub fn path(&self, id: GlobalFileId) -> &Path {
        &self.paths[id.0 as usize]
    }

    /// Try-variant. `GlobalFileId`s from a different interner are the anticipated
    /// caller error, so panicking on them is not a service.
    pub fn try_path(&self, id: GlobalFileId) -> Option<&Path> {
        self.paths.get(id.0 as usize).map(|p| p.as_path())
    }

    /// Renumber files into path order and return `old id -> new id`. Macro entities are
    /// renumbered to match, so every id in the index can be remapped mechanically.
    pub(crate) fn canonicalize(&mut self) -> (Vec<GlobalFileId>, Vec<MacroEntity>) {
        // A second canonicalization would compute an identity file remap and silently
        // leave any *other* index over this interner un-remapped, giving it wrong paths
        // with no error. Once is the contract.
        assert!(
            !self.canonicalized,
            "GlobalInterner::canonicalize called twice; finalize exactly one index per interner"
        );
        self.canonicalized = true;
        let mut order: Vec<usize> = (0..self.paths.len()).collect();
        order.sort_by(|&a, &b| self.paths[a].cmp(&self.paths[b]));

        let mut remap = vec![GlobalFileId(0); self.paths.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = GlobalFileId(new as u32);
        }

        let mut paths = vec![PathBuf::new(); self.paths.len()];
        for (old, p) in self.paths.iter().enumerate() {
            paths[remap[old].0 as usize] = p.clone();
        }
        self.paths = paths;
        self.files = self
            .files
            .iter()
            .map(|(p, id)| (p.clone(), remap[id.0 as usize]))
            .collect();
        self.files.sort_unstable_keys();

        // Macro keys carry a file id, so they move too — and the entity *values* are
        // renumbered into the sorted key order, so entity ids are order-independent.
        let mut keys: Vec<(MacroKey, MacroEntity)> = self
            .macros
            .iter()
            .map(|((f, n, l), e)| ((remap[f.0 as usize], n.clone(), *l), *e))
            .collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        let mut entity_remap = vec![MacroEntity(0); self.macros.len()];
        self.macros.clear();
        for (new, (k, old)) in keys.into_iter().enumerate() {
            entity_remap[old.0 as usize] = MacroEntity(new as u32);
            self.macros.insert(k, MacroEntity(new as u32));
        }
        (remap, entity_remap)
    }

    /// The synthetic file that owns command-line (`-D`) and builtin macros, so they get
    /// a real identity instead of being attributed to whatever occupies offset 0.
    pub fn builtin_file(&mut self) -> GlobalFileId {
        self.intern_file(Path::new("<builtin>"))
    }

    /// Identity is `(defining file, name, definition line)` — matching
    /// `Entity::Macro` in [031 §1], and keeping a redefinition of the same name at a
    /// different line distinct.
    fn intern_macro(&mut self, file: GlobalFileId, name: &str, line: u32) -> MacroEntity {
        let key = (file, Arc::<str>::from(name), line);
        if let Some(&e) = self.macros.get(&key) {
            return e;
        }
        let e = MacroEntity(self.macros.len() as u32);
        self.macros.insert(key, e);
        e
    }

    /// The first entity for `(file, name)`. When a macro is `#undef`ed and redefined
    /// there is more than one — use [`Self::lookup_macros`] rather than silently taking
    /// whichever came first.
    pub fn lookup_macro(&self, file: &str, name: &str) -> Option<MacroEntity> {
        self.lookup_macros(file, name).first().copied()
    }

    /// Every entity for `(file, name)`, in definition-line order.
    pub fn lookup_macros(&self, file: &str, name: &str) -> Vec<MacroEntity> {
        let Some(fid) = self.files.get(&normalize_path(Path::new(file))).copied() else {
            return Vec::new();
        };
        let mut v: Vec<(u32, MacroEntity)> = self
            .macros
            .iter()
            .filter(|((f, n, _), _)| *f == fid && &**n == name)
            .map(|((_, _, l), &e)| (*l, e))
            .collect();
        v.sort_unstable();
        v.into_iter().map(|(_, e)| e).collect()
    }

    pub fn macro_count(&self) -> usize {
        self.macros.len()
    }
}

impl CookedExpansionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve every expansion in `sm` to `(global file, line)` **before** the per-TU
    /// tables are dropped, and merge into the whole-tree index.
    ///
    /// This eager resolution is the whole point of §6.2: `ExpnCtx` and `MacroId` are
    /// indices into `sm`, so retaining them and resolving later would dangle.
    pub fn cook_tu(&mut self, interner: &mut GlobalInterner, sm: &SourceMap) {
        self.cook_tu_with_config(interner, sm, 0);
    }

    /// As [`Self::cook_tu`], recording which build configuration produced these
    /// expansions (010 contract 19).
    pub fn cook_tu_with_config(
        &mut self,
        interner: &mut GlobalInterner,
        sm: &SourceMap,
        config: u64,
    ) {
        // Intern every macro, so a defined-but-unused macro still gets an identity.
        let mut entity_of: Vec<MacroEntity> = Vec::new();
        for m in &sm.macros {
            // Identity comes from the *recorded* def_file/def_line, never from
            // resolving def_span.lo — a builtin's DUMMY span would otherwise resolve to
            // whichever file occupies offset 0 (010 §4 forbids fabricating a location).
            let gf = match m.def_file {
                Some(f) => interner.intern_file(sm.file(f).path()),
                None => interner.builtin_file(),
            };
            let e = interner.intern_macro(gf, &m.name, m.def_line);
            entity_of.push(e);
            self.sites.entry(e).or_default();
        }
        debug_assert_eq!(entity_of.len(), sm.macros.len());

        for (i, expn) in sm.expansions.iter().enumerate() {
            let ctx = ExpnCtx(i as u32 + 1);
            let Some(mid) = expn.macro_id else { continue };
            let Some(entity) = entity_of.get(mid.0 as usize).copied() else {
                // A macro_id with no entry means the map is malformed. Count it rather
                // than dropping the expansion silently: a vanished site is a false
                // negative in change impact, which is where a false negative ships bugs.
                self.dropped += 1;
                continue;
            };
            // A synthesized call site (`##` paste, `#` stringize, `_Pragma`, a builtin)
            // has no written location. Resolving `DUMMY.lo` would land at offset 0 and
            // report the expansion against whichever file happens to be there — the same
            // fabrication the definition side guards against, one step later.
            if expn.call_site.is_dummy() {
                self.dropped += 1;
                continue;
            }
            // `expansion_loc` of the call site is the line gcov attributes to — the
            // outermost `.c` line, even when this expansion is nested inside another.
            let probe = Span::new(expn.call_site.lo, expn.call_site.hi, ctx);
            let Some(loc) = sm.expansion_loc(probe) else {
                // Unresolvable location: count it. A vanished site is a false negative
                // in change impact, and `dropped() == 0` is a soundness claim.
                self.dropped += 1;
                continue;
            };
            let gf = interner.intern_file(sm.file(loc.file).path());
            let depth = sm.expansion_backtrace(probe).len().saturating_sub(1) as u32;
            let v = self.sites.entry(entity).or_default();
            // **Sites, not events.** 010 §6.3's whole justification for the cooked index
            // is that it is bounded by expansion *sites* (millions) rather than
            // expansion *events* (10^8–10^9). `M + M + M` on one line is three events
            // and one site; pushing unconditionally reproduces the tens-of-gigabytes
            // footprint the design exists to avoid.
            match v
                .iter_mut()
                .find(|s| s.file == gf && s.line == loc.line && s.config == config)
            {
                // A site reachable both directly and through nesting is depth 0.
                Some(existing) => existing.depth = existing.depth.min(depth),
                None => v.push(CookedSite {
                    file: gf,
                    line: loc.line,
                    depth,
                    config,
                }),
            }
        }

        // Deterministic regardless of the order TUs were cooked in (001 §5).
        for v in self.sites.values_mut() {
            v.sort_unstable_by_key(|s| (s.file, s.line, s.depth));
        }
        self.sites.sort_unstable_keys();
    }

    /// Renumber file and macro ids into a canonical order, so the index is
    /// **byte-identical** regardless of the order TUs were cooked in (010 contract 17,
    /// 001 §5).
    ///
    /// Sorting sites is not sufficient on its own: `GlobalFileId`s are assigned in
    /// first-seen order, so cooking the same TUs in reverse yields the same *content*
    /// under a different numbering — which is still a different index, and would
    /// surface downstream as nondeterministic JSON.
    ///
    /// Call once after the last `cook_tu`.
    pub fn finalize(&mut self, interner: &mut GlobalInterner) {
        let (file_remap, entity_remap) = interner.canonicalize();
        let mut rebuilt: indexmap::IndexMap<MacroEntity, Vec<CookedSite>> =
            indexmap::IndexMap::with_capacity(self.sites.len());
        for (e, mut v) in std::mem::take(&mut self.sites) {
            for s in v.iter_mut() {
                s.file = file_remap[s.file.0 as usize];
            }
            v.sort_unstable_by_key(|s| (s.file, s.line, s.depth));
            // Entities are renumbered too. Renumbering only *files* left the site map
            // keyed on first-seen entity order, so cooking two TUs that define different
            // macros in the opposite order produced a different index — and the test
            // could not see it, because its TUs all shared one macro from one header.
            rebuilt.insert(entity_remap[e.0 as usize], v);
        }
        rebuilt.sort_unstable_keys();
        self.sites = rebuilt;
        // The side index's keys are stale after renumbering; rebuild it.
        self.by_site.clear();
        for (e, v) in &self.sites {
            for (i, s) in v.iter().enumerate() {
                self.by_site.insert((*e, s.file, s.line, s.config), i);
            }
        }
    }

    /// Expansions that could not be attributed to a macro entity. Non-zero means the
    /// index is incomplete and change impact must not be reported as complete.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    pub fn sites(&self, m: MacroEntity) -> impl Iterator<Item = &CookedSite> + '_ {
        self.sites.get(&m).into_iter().flatten()
    }

    /// The sites of `m` under one build configuration (010 contract 19).
    pub fn sites_for_config(
        &self,
        m: MacroEntity,
        config: u64,
    ) -> impl Iterator<Item = &CookedSite> + '_ {
        self.sites(m).filter(move |s| s.config == config)
    }
}
