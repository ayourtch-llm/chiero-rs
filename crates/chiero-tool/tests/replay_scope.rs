//! **What the replay harness may and may not adjudicate — the fourth review's findings.**
//!
//! 041 contract 11 exists to catch chiero being wrong:
//!
//! > "a divergence the harness fails to demonstrate is downgraded and flagged, never silently
//! > trusted."
//!
//! It was applied to divergences the harness **cannot see**. The harness compares two return
//! values at one input; `prove_equivalent` also reports `SideEffect`, `Termination` and
//! `Memory` divergences. For those the harness always "fails to demonstrate", so a true
//! finding was downgraded from `Exact` to `Approximated` with an assumption reading
//! *"chiero's semantics and this compiler do not agree here"* — a false statement, since the
//! compiler was never asked.
//!
//! **The contract inverted: built to catch chiero being wrong, punishing it for being right.**
//!
//! The rule these tests assert: a harness may only change a verdict about something it
//! measured, and it must refuse — visibly — everything else.

use chiero_cir::Module;
use chiero_tool::{Fidelity, ReplayPolicy, ReplaySources, prove_equivalent_with_replay};
use std::path::PathBuf;

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-scope-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn sources(tag: &str, before: &str, after: &str, entry: &str) -> ReplaySources {
    let d = dir(tag);
    let (b, a) = (d.join("before.c"), d.join("after.c"));
    std::fs::write(&b, before).expect("write");
    std::fs::write(&a, after).expect("write");
    ReplaySources {
        before: b,
        after: a,
        entry: entry.into(),
        scratch: d,
        flags: Vec::new(),
    }
}

fn cfg() -> Option<chiero_opt::EquivCfg> {
    let c = chiero_opt::EquivCfg::new("f");
    c.backend.is_some().then_some(c)
}

/// Two calls to one extern in opposite orders: a real `SideEffect` divergence, and one the
/// harness's return-value comparison cannot see.
const AB: &str = "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @p(1i32)
  call @p(2i32)
  ret %0
}";

const BA: &str = "\
func @p(%0: i32) -> void

func @f(%0: i32) -> i32 {
entry:
  .line 1
  call @p(2i32)
  call @p(1i32)
  ret %0
}";

/// **A divergence the harness cannot observe must not be downgraded by it.**
#[test]
fn a_side_effect_divergence_is_not_downgraded_by_a_return_value_harness() {
    let Some(cfg) = cfg() else { return };
    if chiero_replay::compiler().is_none() {
        return;
    }
    // The C the two modules correspond to: the same calls, in opposite orders. Both return x,
    // so any return-value comparison finds them equal.
    let src = |first: i32, second: i32| {
        format!(
            "#include <stdio.h>\nstatic void p (int v) {{ fprintf (stderr, \"%d\", v); }}\n\
             int f (int x) {{ p ({first}); p ({second}); return x; }}\n"
        )
    };
    let s = sources("sideeffect", &src(1, 2), &src(2, 1), "f");

    let plain = chiero_tool::prove_equivalent(&m(AB), &m(BA), &cfg);
    let with = prove_equivalent_with_replay(&m(AB), &m(BA), &cfg, Some(&s), ReplayPolicy::Run);
    let v: serde_json::Value = serde_json::from_str(&with.to_json()).expect("valid JSON");

    assert_eq!(
        v["result"]["verdict"], "differs",
        "the two versions call p in different orders: {v}"
    );
    assert_eq!(
        with.fidelity, plain.fidelity,
        "a harness that cannot see this divergence must not change the verdict about it: {v}"
    );
    assert!(
        !with
            .assumptions
            .iter()
            .any(|(k, _)| k == "harness_disagreed"),
        "the compiler was never asked about effect order: {:?}",
        with.assumptions
    );
    // And it must say why it could not check, rather than saying nothing.
    assert!(
        with.blind_spots
            .iter()
            .any(|b| b.contains("side-effect") || b.contains("return value")),
        "the refusal must name what the harness cannot measure: {:?}",
        with.blind_spots
    );
}

/// **A return-value divergence the harness *can* see still downgrades when it disagrees.**
///
/// The narrowing must not turn contract 11 off. Given C whose behaviour genuinely matches at
/// the witness while chiero claims otherwise, the downgrade is the whole point.
#[test]
fn a_return_value_divergence_the_compiler_denies_is_still_downgraded() {
    let Some(cfg) = cfg() else { return };
    if chiero_replay::compiler().is_none() {
        return;
    }
    // chiero compares `x * 2` against `x * 3` and reports a real ReturnValue divergence — but
    // the C given to the harness is two copies of the same function, so the compiler says they
    // agree. That is exactly the situation contract 11 is for.
    let same = "int f (int x) { return x * 2; }\n";
    let s = sources("agrees", same, same, "f");
    let double =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = mul i32 %0, 2i32\n  ret %1\n}";
    let triple =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = mul i32 %0, 3i32\n  ret %1\n}";

    let env =
        prove_equivalent_with_replay(&m(double), &m(triple), &cfg, Some(&s), ReplayPolicy::Run);
    assert_eq!(
        env.fidelity,
        Fidelity::Approximated,
        "the harness measured this and disagreed: {}",
        env.to_json()
    );
    assert!(
        env.assumptions
            .iter()
            .any(|(k, _)| k == "harness_disagreed"),
        "{:?}",
        env.assumptions
    );
}

