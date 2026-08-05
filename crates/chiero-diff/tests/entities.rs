//! **031 §1 and §2: what changed, and the four contracts that say "nothing did".**
//!
//! > 1. Reformatting a file (whitespace only) yields an empty `ImpactSet`.
//! > 2. Adding a comment inside a function body yields an empty `ImpactSet`.
//! > 3. Moving a function 100 lines down without editing it yields an empty `ImpactSet`.
//! > 20. Impact of a diff against itself (empty diff) is the empty set.
//!
//! These four come first because they are what separates a change-impact tool from `grep`. Every
//! one of them is a commit that happens constantly, and a tool that answers "everything" to any
//! of them selects the whole test suite and is worse than useless — the maintainer stops
//! believing it, which is a failure mode no amount of correctness elsewhere recovers from.
//!
//! 031 §2 puts it plainly: *"`Cosmetic` producing no impact is a real, load-bearing feature."*
//!
//! # Why comparison is on tokens, not text or spans
//!
//! Moving a function down ten lines changes every span in it and every coverage line associated
//! with it, so a line-based tool sees the whole file as changed. Comparing the *entity's token
//! spelling* sees nothing, because whitespace, comments and positions are all gone by then —
//! which is 031 §2's "normalized token stream" and the reason the AST carries one `Decl` per
//! declared name rather than the C grammar's grouped declarator list.
//!
//! # Where this deviates from the spec, and why
//!
//! 031 §1 keys entities by `FileId` and `Symbol`. Both are indices into *one* `SourceMap` and one
//! interner, and impact compares two separately-parsed programs whose indices are unrelated —
//! `FileId(3)` is a different file on each side. So [`Entity`] holds the file's name and the
//! entity's name as text, which is the only identity that survives the crossing.

use chiero_diff::{ChangeClass, Entity, Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("the fixture parses")
}

const BASE: &str = r#"
static int helper (int x)
{
  return x + 1;
}

int total;

int add (int a, int b)
{
  return helper (a) + b;
}
"#;

/// **Contract 20.** A program against itself.
#[test]
fn a_program_against_itself_has_no_impact() {
    let p = prog(BASE);
    assert!(
        impact(&p, &p).entities.is_empty(),
        "an empty diff is the easiest case to get right and the one whose failure is loudest"
    );
}

/// **Contract 1.** Reformatting: different whitespace, same tokens.
#[test]
fn reformatting_changes_nothing() {
    let reformatted = "\nstatic int helper(int x){return x+1;}\nint total;\nint add(int a,int b)\
                       {return helper(a)+b;}\n";
    let set = impact(&prog(BASE), &prog(reformatted));
    assert!(
        set.entities.is_empty(),
        "whitespace is not a change, and this is the most common commit there is: {:?}",
        set.entities.keys().collect::<Vec<_>>()
    );
}

/// **Contract 2.** A comment inside a body.
#[test]
fn a_comment_changes_nothing() {
    let commented = BASE.replace("return x + 1;", "/* off by one? no. */\n  return x + 1;");
    let set = impact(&prog(BASE), &prog(&commented));
    assert!(
        set.entities.is_empty(),
        "the preprocessor removed it before anything could compare it: {:?}",
        set.entities.keys().collect::<Vec<_>>()
    );
}

/// **Contract 3.** A function moved, not edited.
///
/// This is the one a line-based tool cannot get right: every span in `add` differs, and so does
/// every coverage line gcov recorded for it.
#[test]
fn moving_a_function_changes_nothing() {
    let moved = format!("{}\n\n{}", "\n".repeat(100), BASE.trim_start());
    let set = impact(&prog(BASE), &prog(&moved));
    assert!(
        set.entities.is_empty(),
        "100 blank lines moved every span and no token: {:?}",
        set.entities.keys().collect::<Vec<_>>()
    );
}

/// And the gate is not vacuous: editing one statement is seen. A suite that only ever asserts
/// emptiness passes on a function that returns nothing.
///
/// ⚠️ This asserted `[helper]` alone while §3's closure did not exist, with a note saying the
/// callers were its job. They now arrive, so the expectation is `[add, helper]` — `add` calls
/// `helper` and is reached at distance 1. The change is the contract arriving, not an assertion
/// bent to fit; `closure.rs` is where it is pinned properly.
#[test]
fn editing_one_statement_is_seen() {
    let edited = BASE.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(BASE), &prog(&edited));
    assert_eq!(
        set.entities.keys().collect::<Vec<_>>(),
        vec![
            &Entity::function("f.c", "add"),
            &Entity::function("f.c", "helper")
        ],
        "the edit is in `helper`; `add` calls it"
    );
    assert_eq!(
        set.entities[&Entity::function("f.c", "helper")].class,
        ChangeClass::BodyChanged
    );
    assert_eq!(set.entities[&Entity::function("f.c", "helper")].distance, 0);
    assert_eq!(set.entities[&Entity::function("f.c", "add")].distance, 1);
}

/// A signature change is a different class from a body change, because §3 closes over it
/// differently: every caller is affected even when no body did.
#[test]
fn changing_a_signature_is_not_a_body_change() {
    let edited = BASE.replace("static int helper (int x)", "static long helper (int x)");
    let set = impact(&prog(BASE), &prog(&edited));
    assert_eq!(
        set.entities[&Entity::function("f.c", "helper")].class,
        ChangeClass::SignatureChanged
    );
}

/// Adding and removing are their own classes. Removal is also a build break for every caller,
/// which is why it is not merely "the entity is gone".
#[test]
fn adding_and_removing_are_reported_as_such() {
    let with_more = format!("{BASE}\nint extra (void) {{ return 0; }}\n");
    let added = impact(&prog(BASE), &prog(&with_more));
    assert_eq!(
        added.entities[&Entity::function("f.c", "extra")].class,
        ChangeClass::Added
    );

    let removed = impact(&prog(&with_more), &prog(BASE));
    assert_eq!(
        removed.entities[&Entity::function("f.c", "extra")].class,
        ChangeClass::Removed
    );
}

/// Entities are not only functions: a global, a typedef and a record are each their own identity,
/// so that changing one does not read as changing a function that happens to sit near it.
#[test]
fn globals_typedefs_and_records_are_entities() {
    let src = "typedef int word;\nstruct pair { int a; int b; };\nint counter = 1;\n";
    let edited = "typedef long word;\nstruct pair { int a; int b; };\nint counter = 1;\n";
    let set = impact(&prog(src), &prog(edited));
    assert_eq!(
        set.entities.keys().collect::<Vec<_>>(),
        vec![&Entity::typedef("f.c", "word")],
        "only the typedef differs; the struct and the global are untouched"
    );
}

/// **Deterministic order** (031 §5), which is what makes two runs comparable and a report
/// diffable. Entities are ordered by kind, then file, then name — never by discovery.
#[test]
fn the_order_is_by_kind_then_file_then_name() {
    let src = "int b;\nint a;\nvoid g (void) {}\nvoid f (void) {}\n";
    let edited = "int b = 1;\nint a = 1;\nvoid g (void) { ; }\nvoid f (void) { ; }\n";
    let set = impact(&prog(src), &prog(edited));
    let names: Vec<String> = set.entities.keys().map(|e| e.name().to_string()).collect();
    assert_eq!(
        names,
        vec!["f", "g", "a", "b"],
        "functions before globals, and each group sorted by name"
    );
}
