//! `#pragma push_macro` / `pop_macro` — a stack of *name bindings*, not of definitions.
//!
//! Both gcc and clang implement these (originally an MSVC extension), and they agree on every
//! row of simplecpp's `pragma-pushpop-macro.c`. chiero recorded the pragma and did nothing, so a
//! `pop_macro` left the redefinition in place.
//!
//! **A `MacroId` identifies one definition and is never reused** (012 §1), so push/pop must move
//! the *binding* — `by_name` — and leave `macros` alone. That falls out of the existing model
//! rather than needing a new one, which is why this is a small change.

use chiero_pp::{Config, preprocess_str};
use std::process::{Command, Stdio};

fn ours(src: &str) -> Vec<String> {
    preprocess_str("p.c", src, Config::default())
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
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut map = chiero_span::SourceMap::new();
    let file = map.add_file("g.c", text);
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

/// The basic contract: push, redefine, pop, and the old definition is back.
#[test]
fn pop_macro_restores_the_pushed_definition() {
    matches_gcc(
        "#define X 1\n#pragma push_macro(\"X\")\n#define X 2\nX\n#pragma pop_macro(\"X\")\nX\n",
    );
}

/// A **stack**, not a single slot — two pushes need two pops.
#[test]
fn the_saved_definitions_form_a_stack() {
    matches_gcc(
        "#define X 1\n#pragma push_macro(\"X\")\n#define X 2\n#pragma push_macro(\"X\")\n\
         #define X 3\nX\n#pragma pop_macro(\"X\")\nX\n#pragma pop_macro(\"X\")\nX\n",
    );
}

/// Pushing an **undefined** name and popping it must leave it undefined — the saved binding is
/// "no macro", which is a different thing from an empty stack.
#[test]
fn pushing_an_undefined_name_saves_its_absence() {
    matches_gcc("#pragma push_macro(\"U\")\n#define U 9\nU\n#pragma pop_macro(\"U\")\nU\n");
}

/// **An imbalanced pop is not a crash**, which is what the corpus file's trailing stray push
/// exists to check.
#[test]
fn an_imbalanced_pop_is_survivable() {
    matches_gcc("#define X 1\n#pragma pop_macro(\"X\")\nX\n");
    matches_gcc("#define X 1\n#pragma push_macro(\"X\")\nX\n");
}

/// The corpus file itself, which is the reason this exists.
#[test]
fn the_corpus_fixture_agrees_with_gcc() {
    let path = "/home/ubuntu/simplecpp/testsuite/clang-preprocessor-tests/pragma-pushpop-macro.c";
    let Ok(src) = std::fs::read_to_string(path) else {
        eprintln!("simplecpp checkout unavailable; corpus row skipped");
        return;
    };
    matches_gcc(&src);
}
