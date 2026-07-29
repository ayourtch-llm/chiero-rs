//! Covers: 020 §4.1 — **a UB event is observed by a checker and reported**.
//!
//! §4.1 is explicit that the engine's job stops at the event: "CIR is not the place to
//! encode UB as unpredictability: the semantics are defined and total, and a `Checker`
//! observes the overflow event and reports it." The engine holds up its half — every
//! division by zero, over-wide shift and signed overflow it can see becomes a `UbEvent` on
//! the state, and wave 156 taught it to ask the solver about a symbolic divisor as well.
//!
//! Nothing reads them. `default_checkers()` ships one checker and it watches sequence
//! points. So a run over a program that divides by zero finishes with the event recorded,
//! `reports()` empty, and nothing a caller of the library would ever see — which is the
//! same shape as the defect wave 156 fixed one layer down, and for the same reason: the
//! information exists and no one is asked for it.
//!
//! **Fixtures are CIR**, as in `order_dependence.rs`: 001 §4 rule 7 forbids this crate a
//! frontend dependency, and the checker's input is a `UbEvent` on a state, so a hand-built
//! module is the thing under test rather than a C program that happens to lower to one.

use chiero_cir::*;
use chiero_exec::Engine;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn inst(kind: InstKind, lo: u32) -> Inst {
    Inst {
        kind,
        span: at(lo),
        generated: false,
    }
}

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: Default::default(),
        span: at(1),
    }
}

fn k(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn module(blocks: Vec<Block>) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    }
}

/// Run with the checkers 040 §1 turns on unless told otherwise.
fn findings(m: &Module) -> Vec<String> {
    let mut a = TermArena::new();
    let mut e = Engine::new(m);
    for c in chiero_check::default_checkers() {
        e = e.with_checker(c);
    }
    e.run(&mut a)
        .reports()
        .into_iter()
        .map(|f| f.message)
        .collect()
}

/// **A division by zero is a finding, not just an event.**
///
/// The straight-line case, and the one that says the wiring exists at all. `100 / 0` is
/// undefined (C11 6.5.5p5), the engine records it, and a caller asking for the default
/// checkers gets nothing.
#[test]
fn a_division_by_zero_is_reported() {
    let m = module(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op: BinOp::SDiv,
                        ty: CTy::Int(32),
                        a: k(100),
                        b: k(0),
                    },
                },
                10,
            ),
            // Instructions after the fault. §4.1 says the path continues, so a checker
            // that reports on "the state has a UB event" rather than on *this* event
            // reports once per instruction from here on.
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Use(k(1)),
                },
                20,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Use(k(2)),
                },
                30,
            ),
        ],
        Terminator::Return(Some(Operand::Value(ValueId(0)))),
    )]);
    let f = findings(&m);
    assert_eq!(
        f.len(),
        1,
        "one division by zero is one finding, whatever runs after it: {f:?}"
    );
    assert!(
        f[0].to_lowercase().contains("zero"),
        "the finding should name the fault: {f:?}"
    );
}

/// **And an operation with no UB is not reported.**
///
/// The negative half, and the one that makes the positive half worth anything: a checker
/// that reported on every arithmetic instruction would satisfy the test above.
#[test]
fn arithmetic_without_undefined_behaviour_is_not_reported() {
    let m = module(vec![block(
        0,
        vec![inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: k(5),
                },
            },
            10,
        )],
        Terminator::Return(Some(Operand::Value(ValueId(0)))),
    )]);
    assert!(
        findings(&m).is_empty(),
        "100 / 5 is defined: {:?}",
        findings(&m)
    );
}

/// **Every UB kind the engine records reaches a report**, not only division.
///
/// The engine's table has three rows and they share one code path, so a checker matching on
/// `DivByZero` alone would pass the first test and silently drop the other two.
#[test]
fn shifts_and_overflow_are_reported_too() {
    for (op, a_val, b_val, want) in [
        (BinOp::Shl, 1i128, 33i128, "shift"),
        (BinOp::Add, i128::from(i32::MAX), 1, "overflow"),
    ] {
        let m = module(vec![block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op,
                        ty: CTy::Int(32),
                        a: k(a_val),
                        b: k(b_val),
                    },
                },
                10,
            )],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )]);
        let f = findings(&m);
        assert_eq!(f.len(), 1, "{op:?}: expected exactly one finding: {f:?}");
        assert!(
            f[0].to_lowercase().contains(want),
            "{op:?}: the finding should name the fault: {f:?}"
        );
    }
}

/// **One site reached twice on one path is one finding** (023 §6.1).
///
/// A loop runs the same division repeatedly and it is one bug. The engine's own
/// deduplication cannot help here: `Action::Report` carries no §6.1 key, so `reports()`
/// deduplicates a *fork's* copies by id and leaves everything else to the checker — which
/// is why the checker needs per-state memory rather than a counter.
#[test]
fn one_faulting_site_in_a_loop_is_one_finding() {
    // Block 0 divides and jumps to 1; block 1 divides at the *same span* and returns.
    // Two executions of one source site is what a loop looks like to a checker.
    let divide = |dst: u32| {
        inst(
            InstKind::Assign {
                dst: ValueId(dst),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: k(0),
                },
            },
            10,
        )
    };
    let m = module(vec![
        block(0, vec![divide(0)], Terminator::Goto(BlockId(1))),
        block(
            1,
            vec![divide(1)],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        ),
    ]);
    let f = findings(&m);
    assert_eq!(
        f.len(),
        1,
        "the same division at the same place, twice on one path, is one bug: {f:?}"
    );
}

/// **A site already reported before a fork is not reported again on either child**
/// (023 §6.1).
///
/// The engine deduplicates a fork's copies of one report by id, and measurement says that
/// is what carries this case: both children finish holding the *same* pre-fork report and
/// `reports()` collapses them. The children do not re-report at the second site.
///
/// So this fixture pins the observable property — one site, one finding, across a fork —
/// **without** reaching `on_fork`'s copy of `reported`. That copy is currently unobservable:
/// a mutation emptying it survives every test here. Recorded in §9 rather than deleted,
/// because the field is right and the missing fixture is a statement about the suite.
#[test]
fn a_site_reported_before_a_fork_is_not_reported_again_after_it() {
    let divide = |dst: u32| {
        inst(
            InstKind::Assign {
                dst: ValueId(dst),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: k(0),
                },
            },
            10,
        )
    };
    let m = module(vec![
        // Fault, then branch on something symbolic so both sides are explored.
        block(
            0,
            vec![
                divide(0),
                inst(
                    InstKind::Assign {
                        dst: ValueId(5),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    },
                    20,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(6),
                        rv: RValue::Cmp {
                            op: CmpOp::Eq,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(5)),
                            b: k(0),
                        },
                    },
                    21,
                ),
            ],
            Terminator::Br {
                cond: Operand::Value(ValueId(6)),
                t: BlockId(1),
                f: BlockId(2),
            },
        ),
        block(1, vec![], Terminator::Goto(BlockId(3))),
        block(2, vec![], Terminator::Goto(BlockId(3))),
        // Both children reach the same faulting site again.
        block(
            3,
            vec![divide(1)],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        ),
    ]);
    let f = findings(&m);
    assert_eq!(
        f.len(),
        1,
        "the site was reported before the fork; neither child should say it again: {f:?}"
    );
}
