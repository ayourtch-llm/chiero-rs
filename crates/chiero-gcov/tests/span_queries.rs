//! **030 §5's two remaining queries: `tests_for_span` and `uncovered_lines`.**
//!
//! `tests_for_span` is where contract 12 stops being a guard and becomes code. A `Span` inside a
//! macro expansion has two locations — where the token was *written* (`spelling_loc`, in the macro
//! body) and where the expansion *happened* (`expansion_loc`, the call site) — and gcov records
//! only the second. `tests/corpus/coverage/t.c` pins that: a macro used twice on line 3 puts both
//! expansions on line 3 and leaves `m.h:1`, the macro body, with no record at all.
//!
//! So correlating a span through its spelling location asks the index about a line gcov never
//! wrote. The answer is not an error; it is `None`, which reads as "no test covers this" — and
//! 032 acts on that by *not* running tests. The guard in `expansion_loc_only.rs` catches the
//! string `spelling_loc`; these tests catch the behaviour, including the case where a wrong
//! implementation would return a plausible answer rather than nothing.
//!
//! `uncovered_lines` is the crate's absence-versus-zero rule pointed at a whole file. A line gcov
//! recorded as `0` was seen and not executed; a line gcov never mentioned is not evidence of
//! anything. Only the first is an uncovered line, and a query that returned both would report
//! every blank line and comment in the file as untested code.

use chiero_gcov::TestId;
use chiero_span::{BytePos, ExpnCtx, ExpnKind, SourceMap, Span};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// The `t` fixture, ingested as one test: `t.c:3` is covered, `m.h:1` is not recorded.
fn index() -> chiero_gcov::CoverageIndex {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("t decodes");
    idx
}

/// A root-context span of `len` bytes at `needle` + `skip` in `file`.
fn at(sm: &SourceMap, file: chiero_span::FileId, needle: &str, skip: u32, len: u32) -> Span {
    let f = sm.file(file);
    let off = f.src().find(needle).expect("the fixture text contains it") as u32 + skip;
    let lo = BytePos(f.start_pos.0 + off);
    Span::new(lo, BytePos(lo.0 + len), ExpnCtx::ROOT)
}

/// A map holding `t.c` and `m.h` with one expansion of `m.h`'s macro at `t.c:3`.
///
/// Returned with the span of a token *inside the macro body* — the shape the whole question is
/// about. Its spelling location is `m.h:1`; its expansion location is `t.c:3`.
fn map_with_expansion() -> (SourceMap, Span) {
    let mut sm = SourceMap::new();
    // Line 3 is the one the fixture's macro is used on.
    let t_c = sm.add_file("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n");
    let m_h = sm.add_file("m.h", "#define M do { } while (0)\n");

    // `M` on line 3, which `lookup_loc` resolves to line 3 — the line the fixture covers.
    let call_site = at(&sm, t_c, "  M;", 2, 1);
    // `do` inside the macro body, on line 1 of `m.h` — the line gcov never records.
    let body = at(&sm, m_h, "do", 0, 2);

    let ctx = sm.add_expansion(
        ExpnCtx::ROOT,
        None,
        call_site,
        call_site,
        Vec::new(),
        ExpnKind::ObjectLike,
    );
    // The token as it appears *in the expansion*: spelled in `m.h`, expanded at `t.c:3`.
    let inside = Span::new(body.lo, body.hi, ctx);
    (sm, inside)
}

/// **A span inside a macro expansion is asked about at its call site.**
///
/// Its spelling location is `m.h:1`, which gcov never recorded — so an implementation that used
/// it would answer `None` and read as "no test covers this".
#[test]
fn a_span_in_a_macro_body_resolves_to_the_call_site() {
    let (sm, inside) = map_with_expansion();
    let idx = index();
    assert_eq!(
        idx.tests_for_span(&sm, inside),
        Some(vec![TestId(0)]),
        "the expansion happened at t.c:3, which the fixture covers"
    );
}

/// The same span asked about the wrong way round must not accidentally work: `m.h:1` has no
/// record, and that is the answer a spelling-location implementation would give.
#[test]
fn the_macro_body_line_itself_has_no_record() {
    let idx = index();
    assert_eq!(
        idx.tests_for_line("m.h", 1),
        None,
        "gcov records the expansion site, never the macro body — so this must stay the way a \
         wrong implementation looks"
    );
}

/// A span in ordinary code resolves to its own line.
#[test]
fn a_root_span_resolves_to_its_own_line() {
    let (sm, _) = map_with_expansion();
    let t_c = sm.files().find(|f| f.path().ends_with("t.c")).unwrap().id();
    let sp = at(&sm, t_c, "  M;", 2, 1);
    assert_eq!(index().tests_for_span(&sm, sp), Some(vec![TestId(0)]));
}

