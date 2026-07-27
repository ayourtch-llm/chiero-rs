//! Build/CI automation. See `docs/specs/001-architecture.md` §4 and
//! `docs/specs/070-testing-and-tdd-protocol.md` §4.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check-deps") => check_deps(),
        Some("contract-coverage") => contract_coverage(),
        Some("check-vpp-leak") => check_vpp_leak(),
        Some("check-proof-surface") => match xtask::proof_surface::check_proof_surface() {
            0 => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        },
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
         check-vpp-leak   enforce 001 §4 rule 4 / contract 5\n  \
         check-proof-surface  enforce 023 contract 13a (a proof cannot be forged)\n  \
         contract-coverage    report M1 exit coverage over 020-024 (080)"
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

/// 080's M1 exit is "**all** numbered contracts of 020-024 are green", named as documents
/// rather than ranges. This reports which are not cited by any test — a coverage measure,
/// not a correctness one, answering "what has nobody looked at".
fn contract_coverage() -> ExitCode {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    match xtask::contracts::measure(&root) {
        Ok(cov) => {
            let mut total = 0usize;
            let mut missing = 0usize;
            for doc in xtask::contracts::M1_DOCS {
                let declared = cov.declared.get(*doc).map(|v| v.len()).unwrap_or(0);
                let un = cov.uncovered(doc);
                total += declared;
                missing += un.len();
                println!(
                    "{doc}: {}/{} cited{}",
                    declared - un.len(),
                    declared,
                    if un.is_empty() {
                        String::new()
                    } else {
                        format!("  — uncited: {}", un.join(", "))
                    }
                );
            }
            println!(
                "\nM1 exit: {}/{} contracts cited by a test",
                total - missing,
                total
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
