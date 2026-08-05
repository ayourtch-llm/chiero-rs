//! `prove_equivalent` — [041 §1](../../../docs/specs/041-optimization-analysis.md).
//!
//! > "That primitive, not the opportunity detectors, is chiero's most valuable output. An
//! > LLM is good at proposing a faster or clearer version of a C function and bad at being
//! > sure it is correct. chiero is bad at inventing rewrites and good at deciding whether
//! > two functions agree. **The LLM proposes; chiero adjudicates.**"
//!
//! # The signature deviates from the spec, on purpose
//!
//! §1 writes `fn prove_equivalent(before: &Function, after: &Function, cfg) -> Equivalence`.
//! A `Function` cannot be executed on its own: its callees, its globals and its target all
//! live in the `Module`, and a version that took two bare functions would have to invent a
//! module around each — which is exactly the kind of fabrication 010 §4 forbids elsewhere.
//! So this takes two modules and a `cfg.entry` naming the function in both.
//!
//! # Method — [041 §1.2](../../../docs/specs/041-optimization-analysis.md)
//!
//! §1.2 asks for relational (product) execution: "both functions run against the **same**
//! symbolic inputs and the same extern-return symbols, paths are paired by input
//! constraint, and the comparison is a solver query on the disjunction of the three
//! disagreement conditions."
//!
//! Both runs share one [`TermArena`], so a term from either side is a term the same solver
//! query can mention. They do **not** share input *symbols* — `TermArena::var` mints a
//! fresh `VarId` per call, by design — so "the same symbolic inputs" is imposed rather
//! than assumed: every pair of paths is conjoined with an explicit equality per matched
//! input. Imposing it is the more honest of the two, because it makes the matching visible:
//! an input this code cannot match is an input it must refuse to answer about, and
//! [`Unmatched`] is what it refuses with.
//!
//! # What is not built yet, stated rather than papered over
//!
//! §1.1 makes equivalence three claims — return value, observable footprint, ordered side
//! effects — and only the first is decided here. The other two are not silently assumed to
//! hold: a run whose paths touch memory the caller can see, or whose two sides record
//! different effect sequences, is [`Equivalence::Unknown`] naming which claim went
//! unchecked. That is the difference between "chiero proved these agree" and "chiero
//! checked the easy third and said nothing about the rest".

use chiero_cir::Module;
use chiero_exec::{
    Assumption, Binding, Budget, Engine, Fidelity, InputOrigin, State, TermReason, Value, Witness,
};
use chiero_solver::{BvConst, CheckResult, Model, PathCondition, SmtLib, Term, TermArena,
    TieredSolver, UnknownReason};

/// How to run the two sides. Both get the same budget: §1.2's "loops are bounded by the
/// same `k` in both" is not a tuning knob, it is what makes the comparison mean anything.
#[derive(Clone, Debug)]
pub struct EquivCfg {
    /// The function to compare, by name, in both modules.
    pub entry: String,
    pub budget: Budget,
    /// The tier-2 backend, or `None` for tier 1 alone.
    ///
    /// Discovery is a *runtime* fact (022 §4) and chiero never links a solver, so this is
    /// a value a caller supplies rather than a feature flag. [`EquivCfg::new`] discovers
    /// one the way the engine's own default does.
    pub backend: Option<SmtLib>,
}

impl EquivCfg {
    /// Discovers a backend, as `SolverTier::Discover` does.
    pub fn new(entry: impl Into<String>) -> EquivCfg {
        EquivCfg {
            entry: entry.into(),
            budget: Budget::default(),
            backend: SmtLib::discover(),
        }
    }

    /// Tier 1 only — so a test of what tier 1 can and cannot decide says what it means
    /// regardless of whether z3 happens to be installed.
    pub fn lite(entry: impl Into<String>) -> EquivCfg {
        EquivCfg {
            entry: entry.into(),
            budget: Budget::default(),
            backend: None,
        }
    }
}

/// What §1.1 calls the observable footprint, as far as this comparison went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Footprint {
    /// The claims that were actually decided. A reader who sees `Equivalent` needs this to
    /// know *what* was proven equal, and 041 §1.1 lists three separable things.
    pub compared: Vec<Claim>,
}

/// One of §1.1's three observables.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    ReturnValue,
    Memory,
    SideEffects,
    /// Whether the two sides ended the same way at all — §1.1's "abnormal termination".
    Termination,
}

impl Claim {
    pub fn label(self) -> &'static str {
        match self {
            Claim::ReturnValue => "return value",
            Claim::Memory => "caller-visible memory",
            Claim::SideEffects => "side-effect sequence",
            Claim::Termination => "termination",
        }
    }
}

/// How the two sides were seen to disagree — 041 §1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Divergence {
    ReturnValue {
        before: BvConst,
        after: BvConst,
    },
    /// 041 §1's `Memory { object, offset, before, after }`. Not produced yet; the variant
    /// exists so the enum is the spec's, and a `Memory` difference currently surfaces as
    /// `Unknown` rather than as a wrong `Equivalent`.
    Memory {
        object: String,
        offset: u64,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    SideEffect {
        index: u32,
        before: Option<String>,
        after: Option<String>,
    },
    Termination {
        before: TermReason,
        after: TermReason,
    },
}

/// The verdict — 041 §1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Equivalence {
    Equivalent {
        /// **Only `Exact` is a proof** (§1.2). `Bounded` is a statement about inputs
        /// within the loop bound, and [032](../../../docs/specs/032-test-selection.md)
        /// §3.1 must not accept it.
        fidelity: Fidelity,
        footprint: Footprint,
        assumptions: Vec<Assumption>,
    },
    Differs {
        input: Witness,
        observation: Divergence,
        /// 041 §1.3's compiled replay harness. Not built yet — `None` says so, where an
        /// empty `Replay` would claim a harness ran and demonstrated nothing.
        replay: Option<Replay>,
    },
    Unknown {
        reason: String,
    },
}

/// 041 §1.3's replay harness. A placeholder with no constructor: nothing can mint one
/// until the harness is actually compiled and run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    _private: (),
}

/// An input one side has and this comparison could not match to the other side's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unmatched {
    pub label: String,
}

/// Adjudicate two versions of one function — 041 §1.
pub fn prove_equivalent(_before: &Module, _after: &Module, _cfg: &EquivCfg) -> Equivalence {
    Equivalence::Unknown {
        reason: "prove_equivalent is not implemented".to_string(),
    }
}

// Silence the unused-import warnings while the body is a stub; the RED commit is about the
// suite failing, not about the imports being tidy.
#[allow(dead_code)]
fn _keep_imports_alive(
    _: &Engine<'_>,
    _: &State,
    _: &Binding,
    _: &InputOrigin,
    _: &Value,
    _: &Model,
    _: &PathCondition,
    _: &Term,
    _: &TermArena,
    _: &TieredSolver,
    _: &CheckResult,
    _: &UnknownReason,
    _: &Unmatched,
) {
}
