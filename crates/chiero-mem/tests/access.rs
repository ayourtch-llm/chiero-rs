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
    let new = m.realloc(old, 8, sp(200)).value.unwrap();
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
    let new = m.realloc(old, 16, sp(200)).value.unwrap();
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
    let new = m.realloc(old, 16, sp(200)).value.unwrap();
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

// ---------------------------------------------------------------------------
// Wave 9, from the access-layer mutation review (33% escape). All probed first.
// ---------------------------------------------------------------------------

/// **`abs_bit` overflowed** — `off * 8 + lo_bit` unchecked. A wildly out-of-bounds
/// pointer wrapped into a *fault-free* read of byte 0 in release and panicked in debug.
/// This is the exact class the surrounding code claims to have eliminated two commits
/// ago; the byte API went to `i128` and the bit API did not follow.
#[test]
fn a_bit_offset_that_would_overflow_is_out_of_bounds() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 64, 8, sp(1));
    m.write(ptr(o, 0), &[0xAA; 8], sp(2));
    let r = m.read_bits(ptr(o, 1i64 << 61), 0, 8, sp(3));
    assert!(r.value.is_none(), "must not wrap into byte 0");
    assert!(
        matches!(r.faults[..], [MemFault::OutOfBounds { .. }]),
        "{:#?}",
        r.faults
    );
    let w = m.write_bits(ptr(o, 1i64 << 61), 0, 8, 0xFF, sp(4));
    assert!(matches!(w.faults[..], [MemFault::OutOfBounds { .. }]));
    assert_eq!(m.read(ptr(o, 0), 1, sp(5)).value.unwrap(), vec![0xAA]);
}

/// **The bit API must honour `readonly`.** There were two independent `readonly` fields —
/// one on `MemObject`, one on `Memory`'s entry — and `write_bits` consulted the one
/// nothing ever set. Contract 21 failed in *both* halves: no finding, and the bytes
/// changed.
#[test]
fn a_readonly_object_rejects_bit_writes_too() {
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 16, 8, sp(1));
    m.write(ptr(g, 0), &[0x11; 16], sp(2));
    m.set_readonly(g);
    let r = m.write_bits(ptr(g, 0), 0, 8, 0xEE, sp(3));
    assert!(
        matches!(r.faults[..], [MemFault::ReadOnly { .. }]),
        "{:#?}",
        r.faults
    );
    assert_eq!(m.read(ptr(g, 0), 1, sp(4)).value.unwrap(), vec![0x11]);
}

/// **`realloc` must carry the object's graph position across.** The new object inherited
/// neither incoming edges nor outgoing ones nor rootedness, so every `vec_resize` of a
/// live rooted vector — 021 §4's own motivating example — was reported leaked.
#[test]
fn realloc_does_not_leak_the_object_it_replaces() {
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.set_root(g);
    let v = m.alloc(ObjKind::Heap, 16, 8, sp(2));
    let inner = m.alloc(ObjKind::Heap, 16, 8, sp(3));
    m.point_at(g, v);
    m.point_at(v, inner);
    let nv = m.realloc(v, 32, sp(4)).value.unwrap();
    let leaks = m.leaks();
    assert!(
        leaks.is_empty(),
        "reallocating a rooted vector leaked it: {leaks:#?}"
    );
    assert_ne!(nv, v);
    // And a root that is itself reallocated stays a root.
    let r = m.alloc(ObjKind::Heap, 16, 8, sp(5));
    m.set_root(r);
    m.realloc(r, 32, sp(6));
    assert!(m.leaks().is_empty(), "{:#?}", m.leaks());
}

/// **Reachability runs through *live* objects only.** 021 §4 scopes leak roots to live
/// memory, and walking through a freed container hid the commonest leak shape there is:
/// free the head, forget the children.
#[test]
fn a_payload_reachable_only_through_a_freed_container_is_a_leak() {
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.set_root(g);
    let container = m.alloc(ObjKind::Heap, 16, 8, sp(2));
    let payload = m.alloc(ObjKind::Heap, 16, 8, sp(3));
    m.point_at(g, container);
    m.point_at(container, payload);
    m.free(container, sp(4));
    let leaks = m.leaks();
    assert_eq!(leaks.len(), 1, "{leaks:#?}");
    assert_eq!(leaks[0].obj, payload);
}

