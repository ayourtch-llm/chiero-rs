//! **The help pages, as data — one table of operations, one of options, both rendered.**
//!
//! Reported 2026-08-10 by the first end-to-end user: `select-tests --help` printed the global
//! page, so a reader who had already chosen an operation still had to work out which of
//! eighteen options applied to it. 030's `--coverage`/`--stem` semantics cost them three
//! attempts.
//!
//! **Why a table rather than ten strings.** A per-operation page written by hand is a second
//! copy of the argument parser, and this project has watched every hand-kept list drift: the
//! global page that motivated this one had accepted `--march` since 2026-08-09 and never
//! mentioned it. Here an option is described once, an operation names the options it takes,
//! and both pages are rendered from that — the global one cannot omit an option that some
//! operation lists, and no operation can advertise an option that does not exist.
//!
//! `tests/help.rs` closes the remaining gap, which is the one no table can close on its own:
//! it reads `Options::parse` and each operation's *implementation* and demands the tables agree
//! with them.

/// An option: `spec` is what a reader types, `about` is why.
///
/// The first whitespace-delimited word of `spec` is the flag's name and is what
/// [`Op::flags`] refers to.
struct Flag {
    spec: &'static str,
    about: &'static str,
}

/// An operation: one line of synopsis, a paragraph, and the options it reads.
struct Op {
    name: &'static str,
    /// The arguments after the operation name, as a reader types them.
    args: &'static str,
    about: &'static str,
    /// Flag names, exactly as they appear at the head of a [`Flag::spec`]. [`GLOBAL`] is added
    /// to every operation and is not repeated here.
    flags: &'static [&'static str],
}

/// Options that apply to every operation, so no operation lists them.
const GLOBAL: &[&str] = &["-I", "-D", "--march", "--no-system-headers", "--json"];

const FLAGS: &[Flag] = &[
    Flag {
        spec: "--entry <fn>",
        about: "The function to start from. Analysis is per function, not per\n\
                translation unit: chiero enters here with unconstrained arguments.",
    },
    Flag {
        spec: "--line <n>",
        about: "A line number in the file, 1-based.",
    },
    Flag {
        spec: "--col <n>",
        about: "A column on that line, 1-based. Without it, the first expansion\n\
                on the line.",
    },
    Flag {
        spec: "--macro <NAME>",
        about: "The macro to look for, by name.",
    },
    Flag {
        spec: "--cursor <n>",
        about: "Resume after the n-th site, for walking a long list in pages.",
    },
    Flag {
        spec: "--limit <n>",
        about: "At most this many sites. Default 50.",
    },
    Flag {
        spec: "--test <NAME=PATH>",
        about: "One test run's coverage: the test's name, then the object\n\
                without its extension — `--test bfd=build/cov/bfd_main` reads\n\
                `bfd_main.gcno`/`.gcda`. Repeatable, and a name repeated is one\n\
                test that touched several objects. This is the flag that lets\n\
                `select-tests` select: selection needs to know which test\n\
                covered what, and one unattributed object cannot say.",
    },
    Flag {
        spec: "--coverage-manifest <file>",
        about: "The same, one NAME<TAB>PATH line per test run — what a\n\
                `make test-cov TEST=<name>` loop writes. Blank lines and `#`\n\
                comments are skipped.",
    },
    Flag {
        spec: "--coverage <dir>",
        about: "Directory holding a gcov run's `.gcno`/`.gcda` files.",
    },
    Flag {
        spec: "--stem <name>",
        about: "The object's base name inside that directory — `foo` for\n\
                `foo.gcno`/`foo.gcda`, not a path and not `foo.c`.",
    },
    Flag {
        spec: "--cache-line <n>",
        about: "Cache-line size in bytes for `layout`. Default 64.",
    },
    Flag {
        spec: "--replay",
        about: "Emit a C harness demonstrating a `differs` verdict.",
    },
    Flag {
        spec: "--allow-replay-exec",
        about: "Compile and run that harness. Off by default: this builds and\n\
                executes code, so a caller has to ask.  (050 §6)",
    },
    Flag {
        spec: "--report-invented-bounds",
        about: "Show bounds findings against the object chiero invents behind\n\
                an entry pointer. Off by default: chiero knows neither the\n\
                caller's object size nor where in it the pointer points, so\n\
                those say nothing about your program. The count is always\n\
                reported, shown or not.",
    },
    Flag {
        spec: "--entry-ptr-nonnull",
        about: "Assume the pointer parameters of the entry function are not\n\
                null. For a helper whose callers check, the null path is one\n\
                the program does not have. Removes real paths, so it is\n\
                recorded as an assumption in the envelope.",
    },
    Flag {
        spec: "--time-budget <secs>",
        about: "Stop after that many seconds and print what was found so far.\n\
                Decimals allowed; 0 means no limit, as in timeout(1). Default\n\
                60. A run that ends here is marked `nondeterministic_abort`:\n\
                where it stopped depends on the machine, so the answer is a\n\
                measurement.  (023 §8.1)",
    },
    Flag {
        spec: "--solver-rlimit <units>",
        about: "Stop any single solver query after that many of z3's work\n\
                units. 0 (the default) is no limit. Unlike a wall-clock budget\n\
                this is deterministic — work units do not move with machine\n\
                speed or thread count — so a run cut by it is an ordinary\n\
                answer rather than a measurement, and it is the only bound\n\
                that reaches inside one long query.  (023 §8)",
    },
    Flag {
        spec: "-I <dir>",
        about: "Add an include path. Repeatable.",
    },
    Flag {
        spec: "-D <k[=v]>",
        about: "Define a macro. Repeatable.",
    },
    Flag {
        spec: "--march <name>",
        about: "Target the compiler persona at that architecture, as gcc's\n\
                `-march=`. `-m<flag>` is passed through the same way. Not\n\
                decoration: `__SSE4_2__` and `__AVX2__` exist only under the\n\
                right `-march`, so probing with none of them models a\n\
                different compiler than the one your code is built with.",
    },
    Flag {
        spec: "--no-system-headers",
        about: "Do not ask the C compiler where its own headers are. On by\n\
                default, because real C includes <stdio.h>.",
    },
    Flag {
        spec: "--json",
        about: "Print the envelope as JSON. Default is a human rendering.",
    },
    Flag {
        spec: "-h, --help",
        about: "This text. After an operation name, that operation's page.",
    },
];

