//! `solver-lite` — the built-in, deliberately incomplete solver (022 §3).
//!
//! Covers **022 contracts 3, 4, 5, 7, 7b, 7c**.
//!
//! The whole design rests on an asymmetry that must be enforced, not hoped for:
//!
//! - **`Sat` is self-certifying.** A total assignment that evaluates every assertion to
//!   true is a satisfying assignment no matter how it was found, so an incomplete or
//!   even buggy search cannot produce a wrong `Sat` — provided the model is checked.
//! - **`Unsat` carries no witness.** Nothing checks it, so it must be constrained
//!   *syntactically*, in advance, to a fragment where the reasoning is known sound.
//!
//! Contract 7b is what actually holds the second half down, and unlike the z3
//! differential campaign it needs no external solver.

use chiero_solver::{CheckResult, Solver, SolverLite, Sort, TermArena};

/// 022 contract 3: a satisfiable conjunction is `Sat`, and the model must satisfy it
/// under independent evaluation.
#[test]
fn a_satisfiable_conjunction_is_sat_with_a_valid_model() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let five = a.bv(32, 5);
    let two = a.bv(32, 2);
    let lt = a.ult(x, five);
    let gt = a.ult(two, x);

    let mut s = SolverLite::new();
    s.assert(lt);
    s.assert(gt);
    match s.check(&mut a, &[]) {
        CheckResult::Sat(m) => {
            // The model is validated by the solver, but assert it here too: this is the
            // property the whole architecture rests on.
            assert!(a.eval(&m, lt).unwrap().bits() != 0);
            assert!(a.eval(&m, gt).unwrap().bits() != 0);
            let v = m.get(a.var_id(x).unwrap()).unwrap().bits();
            assert!((3..5).contains(&v), "x = {v}");
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// 022 contract 4: `x <u 5 ∧ x >u 5` is `Unsat` from tier 1 alone, with no subprocess.
#[test]
fn an_empty_interval_is_unsat() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let five = a.bv(32, 5);
    let lt = a.ult(x, five);
    let gt = a.ult(five, x);

    let mut s = SolverLite::new();
    s.assert(lt);
    s.assert(gt);
    assert!(matches!(s.check(&mut a, &[]), CheckResult::Unsat));
}

/// 022 contract 5: `x & 0xF0 == 0x0F` is `Unsat` from the known-bits domain.
#[test]
fn contradictory_known_bits_are_unsat() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let mask = a.bv(32, 0xF0);
    let masked = a.and(x, mask);
    let target = a.bv(32, 0x0F);
    let e = a.eq(masked, target);

    let mut s = SolverLite::new();
    s.assert(e);
    assert!(matches!(s.check(&mut a, &[]), CheckResult::Unsat));
}

/// **022 §3.2's wrap-safety rule.** Verified against z3: this is satisfiable at
/// `x = 0xfb, y = 0x05`. A saturating interval transfer computes `x ∈ [251,255]`,
/// `x+10 ∈ [261,265]`, saturates to `[255,255]`, intersects with `[0,9]`, finds empty,
/// and reports a **false `Unsat`** — pruning a real path and licensing a "no bug" claim.
#[test]
fn modular_wraparound_is_not_a_false_unsat() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let c250 = a.bv(8, 250);
    let c10 = a.bv(8, 10);

    let gt = a.ult(c250, x);
    let sum = a.add(x, c10);
    let eq = a.eq(y, sum);
    let lt = a.ult(y, c10);

    let mut s = SolverLite::new();
    s.assert(gt);
    s.assert(eq);
    s.assert(lt);
    assert!(
        !matches!(s.check(&mut a, &[]), CheckResult::Unsat),
        "this is satisfiable at x=0xfb; a saturating transfer would say Unsat"
    );
}

