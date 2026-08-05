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
//! effects — and two of them are decided here: the return value (with termination) and the
//! effect sequence. **The footprint is not.** It is not silently assumed to hold either:
//! [`observable_beyond_the_return`] refuses, before either version is run, any pair that could
//! touch caller-visible memory — a volatile access, a store through an address that is not
//! provably a stack slot, inline asm, a variadic list, an indirect call. That is the difference between "chiero proved these agree" and "chiero checked the
//! easy part and said nothing about the rest".
//!
//! **That paragraph was here for a whole commit before anything implemented it**, and an
//! adversarial review found `g = x; return 0` against `return 0` reported
//! `Equivalent { Exact }`. It is worth saying plainly, because documentation is what a
//! reader checks *instead of* the code: a written intention with no implementation is worse
//! than an admitted gap. `crates/chiero-opt/tests/adversarial.rs` holds that fixture and the
//! five others the same review produced.

use chiero_cir::{
    Body, CTy, Callee, FuncId, Function, InstKind, Module, Operand, RValue, ValueId, Volatility,
};
use chiero_exec::{
    Assumption, AssumptionKind, Binding, Budget, Engine, Fidelity, InputOrigin, SolverTier, State,
    Status, TermReason, Value, Witness,
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
    // **§1.1's other two claims, refused rather than assumed.** The comparison below decides
    // return values and termination. If either version can touch caller-visible memory or
    // produce an observable event, the two claims this cannot check are claims that *matter
    // for this pair*, and answering `Equivalent` would be answering a different question.
    //
    // Found by review, and the review's sharpest point was that this paragraph already
    // existed in the module documentation with nothing implementing it. `g = x; return 0`
    // against `return 0` was `Equivalent { Exact }`.
    for (side, m, f) in [("before", before, fb), ("after", after, fa)] {
        if let Some(why) = observable_beyond_the_return(m, f) {
            return unknown(format!(
                "the {side} version {why}, and comparing caller-visible memory and the \
                 side-effect sequence (041 §1.1) is not implemented"
            ));
        }
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
    // How many pairs actually reached a point where the two could be compared, and how
    // many were cut off before they could. See the check below the loop for why the
    // difference is the whole difference between a verdict and silence.
    let (mut examined, mut cut) = (0usize, 0usize);
    // Whether any pair actually had two return values to compare. A void function's verdict
    // that listed `ReturnValue` among what it compared would be naming a claim nobody made.
    let mut compared_returns = false;
    let mut compared_effects = false;

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

            // **A budget cut is chiero running out, not the program diverging.** A pair
            // where one side was truncated has nothing to say: the two paths did not reach
            // a common point, and reporting `Termination { Return, Budget }` would put
            // chiero's own limit in front of a reader as a defect in their rewrite. §1.2
            // already has the honest name for this region — it is what `Bounded` means.
            let (tb, ta) = (term_reason(sb), term_reason(sa));
            if tb == TermReason::Budget || ta == TermReason::Budget {
                cut += 1;
                continue;
            }
            // **Two crashes are not agreement.** `TermReason::Crashed` says a path faulted,
            // not *how*: a null dereference and a use-after-free are the same variant, and
            // §1.1 counts abnormal termination as observable. Comparing them as `(None,
            // None)` — which is what "neither returned a value" does — reads two unrelated
            // faults as the two versions doing the same thing. Refused until the fault
            // itself is compared.
            if tb == TermReason::Crashed || ta == TermReason::Crashed {
                return unknown(
                    "a path faults on one or both sides, and chiero cannot yet tell two \
                     abnormal terminations apart (041 §1.1)"
                        .to_string(),
                );
            }
            examined += 1;

            // §1.1's third observable, checked structurally: two paths whose termination
            // differs are not equivalent, whatever they returned.
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

            // §1.1's third claim, decided before the return: two versions that made
            // different calls are not equivalent whatever they went on to return, and the
            // effect is the more legible finding of the two.
            if !sb.effects().is_empty() || !sa.effects().is_empty() {
                compared_effects = true;
            }
            match compare_effects(&mut solver, &mut arena, &mut pc, sb, sa, &link) {
                Ok(None) => {}
                Ok(Some((key, d, w))) => {
                    found.push((key, d, w));
                    continue;
                }
                Err(r) => return unknown(r),
            }

            if sb.return_value().is_some() && sa.return_value().is_some() {
                compared_returns = true;
            }
            match compare_returns(&mut solver, &mut arena, &mut pc, sb, sa, &link) {
                Ok(None) => {}
                Ok(Some((key, d, w))) => found.push((key, d, w)),
                Err(r) => return unknown(r),
            }
        }
    }

    // **"No pair disagreed" and "there were no pairs" are the same silence.** Only one of
    // them is a proof, and the difference is invisible in the verdict unless it is counted:
    // a run whose every path was cut by a budget would otherwise report `Equivalent` —
    // hedged to `Bounded`, which reads as "agrees within a bound" and not as "nothing was
    // looked at". Found by asking what the pairing loop does with nothing to iterate over.
    if examined == 0 {
        return unknown(format!(
            "no pair of paths reached a comparable end: {cut} pair(s) were cut by a budget,              {} before-path(s) and {} after-path(s) in all",
            rb.states().len(),
            ra.states().len()
        ));
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
    let fidelity = rb.fidelity().degrade(ra.fidelity());
    // Whether either side has an input this comparison could not match — an extern's return.
    // `link_inputs` already refused such a pair, so reaching here means neither has one; the
    // check is recomputed rather than assumed because `blessable`'s third channel depends on
    // it and a refusal that moves would silently widen the blessing.
    let no_returns = [&rb, &ra].iter().all(|r| {
        r.states().iter().all(|s| {
            s.inputs()
                .iter()
                .all(|(_, o)| matches!(o, InputOrigin::Param { .. }))
        })
    });
    if let Err(why) = blessable(fidelity, &assumptions, no_returns) {
        return unknown(why);
    }
    Equivalence::Equivalent {
        fidelity,
        footprint: Footprint {
            compared: {
                let mut c = vec![Claim::Termination];
                if compared_returns {
                    c.insert(0, Claim::ReturnValue);
                }
                if compared_effects {
                    c.push(Claim::SideEffects);
                }
                c
            },
        },
        assumptions,
    }
}

