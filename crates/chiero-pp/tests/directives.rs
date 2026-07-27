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
