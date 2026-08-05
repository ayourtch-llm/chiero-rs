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

/// Two files: a header and the `.c` that includes it.
///
/// **In memory, not on disk.** `chiero-pp` has no filesystem loader by design, and that is a
/// help here rather than an obstacle: the contract compares a program against one whose header
/// has a *different* content, and a loader that answers from a map can hold both without a
/// temporary directory or an ordering hazard between the two parses.
struct Header(String);

impl chiero_pp::FileLoader for Header {
    fn load(&mut self, path: &std::path::Path) -> std::io::Result<String> {
        if path.file_name().and_then(|f| f.to_str()) == Some("m.h") {
            return Ok(self.0.clone());
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} is not part of this fixture", path.display()),
        ))
    }
}

fn programs(header_before: &str, header_after: &str, main_c: &str) -> (Program, Program) {
    let mut cfg = chiero_pp::Config::default();
    cfg.iquote_paths.push(std::path::PathBuf::from("."));
    let before = Program::parse_with(
        "main.c",
        main_c,
        cfg.clone(),
        &mut Header(header_before.into()),
    )
    .expect("before parses");
    let after = Program::parse_with("main.c", main_c, cfg, &mut Header(header_after.into()))
        .expect("after parses");
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
        !set.entities
            .contains_key(&Entity::function("main.c", "untouched")),
        "and a function that does not expand it stays out"
    );

    // --- what a coverage-only tool answers, on the same diff -----------------------------
    //
    // The diff touches one line: the `#define` in `m.h`. A coverage-only tool intersects the
    // *changed lines* with its index, and gcov has no entry for a macro's definition line — 030
    // §1 measures it and `tests/corpus/coverage/t.c` pins it — so every lookup misses.
    let selected = coverage_only_baseline(&[("m.h", 1)]);
    assert!(
        selected.is_empty(),
        "the coverage-only baseline has nothing to intersect: {selected:?}"
    );
}

/// What a coverage-only tool selects for a set of changed `(file, line)` pairs: the tests its
/// index attributes to those lines.
///
/// **Against the committed fixture, not a mock.** `tests/corpus/coverage/t.c` uses a macro from
/// `m.h` twice on line 3; gcov put both expansions on `t.c:3` and left `m.h:1`, the macro's own
/// line, with no record at all. Using the real artifacts is what stops this baseline drifting
/// from what gcov does — which is the whole force of the comparison.
fn coverage_only_baseline(changed: &[(&str, u32)]) -> Vec<chiero_gcov::TestId> {
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage");
    let mut idx = chiero_gcov::CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, chiero_gcov::TestId(0), &corpus, "t")
        .expect("the pinned fixture");

    let mut out: Vec<chiero_gcov::TestId> = Vec::new();
    for (file, line) in changed {
        for t in idx.tests_for_line(file, *line).unwrap_or_default() {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// The baseline is not vacuous: the same lookup against a line gcov *did* record finds the test.
/// Without this, "coverage selects nothing" would be indistinguishable from a broken lookup.
#[test]
fn the_coverage_baseline_can_see_a_line_it_recorded() {
    assert_eq!(
        coverage_only_baseline(&[("t.c", 3)]),
        vec![chiero_gcov::TestId(0)],
        "the expansion *site* is recorded, which is exactly why the definition is not"
    );
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
        set.entities
            .contains_key(&Entity::function("main.c", "deep")),
        "`deep` expands OUTER, which expands MID, which expands INNER — three levels"
    );
    assert!(
        set.entities
            .contains_key(&Entity::function("main.c", "shallow"))
    );
    assert!(
        !set.entities
            .contains_key(&Entity::function("main.c", "none")),
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
