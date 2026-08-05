//! **030 contract 5, the part that was guessed: how a line's count is computed from its blocks.**
//!
//! A line is compiled into several basic blocks and gcov reports one number for it. Every
//! aggregation of the block counts is wrong, and this file is the record of finding that out the
//! expensive way.
//!
//! | rule | `loop.c:1` `[1,4,5,1,1]` | `inl.c:2` `[1,1,1]` | `cyc.c:5` `[1,4,5,1,1]` |
//! |---|---|---|---|
//! | truth (gcov) | 5 | 3 | 5 |
//! | `max` | 5 ✓ | 1 ✗ | 5 ✓ |
//! | `sum` | 12 ✗ | 3 ✓ | 12 ✗ |
//! | entry counts only | 1 ✗ | 3 ✓ | 1 ✗ |
//!
//! `max` was implemented because it fit `loop.c`, the only fixture that existed. It survived a
//! second fixture, a third, and a cross-validation run against 92,920 real `(file, line)` rows
//! from a VPP coverage build, where it was *closer* than every competing formula — 67 of 98
//! objects agreeing, against 63 for `sum`. Being the best of the wrong answers is what made it
//! durable.
//!
//! # The rule
//!
//! From `gcc/gcov.cc`, `accumulate_line_info`, in gcc's own words:
//!
//! > The user expects the line count to be the number of times a line has been executed. Simply
//! > summing the block count will give an artificially high number. The Right Thing is to sum the
//! > entry counts to the graph of blocks on this line, then find the elementary cycles of the
//! > local graph and add the transition counts of those cycles.
//!
//! So it is not an aggregation at all, it is a graph computation on the subgraph induced by the
//! line's blocks:
//!
//! 1. every arc entering that subgraph from outside it contributes its count;
//! 2. every elementary cycle *within* it contributes its bottleneck — the minimum `cs_count`
//!    along the cycle, which is then subtracted from each of its arcs so that a shared arc is
//!    not counted twice (`handle_cycle`).
//!
//! Cycles are enumerated by Hawick and James' algorithm (`circuit`/`unblock`), which is the part
//! no amount of curve-fitting would have produced: a loop written entirely on one line is counted
//! by *finding the loop*, not by looking at any block's counter.
//!
//! # Why this could not have been fitted
//!
//! It is worth naming, because the instinct to fit was strong and the data set was large. The
//! 92,920 rows contain the counts of the blocks and the answer, and no function of the former
//! yields the latter, because the answer depends on the *arcs* — which the block counts do not
//! determine. The measurement could only ever have said "still wrong", never "wrong in this way".
//! 030 §4 asks for behavioural validation against gcov for exactly this reason, and the discipline
//! that paid here was reading the algorithm instead of fitting a fifth formula to the residue.

use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn count(stem: &str, file: &str, line: u32) -> Option<u64> {
    chiero_gcov::ingest_native(&corpus(), stem)
        .expect("the fixture decodes")
        .line_count(file, line)
}

/// **The cycle term is not zero.** `cyc.c:5` holds an entire `for` loop, so its blocks form a
/// cycle taken four times, entered once. An implementation that sums only the arcs entering the
/// subgraph — the first half of the rule, and the obvious half — answers 1.
#[test]
fn a_loop_written_on_one_line_counts_its_iterations() {
    assert_eq!(
        count("cyc", "cyc.c", 5),
        Some(5),
        "entered once, looped four times: the entry arc plus the elementary cycle"
    );
}

/// **The rule is not `max`.** `inl.c:2`'s three blocks each run once and are each entered from
/// off the line, so the entry counts sum to 3. The header line the same object attributes two
/// blocks to says 2 for the same reason, and is here because a rule that special-cased the
/// three-block shape would still get it wrong.
#[test]
fn blocks_entered_separately_add_up() {
    assert_eq!(count("inl", "inl.c", 2), Some(3));
    assert_eq!(count("inl", "inl.h", 3), Some(2));
}

/// **The rule is not `sum`.** `loop.c:1`'s blocks total 12, and the arcs between them are
/// internal to the line, so they are not entries and do not contribute.
#[test]
fn arcs_inside_the_line_are_not_entries() {
    assert_eq!(
        count("loop", "loop.c", 1),
        Some(5),
        "the blocks sum to 12; only what enters the line from outside it counts, plus its cycles"
    );
}

/// The straight-line case stays right, which is the one every rule agrees on and therefore the
/// one that proves nothing on its own — it is here so that a rewrite cannot trade it away.
#[test]
fn a_line_with_one_block_is_that_blocks_count() {
    assert_eq!(count("t", "t.c", 3), Some(1));
}