/// **`exit_scope` must not erase a `Freed` record.** A heap pointer normally lives in a
/// stack local, so an engine calling `exit_scope` at frame teardown was wiping every free
/// record in the state — and with it all double-free and use-after-free detection.
#[test]
fn leaving_scope_does_not_erase_a_free() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.free(o, sp(200));
    m.exit_scope(o, sp(300));
    match m.free(o, sp(400)).faults[..] {
        [MemFault::DoubleFree { freed_at, .. }] => {
            assert_eq!(freed_at, sp(200), "the original free site must survive")
        }
        ref other => panic!("expected DoubleFree, got {other:?}"),
    }
    assert!(matches!(
        m.read(ptr(o, 0), 4, sp(500)).faults[..],
        [MemFault::UseAfterFree { .. }]
    ));
}

/// 021 §4: globals are `Live` forever. `exit_scope` on one was making it out-of-scope, so
/// a later read of any global reported a use-after-scope.
#[test]
fn a_global_never_goes_out_of_scope() {
    let mut m = Memory::new();
    let g = m.alloc(ObjKind::Global, 8, 8, sp(1));
    m.write(ptr(g, 0), &[1, 2, 3, 4], sp(2));
    m.exit_scope(g, sp(3));
    assert!(m.read(ptr(g, 0), 4, sp(4)).faults.is_empty());
}

/// `realloc` of a freed object is a use-after-free, not a silent copy of dead bytes.
#[test]
fn realloc_of_a_freed_object_is_reported() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[7; 8], sp(2));
    m.free(o, sp(3));
    let r = m.realloc(o, 32, sp(4));
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::UseAfterFree { .. })),
        "{:#?}",
        r.faults
    );
}

/// **The alignment requirement comes from the access, not the object.**
///
/// It was `min(object_align, size)`, which is wrong in both directions: a 3-byte access
/// at offset 1 was reported misaligned with `want: 3` — and 3 is not an alignment — while
/// an 8-byte access at offset 1 of an align-1 object was *not* reported, so misalignment
/// could never be recorded inside a byte array. Every VPP packet buffer is a byte array.
#[test]
fn the_alignment_requirement_comes_from_the_access_size() {
    let mut m = Memory::new();
    let a1 = m.alloc(ObjKind::Heap, 32, 1, sp(1));
    m.write(ptr(a1, 0), &[1; 32], sp(2));
    assert!(
        m.read(ptr(a1, 1), 8, sp(3))
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Misaligned { want: 8, .. })),
        "an 8-byte access at offset 1 is misaligned even in a byte array: {:#?}",
        m.read(ptr(a1, 1), 8, sp(3)).faults
    );
    let a8 = m.alloc(ObjKind::Heap, 32, 8, sp(4));
    m.write(ptr(a8, 0), &[1; 32], sp(5));
    assert!(
        m.read(ptr(a8, 1), 3, sp(6)).faults.is_empty(),
        "a 3-byte access has no alignment requirement: {:#?}",
        m.read(ptr(a8, 1), 3, sp(6)).faults
    );
    assert!(m.read(ptr(a8, 4), 4, sp(7)).faults.is_empty());
    assert!(
        m.read(ptr(a8, 2), 4, sp(8))
            .faults
            .iter()
            .any(|f| matches!(f, MemFault::Misaligned { want: 4, .. }))
    );
}

/// **021 §1: an access through `UNBOUND` is a wild-pointer finding**, not a null
/// dereference. They are different bugs with different causes, and `MemFault` had no
/// variant for the second.
#[test]
fn an_access_through_unbound_is_a_wild_pointer() {
    let mut m = Memory::new();
    let r = m.read(ptr(ObjectId::UNBOUND, 0x1000), 4, sp(1));
    assert!(
        matches!(r.faults[..], [MemFault::WildPointer { .. }]),
        "{:#?}",
        r.faults
    );
}

/// A fault must say *where*. `NullDeref` hardcoded offset 0 while every other path
/// carried the real one — and the crate's own comment says a finding that cannot name
/// the access is not actionable.
#[test]
fn a_null_dereference_names_its_offset() {
    let mut m = Memory::new();
    match m.read(ptr(ObjectId::NULL, 16), 4, sp(1)).faults[..] {
        [MemFault::NullDeref { off, .. }] => assert_eq!(off, 16),
        ref other => panic!("expected NullDeref, got {other:?}"),
    }
}

