//! **An overflow the path forces is invisible, and the run calls itself `Exact`.**
//!
//! `note_ub` needs both operands concrete: anything else falls through to
//! `symbolic_div_by_zero` and returns. So a program with real inputs produces no arithmetic UB
//! event at all — not a weaker one, none — and `fidelity` says `Exact`:
//!
//! ```text
//!   x = Fresh; if (x > 2147483640) return x + 10;    Exact, no findings
//! ```
//!
//! On that path `x` is one of seven values and `x + 10` overflows for every one of them. There is
//! nothing probabilistic about it and no input to argue over.
//!
//! Wave 174 planned this and wave 175 deliberately did not do it: the premise then was that the
//! census's remaining gap was symbolic operands, probing showed the generated programs are closed
//! and every value in them concrete, and the real cause was `sext` not folding. It has been owed
//! since, for exactly the case here — a program with inputs.
//!
//! # The half this file does not ask for
//!
//! §9 records the open question: with unconstrained inputs *every* `x + y` can overflow, so
//! reporting on satisfiability alone would report on every arithmetic instruction in the program.
//! The codebase already has a precedent pointing the other way — wave 156's
//! `symbolic_div_by_zero` reports on `Sat` and carries the condition for the witness — and
//! division by zero needs the divisor to be one specific value where overflow needs only a large
//! one, so the precedent does not obviously transfer.
//!
//! **So this file asks only for the forced case**, which is a definite bug under any reading, and
//! `an_overflow_the_path_merely_admits_is_not_reported` pins today's silence for the other so the
//! decision stays open rather than being made by accident.

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
            linkage: chiero_cir::Linkage::External,
        }],
        ..Default::default()
    }
}

fn findings(m: &Module) -> Vec<String> {
    let mut a = TermArena::new();
    let mut e = Engine::new(m);
    for c in chiero_check::default_checkers() {
        e = e.with_checker(c);
    }
    e.run(&mut a).findings()
}

/// `x = Fresh`, then `cmp x, bound` and a branch; the true block does `x op rhs`.
fn guarded(op: BinOp, cmp: CmpOp, bound: i128, rhs: i128) -> Module {
    module(vec![
        block(
            0,
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    },
                    5,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cmp {
                            op: cmp,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(0)),
                            b: k(bound),
                        },
                    },
                    6,
                ),
            ],
            Terminator::Br {
                cond: Operand::Value(ValueId(1)),
                t: BlockId(1),
                f: BlockId(2),
            },
        ),
        block(
            1,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Bin {
                        op,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(0)),
                        b: k(rhs),
                        signed: true,
                    },
                },
                10,
            )],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        ),
        block(2, vec![], Terminator::Return(Some(k(0)))),
    ])
}

/// **An overflow every value the path admits produces.**
///
/// All three signed operations C11 6.5p5 makes undefined on overflow, because a fix for `Add`
/// alone is the shape this project keeps finding: one arm guarded and not its siblings.
#[test]
fn an_overflow_the_path_forces_is_reported() {
    for (what, m) in [
        // x > 2147483640, so x is one of seven values and every one of them overflows at +10.
        ("Add", guarded(BinOp::Add, CmpOp::SGt, 2_147_483_640, 10)),
        // x < -2147483640, and subtracting 10 leaves the range for all of them.
        ("Sub", guarded(BinOp::Sub, CmpOp::SLt, -2_147_483_640, 10)),
        // x > 1073741824, so 2 * x is past the maximum whatever x is.
        ("Mul", guarded(BinOp::Mul, CmpOp::SGt, 1_073_741_824, 2)),
    ] {
        let f = findings(&m);
        assert!(
            f.iter().any(|s| s.starts_with("signed-overflow")),
            "`{what}`: every value this path admits overflows, which is a definite fault: {f:?}"
        );
    }
}

/// An overflow the path merely **admits** is not reported. **The decision, pinned open.**
///
/// `x + 1` with an unconstrained `x` overflows for exactly one value of `x` and is fine for the
/// other four billion. Reporting it is defensible — wave 156 reports a symbolic divisor on
/// satisfiability and names the witness — and it would put a finding on every addition in every
/// program that takes an input, which is a product decision rather than a correctness one.
///
/// This test does not say the silence is right. It says the silence is *deliberate*, so that
/// changing it is a decision someone makes rather than a side effect of the query above.
#[test]
fn an_overflow_the_path_merely_admits_is_not_reported() {
    let m = module(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                },
                5,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin {
                        op: BinOp::Add,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(0)),
                        b: k(1),
                        signed: true,
                    },
                },
                10,
            ),
        ],
        Terminator::Return(Some(Operand::Value(ValueId(1)))),
    )]);
    let f = findings(&m);
    assert!(
        f.iter().all(|s| !s.contains("overflow")),
        "one overflowing input out of four billion is not yet a finding chiero makes: {f:?}"
    );
}

/// A path that forbids the overflow says nothing. **The control.**
///
/// The same shape as the forced fixtures with the comparison the other way round, so the guard is
/// doing the work rather than the presence of a symbolic operand. Without this a query that
/// reported on every symbolic addition would pass everything above.
#[test]
fn an_overflow_the_path_forbids_is_not_reported() {
    for (what, m) in [
        ("Add", guarded(BinOp::Add, CmpOp::SLt, 1000, 10)),
        ("Sub", guarded(BinOp::Sub, CmpOp::SGt, -1000, 10)),
        ("Mul", guarded(BinOp::Mul, CmpOp::SLt, 1000, 2)),
    ] {
        let f = findings(&m);
        assert!(
            f.iter().all(|s| !s.contains("overflow")),
            "`{what}`: nothing on this path can overflow: {f:?}"
        );
    }
}
