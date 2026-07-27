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
    assert_eq!(r.states().len(), 1, "{:#?}", r.states());
    let s = &r.states()[0];
    assert!(matches!(s.status, Status::Terminated(_)));
    assert_eq!(s.return_value_bits(&mut a), Some(5));
    assert_eq!(s.fidelity(), Fidelity::Exact);
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
    assert_eq!(r.states().len(), 1);
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(10));
    assert_eq!(r.solver_calls, 0);
    assert_eq!(r.states()[0].fidelity(), Fidelity::Exact);
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
    assert_eq!(r.states().len(), 2, "{:#?}", r.states());
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(10),
        "the true branch is explored first"
    );
    assert_eq!(r.states()[1].return_value_bits(&mut a), Some(20));
    assert_eq!(r.states()[0].path.len(), 1);
    assert_eq!(r.states()[1].path.len(), 1);
    assert_ne!(
        r.states()[0].path[0],
        r.states()[1].path[0],
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
    assert_eq!(r.states().len(), 2, "{:#?}", r.states());
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(10));
    assert_eq!(r.states()[1].return_value_bits(&mut a), Some(20));
    for s in r.states() {
        assert_eq!(
            s.fidelity(),
            Fidelity::Unknown,
            "a solver Unknown on a decision that mattered is Unknown, not Approximated"
        );
        assert_eq!(s.assumptions().len(), 1, "{:#?}", s.assumptions());
        assert_eq!(s.assumptions()[0].kind, AssumptionKind::SolverUnknown);
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
    for s in r.states() {
        if s.fidelity() != Fidelity::Exact {
            assert!(
                s.assumptions().iter().any(|x| x.kind.matches(s.fidelity())),
                "a degraded state must name a cause of the right kind: {:#?}",
                s.assumptions()
            );
        }
    }
    // `Opaque` is a modeling lie, not a truncated search (§7's table).
    assert_eq!(r.states()[0].fidelity(), Fidelity::Approximated);
    assert_eq!(
        r.states()[0].assumptions()[0].kind,
        AssumptionKind::OpaqueCode
    );
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
    assert_eq!(
        r.states().len(),
        1,
        "a constant condition has one successor"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Approximated,
        "one imprecise path degrades the run"
    );
    assert!(seal(&r, r.witness()).is_err());

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
    match r.states()[0].local(ValueId(0)) {
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
    assert_eq!(r.states().len(), 2, "both branches are feasible");
    for s in r.states() {
        assert_eq!(
            s.fidelity(),
            Fidelity::Exact,
            "the backend decided it, so nothing was approximated: {:#?}",
            s.assumptions()
        );
        assert!(s.assumptions().is_empty());
    }
    assert!(seal(&r, r.witness()).is_ok(), "an exact run can be sealed");
}

/// The same program without a backend is `Unknown` — which is the honest answer, and the
/// contrast is what shows escalation is doing something rather than the query having been
/// easy all along.
#[test]
fn the_same_branch_without_a_backend_is_unknown() {
    let mut a = TermArena::new();
    let m = undecidable_branch_module();
    let r = Engine::new(&m).run(&mut a);
    assert!(r.states().iter().all(|s| s.fidelity() == Fidelity::Unknown));
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
        r.states()
            .iter()
            .any(|s| s.status == Status::Terminated(TermReason::Budget)),
        "some state must be cut by the bound: {:#?}",
        r.states().iter().map(|s| &s.status).collect::<Vec<_>>()
    );
    let cut: Vec<_> = r
        .states()
        .iter()
        .filter(|s| s.status == Status::Terminated(TermReason::Budget))
        .collect();
    for s in cut {
        assert_eq!(
            s.fidelity(),
            Fidelity::Bounded,
            "a budget is a truncated search, not a modeling lie"
        );
        assert!(
            s.assumptions()
                .iter()
                .any(|x| x.kind == AssumptionKind::BudgetHit && x.detail.contains("back edge")),
            "the assumption must name the back edge: {:#?}",
            s.assumptions()
        );
    }
    assert_eq!(r.fidelity(), Fidelity::Bounded);
    assert!(
        seal(&r, r.witness()).is_err(),
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
        r.states()[0].assumptions()
    );
    assert!(seal(&r, r.witness()).is_ok());
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
        let ids: Vec<u32> = r.states().iter().map(|s| s.id.0).collect();
        let rets: Vec<Option<u128>> = r
            .states()
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
    assert_eq!(r.states().len(), 1);
    assert_eq!(
        r.states()[0].status,
        Status::Terminated(TermReason::Return),
        "eight forward edges are not eight loop iterations"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
}

// ---------------------------------------------------------------------------
// Calls, recursion, and the model boundary (023 §5, contracts 8 and 9).
// ---------------------------------------------------------------------------

fn two_funcs(f0: Function, f1: Function) -> Module {
    Module {
        funcs: vec![f0, f1],
        ..Default::default()
    }
}

fn defined(id: u32, name: &str, blocks: Vec<Block>, ret: CTy) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret,
        variadic: false,
        allocas: vec![],
        blocks,
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    }
}

/// **023 §5: a direct call pushes a frame; there is no inlining.**
///
/// An explicit call stack is what keeps `Span` backtraces honest and makes recursion
/// bounding a counter rather than a heuristic — inlining would make both guesswork.
#[test]
fn a_direct_call_runs_the_callee_and_returns_its_value() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let callee = defined(
        1,
        "answer",
        vec![block(0, vec![], Terminator::Return(Some(i32c(42))))],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, callee)).run(&mut a);
    assert_eq!(r.states().len(), 1, "{:#?}", r.states());
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(42));
    assert_eq!(r.states()[0].fidelity(), Fidelity::Exact);
}

