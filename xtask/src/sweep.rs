//! Sweep an **external** C tree with chiero and with gcc, and say where they disagree.
//!
//! The hermetic corpus is 28 vendored files — the include closure of six `vppinfra` headers —
//! against VPP's 1552 `.c` files. This tool exists to find the disagreements that a corpus that
//! small cannot, without vendoring a tree: `001 §4 rule 4 / contract 5` keeps VPP knowledge inside
//! `chiero-vpp`, and a tree path passed at run time puts none of it in any crate.
//!
//! **It is a reporting tool, never a gate.** The suite must keep running with no external
//! dependency, so nothing here is wired into `xtask gates`. The working loop is: sweep → queue →
//! reduce a finding → vendor the *reduced* case → RED/GREEN as usual.
//!
//! # gcc is the oracle, with the tree's own flags
//!
//! A chiero diagnostic is a finding only if gcc accepts the same file. The trap is *which* gcc:
//! this project calibrates constraint violations to `-pedantic-errors` (wave 314), while VPP
//! builds under `-std=gnu11` where many of those are legal — `int a[0]` alone appears 1777 times.
//! An oracle run pedantically reports all of it. **The census asks what C forbids; the sweep asks
//! what real code does that chiero mishandles.** Those need different gcc invocations, and the
//! sweep takes the tree's.

use std::path::{Path, PathBuf};

/// What one compiler made of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The compiler accepted the file but said something about it.
    ///
    /// Kept apart from `Clean` because a warning *is* a diagnostic: gcc and chiero agreeing
    /// that code is wrong, at different severities, is not chiero being wrong.
    Warned(String),
    /// Compiled or analysed with nothing to say.
    Clean,
    /// Produced diagnostics — the first is kept for the report.
    Diagnosed(String),
    /// Could not be run on this file at all: a flag the tool cannot take, a missing include.
    /// **Never silently dropped**, because a silent skip is how a sweep lies about its coverage.
    NotRun(String),
}

/// Where a file lands once both compilers have spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// gcc warned and chiero diagnosed: they agree on the code, not on how loudly to say so.
    SeverityMismatch,
    /// gcc accepted and chiero did not — the finding, and the top of the queue.
    Finding,
    /// gcc refused and chiero was silent — a missing rule. Lower priority: gcc's reason may
    /// need flags this sweep does not pass.
    Miss,
    /// Both were clean: the file was tested and chiero matched gcc. **The only bucket that is
    /// evidence of agreement.**
    Agree,
    /// Both produced diagnostics. On a real tree this almost always means the *flags* are wrong
    /// for this file — a generated header, a `-D` the build passes — so gcc never judged the C
    /// and chiero refusing it too says nothing. Kept apart from `Agree` because merging them lets
    /// a sweep that tested nothing report `0 findings`, which reads as success: `vlib` gave 45 of
    /// these while gcc compiled **none** of its 47 files under the flags used.
    BothRefused,
    /// One of the two could not be run. Reported as its own bucket rather than skipped.
    ToolGap,
}

/// Classify one file from the pair of outcomes.
///
/// Pure, and separated from the walking and the running so it can be tested exhaustively — the
/// I/O half is what needs a tree, and this is the half that carries the judgement.
pub fn classify(gcc: &Outcome, chiero: &Outcome) -> Bucket {
    match (gcc, chiero) {
        (Outcome::NotRun(_), _) | (_, Outcome::NotRun(_)) => Bucket::ToolGap,
        // **A warning is a diagnostic.** gcc noticing something chiero did not is a miss
        // however quietly gcc said it, and gcc and chiero both noticing is agreement about
        // the code — a severity question, not a defect in either.
        (Outcome::Warned(_), Outcome::Clean) => Bucket::Miss,
        (Outcome::Warned(_), _) => Bucket::SeverityMismatch,
        // chiero has no warning level, so its side is never `Warned` today. Handled beside
        // `Diagnosed` rather than left to a `_` arm, so adding one later is a compile error
        // here and not a silent reclassification.
        (Outcome::Clean, Outcome::Diagnosed(_) | Outcome::Warned(_)) => Bucket::Finding,
        (Outcome::Diagnosed(_), Outcome::Clean) => Bucket::Miss,
        (Outcome::Clean, Outcome::Clean) => Bucket::Agree,
        (Outcome::Diagnosed(_), Outcome::Diagnosed(_) | Outcome::Warned(_)) => Bucket::BothRefused,
    }
}

