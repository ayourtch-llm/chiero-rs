//! `compile-flags` — the harness's replacement for hand-keeping `-I`/`-D`.

use xtask::compile_flags::frontend_flags;

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

/// **Both spellings of every flag, because the format permits either.** CMake writes
/// `-I/path`; a hand-written rule may write `-I /path`, and dropping the second would silently
/// analyse a file under fewer include paths than the build used — the exact defect this command
/// exists to remove (§7.30).
#[test]
fn joined_and_separated_spellings_are_both_kept() {
    let got = frontend_flags(&v(&[
        "cc", "-I", "/a", "-I/b", "-D", "X=1", "-DY", "-U", "Z", "-c", "f.c", "-o", "f.o",
    ]));
    assert_eq!(got, v(&["-I/a", "-I/b", "-DX=1", "-DY", "-UZ"]));
}

/// **Object-file plumbing is dropped, configuration is not.** `-o`/`-c`/`-MD` cannot change what
/// the preprocessor sees; `-std=`, `-march=` and `-m…` can, and VPP passes `-march=x86-64-v2` on
/// every unit while the harness passed none.
#[test]
fn configuration_survives_and_plumbing_does_not() {
    let got = frontend_flags(&v(&[
        "cc",
        "-MD",
        "-MT",
        "obj",
        "-o",
        "x.o",
        "-c",
        "-std=gnu11",
        "-march=x86-64-v2",
        "-mtune=generic",
        "-Wall",
        "-O2",
        "-fPIC",
        "-I/inc",
    ]));
    assert_eq!(
        got,
        v(&["-std=gnu11", "-march=x86-64-v2", "-mtune=generic", "-I/inc"]),
        "-Wall/-O2/-fPIC change codegen and diagnostics, not what the preprocessor sees"
    );
}

/// A file the build never compiles has **no** flags, and that must not read as "no flags needed".
#[test]
fn a_file_outside_the_database_is_an_error_not_an_empty_answer() {
    let db = r#"[{"directory":"/b","file":"/s/a.c","output":"a.o","command":"cc -I/x -c /s/a.c"}]"#;
    let dir = std::env::temp_dir().join(format!("xtask-cf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.json");
    std::fs::write(&path, db).unwrap();

    let ok = xtask::compile_flags::compile_flags(&path, std::path::Path::new("s/a.c"));
    assert_eq!(ok.unwrap(), vec!["-I/x".to_string()]);

    let missing = xtask::compile_flags::compile_flags(&path, std::path::Path::new("s/nope.c"));
    assert!(
        missing.is_err(),
        "a file nothing builds must not answer with empty flags"
    );
}

/// **Every unit, not the first.** VPP compiles 208 sources more than once — multiarch — and each
/// variant is a different configuration. Answering with one of them silently picks a target.
#[test]
fn a_multiarch_source_reports_every_variant() {
    let db = r#"[
      {"directory":"/b","file":"/s/m.c","output":"v2.o","command":"cc -march=x86-64-v2 -c /s/m.c"},
      {"directory":"/b","file":"/s/m.c","output":"v3.o","command":"cc -DCLIB_MARCH_VARIANT=x86_64_v3 -march=x86-64-v3 -c /s/m.c"}
    ]"#;
    let dir = std::env::temp_dir().join(format!("xtask-cf-multi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.json");
    std::fs::write(&path, db).unwrap();
    let got = xtask::compile_flags::compile_flags(&path, std::path::Path::new("s/m.c")).unwrap();
    assert_eq!(got.len(), 2, "both variants: {got:?}");
    assert!(got[1].contains("CLIB_MARCH_VARIANT"), "{got:?}");
}
