//! Covers: 042 §3.1 — the tier-1 candidate filter.

use chiero_recipe::{CallGraph, candidates};

/// The shape 042 §3.1 names from `vnet/interface_cli.c`: the registered handlers are
/// one-line delegations, and the acquisition and every free live in a helper that is not
/// itself registered.
fn interface_cli() -> CallGraph {
    let mut g = CallGraph::new();
    g.add_call("show_hw_interfaces", "show_or_clear_hw_interfaces");
    g.add_call("clear_hw_interfaces", "show_or_clear_hw_interfaces");
    g.add_call("show_or_clear_hw_interfaces", "unformat_line_input");
    g.add_call("show_or_clear_hw_interfaces", "unformat_free");
    g.add_call("unrelated_helper", "memcpy");
    g
}

/// **The filter is a closure, not a conjunction.** The obvious filter — in `scope` *and*
/// containing the acquisition — makes neither function a candidate: the handlers match
/// `scope` but contain no acquisition, and the helper holds the acquisition but is not
/// registered. 042 §3.1 records this as a demonstrated recall hole, and tier 2 would have
/// analysed the helper correctly had it ever been escalated.
#[test]
fn the_candidate_set_is_the_callee_closure_not_the_scope_matches() {
    let g = interface_cli();
    let c = candidates(&g, &["show_hw_interfaces", "clear_hw_interfaces"], 3);

    assert!(
        c.escalated
            .iter()
            .any(|f| f == "show_or_clear_hw_interfaces"),
        "the helper holding the acquisition must be escalated: {:?}",
        c.escalated
    );
    // The roots themselves are candidates too — a handler may do the work inline.
    assert!(c.escalated.iter().any(|f| f == "show_hw_interfaces"));
    // A function reachable from no root is not a candidate.
    assert!(!c.escalated.iter().any(|f| f == "unrelated_helper"));
}

/// The closure is bounded by `max_candidate_depth`, and **what the bound excluded is
/// counted**, never silently dropped.
#[test]
fn the_depth_bound_is_reported_rather_than_applied_silently() {
    let mut g = CallGraph::new();
    g.add_call("root", "d1");
    g.add_call("d1", "d2");
    g.add_call("d2", "d3");

    // **The count is the fringe we declined to follow, not everything beyond it.** `d3` is
    // never reached at all: enumerating it would mean walking the whole graph, which is
    // exactly what the bound exists to avoid. So the number understates the unexamined set
    // by design, and `is_bounded` — not the count — is what carries the honesty.
    let shallow = candidates(&g, &["root"], 1);
    assert_eq!(shallow.escalated, ["root", "d1"]);
    assert_eq!(
        shallow.excluded_by_bound, 1,
        "d2 is the fringe; d3 was never reached"
    );
    assert!(shallow.is_bounded());

    let deep = candidates(&g, &["root"], 3);
    assert_eq!(deep.escalated, ["root", "d1", "d2", "d3"]);
    assert_eq!(deep.excluded_by_bound, 0);
}

/// **A function the filter excluded is exactly as unexamined as one never escalated, and
/// must degrade the result identically.** 042 §3.1 records that an earlier draft counted
/// only unescalated candidates, so a function dropped *before* escalation was invisible and
/// the recipe reported "conforms" over a set it never looked at.
#[test]
fn anything_unexamined_forces_bounded_however_it_was_lost() {
    let mut g = CallGraph::new();
    g.add_call("root", "d1");
    g.add_call("d1", "d2");

    // Lost to the depth bound.
    assert!(candidates(&g, &["root"], 1).is_bounded());
    // Nothing lost at all: the only case that may claim an exhaustive look.
    assert!(!candidates(&g, &["root"], 5).is_bounded());
}

/// A cyclic call graph terminates and reports each function once. VPP has recursion and
/// mutual recursion; a closure that revisits would not finish the sweep.
#[test]
fn recursion_terminates_and_does_not_duplicate() {
    let mut g = CallGraph::new();
    g.add_call("a", "b");
    g.add_call("b", "a");
    g.add_call("b", "c");
    let c = candidates(&g, &["a"], 10);
    assert_eq!(c.escalated, ["a", "b", "c"]);
    assert!(!c.is_bounded(), "a cycle is fully explored, not truncated");
}