/// Every `.c` file under `tree`, sorted, so a sweep is reproducible run to run.
///
/// Headers are not translation units and are swept only through the files that include them.
pub fn translation_units(tree: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(tree, &mut out)?;
    // **Sorted, so two sweeps of one tree can be diffed.** `read_dir` order is whatever the
    // filesystem gives, which differs between machines and after any file is rewritten.
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "c") {
            out.push(path);
        }
    }
    Ok(())
}

/// How to invoke both compilers over this tree.
///
/// **The tree's own flags, not the project's.** A sweep of VPP passes what VPP builds with; the
/// module docs say why that must not be `-pedantic-errors`.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    /// `-pedantic-errors` (the wave-314 calibration) or gcc's `-std=gnu11` default.
    pub dialect: chiero_ast::Dialect,
    /// `-I` paths, in order.
    pub includes: Vec<PathBuf>,
    /// `-D` definitions, as `NAME` or `NAME=VALUE`.
    pub defines: Vec<String>,
    /// The dialect, e.g. `gnu11`.
    pub std: Option<String>,
}

impl Flags {
    pub fn gcc_args(&self) -> Vec<String> {
        let mut a = vec!["-fsyntax-only".to_owned()];
        if let Some(s) = &self.std {
            a.push(format!("-std={s}"));
        }
        // **The dialect goes to gcc too, or the two are answering different questions.** A
        // default sweep otherwise compared strict chiero against permissive gcc, and a `--gnu`
        // sweep could never show chiero being too *permissive* under the strict dialect.
        if self.dialect.pedantic {
            a.push("-pedantic-errors".to_owned());
        }
        // **Warnings stay on.** They were suppressed with `-w` under the argument that gcc's
        // default noise would "put clean files in the wrong bucket" — true while a warning made
        // a file a `Finding`, and false since `Bucket::SeverityMismatch` exists. With `-w` the
        // severity bucket was unreachable and its count was a constant, not a measurement.
        for i in &self.includes {
            a.push(format!("-I{}", i.display()));
        }
        for d in &self.defines {
            a.push(format!("-D{d}"));
        }
        a
    }
}

/// Run gcc over one file and say what it made of it.
pub fn gcc_outcome(path: &Path, flags: &Flags) -> Outcome {
    let mut cmd = std::process::Command::new("gcc");
    cmd.args(flags.gcc_args()).arg(path);
    match cmd.output() {
        Err(e) => Outcome::NotRun(format!("gcc could not be run: {e}")),
        // **Exit zero is not silence.** gcc warns and still succeeds, and a warning is a
        // diagnostic: filing such a file as `Clean` made every chiero complaint on it look
        // like an over-rejection. The first `warning:` line stands for the lot, matching how
        // the diagnosed path reports the first `error:`.
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stderr);
            match text.lines().find(|l| l.contains("warning:")) {
                None => Outcome::Clean,
                Some(w) => Outcome::Warned(w.trim().to_owned()),
            }
        }
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stderr);
            // **A missing include is a tool gap, not a verdict on the file.** VPP generates some
            // headers at build time; without them gcc has not judged the C at all, and putting
            // such a file in `Miss` would invent a rule chiero is supposedly lacking.
            let first = text
                .lines()
                .find(|l| l.contains("error:"))
                .unwrap_or("(no error line)")
                .trim()
                .to_owned();
            if text.contains("fatal error:") {
                Outcome::NotRun(first)
            } else {
                Outcome::Diagnosed(first)
            }
        }
    }
}

