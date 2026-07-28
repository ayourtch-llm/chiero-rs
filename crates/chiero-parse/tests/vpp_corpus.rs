//! Covers: 013 contracts 19, 20.
//!
//! The corpus is **real, unmodified VPP** — the transitive local include closure of six
//! vppinfra headers, copied at commit `7fe9c26`. See `corpus/vpp/PROVENANCE.md`.
//!
//! Every other test in this crate is a fixture someone wrote to make a point. These are
//! not: each of these six headers expands to a quarter of a million tokens of code that
//! predates chiero and was never adjusted to suit it, and it is the only evidence that
//! the parser works on the thing it was built for rather than on the examples in 013.

use chiero_parse::{ScopedTypedefs, parse_tu};
use chiero_pp::{Config, FileLoader, PreprocessedTu, PreprocessorSession};
use std::io;
use std::path::{Path, PathBuf};

struct Disk;
impl FileLoader for Disk {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/vpp")
}

/// The six seed headers, each preprocessed as its own translation unit.
const SEEDS: &[&str] = &[
    "vppinfra/vec.h",
    "vppinfra/pool.h",
    "vppinfra/bitmap.h",
    "vppinfra/format.h",
    "vppinfra/hash.h",
    "vppinfra/error.h",
];

/// gcc's own include directories, asked of gcc rather than guessed.
///
/// Returns `None` when gcc is not available, and the caller then **skips with a printed
/// reason** rather than passing — 022 contract 2's rule. A corpus test that silently
/// succeeded because it parsed nothing is the exact failure mode 020's empty-corpus
/// vacuity already cost this project once.
fn system_include_paths() -> Option<Vec<PathBuf>> {
    let out = std::process::Command::new("gcc")
        .args(["-E", "-v", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut paths = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("#include <...>") {
            inside = true;
            continue;
        }
        if line.starts_with("End of search list") {
            break;
        }
        if inside {
            let p = PathBuf::from(line.trim());
            if p.is_dir() {
                paths.push(p);
            }
        }
    }
    (!paths.is_empty()).then_some(paths)
}

/// gcc's predefined macros, all 391 of them.
///
/// **This is not a convenience.** Under a stub predefine set, real headers take
/// *different branches* than they do under a compiler — `__GNUC_PREREQ` fails, `__THROW`
/// and every `__attribute__` vanish, and 13% of the token stream differs. The M2 review
/// found "zero diagnostics on real headers" to be literally true and analytically
/// worthless for exactly that reason. A corpus parsed under a stub is a corpus of code
/// nobody compiles.
fn gcc_predefines() -> Option<Vec<(String, String)>> {
    let out = std::process::Command::new("gcc")
        .args(["-dM", "-E", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut defs = Vec::new();
    for line in text.lines() {
        let mut it = line.splitn(3, ' ');
        if it.next() != Some("#define") {
            continue;
        }
        let Some(name) = it.next() else { continue };
        // Function-like predefines need a parameter list this shape cannot carry, and
        // the date/time/counter ones are deliberately left to chiero.
        if name.contains('(')
            || matches!(
                name,
                "__FILE__" | "__LINE__" | "__DATE__" | "__TIME__" | "__COUNTER__"
            )
        {
            continue;
        }
        defs.push((name.to_string(), it.next().unwrap_or("1").to_string()));
    }
    (!defs.is_empty()).then_some(defs)
}

fn preprocess(seed: &str) -> Option<PreprocessedTu> {
    let sys = system_include_paths()?;
    let defines = gcc_predefines()?;
    let cfg = Config {
        include_paths: vec![corpus_dir()],
        system_paths: sys,
        defines,
        ..Config::default()
    };
    let session = PreprocessorSession::new();
    let src = format!("#include <{seed}>\n");
    Some(session.preprocess_with_loader(corpus_dir().join("tu.c"), &src, cfg, &mut Disk))
}

/// **Contract 19.** Parsing every preprocessed TU in the corpus produces zero panics, and
/// the count of TUs with diagnostics is a pinned regression metric.
///
/// Zero panics is asserted by the test completing: a panic in `parse_tu` fails the test
/// directly, and 013 §1's "never returns `Err`" means there is no other failure channel
/// to check. What needs a *number* is the diagnostic count, because "the parser produced
/// some diagnostics on real code" is true of every parser ever written and says nothing
/// about whether today's version is worse than yesterday's.
#[test]
fn every_corpus_tu_parses_without_panicking() {
    let Some(_) = system_include_paths() else {
        eprintln!("skipping: no gcc system include path found (013 contract 19)");
        return;
    };

    let mut total_tokens = 0usize;
    let mut total_nodes = 0usize;
    let mut with_diagnostics = 0usize;
    let mut report = Vec::new();

    for seed in SEEDS {
        let tu = preprocess(seed).expect("gcc was found once already");
        assert!(
            tu.diagnostics.is_empty(),
            "{seed}: the *preprocessor* must be clean before the parser's diagnostics \
             mean anything — otherwise a missing header silently empties the token \
             stream and the parser is graded on nothing: {:?}",
            tu.diagnostics.iter().take(4).collect::<Vec<_>>()
        );
        assert!(
            tu.tokens.len() > 100_000,
            "{seed} produced only {} tokens; the real header is a quarter of a million, \
             so something resolved to the wrong file",
            tu.tokens.len()
        );

        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);

        total_tokens += tu.tokens.len();
        total_nodes += parsed.ast.node_count();
        if !parsed.diagnostics.is_empty() {
            with_diagnostics += 1;
        }
        report.push((
            *seed,
            tu.tokens.len(),
            parsed.ast.node_count(),
            parsed.diagnostics.len(),
            parsed.truncated,
            parsed
                .diagnostics
                .first()
                .map(|d| d.message.clone())
                .unwrap_or_default(),
        ));
    }

    for (seed, toks, nodes, diags, truncated, first) in &report {
        eprintln!(
            "{seed}: {toks} tokens, {nodes} nodes, {diags} diagnostics{}{}",
            if *truncated { " (TRUNCATED)" } else { "" },
            if first.is_empty() {
                String::new()
            } else {
                format!(" — first: {first}")
            }
        );
    }
    eprintln!("corpus total: {total_tokens} tokens, {total_nodes} nodes");

    assert_eq!(
        with_diagnostics,
        0,
        "the pinned regression metric: {with_diagnostics} of {} corpus TUs produced \
         parser diagnostics. Real vppinfra is valid C, so any diagnostic here is a \
         parser defect, not a source defect. If a change legitimately moves this, move \
         the number **and say what construct caused it** — a metric quietly re-pinned is \
         not a metric.",
        SEEDS.len()
    );
    assert!(
        report.iter().all(|r| !r.4),
        "and no TU hit the diagnostic cap"
    );
}

/// **Contract 20.** The AST for a preprocessed TU stays under 10× the token stream size.
///
/// 013 §7 says the practical constraint is memory, not CPU: whole-tree analysis holds many
/// TUs at once, and a VPP `.c` expands to tens of megabytes of tokens. The bound is on the
/// *ratio*, so it is measured against real expansions rather than against a fixture whose
/// size someone chose.
#[test]
fn the_ast_stays_under_ten_times_the_token_stream() {
    let Some(_) = system_include_paths() else {
        eprintln!("skipping: no gcc system include path found (013 contract 20)");
        return;
    };

    // Sizes of the arena element types, which is what actually gets allocated.
    let node_bytes = std::mem::size_of::<chiero_ast::Expr>()
        .max(std::mem::size_of::<chiero_ast::Stmt>())
        .max(std::mem::size_of::<chiero_ast::Decl>())
        .max(std::mem::size_of::<chiero_ast::TypeExpr>());
    let token_bytes = std::mem::size_of::<chiero_lex::PpToken>();

    for seed in SEEDS {
        let tu = preprocess(seed).expect("gcc was found once already");
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);

        let stream = tu.tokens.len() * token_bytes;
        let ast = parsed.ast.node_count() * node_bytes;
        let ratio = ast as f64 / stream as f64;
        eprintln!(
            "{seed}: tokens {} x {token_bytes}B = {stream}B, nodes {} x <= {node_bytes}B \
             = {ast}B, ratio {ratio:.2}x",
            tu.tokens.len(),
            parsed.ast.node_count()
        );
        assert!(
            ratio < 10.0,
            "{seed}: the AST is {ratio:.2}x the token stream, over 013 §7's 10x bound"
        );
        // The bound is only meaningful if the tree is real. An empty AST has a ratio of
        // zero and would pass every version of this assertion.
        assert!(
            parsed.ast.node_count() > tu.tokens.len() / 20,
            "{seed}: {} nodes for {} tokens is too few for the tree to be real — a \
             parser that gave up early passes a memory bound trivially",
            parsed.ast.node_count(),
            tu.tokens.len()
        );
    }
}
