//! The `chiero` binary — [050 §1](../../../docs/specs/050-tool-interface.md).
//!
//! > `chiero-cli` is a thin wrapper over the identical operations the MCP server exposes.
//!
//! **Thin is the requirement, not an aspiration.** Every subcommand does the same three
//! things: turn arguments into inputs, call exactly one `chiero_tool` operation, print the
//! envelope. Nothing here decides anything about the code — a second place that judged
//! fidelity, or summarised a result, would be a second implementation of the one thing in this
//! system that must not have two.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod frontend;

use frontend::{Frontend, lower};

const USAGE: &str = "\
chiero — a symbolic C execution environment

USAGE:
    chiero <operation> [args] [options]

OPERATIONS:
    prove-equivalent <before.c> <after.c> --entry <fn>
            Adjudicate a rewrite. Either a proof that the two agree for every
            input, or a concrete input at which they do not.  (041 §1)

    find-bugs <file.c> --entry <fn>
            Run 040's defect checkers from a function. An empty list is an
            answer only when the envelope says the search finished.  (050 §3)

    check-reachable <file.c> --entry <fn> --line <n>
            Can execution get to that line? Proved-nothing-does and
            chiero-did-not are different answers, and it says which.  (050 §3)

    layout <file.c> [--cache-line <n>]
            Cache-line and padding analysis of the structs in a translation
            unit. Proposals only — nothing is ever rewritten.  (041 §3)

    find-optimizations <file.c> --entry <fn>
            Proposals with obligations and benefit labels. Never rewrites
            anything.  (041 §2)

    impact <before.c> <after.c>
            What a source change reaches — through calls, types, globals and
            macro expansions.  (031)

    select-tests <before.c> <after.c> --coverage <dir> --stem <name>
            Which tests are worth running for that change, ranked, with the
            reason for each.  (032)

    expansion-sites <file.c> --macro <NAME> [--cursor <n>] [--limit <n>]
            Every place a macro expands in this translation unit.  (050 §3)

    cir <file.c> [--entry <fn>]
            Print the lowered module in 020's normative textual format. The
            answer is about chiero rather than about your program, so it
            carries no envelope -- it round-trips instead.

    explain-macro <file.c> --line <n> [--col <n>]
            What macro chain produced the code on a line, innermost first.

OPTIONS:
    --cache-line <n>
                    Cache-line size in bytes for `layout`. Default 64.
    --json          Print the envelope as JSON. Default is a human rendering.
    --replay        Emit a C harness demonstrating a `differs` verdict.
    --allow-replay-exec
                    Compile and run that harness. Off by default: this builds
                    and executes code, so a caller has to ask.  (050 §6)
    -I <dir>        Add an include path. Repeatable.
    --no-system-headers
                    Do not ask the C compiler where its own headers are. On by
                    default, because real C includes <stdio.h>.
    --report-invented-bounds
                    Show bounds findings against the object chiero invents behind
                    an entry pointer. Off by default: chiero knows neither the
                    caller's object size nor where in it the pointer points, so
                    those say nothing about your program. The count is always
                    reported, shown or not.
    --time-budget <secs>
                    Stop `find-bugs` and `check-reachable` after that many seconds
                    and print what was found so far. Decimals allowed; 0 means no
                    limit, as in timeout(1). Default 60. A run that ends here is
                    marked `nondeterministic_abort`: where it stopped depends on
                    the machine, so the answer is a measurement.  (023 §8.1)
    --solver-rlimit <units>
                    Stop any single solver query after that many of z3's work
                    units. 0 (the default) is no limit. Unlike --time-budget this
                    is deterministic — work units do not move with machine speed
                    or thread count — so a run cut by it is an ordinary answer
                    rather than a measurement, and it is the only bound that
                    reaches inside one long query.  (023 §8)
    --entry-ptr-nonnull
                    Assume the pointer parameters of --entry are not null. For a
                    helper whose callers check, the null path is one the program
                    does not have. Removes real paths, so it is recorded as an
                    assumption in the envelope.
    -D <k[=v]>      Define a macro. Repeatable.
    -h, --help      This text.

