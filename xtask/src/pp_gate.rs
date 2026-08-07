//! Preprocessor conformance over simplecpp's `testsuite/` — **gcc and clang are the oracle**.
//!
//! Every corpus in this repo so far is real VPP code. VPP exercises macros as people write
//! them: `foreach_*` X-macros, `PREDICT_FALSE`, argument-heavy registration macros. It never
//! exercises the dark corners — `#`/`##` at the edges, recursive expansion, `__VA_ARGS__`
//! rescanning, `#line`, GNU `args...` with `##`. §8.3's widening pattern says the defects live
//! past the edge of the corpus, and this is a **new kind of edge**, not a wider slice of the
//! same one.
//!
//! # Why the checkout is not vendored
//!
//! `testsuite/clang-preprocessor-tests/` is 211 verbatim clang files
//! (Apache-2.0-with-LLVM-exception) and `testsuite/gcc-preprocessor-tests/` is 26 gcc ones
//! (GPL). Neither may be copied into an MIT-OR-Apache-2.0 repository. So this reads a
//! **checkout**, exactly as every VPP gate reads `/home/ubuntu/vpp` rather than a copy. Nothing
//! about the corpus enters the tree; only the method does.
//!
//! # simplecpp is the source of inputs, never the authority
//!
//! The files carry no expected output. simplecpp's own `run-tests.py` runs each through clang
//! *and* gcc and passes if it matches **either**. That is mirrored here, which means simplecpp's
//! verdicts are never consulted: the two compilers answer directly. Where they disagree with
//! each other the row is reported as such — those are the interesting ones, and simplecpp
//! silently takes whichever matches.
//!
//! Its `skip` and `todo` lists are carried as **priors, not truth** (§9.1). `todo` is a
//! ready-made difficulty gradient: it is where a good hand-written preprocessor still fails, so
//! it is where chiero's gaps are most likely. A chiero pass on a `todo` file is a real result.
//!
//! # Comparison is by token sequence, not by stripped whitespace
//!
//! simplecpp's `cleanup()` drops every line beginning with `#` and then strips *all* whitespace.
//! That is coarse — it makes `a b` and `ab` identical. This lexes both sides through
//! `chiero_lex` and compares token spellings, which `crates/chiero-pp/tests/compiler_oracle.rs`
//! already does for hand-written fixtures. The `#`-line drop is kept, because it is what makes
//! chiero comparable at all: linemarkers are a `-P` concern and `#pragma` is a record on the TU
//! rather than a token in the stream.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where simplecpp is checked out. Overridable so this is not a fact about one machine.
pub fn corpus_root() -> PathBuf {
    std::env::var_os("SIMPLECPP")
        .map_or_else(|| PathBuf::from("/home/ubuntu/simplecpp"), PathBuf::from)
}

/// simplecpp's own prior about a file. **Carried, not obeyed.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prior {
    /// simplecpp runs it and expects to match.
    Expected,
    /// simplecpp skips it: `_Pragma`, `__has_attribute`, locale-dependent output, `-march`.
    Skipped,
    /// simplecpp runs it and knows it fails. The difficulty gradient.
    Todo,
}

/// One command: a file plus the `-D`s taken off its `RUN` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub path: PathBuf,
    pub defines: Vec<(String, String)>,
    pub prior: Prior,
}

impl Case {
    /// The bare filename, which is what simplecpp's lists are keyed by.
    pub fn name(&self) -> &str {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("")
    }
}

