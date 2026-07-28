//! Covers: 014 contracts 1, 2, 3, 4, 5, 6, 7, 8, 10.
//!
//! **Every layout in this file is checked twice**: once against the number 014 §8 states,
//! and once against gcc. The second check is the one that matters, and 014 §7 says why —
//! a one-byte error in a struct offset produces confident wrong answers everywhere in 021
//! rather than a visible failure, and a hand-written expectation is exactly the thing a
//! layout bug corrupts. Asserting only what I believe the answer to be would test my
//! belief.
//!
//! The gcc check has two halves, because `_Static_assert` alone cannot see bit-fields:
//! `__builtin_offsetof` is ill-formed on one. So byte offsets are compared at compile
//! time, and **bit placement is compared at run time** by writing all-ones into one field
//! and reading the object back as bytes. Without the second half, contract 5's straddling
//! rules would be asserted only against my own arithmetic.

mod harness;

use chiero_sema::{ArrayLen, RecordId, TargetConfig, Ty};
use harness::{Parsed, gcc_available, parse};

fn layout_of<'a>(p: &'a Parsed, tag: &str) -> (&'a chiero_sema::Analysis, RecordId) {
    let sym = p.symbol(tag).unwrap_or_else(|| panic!("no symbol `{tag}`"));
    let rid = p
        .analysis
        .record_by_tag(sym)
        .unwrap_or_else(|| panic!("no record laid out for `struct {tag}`"));
    (&p.analysis, rid)
}

/// Parse, analyse, assert the stated numbers, **and** put the same numbers to gcc.
fn check(src: &str, tag: &str, size: u64, align: u64, offsets: &[(&str, u64)]) -> Parsed {
    let p = parse(src, TargetConfig::x86_64_linux());
    assert!(
        p.analysis.diagnostics.is_empty(),
        "sema must be clean: {:?}",
        p.analysis.diagnostics
    );
    let (a, rid) = layout_of(&p, tag);
    let l = a.layout(rid);

    assert_eq!(l.size, size, "sizeof(struct {tag})");
    assert_eq!(l.align, align, "_Alignof(struct {tag})");
    for (field, want) in offsets {
        let f = l
            .fields
            .iter()
            .find(|f| f.name.and_then(|n| p.text(n)).as_deref() == Some(*field))
            .unwrap_or_else(|| panic!("no field `{field}` in `struct {tag}`"));
        assert_eq!(f.offset, *want, "offsetof(struct {tag}, {field})");
    }

    if gcc_available() {
        harness::assert_agrees_with_gcc(src, tag, l, &p);
    } else {
        eprintln!("skipping the gcc cross-check: gcc not found (014 §7)");
    }
    p
}

/// **Contract 1.** The baseline: padding between a `char` and an `int`.
#[test]
fn a_char_followed_by_an_int_is_padded_to_the_ints_alignment() {
    check(
        "struct S { char a; int b; };",
        "S",
        8,
        4,
        &[("a", 0), ("b", 4)],
    );
}

/// **Contract 2.** `packed` removes internal padding and sets member alignment to 1. VPP
/// uses it 112 times, predominantly on wire-format structs, where a wrong offset means
/// every parsed packet field is wrong.
#[test]
fn packed_removes_internal_padding() {
    check(
        "struct __attribute__((packed)) S { char a; int b; };",
        "S",
        5,
        1,
        &[("a", 0), ("b", 1)],
    );
}

/// **Contract 3.** Bit-fields pack from the least significant bit on a little-endian
/// target, so `b` starts at bit 3 rather than at a fresh byte.
#[test]
fn adjacent_bitfields_share_an_allocation_unit() {
    let p = check("struct S { int a:3; int b:5; };", "S", 4, 4, &[]);
    let (a, rid) = layout_of(&p, "S");
    let l = a.layout(rid);
    let bits = |name: &str| {
        l.fields
            .iter()
            .find(|f| f.name.and_then(|n| p.text(n)).as_deref() == Some(name))
            .and_then(|f| f.bits)
            .unwrap_or_else(|| panic!("`{name}` is not a bit-field"))
    };
    assert_eq!(bits("a").bit_offset, 0);
    assert_eq!(bits("a").width, 3);
    assert_eq!(
        bits("b").bit_offset,
        3,
        "little-endian allocation runs from the least significant bit"
    );
    assert_eq!(bits("b").width, 5);
}

