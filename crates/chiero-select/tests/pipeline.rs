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
