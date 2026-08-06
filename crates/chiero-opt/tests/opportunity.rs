//! **041 §2 — opportunity detection. Detectors propose; they never rewrite.**
//!
//! > 15. A branch whose condition is implied by the path condition is proposed as dead, with
//! >     the implying constraints listed.
//! > 16. Every proposal in the corpus has either a discharged obligation or an advisory label
//! >     (structural check over all proposals).
//!
//! §2's rule for what a proposal is worth:
//!
//! > **A proposal with any `Open` obligation is advisory and labelled as such.** The honest
//! > statement "this looks redundant but I could not prove the intervening call does not write
//! > it" is more useful than a confident wrong claim, and it is what an LLM needs in order to
//! > decide whether to investigate.

use chiero_cir::Module;
use chiero_opt::opportunity::*;

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn cfg(entry: &str) -> OppCfg {
    OppCfg::new(entry)
}

/// `if (x > 0) { if (x > 0) { ... } }` — the inner test is decided by the outer one.
const NESTED: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp sgt i32 %0, 0i32
  br %1, bb1, bb4
bb1:
  .line 2
  %2 = cmp sgt i32 %0, 0i32
  br %2, bb2, bb3
bb2:
  .line 3
  ret 1i32
bb3:
  .line 4
  ret 2i32
bb4:
  .line 5
  ret 3i32
}";

/// **Contract 15.** The second test cannot fail once the first passed, and line 4 is dead.
#[test]
fn a_branch_the_path_condition_already_decides_is_proposed_as_dead() {
    let props = detect(&m(NESTED), &cfg("f"));
    let dead: Vec<&Proposal> = props
        .iter()
        .filter(|p| matches!(p.kind, OppKind::DeadBranch { .. }))
        .collect();
    assert!(
        !dead.is_empty(),
        "`x > 0` inside `x > 0` decides the inner branch: {props:?}"
    );
    // **With the implying constraints listed** — a proposal saying "this is dead" and not why
    // is one nobody can check.
    let p = dead[0];
    assert!(
        !p.evidence.is_empty(),
        "the constraints that imply it must be listed: {p:?}"
    );
    // Real SMT-LIB terms mentioning the parameter, not a count and not a sentence. The
    // operator is whatever the arena canonicalised to — `x > 0` arrives as `0 < x`, and a test
    // that pinned the spelling would be testing the arena's normalisation rather than the
    // detector.
    assert!(
        p.evidence
            .iter()
            .any(|e| e.contains("param0") && e.contains("bv")),
        "and they must be the actual constraints, not a count: {p:?}"
    );
    assert!(
        p.evidence.iter().any(|e| e.starts_with("decided:")),
        "and the condition that was decided, so a reader sees what as well as by what: {p:?}"
    );
}

/// **A branch that is genuinely live is not proposed.** Without this, a detector that proposes
/// every branch passes the test above.
#[test]
fn a_live_branch_is_not_proposed() {
    let live = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp sgt i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  ret 1i32
bb2:
  .line 3
  ret 2i32
}";
    let props = detect(&m(live), &cfg("f"));
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OppKind::DeadBranch { .. })),
        "both sides of `x > 0` are reachable: {props:?}"
    );
}

/// **Contract 16, structurally: every proposal is discharged or advisory.**
///
/// Not "most", and not checked per fixture — over every proposal every fixture here produces,
/// so a detector added later cannot quietly emit one that is neither.
#[test]
fn every_proposal_is_discharged_or_advisory() {
    let fixtures = [
        NESTED,
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  ret %0\n}",
    ];
    let mut seen = 0;
    for src in fixtures {
        for p in detect(&m(src), &cfg("f")) {
            seen += 1;
            let discharged = p
                .obligations
                .iter()
                .all(|o| matches!(o, Obligation::Discharged { .. }));
            assert!(
                (discharged && !p.advisory) || p.advisory,
                "a proposal that is neither discharged nor advisory: {p:?}"
            );
            assert!(
                !p.obligations.is_empty(),
                "a proposal with no obligations at all has nothing to be judged by: {p:?}"
            );
        }
    }
    assert!(seen > 0, "the check must run over something");
}

