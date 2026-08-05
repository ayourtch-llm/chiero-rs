//! **The observable side-effect sequence** — 020 §4.2, and what
//! [041 §1.1](../../../docs/specs/041-optimization-analysis.md) needs from it.
//!
//! 041 §1.1 makes observational equivalence three claims, and the third is:
//!
//! > the ordered sequence of observable side effects: **calls to unmodeled or effectful
//! > externs with their arguments**, volatile accesses, and abnormal termination.
//!
//! and says, of the choices that change the answer:
//!
//! > the order of two independent extern calls **is** observable (C fixes it, and reordering
//! > visible I/O is not a safe refactor).
//!
//! Today `State::effects` records only volatile stores, so a rewrite that swaps two `printf`
//! calls is invisible to it — 041 contract 6 is unreachable, not merely unimplemented.
//!
//! # Why the arguments, and not just the names
//!
//! Contract 6's rewrite swaps two calls **to the same function**. A sequence of callee names
//! is identical before and after, so a comparison built on names alone would report those two
//! programs equivalent — and would do it while looking like it had checked. The arguments are
//! the only thing that distinguishes `printf("a")` from `printf("b")`, which makes them the
//! load-bearing half rather than a detail.
//!
//! # What counts as effectful
//!
//! `FnAttrs::no_side_effects`, defaulting to false. A function chiero has never seen the body
//! of is assumed to do something, which is the conservative direction: over-recording makes a
//! safe refactor look unsafe, under-recording blesses a rewrite that reordered I/O.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;

fn m(text: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{text}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}"))
}

fn run(module: &Module, entry: &str) -> (RunResult, TermArena) {
    let mut a = TermArena::new();
    let r = Engine::new(module).with_entry(entry).run(&mut a);
    (r, a)
}

/// The effect sequence of the single path, as `(kind, detail)` pairs.
fn effects(r: &RunResult) -> Vec<(EffectKind, String)> {
    let terminated: Vec<&State> = r
        .states()
        .iter()
        .filter(|s| matches!(s.status, Status::Terminated(TermReason::Return)))
        .collect();
    assert_eq!(terminated.len(), 1, "these fixtures are straight-line");
    terminated[0]
        .effects()
        .iter()
        .map(|e| (e.kind, e.detail.clone()))
        .collect()
}

/// Two calls to externs with no body: both are observable, in program order.
const TWO_CALLS: &str = "\
func @sink(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @sink(1i32)
  call @sink(2i32)
  ret %0
}";

/// **A call to an extern with no body is an observable effect.**
#[test]
fn an_unmodeled_extern_call_lands_in_the_effect_sequence() {
    let module = m(TWO_CALLS);
    let (r, _) = run(&module, "f");
    let es = effects(&r);
    assert_eq!(
        es.len(),
        2,
        "two calls, two effects — got {es:?}; 041 §1.1 counts extern calls as observable"
    );
    for (kind, detail) in &es {
        assert_eq!(*kind, EffectKind::Call);
        assert_eq!(detail, "sink", "the effect names the callee");
    }
}

/// **The arguments are recorded, because contract 6's two calls have the same name.**
#[test]
fn the_effect_carries_the_arguments_that_distinguish_two_calls_to_one_function() {
    let module = m(TWO_CALLS);
    let (r, a) = run(&module, "f");
    let terminated: Vec<&State> = r
        .states()
        .iter()
        .filter(|s| matches!(s.status, Status::Terminated(TermReason::Return)))
        .collect();
    let es = terminated[0].effects();
    assert_eq!(es.len(), 2);

    let arg = |e: &Effect| -> u128 {
        assert_eq!(e.args.len(), 1, "one argument each: {:?}", e.args);
        match e.args[0] {
            Some(Value::Scalar(t)) => a
                .eval_ground(t)
                .expect("a literal argument is ground")
                .bits(),
            ref other => panic!("expected a scalar argument, got {other:?}"),
        }
    };
    assert_eq!(arg(&es[0]), 1, "first call passes 1");
    assert_eq!(arg(&es[1]), 2, "second call passes 2");
}

/// **Order is preserved**, which is the whole reason the sequence is a sequence: the swapped
/// program produces the swapped sequence, and nothing else about it differs.
#[test]
fn swapping_two_calls_swaps_the_sequence() {
    let swapped = "\
func @sink(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @sink(2i32)
  call @sink(1i32)
  ret %0
}";
    let (a_mod, b_mod) = (m(TWO_CALLS), m(swapped));
    let (ra, mut aa) = run(&a_mod, "f");
    let (rb, mut ab) = run(&b_mod, "f");

    let firsts = |r: &RunResult, arena: &mut TermArena| -> Vec<u128> {
        r.states()
            .iter()
            .find(|s| matches!(s.status, Status::Terminated(TermReason::Return)))
            .expect("one returning path")
            .effects()
            .iter()
            .map(|e| match e.args[0] {
                Some(Value::Scalar(t)) => arena.eval_ground(t).expect("ground").bits(),
                _ => panic!("scalar"),
            })
            .collect()
    };
    assert_eq!(firsts(&ra, &mut aa), vec![1, 2]);
    assert_eq!(firsts(&rb, &mut ab), vec![2, 1]);
}

/// **A function chiero has the body of is not an extern**, so calling it is not itself an
/// effect — whatever the body does is recorded by the body.
///
/// Without this the sequence would grow an entry per call in the program and stop being a
/// record of what is *observable from outside*, which is the only thing it is for.
#[test]
fn calling_a_function_with_a_body_is_not_an_observable_effect() {
    let module = m("\
func @inner(%0: i32) -> i32 {
entry:
  .line 1
  ret %0
}

func @f(%0: i32) -> i32 {
entry:
  .line 4
  %1 = call @inner(%0)
  ret %1
}");
    let (r, _) = run(&module, "f");
    assert!(
        effects(&r).is_empty(),
        "a call chiero can see through is not an external event: {:?}",
        effects(&r)
    );
}

/// **`no_side_effects` is honoured — as a *kind*, not as an absence.**
///
/// A pure declared call is not an observable event, which is what makes the sequence usable on
/// real code: VPP's headers are full of them. It is nonetheless *in* the sequence, as
/// `EffectKind::PureCall`, because that is what gives every declared call one stable ordinal
/// for `InputOrigin::ExternReturn` to name (041 §1.2). A numbering that skipped pure calls
/// would have two runs counting different things the moment one of them dropped a call.
///
/// So the contract is sharper than "not recorded": a consumer looking for observable effects
/// filters this kind out, and a consumer matching calls between two runs must not. This test
/// asserts both halves — it used to assert the sequence was empty, which was the same claim
/// while there was only one kind in it.
#[test]
fn a_declared_function_marked_pure_is_not_an_observable_effect() {
    let module = m("\
func @clean(%0: i32) -> i32 pure

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = call @clean(%0)
  ret %1
}");
    let (r, _) = run(&module, "f");
    let es = effects(&r);
    assert_eq!(
        es,
        vec![(EffectKind::PureCall, "clean".to_string())],
        "the call is in the sequence, for its ordinal"
    );
    assert!(
        es.iter().all(|(k, _)| *k != EffectKind::Call),
        "and none of it is observable: {es:?}"
    );
}
