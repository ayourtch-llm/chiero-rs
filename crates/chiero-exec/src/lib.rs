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

mod report;
mod witness;
pub use report::render;
pub use witness::{Binding, InputOrigin, Witness};

use chiero_cir::*;
use chiero_mem::{Endian, HavocFill, Memory, ObjKind, ObjectId, Pointer};
use chiero_model::{
    AllocPolicy, HavocInit, HavocSpec, ModelCtx, ModelOutcome, ModelRegistry, Precision,
    StringPolicy,
};
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
            // **`ModelApproximate` belongs here**, and its absence made contract 12 false
            // of shipped code: `scanf` degrades a run to `Approximated` with exactly one
            // assumption, whose kind then accounted for nothing. 024 §2.1 calls the
            // modeled-imprecise path *more* dangerous than the unmodeled one because it
            // looks deliberate — so of the three it is the one that must not be missing.
            // Found by review.
            AssumptionKind::OpaqueCode
                | AssumptionKind::UnmodeledCall
                | AssumptionKind::ModelApproximate
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

/// 023 §6.1's deduplication key, minus the `checker` component the checker framework will
/// add. Two reports of one bug — the same fault, at the same place, about the same object
/// — are one finding however many times the path runs through them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FindingKey {
    kind: &'static str,
    span: Span,
    object: Option<chiero_mem::ObjectId>,
    /// **Where the access was, not just what it touched.** Three fault kinds —
    /// `NullDeref`, `WildPointer`, `BadRange` — have no object by construction, and a
    /// hand-written module has `Span::DUMMY` everywhere, so without this the key is
    /// identical for every one of them and two distinct bugs merge. A real frontend gives
    /// them distinct spans; the key must not depend on that to be correct.
    func: FuncId,
}

/// One report as the state recorded it. `Finding` is the public shape (023 §9); this is
/// what a state carries before the run is over and the witness exists.
#[derive(Clone, Debug)]
struct StateFinding {
    id: u64,
    key: Option<FindingKey>,
    message: String,
    span: Span,
}

/// A report the engine produced, with everything a reader needs to act on it —
/// 023 §9's `Finding`, minus the fields that need machinery that does not exist yet
/// (`backtrace`, `trace`, `object`, and the `FindingKind` enum a checker framework will
/// supply). Adding those is additive; what is here already has to be right.
#[derive(Clone, Debug)]
pub struct Finding {
    /// The report's identity — shared by every state descended from the one that made it,
    /// which is how a fork's copies are recognised as one report.
    pub id: u64,
    pub message: String,
    pub span: Span,
    /// 023 §9. `None` **only** with `unwitnessed` set (contract 15).
    pub witness: Option<Witness>,
    /// Why there is no witness. The absence is allowed; the silence is not.
    pub unwitnessed: Option<String>,
    /// The fidelity of the path this was found on — not the run's. A definite fault on an
    /// `Exact` path stays actionable in a run some *other* path degraded.
    pub fidelity: Fidelity,
}

