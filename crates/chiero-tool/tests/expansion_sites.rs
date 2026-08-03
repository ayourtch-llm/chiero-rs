//! Covers: 050 contract 7.

use chiero_pp::{Config, preprocess_str};

/// A TU expanding `M` at `n` distinct sites, one per line.
fn many_sites(n: usize) -> String {
    let mut s = String::from("#define M(x) ((x)+1)\n");
    for i in 0..n {
        s.push_str(&format!("int v{i} = M({i});\n"));
    }
    s
}

/// 050 contract 7: a summary carrying `total` and `shown`, plus a cursor that pages through
/// **exactly** the sites, with no duplicates and none missed.
///
/// The contract's number is 1043 — VPP scale. The property under test is the paging, not the
/// magnitude, so this uses 120 sites over a page of 50: enough for three pages, two of them
/// full and one short, which is where an off-by-one lives.
#[test]
fn paging_yields_every_site_exactly_once() {
    let n = 120;
    let tu = preprocess_str("t.c", &many_sites(n), Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);

    let first = chiero_tool::expansion_sites(&tu.source_map, "M", None, 50);
    assert_eq!(first.total, n, "total is the whole population, not the page");
    assert_eq!(first.shown, 50);
    assert_eq!(first.sites.len(), 50);

    let mut seen = Vec::new();
    seen.extend(first.sites.iter().cloned());
    let mut cursor = first.cursor;
    let mut pages = 1;
    while let Some(c) = cursor {
        let page = chiero_tool::expansion_sites(&tu.source_map, "M", Some(c), 50);
        assert_eq!(page.total, n, "total does not shrink as we page");
        seen.extend(page.sites.iter().cloned());
        cursor = page.cursor;
        pages += 1;
        assert!(pages <= 10, "cursor is not terminating");
    }
    assert_eq!(pages, 3, "120 over a page of 50 is 50 + 50 + 20");
    assert_eq!(seen.len(), n, "every site, exactly once");

    let mut distinct = seen.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), n, "no duplicates across pages");

    // Sites are the *written* positions, one per line of the fixture.
    assert_eq!(seen[0].line, 2);
    assert_eq!(seen[0].file, "t.c");
}

/// **A macro invoked from inside another macro's body is still a site of the inner macro**,
/// resolved to the line the user wrote. This is the case that matters for VPP: `vec_len` is
/// barely written by hand, and every site that counts is reached through `vec_end` or
/// `vec_foreach`. Counting only hand-written calls would report almost none of them.
#[test]
fn a_site_reached_through_another_macro_resolves_to_the_written_line() {
    let tu = preprocess_str(
        "t.c",
        "#define INNER(x) ((x)+1)\n#define OUTER(x) INNER(x)\nint a = OUTER(1);\nint b = INNER(2);\n",
        Config::default(),
    );
    let s = chiero_tool::expansion_sites(&tu.source_map, "INNER", None, 50);
    assert_eq!(s.total, 2, "the nested one counts");
    assert_eq!(s.sites.iter().map(|x| x.line).collect::<Vec<_>>(), [3, 4]);
    // The cursor is `None` when a single page holds everything — an empty final page would
    // read as "more to come" and make a caller loop once for nothing.
    assert!(s.cursor.is_none());
}

/// A macro that never expands, and one that was never defined, both answer an empty summary
/// rather than an error (050 §1: every operation is total).
#[test]
fn an_unexpanded_or_unknown_macro_is_an_empty_summary_not_a_refusal() {
    let tu = preprocess_str("t.c", "#define UNUSED 1\nint x = 2;\n", Config::default());
    let s = chiero_tool::expansion_sites(&tu.source_map, "UNUSED", None, 50);
    assert_eq!(s.total, 0);
    assert!(s.sites.is_empty() && s.cursor.is_none());
    let s = chiero_tool::expansion_sites(&tu.source_map, "NEVER_HEARD_OF_IT", None, 50);
    assert_eq!(s.total, 0);
}
