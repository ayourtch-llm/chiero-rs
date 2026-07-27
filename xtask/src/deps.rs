//! Mechanical enforcement of the dependency rules in `docs/specs/001-architecture.md` §4.
//!
//! The rules are checked against an abstract graph so they can be unit-tested with
//! synthetic violating fixtures ([001](../../docs/specs/001-architecture.md) contract 8)
//! rather than only against the real workspace, which is expected to be clean.

use indexmap::{IndexMap, IndexSet};

/// Which architectural layer a crate belongs to (001 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Foundation,
    Frontend,
    Core,
    Vertical,
    Surface,
}

/// Layer membership. This table *is* the architecture; 001 §2 is its prose form.
pub fn layer(crate_name: &str) -> Option<Layer> {
    Some(match crate_name {
        "chiero-span" => Layer::Foundation,
        "chiero-lex" | "chiero-pp" | "chiero-ast" | "chiero-parse" | "chiero-sema"
        | "chiero-lower" => Layer::Frontend,
        "chiero-cir" | "chiero-solver" | "chiero-mem" | "chiero-model" | "chiero-exec" => {
            Layer::Core
        }
        "chiero-gcov" | "chiero-diff" | "chiero-select" | "chiero-check" | "chiero-opt"
        | "chiero-recipe" => Layer::Vertical,
        "chiero-vpp" | "chiero-tool" | "chiero-cli" => Layer::Surface,
        _ => return None,
    })
}

/// Verticals permitted to depend on frontend crates because they need the typed AST
/// (001 §4 rule 7).
const FRONTEND_USING_VERTICALS: &[&str] = &["chiero-diff", "chiero-recipe"];

/// Permitted vertical→vertical edges (001 §4 rule 6). Anything else is a violation.
const ALLOWED_VERTICAL_EDGES: &[(&str, &str)] = &[
    ("chiero-select", "chiero-gcov"),
    ("chiero-select", "chiero-diff"),
    ("chiero-select", "chiero-opt"),
    ("chiero-opt", "chiero-check"),
    ("chiero-recipe", "chiero-check"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub rule: &'static str,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.rule, self.detail)
    }
}

/// A crate dependency graph: crate name → the `chiero-*` crates it depends on.
pub type Graph = IndexMap<String, Vec<String>>;

/// Check every rule in 001 §4. Returns violations in a deterministic order.
pub fn check(graph: &Graph) -> Vec<Violation> {
    let mut v = Vec::new();
    check_cycles(graph, &mut v);
    check_layering(graph, &mut v);
    v
}

/// Rule 1: no cycles.
fn check_cycles(graph: &Graph, out: &mut Vec<Violation>) {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unvisited,
        InProgress,
        Done,
    }
    let mut mark: IndexMap<&str, Mark> = graph
        .keys()
        .map(|k| (k.as_str(), Mark::Unvisited))
        .collect();
    let mut stack: Vec<&str> = Vec::new();
    let mut reported: IndexSet<String> = IndexSet::new();

    fn visit<'a>(
        node: &'a str,
        graph: &'a Graph,
        mark: &mut IndexMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
        reported: &mut IndexSet<String>,
    ) {
        match mark.get(node).copied() {
            Some(Mark::Done) | None => return,
            Some(Mark::InProgress) => {
                // Found a cycle: report the segment of the stack from `node` onward.
                let start = stack.iter().position(|n| *n == node).unwrap_or(0);
                let mut cyc: Vec<&str> = stack[start..].to_vec();
                cyc.push(node);
                reported.insert(cyc.join(" -> "));
                return;
            }
            Some(Mark::Unvisited) => {}
        }
        mark.insert(node, Mark::InProgress);
        stack.push(node);
        if let Some(deps) = graph.get(node) {
            for d in deps {
                visit(d, graph, mark, stack, reported);
            }
        }
        stack.pop();
        mark.insert(node, Mark::Done);
    }

    let names: Vec<&str> = graph.keys().map(|s| s.as_str()).collect();
    for n in names {
        visit(n, graph, &mut mark, &mut stack, &mut reported);
    }
    for c in reported {
        out.push(Violation {
            rule: "no-cycles",
            detail: format!("dependency cycle: {c}"),
        });
    }
}

