//! **031 §4 and contract 15: what the analysis could not see.**
//!
//! > 15. A file that fails to parse puts all of its entities in the set and sets `Partial`.
//!
//! §4 is the sharpest paragraph in the spec, and it is about direction:
//!
//! > Impact analysis must be **over-approximate to be useful**: missing an impacted entity means
//! > silently skipping the test that would have caught the regression. So every gap widens the
//! > set rather than narrowing it. […] `Partial` is reported prominently and is what makes 032's
//! > always-run set non-empty. **A tool that quietly narrows here is worse than no tool: it
//! > converts an unknown into a false assurance.**
//!
//! This is the same distinction `chiero-gcov` spent four waves getting right, arriving from the
//! other side: there, a line with *no record* must not read as a line no test covers. Here, a file
//! that could not be read must not read as a file that did not change. Both are an absence being
//! mistaken for a measurement, and in both the mistake is silent.
//!
//! # Why `Program::parse` returning `None` was not enough
//!
//! It was honest — better than reporting an empty impact for a file nobody could read — but a
//! caller cannot act on it. `None` says "no answer"; `Partial` says *which* files, so 032 can put
//! their tests in the always-run set and a report can lead with it.

use chiero_diff::{ChangeClass, Completeness, Entity, ImpactEdge, Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("a program is always produced, parseable or not")
}

const GOOD: &str = "int a (int x) { return x + 1; }\nint b (int x) { return a (x); }\n";

/// A clean pair is `Complete`, so `Partial` means something when it appears.
#[test]
fn a_pair_that_parses_is_complete() {
    let edited = GOOD.replace("x + 1", "x + 2");
    assert_eq!(
        impact(&prog(GOOD), &prog(&edited)).completeness,
        Completeness::Complete
    );
}

/// **Contract 15.** A file that does not parse is `Partial` and names itself.
#[test]
fn a_file_that_does_not_parse_is_partial_and_names_itself() {
    let broken = "int a (int x) { return x + ; }\nint b (int x) { return a (x); }\n";
    let set = impact(&prog(GOOD), &prog(broken));
    match &set.completeness {
        Completeness::Partial { unparsed_files, .. } => {
            assert_eq!(unparsed_files, &vec!["f.c".to_string()])
        }
        other => panic!("expected Partial naming f.c, got {other:?}"),
    }
}

/// And **every entity of that file is in the set**, because nothing about it is known.
///
/// Not "the entities that look different" — the ones that parsed on the good side are exactly the
/// ones a narrowing tool would quietly drop.
#[test]
fn an_unparsed_files_entities_are_all_impacted() {
    let broken = "int a (int x) { return x + ; }\nint b (int x) { return a (x); }\n";
    let set = impact(&prog(GOOD), &prog(broken));
    let names: Vec<&str> = set.entities.keys().map(Entity::name).collect();
    assert!(names.contains(&"a"), "{names:?}");
    assert!(
        names.contains(&"b"),
        "`b` may be unchanged for all anyone knows, and that is the point: {names:?}"
    );
}

/// The class says *why*, and it is not a guess about what changed.
#[test]
fn an_unreadable_entity_is_classed_unknown() {
    let broken = "int a (int x) { return x + ; }\nint b (int x) { return a (x); }\n";
    let set = impact(&prog(GOOD), &prog(broken));
    let j = &set.entities[&Entity::function("f.c", "b")];
    assert_eq!(j.class, ChangeClass::Unknown);
    assert_eq!(
        j.edges,
        vec![ImpactEdge::FileUnparsed {
            file: "f.c".to_string()
        }],
        "a report must be able to say `chiero could not read f.c`, not invent a reason"
    );
}

/// Either side failing is enough. A file that parsed *before* and not after is the common shape —
/// someone broke it — and a file that parsed after and not before is a fix; neither is knowable.
#[test]
fn either_side_failing_makes_the_result_partial() {
    let broken = "int a (int x) { return x + ; }\n";
    assert!(matches!(
        impact(&prog(broken), &prog(GOOD)).completeness,
        Completeness::Partial { .. }
    ));
    assert!(matches!(
        impact(&prog(GOOD), &prog(broken)).completeness,
        Completeness::Partial { .. }
    ));
}

/// **`Partial` widens, it never narrows.** A real change alongside an unreadable file keeps its
/// own justification: the gap adds entities, it does not replace them.
#[test]
fn partial_adds_to_the_set_rather_than_replacing_it() {
    let broken = "int a (int x) { return x + 2; }\nint b (int x) { return a (x); }\nint c (\n";
    let set = impact(&prog(GOOD), &prog(broken));
    assert!(matches!(set.completeness, Completeness::Partial { .. }));
    assert!(
        set.entities.contains_key(&Entity::function("f.c", "a")),
        "the edit to `a` is still there"
    );
}

/// A program that does not parse still yields entities where it can. The parser recovers, and
/// what it recovered is more useful than nothing — as long as the set is marked `Partial`, which
/// is what stops it being read as a complete answer.
#[test]
fn recovery_still_yields_what_it_found() {
    let broken = "int a (int x) { return x + ; }\nint b (int x) { return a (x); }\n";
    let p = prog(broken);
    let names: Vec<&str> = p.entities().map(Entity::name).collect();
    assert!(
        names.contains(&"b"),
        "the parser recovered past the bad statement: {names:?}"
    );
}
