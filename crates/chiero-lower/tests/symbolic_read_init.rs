//! **A symbolic read of memory nobody wrote is silent once the object is promoted.**
//!
//! `read_term_at`'s promoted branch returns `select(arr.data, i)` with `faults: vec![]`. The
//! object's `init` array is right there and never consulted, so an uninitialized read
//! disappears the moment the candidate set outgrows `ITE_THRESHOLD`:
//!
//! ```text
//!   char ca[64]; return ca[i & 15];   8 findings ... uninitialized-read      (Bytes, ite chain)
//!   char ca[64]; return ca[i & 31];   []                                     (promoted to Array)
//! ```
//!
//! Those are the same program with one bit of mask difference, and 021 §3 says promotion is a
//! *representation* change. A representation change that loses a finding is the thing 023 §7
//! forbids without a fidelity drop, and there is none here — the run reports `Exact` and says
//! nothing.
//!
//! # Why this is the fault and not the fixture
//!
//! The concrete side is not merely a different code path, it is the *same fault reported
//! correctly*: below the threshold the read forks per candidate and each fork's concrete read
//! consults the init mask. Nothing about a symbolic index makes the answer unavailable — the
//! guard is `select(arr.init, off * 8 + k)` for the eight bits of the byte, and the engine has
//! had somewhere to send it since wave 204's discharge.
//!
//! # The objection this has to answer
//!
//! Wave 202 built this check and rejected it: a byte written at the *same* symbolic offset
//! came back `maybe-uninitialized-read`, because a read of bit `k` had to walk past seven
//! stores whose symbolic indices it could not compare, so only the outermost folded. A false
//! `maybe` on memory the program definitely wrote is worse than silence, and that judgement
//! was right.
//!
//! What changed is where the guard goes. Wave 202 needed it to fold in the arena; wave 204
//! made an unresolved guard a *solver* question, and wave 205 eliminates the array from the
//! question entirely — `select_expand` turns the chain into `ite` comparisons, so what reaches
//! the solver is bitvector arithmetic and no walk has to fold anything. So the check may now
//! emit what it could not previously prove. `symbolic_readback_init.rs` holds the three fixtures that fail
//! if this reasoning is wrong, and they are the controls for this file.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

fn reports_uninit(src: &str) -> bool {
    findings(src)
        .iter()
        .any(|f| f.contains("uninitialized-read"))
}

/// **The threshold must not decide whether a fault exists.**
///
/// `i & 15` yields sixteen candidates and stays in the `Bytes` representation; `i & 31` yields
/// thirty-two and promotes. Asserting both in one test is the point: the pair is what makes
/// this a lost finding rather than a limit, and a fix that reported on neither would still
/// satisfy a test that only asked about the promoted side.
#[test]
fn an_uninitialized_symbolic_read_reports_on_both_sides_of_the_threshold() {
    let below = "int probe(int i){ char ca[64]; return ca[i & 15]; }";
    assert!(
        reports_uninit(below),
        "below the threshold this already works, and is the control that the fault is real"
    );
    let above = "int probe(int i){ char ca[64]; return ca[i & 31]; }";
    assert!(
        reports_uninit(above),
        "the same read of the same never-written array, one mask bit wider: {:?}",
        findings(above)
    );
}

/// A read that lands *only* on never-written bytes, in an object that has been written.
///
/// Separate from the case above because promotion there is triggered by an unwritten object
/// with no stores at all, and a fix keyed on "the array has no stores" would pass it. Here the
/// program writes byte 0 and reads somewhere in 32..64, so `arr.init` has a store on it and
/// the guard still has to come out false.
#[test]
fn a_symbolic_read_of_an_unwritten_range_reports_in_a_written_object() {
    let src = "int probe(int i){ char ca[64]; ca[0] = 5; return ca[(i & 31) + 32]; }";
    assert!(
        reports_uninit(src),
        "no store reached bytes 32..64, so every byte this read can name is uninitialized: {:?}",
        findings(src)
    );
}

/// A symbolic read of a range a symbolic write *could not* have reached.
///
/// The write covers 0..32 and the read covers 32..64, both symbolically. Neither side is a
/// concrete offset, so this is the case that needs the guard to be resolved against the path
/// rather than inspected syntactically — and it is the one that separates a real check from one
/// that gives up whenever either index is a term.
#[test]
fn a_symbolic_read_disjoint_from_a_symbolic_write_reports() {
    let src = "int probe(int i){ char ca[64]; ca[i & 31] = 7; return ca[(i & 31) + 32]; }";
    assert!(
        reports_uninit(src),
        "the write can only land in 0..32 and the read only in 32..64: {:?}",
        findings(src)
    );
}

/// **All eight bits of the byte, not just the first.**
///
/// `arr.init` is bit-indexed and the read is byte-wide, so the guard is a conjunction over
/// eight bits. Checking one of them survives every other test in the tree, because C gives a
/// program almost no way to write part of a byte — the exception is a bit-field, which is why
/// this fixture reaches for one.
///
/// `sa[0].f = 1` writes bit 0 of byte 0 and nothing else. Every byte of `sa` therefore has at
/// least seven bits nobody wrote, so the guard is false for every offset the read can name and
/// the verdict is **definite**. A guard that consults only bit 0 finds it set at offset 0,
/// cannot refute itself, and degrades to `maybe` — the same fault, reported as weaker than it
/// is. Asserting the *kind* rather than a substring is what makes the two distinguishable
/// (`maybe-uninitialized-read` contains `uninitialized-read`).
#[test]
fn the_guard_covers_every_bit_of_the_byte() {
    if !harness::backend_or_skip("the_guard_covers_every_bit_of_the_byte") {
        return;
    }
    let src = "struct S { unsigned char f:1; };\n\
               int probe(int i){ struct S sa[40]; sa[0].f = 1;\n\
               char *p = (char *)sa; return p[i & 31]; }";
    let f = findings(src);
    assert!(
        f.iter().any(|s| s.starts_with("uninitialized-read")),
        "seven bits of every byte were never written, so this is definite, not a maybe: {f:?}"
    );
}
