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
        assert!(
            matches!(m.init_bit_of(o, k * 8), InitBit::Cond(_)),
            "byte {k}"
        );
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
    // The byte API cannot serve a promoted object — it has no arena with which to read
    // the arrays, and answering from the frozen `Bytes` view is the drift promotion
    // exists to avoid.
    assert!(matches!(
        m.read(ptr(o, 0), 4, sp(2)).faults[..],
        [MemFault::SymbolicByte { .. }]
    ));
    assert!(!m.is_bytes(o), "an ordinary access does not demote it back");
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
            m.init_bit_via(&mut a, o, k * 8),
            InitBit::No,
            "byte {k} was written and must not read as never-touched"
        );
        // The object is promoted, so the *term* API is the one that can answer — the
        // byte API has no arena with which to consult the arrays.
        assert!(
            !m.read_term(&mut a, ptr(o, k as i64), 1, Endian::Little, sp(3))
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
    assert!(matches!(m.init_bit_of(o, 0), InitBit::Cond(_)));

    // Byte 1 was never written; reading both reports it and must leave byte 0 alone.
    let r = m.read(ptr(o, 0), 2, sp(3));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "byte 1 is a definite uninitialized read"
    );
    assert!(
        matches!(m.init_bit_of(o, 0), InitBit::Cond(_)),
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
    assert!(matches!(m.init_bit_of(o, 6 * 8), InitBit::Cond(_)));
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

// ---------------------------------------------------------------------------
// The array path, and contract 6 as it was actually written.
// ---------------------------------------------------------------------------

/// **`InitBit::Cond` carries its guard** (021 §3.1 writes it as `Cond(Term)`).
///
/// Without the term there is nothing for the engine to discharge, so `MaybeUninitialized`
/// is a report the caller can only accept or reject wholesale — the two outcomes §3.1
/// rejects. It is also what makes §3.1's "`Cond` collapses to `Yes`/`No` whenever its
/// guard folds to a constant" expressible at all, and what promotion needs to build the
/// init array as `ite(t, 1, 0)`.
#[test]
fn a_conditional_init_bit_carries_the_guard_that_makes_it_conditional() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 1);
    m.write_at_symbolic_offset(&mut a, o, off, &[2], val, sp(2));

    let InitBit::Cond(g) = m.init_bit_of(o, 2 * 8) else {
        panic!("expected a guarded bit, got {:?}", m.init_bit_of(o, 2 * 8))
    };
    // The guard is `off == 2`, so it holds exactly when the write landed.
    let ov = a.var_id(off).unwrap();
    let mut model = Model::new();
    model.set(ov, BvConst::new(8, 2));
    assert_eq!(a.eval(&model, g).unwrap().bits(), 1);
    model.set(ov, BvConst::new(8, 3));
    assert_eq!(a.eval(&model, g).unwrap().bits(), 0);
}

/// Two conditional writes with *different* guards must be distinguishable. Dropping the
/// term made every `MaybeUninitialized` byte-identical, so the engine could not tell one
/// pending question from another.
#[test]
fn two_conditional_writes_carry_different_guards() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let i = a.var(Sort::BitVec(8), "i");
    let j = a.var(Sort::BitVec(8), "j");
    let val = a.bv(8, 1);
    m.write_at_symbolic_offset(&mut a, o, i, &[2], val, sp(2));
    m.write_at_symbolic_offset(&mut a, o, j, &[3], val, sp(3));
    let (InitBit::Cond(g2), InitBit::Cond(g3)) = (m.init_bit_of(o, 2 * 8), m.init_bit_of(o, 3 * 8))
    else {
        panic!("both bytes should be guarded")
    };
    assert_ne!(g2, g3, "different writes, different guards");
}

/// **021 contract 6, as written.** "For every feasible offset the two paths agree on the
/// `(value, initialization-status)` pair, not merely the value."
///
/// Until now there was only one path, so the test compared `Bytes` to itself. Two objects
/// are given identical histories, one promoted before the symbolic write and one after,
/// and every byte is compared *as a pair*. Comparing values alone would leave exactly the
/// disagreement the tri-state exists to prevent untested.
#[test]
fn the_bytes_path_and_the_array_path_agree_on_value_and_initialization() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let via_bytes = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let via_array = m.alloc(ObjKind::Heap, 16, 8, sp(2));

    // Identical histories: some concrete bytes, some never written.
    for o in [via_bytes, via_array] {
        m.write(ptr(o, 0), &[0x11, 0x22, 0x33, 0x44], sp(3));
    }
    // One is promoted *before* the symbolic write, so it takes the array path.
    m.promote_to_array(&mut a, via_array);

    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0x7F);
    let candidates = [5u64, 6, 7];
    for o in [via_bytes, via_array] {
        m.write_at_symbolic_offset(&mut a, o, off, &candidates, val, sp(4));
    }
    assert!(m.is_bytes(via_bytes));
    assert!(!m.is_bytes(via_array));

    let ov = a.var_id(off).unwrap();
    for k in candidates.iter().copied().chain([0, 1, 2, 3, 4, 8, 15]) {
        // Initialization status must agree, and `Cond` must agree *as a condition*, not
        // merely as a tag: both guards are evaluated under the same models.
        let (ib, ia) = (
            m.init_bit_via(&mut a, via_bytes, k * 8),
            m.init_bit_via(&mut a, via_array, k * 8),
        );
        match (ib, ia) {
            (InitBit::Cond(gb), InitBit::Cond(ga)) => {
                for probe in 0..16u128 {
                    let mut model = Model::new();
                    model.set(ov, BvConst::new(8, probe));
                    assert_eq!(
                        a.eval(&model, gb).unwrap().bits(),
                        a.eval(&model, ga).unwrap().bits(),
                        "byte {k}: guards disagree at off = {probe}"
                    );
                }
            }
            (x, y) => assert_eq!(x, y, "byte {k}: initialization status differs"),
        }

        // Values are compared only where the byte was *written*. A never-written byte
        // gets a fresh symbol per object (021 §3), and the two objects standing in for
        // the two paths are necessarily different objects — so their symbols differ by
        // construction and comparing them would be comparing the stand-in rather than
        // the contract. Promotion of a *single* object is checked separately.
        if ib == InitBit::No {
            continue;
        }
        let tb = m
            .read_term(&mut a, ptr(via_bytes, k as i64), 1, Endian::Little, sp(5))
            .value
            .unwrap();
        let ta = m
            .read_term(&mut a, ptr(via_array, k as i64), 1, Endian::Little, sp(6))
            .value
            .unwrap();
        for probe in candidates.iter().copied().chain([9, 10]) {
            let mut model = Model::new();
            model.set(ov, BvConst::new(8, probe as u128));
            assert_eq!(
                a.eval(&model, tb).unwrap().bits(),
                a.eval(&model, ta).unwrap().bits(),
                "byte {k}: values disagree at off = {probe}"
            );
        }
    }
}

/// Promotion itself preserves the pair. The previous test for this **measured after
/// mutating**: its `before` pass called `read`, which memoizes an uninitialized byte to
/// `Yes`, so every `No` had already become `Yes` by the time `after` was taken — and a
/// promotion that marked every never-written byte initialized passed it.
#[test]
fn promotion_preserves_value_and_initialization_without_reading_first() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 4), &[7, 8], sp(2)); // 0..4 and 6..16 never written
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 5);
    m.write_at_symbolic_offset(&mut a, o, off, &[9], val, sp(3));

    // `init_bit_of` is a pure observation; `read` is not.
    let before: Vec<InitBit> = (0..16u64).map(|b| m.init_bit_of(o, b * 8)).collect();
    m.promote_to_array(&mut a, o);
    let after: Vec<InitBit> = (0..16u64)
        .map(|b| m.init_bit_via(&mut a, o, b * 8))
        .collect();
    let ov = a.var_id(off).unwrap();
    for (b, (x, y)) in before.iter().zip(after.iter()).enumerate() {
        match (x, y) {
            // Guards are compared **semantically**, not by term identity: the array path
            // states the same condition as `select(init, bit) == 1`, which is a different
            // term saying the same thing. Comparing identity would fail on a correct
            // implementation and pass on one that merely reused the term.
            (InitBit::Cond(gx), InitBit::Cond(gy)) => {
                for probe in 0..16u128 {
                    let mut model = Model::new();
                    model.set(ov, BvConst::new(8, probe));
                    assert_eq!(
                        a.eval(&model, *gx).unwrap().bits(),
                        a.eval(&model, *gy).unwrap().bits(),
                        "byte {b}: guards disagree at off = {probe}"
                    );
                }
            }
            _ => assert_eq!(x, y, "byte {b}: promotion altered the mask"),
        }
    }
    assert!(
        before.contains(&InitBit::No),
        "the fixture must contain a never-written byte, or this proves nothing"
    );
    assert!(before.iter().any(|b| matches!(b, InitBit::Cond(_))));
    assert!(before.contains(&InitBit::Yes));
}

