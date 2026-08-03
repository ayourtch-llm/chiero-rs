//! The sweep tool's own tests, over a **synthetic** tree.
//!
//! The tool exists to run against an external checkout, so its tests must not: a test that needed
//! `/home/ubuntu/vpp` would make the suite non-hermetic, which is the one property the 28-file
//! vendored corpus was built to protect.

use std::path::{Path, PathBuf};
use xtask::sweep::{Bucket, Outcome, classify, translation_units};

fn d(s: &str) -> Outcome {
    Outcome::Diagnosed(s.to_owned())
}

/// **Every combination**, because the classification is the whole judgement of the tool and a
/// table with a hole in it would silently drop a bucket.
#[test]
fn the_pair_of_outcomes_decides_the_bucket() {
    // gcc accepted, chiero complained: the finding, and the reason the tool exists.
    assert_eq!(
        classify(&Outcome::Clean, &d("array length is negative")),
        Bucket::Finding
    );

    // gcc refused, chiero was silent: a missing rule.
    assert_eq!(
        classify(&d("size of array is negative"), &Outcome::Clean),
        Bucket::Miss
    );

    // **Agreement is two different facts and must not share a bucket.** Both clean means the
    // file was tested and chiero matched gcc. Both *diagnosed* means gcc refused the file too —
    // on a real tree that is the flags being wrong, not the code — so chiero refusing it is no
    // evidence of anything. Sweeping `vlib` produced 45 of the second kind and reported
    // "0 findings", which reads as success and meant nothing was tested at all.
    assert_eq!(classify(&Outcome::Clean, &Outcome::Clean), Bucket::Agree);
    assert_eq!(classify(&d("x"), &d("y")), Bucket::BothRefused);

    // **A tool gap is never a skip.** Whichever side could not run, the file is reported.
    assert_eq!(
        classify(&Outcome::NotRun("no gcc".into()), &Outcome::Clean),
        Bucket::ToolGap
    );
    assert_eq!(
        classify(&Outcome::Clean, &Outcome::NotRun("-m32".into())),
        Bucket::ToolGap
    );
    assert_eq!(
        classify(&Outcome::NotRun("a".into()), &d("b")),
        Bucket::ToolGap
    );
    assert_eq!(
        classify(&d("b"), &Outcome::NotRun("a".into())),
        Bucket::ToolGap
    );
    assert_eq!(
        classify(&Outcome::NotRun("a".into()), &Outcome::NotRun("b".into())),
        Bucket::ToolGap
    );
}

fn fixture_tree() -> PathBuf {
    let tmp = std::env::temp_dir().join("chiero-sweep-fixture");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("sub/deeper")).unwrap();
    std::fs::write(tmp.join("a.c"), "int a;\n").unwrap();
    std::fs::write(tmp.join("b.c"), "int b;\n").unwrap();
    std::fs::write(tmp.join("sub/c.c"), "int c;\n").unwrap();
    std::fs::write(tmp.join("sub/deeper/d.c"), "int d;\n").unwrap();
    // Not translation units: a header is swept through whatever includes it, and the rest is
    // not C at all.
    std::fs::write(tmp.join("h.h"), "int h;\n").unwrap();
    std::fs::write(tmp.join("sub/notes.md"), "text\n").unwrap();
    std::fs::write(tmp.join("Makefile"), "all:\n").unwrap();
    tmp
}

