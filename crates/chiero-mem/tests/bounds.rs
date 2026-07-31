//! Symbolic bounds checking and `AccessCtx` (021 §5 step 2).
//!
//! Covers **021 contract 2's symbolic half**.
//!
//! Everything so far has taken a concrete offset, where "in bounds" is a fact. With a
//! symbolic offset it is a *question for the solver*, and §5 step 2 says exactly what to
//! do with each of the three answers:
//!
//! - **Definitely in bounds** — the out-of-bounds condition is unsatisfiable under the
//!   path condition. No finding.
//! - **May be out of bounds** — both branches are satisfiable. One finding **with a
//!   concrete witness**, and execution *continues on the in-bounds branch* with the
//!   in-bounds constraint added. Continuing is what keeps one early OOB from hiding
//!   everything downstream of it; killing the state would make the first symbolic index
//!   in a function the last thing chiero ever says about it.
//! - **Definitely out of bounds** — the in-bounds condition is unsatisfiable. One
//!   finding, and the state **terminates**: there is no in-bounds branch to continue on,
//!   and continuing would carry a path condition that is itself unsatisfiable, which
//!   023 §3 treats as a chiero bug rather than a finding.
//!
//! `AccessCtx` exists because none of that belongs to `Memory`. Bounds checking *adds a
//! constraint to the path condition*, which is the engine's state, not the heap's.

use chiero_mem::*;
use chiero_solver::{Sort, TermArena};
use chiero_span::{BytePos, ExpnCtx, Span};

fn sp(lo: u32) -> Span {
    Span {
        lo: BytePos(lo),
        hi: BytePos(lo + 4),
        ctx: ExpnCtx(0),
    }
}

/// An index the path condition already pins below the object's size is definitely in
/// bounds, and produces no finding at all.
#[test]
fn a_provably_in_bounds_symbolic_offset_is_not_reported() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(Pointer { base: o, off: 0 }, 0, 64, sp(2));

    let i = a.var(Sort::BitVec(64), "i");
    let ten = a.bv(64, 10);
    let bound = a.ult(i, ten);
    let mut cx = AccessCtx::new();
    cx.assume(bound);

    let r = m.read_sym(&mut cx, &mut a, o, i, 4, sp(3));
    assert!(
        r.faults.is_empty(),
        "i < 10 in a 64-byte object: {:#?}",
        r.faults
    );
    assert_eq!(
        cx.path().len(),
        1,
        "nothing was added to a fact already known"
    );
}

/// **The may-OOB case: report, then continue on the in-bounds branch.**
///
/// The finding must carry a concrete witness — an offset a reader can plug in — because
/// "some value of `i` is out of bounds" is not a bug report anyone can act on.
#[test]
fn a_may_be_out_of_bounds_access_reports_with_a_witness_and_continues() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(Pointer { base: o, off: 0 }, 0, 64, sp(2));

    // Unconstrained: `i` may be in bounds and may not.
    let i = a.var(Sort::BitVec(64), "i");
    let mut cx = AccessCtx::new();

    let r = m.read_sym(&mut cx, &mut a, o, i, 4, sp(3));
    match r.faults.as_slice() {
        [MemFault::OutOfBoundsMaybe { witness, .. }] => {
            assert!(
                *witness < 0 || *witness + 4 > 64,
                "the witness must actually be out of bounds, got {witness}"
            );
        }
        other => panic!("expected one may-OOB finding, got {other:#?}"),
    }
    assert!(
        r.value.is_some(),
        "execution continues on the in-bounds branch, so there is a value"
    );
    assert_eq!(
        cx.path().len(),
        1,
        "the in-bounds constraint is added to the path condition"
    );
    // Having added it, the same access is now provably in bounds and silent.
    let again = m.read_sym(&mut cx, &mut a, o, i, 4, sp(4));
    assert!(
        again.faults.is_empty(),
        "the constraint is what makes the continuation sound: {:#?}",
        again.faults
    );
}

/// **The must-OOB case terminates.** There is no in-bounds branch to continue on, and
/// continuing would carry an unsatisfiable path condition.
#[test]
fn a_provably_out_of_bounds_symbolic_offset_terminates() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));

    let i = a.var(Sort::BitVec(64), "i");
    let hundred = a.bv(64, 100);
    let past = a.ult(hundred, i);
    let mut cx = AccessCtx::new();
    cx.assume(past);

    let r = m.read_sym(&mut cx, &mut a, o, i, 4, sp(2));
    assert!(r.value.is_none(), "no in-bounds branch exists");
    assert!(
        matches!(r.faults[..], [MemFault::OutOfBounds { .. }]),
        "a must-OOB is definite, not a maybe: {:#?}",
        r.faults
    );
    assert_eq!(cx.path().len(), 1, "nothing is added to a dead path");
}

