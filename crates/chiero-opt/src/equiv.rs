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
//! ## Why pairing every path with every path is complete
//!
//! For any input `i`, the `before` run has some path `p` that `i` follows and the `after`
//! run some path `q`, so the pair `(p, q)` is feasible under `i` and is checked. A pair no
//! input satisfies is `Unsat` on its own conjunction and contributes nothing. **This rests
//! on both runs being exhaustive**, which is exactly what `Fidelity::Exact` means and what
//! a `TermReason::Budget` state denies — hence [`Equivalence::Equivalent`]'s fidelity being
//! the degradation of both runs' rather than a constant.
//!
//! ## The witness is minimized, not whichever model came back first
//!
//! Contract 13 wants `prove_equivalent(a, b)` and `prove_equivalent(b, a)` to produce "a
//! correspondingly swapped witness". Two solver queries that differ only in which version
//! minted variable 0 are free to return different satisfying models, and nothing about
//! SMT-LIB promises otherwise — so a witness taken straight from the first `Sat` makes the
//! contract a coin flip. Instead the distinguishing input is minimized to the numerically
//! smallest one, by binary search per input in index order. That is canonical, so it does
//! not depend on argument order; it is reproducible, which 001 §5 requires anyway; and
//! "the smallest input that distinguishes them" is a better thing to hand a reader than
//! "an input that distinguishes them".
//!
//! # What is not built yet, stated rather than papered over
//!
//! §1.1 makes equivalence three claims — return value, observable footprint, ordered side
//! effects — and only the return value and termination are decided here. The other two are
//! not silently assumed to hold: a comparison that would have to reason about caller-visible
//! memory or about a side-effect sequence answers [`Equivalence::Unknown`] naming the claim
//! it could not check. That is the difference between "chiero proved these agree" and
//! "chiero checked the easy part and said nothing about the rest".

use chiero_cir::{CTy, Function, Module};
use chiero_exec::{
    Assumption, Binding, Budget, Engine, Fidelity, InputOrigin, SolverTier, State, Status,
    TermReason, Value, Witness,
};
use chiero_solver::{
    BvConst, CheckResult, Model, PathCondition, SmtLib, Term, TermArena, TieredSolver,
};

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

impl Divergence {
    /// A total order over the *kinds*, so that two runs which found the same set of
    /// divergences report the same one. Ties within a kind are broken by the witness,
    /// which is minimized and therefore already canonical.
    fn rank(&self) -> u8 {
        match self {
            Divergence::ReturnValue { .. } => 0,
            Divergence::Memory { .. } => 1,
            Divergence::SideEffect { .. } => 2,
            Divergence::Termination { .. } => 3,
        }
    }