/// One divergence, keyed by the minimized input that produces it.
type Candidate = (Vec<u128>, Divergence, Witness);

/// Whether this function can do anything observable that the comparison below does not
/// decide — §1.1's claims 2 and 3.
///
/// **Conservative by construction, and deliberately syntactic.** It answers "could this
/// touch caller-visible memory or produce an observable event", not "does it, on this path".
/// A precise answer needs the run, and the run is what would be blessed on the strength of
/// it; a syntactic over-approximation can only cost a refusal, while a precise-but-wrong one
/// costs a wrong proof. When §1.1's other two claims are actually compared this whole
/// function goes away rather than getting cleverer.
///
/// Walks the entry and every function it can reach with a body, because a store to a global
/// two calls down is no less a store.
fn observable_beyond_the_return(m: &Module, entry: &Function) -> Option<String> {
    let mut seen: Vec<FuncId> = vec![entry.id];
    let mut queue = vec![entry.id];
    while let Some(id) = queue.pop() {
        let Some(f) = m.funcs.iter().find(|f| f.id == id) else {
            return Some("calls a function that is not in the module".to_string());
        };
        for b in &f.blocks {
            for i in &b.insts {
                if let Some(why) = observable_inst(m, f, &i.kind, &mut seen, &mut queue) {
                    return Some(why);
                }
            }
        }
    }
    None
}

/// The values in this function that are **provably** the address of one of its own stack
/// slots. Seeded by `AddrOfLocal` and grown through pointer arithmetic and copies.
///
/// Everything else is treated as escaping, `Phi` included: a merge of a local and a global
/// address is not a local address, and a conservative set is the only kind that is safe to
/// bless on. `AddrOfLocal` is the one address a function knows nobody else holds — 041 §1.1's
/// "not stack temporaries, which is what permits most real refactors".
fn local_addresses(f: &Function) -> Vec<ValueId> {
    let mut local: Vec<ValueId> = Vec::new();
    // Two passes, because CIR blocks are in no particular order and a `ptradd` may precede
    // the `addrlocal` it builds on in block order. A fixpoint would be more general; two
    // passes is enough for the straight-line shapes and errs by treating a late-discovered
    // local as escaping, which refuses rather than blesses.
    for _ in 0..2 {
        for b in &f.blocks {
            for i in &b.insts {
                let InstKind::Assign { dst, rv } = &i.kind else {
                    continue;
                };
                let from = |o: &Operand| match o {
                    Operand::Value(v) => local.contains(v),
                    _ => false,
                };
                let is_local = match rv {
                    RValue::AddrOfLocal { .. } => true,
                    RValue::PtrAdd { base, .. } => from(base),
                    RValue::Use(o) => from(o),
                    RValue::Cast { a, .. } => from(a),
                    _ => false,
                };
                if is_local && !local.contains(dst) {
                    local.push(*dst);
                }
            }
        }
    }
    local
}

