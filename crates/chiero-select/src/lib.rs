//! `chiero-select` — which tests must run for this change? (032)
//!
//! ```text
//! ImpactSet ──▶ ① coverage intersection ──▶ candidates
//!                         │
//! CoverageIndex ──────────┘
//!                         ▼
//!               ② symbolic refinement (removal requires proof)
//!                         ▼
//!               ③ ∪ always-run safety set
//!                         ▼
//!               ④ ranking + justification ──▶ Selection
//! ```
//!
//! # Why the join works at all
//!
//! 032 §2, on the case the whole architecture exists for:
//!
//! > **For macro-body changes it is the whole trick.** The changed entity is a macro, which has no
//! > coverage lines of its own; but 031 §3.2 already converted that change into a set of
//! > *impacted functions*, and functions do have coverage. So the intersection is well-defined
//! > precisely because impact closure ran first. A tool that tried to intersect coverage with the
//! > diff directly would find nothing.
//!
//! # The direction
//!
//! Everything upstream over-approximates deliberately. **This is the first step that removes
//! anything**, so what it may not remove is the load-bearing part: a test with no coverage is
//! *unmeasured*, not unaffected. §4's safety set is unioned in unconditionally and is never
//! subject to refinement.
//!
//! Step ② — symbolic refinement, the only step that prunes — is not yet implemented. Its absence
//! is safe by construction: without it the selection is a superset, which is the direction that
//! never misses a regression.

use indexmap::IndexMap;

use chiero_diff::{Completeness, ImpactSet, Program};
use chiero_gcov::{CoverageIndex, TestId};

/// How much a proof is worth (023 §7).
///
/// **Only `Exact` spends.** A selection tool's entire safety argument is that everything
/// upstream over-approximates; a refinement that dropped a test on `Bounded` would undo all of
/// it, and would look identical in every case where it happened to be right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// Proven for all inputs.
    Exact,
    /// Proven within a bound — a loop unrolled to a depth, an array modelled to a size.
    Bounded,
    /// Modelled with an approximation somewhere in the chain.
    Approximated,
    /// The engine reached something it does not model.
    Unknown,
}

/// What a prover concluded about one entity's two versions (032 §3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Equivalence {
    /// Observationally identical — *at this fidelity*, which is what decides whether it counts.
    Equivalent { fidelity: Fidelity },
    /// A difference was found.
    Differs,
    /// The solver ran out of time. **Not a proof, and not a non-event**: it reduces confidence.
    TimedOut,
    /// No attempt was made — the default.
    NotAttempted,
}

/// 041's `prove_equivalent`, as a seam (032 §3.1).
///
/// The trait exists so the *discipline* can be built before the prover is: there is no way to
/// remove a test except by returning a verdict, and only one verdict removes anything.
pub trait Prover {
    fn prove_equivalent(&mut self, entity: &chiero_diff::Entity) -> Equivalence;
}

/// The default: attempts nothing, proves nothing, removes nothing.
///
/// **A seam that defaulted to attempting proofs would change the meaning of every existing
/// call.** This is `chiero-gcov`'s `MarchResolver` shape for the same reason: an extension point
/// whose default does not guess.
#[derive(Debug)]
pub struct NoProver;

impl Prover for NoProver {
    fn prove_equivalent(&mut self, _entity: &chiero_diff::Entity) -> Equivalence {
        Equivalence::NotAttempted
    }
}

/// A test that was a candidate and was removed, with the proof that justified it (032 §5).
///
/// **Every field is required.** §3: "Every dropped test records *why*, with the proof's fidelity,
/// so a selection can be audited after a regression escapes." An exclusion nobody can audit is
/// indistinguishable from a bug.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcludedTest {
    pub test: TestId,
    /// Which refinement removed it — "equivalence" for §3.1.
    pub refinement: String,
    /// The entity whose proof did it.
    pub entity: String,
    /// Always `Exact`; the type permits nothing else to reach here.
    pub fidelity: Fidelity,
}

/// Why one test is in the selection (032 §5).
///
/// **Every selected test carries at least one** (contract 15). A maintainer told to run 400 tests
/// must be able to ask why of any one of them, and a selection is only actionable if the answer
/// is per-test rather than per-run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionReason {
    /// It executed a line of an impacted entity.
    CoversEntity {
        entity: String,
        file: String,
        line: u32,
        /// 031's shortest path from the entity that actually changed. 0 means this test covers
        /// the change itself.
        distance: u32,
    },
    /// It is in the safety set (§4) and was never a candidate for removal.
    AlwaysRun { why: String },
}

