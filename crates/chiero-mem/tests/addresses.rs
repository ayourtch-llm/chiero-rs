//! Concrete address assignment and pointer provenance (021 §7, §7.1).
//!
//! Covers **021 contracts 12, 12b, 12c, 13, 14, 15**.
//!
//! §7.1 is the load-bearing part and the reason this file exists. `PtrToInt` yields
//! `addr + off`, and the naive inverse — have `IntToPtr` search address ranges — is wrong
//! in *both* directions:
//!
//! - It turns a real bug into a legitimate access. An OOB pointer far enough past object
//!   A lands inside unrelated object B, and the out-of-bounds write becomes a silent,
//!   legal-looking write to the wrong object. Guard gaps only bound OOB distances smaller
//!   than the gap.
//! - It reports a bug on conforming code. For a page-aligned object of size exactly one
//!   gap, the legal one-past-the-end pointer lands in the guard gap, misses every range,
//!   and becomes a wild-pointer finding.
//!
//! So provenance is recorded in the term and consulted first; range search is the
//! fallback, not the mechanism. Contract 12 alone only tests the easy case — 12b is the
//! contract that actually distinguishes the two designs.

use chiero_mem::*;
use chiero_span::Span;

fn space() -> AddressSpace {
    AddressSpace::new()
}

/// 021 §7: every region has its own base, and objects are separated by a guard gap.
#[test]
fn objects_are_placed_in_their_region_with_a_guard_gap() {
    let mut s = space();
    let a = s.alloc(ObjKind::Heap, 100, 8, Span::DUMMY);
    let b = s.alloc(ObjKind::Heap, 100, 8, Span::DUMMY);
    let g = s.alloc(ObjKind::Global, 64, 8, Span::DUMMY);

    assert!(s.addr_of(a).unwrap() >= 0x0000_2000_0000, "heap region");
    assert!(s.addr_of(g).unwrap() >= 0x0000_1000_0000, "global region");
    assert!(
        s.addr_of(g).unwrap() < 0x0000_2000_0000,
        "regions must not run into each other"
    );
    // **The literal, not the constant.** `>= GUARD_GAP` is satisfied by setting
    // `GUARD_GAP` to 0 — three assertions here proved only that the code and the test
    // read the same constant. What the gap is *for* is that an overrun smaller than a
    // page cannot reach a neighbour, so a page is what gets asserted. Found by review
    // as G1.
    assert_eq!(
        GUARD_GAP, 4096,
        "a page: an overrun smaller than one cannot reach across"
    );
    assert!(
        s.addr_of(b).unwrap() - (s.addr_of(a).unwrap() + 100) >= 4096,
        "objects must be separated by at least one page"
    );
}

/// **021 contract 14 — the property that actually matters.**
///
/// Separation alone is satisfied by spacing objects a megabyte apart. The real
/// requirement is that an OOB pointer just past the gap does not resolve to the
/// neighbour, which is a statement about *resolution*, not spacing.
#[test]
fn no_two_objects_overlap_across_random_allocation_sequences() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..2000 {
        let mut s = space();
        let mut placed: Vec<(u64, u64)> = Vec::new();
        for _ in 0..(rng() % 8 + 2) {
            let size = rng() % 8192 + 1;
            let kind = match rng() % 3 {
                0 => ObjKind::Heap,
                1 => ObjKind::Stack,
                _ => ObjKind::Global,
            };
            let id = s.alloc(kind, size, 8, Span::DUMMY);
            placed.push((s.addr_of(id).unwrap(), size));
        }
        placed.sort_unstable();
        for w in placed.windows(2) {
            let (a_addr, a_size) = w[0];
            let (b_addr, _) = w[1];
            assert!(
                b_addr >= a_addr + a_size + 4096,
                "objects at {a_addr:#x}+{a_size} and {b_addr:#x} are not gap-separated"
            );
        }
    }
}