/// What one run of one case established.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Both compilers produced the same token sequence and chiero produced it too. The only
    /// row that is evidence chiero is right.
    Agree,
    /// The compilers disagreed with each other and chiero matched one. Not a defect in chiero;
    /// the row names which side it took.
    MatchedOne { compiler: &'static str },
    /// The compilers agreed and chiero did not. **The finding.**
    Differs,
    /// The token sequences differ but the **programs do not**: the two rendered the same result
    /// differently, and re-lexing a rendering is lossy.
    ///
    /// Two causes, both real and neither a defect. `gcc -E` **normalizes** a universal character
    /// name (`\u00AA` → `\U000000aa`) while chiero preserves what was written, because 010
    /// contract 11 wants a token's bytes to re-lex to its own spelling (011 §2.0). And an
    /// **unterminated** character constant — `-Dfoo='bar\'`, where `\'` is an escape — is one
    /// token to both, but gcc's rendered output re-lexes into a literal that swallows the
    /// following token.
    ///
    /// ⚠️ **Reported as its own verdict, never merged into `Agree`.** The comparison behind it is
    /// deliberately coarser — UCN escapes canonicalized, then all whitespace stripped, which is
    /// simplecpp's own `cleanup()` — and a coarser comparison that reported "agree" would be a
    /// gate quietly lowering its own standard.
    RendersDifferently,
    /// The compilers disagreed and chiero matched neither. Also a finding, and a worse one.
    MatchedNeither,
    /// Both compilers rejected the file **and so did chiero**. These are the corpus's negative
    /// tests — `#if` with no expression, `#ifdef` with no name, a paste of `/` and `*` — and
    /// agreeing to reject one is a real result, not a skip.
    RefusedByAll,
    /// **Both compilers rejected the file and chiero said nothing.** A missing diagnostic, and
    /// its own finding class.
    ///
    /// The first version of this gate folded it into a single "no compiler ran it" bucket, on
    /// the reasoning that nothing had been asked of chiero. That was wrong in the way
    /// `sweep::Bucket::Miss` is written to prevent: 21 of 141 cases landed there, they are the
    /// corpus's *error-recovery* half, and the question they ask — does chiero notice — was
    /// being answered by not asking it.
    AcceptedWhatBothRejected,
    /// chiero **crashed** on the file. Its own class, and above every other finding.
    ///
    /// §7.6 filed two source-triggerable panics in the same row as a file that would not
    /// preprocess, so two crashes on real code read as two files chiero could not read. A
    /// panic is not a divergence and not a refusal: it is the one outcome that says the
    /// program under test has no defined behaviour at all.
    Panicked(String),
    /// The file has no `RUN` line simplecpp's rule takes anything from.
    NoCommand,
}

/// One row of the report.
#[derive(Debug, Clone)]
pub struct Row {
    pub case: Case,
    pub verdict: Verdict,
    /// chiero's own diagnostic count. **Reported beside the token verdict, never folded into
    /// it**: a diagnostic on a file both compilers accept is a false rejection, which is a
    /// different fact from a wrong token, and a run can have both.
    pub chiero_diagnostics: Vec<String>,
    /// The first divergence, as `(index, ours, theirs)`, for a finding.
    pub first_difference: Option<(usize, String, String)>,
}

/// `// RUN: %clang_cc1 …` → the `-D`s, by simplecpp's rule: **only `-E` and `-D*` are taken and
/// everything else on the line is ignored**. A line that yields neither is not a command.
///
/// Mirrored rather than improved on, because the point of this corpus is that simplecpp already
/// established these lines run under both compilers. Taking more flags would be a different
/// experiment with no evidence behind it.
pub fn defines_from_run_line(line: &str) -> Option<Vec<(String, String)>> {
    let rest = line.strip_prefix("// RUN: %clang_cc1 ")?;
    let mut saw_flag = false;
    let mut defines = Vec::new();
    for arg in rest.split_whitespace() {
        if arg == "-E" {
            saw_flag = true;
        } else if arg.len() >= 3 && arg.starts_with("-D") {
            saw_flag = true;
            defines.push(split_define(&arg[2..]));
        }
    }
    saw_flag.then_some(defines)
}

/// `FOO=bar` → `("FOO", "bar")`; a bare `FOO` is `1`, as every compiler has it.
fn split_define(text: &str) -> (String, String) {
    text.split_once('=').map_or_else(
        || (text.to_owned(), "1".to_owned()),
        |(name, value)| (name.to_owned(), value.to_owned()),
    )
}

