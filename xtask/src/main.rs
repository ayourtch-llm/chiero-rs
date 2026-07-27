//! Build/CI automation. See `docs/specs/001-architecture.md` §4 and
//! `docs/specs/070-testing-and-tdd-protocol.md` §4.

fn main() -> std::process::ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-deps") => std::process::ExitCode::SUCCESS,
        Some(other) => {
            eprintln!("unknown task: {other}");
            std::process::ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: cargo xtask <check-deps>");
            std::process::ExitCode::FAILURE
        }
    }
}
