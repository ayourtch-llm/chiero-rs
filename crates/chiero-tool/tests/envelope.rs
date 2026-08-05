//! **050 §2 — the result envelope, and the one invariant it exists to enforce.**
//!
//! > 2. `proven: true` appears only when `fidelity == "Exact"` — property test over all.
//!
//! 050 calls the envelope "the single most important design decision in the crate", and says
//! exactly which failure it prevents:
//!
//! > an LLM reading `"findings": []` will report "the code is safe." It must instead read
//! > `"findings": [], "proven": false, "blind_spots": [...]` and be structurally unable to miss
//! > the qualification.
//!
//! **Structurally unable** is the requirement, so `proven` is not a field anyone sets. It is
//! derived from the fidelity, and the property test below is over every fidelity there is — not a
//! sample, because the invariant has to hold for all of them or it is not an invariant.
//!
//! # Why this is the same discipline as everything upstream
//!
//! 030 keeps "no record" apart from "recorded zero". 031 §4 widens every gap rather than
//! narrowing it. 032 §3 drops a test only on an `Exact` proof. This is that rule at the surface a
//! consumer actually reads: an empty answer must carry what made it empty, because an empty
//! answer and a confident one are otherwise indistinguishable — which is the failure this project
//! has now met four times in its own harnesses.

use chiero_tool::{Envelope, Fidelity};

/// Every fidelity there is. A property test over a sample would not be one.
const ALL: &[Fidelity] = &[
    Fidelity::Exact,
    Fidelity::Bounded,
    Fidelity::Approximated,
    Fidelity::Unknown,
];

/// **Contract 2.** Over all fidelities, `proven` iff `Exact`.
#[test]
fn proven_is_true_exactly_when_the_fidelity_is_exact() {
    for f in ALL {
        let env = Envelope::new(serde_json::json!({"findings": []}), *f);
        assert_eq!(
            env.proven,
            *f == Fidelity::Exact,
            "{f:?} must {} proven",
            if *f == Fidelity::Exact {
                "be"
            } else {
                "not be"
            }
        );
    }
}

/// And the JSON cannot disagree with the value, because it is rendered from it.
#[test]
fn the_json_carries_the_same_answer() {
    for f in ALL {
        let env = Envelope::new(serde_json::json!({}), *f);
        let v: serde_json::Value =
            serde_json::from_str(&env.to_json()).expect("the envelope is valid JSON");
        assert_eq!(v["proven"], env.proven);
        assert_eq!(v["fidelity"], format!("{f:?}"));
    }
}

/// An empty result **carries what made it empty**. This is the whole point: `findings: []` alone
/// reads as "safe", and the envelope makes that reading unavailable.
#[test]
fn an_unproven_empty_answer_carries_its_qualification() {
    let env = Envelope::new(serde_json::json!({"findings": []}), Fidelity::Bounded)
        .with_blind_spot("single-threaded execution")
        .with_assumption("unmodeled_extern", "rte_eth_rx_burst");

    assert!(!env.proven);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["findings"].as_array().map(Vec::len), Some(0));
    assert!(
        !v["blind_spots"].as_array().expect("present").is_empty(),
        "an empty finding list without blind spots is the sentence this exists to prevent"
    );
    assert_eq!(v["assumptions"][0]["kind"], "unmodeled_extern");
}

/// The text rendering follows the same rule: **"within" a bound, never bare** (050 §2).
#[test]
fn the_text_rendering_never_says_no_defects_bare() {
    let bounded = Envelope::new(serde_json::json!({"findings": []}), Fidelity::Bounded).render();
    assert!(
        bounded.contains("within"),
        "an unproven answer must name its bound: {bounded}"
    );

    let exact = Envelope::new(serde_json::json!({"findings": []}), Fidelity::Exact).render();
    assert!(
        !exact.contains("within"),
        "a proven answer may speak plainly: {exact}"
    );
}

/// Truncation is reported rather than silent — an LLM shown 50 of 1043 sites and told nothing
/// will reason about 50.
#[test]
fn truncation_is_visible() {
    let env = Envelope::new(serde_json::json!({"sites": []}), Fidelity::Exact).truncated(50, 1043);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["truncation"]["truncated"], true);
    assert_eq!(v["truncation"]["shown"], 50);
    assert_eq!(v["truncation"]["total"], 1043);
}

/// Two identical runs produce the same `determinism_key`, and a different result does not.
#[test]
fn the_determinism_key_tracks_the_result() {
    let a = Envelope::new(serde_json::json!({"x": 1}), Fidelity::Exact);
    let b = Envelope::new(serde_json::json!({"x": 1}), Fidelity::Exact);
    let c = Envelope::new(serde_json::json!({"x": 2}), Fidelity::Exact);
    assert_eq!(a.determinism_key(), b.determinism_key());
    assert_ne!(a.determinism_key(), c.determinism_key());
}

/// **The human rendering must be readable by a human.**
///
/// It interpolated the result `serde_json::Value` with `Display`, which prints compact JSON —
/// so `chiero prove-equivalent` greeted a reader with a single 300-character brace soup and
/// then, underneath it, the carefully-worded qualification. The qualification was the part
/// that got the attention; the answer was the part somebody actually wanted.
///
/// The user asked for a command line so the operations could be used *without* programming.
/// A JSON blob is a programmer's output, and `--json` already exists for that.
#[test]
fn the_rendering_is_not_a_json_blob() {
    let env = Envelope::new(
        serde_json::json!({
            "verdict": "differs",
            "input": [{ "origin": "parameter 0", "width": 32, "signed": "-2147483648" }],
            "observation": { "kind": "return_value", "before_signed": "-2147483648" },
            "replay": serde_json::Value::Null,
        }),
        Fidelity::Exact,
    );
    let r = env.render();

    // Structure, not punctuation: every field on its own line, no object braces.
    assert!(
        !r.contains("{\"") && !r.contains("\":"),
        "the rendering is still JSON:\n{r}"
    );
    assert!(
        r.lines().count() >= 5,
        "a nested result rendered onto one line is the blob again:\n{r}"
    );
    for want in [
        "verdict",
        "differs",
        "origin",
        "parameter 0",
        "before_signed",
    ] {
        assert!(r.contains(want), "`{want}` is missing from:\n{r}");
    }
    // A JSON null is "nothing here", and printing the word `null` at a reader is a programmer's
    // habit. Whatever it says, it must not be that.
    assert!(
        !r.contains("null"),
        "raw JSON null in a human rendering:\n{r}"
    );
}

/// And an unproven one still leads with what it is worth — the property the blob obscured.
#[test]
fn the_rendering_still_qualifies_itself() {
    let env = Envelope::new(serde_json::json!({ "findings": [] }), Fidelity::Bounded)
        .with_blind_spot("loops were unrolled to a depth");
    let r = env.render();
    assert!(r.contains("not proven"), "{r}");
    assert!(r.contains("loops were unrolled"), "{r}");
}
