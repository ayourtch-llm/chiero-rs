# 032 — Test selection

`chiero-select` answers the user-facing question: **given this change, which tests should
I run?** It consumes an `ImpactSet` ([031](031-change-impact.md)) and a `CoverageIndex`
([030](030-coverage-gcov.md)) and produces a ranked, justified test list.

The governing asymmetry: **a missed test is a shipped regression; an extra test is
wasted CPU.** Every design decision below resolves in favour of running too much, and
every mechanism that removes a test requires a proof rather than a heuristic.

## 1. Pipeline

```
ImpactSet ──▶ ① coverage intersection ──▶ candidates
                        │
CoverageIndex ──────────┘
                        ▼
              ② symbolic refinement (removal requires proof)
                        ▼
              ③ ∪ always-run safety set
                        ▼
              ④ ranking + justification ──▶ Selection
```

## 2. Coverage intersection

For each impacted entity, its lines are mapped to tests:

```rust
fn tests_for_entity(&self, e: &Entity) -> TestBitmap    // union over the entity's lines
```

Lines come from the entity's `Span`s through `SourceMap::expansion_loc`, never
`spelling_loc` — the rule from [030 §1](030-coverage-gcov.md). For non-macro entities
this is the obvious join.

**For macro-body changes it is the whole trick.** The changed entity is a macro, which has
no coverage lines of its own; but [031 §3.2](031-change-impact.md) already converted that
change into a set of *impacted functions*, and functions do have coverage. So the
intersection is well-defined precisely because impact closure ran first. A tool that
tried to intersect coverage with the diff directly would find nothing, which is the
failure this whole architecture exists to avoid.

Entities with **no coverage at all** are not "unaffected" — they are unmeasured, and go
to the safety set (§4).

## 3. Symbolic refinement

Intersection over-approximates: a test that executed a changed line may still be unable
to observe the change. Refinement prunes those, and it is the only step that *removes*
candidates, so it is governed by one rule:

> **A test may be dropped only on an `Exact` proof ([023 §7](023-execution-engine.md)).
> `Bounded`, `Approximated`, `Unknown`, or a solver timeout all mean "keep".**

Three refinements, cheapest first:

### 3.1 Equivalence refinement (entity-level)

For each changed function, ask [041](041-optimization-analysis.md)'s
`prove_equivalent(before, after)`. If the two versions are provably equivalent —
observationally identical for all inputs — the entity is dropped from the impact set
entirely, along with every test selected only because of it.

This is the highest-leverage refinement because it removes an entity, not a test. It
fires on real commits more often than intuition suggests: refactors, renamed locals,
reordered independent statements, strength reductions, and `Cosmetic`-adjacent edits that
were not quite textually cosmetic. It is also exactly the primitive
[050](050-tool-interface.md) exposes to an LLM.

### 3.2 Reachability refinement (block-level)

Requires `CoverageDetail::LinesAndArcs` (native `.gcno`/`.gcda` only,
[030 §4](030-coverage-gcov.md)). A test whose arc-level coverage shows it never entered
the block containing the change cannot observe it. This is bookkeeping rather than
solving, and it matters because line-level coverage attributes a whole line — including a
multi-statement macro expansion — to a test that only executed part of it.

### 3.3 Observability refinement (path-level)

For a test that does reach the change: reconstruct the constraint implied by its recorded
arc coverage, and ask whether old and new differ under it. If the solver proves they
cannot, the test is dropped.

This is the most expensive and the most likely to return `Unknown`, so it runs last,
under a budget, and only for tests that survived §3.1 and §3.2. Budget exhaustion keeps
the test — never drops it.

Every dropped test records **why**, with the proof's fidelity, so a selection can be
audited after a regression escapes.

## 4. The always-run safety set

Unioned in unconditionally, never subject to refinement:

| Trigger | Reason |
|---|---|
| Tests with no coverage data | Unmeasured ≠ unaffected. |
| New tests (present in the tree, absent from the index) | Never had a chance to be measured. |
| `coverage_complete == false` ([030 §6](030-coverage-gcov.md)) | Crashed or killed mid-run — the most suspicious tests in the suite. |
| `CoverageIndex::validity() != Fresh` | Stale index; for `Stale { files }`, all tests touching those files. |
| `ImpactSet::completeness == Partial` | Unparsed files, unresolved indirect calls, unknown configs. |
| Build-system, config, or toolchain changes | Recompilation semantics changed underneath the index. |
| Changes to test infrastructure itself | |
| Explicit user pins (`--always-run <glob>`) | Escape hatch for knowledge chiero does not have. |

If the safety set swallows the suite, **that is the correct output**, and the report says
why in one line rather than burying it.

## 5. Ranking and justification

