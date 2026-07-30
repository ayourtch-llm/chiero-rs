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
    let (_, gaps) = run("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[0]; }");
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
    let (vals, _) = run("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[0]; }");
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
    let m = harness::lower("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[i & 63]; }");
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

/// **A concrete access after a symbolic one lands.**
///
/// §9's promotion warning, and the wave that answered it. A symbolic write promotes the
/// object to an array representation, and wave 197 shipped that knowing the cost: the
/// *concrete* store and load reached for the **arena-free** byte and bit APIs, which refuse a
/// promoted object (`promoted_fault`), so every ordinary access afterwards declined.
///
/// Wave 197 pinned the refusal deliberately, so this test would be the thing that changes
/// when it was fixed rather than a mystery rediscovered later. This is that change.
///
/// The store and the load are both here: a promoted object that can be written but not read,
/// or read but not written, is half-served in a way that shows up as a wrong value rather
/// than a refusal.
#[test]
fn a_concrete_store_after_a_symbolic_one_still_lands() {
    let (vals, gaps) =
        run("int probe(int i){ char ca[64]; ca[i & 63] = 7; ca[1] = 3; return ca[1]; }");
    assert!(
        vals.contains(&3),
        "the concrete store and the load after it must both survive promotion: \
         {vals:?} {gaps:?}"
    );
}

/// **A concrete byte written *before* promotion is still there after it.**
///
/// The reproduction §9 has carried since wave 200, now a test rather than a comment. It was
/// written then and withheld because it failed and the cause was unknown — but a failing test is
/// what a RED commit is, and a defect described in prose is one nobody's suite can observe.
///
/// ```text
///   ca[0] = 5;  ca[(i & 31) + 32] = 7;  return ca[0];   ->  solves to 0, not 5
/// ```
///
/// The symbolic write is masked into the *upper* half, so it cannot reach byte 0 whatever `i` is.
/// Byte 0 was written concretely before the object was promoted to an array, and reads back as
/// zero — **a wrong value, not a refusal**, which is the outcome 023 §7 forbids outright.
///
/// What waves 200 and 201 established, so the next reader does not re-establish it:
///
///   - **promotion seeds correctly** — `byte0=5`, `init0=Yes`, observed at the seeding loop
///   - **the read goes to the array**, on the right object, with `repr == Array`
///   - **`eval` does walk store chains for `Select`**, so a seeded store at index 0 would be found
///
/// **Wave 247 disproved the hypothesis those three led to.** §9 concluded that "the `arr.data` the
/// read sees does not contain the seeding stores", and said to check it by printing the term id at
/// promotion and at the read. Doing exactly that says otherwise:
///
/// ```text
///   PROMOTE-SET obj=ObjectId(4) data=Term(192) select0_ground=Some(5)
///   WRITE-3287  before=Term(192) after=Term(204)      <- the symbolic store, over the seeded array
///   READ        obj=ObjectId(4) data=Term(204)        <- the read sees that chain
///   READ-BYTE   obj=ObjectId(4) b=0 data=Term(204)    <- and builds select(Term(204), 0)
/// ```
///
/// The seeded array holds **5 at index 0** and folds to it without a solver; the symbolic store is
/// built over that array; the read consults the result and produces a term. Every link §9 doubted is
/// intact, and §9's own escape clause applies — it said to revisit only "if the term ids turn out to
/// match", and they do.
///
/// **What is actually happening is one layer up.** The engine reports
/// `a load produced no value, so its result is invented`, mints a *fresh unconstrained symbol* and
/// degrades to `Fidelity::Unknown`. So the returned byte is not a stale zero — it is a free
/// variable, and the solver is entitled to answer 0, or 5, or anything. "Solves to 0, not 5" was a
/// symptom of an unconstrained symbol being reported as a value, not of a stale array, and chasing
/// the array is what six waves of notes have been pointing at.
///
/// **This is wave 244's rule in `chiero-exec`**: a refusal upstream becoming an invented value
/// downstream. And the last link is now checked too, rather than guessed:
///
/// ```text
///   DISCARDED faults=["MaybeUninitialized"]
/// ```
///
/// The load does *not* get `None` from memory. It gets a value **and** a `MaybeUninitialized`
/// fault, and `r.value.filter(|_| !unusable(&r.faults))` throws the value away, because
/// `yields_unknown_value` lists that fault. The term it discards is the correct one.
///
/// **So the question is whether a `maybe` about *definedness* should discard a value about
/// *contents*.** They are different claims: the memory model knows byte 0 holds 5 — it folds to 5
/// with no solver — and is separately unsure whether reading it is defined, because the init chain
/// after a symbolic store is 512 nodes deep and `init_guard` gives up past `EXPAND_LIMIT`. Reporting
/// the `maybe` is right. Replacing the value with a free variable is what makes a program that
/// *has* an answer report a different one.
///
/// Not fixed here because `unusable` gates every `maybe`-uninitialized read in the engine, and
/// changing what it means is a wave with its own fixtures rather than a line at the end of this one.
///
/// `#[ignore]`d rather than left red, following `replay_coverage.rs`'s precedent for a known-blocked
/// defect: a runnable test that names the failure beats the prose comment it replaces, and a
/// permanently red suite is how a real regression goes unread.
#[ignore = "open: a MaybeUninitialized fault discards a value the memory model computed correctly, and the engine invents a symbol in its place — see the doc comment"]
#[test]
fn a_concrete_byte_written_before_promotion_survives_it() {
    use chiero_solver::{CheckResult, Solver, TieredSolver};
    let m = harness::lower(
        "int probe(int i){ char ca[64]; ca[0] = 5; ca[(i & 31) + 32] = 7; return ca[0]; }",
    );
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let mut vals: Vec<u128> = Vec::new();
    for st in r.states() {
        let mut solver = TieredSolver::new();
        if let CheckResult::Sat(model) = solver.check(&mut arena, &st.path)
            && let Some(bits) = st.return_value_under(&model, &arena)
        {
            vals.push(bits);
        }
    }
    let gaps: Vec<String> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    assert!(
        !vals.is_empty() && vals.iter().all(|v| *v == 5),
        "byte 0 was written before promotion and the symbolic write is masked into the upper \
         half, so nothing can have touched it: {vals:?} {gaps:?}"
    );
}

