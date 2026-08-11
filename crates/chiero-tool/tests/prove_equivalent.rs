//! `prove_equivalent` through 050 §2's envelope — 050 contract 8, and contract 2's rule
//! about `proven`.
//!
//! > **Contract 8.** `prove_equivalent` on an LLM-style rewrite that differs at `INT_MIN`
//! > returns the distinguishing input and a harness that compiles.
//!
//! **The harness half is not built** (041 §1.3), and this file asserts that fact rather
//! than working around it: the response must carry a blind spot saying the divergence has
//! not been demonstrated against a compiler. An un-run harness reported as nothing at all
//! is the failure 050 §2 exists to prevent, and it would be committed here by the crate
//! that enforces it.

use chiero_cir::Module;
use chiero_tool::{Fidelity, prove_equivalent};

fn m(body: &str) -> Module {
    let text = format!("target x86_64-unknown-linux-gnu\n\n{body}\n");
    chiero_cir::text::parse(&text).expect("fixture parses")
}

/// **The solver guard, and it says when it fires.**
///
/// Every caller is `let Some(cfg) = cfg() else { return }` — which returned in silence until
/// 2026-08-11, so on the solverless leg these tests reported `ok` having asserted nothing and
/// `check.sh`'s skip counter could not see them. 54 returns across the suite were invisible that
/// way, against 103 that announced. The message belongs here rather than at each call site
/// because this is what knows *why*.
fn cfg() -> Option<chiero_opt::EquivCfg> {
    let c = chiero_opt::EquivCfg::new("f");
    if c.backend.is_none() {
        eprintln!(
            "skipping a solver-dependent assertion in prove_equivalent.rs: no SMT-LIB backend on PATH \
             (022 contract 2)"
        );
        return None;
    }
    Some(c)
}

/// `int abs(int x) { return x < 0 ? -x : x; }` — what was there.
const BEFORE: &str = "\
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

/// The branchless rewrite an LLM reaches for: `(x ^ (x >> 31)) - (x >> 31)`, made to
/// saturate at `INT_MIN` — the plausible-looking improvement that is not the same function.
const AFTER: &str = "\
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

/// **Contract 8: the distinguishing input comes back through the envelope.**
#[test]
fn a_rewrite_that_differs_at_int_min_returns_the_input_that_shows_it() {
    let Some(cfg) = cfg() else { return };
    let env = prove_equivalent(&m(BEFORE), &m(AFTER), &cfg);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let r = &v["result"];

    assert_eq!(r["verdict"], "differs");

    let input = r["input"].as_array().expect("an input array");
    assert_eq!(input.len(), 1, "one parameter, one binding");
    assert_eq!(
        input[0]["signed"].as_str(),
        Some(i32::MIN.to_string()).as_deref(),
        "INT_MIN is the input that distinguishes them: {input:?}"
    );
    // Both readings are present, because -2147483648 and 2147483648 are the same input and
    // only one of them makes the divergence legible.
    assert_eq!(input[0]["value"].as_str(), Some("2147483648"));
    assert_eq!(input[0]["width"].as_u64(), Some(32));

    let obs = &r["observation"];
    assert_eq!(obs["kind"], "return_value");
    assert_eq!(
        obs["before_signed"].as_str(),
        Some(i32::MIN.to_string()).as_deref()
    );
    assert_eq!(
        obs["after_signed"].as_str(),
        Some(i32::MAX.to_string()).as_deref()
    );
}

/// **The half of contract 8 that is not built must be visible.**
///
/// Not "the test is relaxed because the harness is missing" — the missing harness is the
/// thing under test. A consumer reading `"verdict": "differs"` must be told, in the
/// envelope, that no compiler has confirmed it.
#[test]
fn the_absent_replay_harness_is_a_blind_spot_not_a_silence() {
    let Some(cfg) = cfg() else { return };
    let env = prove_equivalent(&m(BEFORE), &m(AFTER), &cfg);
    assert!(
        env.blind_spots.iter().any(|b| b.contains("replay harness")),
        "041 §1.3's harness did not run and nothing says so: {:?}",
        env.blind_spots
    );
    let rendered = env.render();
    assert!(
        rendered.contains("replay harness"),
        "the rendering a human reads must carry it too:\n{rendered}"
    );
}

