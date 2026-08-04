//! The `CC=` shim: chiero observing a real build.
//!
//! A build system invokes the compiler for many things that are not compilations — linking,
//! `--version`, dependency generation, `configure`'s throwaway probes. Getting this wrong is
//! not a missed measurement but a **broken build**, so the classification is the part worth
//! testing hardest.

use std::path::{Path, PathBuf};
use xtask::cc::sources_to_analyse;

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn a_compilation_is_told_from_everything_else() {
    // The ordinary case, and the one that matters.
    assert_eq!(
        sources_to_analyse(&args("-c foo.c -o foo.o -I/inc -DX=1")),
        vec![PathBuf::from("foo.c")]
    );

    // Compile-and-link in one step: still a compilation of `main.c`. `configure` does this
    // constantly, and skipping it would lose the probes that reveal a project's real flags.
    assert_eq!(
        sources_to_analyse(&args("main.c -o prog -lm")),
        vec![PathBuf::from("main.c")]
    );

    // Several sources in one invocation are several compilations.
    assert_eq!(
        sources_to_analyse(&args("-c a.c b.c")),
        vec![PathBuf::from("a.c"), PathBuf::from("b.c")]
    );

    // **Not compilations.** Linking has no source; `-E`, `-M` and `-MM` ask the compiler for
    // something other than a translation; `--version` is a probe. Analysing any of these
    // wastes time at best, and at worst reports a diagnostic for a file the build never
    // compiled.
    for not in [
        "-o prog a.o b.o -lm",
        "-E foo.c",
        "-M foo.c",
        "-MM foo.c",
        "--version",
        "-print-file-name=include",
    ] {
        assert!(
            sources_to_analyse(&args(not)).is_empty(),
            "not a compilation: `{not}` -> {:?}",
            sources_to_analyse(&args(not))
        );
    }

    // **An argument's *value* is not a source.** `-o out.c` names an output and `-include x.c`
    // names a prefix header; treating either as a translation unit would analyse a file the
    // build is not compiling.
    assert!(sources_to_analyse(&args("-c q.S -o out.c")).is_empty());
    assert_eq!(
        sources_to_analyse(&args("-c real.c -include prefix.c")),
        vec![PathBuf::from("real.c")]
    );
}

/// **The flags come from the build, which is the whole point.** The sweep guesses them; a
/// compiler invocation states them.
#[test]
fn flags_are_read_from_the_compilers_own_arguments() {
    let f = xtask::cc::flags_from_args(
        &args("-c foo.c -I/a -I /b -DX=1 -D Y -std=gnu11 -O2"),
        chiero_ast::Dialect::gnu(),
    );
    assert_eq!(f.includes, [PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(f.defines, ["X=1", "Y"]);
    assert_eq!(f.std.as_deref(), Some("gnu11"));
}

/// A record is one JSON line per translation unit, and survives a message containing quotes —
/// diagnostics are full of them.
#[test]
fn a_record_is_one_json_line() {
    use xtask::sweep::Outcome;
    let line = xtask::cc::record_line(
        Path::new("/x/a.c"),
        &Outcome::Diagnosed("sema: `x` was not declared".into()),
        12,
    );
    assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
    assert!(line.contains("\"status\":\"diagnosed\""), "{line}");
    assert!(line.contains("\\`x\\`") || line.contains("`x`"), "{line}");
    assert!(!line.contains('\n'), "one line: {line}");
}

/// **Records must survive a parallel build.** `make -j` runs many compilers at once, all
/// appending to one log. A record written as two calls — the line, then the newline — can be
/// interleaved with another process's line, producing a file that is not JSONL and losing both
/// records. One `write` per record, opened `O_APPEND`, is what makes concurrent appends safe.
#[test]
fn concurrent_writers_produce_whole_lines() {
    let log = std::env::temp_dir().join("chiero-cc-parallel.jsonl");
    let _ = std::fs::remove_file(&log);

    let mut hs = Vec::new();
    for w in 0..8 {
        let log = log.clone();
        hs.push(std::thread::spawn(move || {
            for i in 0..40 {
                let line = xtask::cc::record_line(
                    &PathBuf::from(format!("w{w}/f{i}.c")),
                    &xtask::sweep::Outcome::Diagnosed(format!("sema: worker {w} item {i}")),
                    i,
                );
                xtask::cc::append_record(&log, &line);
            }
        }));
    }
    for h in hs {
        h.join().expect("worker");
    }

    let text = std::fs::read_to_string(&log).expect("log");
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 320, "every record present exactly once");
    for l in &lines {
        assert!(
            l.starts_with('{') && l.ends_with('}'),
            "a line was torn by a concurrent writer: {l}"
        );
    }
}

/// **The point of collecting is reading it back.** A summary over the log, grouped the way the
/// sweep groups: by kind, with a count, so 400 records become a handful of rows.
#[test]
fn the_log_summarises_by_kind() {
    let lines = vec![
        xtask::cc::record_line(&PathBuf::from("a.c"), &xtask::sweep::Outcome::Clean, 1),
        xtask::cc::record_line(
            &PathBuf::from("b.c"),
            &xtask::sweep::Outcome::Diagnosed("sema: /b.c:1:1: `x` was not declared".into()),
            2,
        ),
        xtask::cc::record_line(
            &PathBuf::from("c.c"),
            &xtask::sweep::Outcome::Diagnosed("sema: /c.c:9:9: `y` was not declared".into()),
            3,
        ),
    ];
    let s = xtask::cc::summarise(&lines);
    assert_eq!(s.total, 3);
    assert_eq!(s.clean, 1);
    // Two files, one kind: locations differ and must not split the row.
    assert_eq!(s.kinds.len(), 1, "{:?}", s.kinds);
    assert_eq!(s.kinds[0].1, 2);
    assert!(s.kinds[0].0.contains("was not declared"), "{:?}", s.kinds);
}
