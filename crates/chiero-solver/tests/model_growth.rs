//! **Reading a model back must not be quadratic in the number of variables.**
//!
//! Found by sampling a real run: `find-bugs` on `plugins/nsh/nsh_node.c --entry nsh_md2_encap`
//! ran past 120 s with nothing to show, and two stack samples 50 s apart both landed in
//! `parse_model`. It was not waiting on z3 — it was reading z3's answer.
//!
//! The shape is item 5b's, in different clothes: a **full scan inside a per-item loop**.
//! `parse_model` did `text.split(&format!("define-fun {key} "))` once per variable, and the
//! text it scans grows with the variable count, so the work is O(V²) — invisible on the small
//! models every fixture produces and decisive on a real path condition.

use chiero_solver::{Sort, TermArena, parse_model};
use std::time::Instant;

/// Build a model answer of `n` variables, as z3 prints one.
fn model_of(n: usize) -> (TermArena, Vec<chiero_solver::VarId>, String) {
    let mut a = TermArena::new();
    let mut vars = Vec::new();
    let mut text = String::from("(\n");
    for i in 0..n {
        let name = format!("x{i}");
        let t = a.var(Sort::BitVec(32), &name);
        let v = a.var_id(t).expect("a var has an id");
        // The value is the index, so a parser that returns the *wrong* variable's value —
        // or zero, which `unwrap_or(0)` makes the silent failure mode — is visible.
        text.push_str(&format!(
            "  (define-fun v{}_{name} () (_ BitVec 32) #x{:08x})\n",
            v.0, i
        ));
        vars.push(v);
    }
    text.push_str(")\n");
    (a, vars, text)
}

/// Correctness first: the growth assertion below is worthless if the parse is wrong.
#[test]
fn every_variable_reads_back_its_own_value() {
    let (a, vars, text) = model_of(64);
    let m = parse_model(&a, &text, &vars);
    for (i, v) in vars.iter().enumerate() {
        let got = m.get(*v).expect("a value for every variable");
        assert_eq!(
            got.bits(),
            i as u128,
            "variable {i} read back the wrong value"
        );
        assert_eq!(got.width(), 32);
    }
}

/// The size axis. **Sized so the largest point dominates process noise**, which is the rule
/// `chiero-gcov/tests/growth.rs` states as *"a curve has to keep growing past the point where
/// the thing it measures dominates the clock"*. At 500..4000 the whole test ran in ~15 ms and a
/// single deschedule was a large share of a 3 ms point.
const SIZES: [usize; 4] = [8000, 16000, 32000, 64000];

/// The minimum of five timings per size, which is the rule `chiero-lower/tests/scale.rs`
/// already carries and this neighbour never got.
const REPEATS: usize = 5;

/// One measurement of the curve: the best time seen at each size.
///
/// **Rounds over all sizes, and the first one thrown away.** Timing each size to completion in
/// turn charges the whole warm-up — allocator growth, first-touch page faults, the branch
/// predictor — to whichever size runs first, and that is the *denominator* of the first ratio.
/// Measured that way the n=8000 point came out slower than n=16000, which is not a property of
/// the parse. One discarded round warms it; the remaining rounds interleave, so a slow patch of
/// machine spreads over every size instead of landing on one.
fn curve() -> Vec<(usize, f64)> {
    let models: Vec<_> = SIZES.iter().map(|&n| (n, model_of(n))).collect();
    let time_one = |i: usize| -> f64 {
        let (n, (a, vars, text)) = &models[i];
        let t0 = Instant::now();
        let m = parse_model(a, text, vars);
        let secs = t0.elapsed().as_secs_f64();
        // Keep the result alive and prove the run did the work.
        assert_eq!(
            m.get(vars[n - 1]).expect("last var").bits(),
            (n - 1) as u128
        );
        // ⚠️ **A tripwire, and it is the one absolute number here.** The mutation that
        // reintroduces the O(V²) scan ran past **ten minutes** without reaching an assertion:
        // the size axis is chosen for a *linear* parse, where n=64000 costs ~0.1 s, and that
        // same point costs ~90 s quadratic with two dozen parses to do. A gate that has to be
        // killed rather than read is worse than one that is merely slow.
        //
        // It is a wall-clock bound, which this project normally refuses — but it is not the
        // gate, and it fails in only one direction: the largest linear point is 0.1 s, so 2 s
        // is 20x of headroom against any scheduling noise, and a *faster* machine only makes it
        // safer. The verdict below is still a ratio.
        assert!(
            secs < 2.0,
            "one parse of {n} variables took {secs:.1} s. The largest point on this axis costs \
             ~0.1 s when the parse is linear, so this is not noise — it is the O(V²) shape this \
             test exists for, and letting the full measurement run would take tens of minutes."
        );
        secs
    };

    // The warm-up round, discarded: see this function's own note.
    for i in 0..models.len() {
        time_one(i);
    }

    let mut best = vec![f64::INFINITY; SIZES.len()];
    for _ in 0..REPEATS {
        for (i, b) in best.iter_mut().enumerate() {
            *b = b.min(time_one(i));
        }
    }
    SIZES.iter().copied().zip(best).collect()
}

