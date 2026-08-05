//! **050 contract 7 — `expansion_sites` in the envelope.**
//!
//! > 7. `expansion_sites` on a macro with 1043 sites returns a summary with [the truncation
//! >    reported].
//!
//! `expansion_sites` predates the envelope and reports its own truncation, which is right for a
//! Rust caller and wrong for the JSON surface: 050 §2 puts truncation in one place so that a
//! consumer reads it in one place. An operation that reported it in its own shape would be an
//! operation a reader has to learn separately, and 050 §2's whole argument is that they must not
//! have to.
//!
//! # The fidelity question, asked again
//!
//! This one *can* be `Exact`, and it is the first operation that can. The sites come from the
//! preprocessor's own expansion table — not a scan, not an approximation — so the answer is
//! complete for the translation unit it was given. **A page of it is not**, which is exactly what
//! truncation records.

use chiero_pp::{Config, preprocess_str};
use chiero_tool::{Fidelity, expansion_sites_envelope};

const SRC: &str = "#define M(v) ((v) + 1)\nint a (int x) { return M (x); }\n\
                   int b (int x) { return M (x) + M (x); }\n";

fn tu() -> chiero_pp::PreprocessedTu {
    let tu = preprocess_str("f.c", SRC, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    tu
}

/// A complete answer is `Exact` and **proven** — the sites are the preprocessor's own record, not
/// an estimate.
#[test]
fn a_complete_page_is_proven() {
    let env = expansion_sites_envelope(&tu().source_map, "M", None, 50);
    assert!(env.proven, "the expansion table is the ground truth");
    assert_eq!(env.fidelity, Fidelity::Exact);

    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["total"], 3, "three expansions of M");
    assert_eq!(v["truncation"]["truncated"], false);
}

/// **A truncated page is not proven**, because the answer a caller has is not the answer.
///
/// This is the distinction 050 §2 is built on, arriving somewhere it would be easy to overlook:
/// the *operation* is exact, and the *response* is a page of it.
#[test]
fn a_truncated_page_is_not_proven() {
    let env = expansion_sites_envelope(&tu().source_map, "M", None, 2);
    assert!(
        !env.proven,
        "a caller holding 2 of 3 sites does not hold a proven answer"
    );

    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["truncation"]["truncated"], true);
    assert_eq!(v["truncation"]["shown"], 2);
    assert_eq!(v["truncation"]["total"], 3);
    assert!(
        v["result"]["cursor"].is_number(),
        "and it says how to get the rest: {v}"
    );
}

/// A macro that expands nowhere is a **proven empty** answer — the one case where an empty result
/// may be read plainly, because the expansion table is complete.
#[test]
fn a_macro_that_expands_nowhere_is_proven_empty() {
    let src = "#define UNUSED(v) (v)\nint a (int x) { return x; }\n";
    let tu = preprocess_str("f.c", src, Config::default());
    let env = expansion_sites_envelope(&tu.source_map, "UNUSED", None, 50);

    assert!(env.proven, "nothing expanded it, and that is known rather than unmeasured");
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["total"], 0);
}

/// Each site carries where it was **written** and where the invocation is, because a list macro's
/// items share the second and differ in the first (060 §3).
#[test]
fn each_site_carries_both_positions() {
    let env = expansion_sites_envelope(&tu().source_map, "M", None, 50);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    for s in v["result"]["sites"].as_array().expect("sites") {
        assert!(s["file"].is_string());
        assert!(s["line"].is_number() && s["item_line"].is_number(), "{s}");
    }
}
