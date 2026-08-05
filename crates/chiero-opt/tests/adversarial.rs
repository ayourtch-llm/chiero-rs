//! **What an adversarial review found in `prove_equivalent` on the day it was written.**
//!
//! Every fixture here was constructed by a reviewer asked one question — *where would this
//! report `Equivalent` for two functions that are not equivalent, or hand back a witness that
//! does not distinguish them* — and every one was confirmed against the real crate before it
//! was written down. They are kept together because they share a cause worth naming: each is
//! a place where **chiero's own limits leaked into a claim about the program**, and each was
//! invisible to the contract suite because the contract fixtures are pure, one-parameter,
//! branch-light arithmetic.
//!
//! The worst of them was a paragraph. The module documentation said a comparison that would
//! have to reason about caller-visible memory or a side-effect sequence answers `Unknown`
//! naming the claim — and nothing implemented that sentence. A written intention that the
//! code does not have is worse than an acknowledged gap, because it is what a reader checks
//! instead of the code.

use chiero_cir::Module;
use chiero_exec::Fidelity;
use chiero_opt::{Divergence, EquivCfg, Equivalence, prove_equivalent};

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn cfg() -> Option<EquivCfg> {
    let c = EquivCfg::new("f");
    c.backend.is_some().then_some(c)
}

/// `Equivalent` is the only verdict that licenses anything (032 §3.1, 050 contract 8), so
/// "not equivalent" is the assertion these fixtures share. Whether the right answer is
/// `Differs` or an honest `Unknown` depends on how much is built; blessing them is wrong
/// under either.
#[track_caller]
fn must_not_bless(what: &str, v: Equivalence) {
    if let Equivalence::Equivalent { fidelity, .. } = &v {
        panic!("{what}: blessed with fidelity {fidelity:?} — {v:?}");
    }
}

const RETURNS_ZERO: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  ret 0i32
}";

/// **A store to a global is caller-visible memory** — §1.1's claim 2 names globals first.
#[test]
fn writing_a_global_is_not_the_same_as_not_writing_it() {
    let Some(cfg) = cfg() else { return };
    let writes = "\
global @g : size 4 align 4 bytes 00000000

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = addrglobal @g
  store i32 %0 -> %1 align 4
  ret 0i32
}";
    must_not_bless(
        "a global store against no store",
        prove_equivalent(&m(writes), &m(RETURNS_ZERO), &cfg),
    );
}

/// **A volatile store is an observable event in program order** (020 §4.2), and dropping one
/// is not a refactor — it is deleting a device write.
#[test]
fn dropping_a_volatile_store_is_not_a_refactor() {
    let Some(cfg) = cfg() else { return };
    let writes = "\
global @g : size 4 align 4 bytes 00000000

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = addrglobal @g
  storevolatile i32 %0 -> %1 align 4
  ret 0i32
}";
    must_not_bless(
        "a volatile store against no store",
        prove_equivalent(&m(writes), &m(RETURNS_ZERO), &cfg),
    );
}

/// **§1.1's third claim.** A call to an extern with no body does something chiero cannot see;
/// removing it changes the program even when the return value is untouched.
///
/// This one also carried a second defect: the run degrades to `Approximated` for the
/// unmodeled call, and the verdict passed that straight through as `Equivalent`. §1.2 gives
/// `Equivalent` two fidelities, `Exact` and `Bounded`; `Approximated` is 023 §7's phrase for
/// *a deliberate lie about semantics*, which is not a thing to build a blessing on.
#[test]
fn removing_a_call_to_an_unmodeled_extern_is_not_a_refactor() {
    let Some(cfg) = cfg() else { return };
    let calls = "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @p(%0)
  ret 0i32
}";
    must_not_bless(
        "a dropped extern call",
        prove_equivalent(&m(calls), &m(RETURNS_ZERO), &cfg),
    );
}

/// `int f(int x, int y) { return x + y; }`
const SUM2: &str = "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %0, %1
  ret %2
}";