/// 022 contract 7c: anything outside the §3.2 fragment yields `Unknown`, never `Unsat`.
/// A propagator that descends into a disjunction and applies both sides reports a false
/// `Unsat` for `(x <u 5 ∨ x >u 200) ∧ x <u 3`, which is satisfiable.
#[test]
fn a_disjunction_yields_unknown_not_unsat() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let c5 = a.bv(8, 5);
    let c200 = a.bv(8, 200);
    let c3 = a.bv(8, 3);
    let lo = a.ult(x, c5);
    let hi = a.ult(c200, x);
    let disj = a.or(lo, hi);
    let small = a.ult(x, c3);

    let mut s = SolverLite::new();
    s.assert(disj);
    s.assert(small);
    match s.check(&mut a, &[]) {
        CheckResult::Unknown(_) => {}
        other => panic!("outside the fragment must be Unknown, got {other:?}"),
    }
}

/// 022 contract 6: multiplication is outside what tier 1 decides, so it escalates
/// rather than guessing.
#[test]
fn nonlinear_arithmetic_is_unknown() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let y = a.var(Sort::BitVec(32), "y");
    let p = a.mul(x, y);
    let seven = a.bv(32, 7);
    let one = a.bv(32, 1);
    let e = a.eq(p, seven);
    let gx = a.ult(one, x);
    let gy = a.ult(one, y);

    let mut s = SolverLite::new();
    s.assert(e);
    s.assert(gx);
    s.assert(gy);
    assert!(matches!(s.check(&mut a, &[]), CheckResult::Unknown(_)));
}

/// 022 contract 7: **tier 1 never returns `Sat` with a model that fails independent
/// evaluation.** Property test over random constraint sets.
#[test]
fn sat_always_carries_a_validated_model() {
    let mut seed = 0x243f_6a88_85a3_08d3u64;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for _ in 0..2000 {
        let mut a = TermArena::new();
        let x = a.var(Sort::BitVec(8), "x");
        let y = a.var(Sort::BitVec(8), "y");
        let mut s = SolverLite::new();
        let mut asserted = Vec::new();
        for _ in 0..(rng() % 4 + 1) {
            let v = if rng() % 2 == 0 { x } else { y };
            let k = a.bv(8, (rng() % 256) as u128);
            let t = match rng() % 4 {
                0 => a.ult(v, k),
                1 => a.ult(k, v),
                2 => a.eq(v, k),
                _ => {
                    let m = a.bv(8, (rng() % 256) as u128);
                    let masked = a.and(v, m);
                    a.eq(masked, k)
                }
            };
            s.assert(t);
            asserted.push(t);
        }
        if let CheckResult::Sat(model) = s.check(&mut a, &[]) {
            for t in &asserted {
                let v = a
                    .eval(&model, *t)
                    .expect("the model must be total over the asserted terms");
                assert!(v.bits() != 0, "Sat returned a model that fails evaluation");
            }
        }
    }
}

/// **022 contract 7b — the contract that closes the Sat/Unsat asymmetry.**
///
/// For random constraint sets over a small width, whenever tier 1 answers `Unsat`,
/// brute-force enumeration of every assignment confirms none satisfies. Unlike the z3
/// differential campaign this needs no external solver, so it runs everywhere.
#[test]
fn every_unsat_is_confirmed_by_exhaustive_enumeration() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    const W: u32 = 4;
    const N: u128 = 1 << W;

    let mut unsat_seen = 0;
    for _ in 0..3000 {
        let mut a = TermArena::new();
        let x = a.var(Sort::BitVec(W), "x");
        let y = a.var(Sort::BitVec(W), "y");
        let mut s = SolverLite::new();
        let mut asserted = Vec::new();
        for _ in 0..(rng() % 4 + 1) {
            let v = if rng() % 2 == 0 { x } else { y };
            let k = a.bv(W, (rng() % N as u64) as u128);
            let t = match rng() % 5 {
                0 => a.ult(v, k),
                1 => a.ult(k, v),
                2 => a.eq(v, k),
                3 => {
                    let m = a.bv(W, (rng() % N as u64) as u128);
                    let masked = a.and(v, m);
                    a.eq(masked, k)
                }
                _ => {
                    let d = a.bv(W, (rng() % N as u64) as u128);
                    let sum = a.add(v, d);
                    a.ult(sum, k)
                }
            };
            s.assert(t);
            asserted.push(t);
        }

        if matches!(s.check(&mut a, &[]), CheckResult::Unsat) {
            unsat_seen += 1;
            let (xv, yv) = (a.var_id(x).unwrap(), a.var_id(y).unwrap());
            for i in 0..N {
                for j in 0..N {
                    let mut m = chiero_solver::Model::new();
                    m.set(xv, chiero_solver::BvConst::new(W, i));
                    m.set(yv, chiero_solver::BvConst::new(W, j));
                    let all = asserted.iter().all(|t| a.eval(&m, *t).unwrap().bits() != 0);
                    assert!(
                        !all,
                        "claimed Unsat but x={i}, y={j} satisfies every assertion"
                    );
                }
            }
        }
    }
    assert!(
        unsat_seen > 50,
        "only {unsat_seen} Unsat answers — the corpus is not exercising the claim"
    );
}

