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
    // The **heading**, not the first mention. `text.find("Testable contracts")` matched a
    // cross-reference in §1 and absorbed every numbered list before the real section — one
    // editorial sentence moved the denominator from 44 to 60. Anchor on a line that starts
    // with `#` and ends with the phrase.
    let Some(start) = text
        .lines()
        .scan(0usize, |off, l| {
            let at = *off;
            *off += l.len() + 1;
            Some((at, l))
        })
        .find(|(_, l)| l.starts_with('#') && l.trim_end().ends_with("Testable contracts"))
        .map(|(at, _)| at)
    else {
        return Vec::new();
    };
    // **And stop at the next top-level heading.** Slicing to end of file made an appendix
    // with a numbered list into phantom contracts — three of which collided with real ids
    // 1-3 and were counted as *cited*. 020 already has `###` subsections inside this
    // block, so only `##` ends it.
    let rest = &text[start..];
    let body = match rest
        .match_indices("\n## ")
        .find(|(i, _)| *i > 0)
        .map(|(i, _)| i)
    {
        Some(end) => &rest[..end],
        None => rest,
    };
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
    // **This file cites contracts as syntax examples in its own doc comments**, and
    // counted them: `023 contract 10`, `023 contract 13a` and `020 contracts 19 and 20`
    // were covered by the scanner describing itself. A measuring instrument must not be
    // part of what it measures.
    sources.retain(|p| !p.ends_with("contracts.rs"));
    // **Only tests and gates count.** Prose in `src/` mentioning a contract is not a test
    // of it: `024 contract 22` was "covered" by a doc comment about deduplication that
    // happened to list it, and `021 contract 3` by a sentence explaining why it is
    // *unimplementable*. Both are entirely untested. `xtask/` stays because gates like
    // `check-proof-surface` genuinely enforce contracts.
    sources.retain(|p| {
        p.components().any(|c| c.as_os_str() == "tests")
            || p.components().any(|c| c.as_os_str() == "xtask")
    });
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
        // The document number, which is not always immediately before the marker:
        // `024 §4, contracts 6-9` is a real form in this tree and was missed entirely,
        // reporting a section header's contracts as uncited. Scan back over a short run of
        // section/punctuation characters to the three digits.
        if at >= 3 {
            let doc = doc_before(bytes, at);
            if let Some(doc) = doc {
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
                    // A range — `contracts 6-9`, `contracts 1–3` — cites every id in it.
                    // Both forms are already in the tree and both were dropping all but
                    // the first, so real coverage read as missing.
                    let after = &r[id.len()..];
                    if let Some(tail) = after
                        .strip_prefix('-')
                        .or_else(|| after.strip_prefix('\u{2013}'))
                        && let Ok(lo) = id.parse::<u32>()
                    {
                        let hi: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(hi) = hi.parse::<u32>()
                            && hi > lo
                            && hi - lo < 64
                        {
                            for k in (lo + 1)..=hi {
                                out.push(format!("{doc}:{k}"));
                            }
                            rest = &tail[hi.to_string().len()..];
                            if !(rest.starts_with(", ") || rest.starts_with(" and ")) {
                                break;
                            }
                            continue;
                        }
                    }
                    rest = &r[id.len()..];
                    // Only continue into another *contract* id. `024 contract 8 and 023
                    // §7` parsed the `023` as a second contract of 024 — a phantom that
                    // was harmless only because "023" != "23". A continuation followed by
                    // a document reference is not a continuation.
                    let next = rest
                        .strip_prefix(", ")
                        .or_else(|| rest.strip_prefix(" and "))
                        .unwrap_or("");
                    let looks_like_a_doc = next.len() >= 4
                        && next[..3].chars().all(|c| c.is_ascii_digit())
                        && !next[3..4]
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_digit());
                    if next.is_empty() || looks_like_a_doc {
                        break;
                    }
                }
            }
        }
        i = at + pat.len();
    }
    out
}

/// The `NNN` a citation belongs to, allowing a short `§4,`-style interlude between it and
/// the word `contract`. Bytes, not `&str`: a `§` three bytes back is not a char boundary.
fn doc_before(bytes: &[u8], at: usize) -> Option<String> {
    let mut end = at;
    // Skip back over at most a dozen bytes of section reference and punctuation.
    let lo = at.saturating_sub(12);
    while end > lo {
        let c = bytes[end - 1];
        if c.is_ascii_digit() && end >= 3 {
            let start = end - 3;
            let d = std::str::from_utf8(&bytes[start..end]).ok()?;
            if d.len() == 3 && d.chars().all(|c| c.is_ascii_digit()) {
                // Not part of a longer number.
                if start == 0 || !bytes[start - 1].is_ascii_digit() {
                    return Some(d.to_string());
                }
            }
            return None;
        }
        if matches!(c, b',' | b';' | b' ') || !c.is_ascii() {
            end -= 1;
            continue;
        }
        return None;
    }
    None
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
