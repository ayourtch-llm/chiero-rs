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
int gate (int x);
";

/// The library: three functions expand `SCALE` or `OFFSET`, one expands neither.
const LIB: &str = "\
#include \"lib.h\"
int scaled (int x) { return SCALE (x); }
int offset (int x) { return OFFSET (x); }
int both (int x) { return SCALE (OFFSET (x)); }
int untouched (int x) { return x - 1; }
int gate (int x) { return x > 0 ? 1 : 0; }
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
    Case {
        name: "t_gate",
        body: "return gate (0) == 0 ? 0 : 1;",
    },
];

/// One mutation: which file it edits, and the replacement.
struct Mutation {
    name: &'static str,
    /// `"lib.h"` or `"lib.c"` — which side of the comparison it is a test of.
    file: &'static str,
    from: &'static str,
    to: &'static str,
}

/// The mutations.
///
/// **Both kinds, deliberately.** The macro-body edits are the case 032 §2 exists for and the one
/// a coverage-only tool provably fails. The `.c` edits are the ordinary case — a flipped
/// comparison, a changed constant, exactly what §6 lists beside them — where a coverage-only tool
/// **works**, because the changed line is one gcov recorded.
///
/// A gate containing only the first kind would be a rigged comparison: it would report a baseline
/// of 0% and say nothing about whether chiero is *worse* anywhere. The second kind is what makes
/// the two numbers a measurement rather than an advertisement.
const MUTATIONS: &[Mutation] = &[
    // --- macro bodies, in the header: no .c file is touched ---------------------------------
    Mutation {
        name: "SCALE x3",
        file: "lib.h",
        from: "#define SCALE(v) ((v) * 2)",
        to: "#define SCALE(v) ((v) * 3)",
    },
    Mutation {
        name: "SCALE x0",
        file: "lib.h",
        from: "#define SCALE(v) ((v) * 2)",
        to: "#define SCALE(v) ((v) * 0)",
    },
    Mutation {
        name: "SCALE +1",
        file: "lib.h",
        from: "#define SCALE(v) ((v) * 2)",
        to: "#define SCALE(v) ((v) * 2 + 1)",
    },
    Mutation {
        name: "OFFSET +2",
        file: "lib.h",
        from: "#define OFFSET(v) ((v) + 1)",
        to: "#define OFFSET(v) ((v) + 2)",
    },
    Mutation {
        name: "OFFSET -1",
        file: "lib.h",
        from: "#define OFFSET(v) ((v) + 1)",
        to: "#define OFFSET(v) ((v) - 1)",
    },
    Mutation {
        name: "OFFSET x2",
        file: "lib.h",
        from: "#define OFFSET(v) ((v) + 1)",
        to: "#define OFFSET(v) (((v) + 1) * 2)",
    },
    // --- ordinary code, in the .c: the case a coverage-only tool handles --------------------
    Mutation {
        name: "const",
        file: "lib.c",
        from: "return x - 1;",
        to: "return x - 2;",
    },
    Mutation {
        name: "compare",
        file: "lib.c",
        from: "int gate (int x) { return x > 0 ? 1 : 0; }",
        to: "int gate (int x) { return x >= 0 ? 1 : 0; }",
    },
];

fn write_tree(dir: &Path, header: &str, lib: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("lib.h"), header)?;
    std::fs::write(dir.join("lib.c"), lib)?;
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

