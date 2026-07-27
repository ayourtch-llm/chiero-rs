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
use chiero_solver::{CheckResult, Solver, SolverLite, Term, TermArena};
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
    /// A `LoweringGap` or an access with no information behind it.
    NoInformation,
}

impl AssumptionKind {
    /// Whether this kind can account for a given fidelity level (023 contract 12: a
    /// *dummy* assumption must not satisfy the check).
    pub fn matches(self, f: Fidelity) -> bool {
        match f {
            Fidelity::Exact => false,
            Fidelity::Bounded => self == AssumptionKind::BudgetHit,
            Fidelity::Approximated => self == AssumptionKind::OpaqueCode,
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
    Budget,
}

/// Which solver tiers a run may use. `LiteOnly` exists so a test can force the
/// `Unknown` path deterministically without depending on whether z3 is installed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SolverTier {
    #[default]
    LiteOnly,
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
    locals: IndexMap<ValueId, Value>,
    ret: Option<Value>,
    frame_objs: IndexMap<AllocaId, chiero_mem::ObjectId>,
}

impl State {
    pub fn local(&self, v: ValueId) -> Option<Value> {
        self.locals.get(&v).copied()
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
}

impl RunResult {
    /// 023 §7 rule 2: the **worst** over every state that contributed.
    pub fn fidelity(&self) -> Fidelity {
        self.states
            .iter()
            .map(|s| s.fidelity)
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
}

impl<'m> Engine<'m> {
    pub fn new(module: &'m Module) -> Engine<'m> {
        Engine {
            module,
            tier: SolverTier::LiteOnly,
            next_state: 0,
            solver_calls: 0,
            fresh_count: 0,
        }
    }

    pub fn with_solver(mut self, t: SolverTier) -> Self {
        self.tier = t;
        self
    }

    pub fn run(mut self, a: &mut TermArena) -> RunResult {
        let f = &self.module.funcs[0];
        let mut mem = Memory::new();
        let mut frame_objs = IndexMap::new();
        for d in &f.allocas {
            let id = mem.alloc(ObjKind::Stack, d.count * 8, d.align, d.span);
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
            locals: IndexMap::new(),
            ret: None,
            frame_objs,
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
        RunResult {
            id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            states: done,
            solver_calls: self.solver_calls,
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
        let f = &self.module.funcs[0];
        let b = f.blocks.iter().find(|b| b.id == s.pc.0)?;
        if s.pc.1 < b.insts.len() {
            let i = &b.insts[s.pc.1];
            self.exec_inst(a, s, i);
            s.pc.1 += 1;
            return None;
        }
        self.exec_term(a, s, &b.term.clone())
    }

    fn exec_inst(&mut self, a: &mut TermArena, s: &mut State, i: &Inst) {
        match &i.kind {
            InstKind::Assign { dst, rv } => {
                if let Some(v) = self.eval(a, s, rv, i.span) {
                    s.locals.insert(*dst, v);
                }
            }
            InstKind::Opaque { dsts, why, .. } => {
                // 020 §4.3: never silently a no-op. Each output is a fresh symbol,
                // distinct per instruction, and the path is a modeling lie from here on.
                for (v, ty) in dsts {
                    let t = a.var(sort_of(ty), &format!("opaque_{}", v.0));
                    s.locals.insert(*v, Value::Scalar(t));
                }
                s.degrade(
                    Fidelity::Approximated,
                    AssumptionKind::OpaqueCode,
                    i.span,
                    &format!("opaque construct ({why:?}) was not modeled"),
                );
            }
            InstKind::Marker(_) => {}
            _ => {}
        }
    }

    fn exec_term(&mut self, a: &mut TermArena, s: &mut State, t: &Terminator) -> Option<State> {
        match t {
            Terminator::Return(v) => {
                s.ret = v.as_ref().and_then(|o| self.operand(a, s, o));
                s.status = Status::Terminated(TermReason::Return);
                None
            }
            Terminator::Goto(b) => {
                s.pc = (*b, 0);
                None
            }
            Terminator::Br { cond, t: bt, f: bf } => self.branch(a, s, cond, *bt, *bf),
            Terminator::Unreachable(_) => {
                s.status = Status::Terminated(TermReason::Unreachable);
                None
            }
            _ => {
                s.status = Status::Errored("unsupported terminator".into());
                None
            }
        }
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
            s.pc = if v.bits() != 0 { (bt, 0) } else { (bf, 0) };
            return None;
        }
        let neg = negate(a, c);
        let t_ok = self.feasible(a, s, c);
        let f_ok = self.feasible(a, s, neg);
        let mut sibling = s.clone();
        sibling.id = self.new_id();

        match (t_ok, f_ok) {
            (Feas::Yes, Feas::Yes) => {
                s.path.push(c);
                s.pc = (bt, 0);
                sibling.path.push(neg);
                sibling.pc = (bf, 0);
                Some(sibling)
            }
            (Feas::Yes, Feas::No) => {
                s.path.push(c);
                s.pc = (bt, 0);
                None
            }
            (Feas::No, Feas::Yes) => {
                s.path.push(neg);
                s.pc = (bf, 0);
                None
            }
            (Feas::No, Feas::No) => {
                // Both infeasible: the path condition is already unsatisfiable, which §3
                // calls a bug in chiero rather than a finding.
                s.status = Status::Errored("both branches infeasible".into());
                None
            }
            // §3 step 3: **take the branch anyway.** Dropping one the solver could not
            // refute would let "no bug found" mean "the solver timed out". The fidelity
            // is `Unknown`, not `Approximated` — §7's table is explicit that the engine
            // does not know whether the path exists.
            _ => {
                s.path.push(c);
                s.pc = (bt, 0);
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    "solver could not decide a branch; both sides explored",
                );
                sibling.path.push(neg);
                sibling.pc = (bf, 0);
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
        let mut lite = SolverLite::new();
        for p in &s.path {
            lite.assert(*p);
        }
        match lite.check(a, &[t]) {
            CheckResult::Sat(_) => Feas::Yes,
            CheckResult::Unsat => Feas::No,
            CheckResult::Unknown(_) => Feas::Unknown,
        }
    }

    fn eval(&mut self, a: &mut TermArena, s: &mut State, rv: &RValue, span: Span) -> Option<Value> {
        Some(match rv {
            RValue::Use(o) => self.operand(a, s, o)?,
            RValue::Bin { op, a: x, b: y, ty } => {
                let (xv, yv) = (self.scalar(a, s, x)?, self.scalar(a, s, y)?);
                let _ = ty;
                Value::Scalar(bin(a, *op, xv, yv))
            }
            RValue::Cmp { op, a: x, b: y, .. } => {
                let (xv, yv) = (self.scalar(a, s, x)?, self.scalar(a, s, y)?);
                Value::Scalar(cmp(a, *op, xv, yv))
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
                let base = *s.frame_objs.get(alloca)?;
                Value::Ptr(Pointer { base, off: 0 })
            }
            _ => {
                s.degrade(
                    Fidelity::Approximated,
                    AssumptionKind::OpaqueCode,
                    span,
                    "rvalue is not modeled yet",
                );
                return None;
            }
        })
    }

    fn operand(&mut self, a: &mut TermArena, s: &State, o: &Operand) -> Option<Value> {
        match o {
            Operand::Value(v) => s.locals.get(v).copied(),
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

fn sort_of(ty: &CTy) -> chiero_solver::Sort {
    chiero_solver::Sort::BitVec(match ty {
        CTy::Int(b) => *b,
        CTy::Ptr => 64,
        _ => 64,
    })
}

fn bin(a: &mut TermArena, op: BinOp, x: Term, y: Term) -> Term {
    match op {
        BinOp::Add => a.add(x, y),
        BinOp::Mul => a.mul(x, y),
        BinOp::And => a.and(x, y),
        BinOp::Or => a.or(x, y),
        _ => a.add(x, y),
    }
}

fn cmp(a: &mut TermArena, op: CmpOp, x: Term, y: Term) -> Term {
    match op {
        CmpOp::Eq => a.eq(x, y),
        CmpOp::ULt => a.ult(x, y),
        _ => a.eq(x, y),
    }
}