struct Disk;
impl chiero_pp::FileLoader for Disk {
    fn load(&mut self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

struct Names(chiero_parse::ParsedTu);
impl chiero_sema::SymbolText for Names {
    fn text(&self, sym: chiero_span::Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

/// Run chiero's frontend over one file: preprocess, parse, analyse.
///
/// **The first stage to speak wins**, and the stage is named in the message. A parse error and a
/// sema diagnostic are both "chiero complained", but a reader triaging a queue needs to know
/// which — the fixes live in different crates.
pub fn chiero_outcome(
    path: &Path,
    flags: &Flags,
    system: &[PathBuf],
    predefines: &[(String, String)],
) -> Outcome {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return Outcome::NotRun(format!("unreadable: {e}")),
    };
    let cfg = chiero_pp::Config {
        // **Not pedantic.** The sweep asks what real code does that chiero mishandles, and the
        // tree builds without `-pedantic-errors`; see the module docs.
        pedantic: false,
        include_paths: flags.includes.clone(),
        system_paths: system.to_vec(),
        // **gcc's predefines first, then the tree's own `-D`.** Without the predefines chiero
        // takes `#if` branches gcc never compiles; see `gcc_predefines`.
        defines: predefines
            .iter()
            .cloned()
            .chain(flags.defines.iter().map(|d| match d.split_once('=') {
                Some((k, v)) => (k.to_owned(), v.to_owned()),
                None => (d.clone(), "1".to_owned()),
            }))
            .collect(),
        ..chiero_pp::Config::default()
    };
    let session = chiero_pp::PreprocessorSession::new();
    let tu = session.preprocess_with_loader(path, &src, cfg, &mut Disk);
    if let Some(first) = tu.diagnostics.first() {
        // A `#include` that does not resolve means chiero never saw the C, exactly as a gcc
        // `fatal error` does — a tool gap, not a verdict.
        let m = format!(
            "pp: {}",
            describe(&tu.source_map, first.span, &first.message)
        );
        return if first.message.contains("cannot open") || first.message.contains("not found") {
            Outcome::NotRun(m)
        } else {
            Outcome::Diagnosed(m)
        };
    }
    let mut oracle = chiero_parse::ScopedTypedefs::new();
    let parsed = chiero_parse::parse_tu_with(&tu, &mut oracle, flags.dialect);
    if let Some(first) = parsed.diagnostics.first() {
        return Outcome::Diagnosed(format!(
            "parse: {}",
            describe(&tu.source_map, first.span, &first.message)
        ));
    }
    let names = Names(parsed);
    let analysis = chiero_sema::analyze_with(
        &names.0.ast,
        &chiero_sema::TargetConfig::x86_64_linux(),
        &names,
        flags.dialect,
    );
    // **A diagnostic from a system header is not the project's defect**, and gcc does not
    // report one: it suppresses `-pedantic` diagnostics originating in a system header. Without
    // this the strict sweep's last finding was `__int128` inside `/usr/include/linux/types.h`,
    // which no reader could act on and gcc never mentions.
    match analysis
        .diagnostics
        .iter()
        .find(|d| !in_system_header(&tu.source_map, d.span, system))
    {
        Some(first) => Outcome::Diagnosed(format!(
            "sema: {}",
            describe(&tu.source_map, first.span, &first.message)
        )),
        None => Outcome::Clean,
    }
}

/// Render a diagnostic as `path:line:col: message`, the form an editor and a person both accept.
///
/// **A dummy span is rejected by name, not by a failed lookup.** `Span::DUMMY` is `BytePos(0)`
/// and the first file in the global space starts at 0, so `lookup_loc` succeeds on it and
/// answers line 1, column 1 — a location that looks real and is not. Everything else that fails
/// to resolve falls back to the bare message for the same reason: a report is allowed to say
/// less than it knows, never more.
pub fn describe(map: &chiero_span::SourceMap, span: chiero_span::Span, message: &str) -> String {
    if span.is_dummy() {
        return message.to_owned();
    }
    match map.lookup_loc(span.lo) {
        Some(loc) => format!(
            "{}:{}:{}: {message}",
            map.file(loc.file).path().display(),
            loc.line,
            loc.col
        ),
        None => message.to_owned(),
    }
}

/// The grouping key: a diagnostic's *kind*, with any leading `path:line:col:` removed.
///
/// **Stripping is by shape, not by the first colon.** A message is full of colons — gcc writes
/// `error: redefinition of 'f'`, sema writes prose — so the scan accepts a segment only when it
/// is followed by two all-digit segments and then more text. That is a location; anything else
/// is the message and passes through whole.
pub fn kind(message: &str) -> String {
    // An optional `tool: ` prefix that this module added, kept on the front of the key.
    let (prefix, rest) = match message.split_once(": ") {
        Some((p, r)) if matches!(p, "pp" | "parse" | "sema") => (format!("{p}: "), r),
        _ => (String::new(), message),
    };
    // Find `<path>:<line>:<col>: ` at the front. The path may itself contain colons, so take the
    // *last* candidate split that still leaves two numbers and a message.
    let bytes: Vec<usize> = rest.match_indices(':').map(|(i, _)| i).collect();
    for w in bytes.windows(2) {
        let (a, b) = (w[0], w[1]);
        let line = &rest[a + 1..b];
        let after = &rest[b + 1..];
        let Some(end) = after.find(':') else { continue };
        let col = &after[..end];
        let tail = &after[end + 1..];
        if !line.is_empty()
            && !col.is_empty()
            && line.bytes().all(|c| c.is_ascii_digit())
            && col.bytes().all(|c| c.is_ascii_digit())
            && let Some(msg) = tail.strip_prefix(' ')
        {
            return format!("{prefix}{msg}");
        }
    }
    format!("{prefix}{rest}")
}

/// The functions a translation unit **defines**, for tier 1 (042 c7).
///
/// Preprocessed with no include paths: the caller sweeping a real tree supplies its own via
/// [`chiero_outcome`], while this entry point exists to be given source directly.
///
/// **Definitions, not declarations.** A prototype has no body for tier 2 to analyse, and
/// counting it as a candidate would inflate a recipe's tally with functions nothing could
/// examine. `static` definitions *are* included — 042 §3.1's recall hole is an unregistered
/// helper, and in VPP those are routinely `static`.
pub fn functions_in(path: &Path, src: &str) -> Result<Vec<chiero_recipe::FunctionRef>, String> {
    functions_in_cfg(path, src, chiero_pp::Config::default())
}

/// As [`functions_in`], with the caller's include paths and defines.
///
/// A sweep over a real tree needs gcc's predefines and the project's `-I` set for the same
/// reason [`chiero_outcome`] does: without them chiero takes `#if` branches gcc never
/// compiles, and every file fails to resolve its first header.
pub fn functions_in_cfg(
    path: &Path,
    src: &str,
    cfg: chiero_pp::Config,
) -> Result<Vec<chiero_recipe::FunctionRef>, String> {
    let session = chiero_pp::PreprocessorSession::new();
    let tu = session.preprocess_with_loader(path, src, cfg, &mut Disk);
    if let Some(first) = tu.diagnostics.first() {
        return Err(format!("pp: {}", first.message));
    }
    let mut oracle = chiero_parse::ScopedTypedefs::new();
    let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
    if let Some(first) = parsed.diagnostics.first() {
        return Err(format!("parse: {}", first.message));
    }
    let mut out = Vec::new();
    for &item in parsed.ast.items() {
        if let chiero_ast::DeclKind::Func {
            name,
            body: Some(_),
            ..
        } = parsed.ast.decl(item).kind
            && let Some(text) = parsed.text(name)
        {
            // **The defining file, resolved from the declaration's span** — not the
            // translation unit's path. A `static inline` in a shared header would otherwise be
            // counted once per includer, which is what produced 186,623 functions from 36
            // files, and would make `in_file` match a vppinfra header under a `vnet` glob.
            //
            // `expansion_loc`, not `lookup_loc`: a function generated by a macro is written by
            // the file that *invokes* it, and the raw written position of its tokens is the
            // header holding the macro body. VPP generates node functions this way constantly,
            // so `lookup_loc` would file every node in the tree under one header — the same
            // error as keying on the includer, pointing the other way.
            let decl_span = parsed.ast.decl(item).span;
            let file = tu
                .source_map
                .expansion_loc(decl_span)
                .and_then(|l| tu.source_map.try_file(l.file))
                .map_or_else(
                    || path.display().to_string(),
                    |f| f.path().display().to_string(),
                );
            out.push(chiero_recipe::FunctionRef {
                name: text.to_owned(),
                file,
            });
        }
    }
    Ok(out)
}

/// What a tier-1 sweep found, and what it could not look at (042 c7).
#[derive(Debug, Clone)]
pub struct Tier1Report {
    pub tallies: Vec<chiero_recipe::RecipeTally>,
    pub files: usize,
    pub functions: usize,
    /// Translation units that did not preprocess or parse, so contributed no functions.
    pub unreadable: usize,
    /// Workers actually used. Reported because 042 c7's budget is a time *on a core count*,
    /// so a duration without it is not a measurement anyone can reproduce.
    pub threads: usize,
}

impl Tier1Report {
    /// Whether every file was read **and** every recipe could be evaluated.
    ///
    /// The two failures are different — an unread file and an undecidable selector — but a
    /// caller publishing a candidate count may not claim completeness under either.
    pub fn is_complete(&self) -> bool {
        self.unreadable == 0
            && self
                .tallies
                .iter()
                .all(chiero_recipe::RecipeTally::is_complete)
    }
}

/// Run tier 1 over a set of translation units (042 c7).
/// Run tier 1 over a set of translation units (042 c7), fanned out across the machine.
pub fn tier1_sweep(
    files: &[PathBuf],
    recipes: &[chiero_recipe::Recipe],
    cfg: &chiero_pp::Config,
) -> Tier1Report {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    tier1_sweep_with(files, recipes, cfg, threads)
}

/// As [`tier1_sweep`], with an explicit thread count.
///
/// **The answer does not depend on the split.** Each worker returns its own results and the
/// merge happens afterwards, in file order — nothing is accumulated into shared state, so the
/// only thing threading changes is when the work happens. 001 §5 makes that mandatory: this
/// report is an output path, and 042 c5d wants the counts as a CI baseline, which a figure
/// that wobbled with core count could never be.
pub fn tier1_sweep_with(
    files: &[PathBuf],
    recipes: &[chiero_recipe::Recipe],
    cfg: &chiero_pp::Config,
    threads: usize,
) -> Tier1Report {
    // Only the lower bound is needed. `per = ceil(len / threads)` already bounds the chunk
    // count by *both* `threads` and `len`: chunks = ceil(len / per) <= threads, and <= len
    // because `per >= 1`. An upper clamp to `files.len()` was unreachable — mutation removed
    // it and nothing changed. The `max(1)` is real: `div_ceil(0)` divides by zero.
    let threads = threads.max(1);
    // Contiguous chunks rather than a work queue: the merge below relies on chunk *k* holding
    // the k-th run of files, and a queue would make the merge order depend on scheduling.
    let per = files.len().div_ceil(threads);
    let chunks: Vec<&[PathBuf]> = if per == 0 {
        Vec::new()
    } else {
        files.chunks(per).collect()
    };

    let mut results: Vec<(Vec<chiero_recipe::FunctionRef>, usize)> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| scope.spawn(move || scan_chunk(chunk, cfg)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| (Vec::new(), 0)))
            .collect()
    });

