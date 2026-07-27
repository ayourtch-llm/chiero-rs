//! The access API: faults **alongside** a value, not instead of one (021 §5).
//!
//! Covers **021 contracts 2, 7, 8, 9, 10, 11, 21** and §5's five-step ordering.
//!
//! 021 §5 says in bold that `Result<Term, MemFault>` cannot express the normal case, and
//! it is right. Three of the model's most important outcomes produce a value *and* a
//! finding:
//!
//! - an uninitialized read yields a fresh symbol **and** a finding (contract 7),
//! - a misaligned access is recorded **and** succeeds — x86-64 tolerates it and VPP
//!   relies on that in places,
//! - a may-OOB access reports **and** continues on the in-bounds branch, which is what
//!   keeps one early OOB from hiding everything downstream of it.
//!
//! A single access can also produce *several* faults at once — misaligned and partially
//! uninitialized is an ordinary combination — so `faults` is a vector, not an option.
//!
//! The other decision on display here: **objects are never deleted.** A freed or
//! out-of-scope object keeps its identity and extent; only its state changes. Deleting it
//! would make a dangling access indistinguishable from a wild pointer, and the model
//! could not say *which* object ended or *where*.

use chiero_mem::*;
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

/// **021 contract 7: an uninitialized read yields a value *and* a finding.**
///
/// This is the case `Result` cannot express, and getting it wrong in either direction is
/// expensive: returning no value forces the engine to invent one, and returning no
/// finding is the "silently read zero" failure that makes a symbolic executor
/// confidently wrong.
#[test]
fn an_uninitialized_read_yields_both_a_value_and_a_fault() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    let r = m.read(ptr(o, 0), 4, sp(20));
    assert!(
        r.value.is_some(),
        "the read must still produce bytes for the engine to carry on with"
    );
    assert_eq!(r.faults.len(), 1, "{:#?}", r.faults);
    assert!(matches!(r.faults[0], MemFault::Uninitialized { .. }));
}

/// A read of initialized memory produces a value and **no** faults — otherwise every test
/// here is satisfied by a model that reports a fault for everything.
#[test]
fn an_ordinary_read_produces_no_faults() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    assert!(m.write(ptr(o, 0), &[1, 2, 3, 4], sp(15)).faults.is_empty());
    let r = m.read(ptr(o, 0), 4, sp(20));
    assert_eq!(r.value.unwrap(), vec![1, 2, 3, 4]);
    assert!(r.faults.is_empty());
}

/// **Several faults from one access.** Misaligned *and* partially uninitialized is an
/// ordinary combination, and a model that reports only the first hides the other.
#[test]
fn one_access_can_produce_several_faults() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    m.write(ptr(o, 0), &[1], sp(15));
    // Offset 1 is misaligned for a 4-byte access, and bytes 1..4 were never written.
    let r = m.read(ptr(o, 1), 4, sp(20));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Misaligned { .. })),
        "{:#?}",
        r.faults
    );
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "{:#?}",
        r.faults
    );
}

/// **021 §5 step 3: misalignment is *recorded* but not fatal.** x86-64 tolerates
/// unaligned access and VPP relies on that in places, so a model that refused would report
/// findings on code that is correct on every target chiero supports.
#[test]
fn a_misaligned_access_is_recorded_and_still_succeeds() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    m.write(ptr(o, 0), &[1, 2, 3, 4, 5, 6, 7, 8], sp(15));
    let r = m.read(ptr(o, 1), 4, sp(20));
    assert_eq!(
        r.value.unwrap(),
        vec![2, 3, 4, 5],
        "the access must succeed; only the alignment is noted"
    );
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Misaligned { .. }))
    );
}

/// **021 contract 2: a concrete must-OOB access terminates.** It is out of bounds under
/// every model, so there is no in-bounds branch to continue on — "continue with the
/// in-bounds constraint" would continue a state whose path condition is unsatisfiable,
/// which 023 §3 treats as a chiero bug rather than a finding.
#[test]
fn a_concrete_out_of_bounds_access_reports_and_yields_no_value() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    let r = m.read(ptr(o, 61), 4, sp(20));
    assert!(
        r.value.is_none(),
        "there is no in-bounds branch to continue on"
    );
    assert_eq!(r.faults.len(), 1);
    assert!(matches!(r.faults[0], MemFault::OutOfBounds { .. }));
}

/// **021 §5 step 1: the state check comes first**, so a use-after-free never reads stale
/// bytes and never also reports "uninitialized" about memory it had no business touching.
#[test]
fn a_freed_object_is_checked_before_its_contents() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    m.free(o, sp(200));
    let r = m.read(ptr(o, 0), 4, sp(300));
    assert!(r.value.is_none(), "no stale bytes");
    assert_eq!(
        r.faults.len(),
        1,
        "one fault, not also 'uninitialized': {:#?}",
        r.faults
    );
    match r.faults[0] {
        MemFault::UseAfterFree { freed_at, at, .. } => {
            assert_eq!(freed_at, sp(200), "must name the free site");
            assert_eq!(at, sp(300), "must name the access site");
        }
        ref other => panic!("expected UseAfterFree, got {other:?}"),
    }
}

