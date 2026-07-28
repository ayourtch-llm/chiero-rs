//! The corpus is a runnable C program too — 024 contract 17.
//!
//! Covers: 024 contract 17.
//!
//! "Every corpus file including `chiero.h` compiles and runs under gcc with the
//! intrinsics as no-ops."
//!
//! §7's reason for the contract: "That dual use is what makes the differential oracle
//! (070) work without maintaining two copies of every test." A corpus file that stops
//! compiling natively does not fail loudly — it quietly removes itself from the oracle,
//! and the symbolic side keeps passing on its own.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/chiero-model`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<name> is two levels down")
        .to_path_buf()
}

fn corpus_files() -> Vec<PathBuf> {
    let dir = repo_root().join("tests/corpus/c");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("no corpus at {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    // Deterministic order (001 §5): a failure names a file, and the file it names should
    // not depend on the filesystem's iteration order.
    v.sort();
    v
}

fn gcc() -> Option<&'static str> {
    Command::new("gcc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| "gcc")
}

/// **024 contract 17.** Every corpus file compiles under gcc and exits 0.
///
/// `-Werror` is deliberate. The header's no-ops take parameters they do not use, and the
/// obvious way to write them produces `-Wunused-parameter` on every call site — which a
/// warning-tolerant test would never notice, and which makes the corpus unusable in any
/// project that builds with warnings as errors.
#[test]
fn every_corpus_file_compiles_and_runs_under_gcc() {
    let Some(cc) = gcc() else {
        eprintln!("skipping: no gcc on PATH");
        return;
    };
    let files = corpus_files();
    assert!(
        !files.is_empty(),
        "the corpus is empty, so this contract is satisfied by having nothing to check"
    );

    let out = std::env::temp_dir().join("chiero-corpus-native");
    std::fs::create_dir_all(&out).expect("temp dir");
    let include = repo_root().join("include");

    for f in &files {
        let bin = out.join(f.file_stem().expect("a .c file has a stem"));
        let compile = Command::new(cc)
            .args(["-std=gnu11", "-Wall", "-Wextra", "-Werror", "-O1", "-I"])
            .arg(&include)
            .arg("-o")
            .arg(&bin)
            .arg(f)
            .output()
            .expect("gcc runs");
        assert!(
            compile.status.success(),
            "{} does not compile natively, so it has silently left 070's differential \
             oracle:\n{}",
            f.display(),
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = Command::new(&bin).output().expect("the corpus binary runs");
        assert!(
            run.status.success(),
            "{} compiled but exited {:?}; the native run is the oracle's ground truth, so \
             it has to be a program that works",
            f.display(),
            run.status.code()
        );
    }
    eprintln!("{} corpus files compile and run natively", files.len());
}

/// **The trap the header's `#ifdef` exists to avoid, pinned.**
///
/// Under `__CHIERO__` the intrinsics must be *declarations*. If the header defined them
/// unconditionally — the natural way to write it — chiero would lower the no-op bodies,
/// because 023 §5 says "the module's own definition always wins" over a registered model.
/// Nothing would become symbolic, every corpus program would be explored along one
/// concrete path, every assertion would hold, and the suite would report success over a
/// symbolic execution that never happened.
///
/// Compiling with `-D__CHIERO__` and *linking* must therefore fail with an undefined
/// reference. A link that succeeds means a body came from somewhere.
#[test]
fn under_the_chiero_macro_the_intrinsics_have_no_body() {
    let Some(cc) = gcc() else {
        eprintln!("skipping: no gcc on PATH");
        return;
    };
    let out = std::env::temp_dir().join("chiero-corpus-native");
    std::fs::create_dir_all(&out).expect("temp dir");
    let include = repo_root().join("include");
    let src = corpus_files()
        .into_iter()
        .next()
        .expect("at least one corpus file");

    // Compiling alone must succeed — the declarations are well-formed.
    let obj = out.join("decl_only.o");
    let compile = Command::new(cc)
        .args([
            "-std=gnu11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-D__CHIERO__",
            "-c",
            "-I",
        ])
        .arg(&include)
        .arg("-o")
        .arg(&obj)
        .arg(&src)
        .output()
        .expect("gcc runs");
    assert!(
        compile.status.success(),
        "the `__CHIERO__` branch must still be valid C:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // Linking must not.
    let bin = out.join("decl_only");
    let link = Command::new(cc)
        .arg("-o")
        .arg(&bin)
        .arg(&obj)
        .output()
        .expect("gcc runs");
    assert!(
        !link.status.success(),
        "`{}` linked under `-D__CHIERO__`, which means the intrinsics have bodies there. \
         chiero would analyse those bodies instead of its models (023 §5), and every \
         corpus program would run fully concrete while appearing to pass.",
        src.display()
    );
    let stderr = String::from_utf8_lossy(&link.stderr);
    assert!(
        stderr.contains("chiero_make_symbolic") || stderr.contains("undefined"),
        "the link failed for some other reason than the missing intrinsics:\n{stderr}"
    );
}

/// Every intrinsic 024 §7 lists is actually declared, in both branches.
///
/// A header that omits one compiles fine for every corpus file that does not use it, so
/// the gap appears only when someone writes the first test that needs it — and looks like
/// their mistake.
#[test]
fn the_header_declares_every_intrinsic_the_spec_lists() {
    let header = std::fs::read_to_string(repo_root().join("include/chiero.h"))
        .expect("include/chiero.h exists");
    for name in [
        "chiero_make_symbolic",
        "chiero_assume",
        "chiero_assert",
        "chiero_is_symbolic",
        "chiero_mark_fidelity",
    ] {
        // Twice: once in the `__CHIERO__` branch, once in the native one.
        let n = header.matches(name).count();
        assert!(
            n >= 2,
            "`{name}` appears {n} time(s) in chiero.h; 024 §7 requires it in both the \
             declaration branch and the no-op branch"
        );
    }
}

/// And the model registry knows them, or a corpus file's `chiero_assume` is an unmodeled
/// extern: chiero would havoc, degrade to `Approximated`, and constrain nothing.
#[test]
fn the_registry_models_the_intrinsics_the_header_declares() {
    let reg = chiero_model::ModelRegistry::with_builtins();
    for name in [
        "chiero_make_symbolic",
        "chiero_assume",
        "chiero_assert",
        "chiero_is_symbolic",
        "chiero_mark_fidelity",
    ] {
        assert!(
            reg.lookup(name).is_some(),
            "`{name}` is declared in chiero.h but has no model, so a corpus file calling \
             it hits 023 §5's unmodeled-extern path: the call havocs, the run degrades to \
             `Approximated`, and nothing it was supposed to do happens"
        );
    }
}
