//! Covers: 012 contract 17 — "Preprocessing every TU in VPP's `compile_commands.json` produces
//! zero panics, and the count of TUs producing at least one diagnostic is tracked as a
//! regression metric."
//!
//! **What was here before measured nothing, twice over.** The contract's test lived in
//! `chiero-pp/tests/directives.rs`, was `#[ignore]`d *and* returned early on a
//! `compile_commands.json` that has never existed, and its one surviving assertion was that the
//! file it had already established was absent contained the substring `"file"`. Two ways of
//! measuring nothing, stacked — and it sat in the frontend crate, which 060 §1 says must not know
//! VPP exists.
//!
//! It is here instead because `chiero-vpp` is "the **only** crate that knows VPP exists", and it
//! is real because [`chiero_vpp::builddb`] can now say what flags each TU actually compiles under.
//! Preprocessing VPP with the wrong `-D`/`-I` preprocesses a different program, so this could not
//! have been written before the ingest existed.

use chiero_pp::{Config, FileLoader, PreprocessorSession};
use chiero_vpp::builddb::BuildDb;
use std::io;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

struct Disk;
impl FileLoader for Disk {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// gcc's own include directories, asked of gcc rather than guessed.
///
/// `None` when gcc is unavailable, and the caller then **skips with a printed reason** rather
/// than passing — a corpus test that silently succeeds because it analysed nothing is the
/// failure mode this whole file exists to undo.
fn system_include_paths() -> Option<Vec<PathBuf>> {
    let out = std::process::Command::new("gcc")
        .args(["-E", "-v", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut paths = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("#include <...>") {
            inside = true;
        } else if line.starts_with("End of search list") {
            break;
        } else if inside && line.starts_with(' ') {
            paths.push(PathBuf::from(line.trim()));
        }
    }
    (!paths.is_empty()).then_some(paths)
}

/// 012 contract 17. Ignored because it needs a built VPP; run with
/// `cargo test -p chiero-vpp --test preprocess_corpus -- --ignored --nocapture`.
///
/// `CHIERO_PP_CORPUS_LIMIT=<n>` preprocesses only the first `n` C units. The default is **all of
/// them**, because the contract says *every* TU and a default that quietly sampled would be the
/// third way of measuring nothing in the same test.
#[test]
#[ignore = "external corpus — needs a built VPP tree"]
fn every_vpp_compile_command_preprocesses_without_panicking() {
    let build = Path::new("/home/ubuntu/vpp/build-root/build-vpp-native/vpp");
    if !build.join("build.ninja").exists() {
        eprintln!("SKIPPED: no VPP build at {}", build.display());
        return;
    }
    let Some(system) = system_include_paths() else {
        eprintln!("SKIPPED: gcc unavailable, so the system include path is unknown");
        return;
    };
    let out = std::process::Command::new("ninja")
        .args(["-C", build.to_str().unwrap(), "-t", "compdb"])
        .output()
        .expect("ninja -t compdb");
    let db = BuildDb::parse(&String::from_utf8(out.stdout).unwrap()).expect("VPP's database");

    let mut units: Vec<_> = db.c_units().collect();
    let all = units.len();
    if let Ok(n) = std::env::var("CHIERO_PP_CORPUS_LIMIT") {
        units.truncate(n.parse().expect("CHIERO_PP_CORPUS_LIMIT is a number"));
    }
    assert!(units.len() > 100, "only {} C units", units.len());

    // The default hook prints a full backtrace per panic; with a corpus this size that buries
    // the result. The message is kept — it is the only thing that says *what* broke.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let (mut panicked, mut diagnosed, mut unreadable, mut tokens) = (Vec::new(), 0usize, 0, 0u64);
    // **What the diagnostics say, not just how many there are.** 25 TUs diagnosed is a number
    // nobody can act on; one message repeated 25 times is a single bug with an address. Counted
    // by TU, because a header included by 400 units would otherwise dominate by sheer arithmetic.
    let mut causes: Vec<(String, usize)> = Vec::new();
    for u in &units {
        let Ok(src) = std::fs::read_to_string(&u.src) else {
            unreadable += 1;
            continue;
        };
        let cfg = Config {
            system_paths: system.clone(),
            ..u.pp_config()
        };
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let s = PreprocessorSession::new();
            let tu = s.preprocess_with_loader(&u.src, &src, cfg, &mut Disk);
            let mut msgs: Vec<String> = tu.diagnostics.iter().map(|d| d.message.clone()).collect();
            msgs.sort_unstable();
            msgs.dedup();
            (msgs, tu.token_texts().count())
        }));
        match r {
            Ok((m, n)) if m.is_empty() => tokens += n as u64,
            Ok((m, n)) => {
                diagnosed += 1;
                tokens += n as u64;
                for msg in m {
                    match causes.iter_mut().find(|(c, _)| *c == msg) {
                        Some((_, k)) => *k += 1,
                        None => causes.push((msg, 1)),
                    }
                }
            }
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("<non-string panic>")
                    .to_string();
                panicked.push((u.src.clone(), msg));
            }
        }
    }
    std::panic::set_hook(hook);

    // **The metric, printed unconditionally.** 012 contract 17 asks for the diagnosed count to
    // be *tracked*, and a number nobody can see is not tracked.
    eprintln!(
        "012 c17: {}/{all} C units preprocessed | {} panicked | {} diagnosed | {} unreadable | {tokens} tokens",
        units.len(),
        panicked.len(),
        diagnosed,
        unreadable
    );
    if unreadable > 0 {
        eprintln!("  ⚠️ {unreadable} sources the database names do not exist on disk");
    }
    causes.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (msg, n) in &causes {
        eprintln!("  {n:>5} TUs  {msg}");
    }

    // Distinct messages first: 1900 panics with one cause is one bug, and a list of paths hides
    // that while a list of messages shows it.
    let mut kinds: Vec<&str> = panicked.iter().map(|(_, m)| m.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    for k in &kinds {
        let n = panicked.iter().filter(|(_, m)| m == k).count();
        let ex = panicked.iter().find(|(_, m)| m == k).unwrap();
        eprintln!("  {n:>5}  {k}  e.g. {}", ex.0.display());
    }

    assert!(
        panicked.is_empty(),
        "{} of {} VPP translation units panicked the preprocessor ({} distinct causes)",
        panicked.len(),
        units.len(),
        kinds.len()
    );
    // Not a claim that the preprocessor is *correct* — only that it ran. A run that produced no
    // tokens would satisfy "zero panics" perfectly.
    assert!(
        tokens > 1_000_000,
        "only {tokens} tokens over {} units",
        units.len()
    );
}
