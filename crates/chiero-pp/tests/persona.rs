//! Covers: 012 §4.1 — the compiler persona as a *named*, *replaceable* value.
//!
//! **The problem this solves, measured rather than argued.** chiero's predefines were an array
//! literal in `Engine::new` — an impersonation of gcc 13.3 on x86-64 Linux that never said so and
//! that no caller could change. Every gap in it was found the same way, by a corpus falling into a
//! `#else`, and every fix was another line in the array. On 2026-08-08 that cost five defects in
//! one session, including `__BYTE_ORDER__` undefined (so `#if __BYTE_ORDER__ ==
//! __ORDER_BIG_ENDIAN__` read `0 == 0` and reversed bit-field order across a whole VPP plugin),
//! and 61 million tokens of VPP — 8% of the program — that were never compiled at all.
//!
//! **And there were two mechanisms for the one fact.** `chiero-cli`'s `frontend::predefines` runs
//! `cc -dM -E` and captures all 401; the library baked 23. A fix to one proved nothing about the
//! other, which is exactly how I misattributed that 8% to the published VPP sweeps.
//!
//! # The format is `cc -dM -E` output, deliberately
//!
//! gcc already prints a persona:
//!
//! ```text
//! $ gcc -dM -E -x c /dev/null > personas/gcc-13-x86_64-linux.h
//! ```
//!
//! So there is **no new parser, no new dependency, and no new syntax to learn or get wrong** —
//! and a persona can be captured from any real compiler, checked in, diffed, and edited by hand.

use chiero_pp::{Config, Persona, preprocess_str};

fn taken(cfg: Config, cond: &str) -> Vec<String> {
    let src = format!("#if {cond}\nY\n#else\nN\n#endif\n");
    preprocess_str("p.c", &src, cfg)
        .token_texts()
        .map(str::to_owned)
        .collect()
}

/// **The baked set has a name now.** It was an anonymous array; the impersonation was real but
/// undeclared, so nothing could report it and nothing could replace it.
#[test]
fn the_baked_persona_is_named_and_describes_itself() {
    let p = Persona::baked();
    assert_eq!(p.name(), "gcc-13-x86_64-linux");
    assert!(p.get("__GNUC__").is_some());
    assert_eq!(p.get("__linux__"), Some("1"));
    assert_eq!(p.get("__ORDER_BIG_ENDIAN__"), Some("4321"));
    assert!(
        p.len() > 20,
        "the baked set is the whole impersonation, not a sample: {}",
        p.len()
    );
}

/// **A persona parses from `cc -dM -E` output**, which is the format precisely so that capturing
/// one is a shell redirect rather than a translation step.
#[test]
fn a_persona_parses_from_compiler_dm_output() {
    let text = "#define __GNUC__ 9\n\
                #define __linux__ 1\n\
                #define __SIZEOF_INT__ 4\n\
                // not a define\n\
                \n\
                #define EMPTY\n";
    let p = Persona::from_defines("gcc-9", text);
    assert_eq!(p.name(), "gcc-9");
    assert_eq!(p.get("__GNUC__"), Some("9"));
    assert_eq!(p.get("__SIZEOF_INT__"), Some("4"));
    // A `#define` with no replacement list is empty, not absent — `#ifdef` sees it, `#if` reads 0.
    assert_eq!(p.get("EMPTY"), Some(""));
    assert_eq!(
        p.get("__STDC__"),
        None,
        "nothing is inherited that was not in the text"
    );
}

/// **Function-like macros are refused, not silently mangled.** `cc -dM` emits them
/// (`#define __has_builtin(x) ...`), and the preprocessor owns those itself — a persona that
/// swallowed them would shadow the engine's own `__has_include` with a broken object macro.
#[test]
fn function_like_macros_are_not_taken_from_a_dump() {
    let p = Persona::from_defines(
        "x",
        "#define __has_builtin(x) 1\n#define PLAIN 7\n#define __FILE__ \"a.c\"\n",
    );
    assert_eq!(p.get("PLAIN"), Some("7"));
    assert_eq!(
        p.get("__has_builtin"),
        None,
        "function-like macros are the engine's"
    );
    assert_eq!(
        p.get("__FILE__"),
        None,
        "and so are the builtins whose value depends on where they appear"
    );
}

/// **The whole point: a caller can replace the persona and the preprocessor believes it.**
///
/// The negative half is what makes it a replacement rather than an addition — a persona that
/// merely *added* to the baked set would still answer `__linux__` here.
#[test]
fn a_config_carrying_a_persona_uses_it_instead_of_the_baked_one() {
    let cfg = Config {
        persona: Persona::from_defines("tiny", "#define __GNUC__ 4\n#define TARGET_IS_TOASTER 1\n"),
        ..Config::default()
    };
    assert_eq!(taken(cfg.clone(), "defined(TARGET_IS_TOASTER)"), ["Y"]);
    assert_eq!(taken(cfg.clone(), "__GNUC__ == 4"), ["Y"]);
    assert_eq!(
        taken(cfg, "defined(__linux__)"),
        ["N"],
        "a persona replaces the baked set; it does not layer on top of it"
    );
}

/// The default `Config` still carries the baked persona, so every existing caller sees exactly
/// what it saw before. This is the assertion that makes the refactor a refactor.
#[test]
fn the_default_config_is_unchanged_by_the_persona_seam() {
    for cond in [
        "defined(__linux__)",
        "defined(__x86_64__)",
        "__GNUC__ == 13",
        "__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__",
        "defined(__SSE2__)",
    ] {
        assert_eq!(taken(Config::default(), cond), ["Y"], "{cond}");
    }
    assert_eq!(taken(Config::default(), "defined(__FreeBSD__)"), ["N"]);
}

/// `Config::defines` still wins over the persona — it is the command line, and a `-D` on the
/// command line beats what the compiler predefines. Real builds depend on this.
#[test]
fn command_line_defines_still_override_the_persona() {
    let cfg = Config {
        defines: vec![("__GNUC__".into(), "99".into())],
        ..Config::default()
    };
    assert_eq!(taken(cfg, "__GNUC__ == 99"), ["Y"]);
}
