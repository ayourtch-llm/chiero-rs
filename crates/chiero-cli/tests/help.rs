//! **`chiero <operation> --help` — and the gate that keeps it from drifting off the code.**
//!
//! Reported 2026-08-10 by the first end-to-end user: *"No per-operation `--help`
//! (`select-tests --help` prints the global page); the 030 path/stem semantics cost me 3 toy
//! attempts."* The global page is one screen listing ten operations and eighteen options, and
//! a reader who has already chosen an operation has to work out which of the eighteen apply to
//! it. Nothing in the page says.
//!
//! **So the interesting half is not that the page exists — it is that it cannot lie.** A
//! hand-written per-operation page is a second copy of the argument parser, and this project
//! has watched every hand-kept list drift (§8.3). Three sources of truth are already in the
//! source and are read here rather than restated:
//!
//! 1. the dispatch `match` in `run` — *which operations exist*;
//! 2. the `match` in `Options::parse` — *which flags are accepted at all*;
//! 3. the `o.<field>` uses inside each operation's own function — *which flags that operation
//!    actually reads*.
//!
//! (3) is the one that makes this more than a formatting test: an operation's page must name
//! every option its implementation consults, so a flag wired into `find-bugs` and left out of
//! its help is red, and so is a page advertising a flag the operation ignores.

use std::collections::BTreeSet;
use std::process::Command;

const MAIN: &str = include_str!("../src/main.rs");

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// Every operation the binary dispatches, as `(command name, implementing fn)`.
///
/// Read from `run`'s `match args[0].as_str()`, plus `cir`, which is dispatched by an `if` above
/// it because it is the one operation that prints no envelope.
fn operations() -> Vec<(String, String)> {
    let start = MAIN
        .find("let env = match args[0].as_str() {")
        .expect("the dispatch match moved");
    let body = &MAIN[start..];
    let end = body
        .find("other =>")
        .expect("the dispatch match has no fallback");
    let mut ops = vec![("cir".to_string(), "cir".to_string())];
    for line in body[..end].lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix('"') else {
            continue;
        };
        let Some((name, tail)) = rest.split_once('"') else {
            continue;
        };
        let Some(call) = tail.split_once("=> ").map(|(_, c)| c) else {
            continue;
        };
        let f = call.split('(').next().unwrap_or_default().trim();
        ops.push((name.to_string(), f.to_string()));
    }
    assert!(
        ops.len() >= 10,
        "read {} operations out of the dispatch match, which cannot be right",
        ops.len()
    );
    ops
}

/// Every flag `Options::parse` accepts, read from its own `match`.
fn accepted_flags() -> BTreeSet<String> {
    let start = MAIN
        .find("fn parse(args: &[String])")
        .expect("Options::parse moved");
    let body = &MAIN[start..];
    let end = body
        .find("\n    fn files(")
        .expect("Options::parse has no end");
    let mut flags = BTreeSet::new();
    for line in body[..end].lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix('"') else {
            continue;
        };
        let Some((name, tail)) = rest.split_once('"') else {
            continue;
        };
        if tail.trim_start().starts_with("=>") && name.starts_with('-') {
            flags.insert(name.to_string());
        }
    }
    assert!(flags.len() >= 15, "read only {} flags", flags.len());
    flags
}

/// The option every operation may take, so no operation has to name them.
const GLOBAL: &[&str] = &[
    "--json",
    "-I",
    "-D",
    "--no-system-headers",
    "--march",
    "-h",
    "--help",
];

/// `Options` field → the flag that sets it. `wall_clock` is the accessor, not a field, and is
/// listed because that is how the operations reach `--time-budget`.
const FIELD_FLAG: &[(&str, &str)] = &[
    ("entry", "--entry"),
    ("macro_name", "--macro"),
    ("line", "--line"),
    ("col", "--col"),
    ("cursor", "--cursor"),
    ("limit", "--limit"),
    ("coverage", "--coverage"),
    ("stem", "--stem"),
    ("cache_line", "--cache-line"),
    ("replay", "--replay"),
    ("allow_replay_exec", "--allow-replay-exec"),
    ("entry_ptr_nonnull", "--entry-ptr-nonnull"),
    ("report_invented_bounds", "--report-invented-bounds"),
    ("solver_rlimit", "--solver-rlimit"),
    ("wall_clock", "--time-budget"),
];

