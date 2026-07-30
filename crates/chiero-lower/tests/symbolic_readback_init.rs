//! **A byte written at a symbolic index reads back with no finding.**
//!
//! This file exists because two mutants in `write_at_symbolic_offset`'s init marking survive
//! every test in the tree:
//!
//! ```text
//!   let base_bit = a.mul(idx, eight);   ->  let base_bit = idx;    SURVIVED
//!   for k in 0..8u128 { ... }           ->  for k in 0..1u128 {    SURVIVED
//! ```
//!
//! **These fixtures do not kill them either, and the reason is worth more than the tests.**
//! Wave 203 called the pair *equivalent* and blamed the missing guard discharge. Wave 204
//! landed the discharge; they survived. So the fixtures below were written to read back what
//! a symbolic write wrote — and they survived that too, including a variant that pins the
//! index through the path condition so a *concrete* read covers the written byte.
//!
//! The cause is stated in `read_term_at` itself: **the symbolic read path does not consult
//! `arr.init` at all.** No reader means no way to tell a right marking from a wrong one; only
//! deleting the marking outright is observable, and that is observable through a *different*
//! object's concrete read. Wave 202 rejected adding the check because proving the byte
//! initialized needed a `select` to fold past seven non-matching stores, and a
//! `maybe-uninitialized-read` on memory the program definitely wrote is worse than silence.
//! That argument was about *syntactic folding*, and wave 204 removed its premise — the guard
//! now goes to the solver, which has array theory and does not need the fold. Section 9
//! carries this as the next front.
//!
//! # What these fixtures do pin
//!
//! Silence on memory the program wrote. That is the exact false positive wave 202 feared, so
//! when the init check lands these become its controls rather than its motivation — and they
//! are the fixtures that would catch it regressing into the answer wave 202 declined to ship.
//! The variants differ in where the write lands and what happens after it, because the marking
//! scales a byte index into a bit index: a fixture at index 0 cannot see that scaling, since
//! `idx * 8 == idx` there.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

/// Write at a symbolic index, read the same index back.
///
/// Third variant included on purpose: a concrete store after the symbolic one, so the marking
/// has to survive a later write rather than merely be the last thing that happened.
#[test]
fn a_byte_written_at_a_symbolic_index_is_initialized_when_read_back() {
    for (what, src) in [
        (
            "at the front of the object",
            "int probe(int i){ char ca[64]; ca[i & 31] = 7; return ca[i & 31]; }",
        ),
        // Offset past byte 0, where a byte-indexed marking would coincide with a bit-indexed
        // one and hide the difference.
        (
            "offset past the first byte",
            "int probe(int i){ char ca[64]; ca[(i & 31) + 32] = 7; return ca[(i & 31) + 32]; }",
        ),
        (
            "with a concrete write in between",
            "int probe(int i){ char ca[64]; ca[i & 31] = 7; ca[0] = 5; return ca[i & 31]; }",
        ),
    ] {
        let f = findings(src);
        assert!(
            f.is_empty(),
            "`{what}`: the program wrote this byte and read it back, so there is nothing to \
             report: {f:?}"
        );
    }
}

/// A byte the program never wrote still reports.
///
/// The control, and the one that matters most here: every assertion above is a claim of
/// *silence*, which a fix that stopped reporting uninitialized reads entirely would satisfy.
/// The write covers bytes 32–63, so byte 0 is certainly unwritten — the same fixture wave
/// 204's discharge turned from a `maybe` into a definite report.
#[test]
fn a_byte_no_write_could_have_reached_still_reports() {
    let f = findings("int probe(int i){ char ca[64]; ca[(i & 31) + 32] = 7; return ca[0]; }");
    assert!(
        f.iter().any(|s| s.contains("uninitialized-read")),
        "no write could reach byte 0, so reading it is a definite fault: {f:?}"
    );
}
