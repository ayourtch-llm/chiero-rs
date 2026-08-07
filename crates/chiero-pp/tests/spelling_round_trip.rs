//! Covers: 010 contract 11 — **a token's byte range re-lexes to that token.**
//!
//! > Round trip: for every token in a preprocessed fixture, the byte range given by
//! > `spelling_loc` re-lexes to the same token text.
//!
//! This was the **only 010 contract with no test anywhere**. Mapping `chiero-span`'s five test
//! files against the twenty contracts leaves 1–2, 3–10, 12, 13–17 and 19 covered and 18 declared
//! as needing a large fixture; 11 was simply absent. It lives here rather than in `chiero-span`
//! because the fixture it asks for is a *preprocessed* one, and only the preprocessor can make
//! one.
//!
//! # Why the invariant is load-bearing
//!
//! Every provenance query, every gcov correlation and every diagnostic location is a byte range
//! interpreted against a file. §11.3 records that **path identity is this project's recurring
//! silent failure** — a lookup that misses reads as success, three times in three places — and a
//! span that points at the wrong bytes fails exactly that way: nothing errors, the location is
//! merely wrong. Re-lexing is the one check that cannot be satisfied by a plausible-looking
//! number.
//!
//! # What is excluded, and why it is a *typed* exclusion
//!
//! A pasted or stringized token has no contiguous source text: 011 §2.2 gives it a zero-width
//! span at the operator's position and keeps its text in a side table. `TokenOrigin::Synthesized`
//! names exactly that set, so the exclusion is the type's own answer rather than a heuristic
//! about which tokens look odd — and the count of skipped tokens is asserted to be a *minority*,
//! because a test that silently skipped everything would pass.

use chiero_lex::{LexConfig, LexSession, PpTokenKind};
use chiero_pp::{Config, preprocess_str};
use chiero_span::TokenOrigin;

/// Re-lex `text` in isolation and return its non-`Eof` token texts.
fn relex(text: &str) -> Vec<String> {
    let mut map = chiero_span::SourceMap::new();
    let file = map.add_file("relex.c", text.to_owned());
    let lexed = LexSession::new().lex(&map, file, LexConfig::default());
    lexed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.kind, PpTokenKind::Eof))
        .map(|token| lexed.text(token).to_owned())
        .collect()
}

/// Check the contract over one preprocessed fixture; returns `(checked, skipped)`.
#[track_caller]
fn round_trips(name: &str, src: &str) -> (usize, usize) {
    let tu = preprocess_str(name, src, Config::default());
    let (mut checked, mut skipped) = (0, 0);
    for (index, token) in tu.tokens.iter().enumerate() {
        if matches!(token.kind, PpTokenKind::Eof) {
            continue;
        }
        // The typed exclusion: a synthesized token has no contiguous source text at all.
        if matches!(tu.source_map.origin(token.span), TokenOrigin::Synthesized) {
            skipped += 1;
            continue;
        }
        let Some(loc) = tu.source_map.spelling_loc(token.span) else {
            panic!("{name}: token {index} has no spelling location");
        };
        let file = tu.source_map.file(loc.file);
        let start = (token.span.lo.0 - file.start_pos.0) as usize;
        let end = (token.span.hi.0 - file.start_pos.0) as usize;
        assert!(
            start <= end && end <= file.src().len(),
            "{name}: token {index} span {start}..{end} is outside {} ({} bytes)",
            file.path().display(),
            file.src().len()
        );
        let slice = &file.src()[start..end];
        let expected = tu.text_at(index).unwrap_or_default();
        assert_eq!(
            relex(slice),
            vec![expected.to_owned()],
            "{name}: token {index} ({expected:?}) does not re-lex from its own byte range \
             {start}..{end}, which is {slice:?}"
        );
        checked += 1;
    }
    (checked, skipped)
}

/// The contract over a fixture exercising every construct that has a span rule of its own.
#[test]
fn every_token_re_lexes_from_its_own_byte_range() {
    let cases = [
        ("plain", "int x = 1 + 2;\n"),
        // 010 §2.2: a body token's span points into the macro definition, an argument's into
        // the call site. Both must slice correctly, and they are in different files' worth of
        // the global space.
        ("object-like", "#define A 7\nint y = A;\n"),
        ("function-like", "#define f(a) [a]\nf(1) f(x + 2)\n"),
        ("nested", "#define B(x) [x]\n#define A(x) B(x)\nA(1) A(2)\n"),
        // 011 §2.2: a spliced token's bytes are not contiguous, and its span covers the whole
        // physical extent including the `\` and the newline.
        ("splice", "#define FOO ba\\\nr\nFOO\n"),
        ("splice-in-code", "int ba\\\nr = 1;\n"),
        ("string-and-char", "char *s = \"a b\"; char c = 'x';\n"),
        ("numbers", "int n = 0x1e+2; double d = 1.0e+5f;\n"),
        ("punctuation", "a <<= b; c >>= d; e ## f;\n"),
        ("comments", "int /* c */ a /* d */ = 1; // trailing\n"),
        // The UCN work of this session: an identifier whose bytes include `À`.
        ("ucn", "int \\u00C0b = 1;\n"),
        ("raw-utf8", "int \u{00C0}b = 1;\n"),
    ];
    let (mut total, mut total_skipped) = (0, 0);
    for (name, src) in cases {
        let (checked, skipped) = round_trips(name, src);
        assert!(checked > 0, "{name}: nothing was checked");
        total += checked;
        total_skipped += skipped;
    }
    // **A test that skipped everything would pass**, so the skip count is bounded rather than
    // merely reported (§11.1: an assertion of absence needs a companion that the run got there).
    assert!(total > 60, "expected a real corpus of tokens, got {total}");
    assert!(
        total_skipped * 4 < total,
        "synthesized tokens should be a small minority: {total_skipped} of {total}"
    );
}

/// Stringize and paste are the tokens the contract excludes — and they must be excluded for the
/// **right reason**, which is that `origin` says so, not that they happen to fail.
#[test]
fn synthesized_tokens_are_the_excluded_set_and_no_others() {
    // `int v =` and `;` are ordinary tokens, so this fixture exercises **both** halves —
    // without them every output token is synthesized and `checked` is legitimately 0, which
    // makes the round-trip half of this test vacuous.
    let src = "#define str(x) #x\n#define cat(a,b) a##b\nint v = str(hello) cat(1,2);\n";
    let tu = preprocess_str("synth.c", src, Config::default());
    let synthesized: Vec<_> = tu
        .tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| !matches!(token.kind, PpTokenKind::Eof))
        .filter(|(_, token)| matches!(tu.source_map.origin(token.span), TokenOrigin::Synthesized))
        .filter_map(|(index, _)| tu.text_at(index))
        .collect();
    assert_eq!(
        synthesized,
        vec!["\"hello\"", "12"],
        "exactly the stringized and pasted tokens are synthesized"
    );
    // And the rest of that same fixture round-trips.
    let (checked, skipped) = round_trips("synth", src);
    assert_eq!(skipped, 2);
    assert!(checked > 0);
}
