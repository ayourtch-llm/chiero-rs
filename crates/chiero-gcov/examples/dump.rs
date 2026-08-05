//! Print one object's line counts as `file<TAB>line<TAB>count`.
//!
//! ```text
//! cargo run --release -p chiero-gcov --example dump -- <dir> <stem>
//! ```
//!
//! The comparable form of an ingest, for oracles that are not `gcov --json-format` — which is
//! every clang build, since `llvm-cov gcov` emits only its `.gcov` text.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let idx = match chiero_gcov::ingest_native(std::path::Path::new(&a[0]), &a[1]) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("INGEST-FAILED {e}");
            std::process::exit(1);
        }
    };
    for file in idx.files() {
        for line in idx.lines_of(file) {
            if let Some(c) = idx.line_count(file, line) {
                println!("{file}\t{line}\t{c}");
            }
        }
    }
}
