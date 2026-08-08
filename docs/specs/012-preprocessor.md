# 012 — Preprocessor

Translation phase 4. This crate exists so chiero owns macro provenance
([010](010-source-and-provenance.md)); correctness of expansion *and* correctness of the
recorded expansion tree are equally load-bearing.

## 1. Macro definitions

```rust
pub struct MacroDef {
    pub id: MacroId,
    pub name: Symbol,
    pub kind: MacroKind,
    /// Body as pp-tokens, spans pointing into the defining file.
    pub body: Vec<PpToken>,
    pub def_span: Span,
    /// Set if #undef'd or redefined later; a MacroId is never reused.
    pub undef_span: Option<Span>,
}

pub enum MacroKind {
    ObjectLike,
    FunctionLike { params: Vec<Symbol>, variadic: Variadic },
}

pub enum Variadic {
    No,
    /// C99 `...` — the body refers to __VA_ARGS__.
    Std,
    /// GNU `args...` — named variadic parameter.
    Named(Symbol),
}
```

A `MacroId` identifies one *definition*, never a name. `#define X 1` / `#undef X` /
`#define X 2` yields two distinct `MacroId`s. This is required by change-impact
analysis: "the body of this macro changed" is a statement about a definition.

## 2. Expansion algorithm

Standard C11 §6.10.3 algorithm, with the blue-paint rule for recursion. The
implementation is the well-understood one (Prosser's algorithm as clarified by the
committee), so this section fixes only the points where chiero's provenance requirement
changes the shape.

### 2.1 Recording an expansion

Every expansion allocates exactly one `Expansion` record before any token is produced:

```rust
let expn = source_map.push_expansion(Expansion {
    parent:      current_ctx,        // NOT ROOT when expanding inside a macro body
    macro_id:    Some(def.id),
    call_site:   name_token.span,
    call_extent: span_from_to(name_token.span, close_paren.span),
    arg_spans:   args.iter().map(|a| a.extent()).collect(),
    kind:        ExpnKind::FunctionLike,
});
```

Then each produced token gets `span.ctx = expn`, with `lo..hi` set per
[010 §2.2](010-source-and-provenance.md):
- token copied from `def.body` → keeps the body token's `lo..hi`;
- token substituted from argument *n* → keeps the **argument token's** `lo..hi`, which
  points at the call site.

This is the whole trick, and it is why `origin()` can distinguish body from argument
without extra storage.

### 2.2 Argument pre-expansion

Arguments are macro-expanded *before* substitution, except where the parameter is an
operand of `#` or `##`. Pre-expansion happens in the caller's context, so tokens
produced during pre-expansion get their own nested `ExpnCtx` whose `parent` is the
caller's context — **not** the context of the macro being called. Getting this
relationship backwards inverts the backtrace, which is contract 7.

### 2.3 Stringization and pasting

- `#param` produces one `StringLit` with `ExpnKind::Stringize`. Spelling text is
  synthesized (spacing normalized per §6.10.3.2), so the token carries a synthesized
  span whose `call_site` is the parameter's use in the body.
