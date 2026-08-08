//! Covers: 021 §5 step 3 on the **copy** path — and the reason it looks like a gap and is not.
//!
//! ⚠️ **This file began as a RED asserting that a misaligned copy records `misaligned`, and that
//! assertion was wrong.** `write_bytewise` strips `Misaligned` deliberately, and the rationale is
//! sound: a copy is defined **byte-wise**, exactly as `memcpy` is, and a byte write has no
//! alignment requirement. Nothing said so — the strip carried no comment — and the measurement
//! that motivated the RED (a copy records nothing where `read_term` at the same offset records
//! `misaligned`) is real and has an innocent explanation. Kept as a test so the next reader does
//! not re-derive it: **an undocumented deliberate behaviour looks exactly like a defect.**
//!
//! # The real gap, which this does not fix
//!
//! `CopyMem` conflates two accesses that C distinguishes. A struct assignment is a byte-wise copy
//! with no alignment requirement; a `u8x32` vector move requires 32-byte alignment, which is the
//! whole difference between vppinfra's `u8x32` and `u8x32u`. Both lower to `copymem`, and the CIR
//! carries the distinguishing information — `copymem %6 -> %13, 32i64 align 16` — which
//! `chiero-exec` then discards with a bare `let _ = align;`.
//!
//! So the model cannot tell a memcpy from a vector move, and `align_fault`'s own `size <= 16`
//! bound means it could not express a 32-byte requirement even if it were asked. Neither changes
//! a report today: the engine filters `Misaligned` until a `ub-strict` mode exists. HANDOFF §9.1.

use chiero_span::Span;

fn obj() -> (chiero_mem::Memory, chiero_mem::ObjectId) {
    let mut m = chiero_mem::Memory::new();
    let o = m.alloc(chiero_mem::ObjKind::Stack, 256, 32, Span::DUMMY);
    m.set(chiero_mem::Pointer { base: o, off: 0 }, 0, 256, Span::DUMMY);
    (m, o)
}

fn copy_faults(size: u64, dst_off: i64) -> Vec<String> {
    let (mut m, o) = obj();
    let mut a = chiero_solver::TermArena::new();
    let r = m.copy_via(
        &mut a,
        chiero_mem::Pointer {
            base: o,
            off: dst_off,
        },
        chiero_mem::Pointer { base: o, off: 0 },
        size,
        chiero_mem::Overlap::Forbidden,
        Span::DUMMY,
    );
    r.faults.iter().map(|f| f.kind().to_string()).collect()
}

/// **A copy is byte-wise, so no offset is misaligned for it** — the deliberate behaviour.
#[test]
fn a_copy_is_bytewise_and_records_no_misalignment() {
    for size in [2u64, 4, 8, 16, 32, 64] {
        let faults = copy_faults(size, 129);
        assert!(
            !faults.iter().any(|f| f == "misaligned"),
            "a copy is defined byte-wise, as `memcpy` is: {size} bytes to offset 129 {faults:?}"
        );
    }
}

/// **And a scalar read of the same width at the same offset does record it** — so the difference
/// is about the *kind* of access, not about the model having lost the check.
#[test]
fn a_scalar_read_at_the_same_offset_still_records_it() {
    let (mut m, o) = obj();
    let mut a = chiero_solver::TermArena::new();
    let r = m.read_term(
        &mut a,
        chiero_mem::Pointer { base: o, off: 129 },
        8,
        chiero_mem::Endian::Little,
        Span::DUMMY,
    );
    let kinds: Vec<&str> = r.faults.iter().map(|f| f.kind()).collect();
    assert!(
        kinds.contains(&"misaligned"),
        "an 8-byte scalar read at offset 129 is misaligned: {kinds:?}"
    );
}

/// ⚠️ **The width bound that would silently defeat a `ub-strict` mode.** `align_fault` derives the
/// requirement from the access size and gives up above 16 bytes, so a 32-byte access records
/// nothing even on the path that does check — it is refused as an unsupported width first.
/// vppinfra is built out of 32- and 64-byte accesses.
#[test]
fn a_wide_scalar_read_is_refused_before_alignment_is_considered() {
    let (mut m, o) = obj();
    let mut a = chiero_solver::TermArena::new();
    let r = m.read_term(
        &mut a,
        chiero_mem::Pointer { base: o, off: 129 },
        32,
        chiero_mem::Endian::Little,
        Span::DUMMY,
    );
    let kinds: Vec<&str> = r.faults.iter().map(|f| f.kind()).collect();
    assert_eq!(
        kinds,
        vec!["unsupported-access-width"],
        "a 32-byte scalar read is refused by width, so its misalignment is never reached"
    );
}
