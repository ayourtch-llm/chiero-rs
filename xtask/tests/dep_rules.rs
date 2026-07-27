//! Tests for the architecture's dependency rules.
//!
//! Covers **001 contract 8**: `xtask check-deps` exits non-zero when a rule in
//! `docs/specs/001-architecture.md` §4 is violated, verified by fixtures that
//! deliberately violate one. Also covers 001 contracts 1–4 at the graph level.
//!
//! Each violating fixture is a synthetic graph, so these tests fail if the checker
//! stops detecting a rule — which a test that only ran against the (clean) real
//! workspace could never catch.

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

fn assert_violates(g: &Graph, rule: &str) {
    let v = check(g);
    assert!(
        v.iter().any(|x| x.rule == rule),
        "expected a `{rule}` violation, got: {v:?}"
    );
}

/// 001 §4 rule 3 / §3 — the rule that makes the symbolic core buildable before the
/// parser exists. The single most important structural rule in the project.
#[test]
fn cir_may_not_depend_on_the_frontend() {
    assert_violates(
        &with_edge("chiero-cir", "chiero-ast"),
        "cir-contract-boundary",
    );
    assert_violates(
        &with_edge("chiero-cir", "chiero-parse"),
        "cir-contract-boundary",
    );
}

/// 001 §4 rule 5.
#[test]
fn span_depends_on_nothing() {
    assert_violates(&with_edge("chiero-span", "chiero-lex"), "span-is-leaf");
}

/// 001 §4 rule 2 — the core never reaches upward into a vertical.
#[test]
fn core_may_not_depend_on_a_vertical() {
    assert_violates(
        &with_edge("chiero-exec", "chiero-check"),
        "no-upward-dependency",
    );
    assert_violates(
        &with_edge("chiero-mem", "chiero-gcov"),
        "no-upward-dependency",
    );
}

/// 001 §4 rule 7 — the core is frontend-free; only two verticals may use the AST.
#[test]
fn frontend_use_is_restricted() {
    assert_violates(
        &with_edge("chiero-mem", "chiero-sema"),
        "core-is-frontend-free",
    );
    assert_violates(
        &with_edge("chiero-gcov", "chiero-ast"),
        "vertical-frontend-allowlist",
    );
    // ...but the two allowlisted verticals are fine, and already exercise this in
    // `legal()`.
    assert!(check(&with_edge("chiero-recipe", "chiero-ast")).is_empty());
}

/// 001 §4 rule 6 — vertical-to-vertical edges are an explicit allowlist.
#[test]
fn vertical_edges_are_allowlisted() {
    assert_violates(
        &with_edge("chiero-gcov", "chiero-check"),
        "vertical-edge-allowlist",
    );
    // A permitted edge is not a violation.
    assert!(check(&with_edge("chiero-select", "chiero-gcov")).is_empty());
}

/// 001 §4 rule 1 / contract 1.
#[test]
fn cycles_are_detected() {
    let g = graph(&[
        ("chiero-cir", &["chiero-solver"]),
        ("chiero-solver", &["chiero-mem"]),
        ("chiero-mem", &["chiero-cir"]),
    ]);
    assert_violates(&g, "no-cycles");
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
    assert_violates(&g, "known-crate");
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
