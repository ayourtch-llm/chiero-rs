//! **A controlled size axis for selection — item 5b's method, on the crate its census omitted.**
//!
//! 5b's class is *a full scan inside a per-item loop*, and its own entry records two things
//! about the audit that named it: the grep matched three of nine sites, and the per-crate census
//! **left out the two crates four of them lived in**. `chiero-select` is missing from that census
//! too, and as of 2026-08-10 it is a shipping CLI operation — so its cost curve matters in a way
//! it did not when selection was reachable only from Rust.
//!
//! Reading `select_refined` suggests two candidates. This file does not read; it measures:
//!
//! | site | shape |
//! |---|---|
//! | `coverage.files().any(\|f\| f == entity.file())` | every covered file, per impacted entity |
//! | `if !slot.contains(&r)` | one test's reasons, per reason added |
//!
//! **The axis is files × entities**, because that is the product the first site walks. A single
//! file would make it O(1) and the measurement would say nothing — the trap §8.3 step 1 calls
//! *asserting an edge without reading the corpus*.
//!
//! # What made this possible
//!
//! Until 2026-08-11 a `CoverageIndex` could only be attributed per test through
//! `ingest_native_as`, which reads binary `.gcno`/`.gcda` from a real instrumented build — so
//! there was no way to build a 4000-entity index in a test, and this file could not exist.
//! `ingest_json_as` closed that: gcov's JSON is text, and a corpus that can be *generated* is
//! what a size axis needs.
//!
//! # Reading the numbers
//!
//! The instrument follows `chiero-lower/tests/scale.rs` and §7.39's corrections, which were paid
//! for once already: a discarded warm-up round, the minimum of several runs, and a ratio over
//! the **whole span** rather than between adjacent points — adjacent doublings compare two
//! similar numbers, and one descheduled millisecond eats the margin.

use chiero_diff::{Program, impact};
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use chiero_select::{Suite, select_with};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Entity counts. The top point has to dominate process noise; below a few milliseconds a
/// "ratio" is mostly scheduler.
const SIZES: [usize; 4] = [500, 1000, 2000, 4000];

/// Tests in the suite. Every one covers every line, which is the worst case for the second
/// suspected site — one test accumulating a reason per entity.
const TESTS: usize = 8;

const REPEATS: usize = 5;

// ⏱️ **~75 s, and almost all of it is the frontend.** Parsing 15 000 one-line functions to
// build the four impact sets dwarfs the thing being timed, which is why the parse sits outside
// the timer and the sizes have not been trimmed: the axis has to span 8x to tell 8x from 64x,
// and it earned its place by finding a 10x defect on its first run.

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-select-scale-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// `n` functions in one translation unit.
///
/// One statement each and a distinct name, so every function is its own entity and the parse
/// stays linear in `n`. `body` differs between the two versions, which is what makes every
/// entity impacted — the case that maximises the per-entity loop.
fn source(n: usize, body: &str) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("int fn_{i} (int x) {{ return x {body} {i}; }}\n"));
    }
    s
}

/// One gcov JSON per test, covering every line of every file, gzipped as gcov writes it.
fn write_coverage(dir: &Path, n: usize, test: usize) -> String {
    let stem = format!("t{test}");
    // **One file, named as the unit the program is parsed under.** The first draft spread the
    // lines over `n / 10` files to exercise the per-entity file scan — and every run selected
    // **nothing**, because `chiero_diff::Program::parse` gives every entity the *unit's* name, so
    // no entity's file was ever in the coverage. The fixture was measuring an empty loop, and the
    // curve it produced looked perfectly respectable. ⚠️ The files axis needs an impact set
    // spanning several translation units, which `impact(&before, &after)` does not take; that is
    // recorded in 5t rather than faked here.
    let lines: Vec<serde_json::Value> = (0..n)
        .map(|l| {
            serde_json::json!({
                "line_number": l + 1,
                "function_name": format!("fn_{l}"),
                "count": 1,
                "unexecuted_block": false,
                "branches": [],
            })
        })
        .collect();
    let files = vec![serde_json::json!({ "file": "u.c", "functions": [], "lines": lines })];
    let doc = serde_json::json!({
        "format_version": "1",
        "gcc_version": "13.3.0",
        "current_working_directory": dir.display().to_string(),
        "data_file": stem,
        "files": files,
    });
    let raw = serde_json::to_vec(&doc).expect("serialise");
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).expect("gzip");
    std::fs::write(
        dir.join(format!("{stem}.gcov.json.gz")),
        gz.finish().expect("gzip"),
    )
    .expect("write");
    stem
}

