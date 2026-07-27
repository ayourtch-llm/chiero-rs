//! `chiero-exec` — the symbolic execution engine (023).
//!
//! Two decisions shape everything here.
//!
//! **A local is a `Value`, not a `Term`.** A pointer keeps its `ObjectId`, because
//! 021 §7 puts guard gaps between objects precisely so an out-of-bounds pointer resolves
//! to *no* object — so recovering a base by searching the address space is lossy by
//! construction, and round-tripping a pointer through a term would turn a detectable OOB
//! into a wild pointer.
//!
//! **Fidelity only degrades, and a negative result is only a proof at `Exact`.** The
//! engine is allowed to be incomplete; it is not allowed to *claim* completeness it does
//! not have.

use chiero_cir::*;
use chiero_mem::{Memory, ObjKind, Pointer};
use chiero_model::{AllocPolicy, ModelCtx, ModelOutcome, ModelRegistry, Precision, StringPolicy};
use chiero_solver::{CheckResult, SmtLib, Solver, Term, TermArena, TieredSolver};
use chiero_span::Span;
use indexmap::IndexMap;

/// 023 §7. **Ordered, worst wins.** The table in §7 is normative; this enum is its
/// encoding and nothing else may restate it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fidelity {
    /// The explored region was explored completely and every solver answer was definite.
    Exact,
    /// Exploration was cut by a documented budget. Findings are real; absence of findings
    /// proves nothing beyond the bound.
    Bounded,
    /// Something was **modeled imprecisely** — a deliberate lie about semantics, not a
    /// truncation of search.
    Approximated,
    /// The engine **does not know** and cannot bound its ignorance.
    Unknown,
}

impl Fidelity {
    /// Degrade towards the worse of the two. Never restores: 023 §7 rule 1.
    pub fn degrade(self, other: Fidelity) -> Fidelity {
        self.max(other)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AssumptionKind {
    /// A solver `Unknown` on a decision that mattered.
    SolverUnknown,
    /// `Opaque`/inline asm, or any other deliberate modeling lie.
    OpaqueCode,
    /// A documented budget was hit.
    BudgetHit,
    /// A call to a function with no body and no model. 023 §5 forbids silently
    /// returning 0 — that is the same confidently-wrong failure as reading uninitialized
    /// memory as zero, one level up.
    UnmodeledCall,
    /// A model declared `Precision::Approximate` was dispatched. 024 §2.1 makes this
    /// mechanical: the *modeled* imprecise path is more dangerous than the unmodeled one
    /// because it looks deliberate.
    ModelApproximate,
    /// A `LoweringGap` or an access with no information behind it.
    NoInformation,
}

impl AssumptionKind {
    /// Whether this kind can account for `Approximated` specifically.
    fn is_modeling_lie(self) -> bool {
        matches!(
            self,
            AssumptionKind::OpaqueCode | AssumptionKind::UnmodeledCall
        )
    }

    /// Whether this kind can account for a given fidelity level (023 contract 12: a
    /// *dummy* assumption must not satisfy the check).
    pub fn matches(self, f: Fidelity) -> bool {
        match f {
            Fidelity::Exact => false,
            Fidelity::Bounded => self == AssumptionKind::BudgetHit,
            Fidelity::Approximated => self.is_modeling_lie(),
            Fidelity::Unknown => {
                matches!(
                    self,
                    AssumptionKind::SolverUnknown | AssumptionKind::NoInformation
                )
            }
        }
    }
}

/// 023 §7 rule 3: every degradation names its cause. "Approximated with no reason" is a
/// bug, so this is recorded at the point of degradation, never after the fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assumption {
    pub kind: AssumptionKind,
    pub span: Span,
    pub detail: String,
}

/// 023 §1.1. A pointer keeps its object; a scalar is a term.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Scalar(Term),
    Ptr(Pointer),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StateId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Terminated(TermReason),
    Errored(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TermReason {
    Return,
    Unreachable,
    /// 023 §8: a *documented* budget was exceeded. Findings on this state are real;
    /// absence of findings proves nothing beyond the bound.
    Budget,
}

/// 023 §8. Only the deterministic budgets are here; `wall_clock` is a non-deterministic
/// abort that §8.1 keeps out of anything that gates output, because 001 §5 requires
/// byte-identical results and a timeout is not reproducible.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_depth: u32,
    /// Per **back edge**, not per syntactic loop — CIR has none (020 §1).
    pub max_loop_iters: u32,
    /// Per `FuncId` in the active stack (023 §5).
    pub max_recursion_depth: u32,
    pub max_states: u32,
    pub max_forks: u32,
    /// Candidates explored at one indirect call site (023 §5).
    pub max_indirect: u32,
}

impl Default for Budget {
    fn default() -> Budget {
        Budget {
            max_depth: 10_000,
            max_loop_iters: 8,
            max_recursion_depth: 32,
            max_states: 10_000,
            max_forks: 10_000,
            max_indirect: 16,
        }
    }
}

/// Which solver tiers a run may use. `LiteOnly` exists so a test can force the
/// `Unknown` path deterministically without depending on whether z3 is installed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SolverTier {
    #[default]
    LiteOnly,
}

/// One activation. Locals live here, not in the state, so a callee cannot overwrite its
/// caller's values — which is silent when it happens, since both are well-typed.
#[derive(Clone, Debug)]
pub struct Frame {
    pub func: FuncId,
    ret_to: Option<(FuncId, BlockId, usize)>,
    ret_dst: Option<ValueId>,
    locals: IndexMap<ValueId, Value>,
    /// **Per activation** (023 §1). `AllocaId` is unique only *within* a function, so
    /// one map per state made a callee's local *be* the caller's object of the same id —
    /// writes aliasing and bounds wrong in both directions, silently. Recursion has the
    /// same shape: each activation needs its own objects.
    frame_objs: IndexMap<AllocaId, chiero_mem::ObjectId>,
}

#[derive(Clone, Debug)]
pub struct State {
    pub id: StateId,
    pub mem: Memory,
    pub pc: (BlockId, usize),
    /// Append-only (023 §1).
    pub path: Vec<Term>,
    /// **Private.** A public field here let a consumer set a genuinely degraded run to
    /// `Exact` and have `seal` bless it — §7.1's claim rests on this not being writable.
    fidelity: Fidelity,
    assumptions: Vec<Assumption>,
    pub status: Status,
    /// 023 §1: the block sequence, for replay and so exploration order is observable at
    /// all. Sorting the result by id erased it from the output entirely.
    /// **`(FuncId, BlockId)`**, because a trace of bare block ids reads as one function
    /// walking a path it never took — a caller in `b0` calling a callee that goes
    /// `b0 -> b1` renders as `main: 0 -> 1`, and cannot be replayed.
    trace: Vec<(FuncId, BlockId)>,
    /// **An explicit stack, not host recursion.** 023 contract 9 requires the
    /// interpreter's own stack usage to be O(1) in program recursion depth, and this is
    /// what delivers it — along with honest `Span` backtraces and a recursion bound that
    /// is a counter rather than a heuristic.
    stack: Vec<Frame>,
    ret: Option<Value>,
    /// How often each back edge has been taken on *this* path, keyed by **function** too
    /// (023 §8) — two functions that both loop `b2 -> b1` are not one loop.
    edge_counts: IndexMap<(FuncId, BlockId, BlockId), u32>,
    steps: u32,
    /// What the models reported on this path. Kept on the state so a fork carries only
    /// what it actually saw.
    findings: Vec<String>,
}