    // **Dedup after the join, never inside a worker.** Two workers that each dropped their own
    // duplicates would still both keep a header function, and the survivor would depend on
    // which chunk saw it first.
    let mut seen: indexmap::IndexSet<(String, String)> = indexmap::IndexSet::new();
    let mut functions = Vec::new();
    let mut unreadable = 0;
    for (found, bad) in results.drain(..) {
        unreadable += bad;
        for f in found {
            if seen.insert((f.file.clone(), f.name.clone())) {
                functions.push(f);
            }
        }
    }

    Tier1Report {
        tallies: chiero_recipe::tier1_counts(recipes, &functions),
        files: files.len(),
        functions: functions.len(),
        unreadable,
        threads: chunks.len(),
    }
}

/// One worker's share: the functions it found and the files it could not read.
fn scan_chunk(
    files: &[PathBuf],
    cfg: &chiero_pp::Config,
) -> (Vec<chiero_recipe::FunctionRef>, usize) {
    let mut found = Vec::new();
    let mut unreadable = 0;
    for path in files {
        let Ok(src) = std::fs::read_to_string(path) else {
            unreadable += 1;
            continue;
        };
        match functions_in_cfg(path, &src, cfg.clone()) {
            Ok(fs) => found.extend(fs),
            // Counted, never skipped: a file that contributed no functions because it did not
            // parse is not the same as one that defines none.
            Err(_) => unreadable += 1,
        }
    }
    (found, unreadable)
}

