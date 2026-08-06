//! **`chiero layout` — 041 §3 with a caller.**
//!
//! The analysis existed and nothing could run it. These tests are against the command, because
//! the point of wiring it up is that somebody can point it at a header and get an answer.
//!
//! The two things worth checking from out here are §3's two constraints, since they are what
//! separates this from a tool that tells people to reorder wire formats.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-layout-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn write(name: &str, src: &str) -> PathBuf {
    let p = scratch().join(name);
    std::fs::write(&p, src).expect("write");
    p
}

fn run(args: &[&str]) -> (i32, serde_json::Value, String) {
    let o = Command::new(bin()).args(args).output().expect("spawn");
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    let v = serde_json::from_str(&out).unwrap_or(serde_json::Value::Null);
    (
        o.status.code().unwrap_or(-1),
        v,
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// `char a; long big; char b;` — 24 bytes that would be 16 reordered.
const PADDED: &str = "struct p { char a; long big; char b; };\nstruct p instance;\n";

#[test]
fn padding_a_reorder_would_recover_is_reported_with_the_delta() {
    let f = write("pad.c", PADDED);
    let (code, v, err) = run(&["layout", f.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "stderr:\n{err}");
    let records = v["result"]["records"].as_array().expect("records");
    let p = records
        .iter()
        .find(|r| r["tag"] == "p")
        .unwrap_or_else(|| panic!("struct p is missing: {v}"));
    let waste = p["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .find(|x| x["kind"] == "padding_waste")
        .unwrap_or_else(|| panic!("14 of these 24 bytes are padding: {p}"));
    assert_eq!(waste["recoverable"].as_u64(), Some(8));
}

/// **041 §3's first constraint, from the command line.** A `packed` struct is a wire format
/// until proven otherwise, and every proposal about one must say so.
#[test]
fn a_packed_struct_gets_only_advisory_proposals() {
    let f = write(
        "wire.c",
        "struct hdr { char pad[60]; long seq; } __attribute__((packed));\nstruct hdr h;\n",
    );
    let (code, v, err) = run(&["layout", f.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "stderr:\n{err}");
    let hdr = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "hdr")
        .unwrap_or_else(|| panic!("struct hdr is missing: {v}"));
    let proposals = hdr["proposals"].as_array().expect("proposals");
    assert!(
        !proposals.is_empty(),
        "`seq` at offset 60 spans a cache line: {hdr}"
    );
    for p in proposals {
        assert_eq!(
            p["advisory"].as_bool(),
            Some(true),
            "reordering a packed struct is a protocol change: {p}"
        );
        assert!(
            p["rationale"]
                .as_str()
                .is_some_and(|s| s.contains("observable")),
            "and it must say so in words: {p}"
        );
    }
}

/// **041 §3's second constraint.** No run, no number.
#[test]
fn every_benefit_is_unquantified_without_a_profile() {
    let f = write("pad2.c", PADDED);
    let (_, v, _) = run(&["layout", f.to_str().unwrap(), "--json"]);
    for r in v["result"]["records"].as_array().expect("records") {
        for p in r["proposals"].as_array().expect("proposals") {
            assert_eq!(
                p["benefit"].as_str(),
                Some("Unquantified"),
                "chiero has no cycle model and must not pretend to one: {p}"
            );
        }
    }
    assert!(
        v["blind_spots"]
            .as_array()
            .expect("blind_spots")
            .iter()
            .any(|b| b.as_str().is_some_and(|s| s.contains("no run"))),
        "and the envelope says why nothing is quantified: {v}"
    );
}

/// **The cache line is a parameter**, because 64 is a fact about a machine rather than about C.
///
/// **The fixture is `packed`, and it has to be.** A naturally-aligned scalar cannot straddle a
/// cache line whose size is a multiple of its alignment — an 8-byte `long` sits at a multiple
/// of 8, and 8 divides 32 and 64. So straddling is reachable only through `packed`, a
/// misaligned outer struct, or an array/aggregate member. That is not a limitation: it is
/// precisely VPP's wire formats and its `CLIB_CACHE_LINE_ALIGN_MARK` structs, which is what
/// §3 is about. Learned by writing this test with an unpacked struct and getting nothing.
#[test]
fn the_cache_line_size_changes_the_answer() {
    let f = write(
        "line.c",
        "struct s { char pad[28]; long v; } __attribute__((packed));\nstruct s x;\n",
    );
    // Packed, so `v` sits at 28: it crosses the boundary at 32 and not the one at 64.
    let (_, at32, _) = run(&[
        "layout",
        f.to_str().unwrap(),
        "--cache-line",
        "32",
        "--json",
    ]);
    let (_, at64, _) = run(&["layout", f.to_str().unwrap(), "--json"]);
    let straddles = |v: &serde_json::Value| {
        v["result"]["records"]
            .as_array()
            .map(|rs| {
                rs.iter().any(|r| {
                    r["proposals"]
                        .as_array()
                        .is_some_and(|ps| ps.iter().any(|p| p["kind"] == "line_straddle"))
                })
            })
            .unwrap_or(false)
    };
    assert!(straddles(&at32), "offset 28..36 crosses 32: {at32}");
    assert!(!straddles(&at64), "and does not cross 64: {at64}");
}

/// **A struct with nothing wrong yields nothing, and the answer is still proven.**
///
/// The layout came from 014, which is measured against gcc; this is one of the few places an
/// empty answer is complete rather than merely quiet.
#[test]
fn a_tight_struct_yields_no_proposals_and_that_is_an_answer() {
    let f = write("tight.c", "struct t { long x; long y; };\nstruct t z;\n");
    let (_, v, _) = run(&["layout", f.to_str().unwrap(), "--json"]);
    let t = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "t")
        .unwrap_or_else(|| panic!("struct t is missing: {v}"));
    assert_eq!(t["proposals"].as_array().map(Vec::len), Some(0));
    assert_eq!(v["proven"].as_bool(), Some(true), "{v}");
}