fn observable_inst(
    m: &Module,
    f: &Function,
    k: &InstKind,
    seen: &mut Vec<FuncId>,
    queue: &mut Vec<FuncId>,
) -> Option<String> {
    let local = local_addresses(f);
    // A write whose destination is not provably a local is a write the caller might see.
    let escapes = |addr: &Operand| match addr {
        Operand::Value(v) => !local.contains(v),
        _ => true,
    };
    match k {
        InstKind::Store { addr, vol, .. } => {
            if *vol == Volatility::Volatile {
                return Some("performs a volatile store (020 §4.2)".to_string());
            }
            escapes(addr).then(|| "stores through an address that is not a local".to_string())
        }
        InstKind::StoreBits { addr, .. } => {
            escapes(addr).then(|| "stores bits through an address that is not a local".to_string())
        }
        InstKind::CopyMem { dst, .. } | InstKind::SetMem { dst, .. } => escapes(dst)
            .then(|| "writes a block through an address that is not a local".to_string()),
        InstKind::Opaque { .. } => {
            Some("contains inline asm or an unmodeled construct".to_string())
        }
        // **A read of caller-visible memory is claim 2 as much as a write is**, and refusing
        // only writes was the first review's defect one indirection out: `tick(x); return g`
        // against `r = g; tick(x); return r` reads the global on opposite sides of a call
        // that may write it, and the two versions return different values. The effect
        // sequence says the call happened in the same place; it says nothing about how the
        // callee's writes interleave with this function's reads. Found by review.
        InstKind::Assign {
            rv: RValue::Load { addr, .. },
            ..
        } if escapes(addr) => Some("loads through an address that is not a local".to_string()),
        InstKind::Assign {
            rv: RValue::LoadBits { addr, .. },
            ..
        } if escapes(addr) => Some("loads bits through an address that is not a local".to_string()),
        InstKind::VaStart { .. } | InstKind::VaCopy { .. } | InstKind::VaEnd { .. } => {
            Some("manipulates a variadic argument list".to_string())
        }
        InstKind::Call { callee, .. } => match callee {
            Callee::Indirect(_) => Some("makes an indirect call".to_string()),
            Callee::Direct(id) => {
                let Some(f) = m.funcs.iter().find(|f| f.id == *id) else {
                    return Some("calls a function that is not in the module".to_string());
                };
                if f.body == Body::Declared {
                    // **No longer a refusal.** A call to a body-less non-pure function is
                    // now in the state's effect sequence (`EffectKind::Call`, with its
                    // arguments), and the comparison below decides §1.1's third claim over
                    // it. What is *not* decided is what the callee did to memory — but that
                    // is claim 2, and every version of it reaches this function through the
                    // store instructions above.
                    return None;
                }
                if !seen.contains(id) {
                    seen.push(*id);
                    queue.push(*id);
                }
                None
            }
        },
        InstKind::Assign {
            rv:
                RValue::Load {
                    vol: Volatility::Volatile,
                    ..
                },
            ..
        } => Some("performs a volatile load (020 §4.2)".to_string()),
        _ => None,
    }
}

