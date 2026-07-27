//! Mechanical enforcement of the dependency rules in `docs/specs/001-architecture.md` §4.

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
pub fn layer(_crate_name: &str) -> Option<Layer> {
    todo!("green")
}

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
pub fn check(_graph: &Graph) -> Vec<Violation> {
    Vec::new()
}

/// Build the graph for the real workspace from `cargo metadata`.
pub fn workspace_graph() -> Result<Graph, String> {
    Ok(Graph::new())
}
