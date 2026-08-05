//! **030 §4: what is *inside* the records, decoded and pinned against the fixture.**
//!
//! Every field below was read out of `t.gcno` with a hex dump before a line of decoder existed,
//! which is the method §4 asks for. Two of them are not what a reader would guess:
//!
//! - **A string is a length and that many bytes, with no padding.** `"main\0"` is 5 bytes and the
//!   next field starts 5 bytes later, at an offset divisible by nothing. The record stream is
//!   byte-aligned throughout.
//! - **`FUNCTION` carries an `artificial` flag between the name and the source file.** It reads 0
//!   for both functions here, so a decoder that omits it stays in step for exactly as long as it
//!   takes to meet a compiler-generated function.
//!
//! `LINES` is a small grammar rather than a record of fields: a block number, then a stream where
//! a **0 introduces a file name** and any other value is a line, ending with a 0 and an empty
//! string. `t.c`'s three covered blocks each carry `0 "t.c" 3 0 0` — the same line, three times,
//! which is what "a macro expanded twice lands on one line" looks like from inside the format.

use std::path::PathBuf;

use chiero_gcov::native::{ArcFlags, Note};

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn notes() -> Note {
    chiero_gcov::native::read_notes(&corpus().join("t.gcno")).expect("t.gcno decodes")
}

/// Both functions, with the identity `FuncKey` will be built from.
#[test]
fn the_functions_carry_their_name_source_and_extent() {
    let n = notes();
    let names: Vec<&str> = n.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["main", "hdr_fn"]);

    let main = &n.functions[0];
    assert_eq!(main.source, "t.c");
    assert_eq!((main.start_line, main.start_column), (3, 5));
    assert_eq!((main.end_line, main.end_column), (3, 88));
    assert!(!main.artificial);
    assert_eq!(main.ident, 0x067072eb);

    // **The header function is its own entry, in its own file.** That is the other half of the
    // macro-attribution fact: a `static inline` is attributed to where it is *defined*.
    let hdr = &n.functions[1];
    assert_eq!(hdr.source, "m.h");
    assert_eq!((hdr.start_line, hdr.start_column), (2, 19));
}

/// Blocks and arcs, with the flags 030 §4 names.
#[test]
fn the_cfg_reads_back_with_its_arc_flags() {
    let n = notes();
    assert_eq!(n.functions[0].blocks, 6);
    assert_eq!(n.functions[1].blocks, 4);

    let arcs: Vec<(u32, u32, ArcFlags)> = n.functions[0]
        .arcs
        .iter()
        .map(|a| (a.from, a.to, a.flags))
        .collect();
    assert_eq!(
        arcs,
        vec![
            (0, 2, ArcFlags::FALLTHROUGH),
            (2, 3, ArcFlags::FALLTHROUGH),
            (2, 1, ArcFlags::ON_TREE | ArcFlags::FAKE),
            (3, 4, ArcFlags::ON_TREE | ArcFlags::FALLTHROUGH),
            (3, 1, ArcFlags::ON_TREE | ArcFlags::FAKE),
            (4, 5, ArcFlags::FALLTHROUGH),
            (5, 1, ArcFlags::ON_TREE),
        ]
    );
    // **The `FAKE` arcs go to the exit block**, from calls that may not return. 030 §4.1 keeps
    // them in the conservation solve and out of arc-level selection, so they have to survive
    // decoding rather than be filtered here.
    assert_eq!(
        arcs.iter()
            .filter(|(_, _, f)| f.contains(ArcFlags::FAKE))
            .count(),
        2
    );
}

/// The line sets, which are what a coverage query is ultimately about.
#[test]
fn each_block_carries_the_lines_it_came_from() {
    let n = notes();
    let main_lines: Vec<(u32, &str, Vec<u32>)> = n.functions[0]
        .lines
        .iter()
        .map(|l| (l.block, l.file.as_str(), l.lines.clone()))
        .collect();
    assert_eq!(
        main_lines,
        vec![
            (2, "t.c", vec![3]),
            (3, "t.c", vec![3]),
            (4, "t.c", vec![3])
        ],
        "three blocks, one line: this is what a twice-expanded macro looks like from inside the \
         format, and why 031 has to join it to the expansion index to say anything about `ADD1`"
    );
    let hdr_lines: Vec<(u32, &str, Vec<u32>)> = n.functions[1]
        .lines
        .iter()
        .map(|l| (l.block, l.file.as_str(), l.lines.clone()))
        .collect();
    assert_eq!(hdr_lines, vec![(2, "m.h", vec![2]), (3, "m.h", vec![2])]);
}

/// **A function whose every counter is zero has every arc zero**, and needs no graph reasoning.
///
/// The spanning tree makes the solution unique — that is what it is for — and the all-zero
/// assignment always satisfies conservation. So if no measured arc ran, no derived arc ran, and
/// the answer follows without propagating anything.
///
/// It matters because "never ran" is the common case: gcc elides the counters of such a function
/// entirely (a negative record length), and 83 of 98 objects of one real build had at least one.
/// `memory_client.c`'s `rx_thread_fn` is one of them — a thread body nothing started — and it has
/// two blocks with no successors, so arc-by-arc propagation stalls and the function was being
/// skipped as unsolvable. gcov reports its 18 lines as `0`; skipping reported nothing at all.
#[test]
fn a_function_that_never_ran_solves_without_propagation() {
    let idx = chiero_gcov::ingest_native(&corpus(), "unrun").expect("unrun decodes");
    let lines = idx.lines_of("unrun.c");
    assert!(!lines.is_empty());
    assert_eq!(
        idx.line_count("unrun.c", 1),
        Some(0),
        "recorded, and nothing ran it"
    );
    assert!(
        chiero_gcov::ingest_native(&corpus(), "unrun")
            .expect("unrun decodes")
            .provenance()
            .iter()
            .all(|r| r.unsolved.is_empty()),
        "and it is solved rather than skipped"
    );
}
