//! **The third outcome, exercised.**
//!
//! Three places ask the solver a question and take three answers: the initialization guard (wave
//! 204), the symbolic-read guard (205) and the forced-overflow query (215). Each treats `Unknown`
//! as "nobody settled this" — and **nothing in the tree ever produced an `Unknown`**, so all three
//! `Unknown` arms were unreachable code with a comment on them. Section 9 has carried the mutants
//! since wave 204:
//!
//! ```text
//!   unknown-is-definite   Unknown collapses into a definite report   SURVIVED
//!   unknown-is-clean      Unknown collapses into silence             SURVIVED
//!   unknown-reports       Unknown becomes an overflow finding        SURVIVED
//! ```
//!
//! A tri-state nothing exercises is a two-state, and the arm that exists to stop a wrong certainty
//! is exactly the one worth pinning.
//!
//! # The seam already existed
//!
//! `SolverTier::LiteOnly` refuses to look for a backend (022 §4), and tier 1 is deliberately
//! incomplete — 022 §3 permits it to be *unable* but never wrong. So a question needing array
//! theory or wide multiplication comes back `Unknown` from a run configured that way, with no
//! injection hook and no test-only code path. §9 recorded this as needing "a seam to inject an
//! `Unknown`"; the seam is a public builder method four existing tests already use.
//!
//! # What each assertion is about
//!
//! Not "the answer is vague" but *which* vague answer. An undecided guard must come back
//! `maybe-uninitialized-read` — neither dropped (a reader never learns the memory might be
//! unwritten) nor promoted to definite (a report chiero cannot support). An undecided overflow must
//! come back as nothing at all, because 023 §7's honest answer for "I could not tell" on an
//! *addition* is silence rather than a finding on every arithmetic instruction in the program.

mod harness;

use chiero_exec::{Engine, SolverTier};
use chiero_solver::TermArena;

/// Run with tier 1 alone, so anything it cannot decide is an honest `Unknown`.
fn lite(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    Engine::new(&m)
        .with_entry("probe")
        .with_solver(SolverTier::LiteOnly)
        .run(&mut arena)
        .findings()
}

/// The same program with whatever backend the machine has, for contrast.
fn discovered(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena)
        .findings()
}

/// Five elements of `struct P { char a; long b; }`, every field written.
///
/// Wave 214's fixture: 280 unwritten bits is past `EXPAND_LIMIT`, so the guard is an opaque
/// `select` and deciding it needs array theory — which is precisely what tier 1 does not have.
fn padded() -> String {
    let mut src =
        String::from("struct P { char a; long b; };\nint probe(int i){ struct P pa[5];\n");
    for k in 0..5 {
        src.push_str(&format!("pa[{k}].a = 1; pa[{k}].b = 2;\n"));
    }
    src.push_str("char *p = (char *)pa;\nreturn p[i & 63];\n}\n");
    src
}

/// **An undecided initialization guard is a `maybe`, and says the solver could not settle it.**
///
/// Both halves matter. The kind must be the weak one — a definite `uninitialized-read` would be a
/// claim chiero cannot support — and the *message* must be `UninitializedSymbolic`'s own, which is
/// the only wording that admits no offset. Wave 205 wrote that arm and recorded it as unreachable;
/// this is it running.
#[test]
fn an_undecided_initialization_guard_is_a_maybe() {
    let f = lite(&padded());
    let m = f
        .iter()
        .find(|m| m.starts_with("maybe-uninitialized-read"))
        .unwrap_or_else(|| panic!("an undecided guard must still be reported, weakly: {f:?}"));
    assert!(
        m.contains("could not settle"),
        "and it must say that nobody settled it, rather than naming an offset it cannot \
         justify: {m:?}"
    );
    assert!(
        !f.iter().any(|m| m.starts_with("uninitialized-read")),
        "`Unknown` is not a definite fault: {f:?}"
    );
}

