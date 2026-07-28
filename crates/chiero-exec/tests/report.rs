//! **The rendered report** — 023 contracts 12 and 14.
//!
//! Covers: 023 contract 12 (every degraded state has an assumption whose kind matches the
//! recorded cause **and whose text appears in the rendered report**), 023 contract 14
//! (`no bugs found` under a hit budget renders as "no bugs found within <bound>" and never
//! as "no bugs exist").
//!
//! Both contracts are about *text a person reads*, which is why they are golden tests
//! rather than assertions about structs: 023 §7 exists because "an LLM will read 'no bugs'
//! as 'safe'", and a run can carry every assumption correctly and still print a sentence
//! that overclaims.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: Default::default(),
        span: Span::DUMMY,
    }
}

fn func(blocks: Vec<Block>, ret: CTy) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret,
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: Span::DUMMY,
        }],
        ..Default::default()
    }
}

/// A module whose entry function calls a declared-but-undefined `name` — which is how
/// CIR spells an extern, modeled (`scanf`) or not.
fn calling(id: u32, name: &str) -> Module {
    let caller = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![Inst {
                kind: InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(id)),
                    args: vec![],
                },
                span: Span::DUMMY,
                generated: false,
            }],
            Terminator::Return(Some(i32c(0))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let ext = Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![caller, ext],
        ..Default::default()
    }
}

/// A loop cut by `max_loop_iters`: the run finds nothing and is `Bounded`.
fn bounded_run(a: &mut TermArena) -> (Module, Budget) {
    let m = func(
        vec![
            block(0, vec![], Terminator::Goto(BlockId(1))),
            block(
                1,
                vec![],
                Terminator::Br {
                    cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
                    t: BlockId(2),
                    f: BlockId(3),
                },
            ),
            block(2, vec![], Terminator::Goto(BlockId(1))),
            block(3, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        CTy::Int(32),
    );
    let b = Budget {
        max_loop_iters: 3,
        ..Budget::default()
    };
    let _ = a;
    (m, b)
}

/// **023 contract 14.** The sentence a reader takes away from an empty finding list must
/// say what bound it holds within. "No bugs found" and "no bugs exist" are the same
/// sentence to a tool that reads the output and reports to a human; §7's rule 4 is that
/// only an `Exact` run may make the second claim, and this run is `Bounded`.
#[test]
fn no_findings_under_a_hit_budget_renders_the_bound_not_a_proof() {
    let mut a = TermArena::new();
    let (m, b) = bounded_run(&mut a);
    let r = Engine::new(&m).with_budget(b).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Bounded, "the fixture must be cut");
    assert!(r.findings().is_empty(), "and must find nothing");

    let text = render(&r);
    assert!(
        text.contains("no bugs found within"),
        "the bound belongs in the sentence: {text}"
    );
    assert!(
        !text.contains("no bugs exist"),
        "only an Exact run may say that, and this one is Bounded: {text}"
    );
    // The bound that was actually hit, by name and value — "within some bound" tells a
    // reader nothing they can act on.
    // `contains('3')` would have passed on "max_recursion_depth 32" — the bound and its
    // value have to be asserted *together* or the assertion is nearly vacuous.
    assert!(
        text.contains("max_loop_iters (3)"),
        "which bound was hit, and what it was set to: {text}"
    );
}

/// **023 contract 12, the half a struct cannot check.** The contract says the assumption's
/// *text* appears in the rendered report — "a dummy assumption must not satisfy this".
/// A run can record every assumption correctly and print none of them, and then the
/// fidelity is a number nobody can act on.
#[test]
fn every_assumption_behind_a_degradation_appears_in_the_report() {
    let mut a = TermArena::new();
    let (m, b) = bounded_run(&mut a);
    let r = Engine::new(&m).with_budget(b).run(&mut a);

    let text = render(&r);
    let assumptions: Vec<_> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .cloned()
        .collect();
    assert!(
        !assumptions.is_empty(),
        "the fixture must degrade, or this checks nothing"
    );
    for asm in &assumptions {
        assert!(
            text.contains(&asm.detail),
            "assumption text missing from the report: {:?}\n---\n{text}",
            asm.detail
        );
    }
}

/// An `Exact` run with nothing found is the *only* case allowed to sound conclusive, and
/// even then the report says what it searched rather than that the program is safe: 023
/// §7.1 makes `seal` the one place that decides, and a renderer that speaks for it would
/// be a second.
#[test]
fn an_exact_run_reports_exhaustion_without_claiming_the_program_is_safe() {
    let mut a = TermArena::new();
    let m = func(
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    // Bounds that were **not** hit, and deliberately not the defaults: 023 §8 reports
    // them either way, "so a reader can tell `Exact`-with-generous-bounds from
    // `Exact`-with-trivial-bounds". A line that prints the names with some other run's
    // numbers tells them the opposite, and nothing in the assumptions would show it —
    // an `Exact` run has none.
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 7,
            max_states: 55,
            ..Budget::default()
        })
        .run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Exact);

    let text = render(&r);
    assert!(
        text.contains("max_loop_iters 7") && text.contains("max_states 55"),
        "the bounds in force, not some other run's: {text}"
    );
    assert!(
        text.contains("no bugs found"),
        "an exhaustive search that found nothing says so: {text}"
    );
    assert!(
        !text.contains("no bugs exist") && !text.contains("safe"),
        "chiero reports what it searched, not a verdict on the program: {text}"
    );
    assert!(
        text.contains("Exact"),
        "and the fidelity it holds at: {text}"
    );
}

