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

    impact <before.c> <after.c>
            What a source change reaches — through calls, types, globals and
            macro expansions.  (031)

    select-tests <before.c> <after.c> --coverage <dir> --stem <name>
            Which tests are worth running for that change, ranked, with the
            reason for each.  (032)

    expansion-sites <file.c> --macro <NAME> [--cursor <n>] [--limit <n>]
            Every place a macro expands in this translation unit.  (050 §3)

    explain-macro <file.c> --line <n> [--col <n>]
            What macro chain produced the code on a line, innermost first.

OPTIONS:
    --json          Print the envelope as JSON. Default is a human rendering.
    --replay        Emit a C harness demonstrating a `differs` verdict.
    --allow-replay-exec
                    Compile and run that harness. Off by default: this builds
                    and executes code, so a caller has to ask.  (050 §6)
    -I <dir>        Add an include path. Repeatable.
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
            println!("{text}");
            ExitCode::SUCCESS
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
    let env = match args[0].as_str() {
        "prove-equivalent" => prove_equivalent(&opts)?,
        "find-bugs" => find_bugs(&opts)?,
        "check-reachable" => check_reachable(&opts)?,
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
    col: Option<u32>,
    cursor: Option<usize>,
    limit: Option<usize>,
    coverage: Option<PathBuf>,
    stem: Option<String>,
    includes: Vec<PathBuf>,
    defines: Vec<(String, String)>,
    json: bool,
    replay: bool,
    allow_replay_exec: bool,
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
                "--replay" => o.replay = true,
                "--allow-replay-exec" => {
                    o.replay = true;
                    o.allow_replay_exec = true;
                }
                "--entry" => {
                    o.entry = Some(need(i, args, "--entry")?);
                    i += 1;
                }
                "--macro" => {
                    o.macro_name = Some(need(i, args, "--macro")?);
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

    fn frontend(&self) -> Frontend {
        Frontend {
            includes: self.includes.clone(),
            defines: self.defines.clone(),
        }
    }
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
    let cfg = chiero_opt::EquivCfg::new(entry.clone());
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
    Ok(chiero_tool::find_bugs(&m, &chiero_tool::BugCfg::new(entry)))
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
    Ok(chiero_tool::check_reachable(
        &m,
        &chiero_tool::BugCfg::new(entry),
        line,
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
