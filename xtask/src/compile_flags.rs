//! **A source file's real compile flags, from the compile database.**
//!
//! `measure.sh` hand-assembles `-I`/`-D` by reading `build.ninja` with the eye, which is a second
//! reader of a fact `chiero_vpp::builddb` already parses — and the two have drifted. Measured
//! 2026-08-09 (§7.30): **198 of 935 plugin C units are exposed to include paths the harness never
//! passes**, and a 32-file sample says ~16% of those actually fail because of it, landing as
//! "chiero cannot read this" when the flags are the cause.
//!
//! This prints what the build actually used, so a harness can stop guessing:
//!
//! ```text
//! ninja -C $VPPBUILD/vpp -t compdb > /tmp/db.json
//! cargo run -p xtask -- compile-flags --db /tmp/db.json src/plugins/lldp/lldp_input.c
//! ```
//!
//! **Reads a database, never runs a build.** Regenerating `build.ninja` invalidates the baseline
//! every published VPP number was taken against (§7.24), so the expensive half stays the
//! caller's decision.

use std::path::Path;

/// The flags chiero cares about, in the order the build gave them.
///
/// **A filter, not a rewrite.** `-o`, `-c`, `-MD` and friends are about producing an object file
/// and mean nothing to a frontend; everything that changes *what the preprocessor sees* is kept,
/// which is why `-U` and `-m…` are here beside the obvious two.
/// The **include paths only** — the half that is safe to adopt.
///
/// ⛔ `frontend_flags` also returns `-march=…`, and taking that from the database is the
/// **parked** target-configuration item, not flag hygiene: the pinned 40 run with full database
/// flags keeps its summary line while 26 of 38 envelopes differ (§7.30). Include paths are
/// separable — 20 plugin files run under harness includes and real includes gave 17
/// byte-identical CIR and 0 differing — so this recovers the files that fail for want of a
/// header without changing what target the analysis is about.
pub fn include_flags(args: &[String]) -> Vec<String> {
    frontend_flags(args)
        .into_iter()
        .filter(|f| f.starts_with("-I") || f.starts_with("-isystem") || f.starts_with("-include"))
        .collect()
}

pub fn frontend_flags(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        // The separated spelling: `-I path`. The joined one falls through to the prefix test.
        if (a == "-I" || a == "-D" || a == "-U" || a == "-include" || a == "-isystem")
            && i + 1 < args.len()
        {
            out.push(format!("{a}{}", args[i + 1]));
            i += 2;
            continue;
        }
        if a.starts_with("-I")
            || a.starts_with("-D")
            || a.starts_with("-U")
            || a.starts_with("-std=")
            || a.starts_with("-m")
            || a.starts_with("-isystem")
        {
            out.push(a.clone());
        }
        i += 1;
    }
    out
}

/// `compile-flags [--db <path>] <source>` — print one line of flags per matching unit.
pub fn compile_flags(db_path: &Path, src: &Path) -> Result<Vec<String>, String> {
    flags_of(db_path, src, false)
}

/// As [`compile_flags`], but include paths only — see [`include_flags`] for why that is the
/// separable half.
pub fn include_only(db_path: &Path, src: &Path) -> Result<Vec<String>, String> {
    flags_of(db_path, src, true)
}

fn flags_of(db_path: &Path, src: &Path, includes_only: bool) -> Result<Vec<String>, String> {
    let json = std::fs::read_to_string(db_path)
        .map_err(|e| format!("cannot read {}: {e}", db_path.display()))?;
    let db = chiero_vpp::builddb::BuildDb::parse(&json)?;
    // **Suffix match, because the caller has a repo-relative path and the database is absolute.**
    // An exact compare would make every invocation a question about which cwd the caller is in.
    let want = src.to_string_lossy().to_string();
    let hits: Vec<String> = db
        .units()
        .iter()
        .filter(|u| u.src.to_string_lossy().ends_with(&want))
        .map(|u| {
            let f = if includes_only {
                include_flags(&u.args)
            } else {
                frontend_flags(&u.args)
            };
            f.join(" ")
        })
        .collect();
    if hits.is_empty() {
        // **Not an empty answer.** A file the build never compiles has no flags, and a caller
        // that treated "" as "no flags needed" would analyse it under the wrong configuration —
        // the exact confusion `--built-only` exists to prevent.
        return Err(format!("{want} is not in the compile database"));
    }
    Ok(hits)
}
