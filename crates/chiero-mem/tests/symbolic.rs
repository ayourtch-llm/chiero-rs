//! Symbolic bytes, symbolic offsets, and promotion to array theory (021 §3, §3.1).
//!
//! Covers **021 contracts 5, 6, 6b**.
//!
//! The design this file pins: the overwhelming majority of VPP accesses are at *concrete*
//! offsets from a symbolic base (`p->field`, `v[i]` with `i` concretizable), and those
//! must not pay array theory's cost. So an object holds concrete bytes with a sparse
//! overlay of symbolic ones, and a read spanning both produces a `Concat` rather than a
//! promotion. Promotion happens on exactly one trigger — a write at a symbolic offset the
//! solver cannot pin to a small set — and is one-way within a state.
//!
//! The `ite_threshold` is a documented constant rather than a heuristic, because results
//! have to be reproducible: an object that promotes on one run and not the next would
//! answer the same program differently for no reason a reader could see.

use chiero_mem::*;
use chiero_solver::{BvConst, Model, Sort, TermArena};
use chiero_span::{BytePos, ExpnCtx, Span};

fn sp(lo: u32) -> Span {
    Span {
        lo: BytePos(lo),
        hi: BytePos(lo + 4),
        ctx: ExpnCtx(0),
    }
}

fn ptr(o: ObjectId, off: i64) -> Pointer {
    Pointer { base: o, off }
}

/// **021 contract 5.** Bytes 0..2 concrete, 2..4 symbolic; reading `Int(32)` yields a
/// term whose low half is concrete, and the object stays `Bytes`.
///
/// This is what makes type punning and partial overwrites exact. A model that promoted
/// here would pay array theory on every packet header parse in VPP.
#[test]
fn a_partly_symbolic_word_reads_as_a_concat_without_promoting() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[0xAA, 0xBB], sp(2));
    let x = a.var(Sort::BitVec(8), "x");
    let y = a.var(Sort::BitVec(8), "y");
    m.write_sym_byte(ptr(o, 2), x, sp(3));
    m.write_sym_byte(ptr(o, 3), y, sp(4));

    assert!(
        m.is_bytes(o),
        "a partly symbolic word must not force array theory"
    );

    let r = m.read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(5));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    let t = r.value.unwrap();
    assert_eq!(a.width(t), 32);

    // Under any assignment of the two symbolic bytes, the low half is what was written.
    let mut model = Model::new();
    model.set(a.var_id(x).unwrap(), BvConst::new(8, 0x11));
    model.set(a.var_id(y).unwrap(), BvConst::new(8, 0x22));
    assert_eq!(a.eval(&model, t).unwrap().bits(), 0x2211_BBAA);
    model.set(a.var_id(x).unwrap(), BvConst::new(8, 0x33));
    assert_eq!(
        a.eval(&model, t).unwrap().bits() & 0xFFFF,
        0xBBAA,
        "the concrete low half does not depend on the symbolic bytes"
    );
}

/// A wholly concrete read still folds to a constant — the overlay must not make every
/// read symbolic, or the fast path this design exists for is gone.
#[test]
fn a_wholly_concrete_read_folds_to_a_constant() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[1, 2, 3, 4], sp(2));
    let t = m
        .read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(3))
        .value
        .unwrap();
    assert_eq!(
        a.eval_ground(t).unwrap().bits(),
        0x0403_0201,
        "a concrete read must evaluate with no model at all"
    );
}

/// **021 contract 6b.** A conditional write leaves the candidate bytes `Cond` — not
/// `Yes`, not `No` — and a read at a *definitely different* offset still reports an
/// uninitialized read while one at a written offset does not.
#[test]
fn a_symbolic_offset_write_leaves_candidates_conditional() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0x7F);
    let r = m.write_at_symbolic_offset(&mut a, o, off, &[2, 3, 4], val, sp(2));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert!(
        m.is_bytes(o),
        "three candidates is well under the threshold"
    );

    for k in [2u64, 3, 4] {
        assert_eq!(m.init_bit_of(o, k * 8), InitBit::Cond, "byte {k}");
    }
    assert_eq!(
        m.init_bit_of(o, 0),
        InitBit::No,
        "byte 0 was not a candidate"
    );

    // A definitely-different offset is still an uninitialized read.
    assert!(
        m.read(ptr(o, 0), 1, sp(3))
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "byte 0 was never written under any condition"
    );
    // A written offset is not a *definite* uninitialized read. Reporting one here is the
    // false-positive storm on `v[i] = x; … use v[i]` that §3.1 exists to prevent.
    let at_k = m.read(ptr(o, 2), 1, sp(4));
    assert!(
        !at_k
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "a candidate byte must not report a definite uninitialized read: {:#?}",
        at_k.faults
    );
}

/// The third state needs a third *outcome*, or it collapses back into one of the two
/// §3.1 rejects. A conditionally-initialized read is reported as such — the engine
/// discharges the guard — rather than being silently accepted or definitely reported.
#[test]
fn a_conditionally_initialized_read_is_reported_conditionally() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 1);
    m.write_at_symbolic_offset(&mut a, o, off, &[2, 3], val, sp(2));
    let r = m.read(ptr(o, 2), 1, sp(3));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::MaybeUninitialized { .. })),
        "the guard is the engine's to discharge, not the model's to guess: {:#?}",
        r.faults
    );
    assert!(r.value.is_some());
}

