//! **030 §4: the record stream, as gcc 13.3.0 actually writes it.**
//!
//! The spec sketches `tag: u32, length: u32 (in 4-byte words), payload` and says in the same
//! paragraph that exact layouts are **not** transcribed from documentation, because a
//! transcription error is undetectable by reading. Measuring the committed fixture says
//! otherwise on two counts, and both matter:
//!
//! - **The length is in bytes, not words.** `FUNCTION` at 121 with length 49 is followed by
//!   `BLOCKS` at 178 — `121 + 8 + 49`. Reading it as words walks off the end of a 617-byte file
//!   on the first record.
//! - **Records are not word-aligned.** They start at 121, 178, 190, … Aligning up, as a reader
//!   who assumed word lengths naturally would, desynchronises the stream immediately.
//!
//! The header before them differs between the two artifacts, which is why each has its own
//! offset rather than a shared constant:
//!
//! ```text
//! .gcno: magic version stamp checksum | cwd_len=97 cwd[97] flag | records at 121
//! .gcda: magic version stamp checksum |                        | records at 16
//! ```
//!
//! # What this test is for
//!
//! It is the scaffolding contract 5 stands on. The cross-validation gate — decoded line counts
//! identical to `gcov --json-format` — cannot distinguish "the flow solve is wrong" from "the
//! stream desynchronised on record two", so the inventory is pinned separately and first.

use std::path::PathBuf;

use chiero_gcov::native::Tag;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// The `.gcno`'s records, exactly as measured: two functions, and for each a `BLOCKS`, its
/// `ARCS` and its `LINES`.
#[test]
fn the_notes_stream_reads_to_its_end() {
    let recs = chiero_gcov::native::records(&corpus().join("t.gcno")).expect("t.gcno decodes");
    let shape: Vec<(Tag, usize)> = recs.iter().map(|r| (r.tag, r.payload.len())).collect();
    assert_eq!(
        shape,
        vec![
            (Tag::Function, 49),
            (Tag::Blocks, 4),
            (Tag::Arcs, 12),
            (Tag::Arcs, 20),
            (Tag::Arcs, 20),
            (Tag::Arcs, 12),
            (Tag::Arcs, 12),
            (Tag::Lines, 28),
            (Tag::Lines, 28),
            (Tag::Lines, 28),
            (Tag::Function, 51),
            (Tag::Blocks, 4),
            (Tag::Arcs, 12),
            (Tag::Arcs, 12),
            (Tag::Arcs, 12),
            (Tag::Lines, 28),
            (Tag::Lines, 28),
        ],
        "the stream must reach the last byte of the file, or it desynchronised somewhere"
    );
}

/// The `.gcda`'s, whose records start at a different offset and carry the counters.
#[test]
fn the_data_stream_reads_to_its_end() {
    let recs = chiero_gcov::native::records(&corpus().join("t.gcda")).expect("t.gcda decodes");
    let shape: Vec<(Tag, usize)> = recs.iter().map(|r| (r.tag, r.payload.len())).collect();
    assert_eq!(
        shape,
        vec![
            (Tag::ObjectSummary, 8),
            (Tag::Function, 12),
            (Tag::CounterArcs, 24),
            (Tag::Function, 12),
            (Tag::CounterArcs, 8),
        ]
    );
}

/// **A truncated stream is a diagnostic, not a short answer** (030 contract 6's half of this:
/// corrupt data produces no partial index). A reader that stops early and returns what it has
/// hands downstream a coverage index quietly missing functions.
#[test]
fn a_truncated_stream_is_refused_rather_than_returned_short() {
    let whole = std::fs::read(corpus().join("t.gcno")).expect("read");
    let dir = std::env::temp_dir().join(format!("chiero-gcov-trunc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let cut = dir.join("t.gcno");
    // Half a record: the header and the first record's tag, with its payload missing.
    std::fs::write(&cut, &whole[..125]).expect("write");
    let err = chiero_gcov::native::records(&cut).expect_err("half a record is not a record");
    assert!(
        err.to_string().contains("truncated"),
        "the message must say the file ended mid-record: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
