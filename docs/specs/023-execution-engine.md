# 023 — Execution engine

`chiero-exec` interprets CIR symbolically. It owns states, forking, the search strategy,
path-explosion budgets, and the hook points checkers plug into. It implements **no
checkers** ([040](040-defect-checkers.md)) and knows nothing about C source
(that stays in `Span` metadata it merely carries).

## 1. State

```rust
pub struct State {
    pub id: StateId,
    pub mem: Memory,                        // 021
    pub locals: IndexMap<ValueId, Term>,    // per active frame, in `stack`
    pub stack: Vec<Frame>,
    pub pc: Pc,                             // (FuncId, BlockId, inst index)
    pub path: PathCondition,                // Vec<Term>, append-only per state
    pub trace: PathTrace,                   // block sequence + fork decisions, for replay
    pub fidelity: Fidelity,                 // monotonically degrading (§7)
    pub assumptions: Vec<Assumption>,       // recorded, human-readable (§7)
    pub budget: BudgetUse,
    pub loop_counts: IndexMap<(FuncId, BlockId), u32>,
    pub status: Status,                     // Running | Terminated(Why) | Errored(Why)
}

pub struct Frame { pub func: FuncId, pub ret_to: Option<Pc>, pub ret_dst: Option<ValueId>,
                   pub locals: IndexMap<ValueId, Term>, pub scope_objs: Vec<ObjectId> }
```

`PathCondition` is append-only and shared structurally between forks (`Arc` prefix chain)
so a 200-constraint path costs one allocation per fork, not 200. The solver's caches
([022 §6](022-solver.md)) are keyed on the resulting term set, so sibling states hit them
constantly — the two designs are chosen together.

## 2. Stepping

One `step()` executes one `Inst` and is total: it either advances `pc`, forks, or
terminates the state. There is no partially-executed instruction, which is what makes
state serialization and mid-run interruption tractable.

Instruction semantics are the direct reading of [020 §4](020-cir.md):
`Assign` evaluates an `RValue` to a `Term`; `Load`/`Store`/`CopyMem`/`SetMem` delegate to
`chiero-mem`; `Marker` updates scope/line/assumption bookkeeping; `Opaque` havocs its
declared writes and degrades fidelity; `Call` is §5.

Evaluating an `RValue` is side-effect-free apart from memory faults, and the mapping from
CIR ops to solver ops is 1:1 by construction — CIR's explicit widths and signedness
([020 §2](020-cir.md)) exist precisely so this function contains no inference.

## 3. Forking

Forking happens at `Br`, `Switch`, `IndirectGoto`, symbolic-base pointer resolution
([021 §5.1](021-memory-model.md)), and wherever a checker requests it.

At `Br { cond, t, f }`:

1. Query the solver for feasibility of `cond` and of `¬cond` under the path condition.
2. Both feasible → fork. Both infeasible → the state is dead (a bug in chiero or an
   unsatisfiable path condition; asserted in debug).
3. Either query returning `Unknown` → **take that branch anyway**, with the constraint
   added and `Fidelity` degraded to at least `Approximated`. Dropping a branch the solver
   could not refute would let "no bug found" mean "the solver timed out", which §7
   forbids.
4. If the condition is a `Const`, no solver call is made. This trivial fast path carries
   most of the traffic and must exist before any benchmark is believed.

Fork order is **deterministic**: the true branch is explored first, `Switch` cases in
sorted order, then `default`. New `StateId`s are allocated from a monotone counter.

## 4. Search

```rust
pub trait Searcher {
    fn add(&mut self, states: Vec<StateId>);
    fn next(&mut self) -> Option<StateId>;
    fn remove(&mut self, id: StateId);
    fn name(&self) -> &'static str;
}
```

Shipped strategies:

| Strategy | Use |
|---|---|
| `Dfs` | Default. Cheap memory, good locality for the solver caches. |
| `Bfs` | Shallow-bug hunting, bounded-depth proofs. |
| `RandomPath` | KLEE's binary-tree-weighted random descent; avoids DFS starvation in loops. |
| `CoverageNew` | Prioritizes states whose next block is uncovered; the default for `find_bugs`. |
| `Interleaved(Vec<Box<dyn Searcher>>)` | Round-robin over the above. |

Every strategy is **deterministic**, including `RandomPath`: its PRNG is seeded from a
config value (default 0) that is recorded in every result. Ties break by `StateId`.
A non-reproducible bug report is not a bug report.

## 5. Calls

- **Direct call to a `Defined` function** — push a `Frame` and continue. There is no
  inlining: an explicit call stack keeps `Span` backtraces honest and makes recursion
  bounding a counter rather than a heuristic.
- **`Declared` (extern)** — consult the model registry ([024](024-environment-models.md)).
  No model → havoc every pointer argument's pointee (conservatively, the whole object),
  return a `Fresh` value, degrade to `Approximated`, and record the function name in
  `assumptions`. Silently returning 0 is forbidden.
