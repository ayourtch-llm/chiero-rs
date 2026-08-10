//! **A finding must say where it is.**
//!
//! Found 2026-08-10 while reading 040 contract 1 — *"≥ 1 positive fixture (fires, exactly once,
//! **at the right span**)"* — and discovering the contract could not be checked from the command
//! at all, because the answer carries no span:
//!
//! ```text
//! findings:
//!   - message: null-dereference: access at offset 0 of NULL
//!     paths: 1
//!     fidelity: Unknown
//! ```
//!
//! `chiero_exec::Finding` **has** `span: Span` and it is dropped when the envelope is built. The
//! engine knows where the defect is; the surface a caller reads throws it away. So a report says
//! *what* is wrong and never *where*, in a translation unit that may be a megabyte of expanded
//! headers, and neither an agent nor a person can act on it without re-running the search by
//! hand.
//!
//! This is the actionability of the flagship operation, so the assertion is deliberately blunt:
//! every finding names a file and a line, and the line is the one the defect is on.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-loc-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn find_bugs(name: &str, src: &str) -> serde_json::Value {
    let p = scratch().join(name);
    std::fs::write(&p, src).expect("write");
    let out = Command::new(bin())
        .args([
            "find-bugs",
            p.to_str().unwrap(),
            "--entry",
            "probe",
            "--json",
            "--no-system-headers",
            "--time-budget",
            "20",
        ])
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "envelope is not JSON ({e}):\n{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// A null dereference on a line chosen so the answer is unambiguous.
const NULL_DEREF: &str = "\
int probe (int n)
{
  int *p = 0;
  if (n > 0)
    return *p;
  return 0;
}
";

#[test]
fn every_finding_names_a_file_and_a_line() {
    let v = find_bugs("loc.c", NULL_DEREF);
    let findings = v["result"]["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("no findings array:\n{v:#}"));
    assert!(
        !findings.is_empty(),
        "the fixture must produce a finding or this test measures nothing:\n{v:#}"
    );
    for f in findings {
        assert!(
            f["file"].is_string(),
            "a finding with no `file` cannot be acted on:\n{f:#}"
        );
        assert!(
            f["line"].is_u64(),
            "a finding with no `line` cannot be acted on:\n{f:#}"
        );
    }
}

/// 040 contract 1's *"at the right span"*, which is the half no test could reach.
#[test]
fn the_line_is_the_one_the_defect_is_on() {
    let v = find_bugs("loc2.c", NULL_DEREF);
    let findings = v["result"]["findings"].as_array().expect("findings");
    let lines: Vec<u64> = findings.iter().filter_map(|f| f["line"].as_u64()).collect();
    assert!(
        lines.contains(&5),
        "the dereference is on line 5 (`return *p;`) and the findings point at {lines:?}:\n{v:#}"
    );
    assert!(
        findings
            .iter()
            .all(|f| f["file"].as_str().is_some_and(|s| s.ends_with("loc2.c"))),
        "a finding names a file that is not the one analysed:\n{v:#}"
    );
}
