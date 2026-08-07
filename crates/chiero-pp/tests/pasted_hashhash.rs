//! C11 6.10.3.3: **a `##` that arrives by substitution is not the paste operator.**
//!
//! The operator is identified in a macro's *replacement list*, at definition time. A `##` that
//! reaches the same sequence any other way — spelled at a call site, or produced by an earlier
//! paste — is an ordinary punctuator, and the standard's own worked example in 6.10.3.3p4 exists
//! to say so:
//!
//! ```c
//! #define hash_hash # ## #
//! #define mkstr(a) # a
//! #define in_between(a) mkstr(a)
//! #define join(c, d) in_between(c hash_hash d)
//! char p[] = join(x, y);        // "x ## y"
//! ```
//!
//! Found by pointing the simplecpp conformance corpus (`cargo run -p xtask -- pp-gate`) at
//! chiero: `c99-6_10_3_3_p4.c` **panicked**, and so did `macro_paste_hashhash.c`. Every corpus
//! before it was real VPP code, which uses macros as people write them and never reaches this.
//!
//! Both compilers are asked on every case, because a hand-written expectation about `##` is
//! exactly the kind that is confidently wrong.

use chiero_pp::{Config, preprocess_str};
use std::process::{Command, Stdio};

fn ours(src: &str) -> Vec<String> {
    preprocess_str("paste.c", src, Config::default())
        .token_texts()
        .map(str::to_owned)
        .collect()
}

fn compiler_tokens(compiler: &str, src: &str) -> Vec<String> {
    let mut child = Command::new(compiler)
        .args(["-E", "-P", "-std=gnu11", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), src.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{compiler} rejected the fixture");
    let text = String::from_utf8(output.stdout).unwrap();
    let mut map = chiero_span::SourceMap::new();
    let file = map.add_file("compiler-output.c", text);
    let lexed = chiero_lex::LexSession::new().lex(&map, file, chiero_lex::LexConfig::default());
    lexed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.kind, chiero_lex::PpTokenKind::Eof))
        .map(|token| lexed.text(token).to_owned())
        .collect()
}

/// The standard's example, end to end. **This panicked**, in `SourceMap::add_expansion`'s
/// invariant that an expansion's call site lives in its parent context — because the `##`
/// produced by `hash_hash` was taken as an operator and pasted `x` to `y` across two contexts
/// that are not nested.
#[test]
fn c11_6_10_3_3_p4_example() {
    let src = "#define hash_hash # ## #\n\
               #define mkstr(a) # a\n\
               #define in_between(a) mkstr(a)\n\
               #define join(c, d) in_between(c hash_hash d)\n\
               char p[] = join(x, y);\n";
    let gcc = compiler_tokens("gcc", src);
    let clang = compiler_tokens("clang", src);
    assert_eq!(gcc, clang, "the independent compilers must agree first");
    assert_eq!(
        gcc,
        vec!["char", "p", "[", "]", "=", "\"x ## y\"", ";"],
        "the standard states this result, so a compiler disagreeing means the fixture is wrong"
    );
    assert_eq!(ours(src), gcc);
}

/// The same rule reached without a paste producing the `##`: it is spelled at the call site.
///
/// Kept as its own case because it is a **different route to the same rule**, and §7.2's
/// recurring lesson is that a fix attached to the route rather than to the rule comes back
/// through a neighbouring door.
#[test]
fn a_hashhash_spelled_in_an_argument_is_not_an_operator() {
    let src = "#define FOO(x) A x B\nFOO(##);\n";
    let gcc = compiler_tokens("gcc", src);
    let clang = compiler_tokens("clang", src);
    assert_eq!(gcc, clang, "the independent compilers must agree first");
    assert_eq!(gcc, vec!["A", "##", "B", ";"]);
    assert_eq!(ours(src), gcc);
}

/// The reduced panic, and the reduced *loss*.
///
/// `m(hh)` did not panic — it silently dropped the `##` and produced `[ ]`. That is the worse
/// of the two: a crash is loud and a vanished token is a wrong answer nobody is told about.
#[test]
fn a_pasted_hashhash_reaching_a_second_expansion_is_an_ordinary_token() {
    for src in [
        "#define hh # ## #\n#define m(a) [a]\nm(hh)\n",
        "#define hh # ## #\n#define m(a) [a]\nm(x hh y)\n",
        "#define hh # ## #\n#define m(a) [a]\n#define n(a) m(a)\nn(x hh y)\n",
    ] {
        let gcc = compiler_tokens("gcc", src);
        let clang = compiler_tokens("clang", src);
        assert_eq!(gcc, clang, "the independent compilers must agree first: {src}");
        assert_eq!(ours(src), gcc, "chiero diverges on: {src}");
    }
}

/// **The companion assertion that the fix did not simply disable pasting.** An absence test
/// with no positive counterpart is close to asserting nothing (§11.1), and the whole risk of
/// this fix is that it stops `##` working where it *is* the operator.
#[test]
fn a_hashhash_in_a_replacement_list_still_pastes() {
    for src in [
        "#define cat(a,b) a##b\ncat(x,y)\n",
        "#define cat x ## y\ncat\n",
        "#define cat(a,b) a ## b\n#define m(p) [p]\nm(cat(1,2))\n",
        "#define hh # ## #\nhh\n",
    ] {
        let gcc = compiler_tokens("gcc", src);
        let clang = compiler_tokens("clang", src);
        assert_eq!(gcc, clang, "the independent compilers must agree first: {src}");
        assert_eq!(ours(src), gcc, "chiero diverges on: {src}");
    }
}
