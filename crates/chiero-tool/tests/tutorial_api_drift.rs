//! **A tutorial is a claim about the API, and until now nothing compared the two.**
//!
//! The first end-to-end user found three drifts on 2026-08-10 by trying to use the tutorials
//! (`~/from-claude-mini-2026-08-10-user-test.md`, finding 5). Two were found and fixed by
//! hand; both were the same shape, and both had survived every gate in a repository full of
//! them, because a tutorial is prose:
//!
//! | tutorial | said | the source says |
//! |---|---|---|
//! | 03 | `ExcludedTest { test, entity, proof, fidelity }` | the field is `refinement`; there is no `proof` |
//! | 01 | `Fresh \| Stale { .. } \| Unknown` | `Fresh \| Stale \| Partial` — there is no `Unknown` |
//!
//! `tutorials.rs` next door says "every code block in `docs/tutorials/` runs here", and it
//! does — but it *re-writes* each example as Rust, so the tutorial's own text is never read.
//! A hand-copied example and its original drift apart silently, which is exactly what
//! happened.
//!
//! **So this reads the tutorials and the crate sources and compares the names.** Two rules,
//! chosen because they are the two shapes the real drifts took:
//!
//! 1. a **field list** — `Type { a, b, c }` where `Type` is a struct this workspace defines:
//!    every name must be one of its fields;
//! 2. a **variant alternation** — `A | B | C` where two or more of the names are variants of
//!    one enum: the rest must be variants of that enum too, and `Enum::Variant` likewise.
//!
//! ⚠️ **What it deliberately does not do.** It says nothing about a type it cannot find: a
//! tutorial may name `Vec`, `Option`, or a type from a dependency, and treating "unknown" as
//! "wrong" would make the gate unusable within a week. So this catches a *wrong name on a
//! known type*, not an invented type. The former is what a reader trips over — they went
//! looking for `Validity::Unknown` and there was nothing there.
//!
//! **Mutation-tested against a corpus whose answer was known in advance** (§8.3's strongest
//! form): restoring either pre-2026-08-10 line turns this red, and naming the exact field.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Lines whose shape trips a rule for a reason that is not drift.
///
/// **A reason, not a name** — an allowlist of bare strings is how a gap becomes permanent.
const EXCLUDED: &[(&str, &str)] = &[];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The name declared by `pub struct NAME` / `pub enum NAME`, if the line declares one.
fn declares<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    let t = line.trim_start();
    let rest = t.strip_prefix("pub ")?;
    let rest = rest.strip_prefix(kw)?.strip_prefix(' ')?;
    let name = rest
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()?;
    // A unit struct (`pub struct Disk;`) has no body to read.
    if !is_ident(name) || !t.contains('{') {
        return None;
    }
    Some(name)
}

/// The body of a `{ ... }` block opened on `lines[i]`, as the lines between the braces.
fn block<'a>(lines: &[&'a str], i: usize) -> Vec<&'a str> {
    let indent = lines[i].len() - lines[i].trim_start().len();
    let mut out = Vec::new();
    for line in &lines[i + 1..] {
        let t = line.trim_start();
        if t.starts_with('}') && line.len() - t.len() <= indent {
            break;
        }
        out.push(*line);
    }
    out
}

/// Every `pub struct`'s field names and every `pub enum`'s variant names, by type name.
///
/// **Union on collision.** Two crates may both define a `Config`; merging their members is the
/// lenient direction, and a gate about documentation should not fail on an ambiguity the
/// documentation does not have.
fn api() -> (
    BTreeMap<String, BTreeSet<String>>,
    BTreeMap<String, BTreeSet<String>>,
) {
    let mut files = Vec::new();
    rust_sources(&root().join("crates"), &mut files);
    let mut structs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut enums: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for i in 0..lines.len() {
            if let Some(name) = declares(lines[i], "struct") {
                let e = structs.entry(name.to_string()).or_default();
                for b in block(&lines, i) {
                    let t = b.trim_start().trim_start_matches("pub ");
                    let t = t.split_once("(crate)").map_or(t, |(_, r)| r.trim_start());
                    if let Some((field, _)) = t.split_once(':')
                        && is_ident(field.trim())
                        && field
                            .trim()
                            .starts_with(|c: char| c.is_lowercase() || c == '_')
                    {
                        e.insert(field.trim().to_string());
                    }
                }
            }
            if let Some(name) = declares(lines[i], "enum") {
                let indent = lines[i].len() - lines[i].trim_start().len();
                let e = enums.entry(name.to_string()).or_default();
                for b in block(&lines, i) {
                    // Only the variants themselves: a struct variant's own fields sit deeper
                    // and are lowercase anyway, but depth is the honest test.
                    if b.len() - b.trim_start().len() != indent + 4 {
                        continue;
                    }
                    let v: String = b
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if is_ident(&v) && v.starts_with(char::is_uppercase) {
                        e.insert(v);
                    }
                }
            }
        }
    }
    assert!(
        structs.len() > 50 && enums.len() > 20,
        "read only {} structs and {} enums out of the crates, which cannot be right",
        structs.len(),
        enums.len()
    );
    (structs, enums)
}

