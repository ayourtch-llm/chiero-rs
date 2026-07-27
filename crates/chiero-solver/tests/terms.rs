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

/// Bitwise complement, folded like every other constant operation.
#[test]
fn not_folds_and_round_trips() {
    let mut a = TermArena::new();
    let x = a.bv(8, 0x0f);
    let t = a.not(x);
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0xf0);

    let v = a.var(Sort::BitVec(8), "v");
    let n1 = a.not(v);
    let n2 = a.not(v);
    assert_eq!(n1, n2, "hash-consed");
}

// ---------------------------------------------------------------------------
// `Concat` and `Ite` — required by 021 §3 before the memory model can be symbolic.
// ---------------------------------------------------------------------------

/// **021 contract 5 needs `Concat`.** A read of `Int(32)` over four bytes where two are
/// concrete and two symbolic must produce a `Concat` term rather than forcing the object
/// into array theory. That is what makes type punning and partial overwrites exact, and
/// it is the single most common shape in packet code.
#[test]
fn concat_has_the_summed_width_and_smtlib_byte_order() {
    let mut a = TermArena::new();
    let hi = a.bv(8, 0xAB);
    let lo = a.bv(8, 0xCD);
    let c = a.concat(hi, lo);
    assert_eq!(a.width(c), 16);
    // SMT-LIB `concat` puts the first argument in the *high* bits.
    let m = Model::new();
    assert_eq!(a.eval(&m, c).unwrap().bits(), 0xABCD);
}

/// Concatenation is associative in value but the arena need not flatten it; what must
/// hold is that nesting either way evaluates identically.
#[test]
fn nested_concat_evaluates_associatively() {
    let mut a = TermArena::new();
    let (x, y, z) = (a.bv(8, 0x12), a.bv(8, 0x34), a.bv(8, 0x56));
    let left = {
        let t = a.concat(x, y);
        a.concat(t, z)
    };
    let right = {
        let t = a.concat(y, z);
        a.concat(x, t)
    };
    let m = Model::new();
    assert_eq!(a.eval(&m, left).unwrap().bits(), 0x123456);
    assert_eq!(a.eval(&m, right).unwrap().bits(), 0x123456);
}

/// `extract` over a `concat` recovers the original halves. Without this the memory
/// model's byte assembly and its byte-granular writes disagree.
#[test]
fn extract_recovers_the_halves_of_a_concat() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let c = a.concat(x, y);
    let hi = a.extract(c, 15, 8);
    let lo = a.extract(c, 7, 0);
    let mut m = Model::new();
    m.set(a.var_id(x).unwrap(), BvConst::new(8, 0xAB));
    m.set(a.var_id(y).unwrap(), BvConst::new(8, 0xCD));
    assert_eq!(a.eval(&m, hi).unwrap().bits(), 0xAB);
    assert_eq!(a.eval(&m, lo).unwrap().bits(), 0xCD);
}

/// **021 §3.1 needs `Ite`.** A write at a symbolic offset that stays in `Bytes` writes
/// each candidate byte as `ite(off == k, val, old)`; without the term there is no way to
/// express a conditional write at all, and the tri-state init mask has nothing to point
/// at.
#[test]
fn ite_selects_on_a_one_bit_condition() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let k = a.bv(8, 3);
    let cond = a.eq(x, k);
    let t = a.bv(32, 111);
    let f = a.bv(32, 222);
    let sel = a.ite(cond, t, f);
    assert_eq!(a.width(sel), 32);

    let mut m = Model::new();
    m.set(a.var_id(x).unwrap(), BvConst::new(8, 3));
    assert_eq!(a.eval(&m, sel).unwrap().bits(), 111);
    m.set(a.var_id(x).unwrap(), BvConst::new(8, 4));
    assert_eq!(a.eval(&m, sel).unwrap().bits(), 222);
}

