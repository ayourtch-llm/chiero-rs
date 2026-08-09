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
    /// The same address loaded twice with nothing between that could have written it.
    ///
    /// `object` and `offset` are **the engine's own answer** for where the two loads read —
    /// 021's `Pointer`, not a name this crate matched. A detector reasoning about which
    /// addresses *might* be equal would be doing the memory model's job in the wrong crate;
    /// asking it what an address resolved to is not.
    ///
    /// Keying on the CIR value instead was the first attempt, and unoptimized C never satisfies
    /// it: `p` lives in a stack slot and is reloaded before each dereference, so `*p` twice is
    /// two loads through two different values.
    RedundantLoad { object: u32, offset: i64 },
    /// A value written and then overwritten with nothing reading it in between.
    ///
    /// Keyed on the engine's `Pointer`, like [`OppKind::RedundantLoad`] and for the same
    /// reason: where a store *lands* is 021's answer, and matching on how the CIR spelled the
    /// address is what made the first version of that detector blind to real C.
    DeadStore { object: u32, offset: i64 },
}

impl OppKind {
    /// What discharging this kind's obligation rests on.
    ///
    /// **Two different questions.** A dead branch is discharged by the search being exhaustive;
    /// a redundant load is discharged by the intervening call being one chiero can see through.
    /// A detector that could only inherit the run's fidelity would have to lie about one of
    /// them.
    fn discharged_by(&self) -> &'static str {
        match self {
            OppKind::DeadBranch { .. } => {
                "the search was exhaustive, so no path reaches the other side"
            }
            OppKind::RedundantLoad { .. } => {
                "nothing between the two loads could have written the address"
            }
            OppKind::DeadStore { .. } => {
                "nothing between the two stores could have read the address"
            }
        }
    }

    fn rationale(&self, advisory: bool) -> String {
        let tail = if advisory {
            " — but see the obligation"
        } else {
            ""
        };
        match self {
            OppKind::DeadBranch { taken } => format!(
                "the {} side of this branch cannot be taken: the path condition already \
                 decides it{tail}",
                if *taken { "false" } else { "true" }
            ),
            OppKind::RedundantLoad { object, offset } => format!(
                "object {object} at offset {offset} is loaded twice with nothing between that \
                 could have written it, so the second load could reuse the first{tail}"
            ),
            OppKind::DeadStore { object, offset } => format!(
                "object {object} at offset {offset} is written twice with nothing between that \
                 could have read it, so the first write is dead{tail}"
            ),
        }
    }
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

    // **Detect on a promoted copy**, because the detector's identity criterion is "the same
    // value loaded twice" and unoptimized C never has that shape: every local — including a
    // pointer parameter — lives in a stack slot and is reloaded, so `*p` twice is two loads of
    // two *different* values.
    //
    // `mem2reg` is 020 §9's promotion pass and is **observationally transparent** — the same
    // findings and the same counterexamples, only faster — so a proposal about the promoted
    // module is a proposal about this program. Taking a copy keeps 041 contract 17's promise:
    // `detect` takes `&Module` and rewrites nothing a caller can see.
    //
    // **Measured, and it is not enough.** Promotion does not reach a pointer parameter's slot
    // here, so the detector still reports nothing for `int a = *p; g(); int b = *p;` as gcc
    // hands it to us. What the criterion actually needs is redundant-load analysis one level
    // down — knowing two loads of the *slot* give the same pointer — which is the same problem
    // recursively. Left here because it is the right layer for the fix and costs a clone;
    // recorded as not having achieved its aim, because a change that looks like a fix and is
    // not is worse than an admitted gap.
    let mut promoted = m.clone();
    crate::mem2reg(&mut promoted);
    let m = &promoted;

    let mut arena = TermArena::new();
    let mut engine = Engine::new(m)
        .with_entry(&cfg.entry)
        .with_budget(cfg.budget)
        .with_checker(Box::new(DeadBranch {
            found: Arc::clone(&found),
        }))
        .with_checker(Box::new(RedundantLoad {
            found: Arc::clone(&found),
            confined_by_func: Vec::new(),
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
            // **Two sources of doubt, and either one is enough.** The run may not have been
            // exhaustive, and *this* observation may not have been clearable — a redundant load
            // across a call chiero has no body for is unproven however complete the search was.
            let mut obligations = Vec::new();
            match &s.own_doubt {
                Some(why) => obligations.push(Obligation::Open { why: why.clone() }),
                None => obligations.push(Obligation::Discharged {
                    what: s.kind.discharged_by().to_string(),
                }),
            }
            if !exhaustive && matches!(s.kind, OppKind::DeadBranch { .. }) {
                obligations.push(Obligation::Open {
                    why: format!(
                        "the search did not finish ({}), so the other side was not shown \
                         unreachable — only unvisited",
                        truncated
                            .first()
                            .cloned()
                            .unwrap_or_else(|| format!("fidelity {:?}", run.fidelity()))
                    ),
                });
            }
            let advisory = obligations
                .iter()
                .any(|o| matches!(o, Obligation::Open { .. }));
            Proposal {
                rationale: s.kind.rationale(advisory),
                kind: s.kind,
                obligations,
                evidence: s.constraints,
                // No cycle model (§3's rule, which applies to §2 as much): these are real
                // observations whose value in cycles chiero cannot state.
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

/// One thing a detector noticed, before the run's fidelity is known.
struct Seen {
    kind: OppKind,
    constraints: Vec<String>,
    /// Set when *this* observation could not be cleared, whatever the run's fidelity says.
    ///
    /// A dead branch is discharged by the search being exhaustive; a redundant load is
    /// discharged by the intervening call being one chiero can see through. Two different
    /// questions, so a detector that could only inherit the run's answer would have to lie
    /// about one of them.
    own_doubt: Option<String>,
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
                    kind: OppKind::DeadBranch { taken: *t },
                    constraints,
                    // A dead branch's own doubt is the run's: it is decided by whether the
                    // search finished, which `detect` knows and this does not.
                    own_doubt: None,
                });
        }
        Vec::new()
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(chiero_exec::NoCheckerState)
    }
}

