//! **A differential instrument: what the persona silently believes, versus what gcc says.**
//!
//! 012 contract 17's corpus gate preprocesses 1967 VPP translation units and counts diagnostics.
//! It found three real defects that way — and it was *structurally* incapable of finding the
//! worst one, because **taking the wrong `#if` branch emits nothing**. The persona defined no
//! `__BYTE_ORDER__`, so `#if __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__` read `0 == 0`, took the
//! big-endian branch on x86-64, and reversed the member order of every bit-field struct in
//! `srv6-mobile`. The gate reported a clean sweep throughout.
//!
//! So this asks a different question. Not *what did chiero complain about* but **what does chiero
//! believe that gcc does not** — intersect gcc's predefines with the identifiers VPP actually
//! tests in `#if`/`#elif`, and subtract what the persona bakes. Seconds to run, no build needed.
//!
//! It is committed rather than kept as a shell one-liner because this project has lost
//! scratchpad-only instruments twice, and because it is worth **standing**: every future gap in
//! the persona shows up here the moment VPP tests for it.

use chiero_pp::{Config, preprocess_str};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// **The boundary of this instrument, stated because a silent one is how the last blind spot got
// in.** The oracle is `gcc -dM -E` with **no flags**, so a macro gcc defines only under some
// `-march` — `__SSE4_2__`, `__AVX2__`, `__AVX512F__` — is not in the oracle's set and can never
// appear as a gap here, however often VPP tests it. That is correct behaviour and it is also this
// gate's ceiling: those macros are **per-translation-unit** facts (VPP compiles one source
// repeatedly under different `-march`), which is the PARKED item in §9.1, and no fixed persona
// has a right answer for them.
//
// A first draft carried a `PARKED_MARCH_GATED` allowlist to partition them out. It was removed
// after it matched nothing on the first real run: an allowlist that cannot fire is not a
// safeguard, it is a claim of coverage the code does not have.
//
// `__SSE__` and `__SSE2__` are *not* in that category and are checked normally — gcc defines both
// with no `-march` at all, at `-march=x86-64-v2` and at `v3`. They are x86-64 baseline, fixed
// facts about `__x86_64__` in the same way `__LP64__` is.

/// Every macro gcc predefines for this target, asked of gcc rather than remembered.
fn gcc_predefines() -> Option<BTreeSet<String>> {
    let out = std::process::Command::new("gcc")
        .args(["-dM", "-E", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let names: BTreeSet<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("#define "))
        .map(|rest| {
            rest.split([' ', '('])
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|n| !n.is_empty())
        .collect();
    (!names.is_empty()).then_some(names)
}

/// Does the persona define `name`? Asked through the preprocessor itself, so the answer is the
/// one real code gets — not a peek at a table that might not be the only source of truth.
fn persona_defines(name: &str) -> bool {
    let src = format!("#ifdef {name}\nY\n#else\nN\n#endif\n");
    preprocess_str("p.c", &src, Config::default())
        .token_texts()
        .eq(["Y"])
}

/// Identifiers named on a `#if`/`#elif` line anywhere under `root`, with one example site each.
fn identifiers_tested_in_conditionals(root: &Path) -> BTreeMap<String, (usize, String)> {
    let mut found: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("c") | Some("h")
            ) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (n, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if !(trimmed.starts_with("#if") || trimmed.starts_with("#elif")) {
                    continue;
                }
                for ident in identifiers(trimmed) {
                    let site = format!("{}:{}", path.display(), n + 1);
                    let e = found.entry(ident).or_insert((0, site));
                    e.0 += 1;
                }
            }
        }
    }
    found
}

fn identifiers(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in line.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|s| !s.starts_with(|c: char| c.is_ascii_digit()));
    out
}

/// **The gate.** A gcc predefine that VPP tests and the persona does not bake is a silent
/// divergence: `#if` reads the missing macro as `0` and picks a branch nobody chose.
///
/// Ignored because it needs the VPP tree and gcc. Run with
/// `cargo test -p chiero-vpp --test persona_gap -- --ignored --nocapture`.
#[test]
#[ignore = "external corpus — needs the VPP tree and gcc"]
fn every_gcc_predefine_vpp_tests_is_one_the_persona_has_an_answer_for() {
    let vpp = Path::new("/home/ubuntu/vpp/src");
    if !vpp.is_dir() {
        eprintln!("SKIPPED: no VPP tree at {}", vpp.display());
        return;
    }
    let Some(gcc) = gcc_predefines() else {
        eprintln!("SKIPPED: gcc unavailable, so there is nothing to differ against");
        return;
    };
    let tested = identifiers_tested_in_conditionals(vpp);
    assert!(
        gcc.len() > 300 && tested.len() > 1000,
        "the search found {} gcc predefines and {} tested identifiers; one of the two \
         inputs is empty and the gate would pass vacuously",
        gcc.len(),
        tested.len()
    );

    let mut gaps: Vec<(&String, usize, &String)> = tested
        .iter()
        .filter(|(name, _)| gcc.contains(*name))
        .filter(|(name, _)| !persona_defines(name))
        .map(|(name, (count, site))| (name, *count, site))
        .collect();
    gaps.sort_by_key(|(_, count, _)| std::cmp::Reverse(*count));

    eprintln!(
        "persona gap: {} gcc predefines, {} identifiers VPP tests in #if/#elif, {} gaps",
        gcc.len(),
        tested.len(),
        gaps.len()
    );
    for (name, count, site) in &gaps {
        eprintln!("  {count:>4}×  {name}  e.g. {site}");
    }

    assert!(
        gaps.is_empty(),
        "{} gcc predefine(s) VPP tests have no answer in the persona, so `#if` reads each as 0 \
         and picks a branch nobody chose — the `__BYTE_ORDER__` failure mode, which emits no \
         diagnostic at all: {:?}",
        gaps.len(),
        gaps.iter().map(|(n, ..)| n.as_str()).collect::<Vec<_>>()
    );
}
