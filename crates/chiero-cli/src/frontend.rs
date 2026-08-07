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

use chiero_opt::locality::{Field, Record};
use std::path::{Path, PathBuf};

pub(crate) struct Disk;

impl chiero_pp::FileLoader for Disk {
    fn load(&mut self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// The `-I` and `-D` a caller supplied, plus whether to ask the system compiler where its own
/// headers are.
#[derive(Clone, Debug, Default)]
pub(crate) struct Frontend {
    pub(crate) includes: Vec<PathBuf>,
    pub(crate) defines: Vec<(String, String)>,
    /// Ask `cc` for its include paths and predefined macros — on by default.
    ///
    /// **Real C starts with `#include <stdio.h>`.** Without this every operation answers
    /// "cannot include stdarg.h" on anything from an actual codebase, which is a fact about the
    /// invocation rather than about the code.
    ///
    /// Discovery at *run* time, like the solver's (022 §4) and the replay compiler's: chiero
    /// links no toolchain (010 §1), and a machine without one gets an empty answer rather than
    /// a build-time dependency.
    pub(crate) system_headers: bool,
}

impl Frontend {
    pub(crate) fn pp(&self) -> chiero_pp::Config {
        let (system, predefined) = if self.system_headers {
            system_environment()
        } else {
            (Vec::new(), Vec::new())
        };
        chiero_pp::Config {
            // The preprocessor's pedantry has to match the parser's, or the two disagree about
            // the same file.
            pedantic: false,
            include_paths: self.includes.clone(),
            system_paths: system,
            // **The caller's `-D` last, so it wins.** A tree that redefines something the
            // compiler predefines means it; taking the predefine instead would analyse a
            // different program.
            defines: predefined
                .into_iter()
                .chain(self.defines.iter().cloned())
                .collect(),
            ..chiero_pp::Config::default()
        }
    }
}

/// The compiler's include paths and its predefined macros — see [`system_environment`].
type SystemEnvironment = (Vec<PathBuf>, Vec<(String, String)>);

/// Where the system compiler keeps its headers, and what it predefines.
///
/// **Both, or neither is much use.** glibc's `bits/floatn.h` branches on a dozen
/// `__HAVE_FLOAT*` macros, so a preprocessor with the paths and not the predefines compiles
/// code the compiler never sees — the full-tree sweep's first run reported 101 findings that
/// were entirely that.
///
/// Probed once: the answer is a property of the machine.
fn system_environment() -> SystemEnvironment {
    static PROBED: std::sync::OnceLock<SystemEnvironment> = std::sync::OnceLock::new();
    PROBED
        .get_or_init(|| {
            let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
            (include_paths(&cc), predefines(&cc))
        })
        .clone()
}

fn include_paths(cc: &str) -> Vec<PathBuf> {
    let Ok(out) = std::process::Command::new(cc)
        .args(["-E", "-v", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stderr);
    let mut paths = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("#include <...>") {
            inside = true;
        } else if line.starts_with("End of search list") {
            break;
        } else if inside {
            paths.push(PathBuf::from(line.trim()));
        }
    }
    paths
}

fn predefines(cc: &str) -> Vec<(String, String)> {
    let Ok(out) = std::process::Command::new(cc)
        .args(["-dM", "-E", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, ' ');
            if it.next() != Some("#define") {
                return None;
            }
            let name = it.next()?;
            // Function-like macros, and the ones the preprocessor must own itself.
            if name.contains('(')
                || matches!(
                    name,
                    "__FILE__" | "__LINE__" | "__DATE__" | "__TIME__" | "__COUNTER__"
                )
            {
                return None;
            }
            Some((name.to_owned(), it.next().unwrap_or("1").to_owned()))
        })
        .collect()
}

/// **Where a frontend diagnostic is**, as `path:line:col`, and where it came from when a macro
/// put it there.
///
/// The span was always on the `Diagnostic`; the command printed only the message and the file
/// it was asked about. Over a 92-plugin sweep that made eleven failures — `expected a type
/// specifier`, `` `clib_crc32c_with_init` was not declared `` — into eleven separate reductions
/// to find the line, and for a construct chiero cannot parse the line is usually in a *header*
/// the file included rather than in the file named on the command line.
///
/// **Both locations when they differ**, which is 010 §4's whole distinction: `spelling_loc` is
/// where the text is, `expansion_loc` is where the program wrote it. For a macro that expands
/// to something chiero cannot handle, a reader needs the second to know what they typed and the
/// first to know what it became.
fn at(map: &chiero_span::SourceMap, sp: chiero_span::Span, d: &str) -> String {
    let show =
        |l: chiero_span::Loc| format!("{}:{}:{}", map.file(l.file).path().display(), l.line, l.col);
    match (map.spelling_loc(sp), map.expansion_loc(sp)) {
        (Some(sl), Some(el)) if sl.pos != el.pos => {
            format!("{}: {d}\n  expanded from {}", show(sl), show(el))
        }
        (Some(l), _) | (None, Some(l)) => format!("{}: {d}", show(l)),
        // A span with no file is a span into nothing; naming the file asked about is still
        // better than naming nowhere.
        (None, None) => d.to_string(),
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
        Some(d) => Err(at(&tu.source_map, d.span, &d.message)),
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
    // **GNU C, not strict ISO.** The tree under analysis is built by gcc or clang with their
    // defaults, and a pedantic frontend rejects `__int128` in VPP's own `vppinfra/format.c` —
    // an ISO complaint about code that compiles, which tells a reader nothing about their
    // program. `Dialect::gnu` is what the full-tree sweep uses for the same reason.
    let dialect = chiero_ast::Dialect::gnu();
    let parsed = chiero_parse::parse_tu_with(&tu, &mut oracle, dialect);
    if let Some(d) = parsed.diagnostics.first() {
        return Err(at(&tu.source_map, d.span, &d.message));
    }
    let names = Names(parsed);
    let analysis = chiero_sema::analyze_with(
        &names.0.ast,
        &chiero_sema::TargetConfig::x86_64_linux(),
        &names,
        dialect,
    );
    if let Some(d) = analysis.diagnostics.first() {
        return Err(at(&tu.source_map, d.span, &d.message));
    }
    let lowered = chiero_lower::lower_tu_with_map(&names.0.ast, &analysis, &names, &tu.source_map);
    // **A lowering diagnostic drops a function from the module**, so continuing here would
    // hand an operation a translation unit with a hole in it and no way to know.
    if let Some(d) = lowered.diagnostics.first() {
        return Err(at(
            &tu.source_map,
            d.span,
            &format!("chiero cannot lower this: {}", d.message),
        ));
    }
    Ok(lowered.module)
}

/// Every complete, named record in a translation unit, as 041 §3's locality analysis wants it.
///
/// **014 §3's layout, converted rather than recomputed.** `chiero-opt` is a vertical that must
/// not depend on the frontend, so the analysis takes a plain description; this is the one place
/// that fills it in, from the layouts sema computed and gcc's own corpus gate checked.
///
/// `externally_visible` is left `false` here and set by the caller's own knowledge. That is the
/// wrong default in isolation — §3 wants the unprovable case treated as observable — so the
/// caller is told to decide, and `chiero layout` says in the envelope which it assumed.
pub(crate) fn records(path: &Path, src: &str, f: Frontend) -> Result<Vec<Record>, String> {
    let tu = preprocess(path, src, f)?;
    let mut oracle = chiero_parse::ScopedTypedefs::new();
    // **GNU C, not strict ISO.** The tree under analysis is built by gcc or clang with their
    // defaults, and a pedantic frontend rejects `__int128` in VPP's own `vppinfra/format.c` —
    // an ISO complaint about code that compiles, which tells a reader nothing about their
    // program. `Dialect::gnu` is what the full-tree sweep uses for the same reason.
    let dialect = chiero_ast::Dialect::gnu();
    let parsed = chiero_parse::parse_tu_with(&tu, &mut oracle, dialect);
    if let Some(d) = parsed.diagnostics.first() {
        return Err(at(&tu.source_map, d.span, &d.message));
    }
    let names = Names(parsed);
    let analysis = chiero_sema::analyze_with(
        &names.0.ast,
        &chiero_sema::TargetConfig::x86_64_linux(),
        &names,
        dialect,
    );
    let mut out = Vec::new();
    for (i, l) in analysis.records().iter().enumerate() {
        if !l.complete || l.is_union {
            // A union's members all start at zero, so neither straddling nor padding means
            // what this analysis means by them.
            continue;
        }
        let Some(tag) = analysis
            .tag_of(chiero_sema::RecordId(i as u32))
            .and_then(|t| names.0.text(t))
        else {
            // An anonymous record has no name to report a proposal against.
            continue;
        };
        // **Every member that occupies bytes is in the list, named or not**, and anything
        // left out marks the list partial. 041 §3's padding proposal is arithmetic over the
        // whole record — lay the members out largest-first, compare with the real size — so a
        // dropped member does not make the answer conservative, it makes it wrong in the
        // flattering direction.
        //
        // This dropped **anonymous members** for a while, because the name was fetched with
        // `fl.name?` inside a `filter_map` and C's anonymous union has none. Measured on VPP's
        // `fib_route_path_t`: a 56-byte anonymous union vanished, the remaining 7 bytes of
        // fields rounded up to the struct's alignment, and a 72-byte struct was reported as
        // able to be 8. The size and alignment were right the whole time, which is what made
        // it read as an answer.
        // **A zero-width bit-field makes the list partial, and no arithmetic fixes that.** It
        // declares no member, so it is in no field list (014 §3 says why it cannot be), and
        // what it leaves behind is a gap in its neighbours' offsets that reads exactly like
        // alignment padding. The difference is the whole question here: padding comes back
        // under a reorder and this boundary does not, because it follows the run wherever the
        // run goes.
        //
        // Measured, on `struct Q { unsigned a:1; unsigned :0; char c; unsigned b:1;
        // unsigned :0; char d; }`: 12 bytes, and summing the members that are visible said it
        // "would be 4" — `proven`, not advisory. gcc's floor over every order that keeps each
        // run together is 8.
        let mut fields_complete = true;
        let mut fields: Vec<Field> = Vec::new();
        for fl in &l.fields {
            // **A bit-field's extent is bits, so it is passed as bits** — 041 §3.1. It used to
            // be dropped, with the field list marked partial and the padding proposal withheld
            // for the whole record; honest, and it left out exactly the packed, hand-tuned
            // structs where padding matters most. `(offset, size)` here is the byte span those
            // bits touch, which is what a consumer reading bytes alone should see, and the
            // analysis reads `bits` to group a run into one member.
            if let Some(b) = fl.bits {
                let offset = b.bit_offset / 8;
                fields.push(Field {
                    name: match fl.name.and_then(|n| names.0.text(n)) {
                        Some(n) => n.to_string(),
                        None => format!("<anonymous bit-field at bit {}>", b.bit_offset),
                    },
                    offset,
                    size: (b.bit_offset + b.width).div_ceil(8).saturating_sub(offset),
                    bits: Some(chiero_opt::locality::BitExtent {
                        bit_offset: b.bit_offset,
                        width: b.width,
                    }),
                });
                continue;
            }
            let Some(size) = analysis.size_of(fl.ty) else {
                // A member whose size sema could not compute is a hole of unknown width.
                fields_complete = false;
                continue;
            };
            fields.push(Field {
                // **An anonymous member is reported by what it is**, since a proposal naming
                // it has to say something a reader can find in the source. It occupies the
                // bytes either way, which is what the padding sum needs from it.
                name: match fl.name.and_then(|n| names.0.text(n)) {
                    Some(n) => n.to_string(),
                    None => format!("<anonymous member at offset {}>", fl.offset),
                },
                offset: fl.offset,
                size,
                bits: None,
            });
        }
        out.push(Record {
            tag: tag.to_string(),
            size: l.size,
            align: l.align,
            packed: l.packed,
            externally_visible: false,
            fields,
            fields_complete,
        });
    }
    Ok(out)
}
