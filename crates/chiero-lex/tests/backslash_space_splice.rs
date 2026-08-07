//! Phase 2: **a backslash separated from the newline by whitespace still splices.**
//!
//! C11 5.1.1.2p1.2 says a backslash *immediately* before a newline is deleted, so `\ ` + newline
//! is not a splice by the letter of the standard. gcc and clang both splice it anyway and warn —
//! `backslash and newline separated by space` — and C99 6.10.3.4p6's own worked example contains
//! one, which is how this surfaced: chiero answered *stray `\` in program* and lost the rest of
//! the macro definition.
//!
//! **Both halves matter.** Splicing without the diagnostic silently accepts what both compilers
//! call out; diagnosing without splicing loses the program, which is what chiero did.

use chiero_lex::{LexConfig, LexSession, PpTokenKind};
use chiero_span::SourceMap;

fn lex(src: &str) -> (Vec<String>, Vec<String>) {
    let mut map = SourceMap::new();
    let file = map.add_file("s.c", src.to_owned());
    let lexed = LexSession::new().lex(&map, file, LexConfig::default());
    (
        lexed
            .tokens()
            .iter()
            .filter(|t| !matches!(t.kind, PpTokenKind::Eof))
            .map(|t| lexed.text(t).to_owned())
            .collect(),
        lexed.diagnostics().iter().map(|d| d.message.clone()).collect(),
    )
}

/// The splice happens, so the two lines are one logical line.
#[test]
fn a_backslash_separated_from_the_newline_by_a_space_splices() {
    let (texts, diagnostics) = lex("int a\\ \nb = 1;\n");
    assert_eq!(texts, vec!["int", "ab", "=", "1", ";"], "{texts:?}");
    assert!(
        diagnostics.iter().any(|d| d.contains("backslash")),
        "both compilers warn about this: {diagnostics:?}"
    );
}

/// Tabs too, and several of them — gcc accepts any horizontal whitespace run.
#[test]
fn any_horizontal_whitespace_run_still_splices() {
    for gap in [" ", "\t", "  \t ", "\t\t"] {
        let (texts, _) = lex(&format!("int a\\{gap}\nb = 1;\n"));
        assert_eq!(texts, vec!["int", "ab", "=", "1", ";"], "gap {gap:?}");
    }
}

/// **The ordinary splice is silent.** Without this, "warn on every splice" passes the tests above
/// and puts a diagnostic on `\` at the end of every multi-line macro in the tree.
#[test]
fn an_immediate_backslash_newline_splices_without_complaint() {
    let (texts, diagnostics) = lex("int a\\\nb = 1;\n");
    assert_eq!(texts, vec!["int", "ab", "=", "1", ";"]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// **A backslash that is not before a newline is still stray.** The fix must not make every
/// backslash disappear.
#[test]
fn a_backslash_in_the_middle_of_a_line_is_untouched() {
    let (texts, _) = lex("int a \\ b = 1;\n");
    assert!(texts.iter().any(|t| t == "\\"), "{texts:?}");
}
