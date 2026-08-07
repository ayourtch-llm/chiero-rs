//! Independence slicing and the satisfiability invariant it rests on — 022 contracts 9
//! and 9b.
//!
//! Covers: 022 contracts 9, 9b.
//!
//! §6.2: "partition the assertion set into connected components by shared variables, and
//! solve only the component(s) containing the query's variables, subject to §6.1. A path
//! condition of 200 constraints usually decomposes into many small independent ones; this
//! is the single largest measured win in KLEE and is specified here as required, not
//! optional."
//!
//! Contract 9 is deliberately worded against the easy test: slicing "sends only the
//! relevant component to the backend **and returns the same answer as the unsliced
//! query**, over the whole corpus. Verifying only that the dumped query got smaller tests
//! that slicing happened, not that it was correct." Both halves are needed and neither
//! alone is worth anything — a slicer that returns the whole set passes the correctness
//! half, and a slicer that drops constraints at random passes the size half.
//!
//! §6.1 is the reason 9b exists. Slicing is equisatisfiable *only if every other component
//! is already known satisfiable*, and chiero breaks that invariant in three places on
//! purpose (023 §3's `Unknown` branch, 021 §5's continue-after-OOB, 024 §4's `strlen`
//! cap). When it is broken, a quietly-`Unsat` component sits in a corner of the path
//! condition while the query slices to a different corner, and every finding on that dead
//! path gets reported with a witness that does not satisfy the path condition.

use chiero_solver::*;

/// A tiny deterministic PRNG — same reasoning as the campaign: a corpus that changes
/// between runs cannot be re-run against a fix.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A path condition made of `groups` variable-disjoint clusters, which is the shape §6.2
/// says a real one has. Returns the assertions and the variables of each group, so a
/// query can be aimed at one cluster and the rest left as the part slicing is supposed to
/// skip.
fn clustered(a: &mut TermArena, r: &mut Rng, groups: usize) -> (Vec<Term>, Vec<Vec<Term>>) {
    let mut asserts = Vec::new();
    let mut vars = Vec::new();
    for g in 0..groups {
        // Three variables per cluster, named per-cluster so nothing accidentally joins
        // two clusters into one component and makes the test measure nothing.
        let vs: Vec<Term> = (0..3)
            .map(|i| a.var(Sort::BitVec(8), &format!("g{g}v{i}")))
            .collect();
        for _ in 0..2 + r.below(3) {
            let x = vs[r.below(3) as usize];
            let y = if r.below(3) == 0 {
                vs[r.below(3) as usize]
            } else {
                a.bv(8, r.below(64) as u128)
            };
            let (l, rr) = if r.below(2) == 0 { (x, y) } else { (y, x) };
            asserts.push(match r.below(4) {
                0 => a.eq(l, rr),
                1 => {
                    let e = a.eq(l, rr);
                    a.not(e)
                }
                _ => a.ult(l, rr),
            });
        }
        vars.push(vs);
    }
    (asserts, vars)
}

/// **022 contract 9, the correctness half.** Over a corpus, the sliced answer is the
/// unsliced answer — and when it is `Sat`, the model is complete and satisfies *every*
/// assertion, not merely the ones in the slice that produced it. A slicer that returns a
/// model covering only the query's component satisfies "same verdict" and still breaks
/// 023 contract 16, whose witnesses have to satisfy the whole path condition.
#[test]
fn a_sliced_query_answers_what_the_unsliced_query_answers() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut rng = Rng(0x0BAD_C0DE_0BAD_C0DE);
    let mut sat = 0u32;
    let mut unsat = 0u32;

    for case in 0..120u32 {
        let mut a = TermArena::new();
        let (asserts, groups) = clustered(&mut a, &mut rng, 4);
        // The query touches exactly one cluster, so three of the four components are the
        // part slicing is entitled to leave alone.
        let g = &groups[(case as usize) % groups.len()];
        let k = a.bv(8, rng.below(64) as u128);
        let query = a.ult(g[0], k);

        let mut pc = PathCondition::new();
        for t in &asserts {
            pc.push_checked(*t);
        }

        let mut on = TieredSolver::with_backend(backend.clone());
        let mut off = TieredSolver::with_backend(backend.clone());
        off.set_slicing(false);
        let r_on = on.check_path(&mut a, &mut pc.clone(), &[query]);
        let r_off = off.check_path(&mut a, &mut pc.clone(), &[query]);

        match (&r_on, &r_off) {
            (CheckResult::Sat(m), CheckResult::Sat(_)) => {
                sat += 1;
                for t in asserts.iter().chain(std::iter::once(&query)) {
                    assert_eq!(
                        a.eval(m, *t).map(|c| c.bits()),
                        Ok(1),
                        "case {case}: the sliced model does not satisfy an assertion \
                         outside the slice it was solved from"
                    );
                }
            }
            (CheckResult::Unsat, CheckResult::Unsat) => unsat += 1,
            // A backend `Unknown` on either side is not a disagreement; it is the one
            // answer that is always permitted.
            (CheckResult::Unknown(_), _) | (_, CheckResult::Unknown(_)) => {}
            _ => panic!("case {case}: slicing changed the answer: {r_on:?} vs {r_off:?}"),
        }
    }
    // Both verdicts have to occur, or "the same answer" was demonstrated for one of them.
    assert!(
        sat > 0 && unsat > 0,
        "corpus is one-sided: {sat} sat, {unsat} unsat"
    );
}

