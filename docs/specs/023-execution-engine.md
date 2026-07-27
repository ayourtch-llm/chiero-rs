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
    pub stack: Vec<Frame>,                  // locals live per frame, not here
    pub pc: Pc,                             // (FuncId, BlockId, inst index)
    pub path: PathCondition,                // append-only; carries possibly_infeasible (022 §6.1)
    pub trace: PathTrace,                   // block sequence + fork decisions, for replay
    pub fidelity: Fidelity,                 // monotonically degrading (§7)
    pub assumptions: Vec<Assumption>,       // recorded, human-readable (§7)
    pub thread: ThreadCtx,                  // 025 §2; mutable — barrier_sync changes it
    pub checkers: IndexMap<CheckerId, Box<dyn CheckerState>>,   // §6.1
    pub budget: BudgetUse,
    pub loop_counts: IndexMap<(FuncId, BlockId), u32>,
    pub status: Status,                     // Running | Terminated(Why) | Errored(Why)
}

pub struct Frame {
    pub id: FrameId,
    pub func: FuncId,
    pub ret_to: Option<Pc>,
    pub ret_dst: Option<ValueId>,
    pub locals: IndexMap<ValueId, Value>,
    /// Which object each alloca materialized to *in this activation* (021 §1).
    pub frame_objs: IndexMap<AllocaId, ObjectId>,
    pub scope_objs: IndexMap<ScopeId, Vec<ObjectId>>,
}
```

### 1.1 `Value`, not `Term` — pointers carry provenance

```rust
pub enum Value { Scalar(Term), Ptr(Pointer) }
```

This is not a convenience. [021 §2](021-memory-model.md) states that provenance is
*never* lost — arithmetic leaving an object's bounds still yields a pointer anchored to
that object — and `Pointer { base: ObjectId, off: Term }` is how. A bare `Term` has no
`ObjectId`, so storing locals as `Term` would leave only one way to recover the base:
searching the address space. That search is lossy **by construction**, because
[021 §7](021-memory-model.md) puts guard gaps between objects precisely so an
out-of-bounds pointer resolves to *no* object. Round-tripping a pointer through `Term`
therefore converts a detectable OOB into `UNBOUND`, and 021 contract 3 (`PtrAdd` past the
end and back again preserves `base`) becomes unimplementable.

`Value` is used consistently wherever a pointer can appear: `Frame::locals`,
`Event::Call { args }`, `Model::call`'s arguments ([024 §1](024-environment-models.md)),
and `Witness`. Without it, `memcpy` overlap detection and `free`'s object identification
have nothing to work from.

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
    Call       { st: &'a State, callee: Callee, args: &'a [Value] },
    /// Fired **in the caller** once the callee's result exists, for defined, modeled
    /// and unmodeled callees alike.
    CallReturn { st: &'a State, callee: Callee, ret: Option<Value>, dst: Option<ValueId> },
    Return     { st: &'a State, val: Option<Value> },
    Terminated { st: &'a State, why: &'a TermReason },
}

pub enum Action {
    Report(Finding),
    Assume(Term),         // constrain and continue
    Kill(TermReason),
    Fork(Term),           // explore both sides of a checker-invented condition
}
```

Two things about this event set are load-bearing for [042](042-conformance-recipes.md)
and were wrong in an earlier draft:

**Arguments are `Value`, not `Term`.** §1.1 argues at length that a bare `Term` cannot
carry pointer provenance, then an earlier `Event::Call` took `&[Term]` anyway. A checker
that cannot tell *which tracked object* an argument refers to cannot implement
`unformat_free($li)`, `free(p)`, or `memcpy` overlap detection — and 021 §7's guard gaps
make recovering it by address search lossy by construction.

**`CallReturn` exists because `Call` fires too early.** A typestate transition guarded on
a call's *result* — `on unformat_user(…, $li) returning nonzero`, the guard in the only
worked example in 042 — has no hook otherwise: `Event::Call` precedes the result, and
`Event::Return` fires in the callee's frame and never fires at all for an unmodeled
extern, which produces a `Fresh` value with no return instruction. `CallReturn` fires in
the caller for every callee kind.

Checkers see everything and decide nothing about execution order. `CheckerCtx` gives them
solver access (`may(cond)`, `must(cond)`) and a `witness()` that extracts a concrete model
— but **only through this interface**, so that every finding is forced to come with a
counterexample or explicitly declare it has none.

### 6.1 Checker state is per-state, and forks with it

A `Checker` is a stateless **observer**; everything it remembers lives in the `State`:

```rust
pub trait CheckerState: DynClone + Any {
    fn on_fork(&self) -> Box<dyn CheckerState> { dyn_clone::clone_box(self) }
}
```

