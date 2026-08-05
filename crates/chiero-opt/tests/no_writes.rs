//! **041 contract 17 — `chiero-opt` never writes to a source file.**
//!
//! > 17. No API in `chiero-opt` writes to a source file (checked by absence of write calls,
//! >     and by a fixture asserting the crate exposes no patch operation).
//!
//! 041's first sentence is *"it never patches code"*, and §2 says *"detectors propose; they
//! never rewrite"*. That is the load-bearing promise of the whole crate: `prove_equivalent`
//! adjudicates a rewrite somebody else proposed, and an adjudicator that could also apply its
//! own verdict is a different and much more dangerous thing.
//!
//! Both halves of the contract, because each catches what the other misses. The scan catches
//! a write added anywhere in the crate; the API check catches one added behind a name that
//! sounds harmless.

use std::path::PathBuf;

fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&p).expect("read");
                out.push((p, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    assert!(!out.is_empty(), "the scan found no source, so it is broken");
    out
}

/// **Absence of write calls**, by name. Every way this crate could open a file for writing.
#[test]
fn the_crate_contains_no_call_that_could_write_a_file() {
    // Not `fs::` broadly: reading is fine, and a rule that banned it would be worked around
    // rather than kept.
    const FORBIDDEN: &[&str] = &[
        "fs::write",
        "fs::create_dir",
        "fs::remove_",
        "fs::rename",
        "fs::copy",
        "File::create",
        "OpenOptions",
        "Command::new",
        "tempfile",
    ];
    let mut hits = Vec::new();
    for (path, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            // A mention in prose is not a call. This crate's documentation is dense enough
            // that the distinction matters, and a test that fired on its own comments would
            // be turned off rather than obeyed.
            let code = line.split("//").next().unwrap_or("");
            for f in FORBIDDEN {
                if code.contains(f) {
                    hits.push(format!("{}:{} {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "041 §1: `chiero-opt` never patches code, and these could write:\n{}",
        hits.join("\n")
    );
}

/// **And no patch operation**, whatever it is called.
///
/// The scan above is about mechanism; this is about intent. A function named `apply`,
/// `patch`, `rewrite`, `fix` or `emit_*` in this crate would be the API through which a
/// proposal became an edit, and the contract is that no such API exists — not that it is
/// currently unimplemented.
#[test]
fn the_crate_exposes_no_operation_that_applies_a_rewrite() {
    const SUSPECT: &[&str] = &[
        "pub fn apply",
        "pub fn patch",
        "pub fn rewrite",
        "pub fn fix",
        "pub fn write",
        "pub fn emit",
    ];
    let mut hits = Vec::new();
    for (path, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            for f in SUSPECT {
                if line.trim_start().starts_with(f) {
                    hits.push(format!("{}:{} {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "041 §2: detectors propose, they never rewrite:\n{}",
        hits.join("\n")
    );
}

/// The passes **do** rewrite — a `Module`, in memory, which is the thing 020 §9 is about.
/// This test exists so the two are not confused: `run_default` mutating a module is not a
/// patch, and nothing above should be read as forbidding it.
#[test]
fn rewriting_a_module_in_memory_is_not_patching_code() {
    let mut m = chiero_cir::text::parse(
        "target x86_64-unknown-linux-gnu\n\nfunc @f() -> i32 {\nentry:\n  .line 1\n  ret 0i32\n}\n",
    )
    .expect("parses");
    chiero_opt::const_fold(&mut m);
    assert_eq!(m.funcs.len(), 1, "the module is still a module");
}