fn tutorials() -> Vec<(String, String)> {
    let dir = root().join("docs/tutorials");
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("docs/tutorials")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.extension().is_some_and(|e| e == "md") {
            out.push((
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).expect("tutorial"),
            ));
        }
    }
    assert!(out.len() >= 8, "found {} tutorials", out.len());
    out
}

fn excluded(line: &str) -> bool {
    EXCLUDED.iter().any(|(l, _)| line.contains(l))
}

/// Rule 1 — `Type { a, b, c }` naming a struct this workspace defines.
#[test]
fn a_field_list_in_a_tutorial_names_fields_that_exist() {
    let (structs, _) = api();
    let mut bad = Vec::new();
    for (file, text) in tutorials() {
        for (n, line) in text.lines().enumerate() {
            if excluded(line) {
                continue;
            }
            for (open, _) in line.match_indices('{') {
                let head = line[..open].trim_end();
                let name: String = head
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let Some(fields) = structs.get(&name) else {
                    continue;
                };
                let Some(close) = line[open..].find('}') else {
                    continue;
                };
                for part in line[open + 1..open + close].split(',') {
                    let word = part.split(':').next().unwrap_or_default().trim();
                    let word = word.trim_start_matches('`').trim_end_matches('`');
                    if word.is_empty() || word == ".." || !is_ident(word) {
                        continue;
                    }
                    if !fields.contains(word) {
                        bad.push(format!(
                            "{file}:{}: `{name} {{ … {word} … }}` — `{name}` has no field \
                             `{word}`. Its fields are {fields:?}",
                            n + 1
                        ));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "tutorial↔API drift:\n  {}",
        bad.join("\n  ")
    );
}

/// The identifier a `|`-separated fragment refers to: the last word before the first `|`, the
/// first word after each later one, with any `Enum::` prefix dropped.
fn alternative(part: &str, first: bool) -> Option<String> {
    let words: Vec<&str> = part.split_whitespace().collect();
    let w = if first {
        *words.last()?
    } else {
        *words.first()?
    };
    let w = w.rsplit("::").next()?;
    let w: String = w
        .chars()
        .skip_while(|c| !c.is_alphanumeric() && *c != '_')
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (is_ident(&w) && w.starts_with(char::is_uppercase)).then_some(w)
}

/// Rule 2 — `A | B | C` where two or more names are variants of one enum.
#[test]
fn a_variant_alternation_in_a_tutorial_names_variants_that_exist() {
    let (_, enums) = api();
    let mut bad = Vec::new();
    for (file, text) in tutorials() {
        for (n, line) in text.lines().enumerate() {
            if excluded(line) || line.contains("||") {
                continue;
            }
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 3 {
                continue;
            }
            let names: Vec<String> = parts
                .iter()
                .enumerate()
                .filter_map(|(i, p)| alternative(p, i == 0))
                .collect();
            if names.len() != parts.len() {
                continue;
            }
            // **Two, not one.** One match is a coincidence — `Full` and `Exact` are variants of
            // something almost everywhere. Two names from the same enum on one line is the
            // alternation this rule is about.
            for (ty, variants) in &enums {
                let known = names.iter().filter(|v| variants.contains(*v)).count();
                if known < 2 || known == names.len() {
                    continue;
                }
                for v in names.iter().filter(|v| !variants.contains(*v)) {
                    bad.push(format!(
                        "{file}:{}: `{}` — `{ty}` has no variant `{v}`. Its variants are \
                         {variants:?}",
                        n + 1,
                        names.join(" | ")
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "tutorial↔API drift:\n  {}",
        bad.join("\n  ")
    );
}

/// Rule 2b — a written-out `Enum::Variant`, which is the same claim without the alternation.
#[test]
fn a_qualified_variant_in_a_tutorial_exists() {
    let (structs, enums) = api();
    let mut bad = Vec::new();
    for (file, text) in tutorials() {
        for (n, line) in text.lines().enumerate() {
            if excluded(line) {
                continue;
            }
            for (at, _) in line.match_indices("::") {
                let ty: String = line[..at]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let Some(variants) = enums.get(&ty) else {
                    continue;
                };
                // A type name that is also a struct is a module path as often as an enum path;
                // and an associated function is not a variant, so only capitalised names count.
                if structs.contains_key(&ty) {
                    continue;
                }
                let item: String = line[at + 2..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !is_ident(&item) || !item.starts_with(char::is_uppercase) {
                    continue;
                }
                if !variants.contains(&item) {
                    bad.push(format!(
                        "{file}:{}: `{ty}::{item}` — `{ty}` has no such variant. Its variants \
                         are {variants:?}",
                        n + 1
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "tutorial↔API drift:\n  {}",
        bad.join("\n  ")
    );
}
