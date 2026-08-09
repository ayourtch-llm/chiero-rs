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
    /// **Target flags handed to the persona probe** — `-march=x86-64-v2`, `-mavx2`, and friends.
    ///
    /// These are not decoration: `__SSE4_2__` and `__AVX2__` exist only under the right `-march`,
    /// and VPP compiles the same source repeatedly under different ones. Probing with no flags
    /// while the build uses `-march=x86-64-v2` predefines a different compiler than the one that
    /// ships — measured: every AVX2 path in vppinfra had never been compiled by any chiero
    /// measurement. HANDOFF §9.1.
    pub(crate) target_flags: Vec<String>,
}

impl Frontend {
    pub(crate) fn pp(&self) -> chiero_pp::Config {
        let (system, persona) = if self.system_headers {
            system_environment(&self.target_flags)
        } else {
            (Vec::new(), chiero_pp::Persona::baked())
        };
        chiero_pp::Config {
            // The preprocessor's pedantry has to match the parser's, or the two disagree about
            // the same file.
            pedantic: false,
            include_paths: self.includes.clone(),
            system_paths: system,
            // **The caller's `-D` wins**, and it is `Config::defines` that carries it: a tree
            // redefining something the compiler predefines means it, and taking the predefine
            // instead would analyse a different program. The persona is installed first.
            defines: self.defines.clone(),
            persona,
            ..chiero_pp::Config::default()
        }
    }
}

/// The compiler's include paths and its predefined macros — see [`system_environment`].
type SystemEnvironment = (Vec<PathBuf>, chiero_pp::Persona);

/// Where the system compiler keeps its headers, and what it predefines.
///
/// **Both, or neither is much use.** glibc's `bits/floatn.h` branches on a dozen
/// `__HAVE_FLOAT*` macros, so a preprocessor with the paths and not the predefines compiles
/// code the compiler never sees — the full-tree sweep's first run reported 101 findings that
/// were entirely that.
///
/// **Probed once *per flag-set*, in `chiero-probe`, which is the whole of this function's
/// history.** It used to memoize in a `OnceLock` that took the target flags and then ignored
/// them, so the first caller's `-march` was answered to every later one. One process, one
/// operation, one flag-set is why that never showed here — and 060 §1.1's multiarch 1:N is
/// precisely many flag-sets in one run.
fn system_environment(target_flags: &[String]) -> SystemEnvironment {
    let probe = chiero_probe::Probe::shared();
    (probe.include_paths(), probe.persona(target_flags))
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
    // **An advisory does not cost the caller the analysis.** Sema's diagnostics carry a
    // severity because "I could not model this" and "I did, and here is a concern about the
    // program" are different events that shared a list. Stopping on the first entry meant a
    // signed-overflowing constant expression -- which chiero folds to exactly the value gcc
    // and clang fold it to -- refused a whole translation unit that gcc compiles with exit 0.
    //
    // The concern is still printed. Downgrading a diagnostic is not deleting it, and an
    // advisory nobody sees is the same as no advisory.
    for d in analysis.diagnostics.iter().filter(|d| !d.is_error()) {
        eprintln!("chiero: {}", at(&tu.source_map, d.span, &d.message));
    }
    if let Some(d) = analysis.diagnostics.iter().find(|d| d.is_error()) {
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
    // **The same rule `lower` applies, and this path did not apply any.** It looked at no sema
    // diagnostic at all, so a TU with an undeclared name still produced a padding proposal
    // stamped `proven — this holds for all inputs (Exact)`. 041 §3's proposal is arithmetic
    // over a record's members, and a record whose type resolution failed still lays out — so
    // the arithmetic held, for a struct the program does not have. `proven` is the word that
    // makes that worse than saying nothing.
    for d in analysis.diagnostics.iter().filter(|d| !d.is_error()) {
        eprintln!("chiero: {}", at(&tu.source_map, d.span, &d.message));
    }
    if let Some(d) = analysis.diagnostics.iter().find(|d| d.is_error()) {
        return Err(at(&tu.source_map, d.span, &d.message));
    }
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
        let mut fields_complete = !l.has_zero_width_bitfield;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Does this machine's `cc` predefine `__AVX2__` under `-march=x86-64-v3` and not without it?
    ///
    /// Checked rather than assumed: with no compiler installed, or on a machine that is not x86,
    /// the two probes are legitimately identical and the test below would be asserting a property
    /// of the machine rather than of chiero — which is the shape of green this file's own history
    /// keeps warning about.
    fn avx2_discriminates() -> bool {
        let dump = |args: &[&str]| -> String {
            let mut a: Vec<&str> = vec!["-dM", "-E"];
            a.extend_from_slice(args);
            a.extend_from_slice(&["-x", "c", "/dev/null"]);
            std::process::Command::new("cc")
                .args(&a)
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        };
        !dump(&[]).contains("__AVX2__") && dump(&["-march=x86-64-v3"]).contains("__AVX2__")
    }

    fn frontend(target_flags: &[&str]) -> Frontend {
        Frontend {
            system_headers: true,
            target_flags: target_flags.iter().map(|s| s.to_string()).collect(),
            ..Frontend::default()
        }
    }

    /// **The `-march` on the command line has to reach the persona the file is preprocessed with.**
    ///
    /// This asks `Frontend::pp()` rather than the probe, because the probe having a keyed cache
    /// proves nothing about whether the composition passes the key: `system_environment` used to
    /// take the target flags and then memoize the answer in a `OnceLock`, so within one process the
    /// first flag-set won and every later one was silently given its persona. One process, one
    /// operation, one flag-set is why that never showed here — and 060 §1.1's multiarch 1:N is
    /// precisely many flag-sets in one run.
    #[test]
    fn each_target_flag_set_gets_its_own_persona() {
        if !avx2_discriminates() {
            eprintln!(
                "SKIPPED: this machine's cc does not discriminate -march=x86-64-v3 by __AVX2__"
            );
            return;
        }
        // Deliberately in this order: under the `OnceLock` this replaces, the *first* call's
        // answer was handed to every later one, so asking the plain one first is what exposes it.
        let plain = frontend(&[]).pp().persona;
        let v3 = frontend(&["-march=x86-64-v3"]).pp().persona;
        assert_eq!(
            v3.get("__AVX2__"),
            Some("1"),
            "-march=x86-64-v3 predefines __AVX2__; this persona was probed for some other flag-set"
        );
        assert_eq!(
            plain.get("__AVX2__"),
            None,
            "no -march predefines no __AVX2__; this persona came from a different probe"
        );
    }

    /// `--no-system-headers` is a deliberate configuration, not an absent one: the baked persona
    /// is what a caller with no compiler gets, and it must not silently acquire a `-march`.
    #[test]
    fn without_system_headers_the_persona_is_the_baked_one() {
        let f = Frontend {
            system_headers: false,
            target_flags: vec!["-march=x86-64-v3".into()],
            ..Frontend::default()
        };
        assert_eq!(f.pp().persona, chiero_pp::Persona::baked());
    }
}
