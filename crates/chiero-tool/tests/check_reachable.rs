//! **050 contract 5 — `check_reachable`, and the two answers that must never be one.**
//!
//! > 5. `check_reachable` returns structurally distinct variants for unreachable-with-proof
//! >    and not-shown-reachable; a fixture forcing each asserts they are not conflatable.
//!
//! This is the project's recurring axis in a single operation. *"I proved nothing gets here"*
//! and *"I did not get here"* are the same observation and opposite claims, and a consumer
//! that cannot tell them apart will delete live code.
//!
//! The contract asks for **structurally** distinct, not differently worded: a reader who
//! matches on the verdict must be unable to conflate them, whatever the prose says.

use chiero_cir::Module;
use chiero_tool::{Fidelity, check_reachable};

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn cfg(entry: &str) -> chiero_tool::BugCfg {
    chiero_tool::BugCfg::new(entry)
}

/// `if (x == 0) return 1;` — line 3 is reachable, line 5 is reachable, both plainly.
const BRANCH: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp eq i32 %0, 0i32
  br %1, bb1, bb2
bb1:
  .line 3
  ret 1i32
bb2:
  .line 5
  ret 2i32
}";

/// The same shape with a condition nothing satisfies: `if (x != x)`.
const DEAD: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = cmp ne i32 %0, %0
  br %1, bb1, bb2
bb1:
  .line 3
  ret 1i32
bb2:
  .line 5
  ret 2i32
}";

/// **Reachable, with the input that gets there.** A verdict of "yes" that could not say how is
/// a guess.
#[test]
fn a_reachable_line_comes_with_an_input_that_reaches_it() {
    let env = check_reachable(&m(BRANCH), &cfg("f"), 3);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["verdict"], "reachable");
    assert!(env.proven, "a witness is a proof of reachability: {v}");
    assert!(
        v["result"]["witness"].as_array().is_some_and(|w| !w.is_empty()),
        "the input that gets there: {v}"
    );
}

/// **Unreachable, with a proof.** The search was exhaustive and nothing arrived.
#[test]
fn an_exhaustively_unreached_line_is_proven_unreachable() {
    let env = check_reachable(&m(DEAD), &cfg("f"), 3);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["verdict"], "unreachable");
    assert!(
        env.proven,
        "an exhaustive search is what makes this a proof: {v}"
    );
    assert_eq!(env.fidelity, Fidelity::Exact);
}

/// A loop chiero has to bound, with the line of interest past a trip count it cannot reach.
const BEYOND_THE_BOUND: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  goto bb1
bb1:
  .line 2
  %1 = phi i32 [entry 0i32] [bb1 %2]
  %2 = add i32 %1, 1i32
  %3 = cmp slt i32 %2, 1000i32
  br %3, bb1, bb2
bb2:
  .line 9
  ret %1
}";

/// **Not shown reachable — and it must not say "unreachable".**
///
/// The line is genuinely reachable; chiero's loop bound stops the search before it gets there.
/// An operation that reported "unreachable" here would be telling a reader to delete live code,
/// which is the most expensive form this project's recurring failure takes.
#[test]
fn a_line_past_the_loop_bound_is_not_shown_reachable_and_says_so() {
    let env = check_reachable(&m(BEYOND_THE_BOUND), &cfg("f"), 9);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(
        v["result"]["verdict"], "not_shown_reachable",
        "chiero did not get there; that is not the same as nothing getting there: {v}"
    );
    assert!(!env.proven, "{v}");
    assert!(
        v["result"]["why"].as_str().is_some_and(|s| s.contains("max_loop_iters")),
        "and it names what stopped the search: {v}"
    );
}

/// **Contract 5's actual requirement: the two are not conflatable.**
///
/// Not "worded differently" — a consumer matching on the verdict must be structurally unable
/// to treat one as the other, and the proven flag must agree with the verdict rather than
/// being a second opinion about it.
#[test]
fn the_two_negative_answers_are_structurally_distinct() {
    let proven = check_reachable(&m(DEAD), &cfg("f"), 3);
    let unknown = check_reachable(&m(BEYOND_THE_BOUND), &cfg("f"), 9);

    let pv: serde_json::Value = serde_json::from_str(&proven.to_json()).expect("valid JSON");
    let uv: serde_json::Value = serde_json::from_str(&unknown.to_json()).expect("valid JSON");

    assert_ne!(
        pv["result"]["verdict"], uv["result"]["verdict"],
        "one verdict for two claims is the conflation this contract forbids"
    );
    assert!(proven.proven && !unknown.proven);

    // The unproven one carries what stopped it; the proven one has nothing to carry.
    assert!(uv["result"]["why"].is_string());
    assert!(pv["result"]["why"].is_null());

    // And the shapes differ in the field that matters, not only in a string.
    assert!(
        !unknown.blind_spots.is_empty(),
        "an unproven answer says what it could not see: {:?}",
        unknown.blind_spots
    );
}

/// **A line that is nowhere in the function is a different answer again.**
///
/// Not "unreachable" — nothing was asked about, and answering "unreachable" about a line that
/// does not exist is the same silence-read-as-success one level out.
#[test]
fn a_line_the_function_does_not_have_is_not_unreachable() {
    let env = check_reachable(&m(BRANCH), &cfg("f"), 4242);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["verdict"], "no_such_line");
    assert!(!env.proven, "{v}");
}
