//! `#pragma GCC error` / `GCC warning` — a pragma that *is* a diagnostic.
//!
//! Both gcc and clang implement them, and both work through `_Pragma` as well as through the
//! directive, which is what `diagnostic-pragma-1.c` exists to pin: a macro can carry
//! `_Pragma("GCC error \"…\"")` so the message fires where the macro is *used*.
//!
//! chiero recorded the pragma and said nothing — accepting a program both compilers reject. That
//! is the `AcceptedWhatBothRejected` class the pp-gate reports separately precisely because a
//! missing diagnostic is a finding rather than an unmeasured row.
//!
//! ⚠️ **chiero's `Diagnostic` has no severity**, so `GCC warning` and `GCC error` both produce
//! one. That is a real fidelity limit and is recorded rather than papered over: the test asserts
//! the *message* reaches the caller, not that it was graded.

use chiero_pp::{Config, preprocess_str};

fn diagnostics(src: &str) -> Vec<String> {
    preprocess_str("p.c", src, Config::default())
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn a_gcc_error_pragma_reports_its_message() {
    let d = diagnostics("#pragma GCC error \"boom\"\n");
    assert!(d.iter().any(|m| m.contains("boom")), "{d:?}");
}

#[test]
fn a_gcc_warning_pragma_reports_its_message() {
    let d = diagnostics("#pragma GCC warning \"careful\"\n");
    assert!(d.iter().any(|m| m.contains("careful")), "{d:?}");
}

/// **Through `_Pragma`, and through a macro** — the shape the corpus fixture uses, and the one
/// that matters: the message must fire at the *use* site, not at the definition.
#[test]
fn the_message_fires_through_a_pragma_operator_in_a_macro() {
    let d = diagnostics("#define C _Pragma(\"GCC error \\\"from-macro\\\"\") 1\nchar a[C];\n");
    assert!(d.iter().any(|m| m.contains("from-macro")), "{d:?}");
    // The macro must still expand — a pragma that swallows the `1` would change the program.
    let texts: Vec<_> = preprocess_str(
        "p.c",
        "#define C _Pragma(\"GCC error \\\"x\\\"\") 1\nchar a[C];\n",
        Config::default(),
    )
    .token_texts()
    .map(str::to_owned)
    .collect();
    assert_eq!(texts, vec!["char", "a", "[", "1", "]", ";"]);
}

/// **A pragma that is not a diagnostic stays silent.** Without this, "report everything" passes
/// the three tests above and breaks every real translation unit.
#[test]
fn an_ordinary_pragma_reports_nothing() {
    for src in [
        "#pragma once\n",
        "#pragma GCC diagnostic push\n",
        "#pragma pack(1)\n",
        "#pragma GCC visibility push(default)\n",
    ] {
        assert!(diagnostics(src).is_empty(), "{src}: {:?}", diagnostics(src));
    }
}
