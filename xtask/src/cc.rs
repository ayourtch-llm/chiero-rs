//! The `CC=` shim: chiero observing a real build (see `xtask/tests/cc.rs`).
//!
//! **It delegates and observes.** The real compiler runs with the same arguments and its exit
//! status is returned unchanged, so nothing chiero does can break a build. Its value over the
//! standalone sweep is the flags: a build knows its own `-I` set and has already generated the
//! headers it includes, both of which the sweep can only guess at.

use std::path::{Path, PathBuf};

/// Options that consume the following argument, whose value must not be mistaken for a source.
///
/// `-o out.c` names an output and `-include prefix.c` a prefix header; a scan for `.c` endings
/// takes both and analyses files the build is not compiling.
const TAKES_VALUE: &[&str] = &[
    "-o",
    "-I",
    "-D",
    "-U",
    "-include",
    "-imacros",
    "-isystem",
    "-iquote",
    "-idirafter",
    "-MF",
    "-MT",
    "-MQ",
    "-x",
    "-Xpreprocessor",
    "-Xlinker",
    "-l",
    "-L",
    "-B",
    "-aux-info",
];

/// Modes that ask the compiler for something other than a translation.
const NOT_A_COMPILATION: &[&str] = &[
    "-E",
    "-M",
    "-MM",
    "--version",
    "-v",
    "-dumpversion",
    "-dumpmachine",
    "-print-search-dirs",
];

/// The C sources this invocation compiles; empty when it compiles none.
///
/// Empty means "delegate and record nothing", which is the safe answer: a build makes many
/// calls that are not compilations, and analysing one of them reports against a file the build
/// never translated.
pub fn sources_to_analyse(args: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for (i, a) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if NOT_A_COMPILATION.iter().any(|m| a == m) || a.starts_with("-print-") {
            return Vec::new();
        }
        if TAKES_VALUE.iter().any(|m| a == m) {
            skip_next = true;
            continue;
        }
        // `-I/inc` and `-DX=1` carry their value in the same argument.
        if a.starts_with('-') {
            continue;
        }
        let _ = i;
        if Path::new(a).extension().is_some_and(|e| e == "c") {
            out.push(PathBuf::from(a));
        }
    }
    out
}

/// One observed translation unit, as a JSON line.
///
/// Deliberately flat text rather than a serde structure: the shim runs once per compilation in
/// somebody else's build, and a record it cannot write must never be a reason that build fails.
pub fn record_line(source: &Path, outcome: &crate::sweep::Outcome, millis: u128) -> String {
    let (status, message) = match outcome {
        crate::sweep::Outcome::Clean => ("clean", String::new()),
        crate::sweep::Outcome::Warned(m) => ("warned", m.clone()),
        crate::sweep::Outcome::Diagnosed(m) => ("diagnosed", m.clone()),
        crate::sweep::Outcome::NotRun(m) => ("not-run", m.clone()),
    };
    format!(
        "{{\"file\":{:?},\"status\":\"{status}\",\"ms\":{millis},\"message\":{:?}}}",
        source.display().to_string(),
        message
    )
}

/// Flags for chiero, read from the compiler's own argument list.
///
/// **This is the point of the shim.** The sweep guesses `-I` paths and stubs generated headers;
/// a build states its include set exactly and has already generated what it includes.
pub fn flags_from_args(args: &[String], dialect: chiero_ast::Dialect) -> crate::sweep::Flags {
    let mut f = crate::sweep::Flags {
        dialect,
        machine: Vec::new(),
        includes: Vec::new(),
        defines: Vec::new(),
        std: None,
    };
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("-I") {
            if v.is_empty() {
                if let Some(n) = it.next() {
                    f.includes.push(PathBuf::from(n));
                }
            } else {
                f.includes.push(PathBuf::from(v));
            }
        } else if let Some(v) = a.strip_prefix("-D") {
            if v.is_empty() {
                if let Some(n) = it.next() {
                    f.defines.push(n.clone());
                }
            } else {
                f.defines.push(v.to_owned());
            }
        } else if let Some(v) = a.strip_prefix("--std=").or_else(|| a.strip_prefix("-std=")) {
            f.std = Some(v.to_owned());
        } else if a.starts_with("-m") && !a.starts_with("-M") {
            // **`-m…` but not `-M…`.** `-MD`, `-MF`, `-MT` are dependency options; handing
            // one to `gcc -dM` makes it write a dependency file instead of predefines.
            f.machine.push(a.clone());
        }
    }
    f
}

/// Run as `CC`: observe, then hand over to the real compiler.
///
/// **Observation comes first**, and not only for tidiness: handing over is a process
/// replacement, so there is no "after". Running first also means a translation unit is recorded
/// even when the real compiler then rejects it, which is exactly the case worth having.
///
/// Returns the real compiler's exit code, always. A build that fails because of the observer is
/// a build that will stop using the observer.
pub fn run(args: &[String]) -> i32 {
    let real = std::env::var("CHIERO_REAL_CC").unwrap_or_else(|_| "cc".to_owned());
    let out = std::env::var("CHIERO_CC_LOG").ok();

    // **Every failure here is swallowed.** A panic, an unreadable file, a missing log directory
    // — none of them may cost the build. `catch_unwind` because a frontend bug on somebody
    // else's source is a real possibility and must not surface as a compiler crash.
    // **A sidecar per translation unit**, keyed on the output it describes. One writer per
    // file by construction, so a parallel build needs no locking and no atomic-append
    // discipline. `CHIERO_CC_LOG` remains for a caller that wants everything in one stream.
    let recorded = std::panic::catch_unwind(|| observe_with_paths(args));
    if let Ok(records) = recorded {
        for (path, line) in records {
            write_sidecar(&path, &line);
            if let Some(log) = &out {
                append_record(std::path::Path::new(log), &line);
            }
        }
    }

    std::process::Command::new(&real)
        .args(args)
        .status()
        .map_or(127, |s| s.code().unwrap_or(1))
}

