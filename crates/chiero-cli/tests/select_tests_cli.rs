//! **`select-tests` from the command line, with coverage attributed per test.**
//!
//! The first end-to-end user's biggest finding, 2026-08-10: the CLI's `select-tests` could
//! never select anything. `--coverage <dir> --stem <name>` reads *one object with no test
//! name*, so `index.tests()` was empty and every invocation answered `0 selected` whatever the
//! diff said. The command now refuses rather than answering — but a refusal is not a product,
//! and the thesis it refuses to demonstrate is the one that held on real VPP: selection ranked
//! `bfd` first, 4582 covering relations against 343, and that one test caught the planted bug.
//!
//! It held **through a 145-line Rust driver**, because `ingest_native_as` — which takes the
//! `TestId` and has existed the whole time — was reachable only from Rust. This is D1 of §9's
//! done-enough-to-use bar: every operation runs from the command line on a real project with no
//! hand-written driver, and `select-tests` was the only one that failed.
//!
//! **The fixture is two programs and one changed file.** `other.c` and `t.c` both define
//! `main`, and each has its own gcov object in `tests/corpus/coverage/`. A change to `other.c`
//! must select the test that ran `other` and leave the one that ran `t` alone — which is the
//! whole claim, small enough to check exactly.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-select-cli-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run(args: &[String]) -> Run {
    let o = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    Run {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// `other.c` before and after a change to the one function it defines.
fn before_and_after() -> (PathBuf, PathBuf) {
    let d = scratch();
    let before = d.join("before.c");
    let after = d.join("after.c");
    std::fs::write(
        &before,
        "int main(void){ int v=2; v=v*3; return v>0?0:1; }\n",
    )
    .expect("write");
    std::fs::write(
        &after,
        "int main(void){ int v=2; v=v*4; return v>0?0:1; }\n",
    )
    .expect("write");
    (before, after)
}

fn object(stem: &str) -> String {
    corpus().join(stem).display().to_string()
}

/// The repeatable spelling: a handful of tests typed at a prompt.
#[test]
fn a_repeated_test_flag_attributes_coverage_and_selects() {
    let (before, after) = before_and_after();
    let args: Vec<String> = vec![
        "select-tests".into(),
        before.display().to_string(),
        after.display().to_string(),
        "--test".into(),
        format!("other={}", object("other")),
        "--test".into(),
        format!("t={}", object("t")),
        "--json".into(),
    ];
    let r = run(&args);
    assert_eq!(
        r.code, 0,
        "the command that demonstrates the product must succeed\nstdout: {}\nstderr: {}",
        r.out, r.err
    );
    let v: serde_json::Value = serde_json::from_str(&r.out)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}\n---\n{}", r.out, r.err));
    let text = r.out.clone();
    assert!(
        text.contains("other"),
        "the test that ran `other.c` is the one the change reaches, and it is not in the \
         answer:\n{v:#}"
    );
}

/// The manifest spelling: what a `make test-cov TEST=<name>` loop writes.
#[test]
fn a_coverage_manifest_attributes_coverage_and_selects() {
    let (before, after) = before_and_after();
    let manifest = scratch().join("tests.tsv");
    std::fs::write(
        &manifest,
        format!("other\t{}\nt\t{}\n", object("other"), object("t")),
    )
    .expect("write");
    let args: Vec<String> = vec![
        "select-tests".into(),
        before.display().to_string(),
        after.display().to_string(),
        "--coverage-manifest".into(),
        manifest.display().to_string(),
    ];
    let r = run(&args);
    assert_eq!(
        r.code, 0,
        "the manifest is what a real test loop produces\nstdout: {}\nstderr: {}",
        r.out, r.err
    );
    assert!(
        r.out.contains("other"),
        "the manifest selected nothing recognisable:\n{}",
        r.out
    );
}

/// **The refusal stays for the spelling that cannot work.** `--coverage`/`--stem` reads one
/// object with no test name; answering `0 selected` from an index that could never be non-empty
/// is the empty answer this project spent 2026-08-10 forbidding elsewhere.
#[test]
fn the_unattributed_spelling_still_refuses_and_now_names_the_alternative() {
    let (before, after) = before_and_after();
    let args: Vec<String> = vec![
        "select-tests".into(),
        before.display().to_string(),
        after.display().to_string(),
        "--coverage".into(),
        corpus().display().to_string(),
        "--stem".into(),
        "other".into(),
    ];
    let r = run(&args);
    assert_ne!(r.code, 0, "an index with no test attribution cannot select");
    assert!(
        r.err.contains("--test") || r.err.contains("--coverage-manifest"),
        "the refusal must name the flag that does work, or it is a dead end:\n{}",
        r.err
    );
}

/// A malformed `--test` says what it wanted, rather than selecting nothing.
#[test]
fn a_test_flag_without_a_name_is_a_usage_error_naming_the_shape() {
    let (before, after) = before_and_after();
    let args: Vec<String> = vec![
        "select-tests".into(),
        before.display().to_string(),
        after.display().to_string(),
        "--test".into(),
        object("other"),
    ];
    let r = run(&args);
    assert_eq!(r.code, 2, "a malformed argument is a usage error");
    assert!(
        r.err.contains("NAME=") || r.err.contains("name="),
        "the error does not say what the argument should look like:\n{}",
        r.err
    );
}
