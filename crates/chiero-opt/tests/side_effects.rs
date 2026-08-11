//! **041 contract 6 — the ordered side-effect sequence.**
//!
//! > 6. A rewrite that changes the order of two `printf` calls is `Differs` with
//! >    `Divergence::SideEffect`.
//!
//! §1.1 states the choice this rests on, because it is a choice:
//!
//! > the order of two independent extern calls **is** observable (C fixes it, and reordering
//! > visible I/O is not a safe refactor).
//!
//! Until now `prove_equivalent` refused any function calling a body-less non-pure function —
//! a correct refusal and the wrong long-term answer, since those are most of the functions
//! anyone wants adjudicated.
//!
//! **Two calls to the same function is the case that matters.** A comparison built on callee
//! names would find the sequences identical before and after a swap and report the two
//! programs equivalent, while looking like it had checked. The arguments are the whole test.

use chiero_cir::Module;
use chiero_exec::Fidelity;
use chiero_opt::{Claim, Divergence, EquivCfg, Equivalence, prove_equivalent};

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

/// **The solver guard, and it says when it fires.**
///
/// Every caller is `let Some(cfg) = cfg() else { return }` — which returned in silence until
/// 2026-08-11, so on the solverless leg these tests reported `ok` having asserted nothing and
/// `check.sh`'s skip counter could not see them. 54 returns across the suite were invisible that
/// way, against 103 that announced. The message belongs here rather than at each call site
/// because this is what knows *why*.
fn cfg() -> Option<EquivCfg> {
    let c = EquivCfg::new("f");
    if c.backend.is_none() {
        eprintln!(
            "skipping a solver-dependent assertion in side_effects.rs: no SMT-LIB backend on PATH \
             (022 contract 2)"
        );
        return None;
    }
    Some(c)
}

fn prints(order: &str) -> String {
    format!(
        "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {{
entry:
  .line 1
{order}
  ret %0
}}"
    )
}

const AB: &str = "  call @p(1i32)\n  call @p(2i32)";
const BA: &str = "  call @p(2i32)\n  call @p(1i32)";

/// **Contract 6.** Same calls, same count, same callee — different order.
#[test]
fn swapping_two_calls_to_one_function_is_a_divergence() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(&prints(AB)), &m(&prints(BA)), &cfg) {
        Equivalence::Differs { observation, .. } => match observation {
            Divergence::SideEffect {
                index,
                before,
                after,
            } => {
                assert_eq!(index, 0, "the first call is where they part company");
                assert!(
                    before.is_some() && after.is_some(),
                    "both sides made a call here; the difference is in it"
                );
            }
            other => panic!("the difference is in the effect sequence, got {other:?}"),
        },
        other => panic!("contract 6 wants a definite Differs, got {other:?}"),
    }
}

/// **And the same order is blessed**, with `SideEffects` named among what was compared —
/// otherwise the operation has simply traded one refusal for another.
#[test]
fn the_same_calls_in_the_same_order_are_equivalent() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(&prints(AB)), &m(&prints(AB)), &cfg) {
        Equivalence::Equivalent {
            fidelity,
            footprint,
            ..
        } => {
            // **`Approximated`, not `Exact`, and that is the honest answer.** The engine
            // cannot say what `p` did — it has no body and no model — so each run on its own
            // is a lie about semantics (023 §7). What the *relational* comparison can say is
            // that both versions called the same function with the same arguments in the
            // same order, so whatever `p` did, it did to both. That licenses the verdict and
            // not the fidelity: 032 §3.1 still refuses to drop a test on this, which is
            // right, because nothing here proved anything about `p`.
            assert_eq!(fidelity, Fidelity::Approximated);
            assert!(
                footprint.compared.contains(&Claim::SideEffects),
                "the sequence was compared and must be claimed: {footprint:?}"
            );
        }
        other => panic!("identical call sequences agree: {other:?}"),
    }
}

/// **A symbolic argument is compared symbolically**, which is the only comparison worth
/// having: two calls agree when they agree for *every* input, and no pair of concrete values
/// says that.
#[test]
fn arguments_are_compared_for_all_inputs_not_for_one() {
    let Some(cfg) = cfg() else { return };
    // `p(x + 0)` and `p(x)` pass the same value for every x.
    let plain = "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @p(%0)
  ret %0
}";
    let rewritten = "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = add i32 %0, 0i32
  call @p(%1)
  ret %0
}";
    assert!(
        matches!(
            prove_equivalent(&m(plain), &m(rewritten), &cfg),
            Equivalence::Equivalent { .. }
        ),
        "x + 0 is x for every x"
    );

    // `p(x + 1)` does not, and the witness must be an input — any input — since they differ
    // at all of them.
    let bumped = "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = add i32 %0, 1i32
  call @p(%1)
  ret %0
}";
    match prove_equivalent(&m(plain), &m(bumped), &cfg) {
        Equivalence::Differs { observation, .. } => assert!(
            matches!(observation, Divergence::SideEffect { .. }),
            "a different argument is a different call: {observation:?}"
        ),
        other => panic!("p(x) and p(x+1) are different calls: {other:?}"),
    }
}

/// **Dropping a call is a divergence, and `after` says nothing happened there.**
///
/// `None` on one side is how a length difference is expressed — 041's own
/// `SideEffect { index, before: Option<Effect>, after: Option<Effect> }` shape.
#[test]
fn dropping_a_call_leaves_a_hole_the_divergence_names() {
    let Some(cfg) = cfg() else { return };
    let one = prints("  call @p(1i32)");
    match prove_equivalent(&m(&prints(AB)), &m(&one), &cfg) {
        Equivalence::Differs { observation, .. } => match observation {
            Divergence::SideEffect {
                index,
                before,
                after,
            } => {
                assert_eq!(index, 1, "the second call is the one that vanished");
                assert!(before.is_some(), "the before version made it");
                assert_eq!(after, None, "and the after version made nothing there");
            }
            other => panic!("got {other:?}"),
        },
        other => panic!("a dropped call is a divergence: {other:?}"),
    }
}

/// **A pure declaration is still not an event.** The relaxation must not turn every call into
/// an observable, or the operation becomes useless on real code — VPP's headers are full of
/// them.
///
/// The callee returns `void` deliberately. A pure call returning a *value* mints an
/// `InputOrigin::ExternReturn` on the side that makes it and nothing on the side that does
/// not, and an input with no counterpart is a refusal — correctly, since matching inputs
/// pairwise is what makes the two runs about the same program. Recognising that an
/// unused pure result may be dropped is a separate piece of reasoning this does not do, and
/// smuggling it into a test about effects would hide which of the two failed.
#[test]
fn a_pure_extern_is_still_invisible() {
    let Some(cfg) = cfg() else { return };
    let calls = "\
func @clean(%0: i32) -> void pure

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @clean(%0)
  ret %0
}";
    let plain = "\
func @clean(%0: i32) -> void pure

func @f(%0: i32) -> i32 {
entry:
  .line 1
  ret %0
}";
    let v = prove_equivalent(&m(calls), &m(plain), &cfg);
    assert!(
        matches!(v, Equivalence::Equivalent { .. }),
        "dropping a call to a pure function changes nothing observable: {v:?}"
    );
}
