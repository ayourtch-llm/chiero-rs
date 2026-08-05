//! **030 §4 and contracts 8–9: the `.gcno`/`.gcda` header, before any record is decoded.**
//!
//! Measured on the committed fixtures, which are gcc 13.3.0's own output:
//!
//! ```text
//! t.gcno:      6f6e6367  2a333342  1f830cd1   "oncg"  "*33B"  stamp
//! t.gcda:      61646367  2a333342  1f830cd1   "adcg"  "*33B"  stamp — the same
//! other.gcno:  6f6e6367  2a333342  9b8924d1   a different compilation, a different stamp
//! ```
//!
//! Magic is `"gcno"`/`"gcda"` little-endian; the version tag `"*33B"` reverses to `B33*`, gcc
//! 13.3. The **stamp is the pairing key**, and a `.gcda` from another compilation is the single
//! most common source of nonsense coverage in a build tree nobody cleaned.
//!
//! # Why this is its own step
//!
//! Contract 5 — decoded line counts identical to `gcov --json-format` — is the real gate, and it
//! needs the record stream, the flow solve and the arc bookkeeping. None of that is worth writing
//! against a file whose *header* has not been checked, because every one of those failures would
//! look like a decode bug rather than a stale file.

use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// The header of each artifact reads back what `xxd` shows.
#[test]
fn a_header_carries_its_magic_version_and_stamp() {
    let no = chiero_gcov::native::header(&corpus().join("t.gcno")).expect("t.gcno");
    assert_eq!(no.kind, chiero_gcov::native::Kind::Notes);
    assert_eq!(no.version_tag(), "*33B");
    assert_eq!(no.gcc_version(), Some((13, 3)));
    assert_eq!(no.stamp, 0xd10c831f);

    let da = chiero_gcov::native::header(&corpus().join("t.gcda")).expect("t.gcda");
    assert_eq!(da.kind, chiero_gcov::native::Kind::Data);
    assert_eq!(da.stamp, no.stamp, "the stamp pairs the two files");
}

/// **Contract 8.** A `.gcda` from another compilation is rejected, and the message names *both*
/// stamps — the number is the only thing that tells a reader which build the file came from.
#[test]
fn a_stale_gcda_is_rejected_naming_both_stamps() {
    let err = chiero_gcov::native::pair(&corpus().join("t.gcno"), &corpus().join("other.gcda"))
        .expect_err("`other.gcda` is from a different compilation");
    let msg = err.to_string();
    assert!(
        msg.contains("d10c831f") && msg.contains("d124899b"),
        "both stamps must appear, or a reader cannot tell which file to rebuild: {msg}"
    );
    // And the matching pair is accepted.
    assert!(chiero_gcov::native::pair(&corpus().join("t.gcno"), &corpus().join("t.gcda")).is_ok());
}

/// **Contract 9.** An unknown version is one diagnostic naming the version, and no decode is
/// attempted — a layout chiero has not been tested against is not a layout to guess at.
#[test]
fn an_unknown_version_is_named_and_not_guessed_at() {
    let err = chiero_gcov::native::header(&corpus().join("badversion.gcno"))
        .expect_err("`*99Z` is not a version chiero decodes");
    let msg = err.to_string();
    assert!(
        msg.contains("*99Z"),
        "the message must name the version it found: {msg}"
    );
}

/// A file that is not a coverage artifact at all fails as one, not as a version.
#[test]
fn a_file_that_is_not_an_artifact_says_so() {
    let err = chiero_gcov::native::header(&corpus().join("t.c")).expect_err("`t.c` is C source");
    let msg = err.to_string();
    assert!(
        msg.contains("magic") || msg.contains("not a"),
        "the message must be about the magic, not about a version read out of source text: {msg}"
    );
}

/// **An LTO `.wpa.gcno` holds a second artifact where its records should be.**
///
/// Found by decoding every `.gcno` of a full VPP build: 1894 of 1895 read, and the one that did
/// not was `libvppinfra.so.26.10.wpa.gcno`, written by gcc's whole-program analysis pass. Its
/// header is ordinary — magic, `*33B`, stamp, working directory, flag — and then, where the first
/// record should begin, there is another complete `oncg*33B` header.
///
/// Read as a record, that magic is a tag whose length word is 1110651690, and the file looks
/// truncated. gcov reports `no functions found` and moves on, which is the right answer: this
/// unit has no records of its own.
///
/// **Stopping at an embedded magic is not a guess about LTO** — it is the one place a `.gcno` can
/// contain the start of another artifact, and a length read out of a magic is never a length.
#[test]
fn a_whole_program_notes_file_has_no_functions_of_its_own() {
    let n = chiero_gcov::native::read_notes(&corpus().join("wpa.gcno"))
        .expect("an LTO notes file reads, it is simply empty");
    assert!(
        n.functions.is_empty(),
        "gcov says `no functions found`, and so must this: {:?}",
        n.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    assert_eq!(n.header.version_tag(), "*33B");
}
