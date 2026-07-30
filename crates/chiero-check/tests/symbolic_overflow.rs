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
    guarded_as(op, cmp, bound, rhs, true)
}

/// The same, with the operation's signedness spelled out.
fn guarded_as(op: BinOp, cmp: CmpOp, bound: i128, rhs: i128, signed: bool) -> Module {
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
                        signed,
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

/// **The boundary is `INT_MAX`, not one past it.**
///
/// `x > 2147483646` leaves exactly one value, and `x + 1` is `2147483648` — the first integer
/// that does not fit. Every other forced fixture here overflows by more than that, so a range
/// whose upper bound is off by one still catches them; mutation found `max = 1 << (w - 1)` passing
/// the whole file. The smallest overflow there is is the one that pins the constant.
#[test]
fn an_overflow_of_exactly_one_past_the_maximum_is_reported() {
    let f = findings(&guarded(BinOp::Add, CmpOp::SGt, 2_147_483_646, 1));
    assert!(
        f.iter().any(|s| s.starts_with("signed-overflow")),
        "2147483647 + 1 is the smallest signed overflow that exists: {f:?}"
    );
}

/// **Unsigned arithmetic does not overflow.** C11 6.2.5p9 makes it wrap, and wrapping is defined.
///
/// The signedness guard, which mutation showed nothing was holding: dropping it put the query on
/// unsigned operations too, where a forced "overflow" is a program doing exactly what the standard
/// says it does. This is the same distinction wave 174 gave CIR `signed` on `RValue::Bin` for.
#[test]
fn unsigned_arithmetic_the_path_forces_to_wrap_is_not_reported() {
    for (what, op, cmp, bound, rhs) in [
        ("Add", BinOp::Add, CmpOp::UGt, 4_294_967_290i128, 10i128),
        ("Mul", BinOp::Mul, CmpOp::UGt, 2_147_483_648, 2),
    ] {
        let f = findings(&guarded_as(op, cmp, bound, rhs, false));
        assert!(
            f.iter().all(|s| !s.contains("overflow")),
            "`{what}`: unsigned arithmetic wraps by definition, so there is nothing to report: \
             {f:?}"
        );
    }
}

/// **`INT_MIN` is a value, not an overflow.**
///
/// The mirror of the test above, and it asserts *silence* because the two ends of the range are not
/// symmetric: the largest representable signed integer is `2^(w-1) - 1` and the smallest is
/// `-2^(w-1)`. A lower bound written as `-max` rather than `-2^(w-1)` is off by one in the
/// direction that invents a fault, and mutation found it passing every other fixture here — they
/// all overflow by more than one, so only the exact boundary can see it.
///
/// `x == -2147483638` pins one value; `x - 10` is exactly `INT_MIN`, which fits.
#[test]
fn a_result_of_exactly_the_minimum_is_not_an_overflow() {
    let f = findings(&guarded(BinOp::Sub, CmpOp::Eq, -2_147_483_638, 10));
    assert!(
        f.iter().all(|s| !s.contains("overflow")),
        "-2147483648 is representable, so computing it is not undefined: {f:?}"
    );
}
