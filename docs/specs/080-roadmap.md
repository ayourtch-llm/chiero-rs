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

**Exit:**
- [020](020-cir.md) contracts 1–30, [021](021-memory-model.md) 1–22,
  [022](022-solver.md) 1–20, [023](023-execution-engine.md) 1–21,
  [024](024-environment-models.md) 1–22 all green.
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