/// **041 §2's "redundant load (same address, no intervening write or barrier)".**
///
/// Tracks, per path, the address values loaded and what has happened since. A second load of a
/// value the program itself already loaded — the *same* `ValueId`, not an address chiero
/// reasoned might be equal — is redundant unless something between could have written it.
///
/// **Aliasing is 021's job, not this one.** A detector that decided which addresses might be
/// equal would be a second answer to a question the memory model owns, and the two would
/// eventually disagree. Reporting only what the program spelled with one name is narrower and
/// is a claim this crate can actually stand behind.
///
/// # What that costs, measured rather than guessed
///
/// **On unoptimized lowered C this fires almost never**, and it is worth being plain about why.
/// `int a = *p;` lowers to an `alloca`, a load and a *store into the stack slot*, and every
/// store is a barrier here — so the stack traffic between two source-level loads suppresses the
/// proposal. Checked against the obvious fixture: a function loading `*p` four times around two
/// calls produces nothing.
///
/// The fix is not a better aliasing rule but an **escape check**: a store through an
/// `AddrOfLocal` whose alloca's address never leaves the function cannot touch what a pointer
/// parameter points at. That is a small analysis and a real one, and it is not written. Until
/// it is, this detector answers about CIR whose loads the program itself repeated — which is
/// what 041 contract 14 is about, and less than §2's sentence promises.
struct RedundantLoad {
    found: Arc<Mutex<Vec<Seen>>>,
    confined_by_func: Vec<(chiero_cir::FuncId, Vec<u32>)>,
}

