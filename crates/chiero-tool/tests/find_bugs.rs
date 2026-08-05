//! **050 contract 3 — `find_bugs`, and the rendering that may never be read as "all clear".**
//!
//! > 3. A `find_bugs` run that hits a budget returns `proven: false`, a non-empty
//! >    `budgets.hit`, and text containing "within"; the string "no defects found" never
//! >    appears unqualified in any rendering.
//!
//! This is the operation 050 §2's whole argument is about. Every other one answers a question
//! whose empty answer is merely uninformative; this one's empty answer is *"your code is
//! fine"*, and it is wrong exactly when the search did not finish.
//!
//! The three cases below are the three an empty finding list can mean, and the envelope must
//! keep them apart:
//!
//! | the run | what the empty list means |
//! |---|---|
//! | finished, `Exact` | nothing here — the strongest thing chiero can say |
//! | hit a budget | nothing *within the bound*, and nothing at all about beyond it |
//! | could not model something | nothing chiero could see |

use chiero_cir::Module;
use chiero_tool::{Fidelity, find_bugs};

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn cfg(entry: &str) -> chiero_tool::BugCfg {
    chiero_tool::BugCfg::new(entry)
}

/// `int f (int x) { return x + 1; }` — nothing wrong, and nothing in the way of saying so.
const CLEAN: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = add i32 %0, 1i32
  ret %1
}";

/// **A clean exhaustive run is the one place an empty list is a real answer.**
///
/// Contract 4b's requirement, for this operation: an implementation that always answers
/// `proven: false` satisfies every other contract here and can never license a negative claim.
#[test]
fn a_finished_run_with_no_findings_is_proven_empty() {
    let env = find_bugs(&m(CLEAN), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["findings"].as_array().map(Vec::len), Some(0));
    assert_eq!(env.fidelity, Fidelity::Exact);
    assert!(
        env.proven,
        "an exhaustive search that found nothing has found nothing: {v}"
    );
    assert_eq!(
        v["result"]["budgets"]["hit"].as_array().map(Vec::len),
        Some(0),
        "nothing was cut: {v}"
    );
}

/// A loop whose trip count is an input, so chiero's bound decides where the search stops.
const UNBOUNDED: &str = "\
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

/// **Contract 3, in full.**
#[test]
fn a_run_that_hit_a_budget_says_which_and_is_not_proven() {
    let env = find_bugs(&m(UNBOUNDED), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");

    assert!(!env.proven, "a truncated search proves nothing: {v}");
    let hit = v["result"]["budgets"]["hit"].as_array().expect("budgets.hit");
    assert!(
        !hit.is_empty(),
        "the budget that stopped the search must be named: {v}"
    );
    assert!(
        hit.iter().any(|b| b.as_str().is_some_and(|s| s.contains("max_loop_iters"))),
        "and named specifically, not as `a budget`: {hit:?}"
    );
    assert!(
        env.render().contains("within"),
        "the rendering must say the answer is within a bound:\n{}",
        env.render()
    );
}

/// **The string a reader must never see unqualified**, over every case.
///
/// 050 §2 names this failure directly: *"an LLM reading `findings: []` will report 'the code
/// is safe'"*. The wording of the empty case is therefore part of the contract, not a
/// presentation detail.
#[test]
fn no_rendering_says_no_defects_found_bare() {
    for (what, src, entry) in [
        ("clean", CLEAN, "f"),
        ("budget-cut", UNBOUNDED, "f"),
        ("missing entry", CLEAN, "nosuch"),
    ] {
        let env = find_bugs(&m(src), &cfg(entry));
        let r = env.render();
        let bare = r.contains("no defects found") && !r.contains("within") && !env.proven;
        assert!(!bare, "{what}: an unqualified all-clear:\n{r}");
        // And an unproven one always carries something to read.
        if !env.proven {
            assert!(
                !env.blind_spots.is_empty() || !env.assumptions.is_empty(),
                "{what}: proven false and nothing said about why:\n{r}"
            );
        }
    }
}

/// **A real defect is reported with the input that reaches it.**
///
/// Without this the operation could satisfy every contract above by finding nothing, ever.
#[test]
fn a_signed_overflow_is_found_and_witnessed() {
    // `int f (int x) { return x + 1; }` overflows at INT_MAX — but only the *maybe* kind,
    // since it depends on the input. `INT_MAX + 1` written as constants is definite.
    let definite = "\
func @f() -> i32 {
entry:
  .line 1
  %0 = add i32 2147483647i32, 1i32
  ret %0
}";
    let env = find_bugs(&m(definite), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let findings = v["result"]["findings"].as_array().expect("findings");
    assert!(
        !findings.is_empty(),
        "signed overflow of INT_MAX + 1 is a defect: {v}"
    );
    assert!(
        findings[0]["message"].as_str().is_some_and(|s| !s.is_empty()),
        "a finding with no message is not a finding: {v}"
    );
    // 023 contract 15: a witness, or a recorded reason there is none. Never silence.
    assert!(
        !findings[0]["witness"].is_null() || !findings[0]["unwitnessed"].is_null(),
        "the absence is allowed; the silence is not: {v}"
    );
}

/// **An entry that is not there is an error, not a clean bill of health.**
#[test]
fn a_missing_entry_is_not_an_empty_finding_list() {
    let env = find_bugs(&m(CLEAN), &cfg("nosuch"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert!(!env.proven, "nothing was analysed: {v}");
    assert!(
        v["result"]["error"].as_str().is_some_and(|e| e.contains("nosuch")),
        "the error must name what was not found: {v}"
    );
}
