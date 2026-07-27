//! **The counterexample cache** — 022 §6 and contracts 10, 11, 12.
//!
//! Covers: 022 contracts 10, 11, 12.
//!
//! Three rules, each one line of justification and each independently tested "because a
//! wrong subset/superset direction is a silent, catastrophic bug":
//!
//! - a cached model that **satisfies** a new query answers `Sat` with no solver call;
//! - a **superset** of a known-`Unsat` set is `Unsat`;
//! - a **subset** of a known-`Sat` set is `Sat`.
//!
//! All of them are tested at **≥1000 cached entries**, as §6 requires in as many words:
//! "at 1 entry they pass against an implementation that remembers only the last query".
//!
//! ⚠️ Two traps this file exists to avoid. Every constraint here is **nonlinear**, so
//! tier 1 returns `Unknown` and a backend call is what a cache miss looks like — over
//! linear constraints tier 1 answers everything and `backend_calls` stays at zero whether
//! the cache works or not. And terms are *reused* rather than rebuilt: `TermArena::var`
//! mints a fresh `VarId` per call, so two `var(BitVec(32), "x0")` are two variables and a
//! "contradiction" built that way is satisfiable.

use chiero_solver::*;

/// `x_i * y_i == 7` for `i` in `0..n` — nonlinear, so tier 1 gives up and the backend is
/// consulted, and independent, so any subset is satisfiable.
fn hard_constraints(a: &mut TermArena, n: usize) -> (Vec<Term>, Vec<Term>) {
    let mut cs = Vec::new();
    let mut prods = Vec::new();
    for i in 0..n {
        let x = a.var(Sort::BitVec(32), &format!("x{i}"));
        let y = a.var(Sort::BitVec(32), &format!("y{i}"));
        let p = a.mul(x, y);
        let c = a.bv(32, 7);
        cs.push(a.eq(p, c));
        // Handed back so a test can build a *new* constraint over the same variables.
        // Calling `var` again with the same name would mint a different one.
        prods.push(p);
    }
    (cs, prods)
}

/// Skips when no backend is installed: without one every query is `Unknown` and the cache
/// has nothing to remember (022 contract 2 requires the suite to run with z3 absent).
fn solver() -> Option<TieredSolver> {
    Some(TieredSolver::with_backend(SmtLib::discover()?))
}

/// A thousand distinct entries, so no rule below can pass against a memory of one query.
///
/// The filler constraints are **linear** on purpose: tier 1 decides them, so filling the
/// cache costs no backend calls and the file runs in a second rather than a minute. What
/// they have to be is *many and distinct*; what the measured queries have to be is hard.
fn fill(s: &mut TieredSolver, a: &mut TermArena) {
    for i in 0..1000 {
        let v = a.var(Sort::BitVec(32), &format!("f{i}"));
        let c = a.bv(32, i as u128);
        let e = a.eq(v, c);
        let _ = s.check(a, &[e]);
    }
    assert!(
        s.stats().cache_entries >= 1000,
        "the fixture must actually fill the cache: {}",
        s.stats().cache_entries
    );
}

/// **022 contract 10.** After `Sat` on `S`, a query on a subset of `S` is `Sat` with zero
/// backend calls: the model that satisfied `S` satisfies every subset of it.
#[test]
fn a_subset_of_a_satisfiable_set_is_satisfiable_with_no_backend_call() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, _prods) = hard_constraints(&mut a, 64);
    fill(&mut s, &mut a);
    let big: Vec<Term> = cs.clone();
    assert!(matches!(s.check(&mut a, &big), CheckResult::Sat(_)));

    let before = s.stats().backend_calls;
    for lo in [0usize, 7, 31] {
        let sub = &big[lo..lo + 8];
        assert!(
            matches!(s.check(&mut a, sub), CheckResult::Sat(_)),
            "a subset of a satisfiable set is satisfiable"
        );
    }
    assert_eq!(
        s.stats().backend_calls,
        before,
        "and answering it needs no backend"
    );
}

