//! Covers: 011 contract 15 — **a universal character name is an identifier character.**
//!
//! C11 6.4.2.1 puts *universal-character-name* in `identifier-nondigit`, so `À` in an
//! identifier is a character, not an escape and not a stray backslash. chiero answered
//! *stray `\` in program* — a diagnostic on valid C11 — while the **raw UTF-8** spelling of the
//! same identifier already lexed correctly, because the byte rule `byte >= 0x80` admits it. So
//! the gap was only ever the escaped spelling, which is what makes it a clean lexer fix.
//!
//! # What is asserted, and what deliberately is not
//!
//! **Identity, not spelling.** `gcc -E` *normalizes* a UCN in its output (`À` comes back as
//! `\U000000c0`) and chiero preserves what was written, because 010 contract 11 requires a
//! token's byte range to re-lex to its own spelling and a normalized spelling is a byte range
//! that exists in no file. 011 §2.0 declares that divergence. So these tests assert **how many
//! tokens there are and where they end**, and use gcc only for the yes/no question it answers
//! unambiguously: does a real compiler accept this program.
//!
//! The macro-name case lives in `chiero-pp`'s tests, since `chiero-lex` cannot depend on the
//! preprocessor — but it is the reason the fix belongs *here*: `chiero-pp` looks macros up by
//! token text, so a name split into three tokens is a macro that can never be called.

use chiero_lex::{LexConfig, LexSession, PpTokenKind};
use chiero_span::SourceMap;

/// Every non-`Eof` token's text.
fn lex(src: &str) -> (Vec<String>, usize) {
    let mut map = SourceMap::new();
    let file = map.add_file("ucn.c", src.to_owned());
    let lexed = LexSession::new().lex(&map, file, LexConfig::default());
    let texts = lexed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.kind, PpTokenKind::Eof))
        .map(|token| lexed.text(token).to_owned())
        .collect();
    (texts, lexed.diagnostics().len())
}

/// Does gcc 13 accept this translation unit? The one question a compiler answers without
/// ambiguity, and the only one this file asks it.
fn gcc_accepts(src: &str) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("chiero-ucn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("u{}.c", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, src).unwrap();
    let output = std::process::Command::new("gcc")
        .args(["-std=c11", "-E", "-P"])
        .arg(&path)
        .output()
        .expect("gcc is required for the UCN oracle");
    let _ = std::fs::remove_file(&path);
    output.status.success()
}

/// A UCN **starts** an identifier, in both spellings, and the identifier is one token.
#[test]
fn a_universal_character_name_begins_an_identifier() {
    for src in ["int \\u00C0 = 1;\n", "int \\U000000C0 = 1;\n"] {
        assert!(gcc_accepts(src), "fixture must be valid C11: {src}");
        let (texts, diagnostics) = lex(src);
        assert_eq!(texts.len(), 5, "expected `int IDENT = 1 ;`, got {texts:?}");
        assert_eq!(texts[0], "int");
        assert!(
            texts[1].starts_with('\\'),
            "the identifier keeps its written spelling (011 §2.0): {texts:?}"
        );
        assert_eq!(texts[2], "=");
        assert_eq!(diagnostics, 0, "valid C11 must not be diagnosed: {src}");
    }
}

/// A UCN **continues** an identifier, and does not split it.
///
/// `àb` is one identifier. Splitting it is the failure that produced `a`, `\`, `u0300b` —
/// three tokens where C has one, which no amount of later analysis recovers from.
#[test]
fn a_universal_character_name_continues_an_identifier() {
    let src = "int a\\u0300b = 2;\n";
    assert!(gcc_accepts(src), "fixture must be valid C11");
    let (texts, diagnostics) = lex(src);
    assert_eq!(texts.len(), 5, "expected one identifier, got {texts:?}");
    assert_eq!(texts[1], "a\\u0300b");
    assert_eq!(diagnostics, 0);
}

/// **The raw spelling already worked and must keep working** — the control that says this fix
/// changed the escaped path and nothing else.
#[test]
fn a_raw_utf8_identifier_still_lexes_as_one_token() {
    let src = "int \u{00C0}b = 3;\n";
    assert!(gcc_accepts(src));
    let (texts, diagnostics) = lex(src);
    assert_eq!(texts.len(), 5, "{texts:?}");
    assert_eq!(texts[1], "\u{00C0}b");
    assert_eq!(diagnostics, 0);
}

/// **A malformed UCN is not a UCN**, and phase 3 does not invent a diagnosis for it.
///
/// `a\u12` has too few hex digits, so it is not a universal-character-name at all. gcc accepts
/// the program; whatever the backslash then is, it is not this contract's business.
#[test]
fn too_few_hex_digits_is_not_a_universal_character_name() {
    let src = "int a\\u12 = 1;\n";
    assert!(gcc_accepts(src), "gcc accepts a malformed UCN at phase 3");
    let (texts, _) = lex(src);
    assert!(
        texts.iter().any(|t| t.contains("12")),
        "the text must survive in some form: {texts:?}"
    );
}

/// C11 6.4.3p2 — a UCN may not designate a **basic-character-set** code point or a surrogate.
///
/// Without this, `Abc` and `Abc` are two spellings of one identifier, which is exactly what
/// the constraint exists to prevent. gcc calls both an error.
#[test]
fn a_ucn_for_a_basic_character_or_surrogate_is_diagnosed() {
    for src in ["int a\\u0041 = 1;\n", "int a\\uD800 = 1;\n"] {
        assert!(!gcc_accepts(src), "fixture must be invalid C11: {src}");
        let (_, diagnostics) = lex(src);
        assert!(diagnostics > 0, "chiero accepted what gcc rejects: {src}");
    }
    // ⚠️ **Two rules, not one — and this fixture asserted otherwise until gcc rejected it.**
    // 6.4.3p2 carves `$`, `@` and `` ` `` out of the basic-set prohibition, so `\\u0040` *is* a
    // well-formed universal-character-name. `@` is still not an identifier character, and gcc
    // says so with a different message. Only `$` gets in, by the same GNU extension that admits
    // a literal `$`.
    let dollar = "int a\\u0024 = 1;\n";
    assert!(gcc_accepts(dollar), "`$` is an identifier character");
    assert_eq!(lex(dollar).1, 0, "a UCN naming `$` is legal in an identifier");
    for src in ["int a\\u0040 = 1;\n", "int a\\u0060 = 1;\n"] {
        assert!(!gcc_accepts(src), "fixture must be invalid C11: {src}");
        let (_, diagnostics) = lex(src);
        assert!(diagnostics > 0, "a valid UCN naming a non-identifier character: {src}");
    }
}

/// Annex D.2 — an identifier may not **begin** with a combining mark.
///
/// The companion is the same code point in a legal position: `à` is fine and `̀`
/// alone is not, so a fix that simply rejects the range everywhere fails the first row.
#[test]
fn a_combining_mark_may_not_start_an_identifier() {
    let bad = "int \\u0300 = 1;\n";
    assert!(!gcc_accepts(bad), "fixture must be invalid C11");
    let (_, diagnostics) = lex(bad);
    assert!(diagnostics > 0, "chiero accepted what gcc rejects: {bad}");

    let good = "int a\\u0300 = 1;\n";
    assert!(gcc_accepts(good), "fixture must be valid C11");
    let (_, diagnostics) = lex(good);
    assert_eq!(diagnostics, 0, "legal in a continuing position: {good}");
}
