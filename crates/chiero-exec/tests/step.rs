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
//! detectable OOB into `UNBOUND`, and contract 3 of 021 becomes unimplementable.
//! (Deliberately not written as a citation: nothing here tests it.)
//!
//! **A branch the solver could not decide is taken anyway.** Dropping it would let "no
//! bug found" mean "the solver timed out", which §7 forbids — and the resulting fidelity
//! is `Unknown`, not `Approximated`, because the engine genuinely does not know whether
//! that path exists. That distinction is the difference between "I modeled this loosely"
//! and "I have no idea".

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::{Sort, TermArena};
use chiero_span::{BytePos, ExpnCtx, Span};

fn inst(kind: InstKind) -> Inst {
    Inst {
        kind,
        span: Span::DUMMY,
    }
}

/// An instruction with a *distinct* span. Almost every fixture here uses `Span::DUMMY`,
/// which is why the finding key's `span` component was unpinned: with one span everywhere,
/// dropping it from the key changes nothing.
fn inst_at(kind: InstKind, lo: u32) -> Inst {
    Inst {
        kind,
        span: Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0)),
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
            // A store *through an integer* is still a gap — there is no object to write
            // to. A store through `Const::Null` used to be here and no longer belongs:
            // stores are implemented now, so that one is a definite null-dereference
            // finding with nothing approximate about it.
            "a store through a non-pointer address",
            func(
                vec![block(
                    0,
                    vec![inst(InstKind::Store {
                        addr: i32c(4096),
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
    // **Rescoped when verification was wired in.** 020 requires an `AllocaDyn` to supply
    // a dynamic extent, so this module is now correctly *rejected* before it runs, and
    // the engine's frame setup is never reached. What survives is the half that matters
    // for a tool — a malformed module is an error, not a panic or a wrapped size.
    // The `object_size_for_test` assertion is owed again once `AllocaDyn` is implemented;
    // recorded in HANDOFF rather than deleted quietly.
    assert!(
        matches!(r.states()[0].status, Status::Errored(_)),
        "{:?}",
        r.states()[0].status
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

/// **A model that is registered but not *dispatched* must not make the run more
/// confident than no model at all.**
///
/// The engine looked up a name's `Precision` and, seeing `Exact`, recorded nothing —
/// while never running the model. So `strcpy` into a four-byte buffer finished `Exact`
/// and `seal` returned a proof, for a textbook overflow. An *unregistered* name degrades
/// and refuses the seal, so registering a correct implementation made the engine **less**
/// safe: adding `strlen` and `strcpy` to the builtin list removed the degradation those
/// calls previously caused.
///
/// `Exact` describes the model's faithfulness *if it runs*. Nothing about it licenses a
/// claim over a call the engine never made.
#[test]
fn a_registered_model_the_engine_cannot_dispatch_still_degrades() {
    for name in ["strcpy", "memcpy", "malloc", "strlen"] {
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
        let mut ext = defined(1, name, vec![], CTy::Int(32));
        ext.body = Body::Declared;
        let mut a = TermArena::new();
        let r = Engine::new(&two_funcs(caller, ext)).run(&mut a);
        assert_ne!(
            r.fidelity(),
            Fidelity::Exact,
            "`{name}` was never run, so nothing about the call is exact"
        );
        assert!(seal(&r, r.witness()).is_err(), "`{name}` sealed a proof");
        assert!(
            r.states()[0]
                .assumptions()
                .iter()
                .any(|x| x.detail.contains(name)),
            "the assumption must name the call: {:#?}",
            r.states()[0].assumptions()
        );
    }
}

/// An **exact** model that the engine *does* dispatch leaves the run exact. Without this
/// the rule above collapses into "every registry call degrades", and the
/// `Exact`/`Approximate` distinction would carry no information at all.
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
    // `chiero_assume(true)` is dispatchable with no arguments the engine must translate,
    // so it exercises the "ran it, and the model is faithful" path rather than the
    // "registered but unreachable" one.
    let mut ext = defined(1, "chiero_assume", vec![], CTy::Void);
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
        max_resolutions: 4,
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

// ---------------------------------------------------------------------------
// Real model dispatch (024 contracts 1, 5, 9, 10 end to end).
// ---------------------------------------------------------------------------

fn extern_fn(id: u32, name: &str, params: Vec<CTy>, ret: CTy) -> Function {
    let mut f = defined(id, name, vec![], ret);
    f.params = params
        .into_iter()
        .enumerate()
        .map(|(i, ty)| Param {
            value: ValueId(i as u32),
            ty,
        })
        .collect();
    f.body = Body::Declared;
    f
}

/// **024 contract 1, through the engine.** `malloc(16)` gives the caller a *pointer* —
/// `Value::Ptr`, not a bare scalar — to a 16-byte heap object, and the run stays `Exact`
/// because the model ran.
///
/// The provenance matters at exactly this call: 023 §1.1 says a pointer keeps its
/// `ObjectId`, and `malloc` is where most heap objects come from, so losing it here loses
/// it everywhere downstream.
#[test]
fn malloc_dispatches_and_returns_a_real_pointer() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![Operand::Const(Const::Int { bits: 64, val: 16 })],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, extern_fn(1, "malloc", vec![CTy::Int(64)], CTy::Ptr)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_alloc_policy(chiero_model::AllocPolicy { may_fail: false })
        .run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => {
            assert_ne!(p.base, chiero_mem::ObjectId::NULL);
            assert_eq!(p.off, 0);
        }
        other => panic!("malloc must yield a pointer, got {other:?}"),
    }
}

/// **024 contract 9, end to end.** `strcpy` into a four-byte destination from a ten-byte
/// source is one finding — the textbook overflow this whole layer exists to catch, and
/// the case that was finishing `Exact` and sealing a proof until dispatch existed.
#[test]
fn strcpy_into_a_short_buffer_is_found_through_the_engine() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                // Ten non-zero bytes and a terminator, so the source is a real
                // ten-character string rather than an uninitialized read.
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(1)),
                        Operand::Const(Const::Int { bits: 32, val: 120 }),
                        Operand::Const(Const::Int { bits: 64, val: 10 }),
                    ],
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(1)),
                        off: Operand::Const(Const::Int { bits: 64, val: 10 }),
                    },
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(2)),
                        Operand::Const(Const::Int { bits: 32, val: 0 }),
                        Operand::Const(Const::Int { bits: 64, val: 1 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4), alloca(1, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "strcpy", vec![CTy::Ptr, CTy::Ptr], CTy::Ptr),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(r.fidelity(), Fidelity::Exact);
    // **The finding has to be the destination check.** This test asserted only
    // "not `Exact`" and "some finding", and the source was never initialized — so
    // `strlen` faulted on byte 0, `strcpy` bailed before the bounds check, and both
    // assertions passed on an uninitialized-read report. Swapping `dst` and `src` gave
    // the same two answers, because the destination is uninitialized too. The classic
    // overflow had no end-to-end evidence at all. Found by review.
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("destination holds 4 bytes and the source needs 11")),
        "the destination bounds check ran: {:#?}",
        r.findings()
    );
}

/// **024 contract 5, through the engine.** `free` of a stack object is a finding, and
/// `free(NULL)` is not — the model's verdict has to reach the run rather than staying
/// inside `ModelCtx`.
#[test]
fn free_of_a_stack_object_is_found_through_the_engine() {
    let mut caller = defined(
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
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller, extern_fn(1, "free", vec![CTy::Ptr], CTy::Void)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("bad-free")),
        "freeing a stack object is a finding: {:#?}",
        r.findings()
    );
}

/// A dispatched model that finds nothing leaves the run **exact and silent**. Without
/// this the tests above are satisfied by dispatch that reports on every call, and every
/// program touching libc would carry a finding.
#[test]
fn a_dispatched_model_with_nothing_to_report_stays_exact() {
    let mut caller = defined(
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
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        // **Neither argument is 0 or 8.** With `(byte 0, size 8)` a
                        // model that swapped them wrote zero bytes, which reports the
                        // same "nothing happened" as writing eight zeroes — the
                        // same-answer trap, and `memset` had no other engine-level test.
                        Operand::Const(Const::Int {
                            bits: 32,
                            val: 0xAB,
                        }),
                        Operand::Const(Const::Int { bits: 64, val: 6 }),
                    ],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(
                1,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
    assert!(seal(&r, r.witness()).is_ok());
    // And it actually wrote: six 0xAB bytes, and the seventh still unreadable.
    let base = match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = r.states()[0].mem.clone();
    assert_eq!(
        mem.read(chiero_mem::Pointer { base, off: 0 }, 6, Span::DUMMY)
            .value,
        Some(vec![0xAB; 6])
    );
    assert!(
        !mem.read(chiero_mem::Pointer { base, off: 6 }, 1, Span::DUMMY)
            .faults
            .is_empty(),
        "the write stopped where it was told to"
    );
}

/// **`can_dispatch` and the model implementations cannot drift.** Both were hand-written
/// lists, so a name could be dispatchable with nothing behind it, or implemented and
/// unreachable — and mutation showed neither direction was pinned.
#[test]
fn everything_dispatchable_is_implemented_and_vice_versa() {
    for n in chiero_model::dispatchable() {
        assert!(
            chiero_model::models::is_implemented(n),
            "`{n}` is dispatchable with no implementation"
        );
    }
    // **The other direction, or the check is one-sided.** `is_implemented` returning
    // `true` unconditionally passed the loop above — every assertion in it was satisfied
    // by a function that says yes to everything. These are registered models chiero has
    // *no* implementation for, which is exactly what the flag exists to distinguish.
    for n in ["printf", "sqrt", "read", "ioctl", "not_a_function_at_all"] {
        assert!(
            !chiero_model::models::is_implemented(n),
            "`{n}` has no implementation here and must not claim one"
        );
    }
    let r = chiero_model::ModelRegistry::with_builtins();
    for n in chiero_model::dispatchable() {
        assert!(
            r.lookup(n).is_some(),
            "`{n}` is dispatchable but unregistered"
        );
    }
}

/// **The list has to mean something.** `everything_dispatchable_is_implemented_and_vice_versa`
/// could not fail while `is_implemented` began with `DISPATCHABLE.contains(&name)` — it
/// was true for every name in the list by construction. So this calls each dispatchable
/// name **through the engine, with arguments it can actually use**, and requires the
/// engine not to say it gave up. Adding a name to `DISPATCHABLE` with no match arm behind
/// it fails here.
///
/// The arguments are per-name rather than uniform because the arities and kinds differ,
/// and a uniform list would land every name in the argument-translation gap — passing for
/// the wrong reason, which is how the previous test passed.
#[test]
fn every_dispatchable_name_is_actually_performed_by_the_engine() {
    // (name, params, args). `ValueId(0)` and `ValueId(1)` are pointers to two 16-byte
    // locals; the source is filled and terminated before every call.
    let sz = |v: i128| Operand::Const(Const::Int { bits: 64, val: v });
    let i32v = |v: i128| Operand::Const(Const::Int { bits: 32, val: v });
    let p0 = Operand::Value(ValueId(0));
    let p1 = Operand::Value(ValueId(1));
    let cases: Vec<(&str, Vec<CTy>, Vec<Operand>)> = vec![
        ("malloc", vec![CTy::Int(64)], vec![sz(16)]),
        (
            "calloc",
            vec![CTy::Int(64), CTy::Int(64)],
            vec![sz(2), sz(8)],
        ),
        ("free", vec![CTy::Ptr], vec![p0.clone()]),
        (
            "memcpy",
            vec![CTy::Ptr, CTy::Ptr, CTy::Int(64)],
            vec![p0.clone(), p1.clone(), sz(4)],
        ),
        (
            "memmove",
            vec![CTy::Ptr, CTy::Ptr, CTy::Int(64)],
            vec![p0.clone(), p1.clone(), sz(4)],
        ),
        (
            "memset",
            vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
            vec![p0.clone(), i32v(0), sz(4)],
        ),
        ("strlen", vec![CTy::Ptr], vec![p1.clone()]),
        (
            "strcpy",
            vec![CTy::Ptr, CTy::Ptr],
            vec![p0.clone(), p1.clone()],
        ),
        ("chiero_assume", vec![CTy::Int(32)], vec![i32v(1)]),
        ("chiero_assert", vec![CTy::Int(32)], vec![i32v(1)]),
        ("chiero_mark_fidelity", vec![CTy::Ptr], vec![p1.clone()]),
        ("longjmp", vec![], vec![]),
        (
            "scanf",
            vec![CTy::Ptr, CTy::Ptr],
            vec![p1.clone(), p0.clone()],
        ),
    ];
    let names: Vec<&str> = cases.iter().map(|(n, _, _)| *n).collect();
    for n in chiero_model::dispatchable() {
        assert!(names.contains(n), "`{n}` has no case here");
    }

    for (name, params, args) in cases {
        let mut caller = defined(
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
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(1),
                        },
                    }),
                    // Fill and terminate the source, so the string models have a real
                    // string and do not bail before doing their job.
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![p1.clone(), i32v(0), sz(16)],
                    }),
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(1)),
                        args,
                    }),
                ],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        );
        caller.allocas = vec![alloca(0, CTy::Int(8), 16), alloca(1, CTy::Int(8), 16)];
        let m = Module {
            funcs: vec![
                caller,
                extern_fn(1, name, params, CTy::Ptr),
                extern_fn(
                    2,
                    "memset",
                    vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                    CTy::Ptr,
                ),
            ],
            ..Default::default()
        };
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        for st in r.states() {
            for x in st.assumptions() {
                assert!(
                    !x.detail.contains("could not be dispatched")
                        && !x.detail.contains("cannot dispatch")
                        && !x.detail.contains("no body and no model"),
                    "`{name}` claims to be dispatchable: {}",
                    x.detail
                );
            }
        }
    }
}

/// **`can_dispatch` itself, pinned.** `a_registered_model_the_engine_cannot_dispatch_still_degrades`
/// called each name with `args: vec![]`, so once those names became dispatchable it kept
/// passing through the *argument-translation* gap rather than through the refusal it
/// exists to check — `can_dispatch → true` survived as a mutation. This registers an
/// `Exact` model for a name the engine has no arm for and calls it with **valid**
/// arguments, so only the refusal can produce the degradation.
#[test]
fn an_exact_model_with_no_engine_arm_degrades_on_valid_arguments() {
    let mut caller = defined(
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
                    args: vec![Operand::Const(Const::Int { bits: 32, val: 8 })],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "__builtin_clz", vec![CTy::Int(32)], CTy::Int(32)),
        ],
        ..Default::default()
    };
    let reg = chiero_model::ModelRegistry::with_builtins();
    assert_eq!(
        reg.lookup("__builtin_clz").map(|e| e.precision.clone()),
        Some(chiero_model::Precision::Exact),
        "the premise: registered, and registered as exact"
    );
    assert!(
        !chiero_model::dispatchable().contains(&"__builtin_clz"),
        "the premise: implemented in chiero-model, no arm in the engine"
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_models(reg).run(&mut a);
    assert_ne!(r.fidelity(), Fidelity::Exact);
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("cannot dispatch")),
        "the refusal, not the argument gap: {:#?}",
        r.states()[0].assumptions()
    );
}

/// **024 contract 1's default.** `malloc` forks into success and `NULL`, and both states
/// run. Allocation failure is a real path; §3 says most real allocation-failure bugs are
/// unreachable without it, and it is the *default* — so the engine dropping the fork on
/// the floor and degrading was the common case, not an edge.
#[test]
fn malloc_forks_the_run_into_success_and_failure() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![Operand::Const(Const::Int { bits: 64, val: 16 })],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, extern_fn(1, "malloc", vec![CTy::Int(64)], CTy::Ptr)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 2, "success and NULL");
    let bases: Vec<_> = r
        .states()
        .iter()
        .map(|s| match s.local(ValueId(0)) {
            Some(Value::Ptr(p)) => Some(p.base),
            _ => None,
        })
        .collect();
    assert!(
        bases.contains(&Some(chiero_mem::ObjectId::NULL)),
        "one path gets NULL: {bases:?}"
    );
    assert!(
        bases
            .iter()
            .any(|b| *b != Some(chiero_mem::ObjectId::NULL) && b.is_some()),
        "and one gets an object: {bases:?}"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "a fork is not an approximation: {:#?}",
        r.states()[0].assumptions()
    );
}

/// **024 contract 15.** `chiero_assert(0)` is a finding. The intrinsic ignored its
/// argument and hardcoded "true", so every assertion in a harness passed — which is
/// §7's named way to make a test suite vacuous, arrived at from the other direction.
#[test]
fn chiero_assert_reads_its_condition() {
    for (cond, want_finding) in [(0i128, true), (1, false)] {
        let caller = defined(
            0,
            "main",
            vec![block(
                0,
                vec![inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Const(Const::Int {
                        bits: 32,
                        val: cond,
                    })],
                })],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        );
        let m = Module {
            funcs: vec![
                caller,
                extern_fn(1, "chiero_assert", vec![CTy::Int(32)], CTy::Void),
            ],
            ..Default::default()
        };
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_eq!(
            !r.findings().is_empty(),
            want_finding,
            "chiero_assert({cond}): {:#?}",
            r.findings()
        );
    }
}

/// **024 contract 16.** `chiero_assume(0)` kills the state with **no** finding — it says
/// "this path cannot happen", which is about the harness rather than the program.
#[test]
fn chiero_assume_of_a_contradiction_kills_the_state_silently() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Const(Const::Int { bits: 32, val: 0 })],
                }),
                // Never reached.
                inst(InstKind::Assign {
                    dst: ValueId(9),
                    rv: RValue::Use(i32c(1)),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "chiero_assume", vec![CTy::Int(32)], CTy::Void),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
    assert!(
        r.states()[0].local(ValueId(9)).is_none(),
        "the state died at the assume"
    );
}

/// **023 contract 10, the part the fork-count test could not see.** Every candidate in
/// `an_indirect_call_forks_per_candidate_plus_one_unresolvable` has an *empty* entry
/// block, so a sibling that skipped straight to the terminator returned the same value as
/// one that executed the body — the same-answer trap, in the fixture rather than the
/// assertion. Give the callee an instruction and the two stop agreeing.
#[test]
fn an_indirect_candidate_executes_its_entry_block() {
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
    let callee = defined(
        1,
        "computes",
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(5),
                rv: RValue::Bin {
                    op: BinOp::Add,
                    ty: CTy::Int(32),
                    a: i32c(40),
                    b: i32c(2),
                },
            })],
            Terminator::Return(Some(Operand::Value(ValueId(5)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let rets: Vec<_> = r
        .states()
        .iter()
        .map(|s| s.return_value_bits(&mut a))
        .collect();
    assert!(
        rets.contains(&Some(42)),
        "the candidate's body ran: {rets:?}"
    );
}

/// **024 contract 8 and 023 §7, through the engine.** A `strlen` that hit `max_string_scan`
/// established nothing, and the engine handed the caller a *fresh unconstrained* 64-bit
/// symbol as the length — while staying `Exact` with no assumption recorded, so the run
/// sealed as `Proven`. `StrScan::CapReached` became `ModelOutcome::Value(None)`, which is
/// not the untranslatable arm: `translated` stayed true and the `dst` fallback minted a
/// symbol.
///
/// The consequence is worse than a missed bug: `n = strlen(buf); if (n == 999999) bug();`
/// is *feasible* against an unconstrained `n`, so chiero reports a bug that cannot happen
/// and calls the run a proof. Found by review.
#[test]
fn a_strlen_that_established_nothing_does_not_leave_the_run_exact() {
    let mut caller = defined(
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
                // Filled with 'x' and never terminated, so the scan runs off the end.
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int { bits: 32, val: 120 }),
                        Operand::Const(Const::Int { bits: 64, val: 16 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(1)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "strlen", vec![CTy::Ptr], CTy::Int(64)),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "an unestablished length is not an exact run"
    );
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("strlen")),
        "and the reason names the model: {:#?}",
        r.states()[0].assumptions()
    );
    assert!(
        seal(&r, r.witness()).is_err(),
        "a run carrying a fabricated length cannot seal"
    );
}

