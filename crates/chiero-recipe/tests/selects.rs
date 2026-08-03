//! Covers: 042 §4.2 selector evaluation.

use chiero_recipe::{FunctionRef, Selection, load};

fn scoped(sel: &str) -> chiero_recipe::Scope {
    let src = format!(
        "recipe r {{\n  title \"t\"\n  scope fn $f where {sel}\n  fixture good \"g.c\"\n  \
         fixture bad \"b.c\" expect 1 at \"b.c:1\"\n}}\n"
    );
    load(&src).expect("loads").scope.expect("scope")
}

fn f(name: &str, file: &str) -> FunctionRef {
    FunctionRef {
        name: name.to_owned(),
        file: file.to_owned(),
    }
}

#[test]
fn name_matches_selects_by_regex() {
    let s = scoped("name matches \"^show_\"");
    assert_eq!(
        s.selects(&f("show_hw_interfaces", "a.c")),
        Selection::Yes
    );
    // Anchored: a literal-substring matcher would wrongly take this one, which is why the
    // dependency had to be a regex engine rather than a substring searcher.
    assert_eq!(
        s.selects(&f("clear_show_hw", "a.c")),
        Selection::No
    );
}

#[test]
fn in_file_selects_by_glob() {
    let s = scoped("in_file \"src/vnet/**/*_cli.c\"");
    assert_eq!(
        s.selects(&f("x", "src/vnet/bfd/bfd_cli.c")),
        Selection::Yes
    );
    // `*` does not cross a separator, so a file directly under `src/vnet` is not matched by
    // `**/` — the pattern asks for a subdirectory.
    assert_eq!(s.selects(&f("x", "src/vnet/tunnel_cli.c")), Selection::No);
    assert_eq!(s.selects(&f("x", "src/vppinfra/mem.c")), Selection::No);
}

/// **A selector this crate cannot yet evaluate answers `NeedsAst`, never `No`.** Returning
/// "does not match" for `registered_via` would silently empty the scope of every CLI recipe
/// and report a clean tree — the same failure as an unknown selector matching nothing, and
/// the reason that one is a load error.
#[test]
fn a_selector_needing_the_ast_says_so_rather_than_answering_no() {
    for sel in [
        "registered_via VLIB_CLI_COMMAND",
        "has_attribute noreturn",
        "signature \"u8 *(u8 *, va_list *)\"",
        "calls \"unformat_line_input\"",
    ] {
        assert_eq!(
            scoped(sel).selects(&f("any", "any.c")),
            Selection::NeedsAst,
            "selector {sel} must not answer No"
        );
    }
}

/// An invalid regex fails the **load**, naming the recipe — not at sweep time, when the
/// catalogue is already running over 1552 files.
#[test]
fn an_invalid_regex_fails_the_load() {
    let src = "recipe r {\n  title \"t\"\n  scope fn $f where name matches \"^show_(\"\n  \
               fixture good \"g.c\"\n  fixture bad \"b.c\" expect 1 at \"b.c:1\"\n}\n";
    let errs = load(src).expect_err("an unparseable regex must not load");
    assert!(errs.iter().any(|e| e.contains('r') && e.contains("regex")), "{errs:?}");
}