- **Indirect** — resolve the callee term against known function addresses; fork per
  candidate, capped at `max_indirect` (default 16), plus one reported "unresolvable
  callee" state. VPP's node dispatch is indirect calls through registration tables, so
  this path is load-bearing, not exotic.
- **Recursion** — bounded by `max_recursion_depth` (default 32) per `(FuncId)` in the
  active stack; exceeding it terminates the state with `Bounded`.

`FnAttrs::noreturn` terminates the state normally at the call.

## 6. Checkers

```rust
pub trait Checker {
    fn name(&self) -> &'static str;
    fn on_event(&mut self, ev: &Event, ctx: &mut CheckerCtx) -> Vec<Action>;
}

pub enum Event<'a> {
    BeforeInst { st: &'a State, inst: &'a Inst },
    AfterInst  { st: &'a State, inst: &'a Inst },
    MemFault   { st: &'a State, fault: &'a MemFault },
    ArithEvent { st: &'a State, kind: ArithKind, inst: &'a Inst },   // overflow, shift, div0
    Fork       { st: &'a State, cond: Term, feasible: (bool, bool) },
    Call       { st: &'a State, callee: FuncId, args: &'a [Term] },
    Return     { st: &'a State, val: Option<Term> },
    Terminated { st: &'a State, why: &'a TermReason },
}

pub enum Action {
    Report(Finding),
    Assume(Term),         // constrain and continue
    Kill(TermReason),
    Fork(Term),           // explore both sides of a checker-invented condition
}
```

Checkers see everything and decide nothing about execution order. `CheckerCtx` gives them
solver access (`may(cond)`, `must(cond)`) and a `witness()` that extracts a concrete model
— but **only through this interface**, so that every finding is forced to come with a
counterexample or explicitly declare it has none.

Multiple checkers observing the same event report independently; deduplication is
[040](040-defect-checkers.md)'s job, by `(checker, span, object, kind)`.

## 7. Fidelity — the hard rule

```rust
pub enum Fidelity { Exact, Bounded, Approximated, Unknown }   // ordered, worst wins
```

| Value | Meaning |
|---|---|
| `Exact` | The explored region was explored completely, and every solver answer was definite. |
| `Bounded` | Exploration was cut by a documented budget (depth, unroll `k`, state cap, time). Findings are real; absence of findings proves nothing beyond the bound. |
| `Approximated` | Something was modeled imprecisely: an unmodeled extern, `Opaque`/asm, a float, an over-cap concretization. |
| `Unknown` | A solver returned `Unknown` on a decision that mattered, or a `LoweringGap` was reached. |

Rules, non-negotiable:

1. `Fidelity` only ever degrades within a state; it is never restored.
2. A result's fidelity is the **worst** over all states that contributed to it.
3. Every degradation appends an `Assumption { kind, span, detail }` naming what caused
   it. "Approximated" with no reason is a bug.
4. **A negative result (`no bug found`, `equivalent`, `test not affected`) may only be
   reported as a proof when fidelity is `Exact`.** At any other level the API returns
   `NotProven { fidelity, assumptions }` and the user-facing text says "not found within
   <bound>", never "does not exist". This is enforced by the type system: the
   proof-carrying variants are unconstructible without an `ExactWitness` token that only
   the engine can mint.

Point 4 is the whole reason the enum exists, and it is what makes chiero usable as an LLM
tool: an LLM will read "no bugs" as "safe", so chiero must be structurally incapable of
saying it loosely.

## 8. Budgets

```rust
pub struct Budget {
    pub max_depth: u32,            // 10_000 instructions per path
    pub max_loop_iters: u32,       // k, per (func, back-edge), default 8
    pub max_states: u32,           // 10_000 live
    pub max_forks: u32,            // per run
    pub wall_clock: Duration,      // default 60s
    pub max_solver_time: Duration, // per query, default 10s
    pub max_memory_objects: u32,
}
```

Loop bounds are per **back edge**, identified by dominator analysis over the CFG, not by
syntactic loop recovery — CIR has no loops ([020 §1](020-cir.md)). Exceeding any budget
terminates the affected state with `TermReason::Budget(which)` and degrades the run to
`Bounded`, recording which budget bound and where.

Budgets are reported in every result whether or not they were hit, so a reader can tell
`Exact` -with-generous-bounds from `Exact`-with-trivial-bounds.

## 9. Results and replay

```rust
pub struct RunResult {
    pub findings: Vec<Finding>,
    pub fidelity: Fidelity,
    pub assumptions: Vec<Assumption>,
    pub budgets: Budget, pub budget_hits: Vec<BudgetHit>,
    pub stats: RunStats,             // states, forks, solver calls, cache hits, time
    pub coverage: BlockCoverage,     // blocks/edges reached — feeds 032
}

pub struct Finding { pub kind: FindingKind, pub span: Span, pub backtrace: Vec<ExpnFrame>,
                     pub trace: PathTrace, pub witness: Option<Witness>, pub fidelity: Fidelity }
```