/// A callee's locals are its own. Without a real frame the callee would overwrite the
/// caller's `%0` and the caller would return the wrong value — silently, since both are
/// well-typed.
#[test]
fn a_callees_locals_do_not_overwrite_the_callers() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Use(i32c(7)),
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(1)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    // The callee assigns *its* %0 a different value.
    let callee = defined(
        1,
        "clobber",
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Use(i32c(99)),
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, callee)).run(&mut a);
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(7),
        "the caller's %0 survives the call"
    );
}

/// **023 contract 8.** A call to an unmodeled extern returns a fresh value, degrades to
/// `Approximated`, and records the function name. **Silently returning 0 is forbidden** —
/// it is the same "confidently wrong" failure as reading uninitialized memory as zero,
/// one level up.
#[test]
fn an_unmodeled_extern_returns_a_fresh_value_and_says_so() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let mut ext = defined(1, "getenv", vec![], CTy::Int(32));
    ext.body = Body::Declared;

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, ext)).run(&mut a);
    assert_eq!(r.states().len(), 1);
    let s = &r.states()[0];
    assert_eq!(
        s.return_value_bits(&mut a),
        None,
        "a fresh symbol, not a concrete zero"
    );
    assert_eq!(s.fidelity(), Fidelity::Approximated);
    assert_eq!(s.assumptions().len(), 1, "{:#?}", s.assumptions());
    assert_eq!(s.assumptions()[0].kind, AssumptionKind::UnmodeledCall);
    assert!(
        s.assumptions()[0].detail.contains("getenv"),
        "the finding must name the function: {:#?}",
        s.assumptions()[0]
    );
}

/// **023 contract 9.** Recursion past `max_recursion_depth` terminates that state as
/// `Bounded` **and does not overflow the interpreter's own stack** — the interpreter's
/// stack usage must be O(1) in program recursion depth, which an explicit frame stack
/// gives for free and a recursive `step` would not.
#[test]
fn unbounded_recursion_is_bounded_without_overflowing_the_interpreter() {
    // f() { return f(); }  — infinite recursion.
    let f = defined(
        0,
        "f",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(0)),
                args: vec![],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&Module {
        funcs: vec![f],
        ..Default::default()
    })
    .with_budget(Budget {
        max_recursion_depth: 32,
        ..Budget::default()
    })
    .run(&mut a);
    assert_eq!(r.states().len(), 1);
    assert_eq!(r.states()[0].status, Status::Terminated(TermReason::Budget));
    assert_eq!(r.states()[0].fidelity(), Fidelity::Bounded);
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.kind == AssumptionKind::BudgetHit && x.detail.contains("recursion")),
        "{:#?}",
        r.states()[0].assumptions()
    );
}

/// Recursion *within* the bound completes normally, or an engine that bounded every call
/// would pass the test above and report `Bounded` on every program with a function call
/// in it.
#[test]
fn recursion_within_the_bound_completes() {
    // f(): if false { f() } else { return 5 }  — one level, decided concretely.
    let f = defined(
        0,
        "f",
        vec![
            block(
                0,
                vec![],
                Terminator::Br {
                    cond: Operand::Const(Const::Int { bits: 1, val: 0 }),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(InstKind::Call {
                    dst: Some(ValueId(0)),
                    callee: Callee::Direct(FuncId(0)),
                    args: vec![],
                })],
                Terminator::Return(Some(Operand::Value(ValueId(0)))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(5)))),
        ],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&Module {
        funcs: vec![f],
        ..Default::default()
    })
    .run(&mut a);
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(5));
    assert_eq!(r.states()[0].fidelity(), Fidelity::Exact);
}

/// `FnAttrs::noreturn` terminates the state at the call (023 §5). Continuing past
/// `exit()` or `abort()` would explore code that cannot run and report findings in it.
#[test]
fn a_noreturn_call_terminates_the_state_at_the_call() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                }),
                // Unreachable: `abort` does not return.
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Use(i32c(1)),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut ext = defined(1, "abort", vec![], CTy::Void);
    ext.body = Body::Declared;
    ext.attrs.noreturn = true;

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, ext)).run(&mut a);
    assert_eq!(r.states().len(), 1);
    assert!(matches!(r.states()[0].status, Status::Terminated(_)));
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        None,
        "the state ended at the call, so nothing was returned"
    );
    assert!(
        r.states()[0].local(ValueId(0)).is_none(),
        "the instruction after a noreturn call must not execute"
    );
}

// ---------------------------------------------------------------------------
// Wave 12, from the engine review. 12 of 15 mutants had survived.
// ---------------------------------------------------------------------------

/// **Arithmetic must compute what the operator says.** Only four `BinOp`s were
/// implemented and the rest fell through to *addition* — at `Fidelity::Exact`, with no
/// assumption. `5 - 3` came out `8`. The arena already exposes every one of these, so
/// this was never an honest lowering gap; it was a wrong answer wearing a proof.
#[test]
fn every_implemented_binop_computes_its_own_operation() {
    let cases: Vec<(BinOp, i128, i128, u128)> = vec![
        (BinOp::Add, 5, 3, 8),
        (BinOp::Sub, 5, 3, 2),
        (BinOp::Mul, 5, 3, 15),
        (BinOp::UDiv, 12, 4, 3),
        (BinOp::URem, 13, 4, 1),
        (BinOp::And, 6, 3, 2),
        (BinOp::Or, 6, 3, 7),
        (BinOp::Xor, 6, 3, 5),
        (BinOp::Shl, 1, 4, 16),
        (BinOp::LShr, 16, 2, 4),
    ];
    for (op, x, y, want) in cases {
        let m = func(
            vec![block(
                0,
                vec![inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op,
                        a: i32c(x),
                        b: i32c(y),
                        ty: CTy::Int(32),
                    },
                })],
                Terminator::Return(Some(Operand::Value(ValueId(0)))),
            )],
            CTy::Int(32),
        );
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(want),
            "{op:?} {x}, {y}"
        );
        assert_eq!(r.states()[0].fidelity(), Fidelity::Exact, "{op:?}");
    }
}