/// **A span the index has nothing for answers `None`, not an empty set.** The crate's rule, at
/// the query 032 will actually call.
#[test]
fn a_span_on_an_unrecorded_line_answers_nothing() {
    let (sm, _) = map_with_expansion();
    let t_c = sm.files().find(|f| f.path().ends_with("t.c")).unwrap().id();
    // Line 5 — inside the file, and not a line gcov recorded.
    let sp = at(&sm, t_c, "}\n", 0, 1);
    assert_eq!(
        index().tests_for_span(&sm, sp),
        None,
        "no record is not an empty set of tests"
    );
}

/// **`uncovered_lines` reports what gcov saw and nothing ran — not what gcov never mentioned.**
///
/// `unrun.c` holds a function nothing calls, so its lines are recorded with count 0.
#[test]
fn uncovered_lines_are_the_recorded_zeroes() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "unrun").expect("unrun decodes");

    let uncovered = idx.uncovered_lines("unrun.c");
    assert!(
        !uncovered.is_empty(),
        "the fixture exists because it has a function nothing calls"
    );
    for line in &uncovered {
        assert_eq!(
            idx.line_count("unrun.c", *line),
            Some(0),
            "an uncovered line is one gcov recorded as zero"
        );
    }
    // Ascending and without repeats, so a caller can diff two of them.
    let mut sorted = uncovered.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(uncovered, sorted);
}

/// A line gcov never mentioned — a comment, a blank line, a declaration — is not uncovered code.
#[test]
fn a_line_with_no_record_is_not_uncovered() {
    let idx = index();
    assert_eq!(
        idx.line_count("t.c", 1),
        None,
        "the fixture records no line 1"
    );
    assert!(
        !idx.uncovered_lines("t.c").contains(&1),
        "a line with no record is not evidence that nothing ran it"
    );
}

/// A file the index has never heard of yields nothing, rather than an empty file's worth of
/// confident answers.
#[test]
fn an_unknown_file_has_no_uncovered_lines() {
    assert!(index().uncovered_lines("nosuch.c").is_empty());
}

/// **A span that reaches the end of its file must not collapse to its first line.**
///
/// `Span::hi` is exclusive, so a span covering a whole file ends *one past* its last byte —
/// and `SourceMap::lookup_file` refuses that position (`pos < end_pos`). `expansion_loc` then
/// answers `None` and the range silently narrows to the first line, dropping every test attached
/// to every line after it. The unsafe direction: 032 skips those.
#[test]
fn a_span_reaching_the_end_of_a_file_keeps_its_last_line() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n");
    let file = sm.file(f);
    let whole = Span::new(
        BytePos(file.start_pos.0),
        BytePos(file.start_pos.0 + file.byte_len()),
        ExpnCtx::ROOT,
    );
    assert_eq!(
        index().tests_for_span(&sm, whole),
        Some(vec![TestId(0)]),
        "the file's covered line is line 3, and a span over the whole file covers it"
    );
}

/// **A dummy span resolves to nothing**, which is what this file's own header claims and what
/// `expansion_loc` does *not* do on its own — 010 §4 says resolving `DUMMY.lo` fabricates a
/// location, and its call sites are expected to guard.
///
/// Synthesized CIR nodes carry `Span::DUMMY` (020 §6). Without the guard they were answered with
/// line 1 of whichever file happens to sit at offset 0 — coverage for a construct that has no
/// source location at all.
#[test]
fn a_dummy_span_resolves_to_nothing() {
    let mut sm = SourceMap::new();
    sm.add_file("loop.c", "while (i--)\n  ;\n");
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(3), &corpus(), "loop").expect("loop decodes");
    assert_eq!(
        idx.tests_for_span(&sm, Span::DUMMY),
        None,
        "a construct with no location has no coverage, and line 1 of the first file is not it"
    );
}

/// **The exclusive end must not pull in the following line.** A span covering exactly one line
/// *including its newline* ends at the first byte of the next one, and resolving that position
/// rather than the last byte the span contains unions in a line the construct does not occupy.
#[test]
fn a_spans_exclusive_end_is_not_a_line_it_covers() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n");
    let file = sm.file(f);
    // Line 2 is `{` — one character and a newline, and gcov records nothing for it.
    let start = file.src().find('{').expect("line 2") as u32;
    let sp = Span::new(
        BytePos(file.start_pos.0 + start),
        BytePos(file.start_pos.0 + start + 2),
        ExpnCtx::ROOT,
    );
    assert_eq!(
        index().tests_for_span(&sm, sp),
        None,
        "line 2 is unrecorded; line 3's tests belong to line 3"
    );
}
