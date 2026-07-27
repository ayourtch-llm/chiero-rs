# 080 — Roadmap

Milestones, each with an exit criterion that is a **checkable gate**, not a feeling. The
ordering honours the decision that the **symbolic core is built first**, against
hand-written CIR, before the frontend exists ([001 §3](001-architecture.md)).

Every milestone is built with the red-green-review loop in
[070 §3](070-testing-and-tdd-protocol.md).

## M0 — Skeleton

Workspace, 21 crates, `xtask`, CI, the corpus directory layout, and the contract-coverage
report ([070 §6](070-testing-and-tdd-protocol.md)) reporting 0%.

**Exit:** `cargo build --no-default-features` and `cargo test` pass on an empty
workspace; `xtask check-deps` enforces [001 §4](001-architecture.md) and fails on a
deliberately-violating fixture; CI runs on every commit.

## M1 — Symbolic core on hand-written CIR

`chiero-cir` (types, builder, verifier, textual parser and printer), `chiero-solver`
(term arena, `solver-lite`, SMT-LIB2 subprocess, `TieredSolver`, caches), `chiero-mem`,
`chiero-exec` (states, forking, searchers, budgets, fidelity), `chiero-model` (libc,
builtins, harness intrinsics).

Everything is tested against `.cir` fixtures. **No C is parsed in this milestone.** This
is the point of the CIR contract boundary, and the discipline that must not slip: if a
frontend dependency appears here, the build order has silently collapsed.

**Prerequisite: [015](015-lowering.md) must be settled before the fixtures are written.**
Hand-written `.cir` encodes conventions — marker placement, block shape, line attribution
— and if they are invented fixture-by-fixture, M2's real lowering will not reproduce them
and the core will turn out subtly wrong for real C. 015 exists to fix them on paper first.

**M1's oracles are weaker than the rest of the project's**, and that is worth stating
rather than discovering: the differential-against-gcc oracle (070 §1.1, the primary one)
cannot run until M2 lands, so M1 relies on `.cir` fixtures, property tests, and the z3
cross-check. This is a second reason to run M1 and M2 concurrently rather than serially.

**Exit:**
- **All** numbered contracts of [020](020-cir.md), [021](021-memory-model.md),
  [022](022-solver.md), [023](023-execution-engine.md) and
  [024](024-environment-models.md) are green.

  Stated as "all", not as numeric ranges. The ranges in an earlier draft were written
  before the review waves and never updated, so they excluded precisely the contracts the
  reviews added — `Opaque` outputs, vector ops, varargs, bit-granular init, provenance
  round-trips, `CheckerState` forking, `CallReturn`. Every one is `.cir`-fixture-testable
  core work with no other milestone home, so the fixes were sitting outside every gate.
  [070 §4](070-testing-and-tdd-protocol.md) now requires gates to name documents rather
  than ranges.
- z3 `paranoid` cross-check clean over the `.cir` corpus.
- The fidelity `trybuild` compile-fail test passes.
- A hand-written `.cir` program with a deliberate OOB produces a finding with a witness.

## M2 — Frontend

`chiero-lex`, `chiero-pp`, `chiero-ast`, `chiero-parse`, `chiero-sema`, `chiero-lower`,
and `chiero-span`'s `SourceMap` in full.

Provenance is implemented first and never retrofitted — `Span`/`ExpnCtx` are load-bearing
from the first token, because adding them later means touching every line of the frontend.

**Exit:**
- Preprocessor matches `gcc -E` and `clang -E` on the corpus (token stream, normalized).
- Layout `_Static_assert` gate passes for every corpus record under both compilers.
- Differential oracle green: corpus C programs agree between gcc, clang and chiero.
- Lowered CIR round-trips against checked-in goldens.
- **vppinfra headers parse**: `vec.h`, `pool.h`, `bitmap.h`, `clib.h` and their
  dependencies, with parser coverage reported.

## M3 — Provenance verticals (early value)

`explain_macro_expansion` and the `chiero-recipe` tier-1 structural sweep over all of VPP.

Placed here deliberately: both need only the frontend, both work across all 1552 files
without the engine being mature, and both are immediately useful. They also stress-test
parser coverage at full scale far earlier than a node-level analysis would, which is
where the frontend's real gaps will surface.

**Exit:**
- `explain_macro_expansion` answers correctly for `vec_add1` and a `foreach_*` site.
- Tier-1 recipe sweep runs over VPP within its time budget; the shipped catalogue passes
  its fixtures ([042](042-conformance-recipes.md) contract 4).
- VPP parser-coverage percentage published and tracked from here on.

## M4 — Coverage, impact, selection

`chiero-gcov`, `chiero-diff`, `chiero-select`.

**Exit:**
- Native gcov decode matches `gcov --json-format` on the corpus
  ([030](030-coverage-gcov.md) contract 5).
- The macro-attribution regression test passes ([030](030-coverage-gcov.md) contract 2).
- **The headline demonstration**: a macro-body-only change selects the right tests, and
  the coverage-only baseline selects none, asserted in one test
  ([032](032-test-selection.md) contract 4).
- Selection recall is 100% on the historical-replay and mutation corpora, with reduction
  reported alongside.

## M5 — Defect checkers and replay

`chiero-check`, `chiero-replay`, plus the concurrency-discipline checker from
[025](025-concurrency-and-threading.md).