/// Whether `span` resolves into one of the `system` include directories.
///
/// **Compared over path components, not as a string prefix.** `/usr/includes-mine/x.h` starts
/// with `/usr/include` and is not inside it; `Path::starts_with` is component-wise and
/// `str::starts_with` is not.
///
/// An empty `system` list suppresses nothing: a sweep run where gcc could not be found must
/// not quietly drop findings, the same reason `Outcome::NotRun` never counts as agreement.
pub fn in_system_header(
    map: &chiero_span::SourceMap,
    span: chiero_span::Span,
    system: &[PathBuf],
) -> bool {
    if span.is_dummy() || system.is_empty() {
        return false;
    }
    let Some(loc) = map.lookup_loc(span.lo) else {
        return false;
    };
    let Some(file) = map.try_file(loc.file) else {
        return false;
    };
    let path = file.path();
    system.iter().any(|dir| path.starts_with(dir))
}

/// One file's verdict.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub path: PathBuf,
    pub bucket: Bucket,
    pub gcc: Outcome,
    pub chiero: Outcome,
}

/// Sweep a tree and return one verdict per translation unit.
pub fn sweep(tree: &Path, flags: &Flags, system: &[PathBuf]) -> std::io::Result<Vec<Verdict>> {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    sweep_with(tree, flags, system, threads)
}