Every operation prints an ENVELOPE: the result, plus `fidelity`, `proven`,
`assumptions` and `blind_spots`. `proven` is true only when the answer holds for
all inputs. An empty result is not the same as a clean one — see
docs/tutorials/05-envelope.md.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            // **A closed pipe is not an error.** `println!` panics on `EPIPE`, so
            // `chiero cir big.c | head` answered a routine `head` with a rustc-internal
            // path and a backtrace note, and exited 101. Every operation prints an
            // envelope and a VPP TU's is megabytes, so `| head`, `| less` and `| grep -m1`
            // are how these are read.
            //
            // Written by hand rather than fixed with `signal(SIGPIPE, SIG_DFL)`, which
            // would need `libc`: 001 §4 keeps this tree linking nothing, and one match arm
            // is a smaller price than a dependency for a behaviour this local.
            use std::io::Write as _;
            let mut out = std::io::stdout().lock();
            match writeln!(out, "{text}").and_then(|()| out.flush()) {
                Ok(()) => ExitCode::SUCCESS,
                // The reader left. That is the reader's business and not a failure of
                // the analysis, which had already finished.
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("chiero: writing the result: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(Fault::Usage(m)) => {
            eprintln!("chiero: {m}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Fault::Failed(m)) => {
            eprintln!("chiero: {m}");
            ExitCode::FAILURE
        }
    }
}

/// **Two kinds of failure, because they mean different things to whoever is reading.**
/// `Usage` is "you asked for something I do not offer" and reprints the operations; `Failed`
/// is "I tried and could not", which is about the input rather than the request.
enum Fault {
    Usage(String),
    Failed(String),
}

fn run(args: &[String]) -> Result<String, Fault> {
    if args.is_empty() {
        return Err(Fault::Usage("no operation given".into()));
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(USAGE.to_string());
    }

    let opts = Options::parse(&args[1..])?;
    // **Not an envelope, and deliberately so.** Every other operation answers a question about a
    // program and carries 050's fidelity/assumptions with it. This one answers a question about
    // *chiero* — "what did lowering produce?" — and the honest form of that answer is 020's
    // normative text, which round-trips. Wrapping it in an envelope would attach a fidelity to a
    // dump, which means nothing.
    if args[0] == "cir" {
        return cir(&opts);
    }
    let env = match args[0].as_str() {
        "prove-equivalent" => prove_equivalent(&opts)?,
        "find-bugs" => find_bugs(&opts)?,
        "check-reachable" => check_reachable(&opts)?,
        "find-optimizations" => find_optimizations(&opts)?,
        "layout" => layout(&opts)?,
        "impact" => impact(&opts)?,
        "select-tests" => select_tests(&opts)?,
        "expansion-sites" => expansion_sites(&opts)?,
        "explain-macro" => explain_macro(&opts)?,
        other => return Err(Fault::Usage(format!("unknown operation `{other}`"))),
    };
    Ok(if opts.json {
        env.to_json()
    } else {
        env.render()
    })
}

/// Everything the operations take, parsed once.
///
/// Hand-rolled rather than a parser crate: 010 §1's build rule keeps the dependency list to
/// what the analysis actually needs, and this is a dozen flags.
#[derive(Debug, Default)]
struct Options {
    positional: Vec<String>,
    entry: Option<String>,
    macro_name: Option<String>,
    line: Option<u32>,
    cache_line: Option<u64>,
    col: Option<u32>,
    cursor: Option<usize>,
    limit: Option<usize>,
    coverage: Option<PathBuf>,
    stem: Option<String>,
    includes: Vec<PathBuf>,
    defines: Vec<(String, String)>,
    json: bool,
    no_system_headers: bool,
    /// Target flags for the compiler persona — `--march x86-64-v2`, or `-m<flag>` passed through.
    ///
    /// **Not decoration.** `__SSE4_2__` and `__AVX2__` exist only under the right `-march`, so
    /// probing the compiler with none of them predefines a different compiler than the one the
    /// code is built with. Every AVX2 path in VPP's vppinfra had never once been compiled by a
    /// chiero measurement because of this.
    target_flags: Vec<String>,
    replay: bool,
    allow_replay_exec: bool,
    entry_ptr_nonnull: bool,
    report_invented_bounds: bool,
    /// `None` is "the caller said nothing" and takes [`Options::wall_clock`]'s default; an
    /// explicit `0` is "no limit", which is a different thing and has to stay tellable apart.
    time_budget: Option<f64>,
    /// 023 §8's deterministic solver budget, in z3 work units. `0` — and the default — is no
    /// limit, so `Option` buys nothing here: unlike the clock, saying nothing and saying zero
    /// mean the same run, and the test that pins that is the point of the field.
    solver_rlimit: u64,
}