/// **022 contract 9, the other half.** Slicing has to actually happen. Without this the
/// test above passes for a `set_slicing` that does nothing at all — the same-answer trap,
/// where the two sides agree because they are the same code.
#[test]
fn slicing_sends_less_to_the_backend_than_the_whole_path_condition() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let mut rng = Rng(0x51CE_51CE_51CE_51CE);
    let (asserts, groups) = clustered(&mut a, &mut rng, 6);
    let k = a.bv(8, 7);
    let query = a.ult(groups[0][0], k);

    let mut pc = PathCondition::new();
    for t in &asserts {
        pc.push_checked(*t);
    }

    let mut on = TieredSolver::with_backend(backend.clone());
    on.check_path(&mut a, &mut pc.clone(), &[query]);
    let skipped = on.stats().sliced_terms_skipped;

    let mut off = TieredSolver::with_backend(backend);
    off.set_slicing(false);
    off.check_path(&mut a, &mut pc.clone(), &[query]);

    assert_eq!(
        off.stats().sliced_terms_skipped,
        0,
        "with slicing off nothing may be withheld from the backend"
    );
    // Five of six clusters are irrelevant to the query, so most of the path condition
    // should never reach z3.
    assert!(
        skipped * 2 > asserts.len() as u64,
        "slicing withheld only {skipped} of {} assertions, which is not a partition into \
         components — it is a rounding error",
        asserts.len()
    );
}

/// An unsatisfiable constraint **tier 1 cannot see through**: `x * 2 == 1` has no
/// solution at any width, since doubling clears the low bit, but multiplication is not in
/// §3.2's transfer set so the interval domain returns `Unknown` and the query reaches the
/// backend.
///
/// This matters more than it looks. Written the obvious way — `x == 1 && x == 2` — tier 1
/// refutes it from two conflicting equalities before any backend call, so every test
/// below passes without slicing or the subsumption index ever running. They did, and the
/// `sliced_terms_skipped` assertions are what caught it.
fn tier1_opaque_contradiction(a: &mut TermArena) -> Term {
    let x = a.var(Sort::BitVec(8), "x");
    let two = a.bv(8, 2);
    let one = a.bv(8, 1);
    let d = a.mul(x, two);
    a.eq(d, one)
}

/// A path condition holding that contradiction in one cluster and a satisfiable
/// constraint on an unrelated variable in another. The contradiction is added *without* a
/// feasibility check, which is exactly what 023 §3 does on solver `Unknown`.
fn poisoned(a: &mut TermArena) -> (PathCondition, Term) {
    let y = a.var(Sort::BitVec(8), "y");
    let ten = a.bv(8, 10);

    let mut pc = PathCondition::new();
    // Nothing established that this is consistent with what is already here, and it is
    // not consistent with anything at all.
    let bad = tier1_opaque_contradiction(a);
    pc.push_unchecked(bad);
    let ylt = a.ult(y, ten);
    pc.push_checked(ylt);

    let three = a.bv(8, 3);
    let query = a.ult(y, three);
    (pc, query)
}

/// **022 contract 9b.** "A state whose path condition contains an unsatisfiable component
/// reports `Unsat` for every subsequent feasibility query, with slicing on and off."
#[test]
fn an_unsatisfiable_component_is_reported_even_when_the_query_is_elsewhere() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    for slicing in [true, false] {
        let mut a = TermArena::new();
        let (mut pc, query) = poisoned(&mut a);
        assert!(
            pc.possibly_infeasible(),
            "a constraint added without a feasibility check sets the flag"
        );
        let mut s = TieredSolver::with_backend(backend.clone());
        s.set_slicing(slicing);
        assert!(
            matches!(s.check_path(&mut a, &mut pc, &[query]), CheckResult::Unsat),
            "slicing={slicing}: the dead component is in another part of the path \
             condition, and the query is satisfiable in isolation — reporting `Sat` here \
             hands every finding on this path a witness that does not satisfy it"
        );
        assert_eq!(
            s.stats().sliced_terms_skipped,
            0,
            "slicing={slicing}: §6.1 disables slicing while the flag is set"
        );
    }
}