/// **A `ModelOutcome::Finding`'s message reaches the run.** `dispatch` matched only
/// `Value`, so `Finding(msg)` fell into the catch-all and the payload was dropped. It
/// looked fine because two of the three producers *also* call `cx.report` — the third,
/// `strcpy`'s source-scan bail, does not, and reported nothing at all. Found by review.
#[test]
fn a_findings_only_outcome_still_reports() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                // 'x' with no terminator: the source scan cannot give a length, so
                // `strcpy` returns a bare `Finding`.
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(1)),
                        Operand::Const(Const::Int { bits: 32, val: 120 }),
                        Operand::Const(Const::Int { bits: 64, val: 16 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4), alloca(1, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "strcpy", vec![CTy::Ptr, CTy::Ptr], CTy::Ptr),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("strcpy")),
        "the model said why it gave up: {:#?}",
        r.findings()
    );
}

/// **`PtrAdd` was a lowering gap** — every pointer walk in a program degraded the run to
/// `Unknown`, which is why building a string in the contract-9 fixture needed a second
/// `memset` that never landed. 020 §4.1 keeps `PtrAdd` distinct from `Add` precisely so
/// provenance survives arithmetic, so the object has to come through unchanged.
///
/// The offset is **signed** (021 §2): vppinfra's vector header lives below the user
/// pointer, so a model that could only step forward could not express `vec_len(v)` at all.
#[test]
fn ptr_add_keeps_the_object_and_takes_a_signed_offset() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 12 }),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(1)),
                        off: Operand::Const(Const::Int { bits: 64, val: -4 }),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    let base = match s.local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        s.local(ValueId(1)),
        Some(Value::Ptr(chiero_mem::Pointer { base, off: 12 }))
    );
    assert_eq!(
        s.local(ValueId(2)),
        Some(Value::Ptr(chiero_mem::Pointer { base, off: 8 }))
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "pointer arithmetic is not an approximation: {:#?}",
        s.assumptions()
    );
}

/// **023 §5 and 024 §1 step 4, end to end.** An unmodeled extern handed a pointer
/// invalidates the pointee. Before this the callee's writes were invisible, so
/// `memset(buf,'x',8); mystery(buf); strlen(buf)` still saw eight concrete `'x'` bytes and
/// `int x = 0; unknown(&x); if (x == 0)` pruned the real path. Fidelity was already
/// `Approximated`, so nothing was *sealed* — the reports were simply untrue, which is the
/// harder failure to notice.
///
/// The assertion is on a *later read*, not on an assumption: an assumption saying "I
/// invalidated it" while the bytes stayed put is exactly the vacuous instrument to avoid.
#[test]
fn an_unmodeled_extern_invalidates_the_pointer_it_was_handed() {
    let mut caller = defined(
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
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int { bits: 32, val: 120 }),
                        Operand::Const(Const::Int { bits: 64, val: 8 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "mystery", vec![CTy::Ptr], CTy::Void),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let mut mem = r.states()[0].mem.clone();
    let base = match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let after = mem.read(chiero_mem::Pointer { base, off: 0 }, 4, Span::DUMMY);
    assert_ne!(
        after.value,
        Some(vec![120, 120, 120, 120]),
        "the callee's writes are not invisible"
    );
    // ...and it is a *known unknown*, not an uninitialized read: 024 §2.1's default fill
    // for an unmodeled extern is `Symbolic`, because the callee most likely wrote
    // something meaningful.
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("havoc") && x.detail.contains("symbolic")),
        "{:#?}",
        r.states()[0].assumptions()
    );
}

/// **024 contract 5's "exactly one".** One `free(&stack_var)` is one finding, however many
/// states the run ends with. A state carries the findings it saw, and a fork carries a
/// *copy* — so a single branch after the call doubled the report, and contracts 4, 9, 10
/// and 22 use the same wording. 023 §6.1 delegates the real dedup key to 040, but a flat
/// `Vec<String>` gave 040 nothing to dedup on either. Found by review.
#[test]
fn one_bad_free_is_one_finding_however_many_states_survive() {
    let mut caller = defined(
        0,
        "main",
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    }),
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![Operand::Value(ValueId(0))],
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Cmp {
                            op: CmpOp::Eq,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(1)),
                            b: i32c(7),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(1)))),
            block(2, vec![], Terminator::Return(Some(i32c(2)))),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![caller, extern_fn(1, "free", vec![CTy::Ptr], CTy::Void)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 2, "the branch really did fork");
    let bad: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("bad-free"))
        .collect();
    assert_eq!(bad.len(), 1, "{:#?}", r.findings());
}

/// **024 §7 and contract 21: `chiero_mark_fidelity` carries the harness's own words.**
/// The engine hardcoded "harness marked this region approximate" and discarded the
/// `const char *why` the call passed, so the one mechanism a harness author has for
/// saying *why* a region is approximate reported the same sentence for every use.
#[test]
fn mark_fidelity_reports_the_reason_the_harness_gave() {
    let mut caller = defined(
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
                // "hw\0" — three stores would be clearer in C, but the CIR spelling of a
                // string literal is bytes, and `memset` is the byte writer to hand.
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int { bits: 32, val: 104 }),
                        Operand::Const(Const::Int { bits: 64, val: 3 }),
                    ],
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 3 }),
                    },
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(1)),
                        Operand::Const(Const::Int { bits: 32, val: 0 }),
                        Operand::Const(Const::Int { bits: 64, val: 1 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "chiero_mark_fidelity", vec![CTy::Ptr], CTy::Void),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(r.fidelity(), Fidelity::Exact);
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("hhh")),
        "the harness's reason, not a fixed sentence: {:#?}",
        r.states()[0].assumptions()
    );
}

/// **024 §1 step 3.** `__builtin_memset` is `memset`. gcc emits the `__builtin_` spelling
/// for anything it recognises, so without the alias every VPP translation unit compiled
/// with optimization hits unmodeled externs for the functions chiero models best — and
/// each one now havocs its buffer, turning the most common calls in the codebase into
/// lost information.
#[test]
fn a_builtin_alias_reaches_the_model_of_the_same_name() {
    let mut caller = defined(
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
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int {
                            bits: 32,
                            val: 0xAB,
                        }),
                        Operand::Const(Const::Int { bits: 64, val: 6 }),
                    ],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(
                1,
                "__builtin_memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    let base = match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = r.states()[0].mem.clone();
    assert_eq!(
        mem.read(chiero_mem::Pointer { base, off: 0 }, 6, Span::DUMMY)
            .value,
        Some(vec![0xAB; 6]),
        "the model ran, it was not merely not-degraded"
    );
}

/// **024 contract 20.** `longjmp` yields exactly one "unsupported" diagnostic and a state
/// **terminated** at `Fidelity::Unknown` — never a silently-continued path. Today it is
/// merely `Approximate`, so execution walks on past a call that in reality never returns,
/// and everything after it is a path the program does not have.
///
/// `Approximated` is the wrong level as well as the wrong control flow: 023 §7 reserves
/// `Unknown` for "the engine does not know and cannot bound its ignorance", which is
/// exactly non-local control flow.
#[test]
fn longjmp_terminates_the_state_at_unknown() {
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
                // Never reached: `longjmp` does not return.
                inst(InstKind::Assign {
                    dst: ValueId(9),
                    rv: RValue::Use(i32c(1)),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, extern_fn(1, "longjmp", vec![], CTy::Void)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Unknown);
    assert!(
        r.states()[0].local(ValueId(9)).is_none(),
        "the instruction after a `longjmp` is not on any path"
    );
    let unsupported: Vec<_> = r.states()[0]
        .assumptions()
        .iter()
        .filter(|x| x.detail.contains("longjmp"))
        .collect();
    assert_eq!(unsupported.len(), 1, "{unsupported:#?}");
}

/// **The fresh value an extern returns has the *declared* return type's sort.** Every
/// unmodeled extern minted a `BitVec(64)` regardless — harmless while the models all
/// returned `size_t`, and a trap for the next one: an `int`-returning function whose
/// result is compared against a 32-bit value produces a width mismatch, and a
/// pointer-returning one produces a scalar where 023 §1.1 wants provenance.
#[test]
fn an_externs_fresh_value_has_the_declared_width() {
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
    let m = Module {
        funcs: vec![caller, extern_fn(1, "mystery", vec![], CTy::Int(16))],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    match r.states()[0].local(ValueId(0)) {
        Some(Value::Scalar(t)) => assert_eq!(a.width(t), 16),
        other => panic!("{other:?}"),
    }

    // And the same on the *dispatch* path, which mints its own `model{n}` when a model
    // ran and produced nothing. That one hardcoded `BitVec(64)` and overwrote the
    // correctly-sorted value the extern path had already set.
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![Operand::Const(Const::Null)],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, extern_fn(1, "free", vec![CTy::Ptr], CTy::Int(16))],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    match r.states()[0].local(ValueId(0)) {
        Some(Value::Scalar(t)) => assert_eq!(a.width(t), 16, "the dispatch fallback too"),
        other => panic!("{other:?}"),
    }
}

/// **024 contract 21c.** A havoc degrades identically whether it came from the default
/// unmodeled-extern fallback or from a registered model.
///
/// The two mechanisms are genuinely different now: `scanf` returns its own `HavocSpec`
/// naming only the pointers it writes through, while the fallback knows nothing and
/// invalidates every pointer argument. The **fidelity effect** must still match, which is
/// what the contract actually says — a model that knows more must not be punished for it.
///
/// An earlier version of this test compared the *bytes* as well and passed for the wrong
/// reason: both sides reached the same fallback, so they agreed by construction rather
/// than by two mechanisms coinciding.
#[test]
fn a_registered_models_havoc_degrades_like_the_default_one() {
    let build = |callee: &str| {
        let mut caller = defined(
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
                        dst: None,
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Const(Const::Int { bits: 32, val: 120 }),
                            Operand::Const(Const::Int { bits: 64, val: 8 }),
                        ],
                    }),
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![Operand::Value(ValueId(0))],
                    }),
                ],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        );
        caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
        Module {
            funcs: vec![
                caller,
                extern_fn(1, callee, vec![CTy::Ptr], CTy::Int(32)),
                extern_fn(
                    2,
                    "memset",
                    vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                    CTy::Ptr,
                ),
            ],
            ..Default::default()
        }
    };
    let run = |m: &Module| {
        let mut a = TermArena::new();
        let r = Engine::new(m).run(&mut a);
        let base = match r.states()[0].local(ValueId(0)) {
            Some(Value::Ptr(p)) => p.base,
            other => panic!("{other:?}"),
        };
        let mut mem = r.states()[0].mem.clone();
        let bytes = mem
            .read(chiero_mem::Pointer { base, off: 0 }, 4, Span::DUMMY)
            .value;
        (r.fidelity(), bytes)
    };
    let modeled = build("scanf");
    let unmodeled = build("some_function_chiero_has_never_heard_of");
    let (f_modeled, b_modeled) = run(&modeled);
    let (f_unmodeled, b_unmodeled) = run(&unmodeled);
    assert_eq!(
        f_modeled, f_unmodeled,
        "a havoc degrades the same wherever it came from"
    );
    // The *object sets* differ on purpose and the fidelity does not — which is exactly
    // what 21c says. `scanf`'s only pointer here is its format string, which it reads,
    // so the model invalidates nothing while the fallback invalidates the buffer. A
    // model that knows more must not be punished for it.
    assert_eq!(
        b_modeled,
        Some(vec![120u8; 4]),
        "the format string is read, not written"
    );
    assert_ne!(
        b_unmodeled,
        Some(vec![120u8; 4]),
        "the fallback, knowing nothing, invalidates it"
    );
}

/// **`AddrOfFunc` was a lowering gap, so an indirect call was never resolvable.** Taking a
/// function's address degraded the run to `Unknown` before the call even happened, and the
/// call itself then forked over *every* defined function plus an unresolvable state —
/// which is the safe answer to a question chiero already had the answer to.
///
/// 023 contract 10 keeps the unresolvable state for a pointer chiero cannot resolve. A
/// pointer it *can* resolve is a different case: one candidate, no unresolvable sibling,
/// and `Exact`. VPP's node dispatch goes through registration tables, so both cases are
/// real, and conflating them makes every table-driven call unanalysable.
#[test]
fn a_function_pointer_with_a_known_target_resolves_to_one_callee() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfFunc(FuncId(1)),
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
    let target = defined(
        1,
        "target",
        vec![block(0, vec![], Terminator::Return(Some(i32c(42))))],
        CTy::Int(32),
    );
    let decoy = defined(
        2,
        "decoy",
        vec![block(0, vec![], Terminator::Return(Some(i32c(7))))],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, target, decoy],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.states().len(),
        1,
        "the pointer is known; there is nothing to fork over"
    );
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(42));
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "a resolved call is not a guess: {:#?}",
        r.states()[0].assumptions()
    );
}

/// **Two `&f` are the same pointer.** C says so — function pointers to the same function
/// compare equal — and chiero needs it for a different reason: a fresh object per
/// `AddrOfFunc` would make `if (cb == handler)` false against itself, silently pruning the
/// branch a registration-table check takes.
#[test]
fn a_functions_address_is_the_same_pointer_every_time() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfFunc(FuncId(1)),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfFunc(FuncId(1)),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::AddrOfFunc(FuncId(2)),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let f1 = defined(
        1,
        "one",
        vec![block(0, vec![], Terminator::Return(Some(i32c(1))))],
        CTy::Int(32),
    );
    let f2 = defined(
        2,
        "two",
        vec![block(0, vec![], Terminator::Return(Some(i32c(2))))],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, f1, f2],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert_eq!(s.local(ValueId(0)), s.local(ValueId(1)));
    // And two *different* functions are different pointers, or the first assertion is
    // satisfied by handing out one object for everything.
    assert_ne!(s.local(ValueId(0)), s.local(ValueId(2)));
}

/// **A registered model's havoc is narrower than the fallback's, and that is the point.**
/// The default invalidates every pointer argument; `scanf(fmt, &x)` only writes through
/// `&x`, so throwing away `fmt` loses a string the callee merely read — and with it any
/// later finding about that string.
///
/// This is also what finally makes 024 contract 21c compare two mechanisms rather than one
/// fallback against itself: the fidelity effect must still be identical even though the
/// object sets differ.
#[test]
fn scanf_invalidates_what_it_writes_and_leaves_its_format_alone() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                // Both buffers hold known bytes before the call.
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int {
                            bits: 32,
                            val: 0xAB,
                        }),
                        Operand::Const(Const::Int { bits: 64, val: 4 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(1)),
                        Operand::Const(Const::Int {
                            bits: 32,
                            val: 0xCD,
                        }),
                        Operand::Const(Const::Int { bits: 64, val: 4 }),
                    ],
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(2)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4), alloca(1, CTy::Int(8), 4)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "scanf", vec![CTy::Ptr, CTy::Ptr], CTy::Int(32)),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    let obj = |v: u32| match s.local(ValueId(v)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = s.mem.clone();
    assert_eq!(
        mem.read(
            chiero_mem::Pointer {
                base: obj(0),
                off: 0
            },
            4,
            Span::DUMMY
        )
        .value,
        Some(vec![0xAB; 4]),
        "the format string is read, not written"
    );
    assert_ne!(
        mem.read(
            chiero_mem::Pointer {
                base: obj(1),
                off: 0
            },
            4,
            Span::DUMMY
        )
        .value,
        Some(vec![0xCD; 4]),
        "the output pointer is invalidated"
    );
    // 024 contract 21c: same fidelity effect as the default havoc, different object set.
    assert_eq!(r.fidelity(), Fidelity::Approximated);
}

/// **D2: a call the engine could not perform still invalidates.** `can_dispatch` is a
/// *name*-level check, so a call that passes it and then fails per-call translation
/// recorded an assumption and havoc'd nothing. `char buf[8]; memset(buf,'x',8);
/// memcpy(buf, src, n);` with a non-constant `n` left chiero believing `buf` was eight
/// `'x'` bytes — and the sharpest case is `strcpy`'s overflow arm, which reports the
/// overflow *and* keeps believing the destination is intact. Found by review.
#[test]
fn a_dispatchable_call_that_could_not_be_performed_still_invalidates() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int { bits: 32, val: 120 }),
                        Operand::Const(Const::Int { bits: 64, val: 8 }),
                    ],
                }),
                // A symbolic size: `memcpy` is dispatchable by name and untranslatable
                // for this call.
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Fresh { ty: CTy::Int(64) },
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Value(ValueId(1)),
                        Operand::Value(ValueId(2)),
                    ],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 8), alloca(1, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(
                1,
                "memcpy",
                vec![CTy::Ptr, CTy::Ptr, CTy::Int(64)],
                CTy::Ptr,
            ),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let base = match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = r.states()[0].mem.clone();
    assert_ne!(
        mem.read(chiero_mem::Pointer { base, off: 0 }, 4, Span::DUMMY)
            .value,
        Some(vec![120u8; 4]),
        "the call was not performed, so what it might have written is unknown"
    );
}

/// **D7 / M30: a `PtrAdd` offset is not always 64 bits.** My own note called
/// `c.bits()` vs `c.signed()` a no-op mutation because at 64 bits they agree — but nothing
/// makes the offset 64-bit; 020 §8 rule 6 constrains only the *base*. At 32 bits, `-4`
/// read through `bits()` is `4294967292`. The relevant C is `p + (int)-offsetof(S,m)`,
/// which is `container_of` — 020 contract 28 names it.
///
/// And an offset wider than a pointer is a **gap**, not a truncation: `2^64 + 4` became
/// `+4`, turning a wildly out-of-bounds walk into an in-bounds one with no assumption and
/// no degradation, so `seal` would mint a proof over it. Found by review.
#[test]
fn ptr_add_reads_narrow_offsets_as_signed_and_refuses_wide_ones() {
    let build = |bits: u32, val: i128| {
        let mut caller = defined(
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
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::PtrAdd {
                            base: Operand::Value(ValueId(0)),
                            off: Operand::Const(Const::Int { bits, val }),
                        },
                    }),
                ],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        );
        caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
        Module {
            funcs: vec![caller],
            ..Default::default()
        }
    };
    // A 32-bit `-4` steps back four bytes, not four billion forward.
    let m = build(32, -4);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    match r.states()[0].local(ValueId(1)) {
        Some(Value::Ptr(p)) => assert_eq!(p.off, -4),
        other => panic!("{other:?}"),
    }

    // A 128-bit offset is more than a pointer can hold; truncating it is a fabricated
    // address, so it is a gap.
    let m = build(128, (1i128 << 64) + 4);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
}

