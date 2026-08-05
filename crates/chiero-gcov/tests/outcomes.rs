//! **030 §6: a test that wrote no coverage is *unknown*, not *uncovered*.**
//!
//! > A test that crashes or is killed writes nothing, which must be recorded as *unknown*
//! > coverage rather than *no* coverage — the two mean opposite things for selection, and
//! > conflating them is how a test-selection tool starts silently dropping the tests most likely
//! > to be broken.
//!
//! This is the same distinction `tests_for_line`'s `None` makes, one level up: there, a line
//! nobody recorded; here, a *test* whose coverage nobody has. Both fail the same way if
//! flattened — as an authoritative "nothing to run".
//!
//! The consequence is concrete. 032 skips a test when nothing it covers has changed. A test that
//! segfaulted covers, as far as the data goes, nothing at all — so flattening makes it *always*
//! skippable, and the test most likely to be catching a real bug is the one that stops running.
//! Hence: any test whose coverage is incomplete joins the always-run set.

use chiero_gcov::{TestId, TestOutcome};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// A test that ran and whose artifacts were ingested has complete coverage.
#[test]
fn an_ingested_passing_test_is_complete() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("t ingests");
    idx.record_outcome(TestId(0), TestOutcome::Passed);
    assert!(idx.coverage_complete(TestId(0)));
    assert_eq!(idx.always_run(), Vec::<TestId>::new());
}

/// **A crash is unknown coverage.** No artifacts, so nothing to ingest — and the test joins the
/// always-run set rather than looking like a test that covers nothing.
#[test]
fn a_crashed_test_is_incomplete_and_always_run() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("t ingests");
    idx.record_outcome(TestId(0), TestOutcome::Passed);
    idx.record_outcome(TestId(1), TestOutcome::Crashed);
    idx.record_outcome(TestId(2), TestOutcome::TimedOut);

    assert!(!idx.coverage_complete(TestId(1)));
    assert!(!idx.coverage_complete(TestId(2)));
    assert_eq!(idx.always_run(), vec![TestId(1), TestId(2)]);
    // And it is *not* mistaken for a test that covers nothing: it has no lines, and asking which
    // tests cover a line it might have touched must not imply anything about it.
    assert_eq!(idx.tests_for_line("t.c", 3), Some(vec![TestId(0)]));
}

/// **A failing test's coverage is still coverage.** It ran, it recorded, and 032 may skip it on
/// the same terms as a passing one — a red test is not an unknown one.
#[test]
fn a_failing_test_that_recorded_is_complete() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("t ingests");
    idx.record_outcome(TestId(0), TestOutcome::Failed);
    assert!(idx.coverage_complete(TestId(0)));
    assert_eq!(idx.always_run(), Vec::<TestId>::new());
}

/// **A test that "passed" but whose artifacts never arrived is incomplete too**, and this is the
/// case a runner gets wrong most easily: the process exited 0, the harness recorded a pass, and
/// `GCOV_PREFIX` pointed somewhere nothing was written.
#[test]
fn a_passing_test_with_no_ingest_is_incomplete() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    idx.record_outcome(TestId(7), TestOutcome::Passed);
    assert!(
        !idx.coverage_complete(TestId(7)),
        "an outcome without an ingest is a claim about the process, not about coverage"
    );
    assert_eq!(idx.always_run(), vec![TestId(7)]);
}

/// A test nobody recorded an outcome for is unknown as well — the index answers about what it was
/// told, and silence is not a pass.
#[test]
fn an_unrecorded_test_is_not_complete() {
    let idx = chiero_gcov::CoverageIndex::default();
    assert!(!idx.coverage_complete(TestId(3)));
}
