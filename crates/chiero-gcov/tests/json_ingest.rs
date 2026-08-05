//! **030 contracts 1–4: the JSON ingest, and the fact the whole project rests on.**
//!
//! The fixture in `tests/corpus/coverage/` is gcov's own output, regenerated against gcc 13.3.0
//! and committed with the two source files that produced it. Its README says how; this file says
//! what it must mean.
//!
//! # Contract 2 is the crown jewel
//!
//! `ADD1` is expanded **twice** at `t.c:3`, and `m.h:1` — the macro's definition — receives no
//! coverage record at all. Not a zero count: no entry. The `static inline` on `m.h:2` beside it
//! is covered normally, at its *definition*.
//!
//! That boundary is the entire justification for chiero owning a preprocessor: "which tests
//! cover this line of `vec.h`" is answerable for a function and unanswerable for a macro, from
//! coverage data alone, at any level of post-processing. If a future gcc starts recording the
//! macro line, this test fails loudly and the justification gets re-argued rather than assumed.

use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// **Contract 1.** The counts are the fixture's, and the versions are recorded.
#[test]
fn ingesting_the_fixture_yields_its_counts_and_versions() {
    let idx = chiero_gcov::ingest_json(&corpus(), "t").expect("the fixture ingests");
    assert_eq!(idx.gcc_version(), "13.3.0");
    assert_eq!(idx.format_version(), "1");
    assert_eq!(idx.line_count("t.c", 3), Some(1));
    assert_eq!(idx.line_count("m.h", 2), Some(1));
}

/// **Contract 2.** No entry for the macro's definition — and `None` is the assertion, not zero.
#[test]
fn a_macro_body_gets_no_coverage_record_at_all() {
    let idx = chiero_gcov::ingest_json(&corpus(), "t").expect("the fixture ingests");
    assert_eq!(
        idx.line_count("m.h", 1),
        None,
        "gcov attributes every statement of `ADD1` to its expansion site; if this line ever \
         gains an entry, 030 §1 and the frontend decision it justifies both need re-arguing"
    );
    assert_eq!(
        idx.lines_of("m.h"),
        vec![2],
        "the `static inline` beside it is covered at its definition, and is the only line"
    );
    assert_eq!(
        idx.lines_of("t.c"),
        vec![3],
        "one line for two expansions of a two-statement macro"
    );
}

/// **Contract 3.** The stem is the *object* name. Handing it the source name is a clear error,
/// not an empty index — an ingest that silently finds nothing is how a coverage-driven tool
/// reports "no tests cover this" about a file it never read.
#[test]
fn the_stem_is_the_object_name_and_a_wrong_one_is_an_error() {
    assert!(chiero_gcov::ingest_json(&corpus(), "t").is_ok());
    let err = chiero_gcov::ingest_json(&corpus(), "t.c").expect_err("`t.c` is not the stem");
    let msg = err.to_string();
    assert!(
        msg.contains("t.c.gcov.json.gz"),
        "the message must name the file it looked for: {msg}"
    );
}

/// **Contract 4.** JSON carries positional branch entries with no arc identity, so the index
/// says `Lines` and arc queries are unavailable — a fact downstream code reads from the type
/// rather than from an empty answer.
#[test]
fn json_ingest_records_line_detail_only() {
    let idx = chiero_gcov::ingest_json(&corpus(), "t").expect("the fixture ingests");
    assert_eq!(idx.detail(), chiero_gcov::CoverageDetail::Lines);
}
