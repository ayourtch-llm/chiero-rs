//! **A pointer that was only *formed* is not an access, and must not be reported as one.**
//!
//! Wave 193 made a symbolic index answerable, and put the check in `fork_on_offset` — which
//! runs for a `PtrAdd`. That placement is correct: C11 6.5.6p8 makes *forming* a pointer
//! more than one past the end undefined, not only dereferencing it. The label is not:
//!
//! ```text
//!   int *p = ga + i;    may-be-out-of-bounds: 1-byte access of ga (256 bytes) may be
//!                       out of bounds — offset 256 is ...
//! ```
//!
//! Two things in that line are untrue. No access happened — `p` is never dereferenced — and
//! the "1-byte" size is a placeholder the caller had to invent, because
//! `MemFault::OutOfBoundsMaybe` describes an access and an access has a width.
//!
//! It matters beyond tidiness. 023 §6 ranks and deduplicates findings by what they are, and
//! these two want different triage: VPP forms pointers past the end deliberately in a few
//! places (`vec_end` and friends), where an out-of-bounds *access* is always a bug. A reader
//! who learns to skip "may-be-out-of-bounds" because the pointer cases are noise will skip
//! the access cases too.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

/// The fault names pointer *formation*, and does not claim an access.
#[test]
fn a_pointer_formed_out_of_range_is_not_reported_as_an_access() {
    for (what, src) in [
        (
            "never dereferenced",
            "int ga[64];\nint probe(int i){ int *p = ga + i; return p != 0; }",
        ),
        (
            "dereferenced after",
            "int ga[64];\nint probe(int i){ return ga[i]; }",
        ),
    ] {
        let f = findings(src);
        let m = f
            .iter()
            .find(|x| x.contains("bounds") || x.contains("pointer"))
            .unwrap_or_else(|| panic!("`{what}` reports nothing: {f:?}"));
        assert!(
            m.contains("pointer"),
            "`{what}`: the fault is about forming a pointer, not touching bytes: {m}"
        );
        assert!(
            !m.contains("-byte access"),
            "`{what}`: there is no access width to state: {m}"
        );
    }
}

/// A real access still reports as one.
///
/// The control. Renaming every bounds fault would satisfy the test above and lose the
/// distinction it exists to draw.
#[test]
fn a_genuine_out_of_bounds_access_still_says_access() {
    let f = findings("int ga[2];\nint probe(void){ return ga[5]; }");
    let m = f
        .iter()
        .find(|x| x.contains("bounds"))
        .unwrap_or_else(|| panic!("the concrete access reports nothing: {f:?}"));
    assert!(m.contains("access"), "bytes really were touched here: {m}");
}

/// The object and the offset that reaches past it are both named.
///
/// A fault a reader cannot act on is not a report (023 §9). "This pointer might be out of
/// range" without saying *which object* or *how far* leaves them to re-derive it.
#[test]
fn the_report_names_the_object_and_the_offset() {
    let f = findings("int ga[64];\nint probe(int i){ int *p = ga + i; return p != 0; }");
    let m = f.iter().find(|x| x.contains("pointer")).expect("reported");
    assert!(m.contains("ga"), "the object it left: {m}");
    assert!(m.contains("256"), "and its size in bytes: {m}");
}