/// **The havoc's shape, at all three call sites.** Mutation showed only one of the three
/// the engine advertises was tested, and that the fill kind, the depth and "every pointer
/// argument, not just the first" were all unpinned. Each is a different way for the
/// invalidation to be quietly narrower than it claims.
#[test]
fn the_default_havoc_covers_every_pointer_argument_at_depth_one() {
    // `sqrt` is registered `Approximate` and has no engine arm; `nothing_known` is
    // unregistered. Both must invalidate, and the exact-but-undispatchable arm is
    // covered by `an_exact_model_with_no_engine_arm_degrades_on_valid_arguments`.
    for callee in ["sqrt", "nothing_known"] {
        let mut caller = defined(
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
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(1),
                        },
                    }),
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Const(Const::Int {
                                bits: 32,
                                val: 0xAB,
                            }),
                            Operand::Const(Const::Int { bits: 64, val: 8 }),
                        ],
                    }),
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![
                            Operand::Value(ValueId(1)),
                            Operand::Const(Const::Int {
                                bits: 32,
                                val: 0xCD,
                            }),
                            Operand::Const(Const::Int { bits: 64, val: 8 }),
                        ],
                    }),
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                    }),
                ],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        );
        caller.allocas = vec![alloca(0, CTy::Int(8), 8), alloca(1, CTy::Int(8), 8)];
        let m = Module {
            funcs: vec![
                caller,
                extern_fn(1, callee, vec![CTy::Ptr, CTy::Ptr], CTy::Void),
                extern_fn(
                    2,
                    "memset",
                    vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                    CTy::Ptr,
                ),
            ],
            ..Default::default()
        };
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        let s = &r.states()[0];
        let mut mem = s.mem.clone();
        for (v, byte) in [(0u32, 0xABu8), (1, 0xCD)] {
            let base = match s.local(ValueId(v)) {
                Some(Value::Ptr(p)) => p.base,
                other => panic!("{other:?}"),
            };
            let read = mem.read(chiero_mem::Pointer { base, off: 0 }, 4, Span::DUMMY);
            // **`Symbolic`, not `Uninitialized`** (024 §2.1's default): a known-unknown,
            // so a later read must not be an uninitialized-read finding. Swapping the two
            // fills is otherwise invisible — both stop the old bytes coming back.
            assert!(
                !read.faults.iter().any(|f| f.kind() == "uninitialized-read"),
                "`{callee}` arg {v}: symbolic is not uninitialized: {:#?}",
                read.faults
            );
            assert_ne!(
                read.value,
                Some(vec![byte; 4]),
                "`{callee}` invalidated argument {v}"
            );
        }
    }
}

/// **A symbolic `PtrAdd` offset is a gap, not a guess.** Concretizing to any particular
/// value — 0 is the tempting one — produces an address the program never computes, and
/// every access through it is then a confident report about the wrong bytes.
#[test]
fn a_symbolic_ptr_add_offset_is_a_gap() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Fresh { ty: CTy::Int(64) },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Value(ValueId(1)),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Unknown);
    assert!(
        r.states()[0].local(ValueId(2)).is_none(),
        "no fabricated pointer was handed out"
    );
}

/// **`Load` and `Store` were not implemented at all.** Every fixture in this suite writes
/// memory through `memset`, because there was no other way — which is exactly why nobody
/// noticed: the workaround was uniform enough to look like a style. A C program that
/// assigns through a pointer and reads it back is the most ordinary thing there is, and
/// it was a `LoweringGap` to `Unknown`.
#[test]
fn a_store_is_visible_to_a_later_load() {
    let mut caller = defined(
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
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Int {
                        bits: 32,
                        val: 0x01020304,
                    }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(0x01020304));
    // **And the bytes are concrete afterwards.** Routing a ground value through the
    // symbolic overlay reads back the same through `read_term`, so the term-level
    // assertion above cannot see it — but a *concrete* read then refuses the bytes as
    // symbolic, and every string model works on concrete reads.
    let base = match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = r.states()[0].mem.clone();
    let bytes = mem.read(chiero_mem::Pointer { base, off: 0 }, 4, Span::DUMMY);
    assert!(bytes.faults.is_empty(), "{:#?}", bytes.faults);
    assert_eq!(bytes.value, Some(vec![4, 3, 2, 1]), "little-endian");
}

/// **A load of memory nobody wrote is a finding, not a zero.** 021 §3.1 names reading
/// uninitialized bytes as zero the single most common way a symbolic executor produces
/// confidently wrong results, and a freshly-implemented `Load` is exactly where that
/// mistake gets made — the backing store really does read as zero.
#[test]
fn a_load_of_unwritten_memory_reports_rather_than_reading_zero() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("uninitialized-read")),
        "{:#?}",
        r.findings()
    );
    assert_ne!(
        r.states()[0].return_value_bits(&mut a),
        Some(0),
        "a fresh symbol, not the backing store's zero"
    );
}

/// **A store carries a symbolic value through memory.** Writing the concrete bytes behind
/// a symbol — or refusing the store — would make `x = f(); *p = x; y = *p;` lose the
/// relationship between `y` and `f()`, and every constraint derived from it.
#[test]
fn a_symbolic_store_reads_back_as_the_same_unknown() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
                // `x - y` is zero exactly when the store round-tripped the *same* symbol.
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Bin {
                        op: BinOp::Sub,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(2)),
                        b: Operand::Value(ValueId(1)),
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(3)))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    // The arena is hash-consed, so the *same* unknown is the same `Term`. `x - x` would
    // have been the tidier assertion if the arena folded it, and it does not — asserting
    // on a fold that does not happen would have tested the arena, not the store.
    let s = &r.states()[0];
    assert_eq!(
        s.local(ValueId(2)),
        s.local(ValueId(1)),
        "the value that came back is the value that went in"
    );
    assert!(
        !matches!(s.local(ValueId(2)), Some(Value::Scalar(t)) if a.eval_ground(t).is_ok()),
        "and it is still an unknown, not the model's bytes"
    );
}

/// **`CopyMem` and `SetMem` are CIR instructions, not just library calls.** A frontend
/// lowers a struct assignment and an array initializer to these directly, with no
/// `memcpy` in the source at all — so leaving them unimplemented means `s = t;` between
/// structs degrades the run to `Unknown` and silently keeps the destination's old bytes.
#[test]
fn copy_mem_and_set_mem_move_bytes() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 0xAB }),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                }),
                inst(InstKind::CopyMem {
                    dst: Operand::Value(ValueId(1)),
                    src: Operand::Value(ValueId(0)),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                    align: 1,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    // Eight-aligned so the *verification* read is not itself misaligned — the fill and
    // the copy are byte-wise and impose no alignment, but reading eight bytes back does.
    caller.allocas = vec![
        AllocaDecl {
            align: 8,
            ..alloca(0, CTy::Int(8), 8)
        },
        AllocaDecl {
            align: 8,
            ..alloca(1, CTy::Int(8), 8)
        },
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    let s = &r.states()[0];
    let mut mem = s.mem.clone();
    for v in [0u32, 1] {
        let base = match s.local(ValueId(v)) {
            Some(Value::Ptr(p)) => p.base,
            other => panic!("{other:?}"),
        };
        let read = mem.read(chiero_mem::Pointer { base, off: 0 }, 8, Span::DUMMY);
        assert!(read.faults.is_empty(), "{:#?}", read.faults);
        assert_eq!(read.value, Some(vec![0xAB; 8]), "buffer {v}");
    }
}

/// **`CopyMem` is `memcpy`, so overlapping ranges are a finding.** 021 contract 22 draws
/// the line between `Overlap::Forbidden` and `Overlap::Allowed`, and a lowering that
/// picked the permissive one would silently accept UB that the library model rejects —
/// the same call, two answers, depending on which spelling the frontend chose.
#[test]
fn copy_mem_forbids_overlap() {
    let mut caller = defined(
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
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 1 }),
                    size: Operand::Const(Const::Int { bits: 64, val: 16 }),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 2 }),
                    },
                }),
                inst(InstKind::CopyMem {
                    dst: Operand::Value(ValueId(1)),
                    src: Operand::Value(ValueId(0)),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                    align: 1,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("overlapping-copy")),
        "{:#?}",
        r.findings()
    );
}

/// **An audit of `exec_inst`/`eval` rather than one discovery at a time.** `Load`/`Store`
/// stayed hidden behind a uniform `memset` workaround, so this enumerates the CIR and
/// checks what the engine can actually perform. `Un`, `Cast` and `Select` were all
/// missing: a unary minus, an integer cast and a ternary are in almost every C function
/// written, and each was a `LoweringGap` to `Unknown`.
///
/// One program, so the assertion is on the *values*: a run that degrades anywhere is
/// caught by the fidelity check, and a wrong value by the arithmetic.
#[test]
fn the_scalar_operations_are_all_performed() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                // -5 as i32
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Un {
                        op: UnOp::Neg,
                        a: i32c(5),
                        ty: CTy::Int(32),
                    },
                }),
                // Sign-extended to 64: still -5, which zero-extension gets wrong.
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Cast {
                        kind: CastKind::SExt,
                        a: Operand::Value(ValueId(0)),
                        from: CTy::Int(32),
                        to: CTy::Int(64),
                    },
                }),
                // ~0u32 truncated to 8 bits is 0xFF.
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Un {
                        op: UnOp::Not,
                        a: Operand::Const(Const::Int { bits: 32, val: 0 }),
                        ty: CTy::Int(32),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Cast {
                        kind: CastKind::Trunc,
                        a: Operand::Value(ValueId(2)),
                        from: CTy::Int(32),
                        to: CTy::Int(8),
                    },
                }),
                // Zero-extending that 0xFF back to 32 is 255, not -1.
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Cast {
                        kind: CastKind::ZExt,
                        a: Operand::Value(ValueId(3)),
                        from: CTy::Int(8),
                        to: CTy::Int(32),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(5),
                    rv: RValue::Cmp {
                        op: CmpOp::SLt,
                        ty: CTy::Int(64),
                        a: Operand::Value(ValueId(1)),
                        b: Operand::Const(Const::Int { bits: 64, val: 0 }),
                    },
                }),
                // The condition is true, so this is 255 — and the *false* arm is a value
                // the true arm could not be confused with.
                inst(InstKind::Assign {
                    dst: ValueId(6),
                    rv: RValue::Select {
                        cond: Operand::Value(ValueId(5)),
                        t: Operand::Value(ValueId(4)),
                        f: i32c(9),
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(6)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    let s = &r.states()[0];
    let bits = |v: u32, a: &mut TermArena| match s.local(ValueId(v)) {
        Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
        other => panic!("{other:?}"),
    };
    assert_eq!(bits(0, &mut a), Some(0xFFFF_FFFB), "-5 in 32 bits");
    assert_eq!(
        bits(1, &mut a),
        Some(0xFFFF_FFFF_FFFF_FFFB),
        "sign-extended, not zero-extended"
    );
    assert_eq!(bits(3, &mut a), Some(0xFF));
    assert_eq!(
        bits(4, &mut a),
        Some(255),
        "zero-extended, not sign-extended"
    );
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(255));
}

/// **A global has an address.** `AddrOfGlobal` was a lowering gap, so any function
/// touching a file-scope variable degraded to `Unknown` before doing anything — and VPP
/// is full of them. Two globals must be two objects, and a `const` one must refuse writes
/// (021 §4), which is also what keeps a havoc from destroying a string literal.
#[test]
fn a_global_has_its_own_object_and_const_means_readonly() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfGlobal { g: GlobalId(0) },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfGlobal { g: GlobalId(0) },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::AddrOfGlobal { g: GlobalId(1) },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(2)),
                    val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        globals: vec![
            Global {
                id: GlobalId(0),
                name: "counter".into(),
                size: 4,
                align: 4,
                is_const: false,
                span: Span::DUMMY,
            },
            Global {
                id: GlobalId(1),
                name: "message".into(),
                size: 8,
                align: 4,
                is_const: true,
                span: Span::DUMMY,
            },
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert_eq!(
        s.local(ValueId(0)),
        s.local(ValueId(1)),
        "one object per global"
    );
    assert_ne!(
        s.local(ValueId(0)),
        s.local(ValueId(2)),
        "and two globals are two objects"
    );
    assert!(
        r.findings().iter().any(|f| f.contains("write-to-readonly")),
        "a `const` global refuses the write: {:#?}",
        r.findings()
    );
}

/// **`LoadBits`/`StoreBits` are why 021 §3.1's init mask is bit-granular at all.** A
/// per-byte mask can only answer "yes" for a whole bitfield word — missing every real
/// uninitialized-bitfield read — or "no", firing on every correct one. The mask has been
/// bit-granular from the start and the engine had no instruction that could reach it.
///
/// `session_types.h` packs nine bitfields into one `u32`, several of them unnamed padding
/// nobody writes, so this is the shape that decides whether VPP is analysable.
#[test]
fn bitfields_are_read_and_written_at_bit_granularity() {
    let mut caller = defined(
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
                // Write only bits 4..7 of the word. Everything else stays untouched —
                // which is the point: a byte-granular implementation would initialize
                // the whole byte and lose the finding below.
                inst(InstKind::StoreBits {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Int {
                        bits: 32,
                        val: 0b1011,
                    }),
                    unit: CTy::Int(32),
                    bits: BitRange { off: 4, width: 4 },
                    align: 4,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::LoadBits {
                        addr: Operand::Value(ValueId(0)),
                        unit: CTy::Int(32),
                        bits: BitRange { off: 4, width: 4 },
                        signed: false,
                        align: 4,
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(0b1011));
}

/// **The neighbouring bits are still uninitialized**, and reading them is a finding. This
/// is the half a byte-granular mask cannot express: bits 4..7 were written and bits 0..3
/// of the same byte were not, so an implementation that rounded to bytes reports nothing
/// here and a real uninitialized-bitfield read goes missing.
#[test]
fn a_neighbouring_bitfield_is_still_uninitialized() {
    let mut caller = defined(
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
                inst(InstKind::StoreBits {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Int {
                        bits: 32,
                        val: 0b1011,
                    }),
                    unit: CTy::Int(32),
                    bits: BitRange { off: 4, width: 4 },
                    align: 4,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::LoadBits {
                        addr: Operand::Value(ValueId(0)),
                        unit: CTy::Int(32),
                        bits: BitRange { off: 0, width: 4 },
                        signed: false,
                        align: 4,
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("uninitialized-read")),
        "{:#?}",
        r.findings()
    );
    assert_ne!(
        r.states()[0].return_value_bits(&mut a),
        Some(0),
        "a fresh symbol, not the backing store's zero"
    );
}

/// **021 §7.1: `IntToPtr` is provenance-first.** The range search is the *fallback* and it
/// is wrong in both directions — an integer that happens to land in an object is followed
/// to it, and a pointer whose object has been freed resolves to whatever now occupies the
/// address. So a round trip through `uintptr_t` must come back as the **same pointer**,
/// recovered from where it came from rather than from what its bits look like.
///
/// The fixture puts a second object at a known address so "recovered the origin" and
/// "found something at that address" are different answers.
#[test]
fn a_pointer_round_trips_through_an_integer_by_provenance() {
    let mut caller = defined(
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
                // Four bytes in: an offset the range search could reach from either
                // neighbour if it were guessing.
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 4 }),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Cast {
                        kind: CastKind::PtrToInt,
                        a: Operand::Value(ValueId(1)),
                        from: CTy::Ptr,
                        to: CTy::Int(64),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(2)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                }),
                // The *integer* has to be right too, not just recoverable. Provenance
                // gives the pointer back whatever the address says, so dropping the
                // offset from the address computation is invisible without this — and a
                // program that prints or compares the integer would see a lie.
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Cast {
                        kind: CastKind::PtrToInt,
                        a: Operand::Value(ValueId(0)),
                        from: CTy::Ptr,
                        to: CTy::Int(64),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(5),
                    rv: RValue::Bin {
                        op: BinOp::Sub,
                        ty: CTy::Int(64),
                        a: Operand::Value(ValueId(2)),
                        b: Operand::Value(ValueId(4)),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 16), alloca(1, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert_eq!(
        s.local(ValueId(3)),
        s.local(ValueId(1)),
        "the same pointer, object and offset"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "provenance is not a guess: {:#?}",
        s.assumptions()
    );
    match s.local(ValueId(5)) {
        Some(Value::Scalar(t)) => {
            assert_eq!(a.eval_ground(t).ok().map(|c| c.bits()), Some(4))
        }
        other => panic!("{other:?}"),
    }
}

/// **An integer with no provenance is not silently resolved.** 021 §7.1 keeps the range
/// search as a fallback, and a fallback that answers `Exact` is indistinguishable from
/// knowing. `(char *)0x1000` is a real thing to write — memory-mapped registers — and
/// chiero must say it is guessing.
#[test]
fn an_integer_with_no_provenance_degrades() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Cast {
                    kind: CastKind::IntToPtr,
                    a: Operand::Const(Const::Int {
                        bits: 64,
                        val: 0x1000,
                    }),
                    from: CTy::Int(64),
                    to: CTy::Ptr,
                },
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(r.fidelity(), Fidelity::Exact);
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("provenance")),
        "{:#?}",
        r.states()[0].assumptions()
    );
}

/// **023 contract 23: a `Value::Ptr` survives a store and a load.** `Store` took its value
/// through `scalar`, which returns `None` for a pointer, so `p->next = q` hit the
/// `let-else` and degraded **every run containing it** to `Unknown` — while blaming the
/// *address*, which was fine, and manufacturing a false uninitialized-read on the reload
/// because the store never happened. Found by review.
///
/// Essentially all of VPP's data structures are this shape.
#[test]
fn a_pointer_survives_a_store_and_a_load() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Value(ValueId(1)),
                    ty: CTy::Ptr,
                    align: 8,
                    vol: Volatility::Normal,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Ptr,
                        align: 8,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![
        AllocaDecl {
            align: 8,
            ..alloca(0, CTy::Ptr, 1)
        },
        AllocaDecl {
            align: 8,
            ..alloca(1, CTy::Int(8), 16)
        },
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert_eq!(
        s.local(ValueId(2)),
        s.local(ValueId(1)),
        "the same pointer, object and offset, came back"
    );
    assert_eq!(r.fidelity(), Fidelity::Exact, "{:#?}", s.assumptions());
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
}