/// Every case the corpus offers, sorted, so two runs are comparable.
///
/// **`.c` only.** The glob simplecpp uses is `*.c*`, which also takes 19 `.cpp`, 3 `.cu` and one
/// `.cl`. chiero is a C tool and C++ is a declared non-goal, so those are out of scope — and
/// they are *counted* rather than dropped, because "did not look" and "found nothing" are
/// different facts (§11.3).
pub fn cases(root: &Path) -> std::io::Result<(Vec<Case>, usize)> {
    let mut cases = Vec::new();
    let mut other_language = 0;
    let mut seen = std::collections::BTreeSet::new();

    let clang_dir = root.join("testsuite/clang-preprocessor-tests");
    for path in sorted_files(&clang_dir)? {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ext.starts_with('c') {
            continue;
        }
        if ext != "c" {
            other_language += 1;
            continue;
        }
        let source = std::fs::read_to_string(&path)?;
        for line in source.lines() {
            if let Some(defines) = defines_from_run_line(line) {
                // simplecpp dedupes commands; a file with two identical RUN lines is one case.
                if seen.insert((path.clone(), defines.clone())) {
                    cases.push(Case {
                        path: path.clone(),
                        prior: prior_of(path.file_name().and_then(|n| n.to_str()).unwrap_or("")),
                        defines,
                    });
                }
            }
        }
    }

    let gcc_dir = root.join("testsuite/gcc-preprocessor-tests");
    for path in sorted_files(&gcc_dir)? {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ext.starts_with('c') {
            continue;
        }
        if ext != "c" {
            other_language += 1;
            continue;
        }
        cases.push(Case {
            prior: prior_of(path.file_name().and_then(|n| n.to_str()).unwrap_or("")),
            path,
            defines: Vec::new(),
        });
    }
    Ok((cases, other_language))
}

fn sorted_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// simplecpp's `skip` list, verbatim from `run-tests.py` at `74a5a63`.
const SIMPLECPP_SKIP: &[&str] = &[
    "assembler-with-cpp.c",
    "builtin_line.c",
    "c99-6_10_3_3_p4.c",
    "clang_headers.c",
    "comment_save.c",
    "has_attribute.c",
    "has_attribute.cpp",
    "header_lookup1.c",
    "line-directive-output.c",
    "microsoft-ext.c",
    "normalize-3.c",
    "pr63831-1.c",
    "pr63831-2.c",
    "pr65238-1.c",
    "_Pragma-location.c",
    "_Pragma-dependency.c",
    "_Pragma-dependency2.c",
    "_Pragma-physloc.c",
    "pragma-pushpop-macro.c",
    "x86_target_features.c",
    "warn-disabled-macro-expansion.c",
    "ucnid-2011-1.c",
];

/// simplecpp's `todo` list, verbatim. **The difficulty gradient**, not an excuse list.
const SIMPLECPP_TODO: &[&str] = &[
    "macro_backslash.c",
    "macro_fn_comma_swallow.c",
    "macro_fn_comma_swallow2.c",
    "macro_expand.c",
    "macro_fn_disable_expand.c",
    "macro_paste_commaext.c",
    "macro_paste_hard.c",
    "macro_rescan_varargs.c",
    "c99-6_10_3_4_p5.c",
    "c99-6_10_3_4_p6.c",
    "expr_usual_conversions.c",
    "stdint.c",
    "diagnostic-pragma-1.c",
    "pr45457.c",
    "pr57580.c",
];

fn prior_of(name: &str) -> Prior {
    if SIMPLECPP_SKIP.contains(&name) {
        Prior::Skipped
    } else if SIMPLECPP_TODO.contains(&name) {
        Prior::Todo
    } else {
        Prior::Expected
    }
}

/// A compiler's `-E` output as a token sequence, or `None` if it refused the file.
///
/// The `#`-line drop is simplecpp's `cleanup()` and is load-bearing here for a different
/// reason: linemarkers are not tokens of the program, and a `#pragma` reaching the output is a
/// `PragmaRecord` on chiero's side rather than a token in its stream. Dropping the lines is what
/// puts the two sides on the same footing.
fn compiler_tokens(compiler: &str, case: &Case) -> Option<Vec<String>> {
    let mut command = Command::new(compiler);
    command.arg("-E");
    for (name, value) in &case.defines {
        command.arg(format!("-D{name}={value}"));
    }
    command.arg(&case.path);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| format!("{line}\n"))
        .collect();
    Some(lex_texts(&body))
}

