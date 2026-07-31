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

/// **The report says the program checks this pointer, when it does.**
///
/// `int v = *p; if (p) …` is the shape a reader most wants explained. Chiero reports it — the null
/// state reaches the dereference before the guard can prune it — and the sentence it produces
/// explains *chiero's* reasoning:
///
/// ```text
///   null-dereference: access at offset 0 of NULL, where %1 is a pointer parameter assumed to be
///   possibly null
/// ```
///
/// "Assumed" is the weakest thing that could be said. The function tests `p` against null four
/// tokens later, so the author's own code says the pointer can be null — and that is evidence a
/// reader cannot argue with, where an assumption invites "but my callers never pass null".
///
/// 023 §9: a report a person cannot act on is not a report. This is the same idea one step further
/// — a report a person can *dismiss* is barely one, and the difference is which evidence it cites.
#[test]
fn a_null_dereference_cites_the_program_s_own_check() {
    // **With a `SourceMap`, because the clause names a line.** The plain `findings` helper builds
    // an engine without one, and `render_loc` then says "source offset 31" — true, and not
    // something a reader can open an editor at. The point of citing the program's own check is that
    // it can be looked at.
    let src = "int probe(int *p){ int v = *p; if (p) return v; return 0; }";
    let (m, map) = harness::lower_maybe_with_map(src).expect("the fixture lowers");
    let mut arena = TermArena::new();
    let f = Engine::new(&m)
        .with_source_map(&map)
        .with_entry("probe")
        .run(&mut arena)
        .findings();
    let d = f
        .iter()
        .find(|x| x.contains("null-dereference"))
        .unwrap_or_else(|| panic!("the dereference must still be reported: {f:?}"));
    assert!(
        d.contains("tests it"),
        "the function checks `p` for null below the dereference, and saying so is stronger \
         evidence than the assumption chiero made: {d:?}"
    );
    assert!(
        d.contains("t.c:1:"),
        "and the check has a location, so a reader can go and look at it: {d:?}"
    );
}

/// The three ways the search could be wrong, each of which mutation kept alive.
///
/// Not "does it find a check" but *which* check, and against what. Every one of these passed the
/// two tests above:
///
/// ```text
///   any comparison counts as a null test    `p == q` would claim a null check
///   only null-on-the-right is matched       `if (0 == p)` would find nothing
///   any slot's load counts                  another parameter's check would be attributed to p
/// ```
///
/// The third is the one that would put a false line number in a report, which is worse than the
/// vague sentence this replaced — a reader who opens the cited line and finds a check on a
/// different variable learns not to trust the next finding either.
#[test]
fn the_cited_check_is_a_null_test_of_this_pointer() {
    let cite = |src: &str| -> String {
        let (m, map) = harness::lower_maybe_with_map(src).expect("lowers");
        let mut arena = TermArena::new();
        Engine::new(&m)
            .with_source_map(&map)
            .with_entry("probe")
            .run(&mut arena)
            .findings()
            .into_iter()
            .find(|x| x.contains("null-dereference"))
            .unwrap_or_else(|| panic!("the dereference must be reported"))
    };

    // Null on the *left*, which C allows and a one-sided match would miss.
    let d = cite("int probe(int *p){ int v = *p; if (0 == p) return v; return 0; }");
    assert!(
        d.contains("tests it"),
        "`0 == p` is a null test whichever side the constant is on: {d:?}"
    );

    // A comparison against another pointer says nothing about either being null.
    let d = cite("int probe(int *p, int *q){ int v = *p; if (p == q) return v; return 0; }");
    assert!(
        !d.contains("tests it"),
        "`p == q` is not a null test, and citing it would send a reader to a line that does not \
         check anything: {d:?}"
    );

    // A null test of a *different* parameter must not be attributed to this one.
    let d = cite("int probe(int *p, int *q){ int v = *p; if (q) return v; return 0; }");
    assert!(
        !d.contains("tests it"),
        "the function tests `q`, not `p`, and a finding that cites the wrong line is worse than \
         one that cites none: {d:?}"
    );
}

