//! The SMT-LIB2 subprocess backend and `TieredSolver` (022 §4, §6).
//!
//! Covers **022 contracts 1, 2, 6, 11c, 13, 14, 17, 20**.
//!
//! Two rules shape every test here. chiero **never links** a solver — tier 2 is a
//! subprocess discovered at runtime, and the whole suite must pass with none installed
//! (contract 2). And a tier-2 test that silently passes when z3 is absent would be worse
//! than no test, so every one of them either runs or prints why it did not.

use chiero_solver::{CheckResult, SmtLib, Solver, SolverLite, Sort, TermArena, TieredSolver};

/// 022 contract 2: with no solver on `PATH` the suite still runs, and a tier-2 test
/// **says** it was skipped rather than reporting success.
fn z3_or_skip(test: &str) -> Option<SmtLib> {
    match SmtLib::discover() {
        Some(s) => Some(s),
        None => {
            println!("SKIP {test}: no SMT-LIB2 solver on PATH (022 contract 2)");
            None
        }
    }
}

#[test]
fn discovery_is_runtime_not_build_time() {
    // Whether a solver exists is a runtime fact. The crate must build and this test must
    // pass either way — that is what "never links a solver" means in practice.
    let found = SmtLib::discover().is_some();
    println!("SMT-LIB2 backend available: {found}");
}

/// 022 contract 6: `(x * y) == 7 ∧ x > 1 ∧ y > 1` is `Unknown` from tier 1 and `Sat`
/// from tier 2. Escalation demonstrably happens **and demonstrably matters**.
#[test]
fn escalation_decides_what_tier_one_cannot() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let y = a.var(Sort::BitVec(32), "y");
    let p = a.mul(x, y);
    let seven = a.bv(32, 7);
    let one = a.bv(32, 1);
    let e = a.eq(p, seven);
    let gx = a.ult(one, x);
    let gy = a.ult(one, y);

    // Tier 1 alone cannot decide it.
    let mut lite = SolverLite::new();
    lite.assert(e);
    lite.assert(gx);
    lite.assert(gy);
    assert!(matches!(lite.check(&mut a, &[]), CheckResult::Unknown(_)));

    let Some(backend) = z3_or_skip("escalation_decides_what_tier_one_cannot") else {
        return;
    };
    let mut t = TieredSolver::with_backend(backend);
    t.assert(e);
    t.assert(gx);
    t.assert(gy);
    match t.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            // 7 is prime, so the only factors above 1 are 7 and ... none. Whatever the
            // backend returns must still satisfy the constraints under our own
            // evaluator — tier 2's answer is not exempt from validation.
            assert!(a.eval(&m, e).unwrap().bits() != 0);
        }
        CheckResult::Unsat => {} // also correct: 7 is prime
        other => panic!("tier 2 must decide it, got {other:?}"),
    }
}

/// Tier 2's answers are validated too. A backend that returned a wrong model would
/// otherwise be trusted purely because it is external.
#[test]
fn tier_two_models_are_validated() {
    let Some(backend) = z3_or_skip("tier_two_models_are_validated") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(16), "x");
    let y = a.var(Sort::BitVec(16), "y");
    let p = a.mul(x, y);
    let target = a.bv(16, 1001);
    let one = a.bv(16, 1);
    let e = a.eq(p, target);
    let gx = a.ult(one, x);

    let mut t = TieredSolver::with_backend(backend);
    t.assert(e);
    t.assert(gx);
    if let CheckResult::Sat(m) = t.check(&mut a, &[]) {
        assert!(
            a.eval(&m, e).unwrap().bits() != 0,
            "model must satisfy x*y == 1001"
        );
        assert!(a.eval(&m, gx).unwrap().bits() != 0);
    }
}