/// As [`sweep`], with an explicit worker count.
///
/// **The verdict list is an ordered report and stays in walk order.** Each worker takes a
/// contiguous run of files and returns its own verdicts; concatenating the chunks in chunk
/// order reproduces the serial sequence exactly. A work queue would return verdicts in
/// completion order, and 001 §5 forbids an output that depends on scheduling.
pub fn sweep_with(
    tree: &Path,
    flags: &Flags,
    system: &[PathBuf],
    threads: usize,
) -> std::io::Result<Vec<Verdict>> {
    let files = translation_units(tree)?;
    // Hoisted out of the workers: `gcc -dM` is a subprocess, and running it once per thread
    // would add a process launch per chunk for an answer that cannot differ between them.
    let predefines = gcc_predefines(flags.std.as_deref());
    let threads = threads.max(1);
    // **The empty tree is checked as an empty tree, not as `per == 0`.** With `div_ceil` the
    // chunk size is zero only when there are no files, so the guard was stating a consequence
    // rather than the condition — and a chunk size computed any other way would silently
    // return "no files swept" for a tree full of them. `chunks(0)` panics, which is the right
    // failure for arithmetic that cannot be right.
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let per = files.len().div_ceil(threads);

    let chunks: Vec<&[PathBuf]> = files.chunks(per).collect();
    let results: Vec<Vec<Verdict>> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let predefines = &predefines;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|path| {
                            let gcc = gcc_outcome(path, flags);
                            let chiero = chiero_outcome(path, flags, system, predefines);
                            Verdict {
                                bucket: classify(&gcc, &chiero),
                                path: path.clone(),
                                gcc,
                                chiero,
                            }
                        })
                        .collect()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });

    Ok(results.into_iter().flatten().collect())
}

