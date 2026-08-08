//! Covers: 021 §5 step 3 — *"Alignment check … is always recorded"* — on the **copy** path.
//!
//! `Memory::copy`'s own comment says *"Both ends run the same five steps; the source is read and
//! the destination written"*. Step 3 was not among them: a misaligned copy recorded nothing at any
//! width, while a `read_term` at the identical offset recorded `misaligned`.
//!
//! **This is the path that matters most for VPP.** C struct assignment lowers to `CopyMem`
//! (020 §4.13b, no aggregate values in CIR) and so does every vector access — a `u8x32` load is
//! `copymem …, 32i64`, and the AVX-512 lowering of one VPP TU holds 7779 copies of 32 bytes or
//! more. So "always recorded" was false for exactly the accesses vppinfra is built out of.
//!
//! It changes no report today: the engine filters `Misaligned` out of findings until there is a
//! `ub-strict` mode (021 §5 step 3, x86-64 tolerates misalignment and VPP relies on it). It
//! decides what that mode will see when it arrives, and a mode built on a blind path is worse
//! than no mode.

use chiero_span::Span;

/// A 256-byte object aligned to 32, filled so nothing reads uninitialized.
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

/// **The read path and the copy path must agree about the same access.**
#[test]
fn a_misaligned_copy_records_the_misalignment() {
    for size in [2u64, 4, 8, 16] {
        let faults = copy_faults(size, 129);
        assert!(
            faults.iter().any(|f| f == "misaligned"),
            "a {size}-byte copy to offset 129 is misaligned and recorded nothing: {faults:?}"
        );
    }
}

/// And an aligned one records nothing — or the assertion above would pass on a model that
/// reported misalignment unconditionally.
#[test]
fn an_aligned_copy_records_nothing() {
    for size in [2u64, 4, 8, 16] {
        let faults = copy_faults(size, 64);
        assert!(
            !faults.iter().any(|f| f == "misaligned"),
            "a {size}-byte copy to offset 64 is aligned: {faults:?}"
        );
    }
}
