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

/// The record tags the envelope says it could not judge — parsed out of the blind spot rather
/// than matched as a substring of it.
///
/// **A test that greps the blind spot for a word in its explanation asserts nothing**: the
/// explanation is prose that the next rewording changes, and two of these tests were greping
/// for "bit-field" *after* the sentence stopped containing it, so they could not fail. What
/// the contract is actually about is which records are named, so that is what this returns.
fn unjudged(v: &serde_json::Value) -> Vec<String> {
    v["blind_spots"]
        .as_array()
        .expect("blind_spots")
        .iter()
        .filter_map(|b| b.as_str())
        .find(|s| s.starts_with("no padding proposal was computed"))
        .and_then(|s| s.rsplit_once(": "))
        .map(|(_, tags)| tags.split(", ").map(str::to_string).collect())
        .unwrap_or_default()
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

/// **Contract 25 — a bit-field is described by its bits, and the struct around it is
/// measured.**
///
/// A bit-field cannot be described by `(offset, size)`, and for a while the answer was to drop
/// it and withhold the padding number for the whole record: honest, and it left out exactly
/// the packed, hand-tuned structs where padding matters most. 041 §3.1 gives the field
/// description a bit extent and models a run of adjacent bit-fields as one member.
///
/// `q` below is **12 bytes under gcc 13.3 and 8 with `int` first**, both compiled and checked
/// rather than reasoned about. The fixture discriminates: counting each bit-field as the byte
/// it starts in sums to 9, rounds to 12, and yields no proposal at all — so this assertion can
/// fail if the run is not treated as one member.
#[test]
fn a_bit_field_run_is_measured_rather_than_dropped() {
    let p = write(
        "bits.c",
        "struct q {\n\
         \x20 char tag;         /* 0, then 3 bytes of nothing */\n\
         \x20 int big;          /* 4 */\n\
         \x20 unsigned a : 1;   /* bit 64 — byte 8, and the next three share it */\n\
         \x20 unsigned b : 1;\n\
         \x20 unsigned c : 1;\n\
         \x20 unsigned d : 1;   /* then 3 more bytes to the end */\n\
         };\n\
         struct q instance;\n",
    );
    let (code, v, err) = run(&["layout", p.to_str().expect("utf-8"), "--json"]);
    assert_eq!(code, 0, "{err}");
    let rec = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "q")
        .expect("the record is analysed");
    assert_eq!(rec["size"], 12, "gcc says 12: {rec}");
    let pad = rec["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .find(|p| p["kind"] == "padding_waste")
        .unwrap_or_else(|| panic!("gcc says the reordered declaration is 8: {rec}"));
    assert_eq!(
        pad["recoverable"], 4,
        "12 with `char` first, 8 with `int` first — both compiled: {pad}"
    );
    let ev = pad["evidence"].to_string();
    assert!(
        ev.contains("bit-field"),
        "the hole after the run names it as a bit-field run, since moving one moves all \
         four: {pad}"
    );
    // **And the record is no longer one chiero declines to judge**, so nothing in the envelope
    // may say it was.
    assert!(
        !unjudged(&v).contains(&"q".to_string()),
        "a record that got a number is not a record that could not be judged: {v}"
    );
}

/// **Contract 25 — nothing to recover and nothing chiero could judge must stay
/// distinguishable.**
///
/// `with_bits` is 16 bytes and no order makes it smaller: the two bit-fields share the byte
/// after `tag`, and `long` needs its alignment. So there is no proposal — and no blind spot
/// either, because silence about a struct that was measured and silence about a struct that
/// was skipped mean opposite things to a reader.
#[test]
fn a_record_whose_bit_fields_already_pack_tight_is_silent_for_the_right_reason() {
    let p = write(
        // Not `tight.c`: the scratch directory is shared and another test owns that name.
        "tight_bits.c",
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
    assert_eq!(rec["size"], 16, "gcc says 16: {rec}");
    assert!(
        rec["proposals"]
            .as_array()
            .expect("proposals")
            .iter()
            .all(|p| p["kind"] != "padding_waste"),
        "8 + 1 + 1 rounds to 16, which is what it already is: {rec}"
    );
    assert!(
        !unjudged(&v).contains(&"with_bits".to_string()),
        "this record was judged and found tight, not skipped: {v}"
    );
}

/// **The same, from the command — because "recoverable: 8" is not advice until it says which
/// fields.**
///
/// Asked on VPP's `fib_route_path_t`, the proposal named a total and left the reader to find
/// the holes in a struct of five members, one of them a 56-byte anonymous union. The offsets
/// were in chiero's hands the whole time: they are the input to the number it already printed.
#[test]
fn the_padding_proposal_names_the_fields_the_holes_are_between() {
    let p = write(
        "where.c",
        "struct s {\n\
         \x20 char tag;      /* 0, then 7 bytes of nothing */\n\
         \x20 long big;      /* 8 */\n\
         \x20 char last;     /* 16, then 7 more to the end */\n\
         };\n\
         struct s instance;\n",
    );
    let (code, v, err) = run(&["layout", p.to_str().expect("utf-8"), "--json"]);
    assert_eq!(code, 0, "{err}");
    let rec = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "s")
        .expect("analysed");
    let pad = rec["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .find(|p| p["kind"] == "padding_waste")
        .expect("24 bytes that would be 16");
    let ev = pad["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .filter_map(|e| e.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        ev.contains("`tag`") && ev.contains("`big`") && ev.contains("`last`"),
        "every field a hole touches is named, so a reader can act without counting: {ev}"
    );
}

/// **A zero-width bit-field's gap is not padding, and proposing it as such was a proven wrong
/// answer.** Found by an adversarial review of the change that made §3.1's runs measurable.
///
/// ```c
/// struct Q { unsigned a:1; unsigned :0; char c; unsigned b:1; unsigned :0; char d; };
/// ```
///
/// gcc 13.3 says 12 bytes, and chiero agreed on the size — then said it "would be 4 with its
/// fields ordered by size", `proven: true`, no advisory. Brute-forcing all 24 orders that move
/// each run as a unit gives sizes in {8, 12}: **the floor is 8, and 4 is only reachable by
/// deleting the `:0`s**, which is the repacking §3.1 promises the reorder never does.
///
/// The cause is that a `:0` declares no member, so it is in no field list (014 §3, and C
/// 6.7.9 is why it cannot be — initializers skip unnamed bit-fields and that check indexes
/// the list positionally). Its effect survives only as a gap in its neighbours' offsets,
/// which reads exactly like alignment padding and is not: the boundary follows the run
/// wherever the run is moved.
///
/// So the record is one this analysis cannot state in full, and the answer is the one §7.7
/// settled for a partial field list — no number, and the envelope names the record. This
/// asserts both halves, because a silent skip and a measured tight struct are the pair
/// contract 25 exists to keep apart.
#[test]
fn a_zero_width_bit_field_makes_the_field_list_partial_rather_than_the_number_wrong() {
    let p = write(
        "zero_width.c",
        "struct Q {\n\
         \x20 unsigned a : 1;\n\
         \x20 unsigned   : 0;   /* forces `c` to byte 4, and no reorder recovers those 3 */\n\
         \x20 char c;\n\
         \x20 unsigned b : 1;\n\
         \x20 unsigned   : 0;\n\
         \x20 char d;\n\
         };\n\
         struct Q instance;\n",
    );
    let (code, v, err) = run(&["layout", p.to_str().expect("utf-8"), "--json"]);
    assert_eq!(code, 0, "{err}");
    let rec = v["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["tag"] == "Q")
        .expect("the record is analysed");
    assert_eq!(rec["size"], 12, "gcc says 12: {rec}");
    assert!(
        rec["proposals"]
            .as_array()
            .expect("proposals")
            .iter()
            .all(|p| p["kind"] != "padding_waste"),
        "gcc's floor for this struct is 8 and the sum of its visible members says 4: {rec}"
    );
    // **Said, not swallowed** — the direction the deleted bit-field test used to cover, and
    // the only end-to-end assertion that the envelope positively names a record it skipped.
    assert!(
        unjudged(&v).contains(&"Q".to_string()),
        "a record chiero could not judge is not a record with nothing to find: {v}"
    );
}

/// **A translation unit sema refused must not produce a layout report stamped `proven`.**
///
/// `chiero layout`'s frontend path never looked at `analysis.diagnostics` at all — not for
/// errors, not for advisories — so a TU containing an undeclared name still produced a padding
/// proposal marked **`proven — this holds for all inputs (Exact)`** and exited 0, with the
/// diagnostic never printed.
///
/// That contradicts the module's own header ("Every stage's diagnostics are a refusal") and,
/// since the severity work, the policy `lower()` implements ten lines above it. Pre-existing —
/// found by the adversarial review of that work, which is the more useful kind of finding: the
/// change did not cause it, it made the inconsistency untenable.
///
/// ⚠️ **`proven` is the word that makes this worse than a missing diagnostic.** 041 §3's
/// proposal is arithmetic over a record's members; a record whose type resolution failed can
/// still be laid out, and the arithmetic then holds for a struct the program does not have.
#[test]
fn layout_refuses_a_translation_unit_sema_could_not_analyse() {
    let dir = std::env::temp_dir().join(format!("chiero-layout-refuse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let run = |name: &str, src: &str| {
        let c = dir.join(name);
        std::fs::write(&c, src).expect("write");
        std::process::Command::new(env!("CARGO_BIN_EXE_chiero"))
            .arg("layout")
            .arg(&c)
            .output()
            .expect("run chiero")
    };

    let bad = run(
        "bad.c",
        "struct S { char a; int b; };\nint f(void) { return undeclared_name; }\n",
    );
    assert!(
        !bad.status.success(),
        "sema refused this TU; a layout report over it cannot be `proven`.\nstdout: {}",
        String::from_utf8_lossy(&bad.stdout)
    );

    // **The discriminator.** The same record without the error must still be reported, or the
    // fix could be "refuse everything".
    let good = run("good.c", "struct S { char a; int b; };\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        good.status.success(),
        "a clean TU must still produce a report: {}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert!(
        String::from_utf8_lossy(&good.stdout).contains('S'),
        "and the report must actually name the record: {}",
        String::from_utf8_lossy(&good.stdout)
    );
}

/// **The typedef's alignment reaches the CIR, not just sema's folder.**
///
/// ⚠️ This test exists because the sema-level fix passed its own unit test while `chiero cir`
/// went on emitting the old numbers. There were **three** readers of "what is this type's
/// alignment" and the fix reached one: `Cx::eval`'s `AlignofType` arm (fixed), lowering's
/// `AlignofType` arm (not, and it is the one the tool uses), and the typing pass that only
/// records a width.
///
/// Lowering asked `align_of` on the resolved `TyId` — where the typedef name is already gone —
/// and fell back to sema's fold only if that returned `None`, which it never does for a
/// complete type. So the fallback that would have been right was unreachable.
///
/// **Checking the original reproduction rather than the new test is what caught it**, and
/// pinning it here is what stops the next fix from reaching one reader again.
#[test]
fn a_typedefs_alignment_reaches_the_lowered_constant() {
    let dir = std::env::temp_dir().join(format!("chiero-tdalign-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let c = dir.join("td.c");
    std::fs::write(
        &c,
        "typedef __attribute__((aligned(16))) struct A { char a; } A_t;\n\
         typedef int I_t __attribute__((aligned(16)));\n\
         int probe(void) { return _Alignof(A_t) * 100 + _Alignof(I_t); }\n",
    )
    .expect("write");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_chiero"))
        .arg("cir")
        .arg(&c)
        .output()
        .expect("run chiero");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // gcc gives 16 for both, so the folded constants must be 16 — and 1 or 4 is the old wrong
    // answer for each, which is what makes the substring check meaningful rather than lucky.
    assert!(
        stdout.contains("16i64"),
        "the typedef's alignment must reach the CIR as 16; gcc says 16 for both:\n{stdout}"
    );
    assert!(
        !stdout.contains("mul i64 1i64") && !stdout.contains("mul i64 4i64"),
        "1 and 4 are the underlying alignments — the typedef's attribute was dropped:\n{stdout}"
    );
}