/// Analyse whatever this invocation compiles, returning one record per translation unit.
fn observe_with_paths(args: &[String]) -> Vec<(PathBuf, String)> {
    let sources = sources_to_analyse(args);
    if sources.is_empty() {
        return Vec::new();
    }
    // The build's own dialect: it is compiling with GNU extensions, so judging it against
    // `-pedantic-errors` would report what its compiler accepts. `CHIERO_CC_PEDANTIC=1` asks
    // the other question.
    let dialect = if std::env::var("CHIERO_CC_PEDANTIC").is_ok() {
        chiero_ast::Dialect::pedantic()
    } else {
        chiero_ast::Dialect::gnu()
    };
    let flags = flags_from_args(args, dialect);
    let system = crate::sweep::system_include_paths();
    let predefines = crate::sweep::gcc_predefines_with(flags.std.as_deref(), &flags.machine);
    sources
        .iter()
        .map(|src| {
            let started = std::time::Instant::now();
            let outcome = crate::sweep::chiero_outcome(src, &flags, &system, &predefines);
            let line = record_line(src, &outcome, started.elapsed().as_millis());
            (sidecar_path(args, src), line)
        })
        .collect()
}

/// Append one record, as a **single** write to an `O_APPEND` handle.
///
/// `make -j` runs many compilers at once against one log. Two writes per record — the line,
/// then the newline — let another process interleave between them, which does not reorder the
/// records but *tears* them: the file stops being JSONL and both writers lose their entry. One
/// write, and the kernel serialises the append.
///
/// Every failure is silent. This runs inside somebody else's build, and a log that cannot be
/// written must never be the reason it fails.
pub fn append_record(log: &Path, line: &str) {
    use std::io::Write;
    let mut whole = String::with_capacity(line.len() + 1);
    whole.push_str(line);
    whole.push('\n');
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let _ = f.write_all(whole.as_bytes());
    }
}

/// What a collected log says, in the shape the sweep's report uses.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    pub total: usize,
    pub clean: usize,
    /// `(kind, count)`, commonest first. Locations are stripped so two files sharing a defect
    /// are one row — the same grouping the sweep needs, for the same reason.
    pub kinds: Vec<(String, usize)>,
}

/// Summarise collected records.
///
/// Reads the fields it needs out of each line rather than parsing JSON: the writer is three
/// lines above, the format is fixed, and a dependency for this would be the tail wagging the
/// dog. A line it cannot read is skipped rather than fatal — a torn log should still be
/// partially useful.
pub fn summarise(lines: &[String]) -> Summary {
    let mut s = Summary::default();
    let mut counts: indexmap::IndexMap<String, usize> = indexmap::IndexMap::new();
    for l in lines {
        let Some(status) = between(l, "\"status\":\"", "\"") else {
            continue;
        };
        s.total += 1;
        if status == "clean" {
            s.clean += 1;
            continue;
        }
        if let Some(msg) = between(l, "\"message\":\"", "\"}") {
            *counts.entry(crate::sweep::kind(&msg)).or_default() += 1;
        }
    }
    let mut kinds: Vec<(String, usize)> = counts.into_iter().collect();
    kinds.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    s.kinds = kinds;
    s
}

fn between(s: &str, open: &str, close: &str) -> Option<String> {
    let i = s.find(open)? + open.len();
    let j = s[i..].find(close)? + i;
    Some(s[i..j].to_owned())
}

/// Where the record for `source` in this invocation goes.
///
/// `<output>.chiero` when the invocation names one output for one source; `<source>.chiero`
/// otherwise. Keyed on the output because VPP compiles one `.c` into several objects with
/// different `-march` flags, and source-keyed records would overwrite each other silently.
pub fn sidecar_path(args: &[String], source: &Path) -> PathBuf {
    let sources = sources_to_analyse(args);
    let output = output_path(args);
    match output {
        // One source, one output: the output names this translation unit uniquely.
        Some(o) if sources.len() == 1 => PathBuf::from(format!("{}.chiero", o.display())),
        // A link, or no output named. The source is the only name that distinguishes them.
        _ => PathBuf::from(format!("{}.chiero", source.display())),
    }
}

fn output_path(args: &[String]) -> Option<PathBuf> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-o" {
            return it.next().map(PathBuf::from);
        }
        if let Some(v) = a.strip_prefix("-o")
            && !v.is_empty()
        {
            return Some(PathBuf::from(v));
        }
    }
    None
}

/// Write one record, replacing any previous one for the same output.
///
/// A plain create-truncate write: one writer per file by construction, so there is nothing to
/// serialise. Silent on failure — this runs inside somebody else's build.
pub fn write_sidecar(path: &Path, line: &str) {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{line}\n"));
}

/// Collect every `*.chiero` record under `root`.
pub fn collect_sidecars(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "chiero")
                && let Ok(t) = std::fs::read_to_string(&p)
            {
                out.extend(t.lines().filter(|l| !l.is_empty()).map(str::to_owned));
            }
        }
    }
    // Sorted, so two runs over one tree produce the same report and can be diffed (001 §5).
    out.sort();
    out
}
