//! Covers: 011 contracts 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 14.

use chiero_lex::{EncPrefix, LexConfig, LexSession, PpTokenKind, Punct};
use chiero_span::SourceMap;

fn lex(src: &str) -> chiero_lex::LexedFile {
    let mut map = SourceMap::new();
    let file = map.add_file("fixture.c", src);
    LexSession::new().lex(&map, file, LexConfig::default())
}

fn texts(lexed: &chiero_lex::LexedFile) -> Vec<&str> {
    lexed
        .tokens()
        .iter()
        .filter(|t| !matches!(t.kind, PpTokenKind::Eof))
        .map(|t| lexed.text(t))
        .collect()
}

#[test]
fn pp_numbers_are_single_tokens() {
    for number in ["0x1e+2", "1.0e+5f", ".5", "0b1010", "1'000"] {
        let out = lex(number);
        assert_eq!(out.tokens().len(), 2, "{number}");
        assert!(matches!(out.tokens()[0].kind, PpTokenKind::Number));
        assert_eq!(out.text(&out.tokens()[0]), number);
    }
}

#[test]
fn splice_forms_one_identifier_with_physical_span() {
    let out = lex("ba\\\nr");
    let token = &out.tokens()[0];
    assert!(matches!(token.kind, PpTokenKind::Ident(_)));
    assert_eq!(out.text(token), "bar");
    assert_eq!(token.span.len(), 5);

    let crlf = lex("ba\\\r\nr");
    assert_eq!(crlf.text(&crlf.tokens()[0]), "bar");
    assert_eq!(crlf.tokens()[0].span.len(), 6);
}

#[test]
fn comments_are_one_separator_and_never_join_identifiers() {
    for src in ["a// comment\nb", "a/* comment */b", "a/**/b"] {
        let out = lex(src);
        assert_eq!(texts(&out), ["a", "b"], "{src:?}");
        assert!(out.tokens()[1].leading_space, "{src:?}");
    }
}

#[test]
fn malformed_input_recovers_at_the_specified_boundary() {
    let string = lex("\"unterminated\nnext");
    assert!(matches!(
        string.tokens()[0].kind,
        PpTokenKind::StringLit {
            prefix: EncPrefix::None
        }
    ));
    assert_eq!(string.text(&string.tokens()[0]), "\"unterminated");
    assert_eq!(string.diagnostics().len(), 1);
    assert_eq!(texts(&string), ["\"unterminated", "next"]);

    let comment = lex("a /* never ends");
    assert_eq!(texts(&comment), ["a"]);
    assert_eq!(comment.diagnostics().len(), 1);
    assert!(matches!(
        comment.tokens().last().unwrap().kind,
        PpTokenKind::Eof
    ));

    let stray = lex("@");
    assert!(matches!(stray.tokens()[0].kind, PpTokenKind::Other('@')));
    assert!(stray.diagnostics().is_empty());
}

#[test]
fn punctuators_use_maximal_munch_including_digraphs() {
    let out = lex("<<= >>= ... -> ++ ## %:%: <::>");
    assert_eq!(
        out.tokens()
            .iter()
            .filter_map(|t| match t.kind {
                PpTokenKind::Punct(p) => Some(p),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            Punct::ShlEq,
            Punct::ShrEq,
            Punct::Ellipsis,
            Punct::Arrow,
            Punct::PlusPlus,
            Punct::HashHash,
            Punct::HashHash,
            Punct::LBracket,
            Punct::RBracket,
        ]
    );
}

#[test]
fn keywords_remain_identifiers_and_bol_is_logical() {
    let out = lex("int \\\nwhile\n  if");
    assert!(
        out.tokens()
            .iter()
            .take(3)
            .all(|t| matches!(t.kind, PpTokenKind::Ident(_)))
    );
    assert!(out.tokens()[0].bol);
    assert!(
        !out.tokens()[1].bol,
        "a splice does not begin a logical line"
    );
    assert!(out.tokens()[2].bol);
}

#[test]
fn trigraphs_are_opt_in() {
    let default = lex("??=");
    assert_eq!(texts(&default), ["?", "?", "="]);
    let mut map = SourceMap::new();
    let file = map.add_file("fixture.c", "??=");
    let enabled = LexSession::new().lex(&map, file, LexConfig { trigraphs: true });
    assert_eq!(texts(&enabled), ["#"]);
}