/// **Contract 4.** A zero-width bit-field declares no member and forces the next one to a
/// fresh allocation unit. It is the reason "is a bit-field" and "has a nonzero width" have
/// to be different questions.
#[test]
fn a_zero_width_bitfield_starts_a_new_allocation_unit() {
    let p = check("struct S { int a:3; int :0; int b:5; };", "S", 8, 4, &[]);
    let (a, rid) = layout_of(&p, "S");
    let l = a.layout(rid);
    let b = l
        .fields
        .iter()
        .find(|f| f.name.and_then(|n| p.text(n)).as_deref() == Some("b"))
        .expect("field b")
        .bits
        .expect("bit-field");
    assert_eq!(
        b.bit_offset, 32,
        "`b` restarts at the next `int`-sized unit, not at bit 3"
    );

    // The discriminator: without the zero-width field, `b` would sit at bit 3 and the
    // struct would be 4 bytes. If it did not force a new unit, this pair would be equal.
    let plain = parse(
        "struct S { int a:3; int b:5; };",
        TargetConfig::x86_64_linux(),
    );
    let (pa, prid) = layout_of(&plain, "S");
    assert_ne!(
        pa.layout(prid).size,
        l.size,
        "the zero-width field has to change something, or the test is about nothing"
    );
}

/// **Contract 5.** A bit-field that would straddle an allocation unit boundary is placed
/// per gcc's rules — and the cross-check is against gcc, not against my arithmetic.
///
/// This is the case 014 §3 calls "historically the buggiest area of any layout
/// implementation", so the run-time bit probe in the harness matters more here than
/// anywhere else: the sizes could be right while every bit sat in the wrong place.
#[test]
fn a_straddling_bitfield_follows_gcc() {
    // 30 bits used, then a 6-bit field that does not fit in the remaining 2.
    check("struct S { int a:30; int b:6; };", "S", 8, 4, &[]);
    // The same shape at a byte boundary, and one that fits exactly.
    check("struct S { char a:6; int b:20; };", "S", 4, 4, &[]);
    check("struct S { int a:24; int b:8; };", "S", 4, 4, &[]);
}

/// **Contract 6.** `struct { char a; int b:24; }` differs packed and unpacked, and both
/// must match gcc.
#[test]
fn a_bitfield_after_a_byte_differs_packed_and_unpacked() {
    let unpacked = check("struct S { char a; int b:24; };", "S", 4, 4, &[]);
    let packed = check(
        "struct __attribute__((packed)) S { char a; int b:24; };",
        "S",
        4,
        1,
        &[],
    );
    let (ua, urid) = layout_of(&unpacked, "S");
    let (pa, prid) = layout_of(&packed, "S");
    assert_ne!(
        ua.layout(urid).align,
        pa.layout(prid).align,
        "packed has to change the alignment, or the pair proves nothing"
    );
}

/// **Contract 7.** A flexible array member contributes 0 to size but does affect
/// alignment, and is recorded as such — 1165 VPP files depend on the zero-length or
/// flexible form.
#[test]
fn a_flexible_array_member_adds_no_size_and_is_recorded() {
    let p = check("struct S { int n; int a[]; };", "S", 4, 4, &[("n", 0)]);
    let (a, rid) = layout_of(&p, "S");
    let l = a.layout(rid);
    let idx = l.flexible_member.expect("the flexible member is recorded");
    assert_eq!(
        l.fields[idx].name.and_then(|n| p.text(n)).as_deref(),
        Some("a")
    );
    assert!(matches!(
        a.ty(l.fields[idx].ty),
        Ty::Array {
            len: ArrayLen::Flexible,
            ..
        }
    ));

    // And a *sized* trailing array is not recorded as flexible, or the field means
    // nothing.
    let sized = parse(
        "struct S { int n; int a[4]; };",
        TargetConfig::x86_64_linux(),
    );
    let (sa, srid) = layout_of(&sized, "S");
    assert_eq!(sa.layout(srid).flexible_member, None);
    assert_eq!(sa.layout(srid).size, 20);
}