/// **Misalignment is a finding only in `ub-strict` mode** (021 §5 step 3), and no such
/// mode exists — so `report_faults` turning *every* fault into a finding reported it
/// unconditionally. `align_fault` compares against the **object's declared** alignment, so
/// any N-byte access into an object declared with less is flagged whatever its address.
/// Anything using `CLIB_PACKED` — every VPP packet header — is a false positive. Found by
/// review, which noted the commit's own pinning test manufactures two and cannot see them.
#[test]
fn misalignment_is_not_a_finding_by_default() {
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Store {
                addr: Operand::Value(ValueId(0)),
                val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                ty: CTy::Int(32),
                align: 1,
                vol: Volatility::Normal,
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.blocks[0].insts.insert(
        0,
        inst(InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        }),
    );
    // Declared align 1 — a packed struct — and a four-byte store into it.
    caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        !r.findings().iter().any(|f| f.contains("misaligned")),
        "x86-64 tolerates this and VPP relies on it: {:#?}",
        r.findings()
    );
}

/// **A wide load must not kill the process.** `<4 x i64>` is thirty-two bytes — an AVX
/// vector VPP uses — and the term API had no width limit, so the arena asserted. Reaching
/// it needed nothing exotic: a load out of `memset`-initialized memory. An abort is not
/// something a caller can contain, which is why 021 keeps `BadRange` distinct from
/// `OutOfBounds`: the object is big enough and chiero still cannot answer.
#[test]
fn a_load_wider_than_chiero_can_carry_faults_rather_than_aborting() {
    let mut caller = defined(
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
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 7 }),
                    size: Operand::Const(Const::Int { bits: 64, val: 32 }),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Vector {
                            elem: Box::new(CTy::Int(64)),
                            lanes: 4,
                        },
                        align: 32,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 32,
        ..alloca(0, CTy::Int(8), 32)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("unsupported-access-width")),
        "{:#?}",
        r.findings()
    );
    assert_ne!(r.fidelity(), Fidelity::Exact);
}

/// **`scanf` must skip an argument *position*, not a surviving pointer.** The model takes
/// the pointers `dispatch` resolved, which is an already-*filtered* list — so an argument
/// that did not resolve to a `Value::Ptr` shifts everything left and `.skip(1)` eats the
/// first real output buffer. Found by review, and not hypothetical: a format string is
/// usually a global, and `scanf("%d", &x)` then has exactly this shape.
#[test]
fn scanf_skips_a_position_not_a_resolved_pointer() {
    let mut caller = defined(
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
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int {
                            bits: 32,
                            val: 0xCD,
                        }),
                        Operand::Const(Const::Int { bits: 64, val: 4 }),
                    ],
                }),
                // The format argument is a bare integer, which resolves to no pointer at
                // all — so the output buffer is the only survivor of the filter.
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Const(Const::Int { bits: 64, val: 0 }),
                        Operand::Value(ValueId(0)),
                    ],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(8), 4)
    }];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "scanf", vec![CTy::Ptr, CTy::Ptr], CTy::Int(32)),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    let base = match s.local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = s.mem.clone();
    assert_ne!(
        mem.read(chiero_mem::Pointer { base, off: 0 }, 4, Span::DUMMY)
            .value,
        Some(vec![0xCD; 4]),
        "the output buffer is still invalidated"
    );
}

/// **020 §8: verification runs before execution, always, including on hand-written
/// fixtures.** `Engine::run` never called it, so a module violating rule 5 — a `Store`
/// whose value is narrower than its `ty` — reached `a.extract` and **panicked**. This is
/// the wave-5 "malformed input panics instead of erroring" class, reopened on a new
/// surface. Found by review.
///
/// A rejected module produces a state that says why; a panic is not something a caller
/// can contain, and chiero is meant to be called as a tool.
#[test]
fn a_module_that_fails_verification_is_not_executed() {
    let caller = defined(
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
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                    ty: CTy::Int(64),
                    align: 8,
                    vol: Volatility::Normal,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let mut caller = caller;
    caller.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(64), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.states()
            .iter()
            .any(|s| matches!(s.status, Status::Errored(_))),
        "the module is rejected, not run: {:#?}",
        r.states().iter().map(|s| &s.status).collect::<Vec<_>>()
    );
    assert_ne!(r.fidelity(), Fidelity::Exact);
}

/// **023 §7 rule 4's forbidden case, pinned in both directions.** Wave 17's headline claim
/// — "a finding is not automatically a degradation" — rested entirely on
/// `yields_unknown_value`, and the review showed it could return `false` for everything or
/// `true` for everything with all 510 tests passing. `false` for everything is the bad one:
/// an uninitialized read then seals a **proof**.
#[test]
fn an_invented_value_forbids_a_proof_and_a_definite_fault_does_not() {
    let build = |init: bool| {
        let mut insts = vec![inst(InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        })];
        if init {
            // A definite fault with nothing invented: a free of stack memory. The run
            // reports it and stays exact, because chiero modeled it faithfully.
            insts.push(inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![Operand::Value(ValueId(0))],
            }));
        } else {
            // An invented value: nobody wrote these bytes.
            insts.push(inst(InstKind::Assign {
                dst: ValueId(1),
                rv: RValue::Load {
                    addr: Operand::Value(ValueId(0)),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                },
            }));
        }
        let mut caller = defined(
            0,
            "main",
            vec![block(0, insts, Terminator::Return(Some(i32c(0))))],
            CTy::Int(32),
        );
        caller.allocas = vec![AllocaDecl {
            align: 4,
            ..alloca(0, CTy::Int(32), 1)
        }];
        Module {
            funcs: vec![caller, extern_fn(1, "free", vec![CTy::Ptr], CTy::Void)],
            ..Default::default()
        }
    };

    let m = build(false);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Unknown,
        "an invented value is not a proof"
    );
    assert!(seal(&r, r.witness()).is_err());

    let m = build(true);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("bad-free")),
        "{:#?}",
        r.findings()
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "a definite fact chiero modeled exactly: {:#?}",
        r.states()[0].assumptions()
    );
}

/// **One bug in a loop is one finding.** `report_faults` mints a fresh sequence id per
/// fault and `RunResult::findings` dedups only on that id, so a faulting access inside a
/// loop produced one byte-identical copy per iteration — nine here, and 256 for a loop
/// over `VLIB_FRAME_SIZE` buffers. 023 §6.1's key is `(checker, span, object, kind)`, and
/// `MemFault::kind()`/`at()`/`object()` were added in an earlier wave precisely to supply
/// it; nothing used them. Found by review.
#[test]
fn one_faulting_access_in_a_loop_is_one_finding() {
    let mut caller = defined(
        0,
        "main",
        vec![
            block(0, vec![], Terminator::Goto(BlockId(1))),
            block(
                1,
                vec![inst(InstKind::Store {
                    addr: Operand::Const(Const::Null),
                    val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                })],
                Terminator::Goto(BlockId(1)),
            ),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let nulls: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("null-dereference"))
        .collect();
    assert_eq!(nulls.len(), 1, "{:#?}", r.findings());
}

/// **The object is part of the key, or deduplication loses real bugs.** Two uninitialized
/// reads of the *same kind* at the *same span* are two findings when they are about two
/// different objects — and a hand-written fixture has `Span::DUMMY` everywhere, so without
/// the object component the key collapses them and one bug disappears. Merging is the
/// dangerous direction: a duplicate is noise, a dropped finding is a missed bug.
#[test]
fn two_objects_faulting_alike_are_two_findings() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(1)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![
        AllocaDecl {
            align: 4,
            ..alloca(0, CTy::Int(32), 1)
        },
        AllocaDecl {
            align: 4,
            ..alloca(1, CTy::Int(32), 1)
        },
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let reads: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("uninitialized-read"))
        .collect();
    assert_eq!(reads.len(), 2, "two buffers, two bugs: {:#?}", r.findings());
}

/// **A zero-sized load is a gap, not a 64-bit symbol.** `size_of_cty(CTy::Void)` is 0, so
/// `read_term` returns no value and no fault, and the `None` arm minted a fresh variable
/// through `sort_of` — which falls through to `BitVec(64)` for anything that is not `Int`
/// or `Ptr`. chiero invented a 64-bit value for a load of nothing. The same fallthrough
/// gives a *faulting* `f32` load 64 bits where the succeeding path gives 32. Found by
/// review.
#[test]
fn a_zero_sized_load_is_not_fabricated() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Void,
                        align: 1,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.states()[0].local(ValueId(1)).is_none(),
        "nothing was invented: {:?}",
        r.states()[0].local(ValueId(1))
    );
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.kind == AssumptionKind::NoInformation),
        "{:#?}",
        r.states()[0].assumptions()
    );
}

/// **Execution does not continue past a definite crash.** `int x; *(int *)0 = 1; return x;`
/// reported the null dereference *and* an uninitialized read of `x` — but `return x` is
/// unreachable, so the second finding is about a path the program does not have. Found by
/// review, which noted it costs a false positive and not merely fidelity.
///
/// The findings *before* the fault are real and stay; what ends is the path. chiero cannot
/// model what happens after undefined behaviour, and inventing a continuation is how a
/// tool produces confident reports about code that never runs.
#[test]
fn nothing_is_reported_after_a_definite_crash() {
    let mut caller = defined(
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
                inst(InstKind::Store {
                    addr: Operand::Const(Const::Null),
                    val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
                // Unreachable: the program crashed above.
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("null-dereference")),
        "the real bug is reported: {:#?}",
        r.findings()
    );
    assert!(
        !r.findings()
            .iter()
            .any(|f| f.contains("uninitialized-read")),
        "and nothing after it is: {:#?}",
        r.findings()
    );
    assert!(
        r.states()[0].local(ValueId(1)).is_none(),
        "the load never ran"
    );
}

/// **A definite out-of-bounds access ends the path too**, and this is the case where
/// continuing is quietest: the write is simply dropped, memory keeps its old contents, and
/// everything downstream reasons about a program that did not perform it. Nothing crashes,
/// so nothing looks wrong.
///
/// The *maybe* out-of-bounds fault is deliberately not fatal — it is a possibility, not a
/// fact, and ending the path on one would drop the analysis of code that runs fine.
#[test]
fn a_definite_out_of_bounds_store_ends_the_path() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 64 }),
                    },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(1)),
                    val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Use(i32c(7)),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("out-of-bounds")),
        "{:#?}",
        r.findings()
    );
    assert!(
        r.states()[0].local(ValueId(2)).is_none(),
        "the instruction after the write never ran"
    );
}

/// **A faulting load has the same width as a succeeding one.** `sort_of` falls through to
/// `BitVec(64)` for anything that is not `Int` or `Ptr`, so a `float` load that faulted
/// produced a 64-bit symbol where the same load succeeding produces 32 bits — a width that
/// depends on whether the memory happened to be initialized. Every downstream comparison
/// against it is then a sort error or a silent truncation. Found by review as the narrow
/// remainder of the zero-sized-load defect.
#[test]
fn a_faulting_load_has_the_width_its_type_declares() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Float(FloatKind::F32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Float(FloatKind::F32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    match r.states()[0].local(ValueId(1)) {
        Some(Value::Scalar(t)) => assert_eq!(
            a.width(t),
            32,
            "an f32 is 32 bits whether the load answered or not"
        ),
        other => panic!("{other:?}"),
    }
}

/// **An errored state is not an exact state.** None of the `Status::Errored` sites calls
/// `degrade`, so `State::fidelity()` answers `Exact` for a state that hit a construct the
/// engine cannot execute. `RunResult::fidelity` covers for it by special-casing the status
/// — one untested line standing between "the engine gave up" and "chiero proved this has
/// no bugs". The review's mutation removed that line and `seal` returned PROVEN for a C
/// function containing a `switch`.
#[test]
fn an_errored_state_is_not_exact_on_its_own() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![],
            Terminator::Switch {
                scrut: i32c(1),
                ty: CTy::Int(32),
                cases: vec![(1, BlockId(9))],
                default: BlockId(9),
            },
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert!(matches!(s.status, Status::Errored(_)), "{:?}", s.status);
    assert_ne!(
        s.fidelity(),
        Fidelity::Exact,
        "the *state* says so, not only the run"
    );
    assert!(
        s.assumptions().iter().any(|x| x.kind.matches(s.fidelity())),
        "and names a cause of the right kind: {:#?}",
        s.assumptions()
    );
    assert!(seal(&r, r.witness()).is_err());
}

/// **A chiero limit must not mask a memory-safety bug.** `too_wide` ran before the state
/// and null checks, so a 32-byte load through a freed or null pointer reported
/// "unsupported-access-width" and *not* the use-after-free. 021 §5 orders state before
/// contents, and this project already recorded the same lesson once — "bounds must precede
/// alignment, or a must-OOB access also reports the alignment of an access that never
/// happens". vppinfra uses `u8x32`/`u8x64` throughout. Found by review, whose mutation
/// swapping the two blocks survived the whole suite *and fixed the behaviour*.
#[test]
fn a_width_limit_does_not_mask_a_use_after_free() {
    let mut a = chiero_solver::TermArena::new();
    let mut m = chiero_mem::Memory::new();
    let o = m.alloc(chiero_mem::ObjKind::Heap, 64, 32, Span::DUMMY);
    m.free(o, Span::DUMMY);
    let p = chiero_mem::Pointer { base: o, off: 0 };
    let r = m.read_term(&mut a, p, 32, chiero_mem::Endian::Little, Span::DUMMY);
    assert!(
        r.faults.iter().any(|f| f.kind() == "use-after-free"),
        "the bug, not chiero's limit: {:#?}",
        r.faults
    );
}

/// **Provenance must not be launderable.** `ptr_ints` was keyed on the `Term`, and an
/// address is a ground `bv(64, …)` constant that the arena hash-conses — so **term
/// identity is value identity**, and *any* integer expression that happens to evaluate to
/// a recorded address recovered that object. The `Unknown` degrade on the range-search
/// fallback was bypassed exactly when it should fire, and the run sealed as **PROVEN**.
///
/// Both doc comments on the table claimed the opposite: "keyed on the term, so arithmetic
/// that produces a *different* term correctly loses the provenance". Hash-consing
/// guarantees they are the same term. Found by review.
///
/// The C is `(uintptr_t)&a + ((uintptr_t)&b - (uintptr_t)&a)` — an integer that reaches
/// `b`'s address with no provenance chain from `b`.
#[test]
fn provenance_cannot_be_laundered_through_arithmetic() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Cast {
                        kind: CastKind::PtrToInt,
                        a: Operand::Value(ValueId(0)),
                        from: CTy::Ptr,
                        to: CTy::Int(64),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Cast {
                        kind: CastKind::PtrToInt,
                        a: Operand::Value(ValueId(1)),
                        from: CTy::Ptr,
                        to: CTy::Int(64),
                    },
                }),
                // delta = addr(b) - addr(a)
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Bin {
                        op: BinOp::Sub,
                        ty: CTy::Int(64),
                        a: Operand::Value(ValueId(3)),
                        b: Operand::Value(ValueId(2)),
                    },
                }),
                // laundered = addr(a) + delta, which *evaluates* to addr(b) while the
                // only pointer it was ever derived from is `a`.
                inst(InstKind::Assign {
                    dst: ValueId(5),
                    rv: RValue::Bin {
                        op: BinOp::Add,
                        ty: CTy::Int(64),
                        a: Operand::Value(ValueId(2)),
                        b: Operand::Value(ValueId(4)),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(6),
                    rv: RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(5)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![
        AllocaDecl {
            align: 8,
            ..alloca(0, CTy::Int(64), 1)
        },
        AllocaDecl {
            align: 8,
            ..alloca(1, CTy::Int(64), 1)
        },
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    // The *object* may well be right — the range search finds it — but chiero must say
    // it guessed. Reaching an address by arithmetic is not a provenance chain.
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "an address reached by arithmetic is a guess: {:#?}",
        s.assumptions()
    );
    assert!(seal(&r, r.witness()).is_err(), "and it cannot seal");
    assert!(
        s.assumptions()
            .iter()
            .any(|x| x.detail.contains("provenance")),
        "{:#?}",
        s.assumptions()
    );
}

/// **A model's fault in a loop is one finding too.** `03d7539`'s dedup key only reached
/// faults raised through `report_faults` — `Store`, `Load`, `*Bits`. Everything routed
/// through a *model* went through `ModelCtx::lift`, which stringified the `MemFault` and
/// threw the struct away, so the engine had nothing to key on and fell back to fork
/// identity. `free`, `memcpy`, `memset`, `strcpy`, `calloc` — most of the bug classes this
/// tool exists for — still flooded. Found by review.
#[test]
fn a_models_fault_in_a_loop_is_one_finding() {
    let mut caller = defined(
        0,
        "main",
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(1),
                        },
                    }),
                ],
                Terminator::Goto(BlockId(1)),
            ),
            block(
                1,
                // `memcpy` into a four-byte destination from a sixteen-byte source: one
                // out-of-bounds report, however many times the loop runs.
                vec![inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Value(ValueId(0)),
                        Operand::Value(ValueId(1)),
                        Operand::Const(Const::Int { bits: 64, val: 16 }),
                    ],
                })],
                Terminator::Goto(BlockId(1)),
            ),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4), alloca(1, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(
                1,
                "memcpy",
                vec![CTy::Ptr, CTy::Ptr, CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let oob: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("out-of-bounds"))
        .collect();
    assert_eq!(oob.len(), 1, "{:#?}", r.findings());
}

/// **Two objectless faults in two functions are two findings.** `FindingKey` had no
/// function component, and `object()` is `None` for `NullDeref`, `WildPointer` and
/// `BadRange` — so with `Span::DUMMY` everywhere the key was identical for all of them and
/// distinct bugs merged. The commit that added the object component argued exactly this
/// for the faults that *have* an object and left the three that do not uncovered.
///
/// Merging is the dangerous direction: a duplicate is noise, a dropped finding is a missed
/// bug. `BadRange` is the usable probe because it is not fatal, so both can happen on one
/// path. Found by review.
#[test]
fn objectless_faults_in_two_functions_do_not_merge() {
    let wide_load = |v: u32| {
        vec![
            inst(InstKind::Assign {
                dst: ValueId(v),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            }),
            inst(InstKind::Assign {
                dst: ValueId(v + 1),
                rv: RValue::Load {
                    addr: Operand::Value(ValueId(v)),
                    ty: CTy::Vector {
                        elem: Box::new(CTy::Int(64)),
                        lanes: 4,
                    },
                    align: 32,
                    vol: Volatility::Normal,
                },
            }),
        ]
    };
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            {
                let mut v = vec![inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                })];
                v.extend(wide_load(0));
                v
            },
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 32,
        ..alloca(0, CTy::Int(8), 32)
    }];
    let mut callee = defined(
        1,
        "other",
        vec![block(0, wide_load(0), Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    callee.allocas = vec![AllocaDecl {
        align: 32,
        ..alloca(0, CTy::Int(8), 32)
    }];
    let m = Module {
        funcs: vec![caller, callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let wide: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("unsupported-access-width"))
        .collect();
    assert_eq!(wide.len(), 2, "two functions, two reports: {:#?}", wide);
}

/// **The `span` component of the finding key, pinned.** Two identical faults on the same
/// object in the same function, at *different* source locations, are two bugs. Nearly
/// every fixture in this file uses `Span::DUMMY`, so dropping `span` from the key changed
/// nothing and the mutation survived — the fixture, not the assertion, was the gap.
#[test]
fn the_same_fault_at_two_places_is_two_findings() {
    let mut caller = defined(
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
                inst_at(
                    InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(0)),
                            ty: CTy::Vector {
                                elem: Box::new(CTy::Int(64)),
                                lanes: 4,
                            },
                            align: 32,
                            vol: Volatility::Normal,
                        },
                    },
                    10,
                ),
                inst_at(
                    InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(0)),
                            ty: CTy::Vector {
                                elem: Box::new(CTy::Int(64)),
                                lanes: 4,
                            },
                            align: 32,
                            vol: Volatility::Normal,
                        },
                    },
                    20,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 32,
        ..alloca(0, CTy::Int(8), 32)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let wide: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("unsupported-access-width"))
        .collect();
    assert_eq!(wide.len(), 2, "two places, two reports: {:#?}", wide);
}