/// Finds every `.c` file, at any depth, and nothing else — **sorted**, so two runs of the sweep
/// over one tree produce the same report and can be diffed.
#[test]
fn the_walk_finds_translation_units_and_only_those() {
    let tree = fixture_tree();
    let found = translation_units(&tree).expect("walk");
    let rel: Vec<String> = found
        .iter()
        .map(|p| {
            p.strip_prefix(&tree)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(rel, vec!["a.c", "b.c", "sub/c.c", "sub/deeper/d.c"]);
}

/// **The walk is not vacuous.** Without this the previous test could pass forever by finding
/// nothing in a tree that had nothing — the same vacuity the dependency gate's workspace test
/// was once guilty of.
#[test]
fn the_walk_of_an_empty_tree_is_empty_and_of_a_missing_one_is_an_error() {
    let tmp = std::env::temp_dir().join("chiero-sweep-empty");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(translation_units(&tmp).expect("walk").is_empty());
    assert!(translation_units(Path::new("/definitely/not/here")).is_err());
}

/// **A finding without a location is not actionable** (023 §9: "a report a person cannot act on
/// is not a report"). The sweep kept only the diagnostic's text and dropped its span, so a
/// finding read `e.g. /path/to/dataplane_node.c` and left the reader to search a 62,000-line
/// preprocessed translation unit by hand. Every diagnostic already carries a span; this renders
/// it.
#[test]
fn a_rendered_diagnostic_carries_the_place_it_happened() {
    let mut map = chiero_span::SourceMap::new();
    let src = "int a;\nstruct S { int; };\n";
    let file = map.add_file("/vpp/src/t.c", src);
    let start = map.file(file).start_pos.0;

    // The span of `int;` on line 2 — column 12, 1-based.
    let off = src.find("int;").expect("fixture") as u32;
    let sp = chiero_span::Span::new(
        chiero_span::BytePos(start + off),
        chiero_span::BytePos(start + off + 4),
        chiero_span::ExpnCtx(0),
    );
    assert_eq!(
        xtask::sweep::describe(&map, sp, "a member declaration must declare a member"),
        "/vpp/src/t.c:2:12: a member declaration must declare a member"
    );

    // **A span with no source must not be given one.** `Span::DUMMY` is `BytePos(0)` and the
    // first file in the global space starts at 0, so the lookup *succeeds* and reports line 1
    // column 1 — a real-looking place that is a fiction. It has to be rejected by name, not by
    // trusting the lookup to fail.
    assert_eq!(
        xtask::sweep::describe(&map, chiero_span::Span::DUMMY, "no place"),
        "no place"
    );
}

/// **Locating a finding must not un-group the report.** The rows are grouped by message, and the
/// whole value of the summary is `29  parse: a member declaration must declare a member` over one
/// example path. Once each message carries its own `path:line:col:`, no two are equal and the
/// group of 29 becomes 29 groups of one — the report would grow longer and say strictly less.
/// So the grouping key is the *kind*, with the located text kept for the example.
#[test]
fn the_grouping_key_is_the_kind_not_the_place() {
    let k = xtask::sweep::kind;
    assert_eq!(
        k(
            "parse: /vpp/src/plugins/acl/dataplane_node.c:1024:12: a member declaration must declare a member"
        ),
        "parse: a member declaration must declare a member"
    );
    // Unlocated messages (a dummy span, a tool that could not run) pass through whole.
    assert_eq!(
        k("pp: cannot include acl.api_enum.h"),
        "pp: cannot include acl.api_enum.h"
    );
    // **gcc's own text is already `path:line:col:` and is grouped the same way** — otherwise the
    // BOTH REFUSED bucket, which is mostly one repeated flags mistake, never groups either.
    assert_eq!(
        k("/vpp/src/vnet/fib/x.c:35:1: error: redefinition of 'f'"),
        "error: redefinition of 'f'"
    );
    // A path containing a colon must not be mistaken for a location, and a bare `:` in prose
    // must not eat the message.
    assert_eq!(
        k("sema: note: this is prose: with colons"),
        "sema: note: this is prose: with colons"
    );
    // **Prose with three colons is the row that actually tests the shape check.** The two-colon
    // row above cannot: the scan needs a third colon to reach the digit comparison at all, so it
    // returns early, and a build with the check deleted passes that row happily. Mutation caught
    // exactly this — the guard was unverified. A version without it mangles the input below into
    // `sema: z`.
    assert_eq!(k("sema: note: x: y: z"), "sema: note: x: y: z");
}

/// 080 M3 exit: **the VPP parser-coverage percentage is published and tracked.**
///
/// The sweep already says how many files gcc accepted. It never said how far *chiero* got,
/// which is the number the milestone asks for — and the two are different questions: a file
/// gcc refused tells us nothing, while a file chiero preprocessed but could not parse is a
/// parser gap and belongs in the denominator of a parser metric.
#[test]
fn coverage_counts_how_far_chiero_got_on_each_translation_unit() {
    use xtask::sweep::{Verdict, coverage};
    let v = |chiero: Outcome, gcc: Outcome| Verdict {
        path: PathBuf::from("t.c"),
        bucket: classify(&gcc, &chiero),
        gcc,
        chiero,
    };
    let verdicts = vec![
        // Reached sema and was clean: full coverage.
        v(Outcome::Clean, Outcome::Clean),
        // Preprocessed and parsed; sema complained. The parser handled it.
        v(d("sema: struct has no members"), Outcome::Clean),
        // Preprocessed; the parser did not. This is the parser gap the metric exists for.
        v(d("parse: expected `)`"), Outcome::Clean),
        // Never got past the preprocessor, so the parser was never asked.
        v(d("pp: cannot include foo.h"), Outcome::Clean),
        // **A second preprocessor failure, so the fixture is asymmetric.** With one `pp:` row
        // and one `parse:` row, swapping the two stages preserves every total and the test
        // cannot tell the stages apart at all — mutation demonstrated exactly that.
        v(d("pp: unterminated conditional"), Outcome::Clean),
        // An outcome with none of this module's prefixes: an unreadable file. The parser was
        // never handed it, so it must count toward no stage — charging it to sema would
        // silently inflate every figure in the report.
        v(
            Outcome::NotRun("unreadable: permission denied".into()),
            Outcome::Clean,
        ),
    ];

    let c = coverage(&verdicts);
    assert_eq!(c.total, 6);
    assert_eq!(c.preprocessed, 3, "the three that reached the parser");
    assert_eq!(c.parsed, 2, "the two that got past the parser");
    assert_eq!(c.analysed, 1);

    // **Parser coverage is out of what the parser was actually handed.** Dividing by `total`
    // would blame the parser for headers the sweep could not resolve, and the number would
    // move whenever the include flags changed rather than when the parser did.
    assert_eq!(c.parser_percent(), 66.7);

    // A sweep that ran nothing reports 0, not a division by zero.
    assert_eq!(coverage(&[]).parser_percent(), 0.0);
}

/// 042 contract 7: the sweep must be able to hand tier 1 the functions a translation unit
/// **defines**.
#[test]
fn function_extraction_finds_definitions_and_not_declarations() {
    use xtask::sweep::functions_in;
    let src = "int defined_here(void) { return 0; }\n\
               extern int only_declared(int);\n\
               static int static_definition(void) { return 1; }\n\
               int a_variable;\n\
               typedef int not_a_function;\n";
    let fns = functions_in(Path::new("src/vnet/x.c"), src).expect("parses");

    // **Definitions only.** A prototype has no body to analyse, and counting it as a candidate
    // would inflate every recipe's tally with functions tier 2 could never examine — the same
    // dishonesty as counting an undecidable function as matched.
    assert_eq!(
        fns.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["defined_here", "static_definition"]
    );

    // The path travels with the function: `in_file` selects on it, and a candidate closure
    // crosses translation units so a bare name would be ambiguous.
    assert!(fns.iter().all(|f| f.file == "src/vnet/x.c"));
}

/// 042 contract 7: a tier-1 sweep reports candidate counts per recipe **and what it could not
/// read**.
#[test]
fn a_tier_one_sweep_reports_counts_and_the_files_it_could_not_read() {
    use xtask::sweep::tier1_sweep;
    let tmp = std::env::temp_dir().join("chiero-tier1-fixture");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("a.c"), "int show_a(void){return 0;}\n").unwrap();
    std::fs::write(
        tmp.join("b.c"),
        "int show_b(void){return 0;}\nint helper(void){return 1;}\n",
    )
    .unwrap();
    // Does not parse. It must be *counted*, not quietly contribute zero functions.
    std::fs::write(tmp.join("broken.c"), "int oops(void) { return \n").unwrap();

    let recipe = chiero_recipe::load(
        "recipe shows {\n  title \"t\"\n  scope fn $f where name matches \"^show_\"\n  \
         fixture good \"g.c\"\n  fixture bad \"b.c\" expect 1 at \"b.c:1\"\n}\n",
    )
    .expect("loads");

    let r = tier1_sweep(&translation_units(&tmp).expect("walk"), &[recipe]);

    assert_eq!(r.files, 3);
    assert_eq!(r.functions, 3, "two in b.c, one in a.c; none from broken.c");
    assert_eq!(r.tallies[0].matched, 2, "show_a and show_b");

    // **A file the sweep could not read makes the counts partial.** Reporting `2 candidates`
    // over a tree where a file never parsed states a number the sweep did not earn — the same
    // failure as a recipe that could not be evaluated reporting zero.
    assert_eq!(r.unreadable, 1);
    assert!(
        !r.is_complete(),
        "an unread file must forbid claiming a complete count"
    );
}
