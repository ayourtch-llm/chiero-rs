//! **A growth curve for native ingest — the instrument the `Vec::contains` audit was blocked on.**
//!
//! HANDOFF §9.1's audit item names two sites in `native.rs` that *look* quadratic
//! (`order.contains(&(a.from, a.to))` while `order` is filled from `f.arcs`; `slot.contains(...)`
//! inside the nested line loop) and then says, correctly, that **a reading is not a measurement**.
//! It was recorded rather than fixed because there are no `.gcno` files under `/home/ubuntu/vpp`
//! and the corpus fixtures are a few hundred bytes each — no curve, no evidence.
//!
//! **The block was a false one.** `.gcno` does not have to be found or hand-written: gcc emits
//! one of any size from generated C. `if (x == i) r += i;` repeated *n* times gives a function
//! with Θ(n) blocks and Θ(n) arcs, and running the binary produces the matching `.gcda`.
//!
//! Ratio per doubling is the whole point (§9.1): a single timing never distinguishes linear from
//! quadratic. Sizes quadruple here, so **4× per step is linear and 16× is quadratic**.
//!
//! # ⚠️ The input shape decides which defect is visible, and that is the real lesson here
//!
//! The first version of this file put one statement per source line. Every line then carries one
//! block, the per-line work looks linear, and the curve reported ~15×. **Three separate fixes were
//! tried against that curve and all three were reverted, because it could not see the dominant
//! cost.** Putting the same statements on *one* line — which is exactly what a multi-statement
//! macro expansion produces, and VPP is macro-heavy — moves every block onto that line:
//!
//! | shape | 200→800 | 800→3200 | n=3200 |
//! |---|---|---|---|
//! | `line` (one statement per line) | 11.5× | 16.4× | 1.10 s |
//! | `onelin` (all on one line) | 21.2× | **50.5×** | **17.1 s** |
//!
//! 50× per 4× arcs is worse than quadratic. **A growth curve is only as good as the shape it
//! grows**, and a generator that varies one parameter while holding the interesting one at 1 will
//! report a clean answer forever.

use std::path::Path;
use std::time::Instant;

/// One instrumented program with Θ(n) arcs in a single function, compiled and run.
///
/// Returns `None` if gcc is unavailable or refuses — the caller then **skips with a printed
/// reason** rather than passing, because a growth curve over zero points would report a clean
/// linear result forever.
fn build_and_run(dir: &Path, n: usize, sep: &str, tag: &str) -> Option<String> {
    let stem = format!("g{tag}{n}");
    let mut src = String::from("int f(int x) {\n  int r = 0;\n");
    for i in 0..n {
        // `sep` is the whole experiment — see `SHAPES`.
        src.push_str(&format!("  if (x == {i}) r += {i};{sep}"));
    }
    src.push_str("\n  return r;\n}\nint main(void) { return f(3) == 3 ? 0 : 1; }\n");
    let c = dir.join(format!("{stem}.c"));
    std::fs::write(&c, src).ok()?;
    let ok = std::process::Command::new("gcc")
        .args(["--coverage", "-O0", "-o"])
        .arg(dir.join(&stem))
        .arg(&c)
        .status()
        .ok()?
        .success();
    if !ok {
        return None;
    }
    // Running it is what writes the `.gcda`; ingest needs both halves.
    std::process::Command::new(dir.join(&stem))
        .current_dir(dir)
        .status()
        .ok()?;
    dir.join(format!("{stem}.gcda")).exists().then_some(stem)
}

/// Ignored: it shells out to gcc and runs what it builds. Run with
/// `cargo test -p chiero-gcov --test growth -- --ignored --nocapture`.
#[test]
#[ignore = "builds and runs instrumented programs with gcc"]
fn native_arc_ingest_does_not_grow_quadratically_in_arcs_per_function() {
    let dir = std::env::temp_dir().join(format!("chiero-gcov-growth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");

    const SIZES: [usize; 4] = [50, 200, 800, 3200];
    // **Two shapes, and the difference between them is the finding.** One statement per line
    // gives each source line a single block, and the per-line work looks linear. Put the same
    // statements on *one* line and every block lands on it — which is exactly what a
    // multi-statement macro expansion does, and VPP is macro-heavy. The second shape is
    // **17x slower at n=3200** and grows ~54x per 4x arcs where the first grows ~15x.
    const SHAPES: [(&str, &str); 2] = [("\n", "line"), (" ", "onelin")];
    let mut verdicts: Vec<(&str, f64)> = Vec::new();
    for (sep, tag) in SHAPES {
        let mut points: Vec<(usize, f64, u64)> = Vec::new();
        for n in SIZES {
            let Some(stem) = build_and_run(&dir, n, sep, tag) else {
                eprintln!("SKIPPED: gcc could not build the n={n} probe");
                return;
            };
            chiero_gcov::native::reset_circuit_starts();
            let start = Instant::now();
            let cov = chiero_gcov::native::arc_coverage(&dir, &stem).expect("ingest");
            let secs = start.elapsed().as_secs_f64();
            let starts = chiero_gcov::native::circuit_starts();
            assert!(
                !cov.functions().is_empty(),
                "n={n} ingested no functions, so the curve would time nothing"
            );
            points.push((n, secs, starts));
        }

        eprintln!("native arc ingest, ratio per 4x arcs (4x = linear, 16x = quadratic):");
        let mut worst: f64 = 0.0;
        for w in points.windows(2) {
            let ((n0, t0, c0), (n1, t1, c1)) = (w[0], w[1]);
            // A floor keeps a sub-millisecond first point from inventing a huge ratio out of noise.
            let ratio = t1 / t0.max(1e-4);
            eprintln!(
                "  {n0:>5} -> {n1:>5}   {t0:>8.4}s -> {t1:>8.4}s   {ratio:>6.1}x   \
                 circuit starts {c0} -> {c1}"
            );
            if n0 >= 200 {
                worst = worst.max(ratio);
            }
        }

        // **The bound is on the ratio, not the clock** — §9.1's rule, learned when the verifier's
        // 5-second assertion silently stopped being able to fail after `opt-level = 2` made every
        // build 6.7× faster. A ratio is invariant to machine speed and build profile; a duration is
        // not. The first step is excluded because n=50 is too fast to time honestly.
        eprintln!("  ^ shape: {tag}  worst {worst:.1}x per 4x arcs");
        verdicts.push((tag, worst));
    }

    // **Both shapes are measured before either is judged.** Asserting inside the loop hid the
    // second curve behind the first one's failure, and the *comparison* is the finding.
    let bad: Vec<_> = verdicts.iter().filter(|(_, w)| *w >= 8.0).collect();
    assert!(
        bad.is_empty(),
        "native arc ingest is superlinear: {bad:?} — ratio per 4x arcs, where 4x is linear and \
         16x is quadratic.\n⚠️ Three hypotheses have been tried and all three reverted (§9.1); \
         do not guess a fourth from reading. The `onelin` shape — every block on one source \
         line, which is what a multi-statement macro expansion produces and VPP is macro-heavy \
         — runs ~17x slower than `line` at n=3200 and grows ~54x per 4x arcs. That is the curve \
         to chase, and `cycles_count`'s `for &start in bs` is the only thing that scales with \
         blocks-per-line."
    );
}
