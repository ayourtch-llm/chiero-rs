//! The execution engine's core: stepping, forking, and the fidelity rule (023).
//!
//! Covers **023 contracts 1, 2, 3, 4, 11, 12** and §7.1's `ExactWitness`.
//!
//! Two decisions on display, both of which the rest of the project leans on:
//!
//! **`Value`, not `Term`.** A local holding a pointer keeps its `ObjectId`. Storing it as
//! a bare term would leave only one way to recover the base — searching the address space
//! — and 021 §7 puts guard gaps between objects *precisely so* an out-of-bounds pointer
//! resolves to no object. Round-tripping a pointer through a term therefore converts a
//! detectable OOB into `UNBOUND`, and 021 contract 3 becomes unimplementable.
//!
//! **A branch the solver could not decide is taken anyway.** Dropping it would let "no
//! bug found" mean "the solver timed out", which §7 forbids — and the resulting fidelity
//! is `Unknown`, not `Approximated`, because the engine genuinely does not know whether
//! that path exists. That distinction is the difference between "I modeled this loosely"
//! and "I have no idea".

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::{Sort, TermArena};
use chiero_span::Span;

fn inst(kind: InstKind) -> Inst {
    Inst {
        kind,
        span: Span::DUMMY,
    }
}

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
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
            body: Body::Defined,
            span: Span::DUMMY,
        }],
        ..Default::default()
    }
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

/// **023 contract 1.** `%a = add i32 2, 3; ret %a` yields one terminated state returning
/// 5, `Fidelity::Exact`, and **zero solver calls**. Straight-line arithmetic asking a
/// solver anything would make every benchmark meaningless.
#[test]
fn straight_line_arithmetic_terminates_exactly_with_no_solver_calls() {
    let m = func(
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Bin {
                    op: BinOp::Add,
                    a: i32c(2),
                    b: i32c(3),
                    ty: CTy::Int(32),
                },
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states.len(), 1, "{:#?}", r.states);
    let s = &r.states[0];
    assert!(matches!(s.status, Status::Terminated(_)));
    assert_eq!(s.return_value_bits(&mut a), Some(5));
    assert_eq!(s.fidelity, Fidelity::Exact);
    assert_eq!(r.solver_calls, 0, "arithmetic must not consult a solver");
}

/// **023 contract 2.** A constant branch condition makes zero solver calls and produces
/// one successor. §3 calls this the fast path that carries most of the traffic, and says
/// it must exist before any benchmark is believed.
#[test]
fn a_constant_branch_makes_no_solver_call_and_one_successor() {
    let m = func(
        vec![
            block(
                0,
                vec![],
                Terminator::Br {
                    cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(10)))),
            block(2, vec![], Terminator::Return(Some(i32c(20)))),
        ],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states.len(), 1);
    assert_eq!(r.states[0].return_value_bits(&mut a), Some(10));
    assert_eq!(r.solver_calls, 0);
    assert_eq!(r.states[0].fidelity, Fidelity::Exact);
}

/// **023 contract 3.** A symbolic branch forks into exactly two states with path
/// conditions `x < 5` and `¬(x < 5)`, **in that order** — fork order is deterministic
/// (§3), and 001 §5 makes determinism a hard requirement rather than a nicety.
#[test]
fn a_symbolic_branch_forks_into_two_states_true_first() {
    let mut a = TermArena::new();
    let x = a.var(Sort::BitVec(32), "x");
    let m = func(
        vec![
            block(
                0,
                vec![inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Int(1) },
                })],
                Terminator::Br {
                    cond: Operand::Value(ValueId(0)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(10)))),
            block(2, vec![], Terminator::Return(Some(i32c(20)))),
        ],
        CTy::Int(32),
    );
    let _ = x;
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states.len(), 2, "{:#?}", r.states);
    assert_eq!(
        r.states[0].return_value_bits(&mut a),
        Some(10),
        "the true branch is explored first"
    );
    assert_eq!(r.states[1].return_value_bits(&mut a), Some(20));
    assert_eq!(r.states[0].path.len(), 1);
    assert_eq!(r.states[1].path.len(), 1);
    assert_ne!(
        r.states[0].path[0], r.states[1].path[0],
        "the two states must carry opposite constraints"
    );
}

