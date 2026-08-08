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

/// The compiler's own include directories and predefines, asked of the compiler rather than
/// guessed — [`chiero_probe`], which is the one place in the workspace that runs `cc`.
///
/// **This gate used to have its own copy of the include-path scrape and no persona at all.** It
/// took each TU's `-D`/`-I` from `builddb` and inherited its predefines from `Config::default()`'s
/// baked table, so it preprocessed VPP under a configuration nobody ships — and, worse, under a
/// compiler with no `-march`, which is the half of the tree multiarch exists to build. Both facts
/// now come from the same probe the CLI uses (HANDOFF §9.1).
fn probe() -> Option<&'static chiero_probe::Probe> {
    let p = chiero_probe::Probe::shared();
    // `None` when there is no compiler, and the caller then **skips with a printed reason** rather
    // than passing — a corpus test that silently succeeds because it analysed nothing is the
    // failure mode this whole file exists to undo.
    (!p.include_paths().is_empty()).then_some(p)
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
    let Some(probe) = probe() else {
        eprintln!("SKIPPED: no compiler, so the system include path and the persona are unknown");
        return;
    };
    let system = probe.include_paths();
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
    // Each cause keeps one example path. **Without it a cause is not addressable**: I spent a
    // wave reasoning about which VPP file produced `redefinition of macro MFD_HUGETLB` from the
    // message alone, and was wrong about the file. The panic list below always printed an
    // example; the diagnostic list did not, and that asymmetry was the whole defect.
    let mut causes: Vec<(String, usize, PathBuf)> = Vec::new();
    for u in &units {
        let Ok(src) = std::fs::read_to_string(&u.src) else {
            unreadable += 1;
            continue;
        };
        // **The unit's own `-march` selects the persona** — 060 §1.1's multiarch is not a label
        // on a struct, it is a different program per variant. Five probes for 1967 units.
        let cfg = Config {
            system_paths: system.clone(),
            ..u.pp_config(probe)
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
                    match causes.iter_mut().find(|(c, ..)| *c == msg) {
                        Some((_, k, _)) => *k += 1,
                        None => causes.push((msg, 1, u.src.clone())),
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
    // **Printed because the token count moves when this number is wrong.** One persona for the
    // whole run is what the gate did before the join, and it is indistinguishable from a correct
    // run by every other number here: taking the wrong `#if` branch emits no diagnostic.
    eprintln!(
        "  personas: {} distinct target flag-sets probed from {}",
        probe.persona_probes(),
        probe.compiler()
    );
    // **A flag-set the compiler refused is a unit analysed under the wrong persona**, and it is
    // silent everywhere else: the run still preprocesses, still emits no diagnostic, and still
    // counts a persona. Named here, and asserted below, because the baked fallback is the right
    // answer and the wrong one to be given without knowing.
    for f in probe.failed_probes() {
        eprintln!("  ⚠️ could not probe a persona for {f:?} — the baked one was substituted");
    }
    if unreadable > 0 {
        eprintln!("  ⚠️ {unreadable} sources the database names do not exist on disk");
    }
    causes.sort_by_key(|(_, n, _)| std::cmp::Reverse(*n));
    for (msg, n, example) in &causes {
        eprintln!("  {n:>5} TUs  {msg}  e.g. {}", example.display());
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
        probe.failed_probes().is_empty(),
        "{} target flag-sets could not be probed, so those units were analysed under a persona \
         that is not theirs: {:?}",
        probe.failed_probes().len(),
        probe.failed_probes()
    );
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