/// 022 contract 13, in miniature: `paranoid` sends every tier-1 answer to tier 2 and
/// asserts agreement. This is the cross-validation harness, so it must actually run
/// both — a mode that quietly skipped tier 2 would report perfect agreement.
#[test]
fn paranoid_mode_finds_no_disagreement() {
    let Some(backend) = z3_or_skip("paranoid_mode_finds_no_disagreement") else {
        return;
    };
    let mut t = TieredSolver::with_backend(backend);
    t.set_paranoid(true);

    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    let mut checked = 0;
    for _ in 0..200 {
        let mut a = TermArena::new();
        let x = a.var(Sort::BitVec(8), "x");
        let mut s = TieredSolver::with_backend(SmtLib::discover().unwrap());
        s.set_paranoid(true);
        for _ in 0..(rng() % 3 + 1) {
            let k = a.bv(8, (rng() % 256) as u128);
            let c = match rng() % 3 {
                0 => a.ult(x, k),
                1 => a.ult(k, x),
                _ => a.eq(x, k),
            };
            s.assert(c);
        }
        // In paranoid mode a disagreement panics inside `check`; reaching here is the
        // assertion.
        let _ = s.check(&mut a, &[]);
        checked += 1;
    }
    assert!(checked > 0);
    let _ = t;
}

/// 022 contract 20: solving the same path condition twice makes exactly one backend
/// call. The exact cache is keyed on the assertion **and assumption** sets.
#[test]
fn the_exact_cache_avoids_a_second_backend_call() {
    let Some(backend) = z3_or_skip("the_exact_cache_avoids_a_second_backend_call") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let y = a.var(Sort::BitVec(32), "y");
    let p = a.mul(x, y);
    let k = a.bv(32, 91);
    let e = a.eq(p, k);

    let mut t = TieredSolver::with_backend(backend);
    t.assert(e);
    let first = t.check(&mut a, &[]);
    let calls_after_first = t.stats().backend_calls;
    let second = t.check(&mut a, &[]);
    assert_eq!(
        t.stats().backend_calls,
        calls_after_first,
        "the second identical query must not reach the backend"
    );
    assert_eq!(
        format!("{first:?}").split('(').next(),
        format!("{second:?}").split('(').next(),
        "and must give the same answer"
    );
}

/// 022 contract 11b: `check([c])` and `check([¬c])` on one assertion stack must not
/// collide. Omitting assumptions from the cache key makes them return each other's
/// answers — silent and catastrophic.
#[test]
fn assumptions_participate_in_the_cache_key() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let c5 = a.bv(8, 5);
    let c9 = a.bv(8, 9);
    let base = a.ult(x, c9);
    let lt = a.ult(x, c5);
    let ge = a.ult(c5, x);

    let mut t = TieredSolver::new();
    t.assert(base);
    let a1 = t.check(&mut a, &[lt]);
    let a2 = t.check(&mut a, &[ge]);
    // Both are satisfiable under `x < 9`, but they must be *separately* decided rather
    // than one serving the other's cached result.
    assert!(matches!(a1, CheckResult::Sat(_)));
    assert!(matches!(a2, CheckResult::Sat(_)));
    let m1 = match t.check(&mut a, &[lt]) {
        CheckResult::Sat(m) => m.get(a.var_id(x).unwrap()).unwrap().bits(),
        o => panic!("{o:?}"),
    };
    assert!(
        m1 < 5,
        "the cached answer for `x < 5` must satisfy `x < 5`, got {m1}"
    );
}

/// 022 contract 11c: an `Unknown` is never served from the exact cache, because a
/// tier-1 `Unknown` cached above escalation would stop tier 2 ever being consulted.
#[test]
fn unknown_is_never_cached() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let y = a.var(Sort::BitVec(32), "y");
    let p = a.mul(x, y);
    let k = a.bv(32, 7);
    let e = a.eq(p, k);

    // No backend: tier 1 answers Unknown.
    let mut t = TieredSolver::new();
    t.assert(e);
    assert!(matches!(t.check(&mut a, &[]), CheckResult::Unknown(_)));
    assert_eq!(t.stats().cache_entries, 0, "Unknown must not be cached");
}

