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

/// A harness that is a whole C program of its own, so the fixtures can attempt things a
/// generated equivalence harness never would.
fn raw(source: &str) -> Replay {
    Replay {
        source: source.to_string(),
        // These fixtures are whole programs; there is no before/after pair to compile beside
        // them, which is itself worth exercising — a harness with no units must still run.
        units: Vec::new(),
        claim: "a fixture that attempts something 050 §6 forbids".into(),
    }
}

/// The result protocol every harness follows: two numbers in a file the harness is told about.
const REPORT: &str = "\
#include <stdio.h>\n\
static void chiero_report (long long b, long long a)\n\
{\n\
  FILE *f = fopen (CHIERO_RESULT, \"w\");\n\
  if (!f) return;\n\
  fprintf (f, \"before=%lld after=%lld\\n\", b, a);\n\
  fclose (f);\n\
}\n";

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
    let src = format!(
        "{REPORT}\
         #include <sys/socket.h>\n\
         #include <netinet/in.h>\n\
         #include <arpa/inet.h>\n\
         #include <unistd.h>\n\
         int main (void)\n\
         {{\n\
         \x20 int s = socket (AF_INET, SOCK_STREAM, 0);\n\
         \x20 long long reached = 0;\n\
         \x20 if (s >= 0) {{\n\
         \x20   struct sockaddr_in a;\n\
         \x20   a.sin_family = AF_INET;\n\
         \x20   a.sin_port = htons (80);\n\
         \x20   a.sin_addr.s_addr = inet_addr (\"1.1.1.1\");\n\
         \x20   reached = connect (s, (struct sockaddr *) &a, sizeof a) == 0;\n\
         \x20   close (s);\n\
         \x20 }}\n\
         \x20 chiero_report (reached, 0);\n\
         \x20 return reached == 0;\n\
         }}\n"
    );
    let d = dir("net");
    match run_with(&raw(&src), &cc, &d, &[]) {
        // `before` is 1 if the connect succeeded. It must not have.
        Outcome::Demonstrated { before, .. } | Outcome::NotDemonstrated { before, .. } => {
            assert_eq!(before, 0, "the harness reached the network");
        }
        other => panic!("the fixture should build and run: {other:?}"),
    }
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
    let src = format!(
        "{REPORT}\
         int main (void)\n\
         {{\n\
         \x20 FILE *f = fopen (\"{}\", \"w\");\n\
         \x20 long long escaped = 0;\n\
         \x20 if (f) {{ fputs (\"x\", f); fclose (f); escaped = 1; }}\n\
         \x20 chiero_report (escaped, 0);\n\
         \x20 return 0;\n\
         }}\n",
        outside.display()
    );
    let escaped = match run_with(&raw(&src), &cc, &d, &[]) {
        Outcome::Demonstrated { before, .. } | Outcome::NotDemonstrated { before, .. } => {
            before == 1
        }
        other => panic!("the fixture should build and run: {other:?}"),
    };
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
    let src = format!(
        "{REPORT}\
         #include <stdlib.h>\n\
         #include <string.h>\n\
         int main (void)\n\
         {{\n\
         \x20 for (long long i = 0; i < 100000; i++) {{\n\
         \x20   void *p = malloc (16u << 20);\n\
         \x20   if (!p) {{ chiero_report (0, 1); return 0; }}\n\
         \x20   memset (p, 1, 4096);\n\
         \x20 }}\n\
         \x20 chiero_report (1, 1);\n\
         \x20 return 0;\n\
         }}\n"
    );
    let d = dir("mem");
    match run_with(&raw(&src), &cc, &d, &[]) {
        // Either the allocation failed (0, 1) or the process died — both are the cap working.
        Outcome::Demonstrated { before, .. } => assert_eq!(before, 0, "the cap did not bite"),
        Outcome::NotDemonstrated { before, after } => {
            panic!("the harness allocated {before}/{after} GB unchecked")
        }
        Outcome::DidNotRun { .. } => {}
        other => panic!("the fixture should build: {other:?}"),
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