fn define(d: &str) -> (String, String) {
    match d.split_once('=') {
        Some((k, v)) => (k.to_string(), v.to_string()),
        None => (d.to_string(), "1".to_string()),
    }
}

fn need(i: usize, args: &[String], what: &str) -> Result<String, Fault> {
    args.get(i + 1)
        .cloned()
        .ok_or_else(|| Fault::Usage(format!("{what} needs a value")))
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, Fault> {
        let mut o = Options::default();
        let mut i = 0;
        while i < args.len() {
            let a = args[i].clone();
            match a.as_str() {
                "--json" => o.json = true,
                "--no-system-headers" => o.no_system_headers = true,
                "--march" => {
                    o.target_flags
                        .push(format!("-march={}", need(i, args, "--march")?));
                    i += 1;
                }
                "--entry-ptr-nonnull" => o.entry_ptr_nonnull = true,
                "--report-invented-bounds" => o.report_invented_bounds = true,
                "--replay" => o.replay = true,
                "--allow-replay-exec" => {
                    o.replay = true;
                    o.allow_replay_exec = true;
                }
                "--time-budget" => {
                    o.time_budget = Some(secs(&need(i, args, "--time-budget")?)?);
                    i += 1;
                }
                "--solver-rlimit" => {
                    o.solver_rlimit = units(&need(i, args, "--solver-rlimit")?)?;
                    i += 1;
                }
                "--entry" => {
                    o.entry = Some(need(i, args, "--entry")?);
                    i += 1;
                }
                "--macro" => {
                    o.macro_name = Some(need(i, args, "--macro")?);
                    i += 1;
                }
                "--cache-line" => {
                    o.cache_line = Some(u64::from(num(
                        &need(i, args, "--cache-line")?,
                        "--cache-line",
                    )?));
                    i += 1;
                }
                "--line" => {
                    o.line = Some(num(&need(i, args, "--line")?, "--line")?);
                    i += 1;
                }
                "--col" => {
                    o.col = Some(num(&need(i, args, "--col")?, "--col")?);
                    i += 1;
                }
                "--cursor" => {
                    o.cursor = Some(num(&need(i, args, "--cursor")?, "--cursor")? as usize);
                    i += 1;
                }
                "--limit" => {
                    o.limit = Some(num(&need(i, args, "--limit")?, "--limit")? as usize);
                    i += 1;
                }
                "--coverage" => {
                    o.coverage = Some(PathBuf::from(need(i, args, "--coverage")?));
                    i += 1;
                }
                "--stem" => {
                    o.stem = Some(need(i, args, "--stem")?);
                    i += 1;
                }
                "-I" => {
                    o.includes.push(PathBuf::from(need(i, args, "-I")?));
                    i += 1;
                }
                "-D" => {
                    o.defines.push(define(&need(i, args, "-D")?));
                    i += 1;
                }
                _ if a.starts_with("-I") && a.len() > 2 => o.includes.push(PathBuf::from(&a[2..])),
                _ if a.starts_with("-D") && a.len() > 2 => o.defines.push(define(&a[2..])),
                // `-march=…`, `-mavx2`, `-mtune=…`: handed to the persona probe verbatim, since
                // the compiler is the only thing that knows what each implies.
                _ if a.starts_with("-m") && a.len() > 2 => o.target_flags.push(a.clone()),
                _ if a.starts_with('-') => {
                    return Err(Fault::Usage(format!("unknown option `{a}`")));
                }
                _ => o.positional.push(a),
            }
            i += 1;
        }
        Ok(o)
    }

    fn files(&self, n: usize, what: &str) -> Result<Vec<PathBuf>, Fault> {
        if self.positional.len() != n {
            return Err(Fault::Usage(format!(
                "{what} takes {n} file{}, got {}",
                if n == 1 { "" } else { "s" },
                self.positional.len()
            )));
        }
        Ok(self.positional.iter().map(PathBuf::from).collect())
    }

    /// **60 seconds unless told otherwise, and `0` means none** (`timeout(1)`'s convention).
    ///
    /// The library default is `None` — 023 §8.1 requires the determinism contracts to run
    /// without a clock, and `Budget::default()` is what they use. This is the other surface:
    /// somebody is waiting at a terminal, and a command that never returns is the worst answer
    /// available, because it is not one.
    fn wall_clock(&self) -> Option<std::time::Duration> {
        match self.time_budget {
            None => Some(std::time::Duration::from_secs(60)),
            Some(s) if s <= 0.0 => None,
            Some(s) => Some(std::time::Duration::from_secs_f64(s)),
        }
    }

    fn frontend(&self) -> Frontend {
        Frontend {
            includes: self.includes.clone(),
            defines: self.defines.clone(),
            system_headers: !self.no_system_headers,
            target_flags: self.target_flags.clone(),
        }
    }
}