`Witness` is a concrete assignment for every symbolic input on the path: parameter values,
lazily-materialized object contents, extern return values. It is what
[040](040-defect-checkers.md) turns into a compilable C replay harness, and it is what
distinguishes a chiero finding from a plausible-sounding guess.

Findings carry `backtrace: Vec<ExpnFrame>` ([010 §3.1](010-source-and-provenance.md)), so
a bug inside a macro reports both the expansion site and the macro body — the same
provenance machinery that powers test selection.

## 10. Concretization policy

Concretization is the engine's main source of silent unsoundness, so every instance is
explicit, capped, and recorded:

| Situation | Policy |
|---|---|
| Symbolic pointer base | fork per object, cap `max_resolutions` ([021 §5.1](021-memory-model.md)) |
| Symbolic memcpy/memset size | bound by object size; if still symbolic, fork on small values, else concretize + `Bounded` |
| Symbolic allocation size | solver model, add the equality constraint, `Bounded` |
| Symbolic array index into `Bytes` | ite-chain up to `ite_threshold`, else promote to array theory (no concretization) |
| Symbolic `Switch` scrutinee | fork per case; `default` gets the negation of all cases |

Every concretization appends an `Assumption` with the constraint that was added, so a
witness replays deterministically.

## 11. Non-goals for v1

No state merging (deferred; the `Searcher` API is designed not to preclude it).

**No interleaving.** Execution is sequentially consistent and single-threaded, and a
result that touched concurrently-shared state is capped at `Bounded`. The full treatment
— VPP's threading discipline, the thread-context parameter, the discipline checker that
v1 *does* ship, and the v2 hooks — is [025](025-concurrency-and-threading.md). Read that
before concluding chiero says nothing about threads; it says a bounded, useful amount.

Multiple *states* may be explored in parallel OS threads — that is a scheduling detail of
chiero itself, unrelated to the analysed program's concurrency, and it must not change
results (contract 17).

## 12. Testable contracts

1. Executing `%a = add i32 2, 3; ret %a` yields a single terminated state with return
   value 5, `Fidelity::Exact`, and zero solver calls.
2. `Br` on a `Const` condition makes zero solver calls and produces one successor.
3. `Br` on `x < 5` with `x` fresh produces exactly two states with path conditions
   `x < 5` and `¬(x < 5)`, in that order.
4. A `Br` whose feasibility query returns `Unknown` produces a state on that branch with
   `Fidelity ≥ Approximated` and exactly one `Assumption` naming the solver timeout.
5. A loop `while (i < n) i++` with `max_loop_iters = 3` terminates, produces 4 states, and
   the run's fidelity is `Bounded` with a `BudgetHit` naming the back edge.
6. The same program run twice produces identical `StateId` sequences, identical fork
   order, identical findings, and byte-identical output — including under `RandomPath`
   with the default seed.
7. Changing the `RandomPath` seed changes exploration order and the seed appears in
   `RunResult`.
8. A call to an unmodeled extern with a pointer argument havocs the pointee, returns a
   fresh value, sets `Approximated`, and records the function name in `assumptions`.
9. Recursion depth 33 with `max_recursion_depth = 32` terminates that state with
   `Budget(Recursion)` and does not stack-overflow the interpreter (the interpreter's own
   stack usage is O(1) in program recursion depth).
10. An indirect call resolvable to 3 functions forks into 4 states (3 + unresolvable), and
    with `max_indirect = 2` yields `Bounded` and a recorded cap.
11. `Fidelity` never increases: property test over 10 000 random runs asserts monotonicity
    per state.
12. Every state whose fidelity is worse than `Exact` has ≥ 1 `Assumption`.
13. A `RunResult` with `Fidelity != Exact` cannot be converted into a `Proof`-carrying
    answer — this is a *compile-fail* test (`trybuild`), not a runtime check.
14. `no bugs found` under a hit budget renders as "no bugs found within <bound>" and never
    as "no bugs exist" (golden test on the rendered text).
15. Every `Finding` from the corpus has a `Witness`, or an explicit
    `witness: None` with a recorded reason.
16. Replaying a `Witness` through the engine with all inputs concretized re-reaches the
    same `Finding` at the same `Span` — for every finding in the corpus.
17. Running with 1, 2 and 8 worker threads produces identical `RunResult`s (modulo
    wall-clock timings in `stats`).
18. Exceeding `max_states` terminates the run cleanly with `Bounded`, reporting the
    findings already collected — no data loss, no panic.
19. A checker returning `Action::Assume` constrains the state and the subsequent branch
    feasibility reflects it.
20. Two checkers reporting the same event produce two findings at the engine level (the
    engine does not deduplicate).
21. `BlockCoverage` from a run over a corpus function matches the set of blocks the
    concrete gcc-compiled run covers for the same concretized inputs (ties the engine to
    the differential oracle in [070](070-testing-and-tdd-protocol.md)).
