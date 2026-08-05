//! **A clang `.gcno` is a different format, not a different number** (030 §4, contract 9).
//!
//! VPP's cmake defaults to clang, so this is the first thing a real tree presents. The version
//! tag is `*804` — 4.08, where gcc 13.3 writes `*33B` — and the layout behind it differs in four
//! ways that a decoder written for one cannot absorb by relaxing a version check:
//!
//! | | gcc 13.3 (`*33B`) | clang 18 (`*804`) |
//! |---|---|---|
//! | records begin at | `20 + cwd_len + 4` | **12** — no checksum word, no working directory |
//! | record length | bytes | **words** |
//! | string length | bytes including the NUL, unpadded | **words**, NUL-padded to the word |
//! | `FUNCTION` | ident, checksums, name, **artificial**, source, start line, **start col, end line, end col** | ident, checksums, name, source, start line |
//! | `BLOCKS` | payload is a count | payload is one word per block; the **length** is the count |
//! | `.gcda` trailer | `OBJECT_SUMMARY` | `PROGRAM_SUMMARY` |
//!
//! Measured from the bytes of `clg.gcno`, walked record by record, exactly as the gcc layout was.
//!
//! # Why this is worth a decoder rather than a fallback
//!
//! 030 §4 says an unknown version falls back to JSON — and gcc's own `gcov` refuses these files
//! ("version '408*', prefer 'B33*'"), so there is no JSON to fall back *to* unless the tree also
//! has `llvm-cov`. Refusing them correctly, which chiero already did, means refusing every object
//! of a default VPP build.
//!
//! The oracle here is `llvm-cov gcov`'s own `.gcov` text, committed beside the artifacts.

use chiero_gcov::TestId;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// The version decodes to 4.08 rather than to nothing, and is recognised.
#[test]
fn the_clang_version_tag_is_read_as_four_oh_eight() {
    let h = chiero_gcov::native::header(&corpus().join("clg.gcno")).expect("clg.gcno is readable");
    assert_eq!(h.version_tag(), "*804");
    assert_eq!(
        h.gcc_version(),
        Some((4, 8)),
        "the tag reads back to front as `408*`, so it is 4.08 — the letter-for-tens encoding gcc \
         13 uses does not apply below 10"
    );
    assert!(
        h.is_known(),
        "and it is a version this decoder has a fixture for"
    );
}

/// **The line counts match `llvm-cov gcov`'s own**, which is contract 5's gate applied to the
/// other compiler.
#[test]
fn clang_line_counts_match_llvm_covs_own() {
    let idx = chiero_gcov::ingest_native(&corpus(), "clg").expect("clg decodes");
    // From `clg.c.gcov`, committed beside the fixture.
    for (line, want) in [(6, 1), (8, 1), (9, 5), (10, 4), (11, 1), (15, 1), (17, 1)] {
        assert_eq!(idx.line_count("clg.c", line), Some(want), "clg.c:{line}");
    }
    // Line 7 is `{` and line 12 is `}` — llvm-cov records neither.
    assert_eq!(idx.line_count("clg.c", 7), None);
}

/// The loop line proves the cycle rule holds for clang's graphs too: entered once, four
/// iterations, reported as 5. A decoder that read the arcs wrongly would not land on it.
#[test]
fn the_cycle_rule_holds_for_clang_graphs() {
    let idx = chiero_gcov::ingest_native(&corpus(), "clg").expect("clg decodes");
    assert_eq!(idx.line_count("clg.c", 9), Some(5));
}

/// Both functions decode, with the names and start lines the FUNCTION record carries — which is
/// the record whose layout differs most.
#[test]
fn both_functions_decode_with_their_names_and_lines() {
    let cov = chiero_gcov::native::arc_coverage(&corpus(), "clg").expect("clg decodes");
    let mut keys: Vec<(String, u32)> = cov
        .functions()
        .into_iter()
        .map(|k| (k.name.clone(), k.start_line))
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![("f".to_string(), 6), ("main".to_string(), 15)],
        "a decoder that read gcc's extra FUNCTION fields would take the source name out of the \
         start line and every one of these would be wrong"
    );
}

/// Per-test attribution works the same way, so a clang tree gets the same queries.
#[test]
fn a_clang_object_attributes_to_a_test() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "clg").expect("clg as a test");
    assert_eq!(idx.tests_for_line("clg.c", 9), Some(vec![TestId(0)]));
    assert_eq!(idx.detail(), chiero_gcov::CoverageDetail::LinesAndArcs);
}

/// **A block nothing can reach still needs an answer, and the answer is zero.**
///
/// Found by pointing the decoder at a real clang `--coverage` build of `vppinfra`: **66 of 108
/// objects were refused**, every one with "arc N->M could not be determined; the spanning tree
/// guarantees it unless the data is corrupt". The data was fine. These graphs contain blocks with
/// *no incoming arcs at all* — `clib_bihash_copied` in this very fixture has one — whose outgoing
/// arc is on the spanning tree and therefore carries no counter.
///
/// Conservation cannot derive it, because there is nothing on the incoming side to conserve
/// against. The solver read that empty side as "this is the entry or exit block, which conserves
/// nothing" and gave up on the object. An unreachable block is a third case: no flow enters, so
/// no flow leaves, and every one of its arcs is zero.
///
/// # Why the fixture is a real object rather than a written one
///
/// Two attempts to synthesize the shape failed — code after a `return`, and code after a
/// `noreturn` call — because clang folds both away before it emits the graph. Whatever produces
/// an orphan block here is not reproducible by writing the obvious C, so the fixture is the
/// smallest object of the clang build that exhibits it, with `llvm-cov gcov`'s own output beside
/// it. That is the same reason `unrun` and `inl` were taken from a real tree.
#[test]
fn an_unreachable_block_is_zero_rather_than_unknown() {
    let idx = chiero_gcov::ingest_native(&corpus(), "clgdead")
        .expect("an orphan block is a count of zero, not a corrupt file");

    // llvm-cov records 13 lines of this source and executes none of them.
    let file = "/home/ubuntu/vpp/src/vppinfra/bihash_all_vector.c";
    let lines = idx.lines_of(file);
    assert_eq!(lines.len(), 13, "llvm-cov records 13 lines: {lines:?}");
    for line in &lines {
        assert_eq!(
            idx.line_count(file, *line),
            Some(0),
            "{file}:{line} — recorded and never executed, which is not the same as absent"
        );
    }
}
