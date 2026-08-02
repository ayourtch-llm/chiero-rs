//! **What the lexer refuses, and how it says so** — the audit §9 held for `chiero-lex`.
//!
//! Every other crate has a constraints test and a span gate; this one had neither, and nothing
//! in the project had ever rendered one of its two diagnostics. Three findings came out of
//! looking, and only one of them is a missing rule.

use chiero_lex::{LexConfig, LexSession};
use chiero_span::SourceMap;

/// The messages the lexer produced for one source, each with the text its span covers.
fn lexed(src: &str) -> Vec<(String, String)> {
    let mut map = SourceMap::new();
    let file = map.add_file("t.c", src);
    let out = LexSession::new().lex(&map, file, LexConfig::default());
    out.diagnostics()
        .iter()
        .map(|d| {
            (
                d.message.clone(),
                map.span_text(d.span).unwrap_or("<none>").to_owned(),
            )
        })
        .collect()
}

/// **A stray character is named by the lexer, not spelled out by the parser** (C 6.4p3).
///
/// `\`, `@` and a backtick are not in C's source character set outside a literal, and gcc says
/// "stray `@` in program". chiero's lexer passes them through as `Other`, and 013 then produces
/// three to six "expected a declaration" messages, none of which contains the character. That is
/// wave 370's `_Pragma` shape exactly: the fault is recognised nowhere and spelled out by whoever
/// trips over it next.
#[test]
fn a_stray_character_is_named_where_it_is_found() {
    for (src, want) in [
        ("int x = 1 \\ 2;\n", "\\"),
        ("int @x;\n", "@"),
        ("int `x;\n", "`"),
    ] {
        let got = lexed(src);
        assert_eq!(
            got,
            vec![(format!("stray `{want}` in program"), want.to_owned())],
            "the diagnostic for `{src}`"
        );
    }

    // **`$` is not stray.** gcc takes it in an identifier in *both* modes — it is a GNU extension
    // old enough that C never reclaimed the character — and chiero's parser was producing six
    // messages for `int $x = 1;`. That is a false positive, which is the more expensive half of
    // this test.
    for good in ["int $x = 1;\n", "int a$b = 1;\n", "int $ = 1;\n"] {
        assert!(lexed(good).is_empty(), "must lex cleanly: `{good}`");
    }

    for good in [
        "int x = 1;\n",
        "char *s = \"a\\\\b\";\n",
        "char c = '\\\\';\n",
        "/* a \\\\ in a comment */\nint x;\n",
        "// a @ in a line comment\nint x;\n",
    ] {
        assert!(lexed(good).is_empty(), "must lex cleanly: `{good}`");
    }
}

/// **An unterminated literal says which kind it was** (C 6.4.4.4, 6.4.5).
///
/// Both spellings produced "unterminated literal". gcc distinguishes them — "missing terminating
/// `\"` character" against "missing terminating `'` character" — and on a long line with both a
/// string and a character constant that is the whole of what a reader needs.
#[test]
fn an_unterminated_literal_says_which_kind() {
    assert_eq!(
        lexed("char *s = \"abc\n"),
        vec![(
            "missing terminating `\"` character".to_owned(),
            "\"abc".to_owned()
        )]
    );
    assert_eq!(
        lexed("char c = 'a\n"),
        vec![(
            "missing terminating `'` character".to_owned(),
            "'a".to_owned()
        )]
    );
    // The prefixed spellings reach the same code and must say the same thing.
    assert_eq!(
        lexed("char *s = L\"abc\n")[0].0,
        "missing terminating `\"` character"
    );
    assert_eq!(
        lexed("int c = u'a\n")[0].0,
        "missing terminating `'` character"
    );

    // A block comment keeps its own message, which is already precise.
    assert_eq!(lexed("/* abc\n")[0].0, "unterminated block comment");
}

/// **Every lexer diagnostic points at visible text** (023 §9) — the fourth and last crate to get
/// wave 373's gate.
#[test]
fn every_lex_diagnostic_points_at_visible_text() {
    let mut invisible: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for src in [
        "char *s = \"abc\n",
        "char c = 'a\n",
        "/* abc\n",
        "int x = 1 \\ 2;\n",
        "int @x;\n",
        "int `x;\n",
        "char *s = L\"abc\n",
        "int c = u'a\n",
    ] {
        for (message, covered) in lexed(src) {
            checked += 1;
            if covered.is_empty() || covered == "<none>" {
                invisible.push(format!("{src:?}: {message}"));
            }
        }
    }
    assert!(checked >= 8, "only {checked} diagnostics were examined");
    assert!(
        invisible.is_empty(),
        "{} diagnostic(s) point at no visible text:\n  {}",
        invisible.len(),
        invisible.join("\n  ")
    );
}
