//! Covers: 012 contracts 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 18, 19.

use chiero_pp::{Config, ConfigId, preprocess_str};
use chiero_span::TokenOrigin;
use std::collections::BTreeSet;
use std::process::Command;

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
fn variadic_and_paste_edge_cases_are_semantically_pinned() {
    let comma_swallow = preprocess_str(
        "varargs.c",
        "#define D(f,a...) g(f,##a)\nD(\"x\")\n",
        Config::default(),
    );
    assert!(
        comma_swallow.diagnostics.is_empty(),
        "{:?}",
        comma_swallow.diagnostics
    );
    assert_eq!(
        comma_swallow.token_texts().collect::<Vec<_>>(),
        ["g", "(", "\"x\"", ")"]
    );

    assert_eq!(
        texts("#define V(...) [__VA_ARGS__]\nV(1,2,3)\n"),
        ["[", "1", ",", "2", ",", "3", "]"]
    );
    assert_eq!(
        texts("#define CAT(a,b) a##b\nCAT(__COUNTER__,x) __COUNTER__\n"),
        ["__COUNTER__x", "0"]
    );
}

#[test]
fn invalid_paste_diagnoses_and_preserves_both_operands() {
    let tu = preprocess_str(
        "paste.c",
        "#define CAT(a,b) a##b\nCAT(x,+)\n",
        Config::default(),
    );
    assert_eq!(tu.diagnostics.len(), 1, "{:?}", tu.diagnostics);
    assert!(
        tu.diagnostics[0]
            .message
            .contains("not one preprocessing token")
    );
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["x", "+"]);
}

#[test]
fn function_like_definition_requires_no_space_before_the_parameter_list() {
    assert_eq!(
        texts("#define f (x)\nf(1)\n"),
        ["(", "x", ")", "(", "1", ")"]
    );
    assert_eq!(texts("#define empty() ok\nempty()\n"), ["ok"]);
}

