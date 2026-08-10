//! **050 contracts 1 and 4b, over *every* operation — and the registry that makes "every"
//! mean something.**
//!
//! > 1. Every operation's response validates against the envelope schema, including error
//! >    responses (**checked for all operations by a schema test**).
//! >
//! > 4b. An always-degenerate implementation must fail this suite. One that always answers
//! >     `proven: false, fidelity: "Bounded"` while doing real work otherwise satisfies every
//! >     other contract here and can never license a negative claim. **The corpus therefore
//! >     contains, for each operation, at least one input reaching `fidelity: "Exact",
//! >     proven: true`.**
//!
//! Both contracts are quantified over operations, and a test file that names the operations
//! it happens to know about silently stops covering the next one added — in the direction
//! nobody looks. This project has made that mistake in the other direction already, which is
//! why `chiero-opt::PASSES` is a registry and 020 contract 44 is written over it.
//!
//! So the operations are enumerated here **and** [`every_operation_is_registered`] reads
//! `src/lib.rs` and fails if a public function returning an [`Envelope`] is missing from the
//! list. Adding an operation without a sample is a build failure, not a quiet gap.
//!
//! # `Exact` is not possible for every operation, and that is declared rather than assumed
//!
//! 4b's "for each operation" cannot hold for `select_tests`: coverage is historical, so a
//! selection is `Bounded` **by construction** and the crate's own documentation says so. An
//! operation like that declares why, and the test checks the declaration against reality in
//! *both* directions — an operation claiming it cannot be `Exact` that turns out to reach
//! `Exact` is as much a defect as the reverse, because it means the stated reason is wrong.

use chiero_diff::{Program, impact};
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use chiero_pp::{Config, preprocess_str};
use chiero_select::Suite;
use chiero_tool::{Envelope, Fidelity};
use std::path::PathBuf;

/// One 050 §3 operation, with enough representative responses to judge it by.
struct Op {
    /// The public function's name in `src/lib.rs`.
    name: &'static str,
    /// Representative responses, including the degenerate and error ones — contract 1 says
    /// "including error responses", which is where a schema usually first goes wrong.
    samples: fn() -> Vec<Envelope>,
    /// `None`: this operation can be `Exact`, and at least one sample must prove it (4b).
    /// `Some(why)`: it structurally cannot, and no sample may be.
    never_exact: Option<&'static str>,
}

const OPS: &[Op] = &[
    Op {
        name: "select_tests",
        samples: select_tests_samples,
        never_exact: Some(
            "coverage is historical: it records what the tests did on the previous build, \
             so no selection is proven for all inputs (032 §3, 050's own doc comment)",
        ),
    },
    Op {
        name: "select_tests_named",
        samples: select_tests_named_samples,
        never_exact: Some(
            "the same answer as `select_tests` with the caller's names attached: naming a test \
             cannot make a historical measurement into a proof",
        ),
    },
    Op {
        name: "impact_envelope",
        samples: impact_samples,
        never_exact: None,
    },
    Op {
        name: "expansion_sites_envelope",
        samples: expansion_sites_samples,
        never_exact: None,
    },
    Op {
        name: "explain_macro_expansion_envelope",
        samples: explain_samples,
        never_exact: None,
    },
    Op {
        name: "find_optimizations",
        samples: find_optimizations_samples,
        never_exact: None,
    },
    Op {
        name: "layout_envelope",
        samples: layout_samples,
        never_exact: None,
    },
    Op {
        name: "prove_equivalent_with_replay",
        samples: prove_equivalent_replay_samples,
        never_exact: None,
    },
    Op {
        name: "check_reachable",
        samples: check_reachable_samples,
        never_exact: None,
    },
    Op {
        name: "find_bugs",
        samples: find_bugs_samples,
        never_exact: None,
    },
    Op {
        name: "prove_equivalent",
        samples: prove_equivalent_samples,
        never_exact: None,
    },
    Op {
        name: "find_bugs_located",
        samples: find_bugs_located_samples,
        never_exact: None,
    },
];

// ---------------------------------------------------------------------------------------
// Samples
// ---------------------------------------------------------------------------------------

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/coverage")
}