    /// The same divergence seen from the other argument order — contract 13.
    fn swapped(self) -> Divergence {
        match self {
            Divergence::ReturnValue { before, after } => Divergence::ReturnValue {
                before: after,
                after: before,
            },
            Divergence::Memory {
                object,
                offset,
                before,
                after,
            } => Divergence::Memory {
                object,
                offset,
                before: after,
                after: before,
            },
            Divergence::SideEffect {
                index,
                before,
                after,
            } => Divergence::SideEffect {
                index,
                before: after,
                after: before,
            },
            Divergence::Termination { before, after } => Divergence::Termination {
                before: after,
                after: before,
            },
        }
    }
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
///
/// See the module documentation for the method, for why the signature is not §1's, and for
/// which of §1.1's three claims are decided today.
pub fn prove_equivalent(before: &Module, after: &Module, cfg: &EquivCfg) -> Equivalence {
    let unknown = |r: String| Equivalence::Unknown { reason: r };

    let (Some(fb), Some(fa)) = (entry_of(before, &cfg.entry), entry_of(after, &cfg.entry)) else {
        return unknown(format!("no function named `{}` in both modules", cfg.entry));
    };
    if let Some(r) = signature_mismatch(fb, fa) {
        return unknown(r);
    }
    // A pointer parameter is a *caller-visible object*, and comparing what the two versions
    // left in it is §1.1's second claim plus the object bijection of §1.1's own footnote —
    // neither of which is built. Refusing here rather than at the end is the difference
    // between "not checked" and a proof of the wrong theorem.
    if let Some(i) = fb.params.iter().position(|p| p.ty == CTy::Ptr) {
        return unknown(format!(
            "parameter {i} is a pointer: comparing caller-visible memory (041 §1.1) and \
             matching objects up to the bijection are not implemented"
        ));
    }

    let mut arena = TermArena::new();
    let rb = run_side(before, cfg, &mut arena);
    let ra = run_side(after, cfg, &mut arena);

    // A state the engine could not finish is not a path that agrees; it is a path nobody
    // looked at. Contract 12's rule generalized: never `Equivalent` on the strength of a
    // question that was not asked.
    for (side, r) in [("before", &rb), ("after", &ra)] {
        for s in r.states() {
            match &s.status {
                Status::Terminated(TermReason::Unsupported) => {
                    return unknown(format!(
                        "the {side} version has a path chiero cannot follow"
                    ));
                }
                Status::Errored(e) => {
                    return unknown(format!("the {side} version errored: {e}"));
                }
                Status::Running => {
                    return unknown(format!("the {side} version left a path unfinished"));
                }
                _ => {}
            }
        }
    }

    let mut solver = match cfg.backend.clone() {
        Some(b) => TieredSolver::with_backend(b),
        None => TieredSolver::new(),
    };

    // Every divergence found, each with its minimized witness. Collected rather than
    // returned at the first hit so the reported one can be chosen canonically (contract 13).
    let mut found: Vec<Candidate> = Vec::new();

    for sb in rb.states() {
        for sa in ra.states() {
            let Some((link, eqs)) = link_inputs(&mut arena, sb, sa) else {
                return unknown(
                    "the two versions' symbolic inputs cannot be matched pairwise; only \
                     entry parameters are matched today (041 §1.2 wants the extern-return \
                     symbols shared too)"
                        .to_string(),
                );
            };
            let mut pc = pair_condition(sb, sa, &eqs);
            match solver.check_path(&mut arena, &mut pc, &[]) {
                // No input follows both paths: the pair says nothing.
                CheckResult::Unsat => continue,
                CheckResult::Unknown(r) => {
                    return unknown(format!(
                        "solver could not decide whether a pair of paths is reachable: {r:?}"
                    ));
                }
                CheckResult::Sat(_) => {}
            }

            // §1.1's third observable, checked structurally: two paths whose termination
            // differs are not equivalent, whatever they returned.
            let (tb, ta) = (term_reason(sb), term_reason(sa));
            if tb != ta {
                let m = match solver.check_path(&mut arena, &mut pc, &[]) {
                    CheckResult::Sat(m) => m,
                    _ => unreachable!("just checked Sat"),
                };
                let (key, w) = minimized_witness(&mut solver, &mut arena, &mut pc, sb, &link, m);
                found.push((
                    key,
                    Divergence::Termination {
                        before: tb,
                        after: ta,
                    },
                    w,
                ));
                continue;
            }

            match compare_returns(&mut solver, &mut arena, &mut pc, sb, sa, &link) {
                Ok(None) => {}
                Ok(Some((key, d, w))) => found.push((key, d, w)),
                Err(r) => return unknown(r),
            }
        }
    }

    if let Some((_, observation, input)) = pick(found) {
        return Equivalence::Differs {
            input,
            observation,
            replay: None,
        };
    }

    let mut assumptions: Vec<Assumption> = Vec::new();
    for r in [&rb, &ra] {
        for s in r.states() {
            for a in s.assumptions() {
                if !assumptions.contains(a) {
                    assumptions.push(a.clone());
                }
            }
        }
    }
    Equivalence::Equivalent {
        fidelity: rb.fidelity().degrade(ra.fidelity()),
        footprint: Footprint {
            compared: vec![Claim::ReturnValue, Claim::Termination],
        },
        assumptions,
    }
}

/// One divergence, keyed by the minimized input that produces it.
type Candidate = (Vec<u128>, Divergence, Witness);

/// The canonical divergence: smallest witness first, then the divergence kind.
fn pick(mut found: Vec<Candidate>) -> Option<Candidate> {
    found.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.rank().cmp(&y.1.rank())));
    found.into_iter().next()
}

fn entry_of<'m>(m: &'m Module, name: &str) -> Option<&'m Function> {
    m.funcs.iter().find(|f| &*f.name == name)
}

/// The one structural precondition. Two functions with different signatures are not two
/// versions of one function, and "they returned different widths" is a fact about the
/// comparison rather than about the program.
fn signature_mismatch(a: &Function, b: &Function) -> Option<String> {
    if a.params.len() != b.params.len() {
        return Some(format!(
            "different arity: {} parameters before, {} after",
            a.params.len(),
            b.params.len()
        ));
    }
    for (i, (p, q)) in a.params.iter().zip(&b.params).enumerate() {
        if p.ty != q.ty {
            return Some(format!(
                "parameter {i} has a different type: {:?} vs {:?}",
                p.ty, q.ty
            ));
        }
    }
    if a.ret != b.ret {
        return Some(format!("different return type: {:?} vs {:?}", a.ret, b.ret));
    }
    None
}