/// **Promotion carries the symbolic overlay across.** Every promotion test so far
/// promoted an object whose bytes were all concrete, so dropping the overlay entirely
/// passed them — and dropping it silently replaces every symbolic byte with whatever
/// stale concrete value sat underneath.
#[test]
fn promotion_carries_symbolic_bytes_across() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[0xEE; 16], sp(2));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 3), x, sp(3));

    m.promote_to_array(&mut a, o);
    let t = m
        .read_term(&mut a, ptr(o, 3), 1, Endian::Little, sp(4))
        .value
        .unwrap();
    let mut model = Model::new();
    model.set(a.var_id(x).unwrap(), BvConst::new(8, 0x77));
    assert_eq!(
        a.eval(&model, t).unwrap().bits(),
        0x77,
        "the symbolic byte became its stale concrete shadow"
    );
    // And a concrete neighbour still reads as itself.
    let n = m
        .read_term(&mut a, ptr(o, 4), 1, Endian::Little, sp(5))
        .value
        .unwrap();
    assert_eq!(a.eval_ground(n).unwrap().bits(), 0xEE);
}

/// **One-way means a second promotion is a no-op, not a rebuild.** Rebuilding the arrays
/// from the `Bytes` view — which is frozen at the first promotion — would discard
/// everything written since, silently. The existing test only checked that the flag
/// stayed set, which a rebuild also does.
#[test]
fn a_second_promotion_does_not_rebuild_from_the_frozen_bytes() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[0xEE; 16], sp(2));
    m.promote_to_array(&mut a, o);

    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0x5A);
    m.write_at_symbolic_offset(&mut a, o, off, &[6], val, sp(3));
    m.promote_to_array(&mut a, o); // must change nothing

    let t = m
        .read_term(&mut a, ptr(o, 6), 1, Endian::Little, sp(4))
        .value
        .unwrap();
    let mut model = Model::new();
    model.set(a.var_id(off).unwrap(), BvConst::new(8, 6));
    assert_eq!(
        a.eval(&model, t).unwrap().bits(),
        0x5A,
        "the second promotion rebuilt from the frozen Bytes view and lost the write"
    );
}

/// **`read_term` memoizes like `read` does** (021 §5, contract 26).
///
/// Without it one never-written byte is reported on *every* read, and the two read APIs
/// disagree about the same byte — `read` says "reported once, now defined", `read_term`
/// says "still uninitialized". Two APIs over one object cannot hold different opinions
/// about whether anybody wrote to it.
#[test]
fn read_term_memoizes_the_fresh_symbol_like_the_byte_api() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let first = m.read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(2));
    assert_eq!(first.faults.len(), 1, "{:#?}", first.faults);
    let second = m.read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(3));
    assert!(
        second.faults.is_empty(),
        "the fresh symbol is memoized, so the second read is not a new finding: {:#?}",
        second.faults
    );
    // The byte API no longer reports them *uninitialized* — but it cannot answer either,
    // because they now hold fresh symbols. `SymbolicByte` is the honest reply, and it is
    // a different statement from "nobody wrote this".
    let byte_api = m.read(ptr(o, 0), 4, sp(4));
    assert!(
        !byte_api
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "{:#?}",
        byte_api.faults
    );
    assert!(
        byte_api
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "{:#?}",
        byte_api.faults
    );
}

/// **Contract 6b holds for the bit API too.** §3.1 argues the tri-state *from bitfields*,
/// so enforcing it only on the byte path leaves the case it was designed for unguarded: a
/// conditionally-written bitfield must not report a *definite* uninitialized read.
#[test]
fn a_conditionally_written_bitfield_is_not_a_definite_finding() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0b101);
    m.write_at_symbolic_offset(&mut a, o, off, &[0], val, sp(2));

    let r = m.read_bits(ptr(o, 0), 0, 3, sp(3));
    assert!(
        !r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "a conditionally-written bitfield is not definitely uninitialized: {:#?}",
        r.faults
    );
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::MaybeUninitialized { .. })),
        "but it is not silently initialized either: {:#?}",
        r.faults
    );
    assert!(r.value.is_some(), "and a value comes back regardless");
    // A bitfield in a byte nobody touched is still a definite finding.
    let far = m.read_bits(ptr(o, 4), 0, 3, sp(4));
    assert!(
        far.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "{:#?}",
        far.faults
    );
}

// ---------------------------------------------------------------------------
// Wave 11, chiero-mem half. All four probed before being treated as findings.
// ---------------------------------------------------------------------------

/// **A byte write to a promoted object must not vanish.** `read` refuses one — its
/// contents live in the arrays and the `Bytes` view beneath is frozen — but no *write*
/// path did, so a store returned no faults, mutated the frozen view, and was invisible:
/// a wrong value *and* a spurious uninitialized-read finding on the byte just written.
#[test]
fn a_byte_write_to_a_promoted_object_is_refused_rather_than_lost() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.promote_to_array(&mut a, o);
    for faults in [
        m.write(ptr(o, 0), &[0xAB], sp(2)).faults,
        m.set(ptr(o, 0), 0xAB, 1, sp(3)).faults,
        m.write_bits(ptr(o, 0), 0, 8, 0xAB, sp(4)).faults,
    ] {
        assert!(
            matches!(faults[..], [MemFault::SymbolicByte { .. }]),
            "a write that cannot be represented must say so: {faults:#?}"
        );
    }
    let x = a.var(Sort::BitVec(8), "x");
    assert!(matches!(
        m.write_sym_byte(ptr(o, 0), x, sp(5)).faults[..],
        [MemFault::SymbolicByte { .. }]
    ));
}

/// **The join of two guarded writes is the *disjunction* of their guards.**
///
/// Taking only the newer one loses initialization: after `v[i] = 0x11` then `v[j] = 0x22`
/// at the same candidate, the model `i = 5, j = 9` has the byte holding `0x11` — the
/// first write fired — while the guard said "uninitialized". A byte cannot have a value
/// and simultaneously report that nobody wrote it, and the array path disagreed with the
/// Bytes path about exactly this, which is a **021 contract 6 violation**.
#[test]
fn two_guarded_writes_join_their_guards() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let i = a.var(Sort::BitVec(8), "i");
    let j = a.var(Sort::BitVec(8), "j");
    let v1 = a.bv(8, 0x11);
    let v2 = a.bv(8, 0x22);
    m.write_at_symbolic_offset(&mut a, o, i, &[5], v1, sp(2));
    m.write_at_symbolic_offset(&mut a, o, j, &[5], v2, sp(3));

    let InitBit::Cond(g) = m.init_bit_of(o, 5 * 8) else {
        panic!("expected a guarded bit")
    };
    let (iv, jv) = (a.var_id(i).unwrap(), a.var_id(j).unwrap());
    for (ival, jval, want) in [(5u128, 9u128, 1u128), (9, 5, 1), (9, 9, 0), (5, 5, 1)] {
        let mut model = Model::new();
        model.set(iv, BvConst::new(8, ival));
        model.set(jv, BvConst::new(8, jval));
        assert_eq!(
            a.eval(&model, g).unwrap().bits(),
            want,
            "i={ival}, j={jval}: initialized iff *either* write fired"
        );
    }
}

