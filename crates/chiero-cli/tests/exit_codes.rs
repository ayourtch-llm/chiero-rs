//! **050 contracts 19 and 20 — the exit status is part of the interface.**
//!
//! D2 of §9's done-enough-to-use bar. Agents are the consumer, so the machine surface *is* the
//! interface: an agent branches on the status before it reads a byte of output. `main.rs` has
//! distinguished `2` (usage) from `1` (failed) since it was written, `cli.rs` only ever checked
//! `0` against not-`0`, and 050 did not mention an exit code at all. A distinction that is real
//! in the code, asserted nowhere and documented nowhere is one a consumer is guessing at.
//!
//! | status | means |
//! |---|---|
//! | `0` | the operation ran and the envelope is the answer — **including** "nothing found" and "not proven" |
//! | `1` | the request was well formed and the operation could not complete |
//! | `2` | the request was malformed, so nothing was analysed |
//!
//! `0` versus `1` is the difference between *the tool answered* and *the tool did not run*, and
//! no envelope can express the second — there is no envelope in that case. Which is also why
//! contract 20 matters: stdout carries the envelope and nothing else, so `… --json | jq` cannot
//! be handed a diagnostic instead.

use std::path::PathBuf;
use std::process::Command;

const MAIN: &str = include_str!("../src/main.rs");

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-exit-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// A translation unit every operation can accept.
fn good_c() -> PathBuf {
    let p = scratch().join("ok.c");
    std::fs::write(
        &p,
        "#define M 1\nstruct s { int a; char b; };\nint f (int x) { return x + M; }\n",
    )
    .expect("write");
    p
}

struct Out {
    code: i32,
    out: String,
    err: String,
}