/// `push`/`pop` restore the exact assertion set (022 contract 16).
#[test]
fn push_and_pop_restore_the_assertion_set() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let c5 = a.bv(8, 5);
    let c3 = a.bv(8, 3);
    let lt5 = a.ult(x, c5);
    let gt3 = a.ult(c3, x);

    let mut s = SolverLite::new();
    s.assert(lt5);
    s.push();
    s.assert(gt3);
    assert!(matches!(s.check(&mut a, &[]), CheckResult::Sat(_)));
    s.pop(1);

    // With only `x < 5` asserted, `x < 3` is still satisfiable.
    let lt3 = a.ult(x, c3);
    s.assert(lt3);
    assert!(matches!(s.check(&mut a, &[]), CheckResult::Sat(_)));
}

/// **The negation of an atom is an atom**, and until wave 153 it was not.
///
/// This is not a fine point of the fragment's edge — it is *half of every branch*. A
/// conditional contributes its condition to one path and the **negation** of that condition
/// to the other, so a solver that decides `x >s 10` and gives up on `!(x >s 10)` can answer
/// for exactly one side of every `if` in every program. `SolverLite::check` collected atoms
/// with `as_atom`, which matches only a bare `Node::Bin(pred, ..)`, so a `Node::Not` wrapper
/// fell straight through to "assertion is outside the conjunction-of-atoms fragment".
///
/// Nothing noticed for the same reason 023 §9 keeps giving: every channel that runs programs
/// runs *closed* ones. A program with no inputs has no symbolic branch, so no path condition
/// ever contained a negated comparison. Wave 153's symbolic oracle put one there on its
/// first run and three fixtures failed at once.
///
/// The three predicates are checked separately because each negates to a different relation
/// — `!(a <u b)` is `a >=u b`, `!(a <s b)` is `a >=s b`, `!(a == b)` is `a != b` — and none
/// of the three is expressible as one of the other two.
#[test]
fn the_negation_of_an_atom_is_still_in_the_fragment() {
    for (name, build) in [
        (
            "!(x <u 10), satisfied by x = 10",
            (|a: &mut TermArena, x| {
                let ten = a.bv(32, 10);
                let lt = a.ult(x, ten);
                a.not(lt)
            }) as fn(&mut TermArena, chiero_solver::Term) -> chiero_solver::Term,
        ),
        ("!(x <s 10), satisfied by x = 10", |a: &mut TermArena, x| {
            let ten = a.bv(32, 10);
            let lt = a.slt(x, ten);
            a.not(lt)
        }),
        (
            "!(x == 0), satisfied by any other x",
            |a: &mut TermArena, x| {
                let zero = a.bv(32, 0);
                let eq = a.eq(x, zero);
                a.not(eq)
            },
        ),
    ] {
        let mut a = TermArena::new();
        let x = a.var(Sort::BitVec(32), "x");
        let t = build(&mut a, x);
        let mut s = SolverLite::default();
        match s.check(&mut a, &[t]) {
            CheckResult::Sat(m) => {
                // `Sat` is self-certifying only if the model is checked (022 §3.1), and a
                // negated atom is exactly where a propagator is most likely to invert a
                // bound. Evaluate independently rather than trusting the verdict.
                assert_eq!(
                    a.eval(&m, t).map(|v| v.bits() != 0),
                    Ok(true),
                    "{name}: the model does not satisfy the assertion it was produced for"
                );
            }
            other => panic!(
                "{name}: {other:?}. A negated comparison is one side of every branch on an \
                 input, so this is not an edge of the fragment — it is half of it."
            ),
        }
    }
}

