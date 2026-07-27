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
}

/// **The index width must match the offset's**, not a hardcoded 64. A `store` whose index
/// is a different width from the array's is a sort error the backend rejects, and the
/// arena's own width assertions would not catch it because arrays carry no scalar width.
#[test]
fn the_array_index_width_follows_the_offset() {
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
    let r = m.read_term_at(&mut a, o, off, &[], sp(3));
    assert!(r.value.is_some(), "{:#?}", r.faults);
}
