//! **An answer must say where it is.**
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

/// One access, one fault, one entry — 040 contract 1's *"exactly once"*, which became a question
/// somebody could ask the moment findings carried a line.
///
/// **The parameter is the point.** `p` is never dereferenced; `q` is. Before 2026-08-10 this
/// produced *two* findings at the same line, identical but for a clause about `p` that is true
/// on one path and not the other — because the consumer grouped on the message, and a message
/// is a rendering rather than an identity.
#[test]
fn one_access_is_one_finding_however_many_paths_reach_it() {
    let src = "int probe (int *p)\n{\n  int *q = 0;\n  return *q;\n}\n";
    let v = find_bugs("once.c", src);
    let findings = v["result"]["findings"].as_array().expect("findings");
    let nulls: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("null-dereference:"))
        })
        .collect();
    assert_eq!(
        nulls.len(),
        1,
        "one dereference of one null pointer is one finding, on however many paths:\n{v:#}"
    );
    // **And the paths are counted rather than dropped.** "1 finding" and "1 finding on 2 paths"
    // are different facts, and the second is what says the parameter split the search.
    assert!(
        nulls[0]["paths"].as_u64().unwrap_or(0) >= 2,
        "the merged entry must say how many paths reached it:\n{}",
        nulls[0]
    );
    // The kept message is the more specific one: every variant is true of some path, and the
    // longer carries the clause a reader cannot get anywhere else.
    assert!(
        nulls[0]["message"]
            .as_str()
            .is_some_and(|m| m.contains("pointer parameter")),
        "the group kept the less informative of its messages:\n{}",
        nulls[0]
    );
}

/// The same rule, one operation over. A proposal that names no location is one a reader cannot
/// navigate to — `dead_branch` said which side was live and nothing about *which branch*.
#[test]
fn every_proposal_names_a_file_and_a_line() {
    let p = scratch().join("opt.c");
    std::fs::write(
        &p,
        "int probe (int x)\n{\n  int a[2];\n  a[0] = 1;\n  return a[0] + a[0];\n}\n",
    )
    .expect("write");
    let out = Command::new(bin())
        .args([
            "find-optimizations",
            p.to_str().unwrap(),
            "--entry",
            "probe",
            "--json",
            "--no-system-headers",
        ])
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"));
    let proposals = v["result"]["proposals"]
        .as_array()
        .unwrap_or_else(|| panic!("no proposals array:\n{v:#}"));
    assert!(
        !proposals.is_empty(),
        "the fixture must produce a proposal or this test measures nothing:\n{v:#}"
    );
    for pr in proposals {
        assert!(
            pr["file"].is_string() && pr["line"].is_u64(),
            "a proposal with no location cannot be acted on:\n{pr:#}"
        );
    }
    // The second `a[0]` is on line 5, and that is the load a reader would remove.
    assert!(
        proposals.iter().any(|pr| pr["line"].as_u64() == Some(5)),
        "no proposal points at the redundant load's own line:\n{v:#}"
    );
}

/// And the third operation that answers "here is something about your code": a record.
///
/// `layout` names the record's tag, which a reader can at least grep for — survivable, and not
/// the same as an answer. A header included from twenty translation units defines the tag in one
/// of them, and that is the file a reader wants.
#[test]
fn every_layout_record_names_a_file_and_a_line() {
    let p = scratch().join("rec.c");
    std::fs::write(
        &p,
        "struct pad\n{\n  char a;\n  int b;\n  char c;\n};\nint probe (void) { return sizeof (struct pad); }\n",
    )
    .expect("write");
    let out = Command::new(bin())
        .args([
            "layout",
            p.to_str().unwrap(),
            "--json",
            "--no-system-headers",
        ])
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"));
    let records = v["result"]["records"]
        .as_array()
        .unwrap_or_else(|| panic!("no records array:\n{v:#}"));
    assert!(!records.is_empty(), "no records to check:\n{v:#}");
    for r in records {
        assert!(
            r["file"].is_string() && r["line"].is_u64(),
            "a record with no location cannot be found in a tree of headers:\n{r:#}"
        );
    }
    assert!(
        records.iter().any(|r| r["line"].as_u64() == Some(1)),
        "`struct pad` is declared on line 1:\n{v:#}"
    );
}