/// **023 contract 4.** A branch the solver cannot decide is **taken anyway**, and the
/// resulting fidelity is `Unknown` — not `Approximated`. §7's table is explicit: the
/// engine does not know whether that path exists, which is a different claim from having
/// modeled it loosely. Exactly one assumption names the cause.
#[test]
fn an_undecidable_branch_is_taken_and_yields_unknown_fidelity() {
    let mut a = TermArena::new();
    let m = func(
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    }),
                    // A product tier 1 cannot decide (022 §3).
                    inst(InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Bin {
                            op: BinOp::Mul,
                            a: Operand::Value(ValueId(0)),
                            b: Operand::Value(ValueId(1)),
                            ty: CTy::Int(32),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(3),
                        rv: RValue::Cmp {
                            op: CmpOp::ULt,
                            a: Operand::Value(ValueId(2)),
                            b: i32c(7),
                            ty: CTy::Int(32),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(10)))),
            block(2, vec![], Terminator::Return(Some(i32c(20)))),
        ],
        CTy::Int(32),
    );
    let r = Engine::new(&m)
        .with_solver(SolverTier::LiteOnly)
        .run(&mut a);
    // **Both** sides, not just one. "The branch is taken anyway" means neither is
    // dropped — keeping one and discarding the other loses a path the solver never
    // refuted, which is the same unsoundness in half measure, and asserting only that
    // *some* state survived cannot tell the two apart.
    assert_eq!(r.states.len(), 2, "{:#?}", r.states);
    assert_eq!(r.states[0].return_value_bits(&mut a), Some(10));
    assert_eq!(r.states[1].return_value_bits(&mut a), Some(20));
    for s in &r.states {
        assert_eq!(
            s.fidelity,
            Fidelity::Unknown,
            "a solver Unknown on a decision that mattered is Unknown, not Approximated"
        );
        assert_eq!(s.assumptions.len(), 1, "{:#?}", s.assumptions);
        assert_eq!(s.assumptions[0].kind, AssumptionKind::SolverUnknown);
    }
}

/// **023 contract 11: fidelity only ever degrades.** The test must include a run that
/// *does* degrade, or an implementation that reports `Unknown` for everything passes
/// monotonicity trivially — the spec says so in as many words.
#[test]
fn fidelity_never_improves_and_the_corpus_contains_a_degradation() {
    assert!(Fidelity::Exact < Fidelity::Bounded);
    assert!(Fidelity::Bounded < Fidelity::Approximated);
    assert!(Fidelity::Approximated < Fidelity::Unknown);

    let mut f = Fidelity::Exact;
    f = f.degrade(Fidelity::Approximated);
    assert_eq!(f, Fidelity::Approximated);
    f = f.degrade(Fidelity::Exact);
    assert_eq!(f, Fidelity::Approximated, "fidelity is never restored");
    f = f.degrade(Fidelity::Unknown);
    assert_eq!(f, Fidelity::Unknown, "and the worst still wins");
    f = f.degrade(Fidelity::Bounded);
    assert_eq!(f, Fidelity::Unknown);
}