/// **A run that could not finish must not propose a branch dead.**
///
/// "No state took that edge" and "no state can take that edge" are the same observation and
/// opposite claims — the project's recurring axis, arriving in a new detector. A budget-cut
/// search has not shown anything unreachable.
#[test]
fn a_truncated_search_proposes_nothing_dead() {
    let looping = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  goto bb1
bb1:
  .line 2
  %1 = phi i32 [entry 0i32] [bb1 %2]
  %2 = add i32 %1, 1i32
  %3 = cmp slt i32 %2, %0
  br %3, bb1, bb2
bb2:
  .line 3
  %4 = cmp sgt i32 %0, 0i32
  br %4, bb3, bb4
bb3:
  .line 4
  ret 1i32
bb4:
  .line 5
  ret 2i32
}";
    let props = detect(&m(looping), &cfg("f"));
    for p in &props {
        if matches!(p.kind, OppKind::DeadBranch { .. }) {
            assert!(
                p.advisory,
                "a truncated search has not proved anything unreachable: {p:?}"
            );
        }
    }
}

/// **041 contract 17, for this module too: nothing here rewrites.**
#[test]
fn detection_does_not_change_the_module() {
    let before = m(NESTED);
    let after = m(NESTED);
    let _ = detect(&before, &cfg("f"));
    assert_eq!(
        format!("{before:?}"),
        format!("{after:?}"),
        "detect() must take the module by reference and leave it alone"
    );
}

/// **Contract 14 — the same finding, two labels, and the difference is what chiero could
/// prove.**
///
/// > 14. A redundant load across a call to a function proven not to write the address is
/// >     proposed with all obligations `Discharged`; across an unmodeled extern it is proposed
/// >     with an `Open` obligation and labelled advisory.
///
/// This is the contract that makes the obligation machinery mean something: the *observation*
/// is identical in both cases — the same address loaded twice with a call between — and only
/// the strength of the claim differs. §2 puts it plainly:
///
/// > The honest statement "this looks redundant but I could not prove the intervening call does
/// > not write it" is more useful than a confident wrong claim.
///
/// A load across a callee chiero can see through and which contains no store at all.
const ACROSS_A_PURE_CALLEE: &str = "\
func @quiet(%0: i32) -> i32 {
entry:
  .line 1
  ret %0
}

func @f(%0: ptr) -> i32 {
entry:
  .line 4
  %1 = load i32, %0 align 4
  %2 = call @quiet(%1)
  %3 = load i32, %0 align 4
  %4 = add i32 %1, %3
  ret %4
}";

/// The same shape across a function chiero has no body for.
const ACROSS_AN_UNMODELED_EXTERN: &str = "\
func @opaque(%0: i32) -> i32

func @f(%0: ptr) -> i32 {
entry:
  .line 4
  %1 = load i32, %0 align 4
  %2 = call @opaque(%1)
  %3 = load i32, %0 align 4
  %4 = add i32 %1, %3
  ret %4
}";

#[test]
fn a_redundant_load_across_a_callee_that_cannot_write_is_discharged() {
    let props = detect(&m(ACROSS_A_PURE_CALLEE), &cfg("f"));
    let p = props
        .iter()
        .find(|p| matches!(p.kind, OppKind::RedundantLoad { .. }))
        .unwrap_or_else(|| panic!("the second load is redundant: {props:?}"));
    assert!(
        p.obligations
            .iter()
            .all(|o| matches!(o, Obligation::Discharged { .. })),
        "`quiet` contains no store, so nothing can have written the address: {p:?}"
    );
    assert!(!p.advisory, "{p:?}");
    assert!(
        p.evidence.iter().any(|e| e.contains("quiet")),
        "the callee that was cleared must be named: {p:?}"
    );
}

#[test]
fn the_same_load_across_an_unmodeled_extern_is_advisory() {
    let props = detect(&m(ACROSS_AN_UNMODELED_EXTERN), &cfg("f"));
    let p = props
        .iter()
        .find(|p| matches!(p.kind, OppKind::RedundantLoad { .. }))
        .unwrap_or_else(|| panic!("the observation is the same one: {props:?}"));
    assert!(
        p.obligations
            .iter()
            .any(|o| matches!(o, Obligation::Open { .. })),
        "chiero has no body for `opaque`, so it cannot say the address was not written: {p:?}"
    );
    assert!(
        p.advisory,
        "an open obligation means advisory (041 §2): {p:?}"
    );
    assert!(
        p.rationale.to_lowercase().contains("could not prove")
            || p.obligations.iter().any(|o| match o {
                Obligation::Open { why } => why.contains("opaque"),
                _ => false,
            }),
        "and it must say which call it could not clear: {p:?}"
    );
}