/// **A key that is always `(none)` trains a reader to skip the one report where it matters.**
///
/// `Envelope::render` prints a JSON null as `(none)`, and `chiero-tool` already ruled on this
/// once for `witness_omitted`: *"exactly the training this field must not give a reader, who has
/// to notice it on the one finding where it says 10 593."* `check-reachable` did not get the
/// rule. An `unreachable` verdict cannot have a witness — nothing gets there, that is what the
/// verdict means — and it printed `witness: (none)` and `why: (none)` under every one.
#[test]
fn a_verdict_carries_only_the_fields_its_shape_can_have() {
    let p = scratch().join("reach.c");
    std::fs::write(
        &p,
        "int probe (int x)\n{\n  if (x > 0 && x < 0)\n    return 1;\n  return 0;\n}\n",
    )
    .expect("write");
    let ask = |line: &str| -> serde_json::Value {
        let out = Command::new(bin())
            .args([
                "check-reachable",
                p.to_str().unwrap(),
                "--entry",
                "probe",
                "--line",
                line,
                "--json",
                "--no-system-headers",
            ])
            .output()
            .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"))
    };

    let dead = ask("4");
    assert_eq!(dead["result"]["verdict"], "unreachable", "{dead:#}");
    for k in ["witness", "why"] {
        assert!(
            dead["result"].get(k).is_none(),
            "an `unreachable` verdict carries `{k}`, which its shape cannot have:\n{dead:#}"
        );
    }

    // And the field is still there where it is the whole answer.
    let live = ask("5");
    assert_eq!(live["result"]["verdict"], "reachable", "{live:#}");
    assert!(
        live["result"]["witness"].is_array(),
        "a `reachable` verdict without its witness is a guess:\n{live:#}"
    );
}

/// The same rule, in the operation that wrote it down. Every `find-bugs` finding printed
/// `replay: (none)` and `unwitnessed: (none)` — two lines under every finding of every report,
/// and `unwitnessed` is *the reason there is no witness*, so it cannot coexist with one.
#[test]
fn a_finding_carries_only_the_fields_it_can_have() {
    let v = find_bugs("shape.c", NULL_DEREF);
    let findings = v["result"]["findings"].as_array().expect("findings");
    assert!(!findings.is_empty(), "nothing to check:\n{v:#}");
    for f in findings {
        if f["witness"].is_array() {
            assert!(
                f.get("unwitnessed").is_none(),
                "a finding with a witness carries `unwitnessed`, which is the reason there is \
                 no witness:\n{f:#}"
            );
        } else {
            // 023 contract 15: a witness, or the reason there is none. Never neither.
            assert!(
                f.get("unwitnessed").is_some(),
                "a finding with no witness must say why:\n{f:#}"
            );
        }
        assert!(
            f.get("replay").is_none_or(|r| !r.is_null()),
            "`replay` is null on every finding when no harness was emitted, and the envelope \
             already carries that as a blind spot:\n{f:#}"
        );
    }
}

/// The third site of the same shape: `prove-equivalent`'s `differs` verdict printed
/// `replay: (none)` immediately above a blind spot saying, in words, that no harness was
/// compiled. `Replay` has no constructor yet, so the key was null on *every* answer this
/// operation has ever given.
#[test]
fn a_differs_verdict_does_not_print_a_key_that_is_always_empty() {
    let d = scratch();
    let (a, b) = (d.join("abs1.c"), d.join("abs2.c"));
    std::fs::write(&a, "int probe (int x) { return x < 0 ? -x : x; }\n").expect("write");
    std::fs::write(&b, "int probe (int x) { return x & 0x7fffffff; }\n").expect("write");
    let out = Command::new(bin())
        .args([
            "prove-equivalent",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--entry",
            "probe",
            "--json",
            "--no-system-headers",
        ])
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"));
    // The corpus in `injected_rewrites.rs` pins that this pair differs; here only the shape.
    if v["result"]["verdict"] == "differs" {
        assert!(
            v["result"].get("replay").is_none_or(|r| !r.is_null()),
            "`replay` is null on every `differs` this operation can produce, and the blind \
             spot below it already says so:\n{v:#}"
        );
    }
}
