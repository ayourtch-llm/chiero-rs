//! C99 6.10.3.4p2: **the hide set of an invocation intersects at the closing paren.**
//!
//! When a function-like macro's name comes out of an earlier expansion but its argument list
//! comes from the source that followed, the invocation is only partly "inside" that earlier
//! expansion. Prosser's algorithm — the standard formulation of the rescanning rule every C
//! preprocessor implements — states it as
//!
//! ```text
//! HS(invocation) = (HS(macro name) ∩ HS(closing paren)) ∪ { macro name }
//! ```
//!
//! The **intersection** is the whole point: a name painted blue by macro `M` stops being
//! painted by `M` once the invocation reaches outside `M`'s expansion for its arguments,
//! because the resulting tokens are no longer wholly `M`'s. chiero was taking the union — it
//! extended the call's hide set into every substituted token and never consulted the closing
//! paren at all — so it kept painting after the paint should have stopped, and stalled two
//! expansions early.
//!
//! Found by `cargo run -p xtask -- pp-gate` over simplecpp's testsuite: `macro_rescan2.c` and
//! `macro_disable.c`, both of which exist in clang's own suite to pin exactly this paragraph.
//!
//! # The risk this fix carries, and what pins it
//!
//! Loosening a hide set is how a preprocessor starts expanding a self-referential macro
//! forever. So the cases that must keep working are here in the same file and outnumber the two
//! that change — direct self-reference, mutual recursion through two names, the standard's own
//! `A B C` triangle, and 6.10.3.4p2's `f(f(z))`. §11.1: an assertion that something changed is
//! close to worthless without the companion assertion that everything else did not.

use chiero_pp::{Config, preprocess_str};
use std::process::{Command, Stdio};

fn ours(src: &str) -> Vec<String> {
    preprocess_str("rescan.c", src, Config::default())
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

/// Both compilers, then chiero. A hand-written expectation about rescanning is exactly the kind
/// that is confidently wrong, so nothing here is asserted from reasoning alone.
#[track_caller]
fn agrees_with_both(src: &str) {
    let gcc = compiler_tokens("gcc", src);
    let clang = compiler_tokens("clang", src);
    assert_eq!(
        gcc, clang,
        "the independent compilers must agree first: {src}"
    );
    assert_eq!(ours(src), gcc, "chiero diverges on: {src}");
}

/// `macro_rescan2.c`, the `b:` half. `f(2)` yields `2*g`, whose `g` is painted by `f`; but
/// `g(9)` takes its `)` from the source, so `f` drops out of the hide set and the `f(9)` that
/// `g` produces is expanded.
///
/// chiero stopped at `2*f(9)`, two expansions early.
#[test]
fn a_name_from_an_expansion_with_a_source_paren_loses_the_outer_paint() {
    agrees_with_both("#define f(a) a*g\n#define g(a) f(a)\nb: f(2)(9)\n");
}

/// `macro_disable.c`'s `M_0`/`M_1` ladder. `M_0(1)` pastes to `M_1`; `M_1(2)` is invoked with a
/// source-side `)`, so `M_0` leaves the hide set and the `M_0(0)` in `M_1`'s body expands —
/// consuming the `(0)` and painting the `M_0` that the paste then produces.
///
/// chiero left `M_0 ( 0 )` sitting in the output, which is a token stream no compiler produces.
#[test]
fn a_pasted_name_invoked_across_the_expansion_boundary_expands() {
    agrees_with_both(
        "#define M_0(x) M_ ## x\n\
         #define M_1(x) x + M_0(0)\n\
         #define M_2(x) x + M_1(1)\n\
         a: M_0(1)(2)(3);\n",
    );
}

/// The full ladder from the corpus file, five deep and invoked both ways round.
#[test]
fn the_whole_m_ladder_from_macro_disable_c() {
    agrees_with_both(
        "#define M_0(x) M_ ## x\n\
         #define M_1(x) x + M_0(0)\n\
         #define M_2(x) x + M_1(1)\n\
         #define M_3(x) x + M_2(2)\n\
         #define M_4(x) x + M_3(3)\n\
         #define M_5(x) x + M_4(4)\n\
         a: M_0(1)(2)(3)(4)(5);\n\
         b: M_0(5)(4)(3)(2)(1);\n",
    );
}

/// **The companion assertions: everything that must NOT change.**
///
/// Loosening a hide set is how a preprocessor starts expanding a self-referential macro
/// forever, and every one of these passed before the fix. If one of them hangs or grows, the
/// intersection was applied where the standard does not ask for it.
#[test]
fn self_reference_still_terminates() {
    for src in [
        // 6.10.3.4p2's own example.
        "#define f(a) f(x * (a))\n#define x 2\n#define z z[0]\nf(f(z));\n",
        // The standard's mutually-recursive triangle.
        "#define A A B C\n#define B B C A\n#define C C A B\nA\n",
        // Direct self-reference, function-like, argument list entirely from source.
        "#define f(x) f(x)\nf(1)\n",
        // Two object-like macros naming each other.
        "#define a b\n#define b a\na\n",
        // The `a:` half of macro_rescan2.c: `g` is object-like, so there is no paren to
        // intersect at and `f` stays painted. **This is the case the fix must leave alone.**
        "#define f(a) a*g\n#define g f\na: f(2)(9)\n",
        // An argument list split across the boundary the other way (`macro_disable.c`, PR1820).
        "#define i(x) h(x\n#define h(x) x(void)\nextern int i(i));\n",
        // A name that reaches its own invocation through an argument.
        "#define n(v) v\n#define l m\n#define m l a\nc: n(m) X\n",
    ] {
        agrees_with_both(src);
    }
}
