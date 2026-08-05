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

/// **030 §4's arc data, asked the question 032 §3.2 needs: did this run enter a block?**
///
/// > A test whose arc-level coverage shows it never entered the block containing the change
/// > cannot observe it. This is bookkeeping rather than solving, and it matters because
/// > line-level coverage attributes a whole line — including a multi-statement macro expansion —
/// > to a test that only executed part of it.
///
/// # Why `tests_for_arc` cannot answer it
///
/// `tests_for_arc` records a test for an arc the graph *has*, whether or not the run took it —
/// deliberately, and for the crate's absence-versus-zero rule: an arc a test did not take is
/// recorded with a count of zero, because "the graph has no such arc" and "this run did not take
/// it" are different facts. So the set it returns is the tests that were *measured against* the
/// arc, not the tests that traversed it.
///
/// §3.2 asks the second question, and it is the one that may *remove* a test — so it gets its own
/// method rather than a caller re-deriving it from counts and getting the rule slightly wrong.
#[test]
fn a_block_is_entered_only_when_flow_reached_it() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "unrun").expect("unrun decodes");
    let f = cov
        .functions()
        .into_iter()
        .find(|k| k.name == "never_called")
        .cloned()
        .expect("the fixture has a function nothing calls");

    // Its entry block is block 0, which the flow solve gives the function's own count.
    assert_eq!(
        cov.entered_block(&f, 2),
        Some(false),
        "nothing called it, so no flow reached its first real block"
    );
}

/// And a function that ran did enter its blocks — otherwise the query would be a constant.
#[test]
fn a_block_of_a_function_that_ran_is_entered() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    assert_eq!(cov.entered_block(&main_key(), 2), Some(true));
}

/// A block the graph does not have is `None`, not `false` — the crate's rule once more: "no such
/// block" and "flow never reached it" are different facts, and only the second may drop a test.
#[test]
fn a_block_outside_the_graph_answers_nothing() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    assert_eq!(cov.entered_block(&main_key(), 999), None);
}

/// **The entry block is entered whenever the function ran**, even though nothing flows *into* it.
///
/// Reading "no incoming arcs with a count" as "not entered" would call every function's entry
/// block unreached and drop every test — the failure being in the direction that removes tests is
/// exactly why this has its own test.
#[test]
fn the_entry_block_is_entered_when_the_function_ran() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    assert_eq!(cov.entered_block(&main_key(), 0), Some(true));
}

/// **032 §3.2's actual question, in one call: did this run reach the code on that line?**
///
/// §3.2's point is that line-level coverage is too coarse:
///
/// > it matters because line-level coverage attributes a whole line — including a multi-statement
/// > macro expansion — to a test that only executed part of it.
///
/// `t.c:3` is exactly that line: `M; M;` — one line, two expansions of a multi-statement macro,
/// several blocks. Line coverage says "this test covered line 3" for a run that entered any of
/// them; the arcs say which.
///
/// The query takes a file and a line rather than a `FuncKey` and a block number, because that is
/// what a caller has: 031 reports an entity's *lines*, and gcov's block numbering is an internal
/// detail of the graph this crate decoded.
#[test]
fn a_line_is_reached_when_flow_entered_a_block_carrying_it() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    assert_eq!(
        cov.line_reached("t.c", 3),
        Some(true),
        "the run executed the macro expansions on line 3"
    );
}

/// A line whose blocks were never entered is `Some(false)` — which is what may drop a test.
#[test]
fn a_line_in_a_function_that_never_ran_is_not_reached() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "unrun").expect("unrun decodes");
    assert_eq!(
        cov.line_reached("unrun.c", 1),
        Some(false),
        "nothing called it, so no block carrying its line was entered"
    );
}

/// **A line the graph does not mention is `None`, not `false`.** The distinction is the whole
/// crate: `false` may drop a test, and "this line is not in the arc data" supports no such claim.
#[test]
fn a_line_the_arcs_do_not_mention_answers_nothing() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "t").expect("t decodes");
    assert_eq!(cov.line_reached("t.c", 999), None);
    assert_eq!(cov.line_reached("nosuch.c", 3), None);
}

/// **`line_reached` cannot contradict the line index at line granularity** — and that is why
/// 032 §3.2's refinement needs a *block*, not a line.
///
/// The argument is short. `tests_for_line` names a test only when its count for that line is
/// non-zero (030's absence-versus-zero rule, applied per test), and a non-zero line count means
/// flow entered some block carrying the line — which is exactly what `line_reached` asks. So for
/// every test the line index selects, the arcs agree.
///
/// §3.2's value is therefore entirely in the case its own text names:
///
/// > line-level coverage attributes a whole line — including a multi-statement macro expansion —
/// > to a test that only executed part of it.
///
/// *Part of it* is a block. To exploit that, the **change** must be located to a block too, and
/// 031 reports lines. Pinned here so that a refinement which cannot fire is not written and
/// labelled as the contract; the finding is recorded in HANDOFF beside §3.3's cut, which was made
/// for a related reason.
#[test]
fn the_arcs_never_contradict_the_line_index_at_line_granularity() {
    for stem in ["t", "loop", "unrun", "inl", "multi"] {
        let cov = chiero_gcov::native::arc_coverage(&corpus(), stem)
            .unwrap_or_else(|e| panic!("{stem}: {e}"));
        let idx = cov.index();
        for file in idx.files() {
            for line in idx.lines_of(file) {
                let counted = idx.line_count(file, line).unwrap_or(0) > 0;
                if !counted {
                    continue;
                }
                assert_eq!(
                    cov.line_reached(file, line),
                    Some(true),
                    "{stem}: {file}:{line} has a non-zero count, so some block carrying it was \
                     entered — the arcs cannot say otherwise at this granularity"
                );
            }
        }
    }
}
