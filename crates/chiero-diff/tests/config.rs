//! **031 contract 16 and §3.5: a `#if` condition is a change even when nothing moved.**
//!
//! > 16. Changing a `#if` condition impacts every TU under the affected `ConfigId`.
//!
//! > §3.5: A `ConfigChanged` invalidates every TU compiled under the affected `ConfigId` — and
//! > because VPP's multiarch compiles one source under several variants, a config change can
//! > impact a superset of the obvious files.
//!
//! # The trap, which is the whole reason this contract exists
//!
//! **The preprocessor consumes `#if` lines.** A condition that changed while its *outcome* did
//! not produces a byte-identical token stream, so every entity compares equal and the impact set
//! is empty. `#if FOO > 2` becoming `#if FOO > 3` under `FOO=1` is invisible to everything else
//! in this crate — and it is exactly the change that behaves differently under `FOO=3`.
//!
//! That is not a hypothetical shape. VPP's headers are full of `#if` on feature and architecture
//! macros, and one build's answer is not another's; §3.5 says a config change "can impact a
//! superset of the obvious files" for precisely this reason.
//!
//! A tool that reported "no impact" here would be confidently wrong in the direction 032 acts on.

use chiero_diff::{ChangeClass, Completeness, Entity, ImpactEdge, Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("the fixture parses")
}

/// `FOO` is undefined, so both conditions are false and neither branch is compiled. The token
/// streams are identical.
const BEFORE: &str = "#if FOO > 2\nint gated (void) { return 1; }\n#endif\n\
                      int always (void) { return 0; }\n";
const AFTER: &str = "#if FOO > 3\nint gated (void) { return 1; }\n#endif\n\
                     int always (void) { return 0; }\n";

/// The premise: nothing else in this crate can see the difference.
#[test]
fn the_token_streams_really_are_identical() {
    let (a, b) = (prog(BEFORE), prog(AFTER));
    let a_names: Vec<&str> = a.entities().map(Entity::name).collect();
    let b_names: Vec<&str> = b.entities().map(Entity::name).collect();
    assert_eq!(a_names, b_names, "both sides compile the same declarations");
    assert_eq!(a_names, vec!["always"], "the gated branch is in neither");
}

/// **Contract 16.** The condition changed, so every entity under this configuration is impacted.
#[test]
fn a_changed_condition_impacts_the_translation_unit() {
    let set = impact(&prog(BEFORE), &prog(AFTER));
    assert!(
        set.entities
            .contains_key(&Entity::function("f.c", "always")),
        "the TU is compiled under a configuration whose meaning changed: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert_eq!(
        set.entities[&Entity::function("f.c", "always")].class,
        ChangeClass::ConfigChanged
    );
    assert_eq!(
        set.entities[&Entity::function("f.c", "always")]
            .edges
            .first(),
        Some(&ImpactEdge::UnderConfig {
            condition: "FOO > 2".to_string()
        }),
        "and the report names the condition, so a maintainer can check it themselves"
    );
}

/// **And the answer is `Partial`.** chiero evaluated one configuration; the condition's meaning
/// under every *other* configuration is exactly what it did not compute (§3.5), so the set is an
/// approximation and must say so.
#[test]
fn a_config_change_makes_the_answer_partial() {
    match impact(&prog(BEFORE), &prog(AFTER)).completeness {
        Completeness::Partial {
            unknown_configs, ..
        } => assert!(
            !unknown_configs.is_empty(),
            "the other configurations were not enumerated, and that is the gap"
        ),
        other => panic!("expected Partial, got {other:?}"),
    }
}

/// Reformatting a condition is not changing it. `#if FOO>2` and `#if FOO > 2` are the same
/// condition, and contracts 1–3's discipline does not stop at a directive.
#[test]
fn respacing_a_condition_changes_nothing() {
    let respaced = "#if   FOO>2\nint gated (void) { return 1; }\n#endif\n\
                    int always (void) { return 0; }\n";
    let set = impact(&prog(BEFORE), &prog(respaced));
    assert!(
        set.entities.is_empty(),
        "the same tokens with different whitespace: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert_eq!(set.completeness, Completeness::Complete);
}

/// A file with no conditionals at all is unaffected by any of this — the gap applies where there
/// is something chiero could not evaluate, not everywhere.
#[test]
fn a_file_without_conditionals_stays_complete() {
    let src = "int a (void) { return 1; }\n";
    let edited = "int a (void) { return 2; }\n";
    let set = impact(&prog(src), &prog(edited));
    assert_eq!(set.completeness, Completeness::Complete);
    assert_eq!(
        set.entities[&Entity::function("f.c", "a")].class,
        ChangeClass::BodyChanged,
        "an ordinary edit is still an ordinary edit"
    );
}

/// An `#ifdef` counts too: it is a condition on a macro's existence, and changing which macro it
/// names changes what the file compiles to under some configuration.
#[test]
fn an_ifdef_is_a_condition() {
    let a =
        "#ifdef ALPHA\nint gated (void) { return 1; }\n#endif\nint always (void) { return 0; }\n";
    let b =
        "#ifdef BETA\nint gated (void) { return 1; }\n#endif\nint always (void) { return 0; }\n";
    let set = impact(&prog(a), &prog(b));
    assert!(
        set.entities
            .contains_key(&Entity::function("f.c", "always")),
        "`ALPHA` and `BETA` are different questions: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
}
