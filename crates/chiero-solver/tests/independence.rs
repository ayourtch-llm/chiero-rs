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
    // **Both ends of the call.** Checking only `eval_node` left the evaluator free to
    // *be* the folder: replacing `independent_bin`'s whole body with `fold(k, x, y)`
    // satisfied this check and every other test, because `eval_node` still called
    // `independent_bin` and the differential then compared `fold` with itself. Found by
    // review — the one scenario contract 7d exists for.
    for f in ["fn eval_node(", "fn independent_bin("] {
        let body = body_of(src, f);
        assert!(
            !body.contains("fold("),
            "{f} calls the constant folder, so a wrong rule in one is confirmed by the \
             other:\n{body}"
        );
    }
}

/// The brace-balanced body of a function, by signature prefix.
fn body_of<'a>(src: &'a str, sig: &str) -> &'a str {
    let start = src
        .find(sig)
        .unwrap_or_else(|| panic!("{sig} is in this file; if it moved, point this test at it"));
    // To the next top-level `fn` at the same indentation — enough to cover the body.
    // **Brace-balanced**, not "up to the next `fn`". The first version searched for a
    // following `    fn ` and found none — the next item is `pub fn` — so it took the rest
    // of the file, which of course contains the folder. A window that is wrong in the
    // permissive direction fails loudly; one wrong the other way passes silently, and this
    // one happened to be the first.
    let rest = &src[start..];
    let open = rest.find('{').expect("a body");
    let mut depth = 0i32;
    let mut end = rest.len();
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &rest[..end];
    assert!(
        body.len() < rest.len(),
        "the body was not delimited, so this would scan the whole file"
    );
    body
}

/// **022 contract 7d, the differential half.** Two implementations of one set of
/// semantics, compared over the values that have historically differed.
///
/// A disagreement here is a bug in exactly one of them, and that is the *point* of having
/// two: the folder is what builds terms, the evaluator is what validates models, and a
/// model validated by the same rule that produced it validates nothing.
///
/// ⚠️ The first version of this test compared `as_const` against `eval_ground` on a term
/// built from **constants** — which the arena folds on construction, so the evaluator was
/// never reached and the test compared the folder with itself. Three deliberate breakages
/// of the evaluator survived it. The operands must be *variables*, so that folding cannot
/// happen and evaluation has to.
#[test]
fn the_folder_and_the_evaluator_agree_on_every_binary_operation() {
    // Weighted toward the cases that bite: zero, one, all-ones, the signed extremes, and
    // the shift boundary.
    let interesting: [u128; 10] = [0, 1, 2, 5, 7, 0x7f, 0x80, 0xff, 0xfe, 0x40];
    let ops: [&str; 16] = [
        "add", "sub", "mul", "udiv", "sdiv", "urem", "srem", "and", "or", "xor", "shl", "lshr",
        "ashr", "ult", "slt", "eq",
    ];
    let build = |a: &mut TermArena, name: &str, x: Term, y: Term| -> Term {
        match name {
            "add" => a.add(x, y),
            "sub" => a.sub(x, y),
            "mul" => a.mul(x, y),
            "udiv" => a.udiv(x, y),
            "sdiv" => a.sdiv(x, y),
            "urem" => a.urem(x, y),
            "srem" => a.srem(x, y),
            "and" => a.and(x, y),
            "or" => a.or(x, y),
            "xor" => a.xor(x, y),
            "shl" => a.shl(x, y),
            "lshr" => a.lshr(x, y),
            "ashr" => a.ashr(x, y),
            "ult" => a.ult(x, y),
            "slt" => a.slt(x, y),
            _ => a.eq(x, y),
        }
    };
    let mut disagreements = Vec::new();
    // **128 is in the list because the shift guard is not equivalent there.** A comment
    // in the evaluator claimed `>=` versus `>` at the width boundary made no difference
    // "because the value is masked to `w` afterwards" — true at 8 and 32, false at 128,
    // where `u128::wrapping_shl(128)` masks the *count* to 0 and shifts nothing.
    // `MAX_BV_BITS` is 128 and 020 declares `__int128`, so this is reachable. Found by
    // review; two widths could not see it.
    for w in [8u32, 32, 128] {
        let mask = if w == 128 {
            u128::MAX
        } else {
            (1u128 << w) - 1
        };
        // **The width itself, and one either side.** A shift count is only interesting at
        // the boundary SMT-LIB defines — "at or past the width shifts every bit out" — and
        // none of the values above equals 8 or 32, so `>=` and `>` were indistinguishable.
        let mut operands: Vec<u128> = interesting.to_vec();
        operands.extend([u128::from(w) - 1, u128::from(w), u128::from(w) + 1]);
        for x in operands.iter().copied() {
            for y in operands.iter().copied() {
                let (x, y) = (x & mask, y & mask);
                for name in ops {
                    // The folder: constants in, folded on construction.
                    let mut fa = TermArena::new();
                    let (fx, fy) = (fa.bv(w, x), fa.bv(w, y));
                    let ft = build(&mut fa, name, fx, fy);
                    let folded = fa.as_const(ft).map(|c| c.bits());

                    // The evaluator: *variables* in, so nothing folds, and a model that
                    // assigns them the same values.
                    let mut ea = TermArena::new();
                    let vx = ea.var(Sort::BitVec(w), "x");
                    let vy = ea.var(Sort::BitVec(w), "y");
                    let et = build(&mut ea, name, vx, vy);
                    let mut m = Model::new();
                    m.set(ea.var_id(vx).unwrap(), BvConst::new(w, x));
                    m.set(ea.var_id(vy).unwrap(), BvConst::new(w, y));
                    let evaluated = ea.eval(&m, et).ok().map(|c| c.bits());

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
        // **Variables, so the evaluator actually runs.** Built from constants, the arena
        // folds at construction and `eval_ground` returns a `Const` without ever reaching
        // `Node::Bin` — so this compared z3 against the *folder* and passed with the
        // evaluator gutted to return zero. The same trap the sibling test had, in the
        // test written to avoid it. Found by review.
        let vx0 = a.var(Sort::BitVec(8), "ex");
        let vy0 = a.var(Sort::BitVec(8), "ey");
        let mut model = Model::new();
        model.set(a.var_id(vx0).unwrap(), BvConst::new(8, x));
        model.set(a.var_id(vy0).unwrap(), BvConst::new(8, y));
        let (xt, yt) = (vx0, vy0);
        for (name, t) in [
            ("udiv", a.udiv(xt, yt)),
            ("sdiv", a.sdiv(xt, yt)),
            ("urem", a.urem(xt, yt)),
            ("srem", a.srem(xt, yt)),
            ("shl", a.shl(xt, yt)),
            ("ashr", a.ashr(xt, yt)),
        ] {
            let want = a.eval(&model, t).expect("total").bits();
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
