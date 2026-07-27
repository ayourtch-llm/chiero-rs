//! The term language, hash-consing, and the independent evaluator.
//!
//! Covers **022 contracts 19a–19d** (division and remainder by zero) and the evaluator
//! that contracts 3, 7 and 7b rest on.
//!
//! The evaluator is the project's soundness hinge. 022 §3 permits `solver-lite` to be
//! incomplete but never wrong, and the mechanism is that **every `Sat` carries a model
//! that has been concretely evaluated against every assertion**. If the evaluator is
//! wrong, it certifies wrong models — so its arithmetic is pinned here against the
//! SMT-LIB standard, verified against z3 4.8.12.

use chiero_solver::{BvConst, Model, Sort, TermArena};

/// 022 §1: division and remainder by zero are **total** and follow SMT-LIB, whose zero
/// cases are *not* uniform. Verified against z3 4.8.12 on `BitVec(8)`.
///
/// Getting these wrong is uniquely dangerous here: the evaluator and the constant
/// folder would share the error, so the evaluator would happily *validate* a model
/// built on wrong semantics — invisible to model validation, and detectable only by
/// asking z3, which may not be installed.
#[test]
fn division_by_zero_follows_smtlib() {
    let mut a = TermArena::new();
    let five = a.bv(8, 5);
    let neg5 = a.bv(8, 0xfb);
    let zero = a.bv(8, 0);

    // 19a: bvudiv x 0 = all ones.
    let t = a.udiv(five, zero);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xff);

    // 19b: bvsdiv x 0 = -1 when x >= 0.
    let t = a.sdiv(five, zero);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xff);

    // 19c: bvsdiv x 0 = 1 when x < 0. This is the case a uniform "all ones" rule gets
    // wrong, and it is why the table in 022 §1 exists.
    let t = a.sdiv(neg5, zero);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0x01);

    // 19d: bvurem / bvsrem x 0 = x, the dividend — not all ones.
    let t = a.urem(five, zero);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0x05);
    let t = a.srem(neg5, zero);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xfb);
    let t = a.srem(five, zero);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0x05);
}

/// Arithmetic is modular, which is what makes the wrap-safety rule in 022 §3.2
/// necessary in the first place.
#[test]
fn arithmetic_wraps() {
    let mut a = TermArena::new();
    let x = a.bv(8, 0xfb);
    let ten = a.bv(8, 10);
    let t = a.add(x, ten);
    assert_eq!(
        a.eval_ground(t).unwrap().bits(),
        0x05,
        "0xfb + 10 wraps to 5"
    );

    let sixteen = a.bv(8, 16);
    let t = a.mul(sixteen, sixteen);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0x00);
}

/// Signed and unsigned comparison differ on the same bits, and conflating them is a
/// classic source of wrong path conditions.
#[test]
fn signed_and_unsigned_comparison_differ() {
    let mut a = TermArena::new();
    let neg1 = a.bv(8, 0xff);
    let one = a.bv(8, 1);

    let t = a.ult(one, neg1);
    assert!(a.eval_ground_bool(t).unwrap(), "1 <u 255");
    let t = a.slt(neg1, one);
    assert!(a.eval_ground_bool(t).unwrap(), "-1 <s 1");
    let t = a.slt(one, neg1);
    assert!(!a.eval_ground_bool(t).unwrap());
}

/// Shifts by ≥ width yield 0 (SMT-LIB), which differs from x86's masking — the
/// divergence 070 §1.1 routes around in the differential oracle.
#[test]
fn overwide_shifts_yield_zero() {
    let mut a = TermArena::new();
    let x = a.bv(8, 0xff);
    let eight = a.bv(8, 8);
    let big = a.bv(8, 200);
    let t = a.shl(x, eight);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0);
    let t = a.lshr(x, big);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0);
    // Arithmetic shift of a negative value saturates to all ones, not zero.
    let t = a.ashr(x, big);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xff);
}

/// Sign- and zero-extension, and truncation, at the boundary.
#[test]
fn extension_and_truncation() {
    let mut a = TermArena::new();
    let x = a.bv(8, 0xff);
    let t = a.sext(x, 32);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xffff_ffff);
    let t = a.zext(x, 32);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xff);
    let big32 = a.bv(32, 0xdead_beef);
    let t = a.extract(big32, 15, 8);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xbe);
}

/// Hash-consing: structurally identical terms are the same `Term`, which is what makes
/// the caches in 022 §6 structural rather than textual.
#[test]
fn terms_are_hash_consed() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let one = a.bv(32, 1);
    let t1 = a.add(x, one);
    let t2 = a.add(x, one);
    assert_eq!(t1, t2, "identical terms must be one term");

    let t3 = a.add(one, x);
    assert_eq!(
        t1, t3,
        "commutative operands are normalized, so x+1 and 1+x are one term (022 §3)"
    );
}

/// Constant folding happens at construction, so it is an invariant rather than a pass.
#[test]
fn constants_fold_on_construction() {
    let mut a = TermArena::new();
    let two = a.bv(32, 2);
    let three = a.bv(32, 3);
    let t = a.add(two, three);
    assert_eq!(a.as_const(t).map(|c| c.bits()), Some(5));

    let x = a.var(Sort::BitVec(32), "x");
    // Identity and annihilator laws.
    let z = a.bv(32, 0);
    let one32 = a.bv(32, 1);
    assert_eq!(a.add(x, z), x, "x + 0 is x");
    assert_eq!(a.mul(x, one32), x, "x * 1 is x");
    assert_eq!(a.mul(x, z), z, "x * 0 is 0");
    assert_eq!(a.and(x, z), z);
}

/// The evaluator must be **total** over a complete model: every declared variable has a
/// value, so evaluation cannot fail for want of an assignment. That totality is what
/// lets a validated model establish `Sat` (022 §3.1).
#[test]
fn evaluation_is_total_over_a_complete_model() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let xy = a.mul(x, y);
    let three8 = a.bv(8, 3);
    let t = a.add(xy, three8);

    let mut m = Model::new();
    m.set(a.var_id(x).unwrap(), BvConst::new(8, 7));
    m.set(a.var_id(y).unwrap(), BvConst::new(8, 6));
    assert_eq!(a.eval(&m, t).unwrap().bits(), 45, "7*6+3");
}

/// A model missing a variable is an error, not a silent default. A default would let a
/// wrong model validate.
#[test]
fn an_incomplete_model_is_an_error() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let one8 = a.bv(8, 1);
    let t = a.add(x, one8);
    assert!(a.eval(&Model::new(), t).is_err());
}

/// Widths are checked at construction: a malformed term cannot be built, so the
/// evaluator never has to guess.
#[test]
fn mismatched_widths_are_rejected() {
    let mut a = TermArena::new();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut a2 = TermArena::new();
        let x = a2.bv(8, 1);
        let y = a2.bv(32, 1);
        a2.add(x, y)
    }));
    assert!(r.is_err(), "adding an i8 to an i32 must not build");
    let _ = a.bv(8, 1);
}
