//! **Every operation, over every file in the analysis corpus, and none of them panics.**
//!
//! D3 of §9's done-enough-to-use bar, and §8.3's generalisation applied to this project's own
//! sweeps: *take a rule written for one component and ask whether its neighbours obey it.* The
//! VPP harness in `tests/corpus/vpp-findings/` has been pointed at hundreds of entry points and
//! found two engine panics that way — but it runs **`find-bugs` and nothing else**. The other
//! nine operations have never been swept at all, so "chiero does not crash" was a claim about
//! one of them.
//!
//! A panic is the failure that ends trust: an agent that gets a stack trace instead of an
//! envelope cannot tell a limitation from a defect, and everything the envelope discipline buys
//! is spent. So the assertion is deliberately weak and total — **every operation, every file, an
//! exit status that is one of the three the interface defines** (050 contract 19), and no
//! `panicked` on stderr. It says nothing about whether the answers are good; `injected_defects.rs`
//! is where that is asked.
//!
//! ⚠️ **This corpus is small and local on purpose.** It runs in CI, on every commit, with no VPP
//! checkout and no gcov build — which is what makes it a gate rather than an expedition. The
//! wide sweep stays where it is, and the two are not substitutes: this one cannot find what only
//! 1.5M lines of real C contains, and that one cannot run on a laptop.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn corpus() -> PathBuf {
    root().join("tests/corpus/c")
}

/// The corpus files `#include "chiero.h"` for `chiero_assume` and friends, so every run needs
/// the include root. **Leaving it out is how this file first swept nothing**: `cir` failed on
/// every input, `entries()` came back empty, and the four operations that need `--entry` were
/// silently skipped — 70 runs that all passed and proved nothing about seven of the nine
/// operations. The counter at the end of the sweep is what caught it.
fn include_args() -> Vec<String> {
    vec![
        "-I".to_string(),
        root().join("include").display().to_string(),
    ]
}

fn inputs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("tests/corpus/c")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "c"))
        .collect();
    v.sort();
    assert!(v.len() >= 10, "only {} corpus files", v.len());
    v
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

/// **The entry point, asked of chiero rather than guessed.** `cir` prints 020's normative text,
/// whose function headers are `func @name(` — so the tool names its own entries, and a corpus
/// file added tomorrow needs no table here.
fn entries(file: &Path) -> Vec<String> {
    let mut a = vec!["cir".to_string(), file.display().to_string()];
    a.extend(include_args());
    let r = run(&a);
    if r.code != 0 {
        return Vec::new();
    }
    let mut v: Vec<String> = r
        .out
        .lines()
        .filter_map(|l| l.strip_prefix("func @"))
        .filter_map(|l| l.split('(').next())
        .map(str::to_owned)
        .collect();
    v.dedup();
    v.truncate(3);
    v
}

/// `(operation, extra arguments)`; `{entry}` is substituted per function.
const OPS: &[(&str, &[&str])] = &[
    ("cir", &[]),
    ("layout", &[]),
    ("expansion-sites", &["--macro", "M"]),
    ("explain-macro", &["--line", "1"]),
    ("find-bugs", &["--entry", "{entry}", "--time-budget", "3"]),
    (
        "check-reachable",
        &["--entry", "{entry}", "--line", "1", "--time-budget", "3"],
    ),
    ("find-optimizations", &["--entry", "{entry}"]),
];

/// Two-file operations, run against the same file twice — the "nothing changed" input, which is
/// the degenerate one and therefore the one worth sweeping.
const PAIR_OPS: &[(&str, &[&str])] = &[
    ("impact", &[]),
    ("prove-equivalent", &["--entry", "{entry}"]),
];

fn check(op: &str, args: &[String]) {
    let r = run(args);
    assert!(
        !r.err.contains("panicked") && r.code != 101,
        "`chiero {}` panicked — an agent that gets a stack trace instead of an envelope cannot \
         tell a limitation from a defect:\n{}",
        args.join(" "),
        r.err
    );
    assert!(
        (0..=2).contains(&r.code),
        "`chiero {}` exited {}, which is not one of the three statuses 050 contract 19 \
         defines\nstderr: {}",
        args.join(" "),
        r.code,
        r.err
    );
    if r.code != 0 {
        assert!(
            r.out.is_empty(),
            "contract 20: `{op}` printed to stdout while failing:\n{}",
            r.out
        );
    }
}

#[test]
fn no_operation_panics_on_the_analysis_corpus() {
    let files = inputs();
    let (mut ran, mut entry_runs) = (0, 0);
    for f in &files {
        let path = f.display().to_string();
        let entries = entries(f);
        // **Per file, not in total.** One file with fifty functions would hide thirteen with
        // none behind a healthy-looking sum, which is the shape of every false zero in §9.2.
        assert!(
            !entries.is_empty(),
            "`chiero cir {path}` named no function, so every operation needing `--entry` was \
             skipped for this file"
        );
        for (op, extra) in OPS {
            let needs_entry = extra.contains(&"{entry}");
            let each: Vec<String> = if needs_entry {
                entries.clone()
            } else {
                vec![String::new()]
            };
            for e in each {
                let mut args = vec![(*op).to_string(), path.clone()];
                args.extend(include_args());
                args.extend(
                    extra
                        .iter()
                        .map(|a| if *a == "{entry}" { &e } else { *a }.to_string()),
                );
                check(op, &args);
                ran += 1;
                entry_runs += usize::from(needs_entry);
            }
        }
        for (op, extra) in PAIR_OPS {
            let needs_entry = extra.contains(&"{entry}");
            let each: Vec<String> = if needs_entry {
                entries.clone()
            } else {
                vec![String::new()]
            };
            for e in each {
                let mut args = vec![(*op).to_string(), path.clone(), path.clone()];
                args.extend(include_args());
                args.extend(
                    extra
                        .iter()
                        .map(|a| if *a == "{entry}" { &e } else { *a }.to_string()),
                );
                check(op, &args);
                ran += 1;
                entry_runs += usize::from(needs_entry);
            }
        }
    }
    // **A sweep that swept nothing passes every assertion in it.** This is the counter that
    // tells the two apart, and it is the failure mode `check.sh` was rewritten for.
    assert!(
        entry_runs >= files.len() * 4,
        "only {entry_runs} runs of the four operations that take `--entry`, over {} files — the \
         sweep is not sweeping. That is what happened the first time this ran: `cir` could not \
         find `chiero.h`, `entries()` came back empty, and seven of the nine operations went \
         untested while every assertion in the file passed.",
        files.len()
    );

    eprintln!(
        "{ran} operation runs ({entry_runs} from named entries) over {} corpus files",
        files.len()
    );
}