/// A finding's own text is what a reader acts on, so it is in the report — and the
/// report says how many there were, because "1 finding" and "17 findings" are different
/// situations and a list alone makes them look the same.
#[test]
fn findings_are_rendered_with_their_own_text() {
    let mut a = TermArena::new();
    let m = func(
        vec![block(
            0,
            vec![inst_null_deref()],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    let text = render(&r);
    assert!(
        !r.findings().is_empty(),
        "the fixture must find something: {text}"
    );
    for f in r.findings() {
        assert!(text.contains(&f), "finding missing from the report: {f}");
    }
    assert!(
        !text.contains("no bugs found"),
        "a run with findings has not found no bugs: {text}"
    );
}

/// `*(int *)0 = 1` — a null store, which is a definite finding at any tier.
fn inst_null_deref() -> Inst {
    Inst {
        kind: InstKind::Store {
            addr: Operand::Const(Const::Null),
            val: i32c(1),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
        span: Span::DUMMY,
        generated: false,
    }
}

/// **A fork copies its parent's assumptions into every descendant**, so a flat
/// concatenation prints one degradation as many times as the run happened to branch
/// afterwards — which reads as a run in far worse shape than it is. Same argument
/// `RunResult::findings` makes for reports, one field over.
#[test]
fn one_degradation_before_a_fork_is_reported_once() {
    let mut a = TermArena::new();
    let mut f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 1,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }],
        blocks: vec![
            block(
                0,
                vec![
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::AddrOfLocal {
                                alloca: AllocaId(0),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    // Uninitialized: one `NoInformation` degradation, before any fork.
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Load {
                                addr: Operand::Value(ValueId(0)),
                                ty: CTy::Int(32),
                                align: 4,
                                vol: Volatility::Normal,
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Cmp {
                                op: CmpOp::Eq,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(1)),
                                b: i32c(0),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(0)))),
            block(2, vec![], Terminator::Return(Some(i32c(1)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    f.blocks[0].span = Span::DUMMY;
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.states().len() > 1,
        "the fixture must fork, or this checks nothing: {}",
        r.states().len()
    );
    let text = render(&r);
    let carried: usize = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .filter(|x| x.detail.contains("could not produce the program's value"))
        .count();
    assert!(
        carried > 1,
        "every forked state must carry the assumption, or the dedup is untested: {carried}"
    );
    let printed = text
        .matches("could not produce the program's value")
        .count();
    assert_eq!(printed, 1, "printed once, not once per descendant:\n{text}");
}

/// **The proof-shaped sentence belongs to `Exact` alone.** Review's worst surviving
/// mutant widened the condition to `Exact || Approximated || Unknown`, and a degraded run
/// then printed "the search was exhaustive… and none of them was reached" — 023 §7 rule 4
/// verbatim, which contract 14's golden test never caught because its only fixture is
/// `Bounded`. Each level now says what actually happened, which is also §7's preamble:
/// "a cap that was hit is `Bounded`; discarding values is `Approximated`."
#[test]
fn each_fidelity_level_gives_its_own_reason_and_only_exact_sounds_conclusive() {
    // `Approximated`: an approximate model ran. No bound was reached.
    let mut a = TermArena::new();
    let m = calling(1, "scanf");
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Approximated, "{:#?}", r.findings());
    let text = render(&r);
    assert!(
        !text.contains("exhaustive"),
        "an approximated run did not search exhaustively: {text}"
    );
    assert!(
        !text.contains("a bound was reached"),
        "and no bound was reached — that is the Bounded sentence: {text}"
    );
    assert!(
        text.contains("approximately") || text.contains("discarded"),
        "it says what did happen: {text}"
    );

    // `Unknown`: an empty module, which ran nothing at all.
    let mut a = TermArena::new();
    let r = Engine::new(&Module::default()).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Unknown);
    let text = render(&r);
    assert!(
        !text.contains("exhaustive") && !text.contains("a bound was reached"),
        "nothing was searched and no bound was reached: {text}"
    );
    assert!(
        text.contains("says nothing about the program"),
        "and it says so: {text}"
    );
}

/// **023 contract 12's other half — "whose *kind* matches the recorded cause".** A state
/// worse than `Exact` must carry an assumption whose kind accounts for that level; a
/// dummy assumption must not satisfy the check. Review found `ModelApproximate` missing
/// from the kinds that account for `Approximated`, so a `scanf` run — one assumption,
/// exactly the right one — satisfied the contract with **zero** qualifying assumptions.
#[test]
fn every_degraded_state_has_an_assumption_whose_kind_accounts_for_its_fidelity() {
    let mut a = TermArena::new();
    // Three programs, three different causes: a hit bound, an approximate model, and an
    // unmodeled call. One fixture would pin one kind.
    let (m, b) = bounded_run(&mut a);
    let runs = vec![
        Engine::new(&m).with_budget(b).run(&mut a),
        {
            let m2 = calling(1, "scanf");
            let mut a2 = TermArena::new();
            Engine::new(&m2).run(&mut a2)
        },
        {
            let m3 = calling(1, "no_such_function");
            let mut a3 = TermArena::new();
            Engine::new(&m3).run(&mut a3)
        },
    ];
    let mut levels = Vec::new();
    for r in &runs {
        let text = render(r);
        for s in r.states() {
            if s.fidelity() == Fidelity::Exact {
                continue;
            }
            levels.push(s.fidelity());
            let accounting: Vec<_> = s
                .assumptions()
                .iter()
                .filter(|x| x.kind.matches(s.fidelity()))
                .collect();
            assert!(
                !accounting.is_empty(),
                "{:?} with no assumption whose kind accounts for it: {:#?}",
                s.fidelity(),
                s.assumptions()
            );
            // …and contract 12's first half, on the same states: the text is in the report.
            for x in accounting {
                assert!(
                    text.contains(&x.detail),
                    "{:?} missing from:\n{text}",
                    x.detail
                );
            }
        }
    }
    levels.sort_by_key(|f| format!("{f:?}"));
    levels.dedup();
    assert!(
        levels.len() >= 2,
        "the fixtures must reach more than one degraded level, or one kind is pinned and \
         the rest are not: {levels:?}"
    );
}

/// **Every state's assumptions, not the first one's.** Review's `take(1)` mutation
/// survived because the only contract-12 fixture forks *after* its single degradation, so
/// all its states carry the same list — the same-answer trap, in a fixture. A run where
/// one path degrades and another does not is what tells the two apart.
#[test]
fn assumptions_from_every_path_reach_the_report() {
    let ext = |id: u32, name: &str| Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(1) },
                    },
                    span: Span::DUMMY,
                    generated: false,
                }],
                Terminator::Br {
                    cond: Operand::Value(ValueId(0)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            // Both paths meet something with no model, but **different** somethings: two
            // assumptions of the same kind with different texts. Deduplicating on the
            // kind alone drops one, and the reader loses a whole unmodeled function.
            block(
                1,
                vec![Inst {
                    kind: InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![],
                    },
                    span: Span::DUMMY,
                    generated: false,
                }],
                Terminator::Return(Some(i32c(0))),
            ),
            // …and the other meets something with no model.
            block(
                2,
                vec![Inst {
                    kind: InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![],
                    },
                    span: Span::DUMMY,
                    generated: false,
                }],
                Terminator::Return(Some(i32c(0))),
            ),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f, ext(1, "no_such_function"), ext(2, "another_missing_one")],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(r.states().len() > 1, "the fixture must fork");
    let text0 = render(&r);
    assert!(
        text0.contains("no_such_function") && text0.contains("another_missing_one"),
        "both unmodeled functions are named, not one per kind: {text0}"
    );
    let text = render(&r);
    let all: Vec<_> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    assert!(!all.is_empty(), "and one path must degrade");
    for detail in &all {
        assert!(
            text.contains(detail),
            "missing from the report: {detail}\n{text}"
        );
    }
    // **Each state carries a detail the other does not**, which is what makes the loop
    // above about every state rather than about state 0 by luck.
    let per_state: Vec<Vec<String>> = r
        .states()
        .iter()
        .map(|s| s.assumptions().iter().map(|x| x.detail.clone()).collect())
        .collect();
    assert!(
        per_state
            .iter()
            .any(|xs| xs.iter().any(|x| !per_state[0].contains(x))),
        "reading state 0 alone would pass on this fixture: {per_state:?}"
    );
}
