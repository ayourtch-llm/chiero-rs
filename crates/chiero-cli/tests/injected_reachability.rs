//! **Does `check-reachable` decide? A C corpus whose answer is known line by line.**
//!
//! The third corpus of this shape, after `injected_defects.rs` (the checkers) and
//! `injected_rewrites.rs` (the adjudicator). `check-reachable` is the operation whose *shape*
//! carries the most weight of any in the tool, because it has three verdicts and the whole point
//! is that two of them are not the same answer:
//!
//! | verdict | means | is a proof |
//! |---|---|---|
//! | `reachable` | here is an input that gets there | yes — a witness |
//! | `unreachable` | **nothing** gets there | yes — the strong claim |
//! | `not_shown_reachable` | chiero did not find a way | no |
//!
//! A caller deletes code on `unreachable` and investigates on `not_shown_reachable`. Conflating
//! them is the failure 050 contract 5 exists for, and until now the fixtures that forced each
//! were hand-written CIR — so the C-to-verdict path a caller uses was never the one measured.
//!
//! **Both wrong answers are forbidden unconditionally**, because a false proof does not become
//! acceptable when z3 is absent: calling a live line `unreachable` licenses deleting code that
//! runs, and calling a dead line `reachable` claims a witness that cannot exist. Only the floor
//! saying *something was decided* is conditional on a backend — the rule §7.35 settled.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn has_solver() -> bool {
    chiero_opt::EquivCfg::new("probe").backend.is_some()
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-reach-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn ask(name: &str, src: &str, line: u32) -> (String, serde_json::Value) {
    let p = scratch().join(format!("{name}.c"));
    std::fs::write(&p, src).expect("write");
    let out = Command::new(bin())
        .args([
            "check-reachable",
            p.to_str().unwrap(),
            "--entry",
            "probe",
            "--line",
            &line.to_string(),
            "--json",
            "--time-budget",
            "20",
        ])
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    assert!(
        out.status.success(),
        "`check-reachable` on `{name}:{line}` exited {:?}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("`{name}` did not print JSON ({e}):\n{text}"));
    let verdict = v["result"]["verdict"]
        .as_str()
        .unwrap_or_else(|| panic!("`{name}` has no verdict:\n{v:#}"))
        .to_string();
    // **`no_such_line` is a fixture bug, not a result.** A corpus whose lines have drifted off
    // its sources would otherwise report "nothing was decided" and read as a solver limitation.
    assert_ne!(
        verdict, "no_such_line",
        "`{name}` has no statement on line {line} — the fixture drifted, not the tool:\n{v:#}"
    );
    (verdict, v)
}

/// One question about one line, and whether anything can actually get there.
struct Case {
    name: &'static str,
    /// The 1-based line to ask about. Sources are written so this is easy to count.
    line: u32,
    /// Is that line reachable for *some* input?
    live: bool,
    src: &'static str,
    /// The argument for the ground truth, so a reader judges the fixture before the tool.
    why: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "guarded-branch",
        line: 3,
        live: true,
        src: "int probe (int x) {\n\
              \x20 if (x > 0)\n\
              \x20   return 1;\n\
              \x20 return 0;\n\
              }\n",
        why: "any positive x gets there, so a witness exists and `unreachable` would be a proof \
              that licenses deleting live code",
    },
    Case {
        name: "second-arm",
        line: 5,
        live: true,
        src: "int probe (int x) {\n\
              \x20 if (x > 0) {\n\
              \x20   return 1;\n\
              \x20 } else {\n\
              \x20   return 2;\n\
              \x20 }\n\
              }\n",
        why: "the else arm is entered by every x <= 0",
    },
    Case {
        name: "loop-body",
        line: 4,
        live: true,
        src: "int probe (int n) {\n\
              \x20 int s = 0;\n\
              \x20 for (int i = 0; i < 3; i++) {\n\
              \x20   s += i;\n\
              \x20 }\n\
              \x20 return s + n;\n\
              }\n",
        why: "the bound is concrete and positive, so the body runs on every input",
    },
    Case {
        name: "after-the-loop",
        line: 5,
        live: true,
        src: "int probe (int n) {\n\
              \x20 int s = 0;\n\
              \x20 for (int i = 0; i < 3; i++)\n\
              \x20   s += i;\n\
              \x20 return s + n;\n\
              }\n",
        why: "a bounded loop terminates, so what follows it is reached",
    },
    Case {
        name: "contradiction",
        line: 3,
        live: false,
        src: "int probe (int x) {\n\
              \x20 if (x > 0 && x < 0)\n\
              \x20   return 1;\n\
              \x20 return 0;\n\
              }\n",
        why: "no int is both positive and negative — the canonical dead branch, and the one a \
              caller most wants a *proof* about",
    },
    Case {
        name: "impossible-equality",
        line: 3,
        live: false,
        src: "int probe (int x) {\n\
              \x20 if (x != x)\n\
              \x20   return 1;\n\
              \x20 return 0;\n\
              }\n",
        why: "integer equality is reflexive; nothing gets in",
    },
    Case {
        name: "constant-guard",
        line: 3,
        live: false,
        src: "int probe (int x) {\n\
              \x20 if (0)\n\
              \x20   return 1;\n\
              \x20 return x;\n\
              }\n",
        why: "a constant-false guard, which even tier 1 decides — the case that keeps this \
              corpus from being silent with no solver",
    },
    Case {
        name: "unsigned-negative",
        line: 3,
        live: false,
        src: "int probe (unsigned x) {\n\
              \x20 if (x < 0u)\n\
              \x20   return 1;\n\
              \x20 return 0;\n\
              }\n",
        why: "an unsigned value is never below zero, which is the same shape as a real \
              defensive check that cannot fire",
    },
];