/// **021 contract 8, second half: `free(p); free(p)` is one double-free**, not a second
/// use-after-free.
#[test]
fn a_second_free_is_a_double_free() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    assert!(m.free(o, sp(200)).faults.is_empty());
    let r = m.free(o, sp(400));
    match &r.faults[..] {
        [MemFault::DoubleFree { freed_at, at, .. }] => {
            assert_eq!(*freed_at, sp(200));
            assert_eq!(*at, sp(400));
        }
        other => panic!("expected one DoubleFree, got {other:?}"),
    }
}

/// A use-after-free whose bytes happen to be intact is still a fault. The violation is
/// the lifetime, not the data — "it read the right value anyway" is the reasoning that
/// lets these bugs ship.
#[test]
fn use_after_free_fires_even_when_the_bytes_are_intact() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    m.write(ptr(o, 0), &[1, 2, 3, 4], sp(150));
    m.free(o, sp(200));
    assert!(matches!(
        m.read(ptr(o, 0), 4, sp(300)).faults[..],
        [MemFault::UseAfterFree { .. }]
    ));
}

/// **021 contract 10.** Stack objects are not deleted on scope exit; keeping them is what
/// makes this reportable at all, and what lets the fault name the scope that ended.
#[test]
fn use_after_scope_names_the_scope_that_ended() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    m.write(ptr(o, 0), &[7; 4], sp(20));
    m.exit_scope(o, sp(50));
    match m.read(ptr(o, 0), 4, sp(60)).faults[..] {
        [
            MemFault::UseAfterScope {
                scope_ended_at, at, ..
            },
        ] => {
            assert_eq!(scope_ended_at, sp(50));
            assert_eq!(at, sp(60));
        }
        ref other => panic!("expected UseAfterScope, got {other:?}"),
    }
}

/// Freed and out-of-scope are distinct faults. Collapsing them would report `free()` on a
/// stack variable, which is a different bug with a different fix.
#[test]
fn out_of_scope_and_freed_are_distinct_faults() {
    let mut m = Memory::new();
    let stack = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    let hp = m.alloc(ObjKind::Heap, 16, 8, sp(11));
    m.exit_scope(stack, sp(50));
    m.free(hp, sp(51));
    assert!(matches!(
        m.read(ptr(stack, 0), 4, sp(60)).faults[..],
        [MemFault::UseAfterScope { .. }]
    ));
    assert!(matches!(
        m.read(ptr(hp, 0), 4, sp(61)).faults[..],
        [MemFault::UseAfterFree { .. }]
    ));
}

/// **021 contract 21: a `readonly` object rejects writes with a finding and does not
/// alter the bytes.**
#[test]
fn a_readonly_object_rejects_writes_without_altering_bytes() {
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.write(ptr(g, 0), &[1, 2, 3, 4], sp(5));
    m.set_readonly(g);
    let r = m.write(ptr(g, 0), &[0xEE], sp(10));
    assert!(matches!(r.faults[..], [MemFault::ReadOnly { .. }]));
    assert_eq!(
        m.read(ptr(g, 0), 4, sp(15)).value.unwrap(),
        vec![1, 2, 3, 4]
    );
}

/// **021 contract 9: `realloc` preserves the retained prefix and dangles the old
/// pointer.** Modeled as allocate-new + copy + free-old, which is what makes `vec_resize`
/// analysable: any surviving copy of the old pointer is reported. That is a real and
/// frequent VPP bug class.
#[test]
fn realloc_preserves_the_prefix_and_dangles_the_old_pointer() {
    let mut m = Memory::new();
    let old = m.alloc(ObjKind::Heap, 16, 8, sp(100));
    m.write(ptr(old, 0), &[1, 2, 3, 4, 5, 6, 7, 8], sp(110));
    let new = m.realloc(old, 8, sp(200));
    assert_ne!(new, old);
    assert_eq!(
        m.read(ptr(new, 0), 8, sp(210)).value.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8],
        "the retained prefix survives byte for byte"
    );
    assert!(matches!(
        m.read(ptr(old, 0), 4, sp(300)).faults[..],
        [MemFault::UseAfterFree { .. }]
    ));
}

/// Growing leaves the new tail uninitialized rather than zeroed. `realloc` does not zero,
/// and a model that did would hide every read-of-uninitialized-tail bug.
#[test]
fn realloc_growing_leaves_the_new_tail_uninitialized() {
    let mut m = Memory::new();
    let old = m.alloc(ObjKind::Heap, 4, 8, sp(100));
    m.write(ptr(old, 0), &[9; 4], sp(110));
    let new = m.realloc(old, 16, sp(200));
    assert!(m.read(ptr(new, 0), 4, sp(210)).faults.is_empty());
    assert!(
        m.read(ptr(new, 4), 4, sp(220))
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. }))
    );
}