- `a ## b` produces a token lexed from the concatenation, with `ExpnKind::Paste`. If the
  result is not a single valid pp-token, that is UB; chiero emits a diagnostic and keeps
  both tokens (gcc's behavior).
- **The paste operator is identified in the replacement list, at definition time, and
  nowhere else.** C11 6.10.3.3 makes `##` an operator by where it is *spelled*. A `##`
  that reaches a substituted token sequence any other way is an ordinary punctuator:
  spelled at a call site and passed as an argument (`#define FOO(x) A x B` / `FOO(##)` →
  `A ## B`), or produced by an earlier paste — `# ## #` yields a `##` token, which is the
  whole subject of 6.10.3.3p4's worked example:

  ```c
  #define hash_hash # ## #
  #define mkstr(a) # a
  #define in_between(a) mkstr(a)
  #define join(c, d) in_between(c hash_hash d)
  char p[] = join(x, y);            /* "x ## y" */
  ```

  Implementation consequence, and it is not optional: **the "is an operator" bit must be
  carried on the token**, set when a macro body is stored and preserved through
  substitution. Substitution interleaves replacement-list tokens with argument tokens, so
  by the time the paste pass walks the result, where each token came from is precisely
  what it can no longer recover. A paste's own result is minted *without* the bit.
- GNU `, ## __VA_ARGS__` comma-swallowing is supported (§4). The comma is deleted only
  when the variadic argument is **empty**; with a non-empty argument it stays a separate
  token, and fusing it into the argument produces a token that is not a pp-token at all.
- **`__VA_OPT__` is implemented** (C23 6.10.3.1), as of 2026-08-07. It was out of v1 scope by
  measurement — VPP uses `__VA_ARGS__` 230 times across `src/**.h` and `__VA_OPT__` **zero** —
  and was *diagnosed* rather than passed through as four literal tokens, on the rule that
  refusing beats pretending. The owner asked for the preprocessor conformance gap closed, so the
  scope changed; the "never pass through as itself" property did not.

  ⚠️ **Its condition is the variadic argument's *tokens*, and this is the subtler emptiness rule
  the old note warned about.** `__VA_OPT__(c)` yields `c` when the variadic argument is present
  **and non-empty**; `P(1,)` supplies an empty one and yields nothing. That is the **opposite**
  test from GNU comma-swallowing directly above, which turns on whether an argument was supplied
  at all — `debug(Y, )` keeps its comma precisely because one *was*. Two neighbouring rules with
  opposite conditions is how a shared flag ends up wrong, and both are pinned by tests.

  The group is resolved into an *effective replacement list* before substitution, so its contents
  go through the ordinary parameter walk with no second code path. Parenthesis depth is counted,
  so `__VA_OPT__(f(a))` keeps its inner pair; an unterminated group is diagnosed and the rest of
  the body survives (011 §4).

Pasted and stringized tokens have no contiguous source text, so their `lo..hi` is
zero-width at the operator's position and their real text lives in the side table
described in [011 §2.2](011-lexer.md).

### 2.4 Blue paint

A macro currently being expanded is disabled for the duration. chiero tracks this with a
`hide_set` on each token rather than a global stack, because tokens outlive the
expansion that produced them once they are buffered. `hide_set` is a small interned
bitset keyed by `MacroId`.

**The combination rule is the part that matters, and it is an intersection.** C99
6.10.3.4p2 as realized by Prosser's algorithm — the standard formulation every C
preprocessor implements:

| invocation | hide set of the resulting tokens |
|---|---|
| object-like `M` | `HS(M) ∪ {M}` |
| function-like `M ( … )` | `(HS(M) ∩ HS(the closing paren)) ∪ {M}` |

The intersection is not a refinement of a union, it is the opposite of one, and the
difference is observable. When `M`'s name comes out of an earlier expansion but its
argument list is taken from the source tokens that followed, the invocation is only
*partly* inside that earlier expansion — so the outer macro's paint drops off, and
tokens the union would have left inert go on expanding:

```c
#define f(a) a*g
#define g(a) f(a)
b: f(2)(9)                  /* b: 2*9*g — not 2*f(9) */
```

`f(2)` yields `2*g` whose `g` is painted by `f`; but `g(9)` takes its `)` from the
source, whose hide set is empty, so `f` leaves the set and the `f(9)` that `g` produces
expands. Taking the union instead stalls two expansions early.

⚠️ **The failure mode of getting this wrong in the other direction is non-termination**,
so a change here is only safe beside the cases that must not move: 6.10.3.4p2's own
`f(f(z))`, the `A B C` triangle, direct self-reference, mutual recursion through two
object-like names, and — the discriminating one — the same `f(2)(9)` where `g` is
*object-like*, where there is no closing paren to intersect at and `f` must stay painted.

## 3. Directives

| Directive | Notes |
|---|---|
| `#define` / `#undef` | §1 |
| `#include` / `#include_next` | §3.1 |
| `#if` / `#ifdef` / `#ifndef` / `#elif` / `#else` / `#endif` | §3.2 |
| `#line` | Adjusts reported `Loc` only; `BytePos` is unaffected |
| `#error` / `#warning` | Diagnostic; `#error` does not abort the TU |
| `#pragma` | Recorded, mostly passed through; `#pragma once` honored |
| `_Pragma("...")` | Destringized and reprocessed, `ExpnKind::Pragma` |
| `#elifdef` / `#elifndef` | C23; accepted, diagnosed under `-std=c11` |

Unknown directives produce a diagnostic and are skipped, never a hard error.

### 3.1 Include resolution

Search order follows gcc: quoted includes search the including file's directory first,
then `-iquote`, then `-I`, then system directories. `#include_next` continues from the
entry after the one that provided the current file — VPP does not use it heavily but
glibc headers do.

**`#pragma once` and include guards.** Both are honored. The classic include-guard
optimization (detect the `#ifndef X / #define X / … / #endif` wrapper and skip re-reading
the file entirely) is **required**, not optional: VPP headers are included on the order
of 10⁵ times across the tree, and without it whole-tree preprocessing is not viable.

**Computed includes** (`#include MACRO`) are supported: the line is macro-expanded, then
re-parsed as a header-name.

### 3.2 Conditional evaluation

`#if` expressions are evaluated over `intmax_t`/`uintmax_t` with C's usual arithmetic
conversions. Points that are easy to get wrong and are therefore pinned as contracts:

- `defined X` and `defined(X)` are evaluated **before** macro expansion of the rest of
  the line, but a `defined` produced *by* macro expansion is UB (gcc evaluates it;
  chiero matches gcc and diagnoses).
- Identifiers surviving expansion evaluate to `0` — including `true`/`false` under C11.
- Division or modulo by zero in a *live* branch is a diagnostic; in a dead sub-expression
  of `&&`/`||` it must not be evaluated at all.
- Character constants have implementation-defined signedness; chiero follows the target
  config ([014](014-semantics-and-types.md)).

**Inactive branches are lexed but not analyzed.** Tokens in a false branch are skipped
with only enough scanning to track nesting. No diagnostics are emitted from inactive
branches (see [011 §4](011-lexer.md)).

### 3.3 Configuration-space caveat

Preprocessing resolves `#if` against **one** macro configuration. Analysis results are
therefore valid for one build configuration only. VPP's multiarch builds compile the
same source under different `CLIB_MARCH_VARIANT` values, producing genuinely different
programs from one file ([060](060-vpp-integration.md)).

chiero does **not** attempt configuration-independent analysis (à la TypeChef). Instead
every result records the `ConfigId` it was derived under, and the multiarch handling in
`chiero-vpp` runs the analysis once per variant. Presenting a result without its
`ConfigId` is a defect.

## 4. GNU extensions required

VPP does not build without these:

| Extension | Example |
|---|---|
| Named variadic macros | `#define D(fmt, args...) f(fmt, ##args)` |
| Comma swallowing | `, ## __VA_ARGS__` deletes the comma when varargs is empty |
| `__COUNTER__` | Used by VPP for unique symbol generation |
| `__has_include` / `__has_attribute` / `__has_builtin` | Feature detection in headers |
| `#include_next` | glibc |
| `#warning` | |
| Empty macro arguments | Legal C99, but historically shaky; VPP relies on it |

`__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `__STDC__` etc. are `ExpnKind::Builtin`.
`__DATE__`/`__TIME__` are pinned to fixed values from the analysis config — a
nondeterministic preprocessor would break every golden test
([001 §5](001-architecture.md), determinism).

### 4.1 The compiler persona, and what the feature queries answer for

**chiero's predefine set is an impersonation of the build compiler, not a self-report.** The
baked set claims **gcc 13.3 on x86-64**, and `chiero-cli` replaces it wholesale with a real
`cc -dM` capture when one is available. Everything follows from that:

- `__has_attribute(x)` and `__has_builtin(x)` answer **what the impersonated compiler
  recognizes**, never what chiero models. chiero acts on four attributes and models a handful of
  builtins; answering from *that* would make every system header configure for a compiler which
  has never existed, and chiero would then analyse a program nobody compiles. By the same
  argument `__GNUC__` would have to be undefined, which nobody proposes.
- The answers live in a **table measured from the impersonated compiler**, because there is no
  rule to derive them from: `packed` is supported and `minsize` is not, `__builtin_bswap128` is
  and `__builtin_bit_cast` is not. A test re-asks the real compiler for every row, since a table
  of remembered answers nobody re-checks drifts silently into deciding which branch every header
  takes.
- A name the table **does not cover** answers `0` — `#if` must yield a number and there is no
  third value — and the guess is recorded as a diagnostic naming the name. That is the same
  in-band/out-of-band split as `Selection::NeedsAst` and `Tier1Report::unreadable`: the number
  cannot carry "I do not know", so something beside it must. One diagnostic per distinct name per
  TU; `sys/cdefs.h` alone queries many times.
- **The queries are evaluated in `#if` expressions and in program text**, because both gcc and
  clang do: `int y = __has_attribute(packed);` compiles to `int y = 1;`. They are evaluated
  *after* macro expansion, because that is where they arrive — `__glibc_has_attribute(attr)`
  expands *to* `__has_attribute (attr)`. `defined` is the opposite case and is rewritten before
  expansion, as C requires.
- The names are **defined but never expanded**. `#ifdef __has_attribute` must be true as it is
  under gcc, and expanding them would consume the query before it could be answered.

⚠️ **Removing them from the predefine set is not a conservative option.** With `__has_attribute`
undefined, `#if defined __has_attribute && __has_attribute (packed)` is a hard error under gcc
itself — `#if` parses the whole expression whatever short-circuiting would do — which is the
idiom `sys/cdefs.h`'s own comment exists to warn about.

**Every version macro the persona implies must be present.** `__GNUC__` without
`__GNUC_MINOR__` makes glibc's `__GNUC_PREREQ` constant `0`, collapsing every version shield in
every header — a whole-tree configuration change from one absent predefine.

## 5. Output

```rust
pub struct PreprocessedTu {
    pub tokens: Vec<PpToken>,       // spans carry full ExpnCtx
    pub source_map: SourceMap,      // owns files, expansions, macro defs
    pub diagnostics: Vec<Diagnostic>,
    pub config: ConfigId,
    /// Every file opened, for build-dependency and change-impact use.
    pub deps: Vec<FileId>,
}
```

## 6. Performance

Whole-tree preprocessing of VPP must complete in **under 10 minutes** on 12 cores. TUs
are independent and preprocessed in parallel; the lexed-header cache
([011 §5](011-lexer.md)) and the include-guard optimization are what make this reachable.

Expansion records dominate memory. Apply the [010 §6](010-source-and-provenance.md)
mitigations: per-TU expansion tables dropped after lowering, retaining only the
`by_macro` reverse index.

## 7. Testable contracts

1. `#define A B` / `#define B C` / `A` → one `Ident` token `C` with expansion depth 2.
2. Blue paint: `#define f(x) f(x)` / `f(1)` terminates and yields `f(1)`.
3. Mutual recursion `#define a b` / `#define b a` / `a` terminates.
4. `#define str(s) #s` / `#define xstr(s) str(s)` / `xstr(__LINE__)` yields the line
   number as a string, not `"__LINE__"` — pins argument pre-expansion.
5. `#define cat(a,b) a##b` / `cat(1,2)` yields one `Number` token `12`.
6. `#define f(x) x` / `f()` is legal and expands to nothing.
7. For a token produced by pre-expanding an argument, its `ExpnCtx`'s `parent` chain
   reaches the **caller's** context, not the callee's.
8. `origin()` of an argument-derived token is `MacroArg` with the correct `arg_index`;
   `origin()` of a body-derived token is `MacroBody` with the correct `MacroId`.
9. `#define D(f, a...) g(f, ##a)` / `D("x")` expands to `g("x")` with no stray comma.
10. `#if 0` blocks containing `@`, unterminated strings, and unbalanced braces produce
    zero diagnostics.
11. `#if 1/0` in a live branch diagnoses; `#if 0 && 1/0` does not.
12. `#if defined(UNDEFINED_THING)` evaluates false; `#if UNDEFINED_THING` evaluates
    false and diagnoses under a pedantic flag.
13. A header with a well-formed include guard, included twice, is read from disk once
    (asserted by an instrumented file loader).
14. `#pragma once` has the same effect as contract 13.
15. `__COUNTER__` yields distinct increasing values; `__DATE__`/`__TIME__` are constant
    across two runs of the same input.
16. `#define HDR "x.h"` / `#include HDR` resolves `x.h`.
17. ✅ **Met 2026-08-08** — `chiero-vpp/tests/preprocess_corpus.rs`. Preprocessing every TU
    in VPP's compile database produces zero panics, and the count of TUs producing at least
    one diagnostic is tracked as a regression metric.

    ⚠️ **The test that claimed this measured nothing, twice over**, from M2 until now: it was
    `#[ignore]`d *and* returned early on a `compile_commands.json` that has never existed,
    leaving one live assertion — that the file it had just established was absent contained
    the substring `"file"`. It also hardcoded a VPP path into the frontend crate, which
    [060 §1](060-vpp-integration.md) forbids.

    It is real now because [060 contract 1](060-vpp-integration.md)'s ingest can say what
    `-D`/`-I` each TU actually compiles under. Preprocessing VPP under the wrong flags
    preprocesses a different program, so this contract was not writeable before it.
18. Every result record carries a non-default `ConfigId`.
19. Two runs over the same TU produce byte-identical token streams and identical
    `ExpnCtx` numbering.
20. **A `##` that arrives by substitution is not the paste operator** (§2.3). C11
    6.10.3.3p4's worked example — `hash_hash`/`mkstr`/`in_between`/`join` — yields
    `char p[] = "x ## y";`. `#define FOO(x) A x B` / `FOO(##)` yields `A ## B`, three
    tokens. `#define hh # ## #` / `#define m(a) [a]` / `m(x hh y)` yields `[x ## y]`.
    In every case a `##` spelled in a replacement list still pastes.
21. **The invocation hide set intersects at the closing paren** (§2.4).
    `#define f(a) a*g` / `#define g(a) f(a)` / `f(2)(9)` yields `2*9*g`; with `g`
    object-like (`#define g f`) the same input yields `2*f(9)`. Contracts 2 and 3 are
    the non-termination guard on this one and must be re-asserted beside it.
22. **Conformance against two independent compilers over a preprocessor test corpus.**
    `cargo run -p xtask -- pp-gate` runs a simplecpp checkout's `testsuite/` through gcc,
    clang and chiero and reports, per case, whether chiero matched either. gcc and clang
    are the oracle; the corpus supplies inputs only. A case both compilers *reject* is
    reported as a distinct outcome according to whether chiero rejected it too — a
    missing diagnostic is a finding, not an unmeasured row.
23. **The feature queries answer as the persona, and say when they are guessing** (§4.1).
    `#if __has_attribute(packed)` and `#if __has_builtin(__builtin_expect)` are 1;
    `__has_attribute(minsize)` and `__has_builtin(__builtin_debugtrap)` are 0 **with no
    diagnostic**, because the table covers them and gcc agrees; a name the table does not cover
    is 0 **with** a diagnostic naming it, once per distinct name. `#ifdef __has_attribute` is
    true. The guarded idiom `#if defined __has_attribute && __has_attribute (packed)` evaluates,
    as does the same query reached through a wrapper macro, and the same query in program text.
    Every table row is re-asked of the real compiler.
24. **`__GNUC_PREREQ(4,9)` is true under the baked persona** (§4.1). A `__GNUC__` with no
    `__GNUC_MINOR__` makes it constant 0 and silently reconfigures every glibc header.