/// **Deviation from §6.1, pinned deliberately.** The spec's design makes the flag the
/// *only* thing standing between a poisoned path condition and a wrong `Sat`, so this
/// started life as a negative control asserting that a lying `push_checked` slices to
/// `Sat` — the reachable wrong answer §6.1 exists to prevent. It does not, and should
/// not: `ask_backend` completes a sliced `Sat` model over the skipped components, because
/// 023 contract 16 needs a witness that satisfies the *whole* path condition and a model
/// covering one component does not. Completing it catches a dead component on the way,
/// so the wrong answer is unreachable whether or not the flag is right.
///
/// This is worth a test rather than a comment: it says the soundness of slicing does not
/// rest on every one of §6.1's three call sites having remembered to use
/// `push_unchecked`, and a future change that completes models more cheaply — say, from
/// the per-slice cache only, filling misses with zeroes — would put that dependency back
/// and this is what would notice.
#[test]
fn slicing_stays_sound_when_the_flag_is_wrong() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let (poisoned_pc, query) = poisoned(&mut a);
    let mut lying = PathCondition::new();
    for t in poisoned_pc.terms() {
        lying.push_checked(*t);
    }
    assert!(!lying.possibly_infeasible());

    let mut s = TieredSolver::with_backend(backend);
    let r = s.check_path(&mut a, &mut lying, &[query]);
    assert!(
        matches!(r, CheckResult::Unsat),
        "the dead component is in a part of the path condition the query does not touch \
         and the flag does not warn about it, and the answer is still `Unsat`: {r:?}"
    );
    assert!(
        s.stats().sliced_terms_skipped > 0,
        "and it was reached by slicing, not by slicing having silently not happened"
    );
}

/// §6.1: "A single full check that returns `Sat` clears it." A flag that is never cleared
/// makes every state after the first `Unknown` branch permanently unsliced, which is the
/// slow-but-correct failure — worth a test because nothing else would notice.
#[test]
fn a_full_satisfiable_check_clears_the_flag() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let five = a.bv(8, 5);
    let mut pc = PathCondition::new();
    let t = a.ult(x, five);
    pc.push_unchecked(t);
    assert!(pc.possibly_infeasible());

    let mut s = TieredSolver::with_backend(backend);
    // A *full* check — no assumptions, so what was proved satisfiable is the path
    // condition itself. A check with assumptions proves something else and must not
    // clear the flag.
    assert!(matches!(
        s.check_path(&mut a, &mut pc, &[]),
        CheckResult::Sat(_)
    ));
    assert!(
        !pc.possibly_infeasible(),
        "the path condition was proved satisfiable in full, so the invariant holds again"
    );
}

/// §6.1 disables "slicing **and the subset/superset cache rules**" together. The cache
/// half is invisible in the verdict — a superset of an `Unsat` set is `Unsat` either way —
/// so it is only observable in whether the backend was asked.
#[test]
fn the_subsumption_index_is_bypassed_while_the_flag_is_set() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut a = TermArena::new();
    let y = a.var(Sort::BitVec(8), "y");
    let one = a.bv(8, 1);
    let bad = tier1_opaque_contradiction(&mut a);
    let y1 = a.eq(y, one);

    // Prime the index with an `Unsat` set — one the backend had to decide, or the index
    // is never consulted and neither branch of this test means anything.
    let mut s = TieredSolver::with_backend(backend);
    let mut base = PathCondition::new();
    base.push_checked(bad);
    assert!(matches!(
        s.check_path(&mut a, &mut base.clone(), &[]),
        CheckResult::Unsat
    ));

    // A superset, with the invariant intact: answered from the index, no backend call.
    let before = s.stats().backend_calls;
    let mut sup = base.clone();
    sup.push_checked(y1);
    assert!(matches!(
        s.check_path(&mut a, &mut sup, &[]),
        CheckResult::Unsat
    ));
    assert_eq!(
        s.stats().backend_calls,
        before,
        "a superset of a known-`Unsat` set needs no solver (022 contract 11)"
    );

    // A *different* superset with the flag set has to go to the backend instead. It has
    // to be a different one: the exact cache is keyed on the term ids and would answer
    // the previous query without consulting the index at all, which §6.1 does not
    // disable and soundly so — an exact match is the same question, not a subsumed one.
    let z = a.var(Sort::BitVec(8), "z");
    let z1 = a.eq(z, one);
    let before = s.stats().backend_calls;
    let mut poisoned_sup = base.clone();
    poisoned_sup.push_unchecked(z1);
    assert!(matches!(
        s.check_path(&mut a, &mut poisoned_sup, &[]),
        CheckResult::Unsat
    ));
    assert!(
        s.stats().backend_calls > before,
        "§6.1 disables the subset/superset rules while the flag is set, and this query \
         was answered from the index anyway"
    );
}

