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

/// And the value is usable, so the path continues.
///
/// Asserted separately from the finding above, because suppressing the report while still
/// handing back `Undef` would satisfy that test and leave the path dead — every access after
/// the index would go on producing nothing.
#[test]
fn the_path_continues_with_a_real_value() {
    let (_, vals) = run("int ga[64] = {7};\nint probe(int i){ return ga[i]; }");
    assert!(
        vals.iter().any(|v| v.is_some()),
        "some state must reach the return with a value: {vals:?}"
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
    assert!(
        vals.iter().any(|v| v.is_some()),
        "and the read produces a value: {vals:?}"
    );
}
