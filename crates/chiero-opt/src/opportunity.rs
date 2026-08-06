//! **Opportunity detection** — [041 §2](../../../docs/specs/041-optimization-analysis.md).
//!
//! > Detectors propose; they never rewrite. Each proposal carries the evidence and the
//! > obligations that must hold.
//!
//! and the rule that decides what a proposal is worth:
//!
//! > **A proposal with any `Open` obligation is advisory and labelled as such.** The honest
//! > statement "this looks redundant but I could not prove the intervening call does not write
//! > it" is more useful than a confident wrong claim, and it is what an LLM needs in order to
//! > decide whether to investigate.
//!
//! # The one distinction this module is really about
//!
//! A branch whose other side no state took, and a branch whose other side no state *can* take,
//! are the same observation and opposite claims. The first is what an exhausted search reports;
//! the second is what a truncated one reports; and only the first is a proposal. This project
//! has met that confusion in coverage, in test selection, in equivalence and in reachability —
//! here it decides whether a detector tells somebody to delete live code.
//!
//! So a proposal from a run that did not finish is **advisory**, and says which budget stopped
//! it. That costs a reader nothing and is the difference between a suggestion and a bug report.

// **Re-exported, not redefined.** 041 §2 gives one shape for what a proposal is worth, and two
// enums meaning the same thing is how a consumer comes to handle one and forget the other.
pub use crate::locality::{Benefit, Obligation};

use chiero_cir::Module;
use chiero_exec::{
    Action, Budget, Checker, CheckerCtx, CheckerState, Engine, Event, Fidelity, SolverTier,
};
use chiero_solver::{SmtLib, Term, TermArena};
use std::sync::{Arc, Mutex};

/// How to run the detectors.
#[derive(Clone, Debug)]
pub struct OppCfg {
    pub entry: String,
    pub budget: Budget,
    pub backend: Option<SmtLib>,
}

impl OppCfg {
    pub fn new(entry: impl Into<String>) -> OppCfg {
        OppCfg {
            entry: entry.into(),
            budget: Budget::default(),
            backend: SmtLib::discover(),
        }
    }
}

/// What kind of opportunity this is — 041 §2's semantic detectors, as far as they are built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OppKind {
    /// A branch whose condition the path condition already decides.
    ///
    /// `taken` is the side that can happen; the other is the dead one.
    DeadBranch { taken: bool },
}

/// One proposal — 041 §2's shape, sharing [`Benefit`] and [`Obligation`] with the locality
/// analysis rather than redefining them.
///
/// **One definition of "what a proposal is worth", not two.** Two enums meaning the same thing
/// is how a consumer comes to handle one and forget the other, and this project has corrected
/// that in `chiero-diff` and in `chiero-tool` already.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub kind: OppKind,
    pub rationale: String,
    pub obligations: Vec<Obligation>,
    /// The constraints that imply the branch, as SMT-LIB.
    ///
    /// **The actual terms.** Contract 15 says "with the implying constraints listed", and a
    /// proposal that says "this is dead" without saying why is one nobody can check — which is
    /// the entire difference between this and a linter.
    pub evidence: Vec<String>,
    pub benefit: Benefit,
    /// Derived from the obligations, never assigned — the rule 050 §2 applies to `proven`.
    pub advisory: bool,
}

/// Run the detectors over one function — 041 §2.
///
/// **Takes the module by reference and returns proposals.** 041 contract 17: no API in this
/// crate writes to a source file, and none rewrites a module either.
pub fn detect(m: &Module, cfg: &OppCfg) -> Vec<Proposal> {
    if !m.funcs.iter().any(|f| *f.name == cfg.entry) {
        return Vec::new();
    }
    // The checker runs inside the engine and the engine owns it, so the findings come back
    // through a shared handle rather than out of the checker.
    let found: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));

    let mut arena = TermArena::new();
    let mut engine = Engine::new(m)
        .with_entry(&cfg.entry)
        .with_budget(cfg.budget)
        .with_checker(Box::new(DeadBranch {
            found: Arc::clone(&found),
        }));
    engine = match cfg.backend.clone() {
        Some(b) => engine.with_backend(b),
        None => engine.with_solver(SolverTier::LiteOnly),
    };
    let run = engine.run(&mut arena);

    // **Whether the search finished is what decides a proposal from a suggestion.**
    let truncated: Vec<String> = run
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .filter(|a| a.kind == chiero_exec::AssumptionKind::BudgetHit)
        .map(|a| a.detail.clone())
        .collect();
    let exhaustive = run.fidelity() == Fidelity::Exact && truncated.is_empty();

    let seen = std::mem::take(&mut *found.lock().expect("no other thread holds this"));
    let mut out: Vec<Proposal> = seen
        .into_iter()
        .map(|s| {
            let obligations = if exhaustive {
                vec![Obligation::Discharged {
                    what: "the search was exhaustive, so no path reaches the other side"
                        .to_string(),
                }]
            } else {
                vec![Obligation::Open {
                    why: format!(
                        "the search did not finish ({}), so the other side was not shown \
                         unreachable — only unvisited",
                        truncated
                            .first()
                            .cloned()
                            .unwrap_or_else(|| { format!("fidelity {:?}", run.fidelity()) })
                    ),
                }]
            };
            let advisory = obligations
                .iter()
                .any(|o| matches!(o, Obligation::Open { .. }));
            Proposal {
                kind: OppKind::DeadBranch { taken: s.taken },
                rationale: format!(
                    "the {} side of this branch cannot be taken: the path condition already \
                     decides it{}",
                    if s.taken { "false" } else { "true" },
                    if advisory {
                        " — but see the obligation, the search did not finish"
                    } else {
                        ""
                    }
                ),
                obligations,
                evidence: s.constraints,
                // No cycle model (§3's rule, which applies to §2 as much): a dead branch is a
                // real observation whose value in cycles chiero cannot state.
                benefit: Benefit::Unquantified,
                advisory,
            }
        })
        .collect();

    // 041 contract 24's rule, applied here too: the same input yields the same order.
    out.sort_by(|a, b| a.evidence.cmp(&b.evidence));
    out.dedup_by(|a, b| a.kind == b.kind && a.evidence == b.evidence);
    out
}

/// One fork the engine found decided, before the run's fidelity is known.
struct Seen {
    taken: bool,
    constraints: Vec<String>,
}

/// **041 §2's "branch whose condition is implied by the path condition".**
///
/// The engine already answers this: it forks only where both sides are feasible, and reports
/// `Event::Fork` with a `feasible` pair. A checker that re-asked the solver would be a second
/// answer to a question the engine has decided — and the two would eventually disagree.
struct DeadBranch {
    found: Arc<Mutex<Vec<Seen>>>,
}

impl Checker for DeadBranch {
    fn name(&self) -> &'static str {
        "dead-branch"
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut CheckerCtx) -> Vec<Action> {
        if let Event::Fork {
            st,
            cond,
            feasible: (t, f),
        } = ev
        {
            // Both feasible: an ordinary branch, and nothing to say.
            if *t == *f {
                return Vec::new();
            }
            let path: Vec<Term> = st.path.clone();
            let arena = ctx.arena();
            let mut constraints: Vec<String> = path.iter().map(|c| arena.to_smtlib(*c)).collect();
            // The condition itself, so a reader can see what was decided as well as by what.
            constraints.push(format!("decided: {}", arena.to_smtlib(*cond)));
            self.found
                .lock()
                .expect("no other thread holds this")
                .push(Seen {
                    taken: *t,
                    constraints,
                });
        }
        Vec::new()
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(chiero_exec::NoCheckerState)
    }
}
