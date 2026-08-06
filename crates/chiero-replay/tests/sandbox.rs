//! **050 §6 and contract 12 — what running a harness is actually allowed to do.**
//!
//! > 12. Replay execution cannot reach the network and cannot write outside the scratch
//! >     directory (asserted by a fixture harness that attempts both).
//!
//! Running a harness compiles and executes code from the tree under analysis. The contract is
//! about what that code can do, and the fixtures attempt exactly the two things it must not.
//!
//! # The claim is tested against reality, not asserted
//!
//! Not everything in §6 is enforceable on every machine — network isolation needs unprivileged
//! user namespaces, and confining writes without root needs more than that. So
//! [`chiero_replay::sandbox`] *reports* what this machine can enforce, and the tests below
//! check that report against what a harness can actually do. A claim of confinement that is
//! not true is worse than an honest "not enforced": the first is acted on.

use chiero_replay::{Outcome, Replay, run_with, sandbox};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-sandbox-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// A fixture as the two programs `run_with` expects — the same body twice, since these tests
/// are about what a program is *allowed to do* rather than about a divergence.
///
/// **Built through the real shape, not around it.** These used to hand-construct a `Replay`
/// with no units, which meant they exercised a path no caller takes. A reviewer's note: a
/// direct crate user gets an unguarded oracle, and a test that is such a user proves less than
/// it looks.
fn two_programs(body: &str) -> Replay {
    let unit = |tag: &str| {
        (
            format!("chiero_{tag}.c"),
            format!(
                "#include <stdio.h>\n#include <unistd.h>\n\
                 {body}\n\
                 int main (void)\n{{\n  \
                 long long v = chiero_fixture ();\n  \
                 FILE *o = fopen (CHIERO_RESULT, \"w\");\n  \
                 if (!o) return 2;\n  \
                 fprintf (o, \"value=%lld\\n\", v);\n  \
                 fclose (o);\n  \
                 _exit (0);\n}}\n"
            ),
        )
    };
    Replay {
        source: "/* a fixture that attempts something 050 §6 forbids */\n".into(),
        units: vec![unit("before"), unit("after")],
        claim: "a fixture that attempts something 050 §6 forbids".into(),
    }
}

/// What the fixture reported, however the comparison came out — these programs are identical,
/// so the interesting number is the value rather than the verdict.
fn reported(o: Outcome) -> i64 {
    match o {
        Outcome::Demonstrated { before, .. }
        | Outcome::NotDemonstrated { before, .. }
        | Outcome::Nondeterministic { first: before, .. } => before,
        other => panic!("the fixture should build and run: {other:?}"),
    }
}

/// **A harness that tries to open a socket must not reach the network.**
#[test]
fn a_harness_cannot_reach_the_network() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    if !sandbox().network {
        // Nothing here can isolate the network; the report says so and this test says the same
        // rather than passing quietly.
        assert!(
            sandbox().describe().contains("network is NOT"),
            "an unenforced network must be reported as unenforced: {}",
            sandbox().describe()
        );
        return;
    }
    let body = "#include <sys/socket.h>\n#include <netinet/in.h>\n#include <arpa/inet.h>\n\
                static long long chiero_fixture (void)\n{\n  \
                int s = socket (AF_INET, SOCK_STREAM, 0);\n  \
                if (s < 0) return 0;\n  \
                struct sockaddr_in a;\n  \
                a.sin_family = AF_INET;\n  \
                a.sin_port = htons (80);\n  \
                a.sin_addr.s_addr = inet_addr (\"1.1.1.1\");\n  \
                long long reached = connect (s, (struct sockaddr *) &a, sizeof a) == 0;\n  \
                close (s);\n  \
                return reached;\n}";
    let d = dir("net");
    assert_eq!(
        reported(run_with(&two_programs(body), &cc, &d, &[])),
        0,
        "the harness reached the network"
    );
}

/// **What a harness can write is claimed exactly, and the claim is checked.**
///
/// Confining writes without root needs more than a user namespace — remounting the filesystem
/// read-only inside one fails on the underlying device. So this asserts the *report* matches
/// what a harness actually manages, in whichever direction that is. An unenforced limit
/// reported as enforced is the dangerous case; this test fails on it.
#[test]
fn what_a_harness_may_write_is_claimed_accurately() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let d = dir("write");
    let outside = d
        .join("..")
        .join(format!("chiero-escape-{}", std::process::id()));
    let _ = std::fs::remove_file(&outside);
    let body = format!(
        "static long long chiero_fixture (void)\n{{\n  \
         FILE *f = fopen (\"{}\", \"w\");\n  \
         if (!f) return 0;\n  \
         fputs (\"x\", f);\n  \
         fclose (f);\n  \
         return 1;\n}}",
        outside.display()
    );
    let escaped = reported(run_with(&two_programs(&body), &cc, &d, &[])) == 1;
    let _ = std::fs::remove_file(&outside);

    assert_eq!(
        escaped,
        !sandbox().writes_confined,
        "the sandbox reports writes {} and a harness {} — the report must match reality:\n{}",
        if sandbox().writes_confined {
            "confined"
        } else {
            "unconfined"
        },
        if escaped { "escaped" } else { "did not escape" },
        sandbox().describe()
    );
}

