//! **030 §5: two functions with one name are two functions.**
//!
//! > `FuncKey` includes `march` because VPP compiles one source many times under different
//! > `CLIB_MARCH_VARIANT`. Two variants of the same function are **different coverage entities**;
//! > merging them by name would attribute one variant's tests to another's code. `start_line`
//! > disambiguates `static` helpers that repeat across files.
//!
//! `a.c` and `b.c` in the corpus each define `static int helper(int)` — at different lines, with
//! different counts. Keyed by name they merge, and the merge is silent: no error, no empty answer,
//! just one file's tests attributed to another file's code. The counts differ deliberately so
//! that a collision cannot look like agreement.
//!
//! # Why `march` is a resolver rather than a field to parse
//!
//! VPP's `CLIB_MULTIARCH_FN` token-pastes `fn##_##CLIB_MARCH_VARIANT`, so the artifacts contain
//! `ip4_lookup_node_fn_avx2` and nothing else. Splitting that back apart is VPP knowledge, which
//! 001 §4 rule 4 forbids this crate from holding — so it is an extension point whose default
//! splits nothing. A resolver that guessed from a bare suffix would collapse `foo_avx2` into
//! `foo` and attribute the vector variant's coverage to the scalar path, which is the exact
//! misattribution `FuncKey` exists to prevent.

use chiero_gcov::native::{ArcCoverage, FuncKey};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn both_objects() -> ArcCoverage {
    let mut cov = ArcCoverage::default();
    chiero_gcov::native::arc_coverage_into(&mut cov, chiero_gcov::TestId(0), &corpus(), "prog-a")
        .expect("prog-a");
    chiero_gcov::native::arc_coverage_into(&mut cov, chiero_gcov::TestId(0), &corpus(), "prog-b")
        .expect("prog-b");
    cov
}

/// The two `helper`s stay apart, and each keeps its own file and line.
#[test]
fn two_static_helpers_of_one_name_are_two_entities() {
    let cov = both_objects();
    let mut helpers: Vec<&FuncKey> = cov
        .functions()
        .into_iter()
        .filter(|k| k.name == "helper")
        .collect();
    helpers.sort_by_key(|k| k.start_line);
    assert_eq!(helpers.len(), 2, "one per file: {helpers:?}");
    assert_eq!(
        (helpers[0].file.as_str(), helpers[0].start_line),
        ("a.c", 1)
    );
    assert_eq!(
        (helpers[1].file.as_str(), helpers[1].start_line),
        ("b.c", 3)
    );
}

/// **The counts do not merge**, which is the fact a name-keyed index gets wrong silently.
#[test]
fn each_helper_keeps_its_own_counts() {
    let cov = both_objects();
    let a = FuncKey::new("a.c", "helper", 1);
    let b = FuncKey::new("b.c", "helper", 3);

    // `a.c`'s helper is called once and `b.c`'s twice, so their entry arcs differ. Merging by
    // name would give both the same number and neither would be wrong-looking.
    let a_entry = cov.arcs_of(&a).expect("a.c helper")[0];
    let b_entry = cov.arcs_of(&b).expect("b.c helper")[0];
    assert_eq!(cov.arc_count(&a, a_entry), Some(1));
    assert_eq!(cov.arc_count(&b, b_entry), Some(2));
}

/// A key that names no function answers `None`, and getting the line wrong is one of the ways to
/// name no function — the key is the identity, not a hint.
#[test]
fn a_key_that_names_nothing_answers_nothing() {
    let cov = both_objects();
    assert_eq!(cov.arcs_of(&FuncKey::new("a.c", "helper", 3)), None);
    assert_eq!(cov.arcs_of(&FuncKey::new("b.c", "nosuch", 1)), None);
}

/// **The default `MarchResolver` splits nothing.** A name with a suffix that *looks* like a
/// variant is one function until something that knows the variant set says otherwise.
#[test]
fn the_default_resolver_invents_no_variants() {
    let cov = both_objects();
    for k in cov.functions() {
        assert_eq!(
            k.march, None,
            "`{}` was given a variant nobody registered a resolver for",
            k.name
        );
    }
}
