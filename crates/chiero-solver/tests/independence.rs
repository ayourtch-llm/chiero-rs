//! The evaluator and the folder must be two implementations — 022 contract 7d.
//!
//! Covers: 022 contract 7d.
//!
//! §2, on the zero cases of division: "Getting these wrong is uniquely dangerous in this
//! architecture: the independent evaluator (§3) and the constant folder **would share the
//! error**, so the evaluator would happily *validate* a model built on wrong semantics.
//! The mistake would then be invisible to model validation and would surface only as a
//! tier-1/tier-2 disagreement — which requires z3 to be installed."
//!
//! §3's first hard rule is that every tier-1 `Sat` is returned "only with a model that has
//! been concretely evaluated against every asserted constraint **by an independent
//! evaluator**". Independent is the load-bearing word: an evaluator that calls the folder
//! is a spell-checker that consults the same misspelling.
//!
//! This project has already been bitten by exactly that. `bvsdiv -5 0` is `1`, not
//! all-ones; the spec said all-ones, the folder implemented all-ones, and the evaluator
//! agreed with it — the error was found by asking z3, which is the one oracle the shared
//! code could not fool.

use chiero_solver::*;

/// **022 contract 7d, the mechanical half.** "The independent evaluator shares no symbols
/// with the constant folder — checked mechanically."
///
/// Checked by reading the source, which is crude and is the point: the property is about
/// *what the code calls*, and no runtime test can see a shared call. If this file moves,
/// the test fails loudly rather than silently passing.
#[test]
fn the_evaluator_does_not_call_the_folder() {
    let src = include_str!("../src/lib.rs");
    let start = src
        .find("fn eval_node(")
        .expect("the evaluator is in this file; if it moved, point this test at it");
    // To the next top-level `fn` at the same indentation — enough to cover the body.
    let rest = &src[start..];
    let end = rest[1..]
        .find("\n    fn ")
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    let body = &rest[..end];
    assert!(
        !body.contains("fold("),
        "the independent evaluator calls the constant folder, so a wrong rule in one is \
         confirmed by the other:\n{body}"
    );
}

/// **022 contract 7d, the differential half.** Two implementations of one set of
/// semantics, compared over the values that have historically differed.
///
/// A disagreement here is a bug in exactly one of them, and that is the *point* of having
/// two: the folder is what builds terms, the evaluator is what validates models, and a
/// model validated by the same rule that produced it validates nothing.
#[test]
fn the_folder_and_the_evaluator_agree_on_every_binary_operation() {
    // Deterministic pseudo-random operands, weighted toward the cases that bite:
    // zero, one, all-ones, the signed extremes, and the shift boundary.
    let interesting: [u128; 10] = [0, 1, 2, 5, 7, 0x7f, 0x80, 0xff, 0xfe, 0x40];
    let mut disagreements = Vec::new();
    for w in [8u32, 32] {
        let mask = if w == 128 {
            u128::MAX
        } else {
            (1u128 << w) - 1
        };
        for x in interesting {
            for y in interesting {
                let (x, y) = (x & mask, y & mask);
                let mut a = TermArena::new();
                let xt = a.bv(w, x);
                let yt = a.bv(w, y);
                // Every binary operation the arena builds. `bv` folds on construction,
                // so `as_const` is the *folder's* answer; `eval_ground` is the
                // evaluator's.
                let terms = [
                    ("add", a.add(xt, yt)),
                    ("sub", a.sub(xt, yt)),
                    ("mul", a.mul(xt, yt)),
                    ("udiv", a.udiv(xt, yt)),
                    ("sdiv", a.sdiv(xt, yt)),
                    ("urem", a.urem(xt, yt)),
                    ("srem", a.srem(xt, yt)),
                    ("and", a.and(xt, yt)),
                    ("or", a.or(xt, yt)),
                    ("xor", a.xor(xt, yt)),
                    ("shl", a.shl(xt, yt)),
                    ("lshr", a.lshr(xt, yt)),
                    ("ashr", a.ashr(xt, yt)),
                    ("ult", a.ult(xt, yt)),
                    ("slt", a.slt(xt, yt)),
                    ("eq", a.eq(xt, yt)),
                ];
                for (name, t) in terms {
                    let folded = a.as_const(t).map(|c| c.bits());
                    let evaluated = a.eval_ground(t).ok().map(|c| c.bits());
                    if folded != evaluated {
                        disagreements.push(format!(
                            "{name} at {w} bits on {x:#x}, {y:#x}: folder {folded:?}, \
                             evaluator {evaluated:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        disagreements.is_empty(),
        "{} disagreement(s):\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// And both agree with **z3**, when one is installed. This is the oracle that caught the
/// division rules in the first place, and it is the only one that does not share a line of
/// code with either implementation.
#[test]
fn both_implementations_agree_with_the_backend() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let cases: [(u128, u128); 6] = [(5, 0), (0xfb, 0), (0, 0), (7, 3), (0x80, 0xff), (1, 8)];
    for (x, y) in cases {
        let mut a = TermArena::new();
        let xt = a.bv(8, x);
        let yt = a.bv(8, y);
        for (name, t) in [
            ("udiv", a.udiv(xt, yt)),
            ("sdiv", a.sdiv(xt, yt)),
            ("urem", a.urem(xt, yt)),
            ("srem", a.srem(xt, yt)),
            ("shl", a.shl(xt, yt)),
            ("ashr", a.ashr(xt, yt)),
        ] {
            let want = a.eval_ground(t).expect("total").bits();
            // Ask the backend whether the term can differ from what both of ours say.
            // A variable pinned to the same operands keeps the query from being folded
            // away before it is asked.
            let mut b = TermArena::new();
            let vx = b.var(Sort::BitVec(8), "x");
            let vy = b.var(Sort::BitVec(8), "y");
            let cx = b.bv(8, x);
            let cy = b.bv(8, y);
            let px = b.eq(vx, cx);
            let py = b.eq(vy, cy);
            let expr = match name {
                "udiv" => b.udiv(vx, vy),
                "sdiv" => b.sdiv(vx, vy),
                "urem" => b.urem(vx, vy),
                "srem" => b.srem(vx, vy),
                "shl" => b.shl(vx, vy),
                _ => b.ashr(vx, vy),
            };
            let wc = b.bv(8, want);
            let same = b.eq(expr, wc);
            let differs = b.not(same);
            let mut s = TieredSolver::with_backend(backend.clone());
            assert!(
                matches!(s.check(&mut b, &[px, py, differs]), CheckResult::Unsat),
                "z3 disagrees about {name} {x:#x} {y:#x}: chiero says {want:#x}"
            );
        }
    }
}
