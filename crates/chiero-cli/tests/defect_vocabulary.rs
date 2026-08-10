//! **Every `MemFault` kind is either probed by the corpus or excluded on the record.**
//!
//! `injected_defects.rs` answers "does this checker fire?" for the kinds somebody thought to
//! write a case for. This answers the prior question — *which kinds has nobody thought about?*
//! On 2026-08-10 the answer was three (`double-free`, `bad-free`, `misaligned`), found by
//! reading the enum by eye, and the note recording that said "worth re-running whenever the
//! vocabulary or the corpus changes". A five-minute manual check that must be remembered is one
//! that will not be, so it is a gate.
//!
//! **The source of truth is `MemFault::kind`'s match**, because the compiler already forces that
//! to be exhaustive: a new variant cannot be added without giving it a slug there, and this test
//! then demands a case or an entry below. Parsing source is how `xtask contract-coverage` reads
//! the specs and how the handoff lint reads its own file; the alternative is a hand-kept list,
//! which is the drift this exists to catch.

use std::collections::BTreeSet;

/// Kinds with no corpus case, each with the reason it does not need one.
///
/// **A reason, not a name** — an allowlist of bare slugs is how a gap becomes permanent.
const EXCLUDED: &[(&str, &str)] = &[
    (
        "misaligned",
        "filtered by the engine until a `ub-strict` mode exists (chiero-exec/src/lib.rs:3798, \
         item 5j): lowering emits `align 1` for a packed member, so reporting it would call \
         ordinary legal C misaligned. `chiero-mem/tests/copy_alignment.rs` pins both halves.",
    ),
    (
        "pointer-outside-object",
        "has a case, and it is the corpus's one standing miss: the fault fires only for \
         *symbolic* offsets. Possibly deliberate — the variant's doc says forming such a \
         pointer is deliberate in a few real idioms. Owner's judgement, §7.31.",
    ),
    (
        "write-to-readonly",
        "needs a const global written through a cast; the checker is exercised by \
         chiero-mem's own tests.",
    ),
    (
        "unsupported-access-width",
        "a chiero limit, not a defect in the analysed program.",
    ),
    (
        "symbolic-byte",
        "a fidelity statement about chiero's own modelling, not a program defect.",
    ),
    (
        "may-be-out-of-bounds",
        "the maybe-form of a kind the corpus covers definitely.",
    ),
    (
        "maybe-uninitialized-read",
        "likewise the maybe-form of `uninitialized`.",
    ),
    (
        "overlapping-copy",
        "needs a `memcpy` with aliasing arguments; not yet written.",
    ),
    (
        "allocation-too-large",
        "a chiero materialisation limit, reported and verified by hand \
      2026-08-10 (`malloc(1 << 40)` names the size).",
    ),
    (
        "use-after-scope",
        "covered — the corpus case is named `use_after_scope`.",
    ),
    (
        "may-signed-overflow",
        "the weaker claim, deliberately not reported: `UbKind` separates a path that *forces* \
         overflow from one that merely permits it, and the latter would fire on every `x + 1` \
         in existence. Verified 2026-08-10 — `int probe(int x) { return x + 1; }` reports \
         nothing, while the forced form has a corpus case.",
    ),
];

/// `UbKind`'s slugs, from `ub_phrase`'s match — the second vocabulary, and exhaustive for the
/// same reason. Its own comment records that removing the catch-all was deliberate: *"a new
/// variant should be a compile error rather than a silent fallthrough"*.
fn slugs_in_ub_phrase() -> BTreeSet<String> {
    let src = std::fs::read_to_string(root().join("crates/chiero-check/src/lib.rs"))
        .expect("read chiero-check");
    src.lines()
        .filter_map(|l| {
            let (_, tail) = l.trim().strip_prefix("UbKind::")?.split_once("=> \"")?;
            Some(tail.split('"').next()?.to_string())
        })
        .collect()
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn slugs_in_kind_match() -> BTreeSet<String> {
    let src = std::fs::read_to_string(root().join("crates/chiero-mem/src/lib.rs"))
        .expect("read chiero-mem");
    // `MemFault::Foo { .. } => "slug",` — the arms of `kind`, and nothing else in the file has
    // that exact shape.
    src.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("MemFault::")?;
            let (_, tail) = rest.split_once("=> \"")?;
            Some(tail.split('"').next()?.to_string())
        })
        .collect()
}

