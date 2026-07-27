//! Foundational span/provenance properties.
//!
//! Covers **010 contracts 1 and 2**. These are the smallest contracts in the set and
//! also the most load-bearing: `Span` is stored on every token, every AST node and
//! every CIR instruction, so its size is a real budget, and `ExpnCtx::ROOT` being the
//! zero value is what makes "written literally in a file" the default state rather
//! than something that must be constructed.

use chiero_span::{BytePos, ExpnCtx, Span};

/// 010 contract 1: `size_of::<Span>() == 12` and `Span: Copy`.
///
/// 010 §2 states the budget explicitly ("do not grow it"). A regression here is
/// invisible at runtime and expensive at scale, so it is pinned mechanically.
#[test]
fn span_is_twelve_bytes_and_copy() {
    assert_eq!(std::mem::size_of::<Span>(), 12, "Span must stay 12 bytes (010 §2)");
    assert_eq!(std::mem::align_of::<Span>(), 4);

    // `Copy` is asserted by using a span after moving it; this fails to compile
    // rather than fails at runtime if the bound is lost.
    fn assert_copy<T: Copy>(_: &T) {}
    let sp = Span::new(BytePos(0), BytePos(1), ExpnCtx::ROOT);
    let moved = sp;
    assert_copy(&sp);
    assert_eq!(sp, moved);
}

/// 010 contract 2, first half: `ExpnCtx::ROOT.0 == 0`.
///
/// The numeric value matters: `ROOT` must be the default/zero context so that a span
/// built without provenance is correctly "written literally in a file" rather than
/// pointing at expansion 0 as if it were a real macro.
#[test]
fn root_is_zero() {
    assert_eq!(ExpnCtx::ROOT.0, 0);
    assert_eq!(ExpnCtx::default(), ExpnCtx::ROOT);
    assert!(ExpnCtx::ROOT.is_root());
    assert!(!ExpnCtx(1).is_root());
}

/// `BytePos` ordering is what the file lookup binary-searches on, so it must be a
/// real total order over the underlying offset.
#[test]
fn bytepos_orders_by_offset() {
    assert!(BytePos(0) < BytePos(1));
    assert!(BytePos(u32::MAX) > BytePos(0));
    let mut v = vec![BytePos(30), BytePos(10), BytePos(20)];
    v.sort();
    assert_eq!(v, vec![BytePos(10), BytePos(20), BytePos(30)]);
}

/// A span's extent and containment are used throughout the frontend to decide whether
/// a token came from a macro body or an argument (010 §2.2), so they are pinned here.
#[test]
fn span_extent_and_containment() {
    let sp = Span::new(BytePos(10), BytePos(20), ExpnCtx::ROOT);
    assert_eq!(sp.len(), 10);
    assert!(!sp.is_empty());
    assert!(sp.contains(BytePos(10)), "lo is inclusive");
    assert!(sp.contains(BytePos(19)));
    assert!(!sp.contains(BytePos(20)), "hi is exclusive");
    assert!(!sp.contains(BytePos(9)));

    let empty = Span::new(BytePos(5), BytePos(5), ExpnCtx::ROOT);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert!(!empty.contains(BytePos(5)));
}

/// `DUMMY` is what hand-written `.cir` fixtures get (020 §6), so it must be
/// recognisable and must be at ROOT — a fixture has no macro provenance.
#[test]
fn dummy_span_is_recognisable_and_rooted() {
    assert!(Span::DUMMY.is_dummy());
    assert_eq!(Span::DUMMY.ctx, ExpnCtx::ROOT);
    assert!(!Span::new(BytePos(0), BytePos(1), ExpnCtx::ROOT).is_dummy());
}

/// Two spans differing only in `ctx` are different spans. This is the property that
/// keeps one macro's expansions distinguishable from another's when they cover the
/// same source text — the basis of the whole provenance model.
#[test]
fn ctx_participates_in_identity() {
    let a = Span::new(BytePos(0), BytePos(4), ExpnCtx::ROOT);
    let b = Span::new(BytePos(0), BytePos(4), ExpnCtx(7));
    assert_ne!(a, b);

    use std::collections::BTreeSet;
    let set: BTreeSet<_> = [a, b].into_iter().collect();
    assert_eq!(set.len(), 2, "ctx must participate in Ord as well as Eq");
}

/// Deterministic ordering is required for every output path (001 §5), and spans are
/// sorted in diagnostics, so `Span: Ord` must be a total order.
#[test]
fn spans_sort_deterministically() {
    let spans = vec![
        Span::new(BytePos(10), BytePos(12), ExpnCtx(1)),
        Span::new(BytePos(10), BytePos(12), ExpnCtx::ROOT),
        Span::new(BytePos(1), BytePos(99), ExpnCtx::ROOT),
    ];
    let mut a = spans.clone();
    let mut b = spans;
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(a[0].lo, BytePos(1), "sorted by lo first");
    assert_eq!(a[1].ctx, ExpnCtx::ROOT, "then by ctx");
}
