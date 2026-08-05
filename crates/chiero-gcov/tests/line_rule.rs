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

/// **The attribution is to the greatest line of a block's group, not the last one written.**
///
/// gcov sorts each block's line list before anything reads it (`gcc/gcov.cc` ~1413, in the pass
/// that sizes each source's line vector), and only then attributes the block to the group's last
/// entry. The two are the same for straight-line code and differ wherever a call is inlined: the
/// block holding the call at `nonmono.c:21` also carries the callee's lines 10–13, so it reads
/// `[21, 10, 11, 12, 13]` and its last entry is 13 — a line inside a function that block is not
/// in.
///
/// Getting this wrong is quiet. The block lands on some lower line, the line it belongs to is
/// left with no blocks at all and falls back to the accumulated sum, and both numbers stay
/// plausible. On `compress.h:90` of a real VPP build it reports 30 against gcov's 10.
///
/// It is pinned here on the decode rather than on a count because a count hides it: two blocks of
/// equal weight make the sum and the graph answer agree, which is exactly what this fixture's own
/// line 21 does.
#[test]
fn a_blocks_lines_are_sorted_as_gcov_sorts_them() {
    let n = chiero_gcov::native::read_notes(&corpus().join("nonmono.gcno")).expect("nonmono.gcno");
    let main = n
        .functions
        .iter()
        .find(|f| f.name == "main")
        .expect("`main` is in the fixture");
    let group = main
        .lines
        .iter()
        .find(|bl| bl.lines.contains(&21) && bl.lines.contains(&10))
        .expect("the block holding the inlined call at line 21 carries the callee's lines too");
    assert_eq!(
        group.lines,
        vec![10, 11, 12, 13, 21],
        "the block belongs to line 21, the greatest line it carries"
    );
}

/// **A source's lines are accounted once per object, not once per function.**
///
/// gcov keeps one record per `(source, line)` for the whole object: every function's blocks
/// accumulate into it, every function's attributed blocks join one block list, and the graph
/// count is computed over that union (`add_line_counts` writes into `sources[src].lines`, and
/// `accumulate_line_info` overwrites it once). Arcs never cross functions, so the union's count
/// is the sum of the per-function counts.
///
/// `multi.h`'s `bump` is force-inlined into both `one` and `two`, so both contribute to its lines
/// and gcov reports 2. Computing per function and merging by `max` reports 1 — one caller's
/// count, with the other silently dropped. On a real VPP object the same defect reports 310 for
/// `memcpy_x86_64.h:42` where the three callers sum to gcov's 410.
///
/// The direction of the error is the dangerous one. 032 skips a test when a line's coverage says
/// another test already reached it, and a count that is missing a caller is a line that looks
/// less covered than it is — or, where the dropped caller is the only one that ran, a line that
/// looks uncovered.
#[test]
fn every_function_of_an_object_contributes_to_a_shared_line() {
    assert_eq!(
        count("multi", "multi.h", 5),
        Some(2),
        "`one` and `two` each inline `bump` once"
    );
    assert_eq!(count("multi", "multi.h", 6), Some(2));
}