/// **Every checker in `default_checkers()` is one the corpus knows about.**
///
/// The two vocabularies above answer "which *kinds* has nobody probed". This answers the
/// registry question beside it: adding a checker to the defaults changes what every
/// `find-bugs` run does, and it should not be possible to do that silently. There are only
/// three checkers today, which is exactly when a list like this is cheap to keep honest.
#[test]
fn every_default_checker_is_accounted_for() {
    // How each is reached from `injected_defects.rs`. A checker fires *kinds*, so the corpus
    // covers it through those rather than by name.
    const KNOWN: &[(&str, &str)] = &[
        (
            "OrderDependence",
            "the `order_dependence` case — two calls in one unsequenced region",
        ),
        (
            "UndefinedArithmetic",
            "its `UbKind` slugs: div_zero, shift_past_width, signed_overflow_forced,              float_cast_out_of_range",
        ),
    ];
    let src = std::fs::read_to_string(root().join("crates/chiero-check/src/lib.rs"))
        .expect("read chiero-check");
    let body = src
        .split_once("pub fn default_checkers()")
        .and_then(|(_, r)| r.split_once('}'))
        .map(|(b, _)| b.to_string())
        .expect("default_checkers has a body");

    let registered: Vec<&str> = KNOWN
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| body.contains(*n))
        .collect();
    assert_eq!(
        registered.len(),
        KNOWN.len(),
        "a checker listed here is no longer in `default_checkers()` — drop it from KNOWN:          body = {body}"
    );

    // The direction that matters: something in the defaults that nobody here has heard of.
    for line in body.lines() {
        let Some((_, rest)) = line.trim().split_once("Box::new(") else {
            continue;
        };
        let name = rest.split("::").next().unwrap_or("").trim();
        assert!(
            KNOWN.iter().any(|(k, _)| *k == name),
            "`{name}` is in `default_checkers()` and unknown to the defect corpus. Every              `find-bugs` run now uses it. Add a case to injected_defects.rs and an entry to              KNOWN saying how it is reached."
        );
    }

    // `UnionPun` is deliberately absent from the defaults — chiero follows gcc, which defines
    // union punning, so it "exists for the projects that want the stricter reading rather than
    // for this one". Asserted so the comment and the registry cannot drift apart.
    assert!(
        !body.contains("UnionPun"),
        "UnionPun joined the defaults — it is off by design (chiero-check/src/lib.rs:27), so          either that reasoning changed or this is a mistake"
    );
}

#[test]
fn every_memfault_kind_is_probed_or_excluded_with_a_reason() {
    let mut vocabulary = slugs_in_kind_match();
    // **Both channels, one gate.** 023 §6.1 makes the kind half of the dedup key and both build
    // their message as `"{kind}: {detail}"`, so a reader meets them as one vocabulary.
    vocabulary.extend(slugs_in_ub_phrase());
    assert!(
        vocabulary.len() > 10,
        "the parse found only {} kinds — `MemFault::kind`'s shape changed and this gate is \
         now measuring nothing: {vocabulary:?}",
        vocabulary.len()
    );

    let corpus = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/injected_defects.rs"),
    )
    .expect("read the corpus");

    let excluded: BTreeSet<&str> = EXCLUDED.iter().map(|(k, _)| *k).collect();
    let missing: Vec<&String> = vocabulary
        .iter()
        .filter(|k| !corpus.contains(&format!("\"{k}\"")) && !excluded.contains(k.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "these `MemFault` kinds have no corpus case and no recorded reason: {missing:?}\n\
         Add a case to injected_defects.rs, or an entry to EXCLUDED **with the reason** — a \
         bare name is how a gap becomes permanent.",
    );

    // **And the exclusions must stay honest.** A kind that gains a case should leave the list,
    // or the list becomes a place where covered things hide.
    for (k, _) in EXCLUDED {
        if *k == "pointer-outside-object" || *k == "use-after-scope" {
            continue; // documented above as covered-but-noted
        }
        assert!(
            !corpus.contains(&format!("\"{k}\"")),
            "`{k}` is excluded but the corpus now has a case for it — drop the exclusion"
        );
    }
}