/// `free(NULL)` is legal C and a no-op; models call it constantly, so reporting it is a
/// false positive on correct code. Freeing a *global* or a stack object is not.
#[test]
fn freeing_null_is_a_no_op_but_freeing_a_global_is_not() {
    let mut m = Memory::new();
    assert!(m.free(ObjectId::NULL, sp(1)).faults.is_empty());
    let g = m.alloc(ObjKind::Global, 8, 8, sp(2));
    assert!(
        !m.free(g, sp(3)).faults.is_empty(),
        "free() of a global is a real bug"
    );
    let s = m.alloc(ObjKind::Stack, 8, 8, sp(4));
    assert!(
        !m.free(s, sp(5)).faults.is_empty(),
        "free() of a stack object is too"
    );
}

/// **`read_bits` must return a value *and* a fault** — the very thing this API exists to
/// do. It returned a fault instead, while `read` of the same memory returned both.
#[test]
fn an_uninitialized_bit_read_also_yields_a_value() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let r = m.read_bits(ptr(o, 0), 0, 8, sp(2));
    assert!(
        r.value.is_some(),
        "the bit API owes a value like the byte API does"
    );
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, MemFault::Uninitialized { .. }))
    );
}

/// The bit API runs the same five steps as the byte API. Both the state check and the
/// alignment check were skipped, so a use-after-free through a bitfield read silently
/// returned stale bytes.
#[test]
fn the_bit_api_runs_the_same_checks_as_the_byte_api() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    m.write(ptr(o, 0), &[0xFF; 16], sp(2));
    m.free(o, sp(3));
    let r = m.read_bits(ptr(o, 0), 0, 8, sp(4));
    assert!(
        r.value.is_none(),
        "no stale bytes through the bit API either"
    );
    assert!(
        matches!(r.faults[..], [MemFault::UseAfterFree { .. }]),
        "{:#?}",
        r.faults
    );
}

/// **021 §5 / contract 26: `read` is `&mut self` because it *memoizes*.**
///
/// Two reads of one never-written byte must yield one term and one finding. Without the
/// memo, 020 contract 10's "a non-volatile load repeated yields the same value" is false
/// over uninitialized memory, and `x == x` becomes satisfiably false. This is the stated
/// justification for the signature, and it was not implemented.
#[test]
fn two_reads_of_one_uninitialized_byte_give_one_finding_and_one_value() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Stack, 16, 8, sp(1));
    let first = m.read(ptr(o, 0), 4, sp(2));
    assert_eq!(first.faults.len(), 1);
    let second = m.read(ptr(o, 0), 4, sp(3));
    assert!(
        second.faults.is_empty(),
        "the fresh symbol is memoized, so the second read is not a new finding: {:#?}",
        second.faults
    );
    assert_eq!(
        first.value, second.value,
        "and it must be the same value, or x == x is satisfiably false"
    );
}

/// **The byte↔bit scale, pinned across the two APIs.**
///
/// The previous attempt at this test wrote and read through the *same* bit path, so both
/// shared any wrong multiplier — changing `off * 8` to `off * 4` survived it. Writing
/// through the byte API and reading through the bit API cannot agree unless the scale is
/// right. I wrote a comment about this trap and then landed another instance of it.
#[test]
fn the_byte_to_bit_scale_agrees_across_both_apis() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, sp(1));
    let bytes: [u8; 8] = [0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87];
    m.write(ptr(o, 0), &bytes, sp(2));
    for (b, want) in bytes.iter().enumerate() {
        assert_eq!(
            m.read_bits(ptr(o, b as i64), 0, 8, sp(3)).value.unwrap(),
            *want as u128,
            "byte {b} disagrees between the byte and bit APIs"
        );
    }
}

/// `abs_bit` returns an `Option` solely to reject a negative byte offset, and no test
/// ever passed one — the only edge the guard exists for. A negative offset is a real
/// pointer (the vector header) but not a valid *object-relative bit index*, so it is an
/// out-of-bounds fault rather than a wrap.
#[test]
fn a_negative_byte_offset_in_the_bit_api_is_out_of_bounds() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, sp(1));
    let r = m.read_bits(ptr(o, -4), 0, 8, sp(2));
    assert!(
        matches!(r.faults[..], [MemFault::OutOfBounds { .. }]),
        "{:#?}",
        r.faults
    );
}
