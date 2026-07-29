//! **An index the engine cannot enumerate is not an index it knows nothing about.**
//!
//! A *concrete* out-of-bounds access is a finding and has been for many waves. A symbolic
//! one is not:
//!
//! ```text
//!   int ga[2]; return ga[5];    out-of-bounds: 4-byte access at offset 20 of ga, ...
//!   int ga[2]; return ga[i];    (no finding)
//! ```
//!
//! The path is visible in the assumptions the run records:
//!
//! ```text
//!   BudgetHit:      a symbolic pointer offset was not enumerated: 17 value(s) found
//!                   and the search was cut short by the solver
//!   NoInformation:  `a load through a non-pointer address` is not modeled
//! ```
//!
//! `fork_on_offset` tries to *enumerate* the offset into concrete siblings, and an
//! unconstrained `int` has four billion of them. When that fails it yields `Undef`, the load
//! sees a non-pointer, and the access is abandoned — so the one question worth asking, *can
//! this index leave the object*, is never asked.
//!
//! It is answerable, and the machinery exists twice over. `chiero-mem` already implements a
//! feasibility check and has a `MemFault::OutOfBoundsMaybe` for exactly this, with tests in
//! `crates/chiero-mem/tests/bounds.rs`; and wave 156's `symbolic_div_by_zero` is the same
//! shape in the engine — ask the solver, `Sat` reports with a witness, `Unsat` reports
//! nothing, `Unknown` degrades.
//!
//! This is the third instance of one pattern: a check that answers only for concrete
//! operands and stays silent otherwise. Wave 175 was arithmetic, wave 192 the null call.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

fn reports_oob(src: &str) -> bool {
    findings(src).iter().any(|f| f.contains("bounds"))
}

/// An unconstrained index can leave the object, and that is worth saying.
#[test]
fn an_unconstrained_index_is_reported_as_possibly_out_of_bounds() {
    for (what, src) in [
        (
            "global array",
            "int ga[2] = {10,20};\nint probe(int i){ return ga[i]; }",
        ),
        (
            "local array",
            "int probe(int i){ int a[2] = {10,20}; return a[i]; }",
        ),
        (
            "write",
            "int ga[2] = {10,20};\nint probe(int i){ ga[i] = 1; return ga[0]; }",
        ),
    ] {
        let f = findings(src);
        assert!(
            reports_oob(src),
            "`{what}`: `i` is unconstrained, so the access can leave the object: {f:?}"
        );
    }
}

/// A guarded index is not.
///
/// The control, and the one that decides the fix must go through the solver rather than
/// report on every symbolic index. `if (i < 0 || i > 1)` leaves a path condition that makes
/// the out-of-bounds case unsatisfiable, and the engine already handles this correctly —
/// `fidelity Exact`, four states — so the machinery to tell the two apart is present.
#[test]
fn a_guarded_index_is_not_reported() {
    for (what, src) in [
        (
            "range check",
            "int ga[2] = {10,20};\nint probe(int i){ if (i<0||i>1) return 0; return ga[i]; }",
        ),
        (
            "mask",
            "int ga[2] = {10,20};\nint probe(int i){ return ga[i & 1]; }",
        ),
    ] {
        let f = findings(src);
        assert!(
            !reports_oob(src),
            "`{what}` constrains the index to the object: {f:?}"
        );
    }
}

/// And a concrete out-of-bounds access still reports, unchanged.
#[test]
fn a_concrete_out_of_bounds_access_still_reports() {
    assert!(
        reports_oob("int ga[2] = {10,20};\nint probe(void){ return ga[5]; }"),
        "the existing concrete check must survive"
    );
}

/// **Cases that reach the solver check**, which the guarded fixtures above do not.
///
/// Mutation exposed this: `if (i<0||i>1)` leaves only two feasible offsets, so
/// `fork_on_offset` *enumerates* them successfully and the new code never runs. Every mutant
/// on the query — reporting without asking, checking only one bound, comparing unsigned —
/// survived, because nothing exercised it with a constrained-but-unenumerable index.
///
/// A 64-element array does it: `i & 63` has sixty-four feasible values, past the enumeration
/// bound, so the offset arrives at the query already constrained to the object.
#[test]
fn a_constrained_but_unenumerable_index_is_not_reported() {
    let f = findings("int ga[64];\nint probe(int i){ return ga[i & 63]; }");
    assert!(
        !f.iter().any(|x| x.contains("bounds")),
        "`i & 63` cannot leave a 64-element array, however many values it has: {f:?}"
    );
}

/// An index that can only run off the **end**, and one that can only run off the **start**.
///
/// Both directions, because a check testing one bound answers correctly for half the
/// programs and silently for the other half.
///
/// It does **not** pin signedness, and that was worth finding out: with a lower bound of
/// zero the two signed comparisons are *equivalent* to one unsigned one, because a negative
/// two's-complement index is an enormous unsigned one and fails the upper test anyway.
/// `unsigned-comparison` is therefore an equivalent mutant, not a gap — and stops being one
/// only if a lower bound other than zero ever appears.
#[test]
fn each_direction_out_of_the_object_is_reported() {
    let above = findings("int ga[64];\nint probe(int i){ if (i < 0) return 0; return ga[i]; }");
    assert!(
        above.iter().any(|x| x.contains("bounds")),
        "non-negative but unbounded above: can run off the end: {above:?}"
    );
    let below = findings("int ga[64];\nint probe(int i){ if (i > 63) return 0; return ga[i]; }");
    assert!(
        below.iter().any(|x| x.contains("bounds")),
        "bounded above but not below: can run off the start: {below:?}"
    );
}