/// **A multi-byte element, stored and read back at the same symbolic offset.**
///
/// Mutation found no fixture needed more than one byte: every store fixture above used a
/// `char` array, so writing a single byte and ignoring the element size passed them all. An
/// `int` element makes the composition observable — one byte written would read back as
/// `0x00000007` only if the other three happened to be zero, and `0x01020304` cannot be
/// mistaken for any single byte of itself.
#[test]
fn a_multi_byte_store_at_a_symbolic_offset_round_trips() {
    use chiero_solver::{CheckResult, Solver, TieredSolver};
    let m = harness::lower(
        "int probe(int i){ int ga[64]; ga[i & 63] = 0x01020304; return ga[i & 63]; }",
    );
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
            "all four bytes were written, so all four read back"
        );
    }
    assert!(seen > 0, "no state produced a solvable returned value");
}

/// **The written byte is marked initialized.**
///
/// Mutation again: dropping the `init` store in `chiero-mem`'s unpinned write passed every
/// test, because none of them looked for the *finding*. Without it the read immediately after
/// the write reports an uninitialized read of the byte the program just stored — 021 §3.1's
/// distinction, and the same false positive wave 195 spent a wave removing in another guise.
#[test]
fn a_symbolic_store_marks_the_byte_initialized() {
    let m = harness::lower("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[i & 63]; }");
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let f = r.findings();
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "the program wrote this byte one statement earlier: {f:?}"
    );
}

/// **Promotion changes the representation, not the object's extent.**
///
/// Mutation found no fixture writing out of bounds *through* a promoted object: deleting the
/// bounds check from the promoted store passed everything. The concrete path checks bounds
/// before it reaches the array, so the check in the promoted branch is the only one there is —
/// and without it a store past the end silently lands in the SMT array at an index no object
/// byte corresponds to, which is worse than an unreported overflow because the value can be
/// read back.
#[test]
fn a_concrete_store_past_the_end_of_a_promoted_object_is_reported() {
    let m = harness::lower(
        "int probe(int i){ char ca[64]; ca[i & 63] = 7; ca[100] = 3; return ca[0]; }",
    );
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let f = r.findings();
    assert!(
        f.iter().any(|x| x.starts_with("out-of-bounds")),
        "byte 100 is outside a 64-byte object however the object is represented: {f:?}"
    );
}

/// **The init state a symbolic store leaves behind, asserted rather than assumed.**
///
/// Waves 197, 200 and 201 each wrote or fixed an `arr.init` store and none of them could be
/// observed: every mutant on that code survived. Wave 202 blamed the fixtures — file-scope
/// arrays are zero-initialized, so the init mask is all-`Yes` and nothing depends on it — and
/// rewriting them as locals *still* killed nothing. **The fixtures were necessary and not
/// sufficient; what was missing was an assertion that looks at the init finding at all.**
/// Every existing test here asks about a value or a refusal.
///
/// This is that assertion, and the verdict it pins is the interesting part. After
/// `ca[i & 63] = 7`, byte 0 was written *if and only if* the index chose it — so a concrete
/// read of byte 0 is 021 §3.1's middle state:
///
/// - `maybe-uninitialized-read` is correct, and is what the init store makes possible;
/// - `uninitialized-read` is what appears when the init store is missing or mis-indexed,
///   because the array's init mask then stays all-`No`;
/// - silence would be wrong in the other direction.
///
/// So the three outcomes distinguish the three states of the code, which is why this kills the
/// mutants the previous three waves could not.
#[test]
fn a_symbolic_store_leaves_the_touched_byte_maybe_initialized() {
    let m = harness::lower("int probe(int i){ char ca[64]; ca[i & 63] = 7; return ca[0]; }");
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let f = r.findings();
    assert!(
        f.iter().any(|x| x.starts_with("maybe-uninitialized-read")),
        "byte 0 was written only if the index chose it: {f:?}"
    );
    assert!(
        !f.iter().any(|x| x.starts_with("uninitialized-read")),
        "and the store did happen, so this is a maybe and not a definite: {f:?}"
    );
}

/// **A concretely written byte of a promoted object is *definitely* initialized.**
///
/// The assertion above pins the middle state and cannot see *which* bits an init store
/// marked — mutation showed that: indexing init by byte, or marking only the first bit of
/// eight, both leave the guard a `Cond` and so both still produce `maybe`. Only a byte that
/// must come out **definite** distinguishes them.
///
/// `ca[1] = 3` after promotion is that byte: written unconditionally, at a concrete offset, so
/// all eight of its init bits are set and a read of it is clean. With init indexed by byte only
/// one bit is marked and the other seven read as never-written; with only the first bit marked,
/// likewise. Either way the read reports, and this test fails.
#[test]
fn a_concretely_written_byte_of_a_promoted_object_is_definitely_initialized() {
    let m =
        harness::lower("int probe(int i){ char ca[64]; ca[i & 63] = 7; ca[1] = 3; return ca[1]; }");
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let f = r.findings();
    assert!(
        !f.iter().any(|x| x.contains("uninitialized")),
        "byte 1 was written unconditionally at a concrete offset: {f:?}"
    );
}
