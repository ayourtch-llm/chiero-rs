//! **A `maybe` nobody discharges is a verdict the engine could have decided.**
//!
//! 021 §3.1's third initialization state exists so a conditional write is neither reported as a
//! definite bug nor silently forgiven, and the variant's own comment says whose job the rest is:
//! "the guard is the engine's to discharge against the path condition, not the memory model's to
//! guess". The engine does not discharge it. Every conditional write therefore ends as `maybe`,
//! including the ones the path decides outright:
//!
//! ```text
//!   ca[(i & 31) + 32] = 7;  return ca[0];        maybe-uninitialized-read
//!   if ((i & 63) == 0) return 0;
//!   ca[i & 63] = 7;         return ca[0];        maybe-uninitialized-read
//! ```
//!
//! Both are **definite**. The first writes only bytes 32–63, so byte 0 is untouched whatever `i`
//! is — provable from the term alone. The second's surviving path asserts `(i & 63) != 0`, so the
//! write cannot land on byte 0 either. Reporting "possibly uninitialized" understates a read of
//! memory that is certainly uninitialized, and a reader who checks and finds it definite learns
//! to distrust the qualifier.
//!
//! This is also what bounds the five waves of symbolic-memory work before it: while every answer
//! is `maybe`, a correct init marking and a wrong one produce the same report, so mutation cannot
//! tell them apart (wave 203 established that with two equivalent survivors).
//!
//! # The shape of the answer
//!
//! Wave 156's, for the third time: ask the solver, and take three outcomes as three outcomes.
//! The guard `t` under path condition `P` — `P ⇒ t` means initialized and silent; `P ∧ t`
//! unsatisfiable means definitely uninitialized; anything else is a genuine `maybe`.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

/// The write cannot reach the byte, so the read is definitely uninitialized.
#[test]
fn a_guard_the_term_refutes_becomes_a_definite_report() {
    if !harness::backend_or_skip("a_guard_the_term_refutes_becomes_a_definite_report") {
        return;
    }
    let f = findings("int probe(int i){ char ca[64]; ca[(i & 31) + 32] = 7; return ca[0]; }");
    assert!(
        f.iter().any(|x| x.starts_with("uninitialized-read")),
        "the write lands in 32..64, so byte 0 is certainly unwritten: {f:?}"
    );
    assert!(
        !f.iter().any(|x| x.starts_with("maybe-uninitialized-read")),
        "and that is a certainty, not a possibility: {f:?}"
    );
}

/// The same when it is the *path condition* that refutes the guard.
///
/// Separate, because the first case is decidable from the offset term alone and this one is not:
/// `i & 63` can be 0 in general, and only the branch taken rules it out. A fix that folded terms
/// without consulting the path would pass the first and fail this.
#[test]
fn a_guard_the_path_refutes_becomes_a_definite_report() {
    if !harness::backend_or_skip("a_guard_the_path_refutes_becomes_a_definite_report") {
        return;
    }
    let f = findings(
        "int probe(int i){ char ca[64]; if ((i & 63) == 0) return 0; ca[i & 63] = 7; \
         return ca[0]; }",
    );
    assert!(
        f.iter().any(|x| x.starts_with("uninitialized-read")),
        "the surviving path asserts `(i & 63) != 0`, so byte 0 is unwritten: {f:?}"
    );
}

/// A guard the path *implies* is discharged the other way: no report at all.
#[test]
fn a_guard_the_path_implies_is_discharged_silently() {
    if !harness::backend_or_skip("a_guard_the_path_implies_is_discharged_silently") {
        return;
    }
    let f = findings(
        "int probe(int i){ char ca[64]; if ((i & 63) != 0) return 0; ca[i & 63] = 7; \
         return ca[0]; }",
    );
    assert!(
        !f.iter().any(|x| x.contains("uninitialized")),
        "the path forces `i & 63 == 0`, so the write hit byte 0: {f:?}"
    );
}

/// And a genuinely undecided guard stays a `maybe`.
///
/// The control that keeps the fix from collapsing the tri-state in either direction — which is
/// the failure 021 §3.1 introduced the third state to prevent.
#[test]
fn an_undecided_guard_stays_a_maybe() {
    let f = findings("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[0]; }");
    assert!(
        f.iter().any(|x| x.starts_with("maybe-uninitialized-read")),
        "`i & 63` may or may not be 0 and nothing decides it: {f:?}"
    );
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "so neither certainty may be claimed: {f:?}"
    );
}
