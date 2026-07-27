# M2 frontend notes

## Status

- 011 contracts 1–11, 13–14: covered by non-ignored tests.
- 011 contract 12: owed under updated 070 §4. Its ignored release performance test
  passes, but ignored evidence no longer counts as coverage.
- 012 contracts 1–16, 18–19: covered by non-ignored tests, subject to REVIEW-1 fixes.
- 012 contract 17: owed. Its external test is ignored and the required VPP compilation
  database is absent.

## Findings

- The existing `cargo xtask contract-coverage` gate only measures specs 020–024, so it
  cannot report 011/012 coverage. The tests still use the required `Covers:` header.
- `chiero-span::SourceFile` does not expose the splice-position list described by 011
  §2.2. `chiero-lex` therefore keeps a private physical-to-logical mapping while lexing
  and a spelling side table in `LexedFile`; no shared API addition is required yet.
- `/home/ubuntu/vpp/build-root/compile_commands.json` does not exist, and `find
  /home/ubuntu/vpp -name compile_commands.json` returns no alternatives. Contract 17's
  full configured-TU regression metric therefore cannot run in this environment.
- A representative macro/conditional/builtin fixture is compared token-for-token with
  both `gcc -E -P` and `clang -E -P`; all three agree.

## REVIEW-1

- Findings 1 and 2 reproduced exactly: expansion was line-bounded, and `A(1)` with
  object-like `A -> B` stopped at `B(1)`. Expansion now buffers active ordinary tokens
  until a directive boundary and rescans replacements together with the source suffix.
- The suggested nested-parenthesis, taken-`#else`, and evaluated-`#elif` discriminators
  already passed before implementation changes; they are now permanent regression tests.
- Findings 6 and 7 reproduced: hexadecimal constants selected the wrong branch and both
  `#elifdef` forms fell through. `#if` now implements the complete C operator precedence
  table used by integer constant expressions, hexadecimal/octal/binary and character
  constants, short-circuiting, and signed/unsigned 64-bit usual conversions.
- Findings 8 and 9 reproduced: inactive includes performed IO, header builtins and
  spelling spans named the includer, and whitespace variants defeated textual guard
  recognition. Includes are now active directives; every loaded header is its own
  `SourceFile`, quoted/angle search paths are explicit config, guards are token-based,
  and recursion is capped with a diagnostic.
- The live-vs-inactive lexer diagnostic discriminator now passes: the preprocessor
  promotes lexer diagnostics line-by-line only while the conditional state is active.
- REVIEW-1 finding 13's quadratic token spelling lookup was removed from the pipeline
  by adding indexed `LexedFile::text_at`; header lexing now uses the engine's cache.
- Findings 3–5, 10, and 15 reproduced: object paste was skipped, nonempty GNU varargs
  became a `,1` token, raw-only operands still consumed `__COUNTER__`, argument effects
  ran in name order, and operator expansion locations resolved to line 1. Object and
  function paste now share one placemarker-aware path, raw-only operands are not
  pre-expanded, arguments expand left-to-right, and `#`/`##` carry real operator sites.
- The new argument-blue-paint and nested function-parent discriminators pass without an
  implementation change, so those two mutation claims do not reproduce against the
  reviewed tree; the tests remain to prevent regressions.
- Per the updated 012 §2.3, `__VA_OPT__` is removed with a diagnostic rather than passed
  through.
- Findings 11 and 12 reproduced: predefined feature queries broke `#if`, line control
  was ignored, `_Pragma` leaked, and `#include_next` was rejected. Target predefines and
  conservative feature queries now exist; `#line`, `#error`, `#warning`, `_Pragma`, and
  search-position-aware `#include_next` have dedicated handling.
- Configured command-line-style object macros now lex arbitrary replacement lists,
  participate in `#if`, carry synthesized provenance, and obey later `#undef`.
- The unbounded expansion robustness claim reproduced in a subprocess: a 20,000-link
  acyclic macro chain died with `SIGABRT`. Expansion now stops at a configurable depth
  with a diagnostic, and the same child exits normally.
- REVIEW-1 finding 14 reproduced architecturally: one-shot entry points created a fresh
  lexer session, so only lexer-unit tests could hit the cache. `PreprocessorSession` now
  shares one per-worker interner/cache across real TU preprocessing; its pipeline test
  observes one miss followed by one hit.
- A reconstructed 20-case REVIEW-1 torture matrix now matches both GCC 13.3 and Clang
  18 token-for-token under `-std=gnu11`; the compilers agree on every case.
- Direct non-ignored preprocessing of `clib.h`, `vec.h`, `pool.h`, and `bitmap.h` now
  completes with zero diagnostics using VPP, GCC-internal, and system include paths.
  This campaign exposed and fixed a macro-generated argument whose spelling endpoints
  were in reverse global-source order; such a sequence now retains an honest component
  span instead of constructing an inverted or cross-file envelope.

## Mutation checks

- Disabling pp-number exponent-sign absorption made the contract-1 fixture tokenize
  `0x1e+2` into three non-EOF tokens; `pp_numbers_are_single_tokens` failed.
- Disabling cache lookup made the pointer-identity assertion in the contract-13 test
  fail before its timing assertion.
- Substituting raw arguments at ordinary parameter uses changed `xstr(__LINE__)` to
  `"__LINE__"`; the contract-4 test expected `"3"` and failed.
- Evaluating the right side of a false `&&` as live made `#if 0 && 1/0` diagnose;
  the contract-11 short-circuit test failed.
