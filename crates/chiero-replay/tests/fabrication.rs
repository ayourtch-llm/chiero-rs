//! **Can `Demonstrated` be fabricated?** — the fifth review's central question.
//!
//! `Outcome::Demonstrated` is documented as "the only outcome that confirms anything", so a
//! harness that reports it for two *identical* programs is worse than no harness: it converts
//! a wrong finding into a confirmed one and removes the caveat that was true.
//!
//! Every fixture here is **two byte-identical sources**. Any `Demonstrated` is a fabrication.
//!
//! # Why these are one test file and not five
//!
//! They are one defect. `before` and `after` were called sequentially **in one process**, so
//! everything outside a translation unit was shared: libc's PRNG, the clock, both units'
//! constructors, and an `atexit` handler that outlives the result being written. Closing them
//! one at a time is what four previous rounds did, and each time the next door opened.

use chiero_replay::{Outcome, run};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-fab-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// Emit a harness for two copies of one source, and run it.
fn identical(tag: &str, body: &str) -> Outcome {
    let Some(cc) = chiero_replay::compiler() else {
        return Outcome::DidNotRun {
            detail: "no compiler".into(),
        };
    };
    let d = dir(tag);
    let (b, a) = (d.join("b.c"), d.join("a.c"));
    std::fs::write(&b, body).expect("write");
    std::fs::write(&a, body).expect("write");
    let w = chiero_exec::Witness {
        bindings: vec![chiero_exec::Binding {
            origin: chiero_exec::InputOrigin::Param {
                index: 0,
                name: String::new(),
                span: chiero_span::Span::DUMMY,
            },
            width: 32,
            value: 0,
            pinned: true,
        }],
    };
    let r = chiero_replay::emit_equivalence(&b, &a, "f", &w).expect("a scalar witness");
    run(&r, &cc, &d)
}

#[track_caller]
fn must_not_fabricate(what: &str, o: Outcome) {
    if let Outcome::Demonstrated { before, after } = o {
        panic!("{what}: two identical programs reported as differing, {before} vs {after}");
    }
}

/// **libc's PRNG is process-global**, so calling `rand()` from each version in one process
/// gives two different numbers for one program.
#[test]
fn a_process_global_prng_does_not_make_one_program_two() {
    must_not_fabricate(
        "rand()",
        identical(
            "rand",
            "#include <stdlib.h>\nint f (int x) { (void) x; return rand (); }\n",
        ),
    );
}

/// **The clock advances between the two calls.**
#[test]
fn the_clock_moving_does_not_make_one_program_two() {
    must_not_fabricate(
        "clock_gettime",
        identical(
            "clock",
            "#include <time.h>\nint f (int x)\n{\n  (void) x;\n  struct timespec t;\n  \
             clock_gettime (CLOCK_MONOTONIC, &t);\n  return (int) (t.tv_nsec / 1000);\n}\n",
        ),
    );
}

/// **`atexit` outlives the result being written.**
///
/// The verdict travels in a file in the harness's own working directory, under a discoverable
/// name. A handler registered by the analysed code runs after `main` returns and rewrites it.
/// The rule was "a channel the included code cannot write"; only the stdout instance of it was
/// closed.
#[test]
fn an_atexit_handler_cannot_rewrite_the_verdict() {
    must_not_fabricate(
        "atexit",
        identical(
            "atexit",
            "#include <stdio.h>\n#include <stdlib.h>\n#include <dirent.h>\n#include <string.h>\n\
             static void hijack (void)\n{\n  \
             DIR *d = opendir (\".\");\n  struct dirent *e;\n  \
             if (!d) return;\n  \
             while ((e = readdir (d)))\n    \
             if (strncmp (e->d_name, \"chiero_result_\", 14) == 0) {\n      \
             FILE *o = fopen (e->d_name, \"w\");\n      \
             if (o) { fputs (\"before=111 after=222\\n\", o); fclose (o); }\n    }\n  \
             closedir (d);\n}\n\
             int f (int x) { atexit (hijack); return x; }\n",
        ),
    );
}

/// **Both versions' constructors run in one process and interact through libc.**
#[test]
fn a_constructor_does_not_make_one_program_two() {
    must_not_fabricate(
        "constructor",
        identical(
            "ctor",
            "#include <stdlib.h>\n\
             __attribute__((constructor)) static void seed (void) { srand (1); }\n\
             int f (int x) { (void) x; return rand () % 1000; }\n",
        ),
    );
}

/// **And the harness still works** — the guard must not be "refuse everything".
#[test]
fn a_real_divergence_is_still_demonstrated() {
    let Some(cc) = chiero_replay::compiler() else {
        return;
    };
    let d = dir("real");
    let (b, a) = (d.join("b.c"), d.join("a.c"));
    std::fs::write(&b, "int f (int x) { return x + 1; }\n").expect("write");
    std::fs::write(&a, "int f (int x) { return x + 2; }\n").expect("write");
    let w = chiero_exec::Witness {
        bindings: vec![chiero_exec::Binding {
            origin: chiero_exec::InputOrigin::Param {
                index: 0,
                name: String::new(),
                span: chiero_span::Span::DUMMY,
            },
            width: 32,
            value: 1,
            pinned: true,
        }],
    };
    let r = chiero_replay::emit_equivalence(&b, &a, "f", &w).expect("a scalar witness");
    match run(&r, &cc, &d) {
        Outcome::Demonstrated { before, after } => assert_eq!((before, after), (2, 3)),
        other => panic!("x+1 against x+2 at x=1 differs: {other:?}"),
    }
}
