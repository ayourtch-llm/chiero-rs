#![allow(dead_code, unreachable_pub)]

//! Preprocess → parse → analyse → lower, in one call.

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

/// Lower a source string, asserting every earlier stage is clean first.
///
/// Each of those assertions is load-bearing: a fixture that fails to preprocess yields an
/// empty token stream, an empty tree, an empty module — and **every shape assertion in
/// this file passes over nothing**. That has cost this project a fixture in four separate
/// waves, so the guard is not optional politeness.
pub fn lower(src: &str) -> Module {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(
        tu.diagnostics.is_empty(),
        "the fixture must preprocess cleanly: {:?}",
        tu.diagnostics
    );
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(
        parsed.diagnostics.is_empty(),
        "and parse cleanly: {:?}",
        parsed.diagnostics
    );
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    assert!(
        analysis.diagnostics.is_empty(),
        "and analyse cleanly, or lowering is being graded on a broken tree: {:?}",
        analysis.diagnostics
    );
    let lowered = chiero_lower::lower_tu(&parsed.ast, &analysis, &names);
    assert!(
        lowered.diagnostics.is_empty(),
        "and lower without refusing anything: {:?}",
        lowered.diagnostics
    );
    lowered.module
}

pub fn print(m: &Module) -> String {
    chiero_cir::text::print(m)
}