fn run_side(m: &Module, cfg: &EquivCfg, arena: &mut TermArena) -> chiero_exec::RunResult {
    let e = Engine::new(m)
        .with_entry(&cfg.entry)
        .with_budget(cfg.budget);
    let e = match cfg.backend.clone() {
        Some(b) => e.with_backend(b),
        None => e.with_solver(SolverTier::LiteOnly),
    };
    e.run(arena)
}

fn term_reason(s: &State) -> TermReason {
    match &s.status {
        Status::Terminated(r) => *r,
        // The sweep above rejected everything else before any pair was formed.
        _ => TermReason::Unsupported,
    }
}

/// The matched input pairs, and the equality asserting each pair is the same input.
type Link = (Vec<(Term, Term)>, Vec<Term>);

/// Pair up the two paths' symbolic inputs, returning one equality per matched pair.
///
/// `None` means an input on one side has no counterpart on the other. That is a refusal,
/// not a zero: leaving it unconstrained would let the solver pick different values for what
/// is meant to be the same input and report a divergence no caller can reproduce, and
/// dropping it silently would prove equivalence over a smaller input space than the caller
/// asked about.
fn link_inputs(a: &mut TermArena, sb: &State, sa: &State) -> Option<Link> {
    let params = |s: &State| -> Vec<(usize, Term)> {
        s.inputs()
            .iter()
            .filter_map(|(t, o)| match o {
                InputOrigin::Param { index, .. } => Some((*index, *t)),
                _ => None,
            })
            .collect()
    };
    let (pb, pa) = (params(sb), params(sa));
    // Any input that is not an entry parameter is one this code cannot match — an extern's
    // return, a lazily-materialized byte. §1.2 wants those shared too; they are not.
    if pb.len() != sb.inputs().len() || pa.len() != sa.inputs().len() || pb.len() != pa.len() {
        return None;
    }
    let mut out = Vec::with_capacity(pb.len());
    let mut eqs = Vec::with_capacity(pb.len());
    for (i, tb) in &pb {
        let ta = pa.iter().find(|(j, _)| j == i)?.1;
        if a.width(*tb) != a.width(ta) {
            return None;
        }
        out.push((*tb, ta));
        eqs.push(a.eq(*tb, ta));
    }
    Some((out, eqs))
}

/// Both paths' conditions plus the input equalities — §1.2's "paths are paired by input
/// constraint".
fn pair_condition(sb: &State, sa: &State, eqs: &[Term]) -> PathCondition {
    let mut terms = sb.path.clone();
    terms.extend(sa.path.iter().copied());
    terms.extend(eqs.iter().copied());
    PathCondition::from_parts(
        terms,
        sb.path_possibly_infeasible() || sa.path_possibly_infeasible(),
    )
}

/// §1.2's "solver query on the disjunction of the disagreement conditions" — for the one
/// disjunct that is built.
type Divergent = Option<Candidate>;

fn compare_returns(
    solver: &mut TieredSolver,
    arena: &mut TermArena,
    pc: &mut PathCondition,
    sb: &State,
    sa: &State,
    link: &[(Term, Term)],
) -> Result<Divergent, String> {
    let (rb, ra) = (sb.return_value(), sa.return_value());
    let (tb, ta) = match (rb, ra) {
        (None, None) => return Ok(None),
        (Some(Value::Scalar(x)), Some(Value::Scalar(y))) => (x, y),
        // A returned pointer is only comparable up to §1.1's object bijection, which is not
        // built; `Undef` is a value the program did not choose, and choosing one to compare
        // would be choosing for it.
        _ => {
            return Err(
                "a returned value is a pointer or undef: pointer returns need the object \
                 bijection of 041 §1.1, which is not implemented"
                    .to_string(),
            );
        }
    };
    if arena.width(tb) != arena.width(ta) {
        return Err("the two versions return different widths".to_string());
    }
    let eq = arena.eq(tb, ta);
    let neq = arena.not(eq);
    match solver.check_path(arena, pc, &[neq]) {
        CheckResult::Unsat => Ok(None),
        CheckResult::Unknown(r) => Err(format!(
            "solver could not decide whether the return values agree: {r:?}"
        )),
        CheckResult::Sat(m) => {
            let (key, w) = minimized_witness_with(solver, arena, pc, sb, link, m, &[neq]);
            // Re-solve at the minimized input so the reported numbers are the ones the
            // witness produces. A `before`/`after` pair read off the *first* model with a
            // witness taken from a *different* model would be two facts about two different
            // inputs, printed as if they were one.
            let m2 = pin(solver, arena, pc, link, &key, &[neq]);
            let (bb, aa) = match &m2 {
                Some(m2) => (arena.eval(m2, tb), arena.eval(m2, ta)),
                None => return Err("the minimized witness stopped distinguishing".to_string()),
            };
            match (bb, aa) {
                (Ok(bc), Ok(ac)) => Ok(Some((
                    key,
                    Divergence::ReturnValue {
                        before: bc,
                        after: ac,
                    },
                    w,
                ))),
                _ => Err("the model does not evaluate the returned terms".to_string()),
            }
        }
    }
}

