# 010 — Source model and macro provenance

> This is the most important document in the set. Every differentiating capability
> chiero has over a coverage-only or clang-AST-only tool traces back to the model
> defined here.

## 1. The problem

VPP contains 754 distinct `foreach_*` X-macros and a data-plane idiom built almost
entirely out of macros. Consider a one-line change to the body of `vec_add1` in
`src/vppinfra/vec.h`, and ask: **which tests must re-run?**

- **gcov cannot answer.** gcc attributes generated code to the *expansion location* —
  the `.c` line where `vec_add1(v, x)` was written. `vec.h` gets essentially no
  coverage lines of its own. A tool that only intersects "changed lines" with "covered
  lines" returns the empty set and concludes nothing is affected. That is a
  false negative that ships bugs.
- **File-level dependency tracking over-answers.** "Anything that `#include`s `vec.h`"
  is most of VPP. That is a true answer and a useless one.
- **A clang AST gets closer but loses resolution.** Clang records a spelling/expansion
  location pair, but chiero needs the full nesting chain, per-argument attribution, and
  a *reverse* index from macro definition to every expansion site.

chiero answers precisely: *every function that expanded `vec_add1`, transitively through
nested macros*, and from there, via the coverage index, exactly the tests that executed
those expansions.

The model below exists to make that query cheap and exact. It is deliberately modeled on
rustc's `SyntaxContext`/`ExpnId` hygiene machinery, which is a proven shape for exactly
this problem.

## 2. Core types

```rust
/// Byte offset into the *global* concatenated source space owned by SourceMap.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BytePos(pub u32);

/// A byte range plus the expansion context it was produced in.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Span {
    pub lo:  BytePos,
    pub hi:  BytePos,
    pub ctx: ExpnCtx,
}

/// Index into SourceMap::expansions. ExpnCtx::ROOT (== 0) means "written literally
/// in a source file, not produced by any macro".
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ExpnCtx(pub u32);

impl ExpnCtx { pub const ROOT: ExpnCtx = ExpnCtx(0); }
```

`Span` is 12 bytes and `Copy`. It is stored on every token, every AST node, and every
CIR instruction, so its size is a real constraint — do not grow it.

### 2.1 Expansions

```rust
pub struct Expansion {
    /// Enclosing expansion, or ROOT. Gives the full nesting chain.
    pub parent: ExpnCtx,
    /// Which macro produced this. None for non-macro expansions (e.g. _Pragma).
    pub macro_id: Option<MacroId>,
    /// Where the invocation was written. NOTE: this Span may itself carry a
    /// non-ROOT ctx, when a macro is invoked from inside another macro's body.
    pub call_site: Span,
    /// The full extent of the invocation, `NAME(a, b)` inclusive of parens.
    pub call_extent: Span,
    /// Spans of the actual argument token sequences, pre-expansion.
    pub arg_spans: Vec<Span>,
    pub kind: ExpnKind,
}

pub enum ExpnKind {
    ObjectLike,
    FunctionLike,
    /// __FILE__, __LINE__, __COUNTER__, and friends.
    Builtin(BuiltinMacro),
    /// _Pragma("...") destringized into a pragma directive.
    Pragma,
    /// Token produced by ## or # rather than copied from the body.
    Paste,
    Stringize,
}
```

### 2.2 Where a token came from within an expansion

A token emerging from a function-like macro is either copied from the macro's *body* or
substituted from an *argument*. Distinguishing these matters: editing a macro body
affects every call site, but a token that came from an argument was written by the
caller and its provenance should lead back there.

This is encoded in the token's `Span`, not in `Expansion`:

- Token copied from the macro body → `Span { lo, hi }` points into the **macro
  definition's** text, `ctx` = the expansion.
- Token substituted from an argument → `Span { lo, hi }` points into the **argument's**
  text at the call site, `ctx` = the expansion.

So the discriminator is "does `lo..hi` fall inside the macro definition's body extent?"
`SourceMap` exposes this directly rather than making callers do range arithmetic:

```rust
pub enum TokenOrigin { MacroBody(MacroId), MacroArg { expn: ExpnCtx, arg_index: usize },
                       Verbatim(FileId), Synthesized }
```

## 3. SourceMap

```rust
pub struct SourceMap {
    files:      Vec<SourceFile>,           // indexed by FileId
    expansions: Vec<Expansion>,            // indexed by ExpnCtx; [0] is a ROOT sentinel
    macros:     Vec<MacroDef>,             // indexed by MacroId
    /// Reverse index: macro -> every expansion of it. THE test-selection primitive.
    by_macro:   IndexMap<MacroId, Vec<ExpnCtx>>,
}

pub struct SourceFile {
    pub id: FileId,
    pub path: PathBuf,
    pub src: Arc<str>,
    /// Global-space range this file occupies.
    pub start_pos: BytePos,
    /// Byte offset of the start of each line, for O(log n) offset->line.
    line_starts: Vec<BytePos>,
}
```

Files are laid out consecutively in a single global `BytePos` space so a `Span` needs no
`FileId` field. Lookup from `BytePos` to `FileId` is a binary search over `start_pos`.

### 3.1 Required queries

```rust
impl SourceMap {
    /// Where the token's text literally appears. May be inside a macro definition.
    fn spelling_loc(&self, sp: Span) -> Loc;

    /// Walk ctx -> parent -> ... -> ROOT and return the outermost call site.
    /// THIS IS WHAT GCOV SEES. Coverage correlation must use this and nothing else.
    fn expansion_loc(&self, sp: Span) -> Loc;

    /// Full chain, outermost-first: [vec_add1, _vec_resize, clib_mem_realloc].
    fn expansion_backtrace(&self, sp: Span) -> Vec<ExpnFrame>;

    /// Did this span come from expanding `m`, at any nesting depth?
    fn involves_macro(&self, sp: Span, m: MacroId) -> bool;

    /// Every expansion site of `m`, transitively including macros whose bodies
    /// expand `m`. The core of change-impact analysis (031).
    fn expansion_sites(&self, m: MacroId) -> impl Iterator<Item = ExpnCtx> + '_;

    fn origin(&self, sp: Span) -> TokenOrigin;
}

pub struct Loc { pub file: FileId, pub line: u32, pub col: u32, pub pos: BytePos }
```

