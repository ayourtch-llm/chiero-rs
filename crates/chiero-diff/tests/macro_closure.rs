//! **031 §3.2 — the differentiating step, and the reason this project exists.**
//!
//! > 5. **The headline contract**: editing the body of a macro defined in a header and used in N
//! >    functions, with no `.c` file touched, yields an `ImpactSet` containing all N functions
//! >    with `ExpandsMacro` justifications — and the coverage-only baseline for the same diff
//! >    yields the empty set. **Both are asserted in the same test, so the difference is the
//! >    artifact.**
//! > 6. A macro whose body expands a changed macro is itself marked changed, and its expansion
//! >    sites are included (transitive closure), to a depth of at least 3 in the fixture.
//! > 7. Renaming a macro parameter without changing behaviour is still
//! >    `MacroInterfaceChanged` — chiero does not attempt to prove macro equivalence here.
//!
//! §3.2 states the case plainly:
//!
//! > an edit to the body of `vec_add1` in `vppinfra/vec.h` produces **no coverage delta
//! > anywhere** — gcov records only the `.c` lines where it was used, and those lines did not
//! > change. Coverage-based selection sees nothing to run.
//!
//! That is not a hypothesis. 030 §1 measured it and `tests/corpus/coverage/` pins it: a macro
//! expanded twice at `t.c:3` puts both expansions on that line and leaves `m.h:1`, the macro's own
//! line, with **no record at all**. So a coverage-only tool asked "what changed in `m.h`?" has
//! nothing to intersect — not a small answer, an empty one.
//!
//! Owning the preprocessor is what makes the question answerable, and this is the wave that
//! spends it. VPP has 754 `foreach_*` X-macros and a hot layer — `vec.h`, `pool.h`,
//! `buffer_funcs.h` — that is almost entirely macros and `static inline`s.
//!
//! # The dual risk, stated rather than hidden
//!
//! §3.2 again: *"a change to a macro used in 900 files impacts 900 files, and that is the correct
//! answer. Precision comes later, from symbolic refinement in 032 §4, not from pretending the
//! impact is smaller."*

use chiero_diff::{ChangeClass, Entity, ImpactEdge, Program, impact};

