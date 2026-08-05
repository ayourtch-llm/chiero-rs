//! **030 contract 12, a guard rather than a red-green pair.**
//!
//! > `grep -n 'spelling_loc' crates/chiero-gcov/src` yields no hits — correlation is via
//! > `expansion_loc` only (§1).
//!
//! It passes the day it is written and its value is entirely in the day it stops. A `Span` inside
//! a macro expansion has two locations: where the token was *written* (`spelling_loc`, inside the
//! macro body) and where the expansion *happened* (`expansion_loc`, the call site). gcov records
//! only the second — `tests/corpus/coverage/` pins that — so correlating on the first produces
//! line numbers gcov never wrote, which match nothing and are indistinguishable from a line no
//! test covered.
//!
//! That failure is silent, survives every unit test, and would be found by someone wondering why
//! a `vec.h`-heavy change selects no tests. Hence a check a machine makes.

use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn hits_in(dir: &Path, needle: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("readable source directory") {
            let p = e.expect("readable entry").path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("readable source file");
            for (i, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    out.push(format!("{}:{}: {}", p.display(), i + 1, line.trim()));
                }
            }
        }
    }
    out.sort();
    out
}

/// The crate correlates through `expansion_loc` and never through `spelling_loc`.
#[test]
fn nothing_in_this_crate_mentions_spelling_loc() {
    let hits = hits_in(&src_dir(), "spelling_loc");
    assert!(
        hits.is_empty(),
        "030 contract 12: a location inside a macro body is a line gcov never recorded, so \
         correlating on it matches nothing and reads as `no test covers this`:\n{}",
        hits.join("\n")
    );
}

/// The guard can see. A check that scans for a string it could never find is a comment with a
/// test harness attached, so this asserts the scanner finds one that *is* there.
#[test]
fn the_guard_would_notice() {
    assert!(
        !hits_in(&src_dir(), "expansion").is_empty(),
        "the scanner must be able to find a string that is present, or it proves nothing"
    );
}