/// Do two token sequences describe the same program, ignoring how it was rendered?
///
/// Canonicalize universal character names, concatenate, strip all whitespace. The last step is
/// simplecpp's own `cleanup()` and is what makes an unterminated literal's differing token
/// boundaries comparable at all.
///
/// **Only ever used to explain a difference, never to declare agreement** — see
/// [`Verdict::RendersDifferently`].
fn same_program(ours: &[String], theirs: &[String]) -> bool {
    fn canonical(tokens: &[String]) -> Vec<u8> {
        let joined: String = tokens.concat();
        let bytes = joined.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\'
                && let Some((code, len)) = ucn_at(bytes, i)
            {
                // **Decode to the character, not to a canonical escape.** gcc renders `\u00A8`
                // as the literal `¨` in one file and as `\U000000a8` in another, so only the
                // character itself is a form both spellings reach.
                let mut buffer = [0_u8; 4];
                out.extend_from_slice(
                    char::from_u32(code)
                        .unwrap_or('\u{fffd}')
                        .encode_utf8(&mut buffer)
                        .as_bytes(),
                );
                i += len;
            } else {
                // ⚠️ **Bytes, not `char`s.** Casting a byte to `char` is Latin-1, so a UTF-8
                // `¨` became `Â¨` and never matched the decoded escape — the first version of
                // this compared two manglings and reported a difference that was its own.
                if !bytes[i].is_ascii_whitespace() {
                    out.push(bytes[i]);
                }
                i += 1;
            }
        }
        out
    }
    canonical(ours) == canonical(theirs)
}

/// `\uXXXX` / `\UXXXXXXXX` at `at`, as `(code point, byte length)`.
fn ucn_at(bytes: &[u8], at: usize) -> Option<(u32, usize)> {
    let digits = match bytes.get(at + 1) {
        Some(b'u') => 4,
        Some(b'U') => 8,
        _ => return None,
    };
    let text = bytes.get(at + 2..at + 2 + digits)?;
    if !text.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let value = std::str::from_utf8(text).ok()?;
    Some((u32::from_str_radix(value, 16).ok()?, 2 + digits))
}

fn lex_texts(text: &str) -> Vec<String> {
    use chiero_lex::{LexConfig, LexSession, PpTokenKind};
    let mut map = chiero_span::SourceMap::new();
    let file = map.add_file("oracle-output.c", text.to_owned());
    let lexed = LexSession::new().lex(&map, file, LexConfig::default());
    lexed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.kind, PpTokenKind::Eof))
        .map(|token| lexed.text(token).to_owned())
        .collect()
}

struct DiskLoader;

