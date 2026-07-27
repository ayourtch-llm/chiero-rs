//! Contract coverage against the M1 exit criterion.
//!
//! [080](../../docs/specs/080-roadmap.md) states M1's exit as "**all** numbered contracts
//! of 020, 021, 022, 023 and 024 are green", deliberately not as numeric ranges — an
//! earlier draft used ranges written before the review waves, and they excluded precisely
//! the contracts the reviews added. 070 §4 now requires gates to name *documents*.
//!
//! A range cannot be checked mechanically; a name can. This walks the specs for numbered
//! contracts and the test sources for citations of the form `NNN contract K`, and reports
//! what is not yet cited. It is a **coverage** measure, not a correctness one: a citation
//! says a test claims to cover that contract, which this project has repeatedly found is
//! not the same as covering it. It answers "what has nobody looked at", which is the
//! question M1's exit needs and guesswork was answering.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The documents M1's exit names.
pub const M1_DOCS: &[&str] = &["020", "021", "022", "023", "024"];

#[derive(Debug, Default)]
pub struct Coverage {
    /// Contract ids declared by each document, in source order.
    pub declared: BTreeMap<String, Vec<String>>,
    /// Contract ids cited by at least one test.
    pub cited: BTreeSet<String>,
}

impl Coverage {
    pub fn uncovered(&self, doc: &str) -> Vec<String> {
        self.declared
            .get(doc)
            .map(|v| {
                v.iter()
                    .filter(|c| !self.cited.contains(&format!("{doc}:{c}")))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Numbered contracts live under a `## Testable contracts` heading and start a line as
/// `12.` or `21c.` — the shape every spec in this set uses.
fn contracts_in(text: &str) -> Vec<String> {
    // The heading is numbered — "## 10. Testable contracts" — so match the tail, not a
    // fixed string. Matching the whole heading silently found nothing and reported full
    // coverage of an empty set, which is the most misleading answer a gate can give.
    let Some(start) = text.find("Testable contracts") else {
        return Vec::new();
    };
    let body = &text[start..];
    let mut out = Vec::new();
    for line in body.lines() {
        let Some(dot) = line.find('.') else { continue };
        let id = &line[..dot];
        if id.is_empty() || id.len() > 4 || !line[dot..].starts_with(". ") {
            continue;
        }
        let mut chars = id.chars();
        let digits_then_letter = chars.clone().take_while(|c| c.is_ascii_digit()).count();
        if digits_then_letter == 0 {
            continue;
        }
        let rest: String = chars.by_ref().skip(digits_then_letter).collect();
        if rest.len() > 1 || rest.chars().any(|c| !c.is_ascii_lowercase()) {
            continue;
        }
        out.push(id.to_string());
    }
    out
}

pub fn measure(root: &Path) -> std::io::Result<Coverage> {
    let mut cov = Coverage::default();
    for doc in M1_DOCS {
        let dir = root.join("docs/specs");
        let Some(path) = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&format!("{doc}-")))
            })
        else {
            continue;
        };
        let text = std::fs::read_to_string(&path)?;
        cov.declared.insert((*doc).to_string(), contracts_in(&text));
    }

    // Citations are prose — `023 contract 10` — because that is how they are already
    // written, and a machine-readable annotation nobody maintains is worse than a
    // convention the comments already follow.
    // `xtask` too: 023 contract 13a is enforced by `check-proof-surface`, not by a test,
    // and a gate that only looked at `crates/` would report it uncovered and be wrong.
    let mut sources = Vec::new();
    collect_rs(&root.join("crates"), &mut sources)?;
    collect_rs(&root.join("xtask"), &mut sources)?;
    for f in sources {
        let text = std::fs::read_to_string(&f)?;
        for cap in cite_ids(&text) {
            cov.cited.insert(cap);
        }
    }
    Ok(cov)
}

/// Citations are prose, so the scanner reads prose: `023 contract 10`, and also
/// `020 contracts 19 and 20`. Accepting only the singular form would push authors into
/// writing "020 contract 19. 020 contract 20." to satisfy a gate — and a gate that forces
/// unnatural prose gets worked around rather than followed.
fn cite_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let pat = b" contract";
    let mut i = 0;
    while let Some(pos) = find(&bytes[i..], pat) {
        let at = i + pos;
        // Three digits immediately before the marker. Sliced on **bytes**, not `&str`,
        // because a spec comment three bytes earlier may be mid-`§` — the specs are full
        // of them, and `&text[at - 3..at]` panics there.
        if at >= 3 {
            let doc = std::str::from_utf8(&bytes[at - 3..at]).unwrap_or("");
            if doc.len() == 3 && doc.chars().all(|c| c.is_ascii_digit()) {
                let mut rest = &text[at + pat.len()..];
                rest = rest.strip_prefix('s').unwrap_or(rest);
                // A run of ids joined by `, ` or ` and `, so a sentence about two
                // contracts cites both.
                loop {
                    let r = rest.trim_start_matches([' ', ',']);
                    let r = r.strip_prefix("and ").unwrap_or(r);
                    let r = r.trim_start();
                    let id: String = r
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
                        .collect();
                    if id.is_empty() || !id.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        break;
                    }
                    out.push(format!("{doc}:{id}"));
                    rest = &r[id.len()..];
                    if !(rest.starts_with(", ") || rest.starts_with(" and ")) {
                        break;
                    }
                }
            }
        }
        i = at + pat.len();
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn collect_rs(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            if p.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_rs(&p, out)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
    Ok(())
}
