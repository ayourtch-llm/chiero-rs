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
chiero answers precisely: *every function that expanded `vec_add1`, transitively through
nested macros*, and from there, via the coverage index, exactly the tests that executed
those expansions.

### 1.1 Why chiero owns this rather than using clang

An earlier draft of this section claimed clang could not supply the nesting chain,
per-argument attribution, or a reverse index. **That claim was tested and is false**, and
it is corrected here rather than quietly dropped, because a false rationale will not hold
the line when the frontend gets hard.

What clang actually does, verified with clang 18.1.3 on the `vec_add1`/`vec_add1_ha`
fixture in §3.2:

- **Full nesting chain: yes.** A diagnostic inside the nested expansion prints
  `expanded from macro 'vec_add1'` *and* `expanded from macro 'vec_add1_ha'`, with both
  `m.h` definition sites — the same two-frame backtrace §5 specifies as chiero's output.
  `SourceManager` retains the whole tree and `getImmediateMacroCallerLoc` walks it.
- **Per-token argument attribution: yes.** `SourceManager::isMacroArgExpansion` /
  `isMacroBodyExpansion` is exactly the `TokenOrigin` discriminator in §2.2.
- **Reverse index: substantially.** libclang with a detailed preprocessing record emits
  `MACRO_DEFINITION` and `MACRO_INSTANTIATION` cursors per TU. It does *not* enumerate
  *nested* expansions, but `PPCallbacks::MacroExpands` in the C++ API does.

So the honest reasons are these, and they are sufficient:

1. **The no-external-toolchain constraint.** chiero is specified as a modular, embeddable,
   pure-Rust library that builds and runs with `--no-default-features` and links nothing
   ([001 §5](001-architecture.md)). Depending on libclang for a *core* capability — not an
   oracle, not an optional accelerator — forfeits that property outright. This is the
   binding reason.
2. **Both sides of a diff, including non-compiling states.** [031](031-change-impact.md)
   compares two parsed programs. Requiring every analysed revision to be clang-parseable
   with a full, correct compilation database is a much stronger precondition than
   requiring it to be chiero-parseable.
3. **Provenance as a 12-byte `Copy` value.** `Span` is threaded through the AST, CIR, the
   memory model and every finding. Across an FFI boundary, provenance becomes a handle
   into a foreign object graph with a foreign lifetime.
4. **Ownership of lowering.** CIR's `Block::gcov_lines` and per-instruction spans are
   produced by our own lowering; clang would give us a provenance-rich AST and then a gap.

**Contingency, stated plainly** ([080](080-roadmap.md) names the frontend as the riskiest
milestone): a clang-subprocess provenance extractor is a *viable fallback for the
impact/selection vertical specifically*, since that vertical needs expansion sites and
line mappings but not symbolic execution. It would cost the pure-Rust property for that
vertical only. Recording this is deliberate — the alternative is a taboo resting on a
claim that does not survive a five-minute experiment.

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

## 6. Scale, identity across TUs, and the cooked index

### 6.1 Two problems that only appear at whole-tree scale

Everything in §2–§5 is per translation unit. `MacroId`, `FileId` and `ExpnCtx` are indices
into *one* `PreprocessedTu`'s `SourceMap` ([012 §5](012-preprocessor.md)). The headline
capability, though, is a whole-tree query — "every function in VPP that expands
`vec_add1`" — and two things break on the way there:

**Identity.** `vec.h` is one file with one `vec_add1`, but 1552 TUs each mint their own
`FileId` and `MacroId` for it. Meanwhile [030 §5](030-coverage-gcov.md) keys the coverage
index on a global `(FileId, line)` and [031 §1](031-change-impact.md) needs a single
`Entity::Macro`. Per-TU ids cannot serve either.

**Lifetime.** The obvious memory mitigation — drop per-TU expansion tables after lowering,
keep the reverse index — is *incorrect as stated in earlier drafts*, because
`by_macro: IndexMap<MacroId, Vec<ExpnCtx>>` stores `ExpnCtx` values that are indices
**into the table being dropped**. Every retained handle would dangle, and
`expansion_loc` — the one query [031 §3.2](031-change-impact.md) needs to get from an
expansion site to its enclosing function — would be uncomputable. The retained artifact
must be *resolved*, not a set of handles.

