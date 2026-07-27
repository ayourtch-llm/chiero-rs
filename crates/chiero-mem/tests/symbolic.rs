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
    // The byte now holds a *symbol*, so the byte API reports that it cannot answer — a
    // separate matter from initialization, which is what this test is about. Nothing
    // about the write is uninitialized, conditionally or otherwise.
    let f = m.read(ptr(o, 2), 1, sp(4)).faults;
    assert!(
        !f.iter().any(|x| matches!(
            x,
            MemFault::Uninitialized { .. } | MemFault::MaybeUninitialized { .. }
        )),
        "{f:#?}"
    );
    assert!(
        m.read_term(&mut a, ptr(o, 2), 1, Endian::Little, sp(5))
            .faults
            .is_empty()
    );
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

// ---------------------------------------------------------------------------
// Wave 10, from the symbolic-layer review (39% escape). All probed first.
// ---------------------------------------------------------------------------

/// **A write past the threshold must not vanish.**
///
/// It called `promote_to_array` and returned success without writing anything — no byte,
/// no overlay entry, no init bit — so a large feasible set both *lost the value* and then
/// manufactured a **definite** uninitialized read on the very bytes it claimed to have
/// written. That is exactly the false-positive class §3.1 exists to prevent, produced by
/// the code meant to prevent it. The old test passed because it only checked a flag.
#[test]
fn a_write_past_the_threshold_still_writes() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    let off = a.var(Sort::BitVec(16), "off");
    let val = a.bv(8, 0x5A);
    let many: Vec<u64> = (0..17).collect();
    let r = m.write_at_symbolic_offset(&mut a, o, off, &many, val, sp(2));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert!(!m.is_bytes(o), "17 candidates is past the threshold");

    for k in 0..17u64 {
        assert_ne!(
            m.init_bit_of(o, k * 8),
            InitBit::No,
            "byte {k} was written and must not read as never-touched"
        );
        assert!(
            !m.read(ptr(o, k as i64), 1, sp(3))
                .faults
                .iter()
                .any(|f| matches!(f, MemFault::Uninitialized { .. })),
            "byte {k} must not report a *definite* uninitialized read"
        );
    }
}

/// The threshold boundary itself. The only behavioural test used 3 against 1000, and the
/// assertion that `ITE_THRESHOLD == 16` compares the constant against its own definition
/// — a tautology that pins nothing.
#[test]
fn the_threshold_boundary_is_exact() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let off = a.var(Sort::BitVec(16), "off");
    let val = a.bv(8, 1);
    let at_limit = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    let ks: Vec<u64> = (0..ITE_THRESHOLD as u64).collect();
    m.write_at_symbolic_offset(&mut a, at_limit, off, &ks, val, sp(2));
    assert!(
        m.is_bytes(at_limit),
        "exactly the threshold stays on the fast path"
    );

    let past = m.alloc(ObjKind::Heap, 64, 8, sp(3));
    let ks: Vec<u64> = (0..ITE_THRESHOLD as u64 + 1).collect();
    m.write_at_symbolic_offset(&mut a, past, off, &ks, val, sp(4));
    assert!(!m.is_bytes(past), "one more promotes");
}

/// **A read must not launder `Cond` into `Yes`.**
///
/// Memoizing the fresh symbol for a *definitely* uninitialized byte is correct and
/// required (contract 26). Doing it to a *conditionally* written byte silently discharges
/// the guard in chiero's favour — and it happened as a side effect of reading a
/// neighbouring byte, so a `Cond` next to a `No` was upgraded by the read that reported
/// the `No`. The code avoided this on one path and did it on the other.
#[test]
fn memoizing_an_uninitialized_read_does_not_upgrade_a_conditional_neighbour() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 1);
    m.write_at_symbolic_offset(&mut a, o, off, &[0], val, sp(2));
    assert_eq!(m.init_bit_of(o, 0), InitBit::Cond);

    // Byte 1 was never written; reading both reports it and must leave byte 0 alone.
    let r = m.read(ptr(o, 0), 2, sp(3));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "byte 1 is a definite uninitialized read"
    );
    assert_eq!(
        m.init_bit_of(o, 0),
        InitBit::Cond,
        "byte 0's guard must survive a read of its neighbour"
    );
}