/// **The whole span, not adjacent pairs.** Adjacent doublings compare two points of similar
/// size, so linear is 2.0x against a ceiling of 3.0 — a 1.5x margin, which one descheduled
/// millisecond eats. Across the full 8x span linear is 8x and the quadratic this test exists
/// for was 3.4-4.1x *per doubling*, which is 50-60x over the same span.
///
/// Written as a function of the span so the same rule can be applied to a *partial* curve while
/// it is still being measured: `2 · span^1.25` sits between `span` (linear) and `span²`
/// (quadratic) at every span this test uses — 4.8/11.3/26.9 against 2/4/8 and 4/16/64.
fn ceiling(span: f64) -> f64 {
    2.0 * span.powf(1.25)
}

#[test]
fn reading_a_model_back_does_not_grow_quadratically_in_variables() {
    // ⚠️ **A clock is not a counter, and this is a clock.** 011 c13 and §8.3 both say to
    // measure work rather than time, and `parse_model` exposes no work counter to measure — so
    // what is left is a timing read as carefully as a timing can be. Every cheaper defence was
    // tried against a *fully loaded* machine (12 spinners on 12 cores, which is what
    // `cargo test --workspace` does to itself) and each failed on its own:
    //
    // | defence | failures in 10 |
    // |---|---|
    // | one timing per size, adjacent ratios < 3.0 | 3 |
    // | + minimum of 5 | 4 |
    // | + minimum of 9 | 3 |
    // | + a 10x larger size axis, warm-up round, full-span ratio | 1 in 12 |
    //
    // More samples was never the lever: at these sizes *every* round of the top point gets
    // preempted, so its minimum is inflated too. **Retrying the whole curve is**, because the
    // claim is about the smallest ratio the code can produce, and a quadratic parse cannot
    // produce a clean one however many times it is asked. Three attempts take one attempt's
    // time whenever the machine is quiet.
    const ATTEMPTS: usize = 3;
    let mut seen = Vec::new();
    for attempt in 1..=ATTEMPTS {
        let points = curve();
        let (n0, t0) = points[0];
        let (n1, t1) = *points.last().unwrap();
        let ratio = t1 / t0;
        for (n, t) in &points {
            eprintln!("n={n:5}  {t:.4} s (best of {REPEATS}, after a discarded warm-up round)");
        }
        for w in points.windows(2) {
            eprintln!("  {} -> {} vars: {:.1}x", w[0].0, w[1].0, w[1].1 / w[0].1);
        }
        eprintln!("attempt {attempt}: {n0} -> {n1} vars cost {ratio:.1}x");
        if ratio < ceiling(SIZES[SIZES.len() - 1] as f64 / SIZES[0] as f64) {
            return;
        }
        seen.push((ratio, points));
    }
    let span = SIZES[SIZES.len() - 1] as f64 / SIZES[0] as f64;
    panic!(
        "{} attempts and the best of them cost {:.1}x for {span:.0}x the variables; a linear \
         parse costs {span:.0}x and the quadratic one this test exists for cost ~55x. \
         Attempts: {seen:?}",
        ATTEMPTS,
        seen.iter().map(|(r, _)| *r).fold(f64::INFINITY, f64::min),
    );
}

/// **A `Bool` reads its own value, not the next variable's.**
///
/// z3 prints `(define-fun v0_b () Bool true)` — no `#x` or `#b` token anywhere in it. The
/// pre-2026-08-09 parser searched *everything after the key* for the first bit-vector literal,
/// so it ran on into the following definition and gave the bool whatever it found there. The
/// model stayed plausible, which is why nothing caught it until the scan was bounded to one
/// entry and `a_bool_variable_is_usable` went red.
///
/// The fixture puts a non-zero bit-vector immediately after the bool on purpose: that value,
/// `0xdeadbeef`, is exactly what the old parser would return for the bool.
#[test]
fn a_bool_reads_its_own_value_and_not_the_following_variables() {
    let mut a = TermArena::new();
    let bt = a.var(Sort::Bool, "flag");
    let bv = a.var(Sort::BitVec(32), "word");
    let (bid, wid) = (a.var_id(bt).unwrap(), a.var_id(bv).unwrap());
    let text = format!(
        "(\n  (define-fun v{}_flag () Bool true)\n  \
         (define-fun v{}_word () (_ BitVec 32) #xdeadbeef)\n)\n",
        bid.0, wid.0
    );
    let m = parse_model(&a, &text, &[bid, wid]);
    assert_eq!(m.get(wid).expect("word").bits(), 0xdead_beef);
    let flag = m.get(bid).expect("flag");
    assert_ne!(
        flag.bits(),
        0xdead_beef,
        "the bool took the next variable's value — the defect this test exists for"
    );
    assert_eq!(flag.bits(), 1, "`true` is 1");

    // And `false` is 0 rather than absent, which `unwrap_or(0)` would make indistinguishable
    // from "no value at all" if the token were never recognised.
    let text_false = text.replace("Bool true", "Bool false");
    assert_eq!(
        parse_model(&a, &text_false, &[bid])
            .get(bid)
            .unwrap()
            .bits(),
        0
    );
}