/// Rules 2–7: layering.
fn check_layering(graph: &Graph, out: &mut Vec<Violation>) {
    for (name, deps) in graph {
        let Some(from) = layer(name) else {
            out.push(Violation {
                rule: "known-crate",
                detail: format!("`{name}` is not in the layer table (001 §2)"),
            });
            continue;
        };
        for dep in deps {
            let Some(to) = layer(dep) else { continue };

            // Rule 5: chiero-span depends on no other chiero crate.
            if name == "chiero-span" {
                out.push(Violation {
                    rule: "span-is-leaf",
                    detail: format!("`chiero-span` must not depend on `{dep}` (001 §4 rule 5)"),
                });
            }

            // Rule 3: chiero-cir never depends on a frontend crate.
            if name == "chiero-cir" && to == Layer::Frontend {
                out.push(Violation {
                    rule: "cir-contract-boundary",
                    detail: format!(
                        "`chiero-cir` must not depend on frontend crate `{dep}` \
                         (001 §3, §4 rule 3) — this is the rule that makes the symbolic \
                         core buildable before the parser exists"
                    ),
                });
            }

            // Rule 2: only verticals and surfaces may depend on a vertical.
            if to == Layer::Vertical && !matches!(from, Layer::Vertical | Layer::Surface) {
                out.push(Violation {
                    rule: "no-upward-dependency",
                    detail: format!(
                        "`{name}` ({from:?}) must not depend on vertical `{dep}` (001 §4 rule 2)"
                    ),
                });
            }

            // Rule 7: only chiero-diff and chiero-recipe may use frontend crates
            // among the verticals; the core never may.
            if to == Layer::Frontend {
                if from == Layer::Core {
                    out.push(Violation {
                        rule: "core-is-frontend-free",
                        detail: format!(
                            "core crate `{name}` must not depend on frontend crate `{dep}` \
                             (001 §4 rule 7)"
                        ),
                    });
                } else if from == Layer::Vertical
                    && !FRONTEND_USING_VERTICALS.contains(&name.as_str())
                {
                    out.push(Violation {
                        rule: "vertical-frontend-allowlist",
                        detail: format!(
                            "vertical `{name}` must not depend on frontend crate `{dep}`; \
                             only {FRONTEND_USING_VERTICALS:?} may (001 §4 rule 7)"
                        ),
                    });
                }
            }

            // Rule 6: restricted vertical→vertical edges.
            if from == Layer::Vertical
                && to == Layer::Vertical
                && !ALLOWED_VERTICAL_EDGES.contains(&(name.as_str(), dep.as_str()))
            {
                out.push(Violation {
                    rule: "vertical-edge-allowlist",
                    detail: format!(
                        "vertical edge `{name}` -> `{dep}` is not in the permitted set \
                         (001 §4 rule 6)"
                    ),
                });
            }
        }
    }
}

/// Build the graph for the real workspace from `cargo metadata`.
///
/// Only `chiero-*` members are included; third-party dependencies are irrelevant to
/// the layering rules and would only add noise.
pub fn workspace_graph() -> Result<Graph, String> {
    let out = std::process::Command::new(std::env::var("CARGO").as_deref().unwrap_or("cargo"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("running cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("parsing metadata: {e}"))?;

    let packages = json["packages"]
        .as_array()
        .ok_or("metadata has no `packages` array")?;

    let mut graph = Graph::new();
    for pkg in packages {
        let name = pkg["name"].as_str().unwrap_or_default();
        if layer(name).is_none() {
            continue; // xtask and third-party crates are not part of the architecture
        }
        let deps: Vec<String> = pkg["dependencies"]
            .as_array()
            .map(|ds| {
                ds.iter()
                    .filter_map(|d| d["name"].as_str())
                    .filter(|d| layer(d).is_some())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        graph.insert(name.to_string(), deps);
    }
    // Deterministic order regardless of what cargo emits (001 §5).
    graph.sort_unstable_keys();
    for deps in graph.values_mut() {
        deps.sort_unstable();
        deps.dedup();
    }
    Ok(graph)
}