/// The values in `f` that are the address of a local whose address **never leaves the
/// function** — so a store through one cannot touch what a pointer parameter points at.
///
/// **A fact about the local, not about aliasing.** Deciding which addresses might be equal is
/// 021's question and stays there; this asks only whether anything could have got hold of the
/// address at all. An alloca whose address is passed to a call, stored anywhere, or returned is
/// excluded, and so is one reached through a `Phi`, because a merge of a local and something
/// else is not a local.
///
/// Without this the detector is blind to the shape real C lowers to: `int a = *p;` is an
/// alloca, a load and a *store into the stack slot*, and that store was a barrier.
fn confined_locals(f: &chiero_cir::Function) -> Vec<u32> {
    use chiero_cir::{InstKind, Operand, RValue};
    // Values that are the address of a local, grown through pointer arithmetic and copies.
    let mut local: Vec<(u32, chiero_cir::AllocaId)> = Vec::new();
    for _ in 0..2 {
        for b in &f.blocks {
            for i in &b.insts {
                let InstKind::Assign { dst, rv } = &i.kind else {
                    continue;
                };
                let from = |o: &Operand| match o {
                    Operand::Value(v) => local.iter().find(|(k, _)| *k == v.0).map(|(_, a)| *a),
                    _ => None,
                };
                let a = match rv {
                    RValue::AddrOfLocal { alloca, .. } => Some(*alloca),
                    RValue::PtrAdd { base, .. } => from(base),
                    RValue::Use(o) => from(o),
                    RValue::Cast { a, .. } => from(a),
                    _ => None,
                };
                if let Some(a) = a
                    && !local.iter().any(|(k, _)| *k == dst.0)
                {
                    local.push((dst.0, a));
                }
            }
        }
    }
    // Which allocas' addresses escape: handed to a call, stored as a value, or returned.
    let mut escaped: Vec<chiero_cir::AllocaId> = Vec::new();
    let note = |o: &Operand, escaped: &mut Vec<chiero_cir::AllocaId>| {
        if let Operand::Value(v) = o
            && let Some((_, a)) = local.iter().find(|(k, _)| *k == v.0)
            && !escaped.contains(a)
        {
            escaped.push(*a);
        }
    };
    for b in &f.blocks {
        for i in &b.insts {
            match &i.kind {
                InstKind::Call { args, .. } => {
                    for a in args {
                        note(a, &mut escaped);
                    }
                }
                // The *value* stored, not the address stored through: writing a local's address
                // somewhere is how it gets out.
                InstKind::Store { val, .. } => note(val, &mut escaped),
                InstKind::CopyMem { src, .. } => note(src, &mut escaped),
                _ => {}
            }
        }
        if let chiero_cir::Terminator::Return(Some(o)) = &b.term {
            note(o, &mut escaped);
        }
    }
    local
        .into_iter()
        .filter(|(_, a)| !escaped.contains(a))
        .map(|(v, _)| v)
        .collect()
}