/// **The `kind` component, pinned.** Two *different* faults on the same object at the same
/// place are two bugs, and collapsing `kind` made the second disappear behind the first —
/// a dropped finding, which is the failure mode deduplication must never produce.
#[test]
fn two_kinds_of_fault_on_one_object_are_two_findings() {
    let mut caller = defined(
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
                // Nobody wrote these bytes: an uninitialized read.
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
                // Past the end of the same object: a different bug entirely.
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 32 }),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(2)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("uninitialized-read")),
        "{:#?}",
        r.findings()
    );
    assert!(
        r.findings().iter().any(|f| f.contains("out-of-bounds")),
        "the second bug did not vanish behind the first: {:#?}",
        r.findings()
    );
}

/// **`p->next = NULL` must land.** `address_term` asks the memory model for the address of
/// the stored pointer's object, and `addr_of(ObjectId::NULL)` is always `None` because ids
/// start at 1 — so the store hit the lowering gap, the run degraded to `Unknown`, and the
/// reload manufactured a false uninitialized-read. That is verbatim the failure the
/// pointer-store commit says it fixed: fixed for `p->next = q`, not for `p->next = NULL`.
/// Found by review.
#[test]
fn storing_a_null_pointer_lands_like_any_other() {
    let mut caller = defined(
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
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Null),
                    ty: CTy::Ptr,
                    align: 8,
                    vol: Volatility::Normal,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Ptr,
                        align: 8,
                        vol: Volatility::Normal,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Ptr, 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().is_empty(),
        "writing NULL is not a bug: {:#?}",
        r.findings()
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    // **Read back through an independent path.** The pointer check answers from the very
    // table the store populated, so it agrees with *whatever* address the engine chose —
    // and its old fallback arm accepted a `Scalar(0)` as an answer to "is this a null
    // pointer", which is not an answer at all. Reloading the same eight bytes as an
    // integer asks the memory model instead. Found by review as a same-answer trap.
    match r.states()[0].local(ValueId(1)) {
        Some(Value::Ptr(p)) => assert_eq!(p.base, chiero_mem::ObjectId::NULL),
        other => panic!("a null *pointer*, not {other:?}"),
    }
    let base = match r.states()[0].local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = r.states()[0].mem.clone();
    assert_eq!(
        mem.read(chiero_mem::Pointer { base, off: 0 }, 8, Span::DUMMY)
            .value,
        Some(vec![0u8; 8]),
        "and the bytes really are zero"
    );
}

/// **An errored state is not an exact state, at any of the sites that can error.** None of
/// the `Status::Errored` assignments calls `degrade`, so `State::fidelity()` answers
/// `Exact` for a state that gave up — and only one line in `RunResult::fidelity`, which
/// special-cases the status, stands between that and a PROVEN seal. The review's mutation
/// removed that line and `seal` returned PROVEN for a function containing a construct the
/// engine cannot execute.
#[test]
fn every_errored_state_degrades_itself() {
    // A call to a function id the module does not define.
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(7)),
                args: vec![],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert!(matches!(s.status, Status::Errored(_)), "{:?}", s.status);
    assert_ne!(
        s.fidelity(),
        Fidelity::Exact,
        "the state itself says so, not only the run"
    );
    assert!(
        s.assumptions().iter().any(|x| x.kind.matches(s.fidelity())),
        "and names a cause of the right kind: {:#?}",
        s.assumptions()
    );
}

/// **An argument `scanf` could not resolve is not the absence of a buffer.** The positional
/// `&[Option<Pointer>]` fix made `None` mean "not a pointer", and `scanf` then havocs
/// nothing for that position — so a program whose output argument chiero cannot translate
/// keeps whatever was in the buffer, silently. `havoc_args`' own doc comment says this is
/// the failure to avoid: "the findings on the surviving paths are false and the absences
/// are wrong". Found by review.
#[test]
fn scanf_says_so_when_it_could_not_resolve_an_output_argument() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Const(Const::Null),
                    // An untranslatable output argument: a float constant is not a
                    // pointer and `operand` cannot represent it at all.
                    Operand::Const(Const::Float(FloatKind::F64, 0)),
                ],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "scanf", vec![CTy::Ptr, CTy::Ptr], CTy::Int(32)),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.states()[0]
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("could not be resolved")),
        "chiero says which argument it could not follow: {:#?}",
        r.states()[0].assumptions()
    );
    assert_ne!(r.fidelity(), Fidelity::Exact);
}

/// **A store chiero could not perform must not leave the run provable.** The value path
/// falls to `lowering_gap` when `operand` cannot represent the value — floats, wide
/// vectors, global addresses — and the review found nothing pinned that: with the
/// degradation removed, fidelity stayed `Exact` and `seal` returned **PROVEN** over a
/// program whose write was silently discarded.
///
/// This is the shape that matters most in this project: not a wrong answer, but a *missing*
/// one presented as a proof.
#[test]
fn a_store_chiero_cannot_perform_forbids_a_proof() {
    let mut caller = defined(
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
                // 023 §7 approximates floating point, and `operand` cannot represent a
                // float constant at all — so the store cannot happen.
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Float(FloatKind::F64, 0x4000_0000_0000_0000)),
                    ty: CTy::Float(FloatKind::F64),
                    align: 8,
                    vol: Volatility::Normal,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Float(FloatKind::F64), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "the write did not happen: {:#?}",
        r.states()[0].assumptions()
    );
    assert!(seal(&r, r.witness()).is_err(), "and the run cannot seal");
}

/// **`AllocaDyn` gives a dynamic-extent alloca its size at a program point** (020 §3). A
/// VLA and `alloca()` both lower to it, and `vppinfra/mem.h` defines
/// `clib_mem_alloc_stack` as `__builtin_alloca`, so this is a path VPP actually takes.
///
/// The size arrives *here* rather than in `Function::allocas`, because a function-level
/// table would reference a value computed inside a block — making verifier rule 1
/// undefined for it and creating the object before its size exists.
#[test]
fn alloca_dyn_gives_the_object_its_extent() {
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::AllocaDyn {
                    dst: ValueId(0),
                    alloca: AllocaId(0),
                    elem: CTy::Int(8),
                    count: Operand::Const(Const::Int { bits: 64, val: 12 }),
                    align: 1,
                }),
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 0xAB }),
                    size: Operand::Const(Const::Int { bits: 64, val: 12 }),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        count: chiero_cir::DYNAMIC_EXTENT,
        ..alloca(0, CTy::Int(8), 0)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    let s = &r.states()[0];
    let base = match s.local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    // **The extent is the one `AllocaDyn` supplied**, so the twelfth byte is inside and
    // the thirteenth is not. Asserting only that the write succeeded would pass against
    // an object of any size at least twelve.
    let mut mem = s.mem.clone();
    assert_eq!(
        mem.read(chiero_mem::Pointer { base, off: 11 }, 1, Span::DUMMY)
            .value,
        Some(vec![0xAB])
    );
    assert!(
        mem.read(chiero_mem::Pointer { base, off: 12 }, 1, Span::DUMMY)
            .faults
            .iter()
            .any(|f| f.kind() == "out-of-bounds"),
        "and the object ends where it was told to"
    );
    // **The alloca id now names this object.** 020 §3 declares the alloca in the function
    // table and `AllocaDyn` supplies its extent, so a later `AddrOfLocal` must reach the
    // same object — otherwise the declaration and the instruction describe two different
    // pieces of memory.
    assert_eq!(
        s.local(ValueId(1)),
        s.local(ValueId(0)),
        "the alloca id and the AllocaDyn result are the same object"
    );
}

/// **The extent is `count * sizeof(elem)`, and it multiplies rather than wraps.**
/// `DYNAMIC_EXTENT` is `u64::MAX`, and the danger has always been a *small* wrapped object
/// rather than a panic — a wrapped size silently accepts or rejects the wrong accesses.
/// Three `int`s is twelve bytes, which a size that ignored `elem` would get wrong in the
/// safe-looking direction.
///
/// This is the property `a_dynamic_extent_does_not_overflow_the_size_computation` was
/// reaching for before verification landed and made its module unexecutable; the
/// computation now lives in `AllocaDyn`, so this is where it can be observed.
#[test]
fn a_dynamic_extent_multiplies_the_element_size() {
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::AllocaDyn {
                    dst: ValueId(0),
                    alloca: AllocaId(0),
                    elem: CTy::Int(32),
                    count: Operand::Const(Const::Int { bits: 64, val: 3 }),
                    align: 4,
                }),
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 1 }),
                    size: Operand::Const(Const::Int { bits: 64, val: 12 }),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        count: chiero_cir::DYNAMIC_EXTENT,
        ..alloca(0, CTy::Int(32), 0)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().is_empty(),
        "twelve bytes fit in three ints: {:#?}",
        r.findings()
    );
    let s = &r.states()[0];
    let base = match s.local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = s.mem.clone();
    // **The fault kind matters.** An over-allocated object faults here too — with an
    // *uninitialized* read — so asserting merely that something went wrong passes against
    // an object of any larger size. The same-answer trap, in my own assertion.
    assert!(
        mem.read(chiero_mem::Pointer { base, off: 12 }, 1, Span::DUMMY)
            .faults
            .iter()
            .any(|f| f.kind() == "out-of-bounds"),
        "and thirteen do not"
    );
}

/// **Dead code does not make a module unexecutable.** 020 rule 3 makes `UnreachableBlock`
/// a *warning* — "unreachable C code exists and is legal" — and wave 7 fixed the dominance
/// lattice specifically so a live join after dead code would work. Wiring verification into
/// `run` gated on `!errs.is_empty()` instead of on `is_error()`, so a `default:` after an
/// exhaustive switch, or anything after a `return`, refused to run at all. Found by review;
/// the bug is mine, from the commit that added the gate.
#[test]
fn a_module_with_dead_code_still_runs() {
    let caller = defined(
        0,
        "main",
        vec![
            block(0, vec![], Terminator::Return(Some(i32c(7)))),
            // Nothing branches here.
            block(1, vec![], Terminator::Return(Some(i32c(9)))),
        ],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        !matches!(r.states()[0].status, Status::Errored(_)),
        "{:?}",
        r.states()[0].status
    );
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(7));
    assert_eq!(r.fidelity(), Fidelity::Exact);
}

/// **Address zero is `NULL`, not an unknown object.** `int_to_ptr` range-searches and falls
/// through to `UNBOUND`, so `*(int *)0 = 1` reported a *wild pointer* — "matching no known
/// object" — instead of a null dereference, and degraded to `Unknown` on top. C spells
/// `NULL` as `((void *)0)`, so any frontend lowering it through `IntToPtr` mis-classifies
/// the commonest bug in C. Found by review.
#[test]
fn address_zero_is_null_not_an_unknown_object() {
    let mut m = chiero_mem::Memory::new();
    let _o = m.alloc(chiero_mem::ObjKind::Heap, 16, 8, Span::DUMMY);
    assert_eq!(m.object_containing(0).base, chiero_mem::ObjectId::NULL);
    // And a plausible-looking non-zero address still resolves to nothing, so this is not
    // "everything unknown becomes NULL".
    assert_eq!(
        m.object_containing(0xDEAD_BEEF).base,
        chiero_mem::ObjectId::UNBOUND
    );
}

/// **Provenance crosses a return.** Moving `IntToPtr` onto the per-frame dataflow table
/// closed the laundering hole and took an honest case with it: `uintptr_t f(void) { return
/// (uintptr_t)&g; } … int *p = (int *)f();` degraded to `Unknown`, because the callee's
/// frame — and its provenance — is gone by the time the caller casts back. Index↔pointer
/// conversion helpers are the dominant VPP idiom, so this is not a corner. Found by review,
/// which was right that the commit message noted the laundering case degrading and not
/// this one.
///
/// The value is *returned*, which is a dataflow edge like any other; it is the frame
/// boundary that lost it, not the arithmetic.
#[test]
fn provenance_crosses_a_return() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(0)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![];
    let mut callee = defined(
        1,
        "addr_of_local",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Cast {
                        kind: CastKind::PtrToInt,
                        a: Operand::Value(ValueId(0)),
                        from: CTy::Ptr,
                        to: CTy::Int(64),
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(64),
    );
    callee.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(64), 1)
    }];
    let m = Module {
        funcs: vec![caller, callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert!(
        matches!(s.local(ValueId(1)), Some(Value::Ptr(_))),
        "a pointer came back, not a guess: {:?}",
        s.local(ValueId(1))
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "an honest round trip is not a range search: {:#?}",
        s.assumptions()
    );
}

/// **A parameter chiero never bound is not an unresolvable argument.** `void read_num(int
/// *out) { scanf("%d", out); }` analysed as the entry function has no binding for `out` —
/// `run` starts the entry frame with empty locals — so the `unresolved_args` check I added
/// fires and degrades a perfectly ordinary function to `Unknown`. Every VPP function taking
/// an output pointer hits it when analysed in isolation. Found by review.
///
/// The fix is the root cause, not the check: an entry parameter gets a **fresh object** of
/// unknown contents, which is what "this function is called from somewhere chiero has not
/// seen" actually means.
#[test]
fn an_entry_parameter_is_an_object_not_a_hole() {
    let mut f = defined(
        0,
        "read_num",
        vec![block(
            0,
            vec![inst(InstKind::Store {
                addr: Operand::Value(ValueId(0)),
                val: Operand::Const(Const::Int { bits: 32, val: 7 }),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    f.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Ptr,
    }];
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert!(
        matches!(s.local(ValueId(0)), Some(Value::Ptr(_))),
        "the parameter is a pointer to something: {:?}",
        s.local(ValueId(0))
    );
    // The store lands, so nothing downstream reasons about a write that did not happen.
    assert!(
        !r.findings().iter().any(|f| f.contains("out-of-bounds")),
        "{:#?}",
        r.findings()
    );
}

/// **A zeroed pointer field is a null pointer.** The canonical C bug —
/// `struct node *n = calloc(1, sizeof *n); n->next->x = 1;` — reported *nothing*, because
/// a `CTy::Ptr` load consulted only the table `address_term` seeds, and bytes written by
/// `calloc`, `memset`, a `memcpy` of a zeroed struct or a `.bss` global are not in it. The
/// field reloaded as a scalar and the dereference became a lowering gap. Zero recall on a
/// bug class this tool exists for. Found by review.
#[test]
fn a_zeroed_pointer_field_dereferences_as_null() {
    let mut caller = defined(
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
                // The whole struct zeroed, as `calloc` leaves it.
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 0 }),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Ptr,
                        align: 8,
                        vol: Volatility::Normal,
                    },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(1)),
                    val: Operand::Const(Const::Int { bits: 32, val: 1 }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Ptr, 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("null-dereference")),
        "{:#?}",
        r.findings()
    );
}

/// **Null-ness does not depend on what happened earlier on the path.** Because an address
/// term is hash-consed, an unrelated `q = NULL` stored anywhere earlier seeded the table
/// that made a zero word read back as a pointer — so the *same* program answered
/// differently depending on code that had nothing to do with it. Found by review.
#[test]
fn a_zero_word_reads_as_null_whatever_came_before() {
    let build = |with_prior_null: bool| {
        let mut insts = vec![
            inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            }),
            inst(InstKind::Assign {
                dst: ValueId(1),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(1),
                },
            }),
        ];
        if with_prior_null {
            insts.push(inst(InstKind::Store {
                addr: Operand::Value(ValueId(1)),
                val: Operand::Const(Const::Null),
                ty: CTy::Ptr,
                align: 8,
                vol: Volatility::Normal,
            }));
        }
        insts.push(inst(InstKind::SetMem {
            dst: Operand::Value(ValueId(0)),
            byte: Operand::Const(Const::Int { bits: 8, val: 0 }),
            size: Operand::Const(Const::Int { bits: 64, val: 8 }),
        }));
        insts.push(inst(InstKind::Assign {
            dst: ValueId(2),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(0)),
                ty: CTy::Ptr,
                align: 8,
                vol: Volatility::Normal,
            },
        }));
        let mut caller = defined(
            0,
            "main",
            vec![block(0, insts, Terminator::Return(Some(i32c(0))))],
            CTy::Int(32),
        );
        caller.allocas = vec![
            AllocaDecl {
                align: 8,
                ..alloca(0, CTy::Ptr, 1)
            },
            AllocaDecl {
                align: 8,
                ..alloca(1, CTy::Ptr, 1)
            },
        ];
        Module {
            funcs: vec![caller],
            ..Default::default()
        }
    };
    let kind = |m: &Module| {
        let mut a = TermArena::new();
        let r = Engine::new(m).run(&mut a);
        match r.states()[0].local(ValueId(2)) {
            Some(Value::Ptr(p)) => format!("ptr:{:?}", p.base),
            Some(Value::Scalar(_)) => "scalar".to_string(),
            other => format!("{other:?}"),
        }
    };
    assert_eq!(
        kind(&build(false)),
        kind(&build(true)),
        "an unrelated `q = NULL` changed how a zero word reads"
    );
    assert!(kind(&build(false)).starts_with("ptr:"));
}

