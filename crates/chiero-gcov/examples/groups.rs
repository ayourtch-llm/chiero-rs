//! Which functions of one object claim a given `(file, line)`, and where each begins.
//!
//! ```text
//! cargo run --release -p chiero-gcov --example groups -- <dir> <stem> <file-suffix> <line>
//! ```
//!
//! gcov marks two non-artificial functions that share a `(source, start_line)` as a *group*
//! (`process_all_functions`), and a group's line counts go to a private per-function array that,
//! under `--json-format`, is never summed into the source's. This prints the raw material for
//! deciding whether a disagreement has that shape.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, stem, suffix, line) = (
        std::path::PathBuf::from(&a[0]),
        &a[1],
        &a[2],
        a[3].parse::<u32>().expect("line"),
    );
    let notes = chiero_gcov::native::read_notes(&dir.join(format!("{stem}.gcno"))).expect("gcno");
    let data = chiero_gcov::native::read_data(&dir.join(format!("{stem}.gcda"))).expect("gcda");
    for f in &notes.functions {
        let claims: Vec<&chiero_gcov::native::BlockLines> = f
            .lines
            .iter()
            .filter(|bl| bl.file.ends_with(suffix.as_str()) && bl.lines.contains(&line))
            .collect();
        if claims.is_empty() {
            continue;
        }
        let counts = data
            .functions
            .iter()
            .find(|d| d.ident == f.ident)
            .and_then(|d| chiero_gcov::native::debug_block_counts(f, &d.counters));
        let cells: Vec<String> = claims
            .iter()
            .map(|bl| match &counts {
                Some(c) => format!(
                    "b{}={}",
                    bl.block,
                    c.get(bl.block as usize).copied().unwrap_or(0)
                ),
                None => format!("b{}=?", bl.block),
            })
            .collect();
        for bl in &f.lines {
            if bl.file.ends_with(suffix.as_str()) || bl.lines.contains(&line) {
                println!(
                    "    group  block {:3}  {:20} {:?}",
                    bl.block,
                    bl.file.rsplit('/').next().unwrap_or(&bl.file),
                    bl.lines
                );
            }
        }
        println!(
            "{:40} artificial={} {}:{}  {}",
            f.name,
            f.artificial,
            f.source.rsplit('/').next().unwrap_or(&f.source),
            f.start_line,
            cells.join(" ")
        );
    }
}