/// **A negated atom still refutes**, which is the half `Sat` cannot certify.
///
/// 022's asymmetry: a wrong `Sat` is impossible because the model is validated, but nothing
/// validates `Unsat`, so the fragment has to be sound by construction. Admitting negated
/// atoms widens that fragment, and this is the check that the widening did not also make it
/// *wrong* — `x <u 10 && !(x <u 10)` has no model and must come back `Unsat`, not `Sat`.
#[test]
fn a_negated_atom_contradicting_its_positive_is_unsat() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let ten = a.bv(32, 10);
    let lt = a.ult(x, ten);
    let not_lt = a.not(lt);
    let mut s = SolverLite::default();
    assert!(
        matches!(s.check(&mut a, &[lt, not_lt]), CheckResult::Unsat),
        "a term and its negation cannot both hold; answering `Sat` here would be the one \
         failure 022 §3.1 says the design must make impossible"
    );
}

/// **The shape a lowered C comparison actually has**, and the polarity through it.
///
/// The bare-atom tests above cannot pin the polarity of the `ite` peel: both `p` and `!p`
/// are satisfiable, so reading one as the other still produces *a* model, and the model
/// validator accepts it. What distinguishes them is **which side of the branch the model
/// lands on** — so this asserts the value, not just the verdict.
///
/// The term is the one the engine builds for `if (x > 10)`:
///
/// ```text
///   (not (= ((_ zero_extend 31) (ite (bvslt (_ bv10 32) x) #b1 #b0)) (_ bv0 32)))
/// ```
///
/// Three polarity flips compose in it — `not`, `= 0`, and the `ite` — and getting any one
/// of them backwards yields the *other* branch. A mutation swapping the `ite` arms survived
/// the whole symbolic differential channel; this is what kills it.
#[test]
fn a_materialized_comparison_puts_the_model_on_the_right_side() {
    // The true side: x must come out greater than 10.
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let ten = a.bv(32, 10);
    let gt = a.slt(ten, x);
    let one = a.bv(1, 1);
    let zero1 = a.bv(1, 0);
    let materialized = a.ite(gt, one, zero1);
    let widened = a.zext(materialized, 32);
    let zero32 = a.bv(32, 0);
    let is_zero = a.eq(widened, zero32);
    let taken = a.not(is_zero);

    let mut s = SolverLite::default();
    match s.check(&mut a, &[taken]) {
        CheckResult::Sat(m) => {
            let v = a.eval(&m, x).expect("the model binds x").signed();
            assert!(
                v > 10,
                "the model puts x at {v}, which is the *other* branch: the polarity of one \
                 of the three flips in a materialized comparison is inverted"
            );
        }
        other => panic!("the shape every lowered `if` produces came back {other:?}"),
    }

    // The false side: the same term negated, and x must come out at most 10.
    let not_taken = a.not(taken);
    let mut s = SolverLite::default();
    match s.check(&mut a, &[not_taken]) {
        CheckResult::Sat(m) => {
            let v = a.eval(&m, x).expect("the model binds x").signed();
            assert!(
                v <= 10,
                "the model puts x at {v}, which is the *other* branch"
            );
        }
        other => panic!("the false side of every lowered `if` came back {other:?}"),
    }
}