fn run(args: &[String]) -> Out {
    let o = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    Out {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// One operation, and the arguments after its input files.
struct Op {
    name: &'static str,
    files: usize,
    flags: &'static [&'static str],
    /// `Some(why)` when a successful run needs data this file does not have.
    no_success_case: Option<&'static str>,
}

const OPS: &[Op] = &[
    Op {
        name: "prove-equivalent",
        files: 2,
        flags: &["--entry", "f"],
        no_success_case: None,
    },
    Op {
        name: "find-bugs",
        files: 1,
        flags: &["--entry", "f"],
        no_success_case: None,
    },
    Op {
        name: "check-reachable",
        files: 1,
        flags: &["--entry", "f", "--line", "3"],
        no_success_case: None,
    },
    Op {
        name: "layout",
        files: 1,
        flags: &[],
        no_success_case: None,
    },
    Op {
        name: "find-optimizations",
        files: 1,
        flags: &["--entry", "f"],
        no_success_case: None,
    },
    Op {
        name: "impact",
        files: 2,
        flags: &[],
        no_success_case: None,
    },
    Op {
        name: "select-tests",
        files: 2,
        flags: &["--test", "a=/nonexistent/object"],
        // A successful selection needs real gcov objects, which `select_tests_cli.rs` supplies
        // and asserts an exit status on. Duplicating the fixture here would be a second copy of
        // it, not a second check.
        no_success_case: Some("needs gcov objects — covered by select_tests_cli.rs"),
    },
    Op {
        name: "expansion-sites",
        files: 1,
        flags: &["--macro", "M"],
        no_success_case: None,
    },
    Op {
        name: "cir",
        files: 1,
        flags: &[],
        no_success_case: None,
    },
    Op {
        name: "explain-macro",
        files: 1,
        flags: &["--line", "3"],
        no_success_case: None,
    },
];

/// The dispatch `match` in `run`, plus `cir` — so an operation added without a row here is a
/// failure rather than a silent gap. The same registry discipline as `operations.rs`.
#[test]
fn every_operation_has_a_row() {
    let start = MAIN
        .find("let env = match args[0].as_str() {")
        .expect("the dispatch match moved");
    let body = &MAIN[start..];
    let end = body
        .find("other =>")
        .expect("the dispatch match has no fallback");
    let mut names = vec!["cir".to_string()];
    for line in body[..end].lines() {
        if let Some(rest) = line.trim().strip_prefix('"')
            && let Some((name, tail)) = rest.split_once('"')
            && tail.contains("=>")
        {
            names.push(name.to_string());
        }
    }
    for n in &names {
        assert!(
            OPS.iter().any(|o| o.name == n),
            "`{n}` is dispatched and has no exit-status row"
        );
    }
    assert_eq!(names.len(), OPS.len(), "rows: {names:?}");
}

fn args_for(op: &Op, file: &str) -> Vec<String> {
    let mut a = vec![op.name.to_string()];
    for _ in 0..op.files {
        a.push(file.to_string());
    }
    a.extend(op.flags.iter().map(|s| (*s).to_string()));
    a
}

/// Contract 19, `2`: a malformed request, so nothing was analysed.
#[test]
fn a_malformed_request_exits_2() {
    let good = good_c().display().to_string();
    for op in OPS {
        let mut a = args_for(op, &good);
        a.push("--no-such-option".into());
        let r = run(&a);
        assert_eq!(
            r.code, 2,
            "`chiero {}` with an unknown option exited {} — usage is 2\n{}",
            op.name, r.code, r.err
        );
        assert!(
            r.out.is_empty(),
            "contract 20: `{}` printed to stdout while failing:\n{}",
            op.name,
            r.out
        );
    }
    // And the missing required argument, which is the shape a reader actually hits.
    for op in OPS.iter().filter(|o| !o.flags.is_empty()) {
        let mut a = vec![op.name.to_string()];
        for _ in 0..op.files {
            a.push(good.clone());
        }
        let r = run(&a);
        assert_eq!(
            r.code, 2,
            "`chiero {}` with no {} exited {}\n{}",
            op.name, op.flags[0], r.code, r.err
        );
    }
}

/// Contract 19, `1`: the request was well formed and the operation could not complete.
#[test]
fn a_request_that_cannot_be_carried_out_exits_1() {
    for op in OPS {
        let missing = scratch().join("does-not-exist.c").display().to_string();
        let r = run(&args_for(op, &missing));
        assert_eq!(
            r.code, 1,
            "`chiero {}` on a file that does not exist exited {} — a well-formed request that \
             could not be carried out is 1, and 2 would tell an agent to fix its own \
             arguments\nstderr: {}",
            op.name, r.code, r.err
        );
        assert!(
            r.out.is_empty(),
            "contract 20: `{}` printed to stdout while failing:\n{}",
            op.name,
            r.out
        );
        // **Some path from the request, not a fixed one.** `select-tests` reads its coverage
        // objects before the sources, so the first thing it cannot open is the object — which
        // is the file it could not read, and naming it is the house rule being checked here.
        let named = args_for(op, &missing)
            .iter()
            .any(|a| a.contains('/') && r.err.contains(a.rsplit('=').next().unwrap_or(a)));
        assert!(
            named,
            "the error names no path from the request, so a reader cannot tell what was \
             missing:\n{}",
            r.err
        );
    }
}

/// Contract 19, `0`: the operation ran, whatever the answer was.
#[test]
fn an_operation_that_ran_exits_0_even_when_it_found_nothing() {
    let good = good_c().display().to_string();
    for op in OPS.iter().filter(|o| o.no_success_case.is_none()) {
        let mut a = args_for(op, &good);
        a.push("--json".into());
        let r = run(&a);
        assert_eq!(
            r.code, 0,
            "`chiero {}` on a file it can analyse exited {}\nstderr: {}",
            op.name, r.code, r.err
        );
        assert!(
            !r.out.is_empty(),
            "`{}` exited 0 and printed nothing",
            op.name
        );
        // `cir` prints 020's normative text rather than an envelope, on purpose: the answer is
        // about chiero rather than about the program, so there is no fidelity to attach.
        if op.name != "cir" {
            serde_json::from_str::<serde_json::Value>(&r.out).unwrap_or_else(|e| {
                panic!("`{}` --json did not print JSON ({e}):\n{}", op.name, r.out)
            });
        }
    }
}

/// **The distinction is only worth anything if both sides occur**, and a suite that only ever
/// saw one would pass while the codes were identical.
#[test]
fn the_three_statuses_are_actually_distinct() {
    let good = good_c().display().to_string();
    let op = &OPS[1]; // find-bugs
    let ran = run(&{
        let mut a = args_for(op, &good);
        a.push("--json".into());
        a
    });
    let cannot = run(&args_for(
        op,
        &scratch().join("nope.c").display().to_string(),
    ));
    let malformed = run(&{
        let mut a = args_for(op, &good);
        a.push("--no-such-option".into());
        a
    });
    let codes = [ran.code, cannot.code, malformed.code];
    assert_eq!(codes, [0, 1, 2], "the three cases must not share a status");
}