/// The same, but unreachable at exactly two points: `(0, 200)` and `(3, 7)`.
///
/// Two points, deliberately: the divergence set is **not a product**, which is what breaks a
/// minimizer that fixes inputs one at a time from a model taken before any of them were
/// fixed. `x` minimizes to 0 via `(0, 200)`; `y` then starts from the stale 7.
const SUM2_UNREACHABLE_AT_TWO_POINTS: &str = "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %0, %1
  %3 = cmp eq i32 %0, 0i32
  %4 = cmp eq i32 %1, 200i32
  %5 = and i1 %3, %4
  %6 = cmp eq i32 %0, 3i32
  %7 = cmp eq i32 %1, 7i32
  %8 = and i1 %6, %7
  %9 = or i1 %5, %8
  br %9, bbu, bbr
bbu:
  .line 2
  unreachable
bbr:
  .line 3
  ret %2
}";

/// **A `Differs` must produce an input that actually differs** — 041 contract 10.
///
/// The reported witness was `(0, 7)`, at which both versions return 32. A distinguishing
/// input that does not distinguish is the precise thing §1.3 exists to prevent: *"your rewrite
/// returns 0 where the original returns -1 when `n == INT_MIN`, here is the program" ends the
/// discussion* — and it ends it in the wrong place if the program does not reproduce.
#[test]
fn a_termination_witness_must_reproduce_the_termination() {
    let Some(cfg) = cfg() else { return };
    match prove_equivalent(&m(SUM2), &m(SUM2_UNREACHABLE_AT_TWO_POINTS), &cfg) {
        Equivalence::Differs {
            input, observation, ..
        } => {
            assert!(
                matches!(observation, Divergence::Termination { .. }),
                "the difference is where the path ends: {observation:?}"
            );
            let v: Vec<i64> = input
                .bindings
                .iter()
                .map(|b| ((b.value as u64) as u32) as i32 as i64)
                .collect();
            assert_eq!(v.len(), 2, "two parameters, two bindings: {input:?}");
            let (x, y) = (v[0], v[1]);
            assert!(
                (x, y) == (0, 200) || (x, y) == (3, 7),
                "({x}, {y}) is not a point at which these two differ — the only such \
                 points are (0, 200) and (3, 7)"
            );
        }
        other => panic!("these differ at two inputs; got {other:?}"),
    }
}

/// The same two points, as a difference in the *returned value* rather than in termination.
const SUM2_BUMPED_AT_TWO_POINTS: &str = "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %0, %1
  %3 = cmp eq i32 %0, 0i32
  %4 = cmp eq i32 %1, 200i32
  %5 = and i1 %3, %4
  %6 = cmp eq i32 %0, 3i32
  %7 = cmp eq i32 %1, 7i32
  %8 = and i1 %6, %7
  %9 = or i1 %5, %8
  br %9, bbu, bbr
bbu:
  .line 2
  %10 = add i32 %2, 1i32
  ret %10
bbr:
  .line 3
  ret %2
}";

/// **A pair the machine can decide must not come back `Unknown`** — contract 13b's substance.
///
/// The same stale-model defect took the other exit here: the re-solve at the minimized input
/// found it infeasible and the whole comparison was abandoned, throwing away a satisfying
/// model with a real distinguishing input already in hand.
#[test]
fn a_provable_difference_at_two_points_is_not_undecidable() {
    let Some(cfg) = cfg() else { return };
    for (name, a, b) in [
        ("forwards", SUM2, SUM2_BUMPED_AT_TWO_POINTS),
        ("backwards", SUM2_BUMPED_AT_TWO_POINTS, SUM2),
    ] {
        match prove_equivalent(&m(a), &m(b), &cfg) {
            Equivalence::Differs { input, .. } => {
                let v: Vec<i64> = input
                    .bindings
                    .iter()
                    .map(|b| ((b.value as u64) as u32) as i32 as i64)
                    .collect();
                assert!(
                    v == vec![0, 200] || v == vec![3, 7],
                    "{name}: {v:?} is not one of the two points at which these differ"
                );
            }
            other => panic!("{name}: these differ at exactly two inputs; got {other:?}"),
        }
    }
}

