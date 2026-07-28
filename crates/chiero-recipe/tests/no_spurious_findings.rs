//! **A clean function produces no findings.**
//!
//! Not a numbered contract. It is the test shape the suite was missing: 1017 tests passed
//! while every function that read its own scalar parameter reported a spurious
//! `uninitialized-read`, because every differential probe compares the *value* chiero
//! computes against gcc and none asserts the absence of findings.
//!
//! 021 §3.1 is explicit that an uninitialized-read false-positive storm is the failure that
//! makes a symbolic engine unusable — "conflating symbolic with uninitialized turns every
//! UCSE run into an uninitialized-read false-positive storm". A storm does not announce
//! itself; it looks exactly like a suite that passes.
//!
//! These fixtures are ordinary C that is right. Anything reported about them is wrong.

use chiero_cir::Module;
use chiero_parse::{ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, preprocess_str};
use chiero_sema::{SymbolText, TargetConfig, analyze};
use chiero_span::Symbol;

struct Names<'a>(&'a ParsedTu);

impl SymbolText for Names<'_> {
    fn text(&self, sym: Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

/// Whether any path degraded because chiero **invented** a value.
/// gcc's system include directories, so the corpus file's `#include`s resolve.
fn gcc_system_paths() -> Vec<std::path::PathBuf> {
    let Ok(out) = std::process::Command::new("gcc")
        .args(["-E", "-v", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .skip_while(|l| !l.starts_with("#include <...>"))
        .skip(1)
        .take_while(|l| !l.starts_with("End of search"))
        .map(|l| std::path::PathBuf::from(l.trim()))
        .filter(|p| p.is_dir())
        .collect()
}

fn invented(causes: &[(chiero_exec::AssumptionKind, String)]) -> bool {
    causes
        .iter()
        .any(|(k, _)| *k == chiero_exec::AssumptionKind::NoInformation)
}

fn lower(src: &str) -> Module {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let lowered = chiero_lower::lower_tu(&parsed.ast, &analysis, &names);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    lowered.module
}

/// Every finding on every path, and the fidelity of the run.
fn run(src: &str) -> (Vec<String>, Vec<(chiero_exec::AssumptionKind, String)>) {
    let m = lower(src);
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);
    let f: Vec<String> = r.findings();
    // **The causes, not the fidelity.** A symbolic branch the default tier-1 solver cannot
    // decide degrades to `Unknown` with `SolverUnknown`, and that is honest: chiero
    // explored both sides and says it did not know which was taken. What must never happen
    // on correct code is `NoInformation` — the cause the engine records when it *invented*
    // a value because a read produced none. That is the storm, and it is what these
    // fixtures are about.
    let causes = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions().iter().map(|x| (x.kind, x.detail.clone())))
        .collect();
    (f, causes)
}

/// **The one that was broken.** A function that reads its own scalar parameter.
///
/// `store %param -> %n_slot` then `load %n_slot`. Lowering emitted the parameter prologue
/// *before* the function's `.scope enter`, and entering a scope creates fresh objects for
/// the locals in it (020 §4.4 — that is what makes use-after-scope detectable), so the
/// store was wiped before anything could read it.
#[test]
fn reading_a_parameter_is_not_an_uninitialized_read() {
    let (findings, causes) =
        run("int f(int n) { int t = 0; if (n > 10) { t = n * 2; } return t; }");
    assert!(
        findings.is_empty(),
        "this function is correct C and every byte it reads it wrote: {findings:#?}"
    );
    assert!(
        !invented(&causes),
        "and nothing was invented: a value chiero made up because a read produced none is \
         the storm itself, whatever findings it does or does not produce: {causes:?}"
    );
}

/// The simplest possible shape, with no branch at all — so a fix that only repaired the
/// branching case is visible.
#[test]
fn returning_a_parameter_is_not_an_uninitialized_read() {
    let (findings, causes) = run("int f(int n) { return n; }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(!invented(&causes), "{causes:?}");
}

/// **Several parameters, read out of order**, so a fix that happened to work for the first
/// slot is not enough.
#[test]
fn several_parameters_are_all_initialized() {
    let (findings, causes) = run("int f(int a, int b, int c) { return c + a - b; }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(!invented(&causes), "{causes:?}");
}

/// A parameter read **inside a nested scope**, which is where the scope machinery is
/// actually doing something.
#[test]
fn a_parameter_read_in_a_nested_scope_is_initialized() {
    let (findings, causes) = run("int f(int n) { { int q = n + 1; return q; } }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(!invented(&causes), "{causes:?}");
}

/// **The negative control.** A genuinely uninitialized read is still reported.
///
/// Without this, every test above is satisfied by an engine that stopped reporting
/// uninitialized reads at all — which is the other way to make a storm go away, and the
/// one that ships bugs.
#[test]
fn a_genuinely_uninitialized_read_is_still_reported() {
    let (findings, causes) = run("int f(void) { int x; return x; }");
    assert!(
        findings.iter().any(|f| f.contains("uninitialized")),
        "`int x; return x;` reads a byte nobody wrote: {findings:#?}"
    );
    // **And it degrades.** The finding and the degradation are separate mechanisms — the
    // fault is reported by the memory model, the fidelity by the engine — so an
    // implementation can have either without the other. `invented()` is what every test
    // above asserts the *absence* of, and a run where it is never true of anything makes
    // all of them vacuous.
    assert!(
        invented(&causes),
        "a value chiero made up is `NoInformation`, or the assertions above are about a \
         cause that never fires: {causes:?}"
    );
}

/// And a local that *is* written is not reported, so the control above is not simply
/// "everything is reported".
#[test]
fn an_initialized_local_is_not_reported() {
    let (findings, causes) = run("int f(void) { int x = 7; return x; }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(!invented(&causes), "{causes:?}");
}

/// **A finding names the variable, not an engine-internal counter.**
///
/// Wave 102 recorded this as owed and worked around it: `chiero-opt`'s transparency sweep
/// normalizes `ObjectId(N)` out of finding text, because `mem2reg` removes allocas and the
/// remaining objects renumber — so *the same defect in the same program* printed
/// differently under a different pass configuration. That is a workaround for a defect in
/// the finding, not a property of the pass.
///
/// `ObjectId` is an allocation counter. It means nothing to a reader, it is not stable
/// across configurations, and 023 §9 wants a finding to carry "everything a reader needs to
/// act on it".
#[test]
fn a_finding_names_the_variable_not_an_internal_id() {
    let (findings, _) = run("int f(void) { int x; return x; }");
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert!(
        findings[0].contains('x'),
        "the finding names `x`, which is what a reader has to go and look at: {}",
        findings[0]
    );
    assert!(
        !findings[0].contains("ObjectId"),
        "and does not name an allocation counter: {}",
        findings[0]
    );
}

/// **No finding anywhere cites an `ObjectId`.**
///
/// A sweep rather than one fixture, because the id is printed by `MemFault`'s `Display`
/// and every fault kind shares it — fixing one message and leaving the rest is the likely
/// half-fix, and it would leave `chiero-opt`'s normalization still load-bearing.
#[test]
fn no_finding_cites_an_object_id() {
    for src in [
        "int f(void) { int x; return x; }",
        "int f(void) { int a[4]; a[0] = 1; return a[7]; }",
        "struct S { int a; int b; }; int f(void) { struct S s; return s.b; }",
    ] {
        let (findings, _) = run(src);
        assert!(
            !findings.is_empty(),
            "the fixture must report something or it proves nothing: {src}"
        );
        for f in &findings {
            assert!(!f.contains("ObjectId"), "`{src}` reported: {f}");
        }
    }
}

/// A finding about a **struct member** names the member, now that lowering builds
/// `AccessPath`s for real C (wave 110).
#[test]
fn a_member_finding_names_the_member() {
    let (findings, _) = run("struct S { int a; int b; }; int f(void) { struct S s; return s.b; }");
    assert!(
        findings.iter().any(|f| f.contains("s.b")),
        "the path names the member that was read: {findings:#?}"
    );
}

/// **The naming picks the right local out of several**, not simply the first.
///
/// `int f(void) { int a = 1; int b; int c = 3; return a + b + c; }` — `b` is the
/// uninitialized one and it is neither the first alloca nor the last. Every earlier
/// fixture has exactly one local, so "look up the object" and "take any object" are
/// indistinguishable in all of them.
#[test]
fn the_naming_picks_the_right_local() {
    let (findings, _) = run("int f(void) { int a = 1; int b; int c = 3; return a + b + c; }");
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert!(
        findings[0].contains('b'),
        "`b` is the uninitialized one: {}",
        findings[0]
    );
    assert!(
        !findings[0].contains(" a ") && !findings[0].contains(" c "),
        "and neither of its neighbours is named: {}",
        findings[0]
    );
}

/// **A global is named too.**
///
/// Globals live in a different table from locals (`global_objs` rather than the frame's
/// `frame_objs`), so naming them is a separate lookup that no local fixture exercises — and
/// mutation confirms it: making `object_name` never look at globals survives every other
/// test in this file.
///
/// **Blocked, and by two lowering defects rather than by anything here.** Every ordinary
/// way to make a global fault produces invalid CIR:
///
/// - `int g[4]; … g[1]` — `PtrAdd base must be pointer-typed, got Int(32)`. Indexing a
///   *global* array is broken; `lvalue_addr`'s `Ident` arm only looks in `fs().locals`.
/// - `const int g = 1; *(int *)&g = 2` — `WidthMismatch`.
///
/// An `extern int g` reads cleanly and reports nothing, which is correct (021 §6: a global
/// chiero cannot see is symbolic, not uninitialized) and therefore useless here.
///
/// Both defects are recorded in HANDOFF §9. Un-ignore this once a global can fault.
#[test]
fn a_global_is_named_in_a_finding() {
    let (findings, _) = run("int g[4]; int f(void) { return g[9]; }");
    assert!(
        findings.iter().any(|f| f.contains('g')),
        "the out-of-bounds access names `g`: {findings:#?}"
    );
    for f in &findings {
        assert!(!f.contains("ObjectId"), "{f}");
    }
}

/// **A finding raised inside a callee names the callee's local**, not the caller's.
///
/// An `AllocaId` is unique only within a function, so the caller's slot of the same id is a
/// *different object*. Naming a fault after it would be worse than leaving it unnamed — the
/// reader would go and look at the wrong variable.
///
/// **Honest limit**: this does not distinguish `stack.last()` from `stack.first()`, and
/// mutation confirms it. The engine enters `module.funcs.first()`, and valid C requires a
/// callee to be declared before it is used — so the callee is always the first function in
/// the module and the fault has one frame under it either way. Reaching the two-frame case
/// needs a hand-built `.cir` module with the entry first, which belongs in `chiero-exec`'s
/// own suite rather than here. Recorded in HANDOFF §9 rather than left as a silent gap.
#[test]
fn a_finding_in_a_callee_names_the_callees_local() {
    // **`f` is defined first, and that is load-bearing**: the engine enters
    // `module.funcs.first()`, so defining the callee first would run *it* as the entry —
    // one frame, and `stack.first()` and `stack.last()` become the same thing, which makes
    // taking the wrong frame unobservable.
    let (findings, _) = run("static int inner(void);\n\
         int f(void) { int shallow = 1; return inner() + shallow; }\n\
         static int inner(void) { int deep; return deep; }\n");
    assert!(
        findings.iter().any(|f| f.contains("deep")),
        "the fault is `inner`'s: {findings:#?}"
    );
    assert!(
        !findings.iter().any(|f| f.contains("shallow")),
        "and not the caller's local of the same slot index: {findings:#?}"
    );
}

/// **An initialized global's value reaches the engine**, not just the CIR.
///
/// `chiero-lower` records the bytes; nothing had checked that the engine *reads* them. The
/// two are separate mechanisms — `GlobalInit::Bytes` is written into the object at
/// materialization — so an encoding that landed in the module and never in memory would
/// pass every lowering test and still return zero here.
///
/// The branch is the assertion: with `g == 7` the run has one path and no finding. If the
/// engine saw zero it would take the other arm, and if it saw *nothing* the condition would
/// be symbolic and both arms would be explored.
#[test]
fn an_initialized_global_is_read_as_its_value() {
    let m = lower("int g = 7; int f(void) { if (g == 7) { return 1; } return 0; }");
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);
    assert_eq!(
        r.states().len(),
        1,
        "`g == 7` is decidable, so there is one path — two means the engine saw a symbol"
    );
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(1),
        "and it took the arm the initializer implies"
    );
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
}

/// The negative: an **uninitialized** global reads as zero (C11 6.7.9p10), so the same
/// fixture with no initializer takes the other arm. Without this, the test above passes
/// against an engine that reports 7 for everything.
#[test]
fn an_uninitialized_global_reads_as_zero() {
    let m = lower("int g; int f(void) { if (g == 7) { return 1; } return 0; }");
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 1);
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(0),
        "static storage with no initializer is zero, and 7 is not zero"
    );
}

// ---------------------------------------------------------------------------------------
// The corpus, executed
// ---------------------------------------------------------------------------------------

/// **No corpus file invents a value.** `#[ignore]`d: two of them do.
///
/// An invented value is `AssumptionKind::NoInformation` — a read that produced nothing, so
/// the engine made one up. Everything computed from it is unsound, and the *absence* of a
/// finding downstream means nothing at all. For files written so that absence is the whole
/// property, that is the difference between a passing test and no test.
///
/// Blocked on gaps the sweep found, not on anything here:
///
/// - **`array_bounds.c`**: `PtrAdd with a symbolic offset` is not modeled, so `buf[i]` with
///   symbolic `i` — the exact thing the file was written to exercise — falls through to an
///   invented value, and the three "not modeled" reports after it cascade from that one.
///   Symbolic indexing of a local array is core to what a symbolic executor does.
/// - **`abs_branch.c`**: the default tier-1 solver cannot discharge `my_abs(x) >= 0`, which
///   is honest incompleteness rather than a defect, but still leaves the run unable to
///   demonstrate what the file was written to demonstrate.
///
/// Un-ignore when symbolic `PtrAdd` is modeled. Recorded in HANDOFF §9.
#[test]
fn no_corpus_file_invents_a_value() {
    let root = workspace_root();
    let include = root.join("include");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("tests/corpus/c"))
        .expect("the C corpus exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    files.sort();
    for path in &files {
        let Some(m) = lower_corpus(path, &include) else {
            eprintln!("skipping no_corpus_file_invents_a_value: no gcc system include path");
            return;
        };
        let Some(b) = chiero_solver::SmtLib::discover() else {
            eprintln!("skipping no_corpus_file_invents_a_value: no SMT solver on PATH");
            return;
        };
        let mut a = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m).with_backend(b).run(&mut a);
        for st in r.states() {
            let invented: Vec<String> = st
                .assumptions()
                .iter()
                .filter(|x| x.kind == chiero_exec::AssumptionKind::NoInformation)
                .map(|x| x.detail.clone())
                .collect();
            assert!(
                invented.is_empty(),
                "{} invented a value: {invented:?}",
                path.display()
            );
        }
    }
}