`expansion_loc` is the single most-called query in the coverage vertical and must be
O(depth) with no allocation.

### 3.2 Worked example

```c
// vec.h:120
#define vec_add1(V, E) vec_add1_ha (V, E, 0, 0)
// vec.h:118
#define vec_add1_ha(V, E, H, A) (vec_resize_ha(V,1,H,A), (V)[_vec_len(V)-1] = (E))

// ip4_forward.c:900
vec_add1 (adj_list, ai);
```

For the token `vec_resize_ha` in the generated code:

| Query | Result |
|---|---|
| `spelling_loc` | `vec.h:118` — where the text is written |
| `expansion_loc` | `ip4_forward.c:900` — **what gcov reports** |
| `expansion_backtrace` | `[vec_add1 @ ip4_forward.c:900, vec_add1_ha @ vec.h:120]` |
| `origin` | `MacroBody(vec_add1_ha)` |

For the token `ai`: `spelling_loc` = `ip4_forward.c:900`, `origin` =
`MacroArg { expn: <vec_add1 expansion>, arg_index: 1 }`.

Now the payoff. Edit line 118 — the body of `vec_add1_ha`. `expansion_sites(vec_add1_ha)`
yields every direct expansion *and*, transitively, every expansion of `vec_add1` (whose
body invokes it). Map each through `expansion_loc` to a `.c` line, intersect with the
coverage index, and the affected test set falls out — despite gcov having never recorded
a single line in `vec.h`.

## 4. Synthesized spans

Some spans have no source text: compiler-generated initialization, lowering artifacts,
model-provided function bodies. These use `ExpnKind` `Builtin` with a zero-width range
and a descriptive `MacroDef`, so `expansion_backtrace` still renders something useful in
a diagnostic. A span must never be fabricated by pointing at unrelated real source.

## 5. Diagnostics

```rust
pub struct Diagnostic {
    pub severity: Severity,     // Bug | Error | Warning | Note | Help
    pub span: Span,
    pub message: String,
    pub notes: Vec<(Span, String)>,
}
```

Rendering a diagnostic whose span has non-ROOT `ctx` **must** print the macro-expansion
backtrace, in gcc's style:

```
error: possible null dereference
  --> src/vnet/ip/ip4_forward.c:900:3
   |
900|   vec_add1 (adj_list, ai);
   |   ^^^^^^^^
   |
note: in expansion of macro 'vec_add1'
  --> src/vppinfra/vec.h:120:24
note: in expansion of macro 'vec_add1_ha'
  --> src/vppinfra/vec.h:118:33
```

A report that points only at `vec.h:118` is unactionable — nobody can tell which of
several thousand call sites is the problem. The backtrace is what makes chiero's
findings usable on a macro-heavy codebase, so it is a hard requirement on every
user-facing report, including the JSON emitted by [050](050-tool-interface.md).

## 6. Memory budget

VPP is ~1M lines. A rough upper bound on expansions is ~10M for a full-tree analysis. At
~64 bytes per `Expansion` (with a `SmallVec<[Span; 4]>` for `arg_spans`) that is ~640 MB
— acceptable on the 251 GB target machine but not free.

Mitigations, in priority order:
1. `arg_spans` as `SmallVec<[Span; 4]>`; the overwhelming majority of macros have ≤4 args.
2. `Expansion` interning: identical `(macro_id, call_site, parent)` triples are shared.
3. Per-TU expansion tables, dropped after lowering, with only the `by_macro` reverse
   index retained across TUs — the reverse index is all the coverage vertical needs.

Mitigation 3 is the important one: analyses that span the whole tree keep the reverse
index, not the full tables.

## 7. Testable contracts

1. `size_of::<Span>() == 12` and `Span: Copy`.
2. `ExpnCtx::ROOT.0 == 0`, and `spelling_loc(sp) == expansion_loc(sp)` for any `sp` with
   `ctx == ROOT`.
3. For the §3.2 example: `spelling_loc` of the `vec_resize_ha` token resolves to
   `vec.h:118`, `expansion_loc` to `ip4_forward.c:900`.
4. `expansion_backtrace` for that token has length 2 and is ordered outermost-first.
5. `origin` of the `ai` token is `MacroArg { arg_index: 1 }`; `origin` of the
   `vec_resize_ha` token is `MacroBody(vec_add1_ha)`.
6. `involves_macro(sp, vec_add1_ha)` is true for the `vec_resize_ha` token even though
   the source only ever wrote `vec_add1`.
7. `expansion_sites(vec_add1_ha)` includes the `ip4_forward.c:900` site transitively.
8. A file whose content is `#define A B\n#define B C\nA\n` produces an expansion chain
   of depth 2 for the resulting `C` token.
9. `expansion_loc` never allocates (asserted with a counting allocator in tests).
10. Rendering a `Diagnostic` whose span has depth-2 `ctx` emits exactly two
    `in expansion of macro` notes, outermost-first.
11. Round trip: for every token in a preprocessed fixture, the byte range given by
    `spelling_loc` re-lexes to the same token text.
12. `BytePos` → `FileId` lookup agrees with a linear scan over `start_pos` for every
    file boundary in a 100-file fixture.
