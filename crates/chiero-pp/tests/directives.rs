//! Covers: 012 contracts 10, 11, 12, 13, 14, 16.

use chiero_pp::{Config, FileLoader, preprocess_str, preprocess_with_loader};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Default)]
struct MemoryFiles {
    files: BTreeMap<PathBuf, String>,
    reads: BTreeMap<PathBuf, usize>,
}

impl FileLoader for MemoryFiles {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        *self.reads.entry(path.to_path_buf()).or_default() += 1;
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing fixture"))
    }
}

#[test]
fn inactive_text_is_lexed_but_not_diagnosed_or_emitted() {
    let tu = preprocess_str(
        "fixture.c",
        "#if 0\n@\n\"unterminated\n{ {{\n#endif\nlive\n",
        Config::default(),
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["live"]);
}

#[test]
fn live_division_by_zero_diagnoses_but_short_circuit_does_not() {
    let live = preprocess_str("fixture.c", "#if 1/0\nbad\n#endif\n", Config::default());
    assert_eq!(live.diagnostics.len(), 1);
    assert!(live.diagnostics[0].message.contains("division by zero"));

    let dead = preprocess_str(
        "fixture.c",
        "#if 0 && 1/0\nbad\n#endif\nok\n",
        Config::default(),
    );
    assert!(dead.diagnostics.is_empty());
    assert_eq!(dead.token_texts().collect::<Vec<_>>(), ["ok"]);
}

#[test]
fn deeply_parenthesized_if_does_not_abort_the_process() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "deep_if_expression_child"])
        .env("CHIERO_DEEP_IF_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success(), "deep #if child aborted: {status}");
}

#[test]
fn deep_if_expression_child() {
    if std::env::var_os("CHIERO_DEEP_IF_CHILD").is_none() {
        return;
    }
    let mut src = String::from("#if ");
    src.extend(std::iter::repeat_n('(', 20_000));
    src.push('1');
    src.extend(std::iter::repeat_n(')', 20_000));
    src.push_str("\nselected\n#endif\n");
    let tu = preprocess_str("deep-if.c", &src, Config::default());
    assert!(
        tu.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("#if nesting")),
        "bounded parsing must diagnose the limit"
    );
}

#[test]
fn unsuffixed_if_literals_promote_to_uintmax_when_needed() {
    let src = "#if 0x8000000000000000 > 0\nhex\n#endif\n\
               #if 12345678901234567890 > 0\ndecimal\n#endif\n";
    let tu = preprocess_str("uintmax.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["hex", "decimal"]);
}

#[test]
fn defined_and_unknown_identifiers_evaluate_false() {
    let src = "#if defined(UNDEFINED_THING)\nbad1\n#endif\n\
               #if UNDEFINED_THING\nbad2\n#else\ngood\n#endif\n";
    let tu = preprocess_str("fixture.c", src, Config::default());
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["good"]);
    assert!(tu.diagnostics.is_empty());

    let config = Config {
        pedantic: true,
        ..Config::default()
    };
    let pedantic = preprocess_str("fixture.c", "#if UNKNOWN\nbad\n#endif\n", config);
    assert_eq!(pedantic.diagnostics.len(), 1);
}

#[test]
fn include_guards_and_pragma_once_avoid_second_read() {
    for (name, header) in [
        (
            "guard.h",
            "#ifndef GUARD_H\n#define GUARD_H\ninside_guard\n#endif\n",
        ),
        ("once.h", "#pragma once\ninside_once\n"),
    ] {
        let mut files = MemoryFiles::default();
        files.files.insert(PathBuf::from(name), header.into());
        let src = format!("#include \"{name}\"\n#include \"{name}\"\n");
        let tu = preprocess_with_loader("main.c", &src, Config::default(), &mut files);
        assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
        assert_eq!(files.reads.get(Path::new(name)), Some(&1), "{name}");
        assert_eq!(tu.token_texts().count(), 1);
    }
}