/// **A signed bound below zero is not an unsigned interval**, and pretending otherwise
/// produces a wrong `Unsat`.
///
/// `v >=s -1` holds for -1 and for every non-negative value — as unsigned bit patterns,
/// `0xFFFFFFFF` and `[0, 0x7FFFFFFF]`. That is two ranges, so no single interval implies
/// it, and the narrowing must decline. Reading `-1` as the unsigned `4294967295` and
/// setting `lo` to it instead excludes every value the constraint actually permits, and the
/// domain then goes empty against any upper bound — reported as `Unsat`, which is the one
/// verdict 022 §3.1 says nothing downstream validates.
#[test]
fn a_negative_signed_bound_does_not_narrow_unsoundly() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let minus_one = a.bv(32, u128::from(u32::MAX));
    // !(x <s -1), i.e. x >=s -1 — satisfied by x = 0 among many others.
    let lt = a.slt(x, minus_one);
    let ge = a.not(lt);
    let five = a.bv(32, 5);
    let small = a.ult(x, five);
    let mut s = SolverLite::default();
    match s.check(&mut a, &[ge, small]) {
        CheckResult::Sat(m) => {
            assert_eq!(
                a.eval(&m, ge).map(|v| v.bits() != 0),
                Ok(true),
                "the model does not satisfy the signed bound it was produced for"
            );
        }
        CheckResult::Unsat => panic!(
            "`x >=s -1 && x <u 5` is satisfied by x = 0; answering Unsat means a negative \
             signed bound was narrowed as though it were unsigned"
        ),
        // Declining is allowed — incompleteness is the sanctioned failure here.
        CheckResult::Unknown(_) => {}
    }
}

/// **`and` is bitwise, and only at one bit is it a conjunction.**
///
/// A `switch` default arrives as one term joining its negated cases with `and`, and
/// splitting that into separate assertions is what brings it into the fragment. The split
/// is sound only at width 1. For a wider term, `x & y != 0` says *some* bit is set in both
/// — not that each operand is nonzero in the sense the atom collector would then assert.
/// Splitting regardless asserts something strictly stronger, and a stronger set can reach
/// an empty domain, which is reported as `Unsat`.
#[test]
fn a_wide_bitwise_and_is_not_split_into_conjuncts() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    // (x & 1) is nonzero exactly when x is odd; asserting it alongside `x <u 4` is
    // satisfied by 1 and 3. Split as conjuncts, `x` and `1` would each be asserted
    // separately and the constant conjunct is not an atom at all.
    let one = a.bv(32, 1);
    let masked = a.and(x, one);
    let four = a.bv(32, 4);
    let small = a.ult(x, four);
    let mut s = SolverLite::default();
    match s.check(&mut a, &[masked, small]) {
        CheckResult::Sat(m) => {
            assert_eq!(
                a.eval(&m, masked).map(|v| v.bits() != 0),
                Ok(true),
                "the model does not satisfy `x & 1`"
            );
        }
        CheckResult::Unsat => panic!(
            "`x & 1` with `x <u 4` is satisfied by x = 1; answering Unsat means a wide \
             bitwise `and` was split as though it were a conjunction"
        ),
        CheckResult::Unknown(_) => {}
    }
}

/// **The sign of the bound decides which polarity is an interval**, and wave 153 taught
/// only the half that holds for a non-negative bound.
///
/// The domain is an unsigned range. A signed constraint maps onto one exactly when it
/// confines `v` to a contiguous run of bit patterns:
///
/// ```text
///   k >= 0:  v >=s k  is [k, 2^(w-1)-1]         one interval
///            v <s  k  is negatives + [0, k-1]   two
///   k <  0:  v <s  k  is [2^(w-1), k_u - 1]     one interval
///            v >=s k  is [0, 2^(w-1)-1] + [k_u, …]  two
/// ```
///
/// So each polarity is expressible for exactly one sign of `k`, and wave 153 implemented one
/// row of that table. `x <s 0` — the negative test every C program contains — fell in the
/// unimplemented half, and its `Unknown` was invisible because the *other* side of the
/// branch answered fine.
#[test]
fn a_negative_signed_bound_is_an_interval_in_the_other_polarity() {
    for (name, k, negated, want) in [
        ("x <s 0, satisfied only by a negative", 0i64, false, true),
        ("x <s -5", -5, false, true),
        ("!(x <s -5), i.e. x >=s -5", -5, true, false),
    ] {
        let mut a = TermArena::new();
        let x = a.var(Sort::BitVec(32), "x");
        let bound = a.bv(32, u128::from(k as u32));
        let lt = a.slt(x, bound);
        let t = if negated { a.not(lt) } else { lt };
        let mut s = SolverLite::default();
        match s.check(&mut a, &[t]) {
            CheckResult::Sat(m) => {
                assert_eq!(
                    a.eval(&m, t).map(|v| v.bits() != 0),
                    Ok(true),
                    "{name}: the model does not satisfy the assertion it was produced for"
                );
                let v = a.eval(&m, x).expect("binds x").signed() as i32;
                if want {
                    assert!(v < k as i32, "{name}: the model puts x at {v}");
                }
            }
            // Declining is sound. It is only *this* test's subject when the constraint is
            // one the table above says is an interval.
            other if want => panic!(
                "{name}: {other:?}. A negative bound makes this polarity a single unsigned \
                 interval, so declining it is incompleteness with nothing to justify it."
            ),
            _ => {}
        }
    }
}