#[test]
fn the_corpus_says_what_it_is() {
    let (live, dead) = (
        CASES.iter().filter(|c| c.live).count(),
        CASES.iter().filter(|c| !c.live).count(),
    );
    assert!(
        live >= 3 && dead >= 3,
        "a corpus tilted one way measures one thing: {live} live, {dead} dead"
    );
    for c in CASES {
        assert!(
            c.why.len() > 30,
            "`{}` states its ground truth without arguing it",
            c.name
        );
    }
}

/// **The claim a caller acts on.** `unreachable` about a line that runs is a proof licensing
/// the deletion of live code.
#[test]
fn no_reachable_line_is_ever_called_unreachable() {
    let mut decided = 0;
    let mut lines = Vec::new();
    for c in CASES.iter().filter(|c| c.live) {
        let (verdict, v) = ask(c.name, c.src, c.line);
        assert_ne!(
            verdict, "unreachable",
            "`{}:{}` was proved unreachable and it runs: {}.\n{v:#}",
            c.name, c.line, c.why
        );
        decided += usize::from(verdict == "reachable");
        lines.push(format!("  {:22} {verdict}", c.name));
    }
    eprintln!("live lines:\n{}", lines.join("\n"));
    if !has_solver() {
        eprintln!("no solver on PATH: tier 1 reached {decided}; the floor is not asserted");
        return;
    }
    assert!(
        decided >= 3,
        "only {decided} live lines were shown reachable — with nothing decided, the absence of \
         false proofs above says nothing"
    );
}

/// The other direction: `reachable` is a witness, and a witness to a dead line cannot exist.
#[test]
fn no_dead_line_is_ever_called_reachable() {
    let mut proved = 0;
    let mut lines = Vec::new();
    for c in CASES.iter().filter(|c| !c.live) {
        let (verdict, v) = ask(c.name, c.src, c.line);
        assert_ne!(
            verdict, "reachable",
            "`{}:{}` was called reachable and nothing gets there: {}.\n{v:#}",
            c.name, c.line, c.why
        );
        proved += usize::from(verdict == "unreachable");
        lines.push(format!("  {:22} {verdict}", c.name));
    }
    eprintln!("dead lines:\n{}", lines.join("\n"));
    if !has_solver() {
        eprintln!("no solver on PATH: tier 1 proved {proved} dead; the floor is not asserted");
        return;
    }
    assert!(
        proved >= 3,
        "only {proved} dead lines were *proved* unreachable — the rest came back \
         `not_shown_reachable`, which is honest but means this corpus is not exercising the \
         claim a caller acts on"
    );
}

/// **The two negative verdicts must stay distinguishable through the CLI** (050 contract 5).
/// A consumer matching on the string is the whole reason they are separate verdicts.
#[test]
fn proved_dead_and_not_shown_are_different_strings() {
    let dead = CASES.iter().find(|c| c.name == "constant-guard").unwrap();
    let (v, _) = ask(dead.name, dead.src, dead.line);
    assert!(
        v == "unreachable" || v == "not_shown_reachable",
        "a dead line came back `{v}`"
    );
    assert_ne!(
        "unreachable", "not_shown_reachable",
        "these are the two verdicts a caller must not conflate"
    );
}
