//! **Every code block in `docs/tutorials/` runs here.**
//!
//! Documentation that does not compile is worse than none: a reader who pastes it and gets an
//! error learns not to trust the rest of it, and a reader who does not paste it learns
//! something false. So each tutorial's worked example is a test in this file, and the
//! tutorial quotes it.
//!
//! [`every_tutorial_is_covered`] fails if a tutorial is added without one, which is the same
//! registry discipline `operations.rs` uses and for the same reason: "every tutorial" is a
//! claim that decays silently.

use chiero_diff::{Program, impact};
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use chiero_select::Suite;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

// ---------------------------------------------------------------------------------------
// 01 — Reading coverage
// ---------------------------------------------------------------------------------------

#[test]
fn tutorial_01_reading_coverage() {
    // Ingest one test's artifacts. `stem` is the object's base name: the build wrote
    // `t.gcno` and `t.gcda` next to each other.
    let mut index = CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut index, TestId(0), &corpus(), "t").expect("artifacts");
    index.record_outcome(TestId(0), TestOutcome::Passed);

    // The distinction the whole design turns on: `None` is "nobody measured this line",
    // `Some(0)` is "measured, and it did not run". A tool that returns 0 for both will tell
    // you dead code is dead when in fact nobody looked.
    let file = index.files().next().expect("one file").to_string();
    let measured: Vec<u32> = index.lines_of(&file);
    assert!(!measured.is_empty(), "the fixture has measured lines");

    let executed = measured
        .iter()
        .filter(|l| index.line_count(&file, **l).is_some_and(|c| c > 0))
        .count();
    let never_ran = index.uncovered_lines(&file).len();
    assert_eq!(
        executed + never_ran,
        measured.len(),
        "every measured line either ran or did not; there is no third answer"
    );

    // A line nobody measured answers None — not zero.
    assert_eq!(index.line_count(&file, 99_999), None);

    // Which tests touched a line, for the change-impact join in tutorial 3.
    let hot = measured
        .iter()
        .find(|l| index.line_count(&file, **l).is_some_and(|c| c > 0))
        .expect("some line ran");
    assert!(
        index
            .tests_for_line(&file, *hot)
            .is_some_and(|t| !t.is_empty())
    );
}

// ---------------------------------------------------------------------------------------
// 02 — What a change reaches
// ---------------------------------------------------------------------------------------

const V1: &str = "\
#define SCALE(x) ((x) * 2)
int area (int w) { return SCALE (w) * w; }
int volume (int w) { return area (w) * w; }
";

const V2: &str = "\
#define SCALE(x) ((x) * 3)
int area (int w) { return SCALE (w) * w; }
int volume (int w) { return area (w) * w; }
";

