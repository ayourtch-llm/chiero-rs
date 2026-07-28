//! Two CIR constructs the engine refuses — 020 contracts 42 and 43.
//!
//! Covers: 020 contracts 42 and 43.
//!
//! Both are `_ => give_up` today, which is the shape 020's own note warns about: a
//! construct the IR defines and the engine treats as "chiero cannot follow this" is
//! indistinguishable, in the output, from a program that really is unfollowable.
//!
//! Contract 42 comes with an explicit escape hatch — "VPP contains no computed gotos …
//! if it is dropped from v1, drop this contract with it rather than leaving it untested".
//! Fifteen lines of fork is cheaper than that conversation, and it removes a `give_up`.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

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
        span: Span::DUMMY,
    }
}

fn func(blocks: Vec<Block>) -> Module {
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
            body: Body::Defined,
            span: Span::DUMMY,
        }],
        ..Default::default()
    }
}

/// **020 contract 42.** "`IndirectGoto` executes to each of its declared targets from a
/// `.cir` fixture." Each of them: a terminator that reaches one target is a terminator
/// whose other targets are unexplored, and nothing in the output would say so.
#[test]
fn an_indirect_goto_reaches_each_declared_target() {
    let m = func(vec![
        block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Ptr },
                },
                10,
            )],
            Terminator::IndirectGoto {
                addr: Operand::Value(ValueId(0)),
                targets: vec![BlockId(1), BlockId(2), BlockId(3)],
            },
        ),
        block(1, vec![], Terminator::Return(Some(i32c(1)))),
        block(2, vec![], Terminator::Return(Some(i32c(2)))),
        block(3, vec![], Terminator::Return(Some(i32c(3)))),
    ]);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let mut reached: Vec<BlockId> = r
        .states()
        .iter()
        .filter_map(|s| s.trace().last().map(|(_, b)| *b))
        .collect();
    reached.sort_by_key(|b| b.0);
    reached.dedup();
    assert_eq!(
        reached,
        vec![BlockId(1), BlockId(2), BlockId(3)],
        "one state per declared target"
    );
    assert!(
        r.states()
            .iter()
            .all(|s| s.status == Status::Terminated(TermReason::Return)),
        "and each returns rather than giving up: {:?}",
        r.states().iter().map(|s| &s.status).collect::<Vec<_>>()
    );
}

/// The target list is a *declaration*, and the run says so: reaching every declared
/// target is only exhaustive if the declaration was. `Fidelity` records that the address
/// was not resolved to one of them.
#[test]
fn an_indirect_goto_records_that_its_target_list_is_a_declaration() {
    let m = func(vec![
        block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Ptr },
                },
                10,
            )],
            Terminator::IndirectGoto {
                addr: Operand::Value(ValueId(0)),
                targets: vec![BlockId(1), BlockId(2)],
            },
        ),
        block(1, vec![], Terminator::Return(Some(i32c(1)))),
        block(2, vec![], Terminator::Return(Some(i32c(2)))),
    ]);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "the address was never resolved; the targets are the frontend's word"
    );
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("indirect goto")),
        "and the reason names the construct: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
}

/// **020 contract 43, first half.** "Any arithmetic with an `Undef` operand yields
/// `Undef`." Inventing a value for `Undef` is the opposite of what it means, and folding
/// it to zero is the same confidently-wrong answer as reading uninitialized memory as
/// zero — 021 §3.1's headline failure, one level up.
#[test]
fn arithmetic_on_undef_yields_undef() {
    let m = func(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op: BinOp::Add,
                        ty: CTy::Int(32),
                        a: Operand::Const(Const::Undef(CTy::Int(32))),
                        b: i32c(1),
                    },
                },
                10,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin {
                        op: BinOp::Mul,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(0)),
                        b: i32c(0),
                    },
                },
                20,
            ),
        ],
        Terminator::Return(Some(i32c(0))),
    )]);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    assert_eq!(
        s.local(ValueId(0)),
        Some(Value::Undef),
        "undef + 1 is undef, not 1"
    );
    // **Even times zero.** `x * 0` is 0 for every *value* of x, and `Undef` is not a
    // value — C's undef times zero is still undef, and a model that folds it to 0 has
    // decided what the program left undecided.
    assert_eq!(
        s.local(ValueId(1)),
        Some(Value::Undef),
        "undef * 0 is undef"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Unknown,
        "a run that computed with undef knows less than exactly"
    );
}

/// **020 contract 43, second half.** "A branch on `Undef` forks both ways with
/// `Fidelity::Unknown`." Taking one side would be choosing for the program.
#[test]
fn a_branch_on_undef_forks_both_ways() {
    let m = func(vec![
        block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Use(Operand::Const(Const::Undef(CTy::Int(1)))),
                },
                10,
            )],
            Terminator::Br {
                cond: Operand::Value(ValueId(0)),
                t: BlockId(1),
                f: BlockId(2),
            },
        ),
        block(1, vec![], Terminator::Return(Some(i32c(1)))),
        block(2, vec![], Terminator::Return(Some(i32c(2)))),
    ]);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let mut reached: Vec<BlockId> = r
        .states()
        .iter()
        .filter_map(|s| s.trace().last().map(|(_, b)| *b))
        .collect();
    reached.sort_by_key(|b| b.0);
    assert_eq!(reached, vec![BlockId(1), BlockId(2)], "both ways");
    assert_eq!(r.fidelity(), Fidelity::Unknown);
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("undef")),
        "and says why"
    );
}
