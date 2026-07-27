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
}

impl Default for Budget {
    fn default() -> Budget {
        Budget {
            max_depth: 10_000,
            max_loop_iters: 8,
            max_recursion_depth: 32,
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
}

#[derive(Clone, Debug)]
pub struct State {
    pub id: StateId,
    pub mem: Memory,
    pub pc: (BlockId, usize),
    /// Append-only (023 §1).
    pub path: Vec<Term>,
    pub fidelity: Fidelity,
    pub assumptions: Vec<Assumption>,
    pub status: Status,
    /// **An explicit stack, not host recursion.** 023 contract 9 requires the
    /// interpreter's own stack usage to be O(1) in program recursion depth, and this is
    /// what delivers it — along with honest `Span` backtraces and a recursion bound that
    /// is a counter rather than a heuristic.
    stack: Vec<Frame>,
    ret: Option<Value>,
    frame_objs: IndexMap<AllocaId, chiero_mem::ObjectId>,
    /// How often each back edge has been taken on *this* path.
    edge_counts: IndexMap<(BlockId, BlockId), u32>,
    steps: u32,
}

impl State {
    pub fn object_size_for_test(&self) -> Option<u64> {
        self.frame_objs
            .values()
            .next()
            .and_then(|o| self.mem.size_of_pub(*o))
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

#[derive(Debug)]
pub struct Proven<'a> {
    pub result: &'a RunResult,
}

/// **The only function in the workspace that reads a run's fidelity to decide whether a
/// result may be presented as a proof** (023 §7.1).
///
/// It consumes the witness and checks it belongs to *this* run, so a token minted from a
/// trivial `return 0` cannot bless a different one even though both are `Exact`.
pub fn seal(r: &RunResult, w: ExactWitness) -> Result<Proven<'_>, NotProven> {
    if w.run == r.id && r.fidelity() == Fidelity::Exact {
        Ok(Proven { result: r })
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

#[derive(Debug)]
pub struct RunResult {
    pub id: u32,
    pub states: Vec<State>,
    pub solver_calls: u64,
    /// 022 §4 wants this at one for a whole run. A per-query spawn shows up immediately.
    pub backend_spawns: u64,
    /// How many solvers the run built. One, or the caches are discarded between queries.
    pub solver_inits: u64,
}

impl RunResult {
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

    /// A witness exists only for an `Exact` run.
    pub fn witness(&self) -> Option<ExactWitness> {
        (self.fidelity() == Fidelity::Exact).then_some(ExactWitness { run: self.id })
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
    /// **One solver for the run.** A fresh one per query spawned a process per
    /// escalation and threw away the cache each time, so its hit rate was structurally
    /// zero — and 023 §1.1's argument that sibling states hit the caches constantly was
    /// describing something that could not happen.
    solver: Option<TieredSolver>,
    solver_inits: u64,
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
            backend: None,
            solver: None,
            solver_inits: 0,
        }
    }

    pub fn with_solver(mut self, t: SolverTier) -> Self {
        self.tier = t;
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
        let f = &self.module.funcs[0];
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
            stack: vec![Frame {
                func: f.id,
                ret_to: None,
                ret_dst: None,
                locals: IndexMap::new(),
            }],
            ret: None,
            frame_objs,
            edge_counts: IndexMap::new(),
            steps: 0,
        };
        // Depth-first with the true branch first, so fork order is deterministic (§3) and
        // 001 §5's determinism requirement is met by construction rather than by luck.
        let mut work = vec![start];
        let mut done = Vec::new();
        while let Some(mut s) = work.pop() {
            while s.status == Status::Running {
                if let Some(forked) = self.step(a, &mut s) {
                    // The sibling goes on the stack; this state carries on, which is what
                    // makes the true branch complete first.
                    work.push(forked);
                }
            }
            done.push(s);
        }
        done.sort_by_key(|s| s.id.0);
        let backend_spawns = self.solver.as_ref().map_or(0, |s| s.stats().backend_spawns);
        RunResult {
            id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            states: done,
            solver_calls: self.solver_calls,
            backend_spawns,
            solver_inits: self.solver_inits,
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
        _args: &[Operand],
        span: Span,
    ) {
        let Callee::Direct(id) = callee else {
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                span,
                "indirect call resolution is not implemented",
            );
            return;
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
            // No registry yet, so every extern is unmodeled. **A fresh value, never a
            // zero**: silently returning 0 is the same confidently-wrong failure as
            // reading uninitialized memory as zero, one level up.
            if let Some(d) = dst {
                self.fresh_count += 1;
                let t = a.var(sort_of(&ret_ty), &format!("extern{}", self.fresh_count));
                s.set_local(d, Value::Scalar(t));
            }
            s.degrade(
                Fidelity::Approximated,
                AssumptionKind::UnmodeledCall,
                span,
                &format!("`{name}` has no body and no model"),
            );
            if noreturn {
                s.status = Status::Terminated(TermReason::Return);
            }
            return;
        }
        if noreturn {
            s.status = Status::Terminated(TermReason::Return);
            return;
        }
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
        let ret_to = Some((s.func(), s.pc.0, s.pc.1 + 1));
        s.stack.push(Frame {
            func: *id,
            ret_to,
            ret_dst: dst,
            locals: IndexMap::new(),
        });
        s.pc = (f.entry, 0);
        // `step` increments the index after this returns, which would skip the callee's
        // first instruction — so back it off by one.
        s.pc.1 = usize::MAX;
    }

    fn exec_term(&mut self, a: &mut TermArena, s: &mut State, t: &Terminator) -> Option<State> {
        match t {
            Terminator::Return(v) => {
                let val = v.as_ref().and_then(|o| self.operand(a, s, o));
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
            let n = s.edge_counts.entry((from, to)).or_insert(0);
            *n += 1;
            if *n > self.budget.max_loop_iters {
                s.status = Status::Terminated(TermReason::Budget);
                s.degrade(
                    Fidelity::Bounded,
                    AssumptionKind::BudgetHit,
                    Span::DUMMY,
                    &format!(
                        "max_loop_iters ({}) reached on the back edge {:?} -> {:?}",
                        self.budget.max_loop_iters, from, to
                    ),
                );
                return None;
            }
        }
        s.pc = (to, 0);
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
                let Some(base) = s.frame_objs.get(alloca).copied() else {
                    return self.lowering_gap(s, span, "an alloca with no object");
                };
                Value::Ptr(Pointer { base, off: 0 })
            }
            // 023 §7's table puts a `LoweringGap` under **`Unknown`**, not
            // `Approximated`: an unimplemented lowering is not a modeling lie, it is the
            // engine not knowing.
            other => return self.lowering_gap(s, span, &format!("{other:?}")),
        })
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
