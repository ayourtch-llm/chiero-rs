//! **032 §1–§4: the join, and the safety set that keeps it honest.**
//!
//! > 1. An empty diff selects only the always-run set.
//! > 3. Changing one function's body selects exactly the tests covering it and its transitive
//! >    callers.
//! > 4. **The headline contract**: for a macro-body-only change in a header, chiero selects the
//! >    tests that exercise the expansion sites.
//! > 9. A test with no coverage data is always selected.
//! > 11. A test with `coverage_complete == false` is always selected.
//! > 15. Every selected test has ≥ 1 `SelectionReason`.
//!
//! This is where the two halves meet, and §2 says why the join is even well-defined:
//!
//! > **For macro-body changes it is the whole trick.** The changed entity is a macro, which has
//! > no coverage lines of its own; but 031 §3.2 already converted that change into a set of
//! > *impacted functions*, and functions do have coverage. So the intersection is well-defined
//! > precisely because impact closure ran first. A tool that tried to intersect coverage with the
//! > diff directly would find nothing, which is the failure this whole architecture exists to
//! > avoid.
//!
//! # The direction, one more time
//!
//! Every step before this one over-approximates on purpose. Selection is the first place
//! anything is *removed*, and §4 lists what may never be: a test with no coverage is
//! **unmeasured, not unaffected**. If the safety set swallows the suite, that is the correct
//! output — and the report says why in one line rather than burying it.

use chiero_diff::{Program, impact};
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use chiero_select::{Confidence, SelectionReason, select};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// The `t` fixture's source, as chiero-diff sees it: a macro in `m.h` used twice at `t.c:3`.
///
/// The coverage index for it is the committed `t.gcno`/`t.gcda`, so both halves of this test are
/// real artifacts rather than a mock of what one might contain.
fn t_c() -> &'static str {
    "int main (void)\n{\n  M; M;\n  return 0;\n}\n"
}

fn index_with(tests: &[TestId]) -> CoverageIndex {
    let mut idx = CoverageIndex::default();
    for t in tests {
        chiero_gcov::ingest_native_as(&mut idx, *t, &corpus(), "t").expect("the pinned fixture");
        idx.record_outcome(*t, TestOutcome::Passed);
    }
    idx
}

/// **Contract 1.** An empty diff selects nothing but the safety set — which, with a clean index
/// and every test measured, is empty.
#[test]
fn an_empty_diff_selects_only_the_always_run_set() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let sel = select(&impact(&p, &p), &p, &index_with(&[TestId(0)]));
    assert!(
        sel.tests.is_empty(),
        "nothing changed and every test is measured: {:?}",
        sel.tests
    );
    assert_eq!(sel.confidence, Confidence::Full);
}

/// **Contract 3.** A body edit selects the tests covering it.
#[test]
fn a_body_edit_selects_the_tests_that_cover_it() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    let sel = select(&impact(&before, &after), &after, &index_with(&[TestId(0)]));

    assert!(
        sel.tests.contains_key(&TestId(0)),
        "the test executed `main`: {:?}",
        sel.tests
    );
    // **Contract 15.** Every selected test says why.
    for (t, reasons) in &sel.tests {
        assert!(
            !reasons.is_empty(),
            "{t:?} was selected for no stated reason"
        );
    }
}

/// **Contract 9.** A test the index has no coverage for is always selected — unmeasured is not
/// unaffected.
#[test]
fn a_test_with_no_coverage_is_always_selected() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let mut idx = index_with(&[TestId(0)]);
    // A test that ran and produced nothing the index could attribute.
    idx.record_outcome(TestId(7), TestOutcome::Passed);

    let sel = select(&impact(&p, &p), &p, &idx);
    assert!(
        sel.tests.contains_key(&TestId(7)),
        "it has no coverage, so nothing can say it is unaffected: {:?}",
        sel.tests
    );
    assert!(matches!(
        sel.tests[&TestId(7)].first(),
        Some(SelectionReason::AlwaysRun { .. })
    ));
}

/// **Contract 11.** A test that crashed is the most suspicious test in the suite, not the least.
#[test]
fn a_test_that_crashed_is_always_selected() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let mut idx = index_with(&[TestId(0)]);
    idx.record_outcome(TestId(3), TestOutcome::Crashed);

    let sel = select(&impact(&p, &p), &p, &idx);
    assert!(
        sel.tests.contains_key(&TestId(3)),
        "it wrote no counters, so its coverage is unknown: {:?}",
        sel.tests
    );
}

/// **The safety set swallowing the suite is a correct answer**, and the report says so rather
/// than burying it (§4).
#[test]
fn a_partial_impact_set_reduces_confidence_and_says_why() {
    let good = Program::parse("t.c", t_c()).expect("parses");
    // `return ;` would have been *valid* C — a return with no expression — and this test
    // silently passed on the wrong branch until that was noticed. `return x + ;` is not.
    let broken =
        Program::parse("t.c", "int main (void) { int x = 0; return x + ; }\n").expect("recovers");
    let sel = select(&impact(&good, &broken), &broken, &index_with(&[TestId(0)]));

    match &sel.confidence {
        Confidence::Reduced { reasons } => assert!(
            reasons.iter().any(|r| r.contains("could not be parsed")),
            "the reason names the gap itself, not merely a file: {reasons:?}"
        ),
        other => panic!("an unparsed file must reduce confidence, got {other:?}"),
    }
    assert!(
        sel.tests.contains_key(&TestId(0)),
        "and every measured test comes along, because nothing is known"
    );
}

/// A selection is deterministic (contract 16), which is what makes two runs comparable.
#[test]
fn the_selection_is_deterministic() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    let idx = index_with(&[TestId(0), TestId(1)]);
    let a = select(&impact(&before, &after), &after, &idx);
    let b = select(&impact(&before, &after), &after, &idx);
    assert_eq!(
        a.tests.keys().collect::<Vec<_>>(),
        b.tests.keys().collect::<Vec<_>>()
    );
}
