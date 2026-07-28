//! Covers: 020 §4.4's `AccessPath`, built by lowering.
//!
//! Wave 104 added `AccessPath` to CIR and wave 107 gave it its first consumer — 021
//! contract 19's `Bounded` note names the field that was cut off. Both waves supplied the
//! paths **by hand** in their fixtures, so the naming has never been exercised against
//! real C. This is that gap.
//!
//! §4.4: a path "makes a finding read `p->adj[3].counter` … instead of `*(i64*)(%7 + 24)`"
//! and no analysis may branch on it. Lowering is the only place that knows both the member
//! names and the layout offsets, so it is the only place that can build one.

use chiero_cir::{AccessPath, PathRoot, PathStep};

mod harness;
use harness::lower;

/// The path lowering recorded for the address an access used, found by looking for the
/// only path in the function that ends in `name`.
fn path_ending_in(src: &str, func: &str, name: &str) -> AccessPath {
    let m = lower(src);
    let f = m
        .funcs
        .iter()
        .find(|f| &*f.name == func)
        .unwrap_or_else(|| panic!("no `{func}`"));
    f.access_paths
        .values()
        .find(|p| p.render().ends_with(name))
        .unwrap_or_else(|| {
            panic!(
                "no path ending in `{name}`; lowering recorded {:?}",
                f.access_paths
                    .values()
                    .map(|p| p.render())
                    .collect::<Vec<_>>()
            )
        })
        .clone()
}

/// **A struct member access records a `Field` step with the layout's offset.**
///
/// The offset is the half that cannot be recovered later: `render()` never prints it, and
/// a reader checking a path against a layout needs it. It comes from `RecordLayout` and
/// nowhere else — 015 contract 7's rule, and the reason lowering is where this belongs.
#[test]
fn a_member_access_records_its_field_and_offset() {
    let p = path_ending_in(
        "struct S { int a; int b; }; int f(struct S *s) { return s->b; }",
        "f",
        "b",
    );
    let steps: Vec<&PathStep> = p.steps.iter().collect();
    let last = steps.last().expect("a step");
    match last {
        PathStep::Field { name, off } => {
            assert_eq!(&**name, "b");
            assert_eq!(*off, 4, "`b` is at byte 4 of `struct S`, per the layout");
        }
        other => panic!("expected a field step: {other:?}"),
    }
}

/// **The root is the object, not the address value.**
///
/// A path rooted at whatever `ValueId` happened to hold the pointer would render `%7`,
/// which is the thing §4.4 exists to avoid. `s` is a parameter, so its root is the local
/// slot lowering gave it.
#[test]
fn the_root_names_the_variable() {
    let p = path_ending_in(
        "struct S { int a; int b; }; int f(struct S *s) { return s->b; }",
        "f",
        "b",
    );
    match &p.root {
        PathRoot::Local { name, .. } => {
            assert_eq!(name.as_deref(), Some("s"), "the parameter's own name")
        }
        other => panic!("expected a named local root: {other:?}"),
    }
    assert!(
        p.render().starts_with('s'),
        "so the finding reads `s…` and not `%7…`: {}",
        p.render()
    );
}

/// **A member at offset zero still gets a path.**
///
/// Lowering short-circuits the `PtrAdd` when the offset is 0 and returns the base address
/// unchanged — so the first member of every struct takes a different code path, and it is
/// the one a reader is most likely to hit.
#[test]
fn the_first_member_is_named_too() {
    let p = path_ending_in(
        "struct S { int a; int b; }; int f(struct S *s) { return s->a; }",
        "f",
        "a",
    );
    assert!(
        matches!(p.steps.last(), Some(PathStep::Field { off: 0, .. })),
        "`a` is at offset 0 and still named: {:?}",
        p.steps
    );
}

/// **A nested member accumulates steps**, so `outer.inner.leaf` reads as itself.
#[test]
fn nested_members_accumulate() {
    let p = path_ending_in(
        "struct I { int x; int leaf; }; struct O { int pad; struct I inner; };\n\
         int f(struct O *o) { return o->inner.leaf; }",
        "f",
        "leaf",
    );
    assert_eq!(
        p.render(),
        "o.inner.leaf",
        "the whole chain, not just the last hop: {:?}",
        p.steps
    );
}

/// **A local struct is rooted at the local**, not at a value — the other root shape.
#[test]
fn a_local_struct_is_rooted_at_the_local() {
    let p = path_ending_in(
        "struct S { int a; int b; }; int f(void) { struct S s; s.b = 1; return s.b; }",
        "f",
        "b",
    );
    assert!(p.render().starts_with('s'), "{}", p.render());
}

/// **Paths are reporting-only and never required.** A function with no member access
/// records none, and that is not a failure.
#[test]
fn a_function_with_no_member_access_records_no_paths() {
    let m = lower("int f(int n) { return n + 1; }");
    let f = m.funcs.iter().find(|f| &*f.name == "f").expect("f");
    assert!(
        f.access_paths.is_empty(),
        "nothing to name, so nothing named: {:?}",
        f.access_paths
    );
}
