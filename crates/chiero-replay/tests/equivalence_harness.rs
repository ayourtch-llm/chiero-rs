//! **041 §1.3 — the replay harness for a `Differs`.**
//!
//! > "the output is a **distinguishing input plus a replay harness** that compiles both
//! > versions and demonstrates the divergence. 'Your rewrite is wrong' is an opinion; 'your
//! > rewrite returns 0 where the original returns -1 when `n == INT_MIN`, here is the program'
//! > ends the discussion."
//!
//! And the half that makes it worth having (contract 11):
//!
//! > "the harness is compiled and run, and a divergence the harness fails to demonstrate is
//! > downgraded and flagged, never silently trusted."
//!
//! **The harness is chiero checking itself against a compiler.** Every `Differs` up to now has
//! rested on chiero's C semantics being right; this is the only thing in the system that can
//! catch them being wrong, so a harness that quietly fails is worse than no harness at all.

use chiero_exec::{Binding, InputOrigin, Witness};
use chiero_replay::{Outcome, Replay, emit_equivalence, run};
use chiero_span::Span;
use std::path::PathBuf;

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-replay-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn write(name: &str, src: &str) -> PathBuf {
    let p = scratch().join(name);
    std::fs::write(&p, src).expect("write");
    p
}

fn witness(values: &[(u32, u128)]) -> Witness {
    Witness {
        bindings: values
            .iter()
            .enumerate()
            .map(|(index, (width, value))| Binding {
                origin: InputOrigin::Param {
                    index,
                    name: String::new(),
                    span: Span::DUMMY,
                },
                width: *width,
                value: *value,
                pinned: true,
            })
            .collect(),
    }
}

/// The `abs` rewrite from 041 §1.3's own example: identical everywhere but `INT_MIN`.
fn abs_pair() -> (PathBuf, PathBuf) {
    let before = write("abs_before.c", "int f (int x) { return x < 0 ? -x : x; }\n");
    let after = write(
        "abs_after.c",
        "int f (int x)\n{\n  if (x < 0)\n    return x == (-2147483647 - 1) ? 2147483647 : -x;\n  return x;\n}\n",
    );
    (before, after)
}

/// **Contract 10: every `Differs` produces a harness, and it is a whole C program.**
#[test]
fn the_harness_is_a_self_contained_program_naming_both_versions() {
    let (b, a) = abs_pair();
    let r: Replay = emit_equivalence(&b, &a, "f", &witness(&[(32, i32::MIN as u32 as u128)]));

    for want in ["#include", "int main", "abs_before.c", "abs_after.c"] {
        assert!(
            r.source.contains(want),
            "`{want}` missing from:\n{}",
            r.source
        );
    }
    // The witness is in the program, not described beside it — and spelled the way C can
    // actually express it. `-2147483648` is C's negation of a value that does not fit in an
    // `int`, which is why `<limits.h>` defines `INT_MIN` as `(-2147483647 - 1)`. A harness
    // writing the obvious thing would fail to reproduce exactly the divergence 041 §1.3 uses
    // as its worked example.
    assert!(
        r.source.contains("(-2147483647 - 1)"),
        "INT_MIN must be spelled the way C spells it:\n{}",
        r.source
    );
    // And the two versions must be callable side by side, which means renamed.
    assert!(
        r.source.matches("chiero_before").count() >= 2
            && r.source.matches("chiero_after").count() >= 2,
        "both versions must be reachable under distinct names:\n{}",
        r.source
    );
}

/// **The harness compiles and demonstrates the divergence.**
///
/// The whole point: it exits non-zero when the two versions *agree*, so "it ran and said
/// nothing" cannot be mistaken for success.
#[test]
fn a_real_divergence_is_demonstrated() {
    let Some(cc) = chiero_replay::compiler() else {
        return; // no C compiler here; `run` says so rather than passing
    };
    let (b, a) = abs_pair();
    let r = emit_equivalence(&b, &a, "f", &witness(&[(32, i32::MIN as u32 as u128)]));
    match run(&r, &cc, &scratch()) {
        Outcome::Demonstrated { before, after } => {
            assert_eq!(before, i64::from(i32::MIN));
            assert_eq!(after, i64::from(i32::MAX));
        }
        other => panic!("the divergence is real and the harness must show it: {other:?}"),
    }
}

/// **Contract 11: a harness that fails to demonstrate is a downgrade, not a pass.**
///
/// Given a witness at which the two versions agree, the harness must come back
/// `NotDemonstrated` — the case that says chiero's semantics and the compiler's disagree, and
/// the one an implementation is most tempted to treat as "well, it compiled".
#[test]
fn a_witness_that_does_not_distinguish_is_reported_as_such() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let (b, a) = abs_pair();
    // 7 is not INT_MIN; both versions return 7.
    let r = emit_equivalence(&b, &a, "f", &witness(&[(32, 7)]));
    match run(&r, &cc, &scratch()) {
        Outcome::NotDemonstrated { before, after } => assert_eq!(before, after),
        other => panic!("both versions return 7 here: {other:?}"),
    }
}

/// **A harness that will not build is its own outcome**, distinct from one that built and
/// disagreed. The two mean different things: a build failure is about the harness, a
/// disagreement is about chiero.
#[test]
fn a_harness_that_does_not_compile_is_not_a_disagreement() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let b = write("bad_before.c", "int f (int x) { return x; }\n");
    let a = write("bad_after.c", "this is not C\n");
    let r = emit_equivalence(&b, &a, "f", &witness(&[(32, 1)]));
    match run(&r, &cc, &scratch()) {
        Outcome::DidNotBuild { .. } => {}
        other => panic!("the second file is not C: {other:?}"),
    }
}

/// **A `static` target is the common case** (040 §3.1), and the harness reaches it because it
/// `#include`s the source rather than declaring it `extern`.
#[test]
fn a_static_function_is_reachable() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let b = write("st_before.c", "static int f (int x) { return x + 1; }\n");
    let a = write("st_after.c", "static int f (int x) { return x + 2; }\n");
    let r = emit_equivalence(&b, &a, "f", &witness(&[(32, 1)]));
    match run(&r, &cc, &scratch()) {
        Outcome::Demonstrated { before, after } => {
            assert_eq!((before, after), (2, 3));
        }
        other => panic!("040 §3.1: static is the common case, not the exception: {other:?}"),
    }
}

/// **A TU with its own `main`** (040 §3.1's first hazard) must not collide with the harness's.
#[test]
fn a_translation_unit_with_its_own_main_still_builds() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let src = |k: i32| {
        format!("int f (int x) {{ return x + {k}; }}\nint main (void) {{ return f (0); }}\n")
    };
    let b = write("main_before.c", &src(1));
    let a = write("main_after.c", &src(2));
    let r = emit_equivalence(&b, &a, "f", &witness(&[(32, 0)]));
    match run(&r, &cc, &scratch()) {
        Outcome::Demonstrated { before, after } => assert_eq!((before, after), (1, 2)),
        other => panic!("the TU's own main must be renamed out of the way: {other:?}"),
    }
}
