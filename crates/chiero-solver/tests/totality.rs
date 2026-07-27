//! **Division and remainder are total** — 022 §2 and contracts 19a–19d.
//!
//! Covers: 022 contracts 19a, 19b, 19c, 19d, and 15.
//!
//! §2: "a solver that can return 'no value' poisons every downstream cache", so the zero
//! cases have answers rather than partiality. They are **not uniform**, and getting them
//! wrong is uniquely dangerous here: the constant folder and the independent evaluator
//! (§3) would share the error, so the evaluator would *validate* a model built on wrong
//! semantics and nothing short of a tier-1/tier-2 disagreement would show it.
//!
//! Each case is therefore checked three ways where possible — folded, evaluated, and
//! against z3 itself when one is installed.

use chiero_solver::*;

/// Fold `op(x, 0)` at 8 bits through the arena's constant folder.
fn folded(op: impl Fn(&mut TermArena, Term, Term) -> Term, x: i128) -> u128 {
    let mut a = TermArena::new();
    let xt = a.bv(8, (x as u8) as u128);
    let zero = a.bv(8, 0);
    let t = op(&mut a, xt, zero);
    a.eval_ground(t)
        .expect("total: there is always a value")
        .bits()
}

/// The same expression through a real backend, as an equality it must confirm.
fn z3_agrees(op: impl Fn(&mut TermArena, Term, Term) -> Term, x: i128, want: u128) -> Option<bool> {
    let backend = SmtLib::discover()?;
    let mut a = TermArena::new();
    // A *variable* equal to `x`, so the query cannot be answered by folding it away.
    let v = a.var(Sort::BitVec(8), "x");
    let xc = a.bv(8, (x as u8) as u128);
    let pin = a.eq(v, xc);
    let zero = a.bv(8, 0);
    let t = op(&mut a, v, zero);
    let wc = a.bv(8, want);
    let same = a.eq(t, wc);
    let differs = a.not(same);
    let mut s = TieredSolver::with_backend(backend);
    // "Can the result differ from `want` while `x` is pinned?" must be unsatisfiable.
    Some(matches!(
        s.check(&mut a, &[pin, differs]),
        CheckResult::Unsat
    ))
}

/// **022 contract 19a.** `bvudiv x 0` is all ones, whatever `x` is.
#[test]
fn unsigned_division_by_zero_is_all_ones() {
    for x in [0i128, 1, 5, 127, -1, -5, -128] {
        assert_eq!(folded(|a, p, q| a.udiv(p, q), x), 0xff, "bvudiv {x} 0");
        if let Some(ok) = z3_agrees(|a, p, q| a.udiv(p, q), x, 0xff) {
            assert!(ok, "z3 disagrees about bvudiv {x} 0");
        }
    }
}

/// **022 contract 19b.** `bvsdiv x 0` is all ones for `x >=s 0` and **`1`** for `x <s 0`.
/// One rule for both signs is wrong for half the inputs, and it is the rule the spec
/// itself carried until z3 was asked.
#[test]
fn signed_division_by_zero_depends_on_the_sign_of_the_dividend() {
    for (x, want) in [
        (0i128, 0xff),
        (5, 0xff),
        (127, 0xff),
        (-1, 1),
        (-5, 1),
        (-128, 1),
    ] {
        assert_eq!(folded(|a, p, q| a.sdiv(p, q), x), want, "bvsdiv {x} 0");
        if let Some(ok) = z3_agrees(|a, p, q| a.sdiv(p, q), x, want) {
            assert!(ok, "z3 disagrees about bvsdiv {x} 0");
        }
    }
}

/// **022 contract 19c.** `bvurem x 0` and `bvsrem x 0` give back **the dividend**. This
/// is the case a uniform all-ones rule gets wrong most visibly: `x % 0` reading as `-1`
/// turns a length calculation into a huge one.
#[test]
fn remainder_by_zero_is_the_dividend() {
    for x in [0i128, 1, 5, 127, -1, -5, -128] {
        let want = (x as u8) as u128;
        assert_eq!(folded(|a, p, q| a.urem(p, q), x), want, "bvurem {x} 0");
        assert_eq!(folded(|a, p, q| a.srem(p, q), x), want, "bvsrem {x} 0");
        if let Some(ok) = z3_agrees(|a, p, q| a.urem(p, q), x, want) {
            assert!(ok, "z3 disagrees about bvurem {x} 0");
        }
        if let Some(ok) = z3_agrees(|a, p, q| a.srem(p, q), x, want) {
            assert!(ok, "z3 disagrees about bvsrem {x} 0");
        }
    }
}

