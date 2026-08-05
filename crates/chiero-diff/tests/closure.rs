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
        assert_eq!(j.root, root, "{} was reached from the change to `leaf`", e.name());
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
    let without = CHAIN.replace("  return leaf (x) * 2;", "  return x * 2;")
        .replace("static int leaf (int x)\n{\n  return x + 1;\n}\n", "");
    let set = impact(&prog(CHAIN), &prog(&without));
    assert_eq!(
        set.entities[&Entity::function("f.c", "leaf")].class,
        ChangeClass::Removed
    );
    assert!(
        set.entities.contains_key(&Entity::function("f.c", "middle")),
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
        !set.entities.contains_key(&Entity::function("f.c", "unrelated")),
        "it calls nothing that changed: {:?}",
        names_in(&set)
    );
}

/// **A global's readers are reached too**, not only a function's callers — 032 selects on any
/// entity whose behaviour could differ.
#[test]
fn a_changed_global_reaches_the_functions_that_read_it() {
    let src = "int limit = 10;\nint over (int x) { return x > limit; }\nint idle (int x) { return x; }\n";
    let edited = "int limit = 20;\nint over (int x) { return x > limit; }\nint idle (int x) { return x; }\n";
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
    let edited = src.replace("return x > 0 ? pong (x) : 0;", "return x > 1 ? pong (x) : 0;");
    let set = impact(&prog(src), &prog(&edited));
    assert!(set.entities.contains_key(&Entity::function("f.c", "ping")));
    assert!(set.entities.contains_key(&Entity::function("f.c", "pong")));
}
