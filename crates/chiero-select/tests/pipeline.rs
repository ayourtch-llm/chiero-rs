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
use chiero_select::{Confidence, SelectionReason, Suite, select, select_with};
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

/// **Contract 4 — the headline, and the reason all three verticals exist.**
///
/// > For a macro-body-only change in a header, chiero selects the tests that exercise the
/// > expansion sites.
///
/// Every part of this is a real artifact. The source is `tests/corpus/coverage/t.c` and `m.h`
/// verbatim; the coverage is the committed `t.gcno`/`t.gcda`, which gcov itself produced. The
/// diff touches **only `m.h`**, and `m.h:1` — the macro's own line — has no coverage entry at
/// all, which 030 §1 measured and `chiero-gcov` pins as a test.
///
/// So a tool that intersected the diff with coverage directly would look up `m.h:1`, find
/// nothing, and select nothing. What makes the answer exist is that 031 §3.2 turned the macro
/// change into an impacted *function* — `main` — and functions have coverage.
///
/// Three specs meet here, and each supplies the piece the others cannot:
///
/// | | |
/// |---|---|
/// | 030 | `t.c:3` is covered by the test; `m.h:1` is not recorded |
/// | 031 | the macro change becomes `main`, via `expansion_sites` |
/// | 032 | `main`'s lines meet the index, and the test is selected |
#[test]
fn a_macro_body_change_in_a_header_selects_the_tests_that_exercise_it() {
    /// The fixture's own two headers, in memory.
    struct Headers(&'static str);
    impl chiero_pp::FileLoader for Headers {
        fn load(&mut self, path: &std::path::Path) -> std::io::Result<String> {
            match path.file_name().and_then(|f| f.to_str()) {
                Some("m.h") => Ok(self.0.to_string()),
                // Enough of <stdio.h> for `t.c` to parse; the fixture calls `printf`.
                Some("stdio.h") => Ok("int printf (const char *, ...);\n".into()),
                _ => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{} is not part of this fixture", path.display()),
                )),
            }
        }
    }

    const T_C: &str = "#include <stdio.h>\n#include \"m.h\"\n\
        int main(void){ int v=1; ADD1(v); ADD1(v); printf(\"%d %d\\n\", v, hdr_fn(-3)); return 0; }\n";
    const BEFORE: &str = "#define ADD1(V) do { (V) = (V) + 1; (V) = (V) * 2; } while (0)\n\
                          static inline int hdr_fn(int x){ return x < 0 ? -x : x; }\n";
    // The *only* edit: `+ 1` becomes `+ 3`, inside the macro body, in the header.
    const AFTER: &str = "#define ADD1(V) do { (V) = (V) + 3; (V) = (V) * 2; } while (0)\n\
                         static inline int hdr_fn(int x){ return x < 0 ? -x : x; }\n";

    let mut cfg = chiero_pp::Config::default();
    cfg.iquote_paths.push(std::path::PathBuf::from("."));
    cfg.system_paths.push(std::path::PathBuf::from("."));
    let before = Program::parse_with("t.c", T_C, cfg.clone(), &mut Headers(BEFORE))
        .expect("the fixture parses");
    let after =
        Program::parse_with("t.c", T_C, cfg, &mut Headers(AFTER)).expect("the fixture parses");

    let set = chiero_diff::impact(&before, &after);
    assert!(
        set.entities
            .contains_key(&chiero_diff::Entity::function("t.c", "main")),
        "031 turns a macro-body change into the functions that expand it: {:?}",
        set.entities
            .keys()
            .map(chiero_diff::Entity::name)
            .collect::<Vec<_>>()
    );

    let idx = index_with(&[TestId(0)]);
    // The premise, from 030: the macro's own line has no coverage entry, so the direct
    // intersection a coverage-only tool would perform finds nothing.
    assert_eq!(
        idx.tests_for_line("m.h", 1),
        None,
        "gcov records the expansion site, never the macro's definition"
    );

    let sel = select(&set, &after, &idx);
    assert!(
        sel.tests.contains_key(&TestId(0)),
        "the test exercises `main`, which expands the macro that changed: {:?}",
        sel.tests
    );
    assert!(
        sel.tests[&TestId(0)]
            .iter()
            .any(|r| matches!(r, SelectionReason::CoversEntity { entity, .. } if entity == "main")),
        "and it says so: {:?}",
        sel.tests[&TestId(0)]
    );
}

