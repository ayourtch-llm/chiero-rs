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

/// What `name` expands to under gcc, as text. `None` for anything that is not a plain
/// object-like macro (a function-like one needs arguments and is not comparable this way).
fn gcc_expansion(name: &str, dir: &Path) -> Option<String> {
    let path = dir.join(format!("v_{name}.c"));
    std::fs::write(&path, format!("{name}\n")).ok()?;
    let out = std::process::Command::new("gcc")
        .args(["-E", "-P"])
        .arg(&path)
        .output()
        .ok()?;
    let _ = std::fs::remove_file(&path);
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .collect()
    })
}

/// What the persona expands `name` to.
fn persona_expansion(name: &str) -> String {
    preprocess_str("v.c", &format!("{name}\n"), Config::default())
        .token_texts()
        .collect()
}

/// **Divergences that are deliberate, each with the reason it is deliberate — and each of
/// which must still be happening.**
///
/// ⚠️ This doc used to read *"unlike the allowlist removed above, this one fires — every
/// entry is a difference the gate really sees on every run"*. That was false for five of its
/// six entries, and nothing checked it: the five below have moved to [`NEVER_COMPARABLE`].
/// The gate now asserts the claim instead of making it, so the sentence cannot go stale
/// again without a red run.
///
/// The rule is the one the generated differential suite's `KNOWN_GAPS` arrived at the same
/// day, from the opposite direction: an excuse with no divergence behind it is not inert. It
/// excuses that divergence silently if it ever appears, and by then nobody is deciding.
const DELIBERATE: &[(&str, &str)] = &[
    // 013 makes the parser C11 + GNU extensions. gcc's default here is gnu17, so it reports
    // `201710L`. Claiming C17 in the persona while the parser implements C11 would be a *worse*
    // lie than the one this gate exists to catch, so the difference stays until the language
    // level is a decision someone has made rather than a number that drifted. See §9.1.
    (
        "__STDC_VERSION__",
        "the parser is C11 (013); gcc's default -std here is gnu17",
    ),
];

/// **Names that cannot agree by construction**, exempt from the liveness rule above.
///
/// These are a different kind of claim, and holding them to `DELIBERATE`'s rule would be
/// wrong rather than merely strict. `DELIBERATE` says *"chiero differs from gcc here today"*,
/// which stops being true the day it is fixed. These say *"this name could never match, so
/// excuse it if it ever shows up"* — and whether it shows up depends on whether **VPP**
/// happens to write `#if __DATE__`, which is a property of VPP and not of chiero.
///
/// That is why none of them fires: the comparison loop only visits names VPP tests in an
/// `#if`/`#elif`, and VPP tests none of these. Requiring them to fire would be requiring
/// somebody to write nonsense C in VPP.
///
/// ⚠️ **The exemption is the same one `DECLARED_FIDELITY` gets in the generated differential
/// suite, and it has the same cost: nothing exercises this list.** Two lists reaching the
/// same split independently is some evidence the distinction is real; it is not evidence
/// that either list is right.
const NEVER_COMPARABLE: &[(&str, &str)] = &[
    // Deliberately not constant across runs, by 012 contract 15.
    (
        "__DATE__",
        "012 contract 15 — not constant across compilers",
    ),
    (
        "__TIME__",
        "012 contract 15 — not constant across compilers",
    ),
    (
        "__FILE__",
        "names the file being preprocessed, which differs by construction",
    ),
    ("__LINE__", "names the line, which differs by construction"),
    ("__COUNTER__", "stateful by design"),
];

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

    // **Definedness is only half the question.** A persona that answered `__GNUC__ 4` would have
    // passed everything above; `#if __GNUC__ > 4` is three lines of VPP's own crypto engine.
    // So compare what each side actually *expands to*, for the same set of names.
    let dir = std::env::temp_dir().join(format!("chiero-persona-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let mut differing = Vec::new();
    let mut compared = 0;
    for name in tested.keys().filter(|n| gcc.contains(*n)) {
        let Some(theirs) = gcc_expansion(name, &dir) else {
            continue;
        };
        // A macro that expands to its own name is one gcc did not really define as an object.
        if theirs == *name {
            continue;
        }
        compared += 1;
        let ours = persona_expansion(name);
        if ours != theirs {
            differing.push((name.clone(), ours, theirs));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        compared > 10,
        "only {compared} values compared; the value half of this gate is not running"
    );

    let excuse = |n: &String| {
        DELIBERATE
            .iter()
            .chain(NEVER_COMPARABLE)
            .find(|(d, _)| d == n)
            .map(|(_, why)| *why)
    };
    let (excused, live): (Vec<_>, Vec<_>) =
        differing.iter().partition(|(n, ..)| excuse(n).is_some());
    eprintln!(
        "persona values: {compared} compared, {} differ",
        differing.len()
    );
    for (name, ours, theirs) in &excused {
        let why = excuse(name).unwrap_or("");
        eprintln!("  deliberate  {name}: chiero {ours:?} vs gcc {theirs:?} — {why}");
    }
    for (name, ours, theirs) in &live {
        eprintln!("  DIFFERS     {name}: chiero {ours:?} vs gcc {theirs:?}");
    }
    assert!(
        live.is_empty(),
        "{} predefine value(s) VPP tests disagree with gcc, with no recorded reason: {:?}",
        live.len(),
        live.iter().map(|(n, ..)| n.as_str()).collect::<Vec<_>>()
    );

    // **And the excuse list read backwards.** `DELIBERATE`'s doc claims every entry is a
    // difference the gate really sees on every run — a claim nothing checked, which is the
    // same shape as the `KNOWN_GAPS` entry that outlived its gap by two hundred waves.
    let unfired: Vec<&str> = DELIBERATE
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !differing.iter().any(|(d, ..)| d == n))
        .collect();
    assert!(
        unfired.is_empty(),
        "{} DELIBERATE entr(ies) excused nothing. An excuse with no divergence behind it is \
         a claim about chiero that nothing checks, and it silently excuses the divergence if \
         it ever appears: {unfired:?}",
        unfired.len()
    );

    // **And the exemption from that rule is not a place to file things.**
    //
    // Without this, the liveness rule above is dodged by moving an entry one list down — a
    // mutant that did exactly that with `__STDC_VERSION__` survived, which is how this
    // assertion came to exist. So `NEVER_COMPARABLE` gets the structural property that is
    // the *actual* reason none of its entries fires: VPP does not test any of these names
    // in an `#if`/`#elif`. That is checkable right here, from the same `tested` map the
    // definedness half is built on.
    //
    // If VPP ever does `#if __DATE__`, the name stops qualifying and has to move up to
    // `DELIBERATE`, where it must justify itself as a live divergence. That is the right
    // outcome rather than a nuisance: a name VPP actually branches on is one whose value
    // decides which code gets compiled.
    let misfiled: Vec<&str> = NEVER_COMPARABLE
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| tested.contains_key(*n))
        .collect();
    assert!(
        misfiled.is_empty(),
        "{} NEVER_COMPARABLE entr(ies) name a macro VPP *does* test in an `#if`, so \"cannot \
         agree by construction\" is not why they are exempt. Move them to DELIBERATE, where \
         an excuse has to correspond to a divergence the gate really sees: {misfiled:?}",
        misfiled.len()
    );
}
