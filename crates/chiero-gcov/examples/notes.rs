//! Decode every `.gcno` of a build and report what refuses to read.
//!
//! ```text
//! cargo run --release -p chiero-gcov --example notes -- <build-dir>
//! ```
//!
//! **The `.gcno` is where the format complexity lives** — the version tag, the record stream, the
//! two layouts, the string encoding, the `FUNCTION` and `BLOCKS` records, the multi-file line
//! groups. The `.gcda` is a handful of counters by comparison. So the notes decoder can be
//! exercised across a *whole* tree with nothing but a build, where `scale` needs a build that has
//! also been run — and a full VPP build is 1873 objects against the ~100 that its unit tests
//! produce data for.
//!
//! This reports decode failures and a census of what was read, so that "it did not crash" is
//! backed by a number: a decoder that silently produced no functions would pass a
//! failure-count-only check.

fn main() {
    let root = match std::env::args().nth(1) {
        Some(r) => std::path::PathBuf::from(r),
        None => {
            eprintln!("usage: notes <build-dir>");
            std::process::exit(2);
        }
    };

    let mut found: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("gcno") {
                found.push(p);
            }
        }
    }
    found.sort();

    let (mut ok, mut failed) = (0usize, 0usize);
    let (mut functions, mut blocks, mut arcs, mut line_groups) = (0usize, 0usize, 0usize, 0usize);
    let mut versions: std::collections::BTreeMap<String, usize> = Default::default();
    let mut shapes: std::collections::BTreeMap<String, (usize, String)> = Default::default();

    for p in &found {
        match chiero_gcov::native::read_notes(p) {
            Ok(n) => {
                ok += 1;
                *versions.entry(n.header.version_tag()).or_insert(0) += 1;
                functions += n.functions.len();
                for f in &n.functions {
                    blocks += f.blocks as usize;
                    arcs += f.arcs.len();
                    line_groups += f.lines.len();
                }
            }
            Err(e) => {
                failed += 1;
                let text = format!("{e}");
                // One line per distinct shape: a gap in a header reaches every object that
                // includes it, and 400 identical sentences hide how many things are wrong.
                let shape: String = text
                    .split_whitespace()
                    .filter(|w| !w.contains('/') && !w.contains("0x"))
                    .take(10)
                    .collect::<Vec<_>>()
                    .join(" ");
                let slot = shapes.entry(shape).or_insert((0, text));
                slot.0 += 1;
            }
        }
    }

    println!("{} .gcno files", found.len());
    println!("decoded: {ok} ok, {failed} failed");
    for (tag, n) in &versions {
        println!("  version {tag}: {n}");
    }
    println!(
        "read:    {functions} functions, {blocks} blocks, {arcs} arcs, {line_groups} line groups"
    );
    for (n, example) in shapes.values() {
        println!("  {n:5} x {example}");
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
