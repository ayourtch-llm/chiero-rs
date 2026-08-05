//! **031 contracts 9–11: `LayoutChanged`, where the token stream is not enough.**
//!
//! > 9. Reordering two same-size struct fields is `LayoutChanged`, and impacts accessors of that
//! >    type.
//! > 10. Adding `__attribute__((packed))` to a struct is `LayoutChanged` with the size delta.
//! > 11. Embedding a `LayoutChanged` record inside another record makes the outer record
//! >     `LayoutChanged`.
//!
//! §2 is explicit, and it is the one place in 031 where the machinery that carried contracts 1–8
//! has to be put down:
//!
//! > The `LayoutChanged` test is a **computed comparison** of `RecordLayout` (014 §3), not a
//! > syntactic one. Reordering two same-size fields changes offsets and is `LayoutChanged`;
//! > renaming a field is not layout-affecting but *is* a source-compatibility change for its
//! > users; adding `__attribute__((packed))` changes everything downstream. **VPP's wire-format
//! > structs make this the highest-severity class in the table.**
//!
//! # Why tokens cannot answer this
//!
//! `struct { int a; short b; short c; }` and `struct { int a; short c; short b; }` differ in two
//! tokens and in every offset after the first field. A fingerprint comparison sees "something in
//! the tail differs" and reports *a* change — which happens to be right here, and is right for
//! the wrong reason: it would say the same about a renamed field, which changes no offset, and it
//! has no size delta to report because it never computed one.
//!
//! And the *reordering* case is subtler than it reads. Swapping two **same-size** fields moves no
//! offset in the list — `int a; short b; short c;` lays out at 0, 4, 6 either way — but the offset
//! of the field *named* `b` goes from 4 to 6, and `p->b` reads different bytes. A comparison of
//! bare offsets calls that unchanged, which is why the layout is keyed by field name.
//!
//! Both directions are why only 014 can answer this: it computes what gcc computes, and its own
//! corpus gate checks that against gcc itself.

use chiero_diff::{ChangeClass, Entity, ImpactEdge, Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("the fixture parses")
}

/// **Contract 9.** Two same-size fields swapped: every offset after the first moves.
#[test]
fn reordering_two_same_size_fields_is_a_layout_change() {
    let before = "struct pair { int a; short b; short c; };\n\
                  int get (struct pair *p) { return p->b; }\n";
    let after = "struct pair { int a; short c; short b; };\n\
                 int get (struct pair *p) { return p->b; }\n";
    let set = impact(&prog(before), &prog(after));

    let j = &set.entities[&Entity::record("f.c", "pair")];
    assert_eq!(j.class, ChangeClass::LayoutChanged { size_delta: 0 });
    assert!(
        set.entities.contains_key(&Entity::function("f.c", "get")),
        "and the accessor comes with it: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
}

/// **Contract 10.** `packed` changes the size, and the delta is reported — a maintainer reading
/// "20 → 24" knows immediately whether a wire format moved.
#[test]
fn packing_a_struct_reports_the_size_delta() {
    let before = "struct hdr { char v; int len; };\nint of (struct hdr *h) { return h->len; }\n";
    let after = "struct hdr { char v; int len; } __attribute__((packed));\n\
                 int of (struct hdr *h) { return h->len; }\n";
    let set = impact(&prog(before), &prog(after));

    assert_eq!(
        set.entities[&Entity::record("f.c", "hdr")].class,
        ChangeClass::LayoutChanged { size_delta: -3 },
        "8 bytes with the padding, 5 without"
    );
    assert!(set.entities.contains_key(&Entity::function("f.c", "of")));
}

/// **Contract 11.** A record embedded in another carries the change outward.
#[test]
fn a_layout_change_propagates_to_the_enclosing_record() {
    let with = |inner: &str| {
        format!(
            "struct inner {{ {inner} }};\nstruct outer {{ int tag; struct inner in; }};\n\
             int of (struct outer *o) {{ return o->tag; }}\n"
        )
    };
    let set = impact(
        &prog(&with("int a; short b; short c;")),
        &prog(&with("int a; short c; short b;")),
    );

    assert!(set.entities.contains_key(&Entity::record("f.c", "inner")));
    assert!(
        set.entities.contains_key(&Entity::record("f.c", "outer")),
        "an `outer` holding a changed `inner` is itself changed: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert_eq!(
        set.entities[&Entity::record("f.c", "outer")].edges.first(),
        Some(&ImpactEdge::UsesType {
            name: "inner".to_string()
        })
    );
}

/// **`packed` on an already-tight struct is still a layout change** — and this test asserted the
/// opposite until gcc was asked.
///
/// The premise was that removing no padding changes nothing. It does not change the *size*, and
/// it changes the alignment from 4 to 1:
///
/// ```text
/// struct tight  { int a; int b; };                      size 8  align 4
/// struct tightp { int a; int b; } __attribute__((packed));  size 8  align 1
/// struct wrap  { char c; struct tight  t; };   offsetof(t) == 4
/// struct wrapp { char c; struct tightp t; };   offsetof(t) == 1
/// ```
///
/// So a struct embedding it moves, which is exactly what contract 11 is about. Measured with gcc
/// rather than argued — the same discipline the gcov work needed, and for the same reason: a
/// plausible story about what a compiler does is not a fact about what it does.
///
/// The `size_delta` being 0 is not "no change"; it is the report saying the size held while
/// something else did not.
#[test]
fn packing_an_already_tight_struct_changes_its_alignment() {
    let before = "struct tight { int a; int b; };\nint get (struct tight *p) { return p->b; }\n";
    let after = "struct tight { int a; int b; } __attribute__((packed));\n\
                 int get (struct tight *p) { return p->b; }\n";
    let set = impact(&prog(before), &prog(after));

    assert_eq!(
        set.entities[&Entity::record("f.c", "tight")].class,
        ChangeClass::LayoutChanged { size_delta: 0 },
        "no padding was removed, and the alignment still went from 4 to 1"
    );
}

/// The one that genuinely moves nothing: a field renamed, which changes no offset and no size.
///
/// This is the case a syntactic comparison cannot separate from a reordering — both are "two
/// tokens differ" — and §2 says plainly that renaming "is not layout-affecting but *is* a
/// source-compatibility change for its users".
#[test]
fn a_rename_moves_no_byte() {
    let before = "struct pair { int a; int b; };\nint get (struct pair *p) { return p->b; }\n";
    let after = "struct pair { int a; int renamed; };\nint get (struct pair *p) { return p->b; }\n";
    let set = impact(&prog(before), &prog(after));
    assert_eq!(
        set.entities[&Entity::record("f.c", "pair")].class,
        ChangeClass::BodyChanged,
        "`renamed` sits exactly where `b` did"
    );
}