/// **021 §3.1: `Cond` collapses to `Yes`/`No` when its guard folds to a constant.**
///
/// A write at a *concrete* offset produces the guard `off == k`, which folds — but the
/// tag stayed `Cond`, so a definitely-written byte reported `MaybeUninitialized`. And
/// `init_bit_via` *does* collapse, so the two paths disagreed: a second contract 6
/// violation, on a different input class from the one above.
#[test]
fn a_conditional_write_at_a_concrete_offset_collapses_to_definite() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let off = a.bv(8, 3);
    let v = a.bv(8, 1);
    m.write_at_symbolic_offset(&mut a, o, off, &[3], v, sp(2));
    assert_eq!(
        m.init_bit_of(o, 3 * 8),
        InitBit::Yes,
        "the guard is `3 == 3`, so the byte is definitely written"
    );
    let r = m.read_term(&mut a, ptr(o, 3), 1, Endian::Little, sp(3));
    assert!(
        r.faults.is_empty(),
        "a definitely-written byte reports nothing: {:#?}",
        r.faults
    );
    // A candidate the offset cannot equal is definitely *not* written.
    m.write_at_symbolic_offset(&mut a, o, off, &[4], v, sp(4));
    assert_eq!(m.init_bit_of(o, 4 * 8), InitBit::No);
}

/// Contract 26 on a promoted object too: `read_term` memoized into the `InitMask` while
/// `init_bit_via` reads the array, so the memo was a no-op there and one never-written
/// byte was reported on every read.
#[test]
fn read_term_memoizes_on_a_promoted_object_as_well() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.promote_to_array(&mut a, o);
    let first = m.read_term(&mut a, ptr(o, 0), 1, Endian::Little, sp(2));
    assert_eq!(first.faults.len(), 1, "{:#?}", first.faults);
    let second = m.read_term(&mut a, ptr(o, 0), 1, Endian::Little, sp(3));
    assert!(
        second.faults.is_empty(),
        "the fresh symbol is memoized on both representations: {:#?}",
        second.faults
    );
}

/// Promotion is a state change, so it obeys the state check like every other operation:
/// a freed object is not promoted, and an object too large to materialize reports that
/// rather than silently staying `Bytes` while the caller believes otherwise.
#[test]
fn promotion_respects_object_state() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let freed = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.free(freed, sp(2));
    let r = m.promote_to_array(&mut a, freed);
    assert!(matches!(r.faults[..], [MemFault::UseAfterFree { .. }]));
    assert!(m.is_bytes(freed), "a dead object is not promoted");

    let huge = m.alloc(ObjKind::Heap, u64::MAX / 4, 8, sp(3));
    let r = m.promote_to_array(&mut a, huge);
    assert!(
        matches!(r.faults[..], [MemFault::AllocationTooLarge { .. }]),
        "silently staying Bytes leaves the caller believing a promotion happened: {:#?}",
        r.faults
    );
}

/// **A conditional write must not downgrade bits that were already definite** — even
/// when only *part* of the byte was.
///
/// The guard for this only matters when a byte's bits differ in state, which nothing
/// else in the suite produces: every other test writes whole bytes, so the byte-level
/// decision and the bit-level one agree and the guard is invisible. A bitfield write
/// leaves exactly that mixed byte, and it is the shape §3.1 argues the tri-state from.
#[test]
fn a_symbolic_write_does_not_downgrade_the_definite_bits_of_a_mixed_byte() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    // Bits 0..4 of byte 0 definitely written; 4..8 never touched.
    m.write_bits(ptr(o, 0), 0, 4, 0b1011, sp(2));
    assert_eq!(m.init_bit_of(o, 0), InitBit::Yes);
    assert_eq!(m.init_bit_of(o, 4), InitBit::No);

    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0x7F);
    m.write_at_symbolic_offset(&mut a, o, off, &[0], val, sp(3));

    for bit in 0..4 {
        assert_eq!(
            m.init_bit_of(o, bit),
            InitBit::Yes,
            "bit {bit} was definitely written and a guarded write cannot unwrite it"
        );
    }
    for bit in 4..8 {
        assert!(
            matches!(m.init_bit_of(o, bit), InitBit::Cond(_)),
            "bit {bit} is written only when the guard holds"
        );
    }
}

/// **021 §3: an uninitialized read yields a fresh *symbol*, never zero.**
///
/// The spec names silently reading zero as "the single most common way a symbolic
/// executor produces confidently wrong results", and `read_term` was returning the
/// concrete `0` sitting behind the uninitialized byte. A checker downstream would then
/// reason about a value nobody wrote, and reason about it *confidently*.
///
/// Minting the symbol and memoizing it are the same act: contract 26 wants the repeated
/// read to give the same term, and §3 wants the term to be a symbol.
#[test]
fn an_uninitialized_read_yields_a_fresh_symbol_not_zero() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let r = m.read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(2));
    let t = r.value.expect("a value comes back alongside the fault");
    assert!(
        a.eval_ground(t).is_err(),
        "a never-written word must not evaluate to a constant"
    );
    let mut vars = Vec::new();
    a.vars_of(t, &mut vars);
    assert_eq!(vars.len(), 4, "one fresh symbol per never-written byte");

    // Contract 26: the same term on a repeat read, and no second finding.
    let again = m.read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(3));
    assert_eq!(again.value.unwrap(), t, "the fresh symbol is memoized");
    assert!(again.faults.is_empty());
}

/// Two *different* uninitialized bytes get two different symbols, or a model could not
/// assign them independently and every uninitialized buffer would read as a constant
/// pattern.
#[test]
fn distinct_uninitialized_bytes_get_distinct_symbols() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let b0 = m
        .read_term(&mut a, ptr(o, 0), 1, Endian::Little, sp(2))
        .value
        .unwrap();
    let b1 = m
        .read_term(&mut a, ptr(o, 1), 1, Endian::Little, sp(3))
        .value
        .unwrap();
    assert_ne!(b0, b1);
    let mut v0 = Vec::new();
    let mut v1 = Vec::new();
    a.vars_of(b0, &mut v0);
    a.vars_of(b1, &mut v1);
    assert_ne!(v0, v1);
}

/// A byte that *was* written reads back as its value, not as a symbol — otherwise the
/// tests above are satisfied by a model that symbolizes everything, and every concrete
/// computation would become a solver query.
#[test]
fn an_initialized_read_is_still_concrete() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[1, 2, 3, 4], sp(2));
    let t = m
        .read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(3))
        .value
        .unwrap();
    assert_eq!(a.eval_ground(t).unwrap().bits(), 0x0403_0201);
}

// ---------------------------------------------------------------------------
// Symbolic-offset reads, and the unpinned store promotion exists for.
// ---------------------------------------------------------------------------

/// **021 §3: a read at a symbolic offset with a small feasible set is answered by an
/// if-then-else chain**, without promoting. This is the other half of `ITE_THRESHOLD`,
/// which until now was consulted only on the write path — so the threshold governed half
/// of what §3 says it governs.
#[test]
fn a_symbolic_offset_read_within_the_threshold_is_an_ite_chain() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[10, 11, 12, 13, 14, 15, 16, 17], sp(2));
    let off = a.var(Sort::BitVec(8), "off");

    let r = m.read_term_at(&mut a, o, off, &[2, 3, 4], sp(3));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert!(m.is_bytes(o), "a small feasible set does not promote");

    let t = r.value.unwrap();
    let ov = a.var_id(off).unwrap();
    for (k, want) in [(2u128, 12u128), (3, 13), (4, 14)] {
        let mut model = Model::new();
        model.set(ov, BvConst::new(8, k));
        assert_eq!(
            a.eval(&model, t).unwrap().bits(),
            want,
            "off = {k} must read byte {k}"
        );
    }
}