`State::checkers` holds one `CheckerState` per registered checker, cloned on fork
alongside `Memory` (and, like it, copy-on-write).

Without this the path-sensitive checkers are simply unimplementable. A `&mut self` on the
`Checker` itself is shared across *all* states, while the `Searcher` interleaves events
from unrelated states arbitrarily — DFS backtracks between them, `RandomPath` and
`Interleaved` jump constantly, and parallel workers (§11) cannot share `&mut self` at all.
[025 §3](025-concurrency-and-threading.md) requires "a lock set per state" and a per-path
`Sharing` classification; a lock set accumulated across interleaved states is noise.

**Global checker state is the exception and must be declared.** 025's lock-order graph is
deliberately cross-state and cross-entry-point; it lives in `CheckerCtx::global`, and
025 contract 8 requires its result to be independent of the order states were explored.
Anything not declared global is per-state, and contract 17's requirement that 1, 2 and 8
worker threads produce identical results applies to it.

Multiple checkers observing the same event report independently; deduplication is
[040](040-defect-checkers.md)'s job, by `(checker, span, object, kind)`.

## 7. Fidelity — the hard rule

```rust
pub enum Fidelity { Exact, Bounded, Approximated, Unknown }   // ordered, worst wins
```

**This table is normative.** Every other document references it rather than restating it;
earlier drafts restated it and drifted into four mutually inconsistent versions.

| Value | Meaning | Causes — the complete list |
|---|---|---|
| `Exact` | The explored region was explored completely, and every solver answer was definite. | — |
| `Bounded` | Exploration was cut by a **documented budget**. Findings are real; absence of findings proves nothing beyond the bound. | depth, unroll `k`, state cap, fork cap, recursion cap, `max_resolutions`, `max_indirect`, `max_string_scan`, `LazyPolicy::max_depth`, unescalated recipe candidates, wall clock |
| `Approximated` | Something was **modeled imprecisely** — a deliberate lie about semantics, not a truncation of search. | unmodeled extern; any model with `Precision::Approximate`; `ModelOutcome::Havoc`; `Opaque`/inline asm; floats; `--vec-summary`; concretizing a symbolic value to one model |
| `Unknown` | The engine **does not know** and cannot bound its ignorance. | a solver `Unknown` on a decision that mattered; a `LoweringGap`; an access through `ObjectId::UNBOUND`; a pointer resolution with no information (021 §5.1) |

Two boundaries that earlier drafts got wrong in both directions:

- **A cap that was hit is `Bounded`; discarding values is `Approximated`.** Exceeding
  `max_resolutions` and then *concretizing to one model* does both: the cap is a budget,
  but keeping one of several feasible objects is a modeling lie. It is
  `Approximated`, because the stronger claim is the false one. Same for symbolic
  allocation and `memcpy` sizes.
- **A solver `Unknown` on a branch yields `Unknown`, not `Approximated`.** The table's own
  definition says so, and §3 takes the branch anyway — the engine genuinely does not know
  whether that path exists.

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

### 7.1 `ExactWitness`

The token must be bound to the run it blesses, or it is theatre — mint one from a trivial
`Exact` run (`return 0;`) and it would bless anything.

```rust
pub struct ExactWitness { run: RunId, _seal: PhantomData<Sealed> }  // private field, !Clone

/// The ONLY function in the workspace that reads `RunResult::fidelity` to decide
/// whether a result may be presented as a proof.
pub fn seal(r: &RunResult, w: ExactWitness) -> Result<Proven<'_>, NotProven>;
```

`ExactWitness` is non-`Clone`, has a private field, and is constructible only inside
`chiero-exec`. `seal` consumes it and additionally checks `w.run == r.id`, so a witness
from another run is rejected at runtime even though both are `Exact`.

Being honest about the guarantee: the type system prevents *downstream crates* from
forging a proof. It does not prevent `chiero-exec` itself from minting a witness on a
degraded run — that remains one ordinary `if`, concentrated in one function so it can be
reviewed and property-tested. "Structurally impossible to overclaim" is true of every
consumer and is a single audited branch inside the engine; contract 13b tests that branch
across all four fidelity levels.

## 8. Budgets

```rust
pub struct Budget {
    // Deterministic budgets — these gate output and are reproducible.
    pub max_depth: u32,            // 10_000 instructions per path
    pub max_loop_iters: u32,       // k, per (func, back-edge), default 8
    pub max_states: u32,           // 10_000 live
    pub max_forks: u32,            // per run
    pub max_memory_objects: u32,
    pub max_solver_rlimit: u64,    // z3 :rlimit — deterministic work units, NOT seconds
    // Non-deterministic backstop — never gates a reported result (§8.1).
    pub wall_clock: Option<Duration>,   // default 60s
}
```

### 8.1 Determinism requires that time not gate output

