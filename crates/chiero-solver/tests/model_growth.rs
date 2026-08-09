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

#[test]
fn reading_a_model_back_does_not_grow_quadratically_in_variables() {
    // Sized so the largest point dominates process noise, as `chiero-gcov`'s growth gate had
    // to be re-sized for the same reason once its subject got fast.
    const SIZES: [usize; 4] = [500, 1000, 2000, 4000];
    let mut points = Vec::new();
    for n in SIZES {
        let (a, vars, text) = model_of(n);
        let t0 = Instant::now();
        let m = parse_model(&a, &text, &vars);
        let secs = t0.elapsed().as_secs_f64();
        // Keep the result alive and prove the run did the work.
        assert_eq!(
            m.get(vars[n - 1]).expect("last var").bits(),
            (n - 1) as u128
        );
        points.push((n, secs));
        eprintln!("n={n:5}  {:.4} s", secs);
    }

    // **A ratio, not a bound.** A wall-clock threshold stops being able to fail whenever the
    // machine gets faster; the growth *ratio* between two points is the property being claimed.
    // Doubling the variables doubles the work when the parse is linear and quadruples it when
    // it is not, so 3.0 separates the two with room for noise.
    for w in points.windows(2) {
        let (n0, t0) = w[0];
        let (n1, t1) = w[1];
        // Below a few milliseconds the clock is mostly scheduler noise; skip the ratio rather
        // than assert on it, and say so, since a silently skipped check reads as a pass.
        if t0 < 0.002 {
            eprintln!("n={n0} took {t0:.4} s — too fast to ratio against, skipped");
            continue;
        }
        let ratio = t1 / t0;
        assert!(
            ratio < 3.0,
            "{n0} -> {n1} vars cost {ratio:.1}x; a linear parse doubles, a quadratic one \
             quadruples. Points: {points:?}"
        );
    }
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
