//! **030 §5, one level up from `FuncKey`: which build a line's coverage came from.**
//!
//! `FuncKey` keeps two `static helper`s apart. The *line* index has the same problem and does not
//! yet solve it: it is keyed by `(file, line)`, so two builds of one source — VPP compiles many
//! of them under different `CLIB_MARCH_VARIANT` — union into one entry with nothing recording
//! that they were different code.
//!
//! ```text
//! vppinfra/vector/toeplitz.c   compiled as  _x86_64_v3  and  _x86_64_v4
//!   both report coverage for the same file and the same lines
//!   the union says "these tests covered line 40"
//!   the truth is "these tests covered line 40 of the AVX2 build, those of the AVX-512 one"
//! ```
//!
//! # Why this is not merely imprecise
//!
//! 032 selects tests for a *change*. A change to a `CLIB_MARCH_FN` body changes every variant, so
//! a union is harmless there. A change to code that only one variant compiles — inside
//! `#if defined(CLIB_HAVE_VEC512)` — is attributed by the union to the tests of every variant,
//! including the ones that never contained that code. Those tests get run, which is wasteful and
//! safe. The unsafe direction is the mirror: a variant whose tests are *absent* from the union
//! looks covered by the others.
//!
//! So the index must record the variant beside the line, and a query must be able to ask for one
//! variant or for all of them.

use chiero_gcov::{TestId, Variant};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// Two ingests of one object under different variants stay distinguishable.
#[test]
fn two_variants_of_one_file_do_not_merge_silently() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as_variant(
        &mut idx,
        TestId(0),
        Variant::named("x86_64_v3"),
        &corpus(),
        "t",
    )
    .expect("v3");
    chiero_gcov::ingest_native_as_variant(
        &mut idx,
        TestId(1),
        Variant::named("x86_64_v4"),
        &corpus(),
        "t",
    )
    .expect("v4");

    assert_eq!(
        idx.variants(),
        vec![Variant::named("x86_64_v3"), Variant::named("x86_64_v4")]
    );
    // Each variant knows its own tests.
    assert_eq!(
        idx.tests_for_line_in("t.c", 3, &Variant::named("x86_64_v3")),
        Some(vec![TestId(0)])
    );
    assert_eq!(
        idx.tests_for_line_in("t.c", 3, &Variant::named("x86_64_v4")),
        Some(vec![TestId(1)])
    );
}

/// **The union is still available and still correct** — it is the default question, and most
/// changes are variant-independent.
#[test]
fn the_union_across_variants_is_the_default_answer() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as_variant(
        &mut idx,
        TestId(0),
        Variant::named("x86_64_v3"),
        &corpus(),
        "t",
    )
    .expect("v3");
    chiero_gcov::ingest_native_as_variant(
        &mut idx,
        TestId(1),
        Variant::named("x86_64_v4"),
        &corpus(),
        "t",
    )
    .expect("v4");
    assert_eq!(
        idx.tests_for_line("t.c", 3),
        Some(vec![TestId(0), TestId(1)]),
        "asking without a variant asks about the source line, which both builds have"
    );
}

/// A build with no variant — every tree that is not VPP — keeps working unchanged, and its
/// coverage is not filed under some invented name.
#[test]
fn a_tree_with_no_variants_has_exactly_one() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("plain");
    assert_eq!(idx.variants(), vec![Variant::None]);
    assert_eq!(
        idx.tests_for_line_in("t.c", 3, &Variant::None),
        Some(vec![TestId(0)])
    );
    assert_eq!(idx.tests_for_line("t.c", 3), Some(vec![TestId(0)]));
}

/// Asking for a variant the index never saw is `None`, not an empty set — the crate's rule, once
/// more. "No coverage recorded for the AVX-512 build" and "the AVX-512 build ran nothing" are
/// different, and only the second means a test can be skipped.
#[test]
fn an_unknown_variant_answers_nothing() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("plain");
    assert_eq!(
        idx.tests_for_line_in("t.c", 3, &Variant::named("x86_64_v4")),
        None,
        "nothing was ingested for that variant, which is not the same as it covering nothing"
    );
}

/// **A build that never compiled a file must not claim its lines.**
///
/// Found by adversarial review of the representation change (7d35375), which introduced it. When
/// `per_variant` is empty the per-build query answers from the union, on the reasoning that with
/// one build the union *is* that build's set. That reasoning does not cover a line no build
/// contributed to at all — a JSON ingest adds line counts and no tests, and `ingest_json` never
/// notes a variant.
///
/// The answer was `Some(vec![])`: "the avx2 build recorded this line and no test ran it". That is
/// the empty-set claim 030 §1 exists to keep rare, it is what 032 skips tests on, and here it is
/// made for a build that never saw the file.
#[test]
fn a_variant_does_not_claim_lines_it_never_recorded() {
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as_variant(
        &mut idx,
        TestId(0),
        Variant::named("x86_64_v3"),
        &corpus(),
        "loop",
    )
    .expect("loop as v3");
    // Adds counts for `t.c` and no tests, and notes no variant.
    chiero_gcov::ingest_json_into(&mut idx, &corpus(), "t").expect("t from json");

    assert!(
        idx.line_count("t.c", 3).is_some(),
        "the fixture must actually record the line, or this proves nothing"
    );
    assert_eq!(
        idx.tests_for_line_in("t.c", 3, &Variant::named("x86_64_v3")),
        None,
        "the v3 build never compiled t.c; `Some([])` would say it did and that nothing ran it"
    );
    // The build that did record a line still answers for it.
    assert_eq!(
        idx.tests_for_line_in("loop.c", 1, &Variant::named("x86_64_v3")),
        Some(vec![TestId(0)])
    );
}