[001 §5](001-architecture.md) makes byte-identical output a hard requirement, and a
wall-clock timeout is not reproducible: it changes `Fidelity`, `assumptions` and
`budget_hits`, all of which are output. Worse, contract 17 asks for identical results at
1, 2 and 8 worker threads, and thread count changes solver load, which changes which
queries time out.

So the two kinds of budget are separated:

- **Deterministic budgets** gate results. Solver effort is bounded by z3's `:rlimit`
  (deterministic work units) rather than `:timeout` (seconds) — this is exactly why the
  distinction exists in z3, and it is what makes `Unknown(ResourceLimit)` reproducible.
- **`wall_clock` is a non-deterministic abort.** Hitting it terminates the run and is
  reported as `BudgetHit::WallClock`, but results produced under it are excluded from the
  determinism contracts, and the run is marked `nondeterministic_abort: true`. CI runs the
  determinism gates with `wall_clock: None`.

Without this split, contract 6 and contract 17 are not merely hard, they are false.

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

/// Produced by the engine. `chiero-check` *enriches* this into a `Report::Finding`
/// (040 §2) with severity, confidence, replay verdict and narrative — it does not
/// define a second, incompatible type.
pub struct Finding { pub kind: FindingKind, pub span: Span, pub backtrace: Vec<ExpnFrame>,
                     pub trace: PathTrace, pub witness: Option<Witness>,
                     pub object: Option<ObjOrigin>, pub fidelity: Fidelity }
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
   `Fidelity == Unknown` (per the §7 table, not merely `Approximated`) and exactly one
   `Assumption` naming the cause.
5. A loop `while (i < n) i++` with `max_loop_iters = 3` terminates, produces 4 states, and
   the run's fidelity is `Bounded` with a `BudgetHit` naming the back edge.
6. With `wall_clock: None`, the same program run twice produces identical `StateId`
   sequences, identical fork order, identical findings, and byte-identical output —
   including under `RandomPath` with the default seed. (With a wall clock set, this is not
   required and the run is flagged; see §8.1.)
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
    per state. The test must include a run that *does* degrade (a always-`Unknown`
    implementation passes monotonicity trivially).
12. Every state whose fidelity is worse than `Exact` has ≥ 1 `Assumption` **whose `kind`
    matches the recorded cause and whose text appears in the rendered report** — a dummy
    assumption must not satisfy this.
13a. **Compile-fail** (`trybuild`): `ExactWitness` cannot be constructed outside
     `chiero-exec`, and cannot be `Clone`d.
13b. **Runtime**: `seal` returns `NotProven` for `Bounded`, `Approximated` and `Unknown`
     results, and `Proven` only for `Exact` — property-tested across all four levels; and
     an `ExactWitness` minted by run A is rejected against run B even when both are
     `Exact`.
14. `no bugs found` under a hit budget renders as "no bugs found within <bound>" and never
    as "no bugs exist" (golden test on the rendered text).
15. Every `Finding` from the corpus has a `Witness`, or an explicit
    `witness: None` with a recorded reason.
16. Replaying a `Witness` through the engine with all inputs concretized re-reaches the
    same `Finding` at the same `Span` — for every finding in the corpus.
17. With `wall_clock: None`, running with 1, 2 and 8 worker threads produces identical
    `RunResult`s (modulo wall-clock timings in `stats`). Per-state `CheckerState` is what
    makes this achievable; a checker holding cross-state mutable state would break it.
18. Exceeding `max_states` terminates the run cleanly with `Bounded`, reporting the
    findings already collected — no data loss, no panic.
19. A checker returning `Action::Assume` constrains the state and the subsequent branch
    feasibility reflects it.
20. Two checkers reporting the same event produce two findings at the engine level (the
    engine does not deduplicate).
21. **Per witness replay, at line granularity**: for a single witness replayed with all
    inputs concretized, the `gcov_lines` of the CIR blocks chiero executed equal the lines
    gcov reports for the gcc-compiled program on the same inputs. Stated at *block*
    granularity across a whole symbolic run it would be false twice over — a symbolic run
    covers the union over many paths, and chiero's CFG does not correspond block-to-block
    with gcc's post-gimplification CFG.
22. A stateful checker's `CheckerState` is cloned on fork: a lock acquired before a fork
    is held in both children, and released in one leaves it held in the other.
23. A `Value::Ptr` survives being stored to a local, loaded back, and passed to a model:
    `free(p)` after such a round trip identifies the same `ObjectId`, and the pointer
    reaching a `Checker` through `Event::Call { args }` carries the same `ObjectId`.
24. `CallReturn` fires in the caller for all three callee kinds — a defined function, a
    modeled extern, and an **unmodeled** extern whose fresh return value has no return
    instruction — and carries the value the caller will observe.