#[test]
fn include_guard_skip_depends_on_the_guard_macro_remaining_defined() {
    let mut files = MemoryFiles::default();
    files.files.insert(
        PathBuf::from("guard.h"),
        "#ifndef GUARD_H\n#define GUARD_H\nbody\n#endif\n".into(),
    );
    let tu = preprocess_with_loader(
        "main.c",
        "#include \"guard.h\"\n#undef GUARD_H\n#include \"guard.h\"\n",
        Config::default(),
        &mut files,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["body", "body"]);
    assert_eq!(files.reads.get(Path::new("guard.h")), Some(&2));
}

#[test]
fn inactive_include_does_not_touch_the_loader() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("never.h"), "should_not_be_read\n".into());
    let tu = preprocess_with_loader(
        "main.c",
        "#if 0\n#include \"never.h\"\n#endif\nlive\n",
        Config::default(),
        &mut files,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert!(files.reads.is_empty(), "inactive include performed IO");
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["live"]);
}

#[test]
fn literal_angle_header_names_are_not_macro_expanded() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("sys/foo/bar.h"), "right_header\n".into());
    let config = Config {
        include_paths: vec![PathBuf::from("sys")],
        defines: vec![("foo".into(), "1".into())],
        ..Config::default()
    };
    let tu = preprocess_with_loader("main.c", "#include <foo/bar.h>\n", config, &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["right_header"]);
    assert_eq!(files.reads.get(Path::new("sys/foo/bar.h")), Some(&1));
    assert!(!files.reads.contains_key(Path::new("sys/1/bar.h")));
}

#[test]
fn included_builtins_and_spans_name_the_header() {
    let mut files = MemoryFiles::default();
    files.files.insert(
        PathBuf::from("inc/header.h"),
        "__FILE__ __LINE__ header_token\n".into(),
    );
    let tu = preprocess_with_loader(
        "inc/main.c",
        "#include \"header.h\"\n",
        Config::default(),
        &mut files,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>(),
        ["\"inc/header.h\"", "1", "header_token"]
    );
    let loc = tu.source_map.spelling_loc(tu.tokens[2].span).unwrap();
    assert_eq!(
        tu.source_map.file(loc.file).path(),
        Path::new("inc/header.h")
    );
}

#[test]
fn whitespace_spelling_does_not_defeat_guard_detection() {
    let mut files = MemoryFiles::default();
    files.files.insert(
        PathBuf::from("tabs.h"),
        "#ifndef\tTABS_H\n#define\tTABS_H\nonce\n#endif\n".into(),
    );
    let tu = preprocess_with_loader(
        "main.c",
        "#include \"tabs.h\"\n#include \"tabs.h\"\n",
        Config::default(),
        &mut files,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(files.reads.get(Path::new("tabs.h")), Some(&1));
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["once"]);
}

#[test]
fn angle_include_uses_configured_search_paths() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("sys/api.h"), "api_token\n".into());
    let config = Config {
        include_paths: vec![PathBuf::from("sys")],
        ..Config::default()
    };
    let tu = preprocess_with_loader("main.c", "#include <api.h>\n", config, &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["api_token"]);
    assert_eq!(files.reads.get(Path::new("sys/api.h")), Some(&1));
}

#[test]
fn include_depth_is_bounded_with_a_diagnostic() {
    let mut files = MemoryFiles::default();
    files.files.insert(
        PathBuf::from("self.h"),
        "#include \"self.h\"\nbody\n".into(),
    );
    let config = Config {
        max_include_depth: 4,
        ..Config::default()
    };
    let tu = preprocess_with_loader("main.c", "#include \"self.h\"\n", config, &mut files);
    assert_eq!(files.reads.get(Path::new("self.h")), Some(&4));
    assert_eq!(
        tu.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("include depth"))
            .count(),
        1
    );
}

