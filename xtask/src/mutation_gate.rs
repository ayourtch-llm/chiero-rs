//! **032 contract 19 — the mutation gate, and the quantitative statement of the premise.**
//!
//! > 19. Over N macro-body mutations, chiero's recall is 100% and the coverage-only baseline's is
//! >     measured and reported (expected: near zero) — the quantitative statement of the
//! >     project's premise.
//!
//! 032 §6 puts both harnesses here rather than in a unit test, and the reason is that a gate has
//! to *run the tests*:
//!
//! > A test selector that is never measured drifts into being a random sampler.
//!
//! # What is measured
//!
//! For each mutation of a macro body in a header:
//!
//! 1. build and run the suite on the original tree, recording each test's output **and its
//!    coverage**, per test, with `--coverage` and a per-test `GCOV_PREFIX`;
//! 2. apply the mutation, rebuild, rerun: the tests whose output changed are the ones that
//!    *would have caught it* — the ground truth, observed rather than assumed;
//! 3. ask chiero which tests it would select for that diff, and ask the **coverage-only
//!    baseline** the same;
//! 4. recall = selected ∩ ground-truth ÷ ground-truth, for each.
//!
//! **The baseline is implemented, not asserted.** 032 §6 is explicit about why: *"so every report
//! can state the delta chiero adds rather than asserting it."* A number this project quotes about
//! its own premise must come from running the alternative, not from reasoning about it.
//!
//! The baseline is the honest version of what a coverage-only tool does: intersect the diff's
//! changed `(file, line)` pairs with the coverage index. For a header-only macro edit it looks up
//! the macro's definition line, which gcov never records (030 §1, measured) — so it finds nothing.
//!
//! # Why a purpose-built tree rather than VPP
//!
//! VPP's suite needs root and network namespaces, so its tests cannot be run here at all — and a
//! gate that cannot run is not a gate. The tree below is small and its *shape* is what matters:
//! a macro in a header, several functions expanding it, several tests exercising different
//! subsets, and one test that touches none of it and must **not** be selected. A gate that only
//! measured recall would be satisfied by selecting everything.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One test of the fixture suite: a name and the `main` that exercises some of the library.
struct Case {
    name: &'static str,
    body: &'static str,
}

/// The header every mutation edits. `SCALE` is the macro under test.
const HEADER: &str = "\
#define SCALE(v) ((v) * 2)
#define OFFSET(v) ((v) + 1)
int scaled (int x);
int offset (int x);
int both (int x);
int untouched (int x);
";

/// The library: three functions expand `SCALE` or `OFFSET`, one expands neither.
const LIB: &str = "\
#include \"lib.h\"
int scaled (int x) { return SCALE (x); }
int offset (int x) { return OFFSET (x); }
int both (int x) { return SCALE (OFFSET (x)); }
int untouched (int x) { return x - 1; }
";

const CASES: &[Case] = &[
    Case {
        name: "t_scaled",
        body: "return scaled (3) == 6 ? 0 : 1;",
    },
    Case {
        name: "t_offset",
        body: "return offset (3) == 4 ? 0 : 1;",
    },
    Case {
        name: "t_both",
        body: "return both (3) == 8 ? 0 : 1;",
    },
    Case {
        name: "t_untouched",
        body: "return untouched (3) == 2 ? 0 : 1;",
    },
];

/// The mutations, each a replacement of one macro's replacement list.
///
/// Every one is a **macro body** edit in a header, with no `.c` file touched — the case 032 §2
/// exists for and the case a coverage-only tool provably fails.
const MUTATIONS: &[(&str, &str, &str)] = &[
    (
        "SCALE x3",
        "#define SCALE(v) ((v) * 2)",
        "#define SCALE(v) ((v) * 3)",
    ),
    (
        "SCALE x0",
        "#define SCALE(v) ((v) * 2)",
        "#define SCALE(v) ((v) * 0)",
    ),
    (
        "SCALE +1",
        "#define SCALE(v) ((v) * 2)",
        "#define SCALE(v) ((v) * 2 + 1)",
    ),
    (
        "OFFSET +2",
        "#define OFFSET(v) ((v) + 1)",
        "#define OFFSET(v) ((v) + 2)",
    ),
    (
        "OFFSET -1",
        "#define OFFSET(v) ((v) + 1)",
        "#define OFFSET(v) ((v) - 1)",
    ),
    (
        "OFFSET x2",
        "#define OFFSET(v) ((v) + 1)",
        "#define OFFSET(v) (((v) + 1) * 2)",
    ),
];

fn write_tree(dir: &Path, header: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("lib.h"), header)?;
    std::fs::write(dir.join("lib.c"), LIB)?;
    for c in CASES {
        std::fs::write(
            dir.join(format!("{}.c", c.name)),
            format!("#include \"lib.h\"\nint main (void) {{ {} }}\n", c.body),
        )?;
    }
    Ok(())
}

