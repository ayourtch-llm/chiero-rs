# 022 — Solver

`chiero-solver` decides satisfiability of path conditions. It **knows nothing about C and
nothing about CIR** — its vocabulary is sorts and terms. That isolation is deliberate: it
makes the crate independently useful, makes its test suite pure constraint solving, and
prevents C semantics from leaking into a layer that must be trustworthy.

Environment as verified on 2026-07-26: **gcc 13.3.0**, **clang 18.1.3**, **z3 4.8.12**
(`/usr/bin/z3`, SMT-LIB2 over stdin confirmed working), `libz3-dev` present. The
[hard constraint](000-overview.md) is unchanged: chiero **never links** z3, builds and
runs with `--no-default-features`, and treats every external solver as a subprocess or an
opt-in feature. z3 is an accelerator and a cross-check oracle, never a dependency.

## 1. Sorts and terms

```rust
pub enum Sort {
    Bool,
    BitVec(u32),
    Array { index: u32, elem: u32 },     // BV(index) -> BV(elem); elem is 8 for memory
    Float(FloatSort),                    // v2; declared now so the enum is stable
}

pub struct Term(u32);                    // index into TermArena; hash-consed
```

Terms are a **hash-consed DAG** in a per-context `TermArena`. Structural equality is
`Term` equality, which makes the caches in §6 cheap and makes `x + 0 == x` a pointer
comparison after folding. Terms are immutable and `Copy`.

```rust
pub enum Op {
    Const(BvConst) | BoolConst(bool) | Var(VarId),
    // bitvector
    Add, Sub, Mul, UDiv, SDiv, URem, SRem, And, Or, Xor, Not, Neg,
    Shl, LShr, AShr, Concat, Extract { hi: u32, lo: u32 }, ZeroExt(u32), SignExt(u32),
    // predicates
    Eq, Ne, Ult, Ule, Slt, Sle,
    // boolean
    LAnd, LOr, LNot, Implies, Ite,
    // arrays
    Select, Store,
}
```

Division and remainder are **total**, so terms never carry partiality. Undefined behaviour
is detected and reported at the CIR level ([020 §4.1](020-cir.md)), not encoded as solver
partiality — a solver that can return "no value" poisons every downstream cache.

Totality means matching SMT-LIB exactly, and SMT-LIB's zero cases are **not** uniform.
Verified against z3 4.8.12 on `BitVec(8)`:

| Term | Result | Rule |
|---|---|---|
| `bvudiv x 0` | `#xff` | all ones |
| `bvsdiv 5 0` | `#xff` | `-1` when `x ≥s 0` |
| `bvsdiv -5 0` | `#x01` | **`1` when `x <s 0`** |
| `bvurem x 0` | `x` | **the dividend, not all-ones** |
| `bvsrem x 0` | `x` | **the dividend, not all-ones** |

Getting these wrong is uniquely dangerous in this architecture: the independent evaluator
(§3) and the constant folder would share the error, so the evaluator would happily
*validate* a model built on wrong semantics. The mistake would then be invisible to model
validation and would surface only as a tier-1/tier-2 disagreement — which requires z3 to
be installed. Contracts 19a–19d pin each case separately.

Floats are deferred to v2. Until then, float operations produce a fresh unconstrained
`BitVec` of the right width plus `Fidelity::Approximated`. VPP's data plane is
overwhelmingly integer; buying float support with unsoundness elsewhere is a bad trade.

## 2. The trait

```rust
pub trait Solver {
    fn declare(&mut self, sort: Sort, name: &str) -> VarId;
    fn assert(&mut self, t: Term);
    fn push(&mut self);
    fn pop(&mut self, n: u32);
    fn check(&mut self, assumptions: &[Term]) -> CheckResult;
    fn stats(&self) -> SolverStats;
}

pub enum CheckResult { Sat(Model), Unsat, Unknown(UnknownReason) }

pub enum UnknownReason { Timeout, ResourceLimit, Incomplete(&'static str), BackendError(String) }

pub struct Model { values: IndexMap<VarId, BvConst>, arrays: IndexMap<VarId, ArrayModel> }
```

**Three-valued, always.** `Unknown` is a first-class answer that propagates into
`Fidelity` ([023 §7](023-execution-engine.md)). Any code that pattern-matches
`Sat`/`Unsat` and treats the remainder as one of them is a bug; the enum is
`#[non_exhaustive]`-free precisely so that the compiler forces the third arm.