fn select_tests_samples() -> Vec<Envelope> {
    let mut idx = CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("fixture");
    idx.record_outcome(TestId(0), TestOutcome::Passed);

    let before =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n").expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    let changed =
        chiero_tool::select_tests(&impact(&before, &after), &after, &idx, &Suite::default());
    // The empty answer, which is the one 050 §2 exists for: no change at all.
    let unchanged =
        chiero_tool::select_tests(&impact(&before, &before), &before, &idx, &Suite::default());
    // And with no coverage at all — the degenerate input.
    let no_coverage = chiero_tool::select_tests(
        &impact(&before, &after),
        &after,
        &CoverageIndex::default(),
        &Suite::default(),
    );
    vec![changed, unchanged, no_coverage]
}

/// The same three inputs through the naming variant, because a legend the caller supplied must
/// not move the fidelity, the blind spots or the count.
fn select_tests_named_samples() -> Vec<Envelope> {
    let mut idx = CoverageIndex::default();
    chiero_gcov::ingest_native_as(&mut idx, TestId(0), &corpus(), "t").expect("fixture");
    idx.record_outcome(TestId(0), TestOutcome::Passed);
    let names = [(TestId(0), "t".to_string())];

    let before =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 0;\n}\n").expect("parses");
    let after =
        Program::parse("t.c", "int main (void)\n{\n  M; M;\n  return 1;\n}\n").expect("parses");
    vec![
        chiero_tool::select_tests_named(
            &impact(&before, &after),
            &after,
            &idx,
            &Suite::default(),
            &names,
        ),
        chiero_tool::select_tests_named(
            &impact(&before, &before),
            &before,
            &idx,
            &Suite::default(),
            &names,
        ),
        chiero_tool::select_tests_named(
            &impact(&before, &after),
            &after,
            &CoverageIndex::default(),
            &Suite::default(),
            &names,
        ),
    ]
}

/// The same inputs as `find_bugs`, through the variant that resolves each finding's span.
///
/// **No map here on purpose.** These modules are built from hand-written CIR, which is exactly
/// the configuration in which the location was silently lost for months: with no map the two
/// keys must be *absent* rather than null, and every other field must be what `find_bugs`
/// produces. The located path with a real map is gated end to end in
/// `chiero-cli/tests/finding_location.rs`, which is where a source file exists to point at.
fn find_bugs_located_samples() -> Vec<Envelope> {
    let clean =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = add i32 %0, 1i32\n  ret %1\n}";
    let overflow = "func @f() -> i32 {\nentry:\n  .line 1\n  %0 = add i32 2147483647i32, 1i32 signed\n  ret %0\n}";
    let out = vec![
        chiero_tool::find_bugs_located(&m(clean), &chiero_tool::BugCfg::new("f"), None),
        chiero_tool::find_bugs_located(&m(overflow), &chiero_tool::BugCfg::new("f"), None),
        chiero_tool::find_bugs_located(&m(clean), &chiero_tool::BugCfg::new("nosuch"), None),
    ];
    // **Absent, not null.** With no map there is no file to name, and a `file: (none)` under
    // every finding would train a reader to skip the field on the reports where it is the
    // whole point.
    for e in &out {
        let v: serde_json::Value = serde_json::from_str(&e.to_json()).expect("valid JSON");
        if let Some(fs) = v["result"]["findings"].as_array() {
            for f in fs {
                assert!(
                    f.get("file").is_none() && f.get("line").is_none(),
                    "a finding built with no source map still carries a location: {f}"
                );
            }
        }
    }
    out
}

fn impact_samples() -> Vec<Envelope> {
    let before = Program::parse(
        "geom.c",
        "#define SCALE(x) ((x) * 2)\nint area (int w) { return SCALE (w) * w; }\n",
    )
    .expect("parses");
    let after = Program::parse(
        "geom.c",
        "#define SCALE(x) ((x) * 3)\nint area (int w) { return SCALE (w) * w; }\n",
    )
    .expect("parses");
    vec![
        // A macro edit reaching the function that expands it.
        chiero_tool::impact_envelope(&before, &after),
        // The empty answer: nothing changed at all.
        chiero_tool::impact_envelope(&before, &before),
    ]
}

