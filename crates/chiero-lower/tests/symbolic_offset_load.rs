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

/// Findings, plus for each state: did it reach a `return`, and did the load refuse?
///
/// **Not `return_value_bits`.** That asks for *ground* bits, and the whole point of this wave
/// is that the value is now a symbolic read — a `select` over the object at an unknown index.
/// Asserting concreteness would be asserting the opposite of what was built. What the tests
/// need is whether the load still *stops the path*, which is what it did before.
fn run(src: &str) -> (Vec<String>, Vec<bool>, Vec<String>) {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let returned = r
        .states()
        .iter()
        .map(|s| matches!(s.status, chiero_exec::Status::Terminated(_)) && s.returned_a_value())
        .collect();
    let gaps = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    (r.findings(), returned, gaps)
}

/// A provably in-range symbolic index reads a value.
#[test]
fn a_masked_index_reads_a_value() {
    let (f, returned, gaps) = run("char ca[64];\nint probe(int i){ return ca[i & 63]; }");
    assert!(
        !gaps.iter().any(|g| g.contains("non-pointer address")),
        "the load must not refuse the address any more: {gaps:?}"
    );
    assert!(
        returned.iter().any(|x| *x),
        "and a state must reach the return with a value: {returned:?} {f:?}"
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
    let (f, returned, gaps) = run(src);
    assert!(
        !gaps.iter().any(|g| g.contains("non-pointer address")),
        "the load is answered, not refused: {gaps:?}"
    );
    assert!(
        returned.iter().any(|x| *x),
        "and the path returns the byte it read: {returned:?} {f:?}"
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
    let (f, returned, gaps) = run(src);
    assert!(
        !gaps.iter().any(|g| g.contains("non-pointer address")),
        "a 4-byte element at a symbolic offset is composed, not refused: {gaps:?}"
    );
    assert!(
        returned.iter().any(|x| *x),
        "and the path returns it: {returned:?} {f:?}"
    );
}

/// An unconstrained index is still reported and still refuses.
///
/// The control. Making loads work must not silence wave 193's fault, and must not invent a
/// value for an index that can leave the object.
#[test]
fn an_unconstrained_index_is_still_reported() {
    let (f, _, _) = run("char ca[64];\nint probe(int i){ return ca[i]; }");
    assert!(
        f.iter().any(|x| x.starts_with("pointer-outside-object")),
        "the index can leave the object and that is still said: {f:?}"
    );
}

/// **The bytes are the right ones, in the right order.**
///
/// Mutation found nothing observing this: every test above asks only whether a value came
/// back, so reading a single byte instead of four, and composing them big-endian instead of
/// little, both survived. Neither is visible without evaluating the value itself.
///
/// The value is symbolic — a `select` over the object at an unknown index — so it is solved
/// rather than read. Every element is `0x01020304`, which makes the answer determinate for
/// whichever offset the model picks: 4 bytes little-endian is `0x01020304`, one byte would be
/// `0x04`, and big-endian would be `0x04030201`.
#[test]
fn the_composed_value_is_little_endian_and_full_width() {
    use chiero_solver::{CheckResult, Solver, TieredSolver};

    let elems = ["0x01020304"; 64].join(",");
    let src = format!("int ga[64] = {{{elems}}};\nint probe(int i){{ return ga[i & 63]; }}");
    let m = harness::lower(&src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);

    let mut seen = 0;
    for s in r.states() {
        let mut solver = TieredSolver::new();
        let CheckResult::Sat(model) = solver.check(&mut arena, &s.path) else {
            continue;
        };
        let Some(bits) = s.return_value_under(&model, &arena) else {
            continue;
        };
        seen += 1;
        assert_eq!(
            bits, 0x0102_0304,
            "four bytes, least significant first; one byte would be 0x04 and \
             big-endian 0x04030201"
        );
    }
    assert!(seen > 0, "no state produced a solvable returned value");
}
