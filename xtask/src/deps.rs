//! Mechanical enforcement of the dependency rules in `docs/specs/001-architecture.md` §4.
//!
//! The rules are checked against an abstract graph so they can be unit-tested with
//! synthetic violating fixtures ([001](../../docs/specs/001-architecture.md) contract 8)
//! rather than only against the real workspace, which is expected to be clean.

use indexmap::IndexMap;

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

/// Check the rules in 001 §4 that are decidable from the dependency graph.
///
/// **Rules 1, 2, 3, 5, 6 and 7 are enforced here. Rule 4 is not** — "VPP-specific
/// knowledge lives only in `chiero-vpp`" is a property of source text, not of the
/// dependency graph, and is enforced by `cargo xtask check-vpp-leak` (001 contract 5).
/// Saying "every rule" here would be false, and a green run would read as more
/// assurance than it is.
///
/// Violations are returned in a deterministic order (001 §5).
pub fn check(graph: &Graph) -> Vec<Violation> {
    let mut v = Vec::new();
    check_cycles(graph, &mut v);
    check_layering(graph, &mut v);
    // A duplicate edge must not produce a duplicate violation. `check` is public and
    // takes a hand-built graph, which `workspace_graph`'s dedup does not protect.
    v.dedup_by(|a, b| a == b);
    v
}