**Exit:**
- Every checker has positive and negative fixtures; zero false positives on the clean
  subset.
- Every corpus finding emits a harness that compiles; every memory-safety finding is
  ASan- or UBSan-confirmed.
- The discipline checker's contracts pass on both a VPP-style and a pthread-style corpus
  (025 contracts 1–15).

## M6 — Adjudication

`chiero-opt`: `prove_equivalent`, opportunity detection, locality analysis.

**Exit:** [041](041-optimization-analysis.md) contracts 1–24 green, including the
`x/2` vs `x>>1` and `abs(INT_MIN)` distinguishing-input cases, and every `Differs`
producing a harness that demonstrates the divergence.

## M7 — Tool surface

`chiero-tool` (MCP + JSON-RPC), `chiero-cli`.

**Exit:** envelope schema validated for every operation; `proven` true only at `Exact`;
sandboxing contracts pass; CLI and MCP operation sets verified identical.

## M8 — VPP at scale

Staged per [060 §7](060-vpp-integration.md): vppinfra and its unit tests, then a single
node end to end, then coverage and selection against VPP's own test suite.

**Exit:**
- The vppinfra unit-test suite runs under chiero, every failure triaged as a real bug or
  a recorded limitation.
- One node analysed end to end with the frame model, producing findings with
  ASan-confirmed replays.
- Selection measured on VPP's real suite: recall and reduction both reported.

## Sequencing notes

**What can run in parallel.** M1 and M2 are genuinely independent — that independence is
the CIR contract boundary's reason for existing — and with 12 cores available, running
them concurrently is the single biggest schedule lever. M4's gcov decoder depends on
neither and can start any time.

**M4 can finish before M1.** Its exit gates — native gcov decode, macro attribution, the
headline selection demonstration, recall on the replay corpus — require the frontend and
the coverage/impact stack, not the symbolic engine; §3.1's equivalence refinement is
explicitly optional there. So a slipping M1 must not be allowed to block the headline
demo. This is also the honest reading of the build-order decision: the core-first order
runs *concurrently* with the vertical order rather than in front of it.

**What must not be reordered.** [025](025-concurrency-and-threading.md)'s `Sharing`
classification is an input to M6's false-sharing analysis. M6's `prove_equivalent` is
M4's strongest refinement — M4 ships without it and gains precision when M6 lands, which
is why [032 §3.1](032-test-selection.md) treats it as optional rather than required.

**The riskiest milestone is M2.** A hand-written C frontend for a 1M-line GNU-extension-
heavy codebase is the part most likely to overrun, and the temptation to fall back to
clang will be strongest exactly when it is hardest. That trade was already decided and
the reason stands: clang cannot give us macro provenance, and macro provenance is the
product. The mitigations are that parser coverage is measured continuously from M3, the
extension budget is already known from measurement
([HANDOFF §4.12b](../../HANDOFF.md)), and unparseable constructs degrade to skipped
functions with diagnostics rather than to wrong answers.

## Scope pressure, and what gives first

Honest accounting: this spec set describes a C frontend with compiler-grade layout, a
KLEE-class engine with ~25 checkers and a sanitizer-validated replay pipeline, a gcov
decoder plus impact and selection with recall gates, a relational equivalence prover, a
locality analyzer, a typestate recipe language, a concurrency-discipline checker, and an
MCP server — 21 crates and ~400 contracts, each contractually costing four commits under
[070 §3](070-testing-and-tdd-protocol.md). That is more than one product.

Two things have already been cut on correctness grounds, not schedule:
[032 §3.3](032-test-selection.md) observability refinement (unbuildable as specified), and
[041 §3](041-optimization-analysis.md)'s profile-dependent findings are now opt-in and
absent by default.

If more must give, this is the order and the reasoning — **cut from the tail, not the
middle**:

1. **[042](042-conformance-recipes.md)'s tier-2 typestate DSL.** Keep the tier-1
   structural sweep, which is M3's early value and cheap. Ship the ritual checks first as
   Rust `Checker`s through 042's own escape hatch, and grow the DSL from checks that
   already work — a grammar, metavariable binding, typestate automata and escape analysis
   is a compiler-sized sub-project to build speculatively.
2. **[025 §3](025-concurrency-and-threading.md)'s lock-order-inversion graph**, which
   needs cross-entry-point accumulation infrastructure. The per-path discipline findings
   are independent and stay.
3. **[041 §3](041-optimization-analysis.md)'s locality analysis entirely**, to v2.

What must **not** be cut, because other things depend on them:
`prove_equivalent` ([032 §3.1](032-test-selection.md) and the whole LLM story rest on it)
and the replay/sanitizer loop (it is the credibility mechanism — without it a finding is
an assertion).

## Definition of done for v1

1. Ingests gcov coverage from a real VPP build.
2. Given a diff, selects tests with measured 100% recall — including for macro-body
   changes, where the coverage-only baseline scores near zero.
3. Finds defects with witnesses and sanitizer-confirmed replay harnesses.
4. Adjudicates proposed rewrites via `prove_equivalent`, returning a proof or a
   distinguishing input.
5. Checks conformance recipes across the whole codebase.
6. Exposes all of it to an LLM through an envelope that makes overclaiming structurally
   impossible.
7. Never reports a negative result as a proof unless it is one.