/// **A witness the harness cannot render is a refusal, not a `DidNotBuild`.**
///
/// A pointer parameter mints no binding and an extern return mints one that is not an
/// argument, so rendering every binding positionally produces a call with the wrong arity —
/// which the compiler reports as an error about the harness. That reads as "the harness is
/// broken" when the truth is "this witness is not an argument list".
#[test]
fn a_witness_that_is_not_an_argument_list_is_refused_before_it_is_compiled() {
    let Some(cfg) = cfg() else { return };
    // `p` returns a value, so the witness carries an ExternReturn binding as well as the
    // parameter — two bindings for a one-parameter function.
    let before = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = call @p(%0)
  %2 = add i32 %1, 1i32
  ret %2
}";
    let after = "\
func @p(%0: i32) -> i32

func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = call @p(%0)
  %2 = add i32 %1, 2i32
  ret %2
}";
    let c = "int p (int);\nint f (int x) { return p (x) + 1; }\n";
    let s = sources("externret", c, c, "f");
    let env =
        prove_equivalent_with_replay(&m(before), &m(after), &cfg, Some(&s), ReplayPolicy::Run);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    if v["result"]["verdict"] != "differs" {
        return; // no divergence to emit a harness for
    }
    let outcome = v["result"]["replay"]["outcome"].as_str().unwrap_or("");
    assert_ne!(
        outcome, "did_not_build",
        "a witness that is not an argument list is a refusal about the witness, not a build \
         failure about the harness: {v}"
    );
    assert!(
        outcome == "refused" || v["result"]["replay"]["source"].is_null(),
        "and nothing should have been emitted: {v}"
    );
}

/// **"You said don't run it" and "there was nothing to run it with" are different facts.**
///
/// Both produced `outcome: null`, so a consumer could not tell a deliberate `--replay` from a
/// machine with no C compiler — and the blind spot cited 050 contract 11's gate in both cases,
/// which is the wrong reason for the second.
#[test]
fn not_run_and_no_compiler_are_distinguishable() {
    let Some(cfg) = cfg() else { return };
    let same = "int f (int x) { return x * 2; }\n";
    let s = sources("nocc", same, same, "f");
    let double =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = mul i32 %0, 2i32\n  ret %1\n}";
    let triple =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = mul i32 %0, 3i32\n  ret %1\n}";

    let emitted = prove_equivalent_with_replay(
        &m(double),
        &m(triple),
        &cfg,
        Some(&s),
        ReplayPolicy::EmitOnly,
    );
    let v: serde_json::Value = serde_json::from_str(&emitted.to_json()).expect("valid JSON");
    assert_eq!(
        v["result"]["replay"]["outcome"], "not_run",
        "a deliberate EmitOnly must say so rather than being a null: {v}"
    );
    assert!(
        emitted
            .blind_spots
            .iter()
            .any(|b| b.contains("contract 11")),
        "and cite the gate, which is the right reason here: {:?}",
        emitted.blind_spots
    );
}

/// **A return type the `long long` channel cannot carry is refused.**
///
/// The harness reads both results as `long long`. A `double` return would be *converted*, so
/// 1.25 and 1.75 both arrive as 1 and a true divergence reads as agreement — which, before the
/// narrowing above, fed contract 11's downgrade. The type is knowable here, so it is checked
/// here.
#[test]
fn a_float_return_is_refused_rather_than_truncated() {
    let Some(cfg) = cfg() else { return };
    let ret = |bits: &str| {
        format!("func @f(%0: i32) -> f64 {{\nentry:\n  .line 1\n  ret fconst:f64:{bits}\n}}")
    };
    // 1.0 against 2.0: two doubles that both convert to a different `long long`, so the
    // truncation would not even be visible as agreement here — but the refusal must come
    // first, on the type, rather than on whether this particular pair survives it.
    let before = ret("0x3ff0000000000000");
    let after = ret("0x4000000000000000");
    let c = "double f (int x) { return x; }\n";
    let s = sources("floatret", c, c, "f");
    let env =
        prove_equivalent_with_replay(&m(&before), &m(&after), &cfg, Some(&s), ReplayPolicy::Run);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    if v["result"]["verdict"] != "differs" {
        return; // nothing to emit a harness for
    }
    assert_eq!(
        v["result"]["replay"]["outcome"], "refused",
        "a double read as a long long is a truncation, not a comparison: {v}"
    );
}
