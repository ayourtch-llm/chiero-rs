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
//! Ratio per doubling is the whole point (§9.1): 2× is linear, 4× is quadratic, and a single
//! timing never distinguishes them. Sizes quadruple here, so **4× per step is linear and 16× is
//! quadratic**.

use std::path::Path;
use std::time::Instant;

/// One instrumented program with Θ(n) arcs in a single function, compiled and run.
///
/// Returns `None` if gcc is unavailable or refuses — the caller then **skips with a printed
/// reason** rather than passing, because a growth curve over zero points would report a clean
/// linear result forever.
fn build_and_run(dir: &Path, n: usize) -> Option<String> {
    let stem = format!("g{n}");
    let mut src = String::from("int f(int x) {\n  int r = 0;\n");
    for i in 0..n {
        src.push_str(&format!("  if (x == {i}) r += {i};\n"));
    }
    src.push_str("  return r;\n}\nint main(void) { return f(3) == 3 ? 0 : 1; }\n");
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
    let mut points: Vec<(usize, f64)> = Vec::new();
    for n in SIZES {
        let Some(stem) = build_and_run(&dir, n) else {
            eprintln!("SKIPPED: gcc could not build the n={n} probe");
            return;
        };
        let start = Instant::now();
        let cov = chiero_gcov::native::arc_coverage(&dir, &stem).expect("ingest");
        let secs = start.elapsed().as_secs_f64();
        assert!(
            !cov.functions().is_empty(),
            "n={n} ingested no functions, so the curve would time nothing"
        );
        points.push((n, secs));
    }
    let _ = std::fs::remove_dir_all(&dir);

    eprintln!("native arc ingest, ratio per 4x arcs (4x = linear, 16x = quadratic):");
    let mut worst: f64 = 0.0;
    for w in points.windows(2) {
        let ((n0, t0), (n1, t1)) = (w[0], w[1]);
        // A floor keeps a sub-millisecond first point from inventing a huge ratio out of noise.
        let ratio = t1 / t0.max(1e-4);
        eprintln!("  {n0:>5} -> {n1:>5}   {t0:>8.4}s -> {t1:>8.4}s   {ratio:>6.1}x");
        if n0 >= 200 {
            worst = worst.max(ratio);
        }
    }

    // **The bound is on the ratio, not the clock** — §9.1's rule, learned when the verifier's
    // 5-second assertion silently stopped being able to fail after `opt-level = 2` made every
    // build 6.7× faster. A ratio is invariant to machine speed and build profile; a duration is
    // not. The first step is excluded because n=50 is too fast to time honestly.
    assert!(
        worst < 8.0,
        "ingest cost grew {worst:.1}x per 4x arcs; 4x is linear and 16x is quadratic, so this \
         is the `order.contains(&(a.from, a.to))` shape the §9.1 audit predicted"
    );
}