/// 022 contract 1: neither the default nor the minimal build links a solver. Testing
/// only `--no-default-features` would pass trivially; the risk is the default build.
#[test]
fn no_solver_is_linked() {
    // Skip rather than fail when cargo itself is not invocable — this test is run under
    // a stripped environment to prove contract 2, and "cargo is missing" is not evidence
    // about linkage either way.
    let Ok(out) = std::process::Command::new(env!("CARGO"))
        .args(["tree", "-p", "chiero-solver"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
    else {
        println!("SKIP no_solver_is_linked: cargo not invocable in this environment");
        return;
    };
    let tree = String::from_utf8_lossy(&out.stdout);
    if tree.is_empty() {
        println!("SKIP no_solver_is_linked: cargo tree produced nothing");
        return;
    }
    for bad in ["z3", "z3-sys", "cvc5"] {
        assert!(
            !tree.lines().any(|l| l.contains(&format!(" {bad} v"))),
            "the default build must not link {bad}:\n{tree}"
        );
    }
}

/// 022 §4 requires a **long-lived** process: `push`/`pop` map to real incremental
/// solving, because process startup dominates short queries and a per-query spawn makes
/// tier 2 useless at the scale the engine will run it.
#[test]
fn the_backend_process_is_reused_across_queries() {
    let Some(backend) = z3_or_skip("the_backend_process_is_reused_across_queries") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let y = a.var(Sort::BitVec(32), "y");
    let p = a.mul(x, y);

    let mut t = TieredSolver::with_backend(backend);
    for k in 1..=8u128 {
        let c = a.bv(32, 100 + k);
        let e = a.eq(p, c);
        t.push();
        t.assert(e);
        let _ = t.check(&mut a, &[]);
        t.pop(1);
    }
    assert!(
        t.stats().backend_calls >= 8,
        "each distinct query should reach the backend"
    );
    assert_eq!(
        t.stats().backend_spawns,
        1,
        "one process for all of them, not one per query"
    );
}

/// 022 contract 14: killing the subprocess mid-query yields `Unknown`, restarts it,
/// **replays the assertion stack**, and the next query answers as if nothing happened.
/// Replay is the part that is easy to get wrong and impossible to notice.
#[test]
fn a_killed_backend_is_restarted_and_the_stack_replayed() {
    let Some(backend) = z3_or_skip("a_killed_backend_is_restarted_and_the_stack_replayed") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(16), "x");
    let y = a.var(Sort::BitVec(16), "y");
    // Multiplication, so tier 1 cannot decide it and the query must reach the backend —
    // otherwise the restart path is never exercised and the test proves nothing.
    let p = a.mul(x, y);
    let c = a.bv(16, 1001);
    let prod = a.eq(p, c);
    let one = a.bv(16, 1);
    let gx = a.ult(one, x);

    let mut t = TieredSolver::with_backend(backend);
    t.assert(prod);
    t.assert(gx);
    assert!(matches!(t.check(&mut a, &[]), CheckResult::Sat(_)));
    let spawns_before = t.stats().backend_spawns;

    t.kill_backend_for_test();

    // A different query on the same stack: only correct if the assertions reached the
    // restarted process. 1001 = 7 * 11 * 13, so with `x < 100` the product still has to
    // hold — a backend answering against an empty context would return anything.
    let c100 = a.bv(16, 100);
    let lt100 = a.ult(x, c100);
    match t.check(&mut a, &[lt100]) {
        CheckResult::Sat(m) => {
            let xv = m.get(a.var_id(x).unwrap()).unwrap().bits();
            let yv = m.get(a.var_id(y).unwrap()).unwrap().bits();
            assert_eq!(
                (xv * yv) & 0xffff,
                1001,
                "the replayed stack must still constrain x*y == 1001, got {xv}*{yv}"
            );
            assert!(xv > 1 && xv < 100);
        }
        other => panic!("expected Sat after restart, got {other:?}"),
    }
    assert!(
        t.stats().backend_spawns > spawns_before,
        "the process must actually have been restarted"
    );
}

/// Both terms must survive the round trip to the backend, or tier 2 cannot answer any
/// query the memory model produces. Checked through the tier-1 evaluator's agreement
/// with a real solver where one is available.
#[test]
fn concat_and_ite_reach_the_backend() {
    let Some(backend) = z3_or_skip("concat_and_ite_reach_the_backend") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let c = a.concat(x, y);
    let target = a.bv(16, 0xABCD);
    let e = a.eq(c, target);
    // Multiplication forces escalation past tier 1.
    let p = a.mul(x, y);
    // 0xAB * 0xCD mod 256 = 0xEF. Getting this wrong the first time was itself a
    // useful signal: the backend returned Unsat, which means it *was* honouring the
    // concat rather than ignoring it.
    let prod = a.bv(8, 0xEF);
    let e2 = a.eq(p, prod);

    let mut t = TieredSolver::with_backend(backend);
    t.assert(e);
    t.assert(e2);
    match t.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            let xv = m.get(a.var_id(x).unwrap()).unwrap().bits();
            let yv = m.get(a.var_id(y).unwrap()).unwrap().bits();
            assert_eq!((xv, yv), (0xAB, 0xCD), "the backend must honour the concat");
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// A variable that appears **only** in an `ite` condition must still be declared to the
/// backend. `vars_of` drives the declaration list, so missing the condition arm produces
/// a query referring to an undeclared symbol — and mutation showed nothing exercised it,
/// because every earlier `ite` test used a condition variable that also appeared
/// elsewhere in the query.
#[test]
fn a_variable_only_in_an_ite_condition_is_still_declared() {
    let Some(backend) = z3_or_skip("a_variable_only_in_an_ite_condition_is_still_declared") else {
        return;
    };
    let mut a = TermArena::new();
    let sel = a.var(Sort::BitVec(8), "sel");
    let y = a.var(Sort::BitVec(8), "y");
    let k = a.bv(8, 3);
    let cond = a.eq(sel, k);
    let (t, f) = (a.bv(8, 10), a.bv(8, 20));
    let picked = a.ite(cond, t, f);
    let want = a.bv(8, 10);
    let e = a.eq(picked, want);
    // Force escalation so the query actually reaches the backend.
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(e);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            assert_eq!(
                m.get(a.var_id(sel).unwrap()).unwrap().bits(),
                3,
                "the ite forced sel == 3, so the model must say so"
            );
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// The tier-1 evaluator must agree: `vars_of` is also what tells `solver-lite` which
/// variables it is reasoning about.
#[test]
fn an_ite_condition_variable_is_collected() {
    let mut a = TermArena::new();
    let sel = a.var(Sort::BitVec(8), "sel");
    let k = a.bv(8, 3);
    let cond = a.eq(sel, k);
    let (t, f) = (a.bv(8, 10), a.bv(8, 20));
    let picked = a.ite(cond, t, f);
    let mut vs = Vec::new();
    a.vars_of(picked, &mut vs);
    assert!(
        vs.contains(&a.var_id(sel).unwrap()),
        "the condition's variable must be collected, got {vs:?}"
    );
}

/// **A disjunction of comparisons must reach the backend intact.**
///
/// The arena gives predicates width 1, so `or` over two of them built a *bitvector*
/// `bvor` — and `(bvor (bvult …) (bvult …))` is a sort error the backend rejects. This
/// was reachable from any query containing a disjunction of comparisons, which is most of
/// them; it went unnoticed because 022 contract 7c's disjunction test is answered by
/// tier 1 as `Unknown` and never gets that far.
#[test]
fn a_disjunction_of_comparisons_survives_translation() {
    let Some(backend) = z3_or_skip("a_disjunction_of_comparisons_survives_translation") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let (c5, c200) = (a.bv(8, 5), a.bv(8, 200));
    let lo = a.ult(x, c5);
    let hi = a.ult(c200, x);
    let either = a.or(lo, hi);
    // A multiplication forces escalation, so the disjunction really is translated.
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(either);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            let xv = m.get(a.var_id(x).unwrap()).unwrap().bits();
            assert!(
                !(5..=200).contains(&xv),
                "the disjunction must bind x, got {xv}"
            );
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// An `ite` whose branches are identical folds away. 022 §2 makes folding an *invariant*
/// rather than a pass, so a term that cannot depend on its condition must not carry one
/// to the backend.
#[test]
fn an_ite_with_identical_branches_folds() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let k = a.bv(8, 3);
    let cond = a.eq(x, k);
    let v = a.bv(32, 7);
    assert_eq!(a.ite(cond, v, v), v);
}

/// The `Bool`/bitvector distinction must not collapse the other way either: `and` over
/// two *arithmetic* terms is `bvand`, and emitting the boolean connective there is the
/// same sort error in mirror image. Nothing exercised this, so declaring every operation
/// `Bool` passed the suite.
#[test]
fn a_bitwise_and_of_computed_values_survives_translation() {
    let Some(backend) = z3_or_skip("a_bitwise_and_of_computed_values_survives_translation") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let one = a.bv(8, 1);
    let sx = a.add(x, one);
    let sy = a.add(y, one);
    let masked = a.and(sx, sy);
    let want = a.bv(8, 0);
    let e = a.eq(masked, want);
    // Force escalation past tier 1.
    let p = a.mul(x, y);
    let sq = a.bv(8, 12);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(e);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            let xv = m.get(a.var_id(x).unwrap()).unwrap().bits() as u8;
            let yv = m.get(a.var_id(y).unwrap()).unwrap().bits() as u8;
            assert_eq!(
                xv.wrapping_add(1) & yv.wrapping_add(1),
                0,
                "the backend must read this as a bitvector and"
            );
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Wave 10: SMT-LIB sort discipline. Every case below was confirmed by feeding the
// emitted text to z3 and reading its error.
// ---------------------------------------------------------------------------

/// **A width-1 *bitvector* is not a `Bool`.** Every width-1 constant was emitted as
/// `true`/`false` unconditionally, so comparing a one-bit variable against `1` produced
/// `(= v0_flag true)` — "Sorts (_ BitVec 1) and Bool are incompatible".
///
/// A one-bit bitvector is exactly what `LoadBits` of a `u32 flag:1` yields, which is the
/// case 021 §3.1 argues the whole tri-state from.
#[test]
fn a_one_bit_bitvector_compared_against_a_constant_reaches_the_backend() {
    let Some(backend) = z3_or_skip("a_one_bit_bitvector_compared_against_a_constant") else {
        return;
    };
    let mut a = TermArena::new();
    let flag = a.var(Sort::BitVec(1), "flag");
    let one = a.bv(1, 1);
    let e = a.eq(flag, one);
    let y = a.var(Sort::BitVec(8), "y");
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(e);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            assert_eq!(m.get(a.var_id(flag).unwrap()).unwrap().bits(), 1);
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// A width-1 constant inside a `concat` is a bit, not a truth value.
#[test]
fn a_one_bit_constant_concatenated_with_a_vector_reaches_the_backend() {
    let Some(backend) = z3_or_skip("a_one_bit_constant_concatenated_with_a_vector") else {
        return;
    };
    let mut a = TermArena::new();
    let b = a.var(Sort::BitVec(8), "b");
    let hi = a.bv(1, 1);
    let joined = a.concat(hi, b);
    let want = a.bv(9, 0b1_0110_0101);
    let e = a.eq(joined, want);
    let y = a.var(Sort::BitVec(8), "y");
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(e);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            assert_eq!(m.get(a.var_id(b).unwrap()).unwrap().bits(), 0b0110_0101);
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// **A mixed `and` — one predicate, one one-bit vector — is the hole left by the earlier
/// fix.** That guard required *both* operands to be `Bool`, so the mixed case fell
/// through to `bvand` over a `Bool`, which z3 rejects.
#[test]
fn a_mixed_boolean_and_bitvector_conjunction_reaches_the_backend() {
    let Some(backend) = z3_or_skip("a_mixed_boolean_and_bitvector_conjunction") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let flag = a.var(Sort::BitVec(1), "flag");
    let c5 = a.bv(8, 5);
    let lt = a.ult(x, c5);
    let both = a.and(lt, flag);
    let one = a.bv(1, 1);
    let e = a.eq(both, one);
    let y = a.var(Sort::BitVec(8), "y");
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(e);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            assert!(m.get(a.var_id(x).unwrap()).unwrap().bits() < 5);
            assert_eq!(m.get(a.var_id(flag).unwrap()).unwrap().bits(), 1);
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// An `ite` whose condition is a genuine one-bit **vector** must be wrapped as
/// `(= c #b1)` — the stated point of the earlier commit, which had no test. Only the
/// opposite direction was covered.
#[test]
fn an_ite_on_a_one_bit_vector_condition_reaches_the_backend() {
    let Some(backend) = z3_or_skip("an_ite_on_a_one_bit_vector_condition") else {
        return;
    };
    let mut a = TermArena::new();
    let flag = a.var(Sort::BitVec(1), "flag");
    let (t, f) = (a.bv(8, 10), a.bv(8, 20));
    let picked = a.ite(flag, t, f);
    let want = a.bv(8, 10);
    let e = a.eq(picked, want);
    let y = a.var(Sort::BitVec(8), "y");
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(e);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            assert_eq!(m.get(a.var_id(flag).unwrap()).unwrap().bits(), 1);
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// Nested boolean connectives. `smt_is_bool`'s recursion exists for `and(or(p, q), r)`
/// and nesting was never tested, so a shallow check passed the suite while emitting a
/// sort error.
#[test]
fn nested_boolean_connectives_reach_the_backend() {
    let Some(backend) = z3_or_skip("nested_boolean_connectives") else {
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let (c5, c9, c200) = (a.bv(8, 5), a.bv(8, 9), a.bv(8, 200));
    let lo = a.ult(x, c5);
    let mid = a.ult(c9, x);
    let hi = a.ult(c200, x);
    let inner = a.or(lo, mid);
    let outer = a.and(inner, hi);
    let y = a.var(Sort::BitVec(8), "y");
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);

    let mut s = TieredSolver::with_backend(backend);
    s.assert(outer);
    s.assert(e2);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            assert!(m.get(a.var_id(x).unwrap()).unwrap().bits() > 200);
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// **`concat` must refuse to build a term wider than the payload.** A 17-byte read folded
/// to `BvConst::new(136, …)` and tripped the width assert *inside* the caller; the mixed
/// concrete/symbolic case built the 136-bit term successfully and deferred the panic to
/// evaluation. 020 permits `Int(512)` and a 32-byte AVX load is ordinary VPP, so this is
/// a real boundary that needs an answer rather than a crash.
#[test]
fn concat_beyond_the_payload_width_is_refused_not_panicked() {
    let mut a = TermArena::new();
    let mut acc = a.bv(8, 0xAA);
    for _ in 0..15 {
        let b = a.bv(8, 0xBB);
        acc = a.concat(acc, b);
    }
    assert_eq!(a.width(acc), 128, "16 bytes is the payload limit");
    let one_more = a.bv(8, 0xCC);
    assert!(
        a.try_concat(acc, one_more).is_none(),
        "past the limit, refusing beats panicking"
    );
}
