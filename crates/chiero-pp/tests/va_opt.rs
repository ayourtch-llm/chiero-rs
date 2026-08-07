//! `__VA_OPT__` — C23 6.10.3.1, implemented at the owner's request.
//!
//! 012 §2.3 declared it out of v1 scope *by measurement* (VPP uses `__VA_ARGS__` 230 times and
//! `__VA_OPT__` zero) and **diagnosed** it rather than passing it through as four literal tokens.
//! That was the right default and it is now a scope change, not a defect fix.
//!
//! **The test that decides the semantics** is `P(1,)`: the variadic argument was *supplied* and
//! is *empty*, and `__VA_OPT__` yields **nothing**. So its condition is the argument's **tokens**
//! — the opposite of the GNU comma rule two functions away, which turns on whether an argument
//! was supplied at all. Two neighbouring rules with opposite tests is exactly the shape that
//! produces a wrong shared flag, so both are pinned here.

use chiero_pp::{Config, preprocess_str};
use std::process::{Command, Stdio};

fn ours(src: &str) -> Vec<String> {
    preprocess_str("v.c", src, Config::default())
        .token_texts()
        .map(str::to_owned)
        .collect()
}

fn gcc(src: &str) -> Vec<String> {
    let mut child = Command::new("gcc")
        .args(["-E", "-P", "-std=gnu11", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), src.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let mut map = chiero_span::SourceMap::new();
    let file = map.add_file("g.c", String::from_utf8_lossy(&out.stdout).into_owned());
    let lexed = chiero_lex::LexSession::new().lex(&map, file, chiero_lex::LexConfig::default());
    lexed
        .tokens()
        .iter()
        .filter(|t| !matches!(t.kind, chiero_lex::PpTokenKind::Eof))
        .map(|t| lexed.text(t).to_owned())
        .collect()
}

#[track_caller]
fn matches_gcc(src: &str) {
    assert_eq!(ours(src), gcc(src), "on: {src}");
}

/// Present-and-non-empty yields the contents; **supplied-but-empty yields nothing.**
#[test]
fn the_condition_is_the_arguments_tokens_not_its_presence() {
    let m = "#define P(x,...) f(x __VA_OPT__(,) __VA_ARGS__)\n";
    matches_gcc(&format!("{m}P(1,2)\n"));
    matches_gcc(&format!("{m}P(1)\n"));
    matches_gcc(&format!("{m}P(1,)\n")); // supplied, empty — nothing
}

/// Several tokens, and **parentheses inside the group** — the scan counts depth rather than
/// stopping at the first `)`.
#[test]
fn the_group_may_hold_several_tokens_and_inner_parentheses() {
    matches_gcc("#define M(...) [__VA_OPT__(a b c)]\nM()\nM(z)\n");
    matches_gcc("#define M(...) [__VA_OPT__(f(a))]\nM()\nM(z)\n");
}

/// It composes with `##` and with another macro that also uses it.
#[test]
fn it_composes_with_paste_and_with_nesting() {
    matches_gcc("#define M(x,...) x##__VA_OPT__(y)\nM(1)\nM(1,2)\n");
    matches_gcc(
        "#define P(x,...) f(x __VA_OPT__(,) __VA_ARGS__)\n\
         #define Q(x,...) P(x __VA_OPT__(,) __VA_ARGS__)\nQ(1,2)\nQ(1)\n",
    );
}

/// The corpus fixture, all five calls.
#[test]
fn the_corpus_fixture_agrees_with_gcc() {
    matches_gcc(
        "#define P( x, ...) printf( x __VA_OPT__(,) __VA_ARGS__ )\n\
         #define PF( x, ...) P( x __VA_OPT__(,) __VA_ARGS__ )\n\
         PF( \"%s\", \"Hello\" );\nPF( \"Hello\", );\nPF( \"Hello\" );\nPF( , );\nPF( );\n",
    );
}

/// **A malformed group is diagnosed, not swallowed** — 011 §4's rule for every other malformed
/// construct. Without this, an unterminated `__VA_OPT__(` could eat the rest of a macro body.
#[test]
fn a_malformed_group_is_reported_and_the_body_survives() {
    let tu = preprocess_str(
        "v.c",
        "#define M(...) [__VA_OPT__ a]\nM(z)\n",
        Config::default(),
    );
    assert!(!tu.diagnostics.is_empty(), "expected a diagnostic");
    assert!(
        tu.token_texts().any(|t| t == "a"),
        "the rest of the body must survive: {:?}",
        tu.token_texts().collect::<Vec<_>>()
    );
}