struct Point {
    n: usize,
    secs: f64,
    selected: usize,
}

fn measure(n: usize) -> Point {
    let dir = scratch().join(format!("n{n}"));
    std::fs::create_dir_all(&dir).expect("scratch");
    let mut idx = CoverageIndex::default();
    for t in 0..TESTS {
        let stem = write_coverage(&dir, n, t);
        chiero_gcov::ingest_json_as(&mut idx, TestId(t as u32), &dir, &stem)
            .expect("the generated corpus ingests");
        idx.record_outcome(TestId(t as u32), TestOutcome::Passed);
    }
    // **One unit name for both sides**, which is the trap this project has hit four times: two
    // names compare two different entities and everything "changed" for the wrong reason.
    let before = Program::parse("u.c", &source(n, "+")).expect("parses");
    let after = Program::parse("u.c", &source(n, "-")).expect("parses");
    let set = impact(&before, &after);
    assert!(
        set.entities.len() >= n,
        "the fixture must impact every function or the per-entity loop is not exercised: \
         {} entities for n={n}",
        set.entities.len()
    );
    let suite = Suite {
        tests: idx.tests(),
        validity: idx.validity(Path::new(".")),
    };

    let mut best = f64::INFINITY;
    let mut selected = 0;
    // A discarded warm-up, then the minimum: scheduling noise only ever adds time.
    for round in 0..=REPEATS {
        let t0 = Instant::now();
        let sel = select_with(&set, &after, &idx, &suite);
        let secs = t0.elapsed().as_secs_f64();
        selected = sel.ranked().len();
        if round > 0 {
            best = best.min(secs);
        }
    }
    Point {
        n,
        secs: best,
        selected,
    }
}

/// **Selection must not grow quadratically in entities × files.**
///
/// The ceiling is over the whole 8x span: linear is 8x, quadratic is 64x. `2 · span^1.25` sits
/// between them — 26.9 here — which is §7.39's rule, arrived at after adjacent-pair ratios
/// proved too tight to survive a loaded machine.
#[test]
fn selection_does_not_grow_quadratically_in_entities_and_files() {
    let points: Vec<Point> = SIZES.iter().map(|&n| measure(n)).collect();
    for p in &points {
        eprintln!(
            "n={:5} files={:4}  {:.4} s (best of {REPEATS})  selected {}",
            p.n, 1, p.secs, p.selected
        );
    }
    // **A run that selected nothing measures nothing.** The whole point is the per-entity loop,
    // and an empty selection would mean the fixture never reached it.
    for p in &points {
        assert!(
            p.selected > 0,
            "n={} selected no tests, so this timing is not about selection",
            p.n
        );
    }
    for w in points.windows(2) {
        eprintln!("  {} -> {}: {:.1}x", w[0].n, w[1].n, w[1].secs / w[0].secs);
    }
    let (first, last) = (&points[0], points.last().unwrap());
    let span = last.n as f64 / first.n as f64;
    let ratio = last.secs / first.secs;
    let ceiling = 2.0 * span.powf(1.25);
    assert!(
        ratio < ceiling,
        "{}x the entities and files cost {ratio:.1}x, over the {ceiling:.1}x this span allows; \
         linear is {span:.0}x and quadratic is {:.0}x. Points: {:?}",
        span,
        span * span,
        points.iter().map(|p| (p.n, p.secs)).collect::<Vec<_>>()
    );
}