/// A symbolic *write* runs the same three-way decision. Without this the checking is
/// one-directional and every symbolic store is unchecked, which is the more dangerous
/// half — a wild write corrupts state the analysis then reasons about.
#[test]
fn a_symbolic_write_is_bounds_checked_the_same_way() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    let i = a.var(Sort::BitVec(64), "i");
    let val = a.bv(8, 7);
    let mut cx = AccessCtx::new();

    let r = m.write_sym(&mut cx, &mut a, o, i, val, sp(2));
    assert!(
        matches!(r.faults[..], [MemFault::OutOfBoundsMaybe { .. }]),
        "{:#?}",
        r.faults
    );
    assert_eq!(cx.path().len(), 1);
}

/// The in-bounds constraint is a *conjunction*, not a replacement: an access that adds
/// one must not discard what the path already knew. A context that overwrote its path
/// condition would silently widen every state it touched.
#[test]
fn the_in_bounds_constraint_is_added_not_substituted() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    let i = a.var(Sort::BitVec(64), "i");
    let j = a.var(Sort::BitVec(64), "j");
    let two = a.bv(64, 2);
    let unrelated = a.ult(j, two);
    let mut cx = AccessCtx::new();
    cx.assume(unrelated);

    m.read_sym(&mut cx, &mut a, o, i, 4, sp(2));
    assert_eq!(cx.path().len(), 2, "the earlier assumption must survive");
    assert!(cx.path().contains(&unrelated));
}

/// The state check still comes first (021 §5 step 1): a symbolic access into freed
/// memory is a use-after-free, not a bounds question, and must not consult the solver at
/// all — the object's contents are not the issue.
#[test]
fn a_symbolic_access_into_freed_memory_is_a_use_after_free() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.free(o, sp(2));
    let i = a.var(Sort::BitVec(64), "i");
    let mut cx = AccessCtx::new();
    let r = m.read_sym(&mut cx, &mut a, o, i, 4, sp(3));
    assert!(
        matches!(r.faults[..], [MemFault::UseAfterFree { .. }]),
        "{:#?}",
        r.faults
    );
    assert!(
        cx.path().is_empty(),
        "a dead object raises no bounds constraint"
    );
}

/// A concrete offset expressed as a symbolic term is decided without ambiguity, so
/// folding does not change the answer. Otherwise the symbolic path and the concrete path
/// could disagree about the same access, which is the disagreement contract 6 warns
/// about in a different guise.
#[test]
fn a_constant_offset_term_agrees_with_the_concrete_path() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(Pointer { base: o, off: 0 }, 0, 64, sp(2));
    let mut cx = AccessCtx::new();

    let inside = a.bv(64, 8);
    assert!(
        m.read_sym(&mut cx, &mut a, o, inside, 4, sp(3))
            .faults
            .is_empty()
    );

    let outside = a.bv(64, 62);
    let r = m.read_sym(&mut cx, &mut a, o, outside, 4, sp(4));
    assert!(
        matches!(r.faults[..], [MemFault::OutOfBounds { .. }]),
        "62 + 4 > 64 is definite, not a maybe: {:#?}",
        r.faults
    );
}

/// **When tier 1 cannot decide, nothing is claimed and nothing is assumed.**
///
/// `solver-lite` is deliberately incomplete (022 §3), so the bounds question comes back
/// `Unknown`. Adding the in-bounds constraint anyway would prune the very path escalation
/// exists to explore — chiero would assume the access safe on the strength of an answer the
/// solver never gave. Reporting a finding anyway would invent one. `Unknown` is its own
/// outcome, and neither.
///
/// **The offset is a quotient, not a product.** It was `i * j` until wave 155 gave tier 1 a
/// bounded candidate search: that search moves every unconstrained variable together, and
/// `i = j = 8` puts a product past a 64-byte object, so the question stopped being
/// undecidable and the test started reporting a genuine `OutOfBoundsMaybe`. `i / j` is
/// still outside the fragment and the diagonal cannot exhibit an escaping value either —
/// every candidate it proposes has `i == j`, so the quotient is 1. The offset can still be
/// out of bounds in truth (large `i`, `j = 1`), which is what keeps this a question tier 1
/// *declines* rather than one it answers.
#[test]
fn an_undecidable_bounds_question_neither_claims_nor_assumes() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(Pointer { base: o, off: 0 }, 0, 64, sp(2));

    let i = a.var(Sort::BitVec(64), "i");
    let j = a.var(Sort::BitVec(64), "j");
    let off = a.urem(i, j);

    let mut cx = AccessCtx::new();
    let r = m.read_sym(&mut cx, &mut a, o, off, 4, sp(3));
    assert!(
        r.faults.is_empty(),
        "an undecided question is not a finding: {:#?}",
        r.faults
    );
    assert!(
        cx.path().is_empty(),
        "and it is not licence to assume the access safe: {:?}",
        cx.path()
    );
}