/// How big an object an entry function's pointer parameter points at. The caller is
/// outside the analysis, so there is no right answer — this is a *bound chiero chose*, and
/// an access past it is reported as one rather than silently allowed.
pub const ENTRY_PARAM_BYTES: u64 = 4096;

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
    /// The program would not have continued: a null dereference, a use-after-free, a
    /// definite out-of-bounds access. Distinct from `Unsupported`, which is about chiero,
    /// and from `Unreachable`, which is a claim the *CIR* made.
    Crashed,
    /// The path met something chiero cannot follow and cannot bound — 024 contract 20's
    /// `longjmp`. Distinct from `Budget`, which is a limit chiero chose, and from
    /// `Unreachable`, which is a claim about the *program*.
    Unsupported,
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
    /// 021 §5.1: how many objects a symbolic base may resolve to before the engine
    /// concretizes instead of forking. Past it the run is `Approximated`, which is a
    /// different statement from the `Unknown` an *unconstrained* pointer gets.
    pub max_resolutions: u32,
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
            max_resolutions: 8,
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
    /// **Per activation, for the same reason as `locals`.** A `ValueId` is unique only
    /// *within* a function, so one map per state would let a callee's `%2` inherit the
    /// caller's provenance — and recursion has the same shape.
    ptr_vals: IndexMap<ValueId, Pointer>,
    /// The arguments past the declared parameters of a variadic call, in order. 020 §4.4.1
    /// puts the `va_list` in *memory* so `va_list *` can cross a function boundary; this
    /// is the argument area it iterates, which belongs to the activation that received it.
    varargs: Vec<Option<Value>>,
    /// How wide one lane of a vector-valued local is. `ExtractLane` and `InsertLane` carry
    /// no element type, and a total width cannot say how it is divided — 32 bits is four
    /// `u8` lanes or two `u16` lanes, and the two give different answers.
    lane_w: IndexMap<ValueId, u32>,
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
    /// What the models reported on this path, each paired with the **identity of the
    /// report**. Kept on the state so a fork carries only what it actually saw — but a
    /// fork carries a *copy*, so one `free(&stack_var)` followed by one branch became two
    /// identical findings, and 024 contracts 4, 5, 9, 10 and 22 all say "exactly one".
    ///
    /// The id is minted where the finding is, so every state descended from that call
    /// shares it. Deduplicating on the *text* instead would collapse two genuinely
    /// separate reports that happen to read the same, which is the common case in a loop.
    findings: Vec<StateFinding>,
    /// **Every symbolic input this path ever created**, in creation order (023 §9). The
    /// witness is an assignment to exactly these, so a symbol minted without landing here
    /// is a value the replay harness would leave to chance.
    inputs: Vec<(Term, InputOrigin)>,
    /// How many of a replay's bindings this path has consumed. **Per state**, because a
    /// replay can fork and a run-wide cursor lets one path eat another's bindings.
    replay_used: usize,
    /// Filled once, when the state finishes: the engine still has the arena and the
    /// solver there, and `RunResult` has neither.
    witness: Option<Witness>,
    /// Why there is no witness, when there is none. 023 contract 15 allows the absence
    /// and not the silence.
    unwitnessed: Option<String>,
    /// Where an address term came from, for a pointer that went through **memory**: the
    /// bytes are the carrier, and the program itself declared the field to be a pointer.
    ///
    /// Keyed by *value*, so it cannot be trusted for `IntToPtr` — an address is a ground
    /// constant and the arena hash-conses, so two ways of computing one address are the
    /// same `Term` by construction. That path uses `Frame::ptr_vals`, which follows
    /// dataflow instead.
    ptr_ints: IndexMap<Term, Pointer>,
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
    /// Every object this state's memory knows about, retired ones included — so a test
    /// can see that a scope entered three times made three objects rather than reusing
    /// one. `frame_objs` cannot answer that: it is keyed by `AllocaId` and holds only the
    /// newest.
    pub fn object_ids_for_test(&self) -> Vec<chiero_mem::ObjectId> {
        self.mem
            .resolvable_ranges()
            .into_iter()
            .map(|(id, _, _)| id)
            .filter(|id| *id != chiero_mem::ObjectId::NULL)
            .collect()
    }

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
            inputs: Vec::new(),
            replay_used: 0,
            witness: None,
            unwitnessed: None,
            ptr_ints: IndexMap::new(),
        }
    }

    pub fn fidelity(&self) -> Fidelity {
        self.fidelity
    }

    /// The state gave up. **Sets the status and degrades together**, because they are one
    /// fact: `RunResult::fidelity` special-cases `Errored` so a run containing one cannot
    /// mint a proof, and resting the project's central guarantee on that single line meant
    /// `State::fidelity()` on its own answered `Exact` for a state that had stopped.
    fn give_up(&mut self, why: String, span: Span) {
        self.degrade(Fidelity::Unknown, AssumptionKind::NoInformation, span, &why);
        self.status = Status::Errored(why);
    }

    pub fn assumptions(&self) -> &[Assumption] {
        &self.assumptions
    }

    /// Record that `t` is the address of `p`. See `ptr_ints`.
    pub fn remember_provenance(&mut self, t: Term, p: Pointer) {
        self.ptr_ints.insert(t, p);
    }

    pub fn provenance_of(&self, t: Term) -> Option<Pointer> {
        self.ptr_ints.get(&t).copied()
    }

    /// Record that the local `v` holds the address of `p`. Arithmetic writes a *different*
    /// local with no entry, which is how provenance is lost — the property the value-keyed
    /// table was documented to have and could not.
    pub fn remember_value_provenance(&mut self, v: ValueId, p: Pointer) {
        if let Some(f) = self.stack.last_mut() {
            f.ptr_vals.insert(v, p);
        }
    }

    fn remember_lane_width(&mut self, v: ValueId, w: u32) {
        if let Some(f) = self.stack.last_mut() {
            f.lane_w.insert(v, w);
        }
    }

    fn lane_width_of(&self, v: ValueId) -> Option<u32> {
        self.stack.last().and_then(|f| f.lane_w.get(&v)).copied()
    }

    pub fn value_provenance_of(&self, v: ValueId) -> Option<Pointer> {
        self.stack.last().and_then(|f| f.ptr_vals.get(&v)).copied()
    }

    pub fn findings(&self) -> Vec<&str> {
        self.findings.iter().map(|f| f.message.as_str()).collect()
    }

    /// This path's witness, or `None` with a reason (023 contract 15).
    pub fn witness(&self) -> Option<&Witness> {
        self.witness.as_ref()
    }

    pub fn unwitnessed(&self) -> Option<&str> {
        self.unwitnessed.as_deref()
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

    /// Everything the models reported, across every state — **once each**. States that
    /// forked after a call all carry that call's finding, so a flat concatenation
    /// multiplied every report by the number of surviving descendants.
    ///
    /// 023 §6.1 delegates the real deduplication key — `(checker, span, object, kind)` —
    /// to 040. This is the narrower thing 024's "exactly one" wording needs: the same
    /// *report*, seen from several states, is one report.
    pub fn findings(&self) -> Vec<String> {
        self.reports().into_iter().map(|f| f.message).collect()
    }

    /// The same reports, whole (023 §9). `findings` is the projection to text; this is
    /// what a caller needs to replay one, and the two cannot disagree because there is
    /// one implementation.
    pub fn reports(&self) -> Vec<Finding> {
        let mut seen: Vec<u64> = Vec::new();
        let mut out = Vec::new();
        let mut keys: Vec<(StateId, FindingKey)> = Vec::new();
        for (st, f) in self
            .states
            .iter()
            .flat_map(|st| st.findings.iter().map(move |f| (st, f)))
        {
            let (id, key) = (&f.id, &f.key);
            // 023 §6.1's key when there is one, the report's identity otherwise. The id
            // alone recognises the copies a *fork* makes and cannot recognise the copies
            // a *loop* makes, because those are genuinely separate reports of one bug.
            // **The id is what recognises a fork's copies**, and it is the only thing
            // that may deduplicate *across* paths: two paths that fault differently are
            // two reports, and 023 contract 20 says the engine does not deduplicate —
            // §6.1 delegates the real key to 040. Keying across states collapsed
            // `buf + 64` and `buf + 128` into one finding and threw away the second's
            // witness, so a reader saw one of two bugs and no sign of the other.
            // Found by review.
            if seen.contains(id) {
                continue;
            }
            // Within *one* path, the same fault at the same place about the same object
            // is one report however many times the path runs through it — that is what
            // a loop does, and 024's "exactly one" wording is about the program.
            if let Some(k) = key {
                if keys.iter().any(|(sid, kk)| *sid == st.id && kk == k) {
                    continue;
                }
                keys.push((st.id, k.clone()));
            }
            seen.push(*id);
            out.push(Finding {
                id: f.id,
                message: f.message.clone(),
                span: f.span,
                witness: st.witness.clone(),
                unwitnessed: st.unwitnessed.clone().or_else(|| {
                    // A state that never finished has no witness because nothing tried,
                    // and contract 15 wants that said rather than left blank.
                    st.witness.is_none().then(|| {
                        "the path did not terminate, so no assignment was extracted".to_string()
                    })
                }),
                fidelity: st.fidelity,
            });
        }
        out
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
    /// Identity for a model report, so the copies a fork makes are recognised as one.
    finding_seq: u64,
    /// The local `eval` is filling, for the one rvalue that needs to create sibling
    /// states — 021 §5.1's resolution. `eval` does not otherwise know its destination.
    pending_dst: Option<ValueId>,
    /// One `Function` object per `FuncId`, so two `&f` are the same pointer.
    func_objs: IndexMap<FuncId, ObjectId>,
    /// One object per file-scope variable, allocated on first use.
    global_objs: IndexMap<GlobalId, ObjectId>,
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
    /// 023 contract 16: the bindings a replay supplies instead of minting symbols, in
    /// creation order, and how many have been consumed. A replay is the only way to find
    /// out whether a witness *works*, so it goes through the same `input` seam every
    /// symbol does rather than a second path that could drift from it.
    replay: Option<Witness>,
}

impl<'m> Engine<'m> {
    pub fn new(module: &'m Module) -> Engine<'m> {
        Engine {
            module,
            tier: SolverTier::LiteOnly,
            next_state: 0,
            solver_calls: 0,
            fresh_count: 0,
            finding_seq: 0,
            pending_dst: None,
            func_objs: IndexMap::new(),
            global_objs: IndexMap::new(),
            budget: Budget::default(),
            models: ModelRegistry::with_builtins(),
            alloc_policy: AllocPolicy::default(),
            string_policy: StringPolicy::default(),
            backend: None,
            solver: None,
            solver_inits: 0,
            pending: Vec::new(),
            forks: 0,
            replay: None,
        }
    }

    /// Run with every symbolic input bound to `w`'s values (023 contract 16).
    ///
    /// The bindings are consumed **in creation order**, and each one's origin is checked
    /// against the site that asks for it: a witness whose inputs come in a different
    /// order is a witness for a different path, and binding it positionally anyway would
    /// produce a run that looks like a reproduction and is not.
    pub fn replaying(mut self, w: Witness) -> Self {
        self.replay = Some(w);
        self
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
        // 020 §8: **always**, including on hand-written fixtures. A module that fails
        // verification is never executed — running one anyway means every later answer is
        // about a program the CIR does not describe, and the failures are silent
        // (a narrow value zero-extended) or fatal (a panic in `extract`) rather than
        // reported.
        // **`is_error`, not "any diagnostic".** 020 rule 3 makes `UnreachableBlock` a
        // warning because unreachable C code exists and is legal, and wave 7 fixed the
        // dominance lattice precisely so a live join after dead code would work. Gating on
        // emptiness turned every `default:` after an exhaustive switch into a module
        // chiero refused to run.
        let errs: Vec<_> = chiero_cir::verify(self.module)
            .into_iter()
            .filter(|e| e.kind.is_error())
            .collect();
        if !errs.is_empty() {
            let mut bad = State {
                id: self.new_id(),
                mem,
                pc: (f.entry, 0),
                path: Vec::new(),
                fidelity: Fidelity::Exact,
                assumptions: Vec::new(),
                status: Status::Errored(format!(
                    "the module does not verify ({} error(s)); first: {:?}",
                    errs.len(),
                    errs.first()
                )),
                trace: Vec::new(),
                stack: Vec::new(),
                ret: None,
                edge_counts: IndexMap::new(),
                steps: 0,
                findings: Vec::new(),
                inputs: Vec::new(),
                replay_used: 0,
                witness: None,
                unwitnessed: None,
                ptr_ints: IndexMap::new(),
            };
            bad.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                Span::DUMMY,
                "the module was never executed, so nothing is known about it",
            );
            return RunResult {
                id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                states: vec![bad],
                solver_calls: 0,
                backend_spawns: 0,
                solver_inits: 0,
                // One state, so one entry: the sibling error path does this and the
                // normal path returns a permutation of `states()`. An empty vector here
                // broke that invariant on the new path alone.
                completion_order: vec![0],
                budget: self.budget,
                _seal: Sealed,
            };
        }
        // **The entry function's parameters are unknown, not absent.** Leaving them
        // unbound made every use a lowering gap, so `void f(int *out) { *out = 7; }`
        // analysed on its own wrote nothing — and a whole-program tool is used on a
        // library exactly this way. A pointer parameter gets a fresh object of unknown
        // contents, which is what "called from somewhere chiero has not seen" means; a
        // scalar gets a fresh symbol of its declared width.
        let mut entry_locals: IndexMap<ValueId, Value> = IndexMap::new();
        let mut entry_inputs: Vec<(Term, InputOrigin)> = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let v = if p.ty == CTy::Ptr {
                // Sized by the budget rather than by a guess about the callee: the object
                // exists so accesses have somewhere to land, and its extent is a bound
                // chiero chose, so an access past it is reported as such.
                let obj = mem.alloc(ObjKind::Extern, ENTRY_PARAM_BYTES, 16, f.span);
                Value::Ptr(Pointer { base: obj, off: 0 })
            } else {
                let t = a.var(sort_of(&p.ty), &format!("param{i}"));
                entry_inputs.push((
                    t,
                    InputOrigin::Param {
                        index: i,
                        name: String::new(),
                        span: f.span,
                    },
                ));
                Value::Scalar(t)
            };
            entry_locals.insert(p.value, v);
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
                locals: entry_locals,
                frame_objs,
                ptr_vals: IndexMap::new(),
                varargs: Vec::new(),
                lane_w: IndexMap::new(),
            }],
            ret: None,
            edge_counts: IndexMap::new(),
            steps: 0,
            findings: Vec::new(),
            inputs: entry_inputs,
            replay_used: 0,
            witness: None,
            unwitnessed: None,
            ptr_ints: IndexMap::new(),
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
                    // **The bound is checked before the push, not after.** Checking
                    // afterwards let a step that produced several siblings overshoot —
                    // `max_states = 4` ended with six — so the number a caller set was not
                    // the number it got, and a budget that is not a bound cannot be
                    // reported as one.
                    // `done` + `work` + **the state running right now** is the live
                    // count; pushing adds one more. Forgetting `s` itself is how the bound
                    // still overshot by one after the first fix.
                    let live = done.len() + work.len() + 1;
                    if live + 1 > self.budget.max_states as usize {
                        s.degrade(
                            Fidelity::Bounded,
                            AssumptionKind::BudgetHit,
                            Span::DUMMY,
                            &format!("max_states ({}) reached", self.budget.max_states),
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
            self.attach_witness(a, &mut s);
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

    /// Record a symbol as an **input**: something the program did not compute, that the
    /// replay harness must supply (023 §9). Every `a.var` in the engine goes through
    /// here — one that does not is a value the witness silently leaves to chance.
    fn input(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        sort: chiero_solver::Sort,
        name: &str,
        origin: InputOrigin,
    ) -> Term {
        if let Some(t) = self.replayed(a, s, &origin) {
            return t;
        }
        let t = a.var(sort, name);
        s.inputs.push((t, origin));
        t
    }

    /// The next binding, if this run is a replay and the binding belongs to this site.
    ///
    /// A mismatch is a **finding**, not a silent fallback to a fresh symbol: the run was
    /// asked to reproduce something, and a run that quietly stops reproducing while still
    /// reporting whatever it finds is how a refuted bug and an unrelated one come to look
    /// the same.
    fn replayed(&mut self, a: &mut TermArena, s: &mut State, origin: &InputOrigin) -> Option<Term> {
        // **The cursor is the state's, not the run's.** A replay can still fork — an
        // unmodeled call, a symbolic base — and one cursor on the engine let the
        // first-explored path eat the bindings belonging to the second, silently whenever
        // the two sites' origins happened to match. Found by review.
        let w = self.replay.as_ref()?;
        // **Once diverged, this path is done replaying.** Reporting the same problem
        // again at every remaining site turns one problem into a list and buries it;
        // the sentinel says "abandoned", where before it only said "past the end", which
        // the missing-binding branch then reported as a fresh divergence.
        if s.replay_used == usize::MAX {
            return None;
        }
        let used = s.replay_used;
        let Some(b) = w.bindings.get(used) else {
            let why = format!(
                "replay diverged: this run wants a {} at {:?} that the witness does not \
                 have — it has {} binding(s)",
                origin.label(),
                origin.span(),
                w.bindings.len()
            );
            s.replay_used = usize::MAX;
            self.finding_seq += 1;
            let seq = self.finding_seq;
            let span = origin.span();
            s.findings.push(StateFinding {
                id: seq,
                key: None,
                message: why.clone(),
                span,
            });
            s.degrade(Fidelity::Unknown, AssumptionKind::NoInformation, span, &why);
            return None;
        };
        if &b.origin != origin {
            let why = format!(
                "replay diverged: the witness's binding {} is a {} at {:?}, but this run \
                 wants a {} at {:?}",
                used,
                b.origin.label(),
                b.origin.span(),
                origin.label(),
                origin.span()
            );
            let span = origin.span();
            s.replay_used = usize::MAX;
            self.finding_seq += 1;
            let seq = self.finding_seq;
            s.findings.push(StateFinding {
                id: seq,
                key: None,
                message: why.clone(),
                span,
            });
            s.degrade(Fidelity::Unknown, AssumptionKind::NoInformation, span, &why);
            return None;
        }
        let (width, value) = (b.width, b.value);
        s.replay_used += 1;
        // A *constant*, not a variable with an equality on the path: the point of a
        // replay is that nothing is left to solve, and contract 16's run asks the solver
        // nothing at all.
        Some(a.bv(width, value))
    }

    /// 023 §9: a concrete assignment for every symbolic input on this path.
    ///
    /// Computed when the state finishes rather than when the finding is made: the path is
    /// append-only, so a model of the *final* path satisfies every prefix of it and
    /// therefore replays through every finding on the way. Doing it per finding would
    /// cost a query each and answer the same question.
    fn attach_witness(&mut self, a: &mut TermArena, s: &mut State) {
        // **A replay that re-invented memory did not replay it.** The witness records
        // memory's symbols (023 §9), but `Memory` mints them itself and nothing routes a
        // binding back in, so those bytes come out fresh on the second run. Saying so is
        // the difference between a known limit and a reproduction that quietly is not
        // one — contract 16 asks for "all inputs concretized", and these are not.
        if self.replay.is_some() && !s.mem.minted_symbols().is_empty() {
            let n = s.mem.minted_symbols().len();
            self.finding_seq += 1;
            let msg = format!(
                "replay incomplete: {n} value(s) invented by memory on this path were \
                 re-invented rather than supplied by the witness"
            );
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key: None,
                message: msg.clone(),
                span: Span::DUMMY,
            });
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                Span::DUMMY,
                &msg,
            );
        }
        if s.findings.is_empty() {
            return;
        }
        // **Memory's symbols are inputs too** (023 §9: "lazily-materialized object
        // contents"). The engine does not see them being minted, so they are collected
        // here from the memory that made them. Without this the witness said "no symbolic
        // inputs on this path" about a path whose whole condition was built from havoc'd
        // bytes — not a gap in the witness but a false statement in it. Found by review.
        let mut extra: Vec<(Term, InputOrigin)> = Vec::new();
        let mut unbindable: Option<String> = None;
        for m in s.mem.minted_symbols() {
            if m.array {
                // A whole-object havoc is an *array*; an assignment for it is not a
                // number, so no `Binding` can carry it. Saying so is the point — the
                // alternative is a witness that looks complete and replays into a
                // different program.
                unbindable = Some(format!(
                    "this path reads {} ({:?}), whose value is a whole array rather than \
                     a number, and a witness binds numbers",
                    m.why, m.obj
                ));
            } else {
                extra.push((
                    m.term,
                    InputOrigin::Memory {
                        why: m.why,
                        span: m.at,
                    },
                ));
            }
        }
        if let Some(why) = unbindable {
            s.unwitnessed = Some(why);
            return;
        }
        // **No inputs is a complete answer, not a failed one.** Reporting `None` here
        // would claim a failure that did not happen — and it would cost a solver call to
        // claim it.
        if s.inputs.is_empty() && extra.is_empty() {
            s.witness = Some(Witness::empty());
            return;
        }
        let model = match self.probe(a, s, &[]) {
            CheckResult::Sat(m) => m,
            CheckResult::Unsat => {
                s.unwitnessed = Some(
                    "the path condition is unsatisfiable at termination, so no input \
                     assignment reaches this report"
                        .to_string(),
                );
                return;
            }
            // **Which of the two it is matters.** A backend that returned a model
            // chiero could not verify is chiero's problem, and blaming the path's
            // decidability for it sends a reader to strengthen a program that is already
            // decidable. Found by review.
            CheckResult::Unknown(why) => {
                s.unwitnessed = Some(match &why {
                    chiero_solver::UnknownReason::BackendError(e) => format!(
                        "no input assignment could be extracted: the backend answered but \
                         chiero could not use the model ({e})"
                    ),
                    other => format!(
                        "the solver could not decide this path's condition, so no input \
                         assignment could be extracted: {other:?}"
                    ),
                });
                return;
            }
        };
        let mut bindings = Vec::new();
        for (t, origin) in s.inputs.clone().into_iter().chain(extra) {
            let width = a.width(t);
            // **`pinned` is the honest part.** A model need not assign a variable the
            // path never mentions; binding it to zero and presenting that as the
            // solver's answer would tell a reader the bug needs a value it does not.
            let (value, pinned) = match a.eval(&model, t) {
                Ok(c) => (c.bits(), true),
                Err(_) => (0, false),
            };
            bindings.push(Binding {
                origin,
                width,
                value,
                pinned,
            });
        }
        s.witness = Some(Witness { bindings });
    }

    /// Give this activation a **fresh** object for each `Lifetime::Scope` alloca of
    /// `scope` (021 contract 29).
    ///
    /// The object is the activation of the declaration, not the declaration: a loop body
    /// entered three times has three of them, and reusing one would let this pass read
    /// the last pass's bytes and make a pointer that escaped the last pass look live.
    ///
    /// A function whose CIR carries no `Scope` markers is unaffected — its objects are
    /// still built once at frame entry, which is what every fixture without markers
    /// relies on. `Lifetime::Function` is untouched here for the same reason it survives
    /// `Scope(Exit)`: `alloca()` memory belongs to the activation, not the block.
    fn enter_scope(&mut self, s: &mut State, scope: ScopeId, at: Span) {
        let Some(f) = self.module.funcs.iter().find(|f| f.id == s.func()) else {
            return;
        };
        let decls: Vec<(AllocaId, u64, u64)> = f
            .allocas
            .iter()
            .filter(|d| d.scope == scope && d.lifetime == Lifetime::Scope)
            .map(|d| {
                let elem = size_of_cty(&d.ty);
                let bytes = if d.count == chiero_cir::DYNAMIC_EXTENT {
                    0
                } else {
                    d.count.saturating_mul(elem)
                };
                (d.id, bytes, d.align)
            })
            .collect();
        for (id, bytes, align) in decls {
            let obj = s.mem.alloc(ObjKind::Stack, bytes, align, at);
            if let Some(fr) = s.stack.last_mut() {
                fr.frame_objs.insert(id, obj);
            }
        }
    }

    /// Retire this activation's `Lifetime::Scope` objects belonging to `scope` (021 §4).
    ///
    /// **Only this activation's, and only this scope's.** `AllocaId` is unique within a
    /// function, so retiring across frames would kill a caller's local on a callee's
    /// block exit; and retiring by anything coarser than the alloca's own `ScopeId` would
    /// report a use-after-scope on every function with a nested block.
    ///
    /// `Lifetime::Function` — `alloca()` — is left alone. 020 §4.4 says why in as many
    /// words: "scope lifetime would retire `alloca()` memory early and report
    /// use-after-scope" on a program that has none. That is 021 contracts 30 and 39.
    fn exit_scope(&mut self, s: &mut State, scope: ScopeId, at: Span) {
        let Some(f) = self.module.funcs.iter().find(|f| f.id == s.func()) else {
            return;
        };
        let dying: Vec<chiero_mem::ObjectId> = f
            .allocas
            .iter()
            .filter(|d| d.scope == scope && d.lifetime == Lifetime::Scope)
            .filter_map(|d| {
                s.stack
                    .last()
                    .and_then(|fr| fr.frame_objs.get(&d.id))
                    .copied()
            })
            .collect();
        for id in dying {
            s.mem.exit_scope(id, at);
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
            s.give_up(format!("no such function {cur:?}"), Span::DUMMY);
            return None;
        };
        // **`step` is total** (023 §2). Returning `None` here without setting a status
        // left the run loop spinning forever — no allocation, so not even the OOM killer
        // would end it.
        let Some(b) = f.blocks.iter().find(|b| b.id == s.pc.0) else {
            let why = format!("no such block {:?}", s.pc.0);
            s.give_up(why, Span::DUMMY);
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
                self.pending_dst = Some(*dst);
                let evaluated = self.eval(a, s, rv, i.span);
                self.pending_dst = None;
                if let Some(v) = evaluated {
                    s.set_local(*dst, v);
                    // **Provenance is recorded here, not in `eval`**, because `eval` does
                    // not know which local it is filling — and the local is the thing that
                    // carries the dataflow. `s.set_local` first, so a `PtrToInt` of a
                    // local onto itself sees the new value.
                    if let RValue::Cast {
                        kind: CastKind::PtrToInt,
                        a: src,
                        ..
                    } = rv
                        && let Some(Value::Ptr(p)) = self.operand(a, s, src)
                    {
                        s.remember_value_provenance(*dst, p);
                    }
                    // A vector's lane width travels with the local, for the same reason
                    // as provenance: the shape is not recoverable from the value.
                    if let Some(w) = self.lane_width_produced(a, s, rv) {
                        s.remember_lane_width(*dst, w);
                    }
                }
            }
            InstKind::Opaque {
                dsts, writes, why, ..
            } => {
                // 020 §4.3: never silently a no-op. Each output is a fresh symbol,
                // distinct per instruction, and the path is a modeling lie from here on.
                for (v, ty) in dsts {
                    let t = self.input(
                        a,
                        s,
                        sort_of(ty),
                        &format!("opaque_{}", v.0),
                        InputOrigin::Opaque { span: i.span },
                    );
                    s.set_local(*v, Value::Scalar(t));
                }
                // **A declared write is a write.** Ignoring `writes` left inline asm that
                // says it clobbers a buffer with chiero still believing the old bytes —
                // the same failure as a call that does not invalidate what it was handed.
                // Only the declared range: 020 §4.3 makes the declaration the point, and
                // invalidating everything would be no better than not modelling it.
                for w in writes {
                    let (Some(Value::Ptr(p)), Some(n)) = (
                        self.operand(a, s, &w.addr),
                        self.concrete_size(a, s, &w.size),
                    ) else {
                        self.lowering_gap(s, i.span, "an opaque write chiero cannot place");
                        continue;
                    };
                    // `Symbolic`, as for an unmodeled call: the code wrote *something*,
                    // and `Uninitialized` would fire on every buffer it legitimately
                    // filled.
                    // **The faults are the point.** A declared clobber wider than the
                    // object is a buffer overflow the program announced, and discarding
                    // the fault made it one chiero detected and did not report.
                    let r = s
                        .mem
                        .havoc_range_reporting(a, p, n, HavocFill::Symbolic, i.span);
                    self.report_faults(s, &r.faults, i.span);
                    if r.value != Some(n) {
                        self.lowering_gap(
                            s,
                            i.span,
                            "an opaque write chiero could not perform in full",
                        );
                    }
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
            // 020 §4: the memory instructions. Every fixture in the suite wrote memory
            // through `memset` before these existed, which is how they stayed missing.
            InstKind::Store {
                addr,
                val,
                ty,
                align,
                ..
            } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, addr) else {
                    self.lowering_gap(s, i.span, "a store through a non-pointer address");
                    return;
                };
                // **A stored pointer keeps its object** (023 contract 23). Taking the
                // value through `scalar` refused every `p->next = q`, which is the shape
                // of essentially every data structure — and the dropped store then
                // manufactured a false uninitialized-read on the reload.
                let t = match self.operand(a, s, val) {
                    Some(Value::Scalar(t)) => t,
                    Some(Value::Ptr(q)) => {
                        let Some(t) = self.address_term(a, s, q, i.span) else {
                            return;
                        };
                        t
                    }
                    None => {
                        self.lowering_gap(s, i.span, "a store of an untranslatable value");
                        return;
                    }
                };
                let _ = align;
                let size = size_of_cty(ty);
                let r = s.mem.write_term(a, p, t, size, Endian::Little, i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            // `CopyMem` is `memcpy`, deliberately: 021 contract 22's overlap rule must
            // not depend on whether the frontend spelled a struct assignment as an
            // instruction or as a call.
            InstKind::CopyMem {
                dst,
                src,
                size,
                align,
            } => {
                let _ = align;
                let (Some(Value::Ptr(d)), Some(Value::Ptr(sp)), Some(n)) = (
                    self.operand(a, s, dst),
                    self.operand(a, s, src),
                    self.concrete_size(a, s, size),
                ) else {
                    self.lowering_gap(s, i.span, "a copy with an untranslatable operand");
                    return;
                };
                let r = s.mem.copy(d, sp, n, chiero_mem::Overlap::Forbidden, i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            InstKind::SetMem { dst, byte, size } => {
                let (Some(Value::Ptr(d)), Some(b), Some(n)) = (
                    self.operand(a, s, dst),
                    self.concrete_size(a, s, byte),
                    self.concrete_size(a, s, size),
                ) else {
                    self.lowering_gap(s, i.span, "a fill with an untranslatable operand");
                    return;
                };
                let r = s.mem.set(d, b as u8, n, i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            // 021 §3.1's bit-granular init exists for exactly this pair. A byte-rounded
            // implementation would initialize the neighbouring bitfields too, and every
            // real uninitialized-bitfield read would go missing.
            InstKind::StoreBits {
                addr,
                val,
                bits,
                unit,
                ..
            } => {
                let _ = unit;
                let (Some(Value::Ptr(p)), Some(t)) =
                    (self.operand(a, s, addr), self.scalar(a, s, val))
                else {
                    self.lowering_gap(s, i.span, "a bitfield store through a non-pointer");
                    return;
                };
                let Ok(v) = a.eval_ground(t) else {
                    // A symbolic bitfield value needs `InitBit::Cond` plumbing that the
                    // bit API does not have yet; writing a concretization would put a
                    // made-up value in the program's memory.
                    self.lowering_gap(s, i.span, "a symbolic bitfield store");
                    return;
                };
                let r = s
                    .mem
                    .write_bits(p, bits.off as u64, bits.width as u64, v.bits(), i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            // 020 §3: a runtime extent arrives **here**, at a real program point, and not
            // in `Function::allocas` — a function-level table would reference a value
            // computed inside a block, which is undefined for verifier rule 1 and creates
            // the object before its size exists.
            InstKind::AllocaDyn {
                dst,
                alloca,
                elem,
                count,
                align,
            } => {
                let Some(n) = self.concrete_size(a, s, count) else {
                    // A symbolic count is 021's `SizeVal::Sym` and is owed. Guessing one
                    // would give the object a size the program never chose, and every
                    // bounds answer about it would be about that invented size.
                    self.lowering_gap(s, i.span, "AllocaDyn with a symbolic count");
                    return;
                };
                let Some(bytes) = n.checked_mul(size_of_cty(elem)) else {
                    self.lowering_gap(s, i.span, "AllocaDyn whose extent overflows");
                    return;
                };
                // **A fresh object per execution.** C's `alloca` in a loop accumulates
                // allocations, and reusing one object would make the second iteration
                // alias the first — writes aliasing and lifetimes wrong at once.
                let obj = s.mem.alloc(ObjKind::Stack, bytes, (*align).max(1), i.span);
                if let Some(f) = s.stack.last_mut() {
                    f.frame_objs.insert(*alloca, obj);
                }
                s.set_local(*dst, Value::Ptr(Pointer { base: obj, off: 0 }));
            }
            // 020 §4.4.1. The `va_list` is a real object so `va_list *` can cross a
            // function boundary — VPP's whole format infrastructure has the callee
            // advancing the *caller's* iteration state. The cursor therefore lives in the
            // object's bytes, not beside it.
            InstKind::VaStart { list } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, list) else {
                    self.lowering_gap(s, i.span, "va_start on a non-pointer");
                    return;
                };
                let zero = a.bv(64, 0);
                let r = s.mem.write_term(a, p, zero, 8, Endian::Little, i.span);
                self.report_faults(s, &r.faults, i.span);
                // **Which frame owns the arguments**, in the object's second word. 020
                // §4.4.1's ABI layout has room for it, and without it `va_arg` asked the
                // *current* frame — which is the callee's, and empty, whenever a
                // `va_list *` crosses a boundary. That is contract 37, and the whole
                // reason the list lives in memory.
                let owner = a.bv(64, s.stack.len().saturating_sub(1) as u128);
                let at8 = Pointer {
                    base: p.base,
                    off: p.off + 8,
                };
                let r = s.mem.write_term(a, at8, owner, 8, Endian::Little, i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            InstKind::VaArg { dst, list, ty } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, list) else {
                    self.lowering_gap(s, i.span, "va_arg on a non-pointer");
                    return;
                };
                let cur = s.mem.read_term(a, p, 8, Endian::Little, i.span);
                self.report_faults(s, &cur.faults, i.span);
                let Some(n) = cur
                    .value
                    .filter(|_| !unusable(&cur.faults))
                    .and_then(|t| a.eval_ground(t).ok())
                    .map(|c| c.bits() as usize)
                else {
                    self.lowering_gap(s, i.span, "a va_list cursor chiero cannot read");
                    return;
                };
                // The owning frame, not the current one — see `VaStart`.
                let at8 = Pointer {
                    base: p.base,
                    off: p.off + 8,
                };
                let own = s.mem.read_term(a, at8, 8, Endian::Little, i.span);
                self.report_faults(s, &own.faults, i.span);
                let Some(owner) = own
                    .value
                    .filter(|_| !unusable(&own.faults))
                    .and_then(|t| a.eval_ground(t).ok())
                    .map(|c| c.bits() as usize)
                else {
                    self.lowering_gap(s, i.span, "a va_list with no owning frame");
                    return;
                };
                // **Past the end is not a value**, and neither is a hole. C leaves both
                // undefined, and handing back a fresh symbol would let a `printf` with too
                // few arguments — or one whose float chiero cannot represent — look like
                // it read something real.
                let Some(Some(v)) = s.stack.get(owner).and_then(|fr| fr.varargs.get(n)).copied()
                else {
                    self.lowering_gap(s, i.span, "va_arg of an argument chiero does not have");
                    return;
                };
                // **The declared type decides the width.** Handing the caller's value back
                // verbatim put a 64-bit term in a local the verifier believes is `i32`,
                // and comparing it panicked the solver.
                let v = match (v, bits_of_cty(ty)) {
                    (Value::Scalar(t), Some(w)) if a.width(t) > w => {
                        Value::Scalar(a.extract(t, w - 1, 0))
                    }
                    (Value::Scalar(t), Some(w)) if a.width(t) < w => Value::Scalar(a.zext(t, w)),
                    (other, _) => other,
                };
                s.set_local(*dst, v);
                let next = a.bv(64, n as u128 + 1);
                let r = s.mem.write_term(a, p, next, 8, Endian::Little, i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            // A copy duplicates the *iteration state*, so the two lists advance
            // independently from a shared position — which is what `va_copy` is for.
            InstKind::VaCopy { dst, src } => {
                let (Some(Value::Ptr(d)), Some(Value::Ptr(sp))) =
                    (self.operand(a, s, dst), self.operand(a, s, src))
                else {
                    self.lowering_gap(s, i.span, "va_copy on a non-pointer");
                    return;
                };
                // Both words: the cursor *and* the owning frame.
                let r = s
                    .mem
                    .copy(d, sp, 16, chiero_mem::Overlap::Forbidden, i.span);
                self.report_faults(s, &r.faults, i.span);
            }
            // `va_end` has no effect chiero can observe: the object's lifetime is the
            // frame's. Recording it as a gap would degrade every correct variadic
            // function for doing the right thing.
            InstKind::VaEnd { list } => {
                let _ = self.operand(a, s, list);
            }
            // **`Scope` markers are semantic** (020 §4.4): they bound the lifetime of
            // stack objects, "which is what makes use-after-scope detectable". The other
            // marker kinds are reporting-only and do nothing here.
            InstKind::Marker(MarkerKind::Scope(ev)) if ev.kind == ScopeKind::Exit => {
                self.exit_scope(s, ev.scope, i.span);
            }
            InstKind::Marker(MarkerKind::Scope(ev)) => {
                self.enter_scope(s, ev.scope, i.span);
            }
            InstKind::Marker(_) => {} // **No catch-all.** Every `InstKind` is handled, so adding one is a compile
                                      // error rather than a silent `LoweringGap` — which is how `Load`, `Store`,
                                      // `CopyMem`, `SetMem` and the four `Va*` all stayed missing for waves, behind
                                      // a uniform workaround that read as house style.
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
            Callee::Indirect(op) => {
                self.indirect(a, s, op, dst, span);
                return;
            }
        };
        let Some(f) = self.module.funcs.iter().find(|f| f.id == *id) else {
            s.give_up("call to an unknown function".into(), span);
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
                let t = self.input(
                    a,
                    s,
                    sort_of(&ret_ty),
                    &format!("extern{}", self.fresh_count),
                    InputOrigin::ExternReturn {
                        func: name.to_string(),
                        span,
                    },
                );
                s.set_local(d, Value::Scalar(t));
            }
            // 024 §1 step 3. gcc emits the `__builtin_` spelling for anything it
            // recognises, so without this every optimized translation unit hits unmodeled
            // externs for exactly the functions chiero models best — and an unmodeled
            // call now havocs its buffer, which turns the most common calls in a codebase
            // into the biggest holes. The *declaration* keeps its own name; only the
            // lookup is aliased.
            let name = Symbol::from(resolve_builtin(&name));
            match self.models.lookup(&name).map(|e| e.precision.clone()) {
                // 024 §2.1: dispatching an approximate model degrades *mechanically*, and
                // the model's own reason travels into the report.
                // 024 §2.1: dispatching an approximate model degrades *and* runs it.
                // Recording the reason and then doing nothing was strictly worse than
                // having no model at all, since the fallback at least invalidates the
                // buffers — and a model knows which ones it actually writes through.
                Some(Precision::Approximate(why)) => {
                    self.note_once(
                        s,
                        AssumptionKind::ModelApproximate,
                        span,
                        &format!("`{name}` is modeled approximately: {why}"),
                    );
                    if self.can_dispatch(&name) {
                        self.dispatch(
                            a,
                            s,
                            DispatchCall {
                                name: &name,
                                dst,
                                args,
                                ret_ty: &ret_ty,
                                span,
                            },
                        );
                    } else {
                        self.havoc_args(a, s, &name, args, span);
                    }
                }
                // **An exact model is only faithful if it actually runs.** Reading the
                // precision and recording nothing made a registered name *more* trusted
                // than an unregistered one: `strcpy` into a four-byte buffer finished
                // `Exact` and sealed, for a textbook overflow. Adding a correct model to
                // the library reduced safety, because the registration was read as a
                // claim about the *call* rather than about the model.
                Some(Precision::Exact) if self.can_dispatch(&name) => {
                    self.dispatch(
                        a,
                        s,
                        DispatchCall {
                            name: &name,
                            dst,
                            args,
                            ret_ty: &ret_ty,
                            span,
                        },
                    );
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
                    self.havoc_args(a, s, &name, args, span);
                }
                None => {
                    self.note_once(
                        s,
                        AssumptionKind::UnmodeledCall,
                        span,
                        &format!("`{name}` has no body and no model"),
                    );
                    self.havoc_args(a, s, &name, args, span);
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
        // 020 §4.4.1: everything past the declared parameters is the variadic argument
        // area the callee's `va_list` iterates. Kept as `Value`s rather than bytes so a
        // pointer argument keeps its object — `format_function_t` takes `u8 *` and
        // `va_list *`, so varargs *are* pointers in the VPP paths this exists for.
        // **A hole, not an absence.** `filter_map` compacted the area, so an argument
        // chiero cannot represent — a float, which `operand` documents as a gap — silently
        // handed the *next* one back to the following `va_arg`.
        let varargs: Vec<Option<Value>> = if f.variadic {
            args.iter()
                .skip(f.params.len())
                .map(|o| self.operand(a, s, o))
                .collect()
        } else {
            Vec::new()
        };
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
            ptr_vals: IndexMap::new(),
            varargs,
            lane_w: IndexMap::new(),
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
                let val_operand = v.clone();
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
                    // **The activation's objects die with it** (021 contract 39): every
                    // one of them, `Lifetime::Function` included — that lifetime survives
                    // an inner `Scope(Exit)` and ends *here*. Without this a returned
                    // pointer to a local read as live memory and the run stayed `Exact`
                    // over a program whose whole bug is that the pointer is dead.
                    //
                    // The popped frame's own map, not the state's objects: retiring
                    // anything wider would kill the caller's locals on every return.
                    //
                    // The block's span, since `Terminator` carries none: it is the
                    // closest thing to "where the activation ended" the CIR offers, and
                    // a report has to name somewhere a reader can look.
                    let at = self
                        .module
                        .funcs
                        .iter()
                        .find(|g| g.id == f.func)
                        .and_then(|g| g.blocks.iter().find(|b| b.id == s.pc.0))
                        .map_or(Span::DUMMY, |b| b.span);
                    for id in f.frame_objs.values().copied() {
                        s.mem.exit_scope(id, at);
                    }
                    if let Some((_, b, i)) = f.ret_to {
                        s.pc = (b, i);
                    }
                    if let (Some(d), Some(x)) = (f.ret_dst, val) {
                        s.set_local(d, x);
                        // **A return is a dataflow edge.** The callee's frame is going
                        // away, so provenance it computed has to come across with the
                        // value or an honest `(uintptr_t)` round trip through a helper
                        // degrades — which is the dominant index-to-pointer idiom, not a
                        // corner. Only the returned local's entry travels; the rest of
                        // the callee's table dies with the frame, as it should.
                        if let Some(Operand::Value(rv)) = val_operand
                            && let Some(p) = f.ptr_vals.get(&rv).copied()
                        {
                            s.remember_value_provenance(d, p);
                        }
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
                s.give_up("unsupported terminator".into(), Span::DUMMY);
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
            s.give_up("branch condition is not a scalar".into(), Span::DUMMY);
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
                s.give_up("both branches infeasible".into(), Span::DUMMY);
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

    /// One solver query under the path condition plus `extra`, keeping the model.
    fn probe(&mut self, a: &mut TermArena, s: &State, extra: &[Term]) -> CheckResult {
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
        asks.extend_from_slice(extra);
        solver.check(a, &asks)
    }

    fn feasible(&mut self, a: &mut TermArena, s: &State, t: Term) -> Feas {
        match self.probe(a, s, &[t]) {
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
            // A load of memory nobody wrote is a **finding plus a fresh symbol**, never
            // a zero — 021 §3.1 names reading uninitialized bytes as zero the single most
            // common way a symbolic executor produces confidently wrong results, and the
            // backing store really does read as zero, so this is where it happens.
            RValue::Load { addr, ty, .. } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, addr) else {
                    return self.lowering_gap(s, span, "a load through a non-pointer address");
                };
                let size = size_of_cty(ty);
                if size == 0 {
                    // Nothing to read, so nothing to invent. `sort_of` would have handed
                    // back a 64-bit symbol for a load of a zero-sized type.
                    return self.lowering_gap(s, span, &format!("a load of {ty:?}"));
                }
                let r = s.mem.read_term(a, p, size, Endian::Little, span);
                self.report_faults(s, &r.faults, span);
                match r.value.filter(|_| !unusable(&r.faults)) {
                    // A pointer-typed load comes back as a **pointer**. The recorded
                    // provenance first — the bits alone cannot say which object they name
                    // — and then the address, because bytes written by `calloc`, `memset`
                    // or a `.bss` global were never in that table and reloading them as a
                    // scalar made `n->next->x = 1` on a zeroed struct report nothing.
                    Some(t) if *ty == CTy::Ptr => match s.provenance_of(t) {
                        Some(q) => Value::Ptr(q),
                        None => match a.eval_ground(t) {
                            Ok(c) => {
                                let q = s.mem.object_containing(c.bits() as u64);
                                // Zero is unambiguously null, so that costs nothing. Any
                                // other address was *found*, and 021 §7.1 calls that
                                // search wrong in both directions.
                                if q.base != chiero_mem::ObjectId::NULL {
                                    s.degrade(
                                        Fidelity::Unknown,
                                        AssumptionKind::NoInformation,
                                        span,
                                        "a pointer loaded from memory was resolved by \
                                         address, which is wrong if a different object \
                                         now occupies it",
                                    );
                                }
                                Value::Ptr(q)
                            }
                            Err(_) => Value::Scalar(t),
                        },
                    },
                    Some(t) => Value::Scalar(t),
                    None => {
                        // The access produced nothing, so the caller gets a symbol chiero
                        // made up — which is exactly the case §7 puts under `Unknown`.
                        self.fresh_count += 1;
                        let t = self.input(
                            a,
                            s,
                            sort_of(ty),
                            &format!("load{}", self.fresh_count),
                            InputOrigin::Load { span },
                        );
                        s.degrade(
                            Fidelity::Unknown,
                            AssumptionKind::NoInformation,
                            span,
                            "a load produced no value, so its result is invented",
                        );
                        Value::Scalar(t)
                    }
                }
            }
            RValue::Un { op, a: x, ty } => {
                let Some(xv) = self.scalar(a, s, x) else {
                    return self.lowering_gap(s, span, "a non-scalar unary operand");
                };
                let _ = ty;
                match op {
                    UnOp::Neg => {
                        // Two's complement, at the operand's own width: `0 - x`, so the
                        // wrap is the machine's rather than a special case.
                        let z = a.bv(a.width(xv), 0);
                        Value::Scalar(a.sub(z, xv))
                    }
                    UnOp::Not => Value::Scalar(a.not(xv)),
                    // 023 §7: floating point is approximated, and a negation chiero
                    // cannot perform is a gap rather than a guess at the sign bit.
                    UnOp::FNeg => return self.lowering_gap(s, span, "FNeg"),
                }
            }
            // Integer casts only. A cast is in almost every C function, and getting the
            // *sign* wrong is silent: `(long)(int)-1` is `-1`, and zero-extending it
            // gives 4294967295 with nothing to say so.
            RValue::Cast {
                kind,
                a: x,
                from,
                to,
            } => {
                // 021 §7.1. These two are the only casts whose operand or result is a
                // pointer, so they are settled before the bit-width machinery below.
                if *kind == CastKind::PtrToInt {
                    let Some(Value::Ptr(p)) = self.operand(a, s, x) else {
                        return self.lowering_gap(s, span, "PtrToInt of a non-pointer");
                    };
                    return self.address_term(a, s, p, span).map(Value::Scalar);
                }
                if *kind == CastKind::IntToPtr {
                    let Some(t) = self.scalar(a, s, x) else {
                        return self.lowering_gap(s, span, "IntToPtr of a non-scalar");
                    };
                    // Provenance **first** (021 §7.1), and only the kind that survives
                    // scrutiny: the *dataflow* record for the operand's own local. The
                    // value-keyed table cannot be used here — an address is a ground
                    // constant and the arena hash-conses, so any arithmetic reaching that
                    // address would launder its way to the object and skip the degrade
                    // below, which is the one thing this fallback exists to announce.
                    if let Operand::Value(v) = x
                        && let Some(p) = s.value_provenance_of(*v)
                    {
                        return Some(Value::Ptr(p));
                    }
                    let Ok(c) = a.eval_ground(t) else {
                        // 021 §5.1: a symbolic base is *resolved*, not refused.
                        return self.resolve_symbolic_base(a, s, t, span);
                    };
                    let p = s.mem.object_containing(c.bits() as u64);
                    s.degrade(
                        Fidelity::Unknown,
                        AssumptionKind::NoInformation,
                        span,
                        "IntToPtr of an integer with no provenance: the object was found \
                         by address, which is wrong if a different object now occupies it",
                    );
                    return Some(Value::Ptr(p));
                }
                let Some(xv) = self.scalar(a, s, x) else {
                    return self.lowering_gap(s, span, &format!("{kind:?} of a non-scalar"));
                };
                let (fw, tw) = (bits_of_cty(from), bits_of_cty(to));
                let (Some(fw), Some(tw)) = (fw, tw) else {
                    return self.lowering_gap(
                        s,
                        span,
                        &format!("{kind:?} between {from:?} and {to:?}"),
                    );
                };
                match kind {
                    CastKind::Trunc if tw <= fw => Value::Scalar(a.extract(xv, tw - 1, 0)),
                    CastKind::ZExt if tw >= fw => Value::Scalar(a.zext(xv, tw)),
                    CastKind::SExt if tw >= fw => Value::Scalar(a.sext(xv, tw)),
                    // A `Bitcast` between equal widths is the identity on bits, which is
                    // exactly what 021 §3 means by "bytes are bytes".
                    CastKind::Bitcast if tw == fw => Value::Scalar(xv),
                    other => {
                        return self.lowering_gap(s, span, &format!("{other:?} {fw} -> {tw}"));
                    }
                }
            }
            RValue::Select { cond, t, f } => {
                let (Some(c), Some(tv), Some(fv)) = (
                    self.scalar(a, s, cond),
                    self.scalar(a, s, t),
                    self.scalar(a, s, f),
                ) else {
                    return self.lowering_gap(s, span, "a non-scalar select operand");
                };
                // **Not a fork.** `?:` evaluates one arm and yields a value; forking here
                // would double the state count for every conditional expression in the
                // program and change nothing about what is explored.
                Value::Scalar(a.ite(c, tv, fv))
            }
            RValue::AddrOfGlobal { g } => {
                let base = self.global_object(s, *g);
                Value::Ptr(Pointer { base, off: 0 })
            }
            RValue::LoadBits {
                addr,
                bits,
                signed,
                unit,
                ..
            } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, addr) else {
                    return self.lowering_gap(s, span, "a bitfield load through a non-pointer");
                };
                let r = s.mem.read_bits(p, bits.off as u64, bits.width as u64, span);
                self.report_faults(s, &r.faults, span);
                let w = bits_of_cty(unit).unwrap_or(bits.width);
                // **A value that arrived with a fault is not the program's.** 021 §5
                // hands back faults *alongside* a value, and for an uninitialized read
                // that value is the backing store's zero — the exact answer 021 §3.1
                // calls the most common way a symbolic executor is confidently wrong.
                match r.value.filter(|_| !unusable(&r.faults)) {
                    Some(v) => {
                        let narrow = a.bv(bits.width, v);
                        // A signed bitfield's high bit is a sign bit: `int x : 4` holding
                        // `0b1011` is -5, and zero-extending it gives 11 with nothing to
                        // say which was meant.
                        let t = if *signed {
                            a.sext(narrow, w)
                        } else {
                            a.zext(narrow, w)
                        };
                        Value::Scalar(t)
                    }
                    None => {
                        self.fresh_count += 1;
                        let t = self.input(
                            a,
                            s,
                            chiero_solver::Sort::BitVec(w),
                            &format!("bits{}", self.fresh_count),
                            InputOrigin::Load { span },
                        );
                        s.degrade(
                            Fidelity::Unknown,
                            AssumptionKind::NoInformation,
                            span,
                            "a bitfield load produced no value, so its result is invented",
                        );
                        Value::Scalar(t)
                    }
                }
            }
            RValue::Fresh { ty } => {
                // Named per state and per position, so two `Fresh` values are two
                // symbols and repeating one is not.
                self.fresh_count += 1;
                let t = self.input(
                    a,
                    s,
                    sort_of(ty),
                    &format!("fresh{}", self.fresh_count),
                    InputOrigin::Fresh { span },
                );
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
            // A function's address is a pointer to a **`Function` object**, so it has
            // provenance like any other and an indirect call can ask which function it
            // names. One object per `FuncId`, since two `&f` are the same pointer.
            RValue::AddrOfFunc(id) => {
                let base = self.func_object(s, *id);
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
                // **Signed, at the offset's own width.** 020 §8 rule 6 constrains the
                // *base* to pointer width, not the offset: a 32-bit `-4` read as unsigned
                // is `4294967292`, and `p + (int)-offsetof(S, m)` is `container_of`.
                let Ok(off) = i64::try_from(c.signed()) else {
                    // Wider than a pointer. Truncating turned `2^64 + 4` into `+4`, so a
                    // wildly out-of-bounds walk became in-bounds with no assumption and
                    // no degradation, and `seal` would have minted a proof over it.
                    return self.lowering_gap(s, span, "PtrAdd offset wider than a pointer");
                };
                Value::Ptr(Pointer {
                    base: p.base,
                    off: p.off.wrapping_add(off),
                })
            }
            // 021 §3's "bytes are bytes", for SIMD: a vector is a bit-vector of
            // `lanes * width` bits, **little-endian by lane**, so lane 0 occupies the low
            // bits exactly as a load of the same memory would see it. Every operation
            // below is then slicing and concatenation rather than a separate lane model.
            RValue::Splat { elem, lanes } => {
                let Some(e) = self.scalar(a, s, elem) else {
                    return self.lowering_gap(s, span, "a non-scalar splat element");
                };
                let mut t = e;
                for _ in 1..*lanes {
                    t = a.concat(e, t);
                }
                Value::Scalar(t)
            }
            RValue::ExtractLane { v, lane } => {
                let Some(x) = self.scalar(a, s, v) else {
                    return self.lowering_gap(s, span, "a non-scalar vector operand");
                };
                // **The lane width is recorded, not guessed.** `ExtractLane` carries no
                // element type (020 §4), and a total bit width alone cannot say how it is
                // divided — a 32-bit value is four `u8` lanes or two `u16` lanes and the
                // extract differs. Inferring one would silently read the wrong bits.
                let Some(w) = (match v {
                    Operand::Value(id) => s.lane_width_of(*id),
                    _ => None,
                }) else {
                    return self.lowering_gap(s, span, "an extract from a vector of unknown shape");
                };
                if (*lane + 1) * w > a.width(x) {
                    return self.lowering_gap(s, span, "an extract past the vector's end");
                }
                Value::Scalar(a.extract(x, (*lane + 1) * w - 1, *lane * w))
            }
            RValue::InsertLane { v, lane, val } => {
                let (Some(x), Some(e)) = (self.scalar(a, s, v), self.scalar(a, s, val)) else {
                    return self.lowering_gap(s, span, "a non-scalar insert operand");
                };
                let w = a.width(e);
                let total = a.width(x);
                if w == 0 || (*lane + 1) * w > total {
                    return self.lowering_gap(s, span, "an insert past the vector's end");
                }
                // Rebuilt from the parts either side, so the lane's own bits are the only
                // ones that move.
                let mut t = e;
                if *lane > 0 {
                    let low = a.extract(x, *lane * w - 1, 0);
                    t = a.concat(t, low);
                }
                if (*lane + 1) * w < total {
                    let high = a.extract(x, total - 1, (*lane + 1) * w);
                    t = a.concat(high, t);
                }
                Value::Scalar(t)
            }
            RValue::Shuffle { a: x, b: y, mask } => {
                let (Some(xv), Some(yv)) = (self.scalar(a, s, x), self.scalar(a, s, y)) else {
                    return self.lowering_gap(s, span, "a non-scalar shuffle operand");
                };
                if mask.is_empty() || a.width(xv) != a.width(yv) {
                    return self.lowering_gap(s, span, "a shuffle of mismatched vectors");
                }
                // **The lane count comes from the operand, not the mask.** 020 rule 12
                // reads `lanes` from the operand's declared vector type, and nothing
                // requires the mask to be that long — the mask length is the *result*
                // length, as in `__builtin_shufflevector`. Using `mask.len()` as the
                // divisor made a widening shuffle read nibbles, and using it as the a/b
                // split meant `b` was never read at all.
                let Some(w) = (match x {
                    Operand::Value(id) => s.lane_width_of(*id),
                    _ => None,
                }) else {
                    return self.lowering_gap(s, span, "a shuffle of a vector of unknown shape");
                };
                let Some(lanes) = a.width(xv).checked_div(w).filter(|l| *l > 0) else {
                    return self.lowering_gap(s, span, "a shuffle whose lanes do not fit");
                };
                let mut out: Option<Term> = None;
                for &m in mask {
                    // 020 rule 12: an index past `lanes` addresses the *second* operand,
                    // which is what makes a shuffle able to interleave two vectors.
                    let (src, idx) = if m < lanes { (xv, m) } else { (yv, m - lanes) };
                    if (idx + 1) * w > a.width(src) {
                        return self.lowering_gap(s, span, "a shuffle index past the end");
                    }
                    let piece = a.extract(src, (idx + 1) * w - 1, idx * w);
                    // Lane 0 is the low bits, so each later lane is concatenated above.
                    out = Some(match out {
                        None => piece,
                        Some(acc) => a.concat(piece, acc),
                    });
                }
                match out {
                    Some(t) => Value::Scalar(t),
                    None => return self.lowering_gap(s, span, "an empty shuffle"),
                }
            } // **No catch-all**, as in `exec_inst`: every `RValue` is handled, so adding
              // one is a compile error rather than a silent gap. 023 §7's table put a
              // `LoweringGap` under `Unknown` — an unimplemented lowering is not a modeling
              // lie, it is the engine not knowing — and the individual arms above still say
              // so wherever they genuinely cannot proceed.
        })
    }

    /// 021 §5.1. Resolve a pointer whose value the solver has not pinned.
    ///
    /// The five steps, in the spec's order and for the spec's reasons:
    /// 1. Provenance short-circuits the search — handled by the caller.
    /// 2. Ask which objects the value can fall in, capped at `max_resolutions`.
    /// 3. Exactly one: continue with it.
    /// 4. **Wholly unconstrained** — every object is feasible *and* so is being nowhere —
    ///    is `Unknown`, a finding, and the path **stops**.
    /// 5. Merely over the cap: concretize and record `Approximated`.
    ///
    /// **Steps 4 and 5 must stay distinct.** 021 records that an earlier draft merged
    /// them, so an unconstrained pointer was concretized to an arbitrary object and the run
    /// said `Bounded` — which reads as "we looked and bounded it" when nothing was known.
    /// Whether some object is **provably not** nameable by this address (021 §5.1).
    ///
    /// Told apart from step 4 this way because the alternative — asking whether *every*
    /// object is feasible — is the per-dereference O(objects) sweep §5.1 forbids, and one
    /// object the address cannot reach is already enough to know the solver has more than
    /// "no information at all". At most `cap` objects are probed, so a program with 10⁴
    /// of them costs the same as one with ten.
    ///
    /// `false` when nothing was ruled out, which includes "the probes were inconclusive".
    /// That leans toward step 4, and §5.1 says which way to lean: step 5 concretizes and
    /// *continues*, so its mistake is a whole function analysed against the wrong memory,
    /// while step 4's is stopping a path early.
    fn some_object_ruled_out(
        &mut self,
        a: &mut TermArena,
        s: &State,
        addr: Term,
        ranges: &[(chiero_mem::ObjectId, u64, u64)],
        candidates: &[chiero_mem::ObjectId],
        cap: usize,
    ) -> bool {
        // **From both ends, alternating.** A constraint that bounds an address —
        // `x < &d`, `p >= base` — rules out objects at one *end* of the address space, so
        // probing the first `cap` in placement order can spend every query on the objects
        // most likely to be feasible and conclude nothing. 021 contract 17's own fixture
        // is exactly that shape.
        let mut rest: Vec<(chiero_mem::ObjectId, u64, u64)> = ranges
            .iter()
            .copied()
            .filter(|(id, _, _)| !candidates.contains(id))
            .collect();
        let mut order: Vec<(chiero_mem::ObjectId, u64, u64)> = Vec::new();
        while !rest.is_empty() {
            order.push(rest.remove(0));
            if let Some(last) = rest.pop() {
                order.push(last);
            }
        }
        for (_, base, size) in order.into_iter().take(cap) {
            let lo = a.bv(64, base as u128);
            let hi = a.bv(64, base.wrapping_add(size) as u128);
            let below = a.ult(addr, lo);
            let in_lo = a.not(below);
            let lt_hi = a.ult(addr, hi);
            let eq_hi = a.eq(addr, hi);
            let in_hi = a.or(lt_hi, eq_hi);
            let inside = a.and(in_lo, in_hi);
            if matches!(self.feasible(a, s, inside), Feas::No) {
                return true;
            }
        }
        false
    }

    /// 021 §5.1 step 4: the value is unconstrained, so the path ends here.
    ///
    /// **Not** step 5 — concretizing an unconstrained pointer to some object and
    /// reporting `Bounded` is the failure §5.1 calls the highest-value instance of
    /// "wrong answer instead of honest unknown".
    fn unresolvable_pointer(&mut self, s: &mut State, span: Span) {
        self.finding_seq += 1;
        s.findings.push(StateFinding {
            id: self.finding_seq,
            key: None,
            message: "unresolvable pointer: the value is unconstrained, so it could \
                      refer to any object or to none"
                .to_string(),
            span,
        });
        s.degrade(
            Fidelity::Unknown,
            AssumptionKind::NoInformation,
            span,
            "a symbolic pointer with no constraint at all was not resolved",
        );
        s.status = Status::Terminated(TermReason::Unsupported);
    }

    fn resolve_symbolic_base(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        addr: Term,
        span: Span,
    ) -> Option<Value> {
        // **Freed objects are searchable** (021 §4): the access through the resolved
        // pointer is what reports the use-after-free, and a search that cannot see the
        // object reports a wild pointer at address 0 instead.
        let ranges = s.mem.resolvable_ranges();
        let cap = self.budget.max_resolutions as usize;
        // **The address term itself decides which objects to ask about.** §5.1 requires
        // the search be "over an interval tree keyed on base address, not a linear scan",
        // because §8 concedes a VPP entry point may exceed 10⁴ objects and "a
        // per-dereference O(objects) solver sweep is not viable". The previous loop was
        // exactly that sweep: 36 unrelated allocas cost 36 extra solver queries for an
        // address pinned to one value.
        //
        // So the search is model-driven. Each query asks for *some* value the address can
        // still take; the object containing that value is found by arithmetic, and the
        // next query excludes it. Cost is one query per answer plus one to prove there
        // are no more — proportional to what the address can name, not to what exists.
        // (The per-model containment lookup is a linear scan of placements. It is
        // arithmetic, not solver time, and is where an interval tree goes when the object
        // count makes even that matter.)
        let inside = |a: &mut TermArena, base: u64, size: u64| -> Term {
            let lo = a.bv(64, base as u128);
            let hi = a.bv(64, base.wrapping_add(size) as u128);
            // Only `ult`, `eq`, `not` and `or` exist in the arena, so the range test is
            // built from them: `!(addr < lo) && (addr < hi || addr == hi)`. The upper
            // bound is inclusive because one-past-the-end is a legal C pointer.
            let below = a.ult(addr, lo);
            let in_lo = a.not(below);
            let lt_hi = a.ult(addr, hi);
            let eq_hi = a.eq(addr, hi);
            let in_hi = a.or(lt_hi, eq_hi);
            a.and(in_lo, in_hi)
        };

        // **A bare variable no constraint mentions can take any value at all.** That is
        // §5.1 step 4 read off the path, with no queries — and asking whether every
        // object is feasible instead is the sweep itself.
        //
        // ⚠️ The variable check is not decoration. An earlier version asked only that no
        // path constraint mention the address's variables, and reported step 4 for
        // `&buf[i]` with an 8-bit `i`: `zext_64(i) + &buf` is mentioned nowhere on the
        // path and is still confined to one object plus a guard gap. **The term's own
        // structure constrains the value**, so "the path says nothing" is not "nothing is
        // known" for anything but a variable. Found by review.
        let unconstrained = a.var_id(addr).is_some_and(|v| {
            s.path.iter().all(|c| {
                let mut cv = Vec::new();
                a.vars_of(*c, &mut cv);
                !cv.contains(&v)
            })
        });
        if unconstrained && !ranges.is_empty() {
            self.unresolvable_pointer(s, span);
            return None;
        }

        let mut candidates: Vec<chiero_mem::ObjectId> = Vec::new();
        let mut excluded: Vec<Term> = Vec::new();
        let mut over_cap = false;
        let mut undecided = false;
        let mut can_be_wild = false;
        let mut complete = false;
        // Each round rules out either one object or one whole gap between objects, and
        // there is one more gap than there are objects. The bound is what keeps an
        // address that can land in many separate gaps from spinning; hitting it is
        // reported as a cut exploration, never as a resolved pointer.
        let rounds = 2 * cap + 4;
        for _ in 0..rounds {
            let m = match self.probe(a, s, &excluded) {
                // Nothing left to name: the set found so far is *exactly* the feasible
                // set, which is what makes step 4's test below sound.
                CheckResult::Unsat => {
                    complete = true;
                    break;
                }
                CheckResult::Unknown(_) => {
                    undecided = true;
                    break;
                }
                CheckResult::Sat(m) => m,
            };
            let Ok(v) = a.eval(&m, addr) else {
                undecided = true;
                break;
            };
            let v = v.bits() as u64;
            match ranges
                .iter()
                .copied()
                .find(|(_, base, size)| v >= *base && v <= base.wrapping_add(*size))
            {
                Some((id, base, size)) => {
                    if candidates.len() == cap {
                        over_cap = true;
                        break;
                    }
                    candidates.push(id);
                    let t = inside(a, base, size);
                    let n = a.not(t);
                    excluded.push(n);
                }
                None => {
                    // A model in no object *is* the wild case, proven once rather than
                    // asked about separately.
                    can_be_wild = true;
                    let (glo, ghi) = s.mem.wild_region_around(v);
                    let t = inside(a, glo, ghi.saturating_sub(glo));
                    let n = a.not(t);
                    excluded.push(n);
                }
            }
        }
        let exhausted = !complete && !over_cap && !undecided;

        // **The solver gave up part-way through.** Whatever was found is a *subset* of
        // what the address can name, so the fork below would not be exhaustive: objects
        // nobody enumerated get no state, and the wild sibling is the negation of every
        // range rather than of the ones that were checked. Reporting that as `Bounded`
        // says "we explored a subset correctly" about an exploration that skipped part of
        // the program, so the path ends at `SolverUnknown` instead — a statement about
        // chiero, which is what it is. Found by review; before, only an enumeration that
        // gave up *immediately* was caught.
        //
        // 023 §7's `SolverUnknown` cause, not §5.1 step 4: the fidelity is the same and
        // the reason is not, and only the reason tells a reader whether to strengthen the
        // program or the solver.
        if undecided {
            self.finding_seq += 1;
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key: None,
                message: "a symbolic pointer could not be resolved: the solver did not \
                          decide which objects its value can fall in"
                    .to_string(),
                span,
            });
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::SolverUnknown,
                span,
                "the solver could not decide a pointer's object set",
            );
            s.status = Status::Terminated(TermReason::Unsupported);
            return None;
        }

        // **Step 4 is decided before step 5**, because "every object *and* nowhere" is a
        // statement about the program and the cap is a statement about chiero. Testing the
        // cap first let the number of objects decide which one a reader was told.
        //
        // A *cut* enumeration — over the cap, or out of rounds — that also proved the
        // address can be outside every object is step 4 as well. Telling "every object and
        // nowhere" from "more than `max_resolutions` objects and nowhere" costs one query
        // per object, which is the sweep §5.1 forbids; and of the two, step 4 is the
        // honest side. §5.1's own argument says why: step 5 concretizes and *continues*,
        // so getting this backwards means the rest of the function reads and writes the
        // wrong memory, while over-reporting step 4 only stops early.
        //
        // ⚠️ An earlier version concretized here, which reverted wave 52: at VPP's object
        // counts the enumeration always ends cut, so step 4 became unreachable exactly
        // where §5.1 was written for. Found by review.
        let cut = over_cap || exhausted;
        let step_four = can_be_wild
            && !ranges.is_empty()
            && if cut {
                // The enumeration did not finish, so "every object" cannot be read off
                // `candidates`. **One counterexample settles it**: an object the address
                // provably cannot be in means the solver knows something, which is step 5.
                // Probing at most `cap` of them keeps the cost tied to the cap rather than
                // to the object count — 021 contract 17's own case is settled in two.
                !self.some_object_ruled_out(a, s, addr, &ranges, &candidates, cap)
            } else {
                candidates.len() == ranges.len()
            };
        if step_four {
            self.unresolvable_pointer(s, span);
            return None;
        }
        if cut {
            // Step 5. The address is confined to objects — the wild case was never
            // proved — and there are more of them than the resolution is allowed to
            // explore, so one is kept and the run says the exploration was cut rather
            // than that the pointer was pinned.
            let why = if over_cap {
                format!(
                    "a symbolic pointer could refer to more than max_resolutions ({cap}) \
                     objects, so it was concretized to one"
                )
            } else {
                format!(
                    "a symbolic pointer's object set was not enumerated within {rounds} \
                     queries, so it was concretized to one"
                )
            };
            s.degrade(
                Fidelity::Approximated,
                AssumptionKind::OpaqueCode,
                span,
                &why,
            );
            // **An empty candidate list here ends the path, not the function.** `?` on
            // `first` returned `None` from a state still marked `Running`, so the
            // assignment was silently dropped — the one `None` return in this function
            // that terminated nothing. Unreachable by argument (a cut enumeration with no
            // candidates means every model landed in a gap, which is the wild case
            // handled above), and an argument of exactly that shape has been wrong here
            // before.
            let Some(first) = candidates.first().copied() else {
                self.unresolvable_pointer(s, span);
                return None;
            };
            // **The offset comes from the address here too.** Handing back byte 0 of the
            // chosen object is the wave-51 D4 bug in a branch that had not been looked at.
            let off = s
                .mem
                .addr_of(first)
                .and_then(|base| self.pinned_offset(a, s, addr, base))
                .unwrap_or(0);
            return Some(Value::Ptr(Pointer { base: first, off }));
        }
        if candidates.is_empty() && can_be_wild {
            // 021 §7.1's own scenario: an address provably in a guard gap belongs to no
            // object. That is a **wild pointer**, not "the value is unconstrained" — it is
            // pinned exactly, and blaming the program for saying nothing when it said
            // something precise is the same conflation, one branch over.
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                span,
                "a symbolic pointer resolved to no known object",
            );
            return Some(Value::Ptr(Pointer {
                base: chiero_mem::ObjectId::UNBOUND,
                off: 0,
            }));
        }
        if candidates.is_empty() {
            // Step 4: no information at all. Not a concretization — the path ends.
            self.unresolvable_pointer(s, span);
            return None;
        }
        // Steps 2 and 3: one state per candidate, plus the wild one when it is feasible.
        // *This* state takes the last candidate so the siblings are the earlier ones and
        // exploration order stays deterministic (023 §3).
        let mut sibs: Vec<Pointer> = candidates
            .iter()
            .map(|id| Pointer { base: *id, off: 0 })
            .collect();
        if can_be_wild {
            sibs.push(Pointer {
                base: chiero_mem::ObjectId::UNBOUND,
                off: 0,
            });
        }
        // **Each state constrains the address to its own object** (§5.1 step 3). Without
        // it every resolved state still believes the address could be anywhere, so a later
        // branch can take a side only possible for a *different* object — a false positive
        // carrying a witness that looks replayable.
        let constrain =
            |a: &mut TermArena, ranges: &[(chiero_mem::ObjectId, u64, u64)], p: Pointer| {
                if p.base == chiero_mem::ObjectId::UNBOUND {
                    // The wild state is the negation of all of them, which is what makes the
                    // fork exhaustive rather than merely plural.
                    let mut acc = a.bv(1, 1);
                    for (_, base, size) in ranges.iter().copied() {
                        let lo = a.bv(64, base as u128);
                        let hi = a.bv(64, base.wrapping_add(size) as u128);
                        let below = a.ult(addr, lo);
                        let lt_hi = a.ult(addr, hi);
                        let eq_hi = a.eq(addr, hi);
                        let in_hi = a.or(lt_hi, eq_hi);
                        let above = a.not(in_hi);
                        let not_in = a.or(below, above);
                        acc = a.and(acc, not_in);
                    }
                    return acc;
                }
                let (_, base, size) = ranges
                    .iter()
                    .copied()
                    .find(|(id, _, _)| *id == p.base)
                    .expect("candidate came from these ranges");
                let lo = a.bv(64, base as u128);
                let hi = a.bv(64, base.wrapping_add(size) as u128);
                let below = a.ult(addr, lo);
                let in_lo = a.not(below);
                let lt_hi = a.ult(addr, hi);
                let eq_hi = a.eq(addr, hi);
                let in_hi = a.or(lt_hi, eq_hi);
                a.and(in_lo, in_hi)
            };
        // **The offset comes from the address, not from zero.** Discarding it made every
        // resolution point at byte 0, so `d[i] = x` wrote `d[0]` for every `i`. Where the
        // path pins the address, the offset is exact; where it does not, the object is
        // known and the offset is not, which is a *bounded* answer rather than a wrong one.
        let offset_in = |eng: &mut Self, a: &mut TermArena, s: &mut State, p: Pointer| -> i64 {
            if p.base == chiero_mem::ObjectId::UNBOUND {
                return 0;
            }
            let Some(base) = s.mem.addr_of(p.base) else {
                return 0;
            };
            // Pinned iff no other offset in the object is feasible: ask whether the
            // address can differ from `base + k` for the k the model gives.
            let Some(k) = eng.pinned_offset(a, s, addr, base) else {
                return 0;
            };
            k
        };
        let mine = sibs.pop().expect("non-empty");
        for p in sibs {
            let mut sib = s.clone();
            sib.id = self.new_id();
            sib.pc.1 = sib.pc.1.wrapping_add(1);
            let c = constrain(a, &ranges, p);
            sib.path.push(c);
            let off = offset_in(self, a, &mut sib, p);
            if let Some(d) = self.pending_dst {
                sib.set_local(d, Value::Ptr(Pointer { off, ..p }));
            }
            // **Each path records how its pointer was obtained.** Degrading only the
            // continuing state left every sibling reporting `Exact` with an empty
            // assumption list, and 023 §7 attaches fidelity to *paths*, not only runs.
            sib.degrade(
                Fidelity::Bounded,
                AssumptionKind::BudgetHit,
                span,
                "a symbolic pointer was resolved by searching the address space",
            );
            self.pending.push(sib);
        }
        let c = constrain(a, &ranges, mine);
        s.path.push(c);
        let mine_off = offset_in(self, a, s, mine);
        let mine = Pointer {
            off: mine_off,
            ..mine
        };
        s.degrade(
            Fidelity::Bounded,
            AssumptionKind::BudgetHit,
            span,
            "a symbolic pointer was resolved by searching the address space",
        );
        Some(Value::Ptr(mine))
    }

    /// The offset within `base` that `addr` is pinned to, if the path admits exactly one.
    ///
    /// Asking "is any *other* offset feasible" is what makes this an answer rather than a
    /// guess: a model alone would name one of many, and using it would fabricate a
    /// position the program never chose.
    fn pinned_offset(
        &mut self,
        a: &mut TermArena,
        s: &State,
        addr: Term,
        base: u64,
    ) -> Option<i64> {
        let model = {
            self.solver_calls += 1;
            let solver = self.solver.as_mut()?;
            match solver.check(a, &s.path) {
                CheckResult::Sat(m) => m,
                _ => return None,
            }
        };
        let v = a.eval(&model, addr).ok()?.bits() as u64;
        // Could it be anything else? If so the offset is not pinned and claiming one
        // would be worse than admitting the object is all that is known.
        let k = a.bv(64, v as u128);
        let ne = a.eq(addr, k);
        let differs = a.not(ne);
        if matches!(self.feasible(a, s, differs), Feas::No) {
            Some(v.wrapping_sub(base) as i64)
        } else {
            None
        }
    }

    /// A pointer's address as a term, **remembering where it came from** (021 §7.1). The
    /// origin is recorded rather than recovered: the address alone cannot say which
    /// object it was once that object is freed. Shared by `PtrToInt` and by storing a
    /// pointer, so a pointer that goes through memory recovers exactly like one that goes
    /// through a `uintptr_t`.
    fn address_term(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        p: Pointer,
        span: Span,
    ) -> Option<Term> {
        // **`NULL` is address zero**, and it is not in the address space — ids start at
        // 1, so `addr_of` will never answer for it. Without this, `p->next = NULL` was a
        // lowering gap: the store never happened and the reload invented an
        // uninitialized-read about memory the program had just written.
        let base = if p.base == ObjectId::NULL {
            0
        } else {
            let Some(b) = s.mem.addr_of(p.base) else {
                self.lowering_gap(s, span, "the address of an unplaced object");
                return None;
            };
            b
        };
        let t = a.bv(64, base.wrapping_add(p.off as u64) as u128);
        s.remember_provenance(t, p);
        Some(t)
    }

    /// The object for a file-scope variable, allocated on first use. One per `GlobalId`,
    /// since `&counter` twice is one address — and a `const` global is `readonly`, which
    /// is what makes a write to it a finding and what keeps a havoc off a string literal.
    fn global_object(&mut self, s: &mut State, g: GlobalId) -> ObjectId {
        if let Some(o) = self.global_objs.get(&g) {
            return *o;
        }
        let decl = self.module.globals.iter().find(|d| d.id == g).cloned();
        let (size, align, is_const, span) = match &decl {
            Some(d) => (d.size, d.align, d.is_const, d.span),
            // A global the module never declared: zero-sized, so every access to it
            // faults rather than reading bytes chiero invented.
            None => (0, 1, false, Span::DUMMY),
        };
        let o = s.mem.alloc(ObjKind::Global, size, align, span);
        if is_const {
            s.mem.set_readonly(o);
        }
        self.global_objs.insert(g, o);
        o
    }

    /// The object standing for `id`'s code. Zero-sized: nothing reads a function's bytes,
    /// and a non-zero size would put it in the address space's range search where an
    /// integer near it would resolve to it.
    fn func_object(&mut self, s: &mut State, id: FuncId) -> ObjectId {
        if let Some(o) = self.func_objs.get(&id) {
            return *o;
        }
        let span = self
            .module
            .funcs
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.span)
            .unwrap_or(Span::DUMMY);
        let o = s.mem.alloc(ObjKind::Function, 0, 1, span);
        self.func_objs.insert(id, o);
        o
    }

    fn func_of_object(&self, o: ObjectId) -> Option<FuncId> {
        self.func_objs
            .iter()
            .find(|(_, v)| **v == o)
            .map(|(k, _)| *k)
    }

    /// The lane width of the vector this rvalue produces, where it produces one.
    ///
    /// Recorded rather than derived: `Splat` and `InsertLane` know it from the element
    /// they were given, and `Shuffle` from its operand and mask. A vector arriving any
    /// other way — loaded from memory, returned by a call — has no recorded shape, and an
    /// `ExtractLane` on it is a gap rather than a guess at how the bits divide.
    fn lane_width_produced(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        rv: &RValue,
    ) -> Option<u32> {
        match rv {
            RValue::Splat { elem, .. } => self.scalar(a, s, elem).map(|t| a.width(t)),
            RValue::InsertLane { val, .. } => self.scalar(a, s, val).map(|t| a.width(t)),
            // The result's lanes are the *operand's* width, however long the mask is.
            RValue::Shuffle {
                a: Operand::Value(id),
                ..
            } => s.lane_width_of(*id),
            RValue::Use(Operand::Value(v)) => s.lane_width_of(*v),
            _ => None,
        }
    }

    /// A concrete operand value, or `None`. A symbolic size is 023 §10's territory and is
    /// owed; guessing one would move or resize a real write.
    fn concrete_size(&mut self, a: &mut TermArena, s: &mut State, o: &Operand) -> Option<u64> {
        match self.operand(a, s, o) {
            Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits() as u64),
            _ => None,
        }
    }

    /// Memory faults from an instruction become findings and degrade, the same way a
    /// model's do. A fault that only reached the memory model is a bug chiero found and
    /// did not report.
    fn report_faults(&mut self, s: &mut State, faults: &[chiero_mem::MemFault], span: Span) {
        // 021 §5 step 3: misalignment is **recorded** on every access and is a *finding*
        // only in `ub-strict` mode, because x86-64 tolerates it and VPP relies on that.
        // Reporting it unconditionally fired on every `CLIB_PACKED` packet header — and
        // the check is against the object's *declared* alignment, so the address is not
        // even involved. There is no `ub-strict` mode yet; when there is, this is where
        // it goes.
        let faults: Vec<_> = faults
            .iter()
            // Matched on the **variant**, not the slug. Renaming the slug in chiero-mem
            // kept the whole workspace green while restoring the `CLIB_PACKED`
            // false-positive storm, because the test that guards this greps the same
            // literal — both sides of the coupling break together.
            .filter(|f| !matches!(f, chiero_mem::MemFault::Misaligned { .. }))
            .cloned()
            .collect();
        let faults = &faults[..];
        for f in faults {
            self.finding_seq += 1;
            let key = FindingKey {
                kind: f.kind(),
                span: f.at(),
                object: f.object(),
                func: s.func(),
            };
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key: Some(key),
                message: f.to_string(),
                span: f.at(),
            });
        }
        // **A finding is not automatically a degradation.** A null dereference or a bad
        // free is a definite fact about the program that chiero modeled exactly; saying
        // the run is less than `Exact` there would claim chiero was unsure when it was
        // not, and 023 §7 rule 3 wants degradations to mean something. What does degrade
        // is a value chiero *invented* — an uninitialized read is right about the bug and
        // wrong about the bytes, so everything computed from it is unsound.
        if faults.iter().any(|f| f.yields_unknown_value()) {
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                span,
                "a memory access could not produce the program's value",
            );
        }
        // **The path ends at a definite crash.** Everything reported before it is real;
        // everything after it would be about a program that does not exist. Fidelity is
        // untouched — chiero modeled the crash exactly, and this is the one place where
        // ending a path is the *precise* answer rather than a retreat.
        if let Some(f) = faults.iter().find(|f| f.is_fatal()) {
            s.status = Status::Terminated(TermReason::Crashed);
            let _ = f;
        }
    }

    /// 023 §5 / 024 §1 step 4: **a call chiero did not perform invalidates what it was
    /// handed.** Without this, `int x = 0; unknown(&x); if (x == 0) …` leaves the engine
    /// believing `x` is still zero and pruning the real path — the findings on the
    /// surviving paths are false and the absences are wrong. Fidelity was already
    /// `Approximated`, so nothing was *sealed*; the reports were simply untrue.
    ///
    /// `Symbolic` is 024 §2.1's default for an unmodeled extern: a callee that wrote
    /// something meaningful is the common case, and `Uninitialized` would fire on every
    /// buffer a callee legitimately filled.
    fn havoc_args(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        name: &str,
        args: &[Operand],
        span: Span,
    ) {
        let spec = HavocSpec::unmodeled_extern();
        let objs: Vec<ObjectId> = args
            .iter()
            .filter_map(|o| match self.operand(a, s, o) {
                Some(Value::Ptr(p)) => Some(p.base),
                _ => None,
            })
            .collect();
        if objs.is_empty() {
            return;
        }
        let spec = HavocSpec {
            objects: objs,
            ..spec
        };
        self.apply_havoc(a, s, name, &spec, span);
    }

    /// Perform a `HavocSpec`, whichever produced it. 024 §2.1 requires the fidelity effect
    /// to be identical for the default fallback and for a model that chose to havoc, and
    /// the only way to be sure of that is for both to go through here.
    fn apply_havoc(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        name: &str,
        spec: &HavocSpec,
        span: Span,
    ) {
        if spec.objects.is_empty() {
            return;
        }
        let fill = match spec.init {
            HavocInit::Symbolic => HavocFill::Symbolic,
            HavocInit::Uninitialized => HavocFill::Uninitialized,
        };
        let hit = s
            .mem
            .havoc(a, &spec.objects, spec.reachable_depth, fill, span);
        // The havoc's *own* reason, so a reader can tell "chiero threw away what it knew
        // about these objects" from "the model was imprecise".
        self.note_once(
            s,
            AssumptionKind::ModelApproximate,
            span,
            &format!(
                "`{name}`: {} — {} object(s) invalidated",
                spec.describe(),
                hit.objects.len()
            ),
        );
        // A pointer scan that did not finish means reachable objects may still hold
        // stale contents, which is a strictly worse kind of ignorance than the havoc
        // itself: the havoc is recorded and this would not be.
        if hit.truncated {
            self.note_once(
                s,
                AssumptionKind::NoInformation,
                span,
                &format!(
                    "`{name}`: the havoc's pointer scan did not finish, so objects \
                     reachable from these may still hold stale contents"
                ),
            );
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                span,
                "an incomplete havoc cannot bound what it missed",
            );
        }
    }

    /// Run a model and fold its result back into the state.
    ///
    /// The translation is the whole job: a model wants `Pointer`s and concrete sizes, and
    /// the engine has `Operand`s. Every argument is resolved **before** the context is
    /// built, because `ModelCtx` borrows memory and the arena for the duration.
    ///
    /// Anything that will not translate is a *gap* rather than a silent skip — the point
    /// of dispatch is to stop a registration standing in for a call.
    fn dispatch(&mut self, a: &mut TermArena, s: &mut State, c: DispatchCall<'_>) {
        let DispatchCall {
            name,
            dst,
            args,
            ret_ty,
            span,
        } = c;
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
        let mut keyed: Vec<(Option<chiero_mem::MemFault>, String)> = Vec::new();
        let mut result: Option<Value> = None;
        let mut forks: Vec<Option<Value>> = Vec::new();
        let mut terminate: Option<String> = None;
        let mut havoc: Option<HavocSpec> = None;
        let mut unresolved_args = false;
        let mut gave_up: Option<String> = None;
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
                "longjmp" => Some(models::longjmp(&mut cx)),
                "scanf" => {
                    // An argument chiero could not translate is not the absence of an
                    // output buffer. Saying nothing here means a buffer the callee writes
                    // keeps its old contents and every later read of it is confidently
                    // wrong — the failure `havoc_args` exists to prevent, arrived at from
                    // the inside.
                    if resolved
                        .iter()
                        .skip(1)
                        .any(|v| !matches!(v, Some(Value::Ptr(_))))
                    {
                        unresolved_args = true;
                    }
                    // **Positional**, with a hole where an argument was not a pointer.
                    // Filtering first renumbered the arguments under the model's feet.
                    let ps: Vec<Option<Pointer>> = resolved
                        .iter()
                        .map(|v| match v {
                            Some(Value::Ptr(p)) => Some(*p),
                            _ => None,
                        })
                        .collect();
                    Some(models::scanf(&mut cx, &ps))
                }
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
                    // **Keyed by the call site.** A model giving up has no `MemFault`
                    // behind it, but the *call* is what identifies the bug: one `strcpy`
                    // that cannot scan its source is one report however many times the
                    // loop runs. Deduplicating on the text would not do it — two
                    // iterations produce different messages, because the first one
                    // partly wrote the destination.
                    gave_up = Some(msg);
                    translated = false;
                }
                // 024 contract 20. Not a `Finding`: a finding leaves the state running,
                // and the whole point is that everything after this call is a path the
                // program does not have.
                Some(ModelOutcome::Terminate(why)) => {
                    terminate = Some(why);
                }
                // 024 contract 21c. A model that chose its own `HavocSpec` knows more
                // than the fallback does — which objects it actually writes through —
                // and the fidelity effect is identical either way.
                Some(ModelOutcome::Havoc(spec)) => {
                    havoc = Some(spec);
                }
                // **No catch-all.** Every `ModelOutcome` variant is handled, so adding
                // one is a compile error here rather than a silent `translated = false`
                // — which is how `Finding`'s payload was dropped and how `Havoc` was
                // swallowed, both found by review rather than by the compiler.
                None => translated = false,
            }
            // Each report arrives already paired with the fault behind it, so a model's
            // finding gets 023 §6.1's key exactly as a `Store`'s does. A report the model
            // made itself has no fault to key on and keeps the fork identity.
            keyed.extend(cx.reports().iter().cloned());
        }

        if let Some(msg) = gave_up {
            self.finding_seq += 1;
            let key = FindingKey {
                kind: "model-gave-up",
                span,
                object: None,
                func: s.func(),
            };
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key: Some(key),
                message: msg,
                span,
            });
        }
        for (fault, text) in keyed {
            self.finding_seq += 1;
            let key = fault.as_ref().map(|f| FindingKey {
                kind: f.kind(),
                span: f.at(),
                object: f.object(),
                func: s.func(),
            });
            let at = fault.as_ref().map_or(span, |f| f.at());
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key,
                message: text,
                span: at,
            });
        }
        if let (Some(d), Some(v)) = (dst, result) {
            s.set_local(d, v);
        } else if let Some(d) = dst
            && translated
        {
            // The model ran and produced no value; the caller still needs *something*,
            // and a fresh symbol is the honest one.
            self.fresh_count += 1;
            // The **declared** return type. A hardcoded `BitVec(64)` also overwrote the
            // correctly-sorted value the extern path had just set, so the sort got worse
            // by dispatching a model than by not having one.
            let t = self.input(
                a,
                s,
                sort_of(ret_ty),
                &format!("model{}", self.fresh_count),
                InputOrigin::ModelReturn {
                    func: name.to_string(),
                    span,
                },
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
        if let Some(spec) = havoc {
            self.apply_havoc(a, s, name, &spec, span);
        }
        if unresolved_args {
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                span,
                &format!(
                    "`{name}`: an output argument could not be resolved to a pointer, so \
                     whatever it points at was left untouched and may be stale"
                ),
            );
        }
        if let Some(why) = terminate {
            s.status = Status::Terminated(TermReason::Unsupported);
            s.degrade(Fidelity::Unknown, AssumptionKind::NoInformation, span, &why);
            return;
        }
        if !translated {
            self.note_once(
                s,
                AssumptionKind::UnmodeledCall,
                span,
                &format!("`{name}` could not be dispatched with these arguments"),
            );
            // **A call that was not performed invalidates what it was handed**, and
            // `can_dispatch` is a *name* check — passing it and then failing per-call
            // translation is a call chiero did not perform. `memcpy(buf, src, n)` with a
            // non-constant `n` left `buf` believed intact, and `strcpy`'s overflow arm
            // reported the overflow while still believing the destination was fine.
            self.havoc_args(a, s, name, args, span);
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
            // The harness's own words. A fixed sentence made the one mechanism an author
            // has for saying *why* a region is approximate say the same thing every time.
            _ => {
                let why = args
                    .first()
                    .and_then(|o| match self.operand(a, s, o) {
                        Some(Value::Ptr(p)) => s.mem.c_string_at(p),
                        _ => None,
                    })
                    .unwrap_or_else(|| "harness marked this region approximate".to_string());
                intrinsics::mark_fidelity(&why)
            }
        };
        match out {
            IntrinsicOutcome::Continue | IntrinsicOutcome::Constrain => {}
            IntrinsicOutcome::KillState => {
                s.status = Status::Terminated(TermReason::Return);
            }
            IntrinsicOutcome::Finding(f) => {
                self.finding_seq += 1;
                s.findings.push(StateFinding {
                    id: self.finding_seq,
                    key: None,
                    message: f,
                    span,
                });
            }
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
        chiero_model::dispatchable().contains(&resolve_builtin(name))
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
    fn indirect(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        op: &Operand,
        dst: Option<ValueId>,
        span: Span,
    ) {
        // **A pointer chiero can resolve is not a pointer chiero cannot resolve.** The
        // unresolvable sibling exists because the candidate list would otherwise be
        // implicitly exhaustive — but when the operand names one function there is no
        // list to be wrong about, and forking over every defined function is the safe
        // answer to a question already answered.
        if let Some(Value::Ptr(p)) = self.operand(a, s, op)
            && p.off == 0
            && let Some(id) = self.func_of_object(p.base)
        {
            self.direct_into(s, id, dst, span);
            return;
        }
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
            s.give_up("call to an unknown function".into(), span);
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
            ptr_vals: IndexMap::new(),
            varargs: Vec::new(),
            lane_w: IndexMap::new(),
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

    fn operand(&mut self, a: &mut TermArena, s: &mut State, o: &Operand) -> Option<Value> {
        match o {
            Operand::Value(v) => s.stack.last().and_then(|f| f.locals.get(v)).copied(),
            Operand::Const(Const::Int { bits, val }) => {
                Some(Value::Scalar(a.bv(*bits, *val as u128)))
            }
            Operand::Const(Const::Null) => Some(Value::Ptr(Pointer {
                base: chiero_mem::ObjectId::NULL,
                off: 0,
            })),
            // **The constant forms of the two `AddrOf` rvalues.** A frontend emits these
            // wherever an address is a compile-time constant, so treating them as
            // unrepresentable made `&g` a lowering gap in exactly the places it is most
            // ordinary. They go through the same per-id object tables, because a second
            // object for one global makes `p == &counter` false against itself.
            Operand::Const(Const::GlobalAddr { g, off }) => {
                let base = self.global_object(s, *g);
                Some(Value::Ptr(Pointer { base, off: *off }))
            }
            Operand::Const(Const::FuncAddr(id)) => {
                let base = self.func_object(s, *id);
                Some(Value::Ptr(Pointer { base, off: 0 }))
            }
            // `Float`, `Wide` and `Undef` remain gaps: 023 §7 approximates floating point,
            // and inventing a value for `Undef` is the opposite of what it means.
            _ => None,
        }
    }

    fn scalar(&mut self, a: &mut TermArena, s: &mut State, o: &Operand) -> Option<Term> {
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

/// 024 §1 step 3: `__builtin_x` is `x`, when `x` is something chiero models.
///
/// Not a blanket strip. `__builtin_expect`, `__builtin_frame_address` and friends have no
/// non-prefixed counterpart, and aliasing them to a name that does not exist would turn a
/// clear "no model" into a confusing lookup miss for a different symbol.
fn resolve_builtin(name: &str) -> &str {
    match name.strip_prefix("__builtin_") {
        Some(base) if chiero_model::dispatchable().contains(&base) => base,
        _ => name,
    }
}

/// The call `dispatch` is being asked to perform. A struct rather than eight positional
/// parameters, four of which are `Option`s and references that read the same at a call
/// site.
struct DispatchCall<'c> {
    name: &'c str,
    dst: Option<ValueId>,
    args: &'c [Operand],
    ret_ty: &'c CTy,
    span: Span,
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

/// The **bit** width of an integer-ish type. `None` for anything a bit-vector cast cannot
/// express — floats and vectors are 023 §7's approximated territory, and returning a
/// plausible width for them would silently reinterpret a `double` as an integer.
/// Whether an access's faults mean the value it returned must not be used. See
/// `MemFault::yields_unknown_value`: a definite bug is a fact about the program, but an
/// invented value poisons everything computed from it.
fn unusable(faults: &[chiero_mem::MemFault]) -> bool {
    faults.iter().any(|f| f.yields_unknown_value())
}

fn bits_of_cty(t: &CTy) -> Option<u32> {
    match t {
        CTy::Int(b) => Some(*b),
        CTy::Ptr => Some(64),
        _ => None,
    }
}

/// The bit-vector sort a value of `ty` occupies.
///
/// **Every arm is written out.** The `_ => 64` fallthrough made a *faulting* `f32` load
/// yield 64 bits where a succeeding one yields 32 — a width that depended on whether the
/// bytes behind it happened to be written. 023 §7 approximates floating point, but it
/// approximates an `f32` at *its own width*; the bits are the bits (021 §3).
fn sort_of(ty: &CTy) -> chiero_solver::Sort {
    chiero_solver::Sort::BitVec(match ty {
        CTy::Int(b) => *b,
        CTy::Ptr => 64,
        CTy::Float(FloatKind::F32) => 32,
        CTy::Float(FloatKind::F64) => 64,
        CTy::Float(FloatKind::X87_80) => 80,
        CTy::Vector { .. } | CTy::Void => (size_of_cty(ty) * 8).clamp(1, 128) as u32,
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