/// **The *model* finding key, pinned in all four components.** An earlier commit claimed
/// "all four fail a mutation individually"; that was true at `report_faults` and false at
/// `dispatch`, where only the *existence* of a key was pinned. Two dedup paths, one tested.
/// Found by review.
///
/// Each case merges a pair that differs in exactly one component, so each component is
/// load-bearing on its own.
#[test]
fn the_model_finding_key_distinguishes_all_four_components() {
    let ptr_to = |v: u32, alloca: u32| {
        inst(InstKind::Assign {
            dst: ValueId(v),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(alloca),
            },
        })
    };
    let big_memset = |p: u32, span: u32| {
        inst_at(
            InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Value(ValueId(p)),
                    Operand::Const(Const::Int { bits: 32, val: 0 }),
                    Operand::Const(Const::Int { bits: 64, val: 64 }),
                ],
            },
            span,
        )
    };
    let memset_decl = || {
        extern_fn(
            1,
            "memset",
            vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
            CTy::Ptr,
        )
    };

    // `object`: two 4-byte buffers, same kind, same span, same function.
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                ptr_to(0, 0),
                ptr_to(1, 1),
                big_memset(0, 0),
                big_memset(1, 0),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4), alloca(1, CTy::Int(8), 4)];
    let m = Module {
        funcs: vec![caller, memset_decl()],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("out-of-bounds"))
            .count(),
        2,
        "two buffers: {:#?}",
        r.findings()
    );

    // `span`: one buffer, two call sites.
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![ptr_to(0, 0), big_memset(0, 100), big_memset(0, 200)],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4)];
    let m = Module {
        funcs: vec![caller, memset_decl()],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("out-of-bounds"))
            .count(),
        2,
        "two call sites: {:#?}",
        r.findings()
    );

    // `func`: the **same object** overflowed in a callee and in the caller, at the same
    // span. Two different buffers would be told apart by `object` alone, which is how the
    // first version of this case passed while `func` was still droppable.
    let mut callee = defined(
        2,
        "other",
        vec![block(
            0,
            vec![big_memset(0, 0)],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    callee.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Ptr,
    }];
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                ptr_to(0, 0),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![Operand::Value(ValueId(0))],
                }),
                big_memset(0, 0),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4)];
    let m = Module {
        funcs: vec![caller, memset_decl(), callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("out-of-bounds"))
            .count(),
        2,
        "two functions: {:#?}",
        r.findings()
    );

    // `kind`: one `memcpy` raising an overlap *and* an out-of-bounds on one object at one
    // span — collapsing `kind` makes the second vanish behind the first.
    let mut caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                ptr_to(0, 0),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 6 }),
                    },
                }),
                inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Value(ValueId(1)),
                        Operand::Value(ValueId(0)),
                        Operand::Const(Const::Int { bits: 64, val: 8 }),
                    ],
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 12)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(
                1,
                "memcpy",
                vec![CTy::Ptr, CTy::Ptr, CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let kinds: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("overlapping-copy") || f.contains("out-of-bounds"))
        .collect();
    assert_eq!(
        kinds.len(),
        2,
        "two kinds, one object, one span: {kinds:#?}"
    );
}

/// **`&global` as a constant operand is a pointer.** `Engine::operand` handled only
/// `Const::Int` and `Const::Null`, so `Const::GlobalAddr` and `Const::FuncAddr` — the
/// constant forms of the two `AddrOf` rvalues, which a frontend emits wherever an address
/// is a compile-time constant — were lowering gaps. `(uintptr_t)&g` written as a constant
/// operand reported "PtrToInt of a non-pointer". Found by review.
#[test]
fn a_constant_address_is_a_pointer_like_any_other() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Use(Operand::Const(Const::GlobalAddr {
                        g: GlobalId(0),
                        off: 0,
                    })),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfGlobal { g: GlobalId(0) },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Use(Operand::Const(Const::FuncAddr(FuncId(1)))),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::AddrOfFunc(FuncId(1)),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Use(Operand::Const(Const::GlobalAddr {
                        g: GlobalId(0),
                        off: 2,
                    })),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let other = defined(
        1,
        "other",
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, other],
        globals: vec![Global {
            id: GlobalId(0),
            name: "counter".into(),
            size: 4,
            align: 4,
            is_const: false,
            span: Span::DUMMY,
        }],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    // **The constant and the rvalue name the same object**, which is the property that
    // matters: a second object for the same global would make `p == &counter` false
    // against itself.
    assert_eq!(s.local(ValueId(0)), s.local(ValueId(1)));
    assert_eq!(s.local(ValueId(2)), s.local(ValueId(3)));
    assert_ne!(s.local(ValueId(0)), s.local(ValueId(2)));
    // **The offset is part of the constant.** `&s.field` is a `GlobalAddr` with a non-zero
    // `off`, which is how a frontend spells a member address — dropping it silently aims
    // every such access at the start of the object.
    match s.local(ValueId(4)) {
        Some(Value::Ptr(p)) => assert_eq!(p.off, 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.fidelity(), Fidelity::Exact, "{:#?}", s.assumptions());
}

/// **`va_list` is memory, not engine state** (020 §4.4.1). It has to be, because VPP
/// passes `va_list *` **by address across function boundaries** — `format.h` declares
/// `typedef u8 *(format_function_t)(u8 *s, va_list *args);` and there are 2552 occurrences
/// of `va_list *` in the tree. The whole format, unformat, CLI and trace infrastructure is
/// built on the callee advancing the *caller's* iteration state.
///
/// `VaArg` is an instruction rather than an `RValue` because it **mutates**: it reads the
/// next argument and advances the list. A side-effect-free `RValue` that advanced would be
/// self-contradictory, and any pass allowed to reorder or CSE `RValue`s would silently
/// corrupt argument order.
#[test]
fn va_arg_reads_the_variadic_arguments_in_order() {
    let mut callee = defined(
        1,
        "sum_two",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
                inst(InstKind::VaStart {
                    list: Operand::Value(ValueId(1)),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(2),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(3),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::VaEnd {
                    list: Operand::Value(ValueId(1)),
                }),
                // The *difference*, so reading the same argument twice — or reading them
                // in the wrong order — gives a different answer from reading them once
                // each in order.
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Bin {
                        op: BinOp::Sub,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(2)),
                        b: Operand::Value(ValueId(3)),
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(4)))),
        )],
        CTy::Int(32),
    );
    callee.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Int(32),
    }];
    callee.variadic = true;
    callee.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(8), 24)
    }];
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Const(Const::Int { bits: 32, val: 2 }),
                    Operand::Const(Const::Int { bits: 32, val: 50 }),
                    Operand::Const(Const::Int { bits: 32, val: 8 }),
                ],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(42),
        "50 then 8, in that order"
    );
}

/// **The vector operations, so `eval` is exhaustive like `exec_inst`.** vppinfra is built
/// on `u8x16`/`u32x4` and friends — `vec_len`, the hash lookups, the packet classifiers —
/// so a lowering gap here degrades most of the code this tool targets.
///
/// A vector is a bit-vector of `lanes * width` bits, little-endian by lane, which is what
/// 021 §3's "bytes are bytes" means for SIMD: lane 0 occupies the low bits, exactly as a
/// load of the same memory would see it.
#[test]
fn the_vector_operations_move_lanes_where_they_belong() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                // Four lanes of 0xAB.
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Splat {
                        elem: Operand::Const(Const::Int { bits: 8, val: 0xAB }),
                        lanes: 4,
                    },
                }),
                // Lane 2 becomes 0x01, so an implementation that wrote lane 0, or that
                // indexed from the high end, gives a different answer.
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::InsertLane {
                        v: Operand::Value(ValueId(0)),
                        lane: 2,
                        val: Operand::Const(Const::Int { bits: 8, val: 0x01 }),
                    },
                }),
                // **A distinguishable second operand.** With `b` equal to `a`'s splat,
                // lane 0 of each is the same byte, so a shuffle that never crossed into
                // `b` gave the same answer — the same-answer trap, in the fixture.
                inst(InstKind::Assign {
                    dst: ValueId(8),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(0)),
                        lane: 3,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(7),
                    rv: RValue::Splat {
                        elem: Operand::Const(Const::Int { bits: 8, val: 0x77 }),
                        lanes: 4,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(1)),
                        lane: 2,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(1)),
                        lane: 0,
                    },
                }),
                // A shuffle picking lane 2 of `a` and lane 0 of `b`; indices past `lanes`
                // address the second operand (020 rule 12).
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Shuffle {
                        a: Operand::Value(ValueId(1)),
                        b: Operand::Value(ValueId(7)),
                        mask: vec![2, 4, 2, 4],
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(5),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(4)),
                        lane: 0,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(6),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(4)),
                        lane: 1,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    let s = &r.states()[0];
    let bits = |v: u32, a: &mut TermArena| match s.local(ValueId(v)) {
        Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
        other => panic!("{other:?}"),
    };
    assert_eq!(bits(0, &mut a), Some(0xABAB_ABAB), "splat fills every lane");
    // **The recorded lane width is the element's, not a multiple of it.** A splat that
    // recorded twice the width still produces the right bits and only reads back wrong,
    // so the extract is what pins it — asserting on the splat alone cannot.
    assert_eq!(
        bits(8, &mut a),
        Some(0xAB),
        "one lane of the splat is one element"
    );
    assert_eq!(bits(2, &mut a), Some(0x01), "the lane that was written");
    assert_eq!(
        bits(3, &mut a),
        Some(0xAB),
        "and its neighbour is untouched"
    );
    assert_eq!(bits(5, &mut a), Some(0x01), "mask 2 takes a's lane 2");
    assert_eq!(
        bits(6, &mut a),
        Some(0x77),
        "mask 4 crosses into b's lane 0"
    );
}

/// **Every way of erroring degrades the state itself.** None of the `Status::Errored`
/// assignments called `degrade`, so `State::fidelity()` answered `Exact` for a state that
/// had given up — and only one line in `RunResult::fidelity`, which special-cases the
/// status, stood between that and a PROVEN seal. The review's mutation removed that line
/// and `seal` returned PROVEN. One untested line is not a place to keep the project's
/// central guarantee. Found by review.
#[test]
fn every_way_of_erroring_degrades_the_state() {
    // Each of these reaches a different `Status::Errored` site.
    let unknown_callee = Module {
        funcs: vec![defined(
            0,
            "main",
            vec![block(
                0,
                vec![inst(InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(7)),
                    args: vec![],
                })],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        )],
        ..Default::default()
    };
    // **The targets exist**, so `verify` passes and the only thing that can go wrong is
    // the terminator being unimplemented. An earlier version branched to a missing block,
    // which `verify` rejects — so the run errored on the *verification* path and the
    // engine's own site was never reached.
    let unsupported_term = Module {
        funcs: vec![defined(
            0,
            "main",
            vec![
                block(
                    0,
                    vec![],
                    Terminator::Switch {
                        scrut: i32c(1),
                        ty: CTy::Int(32),
                        cases: vec![(1, BlockId(1))],
                        default: BlockId(1),
                    },
                ),
                block(1, vec![], Terminator::Return(Some(i32c(0)))),
            ],
            CTy::Int(32),
        )],
        ..Default::default()
    };
    let bad_branch = Module {
        funcs: vec![defined(
            0,
            "main",
            vec![
                block(
                    0,
                    vec![inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    })],
                    Terminator::Br {
                        cond: Operand::Value(ValueId(0)),
                        t: BlockId(1),
                        f: BlockId(1),
                    },
                ),
                block(1, vec![], Terminator::Return(Some(i32c(0)))),
            ],
            CTy::Int(32),
        )],
        ..Default::default()
    };
    for (what, m) in [
        ("an unknown callee", unknown_callee),
        ("an unsupported terminator", unsupported_term),
        ("a non-scalar branch condition", bad_branch),
    ] {
        let mut m = m;
        m.funcs[0].allocas = vec![AllocaDecl {
            align: 8,
            ..alloca(0, CTy::Ptr, 1)
        }];
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        let s = &r.states()[0];
        // **No escape hatch.** An earlier version skipped cases that did not error, and
        // every one of them was being rejected by `verify` instead — so the loop body
        // never ran and the test asserted nothing at all.
        assert!(
            matches!(s.status, Status::Errored(_)),
            "{what}: expected this to error, got {:?}",
            s.status
        );
        assert_ne!(
            s.fidelity(),
            Fidelity::Exact,
            "{what}: the state itself must say so"
        );
        assert!(
            s.assumptions().iter().any(|x| x.kind.matches(s.fidelity())),
            "{what}: and name a cause of the right kind: {:#?}",
            s.assumptions()
        );
    }
}

/// **020 contract 37 and 021 contract 31, the case the whole design exists for.**
/// A `va_list` object round-trips through `VaStart`/`VaArg`/`VaEnd` and is **addressable**:
/// taking its address and passing it to another function that advances it is visible to the
/// caller.
///
/// The original note: A callee takes `va_list *`
/// and advances the *caller's* iteration state — `format_function_t` is
/// `u8 *(u8 *s, va_list *args)` and there are 2552 of them in VPP. The cursor crosses the
/// boundary because it lives in the object's bytes, but the *argument values* were read
/// from `stack.last()`, which is the callee's frame — empty, because the callee is not
/// variadic. So every `va_arg` was "past the end" and the whole format infrastructure
/// degraded to `Unknown`.
///
/// My commit message for the varargs work said engine-side state "cannot express that at
/// all" and then kept the argument area as engine-side per-frame state. Found by review.
#[test]
fn a_callee_advances_the_callers_va_list() {
    // int take_one(va_list *ap) { return va_arg(*ap, int); }
    let mut take_one = defined(
        2,
        "take_one",
        vec![block(
            0,
            vec![inst(InstKind::VaArg {
                dst: ValueId(1),
                list: Operand::Value(ValueId(0)),
                ty: CTy::Int(32),
            })],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    take_one.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Ptr,
    }];
    // int outer(int n, ...) { va_start; a = take_one(&ap); b = va_arg(ap,int); return a-b; }
    let mut outer = defined(
        1,
        "outer",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
                inst(InstKind::VaStart {
                    list: Operand::Value(ValueId(1)),
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(2)),
                    callee: Callee::Direct(FuncId(2)),
                    args: vec![Operand::Value(ValueId(1))],
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(3),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::VaEnd {
                    list: Operand::Value(ValueId(1)),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Bin {
                        op: BinOp::Sub,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(2)),
                        b: Operand::Value(ValueId(3)),
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(4)))),
        )],
        CTy::Int(32),
    );
    outer.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Int(32),
    }];
    outer.variadic = true;
    outer.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(8), 24)
    }];
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Const(Const::Int { bits: 32, val: 2 }),
                    Operand::Const(Const::Int { bits: 32, val: 50 }),
                    Operand::Const(Const::Int { bits: 32, val: 8 }),
                ],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, outer, take_one],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(42),
        "the callee consumed 50 and left 8 for the caller"
    );
}

/// **A vararg the engine cannot represent is a hole, not an absence.** `filter_map`
/// compacted the argument area, so a `printf("%f %d", 1.5, 42)` — floats being a
/// documented gap in `operand` — silently handed the *next* argument back for the `%d`,
/// at `Fidelity::Exact` with no finding. That is the confidently-wrong shape this engine
/// refuses everywhere else, on the wrong side of a `filter_map`. Found by review.
#[test]
fn an_unrepresentable_vararg_does_not_shift_the_ones_after_it() {
    let mut pick = defined(
        1,
        "pick",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
                inst(InstKind::VaStart {
                    list: Operand::Value(ValueId(1)),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(2),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Float(FloatKind::F64),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(3),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(3)))),
        )],
        CTy::Int(32),
    );
    pick.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Int(32),
    }];
    pick.variadic = true;
    pick.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(8), 24)
    }];
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Const(Const::Int { bits: 32, val: 3 }),
                    Operand::Const(Const::Float(FloatKind::F64, 0x3FF8_0000_0000_0000)),
                    Operand::Const(Const::Int { bits: 32, val: 42 }),
                    Operand::Const(Const::Int { bits: 32, val: 7 }),
                ],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, pick],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    // The float is a gap, so the run is not exact — but the *second* argument must never
    // come back as the third.
    assert_ne!(
        r.states()[0].return_value_bits(&mut a),
        Some(7),
        "the arguments after the gap did not shift down"
    );
    assert_ne!(r.fidelity(), Fidelity::Exact);
}

/// **`va_arg`'s declared type decides the width.** It handed the caller's `Value` back
/// verbatim, so a 64-bit argument read as `int` produced a 64-bit value in a local the
/// verifier believes is `i32` — and comparing it against an `i32` **panicked the engine**
/// on a width assertion. A verified module must not kill the process; that is wave 5's
/// class one layer down. Found by review.
#[test]
fn va_arg_narrows_to_its_declared_type() {
    let mut f = defined(
        1,
        "take",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
                inst(InstKind::VaStart {
                    list: Operand::Value(ValueId(1)),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(2),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Cmp {
                        op: CmpOp::Eq,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(2)),
                        b: Operand::Const(Const::Int { bits: 32, val: 7 }),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    f.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Int(32),
    }];
    f.variadic = true;
    f.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(8), 24)
    }];
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: None,
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Const(Const::Int { bits: 32, val: 1 }),
                    Operand::Const(Const::Int {
                        bits: 64,
                        val: 0x1_0000_0007,
                    }),
                ],
            })],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    // The point is that this returns at all.
    let r = Engine::new(&m).run(&mut a);
    let s = r.states().iter().find(|s| s.local(ValueId(2)).is_some());
    if let Some(s) = s {
        match s.local(ValueId(2)) {
            Some(Value::Scalar(t)) => assert_eq!(a.width(t), 32, "narrowed to `int`"),
            other => panic!("{other:?}"),
        }
    }
}

/// **A shuffle's lane count comes from the operand, not from the mask.** 020 rule 12 says
/// the mask indices are `< 2 * lanes`, and the verifier reads `lanes` from the operand's
/// declared `CTy::Vector` — nothing requires `mask.len() == lanes`. That is
/// `__builtin_shufflevector` semantics, where the mask length is the *result* length. The
/// engine used `mask.len()` for three things it is not: the element width divisor, the
/// a/b split, and the recorded lane width. A widening shuffle read nibbles and never
/// touched `b`; a narrowing one returned the wrong width. Both at `Fidelity::Exact`.
/// Found by review.
#[test]
fn a_widening_shuffle_reads_whole_lanes_from_both_operands() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Splat {
                        elem: Operand::Const(Const::Int { bits: 8, val: 0x11 }),
                        lanes: 4,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Splat {
                        elem: Operand::Const(Const::Int { bits: 8, val: 0x22 }),
                        lanes: 4,
                    },
                }),
                // Eight lanes out of two four-lane operands: indices 0..3 are `a`'s and
                // 4..7 are `b`'s.
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Shuffle {
                        a: Operand::Value(ValueId(0)),
                        b: Operand::Value(ValueId(1)),
                        mask: vec![0, 1, 2, 3, 4, 5, 6, 7],
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
    let s = &r.states()[0];
    match s.local(ValueId(2)) {
        Some(Value::Scalar(t)) => {
            assert_eq!(a.width(t), 64, "eight 8-bit lanes");
            assert_eq!(
                a.eval_ground(t).ok().map(|c| c.bits()),
                Some(0x2222_2222_1111_1111)
            );
        }
        other => panic!("{other:?}"),
    }
    // Read off the shuffle's own value rather than through `ExtractLane`: the verifier
    // types the result by the *operand's* lane count, so an extract at lane 7 is rejected
    // before the engine runs. The bits are the observable that survives that.
}