/// **Comparisons must not invert.** `Ne` fell through to `Eq`, so every `if (x != y)`
/// took the opposite branch — silently, at `Exact`. `SLt` is the one VPP-style signed
/// index checks rest on, and it was `Eq` too.
#[test]
fn every_implemented_cmpop_answers_its_own_question() {
    let cases: Vec<(CmpOp, i128, i128, u128)> = vec![
        (CmpOp::Eq, 1, 2, 0),
        (CmpOp::Ne, 1, 2, 1),
        (CmpOp::ULt, 1, 2, 1),
        (CmpOp::ULe, 2, 2, 1),
        (CmpOp::UGt, 1, 2, 0),
        (CmpOp::UGe, 2, 2, 1),
        (CmpOp::SLt, -1, 2, 1),
        (CmpOp::SGt, -1, 2, 0),
    ];
    for (op, x, y, want) in cases {
        let m = func(
            vec![block(
                0,
                vec![inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Cmp {
                        op,
                        a: i32c(x),
                        b: i32c(y),
                        ty: CTy::Int(32),
                    },
                })],
                Terminator::Return(Some(Operand::Value(ValueId(0)))),
            )],
            CTy::Int(1),
        );
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(want),
            "{op:?} {x}, {y}"
        );
    }
}

/// **No path ends at `Exact` unless everything on it was modeled.** This is the single
/// rule behind a family of holes: an unsupported terminator, a `LoweringGap`, a dropped
/// instruction and an unrepresentable constant each left the run `Exact` with a witness,
/// so an *unexecuted* program minted a proof — the one thing §7 rule 4 says the crate
/// must be structurally incapable of.
#[test]
fn nothing_unmodeled_can_end_at_exact() {
    let cases: Vec<(&str, Module)> = vec![
        (
            "an unsupported terminator",
            func(
                vec![block(
                    0,
                    vec![],
                    Terminator::Switch {
                        scrut: i32c(1),
                        ty: CTy::Int(32),
                        cases: vec![(1, BlockId(0))],
                        default: BlockId(0),
                    },
                )],
                CTy::Int(32),
            ),
        ),
        (
            "a lowering gap",
            func(
                vec![block(
                    0,
                    vec![],
                    Terminator::Unreachable(UnreachableReason::LoweringGap),
                )],
                CTy::Int(32),
            ),
        ),
        (
            "an unrepresentable constant",
            func(
                vec![block(
                    0,
                    vec![inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Use(Operand::Const(Const::Float(
                            FloatKind::F64,
                            0x4000_0000_0000_0000,
                        ))),
                    })],
                    Terminator::Return(Some(i32c(0))),
                )],
                CTy::Int(32),
            ),
        ),
        (
            "a dropped store",
            func(
                vec![block(
                    0,
                    vec![inst(InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(7),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    })],
                    Terminator::Return(Some(i32c(0))),
                )],
                CTy::Int(32),
            ),
        ),
    ];
    for (what, m) in cases {
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_ne!(r.fidelity(), Fidelity::Exact, "{what} ended at Exact");
        assert!(seal(&r, r.witness()).is_err(), "{what} sealed as proven");
        assert!(
            r.states().iter().all(|s| s.fidelity() == Fidelity::Exact
                || s.assumptions().iter().any(|x| x.kind.matches(s.fidelity()))),
            "{what}: a degraded state must name a cause of the right kind"
        );
    }
}

/// **A branch the solver *refuted* must not be explored.** The catch-all arm covered
/// `(No, Unknown)` and `(Unknown, No)` too, and took *both* edges — so one child carried
/// a path condition the solver had already proved unsatisfiable, and any checker firing
/// there produces a false finding with an impossible witness. §3 says take a branch the
/// solver **could not refute**, not one it did.
#[test]
fn a_refuted_branch_is_not_explored_even_when_the_other_side_is_unknown() {
    let mut a = TermArena::new();
    // b = fresh i1; if (b == 1) { if (b) ... }  — the inner false side is refuted.
    let m = func(
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(1) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cmp {
                            op: CmpOp::Eq,
                            a: Operand::Value(ValueId(0)),
                            b: Operand::Const(Const::Int { bits: 1, val: 1 }),
                            ty: CTy::Int(1),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(3),
                },
            ),
            block(
                1,
                vec![],
                Terminator::Br {
                    cond: Operand::Value(ValueId(0)),
                    t: BlockId(2),
                    f: BlockId(4),
                },
            ),
            block(2, vec![], Terminator::Return(Some(i32c(100)))),
            block(3, vec![], Terminator::Return(Some(i32c(101)))),
            block(4, vec![], Terminator::Return(Some(i32c(102)))),
        ],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    let rets: Vec<_> = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut a))
        .collect();
    assert!(
        !rets.contains(&Some(102)),
        "block 4 needs `b == 1` and `b == 0` at once: {rets:?}"
    );
}

/// The **(refuted, undecided)** shape specifically, which is where the catch-all arm did
/// its damage: tier 1 proves the true side impossible but cannot decide the false side,
/// because the negation of a comparison is outside its fragment (022 §3.2). Taking both
/// then explores a path the solver had already refuted.
#[test]
fn a_side_the_solver_refuted_is_dropped_even_when_the_other_is_undecided() {
    let mut a = TermArena::new();
    // x = fresh; if (x <u 5) { if (5 <u x) A else B }
    // The inner true side contradicts the outer constraint; the inner false side is a
    // negated comparison, which tier 1 answers `Unknown`.
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
                        rv: RValue::Cmp {
                            op: CmpOp::ULt,
                            a: Operand::Value(ValueId(0)),
                            b: i32c(5),
                            ty: CTy::Int(32),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(4),
                },
            ),
            block(
                1,
                vec![inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Cmp {
                        op: CmpOp::ULt,
                        a: i32c(5),
                        b: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                    },
                })],
                Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(2),
                    f: BlockId(3),
                },
            ),
            block(2, vec![], Terminator::Return(Some(i32c(200)))),
            block(3, vec![], Terminator::Return(Some(i32c(201)))),
            block(4, vec![], Terminator::Return(Some(i32c(202)))),
        ],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    let rets: Vec<_> = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut a))
        .collect();
    assert!(
        !rets.contains(&Some(200)),
        "`x <u 5` and `5 <u x` cannot both hold: {rets:?}"
    );
}