/// **A single-bit mask has only one bit to differ in.**
///
/// `v & m == k` is a known-bits fact and pins the selected bits. The negation was declined
/// wholesale, on the correct observation that "one of the masked bits differs" does not say
/// which — correct for a multi-bit mask, and vacuous when the mask selects exactly one bit,
/// where there is nothing else it could be. `if (x & FLAG)` is why that case is worth having.
#[test]
fn a_negated_single_bit_mask_pins_its_bit() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let one = a.bv(32, 1);
    let zero = a.bv(32, 0);
    let masked = a.and(x, one);
    let is_zero = a.eq(masked, zero);
    let odd = a.not(is_zero);
    let mut s = SolverLite::default();
    match s.check(&mut a, &[odd]) {
        CheckResult::Sat(m) => {
            let v = a.eval(&m, x).expect("binds x").bits();
            assert_eq!(v & 1, 1, "the model gives x = {v}, which is even");
        }
        other => panic!(
            "`x & 1 != 0` came back {other:?}. With one bit selected there is exactly one \
             way for the masked value to be nonzero, so this is not the multi-bit case the \
             restriction was written for."
        ),
    }
    // **The multi-bit case must still decline.** `x & 6 != 0` says bit 1 or bit 2 is set and
    // pins neither; pinning either would be unsound, and this is the assertion that says the
    // widening above did not overreach.
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let six = a.bv(32, 6);
    let zero = a.bv(32, 0);
    let masked = a.and(x, six);
    let is_zero = a.eq(masked, zero);
    let some = a.not(is_zero);
    let mut s = SolverLite::default();
    if let CheckResult::Sat(m) = s.check(&mut a, &[some]) {
        assert_eq!(
            a.eval(&m, some).map(|v| v.bits() != 0),
            Ok(true),
            "a multi-bit mask was pinned to one of its bits, which the constraint does not say"
        );
    }
}

/// **A widened operand still constrains its source.**
///
/// `(long)x > 5` is `5 <s sext(x, 64)`, whose operand is not a variable, so nothing narrows.
/// Widening is exact and order-preserving for a bound that fits the narrow width, which
/// makes this a narrowing the domain can do and was never taught.
///
/// This is not a `long` curiosity: integer promotion (C11 6.3.1.1) widens every `char` and
/// `short` before comparing it, so the widened-operand shape is what most comparisons on
/// small types actually produce.
#[test]
fn a_comparison_through_a_widening_narrows_the_narrow_variable() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let wide = a.sext(x, 64);
    let five = a.bv(64, 5);
    let gt = a.slt(five, wide);
    let mut s = SolverLite::default();
    match s.check(&mut a, &[gt]) {
        CheckResult::Sat(m) => {
            let v = a.eval(&m, x).expect("binds x").signed() as i32;
            assert!(
                v > 5,
                "the model gives x = {v}, which is not greater than 5"
            );
        }
        other => panic!("`(long)x > 5` came back {other:?}"),
    }
}

