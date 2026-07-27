//! The shared `.cir` corpus.
//!
//! **020 contracts 1 and 5 quantify over "every module in `tests/corpus/cir/`"** — so
//! with an empty directory they pass over nothing. That is how they stood until this
//! test existed, and it is the sixth instance in this project of an assertion whose
//! instrument could not observe the thing. The guard below is therefore not decoration:
//! it fails if the corpus is missing, empty, or too small to be meaningful.

use chiero_cir::text::{parse, print};
use chiero_cir::verify;
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/cir")
}

fn corpus_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .expect("tests/corpus/cir must exist")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "cir"))
        .collect();
    v.sort(); // deterministic (001 §5)
    v
}

/// The guard that keeps contracts 1 and 5 from being vacuous.
#[test]
fn the_corpus_is_not_empty() {
    let files = corpus_files();
    assert!(
        files.len() >= 5,
        "the corpus quantified over by 020 contracts 1 and 5 must be substantial, \
         found {} file(s): {files:#?}",
        files.len()
    );
}

/// 020 contract 1: `parse(print(m))` is structurally equal to `m`, for every module in
/// the corpus.
#[test]
fn every_corpus_module_round_trips() {
    for path in corpus_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let m = parse(&src).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        let printed = print(&m);
        let again = parse(&printed).unwrap_or_else(|e| panic!("{} reparse: {e:?}", path.display()));
        assert_eq!(
            printed,
            print(&again),
            "{} does not round-trip",
            path.display()
        );
    }
}

/// 020 contract 2, over the corpus: checked-in files are in canonical form, so a
/// hand-edit that drifts from what the printer emits is caught rather than tolerated.
#[test]
fn every_corpus_file_is_canonical() {
    for path in corpus_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let m = parse(&src).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        assert_eq!(
            print(&m),
            src,
            "{} is not in canonical form; run `cargo xtask fmt-corpus`",
            path.display()
        );
    }
}

/// 020 contract 5: the verifier accepts every fixture and reports zero *errors*.
/// Warnings (an unreachable block) are permitted — unreachable C exists.
#[test]
fn every_corpus_module_verifies() {
    for path in corpus_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let m = parse(&src).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        let errs: Vec<_> = verify(&m).into_iter().filter(|e| e.is_error()).collect();
        assert!(errs.is_empty(), "{}: {errs:#?}", path.display());
    }
}

/// The corpus must cover the constructs the engine will be built against, or "every
/// module in the corpus" is a weak quantifier no matter how many files it holds.
#[test]
fn the_corpus_covers_the_load_bearing_constructs() {
    let all: String = corpus_files()
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    for construct in [
        "ptradd",   // pointer provenance (021 §2)
        "loadbits", // bitfields (020 §4.5.1)
        "switch",   // multi-way control flow
        "br",       // forking
        "call",     // the call boundary
        "alloca",   // stack objects
        "copymem",  // aggregate assignment
        ".scope",   // stack lifetime (015 §4)
        ".line",    // the coverage join (015 §5)
    ] {
        assert!(
            all.contains(construct),
            "no corpus fixture exercises `{construct}`"
        );
    }
}