/// Lower one corpus C file, with the include path and gcc's system headers.
fn lower_corpus(path: &std::path::Path, include: &std::path::Path) -> Option<Module> {
    struct Disk;
    impl chiero_pp::FileLoader for Disk {
        fn load(&mut self, p: &std::path::Path) -> std::io::Result<String> {
            std::fs::read_to_string(p)
        }
    }
    let system = gcc_system_paths();
    if system.is_empty() {
        return None;
    }
    let src = std::fs::read_to_string(path).expect("the corpus file exists");
    let cfg = Config {
        include_paths: vec![include.to_path_buf()],
        iquote_paths: vec![include.to_path_buf()],
        system_paths: system,
        defines: vec![("__CHIERO__".into(), "1".into())],
        ..Config::default()
    };
    let session = chiero_pp::PreprocessorSession::new();
    let tu = session.preprocess_with_loader(path, &src, cfg, &mut Disk);
    assert!(
        tu.diagnostics.is_empty(),
        "{}: {:?}",
        path.display(),
        tu.diagnostics
    );
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(
        parsed.diagnostics.is_empty(),
        "{}: {:?}",
        path.display(),
        parsed.diagnostics
    );
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    assert!(
        analysis.diagnostics.is_empty(),
        "{}: {:?}",
        path.display(),
        analysis.diagnostics
    );
    let lowered = chiero_lower::lower_tu(&parsed.ast, &analysis, &names);
    assert!(
        lowered.diagnostics.is_empty(),
        "{}: {:?}",
        path.display(),
        lowered.diagnostics
    );
    Some(lowered.module)
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// **Every corpus file runs clean.**
///
/// `goldens.rs` compares each corpus file's lowered *text* against a checked-in `.cir`. That
/// pins the shape and says nothing about the values: a golden is equally stable whether the
/// program computes the right answer or the wrong one, and it stays stable while a read
/// silently returns `Undef`.
///
/// These files are written to hold. Every `chiero_assert` in them is a claim the author
/// believed, so a finding here is either a real defect in chiero or a fixture that stopped
/// being true — and both are worth stopping for. Wave 114 checked one file this way; this
/// is the sweep §9 asked for.
#[test]
fn every_corpus_file_runs_clean() {
    let root = workspace_root();
    let dir = root.join("tests/corpus/c");
    let include = root.join("include");

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("the C corpus exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    // Sorted: `read_dir` order is the filesystem's, and a sweep that walks it differently
    // on another machine is a different test.
    files.sort();
    assert!(
        files.len() >= 5,
        "found {} files in {}",
        files.len(),
        dir.display()
    );

    let mut ran = 0;
    for path in &files {
        let Some(m) = lower_corpus(path, &include) else {
            // **Announced, not silent.** An oracle that can quietly not run is not an
            // oracle; this project has had to fix that six times.
            eprintln!("skipping every_corpus_file_runs_clean: no gcc system include path");
            return;
        };
        let errs = chiero_cir::verify::verify(&m);
        assert!(
            errs.iter().all(|e| !e.is_error()),
            "{} does not verify: {errs:#?}",
            path.display()
        );
        let Some(b) = chiero_solver::SmtLib::discover() else {
            eprintln!("skipping every_corpus_file_runs_clean: no SMT solver on PATH");
            return;
        };
        let mut a = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m).with_backend(b).run(&mut a);
        assert!(
            !r.states().is_empty(),
            "{} produced no path at all",
            path.display()
        );
        // **A "may" finding is incompleteness, not a defect claim.** With the default
        // tier-1 solver chiero cannot discharge `my_abs(x) >= 0`, and saying so is the
        // honest answer — 023 §7 wants a run to report what it could not decide rather
        // than assume it away. What must never appear is a finding asserting the program
        // *is* wrong, because every one of these files is correct C.
        let definite: Vec<String> = r
            .findings()
            .into_iter()
            .filter(|f| !f.contains("may be") && !f.contains("could not"))
            .collect();
        assert!(
            definite.is_empty(),
            "{} reports a definite defect in correct C: {definite:#?}",
            path.display()
        );

        // **"No findings" is worth nothing on its own.** Every one of these files is
        // written so that the *absence* of a report is the property — `array_bounds.c`
        // says so in its own comment — and absence is exactly what a run that stopped
        // early, ran out of budget, or invented its way past the interesting part also
        // produces. So the run has to have got there:
        for st in r.states() {
            assert!(
                matches!(
                    st.status,
                    chiero_exec::Status::Terminated(chiero_exec::TermReason::Return)
                ),
                "{} ended a path with {:?} rather than returning — the assertions after \
                 that point were never evaluated",
                path.display(),
                st.status
            );
        }
        ran += 1;
    }
    assert_eq!(
        ran,
        files.len(),
        "every file was executed, not skipped past"
    );
}

/// **`tests/corpus/c/globals.c` runs clean.**
///
/// Lowering it and comparing against a golden says the *shape* is stable; it says nothing
/// about whether the values are right. This runs it: every `chiero_assert` in the file
/// holds, and the run reports nothing.
///
/// The file's assertions are chosen so that an engine reading the table as zeros cannot
/// pass — `calls == 1` depends on a global being written and read back across a call, which
/// neither an `Undef` read (wave 112's defect) nor a zero read (wave 113's) satisfies.
#[test]
fn the_globals_corpus_file_runs_clean() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let path = root.join("tests/corpus/c/globals.c");
    let include = root.join("include");

    struct Disk;
    impl chiero_pp::FileLoader for Disk {
        fn load(&mut self, p: &std::path::Path) -> std::io::Result<String> {
            std::fs::read_to_string(p)
        }
    }
    let src = std::fs::read_to_string(&path).expect("the corpus file exists");
    let cfg = Config {
        include_paths: vec![include.clone()],
        iquote_paths: vec![include],
        system_paths: gcc_system_paths(),
        defines: vec![("__CHIERO__".into(), "1".into())],
        ..Config::default()
    };
    if cfg.system_paths.is_empty() {
        // **Skipping is announced.** A silent skip is how an oracle stops running and a
        // suite keeps reporting success; this project has had to fix that six times.
        eprintln!("skipping the_globals_corpus_file_runs_clean: no gcc system include path");
        return;
    }
    let session = chiero_pp::PreprocessorSession::new();
    let tu = session.preprocess_with_loader(&path, &src, cfg, &mut Disk);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let lowered = chiero_lower::lower_tu(&parsed.ast, &analysis, &names);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let m = lowered.module;

    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");

    // The table's bytes really are in the module, so a run that reports nothing is
    // reporting about the right program.
    let t = m
        .globals
        .iter()
        .find(|g| &*g.name == "table")
        .expect("the table is a global");
    assert!(t.is_const, "`static const int table[4]`");
    assert_eq!(
        t.init,
        chiero_cir::GlobalInit::Bytes(vec![10, 0, 0, 0, 20, 0, 0, 0, 30, 0, 0, 0, 40, 0, 0, 0])
    );

    let Some(b) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping the_globals_corpus_file_runs_clean: no SMT solver on PATH");
        return;
    };
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).with_backend(b).run(&mut a);
    assert!(
        !r.states().is_empty(),
        "the entry function ran on at least one path"
    );
    if std::env::var("WHERE").is_ok() {
        for f in r.reports() {
            eprintln!("AT {:?} :: {}", f.span, f.message);
        }
    }
    assert!(
        r.findings().is_empty(),
        "every assertion in the file holds: {:#?}",
        r.findings()
    );
}