const OPS: &[Op] = &[
    Op {
        name: "prove-equivalent",
        args: "<before.c> <after.c> --entry <fn>",
        about: "Adjudicate a rewrite. Either a proof that the two agree for every\n\
                input, or a concrete input at which they do not.  (041 §1)",
        flags: &[
            "--entry",
            "--replay",
            "--allow-replay-exec",
            "--solver-rlimit",
        ],
    },
    Op {
        name: "find-bugs",
        args: "<file.c> --entry <fn>",
        about: "Run 040's defect checkers from a function. An empty list is an\n\
                answer only when the envelope says the search finished.  (050 §3)",
        flags: &[
            "--entry",
            "--entry-ptr-nonnull",
            "--report-invented-bounds",
            "--time-budget",
            "--solver-rlimit",
            "--replay",
            "--allow-replay-exec",
        ],
    },
    Op {
        name: "check-reachable",
        args: "<file.c> --entry <fn> --line <n>",
        about: "Can execution get to that line? Proved-nothing-does and\n\
                chiero-did-not are different answers, and it says which.  (050 §3)",
        flags: &[
            "--entry",
            "--line",
            "--entry-ptr-nonnull",
            "--time-budget",
            "--solver-rlimit",
        ],
    },
    Op {
        name: "layout",
        args: "<file.c> [--cache-line <n>]",
        about: "Cache-line and padding analysis of the structs in a translation\n\
                unit. Proposals only — nothing is ever rewritten.  (041 §3)",
        flags: &["--cache-line"],
    },
    Op {
        name: "find-optimizations",
        args: "<file.c> --entry <fn>",
        about: "Proposals with obligations and benefit labels. Never rewrites\n\
                anything.  (041 §2)",
        flags: &["--entry"],
    },
    Op {
        name: "impact",
        args: "<before.c> <after.c>",
        about: "What a source change reaches — through calls, types, globals and\n\
                macro expansions.  (031)",
        flags: &[],
    },
    Op {
        name: "select-tests",
        args: "<before.c> <after.c> --coverage <dir> --stem <name>",
        about: "Which tests are worth running for that change, ranked, with the\n\
                reason for each.  (032)\n\
                \n\
                The two files are the same translation unit before and after the\n\
                change; they are compared under one unit name, so `old/foo.c` and\n\
                `new/foo.c` is the shape, not two different files.\n\
                \n\
                ⚠️ The coverage has to be attributed per test — one gcov run per\n\
                test, each named. That is what `--test` and `--coverage-manifest`\n\
                are for. `--coverage`/`--stem` read one object with no test name\n\
                attached, so an index built only from them can select nothing, and\n\
                this command refuses rather than answering `0 selected`.",
        flags: &["--test", "--coverage-manifest", "--coverage", "--stem"],
    },
    Op {
        name: "expansion-sites",
        args: "<file.c> --macro <NAME> [--cursor <n>] [--limit <n>]",
        about: "Every place a macro expands in this translation unit.  (050 §3)",
        flags: &["--macro", "--cursor", "--limit"],
    },
    Op {
        name: "cir",
        args: "<file.c> [--entry <fn>]",
        about: "Print the lowered module in 020's normative textual format. The\n\
                answer is about chiero rather than about your program, so it\n\
                carries no envelope -- it round-trips instead.\n\
                \n\
                With --entry it prints that function alone, which is a filter on\n\
                the text: the excerpt names globals and callees it no longer\n\
                declares, so only the whole module is guaranteed to re-parse.",
        flags: &["--entry"],
    },
    Op {
        name: "explain-macro",
        args: "<file.c> --line <n> [--col <n>]",
        about: "What macro chain produced the code on a line, innermost first.",
        flags: &["--line", "--col"],
    },
];