/// **A load that is not redundant is not proposed.** Without this, a detector that proposes
/// every second load passes both tests above.
#[test]
fn a_load_across_a_store_is_not_redundant() {
    let writes = "\
func @f(%0: ptr, %1: i32) -> i32 {
entry:
  .line 1
  %2 = load i32, %0 align 4
  store i32 %1 -> %0 align 4
  %3 = load i32, %0 align 4
  %4 = add i32 %2, %3
  ret %4
}";
    let props = detect(&m(writes), &cfg("f"));
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OppKind::RedundantLoad { .. })),
        "the store between them is exactly what makes the second load necessary: {props:?}"
    );
}

/// **The shape real C lowers to.**
///
/// `int a = *p; quiet(a); int b = *p;` becomes an alloca, a load, a *store into the stack
/// slot*, a call, and a second load. Every store was a barrier, so the stack traffic between
/// two source-level loads suppressed the proposal and the detector fired on hand-written CIR
/// and nothing else.
///
/// **The fix is an escape check, not a cleverer aliasing rule.** A store through the address of
/// a local whose address never leaves the function cannot touch what a pointer parameter points
/// at. That is a fact about the *local*, checkable without deciding which addresses might be
/// equal — which stays 021's question.
const LOWERED_SHAPE: &str = "\
func @quiet(%0: i32) -> i32 {
entry:
  .line 1
  ret %0
}

func @f(%0: ptr) -> i32 {
  alloca %0 : i32 x 1 align 4 scope 0 lifetime scope \"a\"
entry:
  .line 4
  .scope enter 0
  %1 = addrlocal %0
  %2 = load i32, %0 align 4
  store i32 %2 -> %1 align 4
  %3 = call @quiet(%2)
  %4 = load i32, %0 align 4
  %5 = add i32 %2, %4
  .scope exit 0
  ret %5
}";

#[test]
fn a_store_into_a_local_that_never_escapes_is_not_a_barrier() {
    let props = detect(&m(LOWERED_SHAPE), &cfg("f"));
    assert!(
        props
            .iter()
            .any(|p| matches!(p.kind, OppKind::RedundantLoad { .. })),
        "the store is into a local whose address never leaves `f`: {props:?}"
    );
}

/// **And a local whose address *does* escape is still a barrier.**
///
/// Without this, the escape check is "ignore stores to locals", which is unsound the moment the
/// callee is handed the address.
#[test]
fn a_store_into_a_local_whose_address_escapes_is_still_a_barrier() {
    let escapes = "\
func @takes(%0: ptr) -> i32

func @f(%0: ptr) -> i32 {
  alloca %0 : i32 x 1 align 4 scope 0 lifetime scope \"a\"
entry:
  .line 4
  .scope enter 0
  %1 = addrlocal %0
  %2 = call @takes(%1)
  %3 = load i32, %0 align 4
  store i32 %3 -> %1 align 4
  %4 = load i32, %0 align 4
  %5 = add i32 %3, %4
  .scope exit 0
  ret %5
}";
    let props = detect(&m(escapes), &cfg("f"));
    for p in &props {
        if matches!(p.kind, OppKind::RedundantLoad { .. }) {
            assert!(
                p.advisory,
                "`takes` was handed the local's address, so nothing here is proved: {p:?}"
            );
        }
    }
}

/// **The shape gcc actually hands over: the pointer itself lives in a stack slot.**
///
/// `int f (int *p) { int a = *p; quiet (a); int b = *p; ... }` lowers so that `p` is stored
/// into an alloca and *reloaded before each dereference*. So the two loads of `*p` are through
/// two different `ValueId`s, and a criterion of "the same value loaded twice" never holds —
/// which is why the detector reported nothing for real C however the barrier rule was fixed.
///
/// **The engine already knows.** It resolves each address to a `Pointer` — an object and an
/// offset — and that is the memory model's own answer, not a second one this crate invented.
/// Keying on it is both sounder and what makes the detector work on the code people write.
const POINTER_IN_A_SLOT: &str = "\
func @quiet(%0: i32) -> i32 {
entry:
  .line 1
  ret %0
}

