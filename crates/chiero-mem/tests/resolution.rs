//! The two accessors 021 §5.1's search is built on — `resolvable_ranges` and
//! `wild_region_around`.
//!
//! Neither had a direct test: the engine's tests reach them through a resolution whose
//! address is pinned by the path, and a pinned address lands *inside* an object on the
//! first model, so the region logic never runs. Review demonstrated three one-address
//! mutations — `hi.min(base)` for `hi.min(base - 1)`, `lo.max(top)` for `lo.max(top + 1)`,
//! and an exclusive `top` — that the whole engine suite accepted, each of which makes the
//! wild region swallow a real candidate. Two of them swallow the **legal
//! one-past-the-end pointer**, which is the false positive 021 §7.1 names by hand.

use chiero_mem::*;
use chiero_span::Span;

fn mem_with(sizes: &[u64]) -> (Memory, Vec<ObjectId>) {
    let mut m = Memory::new();
    let ids = sizes
        .iter()
        .map(|s| m.alloc(ObjKind::Heap, *s, 8, Span::DUMMY))
        .collect();
    (m, ids)
}

/// The region stops **one below** the next object's base, and **one above** the previous
/// object's last addressable byte — which is one *past* its end, since a one-past-the-end
/// pointer is legal C and belongs to the object it came from (021 §7.1).
#[test]
fn the_wild_region_stops_at_the_addresses_objects_own() {
    let (m, ids) = mem_with(&[32, 32]);
    let (a0, a1) = (m.addr_of(ids[0]).unwrap(), m.addr_of(ids[1]).unwrap());
    assert!(a1 > a0 + 32, "the fixture needs a gap between them");

    // A point in the gap: the region reaches from just past object 0's one-past-the-end
    // address to just below object 1's base.
    let (lo, hi) = m.wild_region_around(a0 + 100);
    assert_eq!(
        lo,
        a0 + 32 + 1,
        "object 0 owns byte {} — its one-past-the-end address",
        a0 + 32
    );
    assert_eq!(hi, a1 - 1, "object 1 owns its base address");

    // The two addresses either edge must not swallow.
    for (addr, owner) in [(a0 + 32, ids[0]), (a1, ids[1])] {
        assert_eq!(
            m.wild_region_around(addr),
            (addr, addr),
            "{addr:#x} belongs to {owner:?}, so there is no wild region around it — a \
             region here would be excluded wholesale and take the object with it"
        );
    }
}

/// **Freed objects bound the region too.** `wild_region_around` computes over
/// `resolvable_ranges`, which keeps freed and out-of-scope entries — a region derived
/// from live objects only would span the freed block, §5.1's search would exclude it in
/// one query, and the use-after-free would become invisible. Review showed the
/// substitution passing the whole engine suite.
#[test]
fn a_freed_object_still_bounds_the_wild_region() {
    let (mut m, ids) = mem_with(&[32, 32]);
    let (a0, a1) = (m.addr_of(ids[0]).unwrap(), m.addr_of(ids[1]).unwrap());
    m.free(ids[1], Span::DUMMY);

    let (lo, hi) = m.wild_region_around(a0 + 100);
    assert_eq!(
        hi,
        a1 - 1,
        "the region stops below the freed object, not at the end of memory"
    );
    assert_eq!(lo, a0 + 32 + 1);
    assert!(
        m.resolvable_ranges().iter().any(|(id, _, _)| *id == ids[1]),
        "a freed object is still nameable by a pointer (021 §4)"
    );
    assert!(
        !m.live_ranges().iter().any(|(id, _, _)| *id == ids[1]),
        "and is still not live — the two questions are different"
    );
}

/// An address inside an object is not wild, and the accessor says so by returning the
/// point rather than a region: a caller that asks anyway must not be handed an interval
/// covering its neighbours.
#[test]
fn an_address_inside_an_object_gets_no_region() {
    let (m, ids) = mem_with(&[32, 32]);
    let a0 = m.addr_of(ids[0]).unwrap();
    assert_eq!(m.wild_region_around(a0 + 8), (a0 + 8, a0 + 8));
    // The boundary case: one past the end is *owned*, so it gets no region either.
    assert_eq!(m.wild_region_around(a0 + 32), (a0 + 32, a0 + 32));
}

/// With no objects at all, everything is wild — and the region is the whole space rather
/// than an empty or inverted one, which a search would read as "nothing to exclude" and
/// then never terminate.
#[test]
fn with_no_objects_the_region_is_the_whole_address_space() {
    let m = Memory::new();
    assert!(m.resolvable_ranges().is_empty());
    assert_eq!(m.wild_region_around(0x4000), (0, u64::MAX));
}

/// `resolvable_ranges` reports each object's **entry** size, which is what the search
/// tests containment against. A range built from anything else — the space's record, a
/// rounded allocation — makes an access at the last byte look out of bounds or an access
/// past the end look inside.
#[test]
fn resolvable_ranges_reports_each_objects_own_extent() {
    let (m, ids) = mem_with(&[1, 4096, 7]);
    let got: Vec<_> = m
        .resolvable_ranges()
        .into_iter()
        .map(|(id, _, size)| (id, size))
        .collect();
    assert_eq!(got, vec![(ids[0], 1), (ids[1], 4096), (ids[2], 7)]);
    for (id, addr, _) in m.resolvable_ranges() {
        assert_eq!(Some(addr), m.addr_of(id), "at the address it was placed at");
    }
}