```rust
pub struct Selection {
    pub tests: Vec<SelectedTest>,       // ranked, deterministic
    pub excluded: Vec<ExcludedTest>,    // with the proof that justified exclusion
    pub confidence: Confidence,         // Complete | Reduced { caveats }
    pub stats: SelectionStats,          // candidates, refined away, safety-set size
}

pub struct SelectedTest {
    pub test: TestId, pub score: f64, pub rank: u32,
    pub reasons: Vec<SelectionReason>,  // ordered, human-readable
    pub est_duration: Option<Duration>,
}
```

Score inputs: shortest impact `distance` ([031 §3](031-change-impact.md)); change class
severity (`LayoutChanged`/`SignatureChanged` outrank `BodyChanged`); how much of the
change the test covers; whether the test hit the changed lines directly or transitively;
execution count; and estimated duration as a tiebreaker so cheap tests sort earlier.

Weights are configuration, printed in the report. A ranking whose weights are invisible
cannot be argued with, and this output will be argued with.

Justification is mandatory and concrete:

```
 1. test_ip4_forward          [direct]   covers ip4_rewrite_inline:912, which expands
                                         vec_add1 (changed body, vec.h:120)
 2. test_adj_midchain         [distance 2] calls ip4_rewrite_inline
 …
EXCLUDED (proof):
    test_span_mirror          equivalence: ip4_lookup_inline before/after proven
                              equivalent (Exact, 0.4s)
EXCLUDED (not impacted): 3 412 tests
ALWAYS-RUN: 12 tests (4 new, 6 no coverage, 2 crashed during collection)
CONFIDENCE: Reduced — 3 files did not parse (see 031 Partial)
```

### 5.1 Budgeted selection

`--budget 10m` truncates the ranked list. Truncation is **not** refinement: the dropped
tests were selected, so the report states the count, the residual risk, and the rank
cutoff, and `Confidence` becomes `Reduced`. A budgeted run must never render as if it
covered the impact.

## 6. Evaluation — how we know this works

A test selector that is never measured drifts into being a random sampler. Two metrics,
always reported together:

- **Safety (recall)** — of the tests that would have failed, what fraction were selected?
  Target: **100%**. Anything less is a bug, not a tuning parameter.
- **Reduction** — what fraction of the suite was skipped? This is the value delivered, and
  it is meaningless without the safety number beside it.

Two measurement harnesses, both in `xtask`:

1. **Historical replay.** For VPP commits with a known test failure, run selection on that
   commit's diff against the parent and assert the failing test was selected. This is the
   ground-truth oracle and the one that would catch a real design flaw.
2. **Mutation-based.** Inject a mutation (flip a comparison, change a constant, alter a
   macro body), find which tests fail, and check selection would have picked them.
   Cheaper, unlimited samples, and specifically able to target macro-body mutations — the
   case that is the entire point of §2, and the case a coverage-only baseline provably
   fails.

The coverage-only baseline is implemented deliberately, so every report can state the
delta chiero adds rather than asserting it.

## 7. Testable contracts

1. An empty diff selects only the always-run set.
2. A whitespace-only diff selects only the always-run set (via
   [031](031-change-impact.md) contract 1).
3. Changing one function's body selects exactly the tests covering it and its transitive
   callers, and no others.
4. **The headline contract**: for a macro-body-only change in a header, chiero selects the
   tests covering the expansion sites, and the coverage-only baseline selects none. Both
   run in the same test and the delta is asserted.
5. A test is dropped by equivalence refinement only when `prove_equivalent` returns
   `Exact`; a `Bounded` or `Unknown` result keeps it. Verified by a fixture that forces
   each fidelity level.
6. A solver timeout during refinement keeps every affected test and sets
   `Confidence::Reduced`.
7. Reachability refinement drops a test that covers the changed line but not the changed
   block, and only when arc-level coverage is available; with line-only coverage the same
   test is kept.
8. Every excluded test carries a proof record naming the refinement and its fidelity.
9. A test with no coverage data is always selected.
10. A test present in the tree but absent from the index is always selected and labelled
    `new`.
11. A test with `coverage_complete == false` is always selected.
12. A stale `CoverageIndex` forces every test touching the stale files into the selection
    and sets `Confidence::Reduced`.
13. `ImpactSet::completeness == Partial` sets `Confidence::Reduced` and the reason appears
    in the rendered report.
14. A build-config change selects the whole suite, with that stated as the reason.
15. Every selected test has ≥ 1 `SelectionReason`, and every reason references a real
    entity and span (verified structurally across all fixtures).
16. Ranking is deterministic and stable: identical inputs produce identical order.
17. `--budget` truncation reports the cutoff rank, the dropped count, and sets
    `Confidence::Reduced`; it never removes a test from `excluded` with a proof record.
18. **Safety gate**: on the historical-replay corpus, recall is 100%. A single miss fails
    CI with the commit and test named.
19. **Mutation gate**: over N macro-body mutations, chiero's recall is 100% and the
    coverage-only baseline's is measured and reported (expected: near zero) — the
    quantitative statement of the project's premise.
20. Reduction and safety are both present in every report; a report containing reduction
    alone is a formatting test failure.
21. Selection over a 500-file VPP diff completes within a documented time budget.
