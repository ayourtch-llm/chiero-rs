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
use chiero_solver::{CheckResult, PathCondition, SmtLib, Solver, Term, TermArena, TieredSolver};
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

/// Something the outside world can see, in the order the path did it (020 §4.2).
///
/// A device register written twice was written twice, and the sequence is what a driver's
/// correctness depends on — so this is a list, not a set, and nothing coalesces entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    pub kind: EffectKind,
    pub span: Span,
    pub detail: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// A `Volatility::Volatile` store.
    VolatileStore,
}

/// What a model-fork branch carries besides its value: a report about the program, or a
/// bound chiero chose. See `ModelOutcome`.
enum BranchNote {
    Finding(String),
    Bounded(String),
}

/// A defined-but-undefined-behaviour operation, as 020 §4.1 wants it recorded.
///
/// §4.1's separation: "defined IR semantics, UB reported as findings". The *value* is
/// always the SMT-LIB one so the IR and the solver cannot disagree, the path always
/// continues, and this is what a `Checker` (040) later turns into a finding. The engine
/// deliberately does not decide whether it is a bug — `memcpy(d, s, n - 1)` with `n = 0`
/// wraps on purpose all over VPP, and a checker with the context is what tells those
/// apart from the mistakes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UbEvent {
    /// The condition under which this is a fault, when the path does not already imply it.
    ///
    /// Empty for a UB the interpreter read off two constants — `100 / 0` faults for every
    /// input, so there is nothing to require. Non-empty when the engine had to *ask*: the
    /// divisor of `100 / (x - 42)` is zero only at `x == 42`, and a witness that does not
    /// say so names an input under which nothing faults.
    ///
    /// It travels with the event because the checker that reports it is the only thing
    /// that knows which event a given report is about.
    pub requires: Vec<Term>,
    pub kind: UbKind,
    pub span: Span,
    /// What the operation was, for a report a reader can act on.
    pub detail: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UbKind {
    /// Signed `Add`/`Sub`/`Mul` whose mathematical result does not fit.
    SignedOverflow,
    /// `Shl`/`LShr`/`AShr` by at least the operand width.
    Shift,
    /// `UDiv`/`SDiv`/`URem`/`SRem` by zero.
    DivByZero,
    /// A signed `Add`/`Sub`/`Mul` the path **allows** to overflow without forcing it.
    ///
    /// A different kind from `SignedOverflow` rather than the same kind with a softer detail,
    /// because 023 §6.1 makes the kind half the dedup key: a reader grouping findings has to be
    /// able to separate "this overflows" from "this can overflow", exactly as the memory channel
    /// separates `out-of-bounds` from `may-be-out-of-bounds`.
    MaybeSignedOverflow,
    /// A float-to-integer conversion whose value the destination cannot represent, NaN
    /// included (C11 6.3.1.4).
    ///
    /// The odd one out: every other kind here is a *binary operation* and reaches `note_ub`
    /// from `RValue::Bin`. A conversion is a `Cast`, so this is detected where the
    /// conversion is performed — which is also the only place that knows the value.
    FloatCastOverflow,
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
    /// The fault's other place, carried through to `Finding::related`. `None` for everything
    /// that is about a single moment — which is every refusal and every single-event fault.
    related: Option<chiero_mem::SecondEvent>,
    /// What a witness must satisfy to reproduce *this* report, beyond the path.
    requires: Vec<Term>,
    /// Solved from `requires`; `None` falls back to the state's witness.
    witness: Option<Witness>,
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
    /// **The other place this finding is about**, where there is one — the `free`, or the
    /// scope's end. `span` is where the bug *is*; this is what made it one.
    ///
    /// Carried as data because the message carries it as prose, and a consumer offering "jump to
    /// the free" should not have to parse a sentence to do it.
    pub related: Option<chiero_mem::SecondEvent>,
    /// The fidelity of the path this was found on — not the run's. A definite fault on an
    /// `Exact` path stays actionable in a run some *other* path degraded.
    pub fidelity: Fidelity,
    /// **022 §4: which solver decided the path this was found on.**
    ///
    /// `solver-lite` when tier 1 answered alone. Carried on the finding rather than left to
    /// the run because that is the clause's stated purpose — a reader looking at one report
    /// should not have to go back to the run to learn what stands behind it, and an upstream
    /// solver bug is reported *from* a finding.
    pub solver: String,
}

/// How big an object an entry function's pointer parameter points at. The caller is
/// outside the analysis, so there is no right answer — this is a *bound chiero chose*, and
/// an access past it is reported as one rather than silently allowed.
pub const ENTRY_PARAM_BYTES: u64 = 4096;

/// The one degradation a later proof can answer.
///
/// Written once and matched against, rather than spelled at each site: discharging it means
/// recognising it, and a reason recognised by a string literal repeated in three places is
/// a reason that stops being discharged the first time someone rewords one of them.
const BRANCH_UNDECIDED: &str = "solver could not decide a branch; both sides explored";

/// 023 §7 rule 3: every degradation names its cause. "Approximated with no reason" is a
/// bug, so this is recorded at the point of degradation, never after the fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assumption {
    pub kind: AssumptionKind,
    pub span: Span,
    pub detail: String,
    /// The fidelity this degradation imposed.
    ///
    /// `State::fidelity` is a running worst-of, which cannot be undone — and one
    /// degradation *can* be answered later. "The solver could not decide this branch" says
    /// the path may not exist; a validated model of the path condition proves it does, and
    /// at that point the reason is discharged rather than merely outvoted. Recomputing the
    /// worst over what remains needs each assumption to remember its own severity.
    pub fidelity: Fidelity,
}

/// 023 §1.1. A pointer keeps its object; a scalar is a term.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Scalar(Term),
    Ptr(Pointer),
    /// 020 §4.1's `Const::Undef`: the program left this undecided, and so does chiero.
    ///
    /// **Not a fresh symbol.** A symbol is a value nobody has pinned *yet* and the solver
    /// may pin later; `Undef` is a value that does not exist, and letting the solver
    /// choose one is choosing for the program. Arithmetic on it stays `Undef` — including
    /// `undef * 0`, which is 0 for every value of the operand and undefined for a
    /// non-value.
    Undef,
    /// **A pointer whose offset is symbolic** — an object, and a term for where in it.
    ///
    /// `Pointer::off` is a concrete `i64`, which is why this needs its own variant rather
    /// than a wider `Pointer`: an address the program computed from an unknown index has no
    /// one offset to put there, and inventing one is what
    /// `a_symbolic_ptr_add_offset_is_a_gap` forbids.
    ///
    /// **Sites that do not name it refuse, and that is the design.** A new variant acquires
    /// pointer semantics only where written; the 59 places matching `Value::Ptr` keep
    /// today's behaviour, which for them is the honest one — they cannot answer for an
    /// unknown offset. 021 §3 is what makes the *load* answerable, because `chiero-mem` has
    /// carried symbolic offsets since then.
    SymPtr {
        base: ObjectId,
        off: Term,
    },
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
    /// **022 §4's default**: use a backend when one is found on `PATH` at *run* time, and
    /// tier 1 alone when none is.
    ///
    /// Discovery is deliberately not a link-time dependency (010 §1's build rule), so this
    /// is a `Command` lookup and not a feature flag. A machine with no solver behaves
    /// exactly as `LiteOnly` — which is why this can be the default without making the
    /// build depend on anything.
    #[default]
    Discover,
    /// Tier 1 only, whatever is installed.
    ///
    /// Not a performance switch: this is how a test of `SolverLite`'s own incompleteness
    /// says what it is testing. Without it, installing z3 would silently rewrite the
    /// meaning of every such test.
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
    /// Locals whose value depends on pointer bits **below the object's guaranteed
    /// alignment** (021 §7.2). Tracked per `ValueId` rather than per `Term` because the
    /// arena folds `concrete_address & 63` to a constant on construction — by the time a
    /// branch sees the condition, the structure that revealed the question is gone.
    bit_inspected: Vec<ValueId>,
    /// The block this activation was in before the current one, or `None` in the entry.
    ///
    /// **Per activation, for the same reason as `locals`**: a `BlockId` is unique only
    /// within a function, so one field per state would let a returning call resume with
    /// the callee's last block as its predecessor. Only `Phi` reads it, and a phi's whole
    /// meaning is "the value belonging to the edge actually taken".
    prev_block: Option<BlockId>,
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
    /// 023 §6.1: one slot per registered checker, cloned on fork by `CheckerStates`'s
    /// own `Clone` so a `State` clone cannot forget to.
    pub checker_states: CheckerStates,
    pub mem: Memory,
    pub pc: (BlockId, usize),
    /// Append-only (023 §1).
    pub path: Vec<Term>,
    /// One object per accessed arena element index (021 §5.2), keyed by the index term.
    arena_objs: IndexMap<u32, ObjectId>,
    /// One object per file-scope variable, allocated on first use **in this state**.
    global_objs: IndexMap<GlobalId, ObjectId>,
    /// 021 §6: how many links from an entry pointer each lazily-materialized object sits.
    /// The entry's own object is 0.
    lazy_depth: IndexMap<ObjectId, u32>,
    /// The single object every link past `max_depth` points at.
    ///
    /// **One shared object, not a fresh one per hop.** That is what makes the bound a
    /// bound: the walk still reaches the return — §6 bounds materialization, it does not
    /// end the run, and killing the state would lose every finding after the cut and
    /// report their absence as if the code were clean — but it stops creating objects.
    lazy_cut: Option<ObjectId>,
    /// 022 §6.1's `possibly_infeasible`, carried alongside the terms.
    ///
    /// Set whenever a constraint is added **without** a feasibility check. While it is
    /// set, independence slicing and the subset/superset cache rules are disabled for
    /// this state's queries. §6.1 lists three sites in the specification; the engine has
    /// five, the two extra ones being a checker's `Action::Assume` (023 §6) and
    /// `chiero_assume` on a symbolic condition (024 §7).
    ///
    /// **Known limit:** §6.1 says "a single full check that returns `Sat` clears it", and
    /// nothing here clears it — the engine only ever asks feasibility questions *with*
    /// assumptions, which prove something other than the path condition alone. The
    /// consequence is that a state downstream of one solver `Unknown` stays unsliced for
    /// the rest of its life. That is the slow direction, not the wrong one.
    path_unchecked: bool,
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
    /// 020 §4.1's UB events, in the order this path met them.
    ub: Vec<UbEvent>,
    /// 020 §4.2's observable effects, in program order.
    effects: Vec<Effect>,
    /// **Every symbolic input this path ever created**, in creation order (023 §9). The
    /// witness is an assignment to exactly these, so a symbol minted without landing here
    /// is a value the replay harness would leave to chance.
    inputs: Vec<(Term, InputOrigin)>,
    /// Conditions a **finding on this path depends on**, which the path condition does not
    /// already imply.
    ///
    /// A division by zero is the motivating case. `100 / (x - 42)` faults only at
    /// `x == 42`, and nothing in the path condition says so — the division is on the only
    /// path there is. Building the witness from the path alone therefore produced the
    /// filler zero, and the finding named an input under which the divisor is -42.
    ///
    /// **Not `path`.** Putting it there would make the branch feasibility queries treat
    /// `x == 42` as given and prune every execution where the divisor is non-zero, which
    /// is most of the program. This is a constraint on the *witness*, not on the run: it
    /// narrows which model is reported, not which paths exist.
    witness_requires: Vec<Term>,
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
    /// The pointer parameter this state was forked to make null, if it was.
    ///
    /// Carried so a null-dereference report can name its own premise. A null chiero
    /// *assumed* (the caller might pass one) and a null the program *produced* (a failed
    /// `malloc`) read identically otherwise, and they want opposite responses from whoever
    /// reads them — 023 §9's "a report a person cannot act on is not a report".
    ///
    /// A plain `Option<String>`: a state is forked for at most one parameter, since wave
    /// 186 chose one null state per parameter rather than every combination.
    entry_null_param: Option<(ValueId, String)>,
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
            checker_states: CheckerStates::default(),
            path_unchecked: false,
            arena_objs: IndexMap::new(),
            global_objs: IndexMap::new(),
            lazy_depth: IndexMap::new(),
            lazy_cut: None,
            id,
            mem: Memory::new(),
            pc: (BlockId(0), 0),
            path: Vec::new(),
            fidelity: Fidelity::Unknown,
            assumptions: vec![Assumption {
                kind: AssumptionKind::NoInformation,
                span: Span::DUMMY,
                detail: why.to_string(),
                fidelity: Fidelity::Unknown,
            }],
            status: Status::Errored(why.to_string()),
            trace: Vec::new(),
            stack: Vec::new(),
            ret: None,
            edge_counts: IndexMap::new(),
            steps: 0,
            findings: Vec::new(),
            inputs: Vec::new(),
            witness_requires: Vec::new(),
            ub: Vec::new(),
            effects: Vec::new(),
            replay_used: 0,
            witness: None,
            unwitnessed: None,
            ptr_ints: IndexMap::new(),
            entry_null_param: None,
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
    /// Record that `v`'s value came from inspecting pointer bits the allocator chose.
    fn mark_bit_inspected(&mut self, v: ValueId) {
        if let Some(f) = self.stack.last_mut()
            && !f.bit_inspected.contains(&v)
        {
            f.bit_inspected.push(v);
        }
    }

    fn is_bit_inspected(&self, v: ValueId) -> bool {
        self.stack
            .last()
            .is_some_and(|f| f.bit_inspected.contains(&v))
    }

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

    /// Add a constraint whose feasibility, together with everything already on the path,
    /// has been established — the ordinary branch case.
    fn constrain_checked(&mut self, t: Term) {
        self.path.push(t);
    }

    /// Add one without that check (022 §6.1), which disables slicing for this state.
    fn constrain_unchecked(&mut self, t: Term) {
        self.path.push(t);
        self.path_unchecked = true;
    }

    /// Whether anything on this path was added without a feasibility check.
    pub fn path_possibly_infeasible(&self) -> bool {
        self.path_unchecked
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

    /// 020 §4.1's UB events on this path.
    pub fn ub_events(&self) -> &[UbEvent] {
        &self.ub
    }

    /// 020 §4.2's observable effects on this path, in program order.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
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

    /// Unbind a local. A model-fork branch that produced no value must not inherit the
    /// value the branch before it produced — see the sibling loop in `call_model`.
    fn clear_local(&mut self, v: ValueId) {
        if let Some(f) = self.stack.last_mut() {
            f.locals.shift_remove(&v);
        }
    }

    fn func(&self) -> FuncId {
        self.stack.last().map_or(FuncId(0), |f| f.func)
    }

    /// **Every symbolic input this path created**, in creation order — the entry
    /// parameters first, since they are minted before the first instruction runs.
    ///
    /// Exposed for the symbolic differential oracle, which must name the variable it is
    /// solving for. Reading it off a one-entry `Model` would work only for a program whose
    /// paths never mint a second symbol, and "the model happens to have one variable" is
    /// not a property any fixture should depend on.
    pub fn inputs(&self) -> &[(Term, InputOrigin)] {
        &self.inputs
    }

    /// The returned value **under a model**, for a path whose return is symbolic.
    ///
    /// [`Self::return_value_bits`] evaluates *ground*, so it answers `None` the moment the
    /// return depends on an input — which is every path a symbolic differential test is
    /// about. Given the model that put the path's own condition on, this is the number the
    /// program must produce for those inputs, and therefore the number a compiler running
    /// the same inputs has to agree with.
    pub fn return_value_under(&self, m: &chiero_solver::Model, a: &TermArena) -> Option<u128> {
        match self.ret? {
            Value::Scalar(t) => a.eval(m, t).ok().map(|c| c.bits()),
            Value::Ptr(_) | Value::Undef | Value::SymPtr { .. } => None,
        }
    }

    /// Did this path reach a `return` **with a value**, whether or not that value is ground?
    ///
    /// Distinct from `return_value_bits`, which asks for concrete bits and answers `None` for
    /// a symbolic read — a `select` over an object at an unknown index is a value the path
    /// computed, and "the path produced nothing" is a different claim from "the value is not
    /// a constant". Wave 196 needed the first and only the second was available.
    pub fn returned_a_value(&self) -> bool {
        matches!(self.ret, Some(Value::Scalar(_)) | Some(Value::Ptr(_)))
    }

    /// The returned value **as the engine holds it** — the term, not a number.
    ///
    /// The two accessors either side of this one each answer a question about a *value*:
    /// `return_value_bits` wants ground bits, `return_value_under` wants bits under a
    /// model. A relational comparison (041 §1.2) wants neither — it wants the term itself,
    /// so it can assert that this path's return and the other version's disagree and hand
    /// that to the solver. Rebuilding the term from a model is not the same thing: a model
    /// is one input, and the question is about all of them.
    pub fn return_value(&self) -> Option<Value> {
        self.ret
    }

    /// The returned value as concrete bits, when it is one.
    pub fn return_value_bits(&self, a: &mut TermArena) -> Option<u128> {
        match self.ret? {
            Value::Scalar(t) => a.eval_ground(t).ok().map(|c| c.bits()),
            // Neither has concrete bits, and `Undef` most emphatically does not: the
            // whole point is that no value was chosen.
            Value::Ptr(_) | Value::Undef | Value::SymPtr { .. } => None,
        }
    }

    /// **A validated model of the path condition proves the path exists**, which answers
    /// the one degradation that said it might not.
    ///
    /// When tier 1 cannot decide a branch the engine takes both edges and degrades each
    /// side to `Unknown`: the path may not be real. That is right at the branch and stale
    /// at the end, because `attach_witness` then solves the path condition — and 022 §3.1
    /// makes `Sat` self-certifying, so a model satisfying it *is* a proof of reachability.
    /// Leaving the caveat on labels a reproducible fault as undecided, which is the label a
    /// reader uses to decide what to ignore.
    ///
    /// **Only that reason, and only recomputed.** Everything else a path assumed still
    /// holds — an unmodeled call is not made modeled by the path being real — so the
    /// remaining assumptions are re-folded rather than the fidelity being set outright. A
    /// path with an undecided branch *and* an opaque call keeps `Unknown` from the call.
    fn discharge_unproven_path(&mut self) {
        let before = self.assumptions.len();
        self.assumptions
            .retain(|a| !(a.kind == AssumptionKind::SolverUnknown && a.detail == BRANCH_UNDECIDED));
        if self.assumptions.len() == before {
            return;
        }
        self.path_unchecked = false;
        self.fidelity = self
            .assumptions
            .iter()
            .fold(Fidelity::Exact, |f, a| f.degrade(a.fidelity));
    }

    fn degrade(&mut self, to: Fidelity, kind: AssumptionKind, span: Span, detail: &str) {
        self.fidelity = self.fidelity.degrade(to);
        self.assumptions.push(Assumption {
            fidelity: to,
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
    /// Which solver decided this run (022 §4). Private: it is a fact the run establishes,
    /// not a knob, and `RunResult` is already forgery-proof by construction.
    solver: String,
    pub solver_calls: u64,
    /// 022 §4 wants this at one for a whole run. A per-query spawn shows up immediately.
    pub backend_spawns: u64,
    /// How many solvers the run built. One, or the caches are discarded between queries.
    pub solver_inits: u64,
    /// How many assertions independence slicing kept out of a backend query (022 §6.2).
    ///
    /// Exposed because the alternative is having no way to tell a run that sliced from
    /// one that could not: slicing was implemented, tested in the solver's own suite, and
    /// **never executed by the engine**, because `probe` called `check` rather than
    /// `check_path`. A number in the result is what makes that visible.
    pub sliced_terms_skipped: u64,
    /// The order states *finished*, so a change of searcher is visible in the output
    /// rather than hidden by sorting.
    completion_order: Vec<u32>,
    /// 023 contract 7: the seed the strategy used, recorded whether or not the strategy
    /// had any randomness to spend it on. A reader must not have to know which strategy
    /// ran in order to know how to reproduce it.
    seed: u64,
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

    /// **022 §4: which solver decided this run.**
    ///
    /// Never blank. `solver-lite` says tier 1 answered alone, which is a different fact
    /// from a backend having been used and is not the same as silence — before this,
    /// `backend_spawns == 0` was what a run reported both when tier 1 sufficed and when no
    /// solver was installed, and wave 161 made the choice implicit so nothing else records
    /// it.
    pub fn solver(&self) -> &str {
        &self.solver
    }

    /// 023 contract 7: the seed the strategy used.
    pub fn seed(&self) -> u64 {
        self.seed
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
                related: f.related,
                // **This finding's own witness when it has one.** A report that recorded
                // a condition was solved separately, because the state's single witness
                // cannot satisfy two findings that need different inputs. Falling back to
                // the state's is the wave-158 behaviour and still names a real input for
                // the path.
                witness: f.witness.clone().or_else(|| st.witness.clone()),
                unwitnessed: st.unwitnessed.clone().or_else(|| {
                    // A state that never finished has no witness because nothing tried,
                    // and contract 15 wants that said rather than left blank.
                    st.witness.is_none().then(|| {
                        "the path did not terminate, so no assignment was extracted".to_string()
                    })
                }),
                fidelity: st.fidelity,
                solver: self.solver.clone(),
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

    /// Every UB event on every path, in state order (020 §4.1).
    pub fn ub_events(&self) -> Vec<UbEvent> {
        self.states.iter().flat_map(|s| s.ub.clone()).collect()
    }

    /// A witness bound to *this* run. Minting is unconditional: 023 §7.1 wants exactly
    /// **one** function reading fidelity to decide whether a result is a proof, and that
    /// function is `seal`. Gating here as well made `seal`'s own check unreachable, which
    /// is why contract 13b could not be written.
    pub fn witness(&self) -> ExactWitness {
        ExactWitness { run: self.id }
    }
}

// ===========================================================================
// 023 §6 — checkers
// ===========================================================================

use std::any::Any;

/// An observer of execution (023 §6).
///
/// A checker "sees everything and decides nothing about execution order". It is
/// **stateless**: everything it remembers lives in the `State`, via
/// [`Checker::initial_state`] and [`CheckerCtx::state_mut`]. §6.1 spells out why —
/// the searcher interleaves events from unrelated states arbitrarily, so memory kept in
/// `&mut self` accumulates across paths that have nothing to do with each other, and
/// contract 17's "1, 2 and 8 worker threads produce identical results" becomes
/// unachievable.
pub trait Checker {
    fn name(&self) -> &'static str;

    fn on_event(&mut self, ev: &Event, ctx: &mut CheckerCtx) -> Vec<Action>;

    /// The per-state memory this checker starts each path with. A checker that remembers
    /// nothing can leave this alone.
    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(NoCheckerState)
    }
}

/// What the engine tells checkers about.
///
/// **Only the variants the engine actually emits are defined here.** §6 also specifies
/// `MemFault` and `ArithEvent`; they arrive with the checkers that consume them (040)
/// rather than sitting in the enum unemitted, because a checker matching on a variant
/// that can never fire is indistinguishable from one whose logic is wrong.
#[allow(missing_debug_implementations)]
pub enum Event<'a> {
    BeforeInst {
        st: &'a State,
        inst: &'a Inst,
    },
    AfterInst {
        st: &'a State,
        inst: &'a Inst,
    },
    Fork {
        st: &'a State,
        cond: Term,
        feasible: (bool, bool),
    },
    Call {
        st: &'a State,
        callee: Callee,
        /// **`Option<Value>` per argument, not a compacted list.** An argument chiero
        /// cannot represent — a float, which `operand` documents as a gap — is a *hole*,
        /// and `filter_map`ing it away silently hands a checker the next argument in its
        /// place. The varargs path was fixed for exactly this; this one was not. Found by
        /// review.
        args: &'a [Option<Value>],
    },
    /// Fired **in the caller** once the callee's result exists, for defined, modeled and
    /// unmodeled callees alike (023 contract 24). `Event::Return` fires in the callee's
    /// frame and never fires at all for an unmodeled extern, whose fresh return value has
    /// no return instruction behind it — which is the whole reason this exists.
    CallReturn {
        st: &'a State,
        callee: Callee,
        ret: Option<Value>,
        dst: Option<ValueId>,
    },
    Return {
        st: &'a State,
        val: Option<Value>,
    },
    Terminated {
        st: &'a State,
        why: TermReason,
    },
}

/// What a checker asks the engine to do.
#[allow(missing_debug_implementations)]
pub enum Action {
    Report(String),
    /// A report **with the condition it depends on** (023 §9).
    ///
    /// Separate from `Report` rather than a field on it: most findings need nothing beyond
    /// the path — a null dereference happens because the path reached it — and every
    /// existing checker would otherwise have to say so. A checker reaches for this only
    /// when the event it is reporting carried a condition of its own.
    ReportRequiring {
        message: String,
        requires: Vec<Term>,
    },
    /// Constrain the state and continue. Contract 19: subsequent branch feasibility
    /// reflects it, which means it joins the path condition rather than a side list.
    Assume(Term),
    Kill(TermReason),
}

impl Action {
    /// The common case, spelled so a checker does not have to build a `String` inline.
    ///
    /// **Deduplication is the checker's, not the engine's** (023 §6.1, settled in wave 225). A
    /// report made here carries no §6.1 key, so the engine merges only a *fork's* copies of one
    /// report — by id, because those are one report that a branch duplicated. Two reports from one
    /// checker are two findings however alike they look.
    ///
    /// That is deliberate rather than unfinished. A checker may have two distinct things to say
    /// about one instruction at one span, and the engine cannot tell that from the same thing said
    /// twice; the checker can, because it knows what it was looking for. The mechanism is
    /// `CheckerState`: keep what you have already reported and return nothing the second time.
    /// `chiero_check::UndefinedArithmetic` does this with a `(kind, span)` list, and
    /// `chiero-check/tests/report_dedup.rs` pins both halves — the engine inventing no key, and a
    /// checker with memory reporting once.
    pub fn report(msg: impl Into<String>) -> Action {
        Action::Report(msg.into())
    }

    /// A report whose witness must satisfy `requires` to reproduce it.
    pub fn report_requiring(msg: impl Into<String>, requires: Vec<Term>) -> Action {
        Action::ReportRequiring {
            message: msg.into(),
            requires,
        }
    }
}

/// What `emit` is asked to send, before the `&State` borrow is attached.
///
/// `Event<'a>` ties the state and the payload to one lifetime, so a closure building it
/// would have to hold the payload for *any* state borrow — `'static` in practice. Keeping
/// the payload separate lets `emit` attach the two borrows where they are both live.
enum Ev<'i> {
    BeforeInst(&'i Inst),
    AfterInst(&'i Inst),
    Fork {
        cond: Term,
        feasible: (bool, bool),
    },
    Call {
        callee: Callee,
        args: Vec<Option<Value>>,
    },
    CallReturn {
        callee: Callee,
        ret: Option<Value>,
        dst: Option<ValueId>,
    },
    Return(Option<Value>),
    Terminated(TermReason),
}

/// A checker's per-state memory (023 §6.1).
///
/// **Deviation from §6.1, recorded here rather than silently.** The spec writes
/// `CheckerState: DynClone + Any` with `on_fork` defaulted to `dyn_clone::clone_box`.
/// `dyn_clone` is not a workspace dependency and `cargo xtask check-deps` gates new ones,
/// so `on_fork` is required and the `Any` projections are explicit. The cost is two lines
/// per implementor; the semantics are identical.
pub trait CheckerState: Any {
    /// The child's copy at a fork. §6.1: "cloned on fork alongside `Memory`".
    fn on_fork(&self) -> Box<dyn CheckerState>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// For a checker that remembers nothing.
#[derive(Debug)]
pub struct NoCheckerState;

impl CheckerState for NoCheckerState {
    fn on_fork(&self) -> Box<dyn CheckerState> {
        Box::new(NoCheckerState)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// One `CheckerState` per registered checker, carried by the `State`.
///
/// The fork semantics live in this type's `Clone`, so a `State` clone cannot forget to
/// call `on_fork` — that is the mistake §6.1 warns about, and putting it anywhere else
/// makes it one edit away from returning.
#[derive(Default)]
pub struct CheckerStates(Vec<Box<dyn CheckerState>>);

impl Clone for CheckerStates {
    fn clone(&self) -> Self {
        CheckerStates(self.0.iter().map(|s| s.on_fork()).collect())
    }
}

impl std::fmt::Debug for CheckerStates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CheckerStates({})", self.0.len())
    }
}

/// The only interface through which a checker reaches the solver, the arena, or its own
/// memory (023 §6). Routing it all through one object is what lets §9 insist every
/// finding either carries a counterexample or says why it does not.
#[allow(missing_debug_implementations)]
pub struct CheckerCtx<'a, 'm> {
    a: &'a mut TermArena,
    /// Which registered checker is running, so `state_mut` reaches the right slot.
    idx: usize,
    states: &'a mut CheckerStates,
    module: &'m Module,
    solver: &'a mut TieredSolver,
    path: &'a [Term],
    /// A checker's questions are solver questions. Counting them nowhere made
    /// `RunResult::solver_calls` report zero for a run that asked several. Found by
    /// review.
    solver_calls: &'a mut u64,
}

impl<'a, 'm> CheckerCtx<'a, 'm> {
    pub fn arena(&mut self) -> &mut TermArena {
        self.a
    }

    /// This checker's memory for the current state, downcast to its own type.
    ///
    /// Panics if the type does not match what [`Checker::initial_state`] returned, which
    /// is a programming error in the checker rather than a condition to handle.
    pub fn state_mut<T: CheckerState>(&mut self) -> &mut T {
        self.states.0[self.idx]
            .as_any_mut()
            .downcast_mut::<T>()
            .expect("checker state type does not match the checker's initial_state")
    }

    /// Is `cond` possible on this path?
    ///
    /// **`Unknown` answers `true`**, which is the opposite of `must` and for the same
    /// reason: each errs toward the answer that keeps a checker looking. Collapsing
    /// `Unknown` into `false` here — the shape `matches!(.., Sat(_))` gives you for free —
    /// tells a checker asking "may this pointer be NULL?" that it cannot be, and the bug
    /// disappears with no finding, no assumption and no fidelity change. With the
    /// engine's default tier-1-only solver that is every question outside 022 §3.2's
    /// fragment, which is most of them. Found by review.
    pub fn may(&mut self, cond: Term) -> bool {
        if let Some(b) = self.ground_truth(cond) {
            return b;
        }
        let mut all: Vec<Term> = self.path.to_vec();
        all.push(cond);
        *self.solver_calls += 1;
        !matches!(self.solver.check(self.a, &all), CheckResult::Unsat)
    }

    /// C's truth rule for a term of any width: nonzero is true.
    ///
    /// `eval_ground_bool` alone is `bits() != 0`, and negating a wider-than-1 term with a
    /// *bitwise* `not` does not produce its logical negation — `must(t)` and `must(¬t)`
    /// were both true for a 32-bit `2`, since `!2` is `0xFFFFFFFD`. `Event::Fork` hands a
    /// checker the CIR's branch condition, whose width is whatever the program used, so
    /// this is reachable in ordinary use. Found by review.
    fn ground_truth(&mut self, cond: Term) -> Option<bool> {
        self.a.eval_ground(cond).ok().map(|c| c.bits() != 0)
    }

    /// Is `cond` forced on this path? **`Unknown` answers `false`** — a checker asking
    /// "must" is about to act on certainty, and the solver declining to decide is not it.
    ///
    /// **A ground condition is decided here, not by the solver.** `must(1 == 1)` sends
    /// tier 1 a constant, which is not an atom and so leaves §3.2's fragment: the answer
    /// comes back `Unknown` and `must` says `false` for a tautology. With the engine's
    /// default tier-1-only solver that is every ground question a checker can ask —
    /// including "did this path return the constant I care about", which is the shape
    /// §6.1's own lock example needs.
    pub fn must(&mut self, cond: Term) -> bool {
        if let Some(b) = self.ground_truth(cond) {
            return b;
        }
        // `cond == 0` — the *logical* negation. `a.not` is bitwise and is only the
        // negation at width 1; see `ground_truth`.
        let w = self.a.width(cond);
        let zero = self.a.bv(w, 0);
        let neg = self.a.eq(cond, zero);
        let mut all: Vec<Term> = self.path.to_vec();
        all.push(neg);
        *self.solver_calls += 1;
        matches!(self.solver.check(self.a, &all), CheckResult::Unsat)
    }

    /// The callee's name, for a checker that keys on it. Indirect calls the engine has
    /// not resolved have none.
    /// The module under execution.
    ///
    /// 040's checkers "run over CIR" (020 §7), so a checker that wants to name what it
    /// found — a global, a function — needs the table those names live in. Without this a
    /// finding can only cite an `ObjectId`, which is an engine-internal counter and means
    /// nothing to a reader.
    pub fn module(&self) -> &'m Module {
        self.module
    }

    pub fn callee_name(&self, callee: &Callee) -> &str {
        match callee {
            Callee::Direct(id) => self
                .module
                .funcs
                .iter()
                .find(|f| f.id == *id)
                .map(|f| &f.name[..])
                .unwrap_or("<unknown>"),
            Callee::Indirect(_) => "<indirect>",
        }
    }
}

/// The registered checkers. A newtype only so `Engine` can keep deriving `Debug`;
/// `dyn Checker` cannot.
#[derive(Default)]
pub struct Checkers(Vec<Box<dyn Checker>>);

impl std::fmt::Debug for Checkers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|c| c.name()))
            .finish()
    }
}

