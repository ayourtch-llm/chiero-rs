# Adversarial review 1 — `chiero-lex` / `chiero-pp`

**Verdict: not mergeable as "011 contracts 1–14 and 012 contracts 1–19 implemented and
tested."** The lexer is largely sound and the provenance model is right. The preprocessor
has defects that make it fail on ordinary C.

Method: a differential harness against **both** `gcc -E -P -std=gnu11` and
`clang -E -P -std=gnu11` over 20 hand-written torture cases plus real VPP headers, and a
25-mutation campaign. gcc and clang agreed on every input, so every mismatch below is ours.
**13 of 20 torture cases diverge. 12 of 25 semantic mutations survive the whole suite.**

A finding is a claim, not a verdict — check each one before acting, and say so if you
disagree with evidence. But the differential cases are reproducible facts: start there.

## Blocking, in the order I would fix them

1. **A function-like macro invocation spanning more than one line is never expanded.**
   `Engine::run` slices input into *lines* and calls `expand` per line, so `f(1,\n2)` stays
   unexpanded. gcc/clang: `<1,2>`; ours: `f ( 1 , 2 )`. C11 §6.10.3 ¶10 defines the next
   tokens over the *token stream*, not the line. **2996 sites in VPP** for just seven
   common macro names with an unclosed paren at end-of-line. This alone makes 012 c17
   unreachable, and it is architectural: expansion has to run over the token stream.

2. **Rescanning does not see tokens after the replacement list.** `#define B(x) [x]` /
   `#define A B` / `A(1)` → gcc `[1]`, ours `B ( 1 )`. C11 §6.10.3.4 ¶1: the result "and
   all subsequent preprocessing tokens of the source file" are rescanned.

3. **`##` in an *object-like* body is never performed.** `paste()` is only called from
   `expand_function`. `#define A x ## y` / `A` → gcc `xy`, ours `x ## y`. C11 §6.10.3.3 ¶3
   applies to both macro kinds.

4. **`,` before `##` is fused with the argument into one garbage token.**
   `#define D(f,a...) g(f,##a)` / `D("x",1)` → gcc `g ( "x" , 1 )`, ours `g ( "x" ,1 )`
   where `,1` is a single token classified as a Number. Comma-swallowing deletes the comma
   only when the variadic argument is **empty**; 012 §2.3 now says so explicitly. Every
   VPP logging macro of that shape produces a corrupt stream.

5. **Placemarker tokens leak into the output.** `#define h(a,b) a##b` / `h(,)` emits an
   empty `Other('\0')` token. C11 §6.10.3.3 ¶2–3: placemarkers are deleted after all `##`
   are processed.

6. **`#if` expressions are wrong in six independent ways**, and the dangerous ones fail
   *silently* by returning the LHS when they hit an operator they do not know:
   `0xFF` → 0 (the alphabetic-suffix trim eats `FF` and the `x` before the `0x` check);
   `010` is decimal 10; no `&`, `|`, `^`, `<<`, `>>`, `~`, `?:`, `,`; char constants are 0;
   all arithmetic is `i128` with no unsigned conversion. `#if 1 & 0` selects the **wrong
   branch** with no diagnostic. VPP has 207 `#elif` and pervasive `#if (X & Y)`.
   012 §3.2 requires `intmax_t`/`uintmax_t` with C's usual arithmetic conversions.

7. **`#elifdef`/`#elifndef` emit both branches** — unhandled, so they fall to the
   "unsupported directive" arm and leave the frame active. 012 §3's table requires them
   accepted-and-diagnosed. Accepting a directive by ignoring it is not accepting it.