**Models are complete and canonical.** Every declared variable appearing in the asserted
constraints has a value (unconstrained variables get 0), and iteration order is by
`VarId`. Two runs producing different counterexamples for the same query would break
golden tests and make findings unreproducible.

## 3. Tier 1 — `solver-lite`

Built in, pure Rust, always available, deliberately incomplete. It resolves the large
majority of real path conditions, which are shallow: `i < n`, `p != NULL`,
`(flags & 4) == 0`, `len - 1 >= 0`.

Three cooperating layers:

1. **Rewriting / constant folding** on term construction — normalization (commutative
   operand ordering by `Term` id, constants to the right), identity and annihilator laws,
   `Extract` of `Concat`, double negation, comparison of constants, `Ite` with a constant
   condition. Applied at hash-cons time, so it is not a pass but an invariant.
2. **Abstract interpretation** over a product domain per variable:
   `{ interval: (signed lo, hi), unsigned interval, known-bits: (zeros mask, ones mask) }`.
   Constraints are propagated to fixpoint (bounded iteration count). If any variable's
   domain becomes empty → **`Unsat`**. If every asserted predicate evaluates to true on
   the domain's witness → candidate `Sat`.
3. **Congruence closure** over uninterpreted structure (`Select`/`Store` chains, equalities
   between opaque terms) to discharge `a == b ∧ f(a) != f(b)`.

**Two hard rules make tier 1 safe to trust:**

- Every `Sat` is returned **only with a model that has been concretely evaluated against
  every asserted constraint** by an independent evaluator (§3.1). A model that fails
  evaluation is a bug: in debug builds it panics, in release it degrades to `Unknown`.
- `Unsat` is returned only from an emptiness proof in layer 2 or a closure contradiction
  in layer 3, **and only over the fragment in §3.2** — never from "I gave up". Giving up
  is `Unknown(Incomplete(reason))`.

These two are **not symmetric**, and the asymmetry is the crux of the design. `Sat` is
self-certifying: a concrete total assignment that evaluates every assertion to true is a
satisfying assignment no matter how it was found, so an incomplete or even buggy search
cannot produce a wrong `Sat`. `Unsat` carries no such witness — nothing checks it — so it
must instead be constrained *syntactically*, in advance, to cases where the reasoning is
known sound. §3.2 does that; without it, "tier 1 can be incomplete without being wrong"
is true of `Sat` and merely hoped for of `Unsat`.

### 3.1 The independent evaluator

`eval(model, term) -> BvConst` is written **from the SMT-LIB standard, in its own module,
sharing no code with the constant folder in layer 1**. If the two share an `eval_op`, a
wrong operator semantics (the zero cases in §1 being the obvious candidate) is applied
identically on both sides and validation certifies it. "Independent" is a structural
requirement, not an adjective.

The evaluator requires a **total** model. `ArrayModel` is therefore a finite map plus a
default, never a partial one:

```rust
pub struct ArrayModel { pub entries: IndexMap<BvConst, BvConst>, pub default: BvConst }
```

With a total model and a correct evaluator, evaluation establishes `Sat` even when the
array decision procedure is incomplete — which is exactly why this is the mechanism that
makes tier 1's incompleteness safe.

### 3.2 The fragment over which layer 2 may answer `Unsat`

Layer 2 may return `Unsat` **only** when every asserted term is a conjunction of atoms of
the form `cmp(a, b)` or `eq/ne(a, b)` over the bitvector operators in §1. Any `LOr`,
`Implies`, `Ite`-valued predicate, or nested disjunction anywhere in the assertion set
puts the query outside the fragment, and the answer is `Unknown(Incomplete("disjunction"))`.

This restriction is not conservatism for its own sake. `assert` takes an arbitrary `Term`
and [023 §6](023-execution-engine.md)'s `Action::Assume` lets a checker assert anything,
so disjunctions do reach the solver. A propagator that descends into `LOr` and applies
both branches reports `Unsat` for `(x <u 5 ∨ x >u 200) ∧ x <u 3`, which is satisfiable.

**Every transfer function must be wrap-safe.** CIR arithmetic is modular
([020 §4.1](020-cir.md)), so an interval transfer that saturates instead of wrapping is
unsound. Verified counterexample, `BitVec(8)`:

```
x >u 250  ∧  y == x + 10  ∧  y <u 10        z3: sat, x = 0xfb, y = 0x05
```

