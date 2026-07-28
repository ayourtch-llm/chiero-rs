//! **UB is an event, not a stopped path** — 020 §4.1 and contracts 8 and 9.
//!
//! Covers: 020 contracts 8 and 9.
//!
//! §4.1: "CIR is not the place to encode UB as unpredictability: the semantics are defined
//! and total, and a `Checker` observes the overflow event and reports it." The IR value is
//! the SMT-LIB value, "so the IR and the solver cannot disagree", and every row of the
//! table **continues** — an earlier draft stopped the path on division alone, "hiding
//! everything downstream of it for no reason the other cases don't share".
//!
//! So there are two separate claims per case, and both are tested: the *value* the
//! program computes, and the *event* a checker will later turn into a finding. A value
//! with no event is a silent UB; an event with the wrong value is a wrong program.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
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

/// `%0 = <op> a, b; %1 = %0` — the second instruction only so the path visibly continues
/// past the UB. Returns the run.
fn run_op(op: BinOp, width: u32, a_val: i128, b_val: i128) -> (RunResult, TermArena) {
    let k = |v: i128| {
        Operand::Const(Const::Int {
            bits: width,
            val: v,
        })
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Bin {
                            op,
                            ty: CTy::Int(width),
                            a: k(a_val),
                            b: k(b_val),
                        },
                    },
                    span: at(10),
                    generated: false,
                },
                // Reached only if the path continued.
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Use(Operand::Const(Const::Int { bits: 32, val: 99 })),
                    },
                    span: at(20),
                    generated: false,
                },
            ],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut arena = TermArena::new();
    // The arena travels with the result: a term is only meaningful in the arena that
    // built it, and evaluating one against a fresh arena reads a different node.
    let r = Engine::new(&m).run(&mut arena);
    (r, arena)
}

fn value_of(r: &RunResult, arena: &TermArena, v: ValueId) -> u128 {
    match r.states()[0].local(v) {
        Some(Value::Scalar(t)) => arena.eval_ground(t).expect("ground").bits(),
        other => panic!("{other:?}"),
    }
}

/// **020 contract 8.** "`Shl` of `Int(32)` by 32 yields 0 and emits exactly one shift-UB
/// event." Exactly one: an operation is one event however many times a report is
/// assembled, and a checker counting two would report one bug twice.
#[test]
fn a_shift_by_the_full_width_yields_zero_and_one_shift_event() {
    let (r, arena) = run_op(BinOp::Shl, 32, 1, 32);
    assert_eq!(
        value_of(&r, &arena, ValueId(0)),
        0,
        "1 << 32 is 0 at 32 bits"
    );
    assert!(
        r.states()[0].local(ValueId(1)).is_some(),
        "and the path continues past the shift"
    );
    let ev: Vec<_> = r
        .ub_events()
        .into_iter()
        .filter(|e| e.kind == UbKind::Shift)
        .collect();
    assert_eq!(ev.len(), 1, "exactly one shift event: {:?}", r.ub_events());
    assert_eq!(ev[0].span, at(10), "at the shift, not at the block");
}

/// The three shift operators all report, and a shift *within* the width reports nothing —
/// otherwise every ordinary `x << 1` in VPP's packet code is a finding.
#[test]
fn every_shift_operator_reports_and_an_in_range_shift_does_not() {
    for op in [BinOp::Shl, BinOp::LShr, BinOp::AShr] {
        let (r, _) = run_op(op, 32, 1, 32);
        assert_eq!(
            r.ub_events()
                .iter()
                .filter(|e| e.kind == UbKind::Shift)
                .count(),
            1,
            "{op:?} by the width is UB"
        );
        let (ok, _) = run_op(op, 32, 1, 31);
        assert!(
            ok.ub_events().is_empty(),
            "{op:?} by width-1 is ordinary code: {:?}",
            ok.ub_events()
        );
    }
}

/// **020 contract 9.** Division and remainder by zero follow §4.1's table **and execution
/// continues**: `UDiv 5 0` and `SDiv 5 0` yield all-ones, `SDiv (-5) 0` yields `1`,
/// `URem 5 0` yields `5`, `SRem (-5) 0` yields `-5` — each emitting exactly one
/// div-by-zero event.
#[test]
fn division_by_zero_follows_the_table_and_the_path_continues() {
    let cases: &[(BinOp, i128, u128)] = &[
        (BinOp::UDiv, 5, 0xffff_ffff),
        (BinOp::SDiv, 5, 0xffff_ffff),
        (BinOp::SDiv, -5, 1),
        (BinOp::URem, 5, 5),
        (BinOp::SRem, -5, 0xffff_fffb),
    ];
    for (op, a_val, want) in cases.iter().copied() {
        let (r, arena) = run_op(op, 32, a_val, 0);
        assert_eq!(
            r.ub_events()
                .iter()
                .filter(|e| e.kind == UbKind::DivByZero)
                .count(),
            1,
            "{op:?} {a_val} 0 is one div-by-zero event: {:?}",
            r.ub_events()
        );
        let s = &r.states()[0];
        assert!(
            s.local(ValueId(1)).is_some(),
            "{op:?}: the path continues — an earlier draft stopped it on division alone"
        );
        assert_eq!(
            s.status,
            Status::Terminated(TermReason::Return),
            "{op:?}: and returns normally"
        );
        assert_eq!(value_of(&r, &arena, ValueId(0)), want, "{op:?} {a_val} 0");
    }
}

/// Signed overflow wraps, and says so. §4.1's first row: the value is the wrapped one —
/// "defined means the SMT-LIB value, so the IR and the solver cannot disagree" — and the
/// event is what a checker turns into the finding.
#[test]
fn signed_overflow_wraps_and_reports() {
    let (r, arena) = run_op(BinOp::Add, 32, i32::MAX as i128, 1);
    assert_eq!(
        value_of(&r, &arena, ValueId(0)),
        0x8000_0000,
        "the value is the wrapped one, which is the SMT-LIB one"
    );
    assert_eq!(
        r.ub_events()
            .iter()
            .filter(|e| e.kind == UbKind::SignedOverflow)
            .count(),
        1,
        "INT_MAX + 1 overflows: {:?}",
        r.ub_events()
    );
    let (ok, _) = run_op(BinOp::Add, 32, 1, 1);
    assert!(ok.ub_events().is_empty(), "1 + 1 does not");
}

/// The events reach the reader. 020 §4.1 keeps the *decision* — is this wrap a bug? — in
/// a checker, because VPP wraps on purpose all over; what a reader must not have to take
/// on trust is what the program actually did.
#[test]
fn ub_events_appear_in_the_rendered_report() {
    let (r, _) = run_op(BinOp::UDiv, 32, 5, 0);
    let text = render(&r);
    assert!(
        text.contains("undefined behaviour") && text.contains("DivByZero"),
        "{text}"
    );
    // And a clean run says nothing about UB, rather than printing an empty section.
    let (ok, _) = run_op(BinOp::Add, 32, 1, 1);
    assert!(
        !render(&ok).contains("undefined behaviour"),
        "{}",
        render(&ok)
    );
}
