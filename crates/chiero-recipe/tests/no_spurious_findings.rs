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
fn run(src: &str) -> (Vec<String>, Vec<chiero_exec::Fidelity>) {
    let m = lower(src);
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);
    let f: Vec<String> = r.findings();
    let fid = r.states().iter().map(|s| s.fidelity()).collect();
    (f, fid)
}

/// **The one that was broken.** A function that reads its own scalar parameter.
///
/// `store %param -> %n_slot` then `load %n_slot`. Lowering emitted the parameter prologue
/// *before* the function's `.scope enter`, and entering a scope creates fresh objects for
/// the locals in it (020 §4.4 — that is what makes use-after-scope detectable), so the
/// store was wiped before anything could read it.
#[test]
fn reading_a_parameter_is_not_an_uninitialized_read() {
    let (findings, fid) = run("int f(int n) { int t = 0; if (n > 10) { t = n * 2; } return t; }");
    assert!(
        findings.is_empty(),
        "this function is correct C and every byte it reads it wrote: {findings:#?}"
    );
    assert!(
        fid.iter().all(|f| *f == chiero_exec::Fidelity::Exact),
        "and chiero modeled it exactly — an invented value degrades the path, so a run \
         that is not `Exact` here has stopped describing this program: {fid:?}"
    );
}

/// The simplest possible shape, with no branch at all — so a fix that only repaired the
/// branching case is visible.
#[test]
fn returning_a_parameter_is_not_an_uninitialized_read() {
    let (findings, fid) = run("int f(int n) { return n; }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(fid.iter().all(|f| *f == chiero_exec::Fidelity::Exact));
}

/// **Several parameters, read out of order**, so a fix that happened to work for the first
/// slot is not enough.
#[test]
fn several_parameters_are_all_initialized() {
    let (findings, fid) = run("int f(int a, int b, int c) { return c + a - b; }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(fid.iter().all(|f| *f == chiero_exec::Fidelity::Exact));
}

/// A parameter read **inside a nested scope**, which is where the scope machinery is
/// actually doing something.
#[test]
fn a_parameter_read_in_a_nested_scope_is_initialized() {
    let (findings, fid) = run("int f(int n) { { int q = n + 1; return q; } }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(fid.iter().all(|f| *f == chiero_exec::Fidelity::Exact));
}

/// **The negative control.** A genuinely uninitialized read is still reported.
///
/// Without this, every test above is satisfied by an engine that stopped reporting
/// uninitialized reads at all — which is the other way to make a storm go away, and the
/// one that ships bugs.
#[test]
fn a_genuinely_uninitialized_read_is_still_reported() {
    let (findings, _) = run("int f(void) { int x; return x; }");
    assert!(
        findings.iter().any(|f| f.contains("uninitialized")),
        "`int x; return x;` reads a byte nobody wrote: {findings:#?}"
    );
}

/// And a local that *is* written is not reported, so the control above is not simply
/// "everything is reported".
#[test]
fn an_initialized_local_is_not_reported() {
    let (findings, fid) = run("int f(void) { int x = 7; return x; }");
    assert!(findings.is_empty(), "{findings:#?}");
    assert!(fid.iter().all(|f| *f == chiero_exec::Fidelity::Exact));
}