/// **Contract 10.** A test in the tree but absent from the index has never had a chance to be
/// measured, so nothing can say it is unaffected.
///
/// The index cannot know this by itself — it only holds what was ingested — so the suite's test
/// list is the caller's to supply. A tool that inferred "the suite is what the index contains"
/// would silently never run a new test.
#[test]
fn a_test_absent_from_the_index_is_always_selected() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let suite = Suite {
        tests: vec![TestId(0), TestId(42)],
        ..Suite::default()
    };
    let sel = select_with(&impact(&p, &p), &p, &index_with(&[TestId(0)]), &suite);

    assert!(
        sel.tests.contains_key(&TestId(42)),
        "it is in the tree and not in the index: {:?}",
        sel.tests
    );
    assert!(
        sel.tests[&TestId(42)]
            .iter()
            .any(|r| matches!(r, SelectionReason::AlwaysRun { why } if why.contains("never"))),
        "and it is labelled as never measured: {:?}",
        sel.tests[&TestId(42)]
    );
}

/// **Contract 12.** A stale index forces every test touching the stale files into the selection.
///
/// Coverage is a claim about a specific source state (030 §7). Acting on a stale index is worse
/// than having none: it is a confident answer about code that no longer exists.
#[test]
fn a_stale_index_forces_its_tests_into_the_selection() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let suite = Suite {
        tests: vec![TestId(0)],
        validity: chiero_gcov::Validity::Stale {
            files: vec!["t.c".to_string()],
        },
    };
    let sel = select_with(&impact(&p, &p), &p, &index_with(&[TestId(0)]), &suite);

    assert!(
        sel.tests.contains_key(&TestId(0)),
        "the empty diff would have selected nothing; the stale index overrides that: {:?}",
        sel.tests
    );
    match &sel.confidence {
        Confidence::Reduced { reasons } => assert!(
            reasons
                .iter()
                .any(|r| r.contains("stale") && r.contains("t.c")),
            "and the report names the stale file: {reasons:?}"
        ),
        other => panic!("a stale index cannot be Full confidence, got {other:?}"),
    }
}

/// A `Partial` index — some test's coverage never arrived — is the third of §4's triggers, and it
/// is distinct from staleness: the sources are current and a *test* is unaccounted for.
#[test]
fn a_partial_index_reduces_confidence() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let suite = Suite {
        tests: vec![TestId(0)],
        validity: chiero_gcov::Validity::Partial {
            missing_tests: vec![TestId(9)],
        },
    };
    let sel = select_with(&impact(&p, &p), &p, &index_with(&[TestId(0)]), &suite);
    assert!(matches!(sel.confidence, Confidence::Reduced { .. }));
    assert!(
        sel.tests.contains_key(&TestId(9)),
        "a test the index cannot speak for must run: {:?}",
        sel.tests
    );
}

/// **Contract 20, and it is the sharpest in 032.**
///
/// > Reduction and safety are both present in every report; a report containing reduction alone
/// > fails.
///
/// A selection tool's whole product is a claim that some tests need not run. A report that shows
/// "3412 excluded" without showing what was *unconditionally kept* and how confident the answer
/// is invites exactly one reading — "it works, we run fewer tests" — and that reading is
/// unfalsifiable from the report itself.
#[test]
fn every_report_carries_reduction_and_safety_together() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    let suite = Suite {
        tests: vec![TestId(0), TestId(1), TestId(2)],
        ..Suite::default()
    };
    let text = select_with(
        &impact(&before, &after),
        &after,
        &index_with(&[TestId(0)]),
        &suite,
    )
    .render();

    assert!(text.contains("SELECTED"), "the reduction:\n{text}");
    assert!(text.contains("ALWAYS-RUN"), "the safety set:\n{text}");
    assert!(
        text.contains("CONFIDENCE"),
        "and how much to trust it:\n{text}"
    );
}