/// **The guard must name the byte it guards.** Every test above checks the
/// *initialization* effect of a symbolic-offset write and none checks the *value*, so
/// building every candidate's guard as `off == 0` survived them all. Evaluating the
/// resulting term under each feasible offset is what distinguishes a correct chain from
/// a chain that writes the same byte three times.
#[test]
fn each_candidate_byte_is_guarded_by_its_own_offset() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[0xEE; 16], sp(2));
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0x7F);
    m.write_at_symbolic_offset(&mut a, o, off, &[2, 3], val, sp(3));

    let b2 = m
        .read_term(&mut a, ptr(o, 2), 1, Endian::Little, sp(4))
        .value
        .unwrap();
    let b3 = m
        .read_term(&mut a, ptr(o, 3), 1, Endian::Little, sp(5))
        .value
        .unwrap();
    let ov = a.var_id(off).unwrap();

    let mut model = Model::new();
    model.set(ov, BvConst::new(8, 2));
    assert_eq!(
        a.eval(&model, b2).unwrap().bits(),
        0x7F,
        "off == 2 writes byte 2"
    );
    assert_eq!(
        a.eval(&model, b3).unwrap().bits(),
        0xEE,
        "and leaves byte 3 alone"
    );

    model.set(ov, BvConst::new(8, 3));
    assert_eq!(
        a.eval(&model, b2).unwrap().bits(),
        0xEE,
        "off == 3 leaves byte 2 alone"
    );
    assert_eq!(
        a.eval(&model, b3).unwrap().bits(),
        0x7F,
        "and writes byte 3"
    );

    model.set(ov, BvConst::new(8, 9));
    assert_eq!(
        a.eval(&model, b2).unwrap().bits(),
        0xEE,
        "an infeasible offset writes neither"
    );
    assert_eq!(a.eval(&model, b3).unwrap().bits(), 0xEE);
}

/// A conditional write over **definitely initialized** memory stays definite: both
/// branches of the `ite` are initialized, so the join is `Yes` and nothing is reported.
#[test]
fn a_symbolic_offset_write_over_initialized_memory_reports_nothing() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[0; 16], sp(2)); // memset
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 9);
    m.write_at_symbolic_offset(&mut a, o, off, &[2, 3, 4], val, sp(3));
    assert_eq!(m.init_bit_of(o, 2 * 8), InitBit::Yes);
    assert!(m.read(ptr(o, 2), 1, sp(4)).faults.is_empty());
}

/// **021 contract 6, the promotion trigger.** Three feasible offsets keep the object as
/// `Bytes`; a set past `ITE_THRESHOLD` promotes it to `Array`.
#[test]
fn a_large_feasible_set_promotes_to_array_theory() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let small = m.alloc(ObjKind::Heap, 2048, 8, sp(1));
    let off = a.var(Sort::BitVec(16), "off");
    let val = a.bv(8, 1);
    let few: Vec<u64> = (0..3).collect();
    m.write_at_symbolic_offset(&mut a, small, off, &few, val, sp(2));
    assert!(m.is_bytes(small));

    let big = m.alloc(ObjKind::Heap, 2048, 8, sp(3));
    let many: Vec<u64> = (0..1000).collect();
    m.write_at_symbolic_offset(&mut a, big, off, &many, val, sp(4));
    assert!(
        !m.is_bytes(big),
        "1000 candidates must not be written as 1000 nested ites"
    );
}

/// **Promotion preserves the `(value, initialization-status)` pair**, not merely the
/// value. Contract 6 says so explicitly, because comparing values alone leaves exactly
/// the disagreement the tri-state exists to prevent untested — the two paths could agree
/// on every byte and still disagree about whether anyone wrote it.
#[test]
fn promotion_preserves_value_and_initialization_together() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.write(ptr(o, 0), &[7, 8], sp(2));
    let off = a.var(Sort::BitVec(16), "off");
    let val = a.bv(8, 5);
    m.write_at_symbolic_offset(&mut a, o, off, &[4], val, sp(3));

    let before: Vec<(Option<Vec<u8>>, InitBit)> = (0..8u64)
        .map(|b| {
            (
                m.read(ptr(o, b as i64), 1, sp(4)).value,
                m.init_bit_of(o, b * 8),
            )
        })
        .collect();
    m.promote_to_array(&mut a, o);
    assert!(!m.is_bytes(o));
    let after: Vec<(Option<Vec<u8>>, InitBit)> = (0..8u64)
        .map(|b| {
            (
                m.read(ptr(o, b as i64), 1, sp(5)).value,
                m.init_bit_of(o, b * 8),
            )
        })
        .collect();
    assert_eq!(
        before, after,
        "promotion changed what the object says about itself"
    );
}

/// Promotion is **one-way within a state** (021 §3). A representation that oscillated
/// would make cost unpredictable and results order-dependent.
#[test]
fn promotion_is_one_way() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.promote_to_array(&mut a, o);
    assert!(!m.is_bytes(o));
    // An ordinary concrete write does not demote it back.
    m.write(ptr(o, 0), &[1, 2, 3, 4], sp(2));
    assert!(!m.is_bytes(o));
    assert_eq!(m.read(ptr(o, 0), 4, sp(3)).value.unwrap(), vec![1, 2, 3, 4]);
}

/// The threshold is a **documented constant**, not a heuristic. An object that promoted
/// on one run and not the next would answer the same program differently for no reason a
/// reader could see, and 001 §5 makes determinism a hard requirement.
#[test]
fn the_threshold_is_a_named_constant_at_the_specified_value() {
    assert_eq!(ITE_THRESHOLD, 16, "021 §3 names 16 as the default");
}