func @f(%0: ptr) -> i32 {
  alloca %0 : ptr x 1 align 8 scope 0 lifetime scope \"p\"
entry:
  .line 4
  .scope enter 0
  %1 = addrlocal %0
  store ptr %0 -> %1 align 8
  %2 = load ptr, %1 align 8
  %3 = load i32, %2 align 4
  %4 = call @quiet(%3)
  %5 = load ptr, %1 align 8
  %6 = load i32, %5 align 4
  %7 = add i32 %3, %6
  .scope exit 0
  ret %7
}";

#[test]
fn two_loads_of_one_address_are_redundant_however_the_address_was_spelled() {
    let props = detect(&m(POINTER_IN_A_SLOT), &cfg("f"));
    assert!(
        props
            .iter()
            .any(|p| matches!(p.kind, OppKind::RedundantLoad { .. })),
        "`%2` and `%5` are the same pointer; the engine resolved both: {props:?}"
    );
}

/// **And two genuinely different addresses are not.** Without this, keying on the engine's
/// answer could collapse into "every second load is redundant".
#[test]
fn loads_of_two_different_addresses_are_not_redundant() {
    let two = "\
func @f(%0: ptr, %1: ptr) -> i32 {
entry:
  .line 1
  %2 = load i32, %0 align 4
  %3 = load i32, %1 align 4
  %4 = add i32 %2, %3
  ret %4
}";
    let props = detect(&m(two), &cfg("f"));
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OppKind::RedundantLoad { .. })),
        "`%0` and `%1` are distinct objects: {props:?}"
    );
}

/// **041 §2's "dead store"** — a value written and then overwritten with nothing reading it in
/// between.
///
/// The mirror of the redundant load, and it keys on the same thing: the engine's answer for
/// where each store lands, rather than how the CIR spelled the address.
const OVERWRITTEN: &str = "\
func @f(%0: ptr, %1: i32, %2: i32) -> i32 {
entry:
  .line 1
  store i32 %1 -> %0 align 4
  store i32 %2 -> %0 align 4
  %3 = load i32, %0 align 4
  ret %3
}";

#[test]
fn a_store_that_is_overwritten_before_any_read_is_proposed() {
    let props = detect(&m(OVERWRITTEN), &cfg("f"));
    let p = props
        .iter()
        .find(|p| matches!(p.kind, OppKind::DeadStore { .. }))
        .unwrap_or_else(|| panic!("the first store is overwritten: {props:?}"));
    assert!(
        !p.advisory,
        "nothing reads it and nothing could have: {p:?}"
    );
    assert!(
        p.obligations
            .iter()
            .all(|o| matches!(o, Obligation::Discharged { .. })),
        "{p:?}"
    );
}

/// **A store that is read before being overwritten is not dead.** Without this, a detector
/// proposing every store passes the test above.
#[test]
fn a_store_that_is_read_is_not_dead() {
    let read = "\
func @f(%0: ptr, %1: i32, %2: i32) -> i32 {
entry:
  .line 1
  store i32 %1 -> %0 align 4
  %3 = load i32, %0 align 4
  store i32 %2 -> %0 align 4
  ret %3
}";
    let props = detect(&m(read), &cfg("f"));
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OppKind::DeadStore { .. })),
        "the load between them is what makes the first store live: {props:?}"
    );
}

/// **A call between the two stores makes it advisory**, for exactly the reason contract 14
/// gives: the callee may have read what was written, and chiero cannot say it did not.
#[test]
fn a_store_overwritten_across_an_unmodeled_call_is_advisory() {
    let across = "\
func @opaque(%0: i32) -> i32

func @f(%0: ptr, %1: i32, %2: i32) -> i32 {
entry:
  .line 1
  store i32 %1 -> %0 align 4
  %3 = call @opaque(%1)
  store i32 %2 -> %0 align 4
  %4 = load i32, %0 align 4
  ret %4
}";
    let props = detect(&m(across), &cfg("f"));
    for p in &props {
        if matches!(p.kind, OppKind::DeadStore { .. }) {
            assert!(
                p.advisory,
                "`opaque` may have read what the first store wrote: {p:?}"
            );
        }
    }
}