/// `x == 0 ? 1 : 2` and `x == 0 ? 1 : 3` — the same on one input, different on every other.
fn pick(other: &str) -> String {
    format!(
        "\
func @f(%0: i32) -> i32 {{
entry:
  .line 1
  %1 = cmp eq i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 2
  ret 1i32
bb2:
  .line 3
  ret {other}i32
}}"
    )
}

/// **A dropped fork is not a bound on inputs.**
///
/// When `max_forks` or `max_states` trips, the engine drops the sibling it did not walk and
/// degrades the survivor to `Bounded` — correctly, because that is all it can say. What is
/// wrong is reading it here as §1.2's `Bounded`, which means *"a statement about inputs
/// within the loop bound"*. These functions have no loop and disagree on 2^32 - 1 inputs;
/// there is no bound within which the statement is true.
///
/// The contract suite's one budget test uses `max_states = 0` — where *nothing* finishes and
/// the `examined == 0` guard fires. That is the single budget configuration that does not
/// exhibit this.
#[test]
fn a_search_truncated_by_forks_or_states_is_not_a_bounded_proof() {
    let Some(base) = cfg() else { return };
    let (a, b) = (m(&pick("2")), m(&pick("3")));

    let mut forks = base.clone();
    forks.budget.max_forks = 0;
    must_not_bless("max_forks = 0", prove_equivalent(&a, &b, &forks));

    let mut states = base.clone();
    states.budget.max_states = 1;
    must_not_bless("max_states = 1", prove_equivalent(&a, &b, &states));
}

/// **And the honest bound still works.** The fix above must not turn every budget into an
/// `Unknown`: a loop cut at `max_loop_iters` is exactly what §1.2's `Bounded` is for, and
/// contract 9 depends on it.
#[test]
fn a_loop_bound_is_still_a_bounded_blessing() {
    let Some(cfg) = cfg() else { return };
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
  ret %1
}";
    match prove_equivalent(&m(looping), &m(looping), &cfg) {
        Equivalence::Equivalent { fidelity, .. } => assert_eq!(fidelity, Fidelity::Bounded),
        other => panic!("contract 9's bound must survive the fix: {other:?}"),
    }
}

// =========================================================================================
// Second review, after contract 6 landed. Four more, three of them false `Equivalent`.
// =========================================================================================

/// **A global read moved across an unmodeled call.**
///
/// The `Approximated` blessing argued that if the effect sequences agree then "whatever the
/// callee did, it did to both". That is true of the call's *position in the sequence* and says
/// nothing about how the callee's writes interleave with the caller's **reads**. `tick`
/// increments `g` — the ordinary reason to call such a function — and one version returns the
/// value before, the other after.
///
/// The guard refused global *stores* and permitted global *loads*, which is the same defect
/// the first review found, one indirection further out.
#[test]
fn reading_a_global_across_an_unmodeled_call_is_not_the_same_either_side_of_it() {
    let Some(cfg) = cfg() else { return };
    let after_call = "\
global @g : size 4 align 4 bytes 00000000

func @tick(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @tick(%0)
  %1 = addrglobal @g
  %2 = load i32, %1 align 4
  ret %2
}";
    let before_call = "\
global @g : size 4 align 4 bytes 00000000

func @tick(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = addrglobal @g
  %2 = load i32, %1 align 4
  call @tick(%0)
  ret %2
}";
    must_not_bless(
        "a global read either side of a call that may write it",
        prove_equivalent(&m(after_call), &m(before_call), &cfg),
    );
}