/// **Contract 4b, for this operation: at least one input reaches `Exact` and `proven`.**
///
/// > "An always-degenerate implementation must fail this suite. One that always answers
/// > `proven: false, fidelity: Bounded` while doing real work otherwise satisfies every
/// > other contract here and can never license a negative claim."
#[test]
fn an_exhaustive_agreement_is_proven_and_a_bounded_one_is_not() {
    let Some(cfg) = cfg() else { return };

    // Exhaustive: no loop, so every input was covered.
    let same = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = mul i32 %0, 2i32
  ret %1
}";
    let shifted = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = shl i32 %0, 1i32
  ret %1
}";
    let env = prove_equivalent(&m(same), &m(shifted), &cfg);
    assert_eq!(env.fidelity, Fidelity::Exact);
    assert!(env.proven, "an exhaustive agreement is a proof");

    // A loop whose trip count is an input: chiero chose the bound, so the answer holds
    // only within it, and 032 §3.1 must not be able to read this as a licence.
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
    let env = prove_equivalent(&m(looping), &m(looping), &cfg);
    assert_eq!(env.fidelity, Fidelity::Bounded);
    assert!(
        !env.proven,
        "041 §1.2: a bound is a statement about inputs within it, not a proof"
    );
}

/// **An `Equivalent` verdict must say which of §1.1's three claims it did not check.**
///
/// `"verdict": "equivalent"` beside a `compared` list of two is exactly the shape 050 §2
/// warns about: a consumer would have to already know the list was meant to be three.
#[test]
fn an_equivalent_verdict_names_the_claims_it_did_not_decide() {
    let Some(cfg) = cfg() else { return };
    let f = "\
func @f(%0: i32, %1: i32) -> i32 {
entry:
  .line 1
  %2 = add i32 %0, %1
  ret %2
}";
    let env = prove_equivalent(&m(f), &m(f), &cfg);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["verdict"], "equivalent");
    for missing in ["caller-visible memory", "side-effect sequence"] {
        assert!(
            env.blind_spots.iter().any(|b| b.contains(missing)),
            "{missing} went unchecked and unmentioned: {:?}",
            env.blind_spots
        );
    }
}

/// **`Unknown` is never `proven`, and says what it could not decide.**
#[test]
fn an_undecided_comparison_is_not_a_verdict() {
    // Tier 1 alone (022 §3.2) cannot decide these, which is the point.
    let cfg = chiero_opt::EquivCfg::lite("f");
    let f = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = mul i32 %0, 2i32
  ret %1
}";
    let env = prove_equivalent(&m(f), &m(f), &cfg);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["verdict"], "unknown");
    assert_eq!(env.fidelity, Fidelity::Unknown);
    assert!(!env.proven);
    assert!(
        v["result"]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("solver")),
        "the reason must name what went undecided: {v:?}"
    );
}