/// **Tutorial 2's "reading a real file" section, compiled.** The one drift the first
/// end-to-end user reported that nobody could find on 2026-08-10 was about `FileLoader::load`
/// — and the reason it could not be found is that no tutorial mentioned `FileLoader` at all.
/// `Program::parse` follows no `#include`, so the moment a reader points chiero at a real
/// translation unit they meet a trait the documentation had never named, and they guessed at
/// its signature. It returns `io::Result`, and this is where that is now written down.
#[test]
fn tutorial_02_reading_a_real_file_with_its_includes() {
    struct Disk;
    impl chiero_diff::FileLoader for Disk {
        fn load(&mut self, path: &std::path::Path) -> std::io::Result<String> {
            std::fs::read_to_string(path)
        }
    }

    let dir = std::env::temp_dir().join(format!("chiero-tut2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    std::fs::write(dir.join("scale.h"), "#define SCALE(x) ((x) * 2)\n").expect("write");
    let src = "#include \"scale.h\"\nint area (int w) { return SCALE (w) * w; }\n";

    let mut cfg = chiero_diff::Config::default();
    cfg.include_paths.push(dir.clone());
    let before =
        Program::parse_with("geom.c", src, cfg.clone(), &mut Disk).expect("parses with includes");

    std::fs::write(dir.join("scale.h"), "#define SCALE(x) ((x) * 3)\n").expect("write");
    let after = Program::parse_with("geom.c", src, cfg, &mut Disk).expect("parses with includes");

    // **The same unit name for both**, which the tutorial now says in as many words: keying by
    // file is what makes `area` one entity rather than two.
    let set = impact(&before, &after);
    let names: Vec<&str> = set.entities.keys().map(|e| e.name()).collect();
    assert!(
        names.contains(&"area"),
        "a macro edited in a *header* still reaches the function that expands it: {names:?}"
    );
}

#[test]
fn tutorial_02_change_impact() {
    let before = Program::parse("geom.c", V1).expect("parses");
    let after = Program::parse("geom.c", V2).expect("parses");
    let set = impact(&before, &after);

    // One macro body changed. Nothing else in the file was edited — and yet:
    let names: Vec<&str> = set.entities.keys().map(|e| e.name()).collect();
    assert!(names.contains(&"SCALE"), "the macro itself: {names:?}");
    assert!(
        names.contains(&"area"),
        "every function that expands it, which no coverage tool can tell you: {names:?}"
    );
    assert!(
        names.contains(&"volume"),
        "and everything reachable from those: {names:?}"
    );

    // Each entry says *why* it is there, so an answer can be checked rather than believed.
    let j = &set.entities[&chiero_diff::Entity::function("geom.c", "area")];
    assert!(
        !j.edges.is_empty() || j.distance > 0,
        "a justification with no edges and no distance is an assertion"
    );

    // And whether the answer is complete.
    assert_eq!(set.completeness, chiero_diff::Completeness::Complete);
}

// ---------------------------------------------------------------------------------------
// 03 — Choosing tests
// ---------------------------------------------------------------------------------------

#[test]
fn tutorial_03_test_selection() {
    let mut index = CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut index, TestId(0), &corpus(), "t").expect("artifacts");
    index.record_outcome(TestId(0), TestOutcome::Passed);

    let before =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n").expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");

    let suite = Suite {
        tests: vec![TestId(0)],
        validity: chiero_gcov::Validity::Fresh,
    };
    let selection = chiero_select::select_with(&impact(&before, &after), &after, &index, &suite);

    // Ranked, so a caller with time for three tests runs the three that matter most.
    let ranked = selection.ranked();
    assert!(!ranked.is_empty());

    // Every selected test carries why it was selected — and every *excluded* one carries the
    // proof that justified dropping it. A selection with no reasons is a guess.
    for t in &ranked {
        assert!(!selection.tests[t].is_empty(), "test {t:?} has no reason");
    }

    // Take the top N and the rest are still accounted for, not forgotten.
    let short = selection.clone().budgeted(1);
    assert!(short.ranked().len() <= 1);
}

// ---------------------------------------------------------------------------------------
// 04 — Adjudicating a rewrite
// ---------------------------------------------------------------------------------------

fn m(body: &str) -> chiero_cir::Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .expect("parses")
}

/// `int f (int x) { return x < 0 ? -x : x; }`
const ABS_BEFORE: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp slt i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  %2 = sub i32 0i32, %0
  ret %2
bb2:
  .line 3
  ret %0
}";

/// The same, rewritten to saturate at `INT_MIN` — plausible, and not the same function.
const ABS_AFTER: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp slt i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  %2 = cmp eq i32 %0, -2147483648i32
  br %2, bb3, bb4
bb3:
  .line 3
  ret 2147483647i32
bb4:
  .line 4
  %3 = sub i32 0i32, %0
  ret %3
bb2:
  .line 5
  ret %0
}";

#[test]
fn tutorial_04_prove_equivalent() {
    let cfg = chiero_opt::EquivCfg::new("f");
    if cfg.backend.is_none() {
        // No solver on PATH: the operation still answers, it just answers `Unknown`. That
        // is the tutorial's other lesson, so assert it rather than skipping silently.
        assert!(matches!(
            chiero_opt::prove_equivalent(&m(ABS_BEFORE), &m(ABS_AFTER), &cfg),
            chiero_opt::Equivalence::Unknown { .. }
        ));
        return;
    }

    let env = chiero_tool::prove_equivalent(&m(ABS_BEFORE), &m(ABS_AFTER), &cfg);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");

    assert_eq!(v["result"]["verdict"], "differs");
    assert_eq!(
        v["result"]["input"][0]["signed"].as_str(),
        Some("-2147483648"),
        "the one input of 2^32 at which they disagree"
    );
    assert_eq!(v["result"]["observation"]["before_signed"], "-2147483648");
    assert_eq!(v["result"]["observation"]["after_signed"], "2147483647");

    // And the answer says what has *not* been done: no harness compiled it.
    assert!(env.blind_spots.iter().any(|b| b.contains("replay harness")));

    // The agreeing direction, for contrast: x * 2 and x << 1 over all 2^32 inputs.
    let double =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = mul i32 %0, 2i32\n  ret %1\n}";
    let shift =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = shl i32 %0, 1i32\n  ret %1\n}";
    let env = chiero_tool::prove_equivalent(&m(double), &m(shift), &cfg);
    assert!(env.proven, "a proof over every input");
    assert_eq!(env.fidelity, chiero_tool::Fidelity::Exact);
}