/// Which cases chiero selects, **through the whole pipeline**.
///
/// ⚠️ The first version of this function called `chiero_diff::impact` and stopped there, so the
/// gate measured 031 and reported it as though it had measured selection. It now runs what a user
/// would: one impact set for the change, one coverage index over the suite, one `select_with` —
/// 030's index, 031's closure and 032's intersection and safety set, in that order.
///
/// The unit compared is the **library**, because that is what the change is in. The tests are
/// whatever the coverage index attributes to the impacted entities' lines, which is 032 §2's join
/// and not a proxy for it.
fn chiero_selects(
    dir: &Path,
    before: &str,
    after: &str,
    lib_before: &str,
    lib_after: &str,
    coverage: &chiero_gcov::CoverageIndex,
) -> Vec<String> {
    // **The unit is named with the path gcov recorded.** `CoverageIndex` stores paths "as gcov
    // wrote them" and says resolving them is the caller's job (030); gcc compiled with an
    // absolute path, so an entity called `lib.c` matches nothing in the index and the join
    // silently yields no tests. Naming the unit the same way is what a real tool does — it knows
    // its own build directory — and the alternative, matching by basename inside `select`, would
    // conflate two files of the same name in different directories.
    let unit = dir.join("lib.c");
    let unit = unit.to_string_lossy().into_owned();
    let mut cfg = chiero_pp::Config::default();
    cfg.iquote_paths.push(dir.to_path_buf());
    let a = chiero_diff::Program::parse_with(
        &unit,
        lib_before,
        cfg.clone(),
        &mut Header(before.to_string()),
    );
    let b = chiero_diff::Program::parse_with(&unit, lib_after, cfg, &mut Header(after.to_string()));
    let (Some(a), Some(b)) = (a, b) else {
        // Unparseable is 031 §4's gap: the answer widens to the whole suite.
        return CASES.iter().map(|c| c.name.to_string()).collect();
    };

    let suite = chiero_select::Suite {
        tests: (0..CASES.len() as u32).map(chiero_gcov::TestId).collect(),
        ..Default::default()
    };
    let selection = chiero_select::select_with(&chiero_diff::impact(&a, &b), &b, coverage, &suite);
    // **A `Reduced` confidence here means the join did not land**, not that the change was
    // subtle: every entity of this fixture has coverage. Surfaced rather than swallowed, because
    // a pipeline that quietly finds no coverage selects nothing and looks like a great reduction.
    if let chiero_select::Confidence::Reduced { reasons } = &selection.confidence {
        eprintln!("  [confidence reduced] {}", reasons.join("; "));
    }
    selection
        .tests
        .keys()
        .filter_map(|t| CASES.get(t.0 as usize).map(|c| c.name.to_string()))
        .collect()
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
fn coverage_only_selects(
    coverage: &chiero_gcov::CoverageIndex,
    file: &str,
    changed_line: u32,
) -> Vec<String> {
    // **Matched by basename**, because gcov records the path *it* saw — an absolute path under
    // the build directory — while a diff names the file relative to the tree. Looking up the bare
    // name missed every time, and the failure was silent and *flattering*: it reported the
    // baseline as 0% on the `.c` mutations, where a coverage-only tool genuinely works. A gate
    // whose bug favours the tool it is measuring is worse than no gate.
    let Some(indexed) = coverage
        .files()
        .find(|f| std::path::Path::new(f).file_name() == std::path::Path::new(file).file_name())
    else {
        return Vec::new();
    };
    match coverage.tests_for_line(indexed, changed_line) {
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
    if write_tree(&root, HEADER, LIB).is_err() {
        eprintln!("mutation gate: could not write the fixture tree");
        return 1;
    }

    let coverage = measure_coverage(&root);
    let baseline_measured = !coverage.files().collect::<Vec<_>>().is_empty();

    let mut rows: Vec<(String, usize, usize, usize, usize)> = Vec::new();
    let (mut truth_total, mut chiero_hits, mut baseline_hits) = (0usize, 0usize, 0usize);
    let (mut chiero_selected_total, mut suite_total) = (0usize, 0usize);

    for m in MUTATIONS {
        let (header, lib) = match m.file {
            "lib.h" => (HEADER.replace(m.from, m.to), LIB.to_string()),
            _ => (HEADER.to_string(), LIB.replace(m.from, m.to)),
        };
        if header == HEADER && lib == LIB {
            eprintln!(
                "mutation gate: `{}` matched nothing — the fixture drifted",
                m.name
            );
            return 1;
        }

        // Ground truth: which cases actually fail once the mutation is applied.
        if write_tree(&root, &header, &lib).is_err() {
            return 1;
        }
        let truth = failing_cases(&root);
        // Restore, so chiero sees the same tree the suite was measured on.
        if write_tree(&root, HEADER, LIB).is_err() {
            return 1;
        }

        let picked = chiero_selects(&root, HEADER, &header, LIB, &lib, &coverage);
        // **The line the diff touched, found in the file** rather than hard-coded: a constant
        // would silently point at the wrong line the moment the fixture grew.
        let original = if m.file == "lib.h" { HEADER } else { LIB };
        let needle = m.from.lines().next().unwrap_or(m.from);
        let line = original
            .lines()
            .position(|l| l.contains(needle))
            .map(|i| i as u32 + 1)
            .unwrap_or(1);
        let baseline = coverage_only_selects(&coverage, m.file, line);

        let hit = truth.iter().filter(|t| picked.contains(t)).count();
        let bhit = truth.iter().filter(|t| baseline.contains(t)).count();
        truth_total += truth.len();
        chiero_hits += hit;
        baseline_hits += bhit;
        chiero_selected_total += picked.len();
        suite_total += CASES.len();
        rows.push((
            format!(
                "{} [{}]",
                m.name,
                if m.file == "lib.h" { "hdr" } else { ".c" }
            ),
            truth.len(),
            hit,
            bhit,
            picked.len(),
        ));
    }

    println!(
        "032 contract 19 — mutation gate over {} mutations",
        MUTATIONS.len()
    );
    println!(
        "  {:<18} {:>6} {:>8} {:>10} {:>9}",
        "mutation", "failed", "chiero", "cov-only", "selected"
    );
    for (name, truth, hit, bhit, picked) in &rows {
        println!("  {name:<18} {truth:>6} {hit:>8} {bhit:>10} {picked:>9}");
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