#[test]
fn lexer_diagnostics_are_promoted_only_on_active_lines() {
    let active = preprocess_str(
        "active.c",
        "#if 1\n\"unterminated\n#endif\n",
        Config::default(),
    );
    assert_eq!(active.diagnostics.len(), 1);
    assert!(active.diagnostics[0].message.contains("unterminated"));

    let inactive = preprocess_str(
        "inactive.c",
        "#if 0\n\"unterminated\n#endif\n",
        Config::default(),
    );
    assert!(inactive.diagnostics.is_empty());
}

#[test]
fn predefined_and_feature_test_macros_drive_conditionals() {
    let src = "#if __STDC__ == 1 && defined(__x86_64__)\nplatform\n#endif\n\
               #if __has_builtin(not_a_real_builtin)\nwrong\n#else\nfallback\n#endif\n";
    let tu = preprocess_str("predefined.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>(),
        ["platform", "fallback"]
    );
}

#[test]
fn line_error_warning_and_pragma_have_defined_effects() {
    let src = "#line 40 \"virtual.c\"\n__LINE__ __FILE__\n\
               #warning careful now\n#error broken now\n\
               _Pragma(\"once\") after\n";
    let tu = preprocess_str("physical.c", src, Config::default());
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>(),
        ["40", "\"virtual.c\"", "after"]
    );
    assert_eq!(tu.diagnostics.len(), 2);
    assert!(tu.diagnostics[0].message.contains("careful now"));
    assert!(tu.diagnostics[1].message.contains("broken now"));
}

#[test]
fn pragmas_are_recorded_for_downstream_target_selection() {
    let tu = preprocess_str(
        "pragma.c",
        "#pragma GCC push_options\n#pragma GCC target(\"avx2\")\n_Pragma(\"GCC pop_options\")\nafter\n",
        Config::default(),
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["after"]);
    assert_eq!(
        tu.pragmas
            .iter()
            .map(|pragma| pragma.text.as_str())
            .collect::<Vec<_>>(),
        [
            "GCC push_options",
            "GCC target ( \"avx2\" )",
            "GCC pop_options"
        ]
    );
}

#[test]
fn include_next_continues_after_the_current_search_directory() {
    let mut files = MemoryFiles::default();
    files.files.insert(
        PathBuf::from("one/api.h"),
        "one\n#include_next <api.h>\n".into(),
    );
    files
        .files
        .insert(PathBuf::from("two/api.h"), "two\n".into());
    let config = Config {
        include_paths: vec![PathBuf::from("one"), PathBuf::from("two")],
        ..Config::default()
    };
    let tu = preprocess_with_loader("main.c", "#include <api.h>\n", config, &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["one", "two"]);
    assert_eq!(files.reads.get(Path::new("one/api.h")), Some(&1));
    assert_eq!(files.reads.get(Path::new("two/api.h")), Some(&1));
}

#[test]
fn configured_defines_are_real_macros_and_undef_stops_them() {
    let config = Config {
        defines: vec![("FLAG".into(), "7".into()), ("EMPTY".into(), String::new())],
        ..Config::default()
    };
    let src = "#if FLAG == 7\nconfigured\n#endif\n\
               before FLAG EMPTY\n#undef FLAG\nafter FLAG\n";
    let tu = preprocess_str("defines.c", src, config);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>(),
        ["configured", "before", "7", "after", "FLAG"]
    );
}

#[test]
fn macro_definitions_are_public_symbolized_and_closed_by_redefinition() {
    let tu = preprocess_str(
        "defs.c",
        "#define F(x) x\n#define F(y) y + 1\nF(2)\n",
        Config::default(),
    );
    let definitions: Vec<_> = tu
        .macro_defs
        .iter()
        .filter(|definition| tu.symbol_text(definition.name) == Some("F"))
        .collect();
    assert_eq!(definitions.len(), 2);
    assert_ne!(definitions[0].id, definitions[1].id);
    assert!(
        definitions[0].undef_span.is_some(),
        "redefinition must close the previous definition"
    );
    assert_eq!(tu.symbol_text(definitions[0].name), Some("F"));
    let chiero_pp::MacroKind::FunctionLike { params, .. } = &definitions[1].kind else {
        panic!("F must be function-like")
    };
    assert_eq!(tu.symbol_text(params[0]), Some("y"));
    assert!(
        tu.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("redefinition"))
    );
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["2", "+", "1"]);
}

