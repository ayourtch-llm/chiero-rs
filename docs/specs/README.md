# chiero-rs specifications

`chiero` is a modular symbolic execution environment for C, written in Rust, designed to
be embedded as a library. Its primary validation target is
[VPP](https://gerrit.fd.io/r/vpp) — a ~1M-line, macro-saturated C codebase.

These documents are **normative**. Implementation follows them via red-green TDD
(see [070](070-testing-and-tdd-protocol.md)). Where an implementation must deviate, the
spec is amended in the same commit as the deviation.

## Reading order

| # | Document | What it fixes |
|---|---|---|
| [000](000-overview.md) | Overview | Goals, non-goals, the four capabilities, glossary |
| [001](001-architecture.md) | Architecture | Crate graph, dependency rules, the CIR contract boundary |

### Frontend

| # | Document | What it fixes |
|---|---|---|
| [010](010-source-and-provenance.md) | Source & provenance | `Span`, `ExpnCtx`, the expansion tree — **the differentiating design** |
| [011](011-lexer.md) | Lexer | Phases 1–3, pp-tokens |
| [012](012-preprocessor.md) | Preprocessor | Phase 4, macro expansion with full provenance |
| [013](013-parser.md) | Parser | C11 + the GNU extensions VPP actually uses |
| [014](014-semantics-and-types.md) | Semantics & types | Type system, layout/ABI, name resolution, const-eval |

### Symbolic core

| # | Document | What it fixes |
|---|---|---|
| [020](020-cir.md) | CIR | The IR; the contract the engine is built against |
| [021](021-memory-model.md) | Memory model | Objects, offsets, lazy initialization |
| [022](022-solver.md) | Solver | `Solver` trait, term language, tiered backends, caching |
| [023](023-execution-engine.md) | Execution engine | States, forking, searchers, path-explosion control |
| [024](024-environment-models.md) | Environment models | libc, builtins, and the model registry |
| [025](025-concurrency-and-threading.md) | Concurrency | Thread context, the discipline checker, and the declared blind spot |

### Capability verticals

| # | Document | What it fixes |
|---|---|---|
| [030](030-coverage-gcov.md) | Coverage ingest | `.gcno`/`.gcda`/JSON, per-test attribution |
| [031](031-change-impact.md) | Change impact | Diff → affected entities, incl. macro-body changes |
| [032](032-test-selection.md) | Test selection | Impact closure ∩ coverage, symbolic refinement |
| [040](040-defect-checkers.md) | Defect checkers | Logic errors, with replayable counterexamples |
| [041](041-optimization-analysis.md) | Optimization analysis | Provable rewrite opportunities, plus cache-line/locality analysis |
| [042](042-conformance-recipes.md) | Conformance recipes | Declarative usage-pattern rules checked across the codebase |
| [050](050-tool-interface.md) | Tool interface | The LLM-facing surface, incl. `prove_equivalent` |
| [060](060-vpp-integration.md) | VPP integration | compile_commands, vppinfra models, multiarch |

### Process

| # | Document | What it fixes |
|---|---|---|
| [070](070-testing-and-tdd-protocol.md) | Testing & TDD protocol | The red-green loop, oracles, adversarial review |
| [080](080-roadmap.md) | Roadmap | Milestones and their exit criteria |

## Status

All 23 documents are at **draft-1**, written 2026-07-26/27, **pending review before
implementation begins**.

Two documents were added during drafting in response to review, and are worth calling out
because they changed the shape of the system rather than filling in detail:

- **[025](025-concurrency-and-threading.md)** — a one-line "concurrency is a non-goal" was
  inadequate for a codebase whose hot path is worker threads. Measuring VPP's actual
  discipline (467 files index by `thread_index`; only ~70 use explicit synchronization)
  showed that a path-sensitive *discipline* checker is tractable even though interleaving
  exploration is not.
- **[042](042-conformance-recipes.md)** — mechanically checking that code follows a
  prescribed ritual, from declarative recipes. VPP has 1187 CLI command registrations
  whose cleanup ritual is hand-repeated on every return path with no enforcement anywhere.

Every document ends with **`## Testable contracts`** — numbered, checkable assertions.
Those are the source of the RED tests in [070 §3](070-testing-and-tdd-protocol.md), and
the contract-coverage report ([070 §6](070-testing-and-tdd-protocol.md)) tracks which are
implemented.