impl chiero_pp::FileLoader for DiskLoader {
    fn load(&mut self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// gcc's complete predefine set, as `frontend::predefines` and `vpp_headers.rs` both take it.
///
/// **This is a choice with a consequence and it is recorded rather than hidden**: chiero is
/// given gcc's predefines, not clang's, so on a file that tests `__GNUC__` or
/// `__STDC_VERSION__` chiero can only realistically match gcc. The alternative — no predefines
/// at all — would make chiero match neither, which measures the harness instead of the
/// preprocessor.
fn gcc_predefines() -> Vec<(String, String)> {
    let output = Command::new("gcc")
        .args(["-dM", "-E", "-x", "c", "/dev/null"])
        .output()
        .expect("gcc is required for the preprocessor conformance gate");
    String::from_utf8(output.stdout)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("#define "))
        .map(|definition| {
            definition.split_once(char::is_whitespace).map_or_else(
                || (definition.to_owned(), String::new()),
                |(name, value)| (name.to_owned(), value.trim_start().to_owned()),
            )
        })
        .collect()
}

fn system_paths() -> Vec<PathBuf> {
    [
        "/usr/lib/gcc/x86_64-linux-gnu/13/include",
        "/usr/local/include",
        "/usr/include/x86_64-linux-gnu",
        "/usr/include",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// Run one case through both compilers and through chiero.
pub fn run_case(
    session: &chiero_pp::PreprocessorSession,
    predefines: &[(String, String)],
    case: &Case,
) -> Row {
    let gcc = compiler_tokens("gcc", case);
    let clang = compiler_tokens("clang", case);

    let Ok(source) = std::fs::read_to_string(&case.path) else {
        return Row {
            case: case.clone(),
            verdict: Verdict::RefusedByAll,
            chiero_diagnostics: vec!["file unreadable".to_owned()],
            first_difference: None,
        };
    };

    let mut defines = predefines.to_vec();
    defines.extend(case.defines.iter().cloned());
    let config = chiero_pp::Config {
        // The file's own directory first: several cases `#include` a sibling under `Inputs/`.
        include_paths: vec![case.path.parent().unwrap_or(Path::new(".")).to_path_buf()],
        system_paths: system_paths(),
        defines,
        ..chiero_pp::Config::default()
    };
    // **A crash on one file must not end the sweep.** The measurement is over the corpus, and a
    // harness that stops at the first panic reports the corpus it reached rather than the one it
    // has — which is how §7.6's two panics came to be filed as two unreadable files.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let tu = session.preprocess_with_loader(&case.path, &source, config, &mut DiskLoader);
        let ours: Vec<String> = tu.token_texts().map(str::to_owned).collect();
        let diagnostics: Vec<String> = tu
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        (ours, diagnostics)
    }));
    let (ours, chiero_diagnostics) = match outcome {
        Ok(pair) => pair,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panicked with a non-string payload".to_owned());
            return Row {
                case: case.clone(),
                verdict: Verdict::Panicked(message),
                chiero_diagnostics: Vec::new(),
                first_difference: None,
            };
        }
    };

    // **Agreeing to reject is agreement.** A compiler that refuses a file returns `None` here, so
    // a row where chiero diagnoses *and* a compiler refuses is the two of them reaching the same
    // verdict about the program — even though there are no tokens to compare. `x######x` is the
    // case: it is UB, gcc silently emits `xx`, clang rejects it, and chiero rejects it too.
    // Scoring that as "matched neither" counts a defensible answer as a divergence.
    let refused_by = [("gcc", &gcc), ("clang", &clang)]
        .into_iter()
        .find(|(_, tokens)| tokens.is_none())
        .map(|(name, _)| name);

    let verdict = match (&gcc, &clang) {
        (None, None) => {
            if chiero_diagnostics.is_empty() {
                Verdict::AcceptedWhatBothRejected
            } else {
                Verdict::RefusedByAll
            }
        }
        (Some(g), Some(c)) if g == c => {
            if &ours == g {
                Verdict::Agree
            } else if same_program(&ours, g) {
                Verdict::RendersDifferently
            } else {
                Verdict::Differs
            }
        }
        (g, c) => {
            if g.as_ref() == Some(&ours) {
                Verdict::MatchedOne { compiler: "gcc" }
            } else if c.as_ref() == Some(&ours) {
                Verdict::MatchedOne { compiler: "clang" }
            } else if g.as_ref().is_some_and(|t| same_program(&ours, t))
                || c.as_ref().is_some_and(|t| same_program(&ours, t))
            {
                // The compilers split, and chiero renders the same program as one of them.
                Verdict::RendersDifferently
            } else if let Some(compiler) = refused_by.filter(|_| !chiero_diagnostics.is_empty()) {
                // One compiler refused the file and so did chiero: they agree it is not valid C,
                // which is a real answer and not a failure to have one.
                Verdict::MatchedOne { compiler }
            } else {
                Verdict::MatchedNeither
            }
        }
    };

    let reference = gcc.as_ref().or(clang.as_ref());
    let first_difference = reference.and_then(|theirs| first_difference(&ours, theirs));

    Row {
        case: case.clone(),
        verdict,
        chiero_diagnostics,
        first_difference,
    }
}

