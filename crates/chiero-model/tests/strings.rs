//! `strlen`'s edge cases — 024 §4, at the boundaries.
//!
//! Covers: 024 contract 7.
//!
//! Three guards in `strlen_symbolic` that review found unpinned: a **negative offset**
//! (the concrete `strlen` carries a comment about `max(0)` "licensing a walk that started
//! before the object" — the symbolic one had the same guard and nothing holding it), an
//! object with **no readable bytes**, and a byte that **faulted** being used as a
//! constrainable term anyway.

use chiero_mem::{Memory, ObjKind, Pointer};
use chiero_model::{ModelCtx, ModelOutcome, StringPolicy, models};
use chiero_solver::TermArena;
use chiero_span::Span;

fn ctx<'a>(m: &'a mut Memory, a: &'a mut TermArena) -> ModelCtx<'a> {
    ModelCtx::new(m, a, Span::DUMMY, chiero_mem::Endian::Little)
}

/// A pointer *before* its object has no room, whatever the object's size. Measuring from
/// `max(0)` licenses a walk that starts outside the object — the mistake the concrete
/// `strlen` documents and the symbolic one repeated.
#[test]
fn a_negative_offset_is_a_finding_not_a_walk() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let id = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let out = models::strlen_symbolic(
        &mut cx,
        Pointer { base: id, off: -4 },
        StringPolicy::default(),
    );
    match out {
        ModelOutcome::Finding(msg) => assert!(
            msg.contains("before the object"),
            "and it says which way it points: {msg}"
        ),
        other => panic!("a walk from outside the object is not an answer: {other:?}"),
    }
}

/// `i64::MIN` is a real offset a program can compute, and the message must not panic
/// producing it. `-p.off` overflows there.
#[test]
fn the_most_negative_offset_does_not_panic() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let id = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let out = models::strlen_symbolic(
        &mut cx,
        Pointer {
            base: id,
            off: i64::MIN,
        },
        StringPolicy::default(),
    );
    assert!(matches!(out, ModelOutcome::Finding(_)), "{out:?}");
}

/// A pointer at the very end of an object has nothing to read, and §4's own rule is that
/// "an unterminated-string finding requires having looked" — so this is a finding about
/// the pointer, not an out-of-bounds about the string.
#[test]
fn no_readable_bytes_is_reported_as_such() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let id = m.alloc(ObjKind::Heap, 4, 1, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let out = models::strlen_symbolic(
        &mut cx,
        Pointer { base: id, off: 4 },
        StringPolicy::default(),
    );
    match out {
        ModelOutcome::Finding(msg) => assert!(
            !msg.contains("unterminated"),
            "nothing was looked at, so nothing overran: {msg}"
        ),
        other => panic!("expected a finding: {other:?}"),
    }
}

/// **A byte that came back with a fault is not a byte to constrain.** An *uninitialized*
/// read is the case that distinguishes this: it yields a value — the materialized symbol —
/// *and* a fault, so a guard built from it would put a path condition on a byte the
/// program never wrote and the read never legitimately obtained.
///
/// The first version of this test used a *freed* object, where the read yields no value at
/// all, so the fault check was never the deciding factor and dropping it changed nothing.
#[test]
fn a_byte_that_faulted_is_not_used_as_a_guard() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    // Uninitialized heap: `malloc` without a write, which is exactly what `strlen` over a
    // fresh buffer meets.
    let id = m.alloc(ObjKind::Heap, 4, 1, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let out = models::strlen_symbolic(
        &mut cx,
        Pointer { base: id, off: 0 },
        StringPolicy::default(),
    );
    match out {
        ModelOutcome::Finding(msg) => assert!(
            msg.contains("could not be read") || msg.contains("no readable"),
            "{msg}"
        ),
        other => panic!("a freed object is not scannable: {other:?}"),
    }
}
