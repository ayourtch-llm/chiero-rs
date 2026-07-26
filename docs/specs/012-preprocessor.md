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
- GNU `, ## __VA_ARGS__` comma-swallowing is supported (§4).

Pasted and stringized tokens have no contiguous source text, so their `lo..hi` is
zero-width at the operator's position and their real text lives in the side table
described in [011 §2.2](011-lexer.md).

### 2.4 Blue paint

A macro currently being expanded is disabled for the duration. chiero tracks this with a
`hide_set` on each token rather than a global stack, because tokens outlive the
expansion that produced them once they are buffered. `hide_set` is a small interned
bitset keyed by `MacroId`.

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
17. Preprocessing every TU in VPP's `compile_commands.json` produces zero panics, and
    the count of TUs producing at least one diagnostic is tracked as a regression metric.
18. Every result record carries a non-default `ConfigId`.
19. Two runs over the same TU produce byte-identical token streams and identical
    `ExpnCtx` numbering.