fn expansion_sites_samples() -> Vec<Envelope> {
    let src = "#define M(v) ((v) + 1)\nint a (int x) { return M (x); }\n\
               int b (int x) { return M (x) + M (x); }\n";
    let tu = preprocess_str("f.c", src, Config::default());
    vec![
        // Complete.
        chiero_tool::expansion_sites_envelope(&tu.source_map, "M", None, 50),
        // A page of a larger population.
        chiero_tool::expansion_sites_envelope(&tu.source_map, "M", None, 2),
        // A macro that does not exist — the error-shaped response.
        chiero_tool::expansion_sites_envelope(&tu.source_map, "NOPE", None, 50),
    ]
}

fn explain_samples() -> Vec<Envelope> {
    let src = "#define INNER(v) ((v) + 1)\n#define OUTER(v) (INNER (v) * 2)\n\
               int a (int x) { return OUTER (x); }\nint b (int x) { return x; }\n";
    let tu = preprocess_str("f.c", src, Config::default());
    vec![
        // A real chain.
        chiero_tool::explain_macro_expansion_envelope(&tu.source_map, "f.c", 3, None),
        // A line with no macro on it: a *proven* empty answer.
        chiero_tool::explain_macro_expansion_envelope(&tu.source_map, "f.c", 4, None),
        // A file nobody has heard of: an empty answer that proves nothing.
        chiero_tool::explain_macro_expansion_envelope(&tu.source_map, "other.c", 3, None),
    ]
}

fn m(body: &str) -> chiero_cir::Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .expect("fixture parses")
}

const DOUBLE: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = mul i32 %0, 2i32
  ret %1
}";

const SHIFTED: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = shl i32 %0, 1i32
  ret %1
}";

const TRIPLED: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = mul i32 %0, 3i32
  ret %1
}";

fn find_optimizations_samples() -> Vec<Envelope> {
    // `if (x > 0) { if (x > 0) ... }` — the inner test is decided by the outer.
    let nested = "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = cmp sgt i32 %0, 0i32\n                    br %1, bb1, bb4\nbb1:\n  .line 2\n  %2 = cmp sgt i32 %0, 0i32\n                    br %2, bb2, bb3\nbb2:\n  .line 3\n  ret 1i32\nbb3:\n  .line 4\n                    ret 2i32\nbb4:\n  .line 5\n  ret 3i32\n}";
    let plain = "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  ret %0\n}";
    vec![
        chiero_tool::find_optimizations(&m(nested), &chiero_opt::opportunity::OppCfg::new("f")),
        // Nothing to propose, from a finished run: the one place an empty answer here is real.
        chiero_tool::find_optimizations(&m(plain), &chiero_opt::opportunity::OppCfg::new("f")),
        // The error-shaped response.
        chiero_tool::find_optimizations(&m(plain), &chiero_opt::opportunity::OppCfg::new("nope")),
    ]
}

fn layout_samples() -> Vec<Envelope> {
    use chiero_opt::locality::{Field, LocalityCfg, Record};
    let padded = Record {
        tag: "p".into(),
        size: 24,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "a".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "big".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
        ],
    };
    let wire = Record {
        tag: "hdr".into(),
        packed: true,
        ..padded.clone()
    };
    vec![
        chiero_tool::layout_envelope(std::slice::from_ref(&padded), &LocalityCfg::default()),
        // A layout that escapes: proposals, all advisory.
        chiero_tool::layout_envelope(&[wire], &LocalityCfg::default()),
        // The empty answer: no records at all.
        chiero_tool::layout_envelope(&[], &LocalityCfg::default()),
    ]
}