/// The closing paragraph both pages carry, because it is the thing a reader most needs to know
/// and least expects.
const ENVELOPE_NOTE: &str = "\
Every operation prints an ENVELOPE: the result, plus `fidelity`, `proven`,
`assumptions` and `blind_spots`. `proven` is true only when the answer holds for
all inputs. An empty result is not the same as a clean one — see
docs/tutorials/05-envelope.md.";

/// Description column, matching the global page this replaced.
const COL: usize = 20;

fn flag(name: &str) -> &'static Flag {
    FLAGS
        .iter()
        .find(|f| f.spec.split([' ', ',']).next() == Some(name))
        .unwrap_or_else(|| panic!("no option `{name}` in the table"))
}

/// One option, as `    --spec          about`, with the description wrapping to [`COL`].
fn render_flag(f: &Flag, out: &mut String) {
    let head = format!("    {}", f.spec);
    if head.len() < COL {
        out.push_str(&format!("{head:<COL$}"));
    } else {
        out.push_str(&head);
        out.push('\n');
        out.push_str(&" ".repeat(COL));
    }
    for (i, line) in f.about.lines().enumerate() {
        if i > 0 {
            out.push('\n');
            out.push_str(&" ".repeat(COL));
        }
        out.push_str(line);
    }
    out.push('\n');
}

fn render_flags(names: &[&str], out: &mut String) {
    out.push_str("\nOPTIONS:\n");
    for n in names {
        render_flag(flag(n), out);
    }
}

/// The global page: every operation in one screen, and where to go from there.
pub(crate) fn usage() -> String {
    let mut s = String::from(
        "chiero — a symbolic C execution environment\n\
         \n\
         USAGE:\n    \
         chiero <operation> [args] [options]\n\
         \n\
         Run `chiero <operation> --help` for that operation's arguments and the\n\
         options it reads — each operation takes a handful of the list below.\n\
         \n\
         OPERATIONS:\n",
    );
    for op in OPS {
        s.push_str(&format!("    {} {}\n", op.name, op.args));
        for line in op.about.lines().take_while(|l| !l.trim().is_empty()) {
            s.push_str(&format!("            {line}\n"));
        }
        s.push('\n');
    }
    s.pop();
    let mut all: Vec<&str> = Vec::new();
    for f in FLAGS {
        all.push(f.spec.split([' ', ',']).next().unwrap_or_default());
    }
    render_flags(&all, &mut s);
    s.push('\n');
    s.push_str(ENVELOPE_NOTE);
    s
}

/// One operation's page, or `None` if that is not an operation.
pub(crate) fn op_help(name: &str) -> Option<String> {
    let op = OPS.iter().find(|o| o.name == name)?;
    // **The first sentence, not the first line.** `about` is hand-wrapped for the operations
    // list, so taking a line cuts the title mid-clause — "…ranked, with the".
    let title = op.about.replace('\n', " ");
    let title = title
        .split_once(". ")
        .map_or(title.trim(), |(head, _)| head)
        .trim()
        .trim_end_matches('.');
    let mut s = format!(
        "chiero {} — {title}.\n\nUSAGE:\n    chiero {} {}\n\n",
        op.name, op.name, op.args
    );
    for line in op.about.lines() {
        s.push_str(line);
        s.push('\n');
    }
    s.pop();
    s.push('\n');
    let names: Vec<&str> = op
        .flags
        .iter()
        .copied()
        .chain(GLOBAL.iter().copied())
        .chain(std::iter::once("-h"))
        .collect();
    render_flags(&names, &mut s);
    s.push('\n');
    s.push_str(ENVELOPE_NOTE);
    Some(s)
}

/// Every operation, as `(name, one-line description, the arguments a reader types)`.
///
/// **One table, two surfaces.** `chiero serve` offers these as JSON-RPC tools and `--help`
/// renders the same rows, which is 050 contract 18's identity check made structural rather than
/// checked after the fact — the two cannot drift because there is nothing to drift *from*.
/// `crates/chiero-cli/tests/serve.rs` asserts it anyway, against the dispatch `match` itself,
/// because "cannot drift by construction" is a claim about code that changes.
pub(crate) fn catalogue() -> Vec<(&'static str, String, &'static str)> {
    OPS.iter()
        .map(|o| {
            // **The whole first paragraph, not the first sentence.** An agent choosing between
            // ten tools needs to know what each one answers; "Adjudicate a rewrite" is a title,
            // not a description, and it is what the first-sentence rule produced.
            let text: String = o
                .about
                .lines()
                .take_while(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            (
                o.name,
                text.split_whitespace().collect::<Vec<_>>().join(" "),
                o.args,
            )
        })
        .collect()
}
