//! **031 §3: impact closure, and the justification that makes it answerable.**
//!
//! > 4. Editing one statement in one function impacts that function and its transitive callers.
//! > 8. Changing a function's return type impacts all callers even if no body changed.
//! > 18. Deleting a function impacts its callers and is reported as `Removed`.
//! > 19. Every entity in the set has a non-empty `Justification` with a valid edge chain back to
//! >     a root.
//!
//! §3 is emphatic about the last one, and it is not decoration:
//!
//! > Every entity in the set carries the **path by which it was reached**. Auditability is a
//! > requirement: a maintainer who is told to run 400 tests must be able to ask why, and get
//! > "because `foo()` expands `vec_add1`, whose body you changed, at `ip4_forward.c:900`".
//!
//! A tool that cannot answer "why" is one whose answers get overridden, and an overridden
//! test-selection tool is a slower way to run the whole suite.
//!
//! # The direction that must not be wrong
//!
//! Closure over-approximates on purpose. Missing a caller means 032 skips a test that could have
//! caught the regression — the one failure this whole system exists to prevent — while an extra
//! caller costs a test run. Every judgement call in here is resolved toward the extra test.

use chiero_diff::{ChangeClass, Entity, ImpactEdge, Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("the fixture parses")
}

/// `leaf` <- `middle` <- `top`, so a change at the bottom has two levels to climb.
const CHAIN: &str = r#"
static int leaf (int x)
{
  return x + 1;
}

static int middle (int x)
{
  return leaf (x) * 2;
}

int top (int x)
{
  return middle (x) + 1;
}
"#;

fn names_in(set: &chiero_diff::ImpactSet) -> Vec<&str> {
    set.entities.keys().map(Entity::name).collect()
}

/// **Contract 4.** The edit is in `leaf`; `middle` calls it and `top` calls `middle`.
#[test]
fn a_body_edit_reaches_its_transitive_callers() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(CHAIN), &prog(&edited));
    assert_eq!(
        names_in(&set),
        vec!["leaf", "middle", "top"],
        "one statement changed, and everything that can reach it is in the set"
    );
    assert_eq!(
        set.entities[&Entity::function("f.c", "leaf")].class,
        ChangeClass::BodyChanged
    );
}

/// **Contract 19.** Every entity carries how it was reached, and the root is the entity that
/// actually changed.
#[test]
fn every_entity_says_why_it_is_there() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(CHAIN), &prog(&edited));
    let root = Entity::function("f.c", "leaf");

    for (e, j) in &set.entities {
        assert_eq!(
            j.root,
            root,
            "{} was reached from the change to `leaf`",
            e.name()
        );
        assert!(!j.edges.is_empty(), "{} has no edge chain", e.name());
    }
    assert_eq!(
        set.entities[&root].edges,
        vec![ImpactEdge::DirectlyChanged],
        "the root's own justification is that it is the thing that changed"
    );
    // Distance is how far the closure walked, which is what a report orders by.
    assert_eq!(set.entities[&root].distance, 0);
    assert_eq!(set.entities[&Entity::function("f.c", "middle")].distance, 1);
    assert_eq!(set.entities[&Entity::function("f.c", "top")].distance, 2);
}

/// The edge names the call, so a report can say *because it calls this*.
#[test]
fn a_caller_is_reached_by_a_call_edge() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(CHAIN), &prog(&edited));
    assert_eq!(
        set.entities[&Entity::function("f.c", "middle")].edges,
        vec![ImpactEdge::Calls {
            callee: "leaf".to_string()
        }]
    );
}

/// **Contract 8.** A return type changed and no body did; every caller is still affected.
#[test]
fn a_signature_change_reaches_callers_with_no_body_edit() {
    let edited = CHAIN.replace("static int leaf (int x)", "static long leaf (int x)");
    let set = impact(&prog(CHAIN), &prog(&edited));
    assert_eq!(names_in(&set), vec!["leaf", "middle", "top"]);
    assert_eq!(
        set.entities[&Entity::function("f.c", "leaf")].class,
        ChangeClass::SignatureChanged
    );
}

