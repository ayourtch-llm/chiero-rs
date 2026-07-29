//! Covers: **022 §4 and contract 17** — "The emitted SMT-LIB2 is a **first-class artifact**:
//! `--dump-queries <dir>` writes every query, which is how a solver disagreement gets
//! reported upstream and how chiero's own bugs get bisected", and "`--dump-queries` output
//! is valid SMT-LIB2 accepted by z3, and re-running it standalone reproduces chiero's
//! answer."
//!
//! `TermArena::to_smtlib` exists and its doc names this contract as the reason it is
//! careful. Nothing calls it to write a file. So the one artifact the spec asks for by
//! name — the thing you hand someone when chiero and z3 disagree — cannot be produced.
//!
//! `paranoid` mode already *detects* that disagreement and asserts on it. What it cannot do
//! is show you the query, which is the whole of the follow-up work.
//!
//! **The knob is an environment variable**, matching `$CHIERO_SMT_SOLVER` and
//! `$CHIERO_SMT_TIMEOUT`. `--dump-queries` is a CLI spelling and `chiero-cli` is a stub; the
//! library needs the mechanism reachable now, and the flag becomes one line over it later.
//!
//! Contract 17 is a **round trip**, and that is the point: a dump that is merely written is
//! worth nothing if z3 rejects it or answers differently. The test runs the file z3 was
//! never given and requires the verdict chiero reported.
//!
//! It is one test rather than two because the knob is process-global: two tests setting it
//! to two directories race, and the loser looks for its file in the winner's directory.

use chiero_solver::*;

/// A constant no other query in the suite uses, so the file this test wrote can be found
/// among any that other tests dumped into the same directory.
const MARKER: u128 = 987_654_321;

fn z3() -> Option<std::path::PathBuf> {
    SmtLib::discover().map(|s| s.path().to_path_buf())
}

/// Ask z3 to run a dumped script exactly as a person would when handed one.
fn rerun(z3: &std::path::Path, file: &std::path::Path) -> String {
    let out = std::process::Command::new(z3)
        .arg("-smt2")
        .arg(file)
        .output()
        .expect("z3 runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.trim().to_string()
}

/// **Every backend query is written, and re-running it reproduces the answer.**
#[test]
fn a_dumped_query_reproduces_chieros_answer() {
    let Some(z3) = z3() else {
        eprintln!("SKIP: no SMT-LIB backend on PATH");
        return;
    };
    let dir = std::env::temp_dir().join(format!("chiero-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");

    // SAFETY: the suite is threaded, and this is process-global. It is set once, never
    // unset, and only ever *enables* a dump — a concurrent test writes extra files into
    // this directory and changes no verdict. The file this test checks is found by its
    // marker rather than by being the only one.
    unsafe {
        std::env::set_var("CHIERO_DUMP_QUERIES", &dir);
    }

    // A query tier 1 cannot decide, so it actually reaches the backend, with a definite
    // answer so "reproduces chiero's answer" has something to compare.
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(64), "x");
    let marker = a.bv(64, MARKER);
    let sq = a.mul(x, x);
    let t = a.eq(sq, marker);
    let mut s = TieredSolver::with_backend(SmtLib::at(&z3));
    let verdict = match s.check(&mut a, &[t]) {
        CheckResult::Sat(_) => "sat",
        CheckResult::Unsat => "unsat",
        other => panic!("the backend should decide this: {other:?}"),
    };

    let files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("the dump directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    assert!(
        !files.is_empty(),
        "022 §4 asks for every query to be written to {dir:?}, and nothing was"
    );

    let marker_text = MARKER.to_string();
    let mine = files
        .iter()
        .find(|f| {
            std::fs::read_to_string(f)
                .map(|c| c.contains(&marker_text))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("no dumped file holds this test's query: {files:?}"));

    let replayed = rerun(&z3, mine);
    assert!(
        !replayed.contains("error"),
        "contract 17: the dump must be valid SMT-LIB2 that z3 accepts. z3 said:\n{replayed}\n\
         from:\n{}",
        std::fs::read_to_string(mine).unwrap_or_default()
    );
    assert!(
        replayed.contains(verdict),
        "contract 17: re-running the dump standalone must reproduce chiero's answer. \
         chiero said {verdict}, the script said {replayed:?}"
    );

    // **And it is a script, not a fragment** — which is what *standalone* means and what
    // the round trip above would not catch on its own: a file that happened to reproduce
    // the answer because z3 was lenient about a missing declaration would still be useless
    // to the person you handed it to.
    //
    // These live in the same test rather than beside it because the knob is an environment
    // variable and therefore process-global: two tests setting it to two directories race,
    // and the loser looks for its file in the winner's directory. One setter, no race.
    let text = std::fs::read_to_string(mine).expect("readable");
    assert!(
        text.contains("declare-fun") || text.contains("declare-const"),
        "a standalone script declares its variables:\n{text}"
    );
    assert!(
        text.contains("(check-sat)"),
        "a standalone script asks the question:\n{text}"
    );
    assert!(
        text.contains("(assert"),
        "a standalone script states the assertions:\n{text}"
    );
}
