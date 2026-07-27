//! Tests for the architecture's dependency rules.
//!
//! Covers **001 contract 8**: `xtask check-deps` exits non-zero when a rule in
//! `docs/specs/001-architecture.md` §4 is violated, verified by fixtures that
//! deliberately violate one. Also covers 001 contracts 1–4 at the graph level.
//!
//! Each violating fixture is a synthetic graph, so these tests fail if the checker
//! stops detecting a rule — which a test that only ran against the (clean) real
//! workspace could never catch.

use std::collections::BTreeSet;
use xtask::deps::{Graph, check};

fn graph(edges: &[(&str, &[&str])]) -> Graph {
    edges
        .iter()
        .map(|(k, v)| {
            (
                (*k).to_string(),
                v.iter().map(|s| (*s).to_string()).collect(),
            )
        })
        .collect()
}

/// A minimal but legal graph, used as the base for the violating fixtures so that
/// each fixture differs from a passing case by exactly one edge.
fn legal() -> Graph {
    graph(&[
        ("chiero-span", &[]),
        ("chiero-lex", &["chiero-span"]),
        ("chiero-ast", &["chiero-span"]),
        ("chiero-sema", &["chiero-span", "chiero-ast"]),
        ("chiero-cir", &["chiero-span"]),
        ("chiero-lower", &["chiero-ast", "chiero-sema", "chiero-cir"]),
        ("chiero-solver", &[]),
        ("chiero-mem", &["chiero-span", "chiero-solver"]),
        (
            "chiero-exec",
            &["chiero-cir", "chiero-mem", "chiero-solver"],
        ),
        ("chiero-gcov", &["chiero-span", "chiero-cir"]),
        ("chiero-diff", &["chiero-span", "chiero-ast", "chiero-sema"]),
        ("chiero-check", &["chiero-cir", "chiero-exec"]),
        ("chiero-opt", &["chiero-exec", "chiero-check"]),
        (
            "chiero-recipe",
            &["chiero-sema", "chiero-exec", "chiero-check"],
        ),
        (
            "chiero-select",
            &["chiero-gcov", "chiero-diff", "chiero-opt"],
        ),
        ("chiero-vpp", &["chiero-check", "chiero-recipe"]),
        ("chiero-cli", &["chiero-exec", "chiero-vpp"]),
    ])
}

/// The base fixture must be clean, or every violation test below is meaningless.
#[test]
fn legal_graph_has_no_violations() {
    let v = check(&legal());
    assert!(v.is_empty(), "legal fixture should be clean, got: {v:?}");
}

fn with_edge(from: &str, to: &str) -> Graph {
    let mut g = legal();
    g.entry(from.to_string()).or_default().push(to.to_string());
    g
}

/// The set of rule names a graph violates.
fn rules(g: &Graph) -> BTreeSet<&'static str> {
    check(g).into_iter().map(|v| v.rule).collect()
}

/// Assert that adding one edge to `legal()` produces **exactly** the named rules.
///
/// Asserting only that the expected rule is *among* the violations is not enough: an
/// implementation that decides legality correctly but tags every violation with all
/// seven rule names passes such a suite completely, while telling an engineer nothing
/// about which invariant they broke. The rule name is the entire diagnostic value of
/// this tool, so the delta is pinned exactly.
#[track_caller]
fn assert_new_rules(from: &str, to: &str, expected: &[&str]) {
    let g = with_edge(from, to);
    // `no-cycles` is excluded: many single-edge additions to `legal()` are back edges
    // that incidentally close a cycle, which would swamp the layering delta under test.
    // Cycles have their own test.
    let got: BTreeSet<_> = rules(&g)
        .difference(&rules(&legal()))
        .copied()
        .filter(|r| *r != "no-cycles")
        .collect();
    let want: BTreeSet<_> = expected.iter().copied().collect();
    assert_eq!(got, want, "adding `{from}` -> `{to}`");

    // The detail must name both crates, or the message is not actionable.
    for v in check(&g).iter().filter(|v| want.contains(v.rule)) {
        assert!(
            v.detail.contains(from) && v.detail.contains(to),
            "detail must name both crates: {}",
            v.detail
        );
    }
}

/// 001 §4 rule 3 / §3 — the rule that makes the symbolic core buildable before the
/// parser exists. The single most important structural rule in the project.
#[test]
fn cir_may_not_depend_on_the_frontend() {
    // Both fire: cir is Core (rule 7) and is also specifically named by rule 3.
    assert_new_rules(
        "chiero-cir",
        "chiero-ast",
        &["cir-contract-boundary", "core-is-frontend-free"],
    );
    assert_new_rules(
        "chiero-cir",
        "chiero-parse",
        &["cir-contract-boundary", "core-is-frontend-free"],
    );
}

/// 001 §4 rule 5.
#[test]
fn span_depends_on_nothing() {
    assert_new_rules("chiero-span", "chiero-lex", &["span-is-leaf"]);
    // Even a dependency on a crate missing from the layer table must trip it.
    assert_new_rules("chiero-span", "chiero-unknown", &["span-is-leaf"]);
}

