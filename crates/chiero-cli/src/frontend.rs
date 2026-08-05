//! **C in, CIR out** — the frontend composition, which lives here and nowhere else.
//!
//! 001 §4 keeps the analysis crates free of a frontend dependency: `chiero-opt` is a vertical,
//! `chiero-tool` has `chiero-pp` as a *dev*-dependency only. This is the binary, which may
//! depend on everything, so the composition belongs here.
//!
//! **Every stage's diagnostics are a refusal, not a warning.** A file that does not preprocess
//! was never seen; one that does not parse was seen and not understood; a function lowering
//! gives up on is *absent from the module*, and an absent function is invisible to everything
//! downstream. Returning a partial module and letting an operation answer about it is how a
//! comparison comes to bless two functions one of which was never there.

use std::path::{Path, PathBuf};

pub(crate) struct Disk;

impl chiero_pp::FileLoader for Disk {
    fn load(&mut self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// The `-I` and `-D` a caller supplied.
#[derive(Clone, Debug, Default)]
pub(crate) struct Frontend {
    pub(crate) includes: Vec<PathBuf>,
    pub(crate) defines: Vec<(String, String)>,
}

impl Frontend {
    pub(crate) fn pp(&self) -> chiero_pp::Config {
        chiero_pp::Config {
            include_paths: self.includes.clone(),
            defines: self.defines.clone(),
            ..chiero_pp::Config::default()
        }
    }
}

/// Preprocess, and refuse on the first diagnostic.
pub(crate) fn preprocess(
    path: &Path,
    src: &str,
    f: Frontend,
) -> Result<chiero_pp::PreprocessedTu, String> {
    let tu = chiero_pp::preprocess_with_loader(path, src, f.pp(), &mut Disk);
    match tu.diagnostics.first() {
        Some(d) => Err(format!("{}: {}", path.display(), d.message)),
        None => Ok(tu),
    }
}

struct Names(chiero_parse::ParsedTu);

impl chiero_sema::SymbolText for Names {
    fn text(&self, sym: chiero_span::Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

/// The whole pipeline: preprocess, parse, analyse, lower.
pub(crate) fn lower(path: &Path, src: &str, f: Frontend) -> Result<chiero_cir::Module, String> {
    let tu = preprocess(path, src, f)?;
    let mut oracle = chiero_parse::ScopedTypedefs::new();
    let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
    if let Some(d) = parsed.diagnostics.first() {
        return Err(format!("{}: {}", path.display(), d.message));
    }
    let names = Names(parsed);
    let analysis = chiero_sema::analyze(
        &names.0.ast,
        &chiero_sema::TargetConfig::x86_64_linux(),
        &names,
    );
    if let Some(d) = analysis.diagnostics.first() {
        return Err(format!("{}: {}", path.display(), d.message));
    }
    let lowered = chiero_lower::lower_tu_with_map(&names.0.ast, &analysis, &names, &tu.source_map);
    // **A lowering diagnostic drops a function from the module**, so continuing here would
    // hand an operation a translation unit with a hole in it and no way to know.
    if let Some(d) = lowered.diagnostics.first() {
        return Err(format!(
            "{}: chiero cannot lower this: {}",
            path.display(),
            d.message
        ));
    }
    Ok(lowered.module)
}
