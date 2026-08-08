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
//! # What it found, so nobody re-runs the dead ends
//!
//! ⚠️ **The two sites the audit named above are not the cost.** They were converted to `IndexSet`
//! and the ratio did not move; a third guess — hoisting `accumulate_line_info`'s arc scan into a
//! predecessor map, the exact fix that took the CIR verifier from hours to 2.4 s — moved nothing
//! either and was reverted. **Three confident readings, three misses** — and that was only the
//! start: the session finished at **six hypotheses refuted by measurement against six that held**.
//! Every one of the six wrong ones looked obvious in the source, and three of them predicted the
//! observed *shape* correctly while being wrong about the cause.
//!
//! What worked was counting. [`chiero_gcov::native::circuit_starts`] showed the cycle
//! enumeration's recursion running **5 128 004** times at n=3200 in the `onelin` shape against
//! **6 405** in `line` — quadratic against linear. Two O(1) lookups then followed from the
//! evidence rather than from reading, for a cumulative **17.31 s → 5.61 s (3.08×)**, with the
//! call count unchanged throughout — which is how you know they were cost-per-call changes and
//! not accidental semantic ones.
//!
//! Four more fixes followed, for **~250x in total** (17.31 s → ~0.068 s at n=3200, two runs; ±20% at these times): an acyclic
//! early-out that skips the enumeration entirely, the conservation fixpoint's incidence lists
//! hoisted out of its loop, `cycles_count`'s scratch allocated once per function, and — on the
//! third attempt, after the two costs hiding it were gone — the `accumulate_line_info` hoist that
//! had been reverted twice for moving nothing.
//!
//! **This test still fails, on purpose, and the residual is UNLOCATED.** ⚠️ Do not read the
//! paragraph above as pointing at it: `circuit` now runs **zero** times on this input, and the
//! enumeration is not the cost. Everything measured is linear — both decodes, the structure
//! build, the conservation fixpoint — and together they are under 3% of the clock at n=12800,
//! while the whole grows ~13.5x per 4x arcs.
//!
//! The next counter belongs in the `ArcCoverage` index building, and it must measure a unit whose
//! cost **tracks time**: a genuinely quadratic cell count (327 808 014) was fixed here and moved
//! the clock by ~7%. See §9.1 — six hypotheses on this item have been refuted by measurement, and
//! every one of them looked obvious in the source.
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

    // **12800 is here because the earlier top end stopped being informative.** After the fixes of
    // 2026-08-08 the n=3200 point runs in ~0.1 s, where process startup, file I/O and gcc's own
    // output size are a visible share of the measurement — a "ratio" there is partly noise. A
    // curve has to keep growing past the point where the thing it measures dominates the clock.
    const SIZES: [usize; 5] = [50, 200, 800, 3200, 12800];
    // **Two shapes, and the difference between them is the finding.** One statement per line
    // gives each source line a single block, and the per-line work looks linear. Put the same
    // statements on *one* line and every block lands on it — which is exactly what a
    // multi-statement macro expansion does, and VPP is macro-heavy. The second shape is
    // **17x slower at n=3200** and grows ~54x per 4x arcs where the first grows ~15x.
    const SHAPES: [(&str, &str); 2] = [("\n", "line"), (" ", "onelin")];
    let mut verdicts: Vec<(&str, f64)> = Vec::new();
    for (sep, tag) in SHAPES {
        let mut points: Vec<(usize, f64, u64, u64, f64, f64)> = Vec::new();
        for n in SIZES {
            let Some(stem) = build_and_run(&dir, n, sep, tag) else {
                eprintln!("SKIPPED: gcc could not build the n={n} probe");
                return;
            };
            // **The parse, timed on its own.** By elimination it is the last suspect for the
            // residual (§9.1), and elimination is not measurement — this is the measurement, and
            // it touches nothing in the solver to get it.
            let notes = dir.join(format!("{stem}.gcno"));
            let p0 = Instant::now();
            let recs = chiero_gcov::native::records(&notes).expect("parse notes");
            let parse = p0.elapsed().as_secs_f64();
            assert!(!recs.is_empty(), "n={n} parsed no records");

            // **Structure building, timed apart from byte decoding.** Conflating the two is what
            // sent §9.1's elimination to the wrong place: `records()` is linear and 0.4% of the
            // clock, but `read_notes` turns those records into functions, blocks, arcs and lines,
            // which is a different amount of work on the same bytes.
            let n0 = Instant::now();
            let note = chiero_gcov::native::read_notes(&notes).expect("read notes");
            let build = n0.elapsed().as_secs_f64();
            assert!(!note.functions.is_empty(), "n={n} built no functions");

            // The `.gcda` side of the byte decode. `records()` reads either artifact, so the data
            // half costs nothing extra to isolate — and until now the whole `.gcda` path had only
            // ever been measured inside the total.
            let d0 = Instant::now();
            let drecs = chiero_gcov::native::records(&dir.join(format!("{stem}.gcda")))
                .expect("parse data");
            let data = d0.elapsed().as_secs_f64();
            assert!(!drecs.is_empty(), "n={n} parsed no data records");

            chiero_gcov::native::reset_circuit_starts();
            let start = Instant::now();
            let cov = chiero_gcov::native::arc_coverage(&dir, &stem).expect("ingest");
            let secs = start.elapsed().as_secs_f64();
            let starts = chiero_gcov::native::circuit_starts();
            let visits = chiero_gcov::native::conservation_arc_visits()
                + chiero_gcov::native::cycles_cells();
            assert!(
                !cov.functions().is_empty(),
                "n={n} ingested no functions, so the curve would time nothing"
            );
            points.push((n, secs, starts, visits, parse + data, build));
        }

        eprintln!("native arc ingest, ratio per 4x arcs (4x = linear, 16x = quadratic):");
        let mut worst: f64 = 0.0;
        for w in points.windows(2) {
            let ((n0, t0, c0, v0, p0, b0), (n1, t1, c1, v1, p1, b1)) = (w[0], w[1]);
            // A floor keeps a sub-millisecond first point from inventing a huge ratio out of noise.
            let ratio = t1 / t0.max(1e-4);
            eprintln!(
                "  {n0:>5} -> {n1:>5}   {t0:>8.4}s -> {t1:>8.4}s   {ratio:>6.1}x   \
                 circuit {c0}->{c1}  conservation {v0}->{v1}  |  parse+data {:.1}x  build {b0:.4}s->{b1:.4}s = {:.1}x",
                p1 / p0.max(1e-4),
                b1 / b0.max(1e-4)
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
         16x is quadratic.\n⚠️ This is queued algorithmic work, not a regression: five fixes have \
         landed against this curve for a cumulative ~250x, and the scoreboard is 6 hypotheses \
         refuted to 5 held (§9.1). **Do not guess a sixth from reading** — every wrong one looked \
         obvious in the source and every right one came from a counter or this curve.\n\
         What the printed columns already rule out: `circuit` is 0 (the acyclic early-out fires \
         on this input), `conservation` is exactly linear, and `parse+data`/`build` are linear \
         and under 3% of the clock at n=12800. The two shapes now sit within noise of each \
         other, so blocks-per-line is no longer the discriminator it was.\n\
         The largest block still measured only as part of a whole is the `ArcCoverage` index \
         building — `line_blocks`, `counts`, `tests`, `order`, each keyed by a `FuncKey` holding \
         two `String`s and cloned per insert. Counter first, in units that track *time* \
         (allocations, hash probes); a quadratic counter is not automatically the bottleneck \
         either, which is how the last one cost a wave."
    );
}
