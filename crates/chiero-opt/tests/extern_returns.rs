//! **041 §1.2's shared extern-return symbols.**
//!
//! > "both functions run against the **same** symbolic inputs and **the same extern-return
//! > symbols**"
//!
//! Without this, a function that calls anything returning a value is unanswerable: the call
//! mints an input on each side, `link_inputs` cannot match them, and an unmatched input is a
//! refusal. That is most of the functions anyone wants adjudicated.
//!
//! **The obvious key is wrong, and cost two false `Equivalent`s to learn.** Keying by
//! "(function, nth call to it)" counts a different thing from the effect sequence:
//! `InputOrigin::ExternReturn` is minted only for a call *with a destination*, so a discarded
//! result shifts the numbering and one version's `p(2)` gets equated with the other's `p(1)`.
//! And a `pure` callee never entered the effect sequence at all, so nothing checked that the
//! nth call on each side was even passed the same arguments — `p(x) == p(x + 1)` outright.
//!
//! The key here is the call's **position in the effect sequence**, with every declared call in
//! that sequence whether pure or not. Then "the nth call" means one thing, and the sequence
//! comparison has already established that the two nth calls are the same call with the same
//! arguments before any return is linked to any other.

use chiero_cir::Module;
use chiero_opt::{EquivCfg, Equivalence, prove_equivalent};

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn cfg() -> Option<EquivCfg> {
    let c = EquivCfg::new("f");
    c.backend.is_some().then_some(c)
}

#[track_caller]
fn must_bless(what: &str, v: Equivalence) {
    if !matches!(v, Equivalence::Equivalent { .. }) {
        panic!("{what}: should be equivalent, got {v:?}");
    }
}

#[track_caller]
fn must_not_bless(what: &str, v: Equivalence) {
    if let Equivalence::Equivalent { fidelity, .. } = &v {
        panic!("{what}: blessed with fidelity {fidelity:?} — {v:?}");
    }
}

/// **The headline: a function whose callee returns a value is answerable at all.**
///
/// `return p(x)` against `return p(x + 0)` — the same call, spelled two ways. Every previous
/// version answered `Unknown` because the two extern returns could not be matched.
#[test]
fn a_value_returning_callee_no_longer_makes_a_function_unanswerable() {
    let Some(cfg) = cfg() else { return };
    let plain = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = call @p(%0)
  ret %1
}";
    let padded = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = add i32 %0, 0i32
  %2 = call @p(%1)
  ret %2
}";
    must_bless(
        "p(x) against p(x + 0)",
        prove_equivalent(&m(plain), &m(padded), &cfg),
    );
}

/// **Two calls, both results used, in the same order.** The returns must line up with the
/// calls that produced them.
#[test]
fn two_value_returning_calls_line_up_with_their_own_results() {
    let Some(cfg) = cfg() else { return };
    let f = "\
func @p(%0: i32) -> i32

func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = call @p(%0)
  %3 = call @p(%1)
  %4 = sub i32 %2, %3
  ret %4
}";
    must_bless(
        "p(a) - p(b) against itself",
        prove_equivalent(&m(f), &m(f), &cfg),
    );

    // And swapping which result is subtracted from which is a real difference.
    let swapped = "\
func @p(%0: i32) -> i32

func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = call @p(%0)
  %3 = call @p(%1)
  %4 = sub i32 %3, %2
  ret %4
}";
    must_not_bless(
        "p(a) - p(b) against p(b) - p(a)",
        prove_equivalent(&m(f), &m(swapped), &cfg),
    );
}

/// **A discarded result must not shift the matching** — the defect the by-name ordinal had.
/// Now with the pair that *should* be blessed, so the fix is not just "refuse everything".
#[test]
fn a_discarded_result_does_not_shift_the_matching() {
    let Some(cfg) = cfg() else { return };
    let f = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @p(1i32)
  %1 = call @p(2i32)
  ret %1
}";
    must_bless(
        "returning p(2) against itself",
        prove_equivalent(&m(f), &m(f), &cfg),
    );

    // The fixture that was blessed and should not have been, kept here beside its twin.
    let moved = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = call @p(1i32)
  call @p(2i32)
  ret %1
}";
    must_not_bless(
        "returning p(2) against returning p(1)",
        prove_equivalent(&m(f), &m(moved), &cfg),
    );
}

/// **A pure callee is matched by its arguments, not waved through.**
///
/// `pure` says no side effects. It says nothing about the return value being independent of
/// the arguments — `abs` is pure — so a pure call's arguments must be compared before its
/// return is linked to anything.
#[test]
fn a_pure_callee_is_matched_by_its_arguments() {
    let Some(cfg) = cfg() else { return };
    let arg = |e: &str| {
        format!(
            "\
func @p(%0: i32) -> i32 pure

func @f(%0: i32) -> i32 {{
entry:
  .line 1
{e}
  %2 = call @p(%1)
  ret %2
}}"
        )
    };
    must_bless(
        "pure p(x + 0) against pure p(x + 0)",
        prove_equivalent(
            &m(&arg("  %1 = add i32 %0, 0i32")),
            &m(&arg("  %1 = use i32 %0")),
            &cfg,
        ),
    );
    must_not_bless(
        "pure p(x) against pure p(x + 1)",
        prove_equivalent(
            &m(&arg("  %1 = add i32 %0, 0i32")),
            &m(&arg("  %1 = add i32 %0, 1i32")),
            &cfg,
        ),
    );
}

/// **Reordering two calls to a pure function is not a divergence** — contract 5, and the false
/// `Differs` the by-name ordinal produced.
#[test]
fn reordering_two_pure_calls_is_still_not_a_divergence() {
    let Some(cfg) = cfg() else { return };
    let ab = "\
func @p(%0: i32) -> i32 pure

func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = call @p(%0)
  %3 = call @p(%1)
  %4 = sub i32 %2, %3
  ret %4
}";
    let ba = "\
func @p(%0: i32) -> i32 pure

func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %3 = call @p(%1)
  %2 = call @p(%0)
  %4 = sub i32 %2, %3
  ret %4
}";
    if let Equivalence::Differs { input, .. } = prove_equivalent(&m(ab), &m(ba), &cfg) {
        panic!("these compute the same value; a Differs is fabricated, witness {input:?}");
    }
}
