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
        includes: Vec::new(),
        defines: Vec::new(),
        std: None,
    };
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("-I") {
            if v.is_empty() {
                it.next().map(|n| f.includes.push(PathBuf::from(n)));
            } else {
                f.includes.push(PathBuf::from(v));
            }
        } else if let Some(v) = a.strip_prefix("-D") {
            if v.is_empty() {
                it.next().map(|n| f.defines.push(n.clone()));
            } else {
                f.defines.push(v.to_owned());
            }
        } else if let Some(v) = a.strip_prefix("--std=").or_else(|| a.strip_prefix("-std=")) {
            f.std = Some(v.to_owned());
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
    if let Some(path) = out {
        let recorded = std::panic::catch_unwind(|| observe(args));
        if let Ok(lines) = recorded
            && !lines.is_empty()
            && let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(lines.join("\n").as_bytes());
            let _ = f.write_all(b"\n");
        }
    }

    std::process::Command::new(&real)
        .args(args)
        .status()
        .map_or(127, |s| s.code().unwrap_or(1))
}

/// Analyse whatever this invocation compiles, returning one record per translation unit.
fn observe(args: &[String]) -> Vec<String> {
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
    let predefines = crate::sweep::gcc_predefines(flags.std.as_deref());
    sources
        .iter()
        .map(|src| {
            let started = std::time::Instant::now();
            let outcome = crate::sweep::chiero_outcome(src, &flags, &system, &predefines);
            record_line(src, &outcome, started.elapsed().as_millis())
        })
        .collect()
}
