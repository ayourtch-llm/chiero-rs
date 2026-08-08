//! Covers: 060 contract 2 — "A source compiled under 3 `CLIB_MARCH_VARIANT`s yields 3
//! `TranslationUnit`s with distinct `march`, and no index keyed on path alone collapses them."
//!
//! **The structural half of that contract is already met** in `builddb.rs`: three rows give three
//! units, `units_for` returns all of them, and each carries its own `target_flags`. This file is
//! the half that makes it mean something — *the variants must preprocess to different programs*.
//!
//! Until now they did not. `TranslationUnit::pp_config` carried the unit's `-D` and `-I` and left
//! the persona at the baked default, so every variant of every source was analysed as a compiler
//! with no `-march` at all. That is not a small difference: `__AVX2__` is defined only under
//! `-march=x86-64-v3` or `-mavx2`, VPP's baseline is `-march=x86-64-v2`, and **every 32-byte
//! vector type in vppinfra sits behind `#if defined(__AVX2__)`** — so the AVX2 and AVX-512 paths
//! of the tree, which are exactly what multiarch exists to compile, were invisible to every
//! measurement chiero has published (HANDOFF §9.1).
//!
//! One source, N units, N *different programs* is what 1:N buys. A field nobody joins on is a
//! field, not a capability.

use chiero_pp::FileLoader;
use chiero_vpp::builddb::{BuildDb, TranslationUnit};
use std::io;
use std::path::Path;

struct Disk;
impl FileLoader for Disk {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// One entry, written the way `ninja -t compdb` writes them.
fn entry(file: &str, flags: &str, object: &str) -> String {
    format!(
        r#"{{"directory": "/b", "command": "clang {flags} -o {object} -c {file}", "file": "{file}", "output": "{object}"}}"#
    )
}

/// Does this machine's `cc` predefine `__AVX2__` under `-march=x86-64-v3` and not under
/// `-march=x86-64-v2`?
///
/// Checked rather than assumed. With no compiler, or on a machine that is not x86, the two
/// personas are legitimately identical and everything below would be asserting a property of the
/// machine rather than of chiero.
fn avx2_discriminates() -> bool {
    let dump = |march: &str| -> String {
        std::process::Command::new("cc")
            .args(["-dM", "-E", march, "-x", "c", "/dev/null"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    !dump("-march=x86-64-v2").contains("__AVX2__") && dump("-march=x86-64-v3").contains("__AVX2__")
}

/// The one line of C that separates the two personas, in the shape vppinfra actually writes it
/// (`vector.h:197` guards `vector_avx2.h` exactly this way).
const SRC: &str = "#if defined(__AVX2__)\nint wide (void);\n#else\nint narrow (void);\n#endif\n";

fn preprocessed(u: &TranslationUnit) -> String {
    let tu = chiero_pp::preprocess_with_loader(&u.src, SRC, u.pp_config(), &mut Disk);
    assert!(
        tu.diagnostics.is_empty(),
        "the fixture preprocesses cleanly: {:?}",
        tu.diagnostics
    );
    tu.token_texts().collect::<Vec<_>>().join(" ")
}

/// **Two variants of one source are two programs.**
#[test]
fn each_variant_preprocesses_under_the_persona_its_own_march_selects() {
    if !avx2_discriminates() {
        eprintln!("SKIPPED: this machine's cc does not discriminate x86-64-v3 from v2 by __AVX2__");
        return;
    }
    let d = BuildDb::parse(&format!(
        "[{}]",
        [
            entry(
                "/src/aes.c",
                "-DCLIB_MARCH_VARIANT=avx2 -march=x86-64-v3",
                "avx2.o"
            ),
            entry("/src/aes.c", "-march=x86-64-v2", "base.o"),
        ]
        .join(",\n")
    ))
    .expect("fixture parses");
    let units: Vec<_> = d.units_for(Path::new("/src/aes.c")).collect();
    assert_eq!(units.len(), 2, "060 §1.1: one source, several units");

    assert!(
        preprocessed(units[0]).contains("wide"),
        "the -march=x86-64-v3 unit compiles the AVX2 branch, which is the only reason the variant exists"
    );
    assert!(
        preprocessed(units[1]).contains("narrow"),
        "and the baseline unit does not — two units of one source that agree here are one program twice"
    );
}
