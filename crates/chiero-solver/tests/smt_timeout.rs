//! Covers: **022 §4** — "Timeouts via `(set-option :timeout N)` **plus** a wall-clock
//! watchdog".
//!
//! Wave 163 built the watchdog and stopped there. The two halves do different jobs and the
//! spec asks for both:
//!
//! - the **option** tells the solver its own budget, so a query it cannot finish comes back
//!   as its own `unknown` and the session survives;
//! - the **watchdog** covers the case the option cannot — a process that is wedged,
//!   swapped out, or stopped, and will not honour any option because it is not running the
//!   solver loop at all.
//!
//! With only the watchdog every slow query costs a killed process, a respawn, and every
//! variable redeclared. That is a real cost paid on exactly the queries that were already
//! the expensive ones.
//!
//! **A recording fake, not a slow query.** The obvious test — give z3 something hard and
//! watch it give up — depends on z3 being slow, which is a property of the machine and the
//! z3 build rather than of chiero. (Measured while writing this: a 64-bit semiprime
//! factorisation this test would have used answers in 241ms.) A test built on it passes
//! today and fails on a faster box for no reason anyone can act on. What is actually
//! contracted is that chiero *tells the solver its budget*, and a fake that records its
//! input observes that exactly.

use chiero_solver::*;

/// A solver that logs everything it is told and answers `unsat` to every `(check-sat)`.
///
/// Answering rather than staying mute keeps this test about the option: a mute fake would
/// trip the watchdog and test wave 163's work instead.
fn recording_solver(tag: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    if !cfg!(unix) {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("chiero-smtopt-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let log = dir.join("log");
    let script = dir.join("recorder");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             while IFS= read -r line; do\n\
             \tprintf '%s\\n' \"$line\" >> '{}'\n\
             \tcase \"$line\" in *'(check-sat)'*) echo unsat;; esac\n\
             done\n",
            log.display()
        ),
    )
    .ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    Some((script, log))
}

/// Force a query through to the backend: nonlinear, so tier 1 declines.
fn ask(script: &std::path::Path) {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let seven = a.bv(32, 7);
    let sq = a.mul(x, x);
    let t = a.eq(sq, seven);
    let mut s = TieredSolver::with_backend(SmtLib::at(script));
    let _ = s.check(&mut a, &[t]);
}

/// **The solver is told its own budget**, and told a smaller one than the watchdog's.
///
/// The second half is the part worth stating. If `:timeout` were set to the watchdog's own
/// duration — or above it — the watchdog would always fire first and the option would be
/// decoration: chiero would still be killing processes it had politely asked to stop. The
/// option only does its job if the solver gets to answer first.
#[test]
fn the_backend_is_told_its_own_timeout() {
    let Some((script, log)) = recording_solver("opt") else {
        eprintln!("SKIP: the fake solver needs a unix shell");
        return;
    };
    ask(&script);
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("(set-option :timeout"),
        "022 §4 asks for `(set-option :timeout N)` alongside the watchdog, and the backend \
         was told nothing about its budget. What it received:\n{text}"
    );

    let ms: u64 = text
        .split("(set-option :timeout")
        .nth(1)
        .and_then(|rest| {
            rest.trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .find(|s| !s.is_empty())
                .and_then(|d| d.parse().ok())
        })
        .unwrap_or_else(|| panic!("the option should carry a number:\n{text}"));
    assert!(ms > 0, "a zero budget would refuse every query: {ms}");

    // The watchdog's default is 10s; `$CHIERO_SMT_TIMEOUT` can raise or lower it, and the
    // option must stay under whatever it is. Read here rather than hard-coded so the two
    // cannot drift apart silently.
    let watchdog_ms = std::env::var("CHIERO_SMT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|s| s * 1000)
        .unwrap_or(10_000);
    assert!(
        ms < watchdog_ms,
        "the solver's own budget ({ms}ms) must expire before the watchdog ({watchdog_ms}ms), \
         or the process is killed while it still had time to answer and the option is \
         decoration"
    );
}

/// **And the option is sent once per process, not once per query.**
///
/// It is session state. Re-sending it before every `(check-sat)` costs a round trip on the
/// hot path to restate something the process already knows, and 022 §4's whole reason for
/// keeping the process alive is that those round trips dominate short queries.
#[test]
fn the_timeout_is_session_state_not_per_query() {
    let Some((script, log)) = recording_solver("once") else {
        eprintln!("SKIP: the fake solver needs a unix shell");
        return;
    };
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let sq = a.mul(x, x);
    let mut s = TieredSolver::with_backend(SmtLib::at(&script));
    for k in 5..9u128 {
        let c = a.bv(32, k);
        let t = a.eq(sq, c);
        let _ = s.check(&mut a, &[t]);
    }
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let sent = text.matches("(set-option :timeout").count();
    let asked = text.matches("(check-sat)").count();
    assert!(
        asked >= 2,
        "the fixture should reach the backend twice: {text}"
    );
    assert_eq!(
        sent, 1,
        "one process, one budget: the option was sent {sent} times for {asked} queries"
    );
}