/// **Contract 8.** A union sizes to its largest member and aligns to the strictest.
#[test]
fn a_union_sizes_to_its_largest_member_and_aligns_to_the_strictest() {
    let p = check(
        "union S { char a[7]; int b; };",
        "S",
        8,
        4,
        &[("a", 0), ("b", 0)],
    );
    let (a, rid) = layout_of(&p, "S");
    assert!(a.layout(rid).is_union, "and it knows it is a union");
}

/// **Contract 10.** `enum { A = 0x100000000 }` widens the underlying type past `int`.
///
/// The discriminator is an enum that does *not* need widening: if the rule were "always
/// use `long`", the first assertion would pass on its own.
#[test]
fn an_enum_widens_its_underlying_type_only_when_a_value_requires_it() {
    let wide = parse(
        "enum E { A = 0x100000000 }; enum E e;",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        wide.analysis.diagnostics.is_empty(),
        "{:?}",
        wide.analysis.diagnostics
    );
    let ty = wide.decl_ty("e").expect("`e` is typed");
    assert!(
        matches!(wide.analysis.ty(ty), Ty::Int { bits, .. } if *bits > 32),
        "a value past `int` widens the underlying type: {:?}",
        wide.analysis.ty(ty)
    );

    let narrow = parse("enum E { A = 1 }; enum E e;", TargetConfig::x86_64_linux());
    let ty = narrow.decl_ty("e").expect("`e` is typed");
    assert!(
        matches!(narrow.analysis.ty(ty), Ty::Int { bits: 32, .. }),
        "and one that fits stays `int`, or the rule is just `always widen`: {:?}",
        narrow.analysis.ty(ty)
    );
}

/// The whole corpus of small records at once, put to gcc.
///
/// 014 §7's argument is that this scales: the same generator points at every record type
/// chiero can parse. This is that generator on a deliberately awkward set — nested
/// records, arrays of records, pointers, `aligned` combined with `packed`, and a union
/// inside a struct — none of which any single contract names, and all of which VPP
/// contains.
#[test]
fn a_spread_of_awkward_records_agrees_with_gcc() {
    if !gcc_available() {
        eprintln!("skipping: gcc not found (014 §7)");
        return;
    }
    let cases: &[(&str, &str)] = &[
        ("struct S { double a; char b; };", "S"),
        ("struct S { char a; short b; char c; long d; };", "S"),
        (
            "struct Inner { char x; int y; }; struct S { char a; struct Inner b; };",
            "S",
        ),
        (
            "struct Inner { char x; }; struct S { struct Inner a[3]; int b; };",
            "S",
        ),
        ("struct S { char a; void *p; char b; };", "S"),
        (
            "struct S { char a; int b __attribute__((aligned(16))); char c; };",
            "S",
        ),
        (
            "struct __attribute__((packed)) S { char a; int b __attribute__((aligned(8))); };",
            "S",
        ),
        (
            "union U { char a[3]; }; struct S { char x; union U u; int y; };",
            "S",
        ),
        ("struct S { char a; long long b; short c; };", "S"),
        ("struct S { int a:1; int b:1; char c; int d:20; };", "S"),
        (
            "struct __attribute__((packed)) S { char a; short b; int c; long d; };",
            "S",
        ),
        ("struct S { char a[0]; int b; };", "S"),
        // Record-level `aligned`, which is a different code path from a member-level one
        // and which a mutation showed was unexercised.
        ("struct __attribute__((aligned(32))) S { int a; };", "S"),
        (
            "struct __attribute__((aligned(32))) S { char a; char b; };",
            "S",
        ),
        (
            "union __attribute__((packed)) S { int a; char b[7]; };",
            "S",
        ),
        // A union of **bit-fields**: each starts at bit 0. Nothing else here had one, so
        // laying them out sequentially was indistinguishable from laying them on top of
        // each other — the sizes come out the same and only the bit probe can tell.
        ("union S { int a:3; int b:20; };", "S"),
        ("union S { unsigned a:1; unsigned b:32; char c; };", "S"),
    ];
    for (src, tag) in cases {
        let p = parse(src, TargetConfig::x86_64_linux());
        assert!(
            p.analysis.diagnostics.is_empty(),
            "{src}: {:?}",
            p.analysis.diagnostics
        );
        let (a, rid) = layout_of(&p, tag);
        harness::assert_agrees_with_gcc(src, tag, a.layout(rid), &p);
    }
}