// ---------------------------------------------------------------------------------------
// 05 — Reading the envelope
// ---------------------------------------------------------------------------------------

#[test]
fn tutorial_05_the_envelope() {
    let src = "#define M(v) ((v) + 1)\nint a (int x) { return M (x); }\n";
    let tu = chiero_pp::preprocess_str("f.c", src, chiero_pp::Config::default());

    // A complete answer: the expansion table is the preprocessor's own record.
    let complete = chiero_tool::expansion_sites_envelope(&tu.source_map, "M", None, 50);
    assert!(complete.proven);
    assert_eq!(complete.fidelity, chiero_tool::Fidelity::Exact);

    // An empty answer that is *proven* empty — the macro genuinely expands nowhere...
    let absent = chiero_tool::expansion_sites_envelope(&tu.source_map, "NOPE", None, 50);
    let v: serde_json::Value = serde_json::from_str(&absent.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["total"], 0);

    // ...against an empty answer that proves nothing, because the file is unknown. Both
    // carry an empty list; only one of them means the code is clean.
    let unknown = chiero_tool::explain_macro_expansion_envelope(&tu.source_map, "other.c", 1, None);
    assert!(!unknown.proven);
    assert!(
        !unknown.blind_spots.is_empty(),
        "an unproven answer always says what made it unproven"
    );

    // The rendering a human reads carries the qualification too — it is never a bare list.
    let text = unknown.render();
    assert!(text.contains("not proven") || text.contains("within"));

    // Same input, same bytes (001 §5).
    let again = chiero_tool::explain_macro_expansion_envelope(&tu.source_map, "other.c", 1, None);
    assert_eq!(unknown.determinism_key(), again.determinism_key());
}

// ---------------------------------------------------------------------------------------
// 06 — Finding defects
// ---------------------------------------------------------------------------------------

#[test]
fn tutorial_06_find_bugs() {
    // A division by zero when `n` is 0, after a loop chiero has to bound.
    let average = "\
func @average(%0: ptr, %1: i32) -> i32 {
entry:
  .line 1
  %2 = sdiv i32 %1, %1
  ret %2
}";
    let env = chiero_tool::find_bugs(&m(average), &chiero_tool::BugCfg::new("average"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let findings = v["result"]["findings"].as_array().expect("findings");
    assert!(!findings.is_empty(), "n == 0 divides by zero: {v}");
    assert!(
        findings[0]["paths"].as_u64().is_some(),
        "one bug, and how many paths reached it: {v}"
    );

    // The clean case: no loop, nothing cut, and the one place an empty list is an answer.
    let clamp = "\
func @clamp(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp slt i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  ret 0i32
bb2:
  .line 3
  ret %0
}";
    let env = chiero_tool::find_bugs(&m(clamp), &chiero_tool::BugCfg::new("clamp"));
    assert!(
        env.proven,
        "an exhaustive search that found nothing found nothing"
    );
    assert_eq!(env.fidelity, chiero_tool::Fidelity::Exact);
    // And it still says what it cannot see.
    assert!(
        env.blind_spots.iter().any(|b| b.contains("checkers")),
        "a defect no checker looks for is not reported, and that is said every time: {:?}",
        env.blind_spots
    );
}

// ---------------------------------------------------------------------------------------
// 07 — What the code can and cannot reach
// ---------------------------------------------------------------------------------------

