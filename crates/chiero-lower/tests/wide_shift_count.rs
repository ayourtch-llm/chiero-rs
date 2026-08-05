//! **Narrowing a shift count must not narrow away the defect.**
//!
//! `1 << i` with a `uword` `i` is an `int` shifted by a 64-bit count (C11 6.5.7p3 promotes the
//! operands independently), and CIR wants both operands at the shift's width. Lowering clamps
//! before it truncates, because a count at or past the width is UB that 020 §4.1 requires as a
//! `shift-UB` event and the engine's test is `count >= width` on the operand it is handed.
//!
//! A plain `Trunc` passes every value test in the differential file — `1 << 64` becomes a 32-bit
//! zero, and a shift by zero is a shift by zero. It is only wrong about the thing this project
//! exists to report, which is exactly the kind of fix that needs its own test.

mod harness;

use chiero_exec::{Engine, UbKind};
use chiero_solver::TermArena;

/// The **events**, not the findings: a shift past the width is a `UbKind::Shift` the engine
/// records as it executes, and `findings()` is a different question asked by 040's checkers.
fn shift_events(src: &str) -> usize {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena)
        .ub_events()
        .into_iter()
        .filter(|e| e.kind == UbKind::Shift)
        .count()
}

/// A 64-bit count past the 32-bit value's width still reports.
///
/// **The count is 2^32, and that number is the test.** 64 would pass a plain `Trunc` too — it
/// still reads 64 in 32 bits and still exceeds the width — so a fixture using it pins nothing.
/// 2^32 has all-zero low bits: truncated it is a shift by *nothing*, silently well-defined, and
/// only a clamp keeps it reportable. Written with 64 first, which is how this was found.
#[test]
fn an_out_of_range_wide_count_still_reports_shift_ub() {
    for count in ["64", "4294967296UL", "18446744073709551615UL"] {
        let src = format!("int probe(void) {{ unsigned long i = {count}; return 1 << i; }}");
        assert_eq!(
            shift_events(&src),
            1,
            "`1 << {count}` is undefined, and the count being wider than the value is not a \
             reason to stop saying so"
        );
    }
}

/// And an in-range one does not, or every `x << 1` in VPP's packet code becomes a finding.
#[test]
fn an_in_range_wide_count_reports_nothing() {
    let n = shift_events("int probe(void) { unsigned long i = 3; return 1 << i; }");
    assert_eq!(n, 0, "a shift by 3 at 32 bits is ordinary C");
}