A non-wrapping propagator computes `x ∈ [251,255]`, `x+10 ∈ [261,265]`, saturates to
`[255,255]`, intersects with `[0,9]`, finds ∅, and reports a **false `Unsat`** — pruning
a real path and, downstream, licensing a "no bug exists" claim. Any transfer that cannot
represent a wrapped result must widen to ⊤ rather than saturate. The interval⊓known-bits
reduction is subject to the same obligation in both directions.

Contract 7b (exhaustive enumeration over small widths) is what actually holds this rule
down, and unlike the z3 differential campaign it needs no external solver.

## 4. Tier 2 — `solver-smtlib` (subprocess)

Feature `smtlib-subprocess`, **on by default when the binary is found at runtime, absent
otherwise** — discovery is runtime, the dependency is not a link-time one.

- Spawns `z3 -in -smt2` (or `cvc5 --incremental --lang smt2`, or `bitwuzla`) and speaks
  SMT-LIB2 over stdin/stdout, keeping the process **alive across queries** so `push`/`pop`
  map to real incremental solving. Process startup dominates short queries; a per-query
  process would make tier 2 useless.
- Logic: `QF_ABV` (`QF_BV` when no array is declared — measurably faster).
- Timeouts via `(set-option :timeout N)` plus a wall-clock watchdog; on watchdog fire the
  process is killed, restarted, the assertion stack is **replayed**, and the query returns
  `Unknown(Timeout)`. Replay correctness is contract 14.
- Any parse error or unexpected reply is `Unknown(BackendError)` and increments a counter
  that surfaces in `stats()`. The engine must never crash because a solver misbehaved.
- The emitted SMT-LIB2 is a **first-class artifact**: `--dump-queries <dir>` writes every
  query, which is how a solver disagreement gets reported upstream and how chiero's own
  bugs get bisected.

Backend selection order: `$CHIERO_SMT_SOLVER`, then `z3`, `cvc5`, `bitwuzla` on `PATH`.
Recorded in the result so a finding says which solver decided it.

## 5. Tier 3 — `z3-sys`

Off-by-default feature `z3-link`, implemented after tiers 1–2 are proven. Same trait, no
subprocess overhead, at the cost of a build-time dependency. **The workspace's default
build must never require it**, and CI builds `--no-default-features` to prove that
(contract 1).

## 6. `TieredSolver` and caching

```rust
pub struct TieredSolver { lite: Lite, heavy: Option<Box<dyn Solver>>, cache: Caches, cfg: TierCfg }
```

Escalation: run tier 1; on `Sat`/`Unsat` return it; on `Unknown` escalate to tier 2 if
available, else return `Unknown`. `TierCfg::paranoid` additionally sends **every** tier-1
answer to tier 2 and asserts agreement — this is the cross-validation harness (§7), too
slow for production and mandatory in CI.

### 6.1 The satisfiability invariant that slicing depends on

Independence slicing — "solve only the component containing the query's variables" — is
equisatisfiable **only if every other component is already known satisfiable**. KLEE gets
this for free: a constraint enters its path condition only after being checked feasible.

**chiero deliberately violates that in three places**, and each one breaks slicing unless
handled:

- [023 §3](023-execution-engine.md): on solver `Unknown`, the engine *takes the branch
  anyway* and adds the constraint — unproven.
- [021 §5](021-memory-model.md): after an out-of-bounds access, execution continues on the
  in-bounds branch with that constraint added — infeasible when the access is
  unconditionally OOB.
- [024 §4](024-environment-models.md): `strlen`'s cap constrains a terminator to exist
  within the bound — infeasible if those bytes are already constrained non-zero.

If a component is quietly `Unsat` and the query touches a different component, slicing
answers `Sat`, the engine forks, and every finding on that dead path is reported with a
witness that does not satisfy the path condition — breaking
[023](023-execution-engine.md) contract 16, and firing 023 §3's "both branches
infeasible → this is a chiero bug" assertion on legitimate runs.

So the path condition carries the flag:

```rust
pub struct PathCondition { terms: Vec<Term>, possibly_infeasible: bool }
```

Set whenever a constraint is added without a feasibility check. While it is set,
**slicing and the subset/superset cache rules are disabled** for that state and queries go
to the full assertion set. A single full check that returns `Sat` clears it.

### 6.2 The caches

Three caches, in lookup order:

1. **Exact cache** — keyed on the **pair** `(sorted assertion Term ids, sorted assumption
   Term ids)`, stored and compared in full. Two mistakes to avoid, both silent: omitting
   `assumptions` makes `check([c])` and `check([¬c])` collide on the same assertion stack
   and return each other's answers; and keying on a *hash value* rather than the ids means
   a collision returns a wrong answer with no detection. The hash is an index into the
   table, never the identity.
2. **Constraint independence slicing** — partition the assertion set into connected
   components by shared variables, and solve only the component(s) containing the query's
   variables, subject to §6.1. A path condition of 200 constraints usually decomposes into
   many small independent ones; this is the single largest measured win in KLEE and is
   specified here as required, not optional.
3. **Counterexample cache** (KLEE-style, per slice):
   - a cached model that **satisfies** a new query's constraints answers `Sat` with no
     solver call;
   - a **superset** of a known-`Unsat` set is `Unsat`;
   - a **subset** of a known-`Sat` set is `Sat`.

   Each rule is a one-line justification and each is independently tested (contracts
   9–11), because a wrong subset/superset direction is a silent, catastrophic bug.

Subset/superset lookup over thousands of cached sets needs a **subsumption index** — a
bitset-per-set signature plus an inverted index from `Term` id to the sets containing it,
giving candidate sets in time proportional to the query's size rather than the cache's.
Contracts 10–11 are tested at ≥1000 cached entries, because at 1 entry they pass against
an implementation that remembers only the last query.

**`Unknown` is never stored in the exact cache.** `Unknown(Timeout)` is a fact about the
wall clock at one moment, not about the formula, and sibling states share long path-
condition prefixes *by design* ([023 §1](023-execution-engine.md)) — so caching one
unlucky timeout would permanently degrade an entire subtree. Caching it would also defeat
escalation: a tier-1 `Unknown` cached above `TieredSolver` means tier 2 is never consulted
for any sibling, silently disabling the mechanism §4 exists for.

The caches sit **inside** `TieredSolver`, below escalation: a lookup can only return a
definite answer, and a miss runs the full tier-1-then-tier-2 ladder.

A cache hit is **indistinguishable from a fresh answer**, including its effect on
`Fidelity`. A cached result that degraded a state when first computed degrades it
identically on every hit.

Cache keys are computed from hash-consed `Term` ids, so they are structural, not textual.
Caches are per-`TermArena`. Because a run may hold 10 000 live states, the counterexample
cache is bounded by a documented entry count with LRU eviction — it is a known memory hog
in KLEE, and "cleared with the arena" is not a policy.

## 7. Validation

The solver is the component where a bug produces confident wrong answers everywhere, so
it gets the heaviest validation in the project:

- **Differential against z3** — a generator produces random well-sorted `QF_ABV` terms
  (bounded depth/width) and every query goes to both tiers. Any disagreement on a
  definite answer is a test failure with the term dumped as SMT-LIB2. Tier 1 answering
  `Unknown` where z3 answers definitely is expected and merely counted (the count is a
  tracked quality metric, and a regression in it fails CI).
- **Model validation** — every `Sat` model from either tier is evaluated against the
  constraints by an independent evaluator (§3). This catches both tier-1 domain bugs and
  SMT-LIB2 serialization bugs, and it is on in release too, because the cost is
  negligible next to solving.
- **Cache validation** — `paranoid` mode also verifies every cache hit against a real
  solve.

## 8. Testable contracts

1. **Neither the default nor the minimal build links a solver**: `cargo tree -p
   chiero-solver` and `cargo tree -p chiero-solver --no-default-features` both contain no
   `z3`/`z3-sys` node, and neither `.rlib` contains z3 symbols. (Testing only
   `--no-default-features` would pass trivially; the risk is the *default* build.)
2. With `z3` absent from `PATH`, the whole test suite still runs; tier-2-only tests are
   skipped with a printed reason, not silently passed.
3. `x < 5 ∧ x > 2` over `BitVec(32)` is `Sat`, and the returned model satisfies both
   constraints under independent evaluation.
4. `x < 5 ∧ x > 5` (unsigned) is `Unsat` from tier 1 alone, with no subprocess spawned.
5. `x & 0xF0 == 0x0F` is `Unsat` from tier 1's known-bits domain.
6. `(x * y) == 7 ∧ x > 1 ∧ y > 1` over `BitVec(32)` is `Unknown` from tier 1 and `Sat`
   from tier 2 — escalation demonstrably happens and demonstrably matters.
7. Tier 1 never returns `Sat` with a model that fails independent evaluation (property
   test, 100 000 random constraint sets).