/// **Which fidelities may carry an `Equivalent`** — §1.2 names exactly two.
///
/// > "for a function with an unbounded loop, the result is `Equivalent { fidelity: Bounded }`
/// > — a statement about inputs within the bound, not a proof."
///
/// `Approximated` is 023 §7's phrase for *a deliberate lie about semantics*, and `Unknown`
/// for *the engine does not know and cannot bound its ignorance*. Neither is a thing to build
/// a blessing on, and both were reachable: an unmodeled void call gave
/// `Equivalent { Approximated }`.
///
/// **And `Bounded` only when the bound is a loop bound.** A run that hit `max_forks` or
/// `max_states` dropped a sibling path and degraded the survivor to `Bounded` — correctly,
/// because that is all the engine can say — but the pairing argument below rests on both runs
/// being exhaustive, and a dropped path is exactly what it is not. The fixtures that found
/// this have no loop at all and disagree on 2^32 - 1 inputs; there is no bound within which
/// the blessing is true.
///
/// Keyed on the assumption's own text, which is fragile in the direction that fails *closed*:
/// a rename in the engine turns a would-be `Bounded` blessing into an `Unknown`.
fn blessable(f: Fidelity, assumptions: &[Assumption], no_returns: bool) -> Result<(), String> {
    match f {
        Fidelity::Exact => Ok(()),
        Fidelity::Bounded => {
            for a in assumptions {
                if a.fidelity == Fidelity::Bounded && !a.detail.starts_with("max_loop_iters") {
                    return Err(format!(
                        "the search was truncated by something other than a loop bound \
                         ({}), so paths were dropped rather than bounded",
                        a.detail
                    ));
                }
            }
            Ok(())
        }
        // **An unmodeled call can be a shared approximation — under conditions much narrower
        // than the ones first written here, which were wrong twice over.**
        //
        // The engine degrades to `Approximated` when it calls something with no body and no
        // model: it cannot say what that function did. The relational question is narrower —
        // did it do the *same* thing to both sides — and there are exactly three channels by
        // which a callee can reach this comparison:
        //
        // 1. **The effect sequence.** Compared position by position, callee and arguments,
        //    before this point is reached. Same call, same arguments, same place.
        // 2. **Memory.** `observable_beyond_the_return` refuses any load *or* store through an
        //    address that is not provably a local, and `compare_effects` refuses a pointer
        //    argument outright — so the callee has no caller-visible object it can reach, and
        //    this comparison no way to fail to notice one.
        // 3. **Its return value**, which is where both earlier attempts failed. An extern
        //    return is an input `link_inputs` cannot match, so `no_returns` is the condition:
        //    if either side has one the pair was already refused there, and if neither does
        //    there is nothing left for the callee to differ through.
        //
        // The two claims removed: that a matching effect sequence alone suffices — it does
        // not, the callee's writes interleave with this function's reads — and that a `pure`
        // callee is harmless — it is not, `pure` means no side effects, not a return value
        // independent of the arguments. `abs` is pure.
        //
        // What this still does not say: nothing here proved anything about the callee. The
        // verdict stays `Approximated`, envelope `proven` stays false, and 032 §3.1 refuses to
        // drop a test on it. The blessing is "these two agree", not "chiero understands this".
        //
        // Only for assumption kinds that account for a call. `OpaqueCode` is deliberately
        Fidelity::Approximated if no_returns => {
            for a in assumptions {
                if a.fidelity == Fidelity::Approximated
                    && !matches!(
                        a.kind,
                        AssumptionKind::UnmodeledCall | AssumptionKind::ModelApproximate
                    )
                {
                    return Err(format!(
                        "the run is Approximated for a reason other than a call whose \
                         arguments were compared ({:?}: {})",
                        a.kind, a.detail
                    ));
                }
            }
            Ok(())
        }
        other => Err(format!(
            "the run is {other:?}, and 041 §1.2 gives `Equivalent` only `Exact` or `Bounded`"
        )),
    }
}

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

/// What makes an input on one side *the same input* as one on the other.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum InputKey {
    /// An entry parameter, by position. **The only thing matched today** — `link_inputs` says
    /// why §1.2's extern-return symbols are not.
    Param(usize),
}