/// **Contract 18.** A deleted function reaches its callers, and is itself `Removed`.
///
/// The callers have to be found in the program that *no longer has it* — which is the one place
/// closure cannot walk the new side alone.
#[test]
fn deleting_a_function_reaches_its_callers() {
    let without = CHAIN
        .replace("  return leaf (x) * 2;", "  return x * 2;")
        .replace("static int leaf (int x)\n{\n  return x + 1;\n}\n", "");
    let set = impact(&prog(CHAIN), &prog(&without));
    assert_eq!(
        set.entities[&Entity::function("f.c", "leaf")].class,
        ChangeClass::Removed
    );
    assert!(
        set.entities
            .contains_key(&Entity::function("f.c", "middle")),
        "`middle` called it and no longer does, which is a change to `middle` as well"
    );
}

/// A function that reaches nothing changed stays out. Closure that pulled in the whole file would
/// pass every test above and be useless.
#[test]
fn an_unrelated_function_stays_out() {
    let src = format!("{CHAIN}\nint unrelated (int x) {{ return x - 1; }}\n");
    let edited = src.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(&src), &prog(&edited));
    assert!(
        !set.entities
            .contains_key(&Entity::function("f.c", "unrelated")),
        "it calls nothing that changed: {:?}",
        names_in(&set)
    );
}

/// **A global's readers are reached too**, not only a function's callers — 032 selects on any
/// entity whose behaviour could differ.
#[test]
fn a_changed_global_reaches_the_functions_that_read_it() {
    let src =
        "int limit = 10;\nint over (int x) { return x > limit; }\nint idle (int x) { return x; }\n";
    let edited =
        "int limit = 20;\nint over (int x) { return x > limit; }\nint idle (int x) { return x; }\n";
    let set = impact(&prog(src), &prog(edited));
    assert_eq!(
        set.entities[&Entity::global("f.c", "limit")].class,
        ChangeClass::InitializerChanged
    );
    assert!(set.entities.contains_key(&Entity::function("f.c", "over")));
    assert!(
        !set.entities.contains_key(&Entity::function("f.c", "idle")),
        "`idle` never mentions it"
    );
}

/// **A cycle terminates.** Mutual recursion is ordinary C, and a fixpoint that revisits an entity
/// it has already placed would not stop.
#[test]
fn mutual_recursion_terminates() {
    let src = "int ping (int x);\nint pong (int x) { return ping (x - 1); }\n\
               int ping (int x) { return x > 0 ? pong (x) : 0; }\n";
    let edited = src.replace(
        "return x > 0 ? pong (x) : 0;",
        "return x > 1 ? pong (x) : 0;",
    );
    let set = impact(&prog(src), &prog(&edited));
    assert!(set.entities.contains_key(&Entity::function("f.c", "ping")));
    assert!(set.entities.contains_key(&Entity::function("f.c", "pong")));
}

/// **Contract 12.** Two `static` functions with one name, in two files: changing one impacts only
/// its own callers.
///
/// 014 §4 makes a `static` function file-scoped and never merged across translation units, and
/// `Entity` carries the file for exactly this. Merging them by name would select the other file's
/// tests for a change it never saw — and, worse, would make the two indistinguishable in a report.
///
/// The same identity question `chiero-gcov`'s `FuncKey` answers, one layer up: there, two
/// `helper`s in two objects must not share coverage.
#[test]
fn two_static_helpers_of_one_name_are_two_entities() {
    let a =
        "static int helper (int x) { return x + 1; }\nint use_a (int x) { return helper (x); }\n";
    let b =
        "static int helper (int x) { return x + 1; }\nint use_b (int x) { return helper (x); }\n";

    let a_edited = a.replace("return x + 1;", "return x + 2;");
    let set = impact(
        &Program::parse("a.c", a).expect("a.c parses"),
        &Program::parse("a.c", &a_edited).expect("a.c parses"),
    );
    assert!(
        set.entities
            .contains_key(&Entity::function("a.c", "helper"))
    );
    assert!(set.entities.contains_key(&Entity::function("a.c", "use_a")));
    assert!(
        !set.entities
            .contains_key(&Entity::function("b.c", "helper")),
        "b.c's helper is a different function that happens to share a name"
    );

    // And b.c, compared against itself, is untouched by any of it.
    let b_prog = Program::parse("b.c", b).expect("b.c parses");
    let b_again = Program::parse("b.c", b).expect("b.c parses");
    assert!(impact(&b_prog, &b_again).entities.is_empty());
}