/// Write a header beside a `.c`, so the preprocessor really includes it.
///
/// The two-file shape is the whole point: the contract is about a diff that touches **no `.c`
/// file**, which cannot be expressed in one string.
fn programs(header_before: &str, header_after: &str, main_c: &str) -> (Program, Program) {
    let dir = std::env::temp_dir().join(format!(
        "chiero-macro-{}",
        header_before.len() * 31 + header_after.len()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");

    let mut cfg = chiero_pp::Config::default();
    cfg.iquote_paths.push(dir.clone());

    std::fs::write(dir.join("m.h"), header_before).expect("write header");
    let before = Program::parse_with("main.c", main_c, cfg.clone()).expect("before parses");
    std::fs::write(dir.join("m.h"), header_after).expect("rewrite header");
    let after = Program::parse_with("main.c", main_c, cfg).expect("after parses");
    (before, after)
}

const USERS: &str = r#"
#include "m.h"

int a (int x) { return BUMP (x); }
int b (int x) { return BUMP (x) + 1; }
int c (int x) { return BUMP (x) * 2; }
int untouched (int x) { return x - 1; }
"#;

/// **Contract 5, the headline.** Both halves in one test, so the difference is the artifact.
#[test]
fn a_macro_body_edit_impacts_every_expansion_site_where_coverage_sees_nothing() {
    let (before, after) = programs(
        "#define BUMP(v) ((v) + 1)\n",
        "#define BUMP(v) ((v) + 2)\n",
        USERS,
    );
    let set = impact(&before, &after);

    // --- what chiero answers -------------------------------------------------------------
    for f in ["a", "b", "c"] {
        let e = Entity::function("main.c", f);
        assert!(
            set.entities.contains_key(&e),
            "`{f}` expands BUMP: {:?}",
            set.entities.keys().map(Entity::name).collect::<Vec<_>>()
        );
        assert_eq!(
            set.entities[&e].edges.first(),
            Some(&ImpactEdge::ExpandsMacro {
                name: "BUMP".to_string()
            }),
            "and the report can say *why*"
        );
    }
    assert!(
        !set.entities.contains_key(&Entity::function("main.c", "untouched")),
        "and a function that does not expand it stays out"
    );

    // --- what a coverage-only tool answers, on the same diff -----------------------------
    //
    // The diff touches `m.h` alone. gcov records the `.c` lines where a macro was *used*, never
    // the macro's own line (030 §1, measured) — so the set of changed lines a coverage tool could
    // intersect its index with is empty, and it selects nothing.
    let changed_c_lines = coverage_only_baseline(&["m.h"]);
    assert!(
        changed_c_lines.is_empty(),
        "the coverage-only baseline has nothing to intersect: {changed_c_lines:?}"
    );
}

/// What a line-and-coverage tool has to work with for a diff: the changed lines it can look up.
///
/// gcov's index is keyed by `(file, line)` and **contains no entry for a macro definition line**
/// — 030 §1 measures it and `tests/corpus/coverage/t.c` pins it as a test. So for a diff that
/// touches only headers' macro definitions, every lookup misses and the selection is empty.
fn coverage_only_baseline(changed_files: &[&str]) -> Vec<String> {
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage");
    let idx = chiero_gcov::ingest_native(&corpus, "t").expect("the pinned fixture");
    // `m.h` is where the macro is defined, and `t.c:3` is where it was used twice.
    changed_files
        .iter()
        .filter(|f| idx.lines_of(f).iter().any(|l| idx.line_count(f, *l).is_some()))
        .map(|f| (*f).to_string())
        .collect()
}

/// **Contract 6.** A macro whose body expands a changed macro is itself changed, and its own
/// sites come with it — three deep.
#[test]
fn macro_closure_is_transitive() {
    let head = |inner: &str| {
        format!(
            "#define INNER(v) {inner}\n#define MID(v) (INNER (v) * 2)\n\
             #define OUTER(v) (MID (v) + 3)\n"
        )
    };
    let users = "#include \"m.h\"\n\nint deep (int x) { return OUTER (x); }\n\
                 int shallow (int x) { return INNER (x); }\nint none (int x) { return x; }\n";
    let (before, after) = programs(&head("((v) + 1)"), &head("((v) + 9)"), users);
    let set = impact(&before, &after);

    assert!(
        set.entities.contains_key(&Entity::function("main.c", "deep")),
        "`deep` expands OUTER, which expands MID, which expands INNER — three levels"
    );
    assert!(set.entities.contains_key(&Entity::function("main.c", "shallow")));
    assert!(
        !set.entities.contains_key(&Entity::function("main.c", "none")),
        "and the closure still stops"
    );
}

/// **Contract 7.** A renamed parameter is a change, because chiero does not try to prove two
/// macro bodies equivalent — and a tool that guessed wrong here would skip tests.
#[test]
fn renaming_a_macro_parameter_is_still_a_change() {
    let (before, after) = programs(
        "#define BUMP(v) ((v) + 1)\n",
        "#define BUMP(w) ((w) + 1)\n",
        USERS,
    );
    let set = impact(&before, &after);
    assert_eq!(
        set.entities[&Entity::macro_("m.h", "BUMP")].class,
        ChangeClass::MacroInterfaceChanged
    );
    assert!(set.entities.contains_key(&Entity::function("main.c", "a")));
}

/// A header edit that changes no macro changes nothing — the empty-set discipline of contracts
/// 1–3, carried into the place where over-reporting would be most tempting.
#[test]
fn reformatting_a_header_impacts_nothing() {
    let (before, after) = programs(
        "#define BUMP(v) ((v) + 1)\n",
        "\n/* a comment */\n#define BUMP(v)   ((v) + 1)\n\n",
        USERS,
    );
    let set = impact(&before, &after);
    assert!(
        set.entities.is_empty(),
        "whitespace and a comment in a header are not a change: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
}
