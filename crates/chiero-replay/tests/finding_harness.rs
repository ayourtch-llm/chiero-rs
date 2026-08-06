//! **040 contract 4 — a harness for a *finding*, not only for a divergence.**
//!
//! > 4. **Every finding in the corpus emits a replay harness that compiles** under the TU's own
//! >    flags. A harness that fails to build fails CI.
//!
//! `find_bugs` reports a division by zero with the input that reaches it, and nothing has ever
//! checked that claim against a compiler. The equivalence harness does that for a *rewrite*;
//! this is the same mechanism pointed at one program.
//!
//! # What "demonstrated" means for a finding
//!
//! Not "the two disagree" — there is only one program. A defect is demonstrated when running it
//! at the witness **faults**: a division by zero raises `SIGFPE`, a null dereference `SIGSEGV`.
//! A program that runs to completion has not reproduced the fault, and saying so is contract
//! 11's rule in the shape a finding needs it.
//!
//! **This must reuse the equivalence harness's machinery rather than copy it.** One process per
//! program, `_exit` after writing, the wall clock, the network namespace, the memory cap, the
//! descendant kill — every one of those was earned by a review finding a hole, and a second
//! implementation would start again from the first hole.

use chiero_replay::{FindingOutcome, compiler, emit_finding, run_finding};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-find-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn witness(values: &[(u32, u128)]) -> chiero_exec::Witness {
    chiero_exec::Witness {
        bindings: values
            .iter()
            .enumerate()
            .map(|(index, (width, value))| chiero_exec::Binding {
                origin: chiero_exec::InputOrigin::Param {
                    index,
                    name: String::new(),
                    span: chiero_span::Span::DUMMY,
                },
                width: *width,
                value: *value,
                pinned: true,
            })
            .collect(),
    }
}

/// **A division by zero at the witness faults, and the harness says so.**
#[test]
fn a_division_by_zero_is_reproduced() {
    let Some(cc) = compiler() else { return };
    let d = dir("divzero");
    let src = d.join("div.c");
    std::fs::write(&src, "int f (int n) { return 100 / n; }\n").expect("write");
    let r = emit_finding(&src, "f", &witness(&[(32, 0)]), "division by zero").expect("scalar");
    match run_finding(&r, &cc, &d, &[]) {
        FindingOutcome::Faulted { signal } => {
            assert_eq!(signal, 8, "SIGFPE is what dividing by zero raises");
        }
        other => panic!("100 / 0 must fault: {other:?}"),
    }
}

/// **A witness that does not reach the fault is reported as not reproducing it.**
///
/// The contract-11 shape for a finding: a harness that ran and showed nothing is a downgrade,
/// never a confirmation.
#[test]
fn a_witness_that_does_not_fault_is_not_a_confirmation() {
    let Some(cc) = compiler() else { return };
    let d = dir("nofault");
    let src = d.join("div.c");
    std::fs::write(&src, "int f (int n) { return 100 / n; }\n").expect("write");
    let r = emit_finding(&src, "f", &witness(&[(32, 5)]), "division by zero").expect("scalar");
    match run_finding(&r, &cc, &d, &[]) {
        FindingOutcome::Completed { value } => assert_eq!(value, 20),
        other => panic!("100 / 5 is 20 and does not fault: {other:?}"),
    }
}

/// **A `static` target is reachable**, as it is for the equivalence harness and for the same
/// reason (040 §3.1): the harness includes the source rather than declaring it `extern`.
#[test]
fn a_static_target_is_reachable() {
    let Some(cc) = compiler() else { return };
    let d = dir("static");
    let src = d.join("s.c");
    std::fs::write(&src, "static int f (int n) { return 100 / n; }\n").expect("write");
    let r = emit_finding(&src, "f", &witness(&[(32, 0)]), "division by zero").expect("scalar");
    assert!(
        matches!(run_finding(&r, &cc, &d, &[]), FindingOutcome::Faulted { .. }),
        "040 §3.1: static is the common case"
    );
}

/// **The same refusals as the equivalence harness**, because they are the same rule: a witness
/// that is not an argument list cannot be passed positionally.
#[test]
fn a_witness_that_is_not_an_argument_list_is_refused() {
    let d = dir("refuse");
    let src = d.join("x.c");
    std::fs::write(&src, "int f (int n) { return n; }\n").expect("write");
    let w = chiero_exec::Witness {
        bindings: vec![chiero_exec::Binding {
            origin: chiero_exec::InputOrigin::ExternReturn {
                func: "p".into(),
                span: chiero_span::Span::DUMMY,
                seq: 0,
            },
            width: 32,
            value: 0,
            pinned: true,
        }],
    };
    assert!(
        emit_finding(&src, "f", &w, "something").is_err(),
        "an extern's return is not a parameter"
    );
}

/// **And it runs inside the same sandbox.** The machinery was earned one review finding at a
/// time; a second implementation would start again from the first hole.
#[test]
fn a_finding_harness_that_never_finishes_is_bounded() {
    let Some(cc) = compiler() else { return };
    let d = dir("hang");
    let src = d.join("h.c");
    std::fs::write(&src, "int f (int n) { for (;;) ; return n; }\n").expect("write");
    let r = emit_finding(&src, "f", &witness(&[(32, 1)]), "a loop").expect("scalar");
    let started = std::time::Instant::now();
    let o = run_finding(&r, &cc, &d, &[]);
    assert!(started.elapsed() < std::time::Duration::from_secs(40), "must be bounded");
    assert!(
        matches!(o, FindingOutcome::DidNotRun { .. }),
        "a harness that never finishes reproduces nothing: {o:?}"
    );
}
