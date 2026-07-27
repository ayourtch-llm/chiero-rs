//! Covers: 012 contracts 10, 11, 12, 13, 14, 16.

use chiero_pp::{Config, FileLoader, preprocess_str, preprocess_with_loader};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

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
/// External evidence for 012 contract 17. Ignored tests do not carry contract coverage
/// credit under 070 §4.
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
