//! Covers: 020 §9's `Phi`, executed.
//!
//! A phi's whole meaning is **which incoming it picks**, and that is the one thing no
//! structural test can see. `chiero-opt`'s transparency sweep runs a module against
//! itself, so an engine that always took the first incoming would be wrong identically on
//! both sides and the comparison would agree. The rules in `chiero-cir/tests/phi.rs` check
//! that a phi is *well formed*; these check that executing one produces the value
//! belonging to the edge that was actually taken.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn inst(kind: InstKind, lo: u32) -> Inst {
    Inst {
        kind,
        span: at(lo),
        generated: true,
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

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

/// A diamond whose condition is the **constant** `cond`, so exactly one edge is taken and
/// the run is ground — no solver, no fork, one state with one answer.
fn diamond(cond: i128) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks: vec![
                block(
                    0,
                    vec![],
                    Terminator::Br {
                        cond: Operand::Const(Const::Int { bits: 1, val: cond }),
                        t: BlockId(1),
                        f: BlockId(2),
                    },
                ),
                block(1, vec![], Terminator::Goto(BlockId(3))),
                block(2, vec![], Terminator::Goto(BlockId(3))),
                block(
                    3,
                    vec![inst(
                        InstKind::Phi {
                            dst: ValueId(9),
                            ty: CTy::Int(32),
                            incomings: vec![(BlockId(1), i32c(11)), (BlockId(2), i32c(22))],
                        },
                        50,
                    )],
                    Terminator::Return(Some(Operand::Value(ValueId(9)))),
                ),
            ],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    }
}

fn returned(m: &Module) -> Option<u128> {
    let mut a = TermArena::new();
    let r = Engine::new(m).run(&mut a);
    assert_eq!(r.states().len(), 1, "a constant condition takes one edge");
    r.states()[0].return_value_bits(&mut a)
}

/// **The phi takes the incoming belonging to the edge actually taken.**
///
/// Both directions in one test, because either alone is passed by an engine that always
/// picks the same one: taking the first incoming is right for the true arm and wrong for
/// the false arm, and taking the last is the reverse.
#[test]
fn a_phi_yields_the_value_of_the_edge_that_was_taken() {
    assert_eq!(
        returned(&diamond(1)),
        Some(11),
        "the true arm reaches the join from bb1, so the phi is 11"
    );
    assert_eq!(
        returned(&diamond(0)),
        Some(22),
        "and the false arm from bb2, so it is 22 — an engine taking the first incoming \
         reports 11 here"
    );
}

/// **A loop phi picks the preheader's value on entry and the latch's afterwards.**
///
/// The diamond above never asks the phi to choose *the same* incoming twice, and a loop
/// does: the header is entered once from outside and then repeatedly from itself. An
/// engine that recorded the predecessor only on the first entry — or that recorded it
/// after moving the program counter — returns the preheader's value forever, and the loop
/// silently computes nothing.
#[test]
fn a_loop_phi_advances_with_the_latch() {
    // i = 0; do { i = i + 1; } while (i < 3); return i;
    let m = Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "count".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks: vec![
                block(0, vec![], Terminator::Goto(BlockId(1))),
                block(
                    1,
                    vec![
                        inst(
                            InstKind::Phi {
                                dst: ValueId(1),
                                ty: CTy::Int(32),
                                incomings: vec![
                                    (BlockId(0), i32c(0)),
                                    (BlockId(1), Operand::Value(ValueId(2))),
                                ],
                            },
                            20,
                        ),
                        inst(
                            InstKind::Assign {
                                dst: ValueId(2),
                                rv: RValue::Bin {
                                    op: BinOp::Add,
                                    a: Operand::Value(ValueId(1)),
                                    b: i32c(1),
                                    ty: CTy::Int(32),
                                },
                            },
                            21,
                        ),
                        inst(
                            InstKind::Assign {
                                dst: ValueId(3),
                                rv: RValue::Cmp {
                                    op: CmpOp::SLt,
                                    a: Operand::Value(ValueId(2)),
                                    b: i32c(3),
                                    ty: CTy::Int(32),
                                },
                            },
                            22,
                        ),
                    ],
                    Terminator::Br {
                        cond: Operand::Value(ValueId(3)),
                        t: BlockId(1),
                        f: BlockId(2),
                    },
                ),
                block(
                    2,
                    vec![],
                    Terminator::Return(Some(Operand::Value(ValueId(2)))),
                ),
            ],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    };
    assert!(
        chiero_cir::verify::verify(&m).iter().all(|e| !e.is_error()),
        "{:#?}",
        chiero_cir::verify::verify(&m)
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 8,
            ..Budget::default()
        })
        .run(&mut a);
    let got = r.states()[0].return_value_bits(&mut a);
    assert_eq!(
        got,
        Some(3),
        "three iterations: the phi is 0, then 1, then 2 — an engine that kept the \
         preheader's incoming would loop on 0 forever and return 1"
    );
}

/// A phi's `dst` is an ordinary value afterwards: it can be read by later instructions in
/// the same block, which is what makes the loop above compute anything at all.
#[test]
fn a_phis_result_is_readable_by_the_rest_of_the_block() {
    let m = diamond(1);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    match r.states()[0].local(ValueId(9)) {
        Some(Value::Scalar(t)) => assert_eq!(a.as_const(t).map(|c| c.bits()), Some(11)),
        other => panic!("the phi defined no local: {other:?}"),
    }
}
