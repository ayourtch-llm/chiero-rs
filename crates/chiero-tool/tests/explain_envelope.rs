//! **050 contract 6 — `explain_macro_expansion` in the envelope.**
//!
//! > 6. `explain_macro_expansion` on the `vec_add1` fixture returns the full chain with each
//! >    frame's definition site and body.
//!
//! # The fidelity, and the case that decides it
//!
//! The chain comes from the preprocessor's own expansion records, so a chain it returns is
//! `Exact`. The interesting answer is the **empty** one: a line with no macro on it is not a
//! failure and not an approximation — it is a complete answer that nothing expanded there, and
//! it is proven.
//!
//! That distinction is the whole envelope in miniature. `[]` from a *scan* would mean "nothing
//! found, within whatever the scan could see"; `[]` from the expansion table means "nothing is
//! there". Only the second may be read plainly, and a consumer cannot tell them apart without
//! `proven`.

use chiero_pp::{Config, preprocess_str};
use chiero_tool::{Fidelity, explain_macro_expansion_envelope};

const SRC: &str = "#define INNER(v) ((v) + 1)\n#define OUTER(v) (INNER (v) * 2)\n\
                   int a (int x) { return OUTER (x); }\nint b (int x) { return x; }\n";

fn tu() -> chiero_pp::PreprocessedTu {
    let tu = preprocess_str("f.c", SRC, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    tu
}

/// A chain is proven, and it carries each frame's definition site and body — so a reader needs no
/// second lookup (050 §3).
#[test]
fn a_chain_is_proven_and_carries_each_frame() {
    let env = explain_macro_expansion_envelope(&tu().source_map, "f.c", 3, None);
    assert!(env.proven);
    assert_eq!(env.fidelity, Fidelity::Exact);

    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let chains = v["result"]["chains"].as_array().expect("chains");
    assert_eq!(chains.len(), 1, "one macro is written on line 3");

    let frames = chains[0].as_array().expect("frames");
    let names: Vec<&str> = frames.iter().filter_map(|f| f["name"].as_str()).collect();
    assert!(
        names.contains(&"OUTER") && names.contains(&"INNER"),
        "the full chain, not just the outermost: {names:?}"
    );
    for f in frames {
        assert!(
            f["def_line"].is_number(),
            "each frame says where it is defined: {f}"
        );
        assert!(f["body"].is_string(), "and what it expands to: {f}");
    }
}

/// **An empty answer here is proven**, and that is the envelope in miniature.
///
/// `[]` from a scan would mean "nothing found within what the scan could see". `[]` from the
/// expansion table means "nothing is there". Only the second may be read plainly, and a consumer
/// cannot tell them apart without `proven`.
#[test]
fn a_line_with_no_macro_is_a_proven_empty_answer() {
    let env = explain_macro_expansion_envelope(&tu().source_map, "f.c", 4, None);
    assert!(
        env.proven,
        "line 4 expands nothing, and the expansion table knows it completely"
    );
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["chains"].as_array().map(Vec::len), Some(0));
}

/// A file the map has never heard of is **not** the same answer, and must not read as "no macros
/// here" — it is a question about something that is not in this translation unit.
#[test]
fn an_unknown_file_is_not_a_proven_empty_answer() {
    let env = explain_macro_expansion_envelope(&tu().source_map, "nosuch.c", 3, None);
    assert!(
        !env.proven,
        "the map holds no such file, so `no macros on line 3` is a claim it cannot make"
    );
    assert!(
        env.assumptions.iter().any(|(k, _)| k == "unknown_file")
            || env.blind_spots.iter().any(|b| b.contains("nosuch.c")),
        "and it says which file it could not find: {:?} {:?}",
        env.assumptions,
        env.blind_spots
    );
}