8. **`#include` is a textual pre-pass, not a directive.** Consequences, all verified:
   angle includes unsupported (`#include <stdio.h>` is dropped — **13437 in VPP**, and 012
   §3.1's search order does not exist, nor does a `Config` field for include paths);
   `#include` inside a false `#if` still reads from disk (012 §3.2: inactive branches are
   lexed, not analyzed); no depth limit, so a self-including header **aborts the process**
   with a stack overflow, which `catch_unwind` cannot contain; and `#define`/`#pragma once`
   are matched textually with a mandatory trailing space, so `#define\tG2` is invisible.

9. **`__FILE__`/`__LINE__` are wrong inside any included file** — a consequence of 8: all
   included text lands in the *includer's* `SourceFile`. This silently poisons 030's
   coverage keying and 032's test selection, which are the reason provenance exists.

10. **`__COUNTER__` is consumed by argument pre-expansion that is then discarded.**
    `str(__COUNTER__)` then `__COUNTER__` → gcc `"__COUNTER__"`, `0`; ours `1`. 012 §2.2:
    arguments are pre-expanded "except where the parameter is an operand of `#` or `##`".
    Related: parameters are pre-expanded in `BTreeMap` order (alphabetical by parameter
    name), not left to right — deterministic but wrong when two arguments have side effects.

11. **`#line`, `#error`, `#warning`, `_Pragma`, `#include_next` are unimplemented** and
    all reach the "unsupported directive" arm. `_Pragma` is not a directive at all and
    leaks four tokens into the output.

12. **No predefined or `-D` macros, and no way to supply them.** `__STDC__` (named in 012
    §4), `__GNUC__`, `__x86_64__`, `__has_include`/`__has_attribute`/`__has_builtin` (012
    §4: "VPP does not build without these") are all absent. This is the only real-header
    divergence found: `clib.h` takes the wrong branch at `#if defined(__x86_64__)`.

13. **Preprocessing is O(n²) in token count.** `Engine::new` calls `lexed.text(token)` per
    token, and `text` finds the index by a linear `ptr::eq` scan. Measured in release:
    clean 4× per doubling, ~313 ms at 320 KB. Extrapolated to a 4 MB post-include TU that
    is ~50 s each; 1552 TUs on 12 cores is ~1.8 h against 012 §6's "under 10 minutes".

14. **The lexed-header cache is never used by the pipeline.** 011 §5 calls it "required,
    not optional". `lex_cached` is called only from `tests/performance.rs`; `Engine::new`
    builds a fresh `LexSession` per TU, so nothing is reused and the interner is per-TU.
    011 c13 is green via a test-only API.

15. **Every `#`/`##`-produced token has a fabricated `expansion_loc` of line 1, column 1**
    — `stringize` and `paste` pass `Span::DUMMY` as the expansion's `call_site`. 010 §4: "a
    span must never be fabricated by pointing at unrelated real source", and 010 §3.1 says
    of `expansion_loc`: "**THIS IS WHAT GCOV SEES.** Coverage correlation must use this and
    nothing else." `chiero-span::add_macro`'s own doc comment names this exact hazard on the
    definition side; this reintroduces it on the expansion side. No test inspects
    `expansion_loc`, `spelling_loc` or `origin` for any `#`/`##` token.

## Test-integrity findings — these are the ones that let the above survive

- **012 c17 is cited by a test that cannot fail**: it is `#[ignore]`d, returns early when
  the compilation database is missing (it is), and even on a hit only asserts the file
  contains `"file"` — it never preprocesses anything.
- **011 c12 is cited by an `#[ignore]`d test.**
- **012 c19 is cited by two tests, only one of which checks anything**, over an input of
  builtins only, which exercises no `ExpnCtx` nesting. (The property is true — I verified
  it on a richer input — it is just not tested.)
- **012 c10 (`#if 0` produces no diagnostics) is vacuously true**: `Engine::new` sets
  `diagnostics = Vec::new()` and never promotes lexer diagnostics at all, so 011 c6/c7's
  unterminated-literal diagnostics — which the lexer produces correctly — are discarded by
  the only consumer. A discriminating test puts an unterminated string in a **live** branch
  and asserts a diagnostic *is* reported.

**070 §4 now states the rule this exposed**: a contract cited only by an `#[ignore]`d test
counts as **uncovered**. Ignored tests are fine for an environment that cannot run them;
they may not carry the `Covers:` line. Move those citations off and list the contracts as
owed in `M2-NOTES.md`.

## Mutation survivors worth fixing directly (12 of 25)

The four kills claimed in `M2-NOTES.md` all reproduce genuinely. The gaps:

- **`add_expansion` parent → `ROOT` survives** — flattens the entire macro nesting chain
  with no test noticing. 012 c7's test observes a token from the `__LINE__` *builtin* path,
  whose parent is set elsewhere, so it cannot fail for the function-like case.
- **`#else` ignoring `taken` survives** — every `#else` fixture uses a *false* `#if`, so
  correct and mutated code agree. Discriminator: `#if 1 / A / #else / B / #endif`.
- **`#elif` never evaluated survives** — `#elif` is untested; 207 uses in VPP.
- **`parse_args` ignoring paren nesting survives** — no test passes a nested-paren argument.
- **Blue paint not applied to argument-derived tokens survives** — only the body path is
  tested (012 §2.4, C11 §6.10.3.4 ¶2).
- **Using pre-expanded args at `#`/`##` survives** — 012 §2.2's "except" clause is tested
  in one direction only. Discriminator: `#define X 1` / `#define cat(a,b) a##b` /
  `cat(X,2)` must be `X2`, not `12`.
- Stringize internal-space normalization; `__LINE__` using `spelling_loc` instead of
  `expansion_loc`; `#undef` as a no-op; CRLF splices; any `#` making a line a directive.

## Robustness

`Engine::expand` and `expand_includes` are both unbounded-recursive. Both abort the process
(`SIGABRT`, not a panic), so a `catch_unwind` harness cannot contain them — and 011 c11 and
012 c17 are phrased as "zero panics". gcc caps include depth at 200 and says so.

## What is sound, and worth not breaking

- **The lexer.** Six real VPP headers tokenize identically to gcc *and* clang. pp-numbers,
  maximal munch, digraphs, trigraph opt-in, comment→space, unterminated-literal recovery,
  keywords-as-`Ident` — all verified and killed by mutation. The corpus lexer test really
  does run over the real tree.
- **Line splicing** inside identifiers, string literals, keywords and `//` comments matches
  gcc byte-for-byte, with the span covering the full physical extent.
- **Blue paint** for direct, mutual, and through-argument recursion matches gcc.
- **The 010 §3.2 worked example reproduces exactly** — body vs. argument spelling sites,
  the two-frame backtrace outermost-first, `MacroBody`/`MacroArg` origins, and nested
  parenting. This is the part that is hardest to retrofit and it is right.
- Nested conditionals, `defined X`/`defined(X)`, short-circuit non-evaluation, and the
  classic `#define foo foo bar` / `#define bar foo` case all match gcc.

## Suggested order

1. Move expansion off lines and onto the token stream (fixes 1 and 2, and is a
   precondition for most of the rest).
2. Rewrite `#if` evaluation properly — full operator set, octal/hex/char literals,
   `intmax_t`/`uintmax_t` conversions. It currently picks wrong branches silently.
3. Make `#include` a directive inside the engine: conditional-aware, angle brackets, a
   search path in `Config`, a depth cap, and its own `SourceFile` per included file (fixes
   8 and 9).
4. `##`/`#` corrections: object-like bodies, comma-before-`##`, placemarkers, `#`/`##`
   operands not pre-expanded, real `call_site` spans instead of `DUMMY` (fixes 3, 4, 5,
   10, 15).
5. Predefined macros and `-D` (12), then `#line`/`#error`/`#warning`/`_Pragma` (11).
6. The O(n²) text lookup and the unused header cache (13, 14).
7. Write the discriminating tests listed under mutation survivors, and fix the four
   test-integrity findings.

`__VA_OPT__` is now **explicitly out of v1 scope** (012 §2.3, just committed on `main`):
VPP uses `__VA_ARGS__` 230 times and `__VA_OPT__` zero times. Encountering it should be a
diagnostic, not a silent pass-through.