/// **A lane is not always eight bits.** Every value in
/// `the_vector_operations_move_lanes_where_they_belong` is a byte, so hard-coding a lane
/// width of 8 survives the whole suite — the "recorded, not derived" claim was unpinned
/// against the simplest wrong implementation. One non-8-bit vector closes it. Found by
/// review, and the trap is in the fixture as usual.
#[test]
fn a_sixteen_bit_lane_is_sixteen_bits() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Splat {
                        elem: Operand::Const(Const::Int {
                            bits: 16,
                            val: 0xBEEF,
                        }),
                        lanes: 2,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::InsertLane {
                        v: Operand::Value(ValueId(0)),
                        lane: 1,
                        val: Operand::Const(Const::Int {
                            bits: 16,
                            val: 0x1234,
                        }),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(1)),
                        lane: 0,
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::ExtractLane {
                        v: Operand::Value(ValueId(1)),
                        lane: 1,
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    let bits = |v: u32, a: &mut TermArena| match s.local(ValueId(v)) {
        Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
        other => panic!("{other:?}"),
    };
    assert_eq!(bits(1, &mut a), Some(0x1234_BEEF));
    assert_eq!(
        bits(2, &mut a),
        Some(0xBEEF),
        "lane 0 is the low sixteen bits"
    );
    assert_eq!(bits(3, &mut a), Some(0x1234));
}

/// **`va_copy` duplicates the iteration state and the two advance independently.** There
/// was no `va_copy` test at all, so 020 contract 36's second half was unpinned in three
/// directions at once — a no-op copy, a zero-length copy and a backwards copy all
/// survived. Found by review.
#[test]
fn va_copy_advances_independently_of_its_source() {
    let mut f = defined(
        1,
        "take",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::VaStart {
                    list: Operand::Value(ValueId(1)),
                }),
                // a consumes one, then b copies from a and consumes two more; a's own
                // next read must be the *second* argument, not the fourth.
                inst(InstKind::VaArg {
                    dst: ValueId(3),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::VaCopy {
                    dst: Operand::Value(ValueId(2)),
                    src: Operand::Value(ValueId(1)),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(4),
                    list: Operand::Value(ValueId(2)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(5),
                    list: Operand::Value(ValueId(2)),
                    ty: CTy::Int(32),
                }),
                inst(InstKind::VaArg {
                    dst: ValueId(6),
                    list: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(6)))),
        )],
        CTy::Int(32),
    );
    f.params = vec![Param {
        value: ValueId(0),
        ty: CTy::Int(32),
    }];
    f.variadic = true;
    f.allocas = vec![
        AllocaDecl {
            align: 8,
            ..alloca(0, CTy::Int(8), 24)
        },
        AllocaDecl {
            align: 8,
            ..alloca(1, CTy::Int(8), 24)
        },
    ];
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Call {
                dst: Some(ValueId(0)),
                callee: Callee::Direct(FuncId(1)),
                args: vec![
                    Operand::Const(Const::Int { bits: 32, val: 4 }),
                    Operand::Const(Const::Int { bits: 32, val: 11 }),
                    Operand::Const(Const::Int { bits: 32, val: 8 }),
                    Operand::Const(Const::Int { bits: 32, val: 33 }),
                    Operand::Const(Const::Int { bits: 32, val: 44 }),
                ],
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(8),
        "a's second read is the second argument, whatever b did"
    );
    assert_eq!(r.fidelity(), Fidelity::Exact);
}

/// **A model that gives up inside a loop reports once.** `ModelOutcome::Finding` is pushed
/// with no key, so it falls back to fork identity and produces one copy per iteration —
/// the exact shape already fixed for *lifted* faults and missed on this path. A `strcpy`
/// overflow inside a loop over `VLIB_FRAME_SIZE` buffers is 256 copies of one bug. Raised
/// as a suspicion by review; confirmed here.
#[test]
fn a_model_that_gives_up_in_a_loop_reports_once() {
    let mut caller = defined(
        0,
        "main",
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(1),
                        },
                    }),
                    // 'x' with no terminator, so `strcpy`'s source scan gives up and the
                    // model returns a bare `Finding`.
                    inst(InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![
                            Operand::Value(ValueId(1)),
                            Operand::Const(Const::Int { bits: 8, val: 120 }),
                            Operand::Const(Const::Int { bits: 64, val: 16 }),
                        ],
                    }),
                ],
                Terminator::Goto(BlockId(1)),
            ),
            // **Two call sites in the loop.** With one, the span component of the key is
            // unpinned: every report already shares it.
            block(
                1,
                vec![
                    inst_at(
                        InstKind::Call {
                            dst: None,
                            callee: Callee::Direct(FuncId(1)),
                            args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                        },
                        10,
                    ),
                    inst_at(
                        InstKind::Call {
                            dst: None,
                            callee: Callee::Direct(FuncId(1)),
                            args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                        },
                        20,
                    ),
                ],
                Terminator::Goto(BlockId(1)),
            ),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 4), alloca(1, CTy::Int(8), 16)];
    let m = Module {
        funcs: vec![
            caller,
            extern_fn(1, "strcpy", vec![CTy::Ptr, CTy::Ptr], CTy::Ptr),
            extern_fn(
                2,
                "memset",
                vec![CTy::Ptr, CTy::Int(32), CTy::Int(64)],
                CTy::Ptr,
            ),
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let gave_up: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("strcpy"))
        .collect();
    assert_eq!(
        gave_up.len(),
        2,
        "one per call site, not per iteration: {gave_up:#?}"
    );
}

/// **020 contract 9.** Division and remainder by zero follow §4.1's table and **execution
/// continues**: `UDiv 5 0` and `SDiv 5 0` are all-ones, `SDiv (-5) 0` is `1`, `URem 5 0`
/// is `5`, `SRem (-5) 0` is `-5`. The contract also requires each to agree with the
/// solver's evaluation of the same term, so the IR and solver conventions cannot drift —
/// which is the half that makes this worth a test rather than a table lookup.
///
/// Continuing matters: C leaves it undefined, but a symbolic executor that stops there
/// stops analysing everything after it, and these are SMT-LIB's total semantics.
#[test]
fn division_by_zero_follows_the_table_and_execution_continues() {
    let cases: [(BinOp, i128, i128, u128); 5] = [
        (BinOp::UDiv, 5, 0, u32::MAX as u128),
        (BinOp::SDiv, 5, 0, u32::MAX as u128),
        (BinOp::SDiv, -5, 0, 1),
        (BinOp::URem, 5, 0, 5),
        (BinOp::SRem, -5, 0, (-5i32) as u32 as u128),
    ];
    for (op, x, y, want) in cases {
        let caller = defined(
            0,
            "main",
            vec![block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Bin {
                            op,
                            ty: CTy::Int(32),
                            a: i32c(x),
                            b: i32c(y),
                        },
                    }),
                    // Execution *continues*: this instruction after the division must run.
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Use(i32c(99)),
                    }),
                ],
                Terminator::Return(Some(Operand::Value(ValueId(0)))),
            )],
            CTy::Int(32),
        );
        let m = Module {
            funcs: vec![caller],
            ..Default::default()
        };
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        let s = &r.states()[0];
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(want),
            "{op:?} {x} {y}"
        );
        assert!(
            s.local(ValueId(1)).is_some(),
            "{op:?}: the instruction after the division ran"
        );
        // **This is where the solver-agreement half used to be, and it was vacuous.**
        // Both operands are constants, so `TermArena::bin` folds at construction and the
        // local is already a `Node::Const`; `return_value_bits` *is* `eval_ground`, the
        // same call written out by hand. Mutation proved it: deleting the assertion lost
        // nothing. Contract 9's drift check needs a term the folder cannot collapse, so it
        // is a separate test below with a symbolic operand — found by review.
    }
}

/// **020 contract 7's value half.** `Int(32)` `Add` of `0x7FFFFFFF` and `1` is
/// `0x80000000` — wrapping is *defined* in the IR even though C leaves signed overflow
/// undefined, because an IR that trapped could not represent the program the compiler
/// actually emits.
///
/// The contract's other half — "emits exactly one signed-overflow event" — needs 023 §6's
/// event surface, which is not implemented. That half is uncovered and recorded, not
/// quietly claimed by this test.
#[test]
fn signed_overflow_wraps_and_is_defined() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![inst(InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Bin {
                    op: BinOp::Add,
                    ty: CTy::Int(32),
                    a: i32c(0x7FFF_FFFF),
                    b: i32c(1),
                },
            })],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(0x8000_0000));
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "wrapping is not an approximation"
    );
}

/// **020 contracts 19 and 20: type punning through a union, both directions.** Storing
/// `0x11223344` as a `u32` and loading byte 2 yields `0x22` little-endian; storing four
/// bytes and loading a `u32` yields the concatenation. **No cast node and no active-member
/// state appear in the CIR** — that is the whole point of 021 §3's "bytes are bytes", and
/// it is what lets VPP's packet code be analysed at all.
///
/// Contract 20 also requires **zero findings**: this is correct C under the default
/// checker set, and a memory model that reported it would be unusable on VPP.
#[test]
fn a_union_puns_in_both_directions_with_no_cast_node() {
    let mut caller = defined(
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
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Int {
                        bits: 32,
                        val: 0x1122_3344,
                    }),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
                // `&u.as_u8[2]`, reached by pointer arithmetic — no cast, no member state.
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 2 }),
                    },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(1)),
                        ty: CTy::Int(8),
                        align: 1,
                        vol: Volatility::Normal,
                    },
                }),
                // The reverse: four byte stores, then one 32-bit load.
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    for i in 0..4u32 {
        caller.blocks[0].insts.push(inst(InstKind::Assign {
            dst: ValueId(10 + i),
            rv: RValue::PtrAdd {
                base: Operand::Value(ValueId(3)),
                off: Operand::Const(Const::Int {
                    bits: 64,
                    val: i as i128,
                }),
            },
        }));
        caller.blocks[0].insts.push(inst(InstKind::Store {
            addr: Operand::Value(ValueId(10 + i)),
            val: Operand::Const(Const::Int {
                bits: 8,
                val: (0x44 - i * 0x11) as i128,
            }),
            ty: CTy::Int(8),
            align: 1,
            vol: Volatility::Normal,
        }));
    }
    caller.blocks[0].insts.push(inst(InstKind::Assign {
        dst: ValueId(20),
        rv: RValue::Load {
            addr: Operand::Value(ValueId(3)),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
    }));
    caller.allocas = vec![
        AllocaDecl {
            align: 4,
            ..alloca(0, CTy::Int(32), 1)
        },
        AllocaDecl {
            align: 4,
            ..alloca(1, CTy::Int(32), 1)
        },
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().is_empty(),
        "correct C under the default checker set: {:#?}",
        r.findings()
    );
    assert_eq!(r.fidelity(), Fidelity::Exact);
    let s = &r.states()[0];
    let bits = |v: u32, a: &mut TermArena| match s.local(ValueId(v)) {
        Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        bits(2, &mut a),
        Some(0x22),
        "byte 2 of 0x11223344, little-endian"
    );
    assert_eq!(
        bits(20, &mut a),
        Some(0x1122_3344),
        "and back the other way"
    );
}

/// **020 contract 26.** `LoadBits` with `signed: true` over a 3-bit field holding `0b111`
/// yields `-1` at `Int(32)`; with `signed: false` it yields `7`. A bitfield's high bit is
/// a sign bit, and reading `int x : 3` as 7 where C says -1 is a wrong answer with no
/// symptom — every comparison against it silently flips.
///
/// I implemented the sign extension and tested only the unsigned side, which is how this
/// stayed uncovered.
#[test]
fn a_signed_bitfield_sign_extends_and_an_unsigned_one_does_not() {
    for (signed, want) in [(true, 0xFFFF_FFFFu128), (false, 7)] {
        let mut caller = defined(
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
                    inst(InstKind::StoreBits {
                        addr: Operand::Value(ValueId(0)),
                        val: Operand::Const(Const::Int {
                            bits: 32,
                            val: 0b111,
                        }),
                        unit: CTy::Int(32),
                        bits: BitRange { off: 0, width: 3 },
                        align: 4,
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::LoadBits {
                            addr: Operand::Value(ValueId(0)),
                            unit: CTy::Int(32),
                            bits: BitRange { off: 0, width: 3 },
                            signed,
                            align: 4,
                        },
                    }),
                ],
                Terminator::Return(Some(Operand::Value(ValueId(1)))),
            )],
            CTy::Int(32),
        );
        caller.allocas = vec![AllocaDecl {
            align: 4,
            ..alloca(0, CTy::Int(32), 1)
        }];
        let m = Module {
            funcs: vec![caller],
            ..Default::default()
        };
        let mut a = TermArena::new();
        let r = Engine::new(&m).run(&mut a);
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(want),
            "signed: {signed}"
        );
    }
}

/// **020 contracts 24 and 25, and 021 contract 23.** `struct { u32 a:3; u32 b:5; }` — writing only `a` and
/// reading `a` produces no uninitialized-read finding, while reading `b` produces exactly
/// one. 020 names this as *the* contract byte-granular initialization cannot satisfy: both
/// fields live in the same byte, so a per-byte mask must answer wrongly for one of them.
///
/// Contract 25's half is that the write leaves every bit of `b` unchanged, which is the
/// same fact from the other side and is what makes the finding above trustworthy.
#[test]
fn writing_one_bitfield_leaves_its_neighbour_uninitialized() {
    let mut caller = defined(
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
                inst(InstKind::StoreBits {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::Int {
                        bits: 32,
                        val: 0b101,
                    }),
                    unit: CTy::Int(32),
                    bits: BitRange { off: 0, width: 3 },
                    align: 4,
                }),
                // `a` reads back with no finding...
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::LoadBits {
                        addr: Operand::Value(ValueId(0)),
                        unit: CTy::Int(32),
                        bits: BitRange { off: 0, width: 3 },
                        signed: false,
                        align: 4,
                    },
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller.clone()],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().is_empty(),
        "reading the field that was written is not a bug: {:#?}",
        r.findings()
    );
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(0b101));

    // ...and `b`, in the same byte, reads back as exactly one finding.
    let mut caller2 = caller;
    caller2.blocks[0].insts.pop();
    caller2.blocks[0].insts.push(inst(InstKind::Assign {
        dst: ValueId(1),
        rv: RValue::LoadBits {
            addr: Operand::Value(ValueId(0)),
            unit: CTy::Int(32),
            bits: BitRange { off: 3, width: 5 },
            signed: false,
            align: 4,
        },
    }));
    let m = Module {
        funcs: vec![caller2],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("uninitialized-read"))
            .count(),
        1,
        "{:#?}",
        r.findings()
    );
}

/// **020 contract 11.** An `Opaque` with a declared write of `[p, p+8)` invalidates
/// exactly those 8 bytes and leaves `[p+8, …)` intact, and marks the state
/// `Fidelity::Approximated`. The engine matched on `dsts` and `why` and ignored `writes`
/// entirely — so inline asm that declares it clobbers a buffer left chiero believing the
/// buffer was untouched, which is the same "a call chiero did not perform must invalidate
/// what it was handed" rule one construct over.
///
/// The "leaves the rest intact" half is what stops the fix being a blanket havoc: 020 §4.3
/// makes the *declaration* the point, and an `Opaque` that invalidated everything would be
/// no better than not modelling it.
#[test]
fn an_opaque_write_invalidates_exactly_what_it_declares() {
    let mut caller = defined(
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
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 0xAB }),
                    size: Operand::Const(Const::Int { bits: 64, val: 16 }),
                }),
                inst(InstKind::Opaque {
                    dsts: vec![],
                    writes: vec![OpaqueWrite {
                        addr: Operand::Value(ValueId(0)),
                        size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                    }],
                    reads: vec![],
                    why: OpaqueReason::InlineAsm,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(8), 16)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Approximated);
    let s = &r.states()[0];
    let base = match s.local(ValueId(0)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("{other:?}"),
    };
    let mut mem = s.mem.clone();
    // A concrete read of the clobbered range refuses it — the bytes are a known unknown
    // now. Comparing `.value` alone is not enough: 021 §5 hands back a value *alongside*
    // a fault, and the stale bytes are still what sits behind the overlay.
    let clobbered = mem.read(chiero_mem::Pointer { base, off: 0 }, 8, Span::DUMMY);
    assert!(
        clobbered.faults.iter().any(|f| f.kind() == "symbolic-byte"),
        "the declared region was invalidated: {:#?}",
        clobbered.faults
    );
    // **And the untouched half is still *readable*.** Comparing `.value` alone would pass
    // even if these bytes had been clobbered too, because the stale bytes sit behind the
    // overlay — the same trap as above, one assertion over.
    let intact = mem.read(chiero_mem::Pointer { base, off: 8 }, 8, Span::DUMMY);
    assert!(intact.faults.is_empty(), "{:#?}", intact.faults);
    assert_eq!(intact.value, Some(vec![0xAB; 8]), "and nothing past it was");
    // Each clobbered byte is its *own* unknown: one shared symbol would make
    // `buf[0] == buf[1]` provably true of memory chiero knows nothing about.
    let b0 = mem.read_term(
        &mut a,
        chiero_mem::Pointer { base, off: 0 },
        1,
        chiero_mem::Endian::Little,
        Span::DUMMY,
    );
    let b1 = mem.read_term(
        &mut a,
        chiero_mem::Pointer { base, off: 1 },
        1,
        chiero_mem::Endian::Little,
        Span::DUMMY,
    );
    assert_ne!(b0.value, b1.value, "two clobbered bytes are two unknowns");
}

/// **020 contract 31.** An `rdtsc`-shaped `Opaque` with two `dsts` yields two values, and
/// **two such instructions with identical operands yield four pairwise-distinct symbols**.
/// Two reads of the cycle counter are not the same number, and a pass that merged them
/// would turn `end - start` into a constant zero — which is exactly the measurement code
/// this construct exists for.
#[test]
fn two_opaque_instructions_yield_four_distinct_symbols() {
    let mk = |a: u32, b: u32| {
        inst(InstKind::Opaque {
            dsts: vec![(ValueId(a), CTy::Int(32)), (ValueId(b), CTy::Int(32))],
            writes: vec![],
            reads: vec![],
            why: OpaqueReason::InlineAsm,
        })
    };
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![mk(0, 1), mk(2, 3)],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    let vals: Vec<_> = (0..4).map(|v| s.local(ValueId(v))).collect();
    assert!(vals.iter().all(|v| v.is_some()), "{vals:?}");
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert_ne!(vals[i], vals[j], "%{i} and %{j} are the same symbol");
        }
    }
}