/// **Slicing must not do more work as the arena fills up.**
///
/// 022 §6.2 makes independence slicing required rather than optional, so `components` runs on
/// **every** backend query, and it calls `vars_of` once per constraint. `vars_of` allocated and
/// zeroed `vec![false; nodes.len()]` on each of those calls — a bool for every term the arena
/// has *ever* held, whether or not the constraint mentions it. A `TermArena` only grows, so the
/// cost of slicing a fixed path condition grew with everything the run had done beforehand:
/// 8.3 µs at a thousand nodes, 699 µs at 256 000, for the same forty one-variable constraints.
///
/// Found by sampling a real VPP run under `gdb`, not by reading: two of eight samples were
/// inside `TermArena::vars_of`, reached through `TieredSolver::components`.
///
/// ⚠️ **This asserted a ratio of two durations first, and that was the wrong instrument.** It
/// passed run alone and failed under the full workspace run, because the two measurements were
/// taken at different machine loads — §11.3's own corollary, that only a full run puts enough
/// load on to expose this. An intermittently red suite is worse than no test.
///
/// `vars_of_visits` counts nodes stamped. It is identical on every machine and under any load,
/// so the assertion is about the algorithm rather than about the afternoon — and it is *exact*,
/// which a timing bound can never be: forty one-variable constraints must cost the same walk
/// whether the arena around them holds two thousand nodes or two hundred thousand.
#[test]
fn slicing_work_does_not_grow_with_unrelated_arena_size() {
    let walked = |pad: u32| {
        let mut a = TermArena::new();
        let mut path = Vec::new();
        for i in 0..40u32 {
            let v = a.var(Sort::BitVec(32), &format!("v{i}"));
            let k = a.bv(32, u128::from(i));
            path.push(a.ult(v, k));
        }
        // Unrelated terms: the rest of a run's history, which `vars_of` must not look at.
        let base = a.var(Sort::BitVec(32), "pad");
        for i in 0..pad {
            let k = a.bv(32, u128::from(i));
            let _ = a.add(base, k);
        }
        // **Warm-up, outside the measurement.** The scratch grows once to match the arena, and
        // a real run pays that incrementally as the arena itself grows. Charging it to the
        // first call here would measure the allocation rather than the query — which is the
        // same distinction the benchmark that found this defect had to make.
        {
            let mut w = Vec::new();
            a.vars_of(path[0], &mut w);
        }
        let before = (a.vars_of_visits(), a.vars_of_scratch_init());
        let mut seen = 0usize;
        for c in &path {
            let mut vs = Vec::new();
            a.vars_of(*c, &mut vs);
            seen += vs.len();
        }
        assert_eq!(seen, 40, "each constraint mentions exactly one variable");
        (
            a.vars_of_visits() - before.0,
            a.vars_of_scratch_init() - before.1,
        )
    };

    let (small, small_init) = walked(2_000);
    let (large, large_init) = walked(200_000);
    // **The assertion that can see the defect.** Forty calls against a 200 000-node arena
    // initialised the whole buffer *each time* before — eight million entries. Reusing it means
    // the growth is one-time, and by the time these forty calls run it has already happened, so
    // the honest number here is zero.
    assert_eq!(
        (small_init, large_init),
        (0, 0),
        "the scratch is initialised per growth, not per call"
    );
    assert_eq!(
        small, large,
        "a hundredfold arena around the same forty constraints changed the walk from {small} \
         nodes to {large}: the work is proportional to the arena rather than to the terms \
         being sliced"
    );
    // And the walk is the terms themselves, not something that merely happens to be constant.
    //
    // ⚠️ Written as 160 first — "the ult, the var, the const, and the root" — and the fixture
    // said 120. There is no separate root: `ult(var, const)` *is* the root, so it is three
    // nodes. The fixture failing rather than the code is the good direction, and a count I had
    // to correct is worth more than one I happened to guess right.
    assert_eq!(small, 120, "40 constraints x (ult, var, const) = 120 nodes");
}