/// Print the report: counts, then the queue.
///
/// **The queue is the point** (023 §9: a report a person cannot act on is not a report). A bare
/// count of findings says a number; grouping by *distinct message* with an example file turns it
/// into work, and the grouping is what shows that fifty files often share one defect.
pub fn report(verdicts: &[Verdict], tree: &Path) {
    let count = |b: Bucket| verdicts.iter().filter(|v| v.bucket == b).count();
    println!(
        "swept {} translation units under {}",
        verdicts.len(),
        tree.display()
    );
    println!(
        "  findings (gcc ok, chiero complained): {}",
        count(Bucket::Finding)
    );
    println!(
        "  misses   (gcc refused, chiero silent): {}",
        count(Bucket::Miss)
    );
    println!(
        "  agree, both clean:                     {}",
        count(Bucket::Agree)
    );
    println!(
        "  both refused (usually wrong flags):    {}",
        count(Bucket::BothRefused)
    );
    println!(
        "  severity mismatch (gcc warned):        {}",
        count(Bucket::SeverityMismatch)
    );
    println!(
        "  tool gaps (one side could not run):    {}",
        count(Bucket::ToolGap)
    );
    // **What was actually tested.** A sweep where gcc refused everything has findings of zero
    // and has learned nothing; saying so here is the difference between a report and a number.
    let tested = count(Bucket::Agree) + count(Bucket::Finding);
    println!(
        "  -> gcc accepted {tested} of {}, so that is what this sweep could test",
        verdicts.len()
    );

    // 080 M3 exit gate: the parser-coverage percentage, published on every run so it is
    // tracked rather than recomputed by hand when someone remembers to ask.
    let c = coverage(verdicts);
    println!(
        "\nCHIERO REACH — {} preprocessed, {} parsed, {} analysed, of {} files",
        c.preprocessed, c.parsed, c.analysed, c.total
    );
    println!(
        "  -> parser coverage {:.1}% of the {} translation units the parser was handed",
        c.parser_percent(),
        c.preprocessed
    );

    for (title, bucket, side) in [
        (
            "FINDINGS — chiero complains where gcc is happy",
            Bucket::Finding,
            true,
        ),
        (
            "MISSES — gcc refuses where chiero is silent",
            Bucket::Miss,
            false,
        ),
        (
            "BOTH REFUSED — usually the flags, not the code",
            Bucket::BothRefused,
            false,
        ),
        (
            "SEVERITY MISMATCH — gcc warned, chiero refused; both saw it",
            Bucket::SeverityMismatch,
            true,
        ),
        ("TOOL GAPS", Bucket::ToolGap, true),
    ] {
        let mut groups: indexmap::IndexMap<String, (usize, PathBuf, String)> =
            indexmap::IndexMap::new();
        for v in verdicts.iter().filter(|v| v.bucket == bucket) {
            let msg = match if side { &v.chiero } else { &v.gcc } {
                Outcome::Diagnosed(m) | Outcome::NotRun(m) | Outcome::Warned(m) => m.clone(),
                Outcome::Clean => continue,
            };
            // Group by kind; keep the first located text as the example.
            let e = groups
                .entry(kind(&msg))
                .or_insert((0, v.path.clone(), msg.clone()));
            e.0 += 1;
        }
        if groups.is_empty() {
            continue;
        }
        let mut rows: Vec<_> = groups.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1.0));
        println!("\n{title}");
        for (msg, (n, example, located)) in rows.iter().take(25) {
            println!("  {n:5}  {msg}");
            // The example is the *located* text when there was a location to render, so the
            // reader has a place to open; otherwise the file is all we can offer, and offering
            // the file is still better than offering nothing.
            if located == msg {
                println!("         e.g. {}", example.display());
            } else {
                println!("         e.g. {located}");
            }
        }
        if rows.len() > 25 {
            println!("  … {} more distinct messages", rows.len() - 25);
        }
    }
}