/// **Which lines of an entity changed — the granularity 032 §3.2 needs and 031 did not report.**
///
/// §3.2 drops "a test whose arc-level coverage shows it never entered the block containing the
/// change". *The change*, not *the entity*: a test that entered a twenty-line function has
/// reached some of its lines, so an impact set located only to entities can never let that
/// refinement fire. `chiero-gcov` can answer `line_reached` for a line; nothing was telling it
/// which line to ask about.
///
/// The lines are computed by trimming the common prefix and suffix of the two sides' per-line
/// tokens — the classic diff trim. It **over-approximates within the entity**, which is the safe
/// direction: a wrong answer here adds lines to ask about, and asking about a line the test
/// reached keeps the test.
#[test]
fn a_body_edit_reports_the_lines_that_differ() {
    let before =
        "int f (int x)\n{\n  int a = 1;\n  int b = 2;\n  int c = 3;\n  return a + b + c;\n}\n";
    let after =
        "int f (int x)\n{\n  int a = 1;\n  int b = 9;\n  int c = 3;\n  return a + b + c;\n}\n";
    let set = impact(
        &Program::parse("f.c", before).expect("parses"),
        &Program::parse("f.c", after).expect("parses"),
    );

    let j = &set.entities[&Entity::function("f.c", "f")];
    assert_eq!(
        j.changed_lines,
        vec![4],
        "only line 4 differs; the rest of the function is untouched"
    );
}

/// An entity reached by the closure changed nothing *in itself*, so it reports no lines — and a
/// caller must read that as "no line to ask about", not "no lines changed anywhere".
#[test]
fn an_entity_reached_by_closure_reports_no_changed_lines() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(CHAIN), &prog(&edited));
    assert!(
        set.entities[&Entity::function("f.c", "middle")]
            .changed_lines
            .is_empty(),
        "`middle` is in the set because it calls `leaf`, not because it changed"
    );
    assert!(
        !set.entities[&Entity::function("f.c", "leaf")]
            .changed_lines
            .is_empty()
    );
}

/// A signature change reports the line the declarator is on, which is what a test would have to
/// have reached to observe it.
#[test]
fn a_signature_change_reports_its_own_line() {
    let before = "int f (int x)\n{\n  return x;\n}\n";
    let after = "long f (int x)\n{\n  return x;\n}\n";
    let set = impact(
        &Program::parse("f.c", before).expect("parses"),
        &Program::parse("f.c", after).expect("parses"),
    );
    assert_eq!(
        set.entities[&Entity::function("f.c", "f")].changed_lines,
        vec![1]
    );
}

/// **Added and removed entities report every line they have**, because the whole thing is the
/// change — and for a removal there is no new side to diff against at all.
#[test]
fn an_added_entity_reports_all_of_its_lines() {
    let before = "int f (int x) { return x; }\n";
    let after = "int f (int x) { return x; }\nint g (int y)\n{\n  return y + 1;\n}\n";
    let set = impact(
        &Program::parse("f.c", before).expect("parses"),
        &Program::parse("f.c", after).expect("parses"),
    );
    let lines = &set.entities[&Entity::function("f.c", "g")].changed_lines;
    assert!(
        lines.contains(&2) && lines.contains(&4),
        "the whole of `g` is new: {lines:?}"
    );
}
