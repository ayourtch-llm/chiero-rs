//! **030 contract 10: which tests executed a line.**
//!
//! Everything before this reads *one* run's coverage. The index exists to answer the question
//! [032](../../../docs/specs/032-test-selection.md) asks — given a changed line, which tests
//! touched it — and that needs runs to be attributed to tests and unioned.
//!
//! The three fixtures here are one program ingested under two test names and a second program
//! under a third, which is enough to pin the semantics without pretending to a scale fixture:
//! overlap unions, a line only one test reached names only that test, and a line nobody reached
//! is absent rather than empty.
//!
//! # Absent, not empty
//!
//! `tests_for_line` on a line no coverage mentions answers `None`. An empty set would say "no
//! test covers this", which is what 032 acts on by *not* running anything — and the whole point
//! of 030 §1 is that a line gcov never recorded is a line nothing can say that about.

use std::path::PathBuf;

use chiero_gcov::TestId;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// Two runs of the same object under different test names, and a third of another object.
fn three_tests() -> chiero_gcov::CoverageIndex {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("t as test 0");
    chiero_gcov::ingest_native_as(&mut idx, TestId(1), &corpus(), "loop").expect("loop as test 1");
    chiero_gcov::ingest_native_as(&mut idx, TestId(2), &corpus(), "t").expect("t as test 2");
    idx
}

/// **Contract 10.** Overlapping coverage unions; a line one test reached names one test.
#[test]
fn overlapping_tests_union_and_a_single_test_stands_alone() {
    let idx = three_tests();
    assert_eq!(
        idx.tests_for_line("t.c", 3),
        Some(vec![TestId(0), TestId(2)]),
        "both runs of `t` executed it, and the answer is the union in test order"
    );
    assert_eq!(
        idx.tests_for_line("loop.c", 1),
        Some(vec![TestId(1)]),
        "only the second object has this file at all"
    );
    assert_eq!(
        idx.tests_for_line("m.h", 2),
        Some(vec![TestId(0), TestId(2)]),
        "the header function is covered by whatever covered its caller"
    );
}

/// **The macro line is still absent**, through the multi-test index as through everything else.
/// This is the one answer that must never become "no test covers it".
#[test]
fn a_line_nobody_recorded_has_no_test_set_at_all() {
    let idx = three_tests();
    assert_eq!(
        idx.tests_for_line("m.h", 1),
        None,
        "gcov recorded no entry for the macro body, and `Some(vec![])` would tell 032 that no \
         test covers it — a claim nothing in the data supports"
    );
    assert_eq!(idx.tests_for_line("t.c", 999), None);
    assert_eq!(idx.tests_for_line("nosuch.c", 1), None);
}

/// The counts still merge as they did, and the test set is carried beside them rather than
/// instead of them.
#[test]
fn counts_and_test_sets_coexist() {
    let idx = three_tests();
    assert_eq!(idx.line_count("t.c", 3), Some(1));
    assert_eq!(idx.line_count("loop.c", 1), Some(5));
    let mut files: Vec<&str> = idx.files().collect();
    files.sort();
    assert_eq!(files, vec!["loop.c", "m.h", "t.c"]);
}

/// A test that ingested nothing is still a test the index knows about — otherwise a selection
/// that answers "these tests" cannot distinguish "ran and covered nothing" from "never ran".
#[test]
fn the_index_knows_which_tests_contributed() {
    let idx = three_tests();
    assert_eq!(idx.tests(), vec![TestId(0), TestId(1), TestId(2)]);
}