/// **020 contract 21.** Storing a *symbolic* `u32` through the union's `as_u32` view, then
/// overwriting `as_u8[1]` with a constant, then loading `as_u32` again, yields a term that
/// is a `Concat` of three extracted symbolic bytes and the constant — not a fresh symbol,
/// and not a concretization.
///
/// This is the case that decides whether partial overwrites of symbolic data are usable at
/// all: VPP rewrites one byte of a packet header constantly, and a model that answered
/// "the whole word is now unknown" would lose every constraint on the other three bytes.
#[test]
fn a_constant_overwrites_one_byte_of_a_symbolic_word() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                    align: 4,
                    vol: Volatility::Normal,
                }),
                // `u.as_u8[1] = 0xEE`
                inst(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: 1 }),
                    },
                }),
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(2)),
                    val: Operand::Const(Const::Int { bits: 8, val: 0xEE }),
                    ty: CTy::Int(8),
                    align: 1,
                    vol: Volatility::Normal,
                }),
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                }),
                // Byte 1 is now concrete...
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(2)),
                        ty: CTy::Int(8),
                        align: 1,
                        vol: Volatility::Normal,
                    },
                }),
                // ...and the word still depends on the original symbol: `w ^ x` has the
                // overwritten byte's bits set from the constant and zeroes elsewhere only
                // if the other three bytes came through unchanged.
                inst(InstKind::Assign {
                    dst: ValueId(5),
                    rv: RValue::Bin {
                        op: BinOp::Xor,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(3)),
                        b: Operand::Value(ValueId(1)),
                    },
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 4,
        ..alloca(0, CTy::Int(32), 1)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "a partial overwrite is exact: {:#?}",
        r.states()[0].assumptions()
    );
    let s = &r.states()[0];
    // The overwritten byte reads back concrete.
    match s.local(ValueId(4)) {
        Some(Value::Scalar(t)) => {
            assert_eq!(a.eval_ground(t).ok().map(|c| c.bits()), Some(0xEE))
        }
        other => panic!("{other:?}"),
    }
    // **The other three bytes are still the symbol.** Under every assignment of `x`, the
    // xor's bytes 0, 2 and 3 are zero — which is only true if they survived the overwrite
    // as *the same term*, not as a fresh unknown.
    let (Some(Value::Scalar(x)), Some(Value::Scalar(diff))) =
        (s.local(ValueId(1)), s.local(ValueId(5)))
    else {
        panic!("expected scalars");
    };
    for probe in [0u128, 0x1234_5678, u32::MAX as u128] {
        let mut model = chiero_solver::Model::new();
        model.set(
            a.var_id(x).expect("fresh is a variable"),
            chiero_solver::BvConst::new(32, probe),
        );
        let v = a.eval(&model, diff).expect("ground under the model").bits();
        assert_eq!(
            v & 0xFFFF_00FF,
            0,
            "bytes 0, 2 and 3 came through untouched, for x = {probe:#x} (diff = {v:#x})"
        );
        // And byte 1 is exactly the difference between the constant and what was there,
        // so "unchanged everywhere" is not satisfied by the write having been dropped.
        assert_eq!(
            (v >> 8) & 0xFF,
            0xEE ^ ((probe >> 8) & 0xFF),
            "byte 1 is the constant, for x = {probe:#x}"
        );
    }
}

/// **020 contract 38.** `AllocaDyn` inside a loop body executed three times creates three
/// **distinct** objects. C's `alloca` accumulates until function return, so reusing one
/// object would make the second iteration alias the first — writes aliasing and lifetimes
/// wrong at once, and a use-after-free that never fires because the memory is still there.
#[test]
fn alloca_dyn_in_a_loop_creates_a_distinct_object_each_time() {
    let mut caller = defined(
        0,
        "main",
        vec![
            block(
                0,
                vec![inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Use(i32c(0)),
                })],
                Terminator::Goto(BlockId(1)),
            ),
            block(
                1,
                vec![inst(InstKind::AllocaDyn {
                    dst: ValueId(1),
                    alloca: AllocaId(0),
                    elem: CTy::Int(8),
                    count: Operand::Const(Const::Int { bits: 64, val: 8 }),
                    align: 8,
                })],
                Terminator::Goto(BlockId(1)),
            ),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        count: chiero_cir::DYNAMIC_EXTENT,
        ..alloca(0, CTy::Int(8), 0)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 3,
            ..Budget::default()
        })
        .run(&mut a);
    // Every allocation is its own object, so the count of live stack objects grows with
    // the iterations rather than staying at one.
    let s = &r.states()[0];
    let objects = s.mem.object_count_for_test();
    assert!(
        objects >= 3,
        "three iterations, three objects, got {objects}"
    );
}

/// **020 contract 35.** `Bitcast` between `Int(128)` and `Vector { Int(32), 4 }` verifies;
/// between `Int(32)` and `Vector { Int(8), 16 }` it is rejected. Verifier rule 12 is that
/// a bitcast preserves total width — 021 §3's "bytes are bytes" only holds if the bytes
/// are the *same* bytes, and a cast that changed their number would be a reinterpretation
/// of memory the program never wrote.
#[test]
fn a_bitcast_preserves_total_width() {
    for (from, to, ok) in [
        (
            CTy::Int(128),
            CTy::Vector {
                elem: Box::new(CTy::Int(32)),
                lanes: 4,
            },
            true,
        ),
        (
            CTy::Int(32),
            CTy::Vector {
                elem: Box::new(CTy::Int(8)),
                lanes: 16,
            },
            false,
        ),
    ] {
        let caller = defined(
            0,
            "main",
            vec![block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: from.clone() },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cast {
                            kind: CastKind::Bitcast,
                            a: Operand::Value(ValueId(0)),
                            from: from.clone(),
                            to: to.clone(),
                        },
                    }),
                ],
                Terminator::Return(Some(i32c(0))),
            )],
            CTy::Int(32),
        );
        let m = Module {
            funcs: vec![caller],
            ..Default::default()
        };
        let errs = chiero_cir::verify(&m);
        assert_eq!(
            errs.iter().all(|e| !e.kind.is_error()),
            ok,
            "{from:?} -> {to:?}: {errs:#?}"
        );
    }
}

/// **An `Opaque` that declares a clobber wider than the object reports the overflow.** The
/// faults from the byte writes were discarded, so inline asm announcing it writes sixteen
/// bytes of an eight-byte buffer was a buffer overflow chiero *detected and did not
/// report* — it only degraded fidelity, which reads as "chiero was unsure" rather than
/// "your program is wrong". Found by review.
#[test]
fn an_opaque_write_past_the_end_is_a_finding() {
    let mut caller = defined(
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
                inst(InstKind::Opaque {
                    dsts: vec![],
                    writes: vec![OpaqueWrite {
                        addr: Operand::Value(ValueId(0)),
                        size: Operand::Const(Const::Int { bits: 64, val: 16 }),
                    }],
                    reads: vec![],
                    why: OpaqueReason::InlineAsm,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![AllocaDecl {
        align: 8,
        ..alloca(0, CTy::Int(8), 8)
    }];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings().iter().any(|f| f.contains("out-of-bounds")),
        "the program declared the overflow itself: {:#?}",
        r.findings()
    );
}

/// **020 contract 9's second half, for real.** The contract requires each division result
/// to agree with *the solver's* evaluation of the same term, so the IR and solver
/// conventions cannot drift. Doing that over constant operands proves nothing: `bin` folds
/// at construction, so there is one value computed once and read back twice.
///
/// Here the dividend is symbolic and constrained by a path condition, so the term survives
/// as a live `Node::Bin` and `eval` must *interpret* it — a genuinely separate execution of
/// the same table. Found by review, which showed the constant version could be deleted with
/// no test failing.
#[test]
fn the_solver_evaluates_division_by_zero_the_same_way_the_engine_does() {
    let mut a = TermArena::new();
    // `x` symbolic, pinned to -5 by a model rather than by folding.
    let x = a.var(chiero_solver::Sort::BitVec(32), "x");
    let zero = a.bv(32, 0);
    let cases: [(&str, chiero_solver::Term, u128); 4] = [
        ("udiv", a.udiv(x, zero), u32::MAX as u128),
        ("sdiv", a.sdiv(x, zero), 1),
        ("urem", a.urem(x, zero), (-5i32) as u32 as u128),
        ("srem", a.srem(x, zero), (-5i32) as u32 as u128),
    ];
    let mut model = chiero_solver::Model::new();
    model.set(
        a.var_id(x).expect("a variable"),
        chiero_solver::BvConst::new(32, (-5i32) as u32 as u128),
    );
    for (what, t, want) in cases {
        assert!(
            a.eval_ground(t).is_err(),
            "{what}: the term must not be folded, or this proves nothing"
        );
        assert_eq!(
            a.eval(&model, t).expect("evaluable under the model").bits(),
            want,
            "{what} of -5 by zero"
        );
    }
}

/// **020 contract 32's execution half, and E5: every declared write is honoured.** An
/// `Opaque` with a declared write *and* a `dst` both havocs the region and produces the
/// value — "dropping either half fails the test", which is only true if the test exercises
/// both. A parser test cites contract 32 and honestly labels itself "the representational
/// half"; nothing ran it.
///
/// Every `OpaqueWrite` fixture in this file had exactly **one** entry, so `writes.take(1)`
/// survived: "each declared write is honoured" was untested. Two regions, two checks.
#[test]
fn an_opaque_honours_every_declared_write_and_still_produces_its_value() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(1),
                    },
                }),
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(0)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 0xAB }),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                }),
                inst(InstKind::SetMem {
                    dst: Operand::Value(ValueId(1)),
                    byte: Operand::Const(Const::Int { bits: 8, val: 0xCD }),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                }),
                inst(InstKind::Opaque {
                    dsts: vec![(ValueId(2), CTy::Int(32))],
                    writes: vec![
                        OpaqueWrite {
                            addr: Operand::Value(ValueId(0)),
                            size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                        },
                        OpaqueWrite {
                            addr: Operand::Value(ValueId(1)),
                            size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                        },
                    ],
                    reads: vec![],
                    why: OpaqueReason::InlineAsm,
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![
        AllocaDecl {
            align: 8,
            ..alloca(0, CTy::Int(8), 8)
        },
        AllocaDecl {
            align: 8,
            ..alloca(1, CTy::Int(8), 8)
        },
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    // The `dst` half: a value came out.
    assert!(
        matches!(s.local(ValueId(2)), Some(Value::Scalar(_))),
        "the output was produced: {:?}",
        s.local(ValueId(2))
    );
    // The write half, for *both* declared regions.
    let mut mem = s.mem.clone();
    for v in [0u32, 1] {
        let base = match s.local(ValueId(v)) {
            Some(Value::Ptr(p)) => p.base,
            other => panic!("{other:?}"),
        };
        let read = mem.read(chiero_mem::Pointer { base, off: 0 }, 8, Span::DUMMY);
        assert!(
            read.faults.iter().any(|f| f.kind() == "symbolic-byte"),
            "region {v} was declared clobbered: {:#?}",
            read.faults
        );
    }
}

/// **021 contract 32.** A function-pointer value stored to a global, loaded back, and
/// called indirectly resolves to exactly **one** `FuncId` with **no fork** — the
/// `ObjKind::Function` contract. This is VPP's registration-table shape end to end: a
/// handler is written into a table at init, read back at dispatch, and called.
///
/// Without it the call forks over every defined function plus an unresolvable state, so a
/// table-driven dispatch becomes unanalysable exactly where it matters most.
#[test]
fn a_function_pointer_through_a_global_resolves_without_forking() {
    let caller = defined(
        0,
        "main",
        vec![block(
            0,
            vec![
                inst(InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfGlobal { g: GlobalId(0) },
                }),
                // The table entry is written at "init"...
                inst(InstKind::Store {
                    addr: Operand::Value(ValueId(0)),
                    val: Operand::Const(Const::FuncAddr(FuncId(1))),
                    ty: CTy::Ptr,
                    align: 8,
                    vol: Volatility::Normal,
                }),
                // ...and read back at "dispatch".
                inst(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Ptr,
                        align: 8,
                        vol: Volatility::Normal,
                    },
                }),
                inst(InstKind::Call {
                    dst: Some(ValueId(2)),
                    callee: Callee::Indirect(Operand::Value(ValueId(1))),
                    args: vec![],
                }),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
        CTy::Int(32),
    );
    let handler = defined(
        1,
        "handler",
        vec![block(0, vec![], Terminator::Return(Some(i32c(42))))],
        CTy::Int(32),
    );
    let decoy = defined(
        2,
        "decoy",
        vec![block(0, vec![], Terminator::Return(Some(i32c(7))))],
        CTy::Int(32),
    );
    let m = Module {
        funcs: vec![caller, handler, decoy],
        globals: vec![Global {
            id: GlobalId(0),
            name: "table".into(),
            size: 8,
            align: 8,
            is_const: false,
            span: Span::DUMMY,
        }],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.states().len(), 1, "no fork: the pointer is known");
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(42),
        "and it is the handler, not the decoy"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "{:#?}",
        r.states()[0].assumptions()
    );
}

/// **021 §5.1: an unresolved pointer stops the path, and says *why* it was unresolved.**
///
/// The reason matters more than the fidelity here. §5.1 step 4 — the value is *wholly
/// unconstrained*, so every object and being nowhere are all feasible — is a statement
/// about the **program**. `solver-lite` returning `Unknown` for the range queries is a
/// statement about **chiero**. Both end the path at `Fidelity::Unknown`, and only the
/// reason tells a reader whether to strengthen the program or the solver.
///
/// ⚠️ **With the current tier, step 4 is not reachable**: `solver-lite`'s §3.2 fragment
/// does not decide `addr ∈ [base, base+size]` over a variable, so every case arrives as
/// `SolverUnknown`. An earlier version of this test asserted the step-4 text and passed —
/// for the wrong reason, on a run whose real cause was the solver. Step 4's own detection
/// is owed a tier that can answer, and is *not* claimed as covered.
#[test]
fn an_unresolvable_pointer_stops_the_path_and_names_its_reason() {
    let mut caller = defined(
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
                inst(InstKind::Assign {
                    dst: ValueId(3),
                    rv: RValue::Fresh { ty: CTy::Int(64) },
                }),
                inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(3)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                }),
                // Never reached: the path ended at the unresolvable pointer.
                inst(InstKind::Assign {
                    dst: ValueId(5),
                    rv: RValue::Use(i32c(1)),
                }),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    caller.allocas = vec![alloca(0, CTy::Int(8), 8)];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Unknown, "nothing was resolved");
    assert!(
        r.states()[0].local(ValueId(5)).is_none(),
        "the path stopped rather than continuing with a guess"
    );
    // **Not a concretization.** Step 5 would produce a pointer and say `Approximated`;
    // neither of the unresolved paths produces one.
    assert!(r.states()[0].local(ValueId(4)).is_none());
    assert_ne!(r.fidelity(), Fidelity::Approximated);
    // And the finding names a cause a reader can act on.
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("solver did not decide") || f.contains("value is unconstrained")),
        "{:#?}",
        r.findings()
    );
}

/// **021 contract 16, with a solver that can answer.** A symbolic base constrained by a
/// real path condition forks into one state per candidate **plus one wild state**. This is
/// §5.1 step 3, and it needs tier 2: `solver-lite`'s §3.2 fragment does not decide
/// `addr ∈ [base, base+size]` over a variable, which is why the previous test can only
/// reach `SolverUnknown`.
///
/// 022 contract 2 requires the suite to run with z3 **absent**, so this skips with a
/// printed reason rather than failing — a skipped test that says why is honest; one that
/// passes silently is not.
#[test]
fn a_constrained_symbolic_base_forks_per_candidate_plus_one_wild() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let mut caller = defined(
        0,
        "main",
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(3),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cast {
                            kind: CastKind::PtrToInt,
                            a: Operand::Value(ValueId(0)),
                            from: CTy::Ptr,
                            to: CTy::Int(64),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Fresh { ty: CTy::Int(64) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(3),
                        rv: RValue::Cmp {
                            op: CmpOp::ULt,
                            ty: CTy::Int(64),
                            a: Operand::Value(ValueId(2)),
                            b: Operand::Value(ValueId(1)),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(2)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                })],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(1)))),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![
        alloca(0, CTy::Int(8), 8),
        alloca(1, CTy::Int(8), 8),
        alloca(2, CTy::Int(8), 8),
        alloca(3, CTy::Int(8), 8),
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let resolved: Vec<_> = r
        .states()
        .iter()
        .filter_map(|s| match s.local(ValueId(4)) {
            Some(Value::Ptr(p)) => Some(p.base),
            _ => None,
        })
        .collect();
    assert!(
        resolved.len() >= 2,
        "the base resolved to several objects: {resolved:?} findings={:#?}",
        r.findings()
    );
    // **The wild state is present**, or the candidate list is implicitly exhaustive and an
    // address outside every object is never explored.
    assert!(
        resolved.contains(&chiero_mem::ObjectId::UNBOUND),
        "one state says the pointer may be nowhere: {resolved:?}"
    );
    // Distinct objects, not one repeated.
    let mut uniq = resolved.clone();
    uniq.sort_by_key(|o| o.0);
    uniq.dedup();
    assert_eq!(uniq.len(), resolved.len(), "{resolved:?}");
}

/// **021 §5.1: "the solver could not tell" is not "the program said nothing".** A base
/// constrained by a real path condition — a branch on `addr < &d` — is not resolvable by
/// `solver-lite`, whose §3.2 fragment does not cover this shape. That must be reported as
/// a *solver* limit, with `SolverUnknown`, and not as 021 §5.1 step 4's unconstrained
/// pointer.
///
/// Folding the two together is the same mistake 021 records against an earlier draft, one
/// level up: it would blame the program for what the tier could not see, and only the
/// reason tells a reader whether to strengthen the program or the solver. I made exactly
/// that mistake writing §5.1 and this is the test that caught it.
///
/// **Contract 16 itself — the fork into candidates plus one wild — is still owed**, and
/// needs a solver tier that can decide these queries. The fork machinery exists and this
/// fixture cannot reach it.
#[test]
fn an_undecided_pointer_blames_the_solver_not_the_program() {
    let mut caller = defined(
        0,
        "main",
        vec![
            block(
                0,
                vec![
                    inst(InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(3),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cast {
                            kind: CastKind::PtrToInt,
                            a: Operand::Value(ValueId(0)),
                            from: CTy::Ptr,
                            to: CTy::Int(64),
                        },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Fresh { ty: CTy::Int(64) },
                    }),
                    inst(InstKind::Assign {
                        dst: ValueId(3),
                        rv: RValue::Cmp {
                            op: CmpOp::ULt,
                            ty: CTy::Int(64),
                            a: Operand::Value(ValueId(2)),
                            b: Operand::Value(ValueId(1)),
                        },
                    }),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(InstKind::Assign {
                    dst: ValueId(4),
                    rv: RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(2)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                })],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(1)))),
        ],
        CTy::Int(32),
    );
    caller.allocas = vec![
        alloca(0, CTy::Int(8), 8),
        alloca(1, CTy::Int(8), 8),
        alloca(2, CTy::Int(8), 8),
        alloca(3, CTy::Int(8), 8),
    ];
    let m = Module {
        funcs: vec![caller],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.findings()
            .iter()
            .any(|f| f.contains("the solver did not decide")),
        "the reason names the solver: {:#?}",
        r.findings()
    );
    assert!(
        !r.findings()
            .iter()
            .any(|f| f.contains("the value is unconstrained")),
        "and does not blame the program: {:#?}",
        r.findings()
    );
    assert!(
        r.states().iter().any(|s| s
            .assumptions()
            .iter()
            .any(|x| x.kind == AssumptionKind::SolverUnknown)),
        "{:#?}",
        r.states()[0].assumptions()
    );
}