/// **050 contract 8, the other half: the harness.**
///
/// > `prove_equivalent` on an LLM-style rewrite that differs at `INT_MIN` returns the
/// > distinguishing input **and a harness that compiles**.
///
/// The blind spot asserted above was the honest report of a missing feature. Now that
/// `chiero-replay` exists, the same response must carry the program — and, when execution is
/// allowed, what running it established.
#[test]
fn the_response_carries_a_harness_that_compiles() {
    let Some(cfg) = cfg() else { return };
    let sources = write_pair();
    let env = chiero_tool::prove_equivalent_with_replay(
        &m(BEFORE),
        &m(AFTER),
        &cfg,
        Some(&sources),
        chiero_tool::ReplayPolicy::EmitOnly,
    );
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let replay = &v["result"]["replay"];
    // The programs are the *units* — one per version, each with its own `main`. `source` is the
    // comment a reader is shown; asserting on it alone stopped meaning "the response carries a
    // program" the moment the harness became two of them.
    let programs: String = replay["units"]
        .as_array()
        .expect("two units")
        .iter()
        .filter_map(|u| u["source"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(replay["units"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        programs.matches("int main").count(),
        2,
        "each version must be its own program: {v}"
    );
    // **050 contract 11**: with execution off, text and no verdict — said, not absent.
    //
    // This asserted `outcome` was *null*, which was the weaker contract: a null could not be
    // told from "execution was allowed and no compiler was found", and the blind spot cited
    // the execution gate in both cases. `not_run` names the reason.
    assert_eq!(
        replay["outcome"], "not_run",
        "nobody ran it, and that is different from it having said nothing: {v}"
    );
    assert!(
        env.blind_spots.iter().any(|b| b.contains("not been run")),
        "and the envelope says so: {:?}",
        env.blind_spots
    );
}

/// **With execution allowed, the divergence is confirmed by a compiler.**
///
/// This is the first claim in the system that does not rest on chiero's own semantics.
#[test]
fn running_the_harness_confirms_the_divergence() {
    let Some(cfg) = cfg() else { return };
    if chiero_replay::compiler().is_none() {
        return;
    }
    let sources = write_pair();
    let env = chiero_tool::prove_equivalent_with_replay(
        &m(BEFORE),
        &m(AFTER),
        &cfg,
        Some(&sources),
        chiero_tool::ReplayPolicy::Run,
    );
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["replay"]["outcome"], "demonstrated", "{v}");
    assert_eq!(v["result"]["replay"]["before"], "-2147483648");
    assert_eq!(v["result"]["replay"]["after"], "2147483647");
    assert!(
        !env.blind_spots.iter().any(|b| b.contains("replay harness")),
        "the caveat about no harness must be gone once one ran: {:?}",
        env.blind_spots
    );
}

/// The C the CIR fixtures above correspond to, on disk, for the harness to include.
fn write_pair() -> chiero_tool::ReplaySources {
    let d = std::env::temp_dir().join(format!("chiero-pe-replay-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    let before = d.join("abs_before.c");
    let after = d.join("abs_after.c");
    std::fs::write(&before, "int f (int x) { return x < 0 ? -x : x; }\n").expect("write");
    std::fs::write(
        &after,
        "int f (int x)\n{\n  if (x < 0)\n    return x == (-2147483647 - 1) ? 2147483647 : -x;\n  return x;\n}\n",
    )
    .expect("write");
    chiero_tool::ReplaySources {
        before,
        after,
        entry: "f".into(),
        scratch: d,
        flags: Vec::new(),
    }
}

/// Two programs that **differ**, where the difference is invisible to chiero.
///
/// `g` returns an 80-bit float with no body, and `FpToSi 80 -> 32` is one of 023 §7's
/// approximated gaps — so neither side's result is a value chiero can reason about, and the
/// `+ 1` between them cannot be seen.
const HIDDEN_A: &str = "\
func @f() -> i32 {
entry:
  .line 1
  %0 = call @g()
  %1 = fptosi f80 %0 to i32
  ret %1
}

func @g() -> f80";

const HIDDEN_B: &str = "\
func @f() -> i32 {
entry:
  .line 1
  %0 = call @g()
  %1 = fptosi f80 %0 to i32
  %2 = add i32 %1, 1i32
  ret %2
}

func @g() -> f80";

/// **The false proof is the worst answer this tool can give**, and this is the shape that would
/// produce one.
///
/// The `unknown` already covered above is a function compared *with itself* under a config that
/// cannot decide — an honest "I could not tell" about programs that are in fact the same. This
/// is the dangerous twin: the programs genuinely **differ**, and the difference sits behind a
/// construct chiero does not model. Answering `equivalent` here would licence a rewrite that
/// changes behaviour, on chiero's authority.
///
/// Verified against the CLI on real C the same day: `(int) g((long double) x)` against the same
/// `+ 1` answers `unknown`.
///
/// ⚠️ **What this test cannot see, checked rather than assumed.** Making the two sides
/// *identical* leaves it passing — chiero answers `unknown` for this shape either way. So it
/// does **not** show that chiero noticed the `+ 1`; it shows that chiero never claims a proof
/// it does not have. That is the property worth guarding (a false `equivalent` is the worst
/// answer this tool can give), but a future change making `f80` decidable would need a
/// stronger case here, and this comment is the warning that it would pass regardless.
#[test]
fn a_difference_chiero_cannot_model_is_never_proven_equivalent() {
    let cfg = chiero_opt::EquivCfg::new("f");
    let env = prove_equivalent(&m(HIDDEN_A), &m(HIDDEN_B), &cfg);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_ne!(
        v["result"]["verdict"], "equivalent",
        "the programs differ by one; claiming equivalence because the difference is \
         unmodelled is a false proof: {v}"
    );
    assert!(
        !env.proven,
        "and nothing about this run may be marked proven: {v}"
    );
}
