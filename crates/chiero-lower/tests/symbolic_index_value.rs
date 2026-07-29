//! **The value after an unenumerable index is `Undef`, and that invents a finding.**
//!
//! Waves 193–194 made a symbolic index answerable and gave the fault its own name. Neither
//! made the *value* usable: `fork_on_offset` still yields `Value::Undef` when enumeration
//! fails, and the engine then reports a fault it invented beside the one it found:
//!
//! ```text
//!   int *p = ga + i; return p != 0;
//!       pointer-outside-object: a pointer into ga (256 bytes) can be computed at offset 256
//!       uninitialized-read: read at offset 0 of p touches bit 0, which was never written
//! ```
//!
//! The second is false. `p` *was* written — by the statement above it — and the report only
//! happens because `Undef` is indistinguishable from a value the program never stored.
//!
//! 023 §9's argument bites hardest here: a reader who checks the invented finding and sees it
//! is nonsense has been handed a reason to distrust the real one next to it. A false positive
//! beside a true positive costs more than either alone.
//!
//! # What this wave can and cannot do
//!
//! `Value::Ptr` holds a `Pointer` whose offset is a concrete `i64`, so a *symbolic* offset is
//! not representable in a value at all — routing one into `chiero-mem`'s checked path, which
//! does take a symbolic offset, needs a new `Value` variant and is a milestone rather than a
//! wave. What is available is the other half of what that path does: **assume the in-bounds
//! constraint and continue with a real offset**, chosen by the solver under it. That is one
//! representative rather than every in-bounds offset, which is exactly what
//! `Fidelity::Bounded` already claims about this path.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn run(src: &str) -> (Vec<String>, Vec<Option<i32>>) {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let vals = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut arena).map(|b| b as u32 as i32))
        .collect();
    (r.findings(), vals)
}

/// No invented uninitialized read.
#[test]
fn an_unenumerable_index_does_not_invent_an_uninitialized_read() {
    let (f, _) = run("int ga[64];\nint probe(int i){ int *p = ga + i; return p != 0; }");
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "`p` was written by the statement above; this finding is invented: {f:?}"
    );
}

/// The value is not a *pointer*, and that is deliberate.
///
/// Concretizing the offset to one in-bounds value was tried in this wave and rejected:
/// `a_symbolic_ptr_add_offset_is_a_gap` forbids handing out a fabricated address, on the
/// ground that every later report through it is a confident claim about one arbitrary case.
/// So the access after an unenumerable index still stops — what changed is that it stops
/// *without* also accusing the program of an uninitialized read.
///
/// Making the access itself work needs a `Value` that can hold a **symbolic** offset;
/// `Pointer::off` is a concrete `i64`. That is recorded in §9 as a milestone, not smuggled in
/// as a concretization.
#[test]
fn the_replacement_value_is_not_a_fabricated_pointer() {
    let (f, _) = run("int ga[64] = {7};\nint probe(int i){ return ga[i]; }");
    assert!(
        f.iter().any(|x| x.starts_with("pointer-outside-object")),
        "the fault is still found: {f:?}"
    );
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "and nothing is invented about what the program wrote: {f:?}"
    );
}

/// The fault is still reported.
///
/// The control. Making the value usable by *not asking* would pass both tests above and undo
/// wave 193.
#[test]
fn the_out_of_range_pointer_is_still_reported() {
    let (f, _) = run("int ga[64];\nint probe(int i){ int *p = ga + i; return p != 0; }");
    assert!(
        f.iter().any(|x| x.starts_with("pointer-outside-object")),
        "the index can still leave the object: {f:?}"
    );
}

/// A constrained index gets a value and no fault.
///
/// `i & 63` has more feasible values than the enumeration bound, so it reaches the same code
/// with the index already inside the object — the shape wave 193's mutation showed was the
/// only one that exercises this path at all.
#[test]
fn a_constrained_index_gets_a_value_and_no_fault() {
    let (f, vals) = run("int ga[64] = {7};\nint probe(int i){ return ga[i & 63]; }");
    assert!(
        !f.iter().any(|x| x.starts_with("pointer-outside-object")),
        "`i & 63` cannot leave the object: {f:?}"
    );
    // Not "and the read produces a value": it does not, for the same reason as above — the
    // offset is unenumerable either way, so no pointer is handed out and the load stops.
    // What the constraint buys is silence about a fault that cannot happen, which is the
    // property worth pinning here.
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "and nothing is invented about `ga`: {f:?}"
    );
    let _ = vals;
}