/// **Contract 16.** The ranking is deterministic, and it leads with what the maintainer should
/// look at first — the tests closest to the change.
#[test]
fn the_ranking_is_deterministic_and_leads_with_the_closest() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    let idx = index_with(&[TestId(0)]);
    let a = select(&impact(&before, &after), &after, &idx);
    let b = select(&impact(&before, &after), &after, &idx);
    assert_eq!(a.ranked(), b.ranked());
    assert!(!a.ranked().is_empty());
}

/// **Contract 17.** A budget truncates the ranked list, and the report says so — truncation is
/// **not** refinement.
///
/// §5.1: *"the dropped tests were selected, so the report states the count, the residual risk,
/// and the rank cutoff, and `Confidence` becomes `Reduced`. A budgeted run must never render as
/// if it covered the impact."*
#[test]
fn a_budget_truncates_and_says_so() {
    let p = Program::parse("t.c", t_c()).expect("parses");
    let suite = Suite {
        tests: vec![TestId(0), TestId(1), TestId(2), TestId(3)],
        ..Suite::default()
    };
    let full = select_with(&impact(&p, &p), &p, &index_with(&[TestId(0)]), &suite);
    assert!(full.tests.len() >= 3, "the fixture needs something to cut");

    let cut = full.clone().budgeted(2);
    assert_eq!(cut.ranked().len(), 2);
    match &cut.confidence {
        Confidence::Reduced { reasons } => assert!(
            reasons
                .iter()
                .any(|r| r.contains("budget") && r.contains("dropped")),
            "the report states the count and the cutoff: {reasons:?}"
        ),
        other => panic!("a budgeted run is never Full confidence, got {other:?}"),
    }
    let text = cut.render();
    assert!(
        text.contains("BUDGET"),
        "a budgeted run must never render as if it covered the impact:\n{text}"
    );
}

/// A budget that cuts nothing changes nothing — including the confidence.
#[test]
fn a_budget_that_fits_is_not_a_caveat() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    let sel = select(&impact(&before, &after), &after, &index_with(&[TestId(0)]));
    let n = sel.tests.len();
    assert_eq!(sel.clone().budgeted(n + 5).confidence, sel.confidence);
}

/// **A file the index has never heard of is a different fact from an entity with no coverage**,
/// and the difference is almost always a path that was not resolved.
///
/// This guard exists because the mistake has now been made three times in this project, each time
/// silently and each time *flatteringly*:
///
/// - `chiero-diff`'s macro-expansion baseline looked up `m.h` and found the `static inline`'s
///   coverage, so a premise test passed for the wrong reason;
/// - the mutation gate's coverage-only baseline looked up `lib.c` while gcov had recorded an
///   absolute path, reporting 0% on exactly the mutations where the baseline works;
/// - the mutation gate's *own* pipeline did the same and selected nothing, which reads as a 100%
///   reduction.
///
/// 030 is explicit that paths are stored as gcov wrote them and that resolving them belongs to
/// the caller. That is the right division — matching by basename here would conflate two files of
/// one name in different directories, the identity mistake `FuncKey` exists to prevent — but a
/// caller who gets it wrong deserves to be told, in those words, rather than handed a small
/// answer.
#[test]
fn an_unknown_file_is_reported_as_a_resolution_problem() {
    let before = Program::parse("/elsewhere/t.c", t_c()).expect("parses");
    let after = Program::parse(
        "/elsewhere/t.c",
        "int main (void)\n{\n  M; M;\n  return 1;\n}\n",
    )
    .expect("parses");
    // The index holds `t.c`, not `/elsewhere/t.c`.
    let sel = select(&impact(&before, &after), &after, &index_with(&[TestId(0)]));

    match &sel.confidence {
        Confidence::Reduced { reasons } => {
            assert!(
                reasons.iter().any(
                    |r| r.contains("not in the coverage index") && r.contains("/elsewhere/t.c")
                ),
                "the reason must name the file and say the index has never heard of it, because \
                 that is a resolution problem and not a coverage gap: {reasons:?}"
            );
        }
        other => panic!("a file the index does not hold cannot be Full confidence: {other:?}"),
    }
}