/// Pair up the two paths' symbolic inputs, returning one equality per matched pair.
///
/// `None` means an input on one side has no counterpart on the other. That is a refusal,
/// not a zero: leaving it unconstrained would let the solver pick different values for what
/// is meant to be the same input and report a divergence no caller can reproduce, and
/// dropping it silently would prove equivalence over a smaller input space than the caller
/// asked about.
fn link_inputs(a: &mut TermArena, sb: &State, sa: &State) -> Option<Link> {
    // **§1.2's "the same extern-return symbols" is *not* matched here, and the attempt is
    // worth recording.** It was keyed by (function, nth call along the path), on the stated
    // grounds that "the ordinal is the same thing the effect sequence orders by". It is not:
    // `InputOrigin::ExternReturn` is minted only for a call that has a destination, so the
    // input list counts *result-bearing* calls while the effect sequence counts *all* calls.
    // A discarded result shifts the numbering, and one version's `p(2)` was equated with the
    // other's `p(1)` — two functions returning the results of different calls, blessed.
    //
    // For a *pure* callee it was worse: those never enter the effect sequence, so nothing
    // checked that the nth call on each side was even passed the same arguments, and
    // `p(x) == p(x + 1)` was asserted outright. `pure` means no side effects, not a return
    // value independent of the arguments.
    //
    // A sound key is the call's position in the **effect sequence**, which needs an ordinal
    // the origin does not carry. Until it does, an extern return is an unmatched input, and
    // an unmatched input is a refusal.
    let params = |s: &State| -> Vec<(InputKey, Term)> {
        s.inputs()
            .iter()
            .filter_map(|(t, o)| match o {
                InputOrigin::Param { index, .. } => Some((InputKey::Param(*index), *t)),
                _ => None,
            })
            .collect()
    };
    let (pb, pa) = (params(sb), params(sa));
    // **`index` is not a key.** `chiero_make_symbolic` mints `InputOrigin::Param { index }`
    // where the index is a *byte offset within a buffer*, so two symbolized buffers give two
    // inputs numbered 0. `find`-first would then equate the second buffer's before-bytes to
    // the first buffer's after-bytes and leave the rest unconstrained — fabricating some
    // divergences and masking others. Latent when found by review; refused rather than left
    // to be discovered by a wrong answer.
    let unique = |v: &[(InputKey, Term)]| {
        let mut ix: Vec<InputKey> = v.iter().map(|(i, _)| i.clone()).collect();
        ix.sort();
        let n = ix.len();
        ix.dedup();
        ix.len() == n
    };
    if !unique(&pb) || !unique(&pa) {
        return None;
    }
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

/// **041 §1.1's third claim**: the ordered sequence of observable side effects.
///
/// Positional, because it is a *sequence*: §1.1 settles that "the order of two independent
/// extern calls **is** observable (C fixes it, and reordering visible I/O is not a safe
/// refactor)", so index `i` on one side is compared with index `i` on the other and a
/// difference in length is a difference at the first index one side does not have.
///
/// **The arguments are the load-bearing half.** Contract 6's rewrite swaps two calls to the
/// *same* function: the callee names match position for position, and only the arguments
/// distinguish the two programs. They are compared symbolically — a solver query asking
/// whether any argument can differ under this pair's input constraint — because the question
/// is whether the two calls agree for *every* input, which no concrete pair of values
/// answers.
fn compare_effects(
    solver: &mut TieredSolver,
    arena: &mut TermArena,
    pc: &mut PathCondition,
    sb: &State,
    sa: &State,
    link: &[(Term, Term)],
) -> Result<Divergent, String> {
    let (eb, ea) = (sb.effects().to_vec(), sa.effects().to_vec());
    let n = eb.len().max(ea.len());
    for i in 0..n {
        let (x, y) = (eb.get(i), ea.get(i));
        let describe = |e: Option<&chiero_exec::Effect>| e.map(|e| e.detail.clone());
        // A length difference, or two different events at the same point in the sequence.
        // Neither needs the solver: the pair is already known feasible, so any input
        // satisfying it takes both these paths and therefore produces both these sequences.
        let structural = match (x, y) {
            (None, _) | (_, None) => true,
            (Some(p), Some(q)) => p.kind != q.kind || p.detail != q.detail,
        };
        if structural {
            let m = match solver.check_path(arena, pc, &[]) {
                CheckResult::Sat(m) => m,
                _ => return Err("a pair known feasible stopped being feasible".to_string()),
            };
            let (key, w) = minimized_witness(solver, arena, pc, sb, link, m);
            return Ok(Some((
                key,
                Divergence::SideEffect {
                    index: i as u32,
                    before: describe(x),
                    after: describe(y),
                },
                w,
            )));
        }
        // Same event, same callee: do the arguments agree for every input?
        let (p, q) = (x.expect("checked"), y.expect("checked"));
        if p.args.len() != q.args.len() {
            return Err(format!(
                "the two versions call `{}` with a different number of arguments",
                p.detail
            ));
        }
        let mut differ: Option<Term> = None;
        for (ab, aa) in p.args.iter().zip(&q.args) {
            let (tb, ta) = match (ab, aa) {
                (Some(Value::Scalar(x)), Some(Value::Scalar(y))) => (*x, *y),
                // A pointer argument is comparable only up to §1.1's object bijection, and
                // `None` is an argument the engine could not evaluate. Refusing keeps the
                // rule this whole module runs on: never bless a question that was not asked.
                _ => {
                    return Err(format!(
                        "an argument to `{}` is a pointer or could not be evaluated; \
                         comparing it needs the object bijection of 041 §1.1",
                        p.detail
                    ));
                }
            };
            if arena.width(tb) != arena.width(ta) {
                return Err(format!("`{}` is called with differing widths", p.detail));
            }
            let eq = arena.eq(tb, ta);
            let ne = arena.not(eq);
            differ = Some(match differ {
                None => ne,
                Some(d) => arena.or(d, ne),
            });
        }
        let Some(d) = differ else { continue };
        match solver.check_path(arena, pc, &[d]) {
            CheckResult::Unsat => {}
            CheckResult::Unknown(r) => {
                return Err(format!(
                    "solver could not decide whether the arguments to `{}` agree: {r:?}",
                    p.detail
                ));
            }
            CheckResult::Sat(m) => {
                let (key, w) = minimized_witness_with(solver, arena, pc, sb, link, m, &[d]);
                return Ok(Some((
                    key,
                    Divergence::SideEffect {
                        index: i as u32,
                        before: describe(x),
                        after: describe(y),
                    },
                    w,
                )));
            }
        }
    }
    Ok(None)
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
    let mut current = m;
    for (tb, _) in link.iter() {
        let w = arena.width(*tb);
        // **Seeded from a model taken under the pins already chosen, not from the first one.**
        //
        // The first `Sat` predates every pin. Where the divergence set is not a product —
        // the review's fixture diverges at exactly `{(0, 200), (3, 7)}` — minimizing input 0
        // to 0 makes the first model's value for input 1 unreachable, and a search seeded
        // from it probes only values that cannot occur, finds every one Unsat, and reports
        // the seed. That is a witness the solver never agreed to: `(0, 7)`, at which both
        // versions return the same thing.
        //
        // Re-solving costs one query per input and makes the loop's invariant true instead
        // of asserted: `best` is always a value this input takes in a model of the pins.
        let mut best = match arena.eval(&current, *tb).map(|c| c.bits()) {
            Ok(v) => v,
            // The model does not bind this input. Rather than invent a value — the same
            // fabrication by a quieter route — take whatever a fresh solve says, and if
            // there is none, stop: `fixed` so far is still a real prefix.
            Err(_) => match pin(solver, arena, pc, link, &fixed, extra) {
                Some(m2) => match arena.eval(&m2, *tb).map(|c| c.bits()) {
                    Ok(v) => v,
                    Err(_) => break,
                },
                None => break,
            },
        };
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
                    best = match arena.eval(&m2, *tb).map(|c| c.bits()) {
                        Ok(v) => v.min(mid),
                        Err(_) => mid,
                    };
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
        // Re-solve with this input pinned too, so the next iteration's seed is achievable.
        match pin(solver, arena, pc, link, &fixed, extra) {
            Some(m2) => current = m2,
            // Unreachable if `best` is genuinely achievable; if it is not, stopping here
            // leaves a shorter witness rather than a wrong one.
            None => break,
        }
    }
    // **The link is truncated to what was actually minimized.** The loop above can stop
    // early — a model that will not evaluate, a re-solve the solver cannot decide — and the
    // comment there says it "leaves a shorter witness rather than a wrong one". That was true
    // of `fixed` and not of the witness: `witness_for` indexed one entry per *link* and
    // panicked on the shorter vector. Found by review.
    let w = witness_for(arena, sb, &link[..fixed.len()], &fixed);
    (fixed.clone(), w)
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