/// **The witness must actually be used.**
///
/// `bounds_decision` computed a concrete in-bounds offset and every branch then proceeded
/// at a hardcoded 0, so a symbolic read whose path condition pinned `i == 4` returned
/// byte 0 — silently, with no fault and no approximation marker. The offset was
/// bounds-checked and then thrown away, which is worse than not checking it: the answer
/// looks authoritative.
#[test]
fn a_symbolic_read_proceeds_at_the_offset_the_path_condition_implies() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    for i in 0..64i64 {
        m.write(Pointer { base: o, off: i }, &[i as u8 + 1], sp(2));
    }
    let i = a.var(Sort::BitVec(64), "i");
    let four = a.bv(64, 4);
    let pinned = a.eq(i, four);
    let mut cx = AccessCtx::new();
    cx.assume(pinned);

    let r = m.read_sym(&mut cx, &mut a, o, i, 1, sp(3));
    assert!(
        r.faults.is_empty(),
        "i == 4 is in a 64-byte object: {:#?}",
        r.faults
    );
    assert_eq!(
        r.value.unwrap(),
        vec![5],
        "byte 4 holds 5; returning byte 0 would be a confident wrong answer"
    );
}

/// A symbolic write lands where the path condition says, for the same reason — and the
/// dangerous direction, since a write to the wrong byte corrupts state the analysis then
/// reasons about.
#[test]
fn a_symbolic_write_lands_at_the_implied_offset() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(Pointer { base: o, off: 0 }, 0xEE, 64, sp(2));
    let i = a.var(Sort::BitVec(64), "i");
    let seven = a.bv(64, 7);
    let pinned = a.eq(i, seven);
    let mut cx = AccessCtx::new();
    cx.assume(pinned);

    let val = a.bv(8, 0x5A);
    assert!(
        m.write_sym(&mut cx, &mut a, o, i, val, sp(3))
            .faults
            .is_empty()
    );
    let t = m.read_term(
        &mut a,
        Pointer { base: o, off: 7 },
        1,
        Endian::Little,
        sp(4),
    );
    assert_eq!(a.eval_ground(t.value.unwrap()).unwrap().bits(), 0x5A);
    // And byte 0 was not touched.
    let z = m.read_term(
        &mut a,
        Pointer { base: o, off: 0 },
        1,
        Endian::Little,
        sp(5),
    );
    assert_eq!(a.eval_ground(z.value.unwrap()).unwrap().bits(), 0xEE);
}

/// A **concrete** offset must be honoured even when the path condition is undecidable.
///
/// The witness is normally read out of a model, and tier 1 returns none when the path
/// contains something outside its fragment — so without deciding a ground offset
/// directly, a constant index inside a function with one multiplication in its path
/// silently became byte 0.
#[test]
fn a_concrete_offset_is_honoured_under_an_undecidable_path_condition() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    for i in 0..64i64 {
        m.write(Pointer { base: o, off: i }, &[i as u8 + 1], sp(2));
    }
    // Something tier 1 cannot decide, in the path condition rather than the offset.
    let x = a.var(Sort::BitVec(64), "x");
    let y = a.var(Sort::BitVec(64), "y");
    let p = a.mul(x, y);
    let twelve = a.bv(64, 12);
    let hard = a.eq(p, twelve);
    let mut cx = AccessCtx::new();
    cx.assume(hard);

    let off = a.bv(64, 9);
    let r = m.read_sym(&mut cx, &mut a, o, off, 1, sp(3));
    assert_eq!(
        r.value.unwrap(),
        vec![10],
        "byte 9 holds 10 regardless of what the solver can say about the path"
    );
}

