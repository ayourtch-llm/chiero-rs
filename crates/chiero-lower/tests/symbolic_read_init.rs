//! **A read at a symbolic offset never checks whether the bytes were written.**
//!
//! The concrete read has reported uninitialized memory since 021 §3.1 was implemented. The
//! symbolic one does not check at all:
//!
//! ```text
//!   char ca[64]; return ca[0];        uninitialized-read: read at offset 0 of ca ...
//!   char ca[64]; return ca[i & 63];   (nothing)
//! ```
//!
//! Same object, same never-written bytes, and the index is provably inside it. 021 §3.1 calls a
//! confidently-wrong byte the single most common way a symbolic executor misleads, and a read
//! that does not ask is the way one arrives.
//!
//! # Why four waves of tests could not see this
//!
//! Every fixture in `symbolic_offset_store.rs` uses a **file-scope** array — `char ca[64];`
//! outside a function — which C zero-initializes (6.7.9p10). Its `init` mask is all-`Yes` from
//! the moment it exists, so no init read or write can change any answer, and every mutation on
//! the init-marking code survived. That is what made waves 197 and 201's init fixes look
//! untestable, and §9's explanation — "the promoted read does not consult `arr.init`" — was
//! wrong: `init_bit_via` selects from it correctly.
//!
//! The fixtures here declare the array **inside** `probe`, so its bytes are genuinely
//! indeterminate and the init state is observable.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

/// The gap: a symbolic read of never-written bytes says nothing.
#[test]
fn a_symbolic_read_of_uninitialized_memory_is_reported() {
    let f = findings("int probe(int i){ char ca[64]; return ca[i & 63]; }");
    assert!(
        f.iter().any(|x| x.contains("uninitialized")),
        "no byte of `ca` was ever written, whichever offset `i & 63` picks: {f:?}"
    );
}

/// The concrete read still reports, unchanged.
///
/// The control that keeps the fix from being "stop reporting concrete ones too".
#[test]
fn a_concrete_read_of_uninitialized_memory_still_reports() {
    let f = findings("int probe(void){ char ca[64]; return ca[0]; }");
    assert!(
        f.iter().any(|x| x.starts_with("uninitialized-read")),
        "the existing concrete check must survive: {f:?}"
    );
}

/// A symbolic read of bytes the program *did* write is not reported.
///
/// The other control, and the one that makes the wave-197 and wave-201 init writes matter: the
/// symbolic store marks the byte initialized, and this read is at the same offset, so there is
/// nothing to report. A fix that reported every symbolic read would pass the first test and
/// fail here.
#[test]
fn a_symbolic_read_of_written_memory_is_not_reported() {
    let f = findings("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[i & 63]; }");
    assert!(
        !f.iter().any(|x| x.contains("uninitialized")),
        "the store one statement earlier wrote exactly this byte: {f:?}"
    );
}

/// And a *partially* written object reports the weaker verdict, not the stronger one.
///
/// `ca[0]` after a symbolic write may or may not have been written, and 021 §3.1's tri-state
/// exists for exactly this: `maybe-uninitialized-read` rather than `uninitialized-read`.
/// Asserted because collapsing the two is the tempting simplification, and it turns a
/// "possibly" into an accusation.
#[test]
fn a_partially_written_object_reports_the_weaker_verdict() {
    let f = findings("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[0]; }");
    assert!(
        f.iter().any(|x| x.starts_with("maybe-uninitialized-read")),
        "byte 0 was written only if the index chose it: {f:?}"
    );
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "and that is a maybe, not a definite: {f:?}"
    );
}
