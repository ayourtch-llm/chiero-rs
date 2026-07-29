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

/// Lower a real file on disk, with an include path — needed by the `gcov_lines` tests,
/// where the *file* a span resolves to is the property under test and a synthetic
/// `preprocess_str` path would resolve to nothing on disk.
pub fn lower_file(
    path: &std::path::Path,
    includes: &[std::path::PathBuf],
) -> (Module, chiero_span::SourceMap) {
    struct Disk;
    impl chiero_pp::FileLoader for Disk {
        fn load(&mut self, p: &std::path::Path) -> std::io::Result<String> {
            std::fs::read_to_string(p)
        }
    }
    let src = std::fs::read_to_string(path).expect("fixture exists");
    let cfg = Config {
        include_paths: includes.to_vec(),
        iquote_paths: includes.to_vec(),
        system_paths: gcc_system_paths(),
        defines: vec![("__CHIERO__".into(), "1".into())],
        ..Config::default()
    };
    let session = chiero_pp::PreprocessorSession::new();
    let tu = session.preprocess_with_loader(path, &src, cfg, &mut Disk);
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
    let lowered = chiero_lower::lower_tu_with_map(&parsed.ast, &analysis, &names, &tu.source_map);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    (lowered.module, tu.source_map)
}

/// gcc's own include directories, asked of gcc rather than guessed. A corpus file
/// includes `<stddef.h>`, so without these it does not preprocess at all.
pub fn gcc_system_paths() -> Vec<std::path::PathBuf> {
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

/// Lower without requiring lowering to be clean — for the tests whose subject *is* a
/// refusal (015 §7, contract 20).
pub fn lower_raw(src: &str) -> chiero_lower::Lowered {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    chiero_lower::lower_tu(&parsed.ast, &analysis, &names)
}

pub fn print(m: &Module) -> String {
    chiero_cir::text::print(m)
}

/// Lower `src` under a named build configuration, with command-line-style defines.
///
/// 020 contract 30's fixture needs two runs of the *same source* that differ only in what
/// the preprocessor saw — which is what a `ConfigId` names.
pub fn lower_with_config(
    src: &str,
    config: chiero_pp::ConfigId,
    defines: &[(&str, &str)],
) -> Module {
    let cfg = Config {
        id: config,
        defines: defines
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ..Config::default()
    };
    let tu = preprocess_str("t.c", src, cfg);
    assert!(
        tu.diagnostics.is_empty(),
        "the fixture must preprocess cleanly: {:?}",
        tu.diagnostics
    );
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
    let lowered =
        chiero_lower::lower_tu_with_config(&parsed.ast, &analysis, &names, None, Some(config.0));
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    lowered.module
}

/// Lower, or `None` if any stage refused. Unlike [`lower`] this does not assert: the
/// census compares chiero against gcc over generated programs, where a refusal is a
/// legitimate outcome to skip rather than a test failure.
pub fn lower_maybe(src: &str) -> Option<Module> {
    let tu = preprocess_str("t.c", src, Config::default());
    if !tu.diagnostics.is_empty() {
        return None;
    }
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    if !analysis.diagnostics.is_empty() {
        return None;
    }
    let lowered = chiero_lower::lower_tu(&parsed.ast, &analysis, &names);
    if !lowered.diagnostics.is_empty() {
        return None;
    }
    Some(lowered.module)
}