#[test]
fn tutorial_07_reachability() {
    // The page's `unreachable` verdict is a proof that the search was exhaustive, which tier 1
    // cannot produce — the tutorials index says the transcripts assume a solver, and this is
    // the assertion that depends on it. 022 contract 2.
    if chiero_solver::SmtLib::discover().is_none() {
        eprintln!("skipping: the page's `unreachable` proof needs a backend (022 contract 2)");
        return;
    }
    // `if (x > 0) { if (x > 0) return 1; return 2; } return 3;`
    let classify = "\
func @classify(%0: i32) -> i32 {
entry:
  .line 3
  %1 = cmp sgt i32 %0, 0i32
  br %1, bb1, bb4
bb1:
  .line 4
  %2 = cmp sgt i32 %0, 0i32
  br %2, bb2, bb3
bb2:
  .line 5
  ret 1i32
bb3:
  .line 6
  ret 2i32
bb4:
  .line 8
  ret 3i32
}";
    let cfg = chiero_tool::BugCfg::new("classify");

    // The two operations agree from opposite directions.
    let dead = chiero_tool::find_optimizations(
        &m(classify),
        &chiero_opt::opportunity::OppCfg::new("classify"),
    );
    let dv: serde_json::Value = serde_json::from_str(&dead.to_json()).expect("valid JSON");
    assert!(
        dv["result"]["count"].as_u64().is_some_and(|n| n > 0),
        "the inner test is decided by the outer: {dv}"
    );

    let unreachable = chiero_tool::check_reachable(&m(classify), &cfg, 6);
    let uv: serde_json::Value = serde_json::from_str(&unreachable.to_json()).expect("valid JSON");
    assert_eq!(uv["result"]["verdict"], "unreachable");
    assert!(
        unreachable.proven,
        "an exhaustive search is what makes it a proof"
    );

    // And the side that can happen comes with the input that gets there.
    let reachable = chiero_tool::check_reachable(&m(classify), &cfg, 5);
    let rv: serde_json::Value = serde_json::from_str(&reachable.to_json()).expect("valid JSON");
    assert_eq!(rv["result"]["verdict"], "reachable");
    assert!(
        rv["result"]["witness"]
            .as_array()
            .is_some_and(|w| !w.is_empty()),
        "a `yes` that cannot say how is a guess: {rv}"
    );

    // A line with no code is its own answer, not `unreachable`.
    let none = chiero_tool::check_reachable(&m(classify), &cfg, 7);
    let nv: serde_json::Value = serde_json::from_str(&none.to_json()).expect("valid JSON");
    assert_eq!(nv["result"]["verdict"], "no_such_line");
    assert!(!none.proven);
}

// ---------------------------------------------------------------------------------------
// 08 — Struct layout
// ---------------------------------------------------------------------------------------

#[test]
fn tutorial_08_layout() {
    use chiero_opt::locality::{Field, LocalityCfg, Record};
    // `char; long; char;` — 24 bytes that would be 16.
    let session = Record {
        tag: "session".into(),
        span: chiero_span::Span::DUMMY,
        size: 24,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "active".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "bytes".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
            Field {
                name: "flags".into(),
                offset: 16,
                size: 1,
                bits: None,
            },
        ],
    };
    let env = chiero_tool::layout_envelope(std::slice::from_ref(&session), &LocalityCfg::default());
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let p = &v["result"]["records"][0]["proposals"][0];
    assert_eq!(p["kind"], "padding_waste");
    assert_eq!(
        p["recoverable"].as_u64(),
        Some(8),
        "the delta, not just the fact"
    );
    assert_eq!(
        p["advisory"].as_bool(),
        Some(false),
        "this layout is internal"
    );
    assert_eq!(p["benefit"], "Unquantified", "no run, no number");

    // A wire format: the finding is true and acting on it is a protocol change.
    let wire = Record {
        tag: "pkt_hdr".into(),
        span: chiero_span::Span::DUMMY,
        size: 68,
        align: 1,
        packed: true,
        externally_visible: false,
        fields_complete: true,
        fields: vec![Field {
            name: "seq".into(),
            offset: 60,
            size: 8,
            bits: None,
        }],
    };
    let env = chiero_tool::layout_envelope(std::slice::from_ref(&wire), &LocalityCfg::default());
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let p = &v["result"]["records"][0]["proposals"][0];
    assert_eq!(p["kind"], "line_straddle");
    assert_eq!(p["advisory"].as_bool(), Some(true));
    assert!(
        p["rationale"]
            .as_str()
            .is_some_and(|r| r.contains("observable")),
        "in words, not just a flag: {p}"
    );
}

// ---------------------------------------------------------------------------------------

/// **A tutorial with no test is a tutorial that has already rotted.**
#[test]
fn every_tutorial_is_covered() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("docs/tutorials");
    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("no tutorials at {}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "the scan found no tutorials");
    assert!(
        found.iter().any(|n| n == "README.md"),
        "the tutorials directory has no index: {found:?}"
    );

    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tutorials.rs"),
    )
    .expect("this file");

    let missing: Vec<&String> = found
        .iter()
        // **A numbered page is a tutorial; anything else is signposting.** `README.md` is the
        // directory's index — it has no worked example because it makes no claim of its own,
        // and demanding `fn tutorial_README.md` would be this check misreading its own rule.
        // The rule it is really enforcing is "every page that teaches something runs", and a
        // page of links teaches nothing that can rot into a false answer.
        .filter(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .filter(|n| {
            // `03-test-selection.md` -> `tutorial_03`
            let num = n.split('-').next().unwrap_or("");
            !src.contains(&format!("fn tutorial_{num}"))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "these tutorials have no worked example under test: {missing:?}"
    );
}