/// **An access *below* an object is out of bounds, and nothing tested that.**
///
/// The concrete check is `off < 0 || end > size`, and mutation says only the second half was
/// observed: deleting `off < 0` — so that a read at offset `-4` is treated as ordinary — survived
/// the whole suite. Every bounds fixture reached past the *end* of an object and none reached
/// before its start.
///
/// That is the same empty cell waves 261–263 found three times in the arithmetic checks: a two-part
/// condition tested in one direction. ASan calls it by its own name — `Memory access at offset 60
/// underflows this variable` — so the direction is worth distinguishing in a report as well as in a
/// check.
///
/// **`AddressSpace::in_bounds` carried the same condition and is deleted rather than asserted.**
/// Three mutants against it survived — dropping its low-end test, moving its boundary by one, and
/// making an unknown-size object answer *true* — and the reason is that nothing called it. A second
/// predicate deciding bounds is the drift risk waves 256 and 257 removed from the cast-kind
/// decisions, and this one had already drifted out of use entirely.
#[test]
fn an_access_below_an_object_is_out_of_bounds() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.set(Pointer { base: o, off: 0 }, 0, 64, sp(2));

    for (what, off) in [("one byte before", -1i64), ("one element before", -4)] {
        let r = m.read(Pointer { base: o, off }, 4, sp(3));
        assert!(
            r.faults
                .iter()
                .any(|f| matches!(f, MemFault::OutOfBounds { .. })),
            "`{what}` starts before the object and is as out of bounds as running off the end: \
             {:#?}",
            r.faults
        );
    }

    // The controls: the first and last legal accesses, which a fix that simply rejected more
    // would break. The last one ends *exactly* at the object's end, which is the off-by-one
    // mutation's cell.
    for (what, off) in [("the first byte", 0i64), ("the last four bytes", 60)] {
        let r = m.read(Pointer { base: o, off }, 4, sp(3));
        assert!(
            r.faults.is_empty(),
            "`{what}` is inside a 64-byte object: {:#?}",
            r.faults
        );
    }
}

/// **A symbolic write past a *promoted* object's end is out of bounds.**
///
/// `write_term` bounds-checks byte by byte, but only in the branch it takes for an object whose
/// representation is `Array` — a promoted one. Every other store path has its own check, and
/// those are covered; this branch's was not.
///
/// **Both directions of the guard were unfalsifiable** (wave 292's sweep of `chiero-mem`):
/// forcing `off < 0 || off as u64 >= obj_size` to `false` reported nothing and forcing it to
/// `true` reported on every byte of every legal write, and 189 tests in this crate plus the
/// engine's passed either way. Every existing fixture writes symbolically *inside* the object,
/// because that is what a correct program does — which is exactly why the out-of-bounds arm had
/// nothing to observe it.
///
/// The negative half is the reason the `true` direction survived and is asserted here too: a
/// legal symbolic write to a promoted object must stay silent, or the guard could be deleted
/// outright and replaced with "always fault".
#[test]
fn a_symbolic_write_past_a_promoted_objects_end_is_out_of_bounds() {
    let mut a = TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    // Promote it: a write at a symbolic offset is what changes the representation to `Array`,
    // and the branch under test only runs for that representation.
    // **`promote_to_array` explicitly.** A symbolic-offset write does not change the
    // representation on its own — the first version of this fixture assumed it did, and the test
    // passed while both directions of the guard were mutated, because the branch was never
    // entered at all.
    m.promote_to_array(&mut a, o);

    let v = a.bv(32, 0x1234_5678);
    // Four bytes starting at offset 6 of an 8-byte object: bytes 6 and 7 are inside, 8 and 9
    // are not. A whole-access check that only looked at the start offset would miss this.
    let r = m.write_term(
        &mut a,
        Pointer { base: o, off: 6 },
        v,
        4,
        Endian::Little,
        sp(3),
    );
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::OutOfBounds { .. })),
        "a four-byte symbolic write at offset 6 of an 8-byte object runs off the end: {:#?}",
        r.faults
    );

    // The control: the same write entirely inside the object must be silent.
    let r = m.write_term(
        &mut a,
        Pointer { base: o, off: 4 },
        v,
        4,
        Endian::Little,
        sp(4),
    );
    assert!(
        r.faults.is_empty(),
        "the last four bytes of an eight-byte object are in bounds: {:#?}",
        r.faults
    );
}
