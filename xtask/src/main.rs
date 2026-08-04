//! Build/CI automation. See `docs/specs/001-architecture.md` §4 and
//! `docs/specs/070-testing-and-tdd-protocol.md` §4.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check-deps") => check_deps(),
        Some("contract-coverage") => contract_coverage(),
        Some("check-vpp-leak") => check_vpp_leak(),
        Some("sweep") => sweep(),
        Some("recipe-sweep") => recipe_sweep(),
        // `CC=...` shim: everything after `cc` is the compiler's own argument list.
        Some("cc-report") => cc_report(),
        Some("cc") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let code = xtask::cc::run(&args);
            if code == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(u8::try_from(code).unwrap_or(1))
            }
        }
        Some("check-proof-surface") => match xtask::proof_surface::check_proof_surface() {
            0 => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        },
        Some(other) => {
            eprintln!("unknown task: {other}");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <task>\n\n  \
         check-deps       enforce the 001 §4 graph rules (1,2,3,5,6,7)\n  \
         check-vpp-leak   enforce 001 §4 rule 4 / contract 5\n  \
         check-proof-surface  enforce 023 contract 13a (a proof cannot be forged)\n  \
         contract-coverage    report M1 exit coverage over 020-024 (080)\n  \
         sweep --tree P [-I dir] [-D name[=v]] [--std gnu11]\n                   \
         report where chiero and gcc disagree over an external C tree"
    );
}