/// **A read past the threshold promotes**, which is what array theory is for: one
/// `select` at a symbolic index rather than a thousand nested selects.
#[test]
fn a_symbolic_offset_read_past_the_threshold_promotes() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 2048, 8, sp(1));
    m.set(ptr(o, 0), 0x5A, 2048, sp(2));
    let off = a.var(Sort::BitVec(16), "off");
    let many: Vec<u64> = (0..1000).collect();

    let r = m.read_term_at(&mut a, o, off, &many, sp(3));
    assert!(!m.is_bytes(o), "1000 candidates is past the threshold");
    let t = r.value.unwrap();
    // Every byte holds 0x5A, so the answer is 0x5A whichever index the model picks.
    let ov = a.var_id(off).unwrap();
    for k in [0u128, 500, 999] {
        let mut model = Model::new();
        model.set(ov, BvConst::new(16, k));
        assert_eq!(a.eval(&model, t).unwrap().bits(), 0x5A);
    }
}

/// **A promoted object takes an *unpinned* symbolic store** — one `store(data, off, val)`
/// at a symbolic index, with no candidate enumeration. That is the write promotion exists
/// *for*, and enumerating candidates after promoting defeats the point.
#[test]
fn a_promoted_object_takes_an_unpinned_symbolic_store() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 256, 8, sp(1));
    m.set(ptr(o, 0), 0xEE, 256, sp(2));
    m.promote_to_array(&mut a, o);

    let off = a.var(Sort::BitVec(8), "off");
    let val = a.bv(8, 0x7F);
    let w = m.store_at(&mut a, o, off, val, sp(3));
    assert!(w.faults.is_empty(), "{:#?}", w.faults);

    // Reading the same symbolic index gives what was just written, for *every* index —
    // no enumeration was involved, so there is nothing to have missed.
    let r = m.read_term_at(&mut a, o, off, &[], sp(4)).value.unwrap();
    let ov = a.var_id(off).unwrap();
    for k in [0u128, 7, 200, 255] {
        let mut model = Model::new();
        model.set(ov, BvConst::new(8, k));
        assert_eq!(a.eval(&model, r).unwrap().bits(), 0x7F, "index {k}");
    }
    // The store also *initializes* what it wrote. Without this the byte holds the right
    // value and simultaneously reports that nobody wrote it, which is the contradiction
    // the init array exists to prevent.
    for b in [0u64, 7, 255] {
        let bit = m.init_bit_via(&mut a, o, b * 8);
        let mut model = Model::new();
        model.set(ov, BvConst::new(8, b as u128));
        match bit {
            InitBit::Yes => {}
            InitBit::Cond(g) => assert_eq!(
                a.eval(&model, g).unwrap().bits(),
                1,
                "byte {b} is written when the offset is {b}"
            ),
            InitBit::No => panic!("byte {b} was written and reports otherwise"),
        }
    }
}

/// **An offset of any width is usable against the array.**
///
/// The array keeps one canonical index width and offsets are coerced at the boundary,
/// rather than each object's arrays taking the width of whichever offset happened to
/// reach them first. Either design works; this one keeps a single array per object no
/// matter how many differently-typed indices address it.
///
/// What must not happen is an index of the wrong width reaching a `store` or `select`:
/// that is a sort error the backend rejects, and the arena cannot catch it because
/// arrays carry no scalar width — so it would surface as another unexplained "backend
/// gave no usable answer", exactly as `as const` under `QF_ABV` did.
#[test]
fn an_offset_of_any_width_is_usable_against_the_array() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.promote_to_array(&mut a, o);
    let off = a.var(Sort::BitVec(16), "off");
    let val = a.bv(8, 1);
    let w = m.store_at(&mut a, o, off, val, sp(2));
    assert!(
        w.faults.is_empty(),
        "a 16-bit offset must be usable against the array: {:#?}",
        w.faults
    );
    // Read at a **different** symbolic index, or `select` folds over the store by
    // syntactic identity and no array term reaches the backend at all — which would make
    // the check below vacuous in exactly the way the first draft of it was.
    let off2 = a.var(Sort::BitVec(16), "off2");
    let r = m.read_term_at(&mut a, o, off2, &[], sp(3));
    let t = r.value.expect("a value comes back");

    // **The check that matters is the backend's.** The arena builds a mis-sorted index
    // happily — arrays carry no scalar width, so there is nothing for it to assert
    // against — and the evaluator compares numerically, so both widths agree there too.
    // Only a solver reads the sorts, which is exactly why this went unnoticed for
    // `as const`.
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping the backend half: no SMT-LIB2 backend on PATH");
        return;
    };
    let want = a.bv(8, 1);
    let e = a.eq(t, want);
    let y = a.var(Sort::BitVec(8), "y");
    let p = a.mul(y, y);
    let sq = a.bv(8, 49);
    let e2 = a.eq(p, sq);
    let mut s = chiero_solver::TieredSolver::with_backend(backend);
    chiero_solver::Solver::assert(&mut s, e);
    chiero_solver::Solver::assert(&mut s, e2);
    assert!(
        !matches!(
            chiero_solver::Solver::check(&mut s, &mut a, &[]),
            chiero_solver::CheckResult::Unknown(_)
        ),
        "a mis-sorted index makes the whole script unparseable"
    );
}

/// **024 §2.1's havoc, `Symbolic`.** The object's contents become unconstrained: a read
/// no longer answers with the byte that was there. It must **not** become an
/// uninitialized-read finding — `Symbolic` means known-unknown, and 021 §3.1's whole
/// point is that symbolic is not uninitialized.
#[test]
fn a_symbolic_havoc_forgets_the_contents_without_inventing_a_finding() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 8, sp(2));
    m.havoc_object(&mut a, o, HavocFill::Symbolic, sp(3));
    let r = m.read_term(&mut a, ptr(o, 0), 4, Endian::Little, sp(4));
    assert!(
        r.faults.is_empty(),
        "symbolic is not uninitialized: {:#?}",
        r.faults
    );
    let t = r.value.expect("a term comes back");
    assert!(
        a.eval_ground(t).is_err(),
        "the old 0xAB bytes are gone, not preserved"
    );
}

/// **024 §2.1's havoc, `Uninitialized`.** The mirror: reading now *is* a finding. The two
/// fills have to be distinguishable, or the choice 024 calls out as having "no safe
/// default" would be decoration.
#[test]
fn an_uninitialized_havoc_makes_the_next_read_a_finding() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 8, sp(2));
    m.havoc_object(&mut a, o, HavocFill::Uninitialized, sp(3));
    let r = m.read(ptr(o, 0), 4, sp(4));
    assert!(
        r.faults.iter().any(|f| {
            matches!(
                f,
                MemFault::Uninitialized { .. } | MemFault::MaybeUninitialized { .. }
            )
        }),
        "the bytes are gone and chiero says so: {:#?}",
        r.faults
    );
}

/// **024 contract 21d.** A havoc follows pointers stored *inside* the object. Provenance
/// is not kept in bytes, so this is the same range search `int_to_ptr` falls back to —
/// an aligned word whose value lands inside a live object.
///
/// Depth 0 must **not** follow them, or the parameter is decoration and the conservative
/// default cannot be distinguished from the cheap one.
#[test]
fn a_havoc_follows_stored_pointers_to_the_declared_depth() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let inner = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let outer = m.alloc(ObjKind::Heap, 8, 8, sp(2));
    m.set(ptr(inner, 0), 0xCD, 8, sp(3));
    let addr = m.addr_of(inner).expect("placed");
    m.write(ptr(outer, 0), &addr.to_le_bytes(), sp(4));

    let mut shallow = m.clone();
    assert_eq!(
        shallow
            .havoc(&mut a, &[outer], 0, HavocFill::Uninitialized, sp(5))
            .objects,
        vec![outer],
        "depth 0 stops at the object itself"
    );
    assert!(
        shallow.read(ptr(inner, 0), 4, sp(6)).faults.is_empty(),
        "the pointee is untouched"
    );

    let reached = m.havoc(&mut a, &[outer], 1, HavocFill::Uninitialized, sp(7));
    assert!(
        reached.objects.contains(&inner),
        "depth 1 reaches the pointee: {reached:?}"
    );
    assert!(
        !m.read(ptr(inner, 0), 4, sp(8)).faults.is_empty(),
        "and invalidates it"
    );
}