/// How much of the selection rests on measurement (032 §5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Confidence {
    /// Every impacted entity had coverage, every test was measured, and the impact set was
    /// complete.
    #[default]
    Full,
    /// Something upstream could not be computed. **Named, not counted** — §4 requires the reason
    /// to appear in the report rather than a flag a reader has to interpret.
    Reduced { reasons: Vec<String> },
}

/// The tests to run, and why (032 §5).
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// Test → its reasons, in deterministic order.
    pub tests: IndexMap<TestId, Vec<SelectionReason>>,
    /// Tests that were candidates and were removed, each with the proof that justified it.
    pub excluded: Vec<ExcludedTest>,
    pub confidence: Confidence,
}

/// What the caller knows and the index cannot (032 §4).
///
/// **Both fields are the caller's because neither is derivable from the index.** The index holds
/// what was *ingested*: it cannot know a test exists in the tree and was never measured, and it
/// cannot know whether the sources still hash to what they did — `validity` needs a tree root,
/// which is 030 §7's own division of labour.
///
/// The defaults are the honest ones for a caller that has neither: no suite list, and a fresh
/// index. A caller that omits the suite gets no contract-10 selections, which is a smaller
/// answer — so this is the one place in the crate where a default is *not* the safe direction,
/// and it is named here rather than discovered.
#[derive(Clone, Debug)]
pub struct Suite {
    /// Every test in the tree, whether or not the index has heard of it.
    pub tests: Vec<TestId>,
    /// What 030 §7 says about the index.
    pub validity: chiero_gcov::Validity,
}

impl Default for Suite {
    fn default() -> Self {
        Suite {
            tests: Vec::new(),
            validity: chiero_gcov::Validity::Fresh,
        }
    }
}

/// Select the tests that must run for a change (032 §1).
///
/// Equivalent to [`select_with`] with a caller that knows nothing beyond the index.
///
/// `program` is the **new** side of the comparison: it is what says which lines an entity now
/// occupies, and therefore what the coverage index is asked about.
pub fn select(impact: &ImpactSet, program: &Program, coverage: &CoverageIndex) -> Selection {
    select_with(impact, program, coverage, &Suite::default())
}

/// Select the tests that must run, given what the caller knows about the suite (032 §1, §4).
pub fn select_with(
    impact: &ImpactSet,
    program: &Program,
    coverage: &CoverageIndex,
    suite: &Suite,
) -> Selection {
    select_refined(impact, program, coverage, suite, &mut NoProver)
}

