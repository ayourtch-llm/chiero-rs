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

/// **`$` belongs to an identifier** — the false positive this audit found first.
///
/// gcc takes `$` in an identifier in *both* modes; it is a GNU extension old enough that C never
/// reclaimed the character, and VPP has a file that uses it. chiero's lexer did not, so `int $x =
/// 1;` reached 013 as an unexpected token and came back as six "expected a declaration"
/// messages.
///
/// **The stray-character rule that came with it is 012's, not 010's**, and this test's first
/// draft put it here. `Other` is not yet an error: gcc takes `S(a\b)` where `#define S(x) #x`
/// stringizes the backslash, and takes `#define M @` until `M` is used. The lexer cannot know
/// whether the token survives, so it classifies and says nothing — and an existing macro fixture
/// refuted the first draft within one test run.
#[test]
fn a_dollar_is_an_identifier_character_and_other_is_not_yet_an_error() {
    for good in ["int $x = 1;\n", "int a$b = 1;\n", "int $ = 1;\n"] {
        assert!(lexed(good).is_empty(), "must lex cleanly: `{good}`");
    }

    // Classified, not judged: no diagnostic here, whatever 012 later decides.
    for quiet in ["int x = 1 \\ 2;\n", "int @x;\n", "int `x;\n"] {
        assert!(
            lexed(quiet).is_empty(),
            "010 classifies and does not judge: `{quiet}` -> {:?}",
            lexed(quiet)
        );
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
    // Five sources, five diagnostics — the stray-character rows moved to 012 with the rule.
    assert!(checked >= 5, "only {checked} diagnostics were examined");
    assert!(
        invisible.is_empty(),
        "{} diagnostic(s) point at no visible text:\n  {}",
        invisible.len(),
        invisible.join("\n  ")
    );
}
