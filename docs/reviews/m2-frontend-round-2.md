# Merge-gate review, round 2 — `chiero-lex` / `chiero-pp`

**Verdict: do not merge.** Round 1's findings are, with two exceptions, genuinely and
well fixed. With gcc's real predefine set, `clib.h` now preprocesses **token-for-token
identical to gcc across 257,310 tokens**, and the 010 provenance model — the hardest part
to retrofit — is correct end to end, including through headers and through `#`/`##`.
Findings 11–14 below are surface defects on a sound engine, not architectural ones.

Method: differential against **gcc 13.3** and **clang 18.1.3** (`-E -P -std=gnu11`), gcc
output re-lexed with chiero's own lexer so the comparison is token-for-token. 25 torture
cases, a 57-expression `#if` battery, 124 real VPP headers, 5 real VPP `.c` files, and a
47-mutation campaign (38 killed, 9 survived).

> **The methodological point that matters most.** The round-2 evidence of "zero
> diagnostics" uses chiero's 5-macro predefine stub, which makes real headers take
> *different branches* from gcc — so agreement was never tested on the same code.
> Re-running with gcc's actual 401 predefines (`gcc -dM -E`) fed through `Config::defines`
> is what exposed findings 1, 2 and 3. Please adopt that as the standing harness.

---

## Verified fixed (spot-checked, not taken on trust)

Multi-line invocations, rescanning into following tokens, object-like `##`, `,##args`,
placemarkers; `#if` evaluation (56/57 match both compilers, including `0b1010`, `'ab'`,
`-1 < 0u`, `(-1u)>>1`, short-circuit non-evaluation); `#elifdef`/`#elifndef`; `#include`
as a real directive with search order, `#include_next`, computed includes, `#pragma once`,
token-based guard detection, and a depth cap instead of `SIGABRT`; `__COUNTER__` ordering;
`#line`/`#error`/`#warning`/predefines/`-D`/`__has_include`; **provenance (round-1 finding
15) fully fixed** — no fabricated line-1/column-1 spans remain, and both `call_site →
Span::DUMMY` mutations are killed. The lexer did not regress.

---

## Blocking

### 11. The macro-expansion depth cap counts *sequential* expansions, so ordinary C is silently left unexpanded

`Engine::expand` (lib.rs:973) increments `expansion_depth` on entry, and `expand_inner`
(lib.rs:1054, 1072) is **tail-recursive over the rest of the token stream** — it appends
`input[i+1..]` to each replacement and recurses. The counter therefore measures "macros
expanded in this directive-free region", not nesting depth. With
`max_macro_expansion_depth: 256`, the **257th macro invocation between two directives
stops expanding** and emits a bogus diagnostic.

`#define M(x) (x)` followed by 400 × `int vN = M(N);` — gcc expands all 400; ours leaves
145 bare `M` tokens. A 20,000-link *acyclic* chain: gcc → `42`, ours → `M256` plus a
diagnostic. `macro_expansion.rs::expansion_chain_child` **asserts that diagnostic**, which
enshrines the wrong behaviour as a test.

Real code, with gcc's full predefine set: `vpp/src/vnet/ip/ip4_forward.c` produces **9**
such diagnostics, `src/vlib/main.c` **3**.

Fix the concept, not the constant: count nesting, and make `expand_inner` iterative over
the stream rather than tail-recursive. That also removes finding 16's OOM.

### 12. The operand of `#include <…>` is macro-expanded; on Linux this silently includes the wrong file

`Engine::include` (lib.rs:444) unconditionally does `self.expand(line.get(2..))`. C11
§6.10.2p4 expands only when the directive matches **neither** header-name form. gcc and
clang in gnu mode predefine `linux` and `unix` as `1`.

```c
#define foo 1
#include <foo/bar.h>     // gcc: inside_foo_bar.  ours: inside_1_bar, no diagnostic.
```

On real headers this yields `cannot include 1/errno.h`, `bits/mman-1.h`, `vlib/1/1.h`, and
leaves `MAP_PRIVATE`/`PROT_READ`/`EWOULDBLOCK`/`EINTR` unexpanded. VPP has **307**
`#include <…>` directives with `unix`/`linux` as a path component (96 × `<vlib/unix/unix.h>`
alone). This is the only residual divergence on `vec.h`/`pool.h`/`bitmap.h` under gcc's
predefines.

### 13. A substituted argument token keeps its *call-site* `leading_space` instead of the parameter's, corrupting `#`

`expand_function`'s substitution loop (lib.rs:1244) copies argument tokens without
adjusting `leading_space` from the body's parameter token. Both directions are wrong:

```c
#define S(x) #x
#define T(x) S(a x)
T(b)                     // gcc/clang: "a b"   ours: "ab"
#define ASSERT(t) S(t)
#define ELT(h,i) ASSERT((i) < len(h))
ELT(a, handle)           // gcc/clang: "(handle) < len(a)"   ours: "( handle) < len(a)"
```

On 57 vppinfra headers with `linux`/`unix` undefined on *both* sides (so finding 12 is
neutralised), **39 match gcc exactly and 18 diverge — 34 of the 36 divergent hunks are this
bug**, all inside `ASSERT`-generated string literals.

Same function, related: `stringize` (lib.rs:1270) escapes `\` and `"` in *every* token.
C11 §6.10.3.2p2 restricts that to character constants and string literals. `S(a\b)` → gcc
`"a\b"`, ours `"a\\b"`.

### 14. `_Pragma` leaks four tokens when its operand comes from a macro — the only form VPP uses

The `_Pragma` test in `expand_inner` (lib.rs:995) requires a `StringLit` already sitting at
`i+2`, and never re-examines a token once pushed to `output`.

```c
#define STR(x) #x
#define P(x) _Pragma (STR(GCC diagnostic x))
P(push)   // gcc: #pragma GCC diagnostic push
          // ours: _Pragma ( "GCC diagnosticpush" )
```

`src/vppinfra/warnings.h` is exactly this shape, so `memcpy_x86_64.h` and every file
reaching `WARN_OFF`/`WARN_ON` emits stray `_Pragma` tokens into the parser's stream. The
existing oracle case only covers the already-a-string form, which is why mutation M28 is
killed but this is not.

---

## Non-blocking but real

### 15. The lexed-header cache achieves zero reuse across TUs — round-1 finding 14 is only cosmetically closed

`CacheKey` (chiero-lex lib.rs:186) includes `start: BytePos`, the file's offset in the
*per-TU* global space, so two TUs including the same header never match. A reviewer test
measures **hits=0, misses=4**. `session_cache.rs` "proves" the fix by preprocessing the
same path with the same source twice — the one case where the key coincidentally matches
(see finding 33). 011 §5 calls the cache "required, not optional" precisely for the
cross-TU case. Related: `LexSession::lex` (line 225) clones the whole shared interner
`Vec<Arc<str>>` into *every* `LexedFile`. End-to-end: `clib.h` 718 ms vs gcc 53 ms (13×),
`vec.h` 1311 ms vs 64 ms (20×).

### 16. Two uncontainable process aborts remain

- `#if ((((…1…))))` — `ExprParser::primary → expression` (lib.rs:1797) is unbounded
  recursive. 20,000 parens, release, 8 MB stack: `stack overflow, aborting`, exit 134.
- `f(f(f(…)))` nested 100,000 deep: `memory allocation of 21587544 bytes failed`, exit 134
  — the O(n²) tail-copying in `expand_inner` under a 6 GB limit.

Both are `SIGABRT`, which `catch_unwind` cannot contain — the class round 1 flagged, and
011 c11 / 012 c17 are phrased as "zero panics".

### 17. No argument-count checking

`#define two(a,b) [a|b]`; `two(1)` → ours `[ 1 | ]`, `two(1,2,3)` → ours `[ 1 | 2 ]`, both
with no diagnostic. gcc and clang both hard-error. A malformed TU silently produces a
plausible-looking wrong token stream.

### 18. A value that does not fit `intmax_t` is not converted to `uintmax_t`

`parse_if_literal` (lib.rs:1866) only sets `unsigned` from a `u`/`U` suffix.
`#if 0x8000000000000000 > 0` and `#if 12345678901234567890 > 0` → gcc/clang yes, ours no.
012 §3.2 names `intmax_t`/`uintmax_t` explicitly. The only remaining divergence in the
57-expression battery.

### 19. `#pragma` is dropped entirely, not recorded

012 §3 says "recorded, mostly passed through"; `directive()` has `Some("pragma") => {}`.
gcc emits **375** `#pragma` directives for `clib.h`, including **113 × `#pragma GCC
target(...)`** and 113 × `push_options`/`pop_options` — the multiarch mechanism 060 depends
on. `ExpnKind::Pragma` records an `Expansion` no token ever references.

### 20. Spec gaps

- `MacroDef.name` and `MacroKind::FunctionLike.params` are filled with **positional indices
  cast to `Symbol`**, not interner symbols (lib.rs:591, 940). `MacroDef` is also
  unreachable from `PreprocessedTu`, so 012 §1's structure is write-only.
- Redefinition does not set the previous definition's `undef_span` (012 §1 says "if
  `#undef`'d **or redefined**"); no redefinition diagnostic.
- The branch is on a **stale `main`**: its `docs/specs/070` lacks the `#[ignore]`d-test
  rule this review is graded against, and its `012` lacks the `__VA_OPT__`/comma-swallowing
  paragraphs the code already implements. Rebase.