impl State {
    pub fn object_size_for_test(&self) -> Option<u64> {
        self.stack
            .last()?
            .frame_objs
            .values()
            .next()
            .and_then(|o| self.mem.size_of_pub(*o))
    }

    /// The loop-counter keys, so a test can see that two functions with identical block
    /// numbering get two counters rather than one. The *note* names the function from
    /// `s.func()`, which is a different source than the key — so asserting on the note
    /// cannot tell a per-function key from a global one.
    pub fn loop_keys_for_test(&self) -> Vec<(FuncId, BlockId, BlockId)> {
        self.edge_counts.keys().copied().collect()
    }

    /// Every live activation's alloca sizes, so a test can see that two frames have two
    /// objects rather than one shared by id.
    pub fn alloca_sizes_for_test(&self) -> Vec<u64> {
        self.stack
            .iter()
            .flat_map(|f| f.frame_objs.values())
            .filter_map(|o| self.mem.size_of_pub(*o))
            .collect()
    }

    fn errored(id: StateId, why: &str) -> State {
        State {
            id,
            mem: Memory::new(),
            pc: (BlockId(0), 0),
            path: Vec::new(),
            fidelity: Fidelity::Unknown,
            assumptions: vec![Assumption {
                kind: AssumptionKind::NoInformation,
                span: Span::DUMMY,
                detail: why.to_string(),
            }],
            status: Status::Errored(why.to_string()),
            trace: Vec::new(),
            stack: Vec::new(),
            ret: None,
            edge_counts: IndexMap::new(),
            steps: 0,
            findings: Vec::new(),
        }
    }

    pub fn fidelity(&self) -> Fidelity {
        self.fidelity
    }

    pub fn assumptions(&self) -> &[Assumption] {
        &self.assumptions
    }

    pub fn findings(&self) -> &[String] {
        &self.findings
    }

    pub fn trace(&self) -> &[(FuncId, BlockId)] {
        &self.trace
    }

    pub fn local(&self, v: ValueId) -> Option<Value> {
        self.stack.last()?.locals.get(&v).copied()
    }

    fn set_local(&mut self, v: ValueId, x: Value) {
        if let Some(f) = self.stack.last_mut() {
            f.locals.insert(v, x);
        }
    }

    fn func(&self) -> FuncId {
        self.stack.last().map_or(FuncId(0), |f| f.func)
    }

    /// The returned value as concrete bits, when it is one.
    pub fn return_value_bits(&self, a: &mut TermArena) -> Option<u128> {
        match self.ret? {
            Value::Scalar(t) => a.eval_ground(t).ok().map(|c| c.bits()),
            Value::Ptr(_) => None,
        }
    }

    fn degrade(&mut self, to: Fidelity, kind: AssumptionKind, span: Span, detail: &str) {
        self.fidelity = self.fidelity.degrade(to);
        self.assumptions.push(Assumption {
            kind,
            span,
            detail: detail.to_string(),
        });
    }
}

/// 023 §7.1. Non-`Clone`, private field, constructible only here — so no downstream crate
/// can forge a proof. It does not stop this crate from minting one on a degraded run;
/// that remains one ordinary `if`, concentrated in `RunResult::witness` so it can be
/// reviewed and tested rather than scattered.
#[derive(Debug)]
pub struct ExactWitness {
    run: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotProven {
    pub fidelity: Fidelity,
    pub assumptions: Vec<Assumption>,
}

/// **Not constructible outside this crate.** All fields public and no marker meant a
/// struct literal was a second route to a `Proven` — so `seal` was not the only way to
/// get one, and a downstream crate could produce a proof for a run of any fidelity
/// without a witness or a runtime check.
#[derive(Debug)]
pub struct Proven<'a> {
    result: &'a RunResult,
    _seal: Sealed,
}

impl<'a> Proven<'a> {
    pub fn result(&self) -> &'a RunResult {
        self.result
    }
}

/// **The only function in the workspace that reads a run's fidelity to decide whether a
/// result may be presented as a proof** (023 §7.1).
///
/// It consumes the witness and checks it belongs to *this* run, so a token minted from a
/// trivial `return 0` cannot bless a different one even though both are `Exact`.
pub fn seal(r: &RunResult, w: ExactWitness) -> Result<Proven<'_>, NotProven> {
    if w.run == r.id && r.fidelity() == Fidelity::Exact {
        Ok(Proven {
            result: r,
            _seal: Sealed,
        })
    } else {
        Err(NotProven {
            fidelity: r.fidelity(),
            assumptions: r
                .states
                .iter()
                .flat_map(|s| s.assumptions.clone())
                .collect(),
        })
    }
}

/// **Not constructible outside this crate.** A hand-built `RunResult` carrying a real
/// run's id and no states used to seal as `Proven`; the private field is what stops the
/// struct literal.
#[derive(Debug)]
pub struct RunResult {
    id: u32,
    states: Vec<State>,
    pub solver_calls: u64,
    /// 022 §4 wants this at one for a whole run. A per-query spawn shows up immediately.
    pub backend_spawns: u64,
    /// How many solvers the run built. One, or the caches are discarded between queries.
    pub solver_inits: u64,
    /// The order states *finished*, so a change of searcher is visible in the output
    /// rather than hidden by sorting.
    completion_order: Vec<u32>,
    /// 023 §8: reported whether or not it was hit, so a reader can tell
    /// `Exact`-with-generous-bounds from `Exact`-with-trivial-bounds.
    budget: Budget,
    _seal: Sealed,
}

#[derive(Debug)]
struct Sealed;

impl RunResult {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn states(&self) -> &[State] {
        &self.states
    }

    pub fn completion_order(&self) -> &[u32] {
        &self.completion_order
    }

    pub fn budget(&self) -> Budget {
        self.budget
    }

    /// Everything the models reported, across every state.
    pub fn findings(&self) -> Vec<String> {
        self.states
            .iter()
            .flat_map(|s| s.findings.clone())
            .collect()
    }

    /// 023 §7 rule 2: the **worst** over every state that contributed.
    pub fn fidelity(&self) -> Fidelity {
        self.states
            .iter()
            .map(|s| match s.status {
                // An errored state did not finish, so nothing about it is exact — and a
                // run containing one must not mint a proof.
                Status::Errored(_) => Fidelity::Unknown,
                _ => s.fidelity,
            })
            .fold(Fidelity::Exact, Fidelity::degrade)
    }

    /// A witness bound to *this* run. Minting is unconditional: 023 §7.1 wants exactly
    /// **one** function reading fidelity to decide whether a result is a proof, and that
    /// function is `seal`. Gating here as well made `seal`'s own check unreachable, which
    /// is why contract 13b could not be written.
    pub fn witness(&self) -> ExactWitness {
        ExactWitness { run: self.id }
    }
}