/// An operation the engine does **not** implement must degrade, not compute something
/// else. Floats are `Approximated` in §7's table and unimplemented here, so the honest
/// answer is a gap — the default that quietly returned an *addition* was the shape of
/// the original defect and must not survive in the tail of the match either.
#[test]
fn an_unimplemented_operation_degrades_rather_than_computing_something_else() {
    let m = func(
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Bin {
                    op: BinOp::FAdd,
                    a: i32c(1),
                    b: i32c(2),
                    ty: CTy::Float(FloatKind::F64),
                },
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(r.fidelity(), Fidelity::Exact);
    assert!(seal(&r, r.witness()).is_err());
    assert!(
        r.states()[0].local(ValueId(0)).is_none(),
        "an unmodeled operation produces no value at all, rather than a wrong one"
    );
}

/// **A `Goto` to a block that does not exist must terminate, not spin.** `step` returned
/// `None` without setting a status and the run loop spun forever — no allocation, so not
/// even the OOM killer would end it. §2's "step is total" was false.
#[test]
fn a_branch_to_a_missing_block_terminates_the_state() {
    let mut a = TermArena::new();
    let m = func(
        vec![block(0, vec![], Terminator::Goto(BlockId(99)))],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 1);
    assert!(matches!(r.states()[0].status, Status::Errored(_)));
    assert_ne!(r.fidelity(), Fidelity::Exact);
}

/// **An alloca is sized by its element type.** Every one was `count * 8`, so
/// `char buf[4]` became a 32-byte object and single-byte writes at offsets 4 through 31
/// produced no fault at all — the memory model eight times too permissive for exactly
/// the buffers overflows happen in.
#[test]
fn an_alloca_is_sized_by_its_element_type() {
    for (ty, count, want) in [
        (CTy::Int(8), 4u64, 4u64),
        (CTy::Int(32), 4, 16),
        (CTy::Int(64), 4, 32),
        (CTy::Ptr, 2, 16),
    ] {
        let mut m = func(
            vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
            CTy::Int(32),
        );
        m.funcs[0].allocas = vec![AllocaDecl {
            id: AllocaId(0),
            ty: ty.clone(),
            count,
            align: 1,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }];
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_eq!(
            r.states()[0].object_size_for_test(),
            Some(want),
            "{ty:?} x {count}"
        );
    }
}

/// A dynamic extent must not overflow the size computation. `DYNAMIC_EXTENT` is
/// `u64::MAX`, so `count * elem_size` panicked in debug and wrapped to an arbitrary
/// *small* object in release — which is the more dangerous of the two, since a wrapped
/// size silently accepts or rejects the wrong accesses.
///
/// The extent is zero until an `AllocaDyn` supplies it at a program point (020 §3), so
/// what this pins is that the size is the *declared* absence of one rather than a
/// wrapped number that looks like a real bound.
#[test]
fn a_dynamic_extent_does_not_overflow_the_size_computation() {
    let mut m = func(
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    m.funcs[0].allocas = vec![AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(32),
        count: chiero_cir::DYNAMIC_EXTENT,
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    }];
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.states().len(),
        1,
        "the run completes rather than panicking"
    );
    assert_eq!(
        r.states()[0].object_size_for_test(),
        Some(0),
        "no extent yet; `AllocaDyn` supplies it, and a wrapped number would masquerade \
         as a real bound"
    );
}

/// **`max_depth` counts instructions**, as §8 documents. It was counted in `take_edge`,
/// so a single block of any length was unbounded.
#[test]
fn max_depth_counts_instructions_not_edges() {
    let insts: Vec<Inst> = (0..500)
        .map(|k| {
            inst(InstKind::Assign {
                dst: ValueId(k),
                rv: RValue::Use(i32c(1)),
            })
        })
        .collect();
    let m = func(
        vec![block(0, insts, Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_depth: 100,
            ..Budget::default()
        })
        .run(&mut a);
    assert_eq!(r.states()[0].status, Status::Terminated(TermReason::Budget));
    assert_eq!(r.states()[0].fidelity(), Fidelity::Bounded);
}

/// **`feasible` asserts the path condition.** Every solver query the suite made had an
/// *empty* path, so nothing verified that earlier branch decisions constrain later ones —
/// which is the whole mechanism by which symbolic execution is not just enumeration.
#[test]
fn a_later_branch_sees_the_constraints_of_an_earlier_one() {
    let Some(backend) = z3_or_skip("a_later_branch_sees_the_constraints_of_an_earlier_one") else {
        return;
    };
    let mut a = TermArena::new();
    // x = fresh; if (x <u 5) { if (x <u 5) A else B } else C
    // The inner false side is refuted by the outer true constraint.
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
                        rv: RValue::Cmp {
                            op: CmpOp::ULt,
                            a: Operand::Value(ValueId(0)),
                            b: i32c(5),
                            ty: CTy::Int(32),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(4),
                },
            ),
            block(
                1,
                vec![],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(2),
                    f: BlockId(3),
                },
            ),
            block(2, vec![], Terminator::Return(Some(i32c(10)))),
            block(3, vec![], Terminator::Return(Some(i32c(11)))),
            block(4, vec![], Terminator::Return(Some(i32c(12)))),
        ],
        CTy::Int(32),
    );
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let rets: Vec<_> = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut a))
        .collect();
    assert!(
        !rets.contains(&Some(11)),
        "the inner false branch contradicts the outer true one: {rets:?}"
    );
    assert_eq!(rets.len(), 2, "{rets:?}");
}