/// Seconds, as a decimal — a test that wants to see the clock fire cannot wait a whole second,
/// and a user who wants ten minutes should not count in milliseconds.
fn secs(s: &str) -> Result<f64, Fault> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && v >= 0.0 => Ok(v),
        _ => Err(Fault::Usage(format!(
            "--time-budget wants a non-negative number of seconds, got `{s}`"
        ))),
    }
}

/// Solver work units, as a whole number.
///
/// Not `secs`: these are not a duration and rounding one to a float would be the *last* thing
/// to do to a value whose only virtue is reproducibility.
fn units(s: &str) -> Result<u64, Fault> {
    s.parse().map_err(|_| {
        Fault::Usage(format!(
            "--solver-rlimit wants a whole number of solver work units, got `{s}`"
        ))
    })
}

fn num(s: &str, what: &str) -> Result<u32, Fault> {
    s.parse()
        .map_err(|_| Fault::Usage(format!("{what} wants a number, got `{s}`")))
}

/// **Read here rather than inside the frontend**, so a path that does not exist is an error
/// naming the path. A tool that answered "0 expansion sites" for a mistyped filename would be
/// the failure 050 §2 is about, arriving through the door a command line opens.
fn read(p: &Path) -> Result<String, Fault> {
    std::fs::read_to_string(p).map_err(|e| Fault::Failed(format!("{}: {e}", p.display())))
}

