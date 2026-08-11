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

/// **The two ingest paths must agree about what merging two objects means.**
///
/// A header's `static inline` function is compiled into every translation unit that includes it,
/// so the same `(file, line)` arrives from several objects. The JSON path added those counts and
/// the native path took their maximum, so one index reported a line as executed twice and the
/// other as executed once, from the same artifacts.
///
/// **The sum is the aggregate**, and `line_count` is documented as one: a line compiled into two
/// objects and executed once in each was executed twice. The maximum was chosen to keep a
/// multi-build tree from over-reporting — but "how often did the program execute this line" is a
/// different question from "how often did any one build", the per-build answer is what
/// `tests_for_line_in` is for, and the sum is the more conservative of the two for 032, which
/// never skips on a count being too high.
#[test]
fn both_ingest_paths_merge_two_objects_the_same_way() {
    let mut native = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut native, chiero_gcov::TestId(0), &corpus(), "loop")
        .expect("once");
    chiero_gcov::ingest_native_as(&mut native, chiero_gcov::TestId(1), &corpus(), "loop")
        .expect("twice");

    let mut json = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_json_into(&mut json, &corpus(), "loop").expect("once");
    chiero_gcov::ingest_json_into(&mut json, &corpus(), "loop").expect("twice");

    for line in json.lines_of("loop.c") {
        assert_eq!(
            native.line_count("loop.c", line),
            json.line_count("loop.c", line),
            "loop.c:{line} — the same artifacts read two ways must aggregate the same"
        );
    }
    assert_eq!(
        native.line_count("loop.c", 1),
        Some(10),
        "five executions in each of two objects is ten"
    );
}

/// **A JSON ingest can carry a test name, and until 2026-08-11 it could not.**
///
/// `ingest_native_as` has taken a `TestId` since it was written; the JSON path had no `_as`
/// sibling, so an index built from `gcov --json-format` output had `test: None` on every line
/// and **selection over it was empty by construction** — the same wall the CLI hit in §7.34.
///
/// That mattered beyond convenience. Native ingest needs `.gcno`/`.gcda`, which are binary and
/// produced only by a real instrumented build; JSON is text, so a caller holding one file per
/// test run — or a *generated* corpus, which is what a size axis for selection needs — could
/// not attribute any of it.
#[test]
fn json_ingest_attributes_a_test_when_it_is_given_one() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_json_as(&mut idx, chiero_gcov::TestId(7), &corpus(), "t")
        .expect("the pinned JSON fixture");
    assert_eq!(
        idx.tests(),
        vec![chiero_gcov::TestId(7)],
        "the index must know the test that produced this coverage"
    );
    // `t.c:3` is the line the fixture's `main` executes — the one contract 2 is written about.
    assert_eq!(
        idx.tests_for_line("t.c", 3),
        Some(vec![chiero_gcov::TestId(7)]),
        "and the line must name it, or selection cannot use it"
    );
}

/// The unattributed spelling keeps its meaning: `Unknown`, not "no test covers this".
#[test]
fn json_ingest_without_a_test_still_answers_unknown() {
    let idx = chiero_gcov::ingest_json(&corpus(), "t").expect("the pinned JSON fixture");
    assert!(
        idx.tests().is_empty(),
        "no test was named, so none is known"
    );
    assert_eq!(
        idx.tests_for_line("t.c", 3),
        None,
        "`None` is 'nobody recorded which tests', which is not the empty set"
    );
}