/// **The `sym` overlay and the concrete bytes must not disagree.** A concrete write over
/// a symbolic byte reported success and updated `data`, while `read_term` kept returning
/// the stale symbol — a *wrong value*, not a missing finding. And the byte API read a
/// symbolic byte back as concrete `0` with no fault at all, which 021 §3 names as the
/// single most common way a symbolic executor produces confidently wrong results.
#[test]
fn a_concrete_write_replaces_a_symbolic_byte() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 0), x, sp(2));
    m.write(ptr(o, 0), &[0x42], sp(3));
    let t = m
        .read_term(&mut a, ptr(o, 0), 1, Endian::Little, sp(4))
        .value
        .unwrap();
    assert_eq!(
        a.eval_ground(t).unwrap().bits(),
        0x42,
        "the concrete write won, so the term must be concrete"
    );
}

/// Reading a symbolic byte through the *byte* API cannot answer, and must say so rather
/// than inventing a concrete zero.
#[test]
fn reading_a_symbolic_byte_through_the_byte_api_is_reported() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 0), x, sp(2));
    let r = m.read(ptr(o, 0), 1, sp(3));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "a concrete read of a symbolic byte must not silently return zero: {:#?}",
        r.faults
    );
}

/// **021 contract 21 holds for every write path, not one of three.** Neither symbolic
/// write consulted `readonly`, and a readonly global accepted both with zero faults while
/// its bytes changed.
#[test]
fn readonly_is_enforced_on_the_symbolic_write_paths() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.write(ptr(g, 0), &[1; 8], sp(2));
    m.set_readonly(g);
    let x = a.var(Sort::BitVec(8), "x");
    assert!(matches!(
        m.write_sym_byte(ptr(g, 0), x, sp(3)).faults[..],
        [MemFault::ReadOnly { .. }]
    ));
    let off = a.var(Sort::BitVec(8), "off");
    assert!(matches!(
        m.write_at_symbolic_offset(&mut a, g, off, &[0, 1], x, sp(4))
            .faults[..],
        [MemFault::ReadOnly { .. }]
    ));
    assert_eq!(m.read(ptr(g, 0), 1, sp(5)).value.unwrap(), vec![1]);
}

/// **A candidate past the end is the buffer overflow, not a candidate to skip.** They
/// were dropped silently, so a symbolic index whose feasible set spills past the object
/// produced no finding at all — and the `continue` guarding the drop was also the only
/// thing stopping a panic, so it was load-bearing for the wrong reason.
#[test]
fn candidates_outside_the_object_are_reported() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let off = a.var(Sort::BitVec(16), "off");
    let val = a.bv(8, 1);
    let r = m.write_at_symbolic_offset(&mut a, o, off, &[6, 7, 8, 4096], val, sp(2));
    let oob: Vec<_> = r
        .faults
        .iter()
        .filter(|f| matches!(f, MemFault::OutOfBounds { .. }))
        .collect();
    assert!(
        !oob.is_empty(),
        "8 and 4096 are past an 8-byte object: {:#?}",
        r.faults
    );
    // The in-bounds candidates still land.
    assert_eq!(m.init_bit_of(o, 6 * 8), InitBit::Cond);
}