/// **A memory cap, and it bites.** A harness that allocates without bound must be stopped by
/// the limit rather than by the machine.
#[test]
fn a_harness_that_allocates_without_bound_is_stopped() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    if sandbox().memory_bytes.is_none() {
        return;
    }
    let body = "#include <stdlib.h>\n#include <string.h>\n\
                static long long chiero_fixture (void)\n{\n  \
                for (long long i = 0; i < 100000; i++) {\n    \
                void *p = malloc (16u << 20);\n    \
                if (!p) return 0;\n    \
                memset (p, 1, 4096);\n  }\n  \
                return 1;\n}";
    let d = dir("mem");
    match run_with(&two_programs(body), &cc, &d, &[]) {
        // Either the allocation failed (0) or the process died — both are the cap working.
        Outcome::NotDemonstrated { before, .. } => assert_eq!(before, 0, "the cap did not bite"),
        Outcome::DidNotRun { .. } => {}
        other => panic!("the cap must bite: {other:?}"),
    }
}

/// **The report is part of the answer**, so a caller can read what its verdict rests on.
#[test]
fn the_sandbox_describes_itself() {
    let d = sandbox().describe();
    for want in ["network", "memory", "writes"] {
        assert!(d.contains(want), "`{want}` missing from: {d}");
    }
}

/// **050 §6 covers compilation, not only execution.**
///
/// > "**Compilation** and replay execution ... run in a sandbox with ... a wall-clock limit,
/// > and a memory cap"
///
/// The limit wrapped the produced binary and not the compiler, so a source whose `#include`
/// names a FIFO hung the tool — the same consequence as the unbounded execution that limit was
/// added for, through the neighbouring door.
#[test]
fn a_compile_that_never_finishes_is_bounded_too() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let d = dir("fifo");
    let fifo = d.join("blocks.h");
    let _ = std::fs::remove_file(&fifo);
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !made {
        return; // no mkfifo here; the property is untestable rather than untrue
    }
    let body = format!(
        "#include \"{}\"\nstatic long long chiero_fixture (void) {{ return 0; }}",
        fifo.display()
    );
    let start = std::time::Instant::now();
    let outcome = run_with(&two_programs(&body), &cc, &d, &[]);
    let _ = std::fs::remove_file(&fifo);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(60),
        "the compile was never bounded; it took {:?}",
        start.elapsed()
    );
    assert!(
        matches!(
            outcome,
            Outcome::DidNotBuild { .. } | Outcome::DidNotRun { .. }
        ),
        "a compile that could not finish is not a verdict: {outcome:?}"
    );
}

/// **A flag that merges the two programs is refused.**
///
/// `ReplaySources::flags` carries the translation unit's real `compile_commands.json` flags,
/// which is 040 §3's requirement — and `-fcommon` merges tentative definitions across the two
/// programs, re-opening the shared-global route the separation exists to close. Passing the
/// TU's flags is right; passing one that undoes the separation is not.
#[test]
fn a_flag_that_undoes_the_separation_is_refused() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let d = dir("fcommon");
    let body = "int g;\nstatic long long chiero_fixture (void) { return ++g; }";
    match run_with(&two_programs(body), &cc, &d, &["-fcommon".to_string()]) {
        Outcome::DidNotRun { detail } => assert!(
            detail.contains("-fcommon"),
            "the refusal must name the flag: {detail}"
        ),
        other => panic!("a flag that merges the two programs must be refused: {other:?}"),
    }
}

/// **A scratch path the launcher cannot use is refused, not silently mishandled.**
///
/// The binary's path is interpolated into `sh -c "... exec '{}'"`, so a relative one becomes a
/// bare word searched on a PATH that does not contain it, and a quote ends the string early.
/// Both produced `DidNotRun: the harness wrote no result`, which points a reader at the
/// harness rather than at the argument they passed.
#[test]
fn a_scratch_path_the_launcher_cannot_use_is_refused_by_name() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let body = "static long long chiero_fixture (void) { return 1; }";
    match run_with(
        &two_programs(body),
        &cc,
        std::path::Path::new("relative/dir"),
        &[],
    ) {
        Outcome::DidNotRun { detail } => assert!(
            detail.contains("absolute"),
            "the refusal must say what is wrong with the path: {detail}"
        ),
        other => panic!("a relative scratch directory must be refused: {other:?}"),
    }
}