/// **A pure extern called with a different argument.**
///
/// `pure` (`no_side_effects`) says the call has no side effects. It does not say the *return
/// value* is independent of the arguments — `abs` is pure. The extern-return linking equated
/// the nth call's return on each side unconditionally, which asserts `p(x) == p(x + 1)`.
#[test]
fn a_pure_extern_called_with_a_different_argument_is_not_the_same_call() {
    let Some(cfg) = cfg() else { return };
    let f = |arg: &str| {
        format!(
            "\
func @p(%0: i32) -> i32 pure

func @f(%0: i32) -> i32 {{
entry:
  .line 1
{arg}
  ret %2
}}"
        )
    };
    let plain = f("  %1 = add i32 %0, 0i32\n  %2 = call @p(%1)");
    let bumped = f("  %1 = add i32 %0, 1i32\n  %2 = call @p(%1)");
    must_not_bless(
        "p(x) against p(x + 1)",
        prove_equivalent(&m(&plain), &m(&bumped), &cfg),
    );
}

/// **A call whose result is discarded desynchronizes the ordinal.**
///
/// `InputOrigin::ExternReturn` is minted only for a call with a destination, so `inputs()`
/// counts *result-bearing* calls while `effects()` counts *all* calls. Keying the link by "the
/// nth call to `p`" therefore counts two different things: here the effect sequences match
/// position for position and the link still equates the before version's `p(2)` with the after
/// version's `p(1)`. The two versions return the results of different calls.
#[test]
fn a_discarded_result_must_not_shift_which_calls_are_matched() {
    let Some(cfg) = cfg() else { return };
    let discard_first = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @p(1i32)
  %1 = call @p(2i32)
  ret %1
}";
    let discard_second = "\
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
        prove_equivalent(&m(discard_first), &m(discard_second), &cfg),
    );
}

/// **And the same defect the other way: a false `Differs` with a witness that witnesses
/// nothing.**
///
/// Two calls to a pure extern, issued in the opposite order, subtracted. The values are the
/// same either way; the ordinal linking crossed them and produced `-1` against `1` at an input
/// where both versions return 0. Contract 5 (reordering independent statements is
/// `Equivalent`) and contract 10 (a `Differs` distinguishes) both broken by one defect.
#[test]
fn reordering_two_pure_calls_is_not_a_divergence() {
    let Some(cfg) = cfg() else { return };
    let order = |first: &str, second: &str| {
        format!(
            "\
func @p(%0: i32) -> i32 pure

func @f(%0: i32, %1: i32) -> i32 {{
entry:
  .line 1
  %2 = call @p({first})
  %3 = call @p({second})
  %4 = sub i32 %2, %3
  ret %4
}}"
        )
    };
    // Both compute p(a) - p(b); they differ only in which call is issued first.
    let ab = order("%0", "%1");
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
    // Equivalent or Unknown are both defensible here. A wrong `Differs` is not.
    if let Equivalence::Differs { input, .. } = prove_equivalent(&m(&ab), &m(ba), &cfg) {
        panic!("these compute the same thing; a Differs here is fabricated, witness {input:?}");
    }
}

/// **One function, two spellings.**
///
/// gcc emits the `__builtin_` spelling for everything it recognises, so an optimized
/// translation unit and its unoptimized original disagree on the name of every libc call they
/// share — which is exactly the before/after pair this operation is for. The effect comparison
/// string-matched the *unresolved* name and returned a definite `Differs` on two byte-identical
/// programs.
#[test]
fn the_builtin_spelling_of_a_function_is_the_same_function() {
    let Some(cfg) = cfg() else { return };
    let call = |name: &str| {
        format!(
            "\
func @{name}(%0: i32) -> void

func @f(%0: i32) -> i32 {{
entry:
  .line 1
  call @{name}(%0)
  ret %0
}}"
        )
    };
    if let Equivalence::Differs { observation, .. } =
        prove_equivalent(&m(&call("memset")), &m(&call("__builtin_memset")), &cfg)
    {
        panic!("the same call spelled two ways is not a divergence: {observation:?}");
    }
}
