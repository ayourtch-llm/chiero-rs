//! **031 contract 14 and §3.4: address-taken conservatism.**
//!
//! > 14. Taking the address of a changed function makes every signature-compatible indirect call
//! >     site an impacted caller, and `address_taken_fallbacks` is incremented.
//!
//! §3.4 says how, and says what not to do:
//!
//! > Indirect calls are handled by **address-taken conservatism**: if a changed function's
//! > address is taken, every indirect call site whose type signature is compatible is treated as
//! > a potential caller. VPP's node dispatch is table-driven indirect calls, so this matters
//! > constantly; `chiero-vpp` narrows it with knowledge of the registration tables, and **the
//! > general engine does not guess**.
//!
//! So this is a *gap*, not an analysis — and §4 governs gaps: every one widens the set. It is the
//! first thing in the crate to increment `address_taken_fallbacks`, which until now was a field
//! that could only ever report zero.
//!
//! # Why the count matters as much as the edge
//!
//! A maintainer reading "412 entities impacted" needs to know how many arrived through a
//! fallback rather than through a resolved call. Without the number, a conservative answer and a
//! precise one look identical — and the whole reason `chiero-vpp` exists is to turn the first
//! into the second where it knows the registration tables.

use chiero_diff::{Completeness, Entity, ImpactEdge, Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("the fixture parses")
}

/// `handler`'s address goes into a table; `dispatch` calls through a pointer.
const TABLE: &str = r#"
int handler (int x)
{
  return x + 1;
}

int other (int y)
{
  return y * 2;
}

typedef int (*fn_t) (int);

fn_t table[2] = { handler, other };

int dispatch (int which, int arg)
{
  fn_t f = table[which];
  return f (arg);
}

int direct (int arg)
{
  return other (arg);
}
"#;

/// **Contract 14.** The changed function's address is taken, so the indirect call site is a
/// potential caller.
#[test]
fn an_address_taken_change_reaches_indirect_call_sites() {
    let edited = TABLE.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(TABLE), &prog(&edited));

    assert!(
        set.entities
            .contains_key(&Entity::function("f.c", "dispatch")),
        "`handler`'s address is in `table` and `dispatch` calls through a pointer: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert_eq!(
        set.entities[&Entity::function("f.c", "dispatch")]
            .edges
            .first(),
        Some(&ImpactEdge::IndirectCall {
            callee: "handler".to_string()
        }),
        "and the report says it is a fallback, not a resolved call"
    );
}

/// **And the count says how many answers are conservative.** Without it, a fallback and a
/// resolved call look the same in a report.
#[test]
fn the_fallback_is_counted() {
    let edited = TABLE.replace("return x + 1;", "return x + 2;");
    let set = impact(&prog(TABLE), &prog(&edited));
    match set.completeness {
        Completeness::Partial {
            address_taken_fallbacks,
            ..
        } => assert!(
            address_taken_fallbacks >= 1,
            "one address-taken function reached one indirect site"
        ),
        other => panic!("a fallback makes the result Partial: {other:?}"),
    }
}

/// **A change to a function whose address is never taken does not reach indirect sites.** This is
/// the difference between conservatism and giving up: `other`'s address *is* taken, `direct`
/// calls it by name, and an edit to a third function reaches neither.
#[test]
fn a_function_whose_address_is_not_taken_reaches_no_indirect_site() {
    let src = format!("{TABLE}\nstatic int private (int z) {{ return z - 1; }}\n");
    let edited = src.replace("return z - 1;", "return z - 2;");
    let set = impact(&prog(&src), &prog(&edited));

    assert!(
        !set.entities
            .contains_key(&Entity::function("f.c", "dispatch")),
        "nothing put `private` in a table: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert_eq!(
        set.completeness,
        Completeness::Complete,
        "and no fallback was applied, so nothing is approximated"
    );
}

/// **Arity is the compatibility filter the general engine can apply.** A call passing one
/// argument cannot be a call to a two-parameter function, so a two-parameter function's address
/// being taken does not drag in every one-argument indirect call.
///
/// This is deliberately weaker than full type compatibility — §3.4 leaves the narrowing to
/// `chiero-vpp`, which knows the registration tables. Weaker in the *safe* direction: it matches
/// more sites than a type check would, never fewer.
#[test]
fn an_incompatible_arity_is_not_a_potential_caller() {
    let src = "int two (int a, int b) { return a + b; }\n\
               typedef int (*two_t) (int, int);\n\
               two_t slot = two;\n\
               typedef int (*one_t) (int);\n\
               int call_one (one_t f, int x) { return f (x); }\n";
    let edited = src.replace("return a + b;", "return a - b;");
    let set = impact(&prog(src), &prog(&edited));

    assert!(
        !set.entities
            .contains_key(&Entity::function("f.c", "call_one")),
        "it calls through a one-argument pointer; `two` takes two: {:?}",
        set.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
}

/// A direct call is still a direct call. The fallback must not replace the precise answer for
/// callers that name the function.
#[test]
fn a_direct_caller_keeps_its_call_edge() {
    let edited = TABLE.replace("return y * 2;", "return y * 3;");
    let set = impact(&prog(TABLE), &prog(&edited));
    assert_eq!(
        set.entities[&Entity::function("f.c", "direct")]
            .edges
            .first(),
        Some(&ImpactEdge::Calls {
            callee: "other".to_string()
        }),
        "`direct` names `other`; nothing about that is approximate"
    );
}