/// **D1: a havoc must not rewrite a read-only object.** `write`, `write_bits` and
/// `write_sym_at_off` all refuse; `havoc_object` did not, and since `Symbolic` promotes,
/// the object came back *unreadable* rather than merely modified. A callee that writes
/// through a `const char *` is UB, so invalidating there discards information the standard
/// guarantees — `printf("count=%d\n", n)` would destroy the literal. Found by review.
#[test]
fn a_havoc_leaves_readonly_objects_alone() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Global, 8, 1, sp(1));
    m.write(ptr(o, 0), b"hello\0", sp(2));
    m.set_readonly(o);
    let reached = m.havoc(&mut a, &[o], 0, HavocFill::Symbolic, sp(3));
    assert!(reached.objects.is_empty(), "nothing was invalidated");
    let r = m.read(ptr(o, 0), 6, sp(4));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert_eq!(r.value, Some(b"hello\0".to_vec()));
}

/// **D6: the reached set names what was actually invalidated.** `havoc` pushed to it
/// before deciding to skip, so `free(p); unknown(p);` reported "1 object(s) invalidated"
/// having invalidated none — and `NULL` and `UNBOUND` were counted too. A count that
/// includes skips is worse than no count: it reads as coverage.
#[test]
fn the_reached_set_excludes_what_the_havoc_skipped() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let live = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let dead = m.alloc(ObjKind::Heap, 8, 8, sp(2));
    m.free(dead, sp(3));
    let reached = m.havoc(
        &mut a,
        &[ObjectId::NULL, ObjectId::UNBOUND, dead, live],
        0,
        HavocFill::Symbolic,
        sp(4),
    );
    assert_eq!(reached.objects, vec![live]);
}

/// **D5: the pointer scan's cap is reported, not silent.** `HAVOC_SCAN_BYTES` bounds the
/// walk, and `havoc` returned only the reached set — so an object with a pointer past the
/// cap kept a stale pointee and nothing said so. The doc claimed the return value carried
/// this; it did not. A cap nobody hears about reads as "followed everything".
#[test]
fn a_truncated_pointer_scan_says_so() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let inner = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let big = m.alloc(ObjKind::Heap, HAVOC_SCAN_BYTES + 16, 8, sp(2));
    m.set(ptr(big, 0), 0, HAVOC_SCAN_BYTES + 16, sp(3));
    let addr = m.addr_of(inner).expect("placed");
    m.write(
        ptr(big, HAVOC_SCAN_BYTES as i64 + 8),
        &addr.to_le_bytes(),
        sp(4),
    );
    let reached = m.havoc(&mut a, &[big], 1, HavocFill::Uninitialized, sp(5));
    assert!(
        !reached.objects.contains(&inner),
        "the pointee is past the cap"
    );
    assert!(
        reached.truncated,
        "and the caller is told the walk was cut short"
    );

    // A scan that fits is not reported as truncated, or the flag says nothing.
    let small = m.alloc(ObjKind::Heap, 16, 8, sp(6));
    m.set(ptr(small, 0), 0, 16, sp(7));
    let reached = m.havoc(&mut a, &[small], 1, HavocFill::Uninitialized, sp(8));
    assert!(!reached.truncated);
}

/// **D4: a second havoc of the same object cannot follow its pointers.** `Symbolic`
/// promotes, and a promoted object has no byte view to scan — so `f(&s); g(&s);` left
/// `*s.q` valid after `g`, and nothing said the depth had collapsed. Silent loss of
/// reachability is the same failure as the silent cap.
#[test]
fn a_havoc_that_cannot_follow_pointers_reports_it() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let inner = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let outer = m.alloc(ObjKind::Heap, 8, 8, sp(2));
    m.set(ptr(inner, 0), 0xCD, 8, sp(3));
    let addr = m.addr_of(inner).expect("placed");
    m.write(ptr(outer, 0), &addr.to_le_bytes(), sp(4));

    let first = m.havoc(&mut a, &[outer], 1, HavocFill::Symbolic, sp(5));
    assert!(first.objects.contains(&inner));
    assert!(!first.truncated);

    let second = m.havoc(&mut a, &[outer], 1, HavocFill::Symbolic, sp(6));
    assert!(
        second.truncated,
        "the bytes it would have scanned are gone, and that is not the same as \
         'there was nothing there'"
    );
}

/// **A havoc does not follow a word it cannot vouch for.** `pointees` reads only
/// initialized concrete words: an uninitialized one is whatever the allocator left there,
/// and following it invents a reference to an object the program never named.
#[test]
fn the_pointer_scan_ignores_uninitialized_words() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let inner = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let outer = m.alloc(ObjKind::Heap, 8, 8, sp(2));
    m.set(ptr(inner, 0), 0xCD, 8, sp(3));
    // The address is *there* in the bytes, but nothing wrote it: `write_raw_for_test`
    // puts the bytes down without marking them initialized, which is exactly the state
    // fresh heap memory is in.
    let addr = m.addr_of(inner).expect("placed");
    m.write_uninit_bytes_for_test(ptr(outer, 0), &addr.to_le_bytes());
    let reached = m.havoc(&mut a, &[outer], 1, HavocFill::Uninitialized, sp(4));
    assert_eq!(
        reached.objects,
        vec![outer],
        "an uninitialized word is not a reference"
    );
}

/// **Clearing an object's contents clears its symbolic overlay too.** A leftover overlay
/// entry would make a byte of a freshly-invalidated object read back as the symbol the
/// *previous* contents had, which is a stale value wearing an unknown's clothes.
#[test]
fn an_uninitialized_havoc_drops_the_symbolic_overlay() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.set(ptr(o, 0), 1, 8, sp(2));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 2), x, sp(3));
    m.havoc(&mut a, &[o], 0, HavocFill::Uninitialized, sp(4));
    let r = m.read(ptr(o, 2), 1, sp(5));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "uninitialized: {:#?}",
        r.faults
    );
    // **And not *also* symbolic.** Clearing the init mask alone makes the two cases give
    // the same answer here, so the absence of the overlay is what the mutation turns on:
    // a leftover entry would make this byte read back as the symbol the *previous*
    // contents had — a stale value wearing an unknown's clothes.
    assert!(
        !r.faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "the overlay is gone too: {:#?}",
        r.faults
    );
}

/// **020 contract 25, the half I cited and did not test.** "`StoreBits` to `a` leaves every
/// bit of `b` unchanged, **including when `b`'s bits are symbolic** — checked by term
/// equality before and after."
///
/// The bit API and the symbolic overlay do not know about each other: `write_bits` touches
/// only `data` and never `sym`, and `read_bits` never calls `first_symbolic`. So a
/// `StoreBits` into a symbolic byte is **silently lost**, and the neighbouring bitfield
/// reads back as a definite constant with no fault — chiero proves `s.b == 0` about bits it
/// knows nothing about and prunes the real path. A false negative wearing a proof.
///
/// I claimed contract 25 was "the same fact from the other side" as 24. It is not: 24 is
/// about the init *mask*, 25 is about the *value*. Found by review.
#[test]
fn a_bit_write_into_a_symbolic_byte_is_not_silently_lost() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 4, 4, sp(1));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 0), x, sp(2));

    // `s.a = 0b101` over bits 0..3 of a byte whose value is unknown.
    // **The write is refused**, not half-applied. A bit-granular write into a symbolic
    // byte cannot be represented — 021 §3.1's `Cond` machinery is what would allow it and
    // does not reach the bit API — so claiming success and changing nothing is the one
    // outcome that must not happen.
    let w = m.write_bits(ptr(o, 0), 0, 3, 0b101, sp(3));
    assert!(
        w.faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "the write says it could not be performed: {:#?}",
        w.faults
    );
    let landed = w.faults.is_empty();

    // Reading the *neighbouring* field must never hand back a definite value for bits that
    // came from the symbol.
    let r = m.read_bits(ptr(o, 0), 3, 5, sp(4));
    assert!(
        r.value.is_none() || !r.faults.is_empty(),
        "b is symbolic: a concrete answer with no fault is a proof about unknown bits \
         (write landed: {landed}, got {:?})",
        r.value
    );
}