---

## Mutation campaign — 9 of 47 survive

The round-1 survivors are genuinely closed, including both `call_site → DUMMY` mutations.
Remaining survivors, each of which needs a discriminating test:

| # | Mutation | What it means |
|---|---|---|
| 21 | `,##a` comma branch deleted | Only a spurious diagnostic changes; finding 4's fix is unpinned. |
| 22 | `classify_paste` accepts any lex result | 012 §2.3's "not one pp-token → diagnose and keep both" is entirely untested. |
| 23 | Variadic arguments joined with no separating comma | **No test uses `__VA_ARGS__` at all**, or any variadic macro with ≥2 arguments — for a spec citing 230 `__VA_ARGS__` uses in VPP. |
| 24 | `##` operands still pre-expanded (only `#` excluded) | Half of 012 §2.2's "except" clause untested. |
| 25 | Guard skip ignores whether the guard macro is defined | Untested. |
| 26 | `__has_include` probe result not reused by the include | The claimed caching has no test. |
| 27 | `#define f (x)` treated as function-like | The object-like-with-space rule is untested. |
| 28 | Empty single argument not cleared for a zero-parameter macro | Untested. |
| 29 | Stringize escaping removed / hide-set not propagated through a paste | Both untested. |

---

## Test integrity

30. **`crates/chiero-lex/tests/contracts.rs:1` claims `Covers: 011 contracts 1, …, 12, 13,
    14`, but the file has no test for 11, 12 or 13.** Contract 12 (throughput) is carried
    *only* by `performance.rs`'s `#[ignore]`d test. `M2-NOTES.md` says 011 c12 "is owed" —
    but the `Covers:` line was never moved, so the citation is dishonest under 070 §4.
31. Fixed: 012 c17 is no longer claimed; c19's determinism test uses a nested fixture with
    ≥4 distinct contexts; c10 has a live-vs-inactive discriminator.
32. Context: `cargo xtask contract-coverage` only measures 020–024, so **every `Covers:`
    line in 011/012 is unverified prose**. Nothing mechanically catches finding 30.
33. `session_cache.rs::repeated_tus_use_the_pipeline_lexer_cache` cannot fail for the
    reason it claims (finding 15).

---

## Suggested order

1. **Finding 11** — affects every large TU; count nesting, and make `expand_inner`
   iterative over the stream. Removes finding 16's OOM as a side effect.
2. **12** (don't expand a well-formed `<h-char-sequence>`), **13** (inherit the parameter's
   `leading_space`; restrict escaping to literals), **14** (`_Pragma` after full expansion).
3. **15** — drop `start` from `CacheKey`, stop cloning the interner per file, re-measure
   against 012 §6.
4. **16** (bound the `#if` parser), **17** (arity), **18** (`uintmax_t` promotion),
   **19** (`#pragma`).
5. Move the 011 c12 citation off `contracts.rs` and list it owed; write the nine
   discriminators above, **especially a `__VA_ARGS__` test with two or more variadic
   arguments**.

---

## Addendum

**A. The 5-macro predefine stub is itself a deliverable, not just a harness setting.**
`Config::default()` supplies `__STDC__`, `__STDC_HOSTED__`, `__STDC_VERSION__`,
`__GNUC__=13`, `__x86_64__=1` and three feature queries hardwired to `0`. Because
`__GNUC_MINOR__` is absent, glibc's `__GNUC_PREREQ(3,3)` evaluates to `0`, so `__THROW`,
`__attribute__((…))` and `__extension__` all vanish: **13% of `clib.h`'s token stream
differs from gcc's — 257,310 tokens against 224,074 — with zero diagnostics reported.**
The "zero diagnostics on real headers" claim is literally true and analytically worthless.
A target predefine table belongs in `Config`.

**B. `PreprocessedTu::text()` still does a linear `ptr::eq` scan.** Harmless today because
the pipeline uses `text_at`, but it is a trap for the next consumer.

**C. Performance context** (not blocking): release, whole-header, `clib.h` 718 ms vs gcc
53 ms (13×), `vec.h` 1311 ms vs 64 ms (20×). Extrapolated to 1552 TUs on 12 cores that is
~3 minutes against 012 §6's budget of 10, so it fits — but the margin comes for free once
the cache and the per-file interner clone are fixed.

**D. The reviewer's artifacts** — differential harness, torture corpus, mutation scripts,
and five discriminating tests as `zz_reviewer.rs.saved` — are at
`/tmp/claude-1000/-home-ubuntu-rust-chiero-rs/936fd0ed-5e4a-4afa-884d-9b79e073f607/scratchpad/harness/`.
Re-run them rather than rebuilding the apparatus.
