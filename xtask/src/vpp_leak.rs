//! 001 §4 rule 4 / contract 5: VPP-specific knowledge lives only in `chiero-vpp`.
//!
//! Also 040 contract 17 — *"`grep -rE 'vec_|pool_|vlib_|clib_' crates/chiero-check/src` yields
//! no hits"* — which is this rule scoped to one crate. It has been enforced here for every
//! crate since this gate existed, and was reported uncited until 2026-08-10 only because the
//! coverage instrument could not see 040 at all (§7.37). A contract met by a gate that does not
//! name it is indistinguishable, to the counter, from one nobody has looked at.
//!
//! This is a property of source text, not of the dependency graph, so it cannot be
//! checked by `deps::check`. Keeping it in its own module makes the split explicit
//! rather than leaving rule 4 silently unenforced while the gate reports success.

use std::path::{Path, PathBuf};

/// Identifiers that only VPP would use. Deliberately narrow: a false positive here
/// blocks a build, so the list names real vppinfra/vlib/vnet prefixes rather than
/// anything that merely looks VPP-ish.
const VPP_MARKERS: &[&str] = &[
    "vec_add1",
    "vec_validate",
    "vec_free",
    "vlib_",
    "vnet_",
    "clib_",
    "vppinfra",
    "unformat_",
    "pool_get",
    "pool_put",
];

/// Crates permitted to mention them.
const ALLOWED: &[&str] = &["chiero-vpp"];

#[derive(Debug, PartialEq, Eq)]
pub struct Leak {
    pub file: PathBuf,
    pub line: u32,
    pub marker: &'static str,
    pub text: String,
}

/// Scan `crates/` for VPP identifiers outside `chiero-vpp`.
///
/// Comments and test fixtures are exempt, matching contract 5's wording: the rule is
/// about knowledge baked into logic, and a comment explaining *why* a crate must not
/// know about VPP is not itself a violation.
pub fn scan(crates_dir: &Path) -> std::io::Result<Vec<Leak>> {
    let mut leaks = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(crates_dir)? {
        let e = entry?;
        let name = e.file_name().to_string_lossy().into_owned();
        if e.file_type()?.is_dir() && !ALLOWED.contains(&name.as_str()) {
            dirs.push(e.path().join("src"));
        }
    }
    dirs.sort(); // deterministic (001 §5)
    for dir in dirs {
        if dir.exists() {
            scan_dir(&dir, &mut leaks)?;
        }
    }
    leaks.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    Ok(leaks)
}

fn scan_dir(dir: &Path, out: &mut Vec<Leak>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            scan_dir(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            let src = std::fs::read_to_string(&path)?;
            for (i, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for m in VPP_MARKERS {
                    if code.contains(m) {
                        out.push(Leak {
                            file: path.clone(),
                            line: i as u32 + 1,
                            marker: m,
                            text: line.trim().to_string(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}
