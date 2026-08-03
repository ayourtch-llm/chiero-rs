//! Covers: 042 contracts 1, 3 (the load-time half) and §5.
//!
//! **Loading, not evaluating.** Contracts 2 and 3 in full require running a recipe over its
//! fixtures, which needs the tier-2 engine. What is testable now is the rule that makes the
//! corpus survivable at all: a recipe that cannot be adjudicated by its own fixtures must not
//! load. Slicing here keeps the gate honest rather than stubbing an evaluator that answers
//! "no findings" and passes everything.

use chiero_recipe::{Severity, Tier, load};

/// The example from 042 §4, verbatim. If the shipped syntax in the spec does not load, the
/// spec and the loader disagree and one of them is wrong.
const EXAMPLE: &str = r#"
recipe cli_line_input_freed {
  title     "CLI line input must be freed on every path"
  severity  error
  tier      semantic
  rationale "unformat_line_input allocates a line_input; VPP's CLI ritual requires
             unformat_free on every return path. 407 acquisition sites, 140 files."

  scope fn $f where registered_via VLIB_CLI_COMMAND

  track $li typestate {
    state unowned initial
    state owned
    state freed

    unowned -> owned  on `unformat_user($_, unformat_line_input, $li)` returning nonzero
    owned   -> freed  on `unformat_free($li)`
    freed   -> freed  on `unformat_free($li)`
  }

  require on_all_paths { at return: state($li) != owned }

  fixture good "fixtures/cli_ok.c"
  fixture bad  "fixtures/cli_leak.c" expect 1 at "cli_leak.c:22"
}
"#;

#[test]
fn the_example_recipe_from_the_spec_loads() {
    let r = load(EXAMPLE).expect("042 §4's own example must load");
    assert_eq!(r.name, "cli_line_input_freed");
    assert_eq!(r.title, "CLI line input must be freed on every path");
    assert_eq!(r.severity, Severity::Error);
    assert_eq!(r.tier, Tier::Semantic);
    // **A rationale spanning source lines collapses to one sentence — asserted exactly.**
    // `starts_with` plus `contains` cannot see this: both hold just as well when the second
    // line keeps its thirteen spaces of indentation, so a build that never collapsed anything
    // passed them. Mutation caught that; the whole string is the only assertion that pins it.
    assert_eq!(
        r.rationale,
        "unformat_line_input allocates a line_input; VPP's CLI ritual requires \
unformat_free on every return path. 407 acquisition sites, 140 files."
    );

    assert_eq!(r.good, ["fixtures/cli_ok.c"]);
    assert_eq!(r.bad.len(), 1);
    assert_eq!(r.bad[0].path, "fixtures/cli_leak.c");
    assert_eq!(r.bad[0].expect, 1);
    assert_eq!(r.bad[0].at, "cli_leak.c:22");

    // The `track`/`require` clauses are kept whole and unparsed for now — recorded as a
    // deliberate slice, so a later wave can see exactly what has not been read yet.
    assert_eq!(r.unparsed_clauses.len(), 3, "scope, track, require");
}

/// 042 contract 1, and the diagnostic must **name the recipe** — a loader that says only
/// "missing good fixture" leaves the reader to find which of 40 recipes it meant.
#[test]
fn a_recipe_without_a_good_fixture_fails_to_load_naming_itself() {
    let src = EXAMPLE.replace("  fixture good \"fixtures/cli_ok.c\"\n", "");
    let errs = load(&src).expect_err("a recipe with no good fixture must not load");
    assert!(
        errs.iter()
            .any(|e| e.contains("cli_line_input_freed") && e.contains("good")),
        "diagnostic must name the recipe and the missing fixture kind: {errs:?}"
    );
}

/// 042 §5: the `bad` fixture is what catches under-matching, the far more dangerous rot.
#[test]
fn a_recipe_without_a_bad_fixture_fails_to_load() {
    let src = EXAMPLE.replace(
        "  fixture bad  \"fixtures/cli_leak.c\" expect 1 at \"cli_leak.c:22\"\n",
        "",
    );
    let errs = load(&src).expect_err("a recipe with no bad fixture must not load");
    assert!(
        errs.iter()
            .any(|e| e.contains("cli_line_input_freed") && e.contains("bad"))
    );
}

/// 042 contract 3 says a `bad` fixture finding **at the wrong location** must fail the
/// recipe. A `bad` fixture that declares no expected location cannot ever detect that, so
/// the location is required at load time rather than treated as optional decoration.
#[test]
fn a_bad_fixture_must_declare_where_the_finding_is_expected() {
    let src = EXAMPLE.replace(" expect 1 at \"cli_leak.c:22\"", "");
    let errs = load(&src).expect_err("a bad fixture with no expected location must not load");
    assert!(
        errs.iter()
            .any(|e| e.contains("cli_line_input_freed") && e.contains("expect"))
    );
}

/// Junk is refused with a diagnostic rather than accepted as an empty recipe — an empty
/// recipe would pass every fixture rule above by having nothing to check.
#[test]
fn a_file_that_is_not_a_recipe_fails_to_load() {
    assert!(load("this is not a recipe").is_err());
    assert!(load("").is_err());
}
