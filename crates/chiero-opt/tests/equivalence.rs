//! `prove_equivalent` — 041 contracts 1, 2, 3, 4, 13 and 13b.
//!
//! **Contract 13b is the reason this file is shaped the way it is:**
//!
//! > "An always-`Unknown` implementation must fail this suite. Contracts 1, 2, 3, 5 and 7
//! > each require a definite `Equivalent`, and 3, 4, 6, 7 and 8 each require a definite
//! > `Differs` with a witness — so neither degenerate answer passes."
//!
//! Every assertion below is therefore for a *definite* verdict. `Unknown` is the answer
//! `prove_equivalent` is allowed to give when it does not know, and it is precisely what
//! this suite exists to refuse to accept.
//!
//! **Fixtures are `.cir`, not C.** `chiero-opt` is a vertical and 001 §4 rule 7 forbids it
//! a frontend dependency, dev-dependencies included — the same reason `passes.rs` gives.

use chiero_cir::Module;
use chiero_exec::{Fidelity, Witness};
use chiero_opt::{Claim, Divergence, EquivCfg, Equivalence, prove_equivalent};

/// One function named `f`, wrapped in the minimum module around it.
fn m(body: &str) -> Module {
    let text = format!("target x86_64-unknown-linux-gnu\n\n{body}\n");
    chiero_cir::text::parse(&text)
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{text}"))
}

/// The comparison every contract here wants: a discovered backend, default budget.
///
/// With no backend installed this is tier 1 alone, and the arithmetic identities below are
/// outside what 022 §3.2 lets tier 1 answer `Unsat` over. Rather than assert a verdict the
/// machine cannot reach, each test returns early — the established pattern in
/// `chiero-exec`'s own suite (`arenas.rs`, `checkers.rs`).
fn cfg() -> Option<EquivCfg> {
    let c = EquivCfg::new("f");
    c.backend.is_some().then_some(c)
}

/// The signed value of the first parameter in a witness.
fn first_param_signed(w: &Witness) -> i64 {
    let b = w
        .bindings
        .iter()
        .find(|b| matches!(b.origin, chiero_exec::InputOrigin::Param { index: 0, .. }))
        .expect("a distinguishing input names the parameter that distinguishes");
    let bits = b.value as u64;
    match b.width {
        32 => bits as u32 as i32 as i64,
        64 => bits as i64,
        w => panic!("unexpected width {w}"),
    }
}

const SUM: &str = "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %0, %1
  ret %2
}";

/// **Contract 1: a function and its verbatim copy are `Equivalent` with `Exact`.**
#[test]
fn a_verbatim_copy_is_exactly_equivalent() {
    let Some(cfg) = cfg() else { return };
    let v = prove_equivalent(&m(SUM), &m(SUM), &cfg);
    match v {
        Equivalence::Equivalent {
            fidelity,
            footprint,
            ..
        } => {
            assert_eq!(
                fidelity,
                Fidelity::Exact,
                "a copy of itself is a proof, not a bound"
            );
            assert!(
                footprint.compared.contains(&Claim::ReturnValue),
                "the footprint must say what was compared: {footprint:?}"
            );
        }
        other => panic!("contract 1 wants a definite Equivalent, got {other:?}"),
    }
}

/// **Contract 2: `return a + b;` vs `return b + a;` is `Equivalent` with `Exact`.**
#[test]
fn commuted_addition_is_exactly_equivalent() {
    let Some(cfg) = cfg() else { return };
    let after = "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %1, %0
  ret %2
}";
    match prove_equivalent(&m(SUM), &m(after), &cfg) {
        Equivalence::Equivalent { fidelity, .. } => assert_eq!(fidelity, Fidelity::Exact),
        other => panic!("contract 2 wants a definite Equivalent, got {other:?}"),
    }
}

const MUL2: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = mul i32 %0, 2i32
  ret %1
}";

const SHL1: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = shl i32 %0, 1i32
  ret %1
}";

const DIV2: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = sdiv i32 %0, 2i32
  ret %1
}";

const ASHR1: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = ashr i32 %0, 1i32
  ret %1
}";

/// **Contract 3, first half: `x * 2` vs `x << 1` on a signed 32-bit input is `Equivalent`
/// with `Exact`.**
#[test]
fn times_two_is_a_left_shift() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(MUL2), &m(SHL1), &cfg) {
        Equivalence::Equivalent { fidelity, .. } => assert_eq!(fidelity, Fidelity::Exact),
        other => panic!("contract 3 wants a definite Equivalent for `x * 2`, got {other:?}"),
    }
}