/// The flags one operation's implementation consults, by reading `o.<field>` in its body.
fn flags_read_by(func: &str) -> BTreeSet<String> {
    let head = format!("\nfn {func}(o: &Options)");
    let start = MAIN
        .find(&head)
        .unwrap_or_else(|| panic!("no `fn {func}(o: &Options)` in main.rs"));
    let body = &MAIN[start + 1..];
    let end = body[1..].find("\nfn ").map_or(body.len(), |i| i + 1);
    let body = &body[..end];
    let mut flags = BTreeSet::new();
    for (field, flag) in FIELD_FLAG {
        if body.contains(&format!("o.{field}")) {
            flags.insert((*flag).to_string());
        }
    }
    flags
}

fn help_for(op: &str) -> String {
    let (code, out, err) = run(&[op, "--help"]);
    assert_eq!(code, 0, "`chiero {op} --help` exited {code}\n{err}");
    out
}

#[test]
fn every_operation_has_a_page_of_its_own() {
    let global = run(&["--help"]).1;
    for (name, _) in operations() {
        let page = help_for(&name);
        assert!(
            page.contains(&format!("chiero {name}")),
            "`chiero {name} --help` never names the operation:\n{page}"
        );
        assert_ne!(
            page.trim(),
            global.trim(),
            "`chiero {name} --help` printed the global page — this is user-test finding 4"
        );
        assert!(
            !page.contains("OPERATIONS:"),
            "`chiero {name} --help` lists every operation; the reader has already chosen one"
        );
        // `-h` is the same request spelled shorter, and a page reachable by only one of the two
        // spellings is the kind of half-wiring this file exists to catch.
        assert_eq!(page, help_for_short(&name), "`chiero {name} -h` differs");
    }
}

fn help_for_short(op: &str) -> String {
    let (code, out, err) = run(&[op, "-h"]);
    assert_eq!(code, 0, "`chiero {op} -h` exited {code}\n{err}");
    out
}

#[test]
fn a_page_names_every_option_its_operation_reads() {
    for (name, func) in operations() {
        let page = help_for(&name);
        for flag in flags_read_by(&func) {
            assert!(
                page.contains(&flag),
                "`chiero {name} --help` never mentions `{flag}`, which `{func}` reads"
            );
        }
    }
}

#[test]
fn a_page_advertises_no_option_its_operation_ignores() {
    let accepted = accepted_flags();
    for (name, func) in operations() {
        let page = help_for(&name);
        let reads = flags_read_by(&func);
        for flag in &accepted {
            if GLOBAL.contains(&flag.as_str()) || reads.contains(flag) {
                continue;
            }
            // Substring is enough because every flag name here is a whole word ending at a
            // space or a `<`, and no flag is a prefix of another except `--entry`, which is
            // checked by its own trailing space.
            let needle = format!("{flag} ");
            assert!(
                !page.contains(&needle) && !page.contains(&format!("{flag}\n")),
                "`chiero {name} --help` advertises `{flag}`, which `{func}` never reads"
            );
        }
    }
}

#[test]
fn every_accepted_flag_is_documented_on_the_global_page() {
    let global = run(&["--help"]).1;
    for flag in accepted_flags() {
        assert!(
            global.contains(&flag),
            "`{flag}` is accepted by Options::parse and appears nowhere in `chiero --help`"
        );
    }
}

/// The finding's own example: the two flags that cost the first user three attempts.
#[test]
fn select_tests_explains_coverage_and_stem() {
    let page = help_for("select-tests");
    for needle in ["--coverage", "--stem", "before", "after"] {
        assert!(
            page.contains(needle),
            "`select-tests --help` never mentions `{needle}`:\n{page}"
        );
    }
    // The command currently refuses when the index has no test attribution (finding 1). A
    // reader meeting that refusal should have been warned by the page they read first.
    assert!(
        page.contains("per-test") || page.contains("per test"),
        "`select-tests --help` does not say the coverage has to be attributed per test, which \
         is the one way this command can refuse:\n{page}"
    );
}

/// The global page must keep working, and keep pointing at the new one.
#[test]
fn the_global_page_still_lists_the_operations_and_names_the_new_pages() {
    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("OPERATIONS:"));
    for (name, _) in operations() {
        assert!(
            out.contains(&name),
            "`{name}` is missing from `chiero --help`"
        );
    }
    assert!(
        out.contains("<operation> --help") || out.contains("<operation> -h"),
        "the global page does not tell a reader that per-operation pages exist"
    );
}
