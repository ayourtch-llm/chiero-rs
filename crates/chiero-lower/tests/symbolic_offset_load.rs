//! **A load at a symbolic offset produces a value.**
//!
//! Three waves have hit the same wall: `Pointer::off` is a concrete `i64`, so a pointer with
//! a symbolic offset cannot be held in a `Value`. Everything follows from that —
//! `fork_on_offset` must enumerate offsets into concrete siblings, enumeration fails for any
//! unconstrained `int`, and the access then stops at "a load through a non-pointer address"
//! *even when the index is provably in range*:
//!
//! ```text
//!   int ga[64] = {7};  return ga[i & 63];    -> no value at all
//! ```
//!
//! `i & 63` cannot leave a 64-element array. There is nothing undecidable here; the engine
//! simply has no way to carry the pointer.
//!
//! `chiero-mem` has carried symbolic offsets since 021 §3 — `read_term_at` reads a byte at
//! one, with an if-then-else chain below `ITE_THRESHOLD` and a promoted `select` above it.
//! The engine cannot call it because it has no value type to pass.
//!
//! # This is an increment, and the boundary is deliberate
//!
//! A `Value` variant carrying `(ObjectId, Term)` is added, and the **load** path is taught to
//! use it. Every other site that matches `Value::Ptr` keeps today's behaviour: it does not
//! recognise the new variant and refuses, exactly as it refused the fresh symbol wave 195
//! handed it. That is what makes the change safe to make in one wave — a new variant does not
//! silently acquire pointer semantics anywhere, it acquires them only where written.

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

/// A provably in-range symbolic index reads a value.
#[test]
fn a_masked_index_reads_a_value() {
    let (f, vals) = run("char ca[64];\nint probe(int i){ return ca[i & 63]; }");
    assert!(
        vals.iter().any(|v| v.is_some()),
        "`i & 63` is inside a 64-byte array, so the read is answerable: {vals:?} {f:?}"
    );
}

/// And it reads the *right* value where every in-range byte is the same.
///
/// Asserted separately: handing back some arbitrary term would satisfy the test above. With
/// every element 7, the answer is 7 whichever offset the index picks — so a correct read is
/// pinned without needing to know which one it was.
#[test]
fn a_masked_index_reads_the_right_value() {
    let src = "char ca[64] = {7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,\
               7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,\
               7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,\
               7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7};\n\
               int probe(int i){ return ca[i & 63]; }";
    let (f, vals) = run(src);
    assert!(
        vals.contains(&Some(7)),
        "every in-range byte is 7, so the read is 7: {vals:?} {f:?}"
    );
}

/// A multi-byte element too, so the read is not byte-only.
///
/// **64 elements, not 4.** `i & 3` has four feasible values, which `fork_on_offset`
/// enumerates successfully — so a small array tests the *old* path and passes without the new
/// one existing. The mask has to exceed the enumeration bound for the fixture to reach the
/// code it is about, which is wave 193's lesson applied before mutation had to teach it again.
#[test]
fn a_masked_index_into_an_int_array_reads_a_value() {
    let sevens = ["7"; 64].join(",");
    let src = format!("int ga[64] = {{{sevens}}};\nint probe(int i){{ return ga[i & 63]; }}");
    let src = src.as_str();
    let (f, vals) = run(src);
    assert!(
        vals.contains(&Some(7)),
        "a 4-byte element at a symbolic offset: {vals:?} {f:?}"
    );
}

/// An unconstrained index is still reported and still refuses.
///
/// The control. Making loads work must not silence wave 193's fault, and must not invent a
/// value for an index that can leave the object.
#[test]
fn an_unconstrained_index_is_still_reported() {
    let (f, _) = run("char ca[64];\nint probe(int i){ return ca[i]; }");
    assert!(
        f.iter().any(|x| x.starts_with("pointer-outside-object")),
        "the index can leave the object and that is still said: {f:?}"
    );
}