/// **A multi-bit mask pinned is a wrong `Unsat`**, not merely a lost answer.
///
/// The single-bit widening above is guarded by `m.count_ones() == 1`, and dropping that
/// guard survives every test that only checks satisfiable inputs: pinning `x & 6 != 0` to
/// "both bits set" still yields a model that validates, because a validated `Sat` cannot be
/// wrong. The damage shows only where nothing validates — an over-strong domain going empty
/// and being reported `Unsat`.
///
/// `x & 6 != 0 && x == 2` is satisfied by 2 alone. Pin both bits and the domain contradicts
/// `x == 2`, and the answer becomes a refutation of a satisfiable set.
#[test]
fn a_multi_bit_mask_is_not_pinned_into_a_wrong_refutation() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let six = a.bv(32, 6);
    let zero = a.bv(32, 0);
    let masked = a.and(x, six);
    let is_zero = a.eq(masked, zero);
    let some = a.not(is_zero);
    let two = a.bv(32, 2);
    let is_two = a.eq(x, two);
    let mut s = SolverLite::default();
    assert!(
        !matches!(s.check(&mut a, &[some, is_two]), CheckResult::Unsat),
        "`x & 6 != 0 && x == 2` is satisfied by x = 2. Answering Unsat means a multi-bit \
         mask was pinned to all of its bits, which the constraint does not say."
    );
}

/// **The widening must match the comparison's signedness**, and dropping that check is also
/// a wrong `Unsat`.
///
/// `zext(x, 64) >=s 5` says the *unsigned* value of `x` is at least 5, which every `x` with
/// the top bit set satisfies. Reading it as `x >=s 5` confines `x` to the non-negative half
/// instead — an interval that excludes exactly those values. Combined with a constraint that
/// requires one, the domain empties and a satisfiable set is refuted.
///
/// A wrong narrowing cannot produce a wrong `Sat` — the model validator catches that — so a
/// test built from satisfiable inputs alone would not see this at all.
#[test]
fn an_unsigned_widening_is_not_narrowed_as_a_signed_one() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let wide = a.zext(x, 64);
    let five = a.bv(64, 5);
    let lt = a.slt(wide, five);
    let ge = a.not(lt);
    // x with its top bit set: unsigned-large, signed-negative.
    let half = a.bv(32, 1u128 << 31);
    let big = a.ult(half, x);
    let mut s = SolverLite::default();
    assert!(
        !matches!(s.check(&mut a, &[ge, big]), CheckResult::Unsat),
        "`zext(x) >=s 5 && x >u 2^31` holds for, say, x = 2^31 + 1. Answering Unsat means a \
         zero-extension was narrowed under a signed comparison."
    );
}

/// **`a <= b` is `!(b < a)`, and the operands must swap.**
///
/// Reading the non-strict form as `!(a < b)` is the same relation with the arguments the
/// wrong way round, which is `a > b` — the complement, not the negation. Both are
/// satisfiable, so a test that only asks "is there a model" cannot tell them apart; this
/// asserts which side of the bound the model lands on.
#[test]
fn the_non_strict_form_swaps_its_operands() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let minus_one = a.bv(32, u128::from(u32::MAX));
    // `x <= -1`, as lowering builds it: `x < -1 || x == -1`.
    let lt = a.slt(x, minus_one);
    let eq = a.eq(x, minus_one);
    let le = a.or(lt, eq);
    let mut s = SolverLite::default();
    match s.check(&mut a, &[le]) {
        CheckResult::Sat(m) => {
            let v = a.eval(&m, x).expect("binds x").signed() as i32;
            assert!(v <= -1, "the model gives x = {v}, which is not <= -1");
        }
        other => panic!("`x <= -1` came back {other:?}"),
    }
}