/// **The candidate constant must be built wide enough to hold the candidate.** It used
/// `width(off)`, and `BvConst` masks — so an 8-bit index into a 512-byte object turned
/// candidate 300 into the guard `off == 44`, writing byte 300 whenever the index was 44.
/// A wrong value under a wrong condition, silently.
#[test]
fn a_candidate_wider_than_the_offset_type_is_not_truncated() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 512, 8, sp(1));
    m.set(ptr(o, 0), 0xEE, 512, sp(2));
    let off = a.var(Sort::BitVec(8), "off"); // too narrow to express 300
    let val = a.bv(8, 0x7F);
    let r = m.write_at_symbolic_offset(&mut a, o, off, &[300], val, sp(3));
    assert!(
        !r.faults.is_empty(),
        "a candidate the offset type cannot represent must be reported, not truncated"
    );
    let b300 = m
        .read_term(&mut a, ptr(o, 300), 1, Endian::Little, sp(4))
        .value
        .unwrap();
    let ov = a.var_id(off).unwrap();
    let mut model = Model::new();
    model.set(ov, BvConst::new(8, 44));
    assert_eq!(
        a.eval(&model, b300).unwrap().bits(),
        0xEE,
        "off == 44 must not write byte 300"
    );
}

/// A second symbolic write to the same byte must layer on the first, not discard it.
/// That is `for (i) v[i] = x` run twice, and dropping the earlier chain loses a whole
/// pass of writes.
#[test]
fn a_second_symbolic_write_layers_on_the_first() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.set(ptr(o, 0), 0xEE, 8, sp(2));
    let i = a.var(Sort::BitVec(8), "i");
    let j = a.var(Sort::BitVec(8), "j");
    let v1 = a.bv(8, 0x11);
    let v2 = a.bv(8, 0x22);
    m.write_at_symbolic_offset(&mut a, o, i, &[0], v1, sp(3));
    m.write_at_symbolic_offset(&mut a, o, j, &[0], v2, sp(4));

    let b0 = m
        .read_term(&mut a, ptr(o, 0), 1, Endian::Little, sp(5))
        .value
        .unwrap();
    let (iv, jv) = (a.var_id(i).unwrap(), a.var_id(j).unwrap());
    let mut model = Model::new();
    // Only the first write fires: its value must still be reachable.
    model.set(iv, BvConst::new(8, 0));
    model.set(jv, BvConst::new(8, 5));
    assert_eq!(
        a.eval(&model, b0).unwrap().bits(),
        0x11,
        "the first write's ite chain was discarded"
    );
    // Only the second fires.
    model.set(iv, BvConst::new(8, 5));
    model.set(jv, BvConst::new(8, 0));
    assert_eq!(a.eval(&model, b0).unwrap().bits(), 0x22);
}

/// `read_term` runs the same five steps as the byte API: it records misalignment and it
/// bounds-checks. Nothing inspected its fault channel at all, so the whole thing could be
/// deleted undetected.
#[test]
fn read_term_reports_the_same_faults_as_the_byte_api() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.set(ptr(o, 0), 0, 16, sp(2));
    let r = m.read_term(&mut a, ptr(o, 1), 4, Endian::Little, sp(3));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Misaligned { .. })),
        "021 §5 step 3 applies to every read path: {:#?}",
        r.faults
    );
    let oob = m.read_term(&mut a, ptr(o, 14), 4, Endian::Little, sp(4));
    assert!(oob.value.is_none());
    assert!(matches!(oob.faults[..], [MemFault::OutOfBounds { .. }]));
}

/// Big-endian `read_term` was entirely untested; only the little-endian direction had a
/// case, so an off-by-one in the index order would only have been caught in one of two
/// orders.
#[test]
fn read_term_assembles_big_endian_in_the_other_order() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.write(ptr(o, 0), &[0x11, 0x22, 0x33, 0x44], sp(2));
    let be = m
        .read_term(&mut a, ptr(o, 0), 4, Endian::Big, sp(3))
        .value
        .unwrap();
    assert_eq!(a.eval_ground(be).unwrap().bits(), 0x1122_3344);
    let le = m
        .read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(4))
        .value
        .unwrap();
    assert_eq!(a.eval_ground(le).unwrap().bits(), 0x4433_2211);
}