/// **`havoc_range` clobbers every byte it was given, not just the first.** The engine test
/// for 020 contract 11 pinned only the *upper* bound — it read eight bytes and asserted
/// *any* symbolic-byte fault, which one clobbered byte satisfies — so `0..size.min(1)`,
/// `0..size-1` and a fixed offset all survived the suite. Found by review.
///
/// Checking each byte individually is the only assertion that distinguishes them.
#[test]
fn a_havoc_range_clobbers_every_byte_it_was_given() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 16, sp(2));
    assert!(m.havoc_range(&mut a, ptr(o, 4), 8, HavocFill::Symbolic, sp(3)));
    for i in 4..12i64 {
        let r = m.read(ptr(o, i), 1, sp(4));
        assert!(
            r.faults
                .iter()
                .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
            "byte {i} was declared clobbered: {:#?}",
            r.faults
        );
    }
    // And the bytes either side are untouched — a fault there would mean the range ran
    // wide, which the per-byte loop above cannot see on its own.
    for i in [0i64, 3, 12, 15] {
        let r = m.read(ptr(o, i), 1, sp(5));
        assert!(r.faults.is_empty(), "byte {i} was not: {:#?}", r.faults);
        assert_eq!(r.value, Some(vec![0xAB]));
    }
}

/// **A range that runs off the end reports the overflow.** `havoc_range` discarded the
/// `OutOfBounds` fault from each byte write, so an inline-asm block declaring a 16-byte
/// clobber of an 8-byte buffer was a buffer overflow chiero *detected and did not report*
/// — it only degraded fidelity. It also returned `false` after having already mutated the
/// object, so the caller could not tell "nothing happened" from "half happened". Found by
/// review.
#[test]
fn a_havoc_range_past_the_end_reports_and_says_how_far_it_got() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 8, sp(2));
    let r = m.havoc_range_reporting(&mut a, ptr(o, 0), 16, HavocFill::Symbolic, sp(3));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::OutOfBounds { .. })),
        "the declared clobber is twice the object: {:#?}",
        r.faults
    );
    // It says how far it got, so a partial mutation is not indistinguishable from none.
    assert_eq!(r.value, Some(8), "eight bytes were actually clobbered");
}

/// **`HavocFill::Uninitialized` refuses what `Symbolic` refuses.** It mutated read-only
/// and freed objects, and on a *promoted* one it reported success while changing nothing —
/// which is precisely what 020 §4.3's "never silently a no-op" forbids. The arm has no
/// in-tree caller today, so every mutation of it survived; that is a reason to fix it
/// before something calls it, not a reason to leave it. Found by review.
#[test]
fn an_uninitialized_havoc_range_refuses_what_it_cannot_do() {
    let mut a = TermArena::new();
    let mut m = Memory::new();

    let ro = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.write(ptr(ro, 0), b"hello!!\0", sp(2));
    m.set_readonly(ro);
    assert!(
        !m.havoc_range(&mut a, ptr(ro, 0), 8, HavocFill::Uninitialized, sp(3)),
        "read-only memory is not written, by anyone"
    );
    let r = m.read(ptr(ro, 0), 8, sp(4));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert_eq!(r.value, Some(b"hello!!\0".to_vec()));

    let dead = m.alloc(ObjKind::Heap, 8, 8, sp(5));
    m.free(dead, sp(6));
    assert!(
        !m.havoc_range(&mut a, ptr(dead, 0), 8, HavocFill::Uninitialized, sp(7)),
        "freed memory is not invalidated further"
    );

    let promoted = m.alloc(ObjKind::Heap, 8, 8, sp(8));
    m.set(ptr(promoted, 0), 1, 8, sp(9));
    m.promote_to_array(&mut a, promoted);
    assert!(
        !m.havoc_range(
            &mut a,
            ptr(promoted, 0),
            8,
            HavocFill::Uninitialized,
            sp(10)
        ),
        "a promoted object has no byte view to clear — reporting success while changing \
         nothing is what 020 §4.3 forbids"
    );
}

/// **A declared clobber at a negative offset is out of bounds, not a wild pointer.**
/// Folding the two together said "matching no known object" about an object it had just
/// looked up, lost the object component of 023 §6.1's dedup key — `WildPointer` has none —
/// and, because `WildPointer` is fatal, **killed the path**, so nothing after the asm block
/// was analysed. vppinfra's vector header lives below the user pointer, so this is the
/// ordinary shape, not a corner. Found by review.
#[test]
fn a_havoc_range_below_the_object_is_out_of_bounds() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let r = m.havoc_range_reporting(&mut a, ptr(o, -8), 4, HavocFill::Symbolic, sp(2));
    let f = r.faults.first().expect("a fault");
    assert_eq!(f.kind(), "out-of-bounds", "{:#?}", r.faults);
    assert_eq!(
        f.object(),
        Some(o),
        "and it names the object, so the dedup key keeps its component"
    );
    assert!(!f.is_fatal() || f.kind() == "out-of-bounds");
}

/// **A refusal is not a success, whatever the size.** `havoc_range`'s bool wrapper compared
/// the byte count against the requested size, and a refusal reports zero — so at `size ==
/// 0` every refusal, including freed and read-only memory, reported success. Found by
/// review.
#[test]
fn a_zero_size_havoc_range_refusal_is_still_a_refusal() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.free(o, sp(2));
    assert!(
        !m.havoc_range(&mut a, ptr(o, 0), 0, HavocFill::Symbolic, sp(3)),
        "freed memory is not successfully clobbered, even zero bytes of it"
    );
}

/// **A bit range spanning into a symbolic byte is refused.** The check looked at the
/// range's bytes, but every fixture kept the symbol in the *first* one — so a mutation
/// examining only that byte survived. `struct S { unsigned a : 12; }` spans two bytes, and
/// a write to `a` reaching a symbolic second byte is the same defect 020 contract 25 fixed,
/// one byte over. The collection-of-one trap again, on a byte range. Found by review.
#[test]
fn a_bit_range_reaching_a_symbolic_byte_is_refused() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 4, 4, sp(1));
    // Byte 0 concrete, byte 1 symbolic.
    m.set(ptr(o, 0), 0, 1, sp(2));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 1), x, sp(3));

    // A 12-bit field starting at bit 0 reaches into byte 1.
    let w = m.write_bits(ptr(o, 0), 0, 12, 0x123, sp(4));
    assert!(
        w.faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "the range reaches a byte chiero cannot represent a bit write into: {:#?}",
        w.faults
    );
    // And the concrete byte is untouched: a refusal is not a partial write.
    assert_eq!(m.read(ptr(o, 0), 1, sp(5)).value, Some(vec![0]));
}

/// **`HavocFill::Uninitialized`'s success path.** Only its three refusals were tested, so
/// making the arm a total no-op — or deleting its bounds check — survived the whole suite.
/// The arm has no in-tree caller yet, which is exactly why it needs a test before one
/// arrives. Found by review.
#[test]
fn an_uninitialized_havoc_range_clears_exactly_the_range() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 16, sp(2));
    assert!(m.havoc_range(&mut a, ptr(o, 4), 8, HavocFill::Uninitialized, sp(3)));

    for i in 4..12i64 {
        let r = m.read(ptr(o, i), 1, sp(4));
        assert!(
            r.faults
                .iter()
                .any(|f| matches!(f, MemFault::Uninitialized { .. })),
            "byte {i} was cleared: {:#?}",
            r.faults
        );
    }
    // Either side untouched, and readable — the absence of a fault is the assertion, not
    // the value, since a stale value accompanies a fault.
    for i in [0i64, 3, 12, 15] {
        let r = m.read(ptr(o, i), 1, sp(5));
        assert!(r.faults.is_empty(), "byte {i}: {:#?}", r.faults);
        assert_eq!(r.value, Some(vec![0xAB]));
    }
}