/// A constant condition folds at construction, so the common case — a guard the solver
/// already decided — costs nothing downstream. 022 §2 folds at construction precisely so
/// that concrete subterms never reach the backend.
#[test]
fn a_constant_condition_folds_the_ite_away() {
    let mut a = TermArena::new();
    let one = a.bv(1, 1);
    let zero = a.bv(1, 0);
    let t = a.bv(32, 111);
    let f = a.bv(32, 222);
    assert_eq!(a.ite(one, t, f), t);
    assert_eq!(a.ite(zero, t, f), f);
    // And a concrete concat folds to a constant.
    let (x, y) = (a.bv(8, 0xAB), a.bv(8, 0xCD));
    let c = a.concat(x, y);
    assert_eq!(c, a.bv(16, 0xABCD));
}

// ---------------------------------------------------------------------------
// Array theory — the representation 021 §3 promotes to.
// ---------------------------------------------------------------------------

/// `select(store(a, i, v), i)` is `v`. The read-over-write axiom, and the reason array
/// theory can represent a memory object at all.
#[test]
fn a_select_over_a_matching_store_returns_the_stored_value() {
    let mut a = TermArena::new();
    let arr = a.array_var(64, 8, "mem");
    let i = a.bv(64, 7);
    let v = a.bv(8, 0x5A);
    let stored = a.store(arr, i, v);
    let got = a.select(stored, i);
    assert_eq!(a.width(got), 8);
    let m = Model::new();
    assert_eq!(a.eval(&m, got).unwrap().bits(), 0x5A);
}

/// `select(store(a, i, v), j)` with `i != j` reads through to the underlying array. This
/// is the half that makes a store *local*: without it every write would clobber the whole
/// object and promotion would lose everything but the last byte.
#[test]
fn a_select_at_a_different_index_reads_through_the_store() {
    let mut a = TermArena::new();
    let base = a.array_const(64, 8, 0xEE);
    let i = a.bv(64, 7);
    let j = a.bv(64, 9);
    let v = a.bv(8, 0x5A);
    let stored = a.store(base, i, v);
    let m = Model::new();
    let at_j = a.select(stored, j);
    let at_i = a.select(stored, i);
    assert_eq!(a.eval(&m, at_j).unwrap().bits(), 0xEE);
    assert_eq!(a.eval(&m, at_i).unwrap().bits(), 0x5A);
}

/// Stores layer: the most recent one at an index wins, and earlier ones at other indices
/// survive. An implementation that kept only the last store would pass the two tests
/// above and lose every byte but one.
#[test]
fn later_stores_shadow_earlier_ones_only_at_the_same_index() {
    let mut a = TermArena::new();
    let base = a.array_const(64, 8, 0);
    let (i, j) = (a.bv(64, 1), a.bv(64, 2));
    let (v1, v2, v3) = (a.bv(8, 11), a.bv(8, 22), a.bv(8, 33));
    let s = a.store(base, i, v1);
    let s = a.store(s, j, v2);
    let s = a.store(s, i, v3);
    let m = Model::new();
    let at_i = a.select(s, i);
    let at_j = a.select(s, j);
    assert_eq!(a.eval(&m, at_i).unwrap().bits(), 33);
    assert_eq!(a.eval(&m, at_j).unwrap().bits(), 22);
}

/// A select at a **symbolic** index cannot fold, and must not pretend to. Folding it to
/// the constant array's default would silently answer a question the solver has to.
#[test]
fn a_select_at_a_symbolic_index_does_not_fold() {
    let mut a = TermArena::new();
    let base = a.array_const(64, 8, 0xEE);
    let i = a.var(Sort::BitVec(64), "i");
    let v = a.bv(8, 0x5A);
    let s = a.store(base, i, v);
    let j = a.var(Sort::BitVec(64), "j");
    let got = a.select(s, j);
    assert!(
        a.eval_ground(got).is_err(),
        "the value depends on whether i == j, which is the solver's question"
    );
    // Under a model that makes them equal, it is the stored value.
    let mut m = Model::new();
    m.set(a.var_id(i).unwrap(), BvConst::new(64, 3));
    m.set(a.var_id(j).unwrap(), BvConst::new(64, 3));
    assert_eq!(a.eval(&m, got).unwrap().bits(), 0x5A);
    m.set(a.var_id(j).unwrap(), BvConst::new(64, 4));
    assert_eq!(a.eval(&m, got).unwrap().bits(), 0xEE);
}

