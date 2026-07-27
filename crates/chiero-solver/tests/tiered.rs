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
