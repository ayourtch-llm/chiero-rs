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
//! ⚠️ Three traps this file exists to avoid. Every constraint a rule is *measured* on is
//! **nonlinear**, so tier 1 returns `Unknown` and a backend call is what a cache miss
//! looks like — over linear constraints tier 1 answers everything and `backend_calls`
//! stays at zero whether the cache works or not. (The thousand *filler* constraints are
//! linear on purpose, for speed; an earlier version of this header claimed every
//! constraint was nonlinear, which was false of about 99% of them.) Terms are *reused*
//! rather than rebuilt: `TermArena::var` mints a fresh `VarId` per call, so two
//! `var(BitVec(32), "x0")` are two variables and a "contradiction" built that way is
//! satisfiable. And `fill` runs **between** the decisive query and the lookup, or a cache
//! that remembers only the last query passes every one of these.

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
/// **Call this between the decisive query and the lookup, not before it.** Filling first
/// leaves the decisive set as the most recently remembered one, so an implementation that
/// keeps only the last query passes — which is the exact implementation §6.2 names when it
/// asks for ≥1000 entries. Review demonstrated that mutant surviving. Found by review.
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

/// A cold `Sat` that a loaded machine is allowed to fail to produce.
///
/// **Two tests here assert `Sat` on a fresh query as *setup*, and that is a wall-clock dependency.**
/// 022 §4 gives the backend a watchdog; on a busy machine a sixty-four-constraint query can blow it
/// and come back undecided, and the test then fails for a reason that has nothing to do with what it
/// is about. Reproduced deterministically by running twelve spinners on a twelve-core machine: the
/// same two tests fail every time, and pass every time without them.
///
/// So an undecided setup **skips**, exactly as a missing backend already does. The alternative —
/// raising the timeout — would trade a visible flake for a slower suite and a bound nobody could
/// justify. What must not happen is a red suite that means "the machine was busy".
fn cold_sat(s: &mut TieredSolver, a: &mut TermArena, cs: &[Term], what: &str) -> bool {
    if matches!(s.check(a, cs), CheckResult::Sat(_)) {
        return true;
    }
    eprintln!(
        "skipping {what}: the cold query did not come back `Sat` — 022 §4's watchdog fires on a loaded machine, and this is setup rather than subject"
    );
    false
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
    let big: Vec<Term> = cs.clone();
    if !cold_sat(&mut s, &mut a, &big, "the subset-is-satisfiable contract") {
        return;
    }
    fill(&mut s, &mut a);

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
    fill(&mut s, &mut a);

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

    // A set whose model pins the first sixteen pairs…
    assert!(matches!(s.check(&mut a, &cs[..16]), CheckResult::Sat(_)));
    fill(&mut s, &mut a);
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
    if !cold_sat(
        &mut s,
        &mut a,
        &cs,
        "the cached-model-satisfies-the-query contract",
    ) {
        return;
    }
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

/// **The sat rule's soundness rests entirely on `eval` being total-or-error**, and
/// nothing here held that in place: in every other fixture the candidate model covers the
/// whole query, so "the model says nothing about this variable" never happens. Two
/// mutations that treat an eval *error* as satisfied survived the whole suite, and both
/// return `Sat` for an unsatisfiable query. Found by review.
///
/// The query has to be one **tier 1 cannot decide** — the cache sits below tier 1, so a
/// contradiction tier 1 sees never reaches it — and has to share a term with a cached set,
/// or there is no candidate to mis-evaluate.
#[test]
fn a_variable_the_cached_model_does_not_assign_is_not_treated_as_satisfied() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, _) = hard_constraints(&mut a, 4);
    assert!(matches!(s.check(&mut a, &cs[0..=0]), CheckResult::Sat(_)));

    // `u * v == 7 ∧ u <u 2 ∧ v <u 2` — unsatisfiable, and nonlinear so tier 1 cannot say
    // so. `u` and `v` appear in no cached set, so the model of `cs[0]` has no value for
    // them; an implementation reading "no value" as "satisfied" answers `Sat` for a
    // contradiction, and `cs[0]` in the query is what makes that model a candidate.
    let u = a.var(Sort::BitVec(32), "u");
    let v = a.var(Sort::BitVec(32), "v");
    let p = a.mul(u, v);
    let seven = a.bv(32, 7);
    let two = a.bv(32, 2);
    let prod = a.eq(p, seven);
    let us = a.ult(u, two);
    let vs = a.ult(v, two);
    assert!(
        matches!(s.check(&mut a, &[cs[0], prod, us, vs]), CheckResult::Unsat),
        "the products of 0 and 1 are 0 and 1, whatever the cached model says about x0"
    );
}