/// **A refusal carries its faults.** `refuse` returning an empty vector survived, so an
/// inline-asm clobber of `const` or freed memory producing **no finding at all** would
/// have shipped green. Found by review.
#[test]
fn a_havoc_range_refusal_says_why() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let ro = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.write(ptr(ro, 0), b"abcdefg\0", sp(2));
    m.set_readonly(ro);
    let r = m.havoc_range_reporting(&mut a, ptr(ro, 0), 8, HavocFill::Symbolic, sp(3));
    assert_eq!(r.value, Some(0));
    assert!(
        r.faults.iter().any(|f| f.kind() == "write-to-readonly"),
        "a refusal names its reason: {:#?}",
        r.faults
    );

    let dead = m.alloc(ObjKind::Heap, 8, 8, sp(4));
    m.free(dead, sp(5));
    let r = m.havoc_range_reporting(&mut a, ptr(dead, 0), 8, HavocFill::Symbolic, sp(6));
    assert!(
        r.faults.iter().any(|f| f.kind() == "use-after-free"),
        "{:#?}",
        r.faults
    );
}

/// **An uninitialized havoc of a promoted object resets its array's `init` mask.**
///
/// The branch wave 267 could not reach, reached. `HavocFill::Uninitialized` has two halves — clear
/// the byte contents, and if the object is a `Repr::Array` reset the array's mask — and only the
/// first was tested. Forcing the mask to all-ones or skipping the branch survived every fixture.
///
/// **`promote_to_array` is called directly, and that is the point rather than a shortcut.** Wave
/// 267 spent itself trying to reach this state through an operation: `havoc_range` *refuses* a
/// promoted object outright, a sixteen-byte object's symbolic offset enumerates so nothing
/// promotes, and a sixty-four-byte `write_sym` still left `Repr::Bytes`. Promotion is a public
/// operation on the memory model and this is a unit test of that model, so the honest way to test
/// a state is to put the model in it. With this, the branch runs — instrumenting it prints
/// `Repr::Array` where every previous attempt printed `Bytes`.
///
/// # And the mask reset is still not independently observable
///
/// Reaching the branch is not the same as seeing what it does, and the two mutants against it
/// survive this fixture too. A read of a promoted object reports `SymbolicByte` — its contents are
/// an SMT array, so there is no concrete byte to hand back — and it reports that *identically*
/// whether the mask says initialized or not. Both faults are in `yields_unknown_value`, so the
/// engine discards the value either way and the outcome a caller sees is the same.
///
/// So the reset changes which *kind* is reported, not whether the value is trusted, and no read can
/// tell the two apart. The assertion below is therefore about the property that is real — the value
/// must not be believed — rather than about a fault kind the branch does not decide. §9 carries the
/// rest: making the kind observable would need the mask exposed, and that is an API question rather
/// than a missing fixture.
///
/// What the branch is for was a bug once, in the other direction: "promotion is one-way within a
/// state. Clearing `arr` here de-promoted a promoted object and discarded its array contents, so a
/// read after it answered from stale bytes." Without the mask reset the same shape returns —
/// contents cleared, mask still claiming every bit is initialized, and a read after an unmodelled
/// call answering confidently from memory the callee may have left in any state.
///
/// **Read through `read_sym`, because the concrete `read` cannot see the mask.** A byte-wise read of
/// a promoted object reports `SymbolicByte` — the contents live in an SMT array and there are no
/// concrete bytes to return — and it reports that identically before and after the havoc. The
/// symbolic path is the one that consults `arr.init`, so it is the only one that can observe this
/// branch at all.
#[test]
fn an_uninitialized_havoc_of_a_promoted_object_resets_its_init_mask() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 16, sp(2));
    m.promote_to_array(&mut a, o);

    // The control: promoted and fully written, nothing says *uninitialized*. Without it a fix that
    // reported everything after a promotion would satisfy the assertion below.
    //
    // It asks about the initialization faults rather than about silence, because a concrete `read`
    // of a promoted object legitimately reports `SymbolicByte`: the contents now live in an SMT
    // array and there are no concrete bytes to hand back. That is 021 contract 6 holding — the
    // *value and its initialization status* survive promotion, the representation does not — and
    // asserting `faults.is_empty()` here fails on correct behaviour, which is how the first version
    // of this test went red.
    let zero = a.bv(64, 0);
    let mut cx = AccessCtx::new();
    let r = m.read_sym(&mut cx, &mut a, o, zero, 1, sp(3));
    assert!(
        !r.faults.iter().any(|f| matches!(
            f,
            MemFault::Uninitialized { .. } | MemFault::MaybeUninitialized { .. }
        )),
        "the object is fully written, and promotion changes only how it is stored: {:#?}",
        r.faults
    );

    let h = m.havoc(&mut a, &[o], 0, HavocFill::Uninitialized, sp(4));
    assert!(!h.objects.is_empty(), "the havoc must reach the object");

    let r = m.read_sym(&mut cx, &mut a, o, zero, 1, sp(5));
    assert!(
        r.faults.iter().any(|f| f.yields_unknown_value()),
        "a callee with no model may have left this byte in any state, so the read must not hand \
         back a value the caller would believe: {:#?}",
        r.faults
    );
}

/// **An uninitialized havoc after a symbolic write still uninitializes the object.**
///
/// Named for what it does, which is less than it was written to do. `HavocFill::Uninitialized` has
/// two halves — clear the byte contents, and if the object was promoted to an array reset the
/// array's `init` mask — and mutation says only the first is tested: forcing the mask to all-ones,
/// or skipping the array branch entirely, survives every fixture including this one.
///
/// **The array branch is never reached with a promoted object, and that is measured.** Logging the
/// representation at the fill across the whole suite: eight calls, every one `Repr::Bytes`. Neither
/// `havoc_range` nor whole-object `havoc` gets there — the ranged form *refuses* a promoted object
/// outright, its own comment listing "promoted" among the refusals, and a symbolic write at
/// `i & 63` on a sixty-four-byte object does not promote either.
///
/// What the branch is for is real and was a bug once: "promotion is one-way within a state.
/// Clearing `arr` here de-promoted a promoted object and discarded its array contents, so a read
/// after it answered from stale bytes." Reaching it needs a shape nobody has found; §9 carries the
/// question rather than this test claiming to answer it.
#[test]
fn an_uninitialized_havoc_after_a_symbolic_write_uninitializes_the_object() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 64, sp(2));

    // Promote it: a store at an offset nothing can pin turns the object into an array.
    let i = a.var(Sort::BitVec(64), "i");
    let mask = a.bv(64, 63);
    let off = a.and(i, mask);
    let seven = a.bv(8, 7);
    let mut cx = AccessCtx::new();
    m.write_sym(&mut cx, &mut a, o, off, seven, sp(3));

    // **Whole-object `havoc`, not `havoc_range`.** The ranged form *refuses* a promoted object —
    // its own comment lists "promoted" among the refusals — so the array branch is unreachable
    // through it, and a fixture built on it exercises the `Bytes` path while looking like it
    // exercises this one. Instrumenting the branch is what showed that: eight calls reached it in
    // the whole suite and every one had `repr = Bytes`.
    let h = m.havoc(&mut a, &[o], 0, HavocFill::Uninitialized, sp(4));
    assert!(
        !h.objects.is_empty(),
        "the havoc must reach the object, or this tests nothing"
    );

    let r = m.read(ptr(o, 0), 1, sp(5));
    assert!(
        r.faults.iter().any(|f| matches!(
            f,
            MemFault::Uninitialized { .. } | MemFault::MaybeUninitialized { .. }
        )),
        "a callee with no model may have left this byte in any state: {:#?}",
        r.faults
    );
}