/// Rule 1: no cycles.
///
/// Reports **every** strongly connected component containing a cycle, not just the
/// first one reached. A DFS that stops at already-visited nodes drops any second cycle
/// sharing nodes with the first, so an engineer fixes the reported cycle, re-runs, and
/// discovers another.
///
/// Iterative rather than recursive: `check` is public and takes an arbitrary graph, and
/// a recursive walk overflows the stack somewhere around 20k nodes.
fn check_cycles(graph: &Graph, out: &mut Vec<Violation>) {
    let n = graph.len();
    let idx_of = |name: &str| graph.get_index_of(name);

    let mut index = vec![usize::MAX; n];
    let mut lowlink = vec![usize::MAX; n];
    let mut on_stack = vec![false; n];
    let mut scc_stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut components: Vec<Vec<usize>> = Vec::new();

    // (node, index of the next successor to visit)
    let mut work: Vec<(usize, usize)> = Vec::new();

    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        work.push((root, 0));
        while let Some(&mut (v, ref mut next)) = work.last_mut() {
            if *next == 0 {
                index[v] = next_index;
                lowlink[v] = next_index;
                next_index += 1;
                scc_stack.push(v);
                on_stack[v] = true;
            }
            let succs = &graph[v];
            if *next < succs.len() {
                let i = *next;
                *next += 1;
                if let Some(w) = idx_of(&succs[i]) {
                    if index[w] == usize::MAX {
                        work.push((w, 0));
                    } else if on_stack[w] {
                        lowlink[v] = lowlink[v].min(index[w]);
                    }
                }
            } else {
                // Finished v: pop it and fold its lowlink into its parent.
                work.pop();
                if lowlink[v] == index[v] {
                    let mut comp = Vec::new();
                    while let Some(w) = scc_stack.pop() {
                        on_stack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    components.push(comp);
                }
                if let Some(&mut (parent, _)) = work.last_mut() {
                    lowlink[parent] = lowlink[parent].min(lowlink[v]);
                }
            }
        }
    }

    for comp in components {
        // A component is cyclic if it has >1 node, or one node with a self-loop.
        let cyclic = comp.len() > 1 || {
            let v = comp[0];
            graph[v].iter().any(|d| idx_of(d) == Some(v))
        };
        if !cyclic {
            continue;
        }
        let mut names: Vec<&str> = comp
            .iter()
            .map(|&i| graph.get_index(i).unwrap().0.as_str())
            .collect();
        names.sort_unstable(); // deterministic regardless of traversal order (001 §5)
        out.push(Violation {
            rule: "no-cycles",
            detail: format!("dependency cycle among: {}", names.join(", ")),
        });
    }
    // Deterministic across runs and across input orderings.
    out.sort_by(|a, b| a.detail.cmp(&b.detail));
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
            // Rule 5 first, and before the layer lookup: `chiero-span` must depend on
            // nothing, including a crate missing from the layer table.
            if name == "chiero-span" {
                out.push(Violation {
                    rule: "span-is-leaf",
                    detail: format!("`chiero-span` must not depend on `{dep}` (001 §4 rule 5)"),
                });
            }

            let Some(to) = layer(dep) else { continue };

            // Rule 3 / **020 contract 6**: chiero-cir never depends on a frontend crate.
            // The CIR boundary is what makes M1 and M2 independently buildable, so a
            // dependency here means the build order has silently collapsed.
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

            // Rule 2: nothing below a layer may reach up into it. Verticals and
            // surfaces are both "above" the core; `chiero-vpp` is filed under Surfaces
            // (001 §2), so keying this on Vertical alone would leave
            // `chiero-cir -> chiero-vpp` legal — exactly the VPP leak §2 warns about.
            let upward = match (from, to) {
                (Layer::Surface, _) => false,
                (Layer::Vertical, Layer::Surface) => true,
                (Layer::Vertical, _) => false,
                (_, Layer::Vertical | Layer::Surface) => true,
                _ => false,
            };
            if upward {
                out.push(Violation {
                    rule: "no-upward-dependency",
                    detail: format!(
                        "`{name}` ({from:?}) must not depend on `{dep}` ({to:?}) \
                         (001 §4 rule 2)"
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
    let mut dev = Graph::new();
    for pkg in packages {
        // A missing `name` must be an error, not an empty string. `unwrap_or_default()`
        // here would make every package fall out of the layer table, yielding a
        // zero-crate graph, zero violations, and a green gate that checks nothing.
        let name = pkg["name"]
            .as_str()
            .ok_or("cargo metadata: package with no `name` field")?;
        if layer(name).is_none() {
            continue; // xtask and third-party crates are not part of the architecture
        }
        // 001 §4 rule 8: normal and build deps are subject to every rule; dev-deps are
        // subject to layering but exempt from the cycle rule, since cargo permits
        // dev-dep cycles and chiero-cir's round-trip tests legitimately need
        // chiero-lower. They are collected separately so `check` can apply that split.
        let (mut deps, mut dev_deps) = (Vec::new(), Vec::new());
        if let Some(ds) = pkg["dependencies"].as_array() {
            for d in ds {
                let Some(dn) = d["name"].as_str() else {
                    continue;
                };
                if layer(dn).is_none() {
                    continue;
                }
                match d["kind"].as_str() {
                    Some("dev") => dev_deps.push(dn.to_string()),
                    // `null` is a normal dependency; "build" is subject to every rule.
                    _ => deps.push(dn.to_string()),
                }
            }
        }
        dev_deps.sort_unstable();
        dev_deps.dedup();
        dev.insert(name.to_string(), dev_deps);
        graph.insert(name.to_string(), deps);
    }
    if graph.is_empty() {
        return Err("cargo metadata yielded no chiero-* crates; the gate would be a no-op".into());
    }
    // Deterministic order regardless of what cargo emits (001 §5).
    graph.sort_unstable_keys();
    for deps in graph.values_mut() {
        deps.sort_unstable();
        deps.dedup();
    }
    // Dev-deps participate in layering only, so fold them in *after* the graph used for
    // cycle detection has been built. `check_layering` sees them; `check_cycles` does not.
    for (k, extra) in &dev {
        if let Some(v) = graph.get_mut(k) {
            for e in extra {
                if !v.contains(e) {
                    v.push(e.clone());
                }
            }
        }
    }
    Ok(graph)
}

/// Print violations and return the process exit code (001 contract 8).
///
/// Lives here rather than in `main.rs` so the exit-code mapping is unit-testable; the
/// binary is a one-line wrapper over it.
pub fn report(graph: &Graph) -> std::process::ExitCode {
    let violations = check(graph);
    if violations.is_empty() {
        println!("check-deps: {} crates, no violations", graph.len());
        return std::process::ExitCode::SUCCESS;
    }
    eprintln!("check-deps: {} violation(s)\n", violations.len());
    for v in &violations {
        eprintln!("  {v}");
    }
    std::process::ExitCode::FAILURE
}