/// Run ids are distinct so a witness cannot bless a different run (023 §7.1). This does
/// not affect determinism, which is about the `StateId` sequence *within* a run.
static NEXT_RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[derive(Debug)]
pub struct Engine<'m> {
    module: &'m Module,
    tier: SolverTier,
    next_state: u32,
    solver_calls: u64,
    fresh_count: u32,
    budget: Budget,
    backend: Option<SmtLib>,
    models: ModelRegistry,
    alloc_policy: AllocPolicy,
    string_policy: StringPolicy,
    /// **One solver for the run.** A fresh one per query spawned a process per
    /// escalation and threw away the cache each time, so its hit rate was structurally
    /// zero — and 023 §1.1's argument that sibling states hit the caches constantly was
    /// describing something that could not happen.
    solver: Option<TieredSolver>,
    solver_inits: u64,
    /// States created mid-instruction — an indirect call makes several at once, and
    /// `step` returns at most one.
    pending: Vec<State>,
    forks: u32,
}

impl<'m> Engine<'m> {
    pub fn new(module: &'m Module) -> Engine<'m> {
        Engine {
            module,
            tier: SolverTier::LiteOnly,
            next_state: 0,
            solver_calls: 0,
            fresh_count: 0,
            budget: Budget::default(),
            models: ModelRegistry::with_builtins(),
            alloc_policy: AllocPolicy::default(),
            string_policy: StringPolicy::default(),
            backend: None,
            solver: None,
            solver_inits: 0,
            pending: Vec::new(),
            forks: 0,
        }
    }

    pub fn with_solver(mut self, t: SolverTier) -> Self {
        self.tier = t;
        self
    }

    /// Replace the model registry — how `chiero-vpp` supplies vppinfra models without
    /// this crate knowing anything about them (024 §8).
    pub fn with_models(mut self, m: ModelRegistry) -> Self {
        self.models = m;
        self
    }

    /// 024 §3: an allocator that aborts instead of returning `NULL` registers with
    /// `may_fail = false`, which is how `chiero-vpp` will model `clib_mem_alloc`.
    pub fn with_alloc_policy(mut self, p: AllocPolicy) -> Self {
        self.alloc_policy = p;
        self
    }

    pub fn with_budget(mut self, b: Budget) -> Self {
        self.budget = b;
        self
    }

    /// Attach a tier-2 backend. Without one every branch tier 1 cannot decide degrades
    /// the run to `Unknown` — honest, but nearly useless on ordinary code.
    pub fn with_backend(mut self, b: SmtLib) -> Self {
        self.backend = Some(b);
        self
    }

    pub fn run(mut self, a: &mut TermArena) -> RunResult {
        // **An empty module is an error, not a panic.** `Module::default()` is what three
        // of the four proof-surface probes construct.
        let Some(f) = self.module.funcs.first() else {
            let s = State::errored(self.new_id(), "the module defines no functions");
            return RunResult {
                id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                states: vec![s],
                solver_calls: 0,
                backend_spawns: 0,
                solver_inits: 0,
                completion_order: vec![0],
                budget: self.budget,
                _seal: Sealed,
            };
        };
        let mut mem = Memory::new();
        let mut frame_objs = IndexMap::new();
        for d in &f.allocas {
            // Sized by the **element type**. `count * 8` made `char buf[4]` a 32-byte
            // object and writes at offsets 4..31 fault-free — eight times too permissive
            // for exactly the buffers overflows happen in. A dynamic extent is a
            // saturating zero here; `AllocaDyn` supplies the real size at a program point.
            let elem = size_of_cty(&d.ty);
            let bytes = if d.count == chiero_cir::DYNAMIC_EXTENT {
                0
            } else {
                d.count.saturating_mul(elem)
            };
            let id = mem.alloc(ObjKind::Stack, bytes, d.align, d.span);
            frame_objs.insert(d.id, id);
        }
        let start = State {
            id: self.new_id(),
            mem,
            pc: (f.entry, 0),
            path: Vec::new(),
            fidelity: Fidelity::Exact,
            assumptions: Vec::new(),
            status: Status::Running,
            trace: vec![(f.id, f.entry)],
            stack: vec![Frame {
                func: f.id,
                ret_to: None,
                ret_dst: None,
                locals: IndexMap::new(),
                frame_objs,
            }],
            ret: None,
            edge_counts: IndexMap::new(),
            steps: 0,
            findings: Vec::new(),
        };
        // Depth-first with the true branch first, so fork order is deterministic (§3) and
        // 001 §5's determinism requirement is met by construction rather than by luck.
        let mut work = vec![start];
        let mut done = Vec::new();
        while let Some(mut s) = work.pop() {
            while s.status == Status::Running {
                let forked = self.step(a, &mut s);
                // An indirect call creates several siblings at once, which `step` cannot
                // return.
                let mut new: Vec<State> = self.pending.drain(..).collect();
                new.extend(forked);
                for f in new {
                    self.forks += 1;
                    if self.forks > self.budget.max_forks {
                        // The *unexplored* sibling is dropped, and the state that
                        // survives says so — silently truncating would report "no bug
                        // found" over paths nobody walked.
                        s.degrade(
                            Fidelity::Bounded,
                            AssumptionKind::BudgetHit,
                            Span::DUMMY,
                            &format!("max_forks ({}) reached", self.budget.max_forks),
                        );
                        continue;
                    }
                    // The sibling goes on the stack; this state carries on, which is what
                    // makes the true branch complete first.
                    work.push(f);
                }
                if done.len() + work.len() > self.budget.max_states as usize {
                    s.status = Status::Terminated(TermReason::Budget);
                    s.degrade(
                        Fidelity::Bounded,
                        AssumptionKind::BudgetHit,
                        Span::DUMMY,
                        &format!("max_states ({}) reached", self.budget.max_states),
                    );
                }
            }
            done.push(s);
        }
        // The order states *finished* is recorded before sorting, so a change of searcher
        // shows up in the output instead of being erased by the sort.
        let completion_order: Vec<u32> = done.iter().map(|s| s.id.0).collect();
        done.sort_by_key(|s| s.id.0);
        let backend_spawns = self.solver.as_ref().map_or(0, |s| s.stats().backend_spawns);
        RunResult {
            id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            states: done,
            solver_calls: self.solver_calls,
            backend_spawns,
            solver_inits: self.solver_inits,
            completion_order,
            budget: self.budget,
            _seal: Sealed,
        }
    }

    fn new_id(&mut self) -> StateId {
        let id = StateId(self.next_state);
        self.next_state += 1;
        id
    }

    /// One instruction. Total: it advances, forks, or terminates — there is no
    /// partially-executed instruction, which is what makes serialization tractable (§2).
    fn step(&mut self, a: &mut TermArena, s: &mut State) -> Option<State> {
        let cur = s.func();
        let Some(f) = self.module.funcs.iter().find(|f| f.id == cur) else {
            s.status = Status::Errored(format!("no such function {cur:?}"));
            return None;
        };
        // **`step` is total** (023 §2). Returning `None` here without setting a status
        // left the run loop spinning forever — no allocation, so not even the OOM killer
        // would end it.
        let Some(b) = f.blocks.iter().find(|b| b.id == s.pc.0) else {
            s.status = Status::Errored(format!("no such block {:?}", s.pc.0));
            return None;
        };
        if s.pc.1 < b.insts.len() {
            // §8 counts `max_depth` in **instructions**. Counting edges left a
            // straight-line path of any length unbounded.
            s.steps += 1;
            if s.steps > self.budget.max_depth {
                s.status = Status::Terminated(TermReason::Budget);
                s.degrade(
                    Fidelity::Bounded,
                    AssumptionKind::BudgetHit,
                    b.span,
                    "max_depth reached",
                );
                return None;
            }
            let i = &b.insts[s.pc.1].clone();
            self.exec_inst(a, s, i);
            // A call re-points `pc` at the callee; anything else advances within the
            // block. `usize::MAX` is the sentinel a call leaves behind so this lands on 0.
            s.pc.1 = s.pc.1.wrapping_add(1);
            return None;
        }
        self.exec_term(a, s, &b.term.clone())
    }

    fn exec_inst(&mut self, a: &mut TermArena, s: &mut State, i: &Inst) {
        match &i.kind {
            InstKind::Assign { dst, rv } => {
                // A dropped assignment is a silent hole, so `eval` degrades on its way to
                // returning `None` and the local simply stays unbound.
                if let Some(v) = self.eval(a, s, rv, i.span) {
                    s.set_local(*dst, v);
                }
            }
            InstKind::Opaque { dsts, why, .. } => {
                // 020 §4.3: never silently a no-op. Each output is a fresh symbol,
                // distinct per instruction, and the path is a modeling lie from here on.
                for (v, ty) in dsts {
                    let t = a.var(sort_of(ty), &format!("opaque_{}", v.0));
                    s.set_local(*v, Value::Scalar(t));
                }
                s.degrade(
                    Fidelity::Approximated,
                    AssumptionKind::OpaqueCode,
                    i.span,
                    &format!("opaque construct ({why:?}) was not modeled"),
                );
            }
            InstKind::Call { dst, callee, args } => {
                self.call(a, s, *dst, callee, args, i.span);
            }
            InstKind::Marker(_) => {}
            other => {
                self.lowering_gap(s, i.span, &format!("{other:?}"));
            }
        }
    }

    /// 023 §5. Direct calls push a frame; externs consult the model registry, and with no
    /// model produce a fresh value rather than a silent zero.
    fn call(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        dst: Option<ValueId>,
        callee: &Callee,
        args: &[Operand],
        span: Span,
    ) {
        let id = match callee {
            Callee::Direct(id) => id,
            // 023 §5. VPP's node dispatch is indirect calls through registration tables,
            // so this is the ordinary path rather than an exotic one.
            Callee::Indirect(_) => {
                self.indirect(a, s, dst, span);
                return;
            }
        };
        let Some(f) = self.module.funcs.iter().find(|f| f.id == *id) else {
            s.status = Status::Errored("call to an unknown function".into());
            return;
        };
        let (noreturn, body, name, ret_ty) = (
            f.attrs.noreturn,
            f.body.clone(),
            f.name.clone(),
            f.ret.clone(),
        );

        if body == Body::Declared {
            // **The module's own definition always wins** — this branch is only reached
            // when there is no body. A registry that shadowed local definitions would
            // analyse a different program than the one on disk, most often exactly where
            // a project reimplemented a libc function for a reason.
            //
            // Every extern returns a fresh value, modeled or not: silently returning 0 is
            // the same confidently-wrong failure as reading uninitialized memory as zero,
            // one level up.
            if let Some(d) = dst {
                self.fresh_count += 1;
                let t = a.var(sort_of(&ret_ty), &format!("extern{}", self.fresh_count));
                s.set_local(d, Value::Scalar(t));
            }
            match self.models.lookup(&name).map(|e| e.precision.clone()) {
                // 024 §2.1: dispatching an approximate model degrades *mechanically*, and
                // the model's own reason travels into the report.
                Some(Precision::Approximate(why)) => {
                    self.note_once(
                        s,
                        AssumptionKind::ModelApproximate,
                        span,
                        &format!("`{name}` is modeled approximately: {why}"),
                    );
                }
                // **An exact model is only faithful if it actually runs.** Reading the
                // precision and recording nothing made a registered name *more* trusted
                // than an unregistered one: `strcpy` into a four-byte buffer finished
                // `Exact` and sealed, for a textbook overflow. Adding a correct model to
                // the library reduced safety, because the registration was read as a
                // claim about the *call* rather than about the model.
                Some(Precision::Exact) if self.can_dispatch(&name) => {
                    self.dispatch(a, s, &name, dst, args, span);
                }
                Some(Precision::Exact) => {
                    self.note_once(
                        s,
                        AssumptionKind::UnmodeledCall,
                        span,
                        &format!(
                            "`{name}` has an exact model, but the engine cannot dispatch \
                             it yet, so the call was not performed"
                        ),
                    );
                }
                None => {
                    self.note_once(
                        s,
                        AssumptionKind::UnmodeledCall,
                        span,
                        &format!("`{name}` has no body and no model"),
                    );
                }
            }
            if noreturn {
                s.status = Status::Terminated(TermReason::Return);
            }
            return;
        }
        // A `noreturn` function with a *body* still runs it: §5's rule is that the call
        // does not return, not that a body which exists is discarded. Skipping it made
        // every bug inside `__attribute__((noreturn)) void die(…) { … }` invisible.
        // The `ret_to: None` below is what makes the call not return.
        // 023 §5: bounded per `FuncId` in the *active stack*, so mutual recursion counts
        // against each participant rather than against a global depth.
        let depth = s.stack.iter().filter(|fr| fr.func == *id).count() as u32;
        if depth >= self.budget.max_recursion_depth {
            s.status = Status::Terminated(TermReason::Budget);
            s.degrade(
                Fidelity::Bounded,
                AssumptionKind::BudgetHit,
                span,
                &format!(
                    "max_recursion_depth ({}) reached in `{name}`",
                    self.budget.max_recursion_depth
                ),
            );
            return;
        }
        // The return address is the instruction *after* the call; `step` has not advanced
        // `pc` yet, so it is one past the current index.
        let ret_to = if noreturn {
            None
        } else {
            Some((s.func(), s.pc.0, s.pc.1 + 1))
        };
        // **Arguments reach the callee.** They were accepted and discarded, so
        // `int id(int x)` called with 42 returned nothing.
        let mut locals = IndexMap::new();
        for (p, o) in f.params.iter().zip(args.iter()) {
            if let Some(v) = self.operand(a, s, o) {
                locals.insert(p.value, v);
            }
        }
        // Each activation gets its **own** objects: `AllocaId` is unique only within a
        // function, so sharing them by id makes a callee's local be the caller's.
        let mut frame_objs = IndexMap::new();
        for d in &f.allocas.clone() {
            let elem = size_of_cty(&d.ty);
            let bytes = if d.count == chiero_cir::DYNAMIC_EXTENT {
                0
            } else {
                d.count.saturating_mul(elem)
            };
            let obj = s.mem.alloc(ObjKind::Stack, bytes, d.align, d.span);
            frame_objs.insert(d.id, obj);
        }
        s.stack.push(Frame {
            func: *id,
            ret_to,
            ret_dst: dst,
            locals,
            frame_objs,
        });
        s.pc = (f.entry, 0);
        s.trace.push((*id, f.entry));
        // `step` increments the index after this returns, which would skip the callee's
        // first instruction — so back it off by one.
        s.pc.1 = usize::MAX;
    }

    fn exec_term(&mut self, a: &mut TermArena, s: &mut State, t: &Terminator) -> Option<State> {
        match t {
            Terminator::Return(v) => {
                let val = match v {
                    None => None,
                    Some(o) => match self.operand(a, s, o) {
                        Some(x) => Some(x),
                        // The one consumer of `operand` that dropped `None` silently, so
                        // `ret %0` with `%0` unassigned sealed as a proof.
                        None => {
                            self.lowering_gap(s, Span::DUMMY, "a return operand");
                            None
                        }
                    },
                };
                // Returning from an inner frame resumes the caller; returning from the
                // outermost one ends the state.
                // The outermost frame is **not** popped: its locals are the result of the
                // run, and a terminated state whose locals have vanished can report
                // nothing about what it computed.
                if s.stack.len() > 1 {
                    let f = s.stack.pop().expect("checked");
                    if let Some((_, b, i)) = f.ret_to {
                        s.pc = (b, i);
                    }
                    if let (Some(d), Some(x)) = (f.ret_dst, val) {
                        s.set_local(d, x);
                    }
                } else {
                    s.ret = val;
                    s.status = Status::Terminated(TermReason::Return);
                }
                None
            }
            Terminator::Goto(b) => self.take_edge(s, *b),
            Terminator::Br { cond, t: bt, f: bf } => self.branch(a, s, cond, *bt, *bf),
            Terminator::Unreachable(why) => {
                // 020 §5: reaching a `LoweringGap` is `Fidelity::Unknown` and a
                // diagnostic — **never a licence to treat the path as infeasible**.
                // Discarding the reason turned "chiero could not lower this" into
                // "execution ended here", which is a proof-shaped answer to a question
                // nobody answered.
                if *why == UnreachableReason::LoweringGap {
                    s.degrade(
                        Fidelity::Unknown,
                        AssumptionKind::NoInformation,
                        Span::DUMMY,
                        "reached a lowering gap",
                    );
                }
                s.status = Status::Terminated(TermReason::Unreachable);
                None
            }
            _ => {
                s.status = Status::Errored("unsupported terminator".into());
                None
            }
        }
    }

    /// Move to `to`, counting the edge. 023 §8: the bound is per **back edge**, and CIR
    /// has no loops (020 §1) — an edge to a block already on this path is the back edge,
    /// which needs no dominator analysis to recognize at run time.
    fn take_edge(&mut self, s: &mut State, to: BlockId) -> Option<State> {
        let from = s.pc.0;
        if s.steps > self.budget.max_depth {
            s.status = Status::Terminated(TermReason::Budget);
            s.degrade(
                Fidelity::Bounded,
                AssumptionKind::BudgetHit,
                Span::DUMMY,
                "max_depth reached",
            );
            return None;
        }
        if to.0 <= from.0 {
            let fid = s.func();
            let n = s.edge_counts.entry((fid, from, to)).or_insert(0);
            *n += 1;
            if *n > self.budget.max_loop_iters {
                s.status = Status::Terminated(TermReason::Budget);
                s.degrade(
                    Fidelity::Bounded,
                    AssumptionKind::BudgetHit,
                    Span::DUMMY,
                    &format!(
                        "max_loop_iters ({}) reached on the back edge {from:?} -> {to:?} in `{}`",
                        self.budget.max_loop_iters,
                        self.module
                            .funcs
                            .iter()
                            .find(|f| f.id == fid)
                            .map_or("?", |f| &f.name),
                    ),
                );
                return None;
            }
        }
        s.pc = (to, 0);
        let fid = s.func();
        s.trace.push((fid, to));
        None
    }

    fn branch(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        cond: &Operand,
        bt: BlockId,
        bf: BlockId,
    ) -> Option<State> {
        let Some(Value::Scalar(c)) = self.operand(a, s, cond) else {
            s.status = Status::Errored("branch condition is not a scalar".into());
            return None;
        };
        // §3 step 4: a constant condition makes **no solver call**. This fast path carries
        // most of the traffic and must exist before any benchmark is believed.
        if let Ok(v) = a.eval_ground(c) {
            return self.take_edge(s, if v.bits() != 0 { bt } else { bf });
        }
        let neg = negate(a, c);
        let t_ok = self.feasible(a, s, c);
        let f_ok = self.feasible(a, s, neg);
        let mut sibling = s.clone();
        sibling.id = self.new_id();
        // The clone shares the trace up to here; each side records its own next block.

        match (t_ok, f_ok) {
            (Feas::Yes, Feas::Yes) => {
                s.path.push(c);
                self.take_edge(s, bt);
                sibling.path.push(neg);
                self.take_edge(&mut sibling, bf);
                Some(sibling)
            }
            (Feas::Yes, Feas::No) => {
                s.path.push(c);
                self.take_edge(s, bt)
            }
            (Feas::No, Feas::Yes) => {
                s.path.push(neg);
                self.take_edge(s, bf)
            }
            (Feas::No, Feas::No) => {
                // Both infeasible: the path condition is already unsatisfiable, which §3
                // calls a bug in chiero rather than a finding.
                s.status = Status::Errored("both branches infeasible".into());
                None
            }
            // One side refuted, the other undecided: explore **only** the undecided one.
            //
            // Currently unreachable from the test suite, and deliberately kept. Tier 1
            // decides the negation of a comparison whenever it decides the comparison, so
            // it produces `(No, Yes)` rather than `(No, Unknown)`; the combination arises
            // with a *backend* that gives up under an rlimit. The catch-all below would
            // take both edges, and one of them carries a path condition the solver
            // already refuted — a false finding with an impossible witness. A guard
            // against unsoundness is worth keeping before the case that triggers it is
            // routine.
            // Taking both would carry a path condition the solver had already proved
            // unsatisfiable, and any checker firing there produces a false finding with
            // an impossible witness. §3 says take a branch the solver *could not* refute.
            (Feas::No, Feas::Unknown) => {
                s.path.push(neg);
                self.take_edge(s, bf);
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    "solver could not decide the false branch; the true branch is refuted",
                );
                None
            }
            (Feas::Unknown, Feas::No) => {
                s.path.push(c);
                self.take_edge(s, bt);
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    "solver could not decide the true branch; the false branch is refuted",
                );
                None
            }
            // §3 step 3: **take the branch anyway.** Dropping one the solver could not
            // refute would let "no bug found" mean "the solver timed out". The fidelity
            // is `Unknown`, not `Approximated` — §7's table is explicit that the engine
            // does not know whether the path exists.
            _ => {
                s.path.push(c);
                self.take_edge(s, bt);
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    "solver could not decide a branch; both sides explored",
                );
                sibling.path.push(neg);
                self.take_edge(&mut sibling, bf);
                sibling.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    "solver could not decide a branch; both sides explored",
                );
                Some(sibling)
            }
        }
    }

    fn feasible(&mut self, a: &mut TermArena, s: &State, t: Term) -> Feas {
        self.solver_calls += 1;
        let _ = self.tier;
        // A fresh solver per query is wasteful and will be replaced by a per-state
        // incremental stack; correctness first, and the backend process itself is
        // long-lived (022 §4) so the cost is the assertion replay, not a spawn.
        if self.solver.is_none() {
            // Counted, because `backend_spawns` cannot distinguish one solver from many:
            // a freshly built solver reports one spawn for its own first query, which is
            // the same number the correct implementation reports for the whole run.
            self.solver_inits += 1;
            self.solver = Some(match self.backend.clone() {
                Some(b) => TieredSolver::with_backend(b),
                None => TieredSolver::new(),
            });
        }
        let solver = self.solver.as_mut().expect("just built");
        // The path condition goes in as *assumptions* rather than assertions, so the
        // solver's own stack stays empty between queries and sibling states share both
        // the process and the caches.
        let mut asks: Vec<Term> = s.path.clone();
        asks.push(t);
        match solver.check(a, &asks) {
            CheckResult::Sat(_) => Feas::Yes,
            CheckResult::Unsat => Feas::No,
            CheckResult::Unknown(_) => Feas::Unknown,
        }
    }

    fn eval(&mut self, a: &mut TermArena, s: &mut State, rv: &RValue, span: Span) -> Option<Value> {
        Some(match rv {
            RValue::Use(o) => match self.operand(a, s, o) {
                Some(v) => v,
                // `operand` returning `None` used to drop the assignment in silence; the
                // `Const` families it cannot represent — floats, wide vectors, global
                // addresses — are §7's `Approximated` and `Unknown` causes, not nothing.
                None => return self.lowering_gap(s, span, &format!("{o:?}")),
            },
            RValue::Bin { op, a: x, b: y, ty } => {
                let (Some(xv), Some(yv)) = (self.scalar(a, s, x), self.scalar(a, s, y)) else {
                    return self.lowering_gap(s, span, "a non-scalar arithmetic operand");
                };
                let _ = ty;
                match bin(a, *op, xv, yv) {
                    Some(t) => Value::Scalar(t),
                    None => return self.lowering_gap(s, span, &format!("{op:?}")),
                }
            }
            RValue::Cmp { op, a: x, b: y, .. } => {
                let (Some(xv), Some(yv)) = (self.scalar(a, s, x), self.scalar(a, s, y)) else {
                    return self.lowering_gap(s, span, "a non-scalar comparison operand");
                };
                match cmp(a, *op, xv, yv) {
                    Some(t) => Value::Scalar(t),
                    None => return self.lowering_gap(s, span, &format!("{op:?}")),
                }
            }
            RValue::Fresh { ty } => {
                // Named per state and per position, so two `Fresh` values are two
                // symbols and repeating one is not.
                self.fresh_count += 1;
                let t = a.var(sort_of(ty), &format!("fresh{}", self.fresh_count));
                Value::Scalar(t)
            }
            RValue::AddrOfLocal { alloca } => {
                // The local keeps the *object*, not an address — §1.1's whole point.
                let Some(base) = s
                    .stack
                    .last()
                    .and_then(|f| f.frame_objs.get(alloca))
                    .copied()
                else {
                    return self.lowering_gap(s, span, "an alloca with no object");
                };
                Value::Ptr(Pointer { base, off: 0 })
            }
            // 020 §4.1 keeps this distinct from `Add` so provenance survives the
            // arithmetic: the object comes through untouched and only the offset moves.
            // The offset is **signed** — vppinfra's vector header sits below the user
            // pointer, so a forward-only walk cannot express `vec_len(v)`.
            RValue::PtrAdd { base, off } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, base) else {
                    return self.lowering_gap(s, span, "PtrAdd on a non-pointer base");
                };
                let Some(t) = self.scalar(a, s, off) else {
                    return self.lowering_gap(s, span, "PtrAdd with a non-scalar offset");
                };
                let Ok(c) = a.eval_ground(t) else {
                    // A symbolic offset is 021 §7's territory and is owed; guessing a
                    // concrete one here would be a fabricated address.
                    return self.lowering_gap(s, span, "PtrAdd with a symbolic offset");
                };
                Value::Ptr(Pointer {
                    base: p.base,
                    off: p.off.wrapping_add(c.signed() as i64),
                })
            }
            // 023 §7's table puts a `LoweringGap` under **`Unknown`**, not
            // `Approximated`: an unimplemented lowering is not a modeling lie, it is the
            // engine not knowing.
            other => return self.lowering_gap(s, span, &format!("{other:?}")),
        })
    }

    /// Run a model and fold its result back into the state.
    ///
    /// The translation is the whole job: a model wants `Pointer`s and concrete sizes, and
    /// the engine has `Operand`s. Every argument is resolved **before** the context is
    /// built, because `ModelCtx` borrows memory and the arena for the duration.
    ///
    /// Anything that will not translate is a *gap* rather than a silent skip — the point
    /// of dispatch is to stop a registration standing in for a call.
    fn dispatch(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        name: &str,
        dst: Option<ValueId>,
        args: &[Operand],
        span: Span,
    ) {
        use chiero_model::models;

        if matches!(
            name,
            "chiero_assume" | "chiero_assert" | "chiero_mark_fidelity"
        ) {
            self.intrinsic(a, s, name, args, span);
            return;
        }

        // Resolve first: `ModelCtx` takes `&mut` on both memory and the arena.
        let resolved: Vec<Option<Value>> = args.iter().map(|o| self.operand(a, s, o)).collect();
        let ptr = |i: usize| match resolved.get(i) {
            Some(Some(Value::Ptr(p))) => Some(*p),
            _ => None,
        };
        let num = |i: usize, a: &TermArena| match resolved.get(i) {
            Some(Some(Value::Scalar(t))) => a.eval_ground(*t).ok().map(|c| c.bits() as u64),
            _ => None,
        };

        let (alloc, strp) = (self.alloc_policy, self.string_policy);
        let mut findings: Vec<String> = Vec::new();
        let mut result: Option<Value> = None;
        let mut forks: Vec<Option<Value>> = Vec::new();
        let mut translated = true;
        {
            let mut cx = ModelCtx::new(&mut s.mem, a, span, chiero_mem::Endian::Little);
            let out = match name {
                "malloc" => num(0, cx.arena()).map(|n| models::malloc(&mut cx, n, alloc)),
                "calloc" => match (num(0, cx.arena()), num(1, cx.arena())) {
                    (Some(n), Some(m)) => Some(models::calloc(&mut cx, n, m, alloc)),
                    _ => None,
                },
                "free" => ptr(0).map(|p| models::free(&mut cx, p)),
                "memcpy" => match (ptr(0), ptr(1), num(2, cx.arena())) {
                    (Some(d), Some(sp), Some(n)) => Some(models::memcpy(&mut cx, d, sp, n)),
                    _ => None,
                },
                "memmove" => match (ptr(0), ptr(1), num(2, cx.arena())) {
                    (Some(d), Some(sp), Some(n)) => Some(models::memmove(&mut cx, d, sp, n)),
                    _ => None,
                },
                "memset" => match (ptr(0), num(1, cx.arena()), num(2, cx.arena())) {
                    (Some(d), Some(b), Some(n)) => Some(models::memset(&mut cx, d, b as u8, n)),
                    _ => None,
                },
                "strlen" => ptr(0).map(|p| {
                    let r = models::strlen(&mut cx, p, strp);
                    match r {
                        chiero_model::StrScan::Exact(n) => {
                            let t = cx.arena().bv(64, n as u128);
                            chiero_model::ModelOutcome::Value(Some(chiero_model::Value::Scalar(t)))
                        }
                        // A length nobody established is not a number to hand back —
                        // and `Value(None)` was not the way to say so. It left
                        // `translated` true, so the `dst` fallback minted a *fresh
                        // unconstrained* symbol and the run stayed `Exact`. `Finding`
                        // carries the reason and marks the gap.
                        other => chiero_model::ModelOutcome::Finding(format!(
                            "strlen: no length was established ({other:?}), so the result \
                             is unknown"
                        )),
                    }
                }),
                "strcpy" => match (ptr(0), ptr(1)) {
                    (Some(d), Some(sp)) => Some(models::strcpy(&mut cx, d, sp, strp)),
                    _ => None,
                },
                _ => None,
            };
            match out {
                Some(ModelOutcome::Value(v)) => {
                    result = v.map(lift_value);
                }
                // 024 contract 1: `malloc`'s `NULL` branch is a real path, and the
                // *default*. Each alternative after the first becomes a sibling state;
                // this one takes the first, so exploration order stays deterministic.
                Some(ModelOutcome::Fork(branches)) => {
                    let mut it = branches.into_iter();
                    match it.next() {
                        Some((_, ModelOutcome::Value(v))) => {
                            result = v.map(lift_value);
                        }
                        _ => translated = false,
                    }
                    for (_, alt) in it {
                        if let ModelOutcome::Value(v) = alt {
                            forks.push(v.map(lift_value));
                        } else {
                            translated = false;
                        }
                    }
                }
                // The payload is the *whole point* of `Finding`; matching only `Value`
                // dropped it. It is still a gap — the call did not produce a value — so
                // `translated` stays false and the assumption is recorded too.
                Some(ModelOutcome::Finding(msg)) => {
                    findings.push(msg);
                    translated = false;
                }
                Some(_) => translated = false,
                None => translated = false,
            }
            findings.extend(cx.findings().iter().cloned());
        }

        for f in findings {
            s.findings.push(f);
        }
        if let (Some(d), Some(v)) = (dst, result) {
            s.set_local(d, v);
        } else if let Some(d) = dst
            && translated
        {
            // The model ran and produced no value; the caller still needs *something*,
            // and a fresh symbol is the honest one.
            self.fresh_count += 1;
            let t = a.var(
                chiero_solver::Sort::BitVec(64),
                &format!("model{}", self.fresh_count),
            );
            s.set_local(d, Value::Scalar(t));
        }
        // Siblings are created *after* this state's own result is in place, so each
        // carries the same history and differs only in what the model returned.
        for v in forks {
            let mut sib = s.clone();
            sib.id = self.new_id();
            if let (Some(d), Some(x)) = (dst, v) {
                sib.set_local(d, x);
            }
            // Past the call, for the same reason as `indirect`: a sibling that still
            // pointed *at* the call re-dispatched it and forked again, forever.
            sib.pc.1 = sib.pc.1.wrapping_add(1);
            self.pending.push(sib);
        }
        if !translated {
            self.note_once(
                s,
                AssumptionKind::UnmodeledCall,
                span,
                &format!("`{name}` could not be dispatched with these arguments"),
            );
        }
    }

    /// 024 §7. The harness intrinsics take no memory, so they are handled apart from the
    /// models that do.
    fn intrinsic(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        name: &str,
        args: &[Operand],
        span: Span,
    ) {
        use chiero_model::{IntrinsicOutcome, intrinsics};
        // `None` means the condition could not be decided here, which the two intrinsics
        // treat differently on purpose: `assume` constrains, `assert` reports. Hardcoding
        // "true" was the safe reading for `assume` and the *unsafe* one for `assert` —
        // every assertion in a harness passed.
        let cond = args.first().and_then(|o| match self.operand(a, s, o) {
            Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits() != 0),
            _ => None,
        });
        let out = match name {
            "chiero_assume" => intrinsics::assume(cond),
            "chiero_assert" => intrinsics::assert_(cond),
            _ => intrinsics::mark_fidelity("harness marked this region approximate"),
        };
        match out {
            IntrinsicOutcome::Continue | IntrinsicOutcome::Constrain => {}
            IntrinsicOutcome::KillState => {
                s.status = Status::Terminated(TermReason::Return);
            }
            IntrinsicOutcome::Finding(f) => s.findings.push(f),
            IntrinsicOutcome::Degrade(why) => {
                s.degrade(
                    Fidelity::Approximated,
                    AssumptionKind::ModelApproximate,
                    span,
                    &why,
                );
            }
        }
    }

    /// Whether the engine can actually *run* this model. Deliberately explicit and short:
    /// a model becomes dispatchable when the code to call it exists, not when it is
    /// registered, and conflating the two is what let a registration mint a proof.
    fn can_dispatch(&self, name: &str) -> bool {
        chiero_model::dispatchable().contains(&name)
    }

    /// An indirect call: fork per candidate, **plus one unresolvable state**.
    ///
    /// The extra state is not bookkeeping. Without it the candidate list is implicitly
    /// claimed exhaustive, so a function pointer that came from anywhere chiero has not
    /// looked is never explored — and the run still reports on the "complete" set.
    ///
    /// Candidates are every defined function whose signature could be called here. A
    /// real resolution against the pointer's value is owed; over-approximating is the
    /// safe direction, and the cap is what keeps it affordable.
    fn indirect(&mut self, _a: &mut TermArena, s: &mut State, dst: Option<ValueId>, span: Span) {
        let all: Vec<FuncId> = self
            .module
            .funcs
            .iter()
            .filter(|f| f.body == Body::Defined && f.id != s.func())
            .map(|f| f.id)
            .collect();
        let cap = self.budget.max_indirect as usize;
        let capped = all.len() > cap;
        let candidates: Vec<FuncId> = all.into_iter().take(cap).collect();
        if capped {
            s.degrade(
                Fidelity::Bounded,
                AssumptionKind::BudgetHit,
                span,
                &format!("max_indirect ({cap}) reached; further callees were not explored"),
            );
        }
        // Each candidate is a sibling state; *this* one becomes the unresolvable branch,
        // so it is always present however the forks are capped.
        for id in candidates {
            let mut sib = s.clone();
            sib.id = self.new_id();
            self.direct_into(&mut sib, id, dst, span);
            // A sibling is stepped straight from the work list, so it never passes
            // through `step`'s post-call increment — it has to arrive already advanced.
            // Without this the callee's entry block skipped to its terminator, which no
            // test could see while every candidate's entry block was empty.
            sib.pc.1 = sib.pc.1.wrapping_add(1);
            self.pending.push(sib);
        }
        s.degrade(
            Fidelity::Unknown,
            AssumptionKind::NoInformation,
            span,
            "unresolvable callee: the pointer may name a function chiero has not seen",
        );
        s.status = Status::Terminated(TermReason::Return);
    }

    /// Push a frame for `id` on an already-cloned state. Shares the body of `call`'s
    /// direct path so an indirect candidate is executed exactly like a direct call.
    fn direct_into(&mut self, s: &mut State, id: FuncId, dst: Option<ValueId>, span: Span) {
        let Some(f) = self.module.funcs.iter().find(|f| f.id == id) else {
            s.status = Status::Errored("call to an unknown function".into());
            return;
        };
        let mut frame_objs = IndexMap::new();
        for d in &f.allocas.clone() {
            let elem = size_of_cty(&d.ty);
            let bytes = if d.count == chiero_cir::DYNAMIC_EXTENT {
                0
            } else {
                d.count.saturating_mul(elem)
            };
            let obj = s.mem.alloc(ObjKind::Stack, bytes, d.align, d.span);
            frame_objs.insert(d.id, obj);
        }
        let ret_to = Some((s.func(), s.pc.0, s.pc.1 + 1));
        s.stack.push(Frame {
            func: id,
            ret_to,
            ret_dst: dst,
            locals: IndexMap::new(),
            frame_objs,
        });
        s.pc = (f.entry, 0);
        s.trace.push((id, f.entry));
        s.pc.1 = usize::MAX;
        let _ = span;
    }

    /// Degrade, recording the reason **once**. A call in a loop must not stack up one
    /// assumption per iteration and drown the finding it exists to explain — but each
    /// call still produces its own fresh value, since deduplicating the report must not
    /// deduplicate the values.
    fn note_once(&mut self, s: &mut State, kind: AssumptionKind, span: Span, detail: &str) {
        if s.assumptions
            .iter()
            .any(|x| x.kind == kind && x.detail == detail)
        {
            s.fidelity = s.fidelity.degrade(Fidelity::Approximated);
            return;
        }
        s.degrade(Fidelity::Approximated, kind, span, detail);
    }

    /// Record that something on this path was not modeled. **This is the one rule behind
    /// a family of holes**: no path may end at `Exact` unless everything on it was
    /// modeled, or an unexecuted program mints a proof — which §7 rule 4 says the crate
    /// must be structurally incapable of.
    fn lowering_gap(&mut self, s: &mut State, span: Span, what: &str) -> Option<Value> {
        s.degrade(
            Fidelity::Unknown,
            AssumptionKind::NoInformation,
            span,
            &format!("`{what}` is not modeled"),
        );
        None
    }

    fn operand(&mut self, a: &mut TermArena, s: &State, o: &Operand) -> Option<Value> {
        match o {
            Operand::Value(v) => s.stack.last().and_then(|f| f.locals.get(v)).copied(),
            Operand::Const(Const::Int { bits, val }) => {
                Some(Value::Scalar(a.bv(*bits, *val as u128)))
            }
            Operand::Const(Const::Null) => Some(Value::Ptr(Pointer {
                base: chiero_mem::ObjectId::NULL,
                off: 0,
            })),
            _ => None,
        }
    }

    fn scalar(&mut self, a: &mut TermArena, s: &State, o: &Operand) -> Option<Term> {
        match self.operand(a, s, o)? {
            Value::Scalar(t) => Some(t),
            Value::Ptr(_) => None,
        }
    }
}