#[test]
fn pasted_tokens_retain_the_callers_hide_set() {
    assert_eq!(
        texts("#define CAT(a,b) a##b\n#define A() CAT(A,)\nA()\n"),
        ["A"]
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

#[test]
fn invocation_can_span_logical_lines() {
    let src = "#define f(a,b) < a , b >\nf(1,\n2)\n";
    assert_eq!(texts(src), ["<", "1", ",", "2", ">"]);
}

#[test]
fn replacement_rescan_includes_following_source_tokens() {
    let src = "#define B(x) [x]\n#define A B\nA(1)\n";
    assert_eq!(texts(src), ["[", "1", "]"]);
}

#[test]
fn nested_parentheses_stay_inside_one_argument() {
    let src = "#define pair(a,b) [ a ] [ b ]\npair((1,2),3)\n";
    assert_eq!(
        texts(src),
        ["[", "(", "1", ",", "2", ")", "]", "[", "3", "]"]
    );
}

#[test]
fn function_macro_arity_mismatches_are_diagnosed_without_fabricating_output() {
    for src in [
        "#define two(a,b) [a|b]\ntwo(1)\n",
        "#define two(a,b) [a|b]\ntwo(1,2,3)\n",
    ] {
        let tu = preprocess_str("arity.c", src, Config::default());
        assert_eq!(tu.diagnostics.len(), 1, "{src}: {:?}", tu.diagnostics);
        assert!(
            tu.diagnostics[0].message.contains("argument"),
            "{src}: {:?}",
            tu.diagnostics
        );
        assert_eq!(
            tu.token_texts().next(),
            Some("two"),
            "malformed invocation must remain distinguishable from expanded output"
        );
    }
}

#[test]
fn taken_if_skips_else_and_elif_is_evaluated() {
    assert_eq!(texts("#if 1\nfirst\n#else\nwrong\n#endif\n"), ["first"]);
    assert_eq!(
        texts("#if 0\nwrong\n#elif 1\nsecond\n#else\nwrong2\n#endif\n"),
        ["second"]
    );
}

#[test]
fn paste_applies_to_object_macros_and_deletes_placemarkers() {
    assert_eq!(texts("#define A x ## y\nA\n"), ["xy"]);
    assert!(texts("#define h(a,b) a##b\nh(,)\n").is_empty());
}

#[test]
fn nonempty_gnu_varargs_keep_a_separate_comma_token() {
    let tu = preprocess_str(
        "fixture.c",
        "#define D(f,a...) g(f,##a)\nD(\"x\",1)\n",
        Config::default(),
    );
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>(),
        ["g", "(", "\"x\"", ",", "1", ")"]
    );
    assert!(matches!(
        tu.tokens[3].kind,
        chiero_lex::PpTokenKind::Punct(chiero_lex::Punct::Comma)
    ));
}

#[test]
fn paste_and_stringize_use_raw_arguments_without_consuming_counter() {
    assert_eq!(
        texts("#define X 1\n#define cat(a,b) a##b\ncat(X,2)\n"),
        ["X2"]
    );
    assert_eq!(
        texts("#define str(x) #x\nstr(__COUNTER__) __COUNTER__\n"),
        ["\"__COUNTER__\"", "0"]
    );
}

#[test]
fn argument_preexpansion_side_effects_are_left_to_right() {
    assert_eq!(
        texts("#define pair(z,a) z a\npair(__COUNTER__,__COUNTER__)\n"),
        ["0", "1"]
    );
}

#[test]
fn blue_paint_follows_argument_derived_tokens() {
    assert_eq!(texts("#define f(x) x\nf(f)(1)\n"), ["f", "(", "1", ")"]);
}

#[test]
fn function_expansion_parent_and_operator_locations_are_real() {
    let nested = preprocess_str(
        "fixture.c",
        "#define F(x) body\n#define OUT F(1)\nOUT\n",
        Config::default(),
    );
    let names: Vec<_> = nested
        .source_map
        .expansion_backtrace(nested.tokens[0].span)
        .iter()
        .filter_map(|frame| frame.macro_id)
        .map(|id| nested.source_map.macro_info(id).unwrap().name.as_ref())
        .collect();
    assert_eq!(names, ["OUT", "F"]);

    for src in [
        "#define str(x) #x\nstr(value)\n",
        "#define cat(x,y) x##y\ncat(a,b)\n",
    ] {
        let tu = preprocess_str("operator.c", src, Config::default());
        let loc = tu.source_map.expansion_loc(tu.tokens[0].span).unwrap();
        assert_eq!(loc.line, 2, "{src}");
        assert_eq!(
            tu.source_map.file(loc.file).path(),
            std::path::Path::new("operator.c")
        );
    }
}

#[test]
fn va_opt_is_out_of_v1_scope_and_diagnosed() {
    let tu = preprocess_str(
        "fixture.c",
        "#define F(...) __VA_OPT__(x)\nF(1)\n",
        Config::default(),
    );
    assert_eq!(tu.diagnostics.len(), 1);
    assert!(tu.diagnostics[0].message.contains("__VA_OPT__"));
    assert!(
        !tu.token_texts().any(|text| text == "__VA_OPT__"),
        "out-of-scope syntax must not silently pass through"
    );
}

#[test]
fn stringize_collapses_internal_whitespace_and_comments() {
    assert_eq!(texts("#define str(x) #x\nstr(a   b/**/c)\n"), ["\"a b c\""]);
}

#[test]
fn substitution_inherits_parameter_spacing_and_stringize_escapes_only_literals() {
    assert_eq!(
        texts("#define S(x) #x\n#define T(x) S(a x)\nT(b)\n"),
        ["\"a b\""]
    );
    assert_eq!(
        texts(
            "#define S(x) #x\n#define ASSERT(t) S(t)\n#define ELT(h,i) ASSERT((i) < len(h))\nELT(a, handle)\n"
        ),
        ["\"(handle) < len(a)\""]
    );
    assert_eq!(texts("#define S(x) #x\nS(a\\b)\n"), ["\"a\\b\""]);
    assert_eq!(
        texts("#define S(x) #x\nS(\"a\\\\b\")\n"),
        ["\"\\\"a\\\\\\\\b\\\"\""]
    );
}

#[test]
fn pragma_operator_accepts_a_macro_produced_string() {
    let tu = preprocess_str(
        "pragma.c",
        "#define STR(x) #x\n#define P(x) _Pragma (STR(GCC diagnostic x))\nP(push)\nafter\n",
        Config::default(),
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["after"]);
}

#[test]
fn deterministic_context_numbering_covers_nested_user_macros() {
    let src = "#define B(x) [x]\n#define A(x) B(x)\nA(1) A(2)\n";
    let first = preprocess_str("deterministic.c", src, Config::default());
    let second = preprocess_str("deterministic.c", src, Config::default());
    assert_eq!(
        first.token_texts().collect::<Vec<_>>(),
        second.token_texts().collect::<Vec<_>>()
    );
    let first_ctx: Vec<_> = first.tokens.iter().map(|token| token.span.ctx).collect();
    let second_ctx: Vec<_> = second.tokens.iter().map(|token| token.span.ctx).collect();
    assert_eq!(first_ctx, second_ctx);
    assert!(
        first_ctx
            .iter()
            .filter(|context| !context.is_root())
            .collect::<BTreeSet<_>>()
            .len()
            >= 4,
        "fixture must exercise nested and repeated user expansion contexts"
    );
}

#[test]
fn expansion_chain_child() {
    if std::env::var_os("CHIERO_DEEP_EXPANSION_CHILD").is_none() {
        return;
    }
    let mut src = String::new();
    for index in 0..20_000 {
        src.push_str(&format!("#define M{index} M{}\n", index + 1));
    }
    src.push_str("M0\n");
    let tu = preprocess_str("deep.c", &src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["M20000"]);
}

#[test]
fn expansion_depth_counts_nesting_not_sequential_calls() {
    let mut src = String::from("#define M(x) (x)\n");
    for index in 0..400 {
        src.push_str(&format!("int v{index} = M({index});\n"));
    }
    let tu = preprocess_str("sequential.c", &src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(
        tu.token_texts().filter(|text| *text == "M").count(),
        0,
        "every independent invocation must expand"
    );
}

#[test]
fn line_builtin_uses_expansion_location_not_macro_spelling() {
    let tu = preprocess_str(
        "line.c",
        "#define HERE __LINE__\n\n\nHERE\n",
        Config::default(),
    );
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["4"]);
}

#[test]
fn stringize_and_paste_tokens_are_synthesized_and_relexed() {
    let stringized = preprocess_str(
        "origin.c",
        "#define str(x) #x\nstr(value)\n",
        Config::default(),
    );
    assert!(matches!(
        stringized.source_map.origin(stringized.tokens[0].span),
        TokenOrigin::Synthesized
    ));

    let pasted = preprocess_str(
        "kind.c",
        "#define cat(a,b) a##b\ncat(L,\"wide\")\n",
        Config::default(),
    );
    assert_eq!(pasted.token_texts().collect::<Vec<_>>(), ["L\"wide\""]);
    assert!(matches!(
        pasted.tokens[0].kind,
        chiero_lex::PpTokenKind::StringLit {
            prefix: chiero_lex::EncPrefix::Wide
        }
    ));
}

#[test]
fn deep_macro_chain_does_not_abort_the_process() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "expansion_chain_child"])
        .env("CHIERO_DEEP_EXPANSION_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success(), "deep expansion child aborted: {status}");
}