/// The same, with 032 §3.1's equivalence refinement — **the only step that removes anything**.
///
/// §3's rule, and the whole safety argument: *"A test may be dropped only on an `Exact` proof.
/// `Bounded`, `Approximated`, `Unknown`, or a solver timeout all mean keep."*
///
/// Refinement removes an *entity* rather than a test, which is why §3.1 calls it the
/// highest-leverage one: the tests that came only from that entity go with it, and any test also
/// selected for another reason stays.
pub fn select_refined(
    impact: &ImpactSet,
    program: &Program,
    coverage: &CoverageIndex,
    suite: &Suite,
    prover: &mut dyn Prover,
) -> Selection {
    let mut tests: IndexMap<TestId, Vec<SelectionReason>> = IndexMap::new();
    let mut reasons: Vec<String> = Vec::new();

    let add = |t: TestId, r: SelectionReason, tests: &mut IndexMap<_, Vec<_>>| {
        let slot: &mut Vec<SelectionReason> = tests.entry(t).or_default();
        if !slot.contains(&r) {
            slot.push(r);
        }
    };

    // **§3.1, before the intersection.** Refinement removes an *entity*, so proving one
    // equivalent removes every test that would have come from it — rather than removing tests one
    // at a time afterwards and hoping the bookkeeping agrees.
    let mut proven: Vec<&chiero_diff::Entity> = Vec::new();
    let mut timed_out = 0usize;
    for entity in impact.entities.keys() {
        match prover.prove_equivalent(entity) {
            // The one verdict that spends.
            Equivalence::Equivalent {
                fidelity: Fidelity::Exact,
            } => proven.push(entity),
            Equivalence::TimedOut => timed_out += 1,
            // Bounded, Approximated, Unknown, Differs, NotAttempted — all mean keep.
            _ => {}
        }
    }
    if timed_out > 0 {
        reasons.push(format!(
            "{timed_out} equivalence proof(s) timed out; every affected test was kept"
        ));
    }

    // ① Coverage intersection (§2).
    for entity in impact.entities.keys() {
        if proven.contains(&entity) {
            continue;
        }
        let lines = program.lines_of(entity);
        if lines.is_empty() {
            // **A macro having no coverage lines is not a gap — it is 030 §1, measured.** gcov
            // records the line a macro was *used* on and never the macro's own, so expecting
            // coverage for a `Macro` entity is expecting something that cannot exist. 031 §3.2
            // already turned the macro change into the *functions* that expand it, and those are
            // what carry the coverage; reducing confidence here would make every macro edit
            // report a caveat that says nothing.
            //
            // Any other entity with no lines is a real gap: something changed that this program
            // cannot locate, so nothing can say which tests reach it.
            if !matches!(entity, chiero_diff::Entity::Macro { .. }) {
                reasons.push(format!(
                    "no source lines for {} `{}`, so no test could be excluded on its account",
                    kind_of(entity),
                    entity.name()
                ));
            }
            continue;
        }
        // **"The index has never heard of this file" is a different fact from "this entity has
        // no coverage", and it is almost always a path that was not resolved.** 030 stores paths
        // as gcov wrote them and leaves resolution to the caller — the right division, since
        // matching by basename here would conflate two files of one name in different
        // directories. But the failure mode is silent and flattering: every lookup misses, no
        // test is selected, and the result reads as an excellent reduction. It has happened three
        // times in this project. So it is named.
        if !coverage.files().any(|f| f == entity.file()) {
            reasons.push(format!(
                "`{}` is not in the coverage index at all — if it should be, the paths were not \
                 resolved against the build directory (030: paths are stored as gcov wrote them)",
                entity.file()
            ));
            continue;
        }

        let mut measured = false;
        for line in lines {
            let Some(covering) = coverage.tests_for_line(entity.file(), *line) else {
                continue;
            };
            measured = true;
            for t in covering {
                add(
                    t,
                    SelectionReason::CoversEntity {
                        entity: entity.name().to_string(),
                        file: entity.file().to_string(),
                        line: *line,
                        distance: impact.entities[entity].distance,
                    },
                    &mut tests,
                );
            }
        }
        if !measured && !matches!(entity, chiero_diff::Entity::Macro { .. }) {
            reasons.push(format!(
                "no coverage recorded for {} `{}` in {}",
                kind_of(entity),
                entity.name(),
                entity.file()
            ));
        }
    }

    // ② Symbolic refinement — the only step that removes anything — is not implemented. Its
    // absence leaves a superset, which is the safe direction; §3 requires a *proof* before a test
    // may be dropped, and there is nothing here that could produce one.

    // ③ The always-run safety set (§4), unioned in unconditionally.
    for t in coverage.always_run() {
        add(
            t,
            SelectionReason::AlwaysRun {
                why: "no complete coverage: the run crashed, timed out, or recorded nothing".into(),
            },
            &mut tests,
        );
    }
    for t in coverage.tests() {
        // A test the index knows about but attributes no line to has never been measured against
        // this program.
        if coverage
            .files()
            .flat_map(|f| coverage.lines_of(f).into_iter().map(move |l| (f, l)))
            .any(|(f, l)| {
                coverage
                    .tests_for_line(f, l)
                    .is_some_and(|ts| ts.contains(&t))
            })
        {
            continue;
        }
        add(
            t,
            SelectionReason::AlwaysRun {
                why: "the index attributes no line to this test".into(),
            },
            &mut tests,
        );
    }

    // §4: a test in the tree that the index never heard of. **Never measured is not
    // unaffected**, and unlike every other trigger here the index cannot see this one at all.
    for t in &suite.tests {
        if !coverage.tests().contains(t) {
            add(
                *t,
                SelectionReason::AlwaysRun {
                    why: "present in the tree and absent from the index: never measured".into(),
                },
                &mut tests,
            );
        }
    }

    // §4: the index itself may not be believable (030 §7).
    match &suite.validity {
        chiero_gcov::Validity::Fresh => {}
        chiero_gcov::Validity::Stale { files } => {
            reasons.push(format!(
                "the coverage index is stale: {} changed since it was recorded",
                files.join(", ")
            ));
            // Every test that touched a stale file, and — because a stale index cannot be trusted
            // to say which those are — every test it holds.
            for t in coverage.tests() {
                add(
                    t,
                    SelectionReason::AlwaysRun {
                        why: "the coverage index is stale".into(),
                    },
                    &mut tests,
                );
            }
        }
        chiero_gcov::Validity::Partial { missing_tests } => {
            reasons.push(format!(
                "the coverage index cannot speak for {} test(s)",
                missing_tests.len()
            ));
            for t in missing_tests {
                add(
                    *t,
                    SelectionReason::AlwaysRun {
                        why: "the index has no complete coverage for this test".into(),
                    },
                    &mut tests,
                );
            }
        }
    }

    // §4: an incomplete impact set is a gap, and every gap widens the selection.
    if let Completeness::Partial {
        unparsed_files,
        unresolved_calls,
        unknown_configs,
        address_taken_fallbacks,
    } = &impact.completeness
    {
        if !unparsed_files.is_empty() {
            reasons.push(format!(
                "the impact set is partial: {} could not be parsed",
                unparsed_files.join(", ")
            ));
        }
        if *address_taken_fallbacks > 0 {
            reasons.push(format!(
                "{address_taken_fallbacks} indirect call site(s) reached by address-taken fallback"
            ));
        }
        if *unresolved_calls > 0 {
            reasons.push(format!("{unresolved_calls} unresolved call(s)"));
        }
        if !unknown_configs.is_empty() {
            reasons.push(format!(
                "condition(s) changed whose other configurations were not enumerated: {}",
                unknown_configs.join(", ")
            ));
        }
        // Nothing is known about the gap, so every measured test comes along.
        for t in coverage.tests() {
            add(
                t,
                SelectionReason::AlwaysRun {
                    why: "the impact set is incomplete".into(),
                },
                &mut tests,
            );
        }
    }

    // ④ Deterministic order, so two runs are comparable and two reports diffable (contract 16).
    tests.sort_keys();

    // §3's audit requirement: an exclusion nobody can audit is indistinguishable from a bug.
    let mut excluded: Vec<ExcludedTest> = Vec::new();
    for entity in &proven {
        for line in program.lines_of(entity) {
            for t in coverage
                .tests_for_line(entity.file(), *line)
                .unwrap_or_default()
            {
                // Only a test that nothing *else* selected is actually excluded.
                if tests.contains_key(&t) || excluded.iter().any(|e| e.test == t) {
                    continue;
                }
                excluded.push(ExcludedTest {
                    test: t,
                    refinement: "equivalence".into(),
                    entity: entity.name().to_string(),
                    fidelity: Fidelity::Exact,
                });
            }
        }
    }
    excluded.sort_by_key(|e| e.test.0);

    Selection {
        tests,
        excluded,
        confidence: if reasons.is_empty() {
            Confidence::Full
        } else {
            Confidence::Reduced { reasons }
        },
    }
}

