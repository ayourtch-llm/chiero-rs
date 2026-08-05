//! **030 contract 5: the gate that makes the native decoder trustworthy.**
//!
//! > for every file in `tests/corpus/coverage/`, native `.gcno`/`.gcda` decoding produces line
//! > counts identical to `gcov --json-format` on the same files.
//!
//! Ground truth is produced by the same gcc that wrote the binary artifacts, so this compares two
//! readings of one fact rather than a reading against an opinion. It is the only check that can
//! catch a flow solve that is *plausible* — every count a small non-negative number, conservation
//! satisfied, and still wrong.
//!
//! # Why the whole directory
//!
//! `t` cannot distinguish the three candidate line rules: its blocks all have count 1, so sum,
//! max and first-block agree. `loop` decides it — five blocks on one line with counts
//! `[1, 4, 5, 1, 1]`, reported as 5. A gate that ran only on `t` would pass a decoder that adds.

use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

/// Every object stem with both a `.gcno`/`.gcda` pair and a `gcov --json-format` document.
fn stems() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(corpus())
        .expect("the corpus directory exists")
        .filter_map(|e| {
            let p = e.ok()?.path();
            let stem = p.file_name()?.to_str()?.strip_suffix(".gcov.json.gz")?;
            corpus()
                .join(format!("{stem}.gcno"))
                .exists()
                .then(|| stem.to_string())
        })
        .collect();
    v.sort();
    assert!(
        v.len() >= 2,
        "the gate needs both fixtures; `t` alone cannot tell three line rules apart: {v:?}"
    );
    v
}

/// **Contract 5.** Identical line counts, file by file and line by line.
#[test]
fn native_decoding_agrees_with_gcov_json() {
    for stem in stems() {
        let json = chiero_gcov::ingest_json(&corpus(), &stem)
            .unwrap_or_else(|e| panic!("{stem}: json ingest: {e}"));
        let native = chiero_gcov::ingest_native(&corpus(), &stem)
            .unwrap_or_else(|e| panic!("{stem}: native ingest: {e}"));

        let mut files: Vec<&str> = json.files().collect();
        files.sort();
        let mut native_files: Vec<&str> = native.files().collect();
        native_files.sort();
        assert_eq!(
            native_files, files,
            "{stem}: the two paths must see the same files"
        );

        for file in files {
            let want: Vec<(u32, Option<u64>)> = json
                .lines_of(file)
                .into_iter()
                .map(|l| (l, json.line_count(file, l)))
                .collect();
            let got: Vec<(u32, Option<u64>)> = native
                .lines_of(file)
                .into_iter()
                .map(|l| (l, native.line_count(file, l)))
                .collect();
            assert_eq!(
                got, want,
                "{stem} / {file}: the native decode and `gcov --json-format` disagree, and gcov \
                 is the compiler that wrote the file"
            );
        }
    }
}

/// The macro-attribution fact holds through the native path too — it is a property of what gcc
/// records, not of which reader is used, and a decoder that invented an entry for `m.h:1` would
/// be *more* informative than gcov and therefore wrong.
#[test]
fn the_native_path_also_sees_no_macro_line() {
    let native = chiero_gcov::ingest_native(&corpus(), "t").expect("t decodes");
    assert_eq!(native.line_count("m.h", 1), None);
    assert_eq!(native.lines_of("m.h"), vec![2]);
}

/// **Contract 6.** A truncated `.gcda` produces a diagnostic and **no** partial index: a
/// half-read counter stream yields counts that satisfy nothing and look like data.
#[test]
fn a_truncated_gcda_produces_no_partial_index() {
    let dir = std::env::temp_dir().join(format!("chiero-gcov-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::copy(corpus().join("t.gcno"), dir.join("t.gcno")).expect("copy notes");
    let data = std::fs::read(corpus().join("t.gcda")).expect("read data");
    // Through the first function's counters and into the second's header.
    std::fs::write(dir.join("t.gcda"), &data[..data.len() - 12]).expect("write");

    let err = chiero_gcov::ingest_native(&dir, "t").expect_err("a half counter stream is not data");
    let msg = err.to_string();
    assert!(
        msg.contains("truncated") || msg.contains("counter"),
        "the message must say what ran out: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A function that never ran** — the case a real build is full of and no earlier fixture had.
///
/// gcc compresses an all-zero counter set to a **negative length with no payload**: `unrun.gcda`
/// holds `16`, `8` and `-16`, and the last belongs to `never_called`. Read as a `u32` that length
/// is 4294967280, and the decoder refused the file — which is how 83 of 98 objects in a
/// `--coverage` build of `vppinfra` failed to ingest at all.
///
/// The counts must come out as gcov's: line 1 recorded **as zero**, not absent, because gcov saw
/// the line and nothing executed it. That is the distinction this whole crate turns on, arriving
/// this time from the format rather than from the API.
#[test]
fn a_function_that_never_ran_decodes_as_zeros() {
    let json = chiero_gcov::ingest_json(&corpus(), "unrun").expect("json");
    let native = chiero_gcov::ingest_native(&corpus(), "unrun").expect("native");
    assert_eq!(json.line_count("unrun.c", 1), Some(0), "gcov: seen, never executed");
    assert_eq!(native.line_count("unrun.c", 1), Some(0));
    assert_eq!(native.line_count("unrun.c", 2), Some(1));
    assert_eq!(native.line_count("unrun.c", 3), Some(1));
}

/// **Contract 4's other half.** The native path recovers arcs, so it says so — and that is what
/// makes `tests_for_arc` available on this index and unavailable on a JSON one.
#[test]
fn native_ingest_records_arc_detail() {
    let native = chiero_gcov::ingest_native(&corpus(), "t").expect("t decodes");
    assert_eq!(native.detail(), chiero_gcov::CoverageDetail::LinesAndArcs);
}
