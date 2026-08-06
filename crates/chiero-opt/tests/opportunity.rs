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