fn kind_of(e: &chiero_diff::Entity) -> &'static str {
    match e {
        chiero_diff::Entity::Function { .. } => "function",
        chiero_diff::Entity::Global { .. } => "global",
        chiero_diff::Entity::Typedef { .. } => "typedef",
        chiero_diff::Entity::Record { .. } => "record",
        chiero_diff::Entity::EnumConst { .. } => "enum constant",
        chiero_diff::Entity::Macro { .. } => "macro",
    }
}

impl Selection {
    /// The tests in the order a maintainer should look at them (032 §5).
    ///
    /// **Closest first**: a test that covers the change itself outranks one three calls away, and
    /// a test kept only by the safety set sorts last — it is not evidence of anything, it is the
    /// absence of evidence. Ties break on `TestId`, so the order is total and two runs agree
    /// (contract 16).
    ///
    /// §5 specifies more inputs — change-class severity, how much of the change a test covers,
    /// execution count, estimated duration — and names the weights as configuration that the
    /// report must print. Those need data this crate does not yet carry; **distance alone is a
    /// deliberate subset, not the finished ranking**, and it is ordered so that adding the rest
    /// refines it rather than reversing it.
    pub fn ranked(&self) -> Vec<TestId> {
        let mut v: Vec<(u32, TestId)> = self.tests.iter().map(|(t, rs)| (score(rs), *t)).collect();
        v.sort_by_key(|(s, t)| (*s, t.0));
        v.into_iter().map(|(_, t)| t).collect()
    }