/// **021 contract 15: determinism.** Two runs assign identical addresses to identical
/// objects. Not a nicety — 001 §5 makes it a hard requirement, and a flaky address makes
/// every `PtrToInt`-dependent branch look flaky.
#[test]
fn two_runs_assign_identical_addresses() {
    let build = || {
        let mut s = space();
        let ids: Vec<ObjectId> = (0..10)
            .map(|i| s.alloc(ObjKind::Heap, 64 + i * 8, 8, Span::DUMMY))
            .collect();
        ids.into_iter()
            .map(|i| s.addr_of(i).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(build(), build());
}

/// **021 contract 12** — the easy case. A pointer round-tripped through an integer
/// resolves back to its object at the same offset.
#[test]
fn a_pointer_round_trips_through_an_integer() {
    let mut s = space();
    let o = s.alloc(ObjKind::Heap, 128, 8, Span::DUMMY);
    let p = Pointer { base: o, off: 16 };
    let n = s.ptr_to_int(p);
    assert_eq!(s.int_to_ptr(n), p, "the round trip must be exact");
}

/// **021 contract 12b, first half — the case address search gets wrong in the dangerous
/// direction.**
///
/// A pointer out of bounds by more than a guard gap must round-trip to its *original*
/// object, not to the neighbour whose address range it collides with. Under range search
/// this OOB write silently becomes a legal-looking write to unrelated memory: the bug
/// disappears and the analysis reports nothing.
#[test]
fn an_out_of_bounds_pointer_does_not_round_trip_into_its_neighbour() {
    let mut s = space();
    let a = s.alloc(ObjKind::Heap, 100, 8, Span::DUMMY);
    let b = s.alloc(ObjKind::Heap, 100, 8, Span::DUMMY);

    // Far enough past A to land inside B's address range.
    let past = (s.addr_of(b).unwrap() - s.addr_of(a).unwrap()) as i64 + 8;
    let p = Pointer { base: a, off: past };
    let n = s.ptr_to_int(p);
    let back = s.int_to_ptr(n);
    assert_eq!(
        back.base, a,
        "an OOB pointer must keep its provenance, not become a valid pointer into B"
    );
    assert_eq!(back.off, past);
    assert_ne!(back.base, b);
}

/// **021 contract 12b, second half — the case address search gets wrong in the noisy
/// direction.**
///
/// The legal one-past-the-end pointer of a gap-sized object lands in the guard gap. Range
/// search misses every object and reports a wild pointer on conforming code.
#[test]
fn a_one_past_the_end_pointer_of_a_gap_sized_object_keeps_its_object() {
    let mut s = space();
    // Sized to the gap deliberately: its one-past-the-end address is the first byte of
    // the gap, which is exactly where range search stops finding it. Spelled out rather
    // than written `GUARD_GAP`, so shrinking the constant cannot quietly move the
    // pointer back inside the object and keep the test green.
    let o = s.alloc(ObjKind::Heap, 4096, 4096, Span::DUMMY);
    let p = Pointer { base: o, off: 4096 };
    let n = s.ptr_to_int(p);
    let back = s.int_to_ptr(n);
    assert_eq!(
        back.base, o,
        "one-past-the-end is legal C and must not become a wild pointer"
    );
    assert_eq!(back.off, 4096);
}

/// **021 contract 12c: provenance propagates through integer arithmetic.**
/// `(T*)((uword)p + 8 - 4)` resolves to `p`'s object at offset 4. VPP does this
/// constantly, and a tag that survived only a bare round trip would miss all of it.
#[test]
fn provenance_survives_integer_arithmetic() {
    let mut s = space();
    let o = s.alloc(ObjKind::Heap, 128, 8, Span::DUMMY);
    let n = s.ptr_to_int(Pointer { base: o, off: 0 });
    let n = s.int_add(n, 8);
    let n = s.int_add(n, -4);
    assert_eq!(s.int_to_ptr(n), Pointer { base: o, off: 4 });

    // The in-bounds case above cannot tell tag propagation apart from range search —
    // both answer `(o, 4)` — so it is satisfied by an implementation that drops the tag.
    // Mutation found exactly that. Arithmetic that lands *outside* the object separates
    // them: with the tag, provenance is preserved; without it, range search returns the
    // neighbour or `UNBOUND`, which is the §7.1 bug in miniature.
    let far = s.alloc(ObjKind::Heap, 128, 8, Span::DUMMY);
    let delta = (s.addr_of(far).unwrap() - s.addr_of(o).unwrap()) as i64;
    let n = s.ptr_to_int(Pointer { base: o, off: 0 });
    let n = s.int_add(n, delta + 16);
    let n = s.int_add(n, -8);
    let back = s.int_to_ptr(n);
    assert_eq!(
        back,
        Pointer {
            base: o,
            off: delta + 8
        },
        "arithmetic must not launder provenance into the neighbouring object"
    );
}

/// Range search must include the one-past-the-end address. Every test above that exercises
/// one-past-the-end uses a *tagged* pointer, which never reaches the fallback — so the
/// boundary in the fallback itself was untested, and narrowing it to `<` survived.
#[test]
fn range_search_includes_one_past_the_end() {
    let mut s = space();
    let o = s.alloc(ObjKind::Heap, 128, 8, Span::DUMMY);
    let end = s.addr_of(o).unwrap() + 128;
    assert_eq!(
        s.int_to_ptr(IntVal::Const(end)),
        Pointer { base: o, off: 128 },
        "one-past-the-end is a legal C pointer and the fallback must find it"
    );
}

/// **021 contract 13: an `IntToPtr` of a constant that matches nothing is `UNBOUND`**,
/// and an access through it is a wild-pointer finding — not a silent success and not a
/// guess at the nearest object.
#[test]
fn an_unprovenanced_constant_resolves_to_unbound() {
    let s = space();
    let p = s.int_to_ptr(IntVal::Const(0xDEAD));
    assert_eq!(p.base, ObjectId::UNBOUND);
}

/// The fallback still exists: an integer with no recorded provenance that *does* land
/// inside a known object resolves there. Range search is the fallback, not the mechanism
/// — but removing it entirely would break every pointer chiero did not itself mint.
#[test]
fn range_search_is_the_fallback_for_an_untagged_address() {
    let mut s = space();
    let o = s.alloc(ObjKind::Heap, 128, 8, Span::DUMMY);
    let inside = s.addr_of(o).unwrap() + 32;
    // Not produced by `ptr_to_int`, so it carries no tag.
    let p = s.int_to_ptr(IntVal::Const(inside));
    assert_eq!(p, Pointer { base: o, off: 32 });
}
