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
//! The direction that matters is the other one. `__attribute__((packed))` on a struct whose
//! fields are already tightly packed changes **nothing**, and a `#pragma pack` interaction can
//! change every offset while the struct's own tokens are untouched. Only asking 014 for the two
//! layouts and comparing them answers those.

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

/// **The computed comparison earns its keep here.** Renaming a field moves no offset, so it is
/// *not* a layout change — but it is still a change, because its users name the field.
///
/// A syntactic comparison cannot tell these apart: both are "two tokens differ in the tail". The
/// class is what a report leads with and what §3.3 closes over, so getting it wrong is not
/// cosmetic.
#[test]
fn renaming_a_field_is_a_change_but_not_a_layout_change() {
    let before = "struct pair { int a; int b; };\nint get (struct pair *p) { return p->b; }\n";
    let after = "struct pair { int a; int renamed; };\nint get (struct pair *p) { return p->b; }\n";
    let set = impact(&prog(before), &prog(after));

    let class = set.entities[&Entity::record("f.c", "pair")].class;
    assert_ne!(
        class,
        ChangeClass::LayoutChanged { size_delta: 0 },
        "no offset moved; `renamed` sits exactly where `b` did"
    );
    assert_eq!(class, ChangeClass::BodyChanged);
}

/// And the other direction: a struct whose fields were already tightly packed is **not** changed
/// by `packed`, however loudly the tokens differ.
///
/// This is the one a syntactic comparison gets wrong in the *dangerous* direction — it would
/// report a layout change, and 032 would run the tests of everything touching the type for a
/// diff that moved nothing.
#[test]
fn packing_an_already_packed_struct_changes_no_layout() {
    let before = "struct tight { int a; int b; };\nint get (struct tight *p) { return p->b; }\n";
    let after = "struct tight { int a; int b; } __attribute__((packed));\n\
                 int get (struct tight *p) { return p->b; }\n";
    let set = impact(&prog(before), &prog(after));

    assert_ne!(
        set.entities
            .get(&Entity::record("f.c", "tight"))
            .map(|j| j.class),
        Some(ChangeClass::LayoutChanged { size_delta: 0 }),
        "every field was already at its natural offset and the size is unchanged"
    );
}
