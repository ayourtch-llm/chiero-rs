//! Promotion preserves initialization — 021 contract 25.
//!
//! Covers: 021 contract 25.
//!
//! "Promotion `Bytes` → `Array` preserves initialization exactly: `No`→0, `Yes`→1,
//! `Cond(t)`→`ite(t,1,0)`, verified by comparing reads before and after promotion."
//!
//! Promotion is a *representation* change, and the whole tri-state exists because 021
//! §3.1 insists symbolic is not uninitialized. If promotion flattened `Cond` to either
//! definite state it would decide, silently, a question the program left open: to `Yes`
//! and a genuine uninitialized read stops being reported, to `No` and every guarded write
//! becomes a false one.

use chiero_mem::{Endian, InitBit, Memory, ObjKind, Pointer};
use chiero_solver::{Sort, TermArena};
use chiero_span::Span;

/// An object with all three initialization states in it: bytes 0..4 written outright,
/// bytes 4..8 written under a symbolic guard, and bytes 8..12 never touched.
fn three_states(m: &mut Memory, a: &mut TermArena) -> Pointer {
    let id = m.alloc(ObjKind::Heap, 12, 8, Span::DUMMY);
    let p = Pointer { base: id, off: 0 };
    let v = a.bv(32, 0xDEAD_BEEF);
    m.write_term(a, p, v, 4, Endian::Little, Span::DUMMY);

    // A write at a symbolic offset lands under a guard, which is what makes a byte
    // `Cond`: it is written on some paths and not others, and neither answer is a fact.
    let off = a.var(Sort::BitVec(64), "i");
    let byte = a.bv(8, 0x55);
    m.write_at_symbolic_offset(a, id, off, &[4, 5, 6, 7], byte, Span::DUMMY);
    p
}

/// **021 contract 25.** Every bit's state is the same before and after promotion.
#[test]
fn promotion_preserves_every_initialization_bit() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let p = three_states(&mut m, &mut a);

    let before: Vec<InitBit> = (0..12 * 8)
        .map(|b| m.init_bit_via(&mut a, p.base, b))
        .collect();
    // The fixture must actually contain all three, or this compares one state with itself.
    assert!(
        before.iter().any(|b| matches!(b, InitBit::Yes)),
        "some bits are definitely written"
    );
    assert!(
        before.iter().any(|b| matches!(b, InitBit::No)),
        "some are definitely not"
    );
    assert!(
        before.iter().any(|b| matches!(b, InitBit::Cond(_))),
        "and some are written only under a guard: {before:?}"
    );

    m.promote_to_array(&mut a, p.base);

    for (bit, was) in before.iter().enumerate() {
        let now = m.init_bit_via(&mut a, p.base, bit as u64);
        match (was, &now) {
            (InitBit::Yes, InitBit::Yes) | (InitBit::No, InitBit::No) => {}
            // The guard need not be the *same term* — the array form is
            // `ite(t, 1, 0) == 1`, which is a different expression with the same meaning —
            // so a `Cond` must stay a `Cond` and the two must agree on every assignment.
            (InitBit::Cond(t0), InitBit::Cond(t1)) => {
                let same = a.eq(*t0, *t1);
                let differs = a.not(same);
                let mut s = match chiero_solver::SmtLib::discover() {
                    Some(b) => chiero_solver::TieredSolver::with_backend(b),
                    None => chiero_solver::TieredSolver::new(),
                };
                use chiero_solver::Solver;
                assert!(
                    !matches!(
                        s.check(&mut a, &[differs]),
                        chiero_solver::CheckResult::Sat(_)
                    ),
                    "bit {bit}'s guard changed meaning across promotion"
                );
            }
            _ => panic!("bit {bit} was {was:?} and is now {now:?}"),
        }
    }
}

/// And the *values* survive too — an initialization mask that matched while the bytes
/// changed would be a strange kind of preservation.
#[test]
fn promotion_preserves_the_bytes_it_had() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let p = three_states(&mut m, &mut a);
    let before = m.read_term(&mut a, p, 4, Endian::Little, Span::DUMMY);
    m.promote_to_array(&mut a, p.base);
    let after = m.read_term(&mut a, p, 4, Endian::Little, Span::DUMMY);
    let (Some(b), Some(af)) = (before.value, after.value) else {
        panic!("both reads produce a value");
    };
    assert_eq!(
        a.eval_ground(b).map(|c| c.bits()),
        a.eval_ground(af).map(|c| c.bits()),
        "the definitely-written prefix reads the same after promotion"
    );
}