7b. **Exhaustive `Unsat` validation, no external solver required**: over random constraint
    sets on `BitVec(4)` and `BitVec(8)`, whenever tier 1 answers `Unsat`, brute-force
    enumeration of all assignments confirms none satisfies. This is the contract that
    closes the Sat/Unsat asymmetry (§3), and unlike contracts 13/18 it runs when z3 is
    absent. The wrap-around case `x >u 250 ∧ y == x+10 ∧ y <u 10` (satisfiable at
    `x=0xfb`) and the disjunction case `(x <u 5 ∨ x >u 200) ∧ x <u 3` are both in the
    seed corpus.
7c. Every assertion set containing a disjunction yields `Unknown(Incomplete)` from tier 1,
    never `Unsat` (§3.2).
7d. The independent evaluator shares no symbols with the constant folder — checked
    mechanically — and is differentially tested against z3's `simplify` on random ground
    terms.
8. The same query answered five ways — via tier 1, via tier 2, from a cold cache, from a
   warm cache, and with slicing disabled — returns byte-identical models. (Testing only
   "two runs agree" is vacuous: the second run hits the exact cache and never calls a
   backend, so a constant-model implementation passes.)
9. Independence slicing sends only the relevant component to the backend **and returns the
   same answer as the unsliced query**, over the whole corpus. Verifying only that the
   dumped query got smaller tests that slicing happened, not that it was correct.
9b. A path condition with `possibly_infeasible` set disables slicing and the
    subset/superset rules; a state whose path condition contains an unsatisfiable
    component reports `Unsat` for every subsequent feasibility query, with slicing on and
    off (§6.1).
10. Counterexample cache: after `Sat` on `S`, a query on a subset of `S` returns `Sat`
    with zero backend calls. Tested with ≥1000 cached entries.
11. Counterexample cache: after `Unsat` on `S`, a query on a superset of `S` returns
    `Unsat` with zero backend calls; a query on a *subset* does **not** hit the cache.
    Tested with ≥1000 cached entries.
11b. `check` distinguishes assumptions: on one assertion stack, `check([c])` and
     `check([¬c])` return different answers and neither serves the other's cached result.
11c. An `Unknown` result is never returned from the exact cache, and a tier-1 `Unknown` on
     query `Q` does not prevent tier 2 from being consulted for `Q` or any superset of it.
11d. A cache hit degrades the consuming state's fidelity identically to a fresh answer.
12. Cached model reuse: a new query whose constraints are all satisfied by a cached model
    returns `Sat` with zero backend calls.
13. `paranoid` mode over the full corpus reports zero tier-1/tier-2 disagreements.
14. Killing the subprocess mid-query yields `Unknown(Timeout)`, restarts it, replays the
    assertion stack, and the next query returns the same answer as an unrestarted solver.
15. A backend emitting garbage yields `Unknown(BackendError)` and increments the error
    counter; no panic, no state corruption.
16. `push`/`pop` restore the exact assertion set: `assert(a); push(); assert(b); pop(1)`
    leaves a context where `¬a` is `Unsat` and `¬b` is `Sat`.
17. `--dump-queries` output is valid SMT-LIB2 accepted by z3, and re-running it
    standalone reproduces chiero's answer.
18. Random differential campaign of 10 000 terms: zero definite-answer disagreements with
    z3; the tier-1 `Unknown` rate is recorded and does not regress by more than 2 points.
19a. `bvudiv x 0` is all ones, for every `x` — the folder and the independent evaluator
     agree, and both agree with z3 when it is installed.
19b. `bvsdiv x 0` is all ones for `x >=s 0` and **`1`** for `x <s 0`. One rule for both
     signs is wrong for half the inputs.
19c. `bvurem x 0` and `bvsrem x 0` are **the dividend**, not all-ones.
19d. None of the above produces a solver error, and no term carries partiality (§2):
     division is total, and UB is 020 §4.1's business, not the solver's.

*(An earlier draft numbered these as one contract reading "`x / 0` evaluates to all-ones",
which §2's own table contradicts in three of the four cases — the table was corrected
against z3 4.8.12 and the contract list was not. §2 already promised "contracts 19a–19d
pin each case separately"; this is that promise kept. The uniform rule is uniquely
dangerous here because the folder and the evaluator would share the error, so model
validation would confirm it.)*
20. Solving the same path condition twice makes exactly one backend call (exact cache).