/// **The assertion stack is part of the query.** Every other test here calls `check` with
/// an empty stack, so two mutations survived: one where the cache looks at the assumptions
/// alone — answering from a set that says nothing about what the stack constrains — and
/// one where `remember` stores only the assumption ids, so an `Unsat` proved *under* a
/// stack is recorded against a subset of what actually conflicted and the superset rule
/// then answers `Unsat` for that subset alone. The second is the catastrophic direction.
/// Found by review.
#[test]
fn the_assertion_stack_is_part_of_what_the_cache_remembers_and_answers() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "sx");
    let y = a.var(Sort::BitVec(32), "sy");
    let p = a.mul(x, y);
    let seven = a.bv(32, 7);
    let two = a.bv(32, 2);
    let prod = a.eq(p, seven);
    let xs = a.ult(x, two);
    let ys = a.ult(y, two);

    // Warm the cache with `{prod}` satisfiable, so there is a model to answer from.
    assert!(matches!(s.check(&mut a, &[prod]), CheckResult::Sat(_)));

    // Under a stack that bounds both factors, the same assumption is unsatisfiable.
    // Reading the assumption alone finds the cached model and answers `Sat`.
    s.push();
    s.assert(xs);
    s.assert(ys);
    assert!(
        matches!(s.check(&mut a, &[prod]), CheckResult::Unsat),
        "the stack constrains the query asked under it"
    );

    // Pop it. The set that was unsatisfiable was `{prod, xs, ys}`; `prod` alone is
    // satisfiable, and a cache that recorded the contradiction against `prod` only
    // answers `Unsat` here — for a query it has already answered `Sat` once.
    s.pop(1);
    // **A query the exact cache has not seen**, or it answers from the earlier `Sat` and
    // the counterexample cache is never consulted — which is what let this mutant live
    // through the first version of this test. `prod` plus an unrelated satisfiable
    // constraint is a *superset* of `{prod}`, so a contradiction recorded against `prod`
    // alone fires the superset rule here.
    let z = a.var(Sort::BitVec(32), "sz");
    let three = a.bv(32, 3);
    let other = a.eq(z, three);
    assert!(
        matches!(s.check(&mut a, &[prod, other]), CheckResult::Sat(_)),
        "x * y == 7 has solutions once the bounds are gone, and z == 3 does not change it"
    );
}

