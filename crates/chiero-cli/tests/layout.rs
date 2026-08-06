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

/// **A member with no name still occupies bytes** — and dropping it made the proposal claim
/// a 72-byte struct could be 8.
///
/// Reported from a real header: `chiero layout` on VPP's `fib_route_path_t` said
///
/// ```text
///   recoverable: 64
///   rationale: `fib_route_path_t_` is 72 bytes and would be 8 with its fields ordered by size
/// ```
///
/// The size and the alignment were right — gcc agrees, 72 and 8. What was wrong is the number
/// underneath: that struct is a 1-byte enum, a **56-byte anonymous union**, two `u8`s and a
/// 4-byte enum, and the anonymous union was not in the field list at all. `frontend::records`
/// built each field from `names.text(fl.name?)` inside a `filter_map`, so a member with no
/// name was silently skipped — the ideal layout was then computed over 7 bytes of fields, and
/// "would be 8" is what 7 bytes rounded up to the struct's alignment looks like.
///
/// **A number computed from part of a struct is not a smaller number, it is a wrong one.** The
/// reader is being told they can recover 64 bytes from a struct whose real floor is 64.
///
/// The fixture is the same shape, reduced: 1 + (7 padding) + 56 + 1 + (7 padding) = 72, and a
/// reorder that puts the union first gets to 58 → 64. Eight bytes, not sixty-four.
#[test]
fn an_anonymous_member_is_counted_in_the_padding_it_costs() {
    let p = write(
        "anon_union.c",
        "struct with_anon {\n\
         \x20 char tag;\n\
         \x20 union { long a; char b[56]; };   /* 56 bytes, and no name */\n\
         \x20 char w;\n\
         };\n\
         struct with_anon instance;\n",
    );
    let (code, v, err) = run(&["layout", p.to_str().expect("utf-8"), "--json"]);
    assert_eq!(code, 0, "{err}");
    let rec = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "with_anon")
        .expect("the record is analysed");
    assert_eq!(rec["size"], 72, "gcc says 72: {rec}");

    let props = rec["proposals"].as_array().expect("proposals");
    let Some(pad) = props.iter().find(|p| p["kind"] == "padding_waste") else {
        panic!("expected a padding proposal — there are 8 real bytes to recover: {rec}");
    };
    assert_eq!(
        pad["recoverable"], 8,
        "56 of those bytes are the union and no reorder removes them: {pad}"
    );
    assert!(
        pad["rationale"]
            .as_str()
            .is_some_and(|s| s.contains("would be 64")),
        "the floor is the union plus its two bytes, rounded to alignment: {pad}"
    );
}

/// **A bit-field cannot be described by (offset, size), and the answer to that is not to
/// pretend the struct is smaller.**
///
/// The same `filter_map` that dropped anonymous members drops bit-fields — deliberately, and
/// the comment says why: "a bit-field's extent is bits within a byte, which straddling and
/// padding do not describe". That is right for the *straddle* finding and wrong for the
/// padding sum, which then adds up a struct that is missing members.
///
/// So the padding proposal is withheld when the field list is known to be partial, and the
/// envelope says which records that happened to. `with_bits` below sums to 9 bytes of visible
/// fields against a real 16, and a proposal computed from that is arithmetic about a struct
/// nobody declared.
#[test]
fn a_record_with_a_bitfield_gets_no_padding_number_it_cannot_stand_behind() {
    let p = write(
        "bits.c",
        "struct with_bits {\n\
         \x20 char tag;\n\
         \x20 unsigned a : 3;\n\
         \x20 unsigned b : 5;\n\
         \x20 long big;\n\
         };\n\
         struct with_bits instance;\n",
    );
    let (code, v, err) = run(&["layout", p.to_str().expect("utf-8"), "--json"]);
    assert_eq!(code, 0, "{err}");
    let rec = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "with_bits")
        .expect("the record is analysed");
    assert!(
        rec["proposals"]
            .as_array()
            .expect("proposals")
            .iter()
            .all(|p| p["kind"] != "padding_waste"),
        "the field list is missing two members, so there is no honest padding number: {rec}"
    );
    // **Said, not swallowed.** A record chiero could not judge is not a record with nothing
    // to find, and the envelope is where that distinction lives.
    assert!(
        v["blind_spots"]
            .as_array()
            .expect("blind_spots")
            .iter()
            .any(|b| b.as_str().is_some_and(|s| s.contains("bit-field"))),
        "the envelope names what it could not judge: {v}"
    );
}
