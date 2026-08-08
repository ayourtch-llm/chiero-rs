//! Covers: 012 contract 25 — a diagnostic whose site is inside a system header is separated
//! from the ones a person can act on, and counted rather than dropped.
//!
//! **Found by 012 contract 17's corpus run, and it is not a standards question.** Five of the 25
//! diagnosed VPP translation units report `redefinition of macro MFD_CLOEXEC` and four siblings.
//! chiero is *right* — C11 6.10.3p2 makes a non-identical redefinition a constraint violation,
//! and `<sys/mman.h>` really does define `MFD_CLOEXEC` as `1U` before `<linux/memfd.h>`
//! redefines it as `0x0001U`. The point is that **nobody can act on it**: both files belong to
//! glibc and the kernel headers, and every C program on this machine has the same clash.
//!
//! gcc's rule, measured rather than recalled — the *same header text* warns from a user path and
//! is silent from a system one:
//!
//! ```text
//! $ cp /usr/include/linux/memfd.h uh/memfd_user.h
//! $ gcc -E -I/usr/include c.c        # includes "uh/memfd_user.h"
//! uh/memfd_user.h:8: warning: "MFD_CLOEXEC" redefined       ← and three more
//! $ gcc -E d.c                       # includes <linux/memfd.h>, byte-identical content
//!                                    ← nothing
//! ```
//!
//! **Separated, not deleted.** A preprocessor that silently dropped these would be claiming a
//! clean tree it never checked, which is the failure mode this project has paid for repeatedly.
//! They stay reachable in `system_diagnostics`, so "did not report" and "found nothing" remain
//! different facts.

use chiero_pp::{Config, FileLoader, preprocess_with_loader};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Files(BTreeMap<PathBuf, String>);

impl FileLoader for Files {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing fixture"))
    }
}

/// `usr/inc` is the system path; `proj` is the user's own.
fn config() -> Config {
    Config {
        include_paths: vec![PathBuf::from("proj")],
        system_paths: vec![PathBuf::from("usr/inc")],
        ..Config::default()
    }
}

fn files(pairs: &[(&str, &str)]) -> Files {
    Files(
        pairs
            .iter()
            .map(|(p, t)| (PathBuf::from(*p), (*t).to_string()))
            .collect(),
    )
}

/// **The `MFD_CLOEXEC` shape, reduced.** The clash is real and the second definition is inside a
/// system header, so it belongs in `system_diagnostics` — visible to anyone who asks, absent
/// from the list a person is expected to act on.
#[test]
fn a_redefinition_inside_a_system_header_is_separated_from_the_actionable_ones() {
    let mut f = files(&[
        ("proj/first.h", "#define MFD_CLOEXEC 1U\n"),
        ("usr/inc/second.h", "#define MFD_CLOEXEC 0x0001U\n"),
    ]);
    let tu = preprocess_with_loader(
        "main.c",
        "#include <first.h>\n#include <second.h>\nMFD_CLOEXEC\n",
        config(),
        &mut f,
    );
    assert!(
        tu.diagnostics.is_empty(),
        "nothing here is the programmer's to fix: {:?}",
        tu.diagnostics
    );
    assert_eq!(
        tu.system_diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>(),
        vec!["redefinition of macro `MFD_CLOEXEC`".to_string()],
        "counted, not dropped — a preprocessor that deleted these would claim a clean tree \
         it never checked"
    );
    // The redefinition still *happened*; suppression is about reporting, not about semantics.
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["0x0001U"]);
}

/// **The direction that must keep working**, and the reason this is not just `diagnostics.clear()`.
/// Identical text, opposite order: the offending `#define` is now in the user's own header, and
/// gcc warns there. So must chiero.
#[test]
fn the_same_redefinition_in_a_users_own_header_is_still_reported() {
    let mut f = files(&[
        ("usr/inc/second.h", "#define MFD_CLOEXEC 0x0001U\n"),
        ("proj/first.h", "#define MFD_CLOEXEC 1U\n"),
    ]);
    let tu = preprocess_with_loader(
        "main.c",
        "#include <second.h>\n#include <first.h>\nMFD_CLOEXEC\n",
        config(),
        &mut f,
    );
    assert_eq!(
        tu.diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>(),
        vec!["redefinition of macro `MFD_CLOEXEC`".to_string()],
        "the site is the user's header, so it is theirs to fix"
    );
    assert!(
        tu.system_diagnostics.is_empty(),
        "{:?}",
        tu.system_diagnostics
    );
}

/// A diagnostic in the **main source file** is never suppressed, whatever the include paths say.
/// Trivial, and it is the assertion that fails if the rule is ever written as "suppress unless
/// the path is under `include_paths`" — the root file is under neither.
#[test]
fn a_diagnostic_in_the_translation_units_own_source_is_always_reported() {
    let mut f = files(&[]);
    let tu = preprocess_with_loader("main.c", "#define A 1\n#define A 2\nA\n", config(), &mut f);
    assert_eq!(tu.diagnostics.len(), 1, "{:?}", tu.diagnostics);
    assert!(tu.system_diagnostics.is_empty());
}

/// **A system directory is matched by containment, not by string equality.** Real system paths
/// are directory roots — `/usr/include` holds `linux/memfd.h` several levels down — and a rule
/// comparing the parent directory to the configured path would suppress `/usr/include/stdio.h`
/// while reporting `/usr/include/linux/memfd.h`, which is the case that started this.
#[test]
fn a_system_header_in_a_subdirectory_counts_as_a_system_header() {
    let mut f = files(&[
        ("proj/first.h", "#define MFD_CLOEXEC 1U\n"),
        ("usr/inc/linux/memfd.h", "#define MFD_CLOEXEC 0x0001U\n"),
    ]);
    let tu = preprocess_with_loader(
        "main.c",
        "#include <first.h>\n#include <linux/memfd.h>\nMFD_CLOEXEC\n",
        config(),
        &mut f,
    );
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(tu.system_diagnostics.len(), 1);
}