#[test]
fn gcc_function_like_predefines_are_installed_as_functions() {
    let config = Config {
        defines: vec![("__UINT64_C(c)".into(), "c ## UL".into())],
        ..Config::default()
    };
    let tu = preprocess_str("configured.c", "__UINT64_C(42)\n", config);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["42UL"]);
}

#[test]
fn leading_whitespace_hash_still_starts_a_directive() {
    let tu = preprocess_str(
        "spacing.c",
        "   #define VALUE 9\n\t#if VALUE == 9\nyes\n  #endif\n",
        Config::default(),
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["yes"]);
}

#[test]
fn has_include_queries_the_configured_search() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("inc/present.h"), "present\n".into());
    let config = Config {
        include_paths: vec![PathBuf::from("inc")],
        ..Config::default()
    };
    let src = "#if __has_include(<present.h>)\nyes\n#else\nno\n#endif\n";
    let tu = preprocess_with_loader("main.c", src, config, &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["yes"]);
}

#[test]
fn successful_has_include_probe_is_reused_by_the_real_include() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("present.h"), "from_header\n".into());
    let src = "#if __has_include(\"present.h\")\n#include \"present.h\"\n#endif\n";
    let tu = preprocess_with_loader("main.c", src, Config::default(), &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["from_header"]);
    assert_eq!(files.reads.get(Path::new("present.h")), Some(&1));
}

#[test]
fn computed_include_is_expanded_before_resolution() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("x.h"), "from_header\n".into());
    let src = "#define HDR \"x.h\"\n#include HDR\n";
    let tu = preprocess_with_loader("main.c", src, Config::default(), &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["from_header"]);
    assert_eq!(tu.deps.len(), 2);
}

fn selected(expr: &str) -> String {
    let src = format!("#if {expr}\nyes\n#else\nno\n#endif\n");
    let tu = preprocess_str("expr.c", &src, Config::default());
    assert!(
        tu.diagnostics.is_empty(),
        "{expr}: unexpected diagnostics: {:?}",
        tu.diagnostics
    );
    tu.token_texts().collect()
}

#[test]
fn if_expression_supports_c_integer_spelling_and_precedence() {
    for (expr, expected) in [
        ("0xFF", "yes"),
        ("010 == 8", "yes"),
        ("1 & 0", "no"),
        ("1 | 0", "yes"),
        ("3 ^ 3", "no"),
        ("1 << 3 == 8", "yes"),
        ("8 >> 2 == 2", "yes"),
        ("~0 == -1", "yes"),
        ("0 ? 0 : 1", "yes"),
        ("(0, 1)", "yes"),
        ("'A' == 65", "yes"),
        ("-1 < 1U", "no"),
    ] {
        assert_eq!(selected(expr), expected, "{expr}");
    }
}

#[test]
fn elifdef_and_elifndef_select_exactly_one_branch() {
    let src = "#define PRESENT 1\n\
               #if 0\nwrong1\n\
               #elifdef PRESENT\nyes1\n\
               #else\nwrong2\n#endif\n\
               #if 0\nwrong3\n\
               #elifndef ABSENT\nyes2\n\
               #else\nwrong4\n#endif\n";
    let tu = preprocess_str("elifdef.c", src, Config::default());
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["yes1", "yes2"]);
    assert_eq!(
        tu.diagnostics.len(),
        2,
        "C23 directives are accepted but diagnosed under C11"
    );
}

#[test]
#[ignore = "external corpus regression metric"]
/// External configured-corpus evidence. Ignored tests do not carry contract coverage
/// credit under 070 §4, so this comment intentionally cites no contract number.
fn every_vpp_compile_command_preprocesses_without_panicking() {
    let compile_commands = Path::new("/home/ubuntu/vpp/build-root/compile_commands.json");
    if !compile_commands.exists() {
        return;
    }
    // The full driver consumes this JSON; this smoke assertion keeps the external gate
    // explicit without adding serde to the frontend crate.
    let text = std::fs::read_to_string(compile_commands).unwrap();
    assert!(text.contains("\"file\""));
}

