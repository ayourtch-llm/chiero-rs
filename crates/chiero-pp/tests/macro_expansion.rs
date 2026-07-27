//! Covers: 012 contracts 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 18, 19.

use chiero_pp::{Config, ConfigId, preprocess_str};
use chiero_span::TokenOrigin;

fn texts(src: &str) -> Vec<String> {
    let tu = preprocess_str("fixture.c", src, Config::default());
    assert!(
        tu.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        tu.diagnostics
    );
    tu.token_texts().map(str::to_owned).collect()
}

#[test]
fn object_chains_expand_with_depth_two() {
    let tu = preprocess_str(
        "fixture.c",
        "#define A B\n#define B C\nA\n",
        Config::default(),
    );
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["C"]);
    assert_eq!(
        tu.source_map.expansion_backtrace(tu.tokens[0].span).len(),
        2
    );
}

#[test]
fn blue_paint_terminates_direct_and_mutual_recursion() {
    assert_eq!(texts("#define f(x) f(x)\nf(1)\n"), ["f", "(", "1", ")"]);
    assert_eq!(texts("#define a b\n#define b a\na\n"), ["a"]);
}

#[test]
fn argument_preexpansion_precedes_stringization_in_outer_macro() {
    let tu = preprocess_str(
        "fixture.c",
        "#define str(s) #s\n#define xstr(s) str(s)\nxstr(__LINE__)\n",
        Config::default(),
    );
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["\"3\""]);
}

#[test]
fn paste_empty_arguments_and_gnu_comma_swallowing() {
    assert_eq!(texts("#define cat(a,b) a##b\ncat(1,2)\n"), ["12"]);
    assert!(texts("#define f(x) x\nf()\n").is_empty());
    assert_eq!(
        texts("#define D(f, a...) g(f, ##a)\nD(\"x\")\n"),
        ["g", "(", "\"x\"", ")"]
    );
}

#[test]
fn preexpanded_argument_parent_is_the_caller_not_the_callee() {
    let tu = preprocess_str(
        "fixture.c",
        "#define ID(x) x\n#define OUTER ID(__LINE__)\nOUTER\n",
        Config::default(),
    );
    let frames = tu.source_map.expansion_backtrace(tu.tokens[0].span);
    let names: Vec<_> = frames
        .iter()
        .filter_map(|f| f.macro_id)
        .map(|m| tu.source_map.macro_info(m).unwrap().name.as_ref())
        .collect();
    assert_eq!(names, ["OUTER", "__LINE__"]);
}

#[test]
fn source_map_distinguishes_body_and_argument_tokens() {
    let tu = preprocess_str(
        "fixture.c",
        "#define pair(x) left x\npair(right)\n",
        Config::default(),
    );
    let origins: Vec<_> = tu
        .tokens
        .iter()
        .map(|t| tu.source_map.origin(t.span))
        .collect();
    assert!(matches!(origins[0], TokenOrigin::MacroBody(_)));
    assert!(matches!(
        origins[1],
        TokenOrigin::MacroArg { arg_index: 0, .. }
    ));
}

#[test]
fn builtins_are_repeatable_except_for_monotonic_counter() {
    let src = "__COUNTER__ __COUNTER__ __DATE__ __TIME__\n";
    let first = preprocess_str("fixture.c", src, Config::default());
    let second = preprocess_str("fixture.c", src, Config::default());
    assert_eq!(
        first.token_texts().collect::<Vec<_>>(),
        ["0", "1", "\"Jan 01 1970\"", "\"00:00:00\""]
    );
    assert_eq!(
        first.token_texts().collect::<Vec<_>>(),
        second.token_texts().collect::<Vec<_>>()
    );
    assert_eq!(
        first.tokens.iter().map(|t| t.span.ctx).collect::<Vec<_>>(),
        second.tokens.iter().map(|t| t.span.ctx).collect::<Vec<_>>()
    );
    assert_ne!(first.config, ConfigId::default());
}