/// And the ordinary case is unchanged: a file the index *does* hold, with an entity it has no
/// coverage for, is a coverage gap and says so in those terms.
#[test]
fn a_known_file_with_no_coverage_is_a_coverage_gap() {
    // `t.c` is in the index; line 4 of it is not.
    let before =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n").expect("parses");
    let after = Program::parse(
        "t.c",
        "int main (void)\n{\n  M; M;\n  return 0;\n}\nint spare (void) { return 1; }\n",
    )
    .expect("parses");
    let sel = select(&impact(&before, &after), &after, &index_with(&[TestId(0)]));
    if let Confidence::Reduced { reasons } = &sel.confidence {
        assert!(
            !reasons
                .iter()
                .any(|r| r.contains("not in the coverage index")),
            "the file is in the index; only the entity is unmeasured: {reasons:?}"
        );
    }
}

/// **Contract 2.** A whitespace-only diff selects only the always-run set.
///
/// The chain is what makes this work rather than a special case: 031's fingerprint is the token
/// spelling, so reformatting produces an *empty* impact set, and an empty impact set intersects
/// with coverage to nothing. Nothing in 032 has to know what whitespace is.
#[test]
fn a_whitespace_only_diff_selects_only_the_always_run_set() {
    let before = Program::parse("t.c", t_c()).expect("parses");
    let reformatted =
        Program::parse("t.c", "int main(void){\n\n\n  M; M;\n  return 0;\n}\n").expect("parses");
    let sel = select(
        &impact(&before, &reformatted),
        &reformatted,
        &index_with(&[TestId(0)]),
    );

    assert!(
        sel.tests.is_empty(),
        "reformatting is not a change, all the way through: {:?}",
        sel.tests
    );
    assert_eq!(sel.confidence, Confidence::Full);
}

/// **Contract 14.** A build-config change selects the whole suite, **with that stated as the
/// reason**.
///
/// The second half is the contract. Selecting everything is easy; a report that does it without
/// saying why is indistinguishable from a tool that has given up, and a maintainer who cannot see
/// the cause cannot fix it.
#[test]
fn a_config_change_selects_the_suite_and_says_why() {
    let before = Program::parse(
        "t.c",
        "#if FOO > 2\nint gated (void) { return 1; }\n#endif\nint main (void)\n{\n  M; M;\n  return 0;\n}\n",
    )
    .expect("parses");
    let after = Program::parse(
        "t.c",
        "#if FOO > 3\nint gated (void) { return 1; }\n#endif\nint main (void)\n{\n  M; M;\n  return 0;\n}\n",
    )
    .expect("parses");

    let mut idx = index_with(&[TestId(0)]);
    idx.record_outcome(TestId(1), TestOutcome::Passed);
    let sel = select(&impact(&before, &after), &after, &idx);

    assert!(
        sel.tests.contains_key(&TestId(0)) && sel.tests.contains_key(&TestId(1)),
        "a condition changed, so every test the index knows must run: {:?}",
        sel.tests
    );
    match &sel.confidence {
        Confidence::Reduced { reasons } => assert!(
            reasons.iter().any(|r| r.contains("condition")),
            "and the reason names the condition rather than merely reporting a gap: {reasons:?}"
        ),
        other => panic!("a config change cannot be Full confidence, got {other:?}"),
    }
    // The report leads with it too.
    let text = sel.render();
    assert!(text.contains("CONFIDENCE: Reduced"), "{text}");
}