/// How far chiero got, per translation unit (080 M3 exit gate).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    pub total: usize,
    /// Got past the preprocessor — i.e. the parser was handed something.
    pub preprocessed: usize,
    /// Got past the parser.
    pub parsed: usize,
    /// Got past sema with nothing to say.
    pub analysed: usize,
}

impl Coverage {
    /// Percentage of the translation units **the parser was handed** that it parsed, to one
    /// decimal place.
    ///
    /// The denominator is `preprocessed`, not `total`. Dividing by everything charges the
    /// parser for headers the sweep could not resolve, and makes the published number move
    /// when the include flags change rather than when the parser does — which would make it
    /// untrackable across waves, the one thing a published metric must not be.
    pub fn parser_percent(&self) -> f64 {
        if self.preprocessed == 0 {
            return 0.0;
        }
        let raw = self.parsed as f64 * 100.0 / self.preprocessed as f64;
        (raw * 10.0).round() / 10.0
    }
}

/// Classify each verdict by the furthest stage chiero reached.
///
/// **This measures "got through without a diagnostic", not "the parser understood it".** A
/// construct the parser reads correctly and then refuses on a constraint — VPP's extra
/// semicolon in a struct, legal under `gnu11` and refused under `-pedantic-errors` — counts
/// here as not parsed. So the published figure is bounded below by the project's calibration,
/// and the owner's open pedantic-mode decision moves it. Read a shortfall as a lead to
/// investigate, never as a count of parser gaps.
///
/// **Reads the `pp:`/`parse:`/`sema:` prefix this module itself writes.** The alternative is
/// threading a stage through `Outcome`, which would put a field on every construction site
/// including the synthetic ones in tests; the prefix is produced in exactly one place, a few
/// lines above, and is already the grouping key.
pub fn coverage(verdicts: &[Verdict]) -> Coverage {
    let mut c = Coverage {
        total: verdicts.len(),
        ..Coverage::default()
    };
    for v in verdicts {
        let stage = match &v.chiero {
            // Clean means it went all the way through.
            Outcome::Clean => 3,
            // A warning means the file compiled, so every stage was reached.
            Outcome::Warned(_) => 3,
            Outcome::Diagnosed(m) | Outcome::NotRun(m) => {
                if m.starts_with("pp:") {
                    0
                } else if m.starts_with("parse:") {
                    1
                } else if m.starts_with("sema:") {
                    2
                } else {
                    // Not one of ours — an unreadable file, say. The parser was never handed
                    // it, so it counts against nothing rather than being charged to a stage.
                    0
                }
            }
        };
        if stage >= 1 {
            c.preprocessed += 1;
        }
        if stage >= 2 {
            c.parsed += 1;
        }
        if stage >= 3 {
            c.analysed += 1;
        }
    }
    c
}

/// gcc's own system include paths, so chiero resolves `<stdio.h>` as the tree's compiler does.
///
/// Empty when gcc cannot be run: the sweep then reports every file as a tool gap rather than
/// pretending the headers were not needed.
pub fn system_include_paths() -> Vec<PathBuf> {
    let Ok(out) = std::process::Command::new("gcc")
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

/// gcc's **predefined macros**, so chiero takes the same `#if` branches the tree's compiler does.
///
/// Without these the sweep is not measuring chiero against gcc at all: glibc's `bits/floatn.h`
/// alone branches on a dozen `__HAVE_FLOAT*` and `__FLT16_*` macros, and a preprocessor that
/// lacks them compiles code gcc never sees. The first run of this tool reported 101 findings that
/// were entirely this — a reminder that a sweep's own configuration is part of its correctness.
///
/// Function-like macros and the ones the preprocessor must own (`__FILE__`, `__LINE__`, …) are
/// dropped, matching the sema harness.
pub fn gcc_predefines(std: Option<&str>) -> Vec<(String, String)> {
    let dialect = format!("-std={}", std.unwrap_or("gnu11"));
    let Ok(out) = std::process::Command::new("gcc")
        .args(["-dM", "-E", &dialect, "-x", "c", "/dev/null"])
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