/// **Contract 3, second half: `x / 2` vs `x >> 1` is `Differs`, with a *negative*
/// distinguishing input.**
///
/// The sign is the whole content of the contract. Both agree on every non-negative input,
/// so a witness of `0` would be a `Differs` that does not distinguish anything — which is
/// the failure mode 041 §1.3 is about ("here is the program" ends the discussion).
#[test]
fn divide_by_two_is_not_an_arithmetic_shift_and_the_witness_is_negative() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(DIV2), &m(ASHR1), &cfg) {
        Equivalence::Differs {
            input, observation, ..
        } => {
            let x = first_param_signed(&input);
            assert!(x < 0, "the distinguishing input must be negative, got {x}");
            // And it must actually distinguish: C rounds `/` toward zero, `>>` toward -inf.
            let (q, s) = (x as i32 / 2, (x as i32) >> 1);
            assert_ne!(q, s, "witness {x} does not distinguish the two");
            match observation {
                Divergence::ReturnValue { before, after } => {
                    assert_eq!(before.bits() as u32 as i32, q, "before is `x / 2`");
                    assert_eq!(after.bits() as u32 as i32, s, "after is `x >> 1`");
                }
                other => panic!("the divergence is in the return value, got {other:?}"),
            }
        }
        other => panic!("contract 3 wants a definite Differs for `x / 2`, got {other:?}"),
    }
}

/// `int f(int x) { return x < 0 ? -x : x; }` — the naive absolute value.
const ABS_NAIVE: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp slt i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  %2 = sub i32 0i32, %0
  ret %2
bb2:
  .line 3
  ret %0
}";

/// The same function written to saturate instead of wrapping at `INT_MIN`.
const ABS_SATURATING: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp slt i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  %2 = cmp eq i32 %0, -2147483648i32
  br %2, bb3, bb4
bb3:
  .line 3
  ret 2147483647i32
bb4:
  .line 4
  %3 = sub i32 0i32, %0
  ret %3
bb2:
  .line 5
  ret %0
}";

/// **Contract 4: `abs(x)` written two ways is `Differs` at `INT_MIN`, with that witness.**
///
/// The two agree on all 2^32 - 1 other inputs, which is what makes this the contract worth
/// having: an adjudicator that answers by sampling, or by trusting that two functions
/// which look alike are alike, passes contracts 1 and 2 and fails this one.
#[test]
fn two_absolute_values_differ_only_at_int_min() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(ABS_NAIVE), &m(ABS_SATURATING), &cfg) {
        Equivalence::Differs {
            input, observation, ..
        } => {
            assert_eq!(
                first_param_signed(&input),
                i32::MIN as i64,
                "INT_MIN is the only input that distinguishes these"
            );
            match observation {
                Divergence::ReturnValue { before, after } => {
                    assert_eq!(before.bits() as u32 as i32, i32::MIN);
                    assert_eq!(after.bits() as u32 as i32, i32::MAX);
                }
                other => panic!("the divergence is in the return value, got {other:?}"),
            }
        }
        other => panic!("contract 4 wants a definite Differs, got {other:?}"),
    }
}

/// **Contract 13: `prove_equivalent` is symmetric — swapping the arguments yields the same
/// verdict and a correspondingly swapped witness.**
#[test]
fn the_verdict_is_symmetric_and_the_witness_swaps_with_it() {
    let Some(cfg) = cfg() else { return };

    // The agreeing direction.
    let fwd = prove_equivalent(&m(MUL2), &m(SHL1), &cfg);
    let rev = prove_equivalent(&m(SHL1), &m(MUL2), &cfg);
    match (&fwd, &rev) {
        (
            Equivalence::Equivalent { fidelity: a, .. },
            Equivalence::Equivalent { fidelity: b, .. },
        ) => assert_eq!(a, b),
        _ => panic!("equivalence must not depend on argument order: {fwd:?} vs {rev:?}"),
    }

    // The disagreeing direction, where the swap is observable: same input, same two
    // numbers, changing places.
    let fwd = prove_equivalent(&m(DIV2), &m(ASHR1), &cfg);
    let rev = prove_equivalent(&m(ASHR1), &m(DIV2), &cfg);
    match (fwd, rev) {
        (
            Equivalence::Differs {
                input: i1,
                observation:
                    Divergence::ReturnValue {
                        before: b1,
                        after: a1,
                    },
                ..
            },
            Equivalence::Differs {
                input: i2,
                observation:
                    Divergence::ReturnValue {
                        before: b2,
                        after: a2,
                    },
                ..
            },
        ) => {
            assert_eq!(
                first_param_signed(&i1),
                first_param_signed(&i2),
                "the same distinguishing input, whichever way round it is asked"
            );
            assert_eq!(b1, a2, "before and after swap with the arguments");
            assert_eq!(a1, b2, "before and after swap with the arguments");
        }
        (fwd, rev) => panic!("both directions must Differ: {fwd:?} vs {rev:?}"),
    }
}