/// **One backend process per run** (022 §4). A fresh `TieredSolver` per query spawned one
/// process per escalation and discarded the cache every time, so its hit rate was
/// structurally zero — and §1.1's argument that sibling states hit the caches constantly
/// was describing something that could not happen.
#[test]
fn a_run_uses_one_backend_process_for_all_its_queries() {
    let Some(backend) = z3_or_skip("a_run_uses_one_backend_process_for_all_its_queries") else {
        return;
    };
    let mut a = TermArena::new();
    let m = undecidable_branch_module();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    assert!(r.solver_calls > 0, "this program does escalate");
    assert_eq!(
        r.backend_spawns, 1,
        "022 §4 wants one process for the whole run, not one per query"
    );
    // `backend_spawns` alone cannot tell one solver from many — a fresh solver reports
    // one spawn for its own first query, the same number the correct implementation
    // reports for the whole run. Counting the solvers is what distinguishes them, and
    // it is the caches, not just the process, that a rebuild throws away.
    assert_eq!(
        r.solver_inits, 1,
        "one solver, so the caches survive between queries"
    );
}

// ---------------------------------------------------------------------------
// Sealing the proof surface (023 §7.1, contract 13).
// ---------------------------------------------------------------------------

/// **`seal` is the only function in the workspace that reads a run's fidelity to decide
/// whether a result may be presented as a proof.** There were two — `witness()` gated
/// minting as well — which meant the branch `seal` exists for was unreachable from any
/// test, and contract 13b asks for it to be property-tested at all four levels.
///
/// Minting is now unconditional and bound to the run; the *decision* lives in one place.
#[test]
fn seal_is_the_only_thing_that_decides_whether_a_result_is_proven() {
    let mut a = TermArena::new();
    let exact = func(
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
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

    let r1 = Engine::new(&exact).run(&mut a);
    assert!(seal(&r1, r1.witness()).is_ok());

    // A witness is minted for a degraded run too — and `seal` refuses it. That is the
    // branch contract 13b is about, and it was unreachable while `witness()` also judged.
    let r2 = Engine::new(&degraded).run(&mut a);
    match seal(&r2, r2.witness()) {
        Err(NotProven {
            fidelity,
            assumptions,
        }) => {
            assert_ne!(fidelity, Fidelity::Exact);
            assert!(!assumptions.is_empty(), "and it says why");
        }
        Ok(_) => panic!("a degraded run must not be sealed"),
    }

    // A witness from another run is still refused, even between two exact runs.
    let r3 = Engine::new(&exact).run(&mut a);
    assert!(seal(&r1, r3.witness()).is_err());
}

/// **A degraded run cannot be laundered by editing the result.** `State::fidelity` and
/// `RunResult`'s fields were public, so a downstream crate could set a real degraded run
/// to `Exact` — or hand-build a `RunResult` carrying another run's id and no states — and
/// `seal` would bless it. §7.1's "the type system prevents downstream crates from forging
/// a proof" was false as written.
#[test]
fn a_runs_verdict_cannot_be_edited_after_the_fact() {
    let mut a = TermArena::new();
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
    let r = Engine::new(&degraded).run(&mut a);
    // Everything a consumer can reach is read-only, so there is nothing to overwrite.
    assert_eq!(r.states()[0].fidelity(), Fidelity::Approximated);
    assert!(!r.states()[0].assumptions().is_empty());
    assert!(seal(&r, r.witness()).is_err());
}

/// **023 contract 13b: the decision is exercised at all four fidelity levels**, not just
/// the two a run happens to produce. `Fidelity::Exact` is the only one that seals.
#[test]
fn only_exact_seals_across_every_fidelity_level() {
    for f in [
        Fidelity::Exact,
        Fidelity::Bounded,
        Fidelity::Approximated,
        Fidelity::Unknown,
    ] {
        assert_eq!(
            Fidelity::Exact.degrade(f) == Fidelity::Exact,
            f == Fidelity::Exact,
            "{f:?} is sealable iff it is Exact"
        );
    }
}

/// **023 contract 12's anti-dummy clause.** A degraded state's assumption must match the
/// *cause*; `matches` returning true for everything satisfied the only caller, which is
/// exactly the dummy the spec says must not pass.
#[test]
fn an_assumption_of_the_wrong_kind_does_not_account_for_a_degradation() {
    assert!(AssumptionKind::BudgetHit.matches(Fidelity::Bounded));
    assert!(!AssumptionKind::BudgetHit.matches(Fidelity::Approximated));
    assert!(!AssumptionKind::BudgetHit.matches(Fidelity::Unknown));
    assert!(AssumptionKind::OpaqueCode.matches(Fidelity::Approximated));
    assert!(!AssumptionKind::OpaqueCode.matches(Fidelity::Bounded));
    assert!(AssumptionKind::SolverUnknown.matches(Fidelity::Unknown));
    assert!(!AssumptionKind::SolverUnknown.matches(Fidelity::Approximated));
    for k in [
        AssumptionKind::BudgetHit,
        AssumptionKind::OpaqueCode,
        AssumptionKind::SolverUnknown,
        AssumptionKind::NoInformation,
        AssumptionKind::UnmodeledCall,
    ] {
        assert!(!k.matches(Fidelity::Exact), "{k:?} cannot explain Exact");
    }
}

/// **Fork order is observable** (023 §1's `PathTrace`). `RunResult::states` is sorted by
/// id, which erases exploration order from the output entirely — so "the true branch is
/// explored first" and contract 6's "identical fork order" had no test, and both "make it
/// BFS" and "delete the sort" survived mutation.
#[test]
fn the_exploration_order_is_recorded_and_puts_the_true_branch_first() {
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
    // The trace records blocks in the order they were entered, per state.
    assert_eq!(
        r.states()[0].trace(),
        &[(FuncId(0), BlockId(0)), (FuncId(0), BlockId(1))],
        "the true branch is explored first"
    );
    assert_eq!(
        r.states()[1].trace(),
        &[(FuncId(0), BlockId(0)), (FuncId(0), BlockId(2))]
    );
    // And the order in which states *completed* is recorded too, so a change of searcher
    // is visible in the output rather than hidden by the sort.
    assert_eq!(r.completion_order(), &[0, 1]);
}

// ---------------------------------------------------------------------------
// The engine consults the model registry (024 contracts 11, 12; §2.1).
// ---------------------------------------------------------------------------

/// **024 contract 12.** A module that *defines* `memcpy` uses its own definition, not the
/// model, and records **no** assumption. A registry that shadowed local definitions would
/// silently analyse a different program than the one on disk — and it would do so most
/// often exactly where a project has reimplemented a libc function for a reason.
#[test]
fn a_locally_defined_function_wins_over_the_model() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    // A local `memcpy` with a body. The registry has a model for that name.
    let local = defined(
        1,
        "memcpy",
        vec![block(0, vec![], Terminator::Return(Some(i32c(77))))],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, local)).run(&mut a);
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(77),
        "the module's own definition ran"
    );
    assert_eq!(r.fidelity(), Fidelity::Exact);
    assert!(
        r.states()[0].assumptions().is_empty(),
        "using a real definition assumes nothing: {:#?}",
        r.states()[0].assumptions()
    );
}