/// 001 §4 rule 2 — the core never reaches upward into a vertical.
#[test]
fn core_may_not_depend_on_a_vertical() {
    assert_new_rules("chiero-exec", "chiero-check", &["no-upward-dependency"]);
    assert_new_rules("chiero-mem", "chiero-gcov", &["no-upward-dependency"]);
}

/// The core must not reach *any* layer above it, and `chiero-vpp` is filed under
/// Surfaces rather than Verticals (001 §2) — so a rule keyed only on Verticals leaves
/// `chiero-cir -> chiero-vpp` legal, which is exactly the leak 001 §2 warns about.
#[test]
fn nothing_below_a_surface_may_depend_on_one() {
    assert_new_rules("chiero-cir", "chiero-vpp", &["no-upward-dependency"]);
    assert_new_rules("chiero-mem", "chiero-tool", &["no-upward-dependency"]);
    assert_new_rules("chiero-gcov", "chiero-vpp", &["no-upward-dependency"]);
    assert_new_rules("chiero-lex", "chiero-cli", &["no-upward-dependency"]);
    // Surface-to-surface stays legal.
    assert_new_rules("chiero-cli", "chiero-vpp", &[]);
}

/// 001 §4 rule 7 — the core is frontend-free; only two verticals may use the AST.
#[test]
fn frontend_use_is_restricted() {
    assert_new_rules("chiero-mem", "chiero-sema", &["core-is-frontend-free"]);
    assert_new_rules(
        "chiero-gcov",
        "chiero-ast",
        &["vertical-frontend-allowlist"],
    );
    // ...but the two allowlisted verticals may.
    assert_new_rules("chiero-recipe", "chiero-ast", &[]);
    assert_new_rules("chiero-diff", "chiero-lex", &[]);
}

/// 001 §4 rule 6 — vertical-to-vertical edges are an explicit allowlist.
#[test]
fn vertical_edges_are_allowlisted() {
    assert_new_rules("chiero-gcov", "chiero-check", &["vertical-edge-allowlist"]);
    assert_new_rules("chiero-check", "chiero-opt", &["vertical-edge-allowlist"]);
    // A permitted edge that is not already in `legal()` is accepted.
    assert_new_rules("chiero-select", "chiero-check", &[]);
}

/// 001 §4 rule 1 / contract 1.
#[test]
fn cycles_are_detected() {
    let three = graph(&[
        ("chiero-cir", &["chiero-solver"]),
        ("chiero-solver", &["chiero-mem"]),
        ("chiero-mem", &["chiero-cir"]),
    ]);
    assert!(rules(&three).contains("no-cycles"));

    // A two-cycle and a self-loop are the boundary cases.
    let two = graph(&[
        ("chiero-cir", &["chiero-mem"]),
        ("chiero-mem", &["chiero-cir"]),
    ]);
    assert!(rules(&two).contains("no-cycles"));

    let self_loop = graph(&[("chiero-cir", &["chiero-cir"])]);
    assert!(rules(&self_loop).contains("no-cycles"));

    // Every distinct cycle is reported, not just the first one found. An engineer
    // who fixes the reported cycle and re-runs should not discover a new one.
    let two_cycles = graph(&[
        ("chiero-cir", &["chiero-mem", "chiero-solver"]),
        ("chiero-mem", &["chiero-cir"]),
        ("chiero-solver", &["chiero-mem"]),
    ]);
    let cycles = check(&two_cycles);
    assert_eq!(
        cycles.iter().filter(|v| v.rule == "no-cycles").count(),
        2,
        "both cycles must be reported, got: {cycles:#?}"
    );
}

/// An unknown crate must be reported rather than silently skipped — otherwise a
/// typo in the layer table would disable every rule for that crate.
#[test]
fn unknown_crates_are_reported() {
    let mut g = legal();
    g.insert(
        "chiero-mystery".to_string(),
        vec!["chiero-span".to_string()],
    );
    assert!(rules(&g).contains("known-crate"));
}

/// Violations must be reported in a stable order (001 §5: determinism).
#[test]
fn violation_order_is_deterministic() {
    let mut g = with_edge("chiero-cir", "chiero-ast");
    g.entry("chiero-mem".to_string())
        .or_default()
        .push("chiero-gcov".to_string());
    let a: Vec<String> = check(&g).iter().map(|v| v.to_string()).collect();
    let b: Vec<String> = check(&g).iter().map(|v| v.to_string()).collect();
    assert_eq!(a, b);
    assert!(a.len() >= 2, "expected both violations, got {a:?}");
}

/// The real workspace must obey its own rules. This is the test that would catch a
/// bad `Cargo.toml` edit; the fixtures above are what keep *it* honest.
#[test]
fn the_real_workspace_is_clean() {
    let g = xtask::deps::workspace_graph().expect("cargo metadata");
    let v = check(&g);
    assert!(
        v.is_empty(),
        "workspace violates its own architecture:\n{v:#?}"
    );
}
