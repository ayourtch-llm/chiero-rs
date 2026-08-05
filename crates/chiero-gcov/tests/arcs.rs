//! **030 contracts 7 and 4: arc-level coverage, and where it is not available.**
//!
//! > `FAKE` arcs are excluded from arc-level queries but included in the conservation solve.
//!
//! A `FAKE` arc runs to the exit block from a call that may not return. It is not control flow the
//! program can take, so selecting tests by it would attribute a `noreturn` edge to a test that
//! never went near one — and it is indispensable to the solve, because conservation at the block
//! it leaves does not balance without it. `t.gcno`'s `main` has two of them, so this fixture
//! decides the question rather than merely being consistent with an answer.
//!
//! # Contract 4, and how it is met
//!
//! > `tests_for_arc` on such an index is a compile-time-unavailable operation, not a runtime empty
//! > answer.
//!
//! There is no `tests_for_arc` on [`chiero_gcov::CoverageIndex`] at all. Arc coverage is a
//! separate value that only the native path can produce, so asking a JSON-derived index for arcs
//! is a compile error rather than a `None` a caller can accidentally treat as "no tests".

use chiero_gcov::TestId;
use chiero_gcov::native::FuncKey;
use std::path::PathBuf;

/// `main` of the `t` fixture, at `t.c:3`.
fn main_key() -> FuncKey {
    FuncKey::new("t.c", "main", 3)
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// **Contract 7, the exclusion half.** The query surface holds the real arcs and not the fake
/// ones.
#[test]
fn fake_arcs_are_absent_from_the_query_surface() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    let main: Vec<(u32, u32)> = cov.arcs_of(&main_key()).expect("`main` is in the fixture");
    assert_eq!(
        main,
        vec![(0, 2), (2, 3), (3, 4), (4, 5), (5, 1)],
        "seven arcs decode and the two `FAKE` ones — 2->1 and 3->1, to the exit block from calls \
         that may not return — are not control flow a test can be selected by"
    );
}

/// **Contract 7, the inclusion half.** The counts are right, which they cannot be unless the
/// `FAKE` arcs took part in conservation: `main`'s block 3 has one of them, and dropping it leaves
/// that block's flow unbalanced.
#[test]
fn fake_arcs_still_participate_in_the_solve() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    assert_eq!(cov.arc_count(&main_key(), (0, 2)), Some(1));
    assert_eq!(cov.arc_count(&main_key(), (3, 4)), Some(1));
    assert_eq!(
        cov.arc_count(&main_key(), (2, 1)),
        None,
        "a fake arc has a count internally and is not answerable *as an arc*"
    );
    // The whole-index line counts come from the same solve, and contract 5 pins them against gcov.
    assert_eq!(cov.index().line_count("t.c", 3), Some(1));
}

/// Arc-level test attribution, which is what 032 selects on when it has it.
#[test]
fn arcs_carry_the_tests_that_took_them() {
    let mut cov = chiero_gcov::native::ArcCoverage::default();
    chiero_gcov::native::arc_coverage_into(&mut cov, TestId(0), &corpus(), "t").expect("t as 0");
    chiero_gcov::native::arc_coverage_into(&mut cov, TestId(1), &corpus(), "t").expect("t as 1");

    assert_eq!(
        cov.tests_for_arc(&main_key(), (0, 2)),
        Some(vec![TestId(0), TestId(1)]),
        "both runs took the entry arc"
    );
    assert_eq!(
        cov.tests_for_arc(&main_key(), (2, 1)),
        None,
        "and a fake arc is not an arc to be selected by"
    );
    assert_eq!(
        cov.tests_for_arc(&main_key(), (9, 9)),
        None,
        "nor is an arc the graph does not have"
    );
}

/// An arc a test's run never took is recorded for that test with a count of zero rather than
/// omitted — the same absence-versus-zero rule the line index follows, one level down.
#[test]
fn an_untaken_arc_is_zero_rather_than_missing() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "loop").expect("loop decodes");
    let arcs = cov
        .arcs_of(&FuncKey::new("loop.c", "f", 1))
        .expect("`f` is in the loop fixture");
    assert!(!arcs.is_empty());
    for a in arcs {
        assert!(
            cov.arc_count(&FuncKey::new("loop.c", "f", 1), a).is_some(),
            "every arc the graph has must have a count, including the ones this run did not take"
        );
    }
}