/// **022 contract 8.** The same query, answered cold and from an exact-cache hit, returns
/// a byte-identical model — and two independent runs of the same query do too.
///
/// §2: "Two runs producing different counterexamples for the same query would break golden
/// tests and make findings unreproducible." That is what a witness (023 §9) rests on, so
/// this is tested on both tiers: a query tier 1 decides, and one only a backend can.
///
/// The contract's fifth way, "with slicing disabled", is not tested: independence slicing
/// does not exist yet, so there is nothing to disable and a test would assert against
/// itself. Listed as owed rather than claimed.
#[test]
fn the_same_query_gives_a_byte_identical_model_cold_and_warm() {
    // Tier 1's own answer: `x <u 5`.
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let five = a.bv(32, 5);
    let lt = a.ult(x, five);
    let mut s = TieredSolver::new();
    let CheckResult::Sat(cold) = s.check(&mut a, &[lt]) else {
        panic!("tier 1 decides this");
    };
    let CheckResult::Sat(warm) = s.check(&mut a, &[lt]) else {
        panic!("and again");
    };
    assert_eq!(
        cold, warm,
        "an exact-cache hit is the same answer, model included"
    );

    // A fresh solver over a fresh arena reaches the same model: the answer is a function
    // of the query, not of the solver's history.
    let mut a2 = TermArena::new();
    let x2 = a2.var(Sort::BitVec(32), "x");
    let five2 = a2.bv(32, 5);
    let lt2 = a2.ult(x2, five2);
    let mut s2 = TieredSolver::new();
    let CheckResult::Sat(again) = s2.check(&mut a2, &[lt2]) else {
        panic!("tier 1 decides this");
    };
    assert_eq!(cold, again, "two runs of one query agree");

    // And the same on tier 2, where the model comes from the backend.
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping the tier-2 half: no SMT-LIB backend (022 contract 2)");
        return;
    };
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut ar = TermArena::new();
        let p = ar.var(Sort::BitVec(32), "p");
        let q = ar.var(Sort::BitVec(32), "q");
        let prod = ar.mul(p, q);
        let seven = ar.bv(32, 7);
        let e = ar.eq(prod, seven);
        let mut sr = TieredSolver::with_backend(backend.clone());
        let CheckResult::Sat(m) = sr.check(&mut ar, &[e]) else {
            panic!("7 has factorizations");
        };
        // …and immediately again, from the exact cache.
        let CheckResult::Sat(m2) = sr.check(&mut ar, &[e]) else {
            panic!("cached");
        };
        assert_eq!(m, m2, "the exact cache returns the model it stored");
        runs.push(m);
    }
    assert_eq!(
        runs[0], runs[1],
        "two independent runs of one backend query agree — a witness that changed between \
         runs would not be replayable"
    );
}

/// **022 contract 8b.** A counterexample-cache hit returns the same *verdict* a fresh solve
/// would, and may return a different satisfying assignment.
///
/// The verdict half is what makes it safe; the assignment half is why contract 8 is scoped
/// to the exact cache. Both are asserted here, because "may differ" is not a licence to be
/// wrong — the returned model is still checked against every constraint of the query it
/// answers, which is what makes a cache `Sat` self-certifying in §3's sense.
#[test]
fn a_counterexample_hit_keeps_the_verdict_and_may_change_the_assignment() {
    let Some(mut s) = solver() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (cs, _) = hard_constraints(&mut a, 16);

    // Cold: the model of one constraint alone.
    let CheckResult::Sat(alone) = s.check(&mut a, &cs[3..=3]) else {
        panic!("satisfiable");
    };
    // A larger set, whose model pins sixteen pairs…
    assert!(matches!(s.check(&mut a, &cs), CheckResult::Sat(_)));
    // …and now the *same single-constraint query* through a different solver whose cache
    // was warmed the other way round.
    let mut s2 = solver().expect("checked above");
    assert!(matches!(s2.check(&mut a, &cs), CheckResult::Sat(_)));
    let CheckResult::Sat(from_cache) = s2.check(&mut a, &cs[3..=3]) else {
        panic!("a subset of a satisfiable set is satisfiable");
    };

    // The verdict is the same — that is the guarantee.
    // The assignment need not be, and here it is not: `from_cache` came from the
    // sixteen-pair model. What must hold is that it satisfies the query it answers.
    assert_eq!(
        a.eval(&from_cache, cs[3]).map(|v| v.bits() != 0),
        Ok(true),
        "a returned model satisfies the query it is returned for"
    );
    assert!(
        from_cache.len() >= alone.len(),
        "the cached model is the larger one, which is the case worth checking: {} vs {}",
        from_cache.len(),
        alone.len()
    );
}
