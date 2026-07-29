//! **A pointer parameter may be null, and a dereference without a check is a finding.**
//!
//! 021 §6 gives an entry function's pointer parameter a lazily-materialized object of
//! `ENTRY_PARAM_BYTES`, because "the caller is outside the analysis, so there is no right
//! answer" about how much it points at. That bound is the right shape for *extent* and the
//! wrong one for *nullability*: it makes the pointer definitely valid, so
//! `int probe(int *p) { return *p; }` was silent.
//!
//! The user's decision (wave 185): *"you can also assume all pointers to be nullable unless
//! there is an assert(p) or thereabout."* So nullability takes the opposite default from
//! extent, and a guard is what discharges it.
//!
//! Almost nothing new is needed to honour that. `malloc` has always modelled its failure as
//! a **fork** — one state holding the object, one holding `ObjectId::NULL` — and the path
//! condition does the rest. An `if (!p) return;` prunes the null state before any access,
//! and an `assert(p)` lowers to a conditional abort that does the same. What changes is only
//! the starting assumption.
//!
//! The fixtures below are the discharge rules, and the last one is the one that costs
//! something: a dereference *before* the check is still a fault, because the null state
//! reaches it. That is the shape the user asked about separately and it falls out of the
//! model rather than needing a checker of its own.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

/// Every finding, as text.
fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

fn nulls(src: &str) -> usize {
    findings(src)
        .iter()
        .filter(|f| f.contains("null-dereference"))
        .count()
}

/// The default: unchecked, so it is reported.
#[test]
fn dereferencing_an_unchecked_pointer_parameter_is_reported() {
    assert!(
        nulls("int probe(int *p){ return *p; }") >= 1,
        "a pointer from outside the analysis may be null: {:?}",
        findings("int probe(int *p){ return *p; }")
    );
}

/// A guard discharges it, whichever way round it is written.
///
/// Both spellings, because the null state must be pruned by the *path condition* rather
/// than by anything recognising a syntactic idiom — a checker keyed on `if (!p)` would pass
/// the first and fail the second.
#[test]
fn a_guard_before_the_dereference_discharges_it() {
    for (what, src) in [
        (
            "early return",
            "int probe(int *p){ if (!p) return 0; return *p; }",
        ),
        (
            "positive test",
            "int probe(int *p){ if (p) return *p; return 0; }",
        ),
        (
            "compared to null",
            "int probe(int *p){ if (p == 0) return 0; return *p; }",
        ),
    ] {
        let f = findings(src);
        assert!(
            !f.iter().any(|x| x.contains("null-dereference")),
            "`{what}` proves the pointer non-null on the path that dereferences it: {f:?}"
        );
    }
}

/// **A check *after* the dereference does not save it**, which is the point.
///
/// The null state reaches the dereference before the guard can prune it, so the fault is
/// real and is reported. This is the pattern the user asked about — a check below a
/// dereference implies the author believed the pointer could be null — and it needs no
/// checker of its own: it falls out of modelling the pointer as nullable.
#[test]
fn a_check_after_the_dereference_does_not_save_it() {
    let src = "int probe(int *p){ int v = *p; if (p) return v; return 0; }";
    assert!(
        nulls(src) >= 1,
        "the dereference happens before the guard, so the null path reaches it: {:?}",
        findings(src)
    );
}

/// A pointer the program never dereferences is not a finding.
///
/// The control against the cheapest wrong fix — reporting nullability at entry rather than
/// at the access. Nothing is undefined about *holding* a null pointer.
#[test]
fn a_pointer_that_is_never_dereferenced_is_not_reported() {
    let f = findings("int probe(int *p){ return p != 0; }");
    assert!(
        !f.iter().any(|x| x.contains("null-dereference")),
        "holding a null pointer is defined; only dereferencing it is not: {f:?}"
    );
}

/// **The null state is added, not substituted.**
///
/// Written because mutation said so: replacing the valid initial state with the null one
/// instead of forking passed every test above. Each of them asks only what is *reported*,
/// and a run that explores nothing but null paths reports the same things — while silently
/// analysing none of the program that follows a successful check.
#[test]
fn the_valid_path_survives_beside_the_null_one() {
    let m = harness::lower("int probe(int *p){ if (!p) return 7; return *p; }");
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    assert_eq!(
        r.states().len(),
        2,
        "one pointer parameter gives one null state beside the valid one"
    );
    // The valid path must reach the dereference and come back with a value; the null path
    // returns 7. Both being present is what says the fork added rather than replaced.
    let returns: Vec<Option<u128>> = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut arena))
        .collect();
    assert!(
        returns.contains(&Some(7)),
        "the null path takes the guard and returns 7: {returns:?}"
    );
    assert!(
        returns.iter().any(|v| *v != Some(7)),
        "and the valid path dereferences and returns the loaded value: {returns:?}"
    );
}