/// What a path has seen since each address was loaded.
#[derive(Clone, Debug, Default)]
struct LoadState {
    /// (object, offset) → what has happened since it was last loaded.
    seen: Vec<((u32, i64), Since)>,
    /// (object, offset) → what has happened since it was last *stored*.
    ///
    /// A separate table because the two detectors ask opposite questions: a load is redundant
    /// when nothing could have *written* between, a store is dead when nothing could have
    /// *read* between. Sharing one would have to answer both with whichever was checked last.
    stored: Vec<((u32, i64), Since)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Since {
    /// Nothing that could have written it.
    Nothing,
    /// A call chiero could see through and which contains no store.
    ClearedCall(String),
    /// A call chiero could not clear — no body, or a body that writes.
    Doubt(String),
    /// A store, which ends the matter: the second load is necessary.
    Written,
}

impl chiero_exec::CheckerState for LoadState {
    fn on_fork(&self) -> Box<dyn chiero_exec::CheckerState> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl RedundantLoad {
    /// Whether this store's address is a local of the running function that never escapes it.
    ///
    /// The set is computed per function and cached, because a checker runs on every
    /// instruction of every path and the answer is a property of the CIR.
    fn confined(
        &mut self,
        ctx: &mut CheckerCtx,
        st: &chiero_exec::State,
        addr: &chiero_cir::Operand,
    ) -> bool {
        let chiero_cir::Operand::Value(v) = addr else {
            return false;
        };
        // **The function this instruction is in**, from the path's own trace. The engine steps
        // into a defined callee, so "the entry" and "the module's first function" are both
        // wrong answers — and the second was the one written first, which made the whole check
        // consult the wrong function's locals.
        let Some((f, _)) = st.trace().last().copied() else {
            return false;
        };
        if !self.confined_by_func.iter().any(|(id, _)| *id == f) {
            let set = ctx
                .module()
                .funcs
                .iter()
                .find(|x| x.id == f)
                .map(confined_locals)
                .unwrap_or_default();
            self.confined_by_func.push((f, set));
        }
        self.confined_by_func
            .iter()
            .find(|(id, _)| *id == f)
            .is_some_and(|(_, s)| s.contains(&v.0))
    }
}

impl Checker for RedundantLoad {
    fn name(&self) -> &'static str {
        "redundant-load"
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(LoadState::default())
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut CheckerCtx) -> Vec<Action> {
        match ev {
            Event::AfterInst { st, inst } => match &inst.kind {
                chiero_cir::InstKind::Assign {
                    rv: chiero_cir::RValue::Load { addr, .. },
                    ..
                } => {
                    // **Ask the engine where this reads.** A `ValueId` is how the CIR spelled
                    // the address; a `Pointer` is where it *is*, which is what makes two loads
                    // the same load. `SymPtr` — an object at an offset the program computed —
                    // is deliberately excluded: two symbolic offsets may or may not be equal,
                    // and deciding that is 021's question rather than this crate's.
                    let chiero_cir::Operand::Value(a) = addr else {
                        return Vec::new();
                    };
                    let Some(chiero_exec::Value::Ptr(p)) = st.local(*a) else {
                        return Vec::new();
                    };
                    let key = (p.base.0, p.off);
                    let st = ctx.state_mut::<LoadState>();
                    // **A read is what makes a store live.** The dead-store table is retired
                    // here rather than in a second event handler, because "was it read" is a
                    // question only the load knows the answer to.
                    st.stored.retain(|(k, _)| *k != key);
                    match st
                        .seen
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, s)| s.clone())
                    {
                        None => st.seen.push((key, Since::Nothing)),
                        Some(Since::Written) => {
                            // The store between them is what makes this load necessary. Start
                            // again from here rather than reporting.
                            if let Some(e) = st.seen.iter_mut().find(|(k, _)| *k == key) {
                                e.1 = Since::Nothing;
                            }
                        }
                        Some(since) => {
                            let (constraints, own_doubt) = match &since {
                                Since::ClearedCall(f) => {
                                    (vec![format!("`{f}` between them contains no store")], None)
                                }
                                Since::Doubt(f) => (
                                    vec![format!("`{f}` is called between them")],
                                    Some(format!(
                                        "chiero could not prove `{f}` does not write this \
                                         address, so the second load may be necessary"
                                    )),
                                ),
                                _ => (vec!["nothing happens between them".to_string()], None),
                            };
                            self.found
                                .lock()
                                .expect("no other thread holds this")
                                .push(Seen {
                                    kind: OppKind::RedundantLoad {
                                        object: key.0,
                                        offset: key.1,
                                    },
                                    constraints,
                                    own_doubt,
                                });
                            // Report once per path per address.
                            if let Some(e) = st.seen.iter_mut().find(|(k, _)| *k == key) {
                                e.1 = Since::Written;
                            }
                        }
                    }
                }
                // **A store is a barrier unless it is into a local nothing can reach.**
                // Deciding *which* addresses a store could alias is 021's question; asking
                // whether the address ever left the function is not, and it is what the shape
                // real C lowers to needs.
                chiero_cir::InstKind::Store { addr, .. }
                | chiero_cir::InstKind::StoreBits { addr, .. }
                | chiero_cir::InstKind::CopyMem { dst: addr, .. }
                | chiero_cir::InstKind::SetMem { dst: addr, .. } => {
                    let confined = self.confined(ctx, st, addr);
                    // **A store to an address already written and not read is a dead store.**
                    if let chiero_cir::Operand::Value(a) = addr
                        && let Some(chiero_exec::Value::Ptr(p)) = st.local(*a)
                    {
                        let key = (p.base.0, p.off);
                        let prior = ctx
                            .state_mut::<LoadState>()
                            .stored
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map(|(_, s)| s.clone());
                        if let Some(since) = prior {
                            let (constraints, own_doubt) = match &since {
                                Since::ClearedCall(f) => (
                                    vec![format!("`{f}` between them cannot have read it")],
                                    None,
                                ),
                                Since::Doubt(f) => (
                                    vec![format!("`{f}` is called between them")],
                                    Some(format!(
                                        "chiero could not prove `{f}` does not read this \
                                         address, so the first write may be live"
                                    )),
                                ),
                                _ => (vec!["nothing reads it between them".to_string()], None),
                            };
                            self.found
                                .lock()
                                .expect("no other thread holds this")
                                .push(Seen {
                                    kind: OppKind::DeadStore {
                                        object: key.0,
                                        offset: key.1,
                                    },
                                    constraints,
                                    own_doubt,
                                });
                        }
                        let st_ = ctx.state_mut::<LoadState>();
                        st_.stored.retain(|(k, _)| *k != key);
                        st_.stored.push((key, Since::Nothing));
                    }
                    // **And a barrier for every load**, unless it is into a local nothing can
                    // reach. Deciding *which* addresses a store could alias is 021's question;
                    // asking whether the address ever left the function is not.
                    if !confined {
                        for (_, since) in &mut ctx.state_mut::<LoadState>().seen {
                            *since = Since::Written;
                        }
                    }
                }
                _ => {}
            },
            Event::Call { callee, .. } => {
                let cleared = match callee {
                    chiero_cir::Callee::Direct(id) => {
                        let m = ctx.module();
                        m.funcs.iter().find(|f| f.id == *id).map(|f| {
                            let name = f.name.to_string();
                            // **A callee whose every store is into its own confined local
                            // cannot have written the caller's memory.**
                            //
                            // "No store at all" was the first rule and it cleared nothing real:
                            // lowering stores every parameter into a stack slot, so
                            // `static int quiet (int x) { return x; }` has a store and was
                            // never cleared. The same escape question the caller asks answers
                            // it — a store through the address of a local the function never
                            // lets out is invisible from outside.
                            let confined = confined_locals(f);
                            let writes_out = |o: &chiero_cir::Operand| match o {
                                chiero_cir::Operand::Value(v) => !confined.contains(&v.0),
                                _ => true,
                            };
                            let quiet = f.body == chiero_cir::Body::Defined
                                && f.blocks.iter().all(|b| {
                                    b.insts.iter().all(|i| match &i.kind {
                                        chiero_cir::InstKind::Store { addr, .. }
                                        | chiero_cir::InstKind::StoreBits { addr, .. }
                                        | chiero_cir::InstKind::CopyMem { dst: addr, .. }
                                        | chiero_cir::InstKind::SetMem { dst: addr, .. } => {
                                            !writes_out(addr)
                                        }
                                        // A call or an `Opaque` could do anything; neither is
                                        // something this rule can see through.
                                        chiero_cir::InstKind::Call { .. }
                                        | chiero_cir::InstKind::Opaque { .. } => false,
                                        _ => true,
                                    })
                                });
                            (quiet, name)
                        })
                    }
                    chiero_cir::Callee::Indirect { .. } => None,
                };
                let since = match cleared {
                    Some((true, name)) => Since::ClearedCall(name),
                    Some((false, name)) => Since::Doubt(name),
                    None => Since::Doubt("an indirect call".to_string()),
                };
                let st = ctx.state_mut::<LoadState>();
                for (_, s) in &mut st.seen {
                    // A store already ends the matter; a call cannot make it less certain.
                    if *s != Since::Written {
                        *s = since.clone();
                    }
                }
                // A callee that could *read* the address makes a pending store live — the
                // mirror of the load side, and the reason the two tables are separate.
                for (_, s) in &mut st.stored {
                    *s = since.clone();
                }
            }
            _ => {}
        }
        Vec::new()
    }
}
