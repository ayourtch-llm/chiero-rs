//! GNU `, ## __VA_ARGS__` — **the extension is about the variadic parameter, not about commas.**
//!
//! 012 §2.3 says the comma is deleted only when the variadic argument is empty. What it does not
//! say, and what chiero did not implement, is that the rule applies **only when the right operand
//! of that `##` is the variadic parameter**. `, ## Y` for an ordinary parameter `Y` is not the
//! GNU extension at all; it is an ordinary paste against an empty argument, and the comma stays.
//!
//! Found in the pp-gate's `Todo` cluster (`macro_fn_comma_swallow.c`). ⚠️ **The gate reports one
//! divergence per file and it named the wrong row** — §11.2's "a dominant finding is a lid, not a
//! summary" applies to a first-divergence report too. Measuring all six rows moved the diagnosis
//! twice:
//!
//! | row | gcc and clang | chiero, before |
//! |---|---|---|
//! | `#define X2(Y) fo2{A,##Y}` / `X2()` | `fo2{A,}` | `fo2{A}` — **the comma is wrongly eaten** |
//! | `#define X5(x,...) x##,##__VA_ARGS__` / `X5(1)` | `1`, silently | `1`, **with a spurious diagnostic** |
//! | `X5(1,2)` | `1 , 2` **and gcc errors** | `1 , 2` and chiero diagnoses — already correct |
//!
//! The third row is why the first probe of this was wrong: it ran gcc with stderr suppressed, saw
//! clean output, and concluded chiero's diagnostic was spurious there too. It is not — gcc calls
//! it an error. **An oracle read through a closed channel answers a different question.**

use chiero_pp::{Config, preprocess_str};
use std::process::{Command, Stdio};

fn ours(src: &str) -> (Vec<String>, usize) {
    let tu = preprocess_str("cs.c", src, Config::default());
    (
        tu.token_texts().map(str::to_owned).collect(),
        tu.diagnostics.len(),
    )
}

/// gcc's tokens **and** whether gcc accepted the program — both halves, because this file turns
/// on a case where the tokens agree and only the diagnosis differs.
fn gcc(src: &str) -> (Vec<String>, bool) {
    let mut child = Command::new("gcc")
        .args(["-E", "-P", "-std=gnu11", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), src.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut map = chiero_span::SourceMap::new();
    let file = map.add_file("gcc.c", text);
    let lexed = chiero_lex::LexSession::new().lex(&map, file, chiero_lex::LexConfig::default());
    let tokens = lexed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.kind, chiero_lex::PpTokenKind::Eof))
        .map(|token| lexed.text(token).to_owned())
        .collect();
    (tokens, output.status.success())
}

/// Tokens must match gcc, and chiero must be silent exactly when gcc is.
#[track_caller]
fn matches_gcc(src: &str) {
    let (their_tokens, gcc_accepted) = gcc(src);
    let (our_tokens, our_diagnostics) = ours(src);
    assert_eq!(our_tokens, their_tokens, "tokens differ on: {src}");
    assert_eq!(
        our_diagnostics == 0,
        gcc_accepted,
        "chiero emitted {our_diagnostics} diagnostic(s) and gcc {} the program: {src}",
        if gcc_accepted { "accepted" } else { "rejected" }
    );
}

/// **`, ## Y` for an ordinary parameter is not the GNU extension.** With `Y` empty the comma
/// stays, because there is nothing variadic here to swallow it for.
#[test]
fn a_comma_before_a_non_variadic_empty_parameter_survives() {
    matches_gcc("#define X2(Y) fo2{A,##Y}\n2: X2()\n");
    matches_gcc("#define X2(Y) fo2{A,##Y}\n2: X2(z)\n");
    // Two ordinary parameters, so the empty one cannot be mistaken for a variadic tail.
    matches_gcc("#define P(a,b) [a,##b]\nP(1,)\n");
}

/// The extension itself, which must keep working — three shapes from the corpus file.
#[test]
fn the_gnu_extension_still_swallows_for_a_variadic_parameter() {
    matches_gcc("#define X3(b, ...) {b, ## __VA_ARGS__}\n3: X3(foo)\n");
    matches_gcc("#define X3(b, ...) {b, ## __VA_ARGS__}\n3: X3(foo, bar)\n");
    matches_gcc("#define X4(...)  AA , ## __VA_ARGS__ BB\n4: X4()\n");
    matches_gcc("#define X4(...)  AA , ## __VA_ARGS__ BB\n4: X4(z)\n");
    // The GNU *named* variadic spelling, which 012 §4 lists as required for VPP.
    matches_gcc("#define D(f, a...) g(f, ##a)\nD(\"x\")\nD(\"y\", 1)\n");
}

/// **A paste whose right operand is a comma the GNU group has claimed does not fire.**
///
/// `x##,##__VA_ARGS__` with an empty tail is `1` under both compilers, *silently*: the `, ## …`
/// group takes the comma before the earlier `##` can paste into it. chiero produced the right
/// token and complained anyway.
///
/// The companion row is the one that keeps this honest — with a **non-empty** tail the group does
/// not claim the comma, `1 ## ,` really is an invalid paste, and **gcc calls it an error**. A fix
/// that simply silenced the diagnostic would break this row.
#[test]
fn a_comma_claimed_by_the_gnu_group_is_not_pasted_into() {
    matches_gcc("#define X5(x,...) x##,##__VA_ARGS__\n5: X5(1)\n");
    matches_gcc("#define X5(x,...) x##,##__VA_ARGS__\n5: X5(1,2)\n");
}

/// The plain row, which never diverged and is here so a regression in the ordinary path is not
/// mistaken for one in the extension.
#[test]
fn an_ordinary_comma_is_untouched() {
    matches_gcc("#define X(Y) foo{A, Y}\n1: X()\n");
    matches_gcc("#define X(Y) foo{A, Y}\n1: X(z)\n");
}

/// **Absent is not the same as empty.** The comma is swallowed only when the variadic parameter
/// received *no argument at all* — not when it received one that happens to be empty.
///
/// `debug(V)` has no variadic argument and loses the comma; `debug(Y, )` and `debug(Z,)` supply
/// one, and it stays. gcc and clang agree on all four rows of `macro_paste_commaext.c`, and
/// chiero swallowed in every case — the emptiness of the *tokens* is not the question, the
/// presence of the *argument* is.
#[test]
fn the_comma_survives_a_supplied_but_empty_variadic_argument() {
    let m = "#define debug(format, ...) format, ## __VA_ARGS__)\n";
    matches_gcc(&format!("{m}debug(V);\n"));
    matches_gcc(&format!("{m}debug(W, 1, 2);\n"));
    matches_gcc(&format!("{m}debug(Y, );\n"));
    matches_gcc(&format!("{m}debug(Z,);\n"));
    // The GNU named-variadic spelling takes the same rule.
    let n = "#define d(f, a...) g(f, ##a)\n";
    matches_gcc(&format!("{n}d(V);\n"));
    matches_gcc(&format!("{n}d(Y, );\n"));
}