/// **A bound the source width cannot hold must not be truncated into one it can.**
///
/// `(long)x < 2^40` is true for every 32-bit `x` — the widening cannot reach that far. Fold
/// the bound into the source width and it becomes `x <s 0`, which is true for *half* of
/// them, and the half it excludes is excluded wrongly.
///
/// The third wrong-`Unsat` in this file, and the same lesson each time: a narrowing that is
/// too strong is invisible wherever a model gets validated, and fatal exactly where one
/// does not.
#[test]
fn a_bound_too_wide_for_the_source_is_not_truncated() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let wide = a.sext(x, 64);
    let huge = a.bv(64, 1u128 << 40);
    let lt = a.slt(wide, huge);
    let zero = a.bv(32, 0);
    let is_zero = a.eq(x, zero);
    let mut s = SolverLite::default();
    assert!(
        !matches!(s.check(&mut a, &[lt, is_zero]), CheckResult::Unsat),
        "`(long)x < 2^40 && x == 0` holds: 0 sign-extends to 0. Answering Unsat means the \
         bound was truncated to 32 bits, turning `always true` into `x is negative`."
    );
}

/// **One candidate is not a search.**
///
/// `Domains::candidate` builds a single model — each variable's *least* feasible value — and
/// `check` validates it. If it fails, the answer is `Unknown`, and that is the end of it. So
/// a set whose only models lie anywhere other than the bottom of the domain is undecidable
/// no matter how much narrowing succeeded.
///
/// The shape that surfaced it is a `switch` default arm. `switch (x & 3)` with cases 0, 1
/// and 2 gives the default the path condition
///
/// ```text
///   x & 3 != 0  &&  x & 3 != 1  &&  x & 3 != 2
/// ```
///
/// which is satisfied by `x = 3` and by nothing smaller. Each conjunct is a *multi-bit*
/// negated mask, which wave 154 established cannot be pinned soundly — so the domain stays
/// full, the candidate is 0, and 0 fails the first conjunct.
///
/// The fix is not more narrowing. Narrowing that could express "not 0, not 1, not 2" would
/// need a set, not an interval. What is missing is that a **failed candidate ends the
/// search**, when trying the next feasible value would have decided it in three more steps.
/// Validation is what makes this safe: every candidate is checked against the original
/// assertions, so a search that guesses cannot answer wrongly — only slowly.
#[test]
fn a_satisfiable_set_whose_model_is_not_the_least_value_is_still_decided() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let three = a.bv(32, 3);
    let masked = a.and(x, three);
    let mut asserts = Vec::new();
    for k in 0..3u128 {
        let c = a.bv(32, k);
        let e = a.eq(masked, c);
        asserts.push(a.not(e));
    }
    let mut s = SolverLite::default();
    match s.check(&mut a, &asserts) {
        CheckResult::Sat(m) => {
            for t in &asserts {
                assert_eq!(
                    a.eval(&m, *t).map(|v| v.bits() != 0),
                    Ok(true),
                    "the model does not satisfy every assertion it was produced for"
                );
            }
        }
        other => panic!(
            "`x & 3` is none of 0, 1, 2 — satisfied by x = 3, three steps above the \
             domain's least value. Answering {other:?} means the search stopped at the \
             first candidate."
        ),
    }
}

/// **A known-bits fact must not turn candidate selection into a linear scan.**
///
/// `least` walks `lo..=hi` looking for a value that matches the known bits. Wave 154 made
/// `x & FLAG != 0` *set* those bits for a single-bit mask — so a flag high in the word now
/// asks that walk to count to 2^30 before finding its first candidate. Nothing hit it,
/// because every fixture written so far masked a low bit.
///
/// The bound is what this test is about, not the answer: taking `Unknown` is always sound,
/// and a solver that takes minutes to say so is not.
#[test]
fn a_high_known_bit_does_not_make_candidate_selection_scan() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let flag = a.bv(32, 1 << 30);
    let zero = a.bv(32, 0);
    let masked = a.and(x, flag);
    let clear = a.eq(masked, zero);
    let set = a.not(clear);
    let start = std::time::Instant::now();
    let mut s = SolverLite::default();
    let r = s.check(&mut a, &[set]);
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "candidate selection took {elapsed:?} for a single high bit, which is a scan"
    );
    if let CheckResult::Sat(m) = r {
        assert_eq!(
            a.eval(&m, set).map(|v| v.bits() != 0),
            Ok(true),
            "the model does not have the bit the assertion requires"
        );
    }
}
