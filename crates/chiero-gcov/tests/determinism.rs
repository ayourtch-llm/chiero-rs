//! **030 contract 19: ingesting the same artifacts twice produces byte-identical indices.**
//!
//! The claim underneath 001 §5's ban on `HashMap` in output paths, checked rather than assumed.
//! An index that differs run to run makes every downstream comparison unreliable in the way that
//! is hardest to notice: 031 diffs two indices to find what a change touched, and a spurious
//! difference there is a test selected or skipped for no reason, once, unreproducibly.
//!
//! # What "byte-identical" is taken to mean
//!
//! There is no serialization format for a `CoverageIndex` yet, so the comparison is of its full
//! derived `Debug` rendering. That is not a weaker check than bytes on disk — it walks every
//! field, including the ones no query exposes, and it renders every collection **in iteration
//! order**, so a container that lost its ordering shows up here even when every query still
//! answers correctly. It is the ordering, not the contents, that a hash map would break.

use chiero_gcov::{TestId, Variant};
use std::path::PathBuf;

/// Where two renderings first differ, with a window either side.
///
/// A whole index is megabytes of `Debug`, and `assert_eq!` on two of them prints both — which
/// buries the one line that differs in the one place nobody will scroll to. This reports the
/// offset and the neighbourhood, which is what a reader needs.
fn first_difference(a: &str, b: &str) -> Option<String> {
    let at = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))?;
    let from = at.saturating_sub(60);
    let window = |s: &str| {
        let to = (at + 60).min(s.len());
        s.get(from..to).unwrap_or("<boundary>").to_string()
    };
    Some(format!(
        "first difference at byte {at}\n  left:  ...{}...\n  right: ...{}...",
        window(a),
        window(b)
    ))
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// Several objects, several tests, several variants — the ingest paths that build every ordered
/// structure in the index between them.
fn build() -> chiero_gcov::CoverageIndex {
    let mut idx = chiero_gcov::CoverageIndex::default();
    for (i, stem) in ["t", "loop", "inl", "multi", "group"].iter().enumerate() {
        chiero_gcov::ingest_native_as(&mut idx, TestId(i as u32), &corpus(), stem)
            .expect("the fixture decodes");
    }
    chiero_gcov::ingest_native_as_variant(
        &mut idx,
        TestId(9),
        Variant::named("x86_64_v3"),
        &corpus(),
        "t",
    )
    .expect("t as a variant");
    chiero_gcov::ingest_json(&corpus(), "loop").expect("the json path too");
    idx
}

/// **Contract 19.** Twice, in one process.
#[test]
fn two_ingests_of_one_corpus_are_identical() {
    let (a, b) = (format!("{:?}", build()), format!("{:?}", build()));
    if let Some(d) = first_difference(&a, &b) {
        panic!(
            "an index that differs between two identical ingests makes 031's diff of two \
             indices report changes nobody made\n{d}"
        );
    }
}

/// **And identical across processes**, which is the part a same-process comparison cannot see: a
/// container seeded per process — every `std` hash map is — is perfectly stable within one run
/// and different in the next.
///
/// Re-runs this test binary in a child process and compares the rendering it prints.
#[test]
fn an_index_is_identical_in_another_process() {
    if std::env::var("CHIERO_DETERMINISM_CHILD").is_ok() {
        print!("{:?}", build());
        return;
    }
    let exe = std::env::current_exe().expect("this test binary");
    let run = || {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "an_index_is_identical_in_another_process",
                "--nocapture",
            ])
            .env("CHIERO_DETERMINISM_CHILD", "1")
            .output()
            .expect("re-running this test binary");
        assert!(out.status.success(), "the child run failed");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let (a, b) = (run(), run());
    assert!(
        a.contains("CoverageIndex"),
        "the child printed an index: {a:.120}"
    );
    if let Some(d) = first_difference(&a, &b) {
        panic!(
            "two processes must agree — this is what a per-process hash seed breaks, and what \
             the same-process check above cannot see\n{d}"
        );
    }
}

/// Ingest **order** changes the index, and must: `TestId`s arrive in the order they are ingested,
/// and a `Vec<TestId>` records that. This pins the difference as intended rather than leaving a
/// reader to wonder whether contract 19 was meant to make order irrelevant.
#[test]
fn a_different_ingest_order_is_a_different_index() {
    let mut a = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut a, TestId(0), &corpus(), "t").unwrap();
    chiero_gcov::ingest_native_as(&mut a, TestId(1), &corpus(), "loop").unwrap();

    let mut b = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut b, TestId(1), &corpus(), "loop").unwrap();
    chiero_gcov::ingest_native_as(&mut b, TestId(0), &corpus(), "t").unwrap();

    assert_ne!(
        format!("{a:?}"),
        format!("{b:?}"),
        "arrival order is recorded, so contract 19 is about repeatability and not about order \
         independence"
    );
    // The answers, however, do not depend on it.
    for file in ["t.c", "loop.c"] {
        for line in a.lines_of(file) {
            assert_eq!(a.tests_for_line(file, line), b.tests_for_line(file, line));
            assert_eq!(a.line_count(file, line), b.line_count(file, line));
        }
    }
}
