//! **A store at a symbolic offset is dropped, and later reads then lie.**
//!
//! Wave 196 gave `Value::SymPtr` a load path. The store side still refuses — and refusing is
//! not what happens to the *program*:
//!
//! ```text
//!   char ca[64];
//!   ca[i & 63] = 7;   ->  "a store through a non-pointer address" is not modeled
//!   return ca[0];     ->  0
//! ```
//!
//! The write is discarded, and the read afterwards answers **0** for a byte the write may
//! have hit. The run degrades its fidelity, so it does not *claim* to be exact — but a reader
//! is still handed a value that is wrong whenever `i & 63` is 0, which is one case in
//! sixty-four. 021 §3.1 calls a confidently wrong byte the single most common way a symbolic
//! executor misleads, and a dropped store is how it happens.
//!
//! `chiero-mem` has `write_at_symbolic_offset`, which either writes an if-then-else over
//! candidates or promotes the object to an array — 021 §3's `ITE_THRESHOLD` decides. §9's
//! standing warning is that a promoted object refuses the *arena-free* byte and bit APIs
//! (`promoted_fault`), so what matters is whether the paths used after a symbolic write
//! thread an arena. They do; the tests below are what prove it.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

/// Every state's returned bits, plus the recorded gaps.
fn run(src: &str) -> (Vec<u128>, Vec<String>) {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let vals = r
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut arena))
        .collect();
    let gaps = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    (vals, gaps)
}

/// The store is not refused.
#[test]
fn a_store_at_a_symbolic_offset_is_not_refused() {
    let (_, gaps) = run("char ca[64];\nint probe(int i){ ca[i & 63] = 7; return ca[0]; }");
    assert!(
        !gaps
            .iter()
            .any(|g| g.contains("store through a non-pointer")),
        "the address is a pointer with an unknown offset, not a non-pointer: {gaps:?}"
    );
}

/// And a later read does not claim a byte the write may have reached.
///
/// The property that matters. `ca[0]` answering **0** is wrong whenever `i & 63` is 0, and
/// this asserts the engine no longer says it — either by reading the written value back
/// symbolically, or by declining. What it must not do is answer a confident 0.
#[test]
fn a_read_after_a_symbolic_store_does_not_claim_zero() {
    let (vals, _) = run("char ca[64];\nint probe(int i){ ca[i & 63] = 7; return ca[0]; }");
    assert!(
        !vals.contains(&0),
        "the write may have hit byte 0, so a ground 0 is a claim the run cannot make: {vals:?}"
    );
}

/// Reading back the byte just written gets what was written.
///
/// The positive half: whatever offset the index picks, the store put 7 there and the read is
/// at the same offset, so the answer is 7 for every model. A store that promoted the object
/// but wrote nothing would satisfy the two tests above and fail this.
#[test]
fn reading_back_the_symbolic_offset_gets_the_written_value() {
    use chiero_solver::{CheckResult, Solver, TieredSolver};
    let m = harness::lower("char ca[64];\nint probe(int i){ ca[i & 63] = 7; return ca[i & 63]; }");
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
        assert_eq!(bits, 7, "the read is at the offset the store wrote");
    }
    assert!(seen > 0, "no state produced a solvable returned value");
}

/// A concrete access after a symbolic one still works.
///
/// This is §9's promotion warning made into a test: if the symbolic write promotes the object
/// to an array representation, an ordinary byte write afterwards must still land.
#[test]
fn a_concrete_store_after_a_symbolic_one_still_lands() {
    let (vals, gaps) =
        run("char ca[64];\nint probe(int i){ ca[i & 63] = 7; ca[1] = 3; return ca[1]; }");
    assert!(
        vals.contains(&3),
        "the concrete store must survive promotion: {vals:?} {gaps:?}"
    );
}
