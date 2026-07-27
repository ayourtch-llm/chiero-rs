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

Division and remainder are **total**: `x / 0` is defined (all-ones, matching SMT-LIB
`bvudiv`) so terms never carry partiality. Undefined behaviour is detected and reported
at the CIR level ([020 §4.1](020-cir.md)), not encoded as solver partiality — a solver
that can return "no value" poisons every downstream cache.

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
  every asserted constraint** by an independent evaluator. A model that fails evaluation
  is a bug, and in debug builds it panics; in release it degrades to `Unknown`.
- `Unsat` is returned only from an emptiness proof in layer 2 or a closure contradiction
  in layer 3 — never from "I gave up". Giving up is `Unknown(Incomplete(reason))`.

With those, tier 1 can be incomplete without being wrong, and `TieredSolver` can escalate
on `Unknown` without ever second-guessing a definite answer.

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

Three caches, in lookup order:

1. **Exact cache** — `hash(sorted assertion set) -> CheckResult`. Cheap, high hit rate
   because sibling states share long prefixes of the path condition.
2. **Constraint independence slicing** — partition the assertion set into connected
   components by shared variables, and solve only the component(s) containing the query's
   variables. A path condition of 200 constraints usually decomposes into many small
   independent ones; this is the single largest measured win in KLEE and is specified
   here as required, not optional.
3. **Counterexample cache** (KLEE-style, per slice):
   - a cached model that **satisfies** a new query's constraints answers `Sat` with no
     solver call;
   - a **superset** of a known-`Unsat` set is `Unsat`;
   - a **subset** of a known-`Sat` set is `Sat`.

   Each rule is a one-line justification and each is independently tested (contracts
   9–11), because a wrong subset/superset direction is a silent, catastrophic bug.

Cache keys are computed from hash-consed `Term` ids, so they are structural, not textual.
Caches are per-`TermArena` and cleared with it.

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

1. `cargo build -p chiero-solver --no-default-features` succeeds and the resulting
   `.rlib` links no external solver (checked by absence of z3 symbols).
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
8. Two runs of the same query return byte-identical models.
9. Independence slicing: a 200-constraint set with 20 independent components sends only
   the relevant component to the backend, verified by inspecting the dumped query.
10. Counterexample cache: after `Sat` on `S`, a query on a subset of `S` returns `Sat`
    with zero backend calls.
11. Counterexample cache: after `Unsat` on `S`, a query on a superset of `S` returns
    `Unsat` with zero backend calls; a query on a *subset* does **not** hit the cache.
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
19. `x / 0` evaluates to all-ones and produces no solver error (totality).
20. Solving the same path condition twice makes exactly one backend call (exact cache).
