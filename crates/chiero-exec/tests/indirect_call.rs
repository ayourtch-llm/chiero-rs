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