/// **024 §2.1 through the engine.** Dispatching an `Approximate` model degrades the run
/// and records the model's own reason — so a program calling `scanf` cannot be sealed
/// (contract 21b). This is the *modeled* path, which is more dangerous than the unmodeled
/// one because it looks deliberate.
#[test]
fn calling_an_approximate_model_degrades_the_run_with_its_reason() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut ext = defined(1, "scanf", vec![], CTy::Int(32));
    ext.body = Body::Declared;

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, ext)).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Approximated);
    assert!(seal(&r, r.witness()).is_err(), "contract 21b");
    let note = &r.states()[0].assumptions()[0];
    assert_eq!(note.kind, AssumptionKind::ModelApproximate);
    assert!(
        note.detail.contains("input"),
        "the model's own reason must reach the report: {note:?}"
    );
}

/// An **exact** model does not degrade. Without this the test above is satisfied by an
/// engine that degrades on every call into the registry, and the `Exact`/`Approximate`
/// distinction would carry no information at all.
#[test]
fn calling_an_exact_model_leaves_the_run_exact() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut ext = defined(1, "memset", vec![], CTy::Void);
    ext.body = Body::Declared;

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, ext)).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    assert!(seal(&r, r.witness()).is_ok());
}

/// **024 contract 11.** An extern with *no* model still returns a fresh value, sets
/// `Approximated`, and records the name **exactly once** — a call in a loop must not
/// stack up one assumption per iteration, or the report drowns the finding it is meant
/// to explain.
#[test]
fn an_unmodeled_extern_is_recorded_exactly_once_per_function() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Call {
                    dst: Some(ValueId(0)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(1)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(2)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut ext = defined(1, "some_unmodeled_thing", vec![], CTy::Int(32));
    ext.body = Body::Declared;

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, ext)).run(&mut a);
    let named: Vec<_> = r.states()[0]
        .assumptions()
        .iter()
        .filter(|x| x.detail.contains("some_unmodeled_thing"))
        .collect();
    assert_eq!(named.len(), 1, "three calls, one assumption: {named:#?}");
    assert_eq!(r.fidelity(), Fidelity::Approximated);
    // Each call still yields its **own** fresh value; deduplicating the *report* must not
    // deduplicate the values, or two calls to `rand()` would return the same number.
    let (v0, v1) = (
        r.states()[0].local(ValueId(0)),
        r.states()[0].local(ValueId(1)),
    );
    assert!(v0.is_some() && v1.is_some());
    assert_ne!(v0, v1, "two calls, two symbols");
}

// ---------------------------------------------------------------------------
// Wave 13, from the registry/call review.
// ---------------------------------------------------------------------------

/// **A `Return` whose operand cannot be evaluated must not end at `Exact`.** Every other
/// consumer of `operand` routes `None` through the lowering-gap rule; this one dropped it
/// silently, so `ret %0` with `%0` never assigned — or `ret 2.0f64`, which `operand`
/// cannot represent — minted a proof. It is the same family
/// `nothing_unmodeled_can_end_at_exact` was written to close, one position further along.
#[test]
fn a_return_of_an_unevaluable_operand_degrades() {
    for (what, ret) in [
        ("an unassigned value", Operand::Value(ValueId(9))),
        (
            "an unrepresentable constant",
            Operand::Const(Const::Float(FloatKind::F64, 0x4000_0000_0000_0000)),
        ),
    ] {
        let m = func(
            vec![block(0, vec![], Terminator::Return(Some(ret)))],
            CTy::Int(32),
        );
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_ne!(r.fidelity(), Fidelity::Exact, "returning {what}");
        assert!(seal(&r, r.witness()).is_err(), "returning {what} sealed");
    }
}

/// **A `noreturn` function with a body still runs it.** 023 §5's "noreturn terminates the
/// state at the call" is about the call not returning, not about discarding a body that
/// exists — and `__attribute__((noreturn)) void die(…) { …; abort(); }` is ordinary C.
/// Skipping it made every bug inside such a function invisible while the run still sealed.
#[test]
fn a_defined_noreturn_function_still_executes_its_body() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut die = defined(
        1,
        "die",
        vec![block(
            0,
            vec![],
            Terminator::Unreachable(UnreachableReason::LoweringGap),
        )],
        CTy::Void,
    );
    die.attrs.noreturn = true;

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, die)).run(&mut a);
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "the body contains a lowering gap, which the run must see"
    );
    assert!(
        r.states()[0].trace().iter().any(|(f, _)| *f == FuncId(1)),
        "the callee's body was entered"
    );
}

