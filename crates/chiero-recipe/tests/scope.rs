//! Covers: 042 §4.2, the `scope` clause and its tier-1 selectors.

use chiero_recipe::{Selector, load};

fn with_scope(scope: &str) -> String {
    format!(
        "recipe r {{\n  title \"t\"\n  {scope}\n  fixture good \"g.c\"\n  \
         fixture bad \"b.c\" expect 1 at \"b.c:1\"\n}}\n"
    )
}

/// Each tier-1 selector 042 §4.2 lists parses to its own variant. They are separate variants
/// rather than a string because the evaluator must fail to compile when a new one is added,
/// not silently treat it as unmatched.
#[test]
fn every_tier_one_selector_parses_to_its_own_variant() {
    let sel = |s: &str| {
        load(&with_scope(s))
            .expect("scope should load")
            .scope
            .expect("scope present")
            .selector
    };
    assert_eq!(
        sel("scope fn $f where registered_via VLIB_CLI_COMMAND"),
        Selector::RegisteredVia("VLIB_CLI_COMMAND".into())
    );
    assert_eq!(
        sel("scope fn $f where in_file \"src/vnet/**/*_cli.c\""),
        Selector::InFile("src/vnet/**/*_cli.c".into())
    );
    assert_eq!(
        sel("scope fn $f where name matches \"^show_\""),
        Selector::NameMatches("^show_".into())
    );
    assert_eq!(
        sel("scope fn $f where has_attribute noreturn"),
        Selector::HasAttribute("noreturn".into())
    );

    // The bound variable is kept: `require`/`track` clauses refer to it by name.
    let s = load(&with_scope("scope fn $f where has_attribute noreturn"))
        .unwrap()
        .scope
        .unwrap();
    assert_eq!(s.var, "$f");
}

/// **An unrecognised selector must fail the load, not degrade to "matches nothing".**
/// A scope that selects no function reports zero violations for every file, which is
/// indistinguishable from a clean tree — the same silent pass that 042 §5 makes fixtures
/// mandatory to prevent. A typo in a selector name is exactly how that happens in practice.
#[test]
fn an_unknown_selector_fails_the_load_rather_than_matching_nothing() {
    let errs = load(&with_scope("scope fn $f where registered_by VLIB_CLI_COMMAND"))
        .expect_err("unknown selector must not load");
    assert!(
        errs.iter().any(|e| e.contains("registered_by") && e.contains('r')),
        "the diagnostic must name the offending selector and the recipe: {errs:?}"
    );
}

/// A recipe with no `scope` clause loads: 042 §4.2 makes `scope` the way to narrow a recipe,
/// not a requirement, and a rule over every function is legitimate.
#[test]
fn scope_is_optional() {
    let src = "recipe r {\n  title \"t\"\n  fixture good \"g.c\"\n  \
               fixture bad \"b.c\" expect 1 at \"b.c:1\"\n}\n";
    assert!(load(src).expect("loads").scope.is_none());
}
