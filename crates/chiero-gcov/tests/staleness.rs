//! **030 contract 18 and §7: coverage is a claim about a specific source state.**
//!
//! > Modifying a source file after ingest makes `validity()` return `Stale` naming that file.
//!
//! > There is no "probably fine" path. `CoverageIndex::validity()` returns
//! > `Fresh | Stale { files } | Partial { missing_tests }`, and 032 is required to pattern-match
//! > on it.
//!
//! Acting on stale coverage is worse than having none: it is a confident answer about code that
//! no longer exists. A line was covered by four tests, someone rewrote the function, and the
//! index still names those four — so 032 runs them and skips the rest, on evidence about a
//! program nobody can build any more.
//!
//! # The decisions this test fixes, and why
//!
//! **The hash is FNV-1a-128, not Blake3.** 030 §7 names Blake3 and this deliberately does not
//! add it. What a hash must do here is notice an *accidental* difference between two versions of
//! one file, and at 128 bits a chance collision is ~2⁻¹²⁸ — the failure mode that matters is
//! having no hash at all, or comparing modification times. What Blake3 additionally buys is
//! resistance to someone *constructing* a collision to make their change look covered, and that
//! is a threat model this project has not adopted anywhere else. The type is a private detail of
//! this crate, so if it is adopted the swap is one function. **This is a judgement, not a
//! measurement** — unlike contract 11, where the benchmark had something to say.
//!
//! **The hash is taken at ingest, per file, by reading the path gcov recorded.** There is nothing
//! in a `.gcno` or `.gcda` that pins the source text: the stamp pairs the two artifacts, and the
//! per-function checksums change only when the *CFG* changes, so both are blind to an edit that
//! rewrites a comparison or a constant. That means ingest does file IO it did not do before, and
//! that an index whose sources have moved cannot check itself — recorded on `validity` rather
//! than discovered.
//!
//! **`Stale` wins over `Partial` when both hold.** They are not the same severity: `Partial` says
//! some tests are unaccounted for and 032 must add them to the always-run set, while `Stale` says
//! the answers are about the wrong source and none of them can be trusted. An enum can only say
//! one, so it says the one that changes what a caller may do with the rest.

use chiero_gcov::{TestId, Validity};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// A scratch copy of one fixture, so a test may edit the source without touching the corpus.
///
/// The `.gcno` records the source path as an absolute path of the machine that built it, so the
/// copy keeps the *name* and the test tells the index where to look — which is the same thing a
/// caller with a relocated build tree has to do.
fn scratch(stem: &str, dir: &str) -> PathBuf {
    let to = std::env::temp_dir().join(format!("chiero-staleness-{dir}"));
    let _ = std::fs::remove_dir_all(&to);
    std::fs::create_dir_all(&to).expect("scratch directory");
    for ext in ["gcno", "gcda", "c", "h"] {
        let from = corpus().join(format!("{stem}.{ext}"));
        if from.exists() {
            std::fs::copy(&from, to.join(format!("{stem}.{ext}"))).expect("copy fixture");
        }
    }
    to
}

/// An index over one object, with its sources hashed from `dir`.
fn index_of(dir: &PathBuf, stem: &str) -> chiero_gcov::CoverageIndex {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), dir, stem).expect("the fixture decodes");
    idx.record_sources(dir);
    idx
}

/// An index whose sources are untouched is `Fresh`.
#[test]
fn an_unmodified_tree_is_fresh() {
    let dir = scratch("loop", "fresh");
    assert_eq!(index_of(&dir, "loop").validity(&dir), Validity::Fresh);
}

/// **Contract 18.** Editing a source after ingest makes the index `Stale`, and it names the file.
#[test]
fn editing_a_source_after_ingest_is_stale_and_names_the_file() {
    let dir = scratch("loop", "stale");
    let idx = index_of(&dir, "loop");

    let src = dir.join("loop.c");
    let text = std::fs::read_to_string(&src).expect("the fixture source");
    std::fs::write(&src, format!("{text}\n/* a comment nobody compiled */\n")).expect("edit");

    match idx.validity(&dir) {
        Validity::Stale { files } => assert_eq!(files, vec!["loop.c".to_string()]),
        other => panic!("expected Stale naming loop.c, got {other:?}"),
    }
}

/// **A comment is a change.** The `.gcno`'s per-function checksums cover the CFG, so an edit that
/// does not move a branch leaves them equal — which is exactly the edit a checksum-based
/// staleness check would call fresh. Two lines of one file differing by whitespace produce
/// different hashes and the same checksums.
#[test]
fn an_edit_that_changes_no_control_flow_is_still_a_change() {
    let dir = scratch("loop", "comment");
    let idx = index_of(&dir, "loop");

    let src = dir.join("loop.c");
    let text = std::fs::read_to_string(&src).expect("the fixture source");
    // Whitespace only: same tokens, same CFG, same checksums, different file.
    std::fs::write(&src, text.replace('\n', "\n\n")).expect("edit");

    assert!(
        matches!(idx.validity(&dir), Validity::Stale { .. }),
        "a hash of the text is the point; anything derived from the CFG cannot see this"
    );
}

/// A source that has been deleted is stale, not fresh. The index describes code that is gone, and
/// "cannot check" must not read as "checked and fine".
#[test]
fn a_deleted_source_is_stale() {
    let dir = scratch("loop", "deleted");
    let idx = index_of(&dir, "loop");
    std::fs::remove_file(dir.join("loop.c")).expect("delete");
    assert!(matches!(idx.validity(&dir), Validity::Stale { .. }));
}

/// **`Partial` when a test the index knows about produced no coverage**, which is §7's "selection
/// falls back to always-run" made checkable.
#[test]
fn a_test_that_produced_no_coverage_makes_the_index_partial() {
    let dir = scratch("loop", "partial");
    let mut idx = index_of(&dir, "loop");
    idx.record_outcome(TestId(1), chiero_gcov::TestOutcome::Crashed);

    match idx.validity(&dir) {
        Validity::Partial { missing_tests } => assert_eq!(missing_tests, vec![TestId(1)]),
        other => panic!("expected Partial naming the crashed test, got {other:?}"),
    }
    assert!(
        idx.always_run().contains(&TestId(1)),
        "and the two must agree: Partial is what always_run is derived from"
    );
}

/// **`Stale` outranks `Partial`.** They are not the same severity, and an enum can only say one:
/// `Partial` means some tests are unaccounted for, `Stale` means none of the answers are about
/// the source in front of you.
#[test]
fn stale_outranks_partial() {
    let dir = scratch("loop", "both");
    let mut idx = index_of(&dir, "loop");
    idx.record_outcome(TestId(1), chiero_gcov::TestOutcome::Crashed);

    let src = dir.join("loop.c");
    let text = std::fs::read_to_string(&src).expect("the fixture source");
    std::fs::write(&src, format!("{text}\nint added (void) {{ return 1; }}\n")).expect("edit");

    assert!(
        matches!(idx.validity(&dir), Validity::Stale { .. }),
        "both hold; the one that says the rest cannot be trusted is the one to report"
    );
}
