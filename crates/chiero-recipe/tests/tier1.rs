//! Covers: 042 contract 7 — the tier-1 sweep reports candidate counts per recipe.

use chiero_recipe::{FunctionRef, Recipe, load, tier1_counts};

fn recipe(name: &str, scope: Option<&str>) -> Recipe {
    let scope = scope.map_or(String::new(), |s| format!("  scope fn $f where {s}\n"));
    load(&format!(
        "recipe {name} {{\n  title \"t\"\n{scope}  fixture good \"g.c\"\n  \
         fixture bad \"b.c\" expect 1 at \"b.c:1\"\n}}\n"
    ))
    .expect("loads")
}

fn fns() -> Vec<FunctionRef> {
    ["show_hw", "clear_hw", "helper"]
        .iter()
        .map(|n| FunctionRef {
            name: (*n).to_owned(),
            file: format!("src/vnet/{n}.c"),
        })
        .collect()
}

#[test]
fn a_scoped_recipe_counts_only_the_functions_it_selects() {
    let r = [recipe("shows", Some("name matches \"^show_\""))];
    let t = &tier1_counts(&r, &fns())[0];
    assert_eq!(t.recipe, "shows");
    assert_eq!(t.matched, 1);
    assert_eq!(t.needs_ast, 0);
    assert!(t.is_complete());
}

/// A recipe with no `scope` applies to every function — 042 §4.2 makes narrowing optional.
#[test]
fn an_unscoped_recipe_counts_every_function() {
    let r = [recipe("all", None)];
    let t = &tier1_counts(&r, &fns())[0];
    assert_eq!(t.matched, 3);
    assert!(t.is_complete());
}

/// **A recipe that found nothing and one that could not be evaluated must not report the same
/// thing.** This is the whole reason `Selection` is three-valued: `registered_via` needs the
/// typed AST, and counting its functions as "not selected" would publish `0 candidates` for
/// every CLI recipe in the catalogue — a number indistinguishable from a rule that ran and
/// matched nothing, over a tree it never examined.
#[test]
fn a_recipe_that_could_not_be_evaluated_is_distinguishable_from_one_that_matched_nothing() {
    let rs = [
        recipe("needs_ast", Some("registered_via VLIB_CLI_COMMAND")),
        recipe(
            "genuinely_empty",
            Some("name matches \"^nothing_matches_this\""),
        ),
    ];
    let t = tier1_counts(&rs, &fns());

    // Both matched zero...
    assert_eq!(t[0].matched, 0);
    assert_eq!(t[1].matched, 0);
    // ...and only one of them actually looked.
    assert_eq!(t[0].needs_ast, 3, "every function was undecidable");
    assert_eq!(t[1].needs_ast, 0);
    assert!(
        !t[0].is_complete(),
        "an unevaluated recipe is not a clean result"
    );
    assert!(t[1].is_complete());
}

/// Tallies come back in catalogue order, so a report can be diffed between runs — 042 c5d
/// makes the per-recipe count a tracked baseline, and a baseline that reorders is not one.
#[test]
fn tallies_are_in_catalogue_order() {
    let rs = [
        recipe("zebra", Some("name matches \"^show_\"")),
        recipe("alpha", Some("name matches \"^clear_\"")),
    ];
    let t = tier1_counts(&rs, &fns());
    assert_eq!(
        t.iter().map(|x| x.recipe.as_str()).collect::<Vec<_>>(),
        ["zebra", "alpha"]
    );
}
