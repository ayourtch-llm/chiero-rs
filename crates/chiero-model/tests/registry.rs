//! The model registry and the standard models (024).
//!
//! Covers **024 contracts 1, 2, 3, 4, 5, 6, 18, 19, 21** and §2.1.
//!
//! §2.1 is the load-bearing rule here, and it is easy to read as editorial when it is
//! not: declaring a model `Approximate` has a **mechanical** fidelity effect. Without it
//! there is a hole straight through the project's central guarantee — a run calling
//! `scanf`, or any `<math.h>` function, or `read`, could finish `Exact`, mint a witness
//! and report "no bugs exist" as a proof. The *unmodeled* path was already loud; this is
//! the modeled path, which is worse because it looks deliberate.

use chiero_model::*;

/// **024 contract 18.** Registering a name twice is an error; `replace` is the way to
/// override. Silent last-wins registration would make which model you got depend on link
/// order, and 001 §5 makes determinism a hard requirement.
#[test]
fn registering_a_name_twice_is_an_error_but_replacing_is_not() {
    let mut r = ModelRegistry::with_builtins();
    let before = r.len();
    assert!(r.lookup("malloc").is_some(), "the builtins are present");

    match r.register(ModelEntry::approximate("malloc", "a second malloc")) {
        Err(ModelError::Duplicate(n)) => assert_eq!(&*n, "malloc"),
        other => panic!("expected Duplicate, got {other:?}"),
    }
    assert_eq!(r.len(), before, "a rejected registration changes nothing");

    r.replace(ModelEntry::approximate("malloc", "a deliberate override"))
        .expect("replace succeeds");
    assert_eq!(r.len(), before, "replacing does not add an entry");
    assert_eq!(
        r.lookup("malloc").unwrap().precision,
        Precision::Approximate("a deliberate override".into())
    );
}

/// **024 §2.1 and contract 21.** Every `Approximate` model carries a reason of at least
/// eight non-whitespace characters. A non-empty check is satisfied by `" "` and says
/// nothing, which is why the spec states the length.
#[test]
fn every_approximate_model_gives_a_real_reason() {
    let r = ModelRegistry::with_builtins();
    let mut checked = 0;
    for e in r.entries() {
        if let Precision::Approximate(reason) = &e.precision {
            let n = reason.chars().filter(|c| !c.is_whitespace()).count();
            assert!(
                n >= 8,
                "`{}` has an approximate precision with a {n}-character reason: {reason:?}",
                e.name
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "the default registry must contain approximate models, or this proves nothing"
    );
}

/// **024 §2.1: `Approximate` is mechanical, not editorial.** Dispatching one sets
/// `Fidelity >= Approximated` and pushes an assumption carrying the reason — so a program
/// calling `scanf` cannot come back `Exact` (contract 21b).
#[test]
fn dispatching_an_approximate_model_degrades_and_says_why() {
    let r = ModelRegistry::with_builtins();
    let e = r.lookup("scanf").expect("scanf is modeled as approximate");
    let Precision::Approximate(reason) = &e.precision else {
        panic!("scanf must be approximate, got {:?}", e.precision)
    };
    assert_eq!(
        e.fidelity_effect(),
        Some(ModelFidelity::Approximated),
        "an approximate model degrades by dispatching, not by anyone remembering to"
    );
    assert!(reason.contains("input"));

    // An exact model does not degrade, or every program would be approximate and the
    // distinction would carry no information.
    let m = r.lookup("memcpy").expect("memcpy is modeled");
    assert_eq!(m.precision, Precision::Exact);
    assert_eq!(m.fidelity_effect(), None);
}

/// **024 contract 21c.** A `Havoc` outcome degrades identically whether it came from the
/// default unmodeled fallback or from a registered model that chose to havoc — otherwise
/// "I don't know" said politely counts for less than "I don't know" said by omission.
#[test]
fn a_havoc_outcome_degrades_wherever_it_comes_from() {
    let from_default = HavocSpec::unmodeled_extern();
    let from_model = HavocSpec {
        objects: vec![],
        reachable_depth: 0,
        init: HavocInit::Uninitialized,
        may_free: false,
    };
    assert_eq!(
        from_default.fidelity_effect(),
        from_model.fidelity_effect(),
        "a deliberate havoc is exactly as imprecise as an accidental one"
    );
    assert_eq!(from_default.fidelity_effect(), ModelFidelity::Approximated);
}

/// **024 §2.1's default, spelled out.** An unmodeled extern havocs with `Symbolic` init
/// and `reachable_depth: 1`. `init` has no safe default and the spec says so: `Symbolic`
/// can mask a genuine uninitialized-read bug, `Uninitialized` produces a false-positive
/// storm on any buffer the callee legitimately filled. The choice is recorded so it is
/// visible rather than folkloric.
#[test]
fn the_unmodeled_default_is_symbolic_at_depth_one_and_says_so() {
    let h = HavocSpec::unmodeled_extern();
    assert_eq!(h.init, HavocInit::Symbolic);
    assert_eq!(h.reachable_depth, 1);
    assert!(!h.may_free, "an unknown function is not assumed to free");
    let note = h.describe();
    assert!(note.contains("symbolic"), "{note}");
    assert!(note.contains("depth 1"), "{note}");
}

/// **024 contract 19**, which is 001 §7's reusable-library requirement made checkable:
/// this crate contains no VPP knowledge. `chiero-vpp` registers vppinfra models *into*
/// it; if the names leak the other way the layering has already failed.
#[test]
fn the_crate_contains_no_vpp_knowledge() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    for e in std::fs::read_dir(&src).expect("src exists").flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            // Whole-token matching: a substring grep lets `clib_` match a comment about
            // *why* the rule exists, which is how this kind of guard becomes decoration.
            for tok in line.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if ["vec_len", "pool_get", "clib_mem_alloc", "vlib_buffer_t"].contains(&tok) {
                    hits.push(format!("{}:{}: {tok}", e.path().display(), n + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "VPP identifiers in chiero-model: {hits:#?}");
}

/// A model that is not registered is not found, and looking one up must not invent an
/// entry — the unmodeled path is the engine's to handle loudly (023 §5).
#[test]
fn an_unregistered_name_is_simply_absent() {
    let r = ModelRegistry::with_builtins();
    assert!(r.lookup("a_function_nobody_modeled").is_none());
}

/// **024 contract 1's shape.** `malloc` forks into success and `NULL` by default, because
/// allocation failure is a real path and pretending otherwise silently prunes it. With
/// `alloc_may_fail = false` it is one state (contract 2).
#[test]
fn malloc_forks_into_success_and_failure_unless_told_not_to() {
    let d = AllocPolicy::default();
    assert!(
        d.may_fail,
        "allocation failure is a real path; pruning it by default hides a bug class"
    );
    assert_eq!(d.outcomes(), 2);
    let never = AllocPolicy { may_fail: false };
    assert_eq!(never.outcomes(), 1);
}