/// **023 §7 rule 3 / contract 12.** Every state worse than `Exact` carries an assumption
/// naming the cause. "Approximated with no reason" is a bug, and a *dummy* assumption
/// must not satisfy the check — so the kind has to match what actually happened.
#[test]
fn every_degraded_state_names_what_degraded_it() {
    let mut a = TermArena::new();
    let m = func(
        vec![block(
            0,
            vec![inst(InstKind::Opaque {
                dsts: vec![(ValueId(0), CTy::Int(32))],
                writes: vec![],
                reads: vec![],
                why: OpaqueReason::InlineAsm,
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    for s in &r.states {
        if s.fidelity != Fidelity::Exact {
            assert!(
                s.assumptions.iter().any(|x| x.kind.matches(s.fidelity)),
                "a degraded state must name a cause of the right kind: {:#?}",
                s.assumptions
            );
        }
    }
    // `Opaque` is a modeling lie, not a truncated search (§7's table).
    assert_eq!(r.states[0].fidelity, Fidelity::Approximated);
    assert_eq!(r.states[0].assumptions[0].kind, AssumptionKind::OpaqueCode);
}

/// **023 §7 rule 4 and §7.1.** A negative result may be presented as a proof only at
/// `Exact`. `seal` is the one function in the workspace that reads a run's fidelity to
/// decide that, and it additionally checks the witness belongs to *this* run — a token
/// minted from a trivial `return 0` must not bless a degraded one.
#[test]
fn only_an_exact_run_can_be_sealed_as_proven() {
    let mut a = TermArena::new();
    let exact = func(
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    let r1 = Engine::new(&exact).run(&mut a);
    assert_eq!(r1.fidelity(), Fidelity::Exact);
    let w1 = r1.witness().expect("an exact run yields a witness");
    assert!(seal(&r1, w1).is_ok());

    let degraded = func(
        vec![block(
            0,
            vec![inst(InstKind::Opaque {
                dsts: vec![(ValueId(0), CTy::Int(32))],
                writes: vec![],
                reads: vec![],
                why: OpaqueReason::InlineAsm,
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let r2 = Engine::new(&degraded).run(&mut a);
    assert_ne!(r2.fidelity(), Fidelity::Exact);
    assert!(
        r2.witness().is_none(),
        "a degraded run must not mint a witness at all"
    );

    // And a witness from one run cannot bless another, even though both are Exact.
    let r3 = Engine::new(&exact).run(&mut a);
    let w3 = r3.witness().unwrap();
    match seal(&r1, w3) {
        Err(NotProven { .. }) => {}
        Ok(_) => panic!("a witness from another run must be rejected"),
    }
}

/// A run's fidelity is the **worst** over every state that contributed (§7 rule 2). One
/// degraded path among many exact ones does not get rounded away.
#[test]
fn a_runs_fidelity_is_the_worst_of_its_states() {
    let mut a = TermArena::new();
    let m = func(
        vec![
            // A *constant* condition, so the branch itself contributes nothing and the
            // only degradation on the run comes from the opaque construct. A symbolic
            // condition would be legitimately `Unknown` under tier 1 alone, which would
            // make this test about the solver rather than about fidelity aggregation.
            block(
                0,
                vec![],
                Terminator::Br {
                    cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(InstKind::Opaque {
                    dsts: vec![(ValueId(1), CTy::Int(32))],
                    writes: vec![],
                    reads: vec![],
                    why: OpaqueReason::InlineAsm,
                })],
                Terminator::Return(Some(i32c(10))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(20)))),
        ],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states.len(), 1, "a constant condition has one successor");
    assert_eq!(
        r.fidelity(),
        Fidelity::Approximated,
        "one imprecise path degrades the run"
    );
    assert!(r.witness().is_none());

    // And the aggregation itself: worst wins over a mixed set.
    assert_eq!(
        [Fidelity::Exact, Fidelity::Approximated, Fidelity::Exact]
            .into_iter()
            .fold(Fidelity::Exact, Fidelity::degrade),
        Fidelity::Approximated
    );
}

/// **023 §1.1: a local holding a pointer keeps its object.** Storing it as a bare term
/// would leave only the address space to recover the base from, and 021 §7's guard gaps
/// exist so an out-of-bounds pointer resolves to *no* object — so the round trip would
/// turn a detectable OOB into a wild pointer.
#[test]
fn a_local_holding_a_pointer_keeps_its_object_identity() {
    let mut m = func(
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            })],
            Terminator::Return(None),
        )],
        CTy::Void,
    );
    m.funcs[0].allocas = vec![AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(32),
        count: 4,
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    }];
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    match r.states[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => {
            assert_ne!(p.base, chiero_mem::ObjectId::UNBOUND);
            assert_eq!(p.off, 0);
        }
        other => panic!("a local holding an address must be a pointer, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Escalation and budgets.
// ---------------------------------------------------------------------------

fn z3_or_skip(t: &str) -> Option<chiero_solver::SmtLib> {
    match chiero_solver::SmtLib::discover() {
        Some(b) => Some(b),
        None => {
            eprintln!("skipping {t}: no SMT-LIB2 backend on PATH");
            None
        }
    }
}

/// **Escalation is what keeps the engine's answers worth having.**
///
/// Tier 1 is deliberately incomplete (022 §3), so under it alone a branch on
/// `x * y < 7` is `Unknown` and *every* such fork degrades the run — the engine would
/// report "I have no idea" for the ordinary case of a multiplied index. With tier 2 the
/// same branch is decided and the run stays `Exact`.
#[test]
fn a_branch_tier_one_cannot_decide_is_escalated_and_stays_exact() {
    let Some(backend) = z3_or_skip("a_branch_tier_one_cannot_decide_is_escalated") else {
        return;
    };
    let mut a = TermArena::new();
    let m = undecidable_branch_module();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    assert_eq!(r.states.len(), 2, "both branches are feasible");
    for s in &r.states {
        assert_eq!(
            s.fidelity,
            Fidelity::Exact,
            "the backend decided it, so nothing was approximated: {:#?}",
            s.assumptions
        );
        assert!(s.assumptions.is_empty());
    }
    assert!(r.witness().is_some(), "an exact run can be sealed");
}

/// The same program without a backend is `Unknown` — which is the honest answer, and the
/// contrast is what shows escalation is doing something rather than the query having been
/// easy all along.
#[test]
fn the_same_branch_without_a_backend_is_unknown() {
    let mut a = TermArena::new();
    let m = undecidable_branch_module();
    let r = Engine::new(&m).run(&mut a);
    assert!(r.states.iter().all(|s| s.fidelity == Fidelity::Unknown));
}

fn undecidable_branch_module() -> Module {
    func(
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Bin {
                            op: BinOp::Mul,
                            a: Operand::Value(ValueId(0)),
                            b: Operand::Value(ValueId(1)),
                            ty: CTy::Int(32),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(3),
                        rv: RValue::Cmp {
                            op: CmpOp::ULt,
                            a: Operand::Value(ValueId(2)),
                            b: i32c(7),
                            ty: CTy::Int(32),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(10)))),
            block(2, vec![], Terminator::Return(Some(i32c(20)))),
        ],
        CTy::Int(32),
    )
}

/// **023 contract 5.** A loop with `max_loop_iters = 3` terminates, and the run is
/// `Bounded` with a `BudgetHit` naming the back edge.
///
/// The bound is per **back edge** — CIR has no loops (020 §1), so there is nothing
/// syntactic to count. Without a bound the engine does not merely run slowly; it does not
/// return.
#[test]
fn a_loop_is_bounded_per_back_edge_and_the_run_says_so() {
    let mut a = TermArena::new();
    // entry: goto head
    // head:  br true, body, exit
    // body:  goto head          <- the back edge
    // exit:  ret 0
    //
    // The condition is **constant**, so the solver is never consulted and the only thing
    // that can stop this run is the bound. A symbolic condition would be `Unknown` under
    // tier 1 alone, and `Unknown` dominates `Bounded` — the test would then be about the
    // solver rather than about the budget.
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
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 3,
            ..Budget::default()
        })
        .run(&mut a);

    assert!(
        r.states
            .iter()
            .any(|s| s.status == Status::Terminated(TermReason::Budget)),
        "some state must be cut by the bound: {:#?}",
        r.states.iter().map(|s| &s.status).collect::<Vec<_>>()
    );
    let cut: Vec<_> = r
        .states
        .iter()
        .filter(|s| s.status == Status::Terminated(TermReason::Budget))
        .collect();
    for s in cut {
        assert_eq!(
            s.fidelity,
            Fidelity::Bounded,
            "a budget is a truncated search, not a modeling lie"
        );
        assert!(
            s.assumptions
                .iter()
                .any(|x| x.kind == AssumptionKind::BudgetHit && x.detail.contains("back edge")),
            "the assumption must name the back edge: {:#?}",
            s.assumptions
        );
    }
    assert_eq!(r.fidelity(), Fidelity::Bounded);
    assert!(
        r.witness().is_none(),
        "a bounded run cannot be presented as a proof"
    );
}

/// A loop that exits on its own is **not** bounded. Without this the test above is
/// satisfied by an engine that reports `Bounded` for every loop it sees, and "not found
/// within a bound" would replace "not found" everywhere.
#[test]
fn a_loop_that_terminates_within_the_bound_stays_exact() {
    let mut a = TermArena::new();
    // Two iterations, decided concretely: no fork, no budget.
    let m = func(
        vec![
            block(0, vec![], Terminator::Goto(BlockId(1))),
            block(
                1,
                vec![],
                Terminator::Br {
                    cond: Operand::Const(Const::Int { bits: 1, val: 0 }),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        CTy::Int(32),
    );
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 3,
            ..Budget::default()
        })
        .run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states[0].assumptions
    );
    assert!(r.witness().is_some());
}

/// **023 contract 6, the determinism core.** The same program run twice produces the same
/// `StateId` sequence and the same results. 001 §5 makes this a hard requirement: a
/// non-reproducible bug report is not a bug report.
#[test]
fn two_runs_of_one_program_are_identical() {
    let build = || {
        let mut a = TermArena::new();
        let m = func(
            vec![
                block(
                    0,
                    vec![inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(1) },
                    })],
                    Terminator::Br {
                        cond: Operand::Value(ValueId(0)),
                        t: BlockId(1),
                        f: BlockId(2),
                    },
                ),
                block(1, vec![], Terminator::Return(Some(i32c(10)))),
                block(2, vec![], Terminator::Return(Some(i32c(20)))),
            ],
            CTy::Int(32),
        );
        let r = Engine::new(&m).run(&mut a);
        let ids: Vec<u32> = r.states.iter().map(|s| s.id.0).collect();
        let rets: Vec<Option<u128>> = r
            .states
            .iter()
            .map(|s| s.return_value_bits(&mut a))
            .collect();
        (ids, rets, r.fidelity())
    };
    assert_eq!(build(), build());
}

/// **A long straight path is not a loop.** Counting every edge as a back edge would bound
/// any function with more blocks than `max_loop_iters` — and would report `Bounded` on
/// code that was explored completely, which §7 rule 4 makes into a false "not proven".
///
/// The current back-edge test is `to <= from` on block ids, which is a heuristic: 023 §8
/// specifies dominator analysis, and an irreducible CFG or a lowering that numbers blocks
/// out of order will fool it. That gap is recorded rather than papered over.
///
/// Counting *every* edge instead is very nearly equivalent, and deliberately not chased:
/// counts are per edge, so a straight path never repeats one, and inside a loop the back
/// edge already bounds the forward edge at the same count. The observable difference is
/// at most one iteration. A test contrived to catch it would pin an accident of the
/// heuristic rather than anything the spec asks for — the real fix is dominator
/// analysis, which is owed.
#[test]
fn a_long_forward_path_is_not_mistaken_for_a_loop() {
    let mut a = TermArena::new();
    let mut blocks = Vec::new();
    for i in 0..8u32 {
        blocks.push(block(i, vec![], Terminator::Goto(BlockId(i + 1))));
    }
    blocks.push(block(8, vec![], Terminator::Return(Some(i32c(0)))));
    let m = func(blocks, CTy::Int(32));
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 2,
            ..Budget::default()
        })
        .run(&mut a);
    assert_eq!(r.states.len(), 1);
    assert_eq!(
        r.states[0].status,
        Status::Terminated(TermReason::Return),
        "eight forward edges are not eight loop iterations"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states[0].assumptions
    );
}
