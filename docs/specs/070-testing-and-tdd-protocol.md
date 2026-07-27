# 070 — Testing and TDD protocol

Two things are specified here: **how chiero knows it is right** (oracles), and **how
chiero gets written** (the red-green loop with adversarial review).

Both matter more than usual for this project. A symbolic executor's failure mode is not a
crash — it is a confident, well-formatted, wrong answer, produced silently and at scale.
Tests written by the same reasoning that wrote the code will not catch that. So the
strategy leans hard on **independent oracles**: other implementations that were written by
other people and can be asked the same question.

Verified available on this machine (2026-07-26): **gcc 13.3.0**, **clang 18.1.3**,
**z3 4.8.12**, ASan/UBSan via both compilers.

## 1. Oracles

### 1.1 Differential execution against real compilers — the primary oracle

For a C program with concrete inputs: compile and run it with gcc, run it under chiero
with the same inputs concretized, and compare.

```rust
pub struct DiffCase { pub src: PathBuf, pub inputs: Vec<ConcreteInput>, pub flags: Vec<String> }
pub struct DiffOutcome { pub exit_code: i32, pub stdout: Vec<u8>,
                         pub observable: Vec<(Symbol, Vec<u8>)>,   // final globals
                         pub trace: Vec<Effect> }                  // ordered extern calls
```

Comparison covers exit code, stdout, the final bytes of every observable global, and the
ordered sequence of side effects. Each program is compiled at `-O0` **and** `-O2`; a
disagreement between the two optimization levels usually means the program has UB, so
that case is routed to the UB checkers rather than reported as a chiero bug.

Both gcc and clang are used. Where they disagree with each other, the program is
unspecified or UB, and the case is reclassified rather than failing chiero. That
three-way comparison is more informative than either compiler alone.

**A third routing rule is required, and it is not optional.** chiero defines UB where
hardware does something else: `Shl` by ≥ width yields 0 (SMT-LIB), while x86 *masks* the
shift count — so gcc and clang agree with each other at both `-O0` and `-O2`, and produce
a value chiero deliberately does not. Both existing UB detectors stay silent and the case
is scored as a chiero bug. The same applies to division by zero
([020 §4.1](020-cir.md)).

So: **a program for which chiero emitted an arithmetic-UB event is excluded from result
comparison** (or compared only up to the point the event fired) and routed to the UB
corpus instead, where the assertion is that the event was emitted at all. Without this
rule the differential oracle systematically punishes chiero for being right.

This oracle is cheap, unlimited, and catches real semantic bugs — wrong integer
promotion, wrong struct offset, wrong evaluation order — that a hand-written expected
value would have encoded incorrectly in the first place.

### 1.2 Layout and constant evaluation against the compiler

Generated `_Static_assert`s for size, alignment and every field offset
([014 §7](014-semantics-and-types.md)), compiled by gcc and clang. A mismatch is a
compile error naming the field. This scales to **every record type in VPP**, which is
thousands of assertions for the cost of a generator.

### 1.3 Preprocessor against `gcc -E` / `clang -E`

Token-stream comparison after normalizing whitespace. Any divergence is a preprocessor
bug — and since chiero owns its preprocessor by choice
([002 decision](../../HANDOFF.md)), this oracle is not optional.

### 1.4 Solver against z3

Random well-sorted `QF_ABV` terms, every query to both tiers, disagreement on a definite
answer fails the build ([022 §7](022-solver.md)). Plus `paranoid` mode cross-checking
every tier-1 answer over the whole corpus.

### 1.5 Sanitizers as a finding oracle

Every emitted replay harness is compiled and run under ASan/UBSan
([040 §3.2](040-defect-checkers.md)). This closes the loop between "the solver says this
input reaches the bug" and "this input reaches the bug", using an implementation that
shares no code with chiero.

### 1.6 Coverage against gcov

Blocks chiero reports as executed for concretized inputs must match gcov's coverage of
the same program with the same inputs ([023](023-execution-engine.md) contract 21). This
validates the engine and the coverage vertical against each other.

### 1.7 Corpus and golden tests

`tests/corpus/` holds C programs, `.cir` modules, coverage artifacts, recipes and their
fixtures, each with expected results. Golden outputs are checked in and diffable;
regenerating them is an explicit `xtask` command, never automatic — a golden file that
silently updates itself tests nothing.

### 1.8 Property tests

Round-trip properties ([020](020-cir.md) contracts 1–2), solver model validation, memory
model invariants (no object overlap, address determinism), fidelity monotonicity, and
impact-set justification well-formedness. Failures shrink to a minimal case and the seed
is recorded.

### 1.9 Fuzzing

Csmith-style random C generation feeding the differential oracle, and structure-aware
fuzzing of the `.cir` parser and the gcov binary decoders (both parse untrusted-ish
bytes). Continuous rather than gating.

## 2. What is NOT an oracle

Chiero's own previous output — except as an explicit, reviewed golden file. A test that
asserts today's behaviour without an independent justification pins bugs in place. Every
golden file's initial creation requires a human or an oracle to have checked it, and the
commit message says which.

## 3. The TDD protocol

Per feature, four steps, four commits (two of which may be empty of changes if review
finds nothing):

### Step 1 — RED

Write a failing test. It comes from a **numbered contract in a spec**; the test names the
contract in a comment (`// 021 contract 9`). Commit `red: <desc>`.

The test must fail **for the right reason**. A test that fails because the function does
not exist yet is acceptable only for the first test of a module; after that, the failure
message must be a value mismatch. Commit the observed failure output in the commit
message body — it is evidence the test was actually red.

### Step 2 — Adversarial review of the test

A subagent reviews the test *before* implementation, adversarially, against:

- Is it **tautological** — asserting something that cannot fail, or that restates the
  implementation?
- Does it pin the **right contract**, or a coincidence of the current design?
- Does it test **behaviour or implementation**? Assertions on internal structure that has
  no specified meaning are churn.
- Would an **obviously wrong implementation** pass it? (Return a constant. Return the
  input. Do nothing.) If yes, the test is inadequate.
- Are the **boundaries** covered — zero, one, max, negative, empty, overflow?
- For a negative test: does it fail for the specified reason, or for an unrelated one?

Findings produce a supplemental `review:` commit.

### Step 3 — GREEN

Implement until the test passes. No unrelated changes. Commit `green: <desc>`.

### Step 4 — Adversarial review of the implementation

A subagent reviews the implementation against:

- **Does it cheat the test** — special-casing the fixture, hard-coding the expected value?
- Is it **correct beyond the test**, on the cases the test does not cover?
- Does it conform to the **spec**, not just to the test? Where it deviates, the spec must
  be amended in the same commit (`README` rule).
- Edge cases: overflow, empty input, maximum sizes, `Unknown` from the solver.
- Does it preserve **determinism** — no `HashMap` iteration in an output path, no time or
  randomness?
- Does it preserve **fidelity discipline** — every degradation recorded with a reason?
- Does it leak layering — VPP knowledge outside `chiero-vpp`, frontend types into
  `chiero-cir`?

Findings produce a supplemental `review:` commit.

**The reviews are not optional and not merged into one.** Reviewing the test before the
implementation exists is what makes the review honest: a reviewer who has seen the
implementation will unconsciously accept a test that matches it.

### Commit prefixes

`red:`, `green:`, `review:`, `spec:`, `chore:`, `perf:`, `fix:`.

## 4. CI gates

Collected from the specs; all must pass:

| Gate | Source |
|---|---|
| No dependency cycles; layering rules hold | [001](001-architecture.md) 1–5, 8 |
| `--no-default-features` builds and links no external solver | [001](001-architecture.md) 6, [022](022-solver.md) 1 |
| Byte-identical output across repeated runs | [001](001-architecture.md) 7 |
| Layout `_Static_assert`s compile for every parseable VPP record | [014](014-semantics-and-types.md) 12 |
| CIR round-trip and verifier contracts | [020](020-cir.md) 1–5 |
| Solver: zero definite-answer disagreements with z3; `paranoid` clean | [022](022-solver.md) 13, 18 |
| Fidelity token compile-fail test (`trybuild`) | [023](023-execution-engine.md) 13 |
| Native gcov decode matches `gcov --json-format` | [030](030-coverage-gcov.md) 5 |
| Macro attribution regression test | [030](030-coverage-gcov.md) 2 |
| Test selection recall 100% on replay and mutation corpora | [032](032-test-selection.md) 18–19 |
| Every checker has positive and negative fixtures; zero false positives on the clean subset | [040](040-defect-checkers.md) 1–2, 20 |
| Every finding emits a harness that compiles; memory-safety findings ASan-confirmed | [040](040-defect-checkers.md) 4–5 |
| Every shipped recipe passes its fixtures | [042](042-conformance-recipes.md) 4 |
| Tool envelope schema; `proven` only when `Exact` | [050](050-tool-interface.md) 1–2 |
| VPP parser coverage percentage does not regress | [060](060-vpp-integration.md) 17 |

**Gates name documents, never contract ranges.** A numeric range silently excludes every
contract added after it was written, which is how the review-wave fixes ended up outside
[080](080-roadmap.md)'s M1 gate. If a subset is genuinely intended, name the excluded
contracts and why.

Tracked-metric regressions (tier-1 `Unknown` rate, replay `NotReproduced` rate, parser
coverage, selection reduction) fail CI on a threshold, because a slow decay in these
never produces a single obviously-broken commit.

## 5. Performance testing

Benchmarks on the corpus with recorded baselines: solver calls per path, cache hit rates,
states per second, memory per state, tier-1 sweep time over VPP. Regressions beyond a
threshold fail. Performance claims in the specs (the `Bytes`-vs-`Array` threshold, the
independence-slicing win) are each backed by a benchmark, or they are deleted.

## 6. Testing chiero's own coverage

chiero is instrumented with `cargo llvm-cov`. The target is not a percentage — it is that
**every numbered spec contract maps to at least one test**, checked mechanically by a
contract-coverage report that lists unimplemented contracts. That report is the
implementation's real progress bar.

## 7. Testable contracts

1. The contract-coverage report enumerates every numbered contract in `docs/specs/` and
   the test(s) covering it; unimplemented contracts are listed, not silently absent.
2. Every test file references at least one spec contract by number.
3. The differential harness runs a corpus program under gcc, clang and chiero and reports
   a three-way comparison; a gcc/clang disagreement is classified as UB, not as a chiero
   failure.
4. A deliberately introduced off-by-one in struct layout is caught by the
   `_Static_assert` oracle, naming the field.
5. A deliberately introduced macro-expansion bug is caught by the `gcc -E` comparison.
6. A deliberately introduced solver bug (a wrong rewrite rule) is caught by the z3
   differential campaign within a bounded number of random terms.
7. Golden files are only regenerated by an explicit `xtask` command; a test run never
   rewrites one (verified by hashing the corpus before and after a full run).
8. The full CI gate set runs in a documented wall-clock budget on 12 cores.
9. Every `red:` commit's message contains the observed failure output.
10. Every `green:` commit is preceded by a `red:` commit touching the same test file
    (checked by a history-lint in `xtask`).
11. Every feature has both review commits, or an explicit recorded reason for their
    absence (history-lint).
12. Benchmarks record baselines, and a regression beyond the threshold fails.