/// The first index at which two token sequences part company, with both spellings.
///
/// A whole-stream diff on a 40k-token file is unreadable; the first divergence is the one a
/// reader reduces from.
pub fn first_difference(ours: &[String], theirs: &[String]) -> Option<(usize, String, String)> {
    let missing = "<end>".to_owned();
    for index in 0..ours.len().max(theirs.len()) {
        let a = ours.get(index).unwrap_or(&missing);
        let b = theirs.get(index).unwrap_or(&missing);
        if a != b {
            return Some((index, a.clone(), b.clone()));
        }
    }
    None
}

/// The whole corpus, reported.
pub fn pp_gate() -> std::io::Result<Report> {
    let root = corpus_root();
    if !root.exists() {
        return Ok(Report {
            root,
            rows: Vec::new(),
            other_language: 0,
        });
    }
    let (cases, other_language) = cases(&root)?;
    let session = chiero_pp::PreprocessorSession::new();
    let predefines = gcc_predefines();
    let rows = cases
        .iter()
        .map(|case| run_case(&session, &predefines, case))
        .collect();
    Ok(Report {
        root,
        rows,
        other_language,
    })
}

#[derive(Debug)]
pub struct Report {
    pub root: PathBuf,
    pub rows: Vec<Row>,
    pub other_language: usize,
}

impl Report {
    /// Rows whose verdict is a finding, i.e. chiero matched no compiler that spoke.
    pub fn findings(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|row| {
            matches!(
                row.verdict,
                Verdict::Differs
                    | Verdict::MatchedNeither
                    | Verdict::Panicked(_)
                    | Verdict::AcceptedWhatBothRejected
            )
        })
    }

    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.rows.is_empty() {
            let _ = writeln!(
                out,
                "simplecpp checkout unavailable at {} — NOTHING WAS MEASURED",
                self.root.display()
            );
            return out;
        }
        let mut tally: BTreeMap<(Prior, String), usize> = BTreeMap::new();
        for row in &self.rows {
            let key = match &row.verdict {
                Verdict::Agree => "agree".to_owned(),
                Verdict::MatchedOne { compiler } => format!("matched {compiler} only"),
                Verdict::Differs => "DIFFERS".to_owned(),
                Verdict::RendersDifferently => "same program, rendered differently".to_owned(),
                Verdict::MatchedNeither => "MATCHED NEITHER".to_owned(),
                Verdict::RefusedByAll => "rejected by all three".to_owned(),
                Verdict::AcceptedWhatBothRejected => "ACCEPTED WHAT BOTH REJECTED".to_owned(),
                Verdict::NoCommand => "no RUN line".to_owned(),
                Verdict::Panicked(_) => "PANICKED".to_owned(),
            };
            *tally.entry((row.case.prior, key)).or_default() += 1;
        }
        let _ = writeln!(out, "corpus: {}", self.root.display());
        let _ = writeln!(
            out,
            "cases: {} C files; {} skipped as not-C (C++/CUDA/OpenCL are a declared non-goal)",
            self.rows.len(),
            self.other_language
        );
        for ((prior, verdict), count) in &tally {
            let _ = writeln!(out, "  {prior:?} / {verdict}: {count}");
        }
        let diagnosed = self
            .rows
            .iter()
            .filter(|row| !row.chiero_diagnostics.is_empty())
            .count();
        let _ = writeln!(
            out,
            "chiero emitted at least one diagnostic on {diagnosed} of {} cases",
            self.rows.len()
        );
        let _ = writeln!(out, "\nfindings, first divergence each:");
        for row in self.findings() {
            let _ = write!(out, "  {:?} {}", row.case.prior, row.case.name());
            if let Verdict::Panicked(message) = &row.verdict {
                let _ = write!(out, " PANIC: {message}");
            }
            if let Some((index, ours, theirs)) = &row.first_difference {
                let _ = write!(out, " @{index} ours={ours:?} theirs={theirs:?}");
            }
            if let Some(first) = row.chiero_diagnostics.first() {
                let _ = write!(
                    out,
                    " [{} diagnostics, first: {first}]",
                    row.chiero_diagnostics.len()
                );
            }
            let _ = writeln!(out);
        }
        out
    }
}
