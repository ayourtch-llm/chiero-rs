//! Build/CI automation. See `docs/specs/001-architecture.md` §4 and
//! `docs/specs/070-testing-and-tdd-protocol.md` §4.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check-deps") => check_deps(),
        Some(other) => {
            eprintln!("unknown task: {other}");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("usage: cargo xtask <task>\n\n  check-deps   enforce the 001 §4 dependency rules");
}

/// 001 contract 8: exit non-zero when a §4 rule is violated.
fn check_deps() -> ExitCode {
    let graph = match xtask::deps::workspace_graph() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let violations = xtask::deps::check(&graph);
    if violations.is_empty() {
        println!("check-deps: {} crates, no violations", graph.len());
        return ExitCode::SUCCESS;
    }
    eprintln!("check-deps: {} violation(s)\n", violations.len());
    for v in &violations {
        eprintln!("  {v}");
    }
    ExitCode::FAILURE
}