/// With a backend, the same program gets a *better* answer.
///
/// The contrast is the point: this is one program whose report improves when a solver is
/// available, which is what 022 §4's tiering is for. It also stops the assertion above from being
/// satisfied by a fix that made every guard undecided.
#[test]
fn the_same_guard_is_settled_when_a_solver_is_available() {
    if !harness::backend_or_skip("the_same_guard_is_settled_when_a_solver_is_available") {
        return;
    }
    let f = discovered(&padded());
    assert!(
        f.iter()
            .any(|m| m.starts_with("maybe-uninitialized-read") && !m.contains("could not settle")),
        "with a backend the guard resolves to a real byte and offset: {f:?}"
    );
}

/// **`Unknown` weakens a verdict; it does not fabricate or drop one.**
///
/// The sharpest thing here, and it came out of a control of mine that guessed wrong. This is wave
/// 204's `a_guard_the_term_refutes_becomes_a_definite_report` fixture — `ca[(i & 31) + 32]` cannot
/// reach byte 0 — and refuting the guard needs arithmetic tier 1 does not do:
///
/// ```text
///   LiteOnly    maybe-uninitialized-read: ... written only under a condition that may not hold
///   Discovered  uninitialized-read: ... which was never written
/// ```
///
/// One program, two truthful answers, and the weaker one is what a run without a backend is
/// entitled to claim. That is 022 §4's tiering doing exactly what it is for, and it is the pair
/// that would catch `Unknown` collapsing in either direction: to silence (no report at all) or to
/// certainty (the definite kind without the evidence).
#[test]
fn an_undecided_guard_weakens_the_verdict_rather_than_removing_it() {
    if !harness::backend_or_skip("an_undecided_guard_weakens_the_verdict_rather_than_removing_it") {
        return;
    }
    let src = "int probe(int i){ char ca[64]; ca[(i & 31) + 32] = 7; return ca[0]; }";
    let l = lite(src);
    assert!(
        l.iter().any(|m| m.starts_with("maybe-uninitialized-read")),
        "tier 1 cannot refute this guard, so the report is the weak one — not nothing: {l:?}"
    );
    let d = discovered(src);
    assert!(
        d.iter().any(|m| m.starts_with("uninitialized-read")),
        "and with a backend the same guard is refuted outright: {d:?}"
    );
}

/// Tier 1 still reports what needs no solver at all. **The control.**
///
/// Without this, a fix that made `LiteOnly` degrade everything would satisfy every assertion above
/// while the run had quietly stopped answering. An unwritten local needs no query: the init mask
/// says `No` and the guard folds before anything is asked.
#[test]
fn tier_one_still_reports_what_needs_no_query() {
    let f = lite("int probe(void){ char ca[8]; return ca[0]; }");
    assert!(
        f.iter().any(|m| m.starts_with("uninitialized-read")),
        "nothing wrote this byte and no solver is needed to know it: {f:?}"
    );
}

/// **An `Unknown` on the *first* query is a `maybe` too, not silence.**
///
/// The discharge asks two questions — is the guard implied, and is it refuted — and the arms take
/// their answers separately, so each needs its own undecided fixture. This is wave 204's
/// `a_guard_the_path_implies_is_discharged_silently`: the path pins `i & 31` to zero, so the write
/// certainly hit byte 0 and a run with a backend says nothing at all. Tier 1 cannot prove the
/// implication, and the honest answer there is the weak report rather than the silence a proof
/// would earn.
///
/// Mutation is why this exists separately: relaxing the *first* `Unsat` to "not `Sat`" — treating
/// an undecided guard as implied and dropping the fault — survived every other test in this file,
/// because in those fixtures the first query comes back `Sat` and only the second is undecided.
#[test]
fn an_undecided_implication_is_a_maybe_and_not_silence() {
    if !harness::backend_or_skip("an_undecided_implication_is_a_maybe_and_not_silence") {
        return;
    }
    let src = "int probe(int i){ char ca[64]; if ((i & 31) != 0) return 0; ca[i & 31] = 7; \
               return ca[0]; }";
    let l = lite(src);
    assert!(
        l.iter().any(|m| m.starts_with("maybe-uninitialized-read")),
        "tier 1 cannot prove the write reached byte 0, and silence would claim it did: {l:?}"
    );
    let d = discovered(src);
    assert!(
        d.is_empty(),
        "with a backend the implication is proved and there is nothing to report: {d:?}"
    );
}
