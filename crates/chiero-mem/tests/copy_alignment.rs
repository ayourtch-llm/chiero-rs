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

/// **The other half of the gap, and it points the opposite way: a false positive.**
///
/// The header above records what dropping `align` costs on the *vector* side — the model
/// cannot express a 32-byte requirement, so it under-reports. Dropping it costs something in
/// the other direction too, and nothing had written that down.
///
/// The requirement is re-derived from the access **size** (`align_fault`: `want = size` for a
/// power of two up to 16). The CIR does not have to be guessed at — it says what the access
/// actually requires. Measured on `struct __attribute__((packed)) P { char c; int v; };`:
///
///     store i32 7i32 -> %5 align 1     ; p->v = 7
///     %9 = load i32, %8 align 1        ; return p->v
///
/// **`align 1` is the compiler saying this access is deliberately unaligned and handled.** A
/// packed member access is ordinary, legal C that gcc compiles into safe code. Re-deriving
/// `want = 4` from the size turns it into a misalignment report, and `chiero-exec` throws away
/// the `align 1` that would have said otherwise.
///
/// Nothing reaches a report today — the engine filters `Misaligned` until a `ub-strict` mode
/// exists — which is exactly why it is worth pinning now. **The under-reporting half fails
/// silently; this half would fail loudly, on legal code, on the day that mode ships**, and
/// this project's own record says a false rejection is the more damaging kind.
///
/// This test pins the *current* behaviour, not the desired one. It is the measurement a
/// `ub-strict` mode has to change, and it will fail when the `align` operand is threaded
/// through — which is the point.
#[test]
fn a_scalar_access_in_an_align_1_object_is_reported_misaligned_today() {
    let mut m = chiero_mem::Memory::new();
    // Alignment 1 is what a packed record's storage looks like.
    let o = m.alloc(chiero_mem::ObjKind::Stack, 256, 1, Span::DUMMY);
    m.set(chiero_mem::Pointer { base: o, off: 0 }, 0, 256, Span::DUMMY);
    let mut a = chiero_solver::TermArena::new();

    // Offset 1, four bytes: exactly `p->v` in `struct __attribute__((packed)) P`.
    let r = m.read_term(
        &mut a,
        chiero_mem::Pointer { base: o, off: 1 },
        4,
        chiero_mem::Endian::Little,
        Span::DUMMY,
    );
    let faults: Vec<String> = r.faults.iter().map(|f| f.kind().to_string()).collect();
    assert!(
        faults.iter().any(|f| f == "misaligned"),
        "pinning today's behaviour: the requirement is derived from the access size, so a \
         packed member read is misaligned as far as the model is concerned. If this stops \
         being true, the `align` operand has been threaded through and the header's \
         description of the gap needs updating with it: {faults:?}"
    );

    // **The discriminator.** A byte access has no requirement, so the assertion above is
    // about alignment rather than about the object being align-1.
    let one = m.read_term(
        &mut a,
        chiero_mem::Pointer { base: o, off: 1 },
        1,
        chiero_mem::Endian::Little,
        Span::DUMMY,
    );
    let one_faults: Vec<String> = one.faults.iter().map(|f| f.kind().to_string()).collect();
    assert!(
        !one_faults.iter().any(|f| f == "misaligned"),
        "a one-byte access is aligned everywhere: {one_faults:?}"
    );
}