/// **021 contract 11: one unreferenced heap object is one leak; a stored one is none.**
#[test]
fn an_unreachable_heap_object_is_a_leak_and_a_stored_one_is_not() {
    let mut m = Memory::new();
    let leaked = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    let kept = m.alloc(ObjKind::Heap, 32, 8, sp(101));
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.set_root(g);
    m.point_at(g, kept);

    let leaks = m.leaks();
    assert_eq!(leaks.len(), 1, "{leaks:#?}");
    assert_eq!(leaks[0].obj, leaked);
    assert_eq!(leaks[0].allocated_at, sp(100));
}

/// Reachability is transitive, or every linked list reports every node but the head.
#[test]
fn reachability_through_a_chain_is_not_a_leak() {
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.set_root(g);
    let head = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    let mid = m.alloc(ObjKind::Heap, 32, 8, sp(101));
    let tail = m.alloc(ObjKind::Heap, 32, 8, sp(102));
    m.point_at(g, head);
    m.point_at(head, mid);
    m.point_at(mid, tail);
    assert!(m.leaks().is_empty(), "{:#?}", m.leaks());
}

/// A freed object is not a leak — reporting both would double-count every correct
/// malloc/free pair the analysis happened to see. A stack object going out of scope is
/// not a leak either, or every return reports every local.
#[test]
fn freed_and_out_of_scope_objects_are_not_leaks() {
    let mut m = Memory::new();
    let h = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    m.free(h, sp(200));
    let s = m.alloc(ObjKind::Stack, 16, 8, sp(10));
    m.exit_scope(s, sp(50));
    assert!(m.leaks().is_empty(), "{:#?}", m.leaks());
}

/// An unreachable cycle is leaked once per object, not once per edge.
#[test]
fn an_unreachable_cycle_is_leaked_once_per_object() {
    let mut m = Memory::new();
    let a = m.alloc(ObjKind::Heap, 32, 8, sp(100));
    let b = m.alloc(ObjKind::Heap, 32, 8, sp(101));
    m.point_at(a, b);
    m.point_at(b, a);
    assert_eq!(m.leaks().len(), 2, "{:#?}", m.leaks());
}

/// **A bitfield below the user pointer must be expressible.**
///
/// The crate's founding premise is `((vec_header_t *)v)[-1]`, and the bit API took an
/// *unsigned* offset while the byte API took a signed one — so the one access the module
/// doc-comment is written about could not be spelled at bit granularity. The old code
/// even reported `off: (lo_bit / 8) as i64`, acknowledging a signed domain it would not
/// accept.
#[test]
fn a_bitfield_below_the_user_pointer_is_expressible() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    // The user pointer is at offset 8; the header's flags are the low bits below it.
    let user = 8i64;
    let w = m.write_bits(ptr(o, user - 8), 0, 5, 0b10110, sp(10));
    assert!(w.faults.is_empty(), "{:#?}", w.faults);
    let r = m.read_bits(ptr(o, user - 8), 0, 5, sp(20));
    assert_eq!(r.value.unwrap(), 0b10110);
    assert!(r.faults.is_empty());

    // `user - 8` *evaluates* to 0, so the case above cannot tell a signed byte offset
    // apart from an implementation that ignores it — the same-answer trap that has now
    // caught me three times. A bit access at a genuinely non-zero byte offset can.
    let w = m.write_bits(ptr(o, 5), 3, 4, 0b1011, sp(30));
    assert!(w.faults.is_empty(), "{:#?}", w.faults);
    assert_eq!(m.read_bits(ptr(o, 5), 3, 4, sp(40)).value.unwrap(), 0b1011);
    assert!(
        m.read_bits(ptr(o, 0), 3, 4, sp(50))
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "byte 0's bits were not the ones written"
    );
}

/// **`realloc` preserves the value *and* the initialization status**, not just the bytes.
///
/// 021 contract 6 makes the point about symbolic offsets — the two paths must agree on
/// the `(value, initialization-status)` pair, not merely the value — and it applies here
/// too: copying bytes while marking them all initialized would silently launder an
/// uninitialized prefix into a clean one, hiding exactly the bug `vec_resize` analysis
/// exists to find.
#[test]
fn realloc_preserves_initialization_status_not_just_bytes() {
    let mut m = Memory::new();
    let old = m.alloc(ObjKind::Heap, 16, 8, sp(100));
    // Only bytes 4..8 are written; 0..4 are never touched.
    m.write(ptr(old, 4), &[1, 2, 3, 4], sp(110));
    let new = m.realloc(old, 16, sp(200));
    assert!(
        m.read(ptr(new, 0), 4, sp(210))
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. })),
        "the uninitialized prefix must stay uninitialized after realloc"
    );
    assert!(m.read(ptr(new, 4), 4, sp(220)).faults.is_empty());
}

/// A hostile or merely unconstrained allocation size must produce a fault, not kill the
/// process. `MemObject` eagerly allocated `size` bytes plus eight times that for the init
/// mask, so an unconstrained `clib_mem_alloc(n)` aborted the run — and an abort is not
/// something `catch_unwind` can contain.
#[test]
fn an_enormous_allocation_is_a_fault_not_an_abort() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, u64::MAX / 4, 8, sp(1));
    let r = m.read(ptr(o, 0), 4, sp(10));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::AllocationTooLarge { .. })),
        "{:#?}",
        r.faults
    );
}
