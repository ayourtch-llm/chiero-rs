//! `#pragma GCC dependency "f"` — errors when the file is not found.
//!
//! ⚠️ **The request is recorded during expansion and answered in `finish`**, because `_Pragma` is
//! handled inside `expand_inner`, which has no `FileLoader` — and `#define DO_PRAGMA _Pragma` /
//! `DO_PRAGMA ("GCC dependency \"…\"")` reaches it only that way, which is exactly what the
//! corpus fixture tests. Recorded where it is seen, answered where it can be.
//!
//! gcc also compares timestamps and warns when the dependency is *newer*. That is not modelled:
//! a stale dependency is a fact about build order, not about the program, and chiero has no
//! build clock. The file-not-found half is the whole observable behaviour the fixture asks for.

use chiero_pp::{Config, FileLoader, preprocess_with_loader};
use std::path::Path;

struct Disk;
impl FileLoader for Disk {
    fn load(&mut self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

fn diagnostics(src: &str) -> Vec<String> {
    preprocess_with_loader("d.c", src, Config::default(), &mut Disk)
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn a_missing_dependency_is_reported() {
    let d = diagnostics("#pragma GCC dependency \"no_such_file_xyzzy.h\"\n");
    assert!(d.iter().any(|m| m.contains("no_such_file_xyzzy.h")), "{d:?}");
}

/// **Through `_Pragma`, through a macro** — the fixture's shape, and the reason the request has
/// to be deferred rather than answered where it is seen.
#[test]
fn a_missing_dependency_is_reported_through_a_macro() {
    let d = diagnostics(
        "#define DO_PRAGMA _Pragma\nDO_PRAGMA (\"GCC dependency \\\"no_such_file_xyzzy.h\\\"\")\n",
    );
    assert!(d.iter().any(|m| m.contains("no_such_file_xyzzy.h")), "{d:?}");
}

/// **A dependency that exists is silent.** Without this, "always report" passes both tests above.
#[test]
fn a_present_dependency_is_silent() {
    let dir = std::env::temp_dir().join(format!("chiero-dep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dep = dir.join("present.h");
    std::fs::write(&dep, "/* here */\n").unwrap();
    let config = Config {
        include_paths: vec![dir.clone()],
        ..Config::default()
    };
    let tu = preprocess_with_loader(
        "d.c",
        "#pragma GCC dependency \"present.h\"\n",
        config,
        &mut Disk,
    );
    let _ = std::fs::remove_file(&dep);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
}
