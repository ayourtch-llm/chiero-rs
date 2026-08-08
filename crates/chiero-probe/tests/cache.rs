//! Covers: the persona probe's cache — one compiler run per *distinct* flag-set, and a distinct
//! answer for each.
//!
//! **The count is the contract, not the speed.** A sweep over VPP asks for a persona 1967 times
//! and there are five distinct `-march` values in that build; "it is cached" is a claim about how
//! many times a subprocess ran, and only a counter can state it. A timing would pass on a fast
//! machine with no cache at all.

use chiero_probe::Probe;

/// Does this machine's `cc` predefine `__AVX2__` under `-march=x86-64-v3` and not without it?
///
/// Checked rather than assumed: with no compiler, or on a machine that is not x86, the two probes
/// are legitimately identical and the assertions below would be about the machine.
fn avx2_discriminates() -> bool {
    let dump = |args: &[&str]| -> String {
        let mut a: Vec<&str> = vec!["-dM", "-E"];
        a.extend_from_slice(args);
        a.extend_from_slice(&["-x", "c", "/dev/null"]);
        std::process::Command::new("cc")
            .args(&a)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    !dump(&[]).contains("__AVX2__") && dump(&["-march=x86-64-v3"]).contains("__AVX2__")
}

fn flags(f: &[&str]) -> Vec<String> {
    f.iter().map(|s| s.to_string()).collect()
}

/// **Six asks over three flag-sets run the compiler three times.**
///
/// This is what makes the join affordable: 1967 VPP translation units carry 1963 target flag-sets
/// between them, but only five distinct ones, so the sweep pays five `cc -dM -E` invocations
/// rather than one per unit.
#[test]
fn the_compiler_runs_once_per_distinct_flag_set() {
    let p = Probe::new();
    for _ in 0..2 {
        p.persona(&[]);
        p.persona(&flags(&["-march=x86-64-v2"]));
        p.persona(&flags(&["-march=x86-64-v3"]));
    }
    assert_eq!(
        p.persona_probes(),
        3,
        "three distinct flag-sets asked twice each: the second round must be answered from the cache"
    );
}

/// **And each flag-set gets its own answer** — a cache that returns one persona for every key is
/// cheaper still and is exactly the defect this crate was extracted to fix.
#[test]
fn each_flag_set_gets_its_own_persona() {
    if !avx2_discriminates() {
        eprintln!("SKIPPED: this machine's cc does not discriminate -march=x86-64-v3 by __AVX2__");
        return;
    }
    let p = Probe::new();
    // Deliberately in this order: under the `OnceLock` this replaces, the *first* call's answer
    // was handed to every later one, so asking the plain one first is what exposes it.
    let plain = p.persona(&[]);
    let v3 = p.persona(&flags(&["-march=x86-64-v3"]));
    assert_eq!(plain.get("__AVX2__"), None);
    assert_eq!(v3.get("__AVX2__"), Some("1"));
    assert!(
        v3.name().contains("-march=x86-64-v3"),
        "a persona names the flags it was probed under, or a report cannot say which it used: {}",
        v3.name()
    );
}

/// **No compiler is a fact about the machine, not about the code.**
///
/// The fallback is [`chiero_pp::Persona::baked`] rather than an empty persona: predefining nothing
/// sends every real header down its `#else`, which is a configuration nobody compiles. And the
/// failure is cached too — a sweep on a machine without a compiler must not spawn 1967 processes
/// to be told the same thing.
#[test]
fn a_missing_compiler_falls_back_to_the_baked_persona_once() {
    let p = Probe::with_compiler("chiero-no-such-compiler");
    assert_eq!(p.persona(&[]), chiero_pp::Persona::baked());
    assert_eq!(p.persona(&[]), chiero_pp::Persona::baked());
    assert_eq!(
        p.persona_probes(),
        1,
        "the failure is cached like any answer"
    );
    assert!(
        p.include_paths().is_empty(),
        "a machine with no compiler has no system include path to report"
    );
}

/// **A probe that fails is not a probe that answered "nothing".**
///
/// `cc -march=bogus` exits non-zero and prints no `#define` at all, and `Persona::from_defines`
/// over that text is a perfectly well-formed persona with **zero** predefines — which is the worst
/// answer available: `__GNUC__`, `__linux__` and `__x86_64__` all undefined sends every real header
/// down its `#else` and configures a program nobody compiles. The same shape as a spawn failure,
/// arriving through a different door, and the same fallback is right.
///
/// It must also be *visible*: a run that quietly substituted the baked set for the flags it was
/// asked about would report a persona count that says nothing went wrong.
#[test]
fn a_probe_that_produces_no_defines_falls_back_rather_than_answering_nothing() {
    let p = Probe::new();
    let bogus = flags(&["-march=chiero-no-such-arch"]);
    assert_eq!(
        p.persona(&bogus),
        chiero_pp::Persona::baked(),
        "an unusable probe falls back to the set chiero has always impersonated"
    );
    assert_eq!(
        p.failed_probes(),
        vec![bogus],
        "and says which flag-set it could not probe, or the substitution is silent"
    );
    p.persona(&[]);
    assert_eq!(
        p.failed_probes().len(),
        1,
        "a probe that answered is not a failure"
    );
}