/// **022 contract 11.** After `Unsat` on `S`, a **superset** of `S` is `Unsat` with zero
/// backend calls — adding constraints cannot rescue a contradiction. And a *subset* must
/// **not** hit: dropping the constraint that caused it can leave a perfectly satisfiable
/// set, and answering `Unsat` there reports bugs that do not exist.
#[test]
fn a_superset_of_an_unsatisfiable_set_is_unsatisfiable_but_a_subset_is_not() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, _prods) = hard_constraints(&mut a, 64);
    fill(&mut s, &mut a);

    // `x * y == 7` with both factors below 2: the products are 0 and 1, never 7.
    let x = a.var(Sort::BitVec(32), "px");
    let y = a.var(Sort::BitVec(32), "py");
    let p = a.mul(x, y);
    let seven = a.bv(32, 7);
    let two = a.bv(32, 2);
    let prod = a.eq(p, seven);
    let xs = a.ult(x, two);
    let ys = a.ult(y, two);
    let contradiction = [prod, xs, ys];
    assert!(
        matches!(s.check(&mut a, &contradiction), CheckResult::Unsat),
        "the fixture must actually be unsatisfiable"
    );

    let before = s.stats().backend_calls;
    let superset = [prod, xs, ys, cs[5], cs[9]];
    assert!(
        matches!(s.check(&mut a, &superset), CheckResult::Unsat),
        "more constraints cannot rescue a contradiction"
    );
    assert_eq!(
        s.stats().backend_calls,
        before,
        "and that needs no backend either"
    );

    // The subset direction. `x * y == 7` on its own is satisfiable, and a cache that
    // answered it from the `Unsat` above would be the silent catastrophic bug §6 names.
    assert!(
        matches!(s.check(&mut a, &[prod]), CheckResult::Sat(_)),
        "dropping the bound that caused the contradiction leaves a satisfiable set"
    );
}

/// **022 contract 12.** A model already in the cache that satisfies a *new* query — not a
/// subset of anything — answers it with no backend call. This is the rule that makes
/// sibling states nearly free, since they share long path-condition prefixes by design
/// (023 §1).
#[test]
fn a_cached_model_that_satisfies_a_new_query_answers_it() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, prods) = hard_constraints(&mut a, 64);
    fill(&mut s, &mut a);

    // A set whose model pins the first sixteen pairs…
    assert!(matches!(s.check(&mut a, &cs[..16]), CheckResult::Sat(_)));
    let before = s.stats().backend_calls;
    // …and a query that is *not* a subset of it: a fresh constraint about the same
    // variables that the model in hand already satisfies. `x3 * y3 == 7` and
    // `x3 * y3 != 0` are both true under it, and the second appears in no cached set.
    let zero = a.bv(32, 0);
    let z = a.eq(prods[3], zero);
    let nonzero = a.not(z);
    assert!(
        matches!(s.check(&mut a, &[cs[3], nonzero]), CheckResult::Sat(_)),
        "a model already in hand satisfies this"
    );
    assert_eq!(
        s.stats().backend_calls,
        before,
        "so no backend was asked: {}",
        s.stats().backend_calls
    );
}

/// A cache hit is still an **answer about this query**: the model handed back has to
/// satisfy the query it is returned for, or every downstream witness is fiction.
#[test]
fn a_model_returned_from_the_cache_satisfies_the_query_it_answers() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, _) = hard_constraints(&mut a, 64);
    assert!(matches!(s.check(&mut a, &cs), CheckResult::Sat(_)));
    let sub = &cs[8..24];
    let CheckResult::Sat(m) = s.check(&mut a, sub) else {
        panic!("a subset of a satisfiable set is satisfiable");
    };
    for t in sub {
        assert_eq!(
            a.eval(&m, *t).map(|v| v.bits() != 0),
            Ok(true),
            "the returned model must satisfy every constraint it is offered for"
        );
    }
}

/// **A candidate is not an answer.** A cached set sharing a constraint with the query
/// makes its model a *candidate*; whether it answers the query is settled by evaluating
/// it. Skipping that returns `Sat` for an unsatisfiable query — the cache would be
/// inventing satisfying assignments, and every finding built on one is fiction.
#[test]
fn a_cached_model_that_fails_the_query_is_not_used_as_an_answer() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, prods) = hard_constraints(&mut a, 8);
    fill(&mut s, &mut a);
    // `x3 * y3 == 7` is satisfiable, and its model is now in the cache.
    assert!(matches!(s.check(&mut a, &cs[3..=3]), CheckResult::Sat(_)));

    // The same product cannot also be 8. The query *shares* `cs[3]`, so the cached model
    // is a candidate — and it does not satisfy the second constraint.
    let eight = a.bv(32, 8);
    let other = a.eq(prods[3], eight);
    assert!(
        matches!(s.check(&mut a, &[cs[3], other]), CheckResult::Unsat),
        "7 and 8 are not the same number"
    );
}
