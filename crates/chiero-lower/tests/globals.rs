//! File-scope variables lower to **valid** CIR.
//!
//! Not a numbered contract; found in wave 111 while trying to make a global produce a
//! finding. Two independent defects surfaced in five minutes, both making `verify` reject
//! the module — so nothing downstream runs at all, and every checker, pass and engine test
//! is silently inapplicable to any translation unit that indexes a file-scope array.
//!
//! That is most of C, and all of VPP: `ip4_main`, the node registration tables, every
//! `static` counter array. The corpus fixtures are function-local almost throughout, which
//! is how this survived 111 waves.
//!
//! **`verify` is the assertion.** These tests do not check shapes; they check that lowering
//! produced something the rest of the system will accept, which is the property that was
//! actually broken.

mod harness;
use harness::lower;

/// Lower `src` and return the verifier's complaints.
fn errors(src: &str) -> Vec<String> {
    let m = lower(src);
    chiero_cir::verify::verify(&m)
        .iter()
        .filter(|e| e.is_error())
        .map(|e| format!("{:?}: {}", e.kind, e.detail))
        .collect()
}

/// **The defect.** Indexing a file-scope array.
///
/// `PtrAdd base must be pointer-typed, got Int(32)` — `lvalue_addr`'s `Ident` arm looks
/// only in `fs().locals`, so a global name falls through to the value path and the index
/// arithmetic is done on the array's first *element* rather than on its address.
#[test]
fn indexing_a_global_array_verifies() {
    assert!(
        errors("int g[4]; int f(void) { return g[1]; }").is_empty(),
        "{:#?}",
        errors("int g[4]; int f(void) { return g[1]; }")
    );
}

/// Writing through the same path, which is the half that corrupts memory rather than
/// merely reading wrongly.
#[test]
fn assigning_to_a_global_array_element_verifies() {
    let e = errors("int g[4]; void f(int n) { g[2] = n; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// A **symbolic** index, so the constant-folding path is not the only one covered.
#[test]
fn a_symbolic_index_into_a_global_verifies() {
    let e = errors("int g[4]; int f(int i) { return g[i]; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// `&g[1]` — the address is the value, so a fix that only repaired loads is visible.
#[test]
fn taking_the_address_of_a_global_element_verifies() {
    let e = errors("int g[4]; int *f(void) { return &g[1]; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// A **global struct member**, which reaches `lvalue_addr` through `Member` rather than
/// `Index` and so is a second route to the same missing case.
#[test]
fn a_global_struct_member_verifies() {
    let e = errors("struct S { int a; int b; }; struct S g; int f(void) { return g.b; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// **The second defect.** Writing through a cast that discards `const`.
///
/// `WidthMismatch`. Undefined behaviour in C and chiero should *report* it (021 c21), which
/// it cannot do while the module is rejected before it runs.
#[test]
fn writing_through_a_cast_away_from_const_verifies() {
    let e = errors("const int g = 1; void f(void) { *(int *)&g = 2; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// A plain global read, as the control: if this were broken too the tests above would be
/// about something much larger, and the failure message should say so.
#[test]
fn reading_a_global_scalar_verifies() {
    let e = errors("int g; int f(void) { return g; }");
    assert!(e.is_empty(), "{e:#?}");
}