    /// Keep the top `n` ranked tests (032 §5.1).
    ///
    /// **Truncation is not refinement.** The dropped tests *were* selected — nothing proved them
    /// unnecessary — so the confidence drops and the report says how many went and where the
    /// cutoff fell. A budgeted run must never render as if it covered the impact.
    ///
    /// A budget that fits is not a caveat, and does not touch the confidence.
    pub fn budgeted(mut self, n: usize) -> Selection {
        let order = self.ranked();
        if order.len() <= n {
            return self;
        }
        let dropped = order.len() - n;
        let cutoff = n;
        let keep: Vec<TestId> = order.into_iter().take(n).collect();
        self.tests.retain(|t, _| keep.contains(t));
        let note = format!(
            "budget: {dropped} selected test(s) dropped below rank {cutoff}; they were not \
             proven unnecessary"
        );
        self.confidence = match self.confidence {
            Confidence::Full => Confidence::Reduced {
                reasons: vec![note],
            },
            Confidence::Reduced { mut reasons } => {
                reasons.push(note);
                Confidence::Reduced { reasons }
            }
        };
        self
    }

    /// The report (032 §5).
    ///
    /// **Reduction and safety always appear together** (contract 20). A selection tool's product
    /// is a claim that some tests need not run; a report showing only how many were excluded
    /// invites the one reading — "it works, we run fewer tests" — that its own output cannot
    /// falsify.
    pub fn render(&self) -> String {
        let ranked = self.ranked();
        let always: Vec<&TestId> = self
            .tests
            .iter()
            .filter(|(_, rs)| {
                rs.iter()
                    .all(|r| matches!(r, SelectionReason::AlwaysRun { .. }))
            })
            .map(|(t, _)| t)
            .collect();

        let mut out = format!("SELECTED: {} test(s)\n", ranked.len());
        for (i, t) in ranked.iter().enumerate() {
            let reasons = &self.tests[t];
            out.push_str(&format!(
                "{:3}. test {:<6} {}\n",
                i + 1,
                t.0,
                describe(reasons.first())
            ));
        }
        out.push_str(&format!(
            "ALWAYS-RUN: {} test(s) — never candidates for removal\n",
            always.len()
        ));
        if !self.excluded.is_empty() {
            out.push_str(&format!(
                "EXCLUDED (proof): {} test(s)\n",
                self.excluded.len()
            ));
            for e in &self.excluded {
                out.push_str(&format!(
                    "  test {:<6} {}: `{}` proven equivalent ({:?})\n",
                    e.test.0, e.refinement, e.entity, e.fidelity
                ));
            }
        }
        match &self.confidence {
            Confidence::Full => out.push_str("CONFIDENCE: Full\n"),
            Confidence::Reduced { reasons } => {
                out.push_str("CONFIDENCE: Reduced\n");
                for r in reasons {
                    out.push_str(&format!("  - {r}\n"));
                    if r.starts_with("budget:") {
                        out.push_str("  BUDGET: this run did not cover the impact\n");
                    }
                }
            }
        }
        out
    }
}

/// Lower sorts earlier. A safety-set-only test has no distance at all, so it sorts last.
fn score(reasons: &[SelectionReason]) -> u32 {
    reasons
        .iter()
        .filter_map(|r| match r {
            SelectionReason::CoversEntity { distance, .. } => Some(*distance),
            SelectionReason::AlwaysRun { .. } => None,
        })
        .min()
        .unwrap_or(u32::MAX)
}

fn describe(r: Option<&SelectionReason>) -> String {
    match r {
        None => "?".into(),
        Some(SelectionReason::CoversEntity {
            entity,
            file,
            line,
            distance,
        }) => format!("[distance {distance}] covers {entity} at {file}:{line}"),
        Some(SelectionReason::AlwaysRun { why }) => format!("[always-run] {why}"),
    }
}