/// 001 contract 8: exit non-zero when a §4 rule is violated.
fn check_deps() -> ExitCode {
    match xtask::deps::workspace_graph() {
        Ok(g) => xtask::deps::report(&g),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 001 §4 rule 4 / contract 5.
fn check_vpp_leak() -> ExitCode {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("crates");
    match xtask::vpp_leak::scan(&crates) {
        Ok(leaks) if leaks.is_empty() => {
            println!("check-vpp-leak: no VPP identifiers outside chiero-vpp");
            ExitCode::SUCCESS
        }
        Ok(leaks) => {
            eprintln!("check-vpp-leak: {} leak(s)\n", leaks.len());
            for l in &leaks {
                eprintln!(
                    "  {}:{}: `{}` in: {}",
                    l.file.display(),
                    l.line,
                    l.marker,
                    l.text
                );
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 080's M1 exit is "**all** numbered contracts of 020-024 are green", named as documents
/// rather than ranges. This reports which are not cited by any test — a coverage measure,
/// not a correctness one, answering "what has nobody looked at".
fn contract_coverage() -> ExitCode {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    match xtask::contracts::measure(&root) {
        Ok(cov) => {
            let mut total = 0usize;
            let mut missing = 0usize;
            for doc in xtask::contracts::M1_DOCS {
                let declared = cov.declared.get(*doc).map(|v| v.len()).unwrap_or(0);
                let un = cov.uncovered(doc);
                total += declared;
                missing += un.len();
                println!(
                    "{doc}: {}/{} cited{}",
                    declared - un.len(),
                    declared,
                    if un.is_empty() {
                        String::new()
                    } else {
                        format!("  — uncited: {}", un.join(", "))
                    }
                );
            }
            println!(
                "\nM1 exit: {}/{} contracts cited by a test",
                total - missing,
                total
            );

            // **The frontend, measured separately.** 080's M2 exit is stated as
            // behaviours rather than as "all contracts of these documents", so this is
            // not a gate and is never folded into the M1 number. It is reported because a
            // coverage tool that cannot see half the work in flight reports a comfortable
            // number about the half it can.
            let mut ftotal = 0usize;
            let mut fmissing = 0usize;
            let mut lines = Vec::new();
            for doc in xtask::contracts::M2_DOCS {
                let declared = cov.declared.get(*doc).map(|v| v.len()).unwrap_or(0);
                if declared == 0 {
                    continue;
                }
                let un = cov.uncovered(doc);
                ftotal += declared;
                fmissing += un.len();
                lines.push(format!(
                    "{doc}: {}/{} cited{}",
                    declared - un.len(),
                    declared,
                    if un.is_empty() {
                        String::new()
                    } else {
                        format!("  — uncited: {}", un.join(", "))
                    }
                ));
            }
            if ftotal > 0 {
                println!("\nfrontend (measure, not a gate):");
                for l in lines {
                    println!("  {l}");
                }
                println!("  total: {}/{} cited", ftotal - fmissing, ftotal);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Sweep an external tree. **A report, never a gate** — it exits 0 whatever it finds, because
/// the tree is not this project's to keep clean and the suite must not depend on one.
fn sweep() -> ExitCode {
    let mut args = std::env::args().skip(2);
    let mut tree = None;
    let mut flags = xtask::sweep::Flags::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tree" => tree = args.next().map(std::path::PathBuf::from),
            "-I" => {
                if let Some(v) = args.next() {
                    flags.includes.push(std::path::PathBuf::from(v));
                }
            }
            "-D" => {
                if let Some(v) = args.next() {
                    flags.defines.push(v);
                }
            }
            "--std" => flags.std = args.next(),
            "--gnu" | "--no-pedantic" => flags.dialect = chiero_ast::Dialect::gnu(),
            other => {
                eprintln!("sweep: unknown argument {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(tree) = tree else {
        eprintln!("sweep: --tree <path> is required");
        return ExitCode::FAILURE;
    };
    // gcc's own system paths, so chiero resolves `<stdio.h>` the way the tree's compiler does.
    let system = xtask::sweep::system_include_paths();
    match xtask::sweep::sweep(&tree, &flags, &system) {
        Ok(v) => {
            xtask::sweep::report(&v, &tree);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("sweep: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `xtask recipe-sweep --tree <dir> --recipes <dir>` — 042 contract 7.
///
/// Reports per-recipe candidate counts and the elapsed time, and **never exits non-zero on
/// what it found**: this is a measurement, and a gate that failed on a candidate count would
/// make every new VPP file a build break.
fn recipe_sweep() -> ExitCode {
    let mut tree = None;
    let mut recipe_dir = None;
    let mut includes = Vec::new();
    let mut defines = Vec::new();
    let mut std_flag = None;
    let mut args = std::env::args().skip(2);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tree" => tree = args.next(),
            "--recipes" => recipe_dir = args.next(),
            "-I" => includes.extend(args.next().map(std::path::PathBuf::from)),
            "-D" => defines.extend(args.next()),
            "--std" => std_flag = args.next(),
            other => eprintln!("recipe-sweep: ignoring `{other}`"),
        }
    }
    let (Some(tree), Some(recipe_dir)) = (tree, recipe_dir) else {
        eprintln!("usage: xtask recipe-sweep --tree <dir> --recipes <dir>");
        return ExitCode::FAILURE;
    };

    let mut recipes = Vec::new();
    let mut entries: Vec<_> = match std::fs::read_dir(&recipe_dir) {
        Ok(d) => d.filter_map(Result::ok).map(|e| e.path()).collect(),
        Err(e) => {
            eprintln!("recipe-sweep: cannot read {recipe_dir}: {e}");
            return ExitCode::FAILURE;
        }
    };
    entries.sort();
    for path in entries {
        if path.extension().is_none_or(|e| e != "recipe") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("recipe-sweep: cannot read {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        match chiero_recipe::load(&text) {
            Ok(r) => recipes.push(r),
            // **A recipe that does not load fails the run.** 042 §5 makes an unadjudicable
            // recipe a load error; carrying on without it would sweep a smaller catalogue than
            // the one the operator asked for and report the result as if it were whole.
            Err(errs) => {
                for e in errs {
                    eprintln!("recipe-sweep: {}: {e}", path.display());
                }
                return ExitCode::FAILURE;
            }
        }
    }

    let files = match xtask::sweep::translation_units(std::path::Path::new(&tree)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("recipe-sweep: cannot walk {tree}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The same configuration `sweep` uses: without gcc's predefines chiero takes `#if`
    // branches gcc never compiles, and every VPP file fails on its first header.
    let cfg = chiero_pp::Config {
        include_paths: includes,
        system_paths: xtask::sweep::system_include_paths(),
        defines: xtask::sweep::gcc_predefines(std_flag.as_deref())
            .into_iter()
            .chain(defines.iter().map(|d| match d.split_once('=') {
                Some((k, v)) => (k.to_owned(), v.to_owned()),
                None => (d.clone(), "1".to_owned()),
            }))
            .collect(),
        ..chiero_pp::Config::default()
    };
    let started = std::time::Instant::now();
    let report = xtask::sweep::tier1_sweep(&files, &recipes, &cfg);
    let elapsed = started.elapsed();

    println!(
        "tier 1 over {} translation units in {:.1}s on {} workers — {} functions, {} files unreadable",
        report.files,
        elapsed.as_secs_f64(),
        report.threads,
        report.functions,
        report.unreadable
    );
    for t in &report.tallies {
        let note = if t.is_complete() {
            String::new()
        } else {
            format!("  (+{} undecidable: selector needs the AST)", t.needs_ast)
        };
        println!("  {:5}  {}{note}", t.matched, t.recipe);
    }
    if !report.is_complete() {
        println!("  -> PARTIAL: these counts are not a baseline until they are complete");
    }
    ExitCode::SUCCESS
}

/// `xtask cc-report --log <file>` — what a build collected.
fn cc_report() -> ExitCode {
    let mut log = std::env::var("CHIERO_CC_LOG").ok();
    let mut tree = None;
    let mut args = std::env::args().skip(2);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--log" => log = args.next(),
            "--tree" => tree = args.next(),
            _ => {}
        }
    }
    // **A tree of sidecars is the default**, since that is what the shim writes: one
    // `<output>.chiero` per translation unit, no shared file to contend for.
    let lines: Vec<String> = match (&tree, &log) {
        (Some(t), _) => xtask::cc::collect_sidecars(std::path::Path::new(t)),
        (None, Some(l)) => match std::fs::read_to_string(l) {
            Ok(t) => t.lines().map(str::to_owned).collect(),
            Err(e) => {
                eprintln!("cc-report: cannot read {l}: {e}");
                return ExitCode::FAILURE;
            }
        },
        (None, None) => {
            eprintln!("usage: xtask cc-report --tree <build-dir>   [or --log <file>]");
            return ExitCode::FAILURE;
        }
    };
    let s = xtask::cc::summarise(&lines);
    println!("{} translation units observed, {} clean", s.total, s.clean);
    // **A build the shim never saw is not a clean build.** Saying so beats printing a
    // reassuring pair of zeroes at someone who set the variable wrongly.
    if s.total == 0 {
        println!("  -> nothing recorded: did the build actually run with CC set to the shim?");
        return ExitCode::SUCCESS;
    }
    for (kind, n) in s.kinds.iter().take(25) {
        println!("  {n:5}  {kind}");
    }
    if s.kinds.len() > 25 {
        println!("  … {} more distinct kinds", s.kinds.len() - 25);
    }
    ExitCode::SUCCESS
}