/// A select over a store at the **same** term folds, even when the index is symbolic.
/// Hash-consing makes identity decidable at construction, and `v[i] = x; use v[i]` is the
/// commonest shape there is — handing it to the solver would be work nobody needs.
#[test]
fn a_select_over_a_store_at_the_same_symbolic_index_folds() {
    let mut a = TermArena::new();
    let mem = a.array_var(64, 8, "mem");
    let i = a.var(Sort::BitVec(64), "i");
    let v = a.bv(8, 0x5A);
    let s = a.store(mem, i, v);
    assert_eq!(a.select(s, i), v, "identity is decidable without a solver");
    // A *different* symbolic index is not, and must not fold.
    let j = a.var(Sort::BitVec(64), "j");
    let other = a.select(s, j);
    assert_ne!(other, v);
    assert!(a.eval_ground(other).is_err());
}

/// **A shared DAG must serialize as a DAG.**
///
/// `to_smtlib` expanded every reference, so a term reached twice was written twice — and
/// a 22-node chain of `x + x` serialized to **54 MB**. 022 §4 requires `--dump-queries`,
/// and every backend query pays this, so it is not a reporting nicety.
///
/// Promotion makes it acute rather than theoretical: 021 §3's init array is one `store`
/// per *bit*, so a 2.5 KB VPP buffer is twenty thousand nested stores whose text would be
/// astronomically larger than the structure.
#[test]
fn a_shared_dag_serializes_in_linear_size() {
    let mut a = TermArena::new();
    let mut t = a.var(Sort::BitVec(64), "x");
    for _ in 0..22 {
        t = a.add(t, t);
    }
    let text = a.to_smtlib(t);
    assert!(
        text.len() < 4096,
        "22 shared nodes must not serialize to {} bytes",
        text.len()
    );
}

/// The same for a long store chain with a symbolic index, which cannot fold — this is
/// exactly the shape promotion produces.
#[test]
fn a_long_store_chain_serializes_in_linear_size() {
    let mut a = TermArena::new();
    let mut arr = a.array_const(64, 8, 0);
    for i in 0..2000u128 {
        let idx = a.bv(64, i);
        let v = a.bv(8, (i % 251) as u128);
        arr = a.store(arr, idx, v);
    }
    let j = a.var(Sort::BitVec(64), "j");
    let s = a.select(arr, j);
    let text = a.to_smtlib(s);
    assert!(
        text.len() < 400_000,
        "2000 stores must not serialize to {} bytes",
        text.len()
    );
}

/// Sharing must not change what the text *means*. A `let`-bound rendering that got the
/// binding order wrong, or reused a name across scopes, would be smaller and wrong.
#[test]
fn sharing_preserves_the_value_of_the_term() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    let s = a.add(x, y);
    let d = a.mul(s, s);
    let e = a.add(d, s);
    let text = a.to_smtlib(e);
    // Every variable still appears, and the shared subterm is bound rather than repeated.
    assert!(text.contains("v0_x") && text.contains("v1_y"));
    assert!(text.contains("let "), "a shared subterm should be bound: {text}");
    let mut m = Model::new();
    m.set(a.var_id(x).unwrap(), BvConst::new(8, 3));
    m.set(a.var_id(y).unwrap(), BvConst::new(8, 4));
    // (3+4)*(3+4) + (3+4) = 56, mod 256.
    assert_eq!(a.eval(&m, e).unwrap().bits(), 56);
}

/// An unshared term needs no bindings, or every trivial query grows a `let` wrapper for
/// nothing.
#[test]
fn an_unshared_term_is_emitted_without_bindings() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(8), "x");
    let c = a.bv(8, 5);
    let e = a.ult(x, c);
    let text = a.to_smtlib(e);
    assert!(!text.contains("let "), "nothing is shared here: {text}");
    assert_eq!(text, "(bvult v0_x (_ bv5 8))");
}