// ---------------------------------------------------------------------------------------
// 021 contract 21 — writing to a read-only global
// ---------------------------------------------------------------------------------------

/// **021 contract 21.** "Writing to a `readonly` global is exactly one finding and does not
/// alter the bytes."
///
/// Reachable since wave 114 taught lowering to set `is_const`, and untested since — the
/// checker was correct and unreachable before that, and nothing exercised it after.
///
/// **Not a corpus fixture**: the corpus sweep asserts no *definite* defect in any file, and
/// this program's whole point is to commit one. A fixture that must report belongs beside
/// the fixtures that must not.
#[test]
fn writing_to_a_const_global_is_exactly_one_finding() {
    let src = "const int g = 7;\n\
               int f(void) { *(int *)&g = 2; return g; }\n";
    let m = lower(src);
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");

    let gd = m.globals.iter().find(|x| &*x.name == "g").expect("g");
    assert!(gd.is_const, "the fixture must actually be read-only");

    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);

    let ro: Vec<String> = r
        .findings()
        .into_iter()
        .filter(|x| x.contains("readonly") || x.contains("read-only"))
        .collect();
    assert_eq!(
        ro.len(),
        1,
        "exactly one — not one per byte written, and not none: {:#?}",
        r.findings()
    );
    assert!(
        ro[0].contains('g'),
        "and it names the global (wave 111): {}",
        ro[0]
    );

    // **The bytes are unchanged**, which is the half a count cannot show. The write is
    // refused, so the read after it still sees the initializer — an engine that reported
    // *and* wrote would satisfy the assertion above and corrupt the program's state.
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(7),
        "the initializer survived the refused write"
    );
}

/// **Writing to a mutable global is not a finding**, or the check above is satisfied by an
/// engine that reports every store to any global.
#[test]
fn writing_to_a_mutable_global_is_not_a_finding() {
    let src = "int g = 7;\n\
               int f(void) { g = 2; return g; }\n";
    let m = lower(src);
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);
    assert!(
        r.findings().is_empty(),
        "an ordinary global is writable: {:#?}",
        r.findings()
    );
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(2),
        "and the write took effect"
    );
}