/// **022 contract 19d.** None of it is an error, and no term carries partiality: division
/// is total and undefined behaviour is 020 §4.1's business. A solver that could answer
/// "no value" would poison every cache downstream of it.
#[test]
fn no_zero_case_is_an_error() {
    let mut a = TermArena::new();
    let x = a.bv(8, 5);
    let zero = a.bv(8, 0);
    for t in [
        a.udiv(x, zero),
        a.sdiv(x, zero),
        a.urem(x, zero),
        a.srem(x, zero),
    ] {
        assert!(
            a.eval_ground(t).is_ok(),
            "every zero case has a value, not an error"
        );
    }
    // And through the solver, a division by zero is an ordinary satisfiable query.
    let v = a.var(Sort::BitVec(8), "y");
    let d = a.udiv(v, zero);
    let all_ones = a.bv(8, 0xff);
    let eq = a.eq(d, all_ones);
    let mut s = match SmtLib::discover() {
        Some(b) => TieredSolver::with_backend(b),
        None => TieredSolver::new(),
    };
    // **`Sat`, not "anything but `Unsat`".** The weaker form is satisfied by `Unknown`,
    // which is what a run with no backend installed returns — so it passed without ever
    // testing what its message claimed. Found by review.
    assert!(
        matches!(s.check(&mut a, &[eq]), CheckResult::Sat(_)),
        "`y / 0 == 0xff` holds for every y, so it is satisfiable"
    );
}

/// **022 contract 15.** "A backend emitting garbage yields `Unknown(BackendError)` and
/// increments the error counter; no panic, no state corruption."
///
/// A wrong answer from a backend is the one failure the tiered solver cannot detect by
/// reasoning — §3's independent evaluator exists for exactly this — so what it does with
/// an *unparseable* one has to be pinned rather than assumed.
#[test]
fn a_backend_emitting_garbage_is_an_error_not_an_answer() {
    // A path unique to this process: a fixed shared one races two concurrent runs of the
    // suite against each other, and the loser reads a half-written script. Found by
    // review.
    let dir = std::env::temp_dir().join(format!("chiero-garbage-backend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script = dir.join("garbage.sh");
    std::fs::write(&script, "#!/bin/sh\necho 'not an smt answer'\nexit 0\n").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let mut a = TermArena::new();
    let v = a.var(Sort::BitVec(8), "x");
    let c = a.bv(8, 7);
    let eq = a.eq(v, c);
    let mut s = TieredSolver::with_backend(SmtLib::at(&script));
    // Tier 1 decides equalities, so the query has to be one it escalates on.
    let w = a.var(Sort::BitVec(8), "w");
    let prod = a.mul(v, w);
    let hard = a.eq(prod, c);
    let r = s.check(&mut a, &[eq, hard]);
    assert!(
        matches!(r, CheckResult::Unknown(UnknownReason::BackendError(_))),
        "garbage is not an answer: {r:?}"
    );
    assert_eq!(
        s.stats().backend_errors,
        1,
        "and it is counted, so a run that quietly stopped deciding anything is visible"
    );
    // No state corruption: the solver still answers the next query.
    let easy = a.eq(v, c);
    assert!(
        !matches!(s.check(&mut a, &[easy]), CheckResult::Unknown(_)),
        "tier 1 still decides what it could decide before"
    );
}

// ⚠️ **Owed: contract 15's other two causes.** `backend_errors` documents itself as
// counting three things — unparseable output, a model that failed independent evaluation,
// and a dead process — and only the first is pinned here, so dropping the increment on
// the second passes the whole suite (review demonstrated it). Testing it needs a backend
// that speaks the SMT-LIB session protocol correctly and *lies* — answers `sat` with a
// model that does not satisfy the query — and a first attempt with a shell script hung,
// because a fake that does not answer every command exactly is indistinguishable from a
// slow one. The honest note is better than the test that was going to be written badly.
