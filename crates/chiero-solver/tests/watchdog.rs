//! Covers: **022 §4** — "Timeouts via `(set-option :timeout N)` plus a wall-clock watchdog;
//! on watchdog fire the process is killed, restarted, the assertion stack is **replayed**,
//! and the query returns `Unknown(Timeout)`. Replay correctness is contract 14."
//!
//! None of that exists. `UnknownReason` has three variants and none is `Timeout`; nothing
//! in the crate mentions a duration or a watchdog. A backend that accepts a query and never
//! answers stops the engine for as long as the process lives.
//!
//! The same section says "the engine must never crash because a solver misbehaved", and
//! that sentence is usually read as being about *parse errors* — `Unknown(BackendError)`
//! covers those, and does. It is equally about a solver that says nothing at all, which is
//! the failure mode a subprocess actually has: killed by the OOM killer, stopped by a
//! debugger, or simply given a formula it will grind on past any useful horizon. Hanging is
//! not crashing, and from a caller's side it is worse — a crash at least ends.
//!
//! **The fake solver is the test.** Pointing `SmtLib::at` at a shell script that reads its
//! input and never replies reproduces every one of those in a form that is deterministic
//! and needs no pathological formula. `z3` on a hard query is the same thing with worse
//! reproducibility.

use chiero_solver::*;

/// A solver that accepts everything and answers nothing.
fn mute_solver(tag: &str) -> Option<std::path::PathBuf> {
    if !cfg!(unix) {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("chiero-watchdog-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let p = dir.join("mute");
    std::fs::write(&p, "#!/bin/sh\ncat > /dev/null\n").ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    Some(p)
}

/// A formula tier 1 cannot decide, so the query really does reach the backend.
fn nonlinear(a: &mut TermArena) -> Term {
    let x = a.var(Sort::BitVec(32), "x");
    let seven = a.bv(32, 7);
    let sq = a.mul(x, x);
    a.eq(sq, seven)
}

/// Run `f` on a worker thread and give it `secs` to finish.
///
/// The bound is the assertion. A test that simply called `check` would *become* the hang it
/// is testing for, and a suite that hangs reports nothing at all — the one outcome worse
/// than a red.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs)).ok()
}

/// **A solver that never answers does not stop the caller.**
#[test]
fn a_mute_backend_times_out_rather_than_hanging() {
    let Some(fake) = mute_solver("hang") else {
        eprintln!("SKIP: the fake solver needs a unix shell");
        return;
    };
    let got = within(90, move || {
        let mut a = TermArena::new();
        let t = nonlinear(&mut a);
        let mut s = TieredSolver::with_backend(SmtLib::at(&fake));
        format!("{:?}", s.check(&mut a, &[t]))
    });
    let got = got.expect(
        "the backend accepted the query and never replied, and `check` never returned. \
         022 §4 asks for a wall-clock watchdog so this is `Unknown(Timeout)` instead",
    );
    assert!(
        got.contains("Unknown"),
        "a solver that said nothing decided nothing: {got}"
    );
    assert!(
        got.contains("Timeout"),
        "022 §4 names the verdict `Unknown(Timeout)`, which is a fact about the clock. A \
         reader who cannot tell it from `Incomplete` cannot tell `ask again with more time` \
         from `this fragment is out of reach`: {got}"
    );
}

/// **And the session survives it.**
///
/// The watchdog kills and *restarts*; without the restart the next query meets a dead pipe
/// or a process midway through reading a query nobody finished sending. Either way the
/// second call is the one that shows whether recovery happened, and a solver stuck after
/// one slow query is a solver that gives up on the whole run.
#[test]
fn a_timed_out_session_still_answers_the_next_query() {
    let Some(fake) = mute_solver("twice") else {
        eprintln!("SKIP: the fake solver needs a unix shell");
        return;
    };
    let got = within(180, move || {
        let mut a = TermArena::new();
        let t = nonlinear(&mut a);
        let mut s = TieredSolver::with_backend(SmtLib::at(&fake));
        let first = format!("{:?}", s.check(&mut a, &[t]));
        let second = format!("{:?}", s.check(&mut a, &[t]));
        (first, second)
    });
    let (first, second) = got.expect("one of the two queries never returned");
    assert!(first.contains("Unknown"), "first: {first}");
    assert!(
        second.contains("Unknown"),
        "the second query must reach a verdict too, which it cannot if the timed-out \
         session was left wedged: {second}"
    );
}
