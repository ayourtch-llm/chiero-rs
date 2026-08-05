//! Emit `(file, line, gcov_count, block_counts...)` for a whole coverage build.
//!
//! ```text
//! cargo run --release -p chiero-gcov --example linerule -- <build-dir>
//! ```
//!
//! Written to derive the line rule from data, which it could not do: the answer depends on the
//! *arcs*, and every column here is a block count. It kept its value as the thing that showed
//! each candidate formula was still wrong, and as the way to see one line's blocks at a glance
//! when a disagreement needs explaining.
/// `(file, line)` -> `[(block, index within that block's line list, block count)]`.
type BlocksPerLine = std::collections::BTreeMap<(String, u32), Vec<(u32, usize, u64)>>;

fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("build dir"));
    let mut stems: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        let Ok(es) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in es.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(s) = p
                .file_name()
                .and_then(|x| x.to_str())
                .and_then(|x| x.strip_suffix(".gcno"))
            else {
                continue;
            };
            if p.with_extension("gcda").exists() {
                stems.push((p.parent().unwrap().to_path_buf(), s.to_string()));
            }
        }
    }
    stems.sort();
    println!("file\tline\tgcov\tblocks");
    for (dir, stem) in &stems {
        let (Ok(notes), Ok(data), Ok(json)) = (
            chiero_gcov::native::read_notes(&dir.join(format!("{stem}.gcno"))),
            chiero_gcov::native::read_data(&dir.join(format!("{stem}.gcda"))),
            chiero_gcov::ingest_json(dir, stem),
        ) else {
            continue;
        };
        for f in &notes.functions {
            let Some(d) = data.functions.iter().find(|d| d.ident == f.ident) else {
                continue;
            };
            let Some(counts) = chiero_gcov::native::debug_block_counts(f, &d.counters) else {
                continue;
            };
            let mut per: BlocksPerLine = Default::default();
            for bl in &f.lines {
                for (i, &l) in bl.lines.iter().enumerate() {
                    per.entry((bl.file.clone(), l)).or_default().push((
                        bl.block,
                        i,
                        counts.get(bl.block as usize).copied().unwrap_or(0),
                    ));
                }
            }
            for ((file, line), bs) in per {
                let Some(want) = json.line_count(&file, line) else {
                    continue;
                };
                let cells: Vec<String> =
                    bs.iter().map(|(b, i, c)| format!("{b}:{i}:{c}")).collect();
                println!("{file}\t{line}\t{want}\t{}", cells.join(","));
            }
        }
    }
}
