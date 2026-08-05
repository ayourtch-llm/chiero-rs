//! Point the decoder at a real `--coverage` build and compare it with gcov, object by object.
//!
//! ```text
//! cargo run --release -p chiero-gcov --example scale -- <build-dir>
//! ```
//!
//! An **example rather than a test**, because it needs a coverage build of a tree this repo does
//! not own. The four committed fixtures are what CI checks; this is what says whether those four
//! were representative — and the frontend's history says that is a different question. VPP went
//! from "the corpus fixtures pass" to 1852 of 1871 translation units failing the moment a real
//! tree was pointed at it.
//!
//! For each object with both a `.gcno` and a `.gcda` it decodes natively and, when a
//! `<stem>.gcov.json.gz` sits beside them, compares every line count against gcov's own — the
//! same gate contract 5 applies to the fixtures, at whatever scale the build offers.

fn main() {
    let root = match std::env::args().nth(1) {
        Some(r) => std::path::PathBuf::from(r),
        None => {
            eprintln!("usage: scale <build-dir>");
            std::process::exit(2);
        }
    };

    let mut stems: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(stem) = p
                .file_name()
                .and_then(|x| x.to_str())
                .and_then(|x| x.strip_suffix(".gcno"))
            else {
                continue;
            };
            if p.with_extension("gcda").exists() {
                stems.push((p.parent().unwrap().to_path_buf(), stem.to_string()));
            }
        }
    }
    stems.sort();
    println!("{} objects with both a .gcno and a .gcda", stems.len());

    let mut idx = chiero_gcov::CoverageIndex::default();
    let (mut ok, mut failed) = (0usize, 0usize);
    let mut errors: Vec<String> = Vec::new();
    // Cross-validation, where gcov's own answer is available beside the artifacts.
    let (mut checked, mut agreed) = (0usize, 0usize);
    // Object-level agreement is all-or-nothing, so it hides progress: one wrong line in a
    // 900-line object scores the same as a decoder that gets nothing right.
    let (mut rows, mut wrong_rows) = (0usize, 0usize);
    let mut disagreements: Vec<String> = Vec::new();

    for (dir, stem) in &stems {
        match chiero_gcov::ingest_native_as(&mut idx, chiero_gcov::TestId(0), dir, stem) {
            Ok(()) => ok += 1,
            Err(e) => {
                failed += 1;
                errors.push(format!("{stem}: {e}"));
                continue;
            }
        }
        if !dir.join(format!("{stem}.gcov.json.gz")).exists() {
            continue;
        }
        let (Ok(json), Ok(native)) = (
            chiero_gcov::ingest_json(dir, stem),
            chiero_gcov::ingest_native(dir, stem),
        ) else {
            continue;
        };
        checked += 1;
        let mut same = true;
        for file in json.files() {
            for line in json.lines_of(file) {
                rows += 1;
                if native.line_count(file, line) != json.line_count(file, line) {
                    same = false;
                    wrong_rows += 1;
                    if disagreements.len() < 20 {
                        disagreements.push(format!(
                            "{stem} {file}:{line} native={:?} gcov={:?}",
                            native.line_count(file, line),
                            json.line_count(file, line)
                        ));
                    }
                }
            }
        }
        if same {
            agreed += 1;
        }
    }

    let files: Vec<String> = idx.files().map(|f| f.to_string()).collect();
    let lines: usize = files.iter().map(|f| idx.lines_of(f).len()).sum();
    println!("ingest:  {ok} ok, {failed} failed");
    println!("index:   {} files, {lines} lines", files.len());
    println!("cross-validated: {agreed}/{checked} objects agree with gcov");
    println!("                 {} of {rows} lines differ", wrong_rows);

    // **The failures, grouped.** One line per distinct shape rather than per object: a decoder
    // gap in a header reaches every object that includes it, and a list of 400 identical
    // sentences hides how many *different* things are wrong.
    let mut shapes: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for e in &errors {
        let shape: String = e
            .split_whitespace()
            .filter(|w| !w.contains('/') && !w.contains("0x"))
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");
        let slot = shapes.entry(shape).or_insert((0, e.clone()));
        slot.0 += 1;
    }
    for (n, example) in shapes.values() {
        println!("  {n:5} x {example}");
    }
    for d in disagreements.iter().take(10) {
        println!("  DISAGREE {d}");
    }
}