/// **The recorded `Variadic` kind, which nothing inside this crate reads.**
///
/// `MacroKind::FunctionLike { variadic }` is part of `PreprocessedTu::macro_defs` and therefore
/// public API. Expansion does **not** use it: it reads a separate `std_variadic` /
/// `variadic_name` pair on the same definition, so the enum is written and never matched except
/// for its `Named` arm, which only feeds symbol interning.
///
/// **Both directions of the branch that chooses it were unfalsifiable** (wave 293's sweep of
/// `chiero-pp`): forcing `std_variadic` to `false` records `No` for every `...` macro and forcing
/// it to `true` records `Std` for every fixed-arity one, and all 67 tests passed either way —
/// including the one that expands `#define V(...) [__VA_ARGS__]`, because that expansion consults
/// the other representation entirely.
///
/// Two representations of one fact, one of them unread, is the shape this project keeps paying
/// for. This pins them together so they cannot drift apart silently.
#[test]
fn a_macros_recorded_variadic_kind_matches_how_it_was_written() {
    use chiero_pp::{MacroKind, Variadic};

    let tu = preprocess_str(
        "fixture.c",
        "#define FIXED(a,b) a\n#define STD(...) __VA_ARGS__\n#define GNU(x, rest...) rest\n\
         #define OBJ 1\n",
        Config::default(),
    );
    assert!(
        tu.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        tu.diagnostics
    );

    let kind_of = |name: &str| {
        tu.macro_defs
            .iter()
            .find(|d| tu.symbol_text(d.name) == Some(name))
            .map(|d| d.kind.clone())
            .unwrap_or_else(|| panic!("no macro `{name}` in {:?}", tu.macro_defs.len()))
    };

    match kind_of("FIXED") {
        MacroKind::FunctionLike { variadic, params } => {
            assert_eq!(params.len(), 2, "`FIXED(a,b)` has two parameters");
            assert_eq!(
                variadic,
                Variadic::No,
                "a fixed-arity macro is not variadic — the direction a mutant forcing `Std` takes"
            );
        }
        other => panic!("`FIXED` is function-like: {other:?}"),
    }
    match kind_of("STD") {
        MacroKind::FunctionLike { variadic, .. } => assert_eq!(
            variadic,
            Variadic::Std,
            "`...` is C99 variadic — the direction a mutant forcing `No` takes"
        ),
        other => panic!("`STD` is function-like: {other:?}"),
    }
    match kind_of("GNU") {
        MacroKind::FunctionLike { variadic, .. } => assert!(
            matches!(variadic, Variadic::Named(_)),
            "`rest...` is GNU named-variadic, and its name is what symbol interning needs: \
             {variadic:?}"
        ),
        other => panic!("`GNU` is function-like: {other:?}"),
    }
    assert!(
        matches!(kind_of("OBJ"), MacroKind::ObjectLike),
        "an object-like macro has no parameter list at all"
    );
}