fn prove_equivalent(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(2, "prove-equivalent")?;
    let entry = o
        .entry
        .clone()
        .ok_or_else(|| Fault::Usage("prove-equivalent needs --entry <fn>".into()))?;
    let before = lower(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    let after = lower(&f[1], &read(&f[1])?, o.frontend()).map_err(Fault::Failed)?;
    let mut cfg = chiero_opt::EquivCfg::new(entry.clone());
    // **Every command that runs a solver honours the budget.** `prove-equivalent` accepted
    // `--solver-rlimit` and ignored it, which is the same defect the flag was written to end.
    cfg.budget.max_solver_rlimit = o.solver_rlimit;
    if !o.replay {
        return Ok(chiero_tool::prove_equivalent(&before, &after, &cfg));
    }
    // **A scratch directory beside the output, never beside the input.** 050 contract 12: a
    // replay may not write outside it, and the analysed tree is not it.
    let scratch = std::env::temp_dir().join(format!("chiero-replay-{}", std::process::id()));
    let sources = chiero_tool::ReplaySources {
        before: f[0].clone(),
        after: f[1].clone(),
        entry,
        scratch,
        // The same `-I` and `-D` the analysis used, so the harness compiles the program the
        // analysis analysed (040 §3).
        flags: o
            .includes
            .iter()
            .map(|p| format!("-I{}", p.display()))
            .chain(o.defines.iter().map(|(k, v)| format!("-D{k}={v}")))
            .collect(),
    };
    Ok(chiero_tool::prove_equivalent_with_replay(
        &before,
        &after,
        &cfg,
        Some(&sources),
        if o.allow_replay_exec {
            chiero_tool::ReplayPolicy::Run
        } else {
            chiero_tool::ReplayPolicy::EmitOnly
        },
    ))
}

/// **One unit name for both sides.** `chiero-diff` keys entities by file, so parsing
/// `before.c` and `after.c` under their own names would compare `before.c`'s `area` against
/// `after.c`'s `area` — two different entities, and every one of them "changed". Path identity
/// is the trap this project has hit four times, always in the flattering direction.
fn programs(
    o: &Options,
    f: &[PathBuf],
) -> Result<(chiero_diff::Program, chiero_diff::Program), Fault> {
    let unit = f[1]
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let cfg = o.frontend();
    let parse = |p: &PathBuf| -> Result<chiero_diff::Program, Fault> {
        let src = read(p)?;
        chiero_diff::Program::parse_with(&unit, &src, cfg.pp(), &mut frontend::Disk)
            .ok_or_else(|| Fault::Failed(format!("{}: could not be parsed", p.display())))
    };
    Ok((parse(&f[0])?, parse(&f[1])?))
}

fn find_bugs(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(1, "find-bugs")?;
    let entry = o
        .entry
        .clone()
        .ok_or_else(|| Fault::Usage("find-bugs needs --entry <fn>".into()))?;
    let m = lower(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    let mut cfg = chiero_tool::BugCfg::new(entry.clone());
    cfg.entry_ptr_nonnull = o.entry_ptr_nonnull;
    cfg.report_invented_bounds = o.report_invented_bounds;
    cfg.budget.wall_clock = o.wall_clock();
    cfg.budget.max_solver_rlimit = o.solver_rlimit;
    if o.replay {
        cfg.source = Some(chiero_tool::ReplaySources {
            before: f[0].clone(),
            // A finding is about one program; `after` is unused and set to the same file
            // rather than left to be a second, silently different, thing.
            after: f[0].clone(),
            entry,
            scratch: std::env::temp_dir().join(format!("chiero-replay-{}", std::process::id())),
            flags: o
                .includes
                .iter()
                .map(|p| format!("-I{}", p.display()))
                .chain(o.defines.iter().map(|(k, v)| format!("-D{k}={v}")))
                .collect(),
        });
        cfg.replay = if o.allow_replay_exec {
            chiero_tool::ReplayPolicy::Run
        } else {
            chiero_tool::ReplayPolicy::EmitOnly
        };
    }
    Ok(chiero_tool::find_bugs(&m, &cfg))
}

fn check_reachable(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(1, "check-reachable")?;
    let entry = o
        .entry
        .clone()
        .ok_or_else(|| Fault::Usage("check-reachable needs --entry <fn>".into()))?;
    let line = o
        .line
        .ok_or_else(|| Fault::Usage("check-reachable needs --line <n>".into()))?;
    let m = lower(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    let mut cfg = chiero_tool::BugCfg::new(entry);
    cfg.entry_ptr_nonnull = o.entry_ptr_nonnull;
    cfg.budget.wall_clock = o.wall_clock();
    cfg.budget.max_solver_rlimit = o.solver_rlimit;
    Ok(chiero_tool::check_reachable(&m, &cfg, line))
}

/// Print the lowered module in 020's normative textual format.
///
/// **The instrument that was missing.** 020 makes this format normative and its round trip a
/// contract, and until now nothing outside Rust could see it — so every question about what a
/// function lowered to was answered by reading `chiero-lower` and guessing. A
/// `pointer-outside-object` investigation on 2026-08-08 spent four probes on hypotheses that one
/// `grep` over this output would have settled.
///
/// `--entry <fn>` prints **just that function**, because a VPP translation unit lowers to a
/// quarter of a million lines and the question is almost always about one of them. It is a
/// filter on the printed text, not a module rewrite: the result is therefore *not* guaranteed to
/// re-parse on its own, since it names globals and callees the excerpt no longer declares.
/// Without it the whole module is printed, and that is the form the round trip is about.
fn cir(o: &Options) -> Result<String, Fault> {
    let f = o.files(1, "cir")?;
    let m = lower(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    let Some(entry) = o.entry.as_deref() else {
        return Ok(chiero_cir::text::print(&m));
    };
    // **Refuse rather than print everything.** A named entry that is not there is a typo or a
    // function the configuration removed, and silently dumping the whole module would answer a
    // question nobody asked — the `nofn` lesson from the sweep harness, one layer up.
    if !m.funcs.iter().any(|x| &*x.name == entry) {
        return Err(Fault::Usage(format!(
            "no function named `{entry}` in {}; this translation unit defines {} function(s)",
            f[0].display(),
            m.funcs.len()
        )));
    }
    // Text in, text out: find the `func @name(` header and stop at the closing brace in column
    // zero, which is where the printer puts it.
    let all = chiero_cir::text::print(&m);
    let head = format!("func @{entry}(");
    let Some(start) = all.find(&head) else {
        return Ok(all);
    };
    let rest = &all[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    Ok(rest[..end].to_string())
}

fn layout(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(1, "layout")?;
    let records = frontend::records(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    let cfg = chiero_opt::locality::LocalityCfg {
        cache_line_bytes: o.cache_line.unwrap_or(64),
        counts: Vec::new(),
    };
    Ok(chiero_tool::layout_envelope(&records, &cfg))
}

fn find_optimizations(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(1, "find-optimizations")?;
    let entry = o
        .entry
        .clone()
        .ok_or_else(|| Fault::Usage("find-optimizations needs --entry <fn>".into()))?;
    let m = lower(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    Ok(chiero_tool::find_optimizations(
        &m,
        &chiero_opt::opportunity::OppCfg::new(entry),
    ))
}

fn impact(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(2, "impact")?;
    let (before, after) = programs(o, &f)?;
    Ok(chiero_tool::impact_envelope(&before, &after))
}

fn select_tests(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(2, "select-tests")?;
    let dir = o
        .coverage
        .clone()
        .ok_or_else(|| Fault::Usage("select-tests needs --coverage <dir>".into()))?;
    let stem = o
        .stem
        .clone()
        .ok_or_else(|| Fault::Usage("select-tests needs --stem <name>".into()))?;
    if !dir.is_dir() {
        return Err(Fault::Failed(format!("{}: not a directory", dir.display())));
    }
    let index = chiero_gcov::ingest_native(&dir, &stem)
        .map_err(|e| Fault::Failed(format!("{}: {e:?}", dir.display())))?;
    // **An index with no test attribution can only select nothing, and saying "0 selected" is
    // the wrong answer to give.**
    //
    // `ingest_native` reads one object's coverage with `test: None` (chiero-gcov:852), so
    // `index.tests()` is empty and every selection over it is empty *whatever the diff says*.
    // Reported 2026-08-10 by the first end-to-end user: "CLI select-tests is structurally
    // empty … every invocation returns 0 selected", found because tutorial 3's console example
    // runs this path and `tutorials.rs` exercises the library one.
    //
    // Selecting needs coverage attributed **per test** — `ingest_native_as` with a `TestId` per
    // run, which is what a `make test-cov TEST=<name>` loop produces. This flag pair cannot
    // express that, and widening it is a design question (050 §3). Until it is answered, refuse
    // rather than answer: an empty selection from an index that could never be non-empty is
    // exactly the "empty answer" this project spent 2026-08-10 forbidding elsewhere.
    if index.tests().is_empty() {
        return Err(Fault::Failed(format!(
            "{}: the coverage index carries no test attribution, so no test can be selected \
             from it. `--coverage`/`--stem` ingest one object with no test name; selection \
             needs per-test coverage (`ingest_native_as`, one `TestId` per test run). This is \
             a limit of the command, not an answer about your change.",
            dir.display()
        )));
    }
    let (before, after) = programs(o, &f)?;
    // **The index's own answer, not one this command invents.** `validity` compares the
    // sources hashed at ingest against the tree; an index that recorded none can only report
    // what it knows, and inventing a verdict here would be exactly the second implementation
    // 050 §1's "thin wrapper" rules out. `select_tests`'s envelope already carries "coverage
    // is historical" as a blind spot, which is the qualification that always applies.
    let suite = chiero_select::Suite {
        tests: index.tests(),
        validity: index.validity(Path::new(".")),
    };
    Ok(chiero_tool::select_tests(
        &chiero_diff::impact(&before, &after),
        &after,
        &index,
        &suite,
    ))
}

fn expansion_sites(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(1, "expansion-sites")?;
    let name = o
        .macro_name
        .clone()
        .ok_or_else(|| Fault::Usage("expansion-sites needs --macro <NAME>".into()))?;
    let tu = frontend::preprocess(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    Ok(chiero_tool::expansion_sites_envelope(
        &tu.source_map,
        &name,
        o.cursor,
        o.limit.unwrap_or(50),
    ))
}

fn explain_macro(o: &Options) -> Result<chiero_tool::Envelope, Fault> {
    let f = o.files(1, "explain-macro")?;
    let line = o
        .line
        .ok_or_else(|| Fault::Usage("explain-macro needs --line <n>".into()))?;
    let tu = frontend::preprocess(&f[0], &read(&f[0])?, o.frontend()).map_err(Fault::Failed)?;
    // The name the `SourceMap` knows this unit by is the path it was preprocessed under.
    let unit = f[0].to_string_lossy().into_owned();
    Ok(chiero_tool::explain_macro_expansion_envelope(
        &tu.source_map,
        &unit,
        line,
        o.col,
    ))
}