/// Binary-search each matched input down to its smallest distinguishing value, in index
/// order. See the module docs for why the first `Sat` is not good enough.
fn minimized_witness(
    solver: &mut TieredSolver,
    arena: &mut TermArena,
    pc: &mut PathCondition,
    sb: &State,
    link: &[(Term, Term)],
    m: Model,
) -> (Vec<u128>, Witness) {
    minimized_witness_with(solver, arena, pc, sb, link, m, &[])
}

fn minimized_witness_with(
    solver: &mut TieredSolver,
    arena: &mut TermArena,
    pc: &mut PathCondition,
    sb: &State,
    link: &[(Term, Term)],
    m: Model,
    extra: &[Term],
) -> (Vec<u128>, Witness) {
    let mut fixed: Vec<u128> = Vec::with_capacity(link.len());
    for (tb, _) in link.iter() {
        let w = arena.width(*tb);
        // `best` is always a value this input is *known* to be able to take given the
        // inputs already fixed; the search only ever lowers it.
        let mut best = arena.eval(&m, *tb).map(|c| c.bits()).unwrap_or(0);
        let (mut lo, mut hi) = (0u128, best);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let bound = arena.bv(w, mid);
            let gt = arena.ult(bound, *tb);
            let le = arena.not(gt);
            let mut asm: Vec<Term> = extra.to_vec();
            asm.extend(pins(arena, link, &fixed));
            asm.push(le);
            match solver.check_path(arena, pc, &asm) {
                CheckResult::Sat(m2) => {
                    best = arena
                        .eval(&m2, *tb)
                        .map(|c| c.bits())
                        .unwrap_or(mid)
                        .min(mid);
                    hi = best;
                }
                CheckResult::Unsat => lo = mid + 1,
                // Stop rather than guess: a bound the solver could not decide is not a
                // bound, and reporting an unconfirmed value as *the* smallest
                // distinguishing input would be the flattering kind of wrong.
                CheckResult::Unknown(_) => break,
            }
        }
        fixed.push(best);
    }
    (fixed.clone(), witness_for(arena, sb, link, &fixed))
}

/// Equalities pinning the already-minimized inputs to their chosen values.
fn pins(arena: &mut TermArena, link: &[(Term, Term)], fixed: &[u128]) -> Vec<Term> {
    let mut out = Vec::with_capacity(fixed.len());
    for (k, v) in fixed.iter().enumerate() {
        let (tb, _) = link[k];
        let w = arena.width(tb);
        let c = arena.bv(w, *v);
        out.push(arena.eq(tb, c));
    }
    out
}

/// Re-solve with every input pinned, to read the two versions' answers at *this* input.
fn pin(
    solver: &mut TieredSolver,
    arena: &mut TermArena,
    pc: &mut PathCondition,
    link: &[(Term, Term)],
    fixed: &[u128],
    extra: &[Term],
) -> Option<Model> {
    let mut asm: Vec<Term> = extra.to_vec();
    asm.extend(pins(arena, link, fixed));
    match solver.check_path(arena, pc, &asm) {
        CheckResult::Sat(m) => Some(m),
        _ => None,
    }
}

/// The `before` side's inputs bound to the minimized values — 023 §9's `Witness`, reused
/// rather than reinvented so 040's replay harness can consume it unchanged.
fn witness_for(arena: &TermArena, sb: &State, link: &[(Term, Term)], fixed: &[u128]) -> Witness {
    let mut bindings = Vec::with_capacity(link.len());
    for (k, (tb, _)) in link.iter().enumerate() {
        let origin = sb
            .inputs()
            .iter()
            .find(|(t, _)| t == tb)
            .map(|(_, o)| o.clone())
            .expect("the link was built from this state's own inputs");
        bindings.push(Binding {
            origin,
            width: arena.width(*tb),
            value: fixed[k],
            pinned: true,
        });
    }
    Witness { bindings }
}

/// Contract 13's swap, for a caller that has one verdict and wants the other direction's.
/// Exposed because a *test* of symmetry that reimplements the swap tests its own copy.
pub fn swap_verdict(v: Equivalence) -> Equivalence {
    match v {
        Equivalence::Differs {
            input,
            observation,
            replay,
        } => Equivalence::Differs {
            input,
            observation: observation.swapped(),
            replay,
        },
        other => other,
    }
}