### 6.2 The cooked index

Resolution happens **before** the per-TU tables are dropped:

```rust
/// Cross-TU interners, owned by the driver, populated as each TU is preprocessed.
pub struct GlobalInterner {
    files:  IndexMap<PathBuf, GlobalFileId>,        // canonicalized real path
    macros: IndexMap<(GlobalFileId, Symbol, u32), MacroEntity>,   // file, name, def line
}

/// The retained whole-tree artifact. Self-contained: no ExpnCtx, no per-TU ids.
pub struct CookedExpansionIndex {
    /// macro -> every site it reached, transitively through nested macros.
    sites: IndexMap<MacroEntity, Vec<CookedSite>>,
}

pub struct CookedSite {
    pub file: GlobalFileId,      // the .c file gcov attributes to
    pub line: u32,               // expansion_loc, already resolved
    pub func: Option<FuncKey>,   // enclosing function (030 §5)
    pub depth: u32,              // 0 = written literally at the call site
    pub config: ConfigId,        // which build configuration produced it
}
```

A macro's identity is `(defining file, name, definition line)` — matching
[031 §1](031-change-impact.md)'s `Entity::Macro`, and keeping a redefinition of the same
name distinct. Files are interned by canonicalized path, so `vec.h` is one
`GlobalFileId` across all 1552 TUs.

Per-TU tables are dropped only after every site they contain has been cooked. The
invariant is worth stating as a rule: **no long-lived structure may hold an `ExpnCtx`,
`MacroId` or `FileId`.** Those are per-TU and short-lived by construction; anything
crossing a TU boundary uses the global ids above.

### 6.3 Budget

An earlier estimate of ~10M expansions tree-wide was optimistic by one to two orders of
magnitude: [013 §7](013-parser.md) notes a single preprocessed VPP TU runs to tens of
megabytes of tokens, and there are 1552 of them, which puts raw expansions at 10⁸–10⁹ and
the naive full-table footprint in the tens of gigabytes. That is precisely why the cooked
index is the *design* and not an optimization — it is bounded by expansion **sites**
(millions), not by expansion **events**.

Within a TU, while the tables are live:
1. `arg_spans` as `SmallVec<[Span; 4]>`; the overwhelming majority of macros have ≤4 args.
2. `Expansion` interning: identical `(macro_id, call_site, parent)` triples are shared.
3. TUs are processed in a streaming fashion and cooked one at a time, so peak memory is
   one TU's tables plus the cooked index — not the whole tree's tables.

Contract 13 pins the property that actually matters: the cooked index answers
`expansion_sites` correctly *after* the per-TU tables are gone.

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

### Cross-TU identity and the cooked index (§6)

13. **The retained index survives the tables it was built from**: after two TUs are
    preprocessed, cooked, and their `SourceMap`s dropped, `expansion_sites` for a macro
    defined in a shared header still returns both TUs' sites, with resolved
    `(file, line, func)`. This is the contract that catches the dangling-`ExpnCtx` design
    error, and it must be written as a test that actually drops the tables.
14. Two TUs including the same header yield **one** `GlobalFileId` for it and **one**
    `MacroEntity` per macro, even though their per-TU `FileId`/`MacroId` values differ.
15. Two macros with the same name defined at different lines (or in different files) are
    distinct `MacroEntity`s; a macro `#undef`ed and redefined at a new line likewise.
16. No type reachable from `CookedExpansionIndex` transitively contains an `ExpnCtx`,
    `MacroId` or `FileId` — checked mechanically, since this is the invariant that keeps
    §6.2 true as the code evolves.
17. Cooking is order-independent: preprocessing the TUs in reverse order yields a
    byte-identical `CookedExpansionIndex`.
18. Peak memory over a 100-TU fixture is bounded by (one TU's tables + the cooked index),
    not by the sum of all TUs' tables — asserted against a recorded high-water mark.
19. A macro expanded under two different `ConfigId`s produces sites carrying each config,
    and querying by config returns only the matching subset.
