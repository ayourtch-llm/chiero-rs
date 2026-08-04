//! The `CC=` shim: chiero observing a real build.
//!
//! A build system invokes the compiler for many things that are not compilations — linking,
//! `--version`, dependency generation, `configure`'s throwaway probes. Getting this wrong is
//! not a missed measurement but a **broken build**, so the classification is the part worth
//! testing hardest.

use std::path::PathBuf;
use xtask::cc::sources_to_analyse;

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn a_compilation_is_told_from_everything_else() {
    // The ordinary case, and the one that matters.
    assert_eq!(
        sources_to_analyse(&args("-c foo.c -o foo.o -I/inc -DX=1")),
        vec![PathBuf::from("foo.c")]
    );

    // Compile-and-link in one step: still a compilation of `main.c`. `configure` does this
    // constantly, and skipping it would lose the probes that reveal a project's real flags.
    assert_eq!(
        sources_to_analyse(&args("main.c -o prog -lm")),
        vec![PathBuf::from("main.c")]
    );

    // Several sources in one invocation are several compilations.
    assert_eq!(
        sources_to_analyse(&args("-c a.c b.c")),
        vec![PathBuf::from("a.c"), PathBuf::from("b.c")]
    );

    // **Not compilations.** Linking has no source; `-E`, `-M` and `-MM` ask the compiler for
    // something other than a translation; `--version` is a probe. Analysing any of these
    // wastes time at best, and at worst reports a diagnostic for a file the build never
    // compiled.
    for not in [
        "-o prog a.o b.o -lm",
        "-E foo.c",
        "-M foo.c",
        "-MM foo.c",
        "--version",
        "-print-file-name=include",
    ] {
        assert!(
            sources_to_analyse(&args(not)).is_empty(),
            "not a compilation: `{not}` -> {:?}",
            sources_to_analyse(&args(not))
        );
    }

    // **An argument's *value* is not a source.** `-o out.c` names an output and `-include x.c`
    // names a prefix header; treating either as a translation unit would analyse a file the
    // build is not compiling.
    assert!(sources_to_analyse(&args("-c q.S -o out.c")).is_empty());
    assert_eq!(
        sources_to_analyse(&args("-c real.c -include prefix.c")),
        vec![PathBuf::from("real.c")]
    );
}
