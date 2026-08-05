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
    let mut tests: IndexMap<TestId, Vec<SelectionReason>> = IndexMap::new();
    let mut reasons: Vec<String> = Vec::new();

    let add = |t: TestId, r: SelectionReason, tests: &mut IndexMap<_, Vec<_>>| {
        let slot: &mut Vec<SelectionReason> = tests.entry(t).or_default();
        if !slot.contains(&r) {
            slot.push(r);
        }
    };

    // ① Coverage intersection (§2).
    for entity in impact.entities.keys() {
        let lines = program.lines_of(entity);
        if lines.is_empty() {
            // **Not "unaffected" — unmeasured.** An entity with no lines here has no coverage to
            // intersect, so nothing can say which tests reach it, and §4 sends the question to
            // the safety set rather than answering it.
            reasons.push(format!(
                "no source lines for {} `{}`, so no test could be excluded on its account",
                kind_of(entity),
                entity.name()
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
                    },
                    &mut tests,
                );
            }
        }
        if !measured {
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

    Selection {
        tests,
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