fn prove_equivalent_replay_samples() -> Vec<Envelope> {
    let d = std::env::temp_dir().join(format!("chiero-ops-replay-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    let before = d.join("b.c");
    let after = d.join("a.c");
    std::fs::write(&before, "int f (int x) { return x * 2; }\n").expect("write");
    std::fs::write(&after, "int f (int x) { return x * 3; }\n").expect("write");
    let sources = chiero_tool::ReplaySources {
        before: before.clone(),
        after: after.clone(),
        entry: "f".into(),
        scratch: d.clone(),
        flags: Vec::new(),
    };
    let cfg = chiero_opt::EquivCfg::new("f");
    let out = vec![
        // No sources: the same answer prove_equivalent gives.
        chiero_tool::prove_equivalent_with_replay(
            &m(DOUBLE),
            &m(TRIPLED),
            &cfg,
            None,
            chiero_tool::ReplayPolicy::EmitOnly,
        ),
        // Emitted and not run — 050 contract 11's default.
        chiero_tool::prove_equivalent_with_replay(
            &m(DOUBLE),
            &m(TRIPLED),
            &cfg,
            Some(&sources),
            chiero_tool::ReplayPolicy::EmitOnly,
        ),
    ];
    // **`ReplayPolicy::Run` is deliberately not sampled here, and the reason is a real
    // boundary in 001 §5's determinism claim.**
    //
    // What chiero *computes* is reproducible: the verdict, the witness, and the text of the
    // emitted program. Whether a compiler on a loaded machine finishes inside the wall-clock
    // limit is a *measurement*, not a computation, and it varies. Sampling it here made
    // `every_operation_is_deterministic` fail intermittently under the full workspace run and
    // pass alone — which is worse than a consistent failure, because it teaches a reader to
    // re-run rather than to look.
    //
    // The Run path is covered by `replay_scope.rs` and `prove_equivalent.rs`, where a
    // non-reproducible outcome is the subject rather than a nuisance.
    out
}

fn check_reachable_samples() -> Vec<Envelope> {
    let branch = "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = cmp eq i32 %0, 0i32\n                    br %1, bb1, bb2\nbb1:\n  .line 3\n  ret 1i32\nbb2:\n  .line 5\n  ret 2i32\n}";
    let dead = "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = cmp ne i32 %0, %0\n                  br %1, bb1, bb2\nbb1:\n  .line 3\n  ret 1i32\nbb2:\n  .line 5\n  ret 2i32\n}";
    vec![
        // Reachable, with the input that gets there.
        chiero_tool::check_reachable(&m(branch), &chiero_tool::BugCfg::new("f"), 3),
        // Proven unreachable — the exhaustive case.
        chiero_tool::check_reachable(&m(dead), &chiero_tool::BugCfg::new("f"), 3),
        // Nothing was asked: no code on that line.
        chiero_tool::check_reachable(&m(branch), &chiero_tool::BugCfg::new("f"), 4242),
    ]
}

fn find_bugs_samples() -> Vec<Envelope> {
    let clean =
        "func @f(%0: i32) -> i32 {\nentry:\n  .line 1\n  %1 = add i32 %0, 1i32\n  ret %1\n}";
    let overflow = "func @f() -> i32 {\nentry:\n  .line 1\n  %0 = add i32 2147483647i32, 1i32 signed\n  ret %0\n}";
    vec![
        // Finished and clean — the one case an empty list is an answer.
        chiero_tool::find_bugs(&m(clean), &chiero_tool::BugCfg::new("f")),
        // A real defect.
        chiero_tool::find_bugs(&m(overflow), &chiero_tool::BugCfg::new("f")),
        // The error-shaped response: an entry that is not there.
        chiero_tool::find_bugs(&m(clean), &chiero_tool::BugCfg::new("nosuch")),
    ]
}

fn prove_equivalent_samples() -> Vec<Envelope> {
    let mut out = Vec::new();
    // The undecided answer needs no backend, so it is always sampled.
    out.push(chiero_tool::prove_equivalent(
        &m(DOUBLE),
        &m(SHIFTED),
        &chiero_opt::EquivCfg::lite("f"),
    ));
    // An entry that is in neither module — the error-shaped response.
    out.push(chiero_tool::prove_equivalent(
        &m(DOUBLE),
        &m(SHIFTED),
        &chiero_opt::EquivCfg::lite("nosuch"),
    ));
    let cfg = chiero_opt::EquivCfg::new("f");
    if cfg.backend.is_some() {
        out.push(chiero_tool::prove_equivalent(&m(DOUBLE), &m(SHIFTED), &cfg));
        out.push(chiero_tool::prove_equivalent(&m(DOUBLE), &m(TRIPLED), &cfg));
    }
    out
}

// ---------------------------------------------------------------------------------------
// The contracts
// ---------------------------------------------------------------------------------------

/// **The registry is the test.** An operation added to `src/lib.rs` and not to [`OPS`] fails
/// here, so "every operation" in contracts 1 and 4b stays true without anyone remembering.
///
/// Reads the source rather than reflecting, because Rust has no reflection and a list that
/// checks itself against another hand-written list checks nothing.
#[test]
fn every_operation_is_registered() {
    let src = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
        .expect("this crate's own source");

    // `pub fn name(` ... `) -> Envelope {`, over a signature that spans lines.
    let mut found: Vec<String> = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // Column 0 only. An indented `pub fn` is an inherent method — `Envelope::new`
        // returns an `Envelope` and is not an operation.
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let Some(name) = rest.split('(').next().map(str::to_string) else {
            continue;
        };
        // Walk forward to the end of the signature.
        let returns_envelope = lines[i..]
            .iter()
            .take_while(|l| !l.contains(" {") || l.contains("-> Envelope"))
            .take(20)
            .any(|l| l.contains("-> Envelope"));
        if returns_envelope {
            found.push(name);
        }
    }

    assert!(
        !found.is_empty(),
        "the scan found no operations at all, which means it is broken rather than that \
         there are none"
    );
    let registered: Vec<&str> = OPS.iter().map(|o| o.name).collect();
    let missing: Vec<&String> = found
        .iter()
        .filter(|f| !registered.contains(&f.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these operations return an Envelope and are not in OPS, so contracts 1 and 4b do \
         not cover them: {missing:?}"
    );
    let stale: Vec<&&str> = registered
        .iter()
        .filter(|r| !found.contains(&r.to_string()))
        .collect();
    assert!(
        stale.is_empty(),
        "these are registered and no longer exist: {stale:?}"
    );
}

/// **Contract 1: every response validates against the envelope schema, error responses
/// included.**
#[test]
fn every_response_has_the_envelope_shape() {
    for op in OPS {
        let samples = (op.samples)();
        assert!(
            !samples.is_empty(),
            "{}: an operation with no samples is an operation with no coverage",
            op.name
        );
        for (i, env) in samples.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(&env.to_json())
                .unwrap_or_else(|e| panic!("{} sample {i}: not valid JSON: {e}", op.name));
            for key in ["result", "fidelity", "proven", "assumptions", "blind_spots"] {
                assert!(
                    !v[key].is_null(),
                    "{} sample {i}: the envelope is missing `{key}`: {v}",
                    op.name
                );
            }
            assert!(
                v["proven"].is_boolean(),
                "{} sample {i}: `proven` must be a boolean: {v}",
                op.name
            );
            assert!(
                v["blind_spots"].is_array() && v["assumptions"].is_array(),
                "{} sample {i}: the qualifications must be lists, so a reader can count them",
                op.name
            );
        }
    }
}

/// **Contract 2, at the operation surface rather than on the type.**
///
/// `envelope.rs` proves the invariant over the constructor. This proves that no operation
/// found a way around it — which is the claim contract 2 actually makes ("over all
/// operations and all corpus inputs").
#[test]
fn no_operation_is_proven_without_being_exact() {
    for op in OPS {
        for (i, env) in (op.samples)().iter().enumerate() {
            assert_eq!(
                env.proven,
                env.fidelity == Fidelity::Exact,
                "{} sample {i}: proven={} with fidelity={:?}",
                op.name,
                env.proven,
                env.fidelity
            );
        }
    }
}

/// **Contract 4b: an always-degenerate implementation must fail this suite.**
///
/// Every operation that *can* license a positive claim must be shown doing it on at least one
/// input, and every operation that cannot must have said why — checked in both directions,
/// because an operation that claims it can never be `Exact` and then is has a wrong reason
/// written down, which is worse than none.
#[test]
fn each_operation_reaches_exact_or_declares_why_it_cannot() {
    // **This is a claim about the implementation, and it needs a machine that can decide.**
    // `prove_equivalent` reaches `Exact` by *proving* two functions agree over every input;
    // with no backend on `PATH` that query comes back `Unknown` and no sample here can be
    // proven — which is 022 contract 2 rather than a degenerate implementation, and telling
    // them apart is what this test exists for. CI runs one leg with z3 so it is not skipped
    // everywhere. (`never_exact` is deliberately *not* the answer: an operation that declares
    // it can never be `Exact` and then is has a wrong reason written down, and "this machine
    // has no solver" is not a property of the operation.)
    if chiero_solver::SmtLib::discover().is_none() {
        eprintln!("skipping: contract 4b needs a solver to be reachable (022 contract 2)");
        return;
    }
    let mut report = Vec::new();
    for op in OPS {
        let samples = (op.samples)();
        let exact = samples.iter().filter(|e| e.proven).count();
        report.push(format!("{}: {exact}/{} exact", op.name, samples.len()));
        match op.never_exact {
            None => assert!(
                exact > 0,
                "{}: no sample reaches Exact, so this operation can never license a \
                 negative claim — contract 4b's degenerate implementation would pass \
                 everything else",
                op.name
            ),
            Some(why) => assert_eq!(
                exact, 0,
                "{}: declared unable to reach Exact because {why}, but {exact} sample(s) \
                 did — the declaration is wrong",
                op.name
            ),
        }
    }
    println!(
        "050 contract 4b — per-operation Exact rate:\n  {}",
        report.join("\n  ")
    );
}

/// **An unproven answer always carries something to read.**
///
/// 050 §2: a consumer must be "structurally unable to miss the qualification". A response
/// with `proven: false` and nothing anywhere is a bare no — the exact shape an LLM reads as
/// "nothing to report".
///
/// **Truncation counts**, and finding out that it had to was the point of writing this over
/// every operation. `expansion_sites_envelope` on a page of a larger population is `Bounded`
/// with empty `assumptions` and empty `blind_spots`: the qualification is real and complete
/// and lives in `truncation`, which 050 §2 gives its own field precisely so a reader does not
/// have to find it in prose. A test that demanded a blind spot there would have been
/// demanding the qualification be said twice.
#[test]
fn every_unproven_answer_says_what_makes_it_unproven() {
    for op in OPS {
        for (i, env) in (op.samples)().iter().enumerate() {
            if env.proven {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
            let truncated = v["truncation"]["truncated"] == serde_json::json!(true);
            assert!(
                !env.assumptions.is_empty() || !env.blind_spots.is_empty() || truncated,
                "{} sample {i}: fidelity {:?}, proven false, and nothing anywhere in the \
                 envelope says why: {v}",
                op.name,
                env.fidelity
            );
            // And the human rendering must carry it too — the JSON is not what a person reads.
            let r = env.render();
            assert!(
                r.contains("not proven") || r.contains("within") || r.contains("showing"),
                "{} sample {i}: the rendering reads as an unqualified answer:\n{r}",
                op.name
            );
        }
    }
}

/// **001 §5: the same input renders byte-identically.** Over every operation, not a sample.
#[test]
fn every_operation_is_deterministic() {
    for op in OPS {
        let a = (op.samples)();
        let b = (op.samples)();
        assert_eq!(
            a.len(),
            b.len(),
            "{}: a different number of samples",
            op.name
        );
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(
                x.determinism_key(),
                y.determinism_key(),
                "{} sample {i} is not reproducible:\n{}\n---\n{}",
                op.name,
                x.to_json(),
                y.to_json()
            );
        }
    }
}

/// **050 contract 14: no operation writes to the analysed repository.**
///
/// > 14. No operation writes to the analysed repository — verified by hashing the tree before
/// >     and after.
///
/// The contract's own method, over every registered operation rather than a chosen one. This
/// is the contract that would be quietly broken by a cache, a scratch file written "next to
/// the input", or a `.gcda` merged in place — none of which look like writing to a repository
/// at the call site where they are added.
///
/// Path, length and modification time rather than content: a write changes at least one of
/// the three, and reading every file in the tree on every test run would make this the
/// slowest test in the workspace for no extra strength.
#[test]
fn no_operation_writes_to_the_tree() {
    fn stamp(root: &std::path::Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(root) else {
            return;
        };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // `target` and `.git` churn for reasons that have nothing to do with an
            // operation, and hashing them would make the test flake rather than fail.
            if name == "target" || name == ".git" {
                continue;
            }
            match std::fs::symlink_metadata(&p) {
                Ok(md) if md.is_dir() => stamp(&p, out),
                Ok(md) => out.push(format!(
                    "{} {} {:?}",
                    p.display(),
                    md.len(),
                    md.modified().ok()
                )),
                Err(_) => {}
            }
        }
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let mut before = Vec::new();
    stamp(&root, &mut before);
    assert!(
        before.len() > 100,
        "the stamp found {} files, which means it is walking the wrong tree",
        before.len()
    );

    for op in OPS {
        let _ = (op.samples)();
    }

    let mut after = Vec::new();
    stamp(&root, &mut after);

    let changed: Vec<&String> = after.iter().filter(|a| !before.contains(a)).collect();
    let vanished: Vec<&String> = before.iter().filter(|b| !after.contains(b)).collect();
    assert!(
        changed.is_empty() && vanished.is_empty(),
        "an operation touched the tree.\n  new or modified: {changed:?}\n  gone: {vanished:?}"
    );
}

/// **An empty proposal list is only a proof if the search could see the code.**
///
/// 050 contract 3 imposes exactly this on `find_bugs` — *"the string 'no defects found' never
/// appears unqualified"*, because an empty list is wrong precisely when the search did not
/// finish. `find_optimizations` answers the same shape of question and had no such rule: its
/// fidelity was chosen from `any_advisory` alone, so **every** empty list came back `Exact` and
/// `proven`.
///
/// Measured on the CLI 2026-08-10, the pair that shows it:
///
/// | program | proposals | verdict |
/// |---|---|---|
/// | `if (x > 10 && x < 5)` | 2 | `Exact`, proven — correct |
/// | the same dead branch behind an unmodelled `long double` call | **0** | `Exact`, **proven** |
///
/// The second is a function that provably contains a dead branch, and chiero says there are no
/// optimizations *and that this holds for all inputs*.
#[test]
fn an_empty_proposal_list_is_not_proven_when_the_run_could_not_see_the_code() {
    // `g` has no body and `FpToSi 80 -> 32` is unmodelled, so the branch condition is a value
    // the engine never forms — the dead branch is real and invisible.
    let src = "\
func @f() -> i32 {
entry:
  .line 1
  %0 = call @g()
  %1 = fptosi f80 %0 to i32
  %2 = cmp sgt i32 %1, 10i32
  br %2, bb1, bb2
bb1:
  .line 2
  ret 1i32
bb2:
  .line 3
  ret 0i32
}

func @g() -> f80";
    let m = chiero_cir::text::parse(src).unwrap_or_else(|e| panic!("fixture: {e:?}"));
    let env = chiero_tool::find_optimizations(&m, &chiero_opt::opportunity::OppCfg::new("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["count"], 0, "nothing is detectable here: {v}");
    assert!(
        !env.proven,
        "an empty list from a run that could not model the branch condition is not a proof \
         that there is nothing to find: {v}"
    );
}

/// **A layout analysis that could not state a record is not a proof about that file.**
///
/// 041 §2 makes `Exact` mean something specific: *"Only `fidelity == Exact` is a proof, and only
/// `Exact` licenses dropping a test in 032"*. `layout_envelope` returned `Exact` unconditionally
/// — including when `fields_complete` was false for a record, which is the case its own comment
/// describes: *"a record chiero could not judge is not a record with nothing to find … a padding
/// number summed over the members that are left is not a smaller number but a wrong one"*.
///
/// The blind spot naming the record was already there; the *machine-readable* claim beside it
/// still said proven. This is `find_optimizations`' defect of the same day, one operation over.
#[test]
fn a_layout_with_an_unstatable_record_is_not_proven() {
    use chiero_opt::locality::Record;
    let cfg = chiero_opt::locality::LocalityCfg {
        cache_line_bytes: 64,
        counts: Vec::new(),
    };
    let rec = |complete: bool| Record {
        tag: "S".into(),
        size: 8,
        align: 4,
        packed: false,
        externally_visible: false,
        fields: Vec::new(),
        fields_complete: complete,
    };

    let complete = rec(true);
    let env = chiero_tool::layout_envelope(std::slice::from_ref(&complete), &cfg);
    assert!(
        env.proven,
        "a fully stated record is a proof: {}",
        env.to_json()
    );

    let partial = rec(false);
    let env = chiero_tool::layout_envelope(std::slice::from_ref(&partial), &cfg);
    assert!(
        !env.proven,
        "a record whose field list cannot be stated leaves the padding number wrong, not \
         merely smaller — so the envelope must not claim a proof: {}",
        env.to_json()
    );
}