/// A registered striding region — 021 §5.2.
///
/// **Three sizes, not two**, and the spec argues the point at length: `index_scale` is
/// the byte width of the program's index unit, `pitch` is the distance between
/// consecutive elements, and `elem_size` is the addressable extent within one. For VPP's
/// buffer pool those are 64, `vlib_buffer_alloc_size()` (~2.5 KB) and the buffer's own
/// extent. Collapsing `index_scale` into `pitch` forces a choice between an `elem_size`
/// of 64 — making every access to `b->data` out of bounds — and overlapping elements,
/// which violates §7's disjointness.
///
/// **Deviation from 021 §5.2, recorded.** The spec's `Arena` carries `base: Term`. A
/// caller cannot name a term before the run starts, so the base is supplied by
/// [`Engine::with_arena`] as an entry-parameter index and bound when the entry state is
/// built. `chiero-vpp` (060) will register the buffer pool the same way, from whichever
/// value holds `buffer_mem_start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaShape {
    /// Bytes between consecutive elements. Elements are disjoint, so
    /// `elem_size <= pitch`.
    pub pitch: u64,
    /// Addressable bytes within an element; `[elem_size, pitch)` is a gap.
    pub elem_size: u64,
    /// Bytes per unit of the *index* the program uses, which is not the pitch.
    pub index_scale: u64,
    /// How many elements the region holds.
    pub count: u64,
}

/// Run ids are distinct so a witness cannot bless a different run (023 §7.1). This does
/// not affect determinism, which is about the `StateId` sequence *within* a run.
static NEXT_RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// How lazily-materialized objects are shaped and bounded (021 §6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LazyPolicy {
    /// Bytes for a pointer to a scalar or struct.
    pub scalar_extent: u64,
    /// A pointer used with `PtrAdd` gets `scalar_extent * array_factor`.
    pub array_factor: u64,
    /// How deep the recursion of linked structures may go — `p->next->next->…`.
    ///
    /// Without a bound this does not terminate: every pointer read out of a lazy object is
    /// another symbolic pointer, and materializing on demand walks an infinite list.
    pub max_depth: u32,
    /// Two lazily-materialized objects are distinct unless `--fork-on-alias`.
    pub distinct_by_default: bool,
}

impl Default for LazyPolicy {
    fn default() -> Self {
        // 021 §6's stated defaults.
        Self {
            scalar_extent: ENTRY_PARAM_BYTES,
            array_factor: 8,
            max_depth: 3,
            distinct_by_default: true,
        }
    }
}

/// How the engine picks the next state to run (023 §4).
///
/// **Every strategy is deterministic**, including `RandomPath`. §4: "A non-reproducible
/// bug report is not a bug report" — a symbolic run finds bugs on some paths and not
/// others, so if the order is not reproducible then neither is the finding nor its
/// absence.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    /// Depth-first. The default: cheap memory, good locality for the solver caches.
    #[default]
    Dfs,
    /// A seeded random descent, which avoids the DFS starvation a loop causes.
    ///
    /// The seed is recorded in every `RunResult`, so a run that found something can be
    /// re-run exactly.
    RandomPath { seed: u64 },
}

/// Take the next state to run, per 023 §4's strategy.
///
/// `Dfs` pops the end, which is what makes the true branch of a fork complete first — the
/// sibling was pushed and this state carried on. `RandomPath` picks an index from the
/// seeded stream and swap-removes it, which is O(1) and, more importantly, a function of
/// the seed and the queue length alone.
///
/// **Ties break by position, not by `StateId` comparison**, because the queue is already
/// in deterministic creation order: 001 §5 bans the hash iteration that would make it
/// otherwise, so an index into it is as stable as an id.
fn pick(work: &mut Vec<State>, strategy: Strategy, rng: &mut u64) -> Option<State> {
    if work.is_empty() {
        return None;
    }
    match strategy {
        Strategy::Dfs => work.pop(),
        Strategy::RandomPath { .. } => {
            let i = (split_mix64(rng) % work.len() as u64) as usize;
            Some(work.swap_remove(i))
        }
    }
}