/// Build and run every case, returning the ones that **fail** — the ground truth for a mutation.
fn failing_cases(dir: &Path) -> Vec<String> {
    let mut failed = Vec::new();
    for c in CASES {
        let bin = dir.join(c.name);
        let built = Command::new("gcc")
            .args(["-O0", "-I"])
            .arg(dir)
            .arg("-o")
            .arg(&bin)
            .arg(dir.join(format!("{}.c", c.name)))
            .arg(dir.join("lib.c"))
            .status();
        match built {
            Ok(s) if s.success() => {}
            _ => {
                // A mutation that does not compile would have been caught by the build, and the
                // suite's answer is "everything" — recorded as every case failing rather than
                // silently as none.
                failed.push(c.name.to_string());
                continue;
            }
        }
        if !Command::new(&bin)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            failed.push(c.name.to_string());
        }
    }
    failed
}

/// Which cases chiero selects for a header-only macro edit.
///
/// The chain is the real one: `chiero-diff` compares the two trees per case, and the selection is
/// the union of every case whose own translation unit is impacted. A test *is* a translation
/// unit here, so "the test is impacted" is the honest reading of "the test would be selected".
fn chiero_selects(dir: &Path, before: &str, after: &str) -> Vec<String> {
    let mut out = Vec::new();
    for c in CASES {
        let src = std::fs::read_to_string(dir.join(format!("{}.c", c.name))).unwrap_or_default();
        let lib = std::fs::read_to_string(dir.join("lib.c")).unwrap_or_default();
        // Each case is compiled with the library, so the unit chiero analyses is both.
        let whole = format!("{src}\n{lib}");
        let mut cfg = chiero_pp::Config::default();
        cfg.iquote_paths.push(dir.to_path_buf());
        let a = chiero_diff::Program::parse_with(
            "unit.c",
            &whole,
            cfg.clone(),
            &mut Header(before.to_string()),
        );
        let b =
            chiero_diff::Program::parse_with("unit.c", &whole, cfg, &mut Header(after.to_string()));
        let (Some(a), Some(b)) = (a, b) else {
            // Unparseable is 031 §4's gap: the answer widens, so the case is selected.
            out.push(c.name.to_string());
            continue;
        };
        // **The test's own entry point**, not "any entity of the unit". Every case is compiled
        // against the whole library, so a unit always *contains* the impacted functions; what
        // decides whether the test would be selected is whether the change reaches `main`, which
        // is exactly what 031's closure computes. Asking the looser question made the harness
        // select all four cases every time and report 0% reduction — a recall gate that selects
        // everything is satisfied and worthless, which is why contract 20 pairs the two numbers.
        if chiero_diff::impact(&a, &b)
            .entities
            .contains_key(&chiero_diff::Entity::function("unit.c", "main"))
        {
            out.push(c.name.to_string());
        }
    }
    out
}

/// A loader answering for the fixture's one header.
struct Header(String);

impl chiero_pp::FileLoader for Header {
    fn load(&mut self, path: &Path) -> std::io::Result<String> {
        if path.file_name().and_then(|f| f.to_str()) == Some("lib.h") {
            return Ok(self.0.clone());
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} is not part of the fixture", path.display()),
        ))
    }
}

/// What a coverage-only tool selects: the tests covering the diff's changed lines.
///
/// **The diff touches only `lib.h`'s macro definition line**, and gcov records no entry for a
/// macro's definition — 030 §1 measured it and `tests/corpus/coverage/` pins it. So every lookup
/// misses and the selection is empty. Implemented rather than asserted, because 032 §6 requires
/// the delta chiero adds to be *stated* rather than claimed.
fn coverage_only_selects(coverage: &chiero_gcov::CoverageIndex, changed_line: u32) -> Vec<String> {
    match coverage.tests_for_line("lib.h", changed_line) {
        Some(ts) => ts
            .iter()
            .filter_map(|t| CASES.get(t.0 as usize).map(|c| c.name.to_string()))
            .collect(),
        None => Vec::new(),
    }
}

/// Build the suite with coverage and ingest each case's own `.gcda`, so the baseline is measured
/// against real artifacts rather than an assumption about what they would contain.
fn measure_coverage(dir: &Path) -> chiero_gcov::CoverageIndex {
    let mut idx = chiero_gcov::CoverageIndex::default();
    for (i, c) in CASES.iter().enumerate() {
        let obj = dir.join(format!("cov_{}", c.name));
        let _ = std::fs::create_dir_all(&obj);
        let bin = obj.join(c.name);
        let ok = Command::new("gcc")
            .args(["--coverage", "-O0", "-I"])
            .arg(dir)
            .arg("-o")
            .arg(&bin)
            .arg(dir.join(format!("{}.c", c.name)))
            .arg(dir.join("lib.c"))
            .current_dir(&obj)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            continue;
        }
        let _ = Command::new(&bin).current_dir(&obj).status();
        // gcc names a one-step compile-and-link's notes `<output>-<source>`, so the stems are
        // `t_scaled-lib` and `t_scaled-t_scaled` rather than `lib` and `t_scaled`. Measured from
        // the directory rather than assumed — the first version of this harness ingested nothing
        // and reported the baseline as "not measured", which is at least a loud failure.
        for stem in [format!("{}-lib", c.name), format!("{0}-{0}", c.name)] {
            let _ =
                chiero_gcov::ingest_native_as(&mut idx, chiero_gcov::TestId(i as u32), &obj, &stem);
        }
    }
    idx
}