/// **Allocas are per activation.** `AllocaId` is unique only *within* a function, so
/// `AllocaId(0)` in every function is normal — and one map per state made a callee's
/// 100-byte local *be* the caller's 4-byte object. Writes alias and bounds are wrong in
/// both directions, silently, at `Exact`. 023 §1 puts `frame_objs` on the `Frame` for
/// exactly this reason.
#[test]
fn each_activation_materializes_its_own_allocas() {
    let caller = {
        let mut f = defined(
            0,
            "main",
            vec![block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    }),
                    inst(InstKind::Call {
                        dst: Some(ValueId(1)),
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![],
                    }),
                ],
                Terminator::Return(Some(Operand::Value(ValueId(1)))),
            )],
            CTy::Int(32),
        );
        f.allocas = vec![alloca(0, CTy::Int(8), 4)];
        f
    };
    let callee = {
        let mut f = defined(
            1,
            "callee",
            vec![block(
                0,
                vec![inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                })],
                // The callee does **not** return, so both frames are still live when the
                // run ends. Otherwise its frame is popped and the test can only see one,
                // which is what a shared map looks like from outside.
                Terminator::Unreachable(UnreachableReason::BuiltinUnreachable),
            )],
            CTy::Int(32),
        );
        f.allocas = vec![alloca(0, CTy::Int(8), 100)];
        f
    };
    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, callee)).run(&mut a);
    let sizes = r.states()[0].alloca_sizes_for_test();
    assert_eq!(
        sizes.len(),
        2,
        "two activations, two objects — not one shared by AllocaId: {sizes:?}"
    );
    assert!(sizes.contains(&4) && sizes.contains(&100), "{sizes:?}");
}

fn alloca(id: u32, ty: CTy, count: u64) -> AllocaDecl {
    AllocaDecl {
        id: AllocaId(id),
        ty,
        count,
        align: 1,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    }
}

/// **Loop budgets are per `(function, back edge)`** (023 §8). One map keyed on block ids
/// alone made two functions that both loop `b2 -> b1` share a counter — a false "not found
/// within a bound" on a program comfortably inside it, and a lost proof.
#[test]
fn two_functions_with_the_same_block_numbers_do_not_share_a_loop_budget() {
    // A back edge taken exactly **once**, so the state survives to reach the second
    // function. `br true` loops forever and the bound terminates the state before the
    // caller's own edge is ever taken — which is what the first draft of this test did.
    //
    //   b0 -> b2  (forward)
    //   b2 -> b1  (back edge: 1 <= 2, counted once)
    //   b1 -> b3  (forward)
    //   b3: return
    let mk = |id: u32, name: &str, call: Option<FuncId>| {
        let mut b0 = vec![];
        if let Some(c) = call {
            b0.push(inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(c),
                args: vec![],
            }));
        }
        defined(
            id,
            name,
            vec![
                block(0, b0, Terminator::Goto(BlockId(2))),
                block(1, vec![], Terminator::Goto(BlockId(3))),
                block(2, vec![], Terminator::Goto(BlockId(1))),
                block(3, vec![], Terminator::Return(Some(i32c(0)))),
            ],
            CTy::Int(32),
        )
    };
    let mut a = TermArena::new();
    let r = Engine::new(&Module {
        funcs: vec![mk(0, "outer", Some(FuncId(1))), mk(1, "inner", None)],
        ..Default::default()
    })
    .with_budget(Budget {
        max_loop_iters: 3,
        ..Budget::default()
    })
    .run(&mut a);

    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "one traversal each is inside the bound"
    );
    // The keys are what distinguishes a per-function counter from a global one. A budget
    // *note* would name the function from `s.func()` — a different source than the key —
    // so asserting on the note cannot tell the two apart.
    let keys = r.states()[0].loop_keys_for_test();
    let funcs: std::collections::BTreeSet<_> = keys.iter().map(|(f, _, _)| f.0).collect();
    assert_eq!(
        funcs.len(),
        2,
        "both functions traverse `b2 -> b1`; one key means one shared budget: {keys:?}"
    );
}

/// **A module with no functions is not a crash.** `funcs[0]` panicked, and
/// `Module::default()` is what three of the four proof-surface probes construct.
#[test]
fn an_empty_module_is_an_error_not_a_panic() {
    let m = Module::default();
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 1);
    assert!(matches!(r.states()[0].status, Status::Errored(_)));
    assert!(seal(&r, r.witness()).is_err());
}

/// **Call arguments reach the callee.** They were accepted and discarded, so
/// `int id(int x) { return x; }` called with 42 returned nothing — silently, because the
/// dropped return was itself silent.
#[test]
fn call_arguments_are_bound_to_the_callees_parameters() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![i32c(42)],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let mut id = defined(
        1,
        "id",
        vec![block(
            0,
            vec![],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    id.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Int(32),
    }];

    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, id)).run(&mut a);
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(42));
    assert_eq!(r.fidelity(), Fidelity::Exact);
}

// ---------------------------------------------------------------------------
// Indirect calls, and the budgets contract 18 needs (023 §5, §8).
// ---------------------------------------------------------------------------

