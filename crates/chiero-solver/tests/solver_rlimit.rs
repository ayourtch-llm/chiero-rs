//! 023 §8's `max_solver_rlimit` — the **deterministic** bound on solver effort.
//!
//! Covers: 023 §8, §8.1.
//!
//! §8.1 is the reason this exists rather than a second timeout: "a wall-clock timeout is not
//! reproducible: it changes `Fidelity`, `assumptions` and `budget_hits`, all of which are
//! output", and contract 17 asks for identical results at 1, 2 and 8 worker threads, which a
//! clock cannot give. z3's `:rlimit` counts *work units*, so the same query cut at the same
//! budget is cut at the same place on every machine and at every thread count.
//!
//! §8 also records the residue this closes: "the clock is checked *between* steps, so a single
//! long solver query outlives it… three entry points of 477 still had to be killed from
//! outside for exactly this reason".
//!
//! ## What the real solver does, measured on z3 4.8.12 before any of this was written
//!
//! | asked | answered |
//! |---|---|
//! | `:rlimit 2000` on a hard `bvmul` | `unknown` |
//! | the same at `:rlimit 100000000` | `sat` — the bound cut it, not the formula |
//! | a hard query then a trivial one, **one process** | `unknown` then `sat` — the budget is per `check-sat`, **not cumulative** |
//!
//! That last row is what makes the option usable at all: chiero keeps one long-lived z3, and a
//! cumulative budget would poison every query after the first expensive one.
//!
//! ## ⚠️ The trap, which is why the reason string is read the way it is
//!
//! `(:reason-unknown "max. resource limit exceeded")` appears only with the assertion stack at
//! **top level**. Inside `(push)`/`(pop)` — which is how `query` always drives the backend —
//! the same exhaustion says **`"canceled"`**, and a `:timeout` firing says `"canceled"` too,
//! byte for byte. So the string cannot say *which* limit fired, and an implementation matching
//! the documented sentence passes a hand-written script and misclassifies every real query.
//!
//! What the string *can* do is separate a limit from z3 giving up on the theory, which reads
//! `"smt tactic failed to show goal to be sat/unsat (incomplete (theory arithmetic))"`. So:
//! `"canceled"` means *the armed limit*, and only one limit is ever armed.

use chiero_solver::*;

fn z3() -> Option<SmtLib> {
    SmtLib::discover()
}

/// A query tier 1 cannot touch and z3 needs real work for: 022 contract 6's own example.
fn hard(a: &mut TermArena) -> Vec<Term> {
    let x = a.var(Sort::BitVec(32), "x");
    let y = a.var(Sort::BitVec(32), "y");
    let seven = a.bv(32, 7);
    let one = a.bv(32, 1);
    let prod = a.mul(x, y);
    // `1 <u x` rather than `x >u 1`: the arena builds one direction and the other is the
    // same term with the operands swapped.
    vec![a.eq(prod, seven), a.ult(one, x), a.ult(one, y)]
}

/// **The bound cuts the query, and says so by name.**
///
/// The negative half is the whole test: a `ResourceLimit` that fired because the formula was
/// unsatisfiable, or because the process died, would be indistinguishable from one that
/// fired because the budget ran out. So the *same* query with a large budget must answer
/// `Sat` — that is what makes the small-budget `Unknown` a fact about the budget.
#[test]
fn a_small_rlimit_cuts_a_hard_query_and_a_large_one_does_not() {
    let Some(backend) = z3() else {
        eprintln!("skipped: no SMT backend on PATH");
        return;
    };

    let mut a = TermArena::new();
    let q = hard(&mut a);

    let mut small = TieredSolver::with_backend(backend.clone()).with_rlimit(2_000);
    for t in &q {
        small.assert(*t);
    }
    let cut = small.check(&mut a, &[]);
    assert!(
        matches!(cut, CheckResult::Unknown(UnknownReason::ResourceLimit)),
        "the budget is what stopped it, and the reason has to say so: {cut:?}"
    );

    let mut large = TieredSolver::with_backend(backend).with_rlimit(1_000_000_000);
    for t in &q {
        large.assert(*t);
    }
    let solved = large.check(&mut a, &[]);
    assert!(
        matches!(solved, CheckResult::Sat(_)),
        "the same query is decidable — so the cut above was the budget, not the formula: \
         {solved:?}"
    );
}

/// **The budget is per query, not per process.**
///
/// chiero keeps one long-lived backend (022 §4), so a cumulative budget would mean the first
/// expensive query poisons every later one — every subsequent answer `Unknown`, every consumer
/// degrading honestly, and a run that decided nothing looking like a run over a hard program.
/// Measured on z3 before the option was written, and pinned here because it is a property of
/// the *backend* that this design rests on: if a future z3 made `:rlimit` cumulative, this
/// test is what would notice.
#[test]
fn an_exhausted_query_does_not_poison_the_next_one() {
    let Some(backend) = z3() else {
        eprintln!("skipped: no SMT backend on PATH");
        return;
    };
    let mut a = TermArena::new();
    let mut s = TieredSolver::with_backend(backend).with_rlimit(2_000);

    let q = hard(&mut a);
    s.push();
    for t in &q {
        s.assert(*t);
    }
    let first = s.check(&mut a, &[]);
    assert!(
        matches!(first, CheckResult::Unknown(UnknownReason::ResourceLimit)),
        "{first:?}"
    );
    s.pop(1);

    // Trivial, and in the same process.
    let z = a.var(Sort::BitVec(8), "z");
    let five = a.bv(8, 5);
    let easy = a.ult(z, five);
    s.assert(easy);
    let second = s.check(&mut a, &[]);
    assert!(
        matches!(second, CheckResult::Sat(_)),
        "the exhausted query must not have spent the next one's budget: {second:?}"
    );
    assert_eq!(
        s.stats().backend_spawns,
        1,
        "and it has to be the same process, or this proves nothing"
    );
}

/// **An `unknown` is an answer, not a dead process** — and it was being treated as one.
///
/// `query` returned `Option<(bool, Model)>`, so `unknown` and "the pipe broke" were both
/// `None`. `ask_backend_raw` reads `None` as a died-mid-query and **replays the entire
/// query**, then reports `BackendError("backend gave no usable answer")`. Two consequences,
/// both wrong and both invisible:
///
/// - the hardest queries in a run — the only ones that answer `unknown` — are the ones charged
///   **twice**, which is the exact opposite of what a budget is for;
/// - `backend_errors` counts them, and 022 contract 15 exists so that a misbehaving backend is
///   visible in that number. A solver honestly saying "I do not know" is not misbehaving, and
///   inflating the count hides one that is.
///
/// Found while implementing the rlimit, which is how it had survived: nothing had ever needed
/// to tell z3's two kinds of silence apart.
#[test]
fn an_unknown_answer_is_not_charged_as_a_backend_error() {
    let Some(backend) = z3() else {
        eprintln!("skipped: no SMT backend on PATH");
        return;
    };
    let mut a = TermArena::new();
    let mut s = TieredSolver::with_backend(backend).with_rlimit(2_000);
    let q = hard(&mut a);
    for t in &q {
        s.assert(*t);
    }
    let r = s.check(&mut a, &[]);
    assert!(
        matches!(r, CheckResult::Unknown(UnknownReason::ResourceLimit)),
        "{r:?}"
    );
    assert_eq!(
        s.stats().backend_errors,
        0,
        "a solver saying it does not know is not a solver misbehaving"
    );
    assert_eq!(
        s.stats().backend_calls,
        1,
        "and the hardest query in the run must not be the one that is asked twice"
    );
}