/// SplitMix64 — a small, fully specified PRNG.
///
/// **Written out rather than pulled in.** The requirement is not statistical quality but
/// that the sequence is *identical everywhere and forever*: a bug report replayed next
/// year on another machine must walk the same paths. A dependency could change its
/// algorithm in a patch release and silently invalidate every recorded seed.
fn split_mix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Debug)]
pub struct Engine<'m> {
    module: &'m Module,
    /// 023 §9: what turns a `BytePos` a fault carries into a line a reader can open.
    source_map: Option<&'m chiero_span::SourceMap>,
    /// Whether an overflow the path merely admits is reported. See `with_admitted_overflow`.
    admitted_overflow: bool,
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
    budget: Budget,
    backend: Option<SmtLib>,
    models: ModelRegistry,
    alloc_policy: AllocPolicy,
    string_policy: StringPolicy,
    entry_param_bytes: u64,
    /// 021 §6's `--fork-on-alias`. Off by default, because it multiplies states.
    fork_on_alias: bool,
    /// Whether an entry function's pointer parameter may be **null**.
    ///
    /// Default **on**, by decision: a pointer arriving from outside the analysis is assumed
    /// nullable unless the program proves otherwise, and a guard — `if (!p)`, `p == 0`, an
    /// `assert(p)` lowering to a conditional abort — is what proves it. 021 §6's
    /// `entry_param_bytes` bounds the object's *extent* and is a bound chiero chose; this is
    /// the opposite default about a different question, because "may be null" is exactly
    /// what a caller outside the analysis can do and what its callee is expected to handle.
    entry_ptr_nullable: bool,
    /// 023 §4. `Dfs` unless a caller says otherwise.
    strategy: Strategy,
    /// 021 §6.
    lazy: LazyPolicy,
    /// The function to analyse, if a caller named one.
    entry: Option<String>,
    /// 023 §6. Registered in order; each gets one `CheckerState` slot per state.
    checkers: Checkers,
    /// 021 §5.2, as `(entry parameter index, shape)` until the entry state binds a term.
    arenas: Vec<(usize, ArenaShape)>,
    /// The same, with the parameter's term filled in.
    arena_bases: Vec<(Term, ArenaShape)>,
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
            strategy: Strategy::default(),
            lazy: LazyPolicy::default(),
            entry: None,
            checkers: Checkers::default(),
            arenas: Vec::new(),
            arena_bases: Vec::new(),
            module,
            source_map: None,
            admitted_overflow: false,
            tier: SolverTier::default(),
            next_state: 0,
            solver_calls: 0,
            fresh_count: 0,
            finding_seq: 0,
            pending_dst: None,
            func_objs: IndexMap::new(),
            budget: Budget::default(),
            models: ModelRegistry::with_builtins(),
            alloc_policy: AllocPolicy::default(),
            string_policy: StringPolicy::default(),
            entry_param_bytes: ENTRY_PARAM_BYTES,
            fork_on_alias: false,
            entry_ptr_nullable: true,
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

    /// How big an object an entry function's pointer parameter points at.
    ///
    /// The default is [`ENTRY_PARAM_BYTES`], a bound chiero chose because the caller is
    /// outside the analysis. A test that is *about* the size — a string scan against the
    /// object's end — has to be able to say it, and 024 §4's rule ties the scan to
    /// `min(max_string_scan, object size)`, so both halves must be settable.
    pub fn with_entry_param_bytes(mut self, n: u64) -> Self {
        self.entry_param_bytes = n;
        self
    }

    /// 021 §6's `--fork-on-alias`: explore the case two lazily-materialized pointer
    /// parameters name the *same* object.
    ///
    /// Off by default — §6 says why, and it is not squeamishness: the alias case for every
    /// pair multiplies the state count, and the default's distinctness assumption is what
    /// keeps an under-constrained run tractable. What the default owes in exchange is
    /// saying that it made the assumption, which it does.
    pub fn with_fork_on_alias(mut self, on: bool) -> Self {
        self.fork_on_alias = on;
        self
    }

    /// Report a signed overflow the path merely **admits**, as well as one it forces.
    ///
    /// **Off by default, and the default is the interesting half of the decision.** With
    /// unconstrained inputs *every* `x + y` admits an overflow, so reporting on satisfiability
    /// puts a finding on every arithmetic instruction in every program that takes an argument.
    /// Wave 215 shipped only the forced case for that reason and §9 carried the rest as a
    /// decision.
    ///
    /// This is the resolution, and it follows this repo's own precedent rather than inventing one:
    /// `chiero_check::UnionPun` is off by default because it is "for the projects that want the
    /// stricter reading rather than for this one". A caller auditing a library whose inputs are
    /// genuinely unconstrained wants every `x + y` flagged; a caller looking for definite defects
    /// does not. Both are legitimate, so it is a knob and not a verdict.
    ///
    /// The two answers are different *kinds*, not one kind with a different detail:
    /// `signed-overflow` for a path that forces it and `may-signed-overflow` for one that allows
    /// it, mirroring `out-of-bounds` against `may-be-out-of-bounds` in the memory channel. 023
    /// §6.1 makes the kind half the dedup key, so a reader can group or filter on the distinction.
    pub fn with_admitted_overflow(mut self, on: bool) -> Self {
        self.admitted_overflow = on;
        self
    }

    /// Give the engine the front end's `SourceMap`, so a report can name a line.
    ///
    /// Optional, and every existing caller omits it. A run without one is not degraded — the
    /// analysis does not depend on source locations — it simply cannot render the second
    /// location a memory fault names, and says less rather than guessing.
    pub fn with_source_map(mut self, map: &'m chiero_span::SourceMap) -> Self {
        self.source_map = Some(map);
        self
    }

    /// Turn off the assumption that an entry pointer parameter may be null.
    ///
    /// For a caller that is known to check — an internal helper reached only through a
    /// guarded path — the null state is a path the program does not have, and every
    /// dereference in it is a finding nobody can act on.
    pub fn with_entry_ptr_nullable(mut self, on: bool) -> Self {
        self.entry_ptr_nullable = on;
        self
    }

    /// Register a striding region over an entry parameter (021 §5.2).
    ///
    /// Without this, `base + (i << 6)` where `base` is an unconstrained symbol is §5.1
    /// step 4 — an unresolvable pointer, and the end of the path. That is
    /// `vlib_buffer_ptr_from_index`, so it is the end of essentially every VPP node
    /// analysis at its first buffer access.
    pub fn with_arena(mut self, param: usize, shape: ArenaShape) -> Self {
        self.arenas.push((param, shape));
        self
    }

    /// Register a checker (023 §6). Order is the registration order, and each checker
    /// gets one `CheckerState` slot per state.
    /// Analyse `name` rather than the default entry.
    ///
    /// The default prefers a defined `main` and falls back to the first defined function,
    /// which is right for a whole program and arbitrary for a library — 021 §6's
    /// under-constrained symbolic execution wants to start at each exported function in
    /// turn, and that caller needs to say which.
    pub fn with_entry(mut self, name: &str) -> Self {
        self.entry = Some(name.to_owned());
        self
    }

    /// 021 §6.
    pub fn with_lazy_policy(mut self, p: LazyPolicy) -> Self {
        self.lazy = p;
        self
    }

    /// 023 §4. `Dfs` unless set.
    pub fn with_strategy(mut self, s: Strategy) -> Self {
        self.strategy = s;
        self
    }

    /// The seed this run's strategy uses, reported whether or not it spends it.
    ///
    fn seed(&self) -> u64 {
        match self.strategy {
            Strategy::Dfs => 0,
            Strategy::RandomPath { seed } => seed,
        }
    }

    pub fn with_checker(mut self, c: Box<dyn Checker>) -> Self {
        self.checkers.0.push(c);
        self
    }

    /// Give `s` one `CheckerState` per registered checker. Called once per *root* state;
    /// forks inherit through `CheckerStates`'s `Clone`.
    fn seed_checker_states(&self, s: &mut State) {
        s.checker_states =
            CheckerStates(self.checkers.0.iter().map(|c| c.initial_state()).collect());
    }

    /// Dispatch one event to every registered checker and apply what they ask for.
    ///
    /// **The checker's own memory is moved out of the state for the call.** `Event`
    /// borrows the `State` immutably while `CheckerCtx::state_mut` needs it mutably;
    /// taking the slots out and putting them back is what makes both available without
    /// handing a checker the ability to mutate the state it is observing — §6's "sees
    /// everything and decides nothing about execution order", enforced by the borrow
    /// checker rather than by convention.
    fn emit(&mut self, a: &mut TermArena, s: &mut State, span: Span, ev: Ev<'_>) {
        if self.checkers.0.is_empty() {
            return;
        }
        // `may`/`must` go through the engine's one long-lived solver (022 §4), so it has
        // to exist before the borrow below hands it out.
        if self.solver.is_none() {
            self.solver_inits += 1;
            self.solver = Some(match self.backend_for_run() {
                Some(b) => TieredSolver::with_backend(b),
                None => TieredSolver::new(),
            });
        }
        let solver = self.solver.as_mut().expect("just created");
        let mut states = std::mem::take(&mut s.checker_states);
        let mut actions: Vec<(usize, Action)> = Vec::new();
        for (idx, c) in self.checkers.0.iter_mut().enumerate() {
            let event = match &ev {
                Ev::BeforeInst(i) => Event::BeforeInst { st: s, inst: i },
                Ev::AfterInst(i) => Event::AfterInst { st: s, inst: i },
                Ev::Fork { cond, feasible } => Event::Fork {
                    st: s,
                    cond: *cond,
                    feasible: *feasible,
                },
                Ev::Call { callee, args } => Event::Call {
                    st: s,
                    callee: callee.clone(),
                    args,
                },
                Ev::CallReturn { callee, ret, dst } => Event::CallReturn {
                    st: s,
                    callee: callee.clone(),
                    ret: *ret,
                    dst: *dst,
                },
                Ev::Return(v) => Event::Return { st: s, val: *v },
                Ev::Terminated(why) => Event::Terminated { st: s, why: *why },
            };
            let mut cx = CheckerCtx {
                a,
                idx,
                states: &mut states,
                module: self.module,
                solver: &mut *solver,
                path: &s.path,
                solver_calls: &mut self.solver_calls,
            };
            for act in c.on_event(&event, &mut cx) {
                actions.push((idx, act));
            }
        }
        s.checker_states = states;
        for (idx, act) in actions {
            let who = self.checkers.0[idx].name();
            match act {
                Action::Report(message) => {
                    self.finding_seq += 1;
                    s.findings.push(StateFinding {
                        id: self.finding_seq,
                        key: None,
                        // **A checker's report is located like any other.** This route reaches a
                        // `StateFinding` without passing `report_faults` or the model loop, so it
                        // inherited none of waves 207-211's work and every arithmetic-UB finding
                        // was unlocatable. `span` is the instruction's, which is the only span
                        // that is right: a `Function`'s and a `Block`'s both point at their own
                        // start.
                        message: self.stamp(span, message),
                        span,
                        requires: Vec::new(),
                        witness: None,
                        related: None,
                    });
                }
                Action::ReportRequiring { message, requires } => {
                    self.finding_seq += 1;
                    // **Also onto the state's list**, so the state's own witness still
                    // tries to satisfy everything. A reader who looks at the state rather
                    // than at one finding should not see a number that reproduces nothing
                    // when one exists that reproduces everything.
                    s.witness_requires.extend(requires.iter().copied());
                    s.findings.push(StateFinding {
                        id: self.finding_seq,
                        key: None,
                        // Both checker routes, for the reason wave 207 recorded: when a fix is
                        // about how a finding is built, ask what else builds one.
                        message: self.stamp(span, message),
                        span,
                        requires,
                        witness: None,
                        related: None,
                    });
                }
                // **Onto the path condition, not a side list** (contract 19). A
                // constraint the solver never sees changes nothing, and every checker
                // written against it would appear to work.
                //
                // **And it degrades.** A checker's `Assume` discards paths the program
                // has, on the analysis's own say-so — 023 §7's `Approximated`, "keeping
                // one of several feasible values". Pushing the term alone left the run
                // `Exact` with an empty `assumptions` list, so `seal` would mint a proof
                // over a program half of whose paths an unaudited checker had deleted,
                // and nothing in the rendered report would say so. Every other place the
                // engine narrows on its own authority — `--fork-on-alias`, the distinct
                // pointer-parameter default, 021 §5's OOB continuation — degrades and
                // records; this is the same act. Found by review.
                //
                // 022 §6.1: this is a constraint added *without* a feasibility check, so
                // it is a fourth `push_unchecked` site once `s.path` becomes a
                // `PathCondition`.
                Action::Assume(t) => {
                    s.constrain_unchecked(t);
                    s.degrade(
                        Fidelity::Approximated,
                        AssumptionKind::OpaqueCode,
                        span,
                        &format!(
                            "the `{who}` checker assumed a condition, so paths the \
                             program allows were not explored"
                        ),
                    );
                }
                // Same reasoning: a killed path is one nobody looked at.
                Action::Kill(why) => {
                    s.status = Status::Terminated(why);
                    s.degrade(
                        Fidelity::Approximated,
                        AssumptionKind::OpaqueCode,
                        span,
                        &format!("the `{who}` checker stopped this path"),
                    );
                }
            }
        }
    }

    /// 024 §4's `max_string_scan` and friends.
    pub fn with_string_policy(mut self, p: StringPolicy) -> Self {
        self.string_policy = p;
        self
    }

    /// What to record as the solver behind this run (022 §4).
    ///
    /// Resolved from the same `backend_for_run` the queries use, so the name cannot
    /// disagree with what actually answered. Reported even on the error paths that make no
    /// query at all — the question a reader is asking is "what was this run configured to
    /// decide with", and "no queries were made" is `solver_calls`'s job.
    fn solver_name(&self) -> String {
        self.backend_for_run()
            .map(|b| b.name().to_string())
            .unwrap_or_else(|| "solver-lite".to_string())
    }

    /// Which backend this run should use, if any — 022 §4's discovery, in one place.
    ///
    /// Three solvers get built during a run (the query path, the checker path, and the
    /// offset enumeration), and each used to decide independently by reading
    /// `self.backend`. A default that has to be applied identically in three places is a
    /// default that will be applied in two.
    fn backend_for_run(&self) -> Option<SmtLib> {
        match (self.backend.clone(), self.tier) {
            // A caller naming a solver has said something more specific than "find one".
            (Some(b), _) => Some(b),
            // `LiteOnly` refuses to look, which is the whole point of it.
            (None, SolverTier::LiteOnly) => None,
            (None, SolverTier::Discover) => SmtLib::discover(),
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
        // **The entry is a *defined* function, and `main` when there is one.**
        //
        // `funcs.first()` was whatever the translation unit declared first — for every
        // corpus file that is `chiero_make_symbolic` from `chiero.h`, a `Body::Declared`
        // function with no blocks. The engine set `pc` to its nonexistent entry block and
        // every run ended `Errored("no such block BlockId(0)")` before executing a single
        // instruction. The goldens stayed green because they compare lowered *text*, and
        // "no findings" stayed true because an errored state reports none.
        let entry = self
            .entry
            .as_deref()
            .and_then(|n| {
                self.module
                    .funcs
                    .iter()
                    .find(|f| &*f.name == n && f.body == Body::Defined)
            })
            .or_else(|| {
                self.module
                    .funcs
                    .iter()
                    .find(|f| &*f.name == "main" && f.body == Body::Defined)
            })
            .or_else(|| self.module.funcs.iter().find(|f| f.body == Body::Defined));
        let Some(f) = entry else {
            let s = State::errored(self.new_id(), "the module defines no functions");
            return RunResult {
                sliced_terms_skipped: 0,
                id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                states: vec![s],
                solver_calls: 0,
                backend_spawns: 0,
                solver_inits: 0,
                completion_order: vec![0],
                seed: self.seed(),
                budget: self.budget,
                _seal: Sealed,
                solver: self.solver_name(),
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
                checker_states: CheckerStates::default(),
                path_unchecked: false,
                arena_objs: IndexMap::new(),
                global_objs: IndexMap::new(),
                lazy_depth: IndexMap::new(),
                lazy_cut: None,
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
                witness_requires: Vec::new(),
                ub: Vec::new(),
                effects: Vec::new(),
                replay_used: 0,
                witness: None,
                unwitnessed: None,
                ptr_ints: IndexMap::new(),
                entry_null_param: None,
            };
            bad.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                Span::DUMMY,
                "the module was never executed, so nothing is known about it",
            );
            return RunResult {
                sliced_terms_skipped: 0,
                id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                states: vec![bad],
                solver_calls: 0,
                backend_spawns: 0,
                solver_inits: 0,
                // One state, so one entry: the sibling error path does this and the
                // normal path returns a permutation of `states()`. An empty vector here
                // broke that invariant on the new path alone.
                completion_order: vec![0],
                seed: self.seed(),
                budget: self.budget,
                _seal: Sealed,
                solver: self.solver_name(),
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
        // How many of the witness's bindings the entry parameters consumed.
        let mut entry_replay_used = 0usize;
        // Which parameters got a lazily-materialized object, for 021 §6's aliasing policy.
        let mut ptr_params: Vec<(ValueId, chiero_mem::ObjectId)> = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let v = if p.ty == CTy::Ptr {
                // Sized by the budget rather than by a guess about the callee: the object
                // exists so accesses have somewhere to land, and its extent is a bound
                // chiero chose, so an access past it is reported as such.
                let obj = mem.alloc(ObjKind::Lazy, self.entry_param_bytes, 16, f.span);
                // **021 §6: "fully symbolic and fully initialized".** The caller filled
                // this buffer; chiero does not know *what with*, which is not the same as
                // nobody having written it. Leaving the bytes uninitialized turned every
                // function that takes a pointer into an uninitialized-read report — §6
                // calls that "an uninitialized-read false-positive storm", and §3.1's
                // whole reason for distinguishing symbolic from uninitialized is to make
                // this expressible. Contract 27.
                // **Byte-wise, not `havoc_object`** (021 §6). The contents must be
                // "fully symbolic and fully initialized", and the two obvious shortcuts
                // are each wrong in one direction, both found by review:
                //
                // - `havoc_object` promotes the object to an array representation, and
                //   every byte-level *write* path refuses a promoted object — so every
                //   store through a pointer parameter was silently dropped and the read
                //   after it explored a path the program does not have.
                // - Marking the bytes initialized without giving them values leaves them
                //   reading as the backing store's **zero**, which 021 §3.1 calls the
                //   single most common way a symbolic executor is confidently wrong.
                //
                // Marking it lazy is neither: the object stays `Repr::Bytes` and
                // writable, and a byte becomes a symbol nobody has claimed **when it is
                // first read**, which is what §6 says ("on first dereference"). Filling
                // eagerly instead cost one term per byte per *state* — a fork clones the
                // memory — measuring 1.3 GB at 8192 states and aborting the process with
                // four pointer parameters, against a default `max_states` of 10 000. An
                // earlier comment here called laziness "the optimisation, not the
                // correctness", which inverted §6's own sentence. Found by review.
                mem.mark_lazy(obj);
                Value::Ptr(Pointer { base: obj, off: 0 })
            } else {
                let origin = InputOrigin::Param {
                    index: i,
                    name: String::new(),
                    span: f.span,
                };
                // **A replay binds the entry parameters too.** They were minted with
                // `a.var` directly, which is the one path that never consults the witness
                // — so `replaying()` concretized inputs invented *during* a run and left
                // the entry parameters symbolic, and every branch on one still forked.
                // "All inputs concretized" (023 contract 21) is exactly these.
                //
                // The state does not exist yet here, so the replay cursor is local and is
                // handed to it below — bindings are consumed positionally and the entry
                // parameters are the first sites, so counting them here is the same order
                // `replayed` would have used.
                let bound = self.replay.as_ref().and_then(|w| {
                    w.bindings
                        .get(entry_replay_used)
                        .map(|b| (b.width, b.value))
                });
                match bound {
                    Some((_, value)) => {
                        entry_replay_used += 1;
                        let w = match sort_of(&p.ty) {
                            chiero_solver::Sort::BitVec(w) => w,
                            _ => 64,
                        };
                        Value::Scalar(a.bv(w, value))
                    }
                    None => {
                        let t = a.var(sort_of(&p.ty), &format!("param{i}"));
                        entry_inputs.push((t, origin));
                        Value::Scalar(t)
                    }
                }
            };
            if let Value::Ptr(q) = v {
                ptr_params.push((p.value, q.base));
            }
            // 021 §5.2: an arena is registered against a parameter, and this is where
            // that parameter finally has a term to be registered against.
            if let Value::Scalar(t) = v {
                for (idx, shape) in &self.arenas {
                    if *idx == i {
                        self.arena_bases.push((t, *shape));
                    }
                }
            }
            entry_locals.insert(p.value, v);
        }
        let mut start = State {
            checker_states: CheckerStates::default(),
            path_unchecked: false,
            arena_objs: IndexMap::new(),
            global_objs: IndexMap::new(),
            lazy_depth: IndexMap::new(),
            lazy_cut: None,
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
                bit_inspected: Vec::new(),
                prev_block: None,
                varargs: Vec::new(),
                lane_w: IndexMap::new(),
            }],
            ret: None,
            edge_counts: IndexMap::new(),
            steps: 0,
            findings: Vec::new(),
            inputs: entry_inputs,
            witness_requires: Vec::new(),
            ub: Vec::new(),
            effects: Vec::new(),
            replay_used: entry_replay_used,
            witness: None,
            unwitnessed: None,
            ptr_ints: IndexMap::new(),
            entry_null_param: None,
        };
        // Depth-first with the true branch first, so fork order is deterministic (§3) and
        // 001 §5's determinism requirement is met by construction rather than by luck.
        // **021 §6's aliasing policy, and the assumption it rests on.** Two lazily
        // materialized objects are distinct by default; that is an assumption about the
        // *caller*, not a fact about the program, and §6 requires it recorded and printed.
        // A run that quietly assumed `memcpy(dst, src, n)` never overlaps would miss the
        // bug that idiom exists to have.
        // **One `CheckerState` per checker, on the root only.** Every other state is a
        // descendant, and `CheckerStates::clone` calls `on_fork` — seeding a fork here
        // instead would silently reset a checker's memory at every branch.
        self.seed_checker_states(&mut start);
        let mut work = vec![start];
        if ptr_params.len() > 1 {
            if self.fork_on_alias {
                // One extra state per pair, each with the later parameter naming the
                // earlier one's object. §6 describes `2^(pairs)`; pairwise is the subset
                // that answers "do *these two* alias", which is the question a checker
                // asks, and it grows quadratically rather than exponentially.
                let pairs: Vec<(usize, usize)> = (0..ptr_params.len())
                    .flat_map(|i| (i + 1..ptr_params.len()).map(move |j| (i, j)))
                    .collect();
                for (i, j) in pairs {
                    let (vi, base) = ptr_params[i];
                    let (vj, _) = ptr_params[j];
                    let mut alias = work[0].clone();
                    alias.id = self.new_id();
                    alias.set_local(vj, Value::Ptr(Pointer { base, off: 0 }));
                    alias.degrade(
                        Fidelity::Approximated,
                        AssumptionKind::OpaqueCode,
                        f.span,
                        &format!(
                            "--fork-on-alias: parameters {} and {} were explored as the \
                             same object",
                            vi.0, vj.0
                        ),
                    );
                    work.push(alias);
                }
            } else {
                let names: Vec<String> = ptr_params
                    .iter()
                    .map(|(v, _)| format!("%{}", v.0))
                    .collect();
                work[0].degrade(
                    Fidelity::Approximated,
                    AssumptionKind::OpaqueCode,
                    f.span,
                    &format!(
                        "the pointer parameters {} are assumed to be distinct objects; \
                         `--fork-on-alias` explores the case they are not",
                        names.join(", ")
                    ),
                );
            }
        }
        // **One extra state per pointer parameter, each with that one null.**
        //
        // Not `2^n`, for the reason the aliasing fork above gives for preferring pairwise:
        // the question a checker asks is "is *this* pointer dereferenced without a check",
        // and one null per parameter answers it. The combinations where two are null at once
        // add states without adding answers, since a dereference of either is already
        // covered.
        //
        // **No fidelity degradation**, matching `malloc`'s failure fork. A null caller is
        // not a limit of the model — it is a case the program has, and 023 §7's fidelity is
        // for what chiero *cannot* represent. Marking these `Approximated` would say the
        // opposite of what is true: this path is modelled exactly.
        // **Only an *exported* entry gets the assumption.** For a `static` function every
        // call site is in this module, so what its callers pass is something the analysis
        // can reach rather than something it must guess: running the module's exported
        // entries walks into this one carrying the real arguments. Assuming null here as
        // well double-counts — and worse, the outer assumption is grounded in a call site
        // while this one is not. 021 §6 says "start at each *exported* function in turn",
        // and this is that sentence enforced.
        //
        // Measured: 3 of 3 null-dereference findings over `tests/corpus/c` were `static`
        // helpers whose callers all pass `&table[i]`. Every one true, none of them chiero's
        // to raise from that entry.
        let exported = f.linkage == chiero_cir::Linkage::External || self.address_escapes(f.id);
        if self.entry_ptr_nullable && exported && !ptr_params.is_empty() {
            let originals = work.clone();
            for (vid, _) in &ptr_params {
                for st in &originals {
                    let mut nul = st.clone();
                    nul.id = self.new_id();
                    nul.set_local(
                        *vid,
                        Value::Ptr(Pointer {
                            base: chiero_mem::ObjectId::NULL,
                            off: 0,
                        }),
                    );
                    nul.entry_null_param = Some((*vid, format!("%{}", vid.0)));
                    work.push(nul);
                }
            }
        }
        let mut done = Vec::new();
        // **The PRNG advances once per selection, not per candidate**, so the sequence a
        // seed produces does not depend on how many states happened to be live — which
        // would make the order depend on the budget and the module rather than the seed.
        let mut rng = self.seed();
        while let Some(mut s) = pick(&mut work, self.strategy, &mut rng) {
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
            // **Before the witness is attached**, so a report a checker makes on the way
            // out is witnessed like any other. Firing it afterwards would give the last
            // finding of every path an empty witness.
            // **`Errored` too.** Gating on `Terminated` alone meant a state that gave up
            // — an indirect goto with no targets, a missing block — never fired the
            // event, and those are precisely the paths where a checker's accumulated
            // state matters most, since `give_up` also sets `Fidelity::Unknown`. Found by
            // review.
            match s.status {
                Status::Terminated(why) => self.emit(a, &mut s, Span::DUMMY, Ev::Terminated(why)),
                Status::Errored(_) => self.emit(
                    a,
                    &mut s,
                    Span::DUMMY,
                    Ev::Terminated(TermReason::Unsupported),
                ),
                Status::Running => {}
            }
            self.attach_witness(a, &mut s);
            done.push(s);
        }
        // The order states *finished* is recorded before sorting, so a change of searcher
        // shows up in the output instead of being erased by the sort.
        let completion_order: Vec<u32> = done.iter().map(|s| s.id.0).collect();
        done.sort_by_key(|s| s.id.0);
        let backend_spawns = self.solver.as_ref().map_or(0, |s| s.stats().backend_spawns);
        let sliced_terms_skipped = self
            .solver
            .as_ref()
            .map_or(0, |s| s.stats().sliced_terms_skipped);
        RunResult {
            id: NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            states: done,
            solver_calls: self.solver_calls,
            backend_spawns,
            sliced_terms_skipped,
            solver_inits: self.solver_inits,
            completion_order,
            seed: self.seed(),
            budget: self.budget,
            _seal: Sealed,
            solver: self.solver_name(),
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
                requires: Vec::new(),
                witness: None,
                related: None,
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
                requires: Vec::new(),
                witness: None,
                related: None,
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
                requires: Vec::new(),
                witness: None,
                related: None,
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
        // **Solve the path *and* what the findings need**, falling back to the path alone.
        //
        // A witness is a claim someone can re-run (023 §9), so it has to satisfy the
        // conditions the findings on this path depend on — otherwise the number beside a
        // division by zero is an input under which nothing divides by zero.
        //
        // The fallback is not decoration. Two findings on one path can need contradictory
        // inputs (`100/(x-1)` and `100/(x-2)`), and their conjunction is then unsatisfiable
        // — at which point the honest answer is the witness the path alone supports, which
        // is exactly what was reported before. Recorded in §9: the complete answer is a
        // witness *per finding*, and this is one per state.
        let requires = s.witness_requires.clone();
        let attempt = match self.probe(a, s, &requires) {
            sat @ CheckResult::Sat(_) => sat,
            weaker if requires.is_empty() => weaker,
            _ => self.probe(a, s, &[]),
        };
        let model = match attempt {
            CheckResult::Sat(m) => {
                // The model satisfies the path condition — with or without the findings'
                // extra requirements, both of which imply it — so the path is reachable.
                s.discharge_unproven_path();
                m
            }
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
        // **Which inputs the path actually mentions.**
        //
        // `pinned` used to be "the model assigned it", which worked only while every model
        // came from the external backend — those assign the variables they were asked
        // about and no others. `SolverLite` builds a **complete** assignment by
        // construction (022 §2: unconstrained variables get 0), so the moment wave 153
        // made it able to answer these queries in-process, every input started reporting
        // as pinned, including ones no term on the path so much as names. That is exactly
        // the claim `witness.rs`'s module doc says a witness must never make.
        //
        // Occurrence in the path condition is the property that was meant. It is a *sound*
        // reading of "free": a variable the path never mentions cannot be constrained by
        // it, so any value does. It is deliberately not the converse — a mentioned
        // variable is reported as pinned even if the path leaves it several values, which
        // overstates nothing a reader acts on, since the value shown does reach the fault.
        let mut mentioned: indexmap::IndexSet<chiero_solver::VarId> = Default::default();
        for t in &s.path {
            vars_of(a, *t, &mut mentioned);
        }
        // **What a finding depends on constrains its witness too.** `100 / (x - 42)` has
        // an empty path condition and still needs `x == 42`; reporting that binding as
        // *free* would say the fault happens for any input, which is the same wrong claim
        // as naming the wrong value — one about the number and one about what it means.
        for t in &s.witness_requires {
            vars_of(a, *t, &mut mentioned);
        }
        s.witness = Some(Witness {
            bindings: bindings_under(a, s, &model, &extra, &mentioned),
        });

        // **And one witness per finding that needs its own.**
        //
        // The state's witness above satisfies as much as it can at once. Two findings can
        // need contradictory inputs — `100/(x-1)` and `100/(x-2)` — and then it satisfies
        // neither, because there is no single assignment that does. 023 §9 is a claim about
        // each *report* a reader is shown, so each is solved on its own.
        //
        // Only findings that recorded a condition get one; everything else is reproduced by
        // the path, which the state's witness already satisfies. A finding whose own solve
        // fails keeps `None` and falls back to the state's, which is the wave-158 answer
        // and still better than no number.
        let needs: Vec<(usize, Vec<Term>)> = s
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.requires.is_empty())
            .map(|(i, f)| (i, f.requires.clone()))
            .collect();
        for (i, requires) in needs {
            let CheckResult::Sat(m) = self.probe(a, s, &requires) else {
                continue;
            };
            let mut mine: indexmap::IndexSet<chiero_solver::VarId> = Default::default();
            for t in &s.path {
                vars_of(a, *t, &mut mine);
            }
            for t in &requires {
                vars_of(a, *t, &mut mine);
            }
            let bindings = bindings_under(a, s, &m, &extra, &mine);
            s.findings[i].witness = Some(Witness { bindings });
        }
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

    /// Ask whether a **symbolic divisor can be zero on this path**, and record what the
    /// answer was — including that there wasn't one.
    ///
    /// Only division and remainder reach here. A constant divisor is settled by the caller
    /// without a query, so the query count is one per *symbolic* division rather than one
    /// per arithmetic instruction.
    /// **An overflow every value the path admits produces** (C11 6.5p5).
    ///
    /// Wave 156's three outcomes, asked the other way round. The divisor query reports on `Sat` --
    /// *some* input divides by zero -- and that does not transfer here: with an unconstrained `x`,
    /// `x + 1` is satisfiably overflowing and fine for four billion values, so `Sat` would put a
    /// finding on every arithmetic instruction in every program that takes an input. §9 records
    /// that as an open decision.
    ///
    /// So this asks whether the path *forces* it: `P ∧ ¬overflow` unsatisfiable means every model
    /// of the path overflows, which is a definite fault under any reading and adds no report to a
    /// program that merely could. `Sat` and `Unknown` both stay silent, which is what chiero did
    /// before this existed.
    ///
    /// The condition is computed one bit wider and compared against the narrow range, which is
    /// the definition rather than a trick: `sext` both operands to `w + 1`, do the arithmetic
    /// there, and ask whether the result left `[-2^(w-1), 2^(w-1) - 1]`. `Mul` needs `2w` because
    /// two `w`-bit factors need `2w` bits to hold their product.
    fn forced_signed_overflow(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        op: BinOp,
        w: u32,
        // The operands as a pair rather than two arguments: clippy caps a function at seven and
        // these two are one thing.
        (x, y): (Term, Term),
        span: Span,
    ) {
        // Only the three C makes undefined on overflow. Division's overflow case
        // (`INT_MIN / -1`) is a different shape and belongs with the divisor query.
        let wide = match op {
            BinOp::Add | BinOp::Sub => w + 1,
            BinOp::Mul => w * 2,
            _ => return,
        };
        // A zero-width type would make the range terms meaningless, and `1 << (w - 1)`
        // underflow. `bits_of_cty` can only return this for `void`, which cannot be an operand.
        if w == 0 {
            return;
        }
        let xs = a.sext(x, wide);
        let ys = a.sext(y, wide);
        let exact = match op {
            BinOp::Add => a.add(xs, ys),
            BinOp::Sub => a.sub(xs, ys),
            _ => a.mul(xs, ys),
        };
        let max = a.bv(wide, (1u128 << (w - 1)) - 1);
        let min_mag = a.bv(wide, 1u128 << (w - 1));
        let zero = a.bv(wide, 0);
        let neg_min = a.sub(zero, min_mag);
        let too_high = a.slt(max, exact);
        let too_low = a.slt(exact, neg_min);
        let overflows = a.or(too_high, too_low);
        let safe = a.not(overflows);
        // **`Unsat` of the negation, not `Sat` of the condition.** The distinction is the whole
        // decision recorded above, and it is one character in the match arm — worth the sentence.
        if matches!(self.probe(a, s, &[safe]), CheckResult::Unsat) {
            s.ub.push(UbEvent {
                kind: UbKind::SignedOverflow,
                span,
                detail: format!("{op:?} overflows for every value this path allows"),
                // **The condition still travels**, even though it is implied by the path. 023 §9
                // wants a witness that reproduces the fault, and a solver handed the overflow
                // condition explicitly names operands that produce it rather than any model of
                // the path — which for a forced overflow is the same set, and for a reader is
                // the difference between an input and *the* input.
                requires: vec![overflows],
            });
            return;
        }
        // **The weaker claim, only when the caller asked for it** (`with_admitted_overflow`).
        //
        // Reached only when the path does *not* force the overflow, because the branch above
        // returns — so one operation yields at most one event and the two kinds can never both
        // describe the same site. That is what makes 023 §6.1's dedup key meaningful here: a
        // reader grouping by kind is grouping by certainty.
        //
        // `Sat` and not "anything other than `Unsat`": an `Unknown` is a question nobody settled,
        // and the weaker *kind* is still a claim that some input overflows. Wave 216's rule — the
        // third outcome is not the second one.
        if self.admitted_overflow && matches!(self.probe(a, s, &[overflows]), CheckResult::Sat(..))
        {
            s.ub.push(UbEvent {
                kind: UbKind::MaybeSignedOverflow,
                span,
                detail: format!("{op:?} overflows for some value this path allows"),
                requires: vec![overflows],
            });
        }
    }

    fn symbolic_div_by_zero(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        op: BinOp,
        w: u32,
        y: Term,
        span: Span,
    ) {
        if !matches!(op, BinOp::UDiv | BinOp::SDiv | BinOp::URem | BinOp::SRem) {
            return;
        }
        // **No early return for a constant divisor**, and that was a real miss. The
        // caller's concrete path requires *both* operands to be constants, so
        // `x / 0` with a symbolic numerator arrives here with `y` a literal zero — and
        // returning on "y is constant" dropped the most obvious division by zero there is.
        // Found by mutation: removing this check changed nothing any test could see, which
        // is what said the check was not earning its place.
        //
        // Letting the query handle it is uniform and costs nothing: `eq(0, 0)` folds to
        // true and `eq(5, 0)` to false, so a constant divisor is decided by the arena
        // before the solver is asked anything.
        let zero = a.bv(w, 0);
        let is_zero = a.eq(y, zero);
        match self.probe(a, s, &[is_zero]) {
            CheckResult::Sat(_) => {
                s.ub.push(UbEvent {
                    kind: UbKind::DivByZero,
                    span,
                    detail: format!("{op:?} by a divisor the path allows to be zero"),
                    requires: vec![is_zero],
                });
                // **The condition that makes this a fault, kept for the witness.** The
                // model that just proved it is not enough on its own: it answers for the
                // path *as it is now*, and the state runs on. Keeping the term lets the
                // witness be solved once, at termination, against the finished path.
                s.witness_requires.push(is_zero);
            }
            CheckResult::Unsat => {}
            CheckResult::Unknown(_) => {
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::NoInformation,
                    span,
                    &format!(
                        "whether the divisor of this {op:?} can be zero was not decided, \
                         so neither a division by zero nor its absence is claimed"
                    ),
                );
            }
        }
    }

    /// Does anything in the module take this function's **address**?
    ///
    /// Wave 188 drops the null-caller assumption for internal linkage, on the ground that
    /// every call site is in this module. A taken address removes that ground: the pointer
    /// can be stored, returned or handed to a library, and the call then arrives from a
    /// translation unit chiero will never see. A `static` node function registered by
    /// address is the ordinary shape in VPP, so this is not a corner case.
    ///
    /// **All three carriers, because missing one is unsound in the quiet direction** — the
    /// assumption stays off and the finding never appears. `AddrOfFunc` is the instruction,
    /// `Const::FuncAddr` the operand, and `GlobalInit::FuncAddr` a file-scope initializer,
    /// which before this wave did not exist: `int (*t)(int*) = helper;` lowered to `Zero`.
    ///
    /// A *direct call* is not an escape and must not count — `Terminator::Call` names the
    /// callee by `FuncId` without going through any of these, which is what keeps wave
    /// 188's suppression alive for the ordinary helper.
    fn address_escapes(&self, id: chiero_cir::FuncId) -> bool {
        let taken_in_global = self
            .module
            .globals
            .iter()
            .any(|g| matches!(g.init, chiero_cir::GlobalInit::FuncAddr(f) if f == id));
        if taken_in_global {
            return true;
        }
        let is_addr = |o: &Operand| matches!(o, Operand::Const(Const::FuncAddr(t)) if *t == id);
        self.module.funcs.iter().any(|f| {
            f.blocks.iter().any(|b| {
                b.insts.iter().any(|i| match &i.kind {
                    InstKind::Assign { rv, .. } => match rv {
                        RValue::AddrOfFunc(t) => *t == id,
                        RValue::Use(o) => is_addr(o),
                        RValue::Select { cond, t, f } => is_addr(cond) || is_addr(t) || is_addr(f),
                        _ => false,
                    },
                    InstKind::Store { val, .. } => is_addr(val),
                    // An argument, which is how the address reaches a registration
                    // function — `VLIB_REGISTER_NODE`'s shape, and the reason this matters
                    // for VPP at all.
                    InstKind::Call { args, callee, .. } => {
                        args.iter().any(is_addr)
                            || matches!(callee, Callee::Indirect(o) if is_addr(o))
                    }
                    _ => false,
                })
            })
        })
    }

    /// Record 020 §4.1's UB event for a binary operation, if this one has one.
    ///
    /// **Concrete operands are answered by arithmetic; a symbolic divisor is asked.**
    ///
    /// This used to answer only for two constants, on the reasoning that deciding a
    /// symbolic case "costs a solver query per arithmetic instruction — which is 040's
    /// business". That reasoning is right for `Add`/`Sub`/`Mul` and for shifts: overflow
    /// is a question about every arithmetic instruction in the program, and asking it
    /// everywhere is the cost the argument was about.
    ///
    /// It is not right for division. Divisions are rare, the question is a single
    /// feasibility check — *can this divisor be zero on this path?* — and the machinery to
    /// ask it is already here and already used for exactly this shape: the
    /// null-dereference checker asks whether an address can be null, reports with a witness
    /// when it can, and stays quiet when it cannot. A symbolic executor that misses
    /// `100 / x` is missing the case it exists for.
    ///
    /// The three answers are three different outcomes, and collapsing any two is the bug
    /// this replaced:
    ///
    /// - **`Sat`** — zero is reachable here, so the event is recorded. Whether the path
    ///   *forces* zero or merely admits it is not a distinction the event makes; 023's
    ///   witness is what tells a reader which, and it carries the value.
    /// - **`Unsat`** — the divisor cannot be zero, so there is nothing to report and
    ///   nothing to degrade. Without this the check would fire on every division with a
    ///   non-constant divisor and bury the real ones.
    /// - **`Unknown`** — the solver could not tell, which is 020 §5's "a gap is a
    ///   diagnostic, not a licence". Staying silent *and* `Exact` would be a positive
    ///   claim that the path was modelled, made about a question that was not answered.
    ///
    /// Shifts and signed overflow keep the concrete-only treatment, and the reason is the
    /// cost argument above rather than an oversight — recorded in §9 as owed.
    #[allow(clippy::too_many_arguments)]
    fn note_ub(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        op: BinOp,
        ty: &CTy,
        signed: bool,
        x: Term,
        y: Term,
        span: Span,
    ) {
        let w = bits_of_cty(ty).unwrap_or_else(|| a.width(x));
        let (Some(xc), Some(yc)) = (a.as_const(x), a.as_const(y)) else {
            self.symbolic_div_by_zero(a, s, op, w, y, span);
            if signed {
                self.forced_signed_overflow(a, s, op, w, (x, y), span);
            }
            return;
        };
        let mut push = |kind: UbKind, detail: String| {
            s.ub.push(UbEvent {
                kind,
                span,
                detail,
                requires: Vec::new(),
            });
        };
        match op {
            // **The count rule applies to every shift** (C11 6.5.7p3): a shift by at least
            // the operand's width is undefined whichever direction it goes and whatever the
            // operand's signedness.
            BinOp::Shl | BinOp::LShr | BinOp::AShr if yc.bits() >= w as u128 => {
                push(
                    UbKind::Shift,
                    format!("{op:?} of a {w}-bit value by {}", yc.bits()),
                );
            }
            // **C11 6.5.7p4's other two clauses, which apply to signed shifts only.**
            //
            // `E1 << E2` is undefined when `E1` is negative, and when `E1 × 2^E2` is not
            // representable in the result type. Both are conditioned on `signed`, and that
            // condition is the whole difficulty: `1 << 31` is undefined for `int` and
            // perfectly ordinary for `unsigned`, which UBSan confirms in both directions.
            // Checking them from the bits alone — which is all CIR carried until `Bin`
            // grew `signed` — reports every `unsigned x << 31` as undefined, a false
            // positive in the most common idiom in C.
            //
            // The product is computed at full width for the same reason the overflow arm
            // below does: recomputing it in the operand's width is exactly the wrap being
            // tested for.
            BinOp::Shl if signed && xc.signed() < 0 => {
                push(
                    UbKind::Shift,
                    format!("left shift of the negative value {}", xc.signed()),
                );
            }
            BinOp::Shl
                if signed
                    && xc
                        .signed()
                        .checked_shl(yc.bits() as u32)
                        .is_none_or(|v| v > signed_range(w).1) =>
            {
                push(
                    UbKind::Shift,
                    format!(
                        "left shift of {} by {} places cannot be represented in {w} signed bits",
                        xc.signed(),
                        yc.bits()
                    ),
                );
            }
            BinOp::UDiv | BinOp::SDiv | BinOp::URem | BinOp::SRem if yc.bits() == 0 => {
                push(UbKind::DivByZero, format!("{op:?} by zero"));
            }
            // **C11 6.5.5p6: a division whose quotient is not representable.** On two's complement
            // that is exactly one pair per signed width — the most negative value over `-1`, whose
            // quotient is one past the top — and the hardware agrees loudly, raising SIGFPE just as
            // division by zero does.
            //
            // **It fell between the two arms it sits between** (wave 263). `DivByZero` tests
            // `y == 0` and the overflow arm below covers `Add`, `Sub` and `Mul`; a division that
            // overflows is neither, so the event was absent rather than misclassified.
            //
            // `SRem` is the same pair for the same reason: `INT_MIN % -1` is the remainder of a
            // division that cannot be performed. C11 6.5.5p6 covers `/` and `%` in one sentence and
            // so does this arm.
            //
            // **Reported as `SignedOverflow`, not `DivByZero`.** The divisor is fine; what does not
            // fit is the result, which is what the other overflow arm means by the name. UBSan
            // gives it a message of its own — "cannot be represented in type" — and groups it with
            // neither, so the choice is chiero's to make and this is the one that reads true.
            BinOp::SDiv | BinOp::SRem
                if signed && xc.signed() == signed_range(w).0 && yc.signed() == -1 =>
            {
                push(
                    UbKind::SignedOverflow,
                    format!(
                        "{op:?} of {} by -1 has no representable result in {w} signed bits",
                        xc.signed()
                    ),
                );
            }
            // Signed overflow is a statement about the *mathematical* result, so it is
            // computed at full width and compared against the range — recomputing it in
            // the operand's width is exactly the wrap being tested for.
            // **Unsigned arithmetic wraps and is defined** (C11 6.2.5p9), so the range
            // test belongs behind `signed`. Without it `3000000000u + 3000000000u`
            // reinterprets as `-1294967296 + -1294967296`, lands outside the signed range
            // and fires — a false report on a program gcc runs clean.
            BinOp::Add | BinOp::Sub | BinOp::Mul if signed => {
                let (xs, ys) = (xc.signed(), yc.signed());
                let exact: Option<i128> = match op {
                    BinOp::Add => xs.checked_add(ys),
                    BinOp::Sub => xs.checked_sub(ys),
                    _ => xs.checked_mul(ys),
                };
                let (lo, hi) = signed_range(w);
                if exact.is_none_or(|v| v < lo || v > hi) {
                    push(
                        UbKind::SignedOverflow,
                        format!("{op:?} of {xs} and {ys} does not fit in {w} signed bits"),
                    );
                }
            }
            _ => {}
        }
    }

    /// 021 §7.2's `PointerBitInspection`: does this operation ask about pointer bits the
    /// *allocator* chose rather than ones the object guarantees?
    ///
    /// An object aligned to `A` really does have its low `log2(A)` bits clear — a fact
    /// about the object, so `p & 7` on a 16-byte-aligned pointer is decidable and firing
    /// there would double the state count of every aligned-pointer test in VPP. A mask
    /// reaching *at or above* `A` asks about bits chiero's bump allocator invented.
    ///
    /// **Mitigation 2 of §7.2's two.** Mitigation 1 — symbolic base addresses constrained
    /// only by alignment and disjointness — is not implemented, and until it is, this is
    /// what stops the allocator answering the program's question.
    fn note_pointer_bits(&mut self, a: &TermArena, s: &mut State, dst: ValueId, rv: &RValue) {
        // **Propagate first.** Any operation over a tainted local is tainted, which is how
        // the `& mask` reaches the `== 0` and then the branch.
        let mut tainted = false;
        let mut visit = |o: &Operand| {
            if let Operand::Value(v) = o
                && s.is_bit_inspected(*v)
            {
                tainted = true;
            }
        };
        match rv {
            RValue::Bin { a: x, b: y, .. } | RValue::Cmp { a: x, b: y, .. } => {
                visit(x);
                visit(y);
            }
            RValue::Un { a: x, .. } | RValue::Cast { a: x, .. } | RValue::Use(x) => visit(x),
            RValue::Select { cond, t, f } => {
                visit(cond);
                visit(t);
                visit(f);
            }
            _ => {}
        }
        if tainted {
            s.mark_bit_inspected(dst);
            return;
        }
        // The source: `ptr_as_int & mask` where the mask reaches past the alignment.
        let RValue::Bin {
            op: BinOp::And,
            a: x,
            b: y,
            ..
        } = rv
        else {
            return;
        };
        for (val, other) in [(x, y), (y, x)] {
            let Operand::Const(Const::Int { val: mask, .. }) = val else {
                continue;
            };
            let Operand::Value(v) = other else { continue };
            let Some(p) = s.value_provenance_of(*v) else {
                continue;
            };
            let align = s.mem.align_of(p.base).unwrap_or(1).max(1);
            if *mask >= i128::from(align) {
                s.mark_bit_inspected(dst);
                return;
            }
        }
        let _ = a;
    }

    /// Whether an operand is 020 §4.1's `Undef`.
    fn is_undef(&mut self, a: &mut TermArena, s: &mut State, o: &Operand) -> bool {
        matches!(self.operand(a, s, o), Some(Value::Undef))
    }

    /// The result of an operation with an `Undef` operand: `Undef`, and the run says it
    /// knows less than exactly.
    ///
    /// `Unknown` rather than `Approximated`: chiero has not *approximated* anything here,
    /// it has propagated the program's own absence of a value. 023 §7's `Approximated`
    /// means "we kept one of several", which would be the wrong claim.
    fn undef_result(&mut self, s: &mut State, span: Span, why: &str) -> Option<Value> {
        s.degrade(Fidelity::Unknown, AssumptionKind::NoInformation, span, why);
        Some(Value::Undef)
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
                    // The value too, not just the field. 023 §8's report exists so a reader
                    // can tell a generous bound from a trivial one, and six of the seven
                    // budget messages already say which they were.
                    &format!("max_depth ({}) reached", self.budget.max_depth),
                );
                return None;
            }
            let i = b.insts[s.pc.1].clone();
            let i = &i;
            self.emit(a, s, i.span, Ev::BeforeInst(i));
            // An `Assume` or a `Kill` at `BeforeInst` has to take effect *before* the
            // instruction runs, or contract 19's "subsequent branch feasibility reflects
            // it" is off by one instruction — which for a two-block fixture is the whole
            // difference.
            if !matches!(s.status, Status::Running) {
                return None;
            }
            self.exec_inst(a, s, i);
            self.emit(a, s, i.span, Ev::AfterInst(i));
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
                // 021 §7.2, before the value is computed: the *structure* of the
                // operation is what reveals the question, and the arena folds it away.
                self.note_pointer_bits(a, s, *dst, rv);
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
                    self.report_faults(a, s, &r.faults, i.span);
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
                vol,
            } => {
                // **A symbolic offset is written, not dropped.** Before wave 197 this fell
                // to the refusal below and the *program's* write vanished — so a later read
                // of the same object answered a ground value for a byte the write may have
                // hit. A refusal that silently keeps stale bytes is worse than a refusal,
                // because the run then produces a confident wrong answer (021 §3.1).
                //
                // `write_at_symbolic_offset` either writes an if-then-else over candidates
                // or promotes the object to an array; `ITE_THRESHOLD` inside `chiero-mem`
                // decides which, and an empty candidate list is its "no pinning available"
                // case. Passing candidates from here would duplicate that policy.
                if let Some(Value::SymPtr { base, off }) = self.operand(a, s, addr) {
                    if *vol == Volatility::Volatile {
                        self.lowering_gap(s, i.span, "a volatile store at a symbolic offset");
                        return;
                    }
                    let Some(v) = self.operand(a, s, val) else {
                        self.lowering_gap(s, i.span, "a store of an untranslatable value");
                        return;
                    };
                    let Some(vt) = self.address_of_value(a, s, v, i.span) else {
                        self.lowering_gap(s, i.span, "a store of a value with no term");
                        return;
                    };
                    // Byte by byte, least significant first, mirroring the load's
                    // composition so a store and the read after it agree by construction.
                    let size = size_of_cty(ty);
                    for k in 0..size {
                        let w = a.width(off);
                        let step = a.bv(w, k as u128);
                        let at = a.add(off, step);
                        let lo = (k * 8) as u32;
                        let byte = a.extract(vt, lo + 7, lo);
                        let r = s
                            .mem
                            .write_at_symbolic_offset(a, base, at, &[], byte, i.span);
                        self.report_faults(a, s, &r.faults, i.span);
                    }
                    return;
                }
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
                    // **Storing `Undef` leaves the destination uninitialized**, which is
                    // what it means: a later read is 021 §3.1's uninitialized-read
                    // finding rather than a read of some value chiero invented here.
                    Some(Value::Undef) => {
                        let size = size_of_cty(ty);
                        let r = s.mem.havoc_range_reporting(
                            a,
                            p,
                            size,
                            chiero_mem::HavocFill::Uninitialized,
                            i.span,
                        );
                        self.report_faults(a, s, &r.faults, i.span);
                        return;
                    }
                    Some(Value::Ptr(q)) => {
                        let Some(t) = self.address_term(a, s, q, i.span) else {
                            return;
                        };
                        t
                    }
                    // **A symbolic pointer stores its address**, like a concrete one. The
                    // draft that refused here re-introduced wave 195's invented
                    // `uninitialized-read`: the destination went unwritten, so reading it
                    // back accused the program of never storing what it had just stored.
                    Some(v @ Value::SymPtr { .. }) => {
                        let Some(t) = self.address_of_value(a, s, v, i.span) else {
                            self.lowering_gap(s, i.span, "a store of a symbolic pointer value");
                            return;
                        };
                        t
                    }
                    None => {
                        // **A store chiero cannot translate still happens.** Returning here left
                        // the slot holding whatever was there before, so `x = x * 3.0L` on 2.0 read
                        // back as 2 — a confidently wrong number, with the run degraded and the
                        // operation named. Declaring the gap and poisoning the value are two
                        // obligations and this site met only the first.
                        //
                        // **A fresh symbol, not an uninitialized slot**, and the comment above says
                        // why: wave 195's draft refused to write and made a later read accuse the
                        // program of never storing what it had just stored. The program did store;
                        // chiero does not know what. "Written, value unknown" is the only answer
                        // that is true of both.
                        //
                        // The same rule the extern-return site states one level up: a value nobody
                        // modeled is a fresh input, because silently keeping a plausible one is the
                        // failure that reads uninitialized memory as zero.
                        self.lowering_gap(s, i.span, "a store of an untranslatable value");
                        // **Only where a term can hold it.** A value wider than the arena's
                        // maximum has no symbol to mint, and asking for one panics rather than
                        // degrading — which is how the wide-store fixture in `step.rs` found this
                        // line. For those the old behaviour stands and the slot keeps what it had;
                        // that is the stale-value defect surviving in the narrow case, and it is
                        // recorded in §9 rather than papered over. `Int(256)` is the only shape in
                        // the tree that reaches it.
                        let width = size_of_cty(ty) * 8;
                        let Ok(w) = u32::try_from(width).map(|w| w.min(chiero_solver::MAX_BV_BITS))
                        else {
                            return;
                        };
                        if u64::from(w) != width {
                            return;
                        }
                        self.fresh_count += 1;
                        self.input(
                            a,
                            s,
                            chiero_solver::Sort::BitVec(w),
                            &format!("opaque{}", self.fresh_count),
                            InputOrigin::Opaque { span: i.span },
                        )
                    }
                };
                let _ = align;
                let size = size_of_cty(ty);
                // **020 §4.2: a volatile store is an observable event.** Recorded before
                // the write so a fault on the write cannot lose it — the outside world
                // saw the attempt either way, and a device register written twice was
                // written twice.
                if *vol == Volatility::Volatile {
                    s.effects.push(Effect {
                        kind: EffectKind::VolatileStore,
                        span: i.span,
                        detail: format!("volatile store of {size} byte(s) to {p:?}"),
                    });
                }
                let r = s.mem.write_term(a, p, t, size, Endian::Little, i.span);
                self.report_faults(a, s, &r.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
            }
            InstKind::VaArg { dst, list, ty } => {
                let Some(Value::Ptr(p)) = self.operand(a, s, list) else {
                    self.lowering_gap(s, i.span, "va_arg on a non-pointer");
                    return;
                };
                let cur = s.mem.read_term(a, p, 8, Endian::Little, i.span);
                self.report_faults(a, s, &cur.faults, i.span);
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
                self.report_faults(a, s, &own.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
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
                self.report_faults(a, s, &r.faults, i.span);
            }
            // `va_end` has no effect chiero can observe: the object's lifetime is the
            // frame's. Recording it as a gap would degrade every correct variadic
            // function for doing the right thing.
            InstKind::VaEnd { list } => {
                let _ = self.operand(a, s, list);
            }
            // **A phi takes the incoming belonging to the edge actually taken** (020 §9).
            //
            // The only instruction whose operands are not all evaluated: evaluating them
            // all would read values that are undefined on this path, and for a loop phi
            // the latch incoming does not exist yet on the first iteration.
            //
            // A phi whose predecessor is not listed is a verifier error, not something to
            // paper over — taking "the first incoming" would report the *other* branch's
            // value as a counterexample, so an unmatched edge leaves the destination
            // unset and the use that follows is the one that reports it.
            InstKind::Phi { dst, ty, incomings } => {
                let from = s.stack.last().and_then(|f| f.prev_block);
                if let Some(op) = from
                    .and_then(|p| incomings.iter().find(|(b, _)| *b == p))
                    .map(|(_, v)| v.clone())
                    && let Some(v) = self.operand(a, s, &op)
                {
                    s.set_local(*dst, v);
                } else {
                    let u = self.operand(a, s, &Operand::Const(Const::Undef(ty.clone())));
                    if let Some(v) = u {
                        s.set_local(*dst, v);
                    }
                }
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
        // **`Value`, not `Term`** (§6): a checker that cannot tell which tracked object an
        // argument refers to cannot implement `free(p)` or `memcpy` overlap detection, and
        // 021 §7's guard gaps make recovering it by address search lossy by construction.
        //
        // **Emitted before the indirect split.** After it, `Event::Call` never fired for
        // an indirect call at all: `CallReturn` arrived unpaired, 042's typestate shape
        // could not arm itself on VPP's node dispatch — which §5 calls "the ordinary path
        // rather than an exotic one" — and `callee_name`'s `Indirect` arm was dead code.
        // Found by review.
        let arg_vals: Vec<Option<Value>> = args.iter().map(|o| self.operand(a, s, o)).collect();
        self.emit(
            a,
            s,
            span,
            Ev::Call {
                callee: callee.clone(),
                args: arg_vals,
            },
        );
        let id = match callee {
            Callee::Direct(id) => id,
            Callee::Indirect(op) => {
                self.indirect(a, s, op, dst, args, span);
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
            // **Contract 24's hard case.** A modeled or unmodeled extern produces its
            // result here and has no return instruction anywhere, so `Event::Return`
            // never fires for it. An implementation that emitted `CallReturn` from the
            // callee's epilogue would be correct for every defined function and silently
            // never fire for any extern — which is exactly the hook 042's only worked
            // example needs.
            let ret = dst.and_then(|d| s.local(d));
            self.emit(
                a,
                s,
                span,
                Ev::CallReturn {
                    callee: callee.clone(),
                    ret,
                    dst,
                },
            );
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
            bit_inspected: Vec::new(),
            prev_block: None,
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
                self.emit(a, s, Span::DUMMY, Ev::Return(val));
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
                    // **In the caller, once the result exists** (contract 24). The frame
                    // is gone and `pc` is back at the call site, so a checker keying on
                    // the returned value sees the state the caller will continue from.
                    let ret = f.ret_dst.and_then(|d| s.local(d));
                    self.emit(
                        a,
                        s,
                        at,
                        Ev::CallReturn {
                            callee: Callee::Direct(f.func),
                            ret,
                            dst: f.ret_dst,
                        },
                    );
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
            // **020 contract 42.** One state per declared target. The address is *not*
            // resolved to a block — a `.cir` label address has no representation the
            // memory model can match against a `BlockId` — so the target list is the
            // frontend's declaration and the run says so rather than implying the set was
            // computed. VPP has zero computed gotos tree-wide (020 §4), so this exists to
            // keep the terminator honest rather than to carry traffic.
            Terminator::IndirectGoto { addr, targets } => {
                let _ = self.operand(a, s, addr);
                if targets.is_empty() {
                    s.give_up(
                        "an indirect goto with no declared targets".into(),
                        Span::DUMMY,
                    );
                    return None;
                }
                s.degrade(
                    Fidelity::Approximated,
                    AssumptionKind::OpaqueCode,
                    Span::DUMMY,
                    "an indirect goto: every declared target was explored, and the \
                     declaration is the frontend's",
                );
                let mut mine = *targets.last().expect("non-empty");
                let rest: Vec<BlockId> = targets[..targets.len() - 1].to_vec();
                for to in rest {
                    if self.forks >= self.budget.max_indirect {
                        s.degrade(
                            Fidelity::Bounded,
                            AssumptionKind::BudgetHit,
                            Span::DUMMY,
                            &format!("max_indirect ({}) reached", self.budget.max_indirect),
                        );
                        mine = to;
                        break;
                    }
                    self.forks += 1;
                    let mut sib = s.clone();
                    sib.id = self.new_id();
                    self.take_edge(&mut sib, to);
                    self.pending.push(sib);
                }
                self.take_edge(s, mine)
            }
            // 020's `Switch`. Nothing produced one until 015's lowering existed, which is
            // why this arm was missing and every `switch` in a C program reached the
            // catch-all below as "unsupported terminator".
            Terminator::Switch {
                scrut,
                cases,
                default,
                ..
            } => self.switch(a, s, scrut, cases, *default),
            // **No catch-all.** Every `Terminator` 020 defines is handled above, and the
            // compiler now says so — an `_` arm here would silently absorb a variant
            // added later, which is exactly how `Switch` went unimplemented until 015
            // produced one.
        }
    }

    /// Move to `to`, counting the edge. 023 §8: the bound is per **back edge**, and CIR
    /// has no loops (020 §1) — an edge to a block already on this path is the back edge,
    /// which needs no dominator analysis to recognize at run time.
    fn take_edge(&mut self, s: &mut State, to: BlockId) -> Option<State> {
        let from = s.pc.0;
        // **The bound is enforced where it is counted, and that is the only place it can be.**
        //
        // This used to repeat the step loop's `s.steps > max_depth` check and degrade here too.
        // Wave 222 showed the branch is unreachable: `steps` is initialized to zero at every
        // construction site and incremented at exactly one, which tests the same comparison
        // immediately afterwards — so a state arriving here has already been cut if it was over.
        // Mutating the two sites separately confirmed it (only the step loop's copy is killed by
        // anything), and an `eprintln!` in the branch fired zero times across the whole workspace.
        //
        // What replaces it is the invariant itself, executed by every test rather than a branch no
        // test can reach: an unreachable guard always survives mutation, so it protects nothing and
        // reports nothing. If a second increment site is ever added without a check beside it, this
        // fires immediately and names the reason.
        debug_assert!(
            s.steps <= self.budget.max_depth,
            "a state reached an edge over max_depth ({}) with {} steps: the step loop is the only \
             place that counts, so a second counting site has appeared without a bound check",
            self.budget.max_depth,
            s.steps
        );
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
        // Recorded **before** `pc` moves, so a `Phi` at the top of `to` reads the edge
        // that brought it there rather than the block it is already in.
        let from = s.pc.0;
        if let Some(fr) = s.stack.last_mut() {
            fr.prev_block = Some(from);
        }
        s.pc = (to, 0);
        let fid = s.func();
        s.trace.push((fid, to));
        None
    }

    /// A `Switch`, which is a multi-way `Br`.
    ///
    /// A **ground** scrutinee makes no solver call at all and takes exactly one edge —
    /// the same fast path 023 §3 step 4 requires of a constant branch, and the one that
    /// carries essentially all real traffic, since a `switch` on a symbolic value is rare
    /// next to a `switch` on a parsed opcode.
    ///
    /// A symbolic one forks per *feasible* case, with `scrut == value` on each and the
    /// conjunction of the negations on the default. Forking unconditionally would create
    /// a state per case label with an unsatisfiable path condition — states that cost a
    /// fork and describe nothing the program can do, which is the same mistake 021 §5.2's
    /// arenas had to avoid.
    fn switch(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        scrut: &Operand,
        cases: &[(i128, BlockId)],
        default: BlockId,
    ) -> Option<State> {
        let Some(v) = self.operand(a, s, scrut) else {
            s.give_up("a switch on an unreadable operand".into(), Span::DUMMY);
            return None;
        };
        let Value::Scalar(t) = v else {
            // `Undef` selects nothing in particular, so every edge is possible and none
            // is constrained — the same rule as a branch on `Undef` (020 contract 43).
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                Span::DUMMY,
                "a switch on undef: every edge explored, none constrained",
            );
            let mut targets: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
            targets.push(default);
            let mine = targets.pop().expect("at least the default");
            for to in targets {
                let mut sib = s.clone();
                sib.id = self.new_id();
                self.take_edge(&mut sib, to);
                self.pending.push(sib);
            }
            return self.take_edge(s, mine);
        };

        if let Ok(g) = a.eval_ground(t) {
            // The case values are `i128` and the scrutinee's bits are unsigned, so the
            // comparison has to happen at the scrutinee's width with C's sign — a
            // `case -1:` on an `int` scrutinee is `0xffffffff` in the bits.
            let w = a.width(t).min(127);
            let bits = g.bits();
            let val = if w > 0 && (bits >> (w - 1)) & 1 == 1 {
                (bits as i128) - (1i128 << w)
            } else {
                bits as i128
            };
            let to = cases
                .iter()
                .find(|(c, _)| *c == val)
                .map(|(_, b)| *b)
                .unwrap_or(default);
            return self.take_edge(s, to);
        }

        // Symbolic: one edge per feasible case, plus the default if no case must match.
        let w = a.width(t);
        let mut live: Vec<(BlockId, Term)> = Vec::new();
        let mut negs: Vec<Term> = Vec::new();
        for (val, to) in cases {
            let k = a.bv(w, *val as u128);
            let eq = a.eq(t, k);
            negs.push(negate(a, eq));
            if !matches!(self.feasible(a, s, eq), Feas::No) {
                live.push((*to, eq));
            }
        }
        let mut none_matches = None;
        for n in negs {
            none_matches = Some(match none_matches {
                None => n,
                Some(acc) => a.and(acc, n),
            });
        }
        if let Some(nm) = none_matches {
            if !matches!(self.feasible(a, s, nm), Feas::No) {
                live.push((default, nm));
            }
        } else {
            // A `switch` with no cases is just a jump to the default.
            live.push((default, a.bv(1, 1)));
        }
        let Some((mine, mine_c)) = live.pop() else {
            s.give_up("every switch edge is infeasible".into(), Span::DUMMY);
            return None;
        };
        for (to, c) in live {
            self.forks += 1;
            let mut sib = s.clone();
            sib.id = self.new_id();
            sib.constrain_checked(c);
            self.take_edge(&mut sib, to);
            self.pending.push(sib);
        }
        s.constrain_checked(mine_c);
        self.take_edge(s, mine)
    }

    fn branch(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        cond: &Operand,
        bt: BlockId,
        bf: BlockId,
    ) -> Option<State> {
        // **A branch on `Undef` forks both ways** (020 contract 43). Taking one side is
        // choosing for a program that did not choose, and no path constraint is added:
        // there is no term to constrain, and inventing one would let a later query
        // "prove" something about a value that does not exist.
        if matches!(self.operand(a, s, cond), Some(Value::Undef)) {
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                Span::DUMMY,
                "a branch on undef: both sides explored, neither constrained",
            );
            let mut sibling = s.clone();
            sibling.id = self.new_id();
            self.take_edge(s, bt);
            self.take_edge(&mut sibling, bf);
            return Some(sibling);
        }
        let Some(Value::Scalar(c)) = self.operand(a, s, cond) else {
            s.give_up("branch condition is not a scalar".into(), Span::DUMMY);
            return None;
        };
        // **021 §7.2: chiero's bump allocator does not get to decide a branch.** A test on
        // pointer bits *below the object's guaranteed alignment* is answered by where
        // chiero happened to place the object — and because addresses are deterministic
        // (contract 15) the wrong answer is stable, reproducible, and never looks flaky.
        // `nsim_input.c`'s `((uword) ep & (CLIB_CACHE_LINE_BYTES - 1)) == 0` is the real
        // case. Both sides are explored and the run says it could not decide.
        if let Operand::Value(cv) = cond
            && s.is_bit_inspected(*cv)
        {
            let why = "a branch tests pointer bits below the object's guaranteed \
                       alignment: the answer would come from where chiero placed the \
                       object, so both sides were explored";
            s.degrade(
                Fidelity::Unknown,
                AssumptionKind::NoInformation,
                Span::DUMMY,
                why,
            );
            let mut sibling = s.clone();
            sibling.id = self.new_id();
            self.take_edge(s, bt);
            self.take_edge(&mut sibling, bf);
            return Some(sibling);
        }
        // §3 step 4: a constant condition makes **no solver call**. This fast path carries
        // most of the traffic and must exist before any benchmark is believed.
        if let Ok(v) = a.eval_ground(c) {
            return self.take_edge(s, if v.bits() != 0 { bt } else { bf });
        }
        let neg = negate(a, c);
        let t_ok = self.feasible(a, s, c);
        let f_ok = self.feasible(a, s, neg);
        // **After the feasibility questions, before the split.** A checker is told which
        // sides are live, which it cannot work out for itself without repeating both
        // solver calls — and it is told once, on the state that is about to become two,
        // so an `Assume` here still applies to both children.
        self.emit(
            a,
            s,
            Span::DUMMY,
            Ev::Fork {
                cond: c,
                feasible: (matches!(t_ok, Feas::Yes), matches!(f_ok, Feas::Yes)),
            },
        );
        let mut sibling = s.clone();
        sibling.id = self.new_id();
        // The clone shares the trace up to here; each side records its own next block.

        match (t_ok, f_ok) {
            (Feas::Yes, Feas::Yes) => {
                s.constrain_checked(c);
                self.take_edge(s, bt);
                sibling.constrain_checked(neg);
                self.take_edge(&mut sibling, bf);
                Some(sibling)
            }
            (Feas::Yes, Feas::No) => {
                s.constrain_checked(c);
                self.take_edge(s, bt)
            }
            (Feas::No, Feas::Yes) => {
                s.constrain_checked(neg);
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
                s.constrain_unchecked(neg);
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
                s.constrain_unchecked(c);
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
                s.constrain_unchecked(c);
                self.take_edge(s, bt);
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    BRANCH_UNDECIDED,
                );
                sibling.constrain_unchecked(neg);
                self.take_edge(&mut sibling, bf);
                sibling.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::SolverUnknown,
                    Span::DUMMY,
                    BRANCH_UNDECIDED,
                );
                Some(sibling)
            }
        }
    }

    /// One solver query under the path condition plus `extra`, keeping the model.
    /// How many distinct offsets a symbolic `PtrAdd` is worth forking on.
    ///
    /// A policy number, not a contract. Large enough for the array indices real code
    /// writes, small enough that an unconstrained `size_t` does not explode the state
    /// space before the bound is noticed.
    const MAX_SYMBOLIC_OFFSETS: usize = 16;

    /// Enumerate the feasible values of a symbolic `PtrAdd` offset and fork one state per
    /// answer, returning the value for the state that carries on.
    ///
    /// Model-driven, exactly like `resolve_symbolic_base`: each query asks for *some*
    /// value the offset can still take and the next excludes it, so the cost is one query
    /// per answer plus one to prove there are no more — proportional to what the offset can
    /// name rather than to the object's size.
    fn fork_on_offset(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        p: Pointer,
        t: Term,
        span: Span,
    ) -> Option<Value> {
        let mut found: Vec<i64> = Vec::new();
        let mut excluded: Vec<Term> = Vec::new();
        // **Exhaustion and the solver giving up are different answers.** `Unsat` means
        // there are no more offsets and the enumeration is complete; `Unknown` means the
        // solver could not tell, and treating that as complete would explore one value and
        // call the run `Exact` — a fabricated address on every path not taken, which is the
        // objection this whole function exists to answer. With the default tier-1 solver
        // `Unknown` is the *common* case, so getting this wrong is not a corner.
        let mut complete = false;
        loop {
            let mut extra = excluded.clone();
            match self.probe(a, s, &extra) {
                CheckResult::Sat(model) => {
                    let Ok(bv) = a.eval(&model, t) else { break };
                    let Ok(v) = i64::try_from(bv.signed()) else {
                        break;
                    };
                    found.push(v);
                }
                CheckResult::Unsat => {
                    complete = true;
                    break;
                }
                CheckResult::Unknown(_) => break,
            }
            if found.len() > Self::MAX_SYMBOLIC_OFFSETS {
                break;
            }
            let v = *found.last().expect("just pushed");
            let w = a.width(t);
            let lit = a.bv(w, v as u128);
            let eq = a.eq(t, lit);
            let ne = a.not(eq);
            excluded.push(ne);
            extra.push(ne);
        }

        if !complete || found.is_empty() || found.len() > Self::MAX_SYMBOLIC_OFFSETS {
            // **Before giving up, ask the one question that is still answerable.**
            //
            // Enumeration failed, which says nothing about whether the access is *safe* —
            // and "can this offset leave the object" is a single feasibility query, not
            // four billion. Wave 156 established the shape for a symbolic divisor and it
            // is the same here: `Sat` means an out-of-range index exists on this path and
            // is reported with the witness that proves it; `Unsat` means the path already
            // constrains the index and there is nothing to say; `Unknown` leaves the
            // degradation below, which is the honest answer for a question not answered.
            //
            // `!(0 <= off && off + size <= obj_size)`, in the offset's own width so the
            // comparison matches the term.
            //
            // **Signed, and the two comparisons are *equivalent* to one unsigned one** —
            // recorded because it is not obvious and mutation proved it. With a lower bound
            // of zero, `unsigned(t) > limit` is exactly `t < 0 || t > limit` in signed
            // terms, since a negative two's-complement value is an enormous unsigned one. So
            // `unsigned-comparison` survives as an equivalent mutant rather than a gap. The
            // signed pair is kept because it states the two bounds the C rule names, and
            // stops being equivalent the moment a lower bound other than zero appears.
            if let Some(obj_size) = s.mem.size_of_pub(p.base) {
                let w = a.width(t);
                let size = 1i128;
                let lo = a.bv(w, 0);
                let hi = a.bv(w, (obj_size as i128 - size).max(0) as u128);
                let below = a.slt(t, lo);
                let above = a.slt(hi, t);
                let out = a.or(below, above);
                if let CheckResult::Sat(m) = self.probe(a, s, &[out]) {
                    // **The witness comes out of the model, not out of `obj_size`.**
                    //
                    // The field's contract is "an offset the path allows, which is past the
                    // object", and `obj_size` satisfies only the second half. One past the end is
                    // always outside, so the sentence never looked absurd — but for
                    // `pool + ((i & 31) + 64)` the path reaches byte offsets 256..380 and the
                    // report said 32, naming an input that does not exist. A false specific claim
                    // is worse than a vague one, because a reader goes looking.
                    //
                    // `m` is the model that made `out` true, so evaluating the offset term under
                    // it yields an offset this path really does allow. Same source as wave 205's
                    // uninitialized-read witness and wave 208's location.
                    let witness = a.eval(&m, t).map_or(obj_size as i64, |c| c.bits() as i64);
                    self.report_faults(
                        a,
                        s,
                        // **The pointer fault, not the access one.** Nothing has been
                        // touched here — this runs for a `PtrAdd` — and the access variant
                        // has a size field there is no honest value for. Wave 193 passed
                        // `1` and the report read "1-byte access of ga", which was untrue
                        // twice over.
                        &[chiero_mem::MemFault::PointerOutsideObject {
                            obj: p.base,
                            obj_size,
                            witness,
                            at: span,
                        }],
                        span,
                    );
                    // **Reported *and* still degraded**, rather than returning here. Both
                    // statements are true and they are about different things: the access
                    // may leave the object, and the offset could not be enumerated. An
                    // early return replaced the second with the first, which
                    // `a_symbolic_ptr_add_offset_is_a_gap` caught — it asserts the
                    // enumeration bound is recorded as `Bounded`/`BudgetHit`, and a
                    // finding does not make that stop being so.
                }
            }
            s.degrade(
                Fidelity::Bounded,
                AssumptionKind::BudgetHit,
                span,
                &format!(
                    "a symbolic pointer offset was not enumerated: {} value(s) found and \
                     the search {}",
                    found.len(),
                    if complete {
                        "exceeded the bound".to_string()
                    } else {
                        "was cut short by the solver".to_string()
                    }
                ),
            );
            // **A pointer with the symbolic offset**, which is what the program computed.
            // The bounds question above has already been asked and answered; what is handed
            // back now carries the object and the term, so a load can go to `chiero-mem`'s
            // symbolic path instead of stopping.
            //
            // Sites that do not recognise `SymPtr` refuse exactly as they refused the fresh
            // symbol below, so nothing gains pointer semantics it was not given.
            return Some(Value::SymPtr {
                base: p.base,
                off: t,
            });
        }

        // Siblings for every value but the first; this state takes the first, which keeps
        // exploration order deterministic (023 §4 — ties break by creation order).
        let mine = found[0];
        for &v in &found[1..] {
            let mut sib = s.clone();
            sib.id = self.new_id();
            let w = a.width(t);
            let lit = a.bv(w, v as u128);
            let eq = a.eq(t, lit);
            sib.constrain_checked(eq);
            // The destination local is filled by `eval`'s caller from the returned
            // `Value`, which a sibling never reaches — so it is set here, from the
            // `pending_dst` the instruction dispatcher recorded for exactly this case.
            if let Some(d) = self.pending_dst {
                sib.set_local(
                    d,
                    Value::Ptr(Pointer {
                        base: p.base,
                        off: p.off.wrapping_add(v),
                    }),
                );
            }
            self.pending.push(sib);
        }
        let w = a.width(t);
        let lit = a.bv(w, mine as u128);
        let eq = a.eq(t, lit);
        s.constrain_checked(eq);
        Some(Value::Ptr(Pointer {
            base: p.base,
            off: p.off.wrapping_add(mine),
        }))
    }

    fn probe(&mut self, a: &mut TermArena, s: &State, extra: &[Term]) -> CheckResult {
        self.solver_calls += 1;
        // A fresh solver per query is wasteful and will be replaced by a per-state
        // incremental stack; correctness first, and the backend process itself is
        // long-lived (022 §4) so the cost is the assertion replay, not a spawn.
        if self.solver.is_none() {
            // Counted, because `backend_spawns` cannot distinguish one solver from many:
            // a freshly built solver reports one spawn for its own first query, which is
            // the same number the correct implementation reports for the whole run.
            self.solver_inits += 1;
            self.solver = Some(match self.backend_for_run() {
                Some(b) => TieredSolver::with_backend(b),
                None => TieredSolver::new(),
            });
        }
        let solver = self.solver.as_mut().expect("just built");
        // The path condition goes in as *assumptions* rather than assertions, so the
        // solver's own stack stays empty between queries and sibling states share both
        // the process and the caches.
        // **`check_path`, not `check`** — 022 §6.2's independence slicing needs to know
        // which variables the *question* is about, and a bare `check` with everything in
        // one list cannot tell. Calling `check` here left slicing unreachable from any
        // real run: it was implemented, tested, and never executed. `extra` is the query,
        // which is exactly the distinction `check_path` is built around.
        let mut pc = PathCondition::from_parts(s.path.clone(), s.path_unchecked);
        solver.check_path(a, &mut pc, extra)
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
            RValue::Bin {
                op,
                a: x,
                b: y,
                ty,
                signed,
            } => {
                // **`Undef` in, `Undef` out** (020 contract 43) — including `undef * 0`,
                // which is 0 for every *value* of the operand and undefined for a
                // non-value. Checked before `scalar`, which has no term to hand back.
                if self.is_undef(a, s, x) || self.is_undef(a, s, y) {
                    return self.undef_result(s, span, "arithmetic on undef");
                }
                let (Some(xv), Some(yv)) = (self.scalar(a, s, x), self.scalar(a, s, y)) else {
                    return self.lowering_gap(s, span, "a non-scalar arithmetic operand");
                };
                // **The event, before the value** — the value is always produced, so an
                // early return added later cannot silently drop the event.
                self.note_ub(a, s, *op, ty, *signed, xv, yv, span);
                match bin(a, *op, xv, yv) {
                    Some(t) => Value::Scalar(t),
                    None => return self.lowering_gap(s, span, &format!("{op:?}")),
                }
            }
            RValue::Cmp { op, a: x, b: y, .. } => {
                if self.is_undef(a, s, x) || self.is_undef(a, s, y) {
                    return self.undef_result(s, span, "a comparison against undef");
                }
                // **A pointer compares as its address.** `scalar` refuses a `Value::Ptr` —
                // rightly, since a pointer is not a bit-vector until someone asks for its
                // address — so every comparison with a pointer operand ended the path as a
                // lowering gap. That is `if (p)`, `if (p == 0)`, `while (p)`, `p ? a : b`,
                // `p && q` and `!p`: the whole of C's null checking.
                //
                // `address_term` is the same function `PtrToInt` uses, so a pointer
                // compared and a pointer cast to an integer agree by construction rather
                // than by two implementations happening to match.
                let (Some(xv), Some(yv)) = (
                    self.cmp_operand(a, s, x, span),
                    self.cmp_operand(a, s, y, span),
                ) else {
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
            RValue::Load { addr, ty, vol, .. } => {
                // **A symbolic offset is answerable, and `chiero-mem` is what answers it.**
                // 021 §3 has carried symbolic offsets since it was written: `read_term_at`
                // reads a byte at one, as an if-then-else chain below `ITE_THRESHOLD` and a
                // promoted `select` above. Before wave 196 the engine had no value type to
                // pass it, so `ca[i & 63]` produced nothing at all — an index that provably
                // cannot leave a 64-byte array.
                //
                // **Composed byte by byte, little-endian**, because `read_term_at` answers
                // for one byte. That is not a shortcut: it is the same decomposition the
                // concrete path uses, and it keeps the `ITE_THRESHOLD` decision per byte
                // where `chiero-mem` makes it, rather than duplicating that policy here.
                if let Some(Value::SymPtr { base, off }) = self.operand(a, s, addr) {
                    if *vol == Volatility::Volatile {
                        return self.lowering_gap(s, span, "a volatile load at a symbolic offset");
                    }
                    let size = size_of_cty(ty);
                    let mut byte_terms: Vec<Term> = Vec::new();
                    for k in 0..size {
                        let w = a.width(off);
                        let step = a.bv(w, k as u128);
                        let at = a.add(off, step);
                        let r = s.mem.read_term_at(a, base, at, &[], span);
                        self.report_faults(a, s, &r.faults, span);
                        // `None` is `chiero-mem` declining the byte — a fault it has already
                        // reported. Refusing the whole load then is right: half a value is
                        // not a value, and composing the rest would answer with bytes the
                        // read never obtained.
                        let Some(v) = r.value else {
                            return self.lowering_gap(
                                s,
                                span,
                                "a byte at a symbolic offset could not be read",
                            );
                        };
                        byte_terms.push(v);
                    }
                    // Little-endian: byte 0 is least significant, matching `GlobalInit::Bytes`
                    // and every other reader in the tree.
                    let mut acc = byte_terms[0];
                    for t in byte_terms.iter().skip(1).copied() {
                        acc = a.concat(t, acc);
                    }
                    return Some(Value::Scalar(acc));
                }
                let Some(Value::Ptr(p)) = self.operand(a, s, addr) else {
                    return self.lowering_gap(s, span, "a load through a non-pointer address");
                };
                // **020 §4.2: a volatile load never reads the stored bytes.** It is a
                // device register, and reading back what was written is the one thing
                // hardware does not do — modelling it as memory makes
                // `*reg = 0; if (*reg == 0)` a certainty and explores branches the device
                // can never take. A fresh symbol *each time*, so two reads are two reads:
                // §4.2 forbids caching or CSE-ing them, and one shared value is CSE by
                // another name.
                if *vol == Volatility::Volatile {
                    self.fresh_count += 1;
                    let t = self.input(
                        a,
                        s,
                        sort_of(ty),
                        &format!("vol{}", self.fresh_count),
                        InputOrigin::Volatile { span },
                    );
                    return Some(Value::Scalar(t));
                }
                let size = size_of_cty(ty);
                if size == 0 {
                    // Nothing to read, so nothing to invent. `sort_of` would have handed
                    // back a 64-bit symbol for a load of a zero-sized type.
                    return self.lowering_gap(s, span, &format!("a load of {ty:?}"));
                }
                let r = s.mem.read_term(a, p, size, Endian::Little, span);
                // **The discharged list decides usability, not the raw one** (wave 249). A `maybe`
                // the engine has just proved away must not still be a reason to throw the value
                // out — that is what made a byte written before promotion read back as an invented
                // symbol while the report, correctly, said nothing at all.
                let live = self.report_faults(a, s, &r.faults, span);
                match r.value.filter(|_| !unusable(&live)) {
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
                            // **021 §6's recursive materialization.** A pointer read out
                            // of a lazy object is itself symbolic and names no object yet;
                            // the next dereference is what asks for one. Without this,
                            // `p->next->next` ends the path as unresolvable rather than
                            // as a bounded walk of a linked structure — which is the shape
                            // every VPP graph node starts with.
                            Err(_) => self.materialize_link(a, s, p.base, addr, t, span),
                        },
                    },
                    // **The declared width, not the width of the bytes read.** Memory is
                    // byte-addressed, so `read_term` hands back `size * 8` bits — and a
                    // type narrower than its storage then had a value wider than itself.
                    // `_Bool` is the case C has: `sizeof(_Bool) == 1` and its CIR type is
                    // `Int(1)`, so `load i1` produced eight bits and `add i1` reached the
                    // solver with operands of 8 and 1, which is an `assert_eq!` there —
                    // `_Bool b = 0; b += 1;` panicked the whole run rather than answering
                    // wrongly. The `None` branch below already used `sort_of(ty)`, the
                    // declared width; only the path that succeeded disagreed with it.
                    Some(t) => {
                        let have = a.width(t);
                        match sort_of(ty) {
                            chiero_solver::Sort::BitVec(w) if have > w => {
                                Value::Scalar(a.extract(t, w - 1, 0))
                            }
                            _ => Value::Scalar(t),
                        }
                    }
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
                    // **Floating negation flips the sign bit**, and for a concrete value
                    // that is arithmetic rather than a guess. A symbolic float still has
                    // no sort to constrain, so it stays a gap — the same line wave 167
                    // drew for `bin`, applied to the operator it did not reach.
                    //
                    // Not `0.0 - x`: that is a *subtraction*, and it turns -0.0 into +0.0
                    // and quiets a signalling NaN. Negation is defined on the sign bit
                    // alone (IEEE-754 §5.5.1), which is what `f64::neg` does.
                    UnOp::FNeg => match a.eval_ground(xv).ok().and_then(|c| match c.width() {
                        32 => Some((
                            32u32,
                            u128::from((-f32::from_bits(c.bits() as u32)).to_bits()),
                        )),
                        64 => Some((
                            64u32,
                            u128::from((-f64::from_bits(c.bits() as u64)).to_bits()),
                        )),
                        // **x87 needs no float arithmetic to negate.** The comment above is the
                        // reason it belongs here rather than waiting for the milestone: negation
                        // is defined on the sign bit alone, so it is exact at any width and
                        // wants no significand. Bit 79 is x87's sign.
                        //
                        // Without this, `-2.5L` was a gap — C has no negative literals, so every
                        // negative `long double` in every program is an `fneg` of a positive one.
                        80 => Some((80u32, c.bits() ^ (1u128 << 79))),
                        _ => None,
                    }) {
                        Some((w, bits)) => Value::Scalar(a.bv(w, bits)),
                        None => return self.lowering_gap(s, span, "FNeg on a symbolic float"),
                    },
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
                    // **The term's width, not the instruction's.** `fw`/`tw` are what the
                    // CIR *declares*; `xv` is what the engine actually holds, and the two
                    // can disagree without the verifier objecting — a `_Bool` that reached
                    // here as a one-bit `Cmp` result under a `Trunc` declared `Int(32) ->
                    // Int(1)` made `extract(xv, 31, 0)` panic on a one-bit term.
                    //
                    // 023's rule is that the engine never crashes on input it was handed;
                    // a declaration it cannot honour is a gap it reports. Widening here
                    // would be worse than the panic — it would invent bits.
                    CastKind::Trunc if tw <= fw => {
                        let have = a.width(xv);
                        if tw > have {
                            return self.lowering_gap(
                                s,
                                span,
                                &format!("Trunc to {tw} bits of a {have}-bit value"),
                            );
                        }
                        Value::Scalar(a.extract(xv, tw - 1, 0))
                    }
                    CastKind::ZExt if tw >= fw => Value::Scalar(a.zext(xv, tw)),
                    CastKind::SExt if tw >= fw => Value::Scalar(a.sext(xv, tw)),
                    // A `Bitcast` between equal widths is the identity on bits, which is
                    // exactly what 021 §3 means by "bytes are bytes".
                    CastKind::Bitcast if tw == fw => Value::Scalar(xv),
                    // **The six FP casts, concretely.** Symbolic operands fall through to
                    // the gap below, for the same reason float arithmetic does: there is no
                    // float sort to constrain.
                    CastKind::SiToFp
                    | CastKind::UiToFp
                    | CastKind::FpToSi
                    | CastKind::FpToUi
                    | CastKind::FpTrunc
                    | CastKind::FpExt => {
                        let mut overflowed = false;
                        match fcast(a, *kind, xv, fw, tw, &mut overflowed) {
                            Some(t) => {
                                // **C11 6.3.1.4's conversion, recorded where it happens.** A
                                // `Cast` never reaches `note_ub` — that is driven from
                                // `RValue::Bin` — and this is the only place that has both the
                                // value and the destination width.
                                if overflowed {
                                    s.ub.push(UbEvent {
                                        kind: UbKind::FloatCastOverflow,
                                        span,
                                        detail: format!(
                                            "{kind:?} of a value the {tw}-bit destination \
                                         cannot represent"
                                        ),
                                        requires: Vec::new(),
                                    });
                                }
                                Value::Scalar(t)
                            }
                            None => {
                                return self.lowering_gap(
                                    s,
                                    span,
                                    &format!("{kind:?} {fw} -> {tw} on a symbolic operand"),
                                );
                            }
                        }
                    }
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
                let base = self.global_object(a, s, *g);
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
                self.report_faults(a, s, &r.faults, span);
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
                let c = match a.eval_ground(t) {
                    Ok(c) => c,
                    Err(_) => {
                        // **A symbolic offset forks, one state per feasible value.**
                        //
                        // `chiero_mem::Pointer` carries a concrete `i64`, so a symbolic
                        // offset cannot be represented — and guessing one would be a
                        // fabricated address on every path but one, which is 021 §7's
                        // objection. Forking is the shape `Switch` already uses for a
                        // symbolic scrutinee (020 c14): each path then has a concrete
                        // offset and the memory model is untouched.
                        //
                        // Bounded, because an unconstrained index over a large object has
                        // more answers than is worth exploring. Past the bound the state
                        // degrades and names the offset rather than picking a value.
                        return self.fork_on_offset(a, s, p, t, span);
                    }
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
            requires: Vec::new(),
            witness: None,
            related: None,
        });
        s.degrade(
            Fidelity::Unknown,
            AssumptionKind::NoInformation,
            span,
            "a symbolic pointer with no constraint at all was not resolved",
        );
        s.status = Status::Terminated(TermReason::Unsupported);
    }

    /// 021 §5.2. Resolve `base + n` against a registered arena, before §5.1's search.
    ///
    /// Returns `None` when the address does not name a registered arena, in which case
    /// the caller falls through to the ordinary five-step resolution.
    ///
    /// **Three outcomes, so three states.** With `n` symbolic, both the element index
    /// `k = (n * index_scale) / pitch` and the within-element offset
    /// `d = (n * index_scale) % pitch` are symbolic, and an unconstrained index genuinely
    /// admits all three:
    ///
    /// - `d == 0` and `k < count` — a well-formed element pointer;
    /// - `d` in `[elem_size, pitch)` — §5.2 step 3's inter-element gap, one finding, and
    ///   **not** a pointer into element `k+1`, which is the silent failure: it reports
    ///   nothing and analyses the wrong buffer for the rest of the function;
    /// - `k >= count` — past the end of the region, which is step 4's bounds check.
    ///
    /// Assuming the first outcome instead of forking would be cheaper and would delete
    /// the two bug classes an arena exists to expose. Reporting the gap whenever it is
    /// *feasible* would be sound and useless, since for an unconstrained index it always
    /// is. Two extra states per arena access is bounded by the access count, not by the
    /// object count — which is what §5.2 step 4 rules out when it forbids "a fork over
    /// the whole address space".
    fn resolve_in_arena(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        addr: Term,
        span: Span,
    ) -> Option<Option<Value>> {
        let arenas = self.arena_bases.clone();
        for (base, shape) in arenas {
            // `base + n`, in either operand order. Nothing else is an arena address —
            // matching more loosely would claim regions the caller never registered.
            let Some((chiero_solver::BinKind::Add, x, y)) = a.as_bin(addr) else {
                continue;
            };
            let n = if x == base {
                y
            } else if y == base {
                x
            } else {
                continue;
            };

            let w = a.width(n);
            let pitch = a.bv(w, shape.pitch as u128);
            let esz = a.bv(w, shape.elem_size as u128);
            let cnt = a.bv(w, shape.count as u128);
            let zero = a.bv(w, 0);
            // `n` is already in **bytes** here: the program computed `i << 6`, and
            // `index_scale` is what says those 64s are index units rather than a stride.
            // Recovering `i` and multiplying it back would be the same term.
            let k = a.udiv(n, pitch);
            let d = a.urem(n, pitch);

            let in_range = a.ult(k, cnt);
            let aligned = a.eq(d, zero);
            let in_elem = a.ult(d, esz);

            // **Each case is created only if it is feasible.** A ground index decomposes
            // to a ground `k` and `d`, so three of the four are refuted immediately and
            // the access produces one state. Creating them unconditionally would litter
            // every concrete buffer access with siblings whose path conditions are
            // unsatisfiable — states that cost a fork, report a finding, and describe
            // nothing the program can do.
            let not_in_elem = a.not(in_elem);
            let past = a.not(in_range);
            let not_aligned = a.not(aligned);
            let mid = a.and(not_aligned, in_elem);

            // (a) the inter-element gap: out of bounds, and **not** a pointer into
            // element k+1 — that is the silent failure, reporting nothing and analysing
            // the wrong buffer for the rest of the function.
            let gap_cond = a.and(not_in_elem, in_range);
            if !matches!(self.feasible(a, s, gap_cond), Feas::No) {
                let mut gap = s.clone();
                gap.id = self.new_id();
                gap.constrain_unchecked(gap_cond);
                self.finding_seq += 1;
                gap.findings.push(StateFinding {
                    id: self.finding_seq,
                    key: None,
                    message: format!(
                        "address falls in the {}-byte gap between arena elements \
                         (elem_size {}, pitch {}), which is out of bounds and is not a \
                         pointer into the next element",
                        shape.pitch - shape.elem_size,
                        shape.elem_size,
                        shape.pitch
                    ),
                    span,
                    requires: Vec::new(),
                    witness: None,
                    related: None,
                });
                gap.status = Status::Terminated(TermReason::Crashed);
                self.pending.push(gap);
            }

            // (b) past the end of the region — §5.2 step 4's bounds check against
            // `count`. An arena that resolves every index has stopped being a bound.
            if !matches!(self.feasible(a, s, past), Feas::No) {
                let mut over = s.clone();
                over.id = self.new_id();
                over.constrain_unchecked(past);
                self.finding_seq += 1;
                over.findings.push(StateFinding {
                    id: self.finding_seq,
                    key: None,
                    message: format!(
                        "arena index is out of bounds: the region holds {} element(s), \
                         and `count` is what bounds it",
                        shape.count
                    ),
                    span,
                    requires: Vec::new(),
                    witness: None,
                    related: None,
                });
                over.status = Status::Terminated(TermReason::Crashed);
                self.pending.push(over);
            }

            // (c) **a valid offset that is not an element start.** §5.2 step 3 puts the
            // gap at `d >= elem_size`, so `0 < d < elem_size` is a legitimate pointer
            // *into* element `k` — and `Pointer::off` is an `i64`, so chiero cannot
            // represent it. Saying so is the point: forcing `d == 0` on the good path
            // would silently delete every one of these, which is the whole failure mode
            // 021 §5.1 calls "a wrong answer instead of an honest unknown".
            let mid_cond = a.and(mid, in_range);
            if !matches!(self.feasible(a, s, mid_cond), Feas::No) {
                let mut part = s.clone();
                part.id = self.new_id();
                part.constrain_unchecked(mid_cond);
                part.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::NoInformation,
                    span,
                    "an arena address lands inside an element at a symbolic offset, \
                     which this memory model cannot represent",
                );
                part.status = Status::Terminated(TermReason::Unsupported);
                self.pending.push(part);
            }

            // (d) the well-formed element start, on the state that continues.
            let good = a.and(aligned, in_range);
            if matches!(self.feasible(a, s, good), Feas::No) {
                // Nothing left for this state to be; the siblings carry the outcomes.
                // **`Some(None)`, not `None`** — the address *was* an arena address, and
                // falling through to §5.1's search would report it as an unresolvable
                // pointer on top of the finding a sibling already carries.
                s.status = Status::Terminated(TermReason::Unreachable);
                return Some(None);
            }
            s.constrain_unchecked(good);
            let detail = format!(
                "an address was resolved through a registered arena element (pitch {}, \
                 elem_size {}, index_scale {}); the index names an element start on this \
                 path, and the other cases are explored separately",
                shape.pitch, shape.elem_size, shape.index_scale
            );
            self.note_once(s, AssumptionKind::OpaqueCode, span, &detail);
            s.degrade(
                Fidelity::Approximated,
                AssumptionKind::OpaqueCode,
                span,
                &detail,
            );
            let obj = self.arena_element(a, s, k, shape, span);
            return Some(Some(Value::Ptr(chiero_mem::Pointer { base: obj, off: 0 })));
        }
        None
    }

    /// One lazily-materialized object per accessed element index (021 §5.2 step 4).
    ///
    /// Keyed by the *term* of `k`, so two accesses computing the same index reach the
    /// same object — without that, `b->data[0]` and `b->data[1]` would live in different
    /// buffers and every intra-buffer invariant would be invisible.
    fn arena_element(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        k: Term,
        shape: ArenaShape,
        span: Span,
    ) -> chiero_mem::ObjectId {
        let _ = a;
        if let Some(id) = s.arena_objs.get(&k.0) {
            return *id;
        }
        let id = s
            .mem
            .alloc(chiero_mem::ObjKind::Heap, shape.elem_size, 64, span);
        // 021 §6: its contents are whatever the program put there, not zero.
        s.mem.mark_lazy(id);
        s.arena_objs.insert(k.0, id);
        id
    }

    /// Materialize the object a pointer loaded out of `from` points at (021 §6).
    ///
    /// Bounded by `max_depth`: past it every further link resolves to **one shared
    /// object** rather than a fresh one, and the state records `Fidelity::Bounded` naming
    /// the field that was cut. One object rather than unboundedly many is what makes the
    /// bound a bound; letting the walk continue is what keeps the findings after it.
    fn materialize_link(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        from: chiero_mem::ObjectId,
        addr: &Operand,
        t: Term,
        span: Span,
    ) -> Value {
        let _ = a;
        // **`saturating_add`, and the cut object is recorded at the ceiling.** Reading a
        // pointer *out of* the cut object was otherwise an unknown depth, defaulted to 0,
        // so the walk started counting again and allocated a fresh object per hop past the
        // bound — bounding the fidelity label and nothing else. Found by asserting that
        // three links and six cost the same.
        let depth = s
            .lazy_depth
            .get(&from)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        if depth > self.lazy.max_depth {
            let what = self
                .field_name_of(s, addr)
                .unwrap_or_else(|| "a pointer".into());
            let obj = match s.lazy_cut {
                Some(o) => o,
                None => {
                    let o =
                        s.mem
                            .alloc(chiero_mem::ObjKind::Lazy, self.lazy.scalar_extent, 16, span);
                    s.mem.mark_lazy(o);
                    // Recorded at the ceiling, so a pointer read back out of it is
                    // already past the bound and cuts again. This is the floor: leaving it
                    // absent made its depth default to 0 and the walk restart.
                    s.lazy_depth.insert(o, u32::MAX);
                    s.lazy_cut = Some(o);
                    o
                }
            };
            s.degrade(
                Fidelity::Bounded,
                AssumptionKind::BudgetHit,
                span,
                &format!(
                    "lazy materialization stopped at `{what}`: the linked structure is \
                     deeper than max_depth ({})",
                    self.lazy.max_depth
                ),
            );
            let p = chiero_mem::Pointer { base: obj, off: 0 };
            s.remember_provenance(t, p);
            return Value::Ptr(p);
        }
        let o = s
            .mem
            .alloc(chiero_mem::ObjKind::Lazy, self.lazy.scalar_extent, 16, span);
        // 021 §6: contents are symbolic and *initialized* — a caller-supplied structure is
        // unknown, not uninitialized, and conflating them is an uninitialized-read storm.
        s.mem.mark_lazy(o);
        s.lazy_depth.insert(o, depth);
        let p = chiero_mem::Pointer { base: o, off: 0 };
        s.remember_provenance(t, p);
        Value::Ptr(p)
    }

    /// The field an address's `AccessPath` names, if the function carries one.
    ///
    /// This is `AccessPath`'s first real consumer: 021 contract 19 asks the note to name
    /// the field that was cut, and the offset alone (`+8`) does not say `next`.
    fn field_name_of(&self, s: &State, addr: &Operand) -> Option<String> {
        let Operand::Value(v) = addr else { return None };
        let f = self.module.funcs.iter().find(|f| f.id == s.func())?;
        let p = f.access_paths.get(v)?;
        p.steps.iter().rev().find_map(|st| match st {
            chiero_cir::PathStep::Field { name, .. }
            | chiero_cir::PathStep::UnionMember { name, .. }
            | chiero_cir::PathStep::Bits { name, .. } => Some(name.to_string()),
            _ => None,
        })
    }

    fn resolve_symbolic_base(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        addr: Term,
        span: Span,
    ) -> Option<Value> {
        // 021 §5.2 comes first: an arena address is *not* a search over the address
        // space, and running §5.1 on it would either concretize it (step 5) or end the
        // path (step 4) before the arena was consulted.
        if let Some(v) = self.resolve_in_arena(a, s, addr, span) {
            return v;
        }
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
                requires: Vec::new(),
                witness: None,
                related: None,
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
            sib.constrain_checked(c);
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
        s.constrain_checked(c);
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
            // A **full** check — no query, so §6.2's slicing has nothing to slice and
            // §6.1's "a single full check that returns `Sat` clears it" would apply. The
            // clearing is not done here because `pinned_offset` holds `&State`; see the
            // note on `State::path_unchecked` for why leaving the flag set is the slow
            // direction rather than the wrong one.
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
    /// The address a `Value` denotes, for a use that is not a dereference.
    ///
    /// Split out because two callers need it — `cmp_operand` and the `Store` handler — and
    /// the first draft implemented it in one and refused in the other, which regressed two of
    /// wave 195's properties at once. One implementation, so they cannot disagree.
    fn address_of_value(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        v: Value,
        span: Span,
    ) -> Option<Term> {
        match v {
            Value::Ptr(p) => self.address_term(a, s, p, span),
            Value::SymPtr { base, off } => {
                let b = self.address_term(a, s, Pointer { base, off: 0 }, span)?;
                Some(a.add(b, off))
            }
            Value::Scalar(t) => Some(t),
            Value::Undef => None,
        }
    }

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
    fn global_object(&mut self, a: &mut TermArena, s: &mut State, g: GlobalId) -> ObjectId {
        // **Per state, not per engine.** The object lives in `s.mem`, which forking clones;
        // the cache used to live on the `Engine`, which forking does not. So a sibling
        // forked *before* a global was first touched got a cache hit for an `ObjectId` its
        // own memory had never allocated, and every access through it reported
        // `wild-pointer: … matching no known object`. `globals.c` hit it because
        // `lookup(i)` forks on a symbolic index and `cfg` is first read afterwards — which
        // is why no unforked fixture could reproduce it.
        if let Some(o) = s.global_objs.get(&g) {
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
        // **Cached before the initializer runs**, not after. An `Addr` initializer resolves
        // its target through this same function, and `struct { void *p; } a = { &a };` is
        // legal C — without the entry here that recurses until the stack ends.
        s.global_objs.insert(g, o);
        // **The initializer is written before the object is read-only.** C gives static
        // storage a defined initial value — zero unless stated — and until 020 carried
        // `GlobalInit` there was nothing to write, so every string literal's bytes read
        // as uninitialized and `s[0]` was a finding rather than `'h'`. Writing after
        // `set_readonly` would fault on the object's own initialization.
        match decl.as_ref().map(|d| &d.init) {
            Some(chiero_cir::GlobalInit::Bytes(b)) => {
                let _ = s.mem.write_bytewise(
                    chiero_mem::Pointer { base: o, off: 0 },
                    &b[..b.len().min(size as usize)],
                    span,
                );
            }
            // C11 6.7.9p10: static storage with no initializer is zero.
            Some(chiero_cir::GlobalInit::Zero) => {
                let zeros = vec![0u8; size as usize];
                let _ = s
                    .mem
                    .write_bytewise(chiero_mem::Pointer { base: o, off: 0 }, &zeros, span);
            }
            // **An address, written as one.** `address_term` is the same function a
            // `Value::Ptr` store goes through, so a pointer that starts life in a global
            // and one assigned at run time are the same value by construction — and the
            // object travels with it, which no byte pattern could carry.
            // The same treatment as `Addr`, through the same `func_object` a `FuncAddr`
            // operand uses — so a function pointer that starts life in a global and one
            // assigned at run time are the same value, and an indirect call through either
            // resolves to the same function.
            // **Bytes first, then each address patched over them.** The order matters:
            // `write_bytewise` would otherwise overwrite a term already placed, and a
            // pointer that lost its provenance reads back as an integer with no object.
            Some(chiero_cir::GlobalInit::Relocated { bytes, relocs }) => {
                let (bytes, relocs) = (bytes.clone(), relocs.clone());
                s.mem
                    .write_bytewise(chiero_mem::Pointer { base: o, off: 0 }, &bytes, span);
                for r in &relocs {
                    let base = match r.target {
                        chiero_cir::RelocTarget::Global(g) => self.global_object(a, s, g),
                        chiero_cir::RelocTarget::Func(f) => self.func_object(s, f),
                    };
                    let p = chiero_mem::Pointer {
                        base,
                        off: r.addend,
                    };
                    // Per relocation, not once for the object: `address_term` records the
                    // term's provenance, and each slot in the aggregate names a different
                    // object.
                    if let Some(t) = self.address_term(a, s, p, span) {
                        let _ = s.mem.write_term(
                            a,
                            chiero_mem::Pointer {
                                base: o,
                                off: r.off as i64,
                            },
                            t,
                            8,
                            chiero_mem::Endian::Little,
                            span,
                        );
                    }
                }
            }
            Some(chiero_cir::GlobalInit::FuncAddr(target)) => {
                let base = self.func_object(s, *target);
                let p = chiero_mem::Pointer { base, off: 0 };
                if let Some(t) = self.address_term(a, s, p, span) {
                    let _ = s.mem.write_term(
                        a,
                        chiero_mem::Pointer { base: o, off: 0 },
                        t,
                        8,
                        chiero_mem::Endian::Little,
                        span,
                    );
                }
            }
            Some(chiero_cir::GlobalInit::Addr { g: target, off }) => {
                let (target, off) = (*target, *off);
                let base = self.global_object(a, s, target);
                let p = chiero_mem::Pointer { base, off };
                if let Some(t) = self.address_term(a, s, p, span) {
                    // `address_term` has already recorded the term's provenance, and
                    // provenance is keyed on the *term* rather than on the location — so a
                    // reload of these bytes yields the same term and resolves to the same
                    // object, with no address-range search and no fidelity degrade.
                    let _ = s.mem.write_term(
                        a,
                        chiero_mem::Pointer { base: o, off: 0 },
                        t,
                        size,
                        Endian::Little,
                        span,
                    );
                }
            }
            // `Extern` is defined in another TU: the bytes are genuinely unknown here,
            // and leaving them uninitialized is the honest answer rather than a zero
            // chiero invented.
            Some(chiero_cir::GlobalInit::Extern) | None => {}
        }
        if is_const {
            s.mem.set_readonly(o);
        }
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
            // **A `Bitcast` to a vector re-divides the same bits** (020 contract 22).
            // That is the whole point of the instruction — `u32x4` and `u8x16` are one
            // 128-bit value cut two ways — so the lane width comes from the *destination*
            // type and not from the operand. Without this the shape is lost at the cast
            // and every `ExtractLane` after it is a lowering gap, which is a silent
            // `Unknown` rather than a wrong answer, but silent all the same: the byte
            // view of a union simply stops working and nothing says why.
            RValue::Cast {
                kind: CastKind::Bitcast,
                to: CTy::Vector { elem, .. },
                ..
            } => elem.bit_width(),
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
    /// What a reader calls the object `o`: a local's declared name, a global's, or `None`
    /// when chiero invented the object and there is nothing to call it.
    /// A span as somewhere a reader can open: `path:line:col`.
    ///
    /// The form every compiler and editor already understands, which is what 023 §9's "a report a
    /// person can act on" means in practice. Falls back to what `chiero-mem` would have said on
    /// its own when there is no map or the span resolves to no file — saying less, never guessing.
    fn render_loc(&self, sp: Span) -> String {
        let Some(loc) = self.source_map.and_then(|m| m.lookup_loc(sp.lo)) else {
            return format!("source offset {}", sp.lo.0);
        };
        let file = self
            .source_map
            .expect("resolved above")
            .file(loc.file)
            .path();
        format!("{}:{}:{}", file.display(), loc.line, loc.col)
    }

    /// Append where this finding *is*, when the engine can name it.
    ///
    /// The access location is already on the finding as a `Span`, and `reports()` hands it over
    /// for a caller that renders locations itself. This puts it in the sentence too, because
    /// `findings()` is the projection most consumers read and a report whose text names the
    /// *free*'s line and not its own is worse than one that names neither.
    ///
    /// **Appended, not prefixed.** `path:line:col:` first is the compiler's convention and would
    /// be the better rendering in isolation; the kind has to lead here, because 023 §6.1 makes it
    /// half the dedup key and every consumer in this repo matches on it.
    ///
    /// An append rather than a substitution, which is why it survived wave 211's refactor: it adds
    /// a clause instead of rewriting one.
    fn stamp(&self, at: Span, message: String) -> String {
        let Some(loc) = self.source_map.and_then(|m| m.lookup_loc(at.lo)) else {
            return message;
        };
        let file = self
            .source_map
            .expect("resolved above")
            .file(loc.file)
            .path();
        format!("{message} (at {}:{}:{})", file.display(), loc.line, loc.col)
    }

    /// How to refer to an object in a report: its name if it has one, a description if not.
    ///
    /// 023 §9 asks for a report a person can act on, and every fault message names the object it
    /// is about. A variable name is the best answer and `object_name` gives it — but anonymous
    /// objects are ordinary, not exceptional: every `malloc` makes one, and it is exactly the
    /// memory whose faults matter most.
    ///
    /// What is left is what `chiero-mem` knows: which kind of storage it is and how big. "the
    /// 4-byte heap allocation" does not say *which* `malloc` — the finding's own span says where
    /// the access is, and the free site is on the fault — but it tells a reader what they are
    /// looking for, which the counter never did.
    fn object_desc(&self, s: &State, o: chiero_mem::ObjectId) -> String {
        if let Some(name) = self.object_name(s, o) {
            return name;
        }
        let size = s.mem.size_of_pub(o);
        // **Kind first, size second.** A reader who is told "heap" already knows to look at
        // allocations; the size narrows it among them. Reversing that reads as a fact about
        // bytes rather than about storage.
        let what = match s.mem.kind_of(o) {
            Some(chiero_mem::ObjKind::Heap) => "heap allocation",
            Some(chiero_mem::ObjKind::Stack) => "unnamed local",
            Some(chiero_mem::ObjKind::Global) => "unnamed global",
            Some(chiero_mem::ObjKind::Extern) => "object defined outside this translation unit",
            Some(chiero_mem::ObjKind::Function) => "function",
            // 021 §6: memory chiero invented on first dereference because the caller's
            // structure had to point somewhere. Saying so is the honest description — a reader
            // told "heap allocation" would go looking for a `malloc` that is not there.
            Some(chiero_mem::ObjKind::Lazy) => "object reached through an unconstrained pointer",
            Some(chiero_mem::ObjKind::VarArgs) => "variadic argument area",
            // No entry at all. Saying so beats inventing a kind, and it is the one case where
            // there is genuinely nothing to describe.
            None => return "an object this run no longer has".to_string(),
        };
        match size {
            Some(n) => format!("the {n}-byte {what}"),
            None => format!("the {what}"),
        }
    }

    fn object_name(&self, s: &State, o: chiero_mem::ObjectId) -> Option<String> {
        if let Some(g) = s.global_objs.iter().find(|(_, id)| **id == o) {
            return self
                .module
                .globals
                .iter()
                .find(|x| x.id == *g.0)
                .map(|x| x.name.to_string());
        }
        // The *current* frame only: an `AllocaId` is unique within a function, so a
        // caller's slot of the same id is a different object and naming this fault after
        // it would be worse than not naming it at all.
        let fr = s.stack.last()?;
        let a = fr.frame_objs.iter().find(|(_, id)| **id == o)?.0;
        let f = self.module.funcs.iter().find(|f| f.id == s.func())?;
        f.allocas
            .iter()
            .find(|d| d.id == *a)?
            .name
            .as_ref()
            .map(|n| n.to_string())
    }

    /// The rendered `AccessPath` for the address the current instruction accesses.
    ///
    /// Looked up from the instruction at the program counter rather than threaded through
    /// the memory model: 020 §4.4 says no analysis may branch on a path, and handing one
    /// to `chiero-mem` would put it exactly where an analysis would find it.
    fn path_for_current_access(&self, s: &State) -> Option<String> {
        let f = self.module.funcs.iter().find(|f| f.id == s.func())?;
        if f.access_paths.is_empty() {
            return None;
        }
        let (bid, ix) = s.pc;
        let b = f.blocks.iter().find(|b| b.id == bid)?;
        let addr = match &b.insts.get(ix)?.kind {
            chiero_cir::InstKind::Assign {
                rv:
                    chiero_cir::RValue::Load { addr, .. } | chiero_cir::RValue::LoadBits { addr, .. },
                ..
            }
            | chiero_cir::InstKind::Store { addr, .. }
            | chiero_cir::InstKind::StoreBits { addr, .. } => addr,
            _ => return None,
        };
        let Operand::Value(v) = addr else { return None };
        f.access_paths.get(v).map(|p| p.render())
    }

    /// Report what survives discharge, **and hand the survivors back**.
    ///
    /// The return value is the point of wave 249. Discharging a `maybe` costs up to three solver
    /// queries per fault, and the result used to be consumed here and dropped — so a caller that
    /// also needed to know whether the *value* was usable consulted the raw list instead, and
    /// discarded values the engine had just proved were fine. One fault list decided two things and
    /// only one of them saw the proof.
    ///
    /// Most callers ignore the return and only want the reporting; the two that decide a value's
    /// usability are the scalar and bit-field loads.
    /// Where the current function compares `vid` against a null pointer, if it does.
    ///
    /// **Only a comparison against a null *constant*.** `p == q` for two pointers says nothing
    /// about either being null, and reporting it as a null test would put a false claim in a
    /// finding — worse than the vaguer sentence it replaces, because a reader would go and look.
    ///
    /// The whole function is searched rather than the blocks already executed: the shape this
    /// exists for is a check *below* the dereference, which by construction has not run. Searching
    /// forward from the fault would find nothing at all.
    fn null_test_of(&self, s: &State, vid: ValueId) -> Option<Span> {
        let f = self.module.funcs.iter().find(|f| f.id == s.func())?;
        let insts = || f.blocks.iter().flat_map(|b| b.insts.iter());

        // **The comparison is not on the parameter, and that is the whole difficulty.** Lowering
        // stores every parameter into a slot at entry, so `if (p)` reads the slot back and compares
        // *that*: `%7 = addrlocal %0`, `%8 = load ptr %7`, `%9 = cmp ne ptr %8, null`. Matching the
        // parameter's own `ValueId` finds nothing — the first version of this did exactly that and
        // the fixture stayed red.
        //
        // So: find the slot the parameter was stored into, then the loads from it, then a
        // comparison of one of those against null.
        let addr_of = |v: ValueId| {
            insts().find_map(|i| match &i.kind {
                InstKind::Assign {
                    dst,
                    rv: RValue::AddrOfLocal { alloca },
                } if *dst == v => Some(*alloca),
                _ => None,
            })
        };
        let slot = insts().find_map(|i| match &i.kind {
            InstKind::Store {
                addr: Operand::Value(a),
                val: Operand::Value(v),
                ..
            } if *v == vid => addr_of(*a),
            _ => None,
        })?;
        let loaded: Vec<ValueId> = insts()
            .filter_map(|i| match &i.kind {
                InstKind::Assign {
                    dst,
                    rv:
                        RValue::Load {
                            addr: Operand::Value(a),
                            ..
                        },
                } if addr_of(*a) == Some(slot) => Some(*dst),
                _ => None,
            })
            .collect();

        // **A null *constant* only.** `p == q` for two pointers says nothing about either being
        // null, and calling it a null test would put a false claim in a finding — worse than the
        // vaguer sentence it replaces, because a reader would go and look at the line.
        // **A null pointer constant reaches the comparison as a *value*, not an operand.** `if (0
        // == p)` lowers to `%7 = inttoptr i32 0i32 to ptr` and then `cmp eq ptr %7, %9` — C11
        // 6.3.2.3p3 makes the conversion explicit and CIR keeps it, so a matcher looking only at
        // `Operand::Const` finds the bare `null` form and misses this one. Both spellings are the
        // same source construct and a reader would be baffled to see one cited and not the other.
        let is_null = |o: &Operand| match o {
            Operand::Const(Const::Null) => true,
            Operand::Const(Const::Int { val: 0, .. }) => true,
            Operand::Value(v) => insts().any(|i| {
                matches!(
                    &i.kind,
                    InstKind::Assign {
                        dst,
                        rv: RValue::Cast {
                            kind: chiero_cir::CastKind::IntToPtr,
                            a: Operand::Const(Const::Int { val: 0, .. }),
                            ..
                        },
                    } if dst == v
                )
            }),
            _ => false,
        };
        let is_it = |o: &Operand| matches!(o, Operand::Value(v) if loaded.contains(v));

        // The *whole* function, not the blocks already executed: the shape this exists for is a
        // check below the dereference, which by construction has not run.
        insts().find_map(|i| {
            let InstKind::Assign {
                rv: RValue::Cmp { a, b, .. },
                ..
            } = &i.kind
            else {
                return None;
            };
            ((is_it(a) && is_null(b)) || (is_null(a) && is_it(b))).then_some(i.span)
        })
    }

    fn report_faults(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        faults: &[chiero_mem::MemFault],
        span: Span,
    ) -> Vec<chiero_mem::MemFault> {
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
        // **021 §3.1's guard is discharged here, which is what the variant's comment always
        // said was the engine's job.** Before wave 204 nothing did it, so every conditional
        // write ended as `maybe` whether or not the path decided it — and a `maybe` on memory
        // the path proves untouched understates a real bug, while one the path proves written
        // is a false report a reader has to dismiss.
        //
        // Wave 156's three outcomes, for the fourth time in this engine: the guard `t` under
        // path condition `P`.
        //
        // - `P ∧ ¬t` unsatisfiable — `t` holds on every model of the path, so the byte *was*
        //   written. Drop the fault.
        // - `P ∧ t` unsatisfiable — `t` cannot hold here, so the byte was certainly not
        //   written. Promote to the definite `Uninitialized`.
        // - anything else, including `Unknown` — genuinely undecided, and the `maybe` stands.
        //   Treating `Unknown` as either certainty is the collapse the third state exists to
        //   prevent.
        let faults: Vec<chiero_mem::MemFault> = faults
            .into_iter()
            .filter_map(|f| match f {
                chiero_mem::MemFault::MaybeUninitialized {
                    obj,
                    off,
                    bit,
                    guard: Some(t),
                    at,
                } => {
                    let neg = a.not(t);
                    if matches!(self.probe(a, s, &[neg]), CheckResult::Unsat) {
                        None
                    } else if matches!(self.probe(a, s, &[t]), CheckResult::Unsat) {
                        Some(chiero_mem::MemFault::Uninitialized { obj, off, bit, at })
                    } else {
                        Some(chiero_mem::MemFault::MaybeUninitialized {
                            obj,
                            off,
                            bit,
                            guard: Some(t),
                            at,
                        })
                    }
                }
                // The same three outcomes, for a read whose *offset* is symbolic too. What
                // differs is that there is no offset to put in the report until a model
                // supplies one, so the `Sat` case is not merely "undecided" — it is where the
                // witness comes from. 023 §9: a report a person cannot act on is not a report,
                // and "some value of `i`" is not actionable.
                chiero_mem::MemFault::UninitializedSymbolic {
                    obj,
                    off,
                    guard,
                    at,
                } => {
                    let neg = a.not(guard);
                    match self.probe(a, s, &[neg]) {
                        // Every bit the read touches was written on every model of the path.
                        CheckResult::Unsat => None,
                        CheckResult::Sat(m) => {
                            // An offset the path allows at which the read is *not* fully
                            // initialized — so it is a witness for whichever of the two
                            // verdicts follows, and both want the same one.
                            let w = a.eval(&m, off).map_or(0, |c| c.bits() as i64);
                            if matches!(self.probe(a, s, &[guard]), CheckResult::Unsat) {
                                Some(chiero_mem::MemFault::Uninitialized {
                                    obj,
                                    off: w,
                                    bit: w as u64 * 8,
                                    at,
                                })
                            } else {
                                Some(chiero_mem::MemFault::MaybeUninitialized {
                                    obj,
                                    off: w,
                                    bit: w as u64 * 8,
                                    guard: Some(guard),
                                    at,
                                })
                            }
                        }
                        // **The one report `UninitializedSymbolic` makes for itself.** No model
                        // means no witness, and the variant's own message says exactly that
                        // rather than naming an offset nobody proved.
                        CheckResult::Unknown(_) => {
                            Some(chiero_mem::MemFault::UninitializedSymbolic {
                                obj,
                                off,
                                guard,
                                at,
                            })
                        }
                    }
                }
                other => Some(other),
            })
            .collect();
        for f in &faults {
            self.finding_seq += 1;
            let key = FindingKey {
                kind: f.kind(),
                span: f.at(),
                object: f.object(),
                func: s.func(),
            };
            // **The access path, if the function carries one for this address.**
            // 020 §4.4: a finding should read `opaque as l2_bridge_t.bd_index`, not
            // "offset 36 of ObjectId(1)" — dozens of VPP nodes reinterpret that region and
            // the offset alone does not say which one is wrong. Reporting-only, so a
            // missing path costs the message some words and nothing else.
            // **The fault composes its own sentence from what only the engine knows.**
            //
            // `chiero-mem` has no module and no `SourceMap`, so it can name neither the variable
            // nor the line; the engine has both and passes them in. Until wave 211 this was two
            // `.replace()` calls over the finished sentence — sound, because each rebuilt its own
            // token from the fault, and four layers deep with no argument for why the fifth would
            // be. The id is an allocation counter: it means nothing to a reader and is not stable
            // across pass configurations, so the *same defect in the same program* printed
            // differently with `mem2reg` on, and `chiero-opt`'s transparency sweep normalized it
            // away for eight waves to keep working.
            let described = f.describe(&|o| self.object_desc(s, o), &|sp| self.render_loc(sp));
            let message = match self.path_for_current_access(s) {
                Some(p) => format!("{described} through {p}"),
                None => described,
            };
            // **A null dereference names the assumption it rests on, when it rests on one.**
            //
            // Only on this state's own null parameter, and only for a null fault: a null the
            // *program* produced — a failed `malloc`, a lookup that missed — must not claim
            // it, because the two want opposite responses. One says "your caller can do
            // this"; the other says "your code does this".
            let message = match (&s.entry_null_param, f.kind()) {
                (Some((vid, p)), "null-dereference") => {
                    let base = format!(
                        "{message}, where {p} is a pointer parameter assumed to be possibly null"
                    );
                    // **And when the program tests it, say so instead of assuming.**
                    //
                    // "Assumed" is the weakest thing chiero can offer, and it invites the reply
                    // that these callers never pass null. A test in the function's own body is the
                    // author stating the pointer can be — evidence a reader cannot argue with, and
                    // it is already sitting in the CIR. 023 §9's rule about a report a person
                    // cannot act on, one step on: a report a person can *dismiss* is barely one.
                    match self.null_test_of(s, *vid) {
                        Some(at) => format!(
                            "{base}; the function tests it against null at {}",
                            self.render_loc(at)
                        ),
                        None => base,
                    }
                }
                _ => message,
            };
            // Last, so the location ends the sentence rather than interrupting a clause that
            // still has something to say.
            let message = self.stamp(f.at(), message);
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key: Some(key),
                message,
                span: f.at(),
                requires: Vec::new(),
                witness: None,
                // The same second place the sentence names, as data. One source for both, so
                // the prose and the field cannot disagree about which event it is.
                related: f.secondary(),
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
        faults
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
        // ⚠️ The `Uninitialized` arm is **unreachable today** and mutation says so: a
        // `ModelEntry` carries only a name and a precision, so the behaviour lives in this
        // crate's dispatch, and no built-in model returns that fill. 024 §2.1 makes the
        // choice a model's to declare, and the memory side of it is pinned directly
        // (`an_uninitialized_havoc_does_produce_a_finding`) — what is not pinned is this
        // one-line translation, and it will not be until a model can declare a havoc.
        // Recorded rather than left as an unexplained survivor.
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
            "chiero_assume"
                | "chiero_assert"
                | "chiero_mark_fidelity"
                | "chiero_make_symbolic"
                | "chiero_is_symbolic"
        ) {
            self.intrinsic(a, s, name, dst, args, span);
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
        /// A model fork's branches: the guard, the value, and a report the branch *is*.
        type Branch = (Option<Term>, Option<Value>, Option<BranchNote>);
        let mut forks: Vec<Branch> = Vec::new();
        let mut mine_guard: Option<Term> = None;
        let mut branch_findings: Vec<String> = Vec::new();
        let mut branch_bounds: Vec<String> = Vec::new();
        let mut keyed: Vec<(Option<chiero_mem::MemFault>, String)> = Vec::new();
        let mut result: Option<Value> = None;
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
                    // The concrete walk's reports are **superseded** when the symbolic
                    // scan re-does the same work: both describe the same defect in their
                    // own words, and the caller saw a negative offset reported twice.
                    // Found by review.
                    let mark = cx.report_mark();
                    let r = models::strlen(&mut cx, p, strp);
                    match r {
                        chiero_model::StrScan::Exact(n) => {
                            let t = cx.arena().bv(64, n as u128);
                            chiero_model::ModelOutcome::Value(Some(chiero_model::Value::Scalar(t)))
                        }
                        // **The symbolic case is a fork, not a shrug** (024 §4 step 2).
                        // The concrete walk stops at the first byte it cannot read as a
                        // number; that is where the interesting strings start, not where
                        // the analysis should.
                        //
                        // At *any* `scanned`, not only zero: gating on `scanned == 0`
                        // meant a single concrete byte before the symbolic one — `buf[0]`
                        // assigned, the rest from the caller — disabled the fork entirely,
                        // and chiero found neither the length nor the overrun. §4 step 1
                        // walks the concrete prefix and step 2 forks at the first byte
                        // that *may* be zero; the prefix is not a reason to stop. Found by
                        // review.
                        chiero_model::StrScan::CapReached { .. } => {
                            cx.drop_reports_after(mark);
                            models::strlen_symbolic(&mut cx, p, strp)
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
                "printf" => {
                    // The two `Value` types are deliberately separate (023 §1.1 keeps the
                    // engine's out of the model API); `Undef` has no model counterpart and
                    // becomes "not an argument this can classify".
                    let vs: Vec<Option<chiero_model::Value>> = resolved
                        .iter()
                        .map(|v| match v {
                            Some(Value::Scalar(t)) => Some(chiero_model::Value::Scalar(*t)),
                            Some(Value::Ptr(p)) => Some(chiero_model::Value::Ptr(*p)),
                            _ => None,
                        })
                        .collect();
                    Some(models::printf(&mut cx, &vs))
                }
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
                    // **The guard travels with the branch.** It was dropped — every
                    // sibling of a model fork carried the *same* path condition, so a
                    // `strlen` fork produced four states that all still believed the
                    // string could be any length, and a later `if (len == 2)` took both
                    // sides in every one of them. `malloc`'s two branches were
                    // indistinguishable to the solver for the same reason.
                    let mut it = branches.into_iter();
                    match it.next() {
                        Some((g, ModelOutcome::Value(v))) => {
                            result = v.map(lift_value);
                            mine_guard = g;
                        }
                        Some((g, ModelOutcome::Bounded(why))) => {
                            mine_guard = g;
                            branch_bounds.push(why);
                        }
                        Some((g, ModelOutcome::Finding(msg))) => {
                            // A branch that *is* the report — 024 §4 step 4's
                            // unterminated string. It constrains this state like any
                            // other branch and reports; it is not a lowering gap.
                            mine_guard = g;
                            branch_findings.push(msg);
                        }
                        _ => translated = false,
                    }
                    for (g, alt) in it {
                        match alt {
                            ModelOutcome::Value(v) => forks.push((g, v.map(lift_value), None)),
                            ModelOutcome::Finding(msg) => {
                                forks.push((g, None, Some(BranchNote::Finding(msg))))
                            }
                            ModelOutcome::Bounded(why) => {
                                forks.push((g, None, Some(BranchNote::Bounded(why))))
                            }
                            _ => translated = false,
                        }
                    }
                }
                // The payload is the *whole point* of `Finding`; matching only `Value`
                // dropped it. It is still a gap — the call did not produce a value — so
                // `translated` stays false and the assumption is recorded too.
                Some(ModelOutcome::Bounded(why)) => {
                    // Deferred: `cx` holds `&mut s.mem` for the whole dispatch, so the
                    // degradation is applied after it is dropped, like every other arm.
                    branch_bounds.push(why);
                    translated = false;
                }
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
                requires: Vec::new(),
                witness: None,
                related: None,
            });
        }
        for (fault, text) in keyed {
            self.finding_seq += 1;
            // **Re-described from the fault rather than patched.** `ModelRegistry::lift` renders
            // with `to_string`, because `chiero-model` knows no more than `chiero-mem` does — but
            // it keeps the fault beside the text, so the engine can compose the sentence properly
            // instead of rewriting the one it was handed. A report with no fault behind it is a
            // checker's own words and stays exactly as written.
            let text = match fault.as_ref() {
                Some(f) => f.describe(&|o| self.object_desc(s, o), &|sp| self.render_loc(sp)),
                None => text,
            };
            let at_span = fault.as_ref().map_or(span, chiero_mem::MemFault::at);
            let text = self.stamp(at_span, text);
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
                requires: Vec::new(),
                witness: None,
                // The model and checker route. Wave 207's rule: when a fix is about a fault's
                // rendering, ask what else builds a finding from one.
                related: fault.as_ref().and_then(chiero_mem::MemFault::secondary),
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
        if let Some(g) = mine_guard {
            // 022 §6.1's third site: 024 §4's `strlen` cap "constrains a terminator to
            // exist within the bound — infeasible if those bytes are already constrained
            // non-zero". The guard is added without checking that.
            s.constrain_unchecked(g);
        }
        let took_bound = !branch_bounds.is_empty();
        let took_finding = !branch_findings.is_empty();
        // **This state's own report is applied after the siblings are cloned.** The
        // guard is undone per sibling by `pop_if`, but a finding and a degradation are
        // not — so a first branch carrying either leaked it onto every sibling, including
        // the ones whose guards contradict it. The same class of defect this arm fixed for
        // guards and values, left in place for reports. Found by review; unreachable
        // today, which is why it survived mutation, and wrong the moment it is reached.
        //
        // **A value-less branch cannot continue.** 024 §4 step 3 says reaching the scan
        // cap "terminates the state with `Bounded`", and the reason is mechanical: the
        // call's destination is unbound, so the *first use* of the result reached
        // `branch condition is not a scalar` — 023 §3's marker for "a bug in chiero" —
        // pinning the run at `Unknown` instead of the `Bounded` the spec asks for. A
        // reported fault ends the path for the same reason: `strlen` running off the end
        // has no length to hand back, and continuing with nothing invents one later.
        // Found by review.
        if result.is_none() && dst.is_some() {
            if took_bound {
                s.status = Status::Terminated(TermReason::Budget);
            } else if took_finding {
                s.status = Status::Terminated(TermReason::Crashed);
            }
        }
        for (guard, v, report) in forks {
            let mut sib = s.clone();
            sib.id = self.new_id();
            // **This state's own guard is not the sibling's.** Cloning after pushing it
            // would put the first branch's constraint on every other branch, which is
            // worse than no guard at all: the states would be mutually contradictory.
            if let Some(g) = mine_guard {
                sib.path.pop_if(|last| *last == g);
            }
            if let Some(g) = guard {
                sib.constrain_unchecked(g);
            }
            match report {
                Some(BranchNote::Finding(msg)) => {
                    self.finding_seq += 1;
                    sib.findings.push(StateFinding {
                        id: self.finding_seq,
                        key: None,
                        message: msg,
                        span,
                        requires: Vec::new(),
                        witness: None,
                        related: None,
                    });
                    if dst.is_some() {
                        sib.status = Status::Terminated(TermReason::Crashed);
                    }
                }
                Some(BranchNote::Bounded(why)) => {
                    sib.degrade(Fidelity::Bounded, AssumptionKind::BudgetHit, span, &why);
                    if dst.is_some() {
                        sib.status = Status::Terminated(TermReason::Budget);
                    }
                }
                None => {}
            }
            match (dst, v) {
                (Some(d), Some(x)) => sib.set_local(d, x),
                // **A branch with no value must not inherit one.** The sibling is cloned
                // after this state's result is in place, so a value-less branch — the
                // unterminated string, the scan cap — kept the *first* branch's length
                // and reported it as its own.
                (Some(d), None) => sib.clear_local(d),
                _ => {}
            }
            // Past the call, for the same reason as `indirect`: a sibling that still
            // pointed *at* the call re-dispatched it and forked again, forever.
            sib.pc.1 = sib.pc.1.wrapping_add(1);
            self.pending.push(sib);
        }
        for why in branch_bounds {
            s.degrade(Fidelity::Bounded, AssumptionKind::BudgetHit, span, &why);
        }
        for msg in branch_findings {
            self.finding_seq += 1;
            s.findings.push(StateFinding {
                id: self.finding_seq,
                key: None,
                message: msg,
                span,
                requires: Vec::new(),
                witness: None,
                related: None,
            });
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

    /// 024 §7. The harness intrinsics, handled apart from the models that take memory.
    fn intrinsic(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        name: &str,
        dst: Option<ValueId>,
        args: &[Operand],
        span: Span,
    ) {
        use chiero_model::{IntrinsicOutcome, intrinsics};

        // **The two that introduce and inspect symbolism**, handled first because neither
        // is about a condition. Without a model for `chiero_make_symbolic` the call is an
        // unmodeled extern and *nothing becomes symbolic*: every corpus program is
        // explored along one concrete path with every assertion holding, which is a whole
        // suite reporting success over a symbolic execution that never happened.
        if name == "chiero_make_symbolic" {
            let (Some(Value::Ptr(p)), Some(n)) = (
                args.first().and_then(|o| self.operand(a, s, o)),
                args.get(1).and_then(|o| self.concrete_size(a, s, o)),
            ) else {
                // A symbolic length is 023 §10's territory: guessing one would symbolize
                // the wrong extent, which is worse than declining.
                self.note_once(
                    s,
                    AssumptionKind::OpaqueCode,
                    span,
                    "`chiero_make_symbolic` with a non-pointer target or symbolic length \
                     was not applied",
                );
                s.degrade(
                    Fidelity::Unknown,
                    AssumptionKind::NoInformation,
                    span,
                    "a harness asked for symbolic bytes and did not get them",
                );
                return;
            };
            // The harness's own name reaches the witness, which is the whole point of the
            // third parameter: a binding called `x` is what a reader can act on.
            let label = args
                .get(2)
                .and_then(|o| match self.operand(a, s, o) {
                    Some(Value::Ptr(q)) => s.mem.c_string_at(q),
                    _ => None,
                })
                .unwrap_or_else(|| format!("sym{}", self.fresh_count + 1));
            for i in 0..n {
                self.fresh_count += 1;
                let t = self.input(
                    a,
                    s,
                    chiero_solver::Sort::BitVec(8),
                    &format!("{label}#{i}"),
                    InputOrigin::Param {
                        index: i as usize,
                        name: label.clone(),
                        span,
                    },
                );
                let at = chiero_mem::Pointer {
                    base: p.base,
                    off: p.off + i as i64,
                };
                let r = s
                    .mem
                    .write_term(a, at, t, 1, chiero_mem::Endian::Little, span);
                self.report_faults(a, s, &r.faults, span);
            }
            return;
        }
        if name == "chiero_is_symbolic" {
            // **Introspection, and honest about being introspection.** The answer is a
            // fact about chiero's own representation, not about the program, so it is
            // decided here rather than by the solver: a term the arena can fold is
            // concrete, anything else is not.
            if let Some(d) = dst {
                let sym = match args.first().and_then(|o| self.operand(a, s, o)) {
                    Some(Value::Scalar(t)) => a.eval_ground(t).is_err(),
                    // A pointer is a tracked object, not a symbol the harness introduced.
                    Some(Value::Ptr(_)) => false,
                    _ => false,
                };
                let w = bits_of_cty(&CTy::Int(32)).expect("int has a width");
                let v = a.bv(w, u128::from(sym));
                s.set_local(d, Value::Scalar(v));
            }
            return;
        }
        // `None` means the condition could not be decided here, which the two intrinsics
        // treat differently on purpose: `assume` constrains, `assert` reports. Hardcoding
        // "true" was the safe reading for `assume` and the *unsafe* one for `assert` —
        // every assertion in a harness passed.
        // **The term, not only its truth value.** `assume` on a symbolic condition
        // returns `Constrain`, and constraining needs something to add to the path.
        let cond_term = match args.first().map(|o| self.operand(a, s, o)) {
            Some(Some(Value::Scalar(t))) => Some(t),
            _ => None,
        };
        let cond = cond_term.and_then(|t| a.eval_ground(t).ok().map(|c| c.bits() != 0));
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
            IntrinsicOutcome::Continue => {}
            // **`Constrain` used to fall in with `Continue` and do nothing**, which made
            // `chiero_assume` a no-op for exactly the conditions it exists for: a ground
            // one is decided by `Some(true)`/`Some(false)` above, so `Constrain` is
            // reached only when the condition is *symbolic*. Every harness assumption
            // over a symbolic value was silently discarded — `chiero_assume(i < 8)`
            // followed by `buf[i]` reported an out-of-bounds access on a program that has
            // none, and the reverse, a harness narrowing inputs to reach a bug, explored
            // everything else instead.
            //
            // 022 §6.1 note: this adds a constraint **without a feasibility check**, which
            // is the situation `PathCondition::possibly_infeasible` exists for. `s.path`
            // is a bare `Vec<Term>` today; when it becomes a `PathCondition` this is a
            // fourth call site for `push_unchecked` and belongs on §6.1's list.
            IntrinsicOutcome::Constrain => {
                if let Some(t) = cond_term {
                    // A non-boolean condition is C's "nonzero is true", so the term is
                    // compared against zero rather than asserted as a bit.
                    let w = a.width(t);
                    let zero = a.bv(w, 0);
                    let is_zero = a.eq(t, zero);
                    let nonzero = a.not(is_zero);
                    s.constrain_unchecked(nonzero);
                }
            }
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
                    requires: Vec::new(),
                    witness: None,
                    related: None,
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
        args: &[Operand],
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
            self.direct_into(a, s, id, dst, args, span);
            return;
        }
        // **A null callee is a fault, not a gap.** C11 6.5.2.2p5 requires the operand to
        // point to a function, and calling through a null one crashes exactly as
        // dereferencing a null data pointer does.
        //
        // Reported through `report_faults` rather than as a bespoke finding, so it gets
        // 023 §6.1's deduplication, the span, the access path and — the part that matters —
        // the "path ends at a definite crash" rule. Without that the state would carry on
        // into a function it never reached.
        //
        // Before this it fell through to the candidate list below, which for a null pointer
        // means forking over every function in the module or, when there is nothing to fork
        // over, degrading. A degraded run says "chiero could not follow this", and a reader
        // scanning for findings sees a clean run — the more misleading of the two ways to be
        // wrong about a definite fault.
        if let Some(Value::Ptr(p)) = self.operand(a, s, op)
            && p.base == chiero_mem::ObjectId::NULL
        {
            self.report_faults(
                a,
                s,
                &[chiero_mem::MemFault::NullDeref {
                    off: p.off,
                    at: span,
                }],
                span,
            );
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
            self.direct_into(a, &mut sib, id, dst, args, span);
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
    fn direct_into(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        id: FuncId,
        dst: Option<ValueId>,
        args: &[Operand],
        span: Span,
    ) {
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
        // **Arguments reach the callee here too.** The direct path binds them and this one
        // did not, so every call through a function pointer — how VPP dispatches every
        // graph node — arrived with no parameters at all and the callee read them as
        // uninitialized. The direct path carries a comment recording this exact bug being
        // fixed once; `direct_into` was written afterwards without it.
        let params: Vec<Param> = f.params.clone();
        let mut locals = IndexMap::new();
        for (p, o) in params.iter().zip(args.iter()) {
            if let Some(v) = self.operand(a, s, o) {
                locals.insert(p.value, v);
            }
        }
        let ret_to = Some((s.func(), s.pc.0, s.pc.1 + 1));
        s.stack.push(Frame {
            func: id,
            ret_to,
            ret_dst: dst,
            locals,
            frame_objs,
            ptr_vals: IndexMap::new(),
            bit_inspected: Vec::new(),
            prev_block: None,
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
            // **A float literal is its bits.** `Const::Float` carries the raw pattern so a
            // NaN payload survives a round trip, and `sort_of` already gives F32 32 bits
            // and F64 64 — so the value the engine wants is the one CIR already holds. The
            // 80-bit x87 form has no Rust primitive to compute with and stays a gap; its
            // *width* is representable, which is why the guard is on the kind and not here.
            Operand::Const(Const::Float(k, bits)) => Some(Value::Scalar(a.bv(
                match k {
                    FloatKind::F32 => 32,
                    FloatKind::F64 => 64,
                    FloatKind::X87_80 => 80,
                },
                *bits,
            ))),
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
                let base = self.global_object(a, s, *g);
                Some(Value::Ptr(Pointer { base, off: *off }))
            }
            Operand::Const(Const::FuncAddr(id)) => {
                let base = self.func_object(s, *id);
                Some(Value::Ptr(Pointer { base, off: 0 }))
            }
            // **`Undef` is a value, not a gap** (020 contract 43). Inventing one for it
            // is the opposite of what it means, and so is refusing the operand: a
            // `LoweringGap` says "chiero cannot follow this", where the truth is that the
            // *program* did not say.
            Operand::Const(Const::Undef(_)) => Some(Value::Undef),
            // `Float` and `Wide` remain gaps: 023 §7 approximates floating point.
            _ => None,
        }
    }

    /// A comparison operand as a bit-vector: a scalar as itself, a pointer as its address.
    ///
    /// Only comparisons get this. Arithmetic on a pointer is `PtrAdd` (020: PtrAdd-not-Add)
    /// and must keep its provenance, so silently turning a `Value::Ptr` into an address
    /// there would lose the object a later dereference needs. Equality does not dereference
    /// anything, which is why the address alone is the whole answer here.
    fn cmp_operand(
        &mut self,
        a: &mut TermArena,
        s: &mut State,
        o: &Operand,
        span: Span,
    ) -> Option<Term> {
        match self.operand(a, s, o)? {
            Value::Scalar(t) => Some(t),
            Value::Ptr(p) => self.address_term(a, s, p, span),
            // **A symbolic offset does have an address: the base's, plus the offset.**
            //
            // The first draft returned `None` here, and two of wave 195's properties
            // regressed at once — a stored `SymPtr` left the destination uninitialized, so
            // the invented `uninitialized-read` came back, and `p == q` on two of them
            // answered nothing instead of "they can differ". Both are the same omission:
            // every use that is not a *dereference* only needs the address as a number, and
            // that number is expressible.
            //
            // Provenance still comes from `address_term` on the base, so the sum carries the
            // object exactly as a concrete pointer's address does.
            Value::SymPtr { base, off } => {
                let b = self.address_term(a, s, Pointer { base, off: 0 }, span)?;
                Some(a.add(b, off))
            }
            Value::Undef => None,
        }
    }

    fn scalar(&mut self, a: &mut TermArena, s: &mut State, o: &Operand) -> Option<Term> {
        match self.operand(a, s, o)? {
            Value::Scalar(t) => Some(t),
            // **`Undef` has no term, and must not get one.** A fresh symbol here would be
            // a value the solver may pin, which is exactly what `Undef` says does not
            // exist. Callers that can propagate it check for it before asking (020
            // contract 43); the rest treat `None` as the gap it is.
            Value::Ptr(_) | Value::Undef | Value::SymPtr { .. } => None,
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
        // **A vector has a width** (020 contract 22). Falling through to `None` made
        // every `Bitcast` between two views of one 128-bit union a lowering gap — so the
        // `u8x16` view of a `u32x4` did not produce a wrong answer, it produced no answer,
        // and the run degraded to `Unknown` for a construct the CIR fully specifies.
        CTy::Vector { elem, lanes } => elem.bit_width().map(|w| w * lanes),
        // **A float has a width now.** This was `None` on the grounds that floats were
        // 023 §7's `Approximated` territory — true while nothing could evaluate one, and
        // the reason the FP casts could not even reach their own match arm: the guard above
        // bailed on the *width* before the kind was looked at. The width was never the
        // uncertain part; `sort_of` has always given F32 32 bits. What is uncertain is a
        // *symbolic* float, and that is declared where it happens rather than by pretending
        // the type has no size.
        CTy::Float(k) => Some(match k {
            FloatKind::F32 => 32,
            FloatKind::F64 => 64,
            FloatKind::X87_80 => 80,
        }),
        // `Void` has no width at all, and writing it out means a new `CTy` is a compile
        // error here rather than a silent gap of the kind above.
        CTy::Void => None,
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
/// One concrete floating-point conversion, or `None` if the operand is symbolic.
///
/// **`FpToSi`/`FpToUi` truncate toward zero**, which is C11 6.3.1.4's rule and not the
/// rounding a reader expects: `(int)-2.7` is `-2`. Rust's `as` on floats does the same and
/// also saturates instead of being undefined at the edges, which is a better answer than
/// the hardware's and is recorded here rather than silently relied upon — a program whose
/// value depends on it is undefined in C, so nothing chiero reports about it is a claim.
/// Whether converting `v` to a `bits`-wide integer is undefined (C11 6.3.1.4).
///
/// The rule is about the **integral part**, so the bound is the destination's range and the
/// comparison is against the truncated value: `2147483647.5` converts to `INT_MAX` and is
/// defined, while `2147483648.0` is not. NaN has no integral part at all and is undefined
/// for every width.
///
/// Compared as `f64` rather than by casting the bound to an integer: `2^31` is exactly
/// representable and the endpoints have to be *inclusive* on one side and *exclusive* on the
/// other, which is the off-by-one this function exists to get right once.
fn out_of_range(v: f64, bits: u32, signed: bool) -> bool {
    if v.is_nan() {
        return true;
    }
    let t = v.trunc();
    if signed {
        let hi = (bits - 1) as f64;
        t >= hi.exp2() || t < -hi.exp2()
    } else {
        t >= (bits as f64).exp2() || t < 0.0
    }
}

fn fcast(
    a: &mut TermArena,
    kind: CastKind,
    x: Term,
    fw: u32,
    tw: u32,
    overflowed: &mut bool,
) -> Option<Term> {
    // **`eval_ground`, not `as_const`.** A term can be fully determined without being a
    // `Const` node: `sitofp` of a `sext` of a loaded byte is ground, and `as_const` sees
    // only the folded form — so `char c = 2; c < 2.5` produced no value at all. Wave 162
    // hit the same distinction reading a widened bound out of a query.
    let c = a.eval_ground(x).ok()?;
    let as_f64 = |w: u32, b: u128| -> Option<f64> {
        match w {
            32 => Some(f64::from(f32::from_bits(b as u32))),
            64 => Some(f64::from_bits(b as u64)),
            _ => None,
        }
    };
    let bits = match kind {
        // The source is an integer; its signedness is the cast's, not the width's.
        CastKind::SiToFp | CastKind::UiToFp => {
            let v = if kind == CastKind::SiToFp {
                c.signed() as f64
            } else {
                c.bits() as f64
            };
            match tw {
                32 => u128::from((v as f32).to_bits()),
                64 => u128::from(v.to_bits()),
                // **x87 takes the integer, not the `f64` above.** Every narrower target rounds
                // through `f64` harmlessly because `f64` is at least as wide; x87 is *wider*, so a
                // 64-bit integer past 2^53 would arrive already rounded — `9007199254740993`
                // becoming an even number. The magnitude and sign go in separately because
                // `unsigned_abs` needs the bit that negating inside `i64` does not have.
                80 => {
                    let (mag, neg) = if kind == CastKind::SiToFp {
                        let n = c.signed();
                        (u64::try_from(n.unsigned_abs()).ok()?, n < 0)
                    } else {
                        (u64::try_from(c.bits()).ok()?, false)
                    };
                    chiero_cir::fp::from_u64(mag, neg)
                }
                _ => return None,
            }
        }
        // **x87 first, and exactly.** Falling through to `as_f64` would round an 80-bit
        // significand into 53 bits before truncating it.
        CastKind::FpToSi | CastKind::FpToUi if fw == 80 => {
            let signed = kind == CastKind::FpToSi;
            *overflowed = chiero_cir::fp::out_of_int_range(c.bits(), tw, signed);
            // A conversion that does not fit is undefined, so any pattern is a legal answer and
            // the *event* is what matters — reported above. Zero is the answer that invents the
            // least.
            let t = chiero_cir::fp::trunc_to_int(c.bits()).unwrap_or(0);
            (t as u128) & mask_bits(tw)
        }
        CastKind::FpToSi => {
            let v = as_f64(fw, c.bits())?;
            // Truncate toward zero, then keep the low `tw` bits. A conversion that does not
            // fit is undefined in C, so any bit pattern is a legal answer — but it is also
            // an *event*, which `out_of_range` reports to the caller. Rust's saturating
            // `as` is what makes silence dangerous here: it produces a defensible number
            // nothing like the hardware's, so the run continues plausibly wrong.
            *overflowed = out_of_range(v, tw, true);
            ((v as i128) as u128) & mask_bits(tw)
        }
        CastKind::FpToUi => {
            let v = as_f64(fw, c.bits())?;
            *overflowed = out_of_range(v, tw, false);
            (v as u128) & mask_bits(tw)
        }
        CastKind::FpTrunc | CastKind::FpExt => {
            // **A narrowing out of x87 goes through the shared decoder**, which rounds to nearest
            // with ties to even and returns `None` where the answer would be a guess (§9). `as_f64`
            // still refuses 80, so this is the only route and the two directions cannot borrow each
            // other's behaviour.
            //
            // **`f80` narrows in one step, per target width** (wave 246). This used to go to `f64`
            // and let the `as f32` below finish the job, which rounds twice — and two roundings are
            // not one, so it refused `f32` outright rather than answering wrongly. `fp::to_f32`
            // rounds the eighty-bit significand straight to twenty-four, so the refusal is gone and
            // the `match tw` below is bypassed for this source width entirely.
            if fw == 80 && tw == 32 {
                return Some(a.bv(tw, u128::from(chiero_cir::fp::to_f32(c.bits()))));
            }
            let v = if fw == 80 {
                if tw != 64 {
                    return None;
                }
                chiero_cir::fp::to_f64(c.bits())?
            } else {
                as_f64(fw, c.bits())?
            };
            match tw {
                32 => u128::from((v as f32).to_bits()),
                64 => u128::from(v.to_bits()),
                // **Widening into x87 is exact, so it needs no rounding rule.** Reaching 80 here
                // can only be a widening — `as_f64` has already refused 80 as a *source* — and
                // x87 has both a wider exponent and a wider significand than `f64`, so every value
                // that arrives is representable. This is the whole reason it lands before
                // `FpTrunc`.
                80 => chiero_cir::fp::from_f64(v),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(a.bv(tw, bits))
}

/// The low `w` bits set, for widths up to 128.
fn mask_bits(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/// The inclusive range of a `w`-bit **signed** value, as `i128`.
///
/// **`1i128 << (w - 1)` is the bug it exists to remove.** At `w = 128` that shift *is*
/// `i128::MIN`, so `- 1` underflows and `-(…)` negates a value with no positive counterpart —
/// both panic. Every narrower width leaves headroom, which is why `__int128` was the only type
/// that crashed and why the arithmetic computing the boundary had the same boundary problem it
/// was checking for.
///
/// `w == 0` cannot occur for an operand — `bits_of_cty` returns it only for `void` — but it is
/// answered rather than shifted, so a future caller gets an empty range instead of a panic.
fn signed_range(w: u32) -> (i128, i128) {
    match w {
        0 => (0, 0),
        128.. => (i128::MIN, i128::MAX),
        _ => (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1),
    }
}

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
        // **Floating point, when both operands are concrete.**
        //
        // There is no float sort in `chiero-solver`, so a symbolic float cannot be
        // constrained and `fbin` returns `None` for one — a declared gap rather than a
        // folded guess. Concrete operands need no theory at all: the bits are already in
        // the arena, IEEE-754 is what `f32`/`f64` implement, and the answer goes back as
        // bits. Pointer differences remain unmodelled.
        BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv | BinOp::FRem => {
            return fbin(a, op, x, y);
        }
        _ => return None,
    })
}

/// One concrete floating-point operation, or `None` if either operand is symbolic.
///
/// **Width comes from the terms, not from the instruction's `ty`.** The two agree in
/// well-formed CIR, and where they disagree the bits are what exist — computing at a width
/// the value does not have is how a reinterpretation becomes a wrong number.
fn fbin(a: &mut TermArena, op: BinOp, x: Term, y: Term) -> Option<Term> {
    // Ground rather than folded — see `fcast`.
    let (xc, yc) = (a.eval_ground(x).ok()?, a.eval_ground(y).ok()?);
    let w = xc.width();
    if w != yc.width() {
        return None;
    }
    let bits = match w {
        32 => {
            let (p, q) = (
                f32::from_bits(xc.bits() as u32),
                f32::from_bits(yc.bits() as u32),
            );
            u128::from(fop32(op, p, q)?.to_bits())
        }
        64 => {
            let (p, q) = (
                f64::from_bits(xc.bits() as u64),
                f64::from_bits(yc.bits() as u64),
            );
            u128::from(fop64(op, p, q)?.to_bits())
        }
        // **x87 multiplication, which needs no `f64` and no loop.** The comment here used to say
        // emulating the format "to get one operation right would be a second float implementation
        // nobody tests" — a correct judgement about one operation before `chiero_cir::fp` existed,
        // and the wrong one now that the format lives in one place and is tested. As of wave 242
        // all four arithmetic operations are here, and the fall-through below is reached only by a
        // width that is not 32, 64 or 80.
        80 if op == BinOp::FMul => chiero_cir::fp::mul(xc.bits(), yc.bits())?,
        80 if op == BinOp::FAdd => chiero_cir::fp::add(xc.bits(), yc.bits())?,
        80 if op == BinOp::FSub => chiero_cir::fp::sub(xc.bits(), yc.bits())?,
        80 if op == BinOp::FDiv => chiero_cir::fp::div(xc.bits(), yc.bits())?,
        _ => return None,
    };
    Some(a.bv(w, bits))
}

fn fop32(op: BinOp, p: f32, q: f32) -> Option<f32> {
    Some(match op {
        BinOp::FAdd => p + q,
        BinOp::FSub => p - q,
        BinOp::FMul => p * q,
        BinOp::FDiv => p / q,
        BinOp::FRem => p % q,
        _ => return None,
    })
}

fn fop64(op: BinOp, p: f64, q: f64) -> Option<f64> {
    Some(match op {
        BinOp::FAdd => p + q,
        BinOp::FSub => p - q,
        BinOp::FMul => p * q,
        BinOp::FDiv => p / q,
        BinOp::FRem => p % q,
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
        // **The floating comparisons, concretely.** Symbolic floats have no sort to
        // constrain, so `fcmp` declines and the caller records a gap — the same line
        // waves 167 and 168 drew for arithmetic and negation.
        CmpOp::FOEq
        | CmpOp::FONe
        | CmpOp::FOLt
        | CmpOp::FOLe
        | CmpOp::FUEq
        | CmpOp::FUNe
        | CmpOp::FULt
        | CmpOp::FULe
        | CmpOp::FOrd
        | CmpOp::FUno => return fcmp(a, op, x, y),
        // **No catch-all**, and that is the point: with the float arms added this match is
        // exhaustive, so a `CmpOp` added later is a compile error here rather than a silent
        // `None` the caller reads as "symbolic operand". The `_` that used to sit here was
        // load-bearing while floats were missing and is a hiding place now.
    })
}

/// One concrete floating comparison, as a one-bit term.
///
/// **Ordered and unordered are not each other's negation.** An *ordered* comparison is
/// false whenever either operand is NaN; an *unordered* one is true whenever either is.
/// So `FOLt` and `FUGe` are complements but `FOLt` and `FOGe` are not — with a NaN both are
/// false. Rust's `<` on floats is the ordered form and `is_nan` supplies the rest, which is
/// why each arm is written out rather than derived from two others.
///
/// C's `isnan` idiom is `x != x`, and it lowers to `FUNe` for exactly this reason: `FONe`
/// is *false* for NaN, which is the opposite of what the idiom asks.
fn fcmp(a: &mut TermArena, op: CmpOp, x: Term, y: Term) -> Option<Term> {
    // Ground rather than folded — see `fcast`.
    let (xc, yc) = (a.eval_ground(x).ok()?, a.eval_ground(y).ok()?);
    if xc.width() != yc.width() {
        return None;
    }
    let (p, q) = match xc.width() {
        32 => (
            f64::from(f32::from_bits(xc.bits() as u32)),
            f64::from(f32::from_bits(yc.bits() as u32)),
        ),
        64 => (
            f64::from_bits(xc.bits() as u64),
            f64::from_bits(yc.bits() as u64),
        ),
        // **x87 is compared on the patterns, not narrowed.** The comment here used to say comparing
        // it "would need a second float implementation nobody tests" — true of arithmetic, and a
        // comparison needs none: `fp::partial_cmp` reads the fields. Narrowing to `f64` first would
        // call `1 + 2^-53` equal to `1.0`.
        80 => {
            let ord = chiero_cir::fp::partial_cmp(xc.bits(), yc.bits());
            let unordered = ord.is_none();
            let eq = ord == Some(core::cmp::Ordering::Equal);
            let lt = ord == Some(core::cmp::Ordering::Less);
            // **Only the four orderings CIR defines, plus `FOrd`/`FUno`.** There is no `FOGt`:
            // 020 spells `a > b` by swapping the operands, so implementing one here would be a
            // variant the rest of the system never emits.
            let r = match op {
                CmpOp::FOEq => eq,
                CmpOp::FONe => !unordered && !eq,
                CmpOp::FOLt => lt,
                CmpOp::FOLe => lt || eq,
                CmpOp::FUEq => unordered || eq,
                CmpOp::FUNe => unordered || !eq,
                CmpOp::FULt => unordered || lt,
                CmpOp::FULe => unordered || lt || eq,
                CmpOp::FOrd => !unordered,
                CmpOp::FUno => unordered,
                _ => return None,
            };
            return Some(a.bv(1, u128::from(r)));
        }
        _ => return None,
    };
    // Widening `f32` to `f64` is exact, so comparing at the wider type gives the same
    // answers — including for NaN, which stays NaN.
    let unordered = p.is_nan() || q.is_nan();
    let r = match op {
        CmpOp::FOEq => p == q,
        CmpOp::FONe => !unordered && p != q,
        CmpOp::FOLt => p < q,
        CmpOp::FOLe => p <= q,
        CmpOp::FUEq => unordered || p == q,
        // `p != q` is already true when either is NaN, which is what makes this the
        // idiom's operator.
        CmpOp::FUNe => p != q,
        CmpOp::FULt => unordered || p < q,
        CmpOp::FULe => unordered || p <= q,
        CmpOp::FOrd => !unordered,
        CmpOp::FUno => unordered,
        // Every integer comparison, which `cmp` handles above and never routes here. Not a
        // catch-all: naming them means a new `CmpOp` is a compile error in this function
        // rather than a silent `None` that reads as "symbolic operand".
        CmpOp::Eq
        | CmpOp::Ne
        | CmpOp::ULt
        | CmpOp::ULe
        | CmpOp::UGt
        | CmpOp::UGe
        | CmpOp::SLt
        | CmpOp::SLe
        | CmpOp::SGt
        | CmpOp::SGe => return None,
    };
    Some(a.bv(1, u128::from(r)))
}

/// Every variable occurring anywhere in `t`, transitively.
///
/// `TermArena::subterms` yields **immediate** children only, which is the whole reason the
/// first attempt at this reported a *constrained* input as free: the variable in a lowered
/// comparison sits four wrappers down, under `not`, `=`, `zext` and `ite`.
fn vars_of(a: &TermArena, t: Term, out: &mut indexmap::IndexSet<chiero_solver::VarId>) {
    let mut stack = vec![t];
    let mut seen: indexmap::IndexSet<Term> = Default::default();
    while let Some(cur) = stack.pop() {
        // A term arena is a DAG with sharing, so revisiting is normal — and unbounded work
        // without this.
        if !seen.insert(cur) {
            continue;
        }
        if let Some(v) = a.var_id(cur) {
            out.insert(v);
        }
        stack.extend(a.subterms(cur));
    }
}

/// One witness's bindings, read off `model`.
///
/// `mentioned` decides `pinned`: a variable nothing in scope constrains gets a value from
/// the model — a complete assignment must give it one — but saying the fault *needs* it
/// would tell a reader to reproduce something that reproduces without it.
fn bindings_under(
    a: &TermArena,
    s: &State,
    model: &chiero_solver::Model,
    extra: &[(Term, InputOrigin)],
    mentioned: &indexmap::IndexSet<chiero_solver::VarId>,
) -> Vec<Binding> {
    let mut bindings = Vec::new();
    for (t, origin) in s.inputs.iter().cloned().chain(extra.iter().cloned()) {
        let width = a.width(t);
        // **`pinned` is the honest part.** A model need not assign a variable the path
        // never mentions; binding it to zero and presenting that as the solver's answer
        // would tell a reader the bug needs a value it does not.
        let mut mine = indexmap::IndexSet::new();
        vars_of(a, t, &mut mine);
        let constrained = mine.iter().any(|v| mentioned.contains(v));
        let (value, pinned) = match a.eval(model, t) {
            Ok(c) => (c.bits(), constrained),
            Err(_) => (0, false),
        };
        bindings.push(Binding {
            origin,
            width,
            value,
            pinned,
        });
    }
    bindings
}
