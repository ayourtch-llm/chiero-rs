//! Calling through a function pointer.
//!
//! VPP dispatches every graph node this way. `tests/corpus/owed/indirect_call.c` lowers and
//! verifies as of wave 120, and then both callees read their parameter as uninitialized:
//! the *dispatch* resolves and the *argument* does not arrive.
//!
//! The pairing is the test. A direct call and an indirect call to the same function, with
//! the same argument, must produce the same answer — anything else is the calling
//! convention differing by how the callee was named, which is not a thing C has.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn i(kind: InstKind) -> Inst {
    Inst {
        kind,
        span: Span::DUMMY,
        generated: false,
    }
}

/// `int twice(int v) { return v * 2; }`
fn twice() -> Function {
    Function {
        id: FuncId(1),
        name: "twice".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Int(32),
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![i(InstKind::Assign {
                dst: ValueId(1),
                rv: RValue::Bin {
                    op: BinOp::Mul,
                    a: Operand::Value(ValueId(0)),
                    b: i32c(2),
                    ty: CTy::Int(32),
                    signed: true,
                },
            })],
            term: Terminator::Return(Some(Operand::Value(ValueId(1)))),
            gcov_lines: Default::default(),
            span: Span::DUMMY,
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        access_paths: Default::default(),
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    }
}

/// `main` calling `twice(21)`, directly or through its address.
fn caller(indirect: bool) -> Module {
    let mut insts = Vec::new();
    let callee = if indirect {
        insts.push(i(InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::AddrOfFunc(FuncId(1)),
        }));
        Callee::Indirect(Operand::Value(ValueId(0)))
    } else {
        Callee::Direct(FuncId(1))
    };
    insts.push(i(InstKind::Call {
        dst: Some(ValueId(1)),
        callee,
        args: vec![i32c(21)],
    }));

    Module {
        funcs: vec![
            Function {
                id: FuncId(0),
                name: "main".into(),
                params: vec![],
                ret: CTy::Int(32),
                variadic: false,
                allocas: vec![],
                blocks: vec![Block {
                    id: BlockId(0),
                    insts,
                    term: Terminator::Return(Some(Operand::Value(ValueId(1)))),
                    gcov_lines: Default::default(),
                    span: Span::DUMMY,
                }],
                entry: BlockId(0),
                attrs: Default::default(),
                body: Body::Defined,
                access_paths: Default::default(),
                span: Span::DUMMY,
                linkage: chiero_cir::Linkage::External,
            },
            twice(),
        ],
        ..Default::default()
    }
}

fn run(m: &Module) -> (Option<u128>, Vec<String>) {
    assert!(
        verify::verify(m).iter().all(|e| !e.is_error()),
        "{:?}",
        verify::verify(m)
    );
    let mut a = TermArena::new();
    let r = Engine::new(m).run(&mut a);
    assert_eq!(r.states().len(), 1, "no symbolic condition, so one path");
    (r.states()[0].return_value_bits(&mut a), r.findings())
}

/// **An indirect call binds its arguments**, exactly as a direct one does.
#[test]
fn an_indirect_call_passes_its_argument() {
    let (direct, _) = run(&caller(false));
    assert_eq!(direct, Some(42), "the direct call is the control: 21 * 2");

    let (indirect, findings) = run(&caller(true));
    assert_eq!(
        indirect, direct,
        "the same callee with the same argument, named two ways: {findings:#?}"
    );
}

/// **And the callee's parameter is not uninitialized.**
///
/// The value assertion above can be satisfied by an engine that invents a symbol and
/// happens to return it; the *finding* is what says the parameter was really bound. This is
/// the exact report `indirect_call.c` produces.
#[test]
fn an_indirect_calls_parameter_is_not_uninitialized() {
    let (_, findings) = run(&caller(true));
    assert!(
        findings.iter().all(|f| !f.contains("uninitialized")),
        "the parameter arrived: {findings:#?}"
    );
    // The direct call is the control: if *it* reported one, the fixture would be wrong
    // rather than the indirect path.
    let (_, direct) = run(&caller(false));
    assert!(direct.is_empty(), "{direct:#?}");
}

/// **Two arguments arrive in order**, which one parameter cannot show.
///
/// `sub(10, 3)` is 7 and `sub(3, 10)` is -7, so a binding that pairs parameters with
/// arguments in the wrong order is visible in the value. With a single parameter, reversing
/// the pairing is a no-op and the mutation survives.
#[test]
fn two_arguments_arrive_in_order() {
    let sub = Function {
        id: FuncId(1),
        name: "sub".into(),
        params: vec![
            Param {
                value: ValueId(0),
                ty: CTy::Int(32),
            },
            Param {
                value: ValueId(1),
                ty: CTy::Int(32),
            },
        ],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![i(InstKind::Assign {
                dst: ValueId(2),
                rv: RValue::Bin {
                    op: BinOp::Sub,
                    a: Operand::Value(ValueId(0)),
                    b: Operand::Value(ValueId(1)),
                    ty: CTy::Int(32),
                    signed: true,
                },
            })],
            term: Terminator::Return(Some(Operand::Value(ValueId(2)))),
            gcov_lines: Default::default(),
            span: Span::DUMMY,
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        access_paths: Default::default(),
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![
            Function {
                id: FuncId(0),
                name: "main".into(),
                params: vec![],
                ret: CTy::Int(32),
                variadic: false,
                allocas: vec![],
                blocks: vec![Block {
                    id: BlockId(0),
                    insts: vec![
                        i(InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::AddrOfFunc(FuncId(1)),
                        }),
                        i(InstKind::Call {
                            dst: Some(ValueId(1)),
                            callee: Callee::Indirect(Operand::Value(ValueId(0))),
                            args: vec![i32c(10), i32c(3)],
                        }),
                    ],
                    term: Terminator::Return(Some(Operand::Value(ValueId(1)))),
                    gcov_lines: Default::default(),
                    span: Span::DUMMY,
                }],
                entry: BlockId(0),
                attrs: Default::default(),
                body: Body::Defined,
                access_paths: Default::default(),
                span: Span::DUMMY,
                linkage: chiero_cir::Linkage::External,
            },
            sub,
        ],
        ..Default::default()
    };
    let (v, findings) = run(&m);
    assert_eq!(v, Some(7), "10 - 3, not 3 - 10: {findings:#?}");
}
