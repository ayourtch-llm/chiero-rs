//! Build/CI automation. See `docs/specs/001-architecture.md` §4 and
//! `docs/specs/070-testing-and-tdd-protocol.md` §4.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check-deps") => check_deps(),
        Some("check-vpp-leak") => check_vpp_leak(),
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
    eprintln!(
        "usage: cargo xtask <task>\n\n  \
         check-deps       enforce the 001 §4 graph rules (1,2,3,5,6,7)\n  \
         check-vpp-leak   enforce 001 §4 rule 4 / contract 5"
    );
}

/// 001 contract 8: exit non-zero when a §4 rule is violated.
fn check_deps() -> ExitCode {
    match xtask::deps::workspace_graph() {
        Ok(g) => xtask::deps::report(&g),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 001 §4 rule 4 / contract 5.
fn check_vpp_leak() -> ExitCode {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("crates");
    match xtask::vpp_leak::scan(&crates) {
        Ok(leaks) if leaks.is_empty() => {
            println!("check-vpp-leak: no VPP identifiers outside chiero-vpp");
            ExitCode::SUCCESS
        }
        Ok(leaks) => {
            eprintln!("check-vpp-leak: {} leak(s)\n", leaks.len());
            for l in &leaks {
                eprintln!(
                    "  {}:{}: `{}` in: {}",
                    l.file.display(),
                    l.line,
                    l.marker,
                    l.text
                );
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