enum Feas {
    Yes,
    No,
    Unknown,
}

fn lift_value(v: chiero_model::Value) -> Value {
    match v {
        chiero_model::Value::Ptr(p) => Value::Ptr(p),
        chiero_model::Value::Scalar(t) => Value::Scalar(t),
    }
}

fn negate(a: &mut TermArena, t: Term) -> Term {
    let zero = a.bv(a.width(t), 0);
    a.eq(t, zero)
}

/// 020 §2's widths. `Void` has no storage, and an aggregate is not an alloca element
/// type in CIR.
fn size_of_cty(t: &CTy) -> u64 {
    match t {
        CTy::Void => 0,
        CTy::Int(b) => (*b as u64).div_ceil(8),
        CTy::Float(FloatKind::F32) => 4,
        CTy::Float(FloatKind::F64) => 8,
        CTy::Float(FloatKind::X87_80) => 16,
        CTy::Ptr => 8,
        CTy::Vector { elem, lanes } => size_of_cty(elem) * *lanes as u64,
    }
}

fn sort_of(ty: &CTy) -> chiero_solver::Sort {
    chiero_solver::Sort::BitVec(match ty {
        CTy::Int(b) => *b,
        CTy::Ptr => 64,
        _ => 64,
    })
}

/// **No default.** An unimplemented operation returns `None` and the caller records a
/// `LoweringGap`; defaulting to addition made `5 - 3` come out `8` at `Fidelity::Exact`,
/// which is a wrong answer wearing a proof. 023 §2's "the mapping from CIR ops to solver
/// ops is 1:1 by construction" is only true if the map is total or says when it is not.
fn bin(a: &mut TermArena, op: BinOp, x: Term, y: Term) -> Option<Term> {
    Some(match op {
        BinOp::Add => a.add(x, y),
        BinOp::Sub => a.sub(x, y),
        BinOp::Mul => a.mul(x, y),
        BinOp::UDiv => a.udiv(x, y),
        BinOp::SDiv => a.sdiv(x, y),
        BinOp::URem => a.urem(x, y),
        BinOp::SRem => a.srem(x, y),
        BinOp::And => a.and(x, y),
        BinOp::Or => a.or(x, y),
        BinOp::Xor => a.xor(x, y),
        BinOp::Shl => a.shl(x, y),
        BinOp::LShr => a.lshr(x, y),
        BinOp::AShr => a.ashr(x, y),
        // Floats and pointer differences are not modeled yet, and saying so is the whole
        // point of this function returning an `Option`.
        _ => return None,
    })
}

fn cmp(a: &mut TermArena, op: CmpOp, x: Term, y: Term) -> Option<Term> {
    Some(match op {
        CmpOp::Eq => a.eq(x, y),
        // Defaulting `Ne` to `Eq` **inverted** it, so every `if (x != y)` took the
        // opposite branch — silently, and at `Exact`.
        CmpOp::Ne => {
            let e = a.eq(x, y);
            a.not(e)
        }
        CmpOp::ULt => a.ult(x, y),
        CmpOp::ULe => {
            let lt = a.ult(x, y);
            let e = a.eq(x, y);
            a.or(lt, e)
        }
        CmpOp::UGt => a.ult(y, x),
        CmpOp::UGe => {
            let gt = a.ult(y, x);
            let e = a.eq(x, y);
            a.or(gt, e)
        }
        CmpOp::SLt => a.slt(x, y),
        CmpOp::SLe => {
            let lt = a.slt(x, y);
            let e = a.eq(x, y);
            a.or(lt, e)
        }
        CmpOp::SGt => a.slt(y, x),
        CmpOp::SGe => {
            let gt = a.slt(y, x);
            let e = a.eq(x, y);
            a.or(gt, e)
        }
        _ => return None,
    })
}