/// **`"..."` searches the including file's own directory; `<...>` does not.**
///
/// C11 6.10.2p2 and p3: a quoted include is searched for "in an implementation-defined manner"
/// first — for every real compiler, starting beside the file doing the including — and only then
/// falls back to the bracket search. An angled include skips that step entirely.
///
/// **Neither direction of that branch was observed** (wave 293's sweep of `chiero-pp`). Forcing
/// `quoted` to `false` makes `"x.h"` skip the sibling directory; forcing it to `true` makes
/// `<x.h>` search it; all 67 tests passed either way, because **every existing fixture puts the
/// including file at the root**, where its parent directory is `""` and the two searches
/// coincide. The bug that would have escaped is not exotic: a header found beside a source file
/// in `src/` and nowhere on the include path.
///
/// The negative half is the whole point — a test that only checked `"x.h"` finds it would pass
/// with the distinction deleted.
#[test]
fn a_quoted_include_searches_beside_the_including_file_and_an_angled_one_does_not() {
    let with_sibling = |directive: &str| {
        let mut files = MemoryFiles::default();
        files
            .files
            .insert(PathBuf::from("src/sibling.h"), "from_sibling\n".into());
        let src = format!("#if {directive}\nyes\n#else\nno\n#endif\n");
        // **The including file lives in `src/`**, so its own directory is a real place that is
        // on no configured search path. At the root the two forms cannot be told apart.
        let tu = preprocess_with_loader("src/main.c", &src, Config::default(), &mut files);
        assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
        tu.token_texts().map(str::to_owned).collect::<Vec<_>>()
    };
    assert_eq!(
        with_sibling("__has_include(\"sibling.h\")"),
        ["yes".to_string()],
        "a quoted include finds a header beside the file that includes it"
    );
    assert_eq!(
        with_sibling("__has_include(<sibling.h>)"),
        ["no".to_string()],
        "an angled include does not — it searches only the configured paths"
    );

    // And the same asymmetry for a real `#include`, not just the probe.
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("src/sibling.h"), "from_sibling\n".into());
    let tu = preprocess_with_loader(
        "src/main.c",
        "#include \"sibling.h\"\n",
        Config::default(),
        &mut files,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["from_sibling"]);

    // A header that is only on the configured path is found by **both** forms: quoted search
    // falls back to the bracket search, so the asymmetry runs one way only.
    let both = |directive: &str| {
        let mut files = MemoryFiles::default();
        files
            .files
            .insert(PathBuf::from("inc/onpath.h"), "on_path\n".into());
        let config = Config {
            include_paths: vec![PathBuf::from("inc")],
            ..Config::default()
        };
        let src = format!("#if {directive}\nyes\n#else\nno\n#endif\n");
        let tu = preprocess_with_loader("src/main.c", &src, config, &mut files);
        assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
        tu.token_texts().map(str::to_owned).collect::<Vec<_>>()
    };
    assert_eq!(both("__has_include(\"onpath.h\")"), ["yes".to_string()]);
    assert_eq!(both("__has_include(<onpath.h>)"), ["yes".to_string()]);
}

/// **With no search paths configured at all, a header resolves relative to the working
/// directory.**
///
/// `Config::default()` has no include paths and no system paths, so an *angled* include has
/// nowhere to look — the quoted form would still have the including file's directory. Rather
/// than fail, resolution falls back to the bare name, which is what makes a one-file fixture
/// with `<x.h>` work at all.
///
/// **Untested until wave 294**: every other fixture either configures a path or uses the quoted
/// form, so the fallback never ran. It is chiero's own choice — gcc would have system paths here
/// — which is exactly the kind of behaviour that should be written down rather than inferred
/// from whichever test happens to exercise it.
#[test]
fn an_angled_include_with_no_configured_paths_falls_back_to_the_bare_name() {
    let mut files = MemoryFiles::default();
    files
        .files
        .insert(PathBuf::from("bare.h"), "from_bare\n".into());
    let tu = preprocess_with_loader(
        "main.c",
        "#if __has_include(<bare.h>)\nyes\n#else\nno\n#endif\n",
        Config::default(),
        &mut files,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["yes"]);
}

