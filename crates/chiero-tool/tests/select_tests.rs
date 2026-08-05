//! **050's `select_tests` — and what fidelity a coverage-based answer may honestly claim.**
//!
//! 050 §1 puts this operation in the table of things chiero decides:
//!
//! > | "this change probably doesn't affect the IP tests" | `select_tests` → ranked list with
//! > justification |
//!
//! # Why it is never `Exact`
//!
//! The envelope's `proven` means proven *for all inputs*. A selection cannot be: **coverage is
//! historical.** It records what the tests did on the code as it was, and the whole method rests
//! on that being a good guide to what they will do on the code as it is.
//!
//! 032's safety set covers the cases where there is no measurement at all — a new test, a crashed
//! run, a stale index — and 031's closure covers a test reaching new code through a caller it
//! already covered. Neither makes the answer a proof. So the fidelity is `Bounded` at best, the
//! bound is named as a blind spot, and `proven` is false.
//!
//! Claiming `Exact` here would be the one thing 050 §2 exists to prevent, committed by the crate
//! that enforces it.

use chiero_diff::{Program, impact};
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use chiero_select::Suite;
use chiero_tool::{Fidelity, select_tests};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn index() -> CoverageIndex {
    let mut idx = CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("fixture");
    idx.record_outcome(TestId(0), TestOutcome::Passed);
    idx
}

const BEFORE: &str = "int main (void)\n{\n  M; M;\n  return 0;\n}\n";
const AFTER: &str = "int main (void)\n{\n  M; M;\n  return 1;\n}\n";

/// **A selection is never `proven`**, and the envelope says why rather than merely saying no.
#[test]
fn a_selection_is_bounded_by_the_coverage_it_was_measured_from() {
    let before = Program::parse("t.c", BEFORE).expect("parses");
    let after = Program::parse("t.c", AFTER).expect("parses");
    let env = select_tests(&impact(&before, &after), &after, &index(), &Suite::default());

    assert!(!env.proven, "coverage is historical; no selection is a proof");
    assert_eq!(env.fidelity, Fidelity::Bounded);
    assert!(
        env.blind_spots.iter().any(|b| b.contains("historical")
            || b.contains("previous")),
        "and the bound is named: {:?}",
        env.blind_spots
    );
}

/// The result carries the ranked tests **and their reasons**, so a caller can act on one test
/// without re-running anything (032 contract 15).
#[test]
fn the_result_carries_the_tests_and_why() {
    let before = Program::parse("t.c", BEFORE).expect("parses");
    let after = Program::parse("t.c", AFTER).expect("parses");
    let env = select_tests(&impact(&before, &after), &after, &index(), &Suite::default());

    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let tests = v["result"]["tests"].as_array().expect("a list of tests");
    assert!(!tests.is_empty());
    for t in tests {
        assert!(t["test"].is_number());
        assert!(
            !t["reasons"].as_array().expect("reasons").is_empty(),
            "a test with no stated reason cannot be acted on: {t}"
        );
    }
}

/// **Reduction and safety travel together** into the envelope too (032 contract 20). A caller
/// reading only `tests` would see a number and no idea what it rests on.
#[test]
fn the_result_reports_reduction_beside_safety() {
    let before = Program::parse("t.c", BEFORE).expect("parses");
    let after = Program::parse("t.c", AFTER).expect("parses");
    let env = select_tests(&impact(&before, &after), &after, &index(), &Suite::default());
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");

    assert!(v["result"]["always_run"].is_number(), "{v}");
    assert!(v["result"]["excluded"].is_number(), "{v}");
}

/// **An incomplete impact set drops the fidelity further** — and the reasons become assumptions,
/// which 050 contract 4 requires to name every kind that actually occurred.
#[test]
fn an_incomplete_analysis_is_reported_as_an_assumption() {
    let good = Program::parse("t.c", BEFORE).expect("parses");
    let broken =
        Program::parse("t.c", "int main (void) { int x = 0; return x + ; }\n").expect("recovers");
    let env = select_tests(&impact(&good, &broken), &broken, &index(), &Suite::default());

    assert_eq!(env.fidelity, Fidelity::Unknown);
    assert!(
        env.assumptions
            .iter()
            .any(|(k, _)| k == "incomplete_analysis"),
        "the gap is an assumption the answer rests on: {:?}",
        env.assumptions
    );
}

/// Two identical runs produce the same envelope key, which is what lets a caller cache one.
#[test]
fn the_answer_is_deterministic() {
    let before = Program::parse("t.c", BEFORE).expect("parses");
    let after = Program::parse("t.c", AFTER).expect("parses");
    let a = select_tests(&impact(&before, &after), &after, &index(), &Suite::default());
    let b = select_tests(&impact(&before, &after), &after, &index(), &Suite::default());
    assert_eq!(a.determinism_key(), b.determinism_key());
}
