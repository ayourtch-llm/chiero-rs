//! **032 §3 — the only step that removes anything, and the rule that governs it.**
//!
//! > 5. A test is dropped by equivalence refinement only when `prove_equivalent` returns `Exact`.
//! > 6. A solver timeout during refinement keeps every affected test and sets `Reduced`.
//! > 8. Every excluded test carries a proof record naming the refinement and its fidelity.
//!
//! §3 states it as one rule, and it is the whole safety argument of the system:
//!
//! > **A test may be dropped only on an `Exact` proof (023 §7). `Bounded`, `Approximated`,
//! > `Unknown`, or a solver timeout all mean "keep".**
//!
//! Everything upstream over-approximates on purpose — 031 §4's gaps widen, 032 §4's safety set is
//! unconditional. All of that care is undone by one refinement that drops a test on a
//! *probably*. So the discipline is built before any prover exists: the seam takes a verdict, and
//! only one verdict spends it.
//!
//! # Why this lands before a solver
//!
//! The default prover proves nothing, so nothing is dropped and the selection stays the superset
//! it already was — the shape `chiero-gcov`'s `MarchResolver` used for the same reason. What this
//! wave buys is that a later solver **cannot** be wired in without producing a verdict and a
//! proof record, because there is no other way to remove a test.

use chiero_diff::{Program, impact};
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use chiero_select::{Equivalence, Fidelity, Prover, Suite, select_refined, select_with};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn t_c() -> &'static str {
    "int main (void)\n{\n  M; M;\n  return 0;\n}\n"
}

fn edited() -> &'static str {
    "int main (void)\n{\n  M; M;\n  return 1;\n}\n"
}

fn index() -> CoverageIndex {
    let mut idx = CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("fixture");
    idx.record_outcome(TestId(0), TestOutcome::Passed);
    idx
}

/// A prover that always returns the same verdict, for pinning what each verdict does.
struct Always(Equivalence);

impl Prover for Always {
    fn prove_equivalent(&mut self, _entity: &chiero_diff::Entity) -> Equivalence {
        self.0.clone()
    }
}

/// **Contract 5.** `Exact` is the only verdict that spends.
#[test]
fn only_an_exact_proof_drops_a_test() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after = Program::parse("t.c", edited()).expect("parses");
    let set = impact(&before, &after);

    let kept = select_with(&set, &after, &index(), &Suite::default());
    assert!(
        kept.tests.contains_key(&TestId(0)),
        "the baseline selects it"
    );

    let dropped = select_refined(
        &set,
        &after,
        &index(),
        &Suite::default(),
        &mut Always(Equivalence::Equivalent {
            fidelity: Fidelity::Exact,
        }),
    );
    assert!(
        !dropped.tests.contains_key(&TestId(0)),
        "proved equivalent: the entity is gone and so is the test that came only from it"
    );
}

/// **Contract 5, the other direction.** Every weaker verdict keeps the test.
///
/// This is the test that matters. A refinement that drops on `Bounded` would look identical in
/// every passing case and be wrong exactly when it counted.
#[test]
fn every_verdict_short_of_exact_keeps_the_test() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after = Program::parse("t.c", edited()).expect("parses");
    let set = impact(&before, &after);

    for verdict in [
        Equivalence::Equivalent {
            fidelity: Fidelity::Bounded,
        },
        Equivalence::Equivalent {
            fidelity: Fidelity::Approximated,
        },
        Equivalence::Equivalent {
            fidelity: Fidelity::Unknown,
        },
        Equivalence::Differs,
        Equivalence::TimedOut,
        Equivalence::NotAttempted,
    ] {
        let sel = select_refined(
            &set,
            &after,
            &index(),
            &Suite::default(),
            &mut Always(verdict.clone()),
        );
        assert!(
            sel.tests.contains_key(&TestId(0)),
            "{verdict:?} is not an Exact proof and must not drop anything"
        );
    }
}

/// **Contract 6.** A timeout is not a proof, and it is not silence either: the confidence drops.
#[test]
fn a_timeout_keeps_everything_and_reduces_confidence() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after = Program::parse("t.c", edited()).expect("parses");
    let sel = select_refined(
        &impact(&before, &after),
        &after,
        &index(),
        &Suite::default(),
        &mut Always(Equivalence::TimedOut),
    );

    assert!(sel.tests.contains_key(&TestId(0)));
    match &sel.confidence {
        chiero_select::Confidence::Reduced { reasons } => assert!(
            reasons.iter().any(|r| r.contains("timed out")),
            "a refinement that could not finish is a caveat, not a non-event: {reasons:?}"
        ),
        other => panic!("expected Reduced, got {other:?}"),
    }
}

/// **Contract 8.** An exclusion carries the refinement that made it and the fidelity behind it,
/// so a selection can be audited *after* a regression escapes.
#[test]
fn every_exclusion_carries_its_proof() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after = Program::parse("t.c", edited()).expect("parses");
    let sel = select_refined(
        &impact(&before, &after),
        &after,
        &index(),
        &Suite::default(),
        &mut Always(Equivalence::Equivalent {
            fidelity: Fidelity::Exact,
        }),
    );

    assert!(!sel.excluded.is_empty(), "something was dropped");
    for e in &sel.excluded {
        assert_eq!(
            e.fidelity,
            Fidelity::Exact,
            "nothing weaker may appear here"
        );
        assert!(
            !e.refinement.is_empty() && !e.entity.is_empty(),
            "an exclusion without a named refinement cannot be audited: {e:?}"
        );
    }
    // And the report says so, beside the reduction.
    let text = sel.render();
    assert!(text.contains("EXCLUDED"), "{text}");
}

/// **The default prover proves nothing**, so `select` is unchanged and the selection stays the
/// superset it was. A refinement seam that defaulted to *attempting* proofs would change the
/// meaning of every existing call.
#[test]
fn the_default_prover_removes_nothing() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after = Program::parse("t.c", edited()).expect("parses");
    let set = impact(&before, &after);
    let plain = select_with(&set, &after, &index(), &Suite::default());
    let refined = select_refined(
        &set,
        &after,
        &index(),
        &Suite::default(),
        &mut chiero_select::NoProver,
    );
    assert_eq!(
        plain.tests.keys().collect::<Vec<_>>(),
        refined.tests.keys().collect::<Vec<_>>()
    );
    assert!(refined.excluded.is_empty());
}