/// A function that never tests the pointer gets no such clause. **The control.**
///
/// Without it, a fix that appended the sentence unconditionally would satisfy the test above and
/// claim the program checks a pointer it never mentions again — which is worse than the assumption
/// it replaced, because it is false rather than weak.
#[test]
fn a_null_dereference_claims_no_check_that_is_not_there() {
    let f = findings("int probe(int *p){ return *p; }");
    let d = f
        .iter()
        .find(|x| x.contains("null-dereference"))
        .unwrap_or_else(|| panic!("the dereference must be reported: {f:?}"));
    assert!(
        !d.contains("tests it"),
        "nothing in this function tests `p`, so the report must not say it does: {d:?}"
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

/// **Every pointer parameter forks, not just the first.**
///
/// Also written because mutation said so: forking only `ptr_params[0]` passed everything
/// above, since every fixture there takes exactly one pointer. A checked first parameter
/// beside an unchecked second is the common shape in real code — the author guarded the one
/// they were thinking about — and it is precisely the case a first-only fork cannot see.
#[test]
fn each_pointer_parameter_gets_its_own_null_state() {
    let src = "int probe(int *p, int *q){ if (!p) return 0; return *q; }";
    assert!(
        nulls(src) >= 1,
        "`q` is dereferenced unchecked even though `p` was guarded: {:?}",
        findings(src)
    );
    // And the mirror, so this cannot pass by forking only the *last* one either.
    let mirrored = "int probe(int *p, int *q){ if (!q) return 0; return *p; }";
    assert!(
        nulls(mirrored) >= 1,
        "and with the roles swapped: {:?}",
        findings(mirrored)
    );
}

/// **A finding must say what it rests on.**
///
/// Measured on `tests/corpus/c` in wave 187: **3 of 3** functions taking a pointer report a
/// null dereference, and all three are *true* — `static unsigned weight_of(const struct
/// entry *e) { return e->weight; }` really does crash on null. All three are also
/// **unactionable as written**, because every caller in the file passes `&table[i]` and the
/// report says nothing about where the null came from:
///
/// ```text
///   null-dereference: access at offset 4 of NULL through e->weight
/// ```
///
/// A reader cannot tell that from a null the *program* produced — a failed `malloc`, a
/// lookup that missed — which is the difference between "fix this" and "chiero assumed your
/// caller might do this". 023 §9's whole argument is that a report a person cannot act on
/// is not a report, and the assumption is the missing half.
///
/// Not a fidelity degradation: wave 186 settled that a null caller is a case the program
/// has, not a limit of the model. This is the report explaining its own premise.
#[test]
fn a_null_from_the_parameter_assumption_says_so() {
    let f = findings("int probe(int *p){ return *p; }");
    let null = f
        .iter()
        .find(|x| x.contains("null-dereference"))
        .expect("the unchecked dereference is reported");
    assert!(
        null.contains("parameter"),
        "the report must name the assumption it rests on: {null}"
    );
}

/// And a null the *program* produced must not claim the parameter assumption.
///
/// The control: `malloc` can fail, and that null is the program's own. A fix that appended
/// the note to every null dereference would pass the test above and fail here.
#[test]
fn a_null_the_program_produced_does_not_claim_the_assumption() {
    let f = findings(
        "void *malloc(unsigned long);\nint probe(void){ int *p = (int*)malloc(4); return *p; }",
    );
    let null = f
        .iter()
        .find(|x| x.contains("null-dereference"))
        .expect("an unchecked malloc result is reported");
    assert!(
        !null.contains("parameter"),
        "this null is the program's, not an assumption chiero made: {null}"
    );
}

/// **Only a *null* fault claims the premise, even on a state that has one.**
///
/// The state forked to make `p` null still executes the rest of the program, and a fault
/// there on a *different* pointer is an ordinary out-of-bounds. Attaching "`%0` is assumed
/// possibly null" to it would point the reader at a parameter that had nothing to do with
/// the fault.
///
/// Mutation found this: appending the clause on every fault kind rather than only
/// `null-dereference` passed every other test, because no fixture had a non-null fault on a
/// null-parameter state.
#[test]
fn a_non_null_fault_on_the_null_state_does_not_claim_the_premise() {
    let f = findings("int probe(int *p, int *q){ if (!p) return q[100000]; return *p; }");
    let oob = f
        .iter()
        .find(|x| x.contains("out-of-bounds"))
        .unwrap_or_else(|| panic!("the guarded branch over-indexes `q`: {f:?}"));
    assert!(
        !oob.contains("assumed to be possibly null"),
        "this fault is about `q`'s extent, not `p`'s nullability: {oob}"
    );
}

/// **An internal function's callers are all in this translation unit, so the assumption is
/// not chiero's to make.**
///
/// Wave 186's default rests on "the caller is outside the analysis". For a `static`
/// function that is false: every call site is in the same module, and analysing the
/// module's *exported* entry points reaches this one through them, carrying whatever the
/// caller actually passes. Assuming null here as well double-counts an assumption the outer
/// analysis makes properly, and 021 §6's own wording is "start at each **exported**
/// function in turn".
///
/// This is what the wave-187 measurement found: 3 of 3 findings on `tests/corpus/c` were
/// `static` helpers whose callers all pass `&table[i]`. Every one was true and none was
/// chiero's to raise from that entry point.
#[test]
fn an_internal_function_does_not_get_the_null_assumption() {
    let src = "static int helper(int *p){ return *p; }\n\
               int use(void){ int x = 7; return helper(&x); }";
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("helper").run(&mut arena);
    let f = r.findings();
    assert!(
        !f.iter().any(|x| x.contains("null-dereference")),
        "`helper` is static; its callers are visible and none passes null: {f:?}"
    );
}

/// And an exported one still does: its callers are genuinely outside.
///
/// The control. A fix that suppressed the assumption for every function would pass the test
/// above and silently retire wave 186 entirely.
#[test]
fn an_exported_function_still_gets_the_null_assumption() {
    let src = "int exported(int *p){ return *p; }\n\
               int use(void){ int x = 7; return exported(&x); }";
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("exported").run(&mut arena);
    let f = r.findings();
    assert!(
        f.iter().any(|x| x.contains("null-dereference")),
        "`exported` can be called from another translation unit: {f:?}"
    );
}

/// **A `static` function whose address escapes is reachable from anywhere.**
///
/// Wave 188 suppressed the null assumption for internal linkage, on the ground that every
/// call site is in this module. Taking the function's *address* breaks that ground: the
/// pointer can be stored in a global, returned, or handed to a library, and the call then
/// comes from a translation unit chiero will never see. §9 recorded this as a trap when the
/// front was written, and it is not a corner case — a `static` node function registered by
/// address is the ordinary shape in VPP.
///
/// Two ways the address escapes, because the fix must key on the *address being taken* and
/// not on one syntax for taking it.
#[test]
fn an_internal_function_whose_address_escapes_gets_the_assumption() {
    for (what, src) in [
        (
            "stored in a global",
            "static int helper(int *p){ return *p; }\n\
             int (*table)(int *) = helper;",
        ),
        (
            "explicit address-of",
            "static int helper(int *p){ return *p; }\n\
             int (*table)(int *) = &helper;",
        ),
    ] {
        let m = harness::lower(src);
        let mut arena = TermArena::new();
        let r = Engine::new(&m).with_entry("helper").run(&mut arena);
        let f = r.findings();
        assert!(
            f.iter().any(|x| x.contains("null-dereference")),
            "`{what}`: the pointer can reach a caller chiero cannot see: {f:?}"
        );
    }
}

/// And one whose address is *not* taken keeps the wave-188 suppression.
///
/// The control. A fix that treated every internal function as escaping would pass the test
/// above and undo wave 188 entirely, putting the corpus back to 3 of 3.
#[test]
fn an_internal_function_called_only_directly_keeps_the_suppression() {
    let src = "static int helper(int *p){ return *p; }\n\
               int use(void){ int x = 7; return helper(&x); }";
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("helper").run(&mut arena);
    let f = r.findings();
    assert!(
        !f.iter().any(|x| x.contains("null-dereference")),
        "a direct call is not an escaping address: {f:?}"
    );
}