/// Run the gate. Returns 0 when chiero's recall is 100%.
pub fn mutation_gate() -> i32 {
    let root = std::env::temp_dir().join("chiero-mutation-gate");
    let _ = std::fs::remove_dir_all(&root);
    if write_tree(&root, HEADER).is_err() {
        eprintln!("mutation gate: could not write the fixture tree");
        return 1;
    }

    let coverage = measure_coverage(&root);
    let baseline_measured = !coverage.files().collect::<Vec<_>>().is_empty();

    let mut rows: Vec<(String, usize, usize, usize, usize)> = Vec::new();
    let (mut truth_total, mut chiero_hits, mut baseline_hits) = (0usize, 0usize, 0usize);
    let (mut chiero_selected_total, mut suite_total) = (0usize, 0usize);

    for (name, from, to) in MUTATIONS {
        let mutated = HEADER.replace(from, to);
        if mutated == HEADER {
            eprintln!("mutation gate: `{name}` matched nothing — the fixture drifted");
            return 1;
        }

        // Ground truth: which cases actually fail once the mutation is applied.
        if write_tree(&root, &mutated).is_err() {
            return 1;
        }
        let truth = failing_cases(&root);
        // Restore, so chiero sees the same tree the suite was measured on.
        if write_tree(&root, HEADER).is_err() {
            return 1;
        }

        let picked = chiero_selects(&root, HEADER, &mutated);
        // The macro's definition line: 1 for SCALE, 2 for OFFSET.
        let line = if name.starts_with("SCALE") { 1 } else { 2 };
        let baseline = coverage_only_selects(&coverage, line);

        let hit = truth.iter().filter(|t| picked.contains(t)).count();
        let bhit = truth.iter().filter(|t| baseline.contains(t)).count();
        truth_total += truth.len();
        chiero_hits += hit;
        baseline_hits += bhit;
        chiero_selected_total += picked.len();
        suite_total += CASES.len();
        rows.push(((*name).to_string(), truth.len(), hit, bhit, picked.len()));
    }

    println!(
        "032 contract 19 — mutation gate over {} macro-body mutations",
        MUTATIONS.len()
    );
    println!(
        "  {:<12} {:>6} {:>8} {:>10} {:>9}",
        "mutation", "failed", "chiero", "cov-only", "selected"
    );
    for (name, truth, hit, bhit, picked) in &rows {
        println!("  {name:<12} {truth:>6} {hit:>8} {bhit:>10} {picked:>9}");
    }

    let recall = |hits: usize| {
        if truth_total == 0 {
            0.0
        } else {
            100.0 * hits as f64 / truth_total as f64
        }
    };
    println!(
        "\n  chiero recall        {:.1}%  ({chiero_hits}/{truth_total})",
        recall(chiero_hits)
    );
    println!(
        "  coverage-only recall {:.1}%  ({baseline_hits}/{truth_total}){}",
        recall(baseline_hits),
        if baseline_measured {
            ""
        } else {
            "  [no coverage was collected — the baseline is not measured]"
        }
    );
    // **Reduction beside safety**, always (contract 20): a recall number alone would be satisfied
    // by selecting the whole suite.
    println!(
        "  reduction            {:.1}%  ({} of {} case-selections avoided)",
        100.0 * (suite_total - chiero_selected_total) as f64 / suite_total as f64,
        suite_total - chiero_selected_total,
        suite_total
    );

    if truth_total == 0 {
        eprintln!("\nFAIL: no mutation changed any test's result — the gate measured nothing");
        return 1;
    }
    if !baseline_measured {
        eprintln!("\nFAIL: the coverage-only baseline was not measured, so the delta is a claim");
        return 1;
    }
    if chiero_hits < truth_total {
        eprintln!("\nFAIL: recall is not 100% — a mutation would have shipped");
        return 1;
    }
    println!("\nPASS: recall 100%, and the coverage-only baseline is measured beside it");
    0
}

/// The fixture root, exposed so a caller can inspect it after a failure.
pub fn gate_root() -> PathBuf {
    std::env::temp_dir().join("chiero-mutation-gate")
}
