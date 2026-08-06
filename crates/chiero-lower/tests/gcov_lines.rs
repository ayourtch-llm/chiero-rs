//! Covers: 015 contracts 15, 15b, 16, 17.
//!
//! **This is the join point of the entire differentiating claim** (§4.1 → 030 → 031 →
//! 032), and 015 §5 says why it lives in lowering: this is the only stage that has both
//! the AST spans and the CIR block structure.
//!
//! Contract 17 alone is not testable. Its property is that a function's lines are a
//! **subset** of the lines gcov reports — and the empty set satisfies that for every
//! function. 015 contract 15b exists precisely because an earlier drop rule emptied
//! `gcov_lines` for every `static inline` in a header, which is VPP's entire hot layer,
//! and contract 17 would have reported success throughout. So the two are written
//! together here and the subset check carries a non-emptiness requirement.
//!
//! The oracle is `gcov --json-format` over the same fixture: chiero must not claim a line
//! gcov does not attribute there, and must not go silent on one it does.

use chiero_span::Span;

mod harness;
use harness::lower_file;

/// Compile `path` with coverage, run it, and return the lines gcov attributes to each
/// file, as `(file basename, sorted lines with a nonzero-or-zero counter)`.
///
/// Lines gcov *records* — not lines it counts as executed. Contract 17 is about which
/// lines exist as counters at all, since a line with a zero count is still a line the
/// selection story can key on.
fn gcov_lines_for(dir: &std::path::Path, main_c: &str) -> Option<Vec<(String, Vec<u32>)>> {
    // **Compile and link separately.** `gcc -o cov both.c --coverage` names the notes
    // file after the *output* — `cov-both.gcno` — and gcov then cannot find it from the
    // source name. Compiling to `both.o` first gives `both.gcno`, which is what
    // `gcov both.c` looks for.
    let obj = "both.o";
    let compile = std::process::Command::new("gcc")
        .args([
            "-std=gnu11",
            "-w",
            "-O0",
            "--coverage",
            "-c",
            "-o",
            obj,
            main_c,
        ])
        .current_dir(dir)
        .output()
        .ok()?;
    assert!(
        compile.status.success(),
        "gcc rejected the coverage fixture:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let link = std::process::Command::new("gcc")
        .args(["--coverage", "-o", "cov", obj])
        .current_dir(dir)
        .output()
        .ok()?;
    assert!(
        link.status.success(),
        "linking the coverage fixture failed:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = std::process::Command::new(dir.join("cov"))
        .current_dir(dir)
        .output()
        .ok()?;
    assert!(run.status.success(), "the fixture must run");

    let g = std::process::Command::new("gcov")
        .args(["--json-format", "--stdout", main_c])
        .current_dir(dir)
        .output()
        .ok()?;
    // **A failure here panics rather than returning `None`.** Two of these tests wrapped
    // the gcov half in `if let Some(...)`, so a gcov invocation that failed skipped the
    // comparison and the test reported success having checked only chiero against itself.
    // That is the vacuity this project has had to fix six times; an oracle that can
    // quietly not run is not an oracle.
    assert!(
        g.status.success(),
        "gcov failed on {main_c}:\n{}\n{}",
        String::from_utf8_lossy(&g.stdout),
        String::from_utf8_lossy(&g.stderr)
    );
    let text = String::from_utf8_lossy(&g.stdout);
    // The JSON is one object; rather than pulling in a parser, the fields needed here are
    // `"file": "name"` and `"line_number": N`, which appear in a fixed nesting order.
    let mut out: Vec<(String, Vec<u32>)> = Vec::new();
    let mut cur: Option<String> = None;
    let mut i = 0usize;
    let bytes: Vec<&str> = text.split('"').collect();
    while i + 2 < bytes.len() {
        if bytes[i] == "file" {
            let name = bytes[i + 2].to_string();
            let base = name.rsplit('/').next().unwrap_or(&name).to_string();
            cur = Some(base.clone());
            if !out.iter().any(|(f, _)| *f == base) {
                out.push((base, Vec::new()));
            }
        }
        if bytes[i] == "line_number" {
            // `"line_number":N` — the number follows the closing quote, before the next
            // comma or brace.
            let tail = bytes[i + 1];
            let digits: String = tail
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let (Some(f), Ok(n)) = (cur.as_ref(), digits.parse::<u32>())
                && let Some(entry) = out.iter_mut().find(|(name, _)| name == f)
                && !entry.1.contains(&n)
            {
                entry.1.push(n);
            }
        }
        i += 1;
    }
    for (_, lines) in &mut out {
        lines.sort_unstable();
    }
    Some(out)
}

fn gcov_available() -> bool {
    std::process::Command::new("gcov")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("chiero-gcov-{}-{n}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// **Contract 15.** A block whose instructions all come from a macro body carries the
/// **expansion-site** line in the `.c` file, and not the macro's own line in the header.
///
/// This is §4.1's whole claim in one assertion. `expansion_loc`, never `spelling_loc`:
/// gcov records the line where the macro was *used*, so header lines would appear in no
/// coverage file and match nothing at all.
#[test]
fn a_macro_bodys_block_is_attributed_to_the_expansion_site() {
    if !gcov_available() {
        eprintln!("skipping: gcov not found (015 contract 15)");
        return;
    }
    let dir = tmpdir("macro");
    std::fs::write(
        dir.join("m.h"),
        "#define BUMP(x) do { (x) = (x) + 1; } while (0)\n",
    )
    .unwrap();
    // `BUMP(v)` is used on line 5 of `m.c`; the macro body lives on line 1 of `m.h`.
    let src = "#include \"m.h\"\n\
               int probe(int n) {\n\
               int v = n;\n\
               if (v) {\n\
               BUMP(v);\n\
               }\n\
               return v;\n\
               }\n";
    std::fs::write(dir.join("m.c"), src).unwrap();
    // **One TU.** gcov attributes lines per compiled file, so the fixture and its `main`
    // have to be in the same compilation — a separate `main.c` declaring `probe` links
    // against nothing and gcc rejects it.
    std::fs::write(
        dir.join("both.c"),
        "#include \"m.c\"\nint main(void){ return probe(1) == 2 ? 0 : 1; }\n",
    )
    .unwrap();

    let (m, map) = lower_file(&dir.join("m.c"), std::slice::from_ref(&dir));
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");

    // Every line chiero attributes, with the file each resolves to.
    let mut attributed: Vec<(String, u32)> = Vec::new();
    for b in &f.blocks {
        for line in &b.gcov_lines {
            attributed.push(("m.c".into(), *line));
        }
    }
    assert!(
        !attributed.is_empty(),
        "the function has lines at all: {:#?}",
        f.blocks
            .iter()
            .map(|b| (b.id, b.gcov_lines.to_vec()))
            .collect::<Vec<_>>()
    );
    let lines: Vec<u32> = attributed.iter().map(|(_, l)| *l).collect();
    assert!(
        lines.contains(&5),
        "the `BUMP(v)` expansion site is line 5 of `m.c`: {lines:?}"
    );
    assert!(
        !lines.contains(&1),
        "and line 1 — the macro *body* in `m.h` — is not claimed for `m.c`: {lines:?}"
    );

    // Against gcov itself.
    let gcov = gcov_lines_for(&dir, "both.c");
    if let Some(gcov) = gcov
        && let Some((_, mc)) = gcov.iter().find(|(f, _)| f == "m.c").cloned()
    {
        {
            let mc = &mc;
            assert!(mc.contains(&5), "gcov also records line 5 of `m.c`: {mc:?}");
            for l in &lines {
                assert!(
                    mc.contains(l),
                    "chiero claims `m.c:{l}`, which gcov does not attribute there: \
                     gcov has {mc:?}"
                );
            }
        }
    }
    let _ = map;
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Contract 15b.** A `static inline` function defined in a **header** gets `gcov_lines`
/// in the header.
///
/// This is the case the earlier drop rule silently emptied, and it is most of vppinfra:
/// `vec.h`, `pool.h` and `buffer_funcs.h` are `static inline` functions, so a rule that
/// dropped header lines would return ∅ for exactly the code that matters most — while
/// contract 17's subset property reported success, because ∅ is a subset of everything.
#[test]
fn a_static_inline_in_a_header_keeps_its_header_lines() {
    if !gcov_available() {
        eprintln!("skipping: gcov not found (015 contract 15b)");
        return;
    }
    let dir = tmpdir("inline");
    std::fs::write(
        dir.join("h.h"),
        "static inline int addone(int x)\n{\n  int y = x + 1;\n  return y;\n}\n",
    )
    .unwrap();
    let src = "#include \"h.h\"\nint probe(int n) { return addone(n); }\n";
    std::fs::write(dir.join("h.c"), src).unwrap();
    std::fs::write(
        dir.join("both.c"),
        "#include \"h.c\"\nint main(void){ return probe(1) == 2 ? 0 : 1; }\n",
    )
    .unwrap();

    let (m, _) = lower_file(&dir.join("h.c"), std::slice::from_ref(&dir));
    let f = m
        .funcs
        .iter()
        .find(|f| &*f.name == "addone")
        .expect("`addone` is lowered — it is a definition, header or not");
    let lines: Vec<u32> = f
        .blocks
        .iter()
        .flat_map(|b| b.gcov_lines.to_vec())
        .collect();
    assert!(
        !lines.is_empty(),
        "a header function keeps its lines; emptying them is what makes coverage \
         correlation return nothing for all of vppinfra"
    );
    // 015 §5: **sorted ascending**. `simplify_cfg` unions these sets when it merges
    // blocks (020 §9), and a union of unsorted sets is a set nobody can compare against a
    // golden.
    for b in &f.blocks {
        let v = b.gcov_lines.to_vec();
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(v, sorted, "block {:?}'s lines are sorted ascending", b.id);
    }
    assert!(
        lines.iter().any(|&l| (2..=5).contains(&l)),
        "and they are the header's own lines, 2..5 of `h.h`: {lines:?}"
    );

    if let Some(gcov) = gcov_lines_for(&dir, "both.c") {
        let hh = gcov.iter().find(|(f, _)| f == "h.h");
        assert!(
            hh.is_some(),
            "gcov gives the header its own entry — that measurement is what 015 §5's \
             keep-header-lines rule rests on: {gcov:?}"
        );
        if let Some((_, hlines)) = hh {
            for l in &lines {
                assert!(
                    hlines.contains(l),
                    "chiero claims `h.h:{l}`, gcov has {hlines:?}"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Contract 16.** A block containing only lowering-generated instructions has an empty
/// `gcov_lines`, and every such instruction is **marked** compiler-generated.
///
/// 020 contract 15 requires the mark to be a recorded property of the instruction rather
/// than a guess — "this had no source span" is not the same fact, since a lowering bug
/// that lost a span would look identical.
#[test]
fn generated_only_blocks_have_no_lines_and_are_marked() {
    let dir = tmpdir("gen");
    // The `&&` join is a generated block; giving it a `goto` rather than the user's
    // `return` as a terminator is what makes it generated through and through.
    let src = "int probe(int a, int b)\n{\n  int c = a && b;\n  return c;\n}\n";
    std::fs::write(dir.join("g.c"), src).unwrap();
    let (m, _) = lower_file(&dir.join("g.c"), &[]);
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");

    let mut generated_only = 0usize;
    for b in &f.blocks {
        // **"All generated" is about the terminator too.** This used to look only at
        // `insts`, which was the same claim while terminators contributed nothing to
        // `gcov_lines`. They do now (a bare `return <constant>;` has no instructions and gcov
        // still counts its line), and the `&&` join block here ends in the *user's* `return`
        // — so it is not a block gcov has no counter for, and asserting it has no lines would
        // be asserting the old implementation rather than the property.
        let all_generated = !b.insts.is_empty()
            && b.insts.iter().all(|i| i.generated)
            && !matches!(b.term, chiero_cir::Terminator::Return(_));
        if all_generated {
            generated_only += 1;
            assert!(
                b.gcov_lines.is_empty(),
                "block {:?} is all lowering-generated, so gcov has no counter for it \
                 either: {:?}",
                b.id,
                b.gcov_lines
            );
        }
    }
    assert!(
        generated_only > 0,
        "the `&&` shape introduces at least one block that is generated through and \
         through, or this test asserts a property of no block at all: {:#?}",
        f.blocks
            .iter()
            .map(|b| (
                b.id,
                b.insts.iter().filter(|i| i.generated).count(),
                b.insts.len()
            ))
            .collect::<Vec<_>>()
    );
    // And the converse: a block with source instructions is not empty, so "generated"
    // is not simply stamped on everything.
    assert!(
        f.blocks.iter().any(|b| !b.gcov_lines.is_empty()),
        "some block has lines"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Contract 17.** Over a corpus, a function's lines are a **subset** of the lines gcov
/// reports for it, **and non-empty** for every function with a non-generated instruction.
///
/// Both halves, in one test, because either alone is passable: the subset property is
/// satisfied by the empty set, and non-emptiness is satisfied by claiming every line in
/// the file. Chiero must neither claim a line gcov does not attribute there nor go silent
/// on one it does.
#[test]
fn corpus_lines_are_a_nonempty_subset_of_gcovs() {
    if !gcov_available() {
        eprintln!("skipping: gcov not found (015 contract 17)");
        return;
    }
    let dir = tmpdir("corpus");
    let src = "int classify(int n)\n\
               {\n\
               int t = 0;\n\
               for (int i = 0; i < n; i++) {\n\
               if (i % 2) { t += i; } else { t -= i; }\n\
               }\n\
               switch (n) {\n\
               case 1: t += 10; break;\n\
               default: t -= 10;\n\
               }\n\
               return t;\n\
               }\n\
               int total(int n)\n\
               {\n\
               int s = 0;\n\
               while (n > 0) { s += classify(n); n--; }\n\
               return s;\n\
               }\n";
    std::fs::write(dir.join("c.c"), src).unwrap();
    std::fs::write(
        dir.join("both.c"),
        "#include \"c.c\"\nint main(void){ return total(4) == total(4) ? 0 : 1; }\n",
    )
    .unwrap();

    let (m, _) = lower_file(&dir.join("c.c"), &[]);
    let gcov = gcov_lines_for(&dir, "both.c").expect("gcov ran");
    // **The defining file's entry**, not the compiled TU's. 015 §5 keys `gcov_lines` on
    // the enclosing function's defining file, and gcov gives `c.c` its own entry because
    // `both.c` includes it — comparing against `both.c` compares against the two lines of
    // `main` and rejects everything.
    let reported: Vec<u32> = gcov
        .iter()
        .find(|(f, _)| f == "c.c")
        .map(|(_, l)| l.clone())
        .unwrap_or_default();
    assert!(
        !reported.is_empty(),
        "gcov reported no lines at all, so the subset check would be vacuous: {gcov:?}"
    );

    let mut checked = 0usize;
    for f in &m.funcs {
        if f.blocks.is_empty() {
            continue;
        }
        let lines: Vec<u32> = f
            .blocks
            .iter()
            .flat_map(|b| b.gcov_lines.to_vec())
            .collect();
        let has_source = f
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .any(|i| !i.generated && i.span != Span::DUMMY);
        if has_source {
            assert!(
                !lines.is_empty(),
                "`{}` has source instructions but no lines — the empty set satisfies the \
                 subset property and says nothing, which is what contract 15b exists to \
                 stop",
                f.name
            );
            checked += 1;
        }
        for l in &lines {
            assert!(
                reported.contains(l),
                "`{}` claims line {l}, which gcov does not attribute in this file: \
                 gcov has {reported:?}",
                f.name
            );
        }
    }
    assert!(checked >= 2, "both functions were checked, not one");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A block whose only source content is `return <constant>;` still has a line.**
///
/// 015 §5's rule is written over *instructions*: "for each `Inst` in the block, take its
/// `Span`". `return 1;` lowers to a `Terminator::Return` of a constant and **no instructions
/// at all**, so the block came out with an empty `gcov_lines` — while gcov records that line
/// and counts it.
///
/// The consequence is not cosmetic. `gcov_lines` is 015 §5's own "join point of the entire
/// differentiating claim (030 → 031 → 032)": a line gcov counts and chiero attributes to no
/// block is a line coverage correlation cannot reach, and `return <constant>;` is one of the
/// most common statements in C. Found by asking `chiero check-reachable` about a `return` line
/// and being told the function has no code there.
///
/// gcov is the oracle, as it is for the rest of this file — the claim is not "chiero should
/// pick this line" but "gcov counts it and chiero must agree".
#[test]
fn a_bare_return_of_a_constant_is_attributed_to_its_line() {
    let dir = tmpdir("bare-return");
    // Lines:            1                    2      3          4      5           6
    let src = "int probe (int v)\n{\n  if (v)\n    return 1;\n  return 2;\n}\n";
    std::fs::write(dir.join("m.c"), src).unwrap();
    std::fs::write(
        dir.join("both.c"),
        "#include \"m.c\"\nint main(void){ return probe(1) == 1 ? 0 : 1; }\n",
    )
    .unwrap();

    let (m, _map) = lower_file(&dir.join("m.c"), std::slice::from_ref(&dir));
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
    let mut lines: Vec<u32> = f
        .blocks
        .iter()
        .flat_map(|b| b.gcov_lines.to_vec())
        .collect();
    lines.sort_unstable();
    lines.dedup();

    for want in [4u32, 5] {
        assert!(
            lines.contains(&want),
            "line {want} is a `return <constant>;` and chiero attributes it to no block: \
             {lines:?}\n{:#?}",
            f.blocks
                .iter()
                .map(|b| (b.id, b.gcov_lines.to_vec(), format!("{:?}", b.term)))
                .collect::<Vec<_>>()
        );
    }

    // And gcov agrees those lines exist as counters.
    if let Some(gcov) = gcov_lines_for(&dir, "both.c")
        && let Some((_, mc)) = gcov.iter().find(|(f, _)| f == "m.c").cloned()
    {
        for want in [4u32, 5] {
            assert!(
                mc.contains(&want),
                "gcov records line {want} of `m.c`, so the premise holds: {mc:?}"
            );
        }
    }
}