/// **A symbolic index wider than the array's is truncated, not handed to `store` as-is.**
///
/// A promoted object's array is indexed at `idx_bits`, which is 64 at its only assignment. An
/// index term can be any width the caller built — a `__int128` computation used as a subscript
/// arrives at 128 — and `fit` narrows it before the `store`.
///
/// **`fit`'s widening arm is covered and its narrowing arm was not** (waves 292 and 295). Every
/// fixture passed a 64-bit or narrower index, so `Ordering::Greater` never ran, and a mutant
/// returning the term unchanged passed all 190 tests. The two arms are not symmetric in how
/// easily they arise: a narrow index comes from an `unsigned char` subscript, which is
/// commonplace, and a wide one from 128-bit arithmetic, which is not — but "rare" is not "never",
/// and the failure mode is a `store` whose index width disagrees with its array.
///
/// This is also the fixture that justifies wave 295 replacing a hand-inlined copy of `fit` in
/// `write_at_symbolic_offset` with a call to it: three call sites, one adjustment, one test.
#[test]
fn a_symbolic_index_wider_than_the_arrays_is_narrowed() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.promote_to_array(&mut a, o);
    // **128 bits wide**, where the array indexes at 64.
    let off = a.var(Sort::BitVec(128), "wide");
    let val = a.bv(8, 0x3C);
    // No candidates: the empty-candidate path is the one that stores at the symbolic index
    // itself rather than enumerating offsets.
    let r = m.write_at_symbolic_offset(&mut a, o, off, &[], val, sp(2));
    assert!(
        r.faults.is_empty(),
        "a wide index is narrowed, not refused: {:#?}",
        r.faults
    );
    // And the narrow direction still works, which is what keeps the two arms honest about
    // being one decision.
    let narrow = a.var(Sort::BitVec(8), "narrow");
    let r = m.write_at_symbolic_offset(&mut a, o, narrow, &[], val, sp(3));
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
}

/// **An uninitialized havoc of a *promoted* object resets its array's init, not just its bytes.**
///
/// 021 §3 makes promotion one-way within a state, so the havoc cannot simply drop the array and
/// go back to bytes — the guard's own comment says clearing `arr` there "de-promoted a promoted
/// object and discarded its array contents, so a read after it answered from stale bytes".
/// Instead it overwrites the array's `init` with all-zero.
///
/// **Both directions of that guard survived wave 292's sweep.** The `Uninitialized` havoc is
/// tested — `an_uninitialized_havoc_makes_the_next_read_a_finding`, right above — but only on an
/// object in `Bytes` representation, which is the one shape where the promoted branch cannot run.
/// A promoted object havocked this way kept every init bit it had.
///
/// The observable is `init_bit_via` rather than a read's faults: 021 contract 6 makes a concrete
/// read of a promoted object report `SymbolicByte` whatever its init state, which is wave 268's
/// finding and would mask this entirely.
#[test]
fn an_uninitialized_havoc_of_a_promoted_object_resets_its_array_init() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    m.set(ptr(o, 0), 0xAB, 8, sp(2));
    m.promote_to_array(&mut a, o);
    let before: Vec<InitBit> = (0..8 * 8).map(|b| m.init_bit_via(&mut a, o, b)).collect();
    assert!(
        before.iter().all(|b| matches!(b, InitBit::Yes)),
        "every bit of a fully written object is initialized before the havoc"
    );

    m.havoc_object(&mut a, o, HavocFill::Uninitialized, sp(3));

    let after: Vec<InitBit> = (0..8 * 8).map(|b| m.init_bit_via(&mut a, o, b)).collect();
    assert!(
        after.iter().all(|b| matches!(b, InitBit::No)),
        "an uninitialized havoc leaves nothing initialized, promoted or not: {:?}",
        after.iter().take(8).collect::<Vec<_>>()
    );
    // **And it is still promoted.** The fix this guard records was that clearing the array sent
    // the object back to bytes and a later read answered from the stale ones.
    assert!(
        !m.is_bytes(o),
        "promotion is one-way within a state (021 §3); the havoc must not undo it"
    );
}

/// **A symbolic write to a freed object names the free, not the representation.**
///
/// `write_at_symbolic_offset` checks object state before anything else, so a use-after-free is
/// reported as such rather than falling through to "this byte is symbolic" — 023 §9's report a
/// person cannot act on.
///
/// **This fixture was written to reach a different line and does not** (wave 296). The target was
/// the early-out that propagates a *promotion* fault further down the same function, which wave
/// 292's sweep could not kill. It is unreachable: `promote_to_array` faults only via
/// `state_fault`, and this function calls `state_fault` at its head, so promotion can no longer
/// fail by the time it runs. Two guards, one of them subsumed — the shape wave 290 found in the
/// CIR verifier. The measurement is recorded at the early-out itself.
///
/// The fixture is kept because the property it *does* pin was also untested.
#[test]
fn a_symbolic_write_to_a_freed_object_reports_the_free_not_a_symbolic_byte() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.free(o, sp(2));
    let off = a.var(Sort::BitVec(64), "i");
    let val = a.bv(8, 0x11);
    let r = m.write_at_symbolic_offset(&mut a, o, off, &[], val, sp(3));
    assert!(
        !r.faults.is_empty(),
        "writing to a freed object is a fault: {:#?}",
        r.faults
    );
    assert!(
        !r.faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "the fault names the free, not the representation it never got: {:#?}",
        r.faults
    );
}

/// **`read_bits_term` agrees with `read_bits` wherever `read_bits` can answer.**
///
/// The term path exists for the case the concrete one cannot serve — a bitfield in memory
/// somebody else filled in, which on VPP is most structs. That case has no independent oracle:
/// the answer is a symbol, and a wrong bit range produces a symbol too, equally plausible and
/// silently wrong for every `p->flags` in the tree.
///
/// So it is pinned where an oracle does exist. Over concrete bytes the two APIs must agree
/// exactly, for every field offset and width across a byte boundary — which is what catches an
/// off-by-one in the extraction or a byte order read backwards, the two ways this can be wrong.
#[test]
fn the_term_bitfield_read_agrees_with_the_concrete_one() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    // An asymmetric pattern: 0x00/0xFF would pass a byte-order bug, and a palindrome would
    // pass a reversed extraction.
    m.write(ptr(o, 0), &[0x12, 0x34, 0x56, 0x78], sp(2));

    for lo_bit in 0u64..24 {
        for n_bits in 1u64..=9 {
            let concrete = m.read_bits(ptr(o, 0), lo_bit, n_bits, sp(3));
            let Some(want) = concrete.value else {
                panic!("the concrete API should answer over concrete bytes at {lo_bit}+{n_bits}");
            };
            let got = m
                .read_bits_term(&mut a, ptr(o, 0), lo_bit, n_bits, Endian::Little, sp(4))
                .unwrap_or_else(|| panic!("no term at {lo_bit}+{n_bits}"));
            let t = got.value.expect("a term");
            assert_eq!(
                a.width(t),
                n_bits as u32,
                "the field's own width, not the containing bytes' ({lo_bit}+{n_bits})"
            );
            assert_eq!(
                a.eval_ground(t).expect("a ground term").bits(),
                want,
                "term and concrete reads disagree at bit {lo_bit}, width {n_bits}"
            );
        }
    }
}

/// And the case it was built for: a symbolic byte gives a **value**, where the concrete API
/// gives a `SymbolicByte` fault and nothing to compute with.
#[test]
fn a_bitfield_of_a_symbolic_byte_reads_as_a_term_not_a_fault() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let x = a.var(Sort::BitVec(8), "x");
    m.write_sym_byte(ptr(o, 0), x, sp(2));

    let concrete = m.read_bits(ptr(o, 0), 1, 1, sp(3));
    assert!(
        concrete
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "the concrete API cannot answer, and says so: {:?}",
        concrete.faults
    );

    let got = m
        .read_bits_term(&mut a, ptr(o, 0), 1, 1, Endian::Little, sp(4))
        .expect("little-endian, one bit");
    assert!(
        !got.faults
            .iter()
            .any(|f| matches!(f, MemFault::SymbolicByte { .. })),
        "the term API can: {:?}",
        got.faults
    );
    let t = got.value.expect("a term");
    assert_eq!(a.width(t), 1);

    // It is bit 1 of `x` and not some other bit: `x = 0b10` makes it 1, `x = 0b01` makes it 0.
    for (v, want) in [(0b10u128, 1u128), (0b01, 0)] {
        let mut model = Model::new();
        model.set(a.var_id(x).unwrap(), BvConst::new(8, v));
        assert_eq!(
            a.eval(&model, t).expect("evaluable").bits(),
            want,
            "x = {v:#b}"
        );
    }
}
