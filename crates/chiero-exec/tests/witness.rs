//! **Witnesses** — 023 §9 and contract 15.
//!
//! Covers: 023 contract 15 (every finding carries a `Witness`, or an explicit
//! `witness: None` with a recorded reason).
//!
//! §9: "`Witness` is a concrete assignment for every symbolic input on the path …
//! It is what 040 turns into a compilable C replay harness, and it is what distinguishes
//! a chiero finding from a plausible-sounding guess." A finding without one is a claim
//! the reader has to take on trust, which is the thing chiero exists not to produce.

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
    Inst { kind, span: at(lo) }
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

/// `int x = <input>; if (x > 10) { *(int *)0 = 1; }` — one input, one branch, and a
/// definite fault reachable only on one side of it. The witness has to name a value of
/// `x` that actually takes that side, or it replays into the other branch and finds
/// nothing.
fn guarded_fault() -> Module {
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
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::SGt,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: i32c(10),
                            },
                        },
                        20,
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
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![f],
        ..Default::default()
    }
}

/// **023 contract 15.** Every report is either witnessed or says why it is not. There is
/// no third option: a finding with a silently absent witness is indistinguishable from
/// one whose witness was never attempted.
#[test]
fn every_finding_is_witnessed_or_says_why_not() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let reports = r.reports();
    assert!(!reports.is_empty(), "the fixture must find the null store");
    for f in &reports {
        match (&f.witness, &f.unwitnessed) {
            (Some(_), None) => {}
            (None, Some(why)) => assert!(!why.is_empty(), "an empty reason is no reason"),
            (Some(_), Some(_)) => panic!("witnessed and excused at once: {f:?}"),
            (None, None) => panic!("a finding with neither witness nor reason: {f:?}"),
        }
    }
}

/// The witness binds the **input**, and to a value that actually reaches the fault. A
/// witness that satisfies the path condition vacuously — any value at all — replays into
/// the other branch, and a replay that does not reproduce is worse than no witness: it
/// reads as a refutation of a real bug.
#[test]
fn the_witness_binds_an_input_to_a_value_that_reaches_the_fault() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = f.witness.expect("this path's inputs are all decidable");
    assert_eq!(
        w.bindings.len(),
        1,
        "one symbolic input on this path: {:?}",
        w.bindings
    );
    let b = &w.bindings[0];
    assert_eq!(b.width, 32, "at the input's own width: {b:?}");
    assert!(
        (b.value as i32) > 10,
        "the guard is `x > 10`, so a witness at {} reaches the other block",
        b.value as i32
    );
    // And it names *which* input, at the site that created it — a bare number is not a
    // replay harness.
    assert_eq!(b.origin.span(), at(10), "the site that created the input");
}

/// A finding on a path with no symbolic inputs at all still gets a witness: the empty
/// assignment is a complete one. Reporting `None` here would say "we could not produce
/// one", which is false and would send a reader looking for a solver problem.
#[test]
fn a_path_with_no_inputs_is_witnessed_by_the_empty_assignment() {
    let m = Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks: vec![block(
                0,
                vec![inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            )],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: Span::DUMMY,
        }],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = f
        .witness
        .expect("nothing symbolic is on this path, so nothing is undecided");
    assert!(w.bindings.is_empty(), "{:?}", w.bindings);
}

/// The rendered report carries the witness, because a witness a reader cannot see is a
/// witness that does not distinguish this finding from a guess (023 §9).
#[test]
fn the_report_shows_the_witness() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let text = render(&r);
    assert!(
        text.contains("witness"),
        "the report names the witness: {text}"
    );
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .unwrap();
    let b = &f.witness.as_ref().unwrap().bindings[0];
    assert!(
        text.contains(&format!("{}", b.value as i32)),
        "with the value it binds: {text}"
    );
}
