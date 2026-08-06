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
use std::path::{Path, PathBuf};

/// **One directory per test.** These tests run in parallel threads of one process, so a
/// directory keyed only on the process id is shared — and the fixtures, which have the same
/// names in several tests, raced. The symptom was
/// `undefined reference to chiero_after_f`: one test reading another's half-written file.
/// Found by the whole workspace failing intermittently while the crate passed alone.
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-replay-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn write(tag: &str, name: &str, src: &str) -> PathBuf {
    let p = scratch(tag).join(name);
    std::fs::write(&p, src).expect("write");
    p
}

/// Emit, or fail the test with the refusal — most tests are about what a harness does once
/// emitted, and a refusal there is a different bug.
#[track_caller]
fn must_emit(before: &Path, after: &Path, entry: &str, w: &Witness) -> Replay {
    emit_equivalence(before, after, entry, w)
        .unwrap_or_else(|r| panic!("the emitter refused a plain scalar witness: {}", r.why))
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
fn abs_pair(tag: &str) -> (PathBuf, PathBuf) {
    let before = write(
        tag,
        "abs_before.c",
        "int f (int x) { return x < 0 ? -x : x; }\n",
    );
    let after = write(
        tag,
        "abs_after.c",
        "int f (int x)\n{\n  if (x < 0)\n    return x == (-2147483647 - 1) ? 2147483647 : -x;\n  return x;\n}\n",
    );
    (before, after)
}

/// **Contract 10: every `Differs` produces a harness, and it is a whole C program.**
#[test]
fn the_harness_is_a_self_contained_program_naming_both_versions() {
    const TAG: &str = "the_harness_is_a_self_contained_program_naming_both_versions";
    let (b, a) = abs_pair(TAG);
    let r: Replay = must_emit(&b, &a, "f", &witness(&[(32, i32::MIN as u32 as u128)]));

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
    const TAG: &str = "a_real_divergence_is_demonstrated";
    let Some(cc) = chiero_replay::compiler() else {
        return; // no C compiler here; `run` says so rather than passing
    };
    let (b, a) = abs_pair(TAG);
    let r = must_emit(&b, &a, "f", &witness(&[(32, i32::MIN as u32 as u128)]));
    match run(&r, &cc, &scratch(TAG)) {
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
    const TAG: &str = "a_witness_that_does_not_distinguish_is_reported_as_such";
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let (b, a) = abs_pair(TAG);
    // 7 is not INT_MIN; both versions return 7.
    let r = must_emit(&b, &a, "f", &witness(&[(32, 7)]));
    match run(&r, &cc, &scratch(TAG)) {
        Outcome::NotDemonstrated { before, after } => assert_eq!(before, after),
        other => panic!("both versions return 7 here: {other:?}"),
    }
}

/// **A harness that will not build is its own outcome**, distinct from one that built and
/// disagreed. The two mean different things: a build failure is about the harness, a
/// disagreement is about chiero.
#[test]
fn a_harness_that_does_not_compile_is_not_a_disagreement() {
    const TAG: &str = "a_harness_that_does_not_compile_is_not_a_disagreement";
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let b = write(TAG, "bad_before.c", "int f (int x) { return x; }\n");
    let a = write(TAG, "bad_after.c", "this is not C\n");
    let r = must_emit(&b, &a, "f", &witness(&[(32, 1)]));
    match run(&r, &cc, &scratch(TAG)) {
        Outcome::DidNotBuild { .. } => {}
        other => panic!("the second file is not C: {other:?}"),
    }
}

/// **A `static` target is the common case** (040 §3.1), and the harness reaches it because it
/// `#include`s the source rather than declaring it `extern`.
#[test]
fn a_static_function_is_reachable() {
    const TAG: &str = "a_static_function_is_reachable";
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let b = write(
        TAG,
        "st_before.c",
        "static int f (int x) { return x + 1; }\n",
    );
    let a = write(
        TAG,
        "st_after.c",
        "static int f (int x) { return x + 2; }\n",
    );
    let r = must_emit(&b, &a, "f", &witness(&[(32, 1)]));
    match run(&r, &cc, &scratch(TAG)) {
        Outcome::Demonstrated { before, after } => {
            assert_eq!((before, after), (2, 3));
        }
        other => panic!("040 §3.1: static is the common case, not the exception: {other:?}"),
    }
}

/// **A TU with its own `main`** (040 §3.1's first hazard) must not collide with the harness's.
#[test]
fn a_translation_unit_with_its_own_main_still_builds() {
    const TAG: &str = "a_translation_unit_with_its_own_main_still_builds";
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let src = |k: i32| {
        format!("int f (int x) {{ return x + {k}; }}\nint main (void) {{ return f (0); }}\n")
    };
    let b = write(TAG, "main_before.c", &src(1));
    let a = write(TAG, "main_after.c", &src(2));
    let r = must_emit(&b, &a, "f", &witness(&[(32, 0)]));
    match run(&r, &cc, &scratch(TAG)) {
        Outcome::Demonstrated { before, after } => assert_eq!((before, after), (1, 2)),
        other => panic!("the TU's own main must be renamed out of the way: {other:?}"),
    }
}

/// **The harness is built somewhere else, so its includes must be absolute.**
///
/// It is compiled in a scratch directory (050 contract 12 keeps it out of the analysed tree),
/// and `#include "before.c"` resolves relative to the *harness*. A caller who ran
/// `chiero prove-equivalent before.c after.c` from the directory holding them got
/// `fatal error: before.c: No such file or directory` — a `did_not_build` that says nothing
/// about the code and everything about the emitter.
///
/// **Asserted as the rule, not the consequence.** The first version of this test changed the
/// process's working directory to exercise a relative path, which is racy the moment the
/// suite runs in parallel — and it did, intermittently failing the whole workspace. The
/// property is "the include is absolute"; building elsewhere is what that buys.
#[test]
fn includes_do_not_depend_on_where_the_harness_is_built() {
    const TAG: &str = "includes_do_not_depend_on_where_the_harness_is_built";
    let (b, a) = abs_pair(TAG);
    let r = must_emit(&b, &a, "f", &witness(&[(32, 1)]));
    for line in r.source.lines().filter(|l| l.starts_with("#include \"")) {
        let path = line.trim_start_matches("#include \"").trim_end_matches('"');
        assert!(
            PathBuf::from(path).is_absolute(),
            "an include resolved against the harness's directory, not the caller's: {line}"
        );
    }

    // And the consequence: built in a directory that holds neither source.
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let elsewhere = scratch(TAG).join("build");
    match run(&r, &cc, &elsewhere) {
        Outcome::Demonstrated { .. } | Outcome::NotDemonstrated { .. } => {}
        other => panic!("the harness must build away from its sources: {other:?}"),
    }
}