/// **Contract 13b, made mechanical: no verdict in this suite may be `Unknown`.**
///
/// The contract's own words are about the suite as a whole, and a suite can lose that
/// property one `#[test]` at a time without anyone noticing. This asserts it over the
/// comparisons directly, so an implementation that starts answering `Unknown` on an input
/// it used to decide fails here with the count, rather than by whichever test happened to
/// go first.
#[test]
fn no_contract_in_this_suite_is_answered_with_unknown() {
    let Some(cfg) = cfg() else { return };
    let cases: &[(&str, &str, &str)] = &[
        ("verbatim copy", SUM, SUM),
        (
            "commuted add",
            SUM,
            "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %1, %0
  ret %2
}",
        ),
        ("x*2 vs x<<1", MUL2, SHL1),
        ("x/2 vs x>>1", DIV2, ASHR1),
        ("abs two ways", ABS_NAIVE, ABS_SATURATING),
    ];
    let unknown: Vec<&str> = cases
        .iter()
        .filter(|(_, b, a)| {
            matches!(
                prove_equivalent(&m(b), &m(a), &cfg),
                Equivalence::Unknown { .. }
            )
        })
        .map(|(name, _, _)| *name)
        .collect();
    assert!(
        unknown.is_empty(),
        "contract 13b: an always-Unknown implementation must fail this suite; \
         {} of {} cases are Unknown: {unknown:?}",
        unknown.len(),
        cases.len()
    );
}

/// A loop whose trip count is an input — so the bound the engine applies is chiero's, not
/// the program's. `int f(int n) { int i = 0; while (i < n) i++; return i; }`
const LOOP_TO_N: &str = "\
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
  ret %1
}";

/// **Contract 9: a function with an unbounded loop returns `Equivalent { fidelity: Bounded }`
/// and the bound is stated.**
///
/// §1.2: "for a function with an unbounded loop, the result is `Equivalent { fidelity:
/// Bounded }` — a statement about inputs within the bound, not a proof."
///
/// The other half of the contract — that 032 §3.1 does not accept it — is already asserted
/// in `chiero-select`'s `refinement.rs`, over the seam rather than over this producer.
#[test]
fn an_unbounded_loop_is_equivalent_only_within_the_bound() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(LOOP_TO_N), &m(LOOP_TO_N), &cfg) {
        Equivalence::Equivalent {
            fidelity,
            assumptions,
            ..
        } => {
            assert_eq!(
                fidelity,
                Fidelity::Bounded,
                "a loop chiero cut is not a proof over all inputs"
            );
            assert!(
                !assumptions.is_empty(),
                "§1.2 requires the bound be stated, not merely the fidelity lowered"
            );
        }
        other => panic!("contract 9 wants Equivalent with a bound, got {other:?}"),
    }
}

/// **A comparison that examined no paths is `Unknown`, never `Equivalent`.**
///
/// Not a numbered contract — a hole found by asking what the implementation does when the
/// pairing loop has nothing to iterate over. "No pair disagreed" is the same sentence
/// whether every pair agreed or there were no pairs, and only one of those is a proof.
/// This is the project's recurring flattering-failure shape: silence read as success.
#[test]
fn a_comparison_with_no_paths_is_not_a_proof() {
    let Some(mut cfg) = cfg() else { return };
    cfg.budget.max_states = 0;
    cfg.budget.max_depth = 0;
    match prove_equivalent(&m(SUM), &m(SUM), &cfg) {
        Equivalence::Unknown { reason } => assert!(
            reason.contains("path"),
            "the reason must say what was missing, got {reason:?}"
        ),
        other => panic!("a comparison with no paths must not be a verdict, got {other:?}"),
    }
}
