//! **030 contract 13: `tests_for_block`, the bridge from coverage to the IR.**
//!
//! > `tests_for_block` on a CIR block whose `gcov_lines` are `{10, 11}` returns the union of the
//! > tests for lines 10 and 11.
//!
//! This is the join the whole differentiating claim runs through — 030 → 031 → 032. `gcov_lines`
//! was computed at lowering with `expansion_loc` ([015 §5]) precisely so that this union is a
//! correct join and not a coincidence: both sides name the line gcov actually recorded, so
//! matching them is an equality rather than a heuristic.
//!
//! # Why the file is a parameter
//!
//! 030 §5 sketches `tests_for_block(&self, sm: &SourceMap, b: &Block)`, deriving the file from
//! the block. That cannot be honoured, and 015 §5 says why twice over:
//!
//! - **The file key is the enclosing function's defining file**, not the block's. `gcov_lines` is
//!   "a bare `SmallVec<u32>` with no `FileId`, so the file is implicit" — a property of the
//!   function, which a `&Block` does not carry.
//! - **Hand-written `.cir` fixtures have no `SourceMap`**, so their spans are `Span::DUMMY` and
//!   the `.line` directive populates `gcov_lines` directly. 015 §5 states that this exists so
//!   that "M1's fixtures could not exercise 030 contract 13 at all" without it — so contract 13
//!   must be answerable for a block with no resolvable span, and a signature that derives the
//!   file from the span cannot answer it.
//!
//! Deriving it from the block's span would happen to work for lowered CIR, because chiero does
//! not inline and so every instruction of a function resolves to its defining file. Working for
//! the wrong reason is what this crate has already paid for once.

use chiero_cir::{Block, BlockId, Terminator, UnreachableReason};
use chiero_gcov::TestId;
use chiero_span::Span;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// A block carrying exactly the given `gcov_lines`, with no span — the shape a `.cir` fixture
/// produces, and the one contract 13 has to answer for.
fn block(lines: &[u32]) -> Block {
    Block {
        id: BlockId(0),
        insts: Vec::new(),
        term: Terminator::Unreachable(UnreachableReason::BuiltinUnreachable),
        gcov_lines: lines.iter().copied().collect(),
        span: Span::DUMMY,
    }
}

/// `loop.c` under two tests, so a union is distinguishable from either half.
fn index() -> chiero_gcov::CoverageIndex {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "loop").expect("loop decodes");
    chiero_gcov::ingest_native_as(&mut idx, TestId(1), &corpus(), "t").expect("t decodes");
    idx
}

/// **Contract 13.** The union over the block's lines.
#[test]
fn a_blocks_tests_are_the_union_over_its_lines() {
    let idx = index();
    let lines = idx.lines_of("loop.c");
    assert!(
        lines.len() >= 2,
        "the fixture needs two recorded lines to make a union mean anything: {lines:?}"
    );

    let b = block(&lines);
    let mut expected: Vec<TestId> = Vec::new();
    for l in &lines {
        for t in idx.tests_for_line("loop.c", *l).unwrap_or_default() {
            if !expected.contains(&t) {
                expected.push(t);
            }
        }
    }
    expected.sort_unstable_by_key(|t| t.0);
    assert_eq!(idx.tests_for_block("loop.c", &b), Some(expected));
}

/// A line of one file is not a line of another. The union is keyed, not positional.
#[test]
fn the_file_is_part_of_the_question() {
    let idx = index();
    let b = block(&idx.lines_of("loop.c"));
    assert_eq!(
        idx.tests_for_block("nosuch.c", &b),
        None,
        "the same line numbers in a file nothing recorded are not coverage"
    );
}

/// **A block with no `gcov_lines` answers `None`.**
///
/// 015 §5: a block of only compiler-generated instructions has an empty `gcov_lines`, "and that
/// is correct — gcov has no counter for it either". So there is no evidence about it, which is
/// not the same as evidence that nothing ran it. An empty answer here would let 032 skip every
/// test for a change to such a block.
#[test]
fn a_generated_block_has_no_evidence_either_way() {
    assert_eq!(
        index().tests_for_block("loop.c", &block(&[])),
        None,
        "no lines is no evidence, not an empty set of tests"
    );
}

/// A block naming lines the index never recorded answers `None` rather than an empty union.
#[test]
fn lines_with_no_record_answer_nothing() {
    assert_eq!(index().tests_for_block("loop.c", &block(&[9998, 9999])), None);
}

/// A block naming one recorded line and one unrecorded line answers for the one there is
/// evidence about — a partial join is still a join, and dropping it would lose real coverage.
#[test]
fn a_partly_recorded_block_answers_for_what_is_recorded() {
    let idx = index();
    let known = idx.lines_of("loop.c")[0];
    assert_eq!(
        idx.tests_for_block("loop.c", &block(&[known, 9999])),
        idx.tests_for_line("loop.c", known)
    );
}