/// **023 contract 10.** An indirect call resolvable to *n* functions forks into `n + 1`
/// states — one per candidate plus one reported "unresolvable callee".
///
/// VPP's node dispatch is indirect calls through registration tables, so §5 calls this
/// path load-bearing rather than exotic. The extra state matters as much as the
/// candidates: dropping it would silently claim the list is exhaustive, and a function
/// pointer that came from anywhere else would simply not be explored.
#[test]
fn an_indirect_call_forks_per_candidate_plus_one_unresolvable() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Ptr },
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(1)),
                    callee: Callee::Indirect(Operand::Value(ValueId(0))),
                    args: vec![],
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    let a1 = defined(
        1,
        "one",
        vec![block(0, vec![], Terminator::Return(Some(i32c(11))))],
        CTy::Int(32),
    );
    let a2 = defined(
        2,
        "two",
        vec![block(0, vec![], Terminator::Return(Some(i32c(22))))],
        CTy::Int(32),
    );
    let a3 = defined(
        3,
        "three",
        vec![block(0, vec![], Terminator::Return(Some(i32c(33))))],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, a1, a2, a3],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 4, "three candidates plus unresolvable");
    let rets: Vec<_> = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut a))
        .collect();
    for want in [11u128, 22, 33] {
        assert!(
            rets.contains(&Some(want)),
            "candidate {want} missing: {rets:?}"
        );
    }
    // The unresolvable state is reported, not silently dropped.
    assert!(
        r.states().iter().any(|s| s
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("unresolvable"))),
        "the callee may point somewhere chiero does not know about"
    );
    assert_ne!(r.fidelity(), Fidelity::Exact, "that state knows nothing");
}

/// **023 contract 10's cap.** With `max_indirect = 2` the same call yields `Bounded` and
/// records the cap. A cap that silently truncated would report "no bug found" over a
/// dispatch table it had only partly explored.
#[test]
fn the_indirect_cap_is_bounded_and_recorded() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Ptr },
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(1)),
                    callee: Callee::Indirect(Operand::Value(ValueId(0))),
                    args: vec![],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mk = |i: u32| {
        defined(
            i,
            &format!("f{i}"),
            vec![block(0, vec![], Terminator::Return(Some(i32c(i as i128))))],
            CTy::Int(32),
        )
    };
    let m = Module {
        funcs: vec![caller, mk(1), mk(2), mk(3), mk(4)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_indirect: 2,
            ..Budget::default()
        })
        .run(&mut a);
    assert!(
        r.states().iter().any(|s| s
            .assumptions()
            .iter()
            .any(|x| x.kind == AssumptionKind::BudgetHit && x.detail.contains("max_indirect"))),
        "the cap must be recorded: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .collect::<Vec<_>>()
    );
    // Recording the cap is not the same as **applying** it: four candidates under a cap
    // of two must produce two forks plus the unresolvable state, not four plus one.
    assert_eq!(
        r.states().len(),
        3,
        "two candidates and one unresolvable: {:?}",
        r.states().iter().map(|s| s.id.0).collect::<Vec<_>>()
    );
}

/// **023 §8's remaining deterministic budgets**, without which contract 18 cannot be
/// written. Every budget is reported whether or not it was hit, so a reader can tell
/// `Exact`-with-generous-bounds from `Exact`-with-trivial-bounds — which are very
/// different claims wearing the same word.
#[test]
fn every_deterministic_budget_is_present_and_reported() {
    let b = Budget::default();
    assert!(b.max_depth > 0);
    assert!(b.max_loop_iters > 0);
    assert!(b.max_recursion_depth > 0);
    assert!(b.max_states > 0);
    assert!(b.max_forks > 0);
    assert!(b.max_indirect > 0);

    let m = func(
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    // A **non-default** budget, or the assertion is satisfied by a `budget()` that
    // returns the default and ignores what the run actually used.
    let used = Budget {
        max_depth: 7,
        max_loop_iters: 5,
        max_recursion_depth: 3,
        max_states: 11,
        max_forks: 13,
        max_indirect: 2,
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_budget(used).run(&mut a);
    assert_eq!(
        r.budget(),
        used,
        "the bounds in force are part of the result, hit or not"
    );
}

/// `max_forks` bounds a run that would otherwise fork without limit, and says so.
#[test]
fn the_fork_cap_bounds_a_run_and_is_recorded() {
    let mut a = TermArena::new();
    // Four sequential symbolic branches: 16 paths without a cap.
    let mut blocks = Vec::new();
    for i in 0..4u32 {
        blocks.push(block(
            i,
            vec![inst(InstKind::Assign {
                dst: ValueId(i),
                rv: RValue::Fresh { ty: CTy::Int(1) },
            })],
            Terminator::Br {
                cond: Operand::Value(ValueId(i)),
                t: BlockId(i + 1),
                f: BlockId(i + 1),
            },
        ));
    }
    blocks.push(block(4, vec![], Terminator::Return(Some(i32c(0)))));
    let m = func(blocks, CTy::Int(32));
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_forks: 3,
            ..Budget::default()
        })
        .run(&mut a);
    assert!(
        r.states().iter().any(|s| s
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("max_forks"))),
        "the cap must be recorded rather than silently truncating exploration"
    );
    assert_ne!(r.fidelity(), Fidelity::Exact);
}

/// **023 §1's `PathTrace` must be replayable**, which means naming the function as well
/// as the block. A trace of bare block ids reads as one function walking a path it never
/// took: a caller sitting in `b0` that calls a callee going `b0 -> b1` looks like
/// `main: 0 -> 1`.
#[test]
fn the_trace_records_which_function_each_block_belongs_to() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let callee = defined(
        1,
        "callee",
        vec![
            block(0, vec![], Terminator::Goto(BlockId(1))),
            block(1, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        CTy::Int(32),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&two_funcs(caller, callee)).run(&mut a);
    let t = r.states()[0].trace();
    assert!(
        t.iter().any(|(f, _)| *f == FuncId(1)),
        "the callee's blocks must be attributed to the callee: {t:?}"
    );
    assert!(
        t.iter().any(|(f, _)| *f == FuncId(0)),
        "and the caller's to the caller: {t:?}"
    );
}