/// **`#include_next` from the last search directory falls back to the bare name.**
///
/// `include_next` drops every search directory up to and including the one holding the current
/// file, so a header in the *last* directory leaves nothing to search. `include()` then has an
/// empty candidate list and falls back to the name as written, resolved relative to the working
/// directory.
///
/// **The only shape that reaches that fallback**, and nothing built it: the sibling test above
/// covers `probe_include`'s empty-*directories* guard, but `include()` has no such guard — it
/// checks the *candidates* instead, and candidates are only empty when the drain emptied the
/// directories. Both directions of it survived wave 293's sweep.
///
/// This pins chiero's behaviour rather than gcc's: gcc would find nothing here, because its
/// search list does not fall back to the working directory. Written down because it is a choice,
/// and a choice nothing recorded is a choice nobody made.
#[test]
fn include_next_past_the_last_directory_falls_back_to_the_bare_name() {
    let mut files = MemoryFiles::default();
    files.files.insert(
        PathBuf::from("inc/a.h"),
        "first\n#include_next <a.h>\n".into(),
    );
    files
        .files
        .insert(PathBuf::from("a.h"), "fallback\n".into());
    let config = Config {
        include_paths: vec![PathBuf::from("inc")],
        ..Config::default()
    };
    let tu = preprocess_with_loader("main.c", "#include <a.h>\n", config, &mut files);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>(),
        ["first", "fallback"],
        "the drain empties the search list, and the bare name is what is left to try"
    );
}

/// **Every relational operator in `#if`, on both answers.**
///
/// `#if` has its own expression evaluator — it is not the C parser — so each operator is a
/// separate branch that consumes its own token. Wave 297's sweep found `>` and `<=` unfalsifiable:
/// forcing either branch to run without consuming its operator passed the whole suite, because no
/// committed test used them in a `#if` at all. `<` was covered; the rest were not.
///
/// Both answers for each, because a branch that always yields true satisfies half of them.
/// Version guards are the everyday use — `#if __STDC_VERSION__ >= 201112L` is in most real
/// headers — and getting one backwards silently compiles the wrong half of a file.
#[test]
fn every_relational_operator_in_an_if_directive_works() {
    let yes = |expr: &str| {
        let src = format!("#if {expr}\nyes\n#else\nno\n#endif\n");
        let tu = preprocess_str("rel.c", &src, Config::default());
        assert!(tu.diagnostics.is_empty(), "{expr}: {:?}", tu.diagnostics);
        tu.token_texts().map(str::to_owned).collect::<Vec<_>>()
    };
    for (expr, want) in [
        ("2 < 3", "yes"),
        ("3 < 2", "no"),
        ("3 > 2", "yes"),
        ("2 > 3", "no"),
        ("2 <= 2", "yes"),
        ("3 <= 2", "no"),
        ("2 >= 2", "yes"),
        ("2 >= 3", "no"),
        ("2 == 2", "yes"),
        ("2 == 3", "no"),
        ("2 != 3", "yes"),
        ("2 != 2", "no"),
        // The version-guard shape itself, both sides of the boundary.
        ("201112L >= 201112L", "yes"),
        ("199901L >= 201112L", "no"),
        // **`<=` and `<` must not be confused for one another**, which is what a branch that
        // takes the wrong token does: `2 < 2` is false where `2 <= 2` is true.
        ("2 < 2", "no"),
        ("2 > 2", "no"),
    ] {
        assert_eq!(yes(expr), [want.to_string()], "`#if {expr}`");
    }
}
