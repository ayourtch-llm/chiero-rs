//! The harness intrinsics actually do what the corpus depends on — 024 §7.
//!
//! Covers: 024 contract 17.
//!
//! `include/chiero.h` declares five intrinsics and 024 §7 says what they mean. Compiling
//! and running the corpus natively (024 contract 17, `chiero-model/tests/corpus_native.rs`)
//! checks the *other* half of the dual-use property. This file checks the half that makes
//! the corpus worth having: under chiero, `chiero_make_symbolic` introduces symbolism and
//! `chiero_assume` constrains it.
//!
//! Neither is a formality. A `chiero_make_symbolic` that does nothing leaves every corpus
//! program on one concrete path with every assertion holding, and a `chiero_assume` that
//! does nothing reports out-of-bounds accesses on programs that have none. Both failures
//! look exactly like a passing test suite.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::{SmtLib, TermArena};
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn assign(dst: u32, rv: RValue) -> Inst {
    Inst {
        kind: InstKind::Assign {
            dst: ValueId(dst),
            rv,
        },
        span: Span::DUMMY,
        generated: false,
    }
}

fn call(dst: Option<u32>, f: u32, args: Vec<Operand>) -> Inst {
    Inst {
        kind: InstKind::Call {
            dst: dst.map(ValueId),
            callee: Callee::Direct(FuncId(f)),
            args,
        },
        span: Span::DUMMY,
        generated: false,
    }
}

fn declared(id: u32, name: &str, ret: CTy) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret,
        variadic: true,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    }
}

/// `int x = 0; chiero_make_symbolic(&x, 4, ...); if (x == 5) return 1; else return 2;`
///
/// The initial store of `0` matters: without `chiero_make_symbolic` doing anything the
/// load reads a definite zero, the branch folds, and one state comes back. So the state
/// count alone distinguishes "the intrinsic worked" from "the intrinsic did nothing",
/// which is the only distinction that matters here.
fn make_symbolic_then_branch(with_assume: bool, with_assert: bool) -> Module {
    let mut insts = vec![
        assign(
            0,
            RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        ),
        Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(0)),
                val: i32c(0),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        },
        // The name string, built byte by byte: a `Global` carries no initializer (020
        // §6), so the only way to hand the intrinsic a real `const char *` without the
        // frontend is to write one.
        assign(
            3,
            RValue::AddrOfLocal {
                alloca: AllocaId(1),
            },
        ),
        Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(3)),
                val: Operand::Const(Const::Int {
                    bits: 8,
                    val: 0x78, // 'x'
                }),
                ty: CTy::Int(8),
                align: 1,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        },
        assign(
            4,
            RValue::PtrAdd {
                base: Operand::Value(ValueId(3)),
                off: Operand::Const(Const::Int { bits: 64, val: 1 }),
            },
        ),
        Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(4)),
                val: Operand::Const(Const::Int { bits: 8, val: 0 }),
                ty: CTy::Int(8),
                align: 1,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        },
        call(
            None,
            1,
            vec![
                Operand::Value(ValueId(0)),
                Operand::Const(Const::Int { bits: 64, val: 4 }),
                Operand::Value(ValueId(3)),
            ],
        ),
        assign(
            1,
            RValue::Load {
                addr: Operand::Value(ValueId(0)),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            },
        ),
        assign(
            2,
            RValue::Cmp {
                op: CmpOp::Eq,
                ty: CTy::Int(32),
                a: Operand::Value(ValueId(1)),
                b: i32c(5),
            },
        ),
    ];
    if with_assume {
        insts.push(call(None, 2, vec![Operand::Value(ValueId(2))]));
    }
    if with_assert {
        insts.push(call(None, 3, vec![Operand::Value(ValueId(2))]));
    }
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![
            AllocaDecl {
                id: AllocaId(0),
                ty: CTy::Int(8),
                count: 4,
                align: 4,
                scope: ScopeId(0),
                lifetime: Lifetime::Scope,
                name: None,
                span: Span::DUMMY,
            },
            AllocaDecl {
                id: AllocaId(1),
                ty: CTy::Int(8),
                count: 2,
                align: 1,
                scope: ScopeId(0),
                lifetime: Lifetime::Scope,
                name: None,
                span: Span::DUMMY,
            },
        ],
        blocks: vec![
            Block {
                id: BlockId(0),
                insts,
                term: Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
                gcov_lines: Default::default(),
                span: Span::DUMMY,
            },
            Block {
                id: BlockId(1),
                insts: vec![],
                term: Terminator::Return(Some(i32c(1))),
                gcov_lines: Default::default(),
                span: Span::DUMMY,
            },
            Block {
                id: BlockId(2),
                insts: vec![],
                term: Terminator::Return(Some(i32c(2))),
                gcov_lines: Default::default(),
                span: Span::DUMMY,
            },
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![
            f,
            declared(1, "chiero_make_symbolic", CTy::Void),
            declared(2, "chiero_assume", CTy::Void),
            declared(3, "chiero_assert", CTy::Void),
        ],
        ..Default::default()
    }
}

/// **`chiero_make_symbolic` introduces symbolism.** Without it the branch is decided by
/// the stored zero and one state comes back.
#[test]
fn make_symbolic_turns_concrete_bytes_into_a_fork() {
    let m = make_symbolic_then_branch(false, false);
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let mut rets: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut a))
        .collect();
    rets.sort_unstable();
    assert_eq!(
        rets,
        vec![1, 2],
        "the bytes were stored as a definite 0 and only `chiero_make_symbolic` can have \
         made the comparison undecidable — one state here means the call did nothing"
    );
}

/// **And the harness's name reaches the witness.** The third parameter exists so a
/// reader gets `x = 5` rather than `sym17 = 5`; a binding nobody can connect to the
/// source is barely better than none.
#[test]
fn the_symbol_carries_the_name_the_harness_gave_it() {
    // **With a violable assert**, because a witness accompanies a finding: a state with
    // nothing to report carries no bindings, so a fixture without one cannot show that
    // the name travelled. `chiero_assert` on a symbolic condition is 024 §7's
    // "the solver could not rule it out", which is exactly the finding a harness wants.
    let m = make_symbolic_then_branch(false, true);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let names: Vec<String> = r
        .states()
        .iter()
        .filter_map(|s| s.witness())
        .flat_map(|w| w.bindings.iter())
        .filter_map(|b| match &b.origin {
            InputOrigin::Param { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !names.is_empty(),
        "the witness binds the symbolic bytes under some name"
    );
    assert!(
        names.iter().all(|n| n == "x"),
        "and the name is the harness's third argument, not a generated one: {names:?}"
    );
}

/// **024 §7 / `IntrinsicOutcome::Constrain`.** A `chiero_assume` on a *symbolic*
/// condition constrains the path.
///
/// This is the case the intrinsic exists for — a ground condition is decided without any
/// constraining — and it was a no-op: `Constrain` shared an arm with `Continue`. A
/// harness narrowing its inputs got no narrowing, and every corpus program that assumes
/// its way into a precondition was explored as though it had not.
#[test]
fn assume_on_a_symbolic_condition_actually_prunes() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = make_symbolic_then_branch(true, false);
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let rets: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut a))
        .collect();
    assert_eq!(
        rets,
        vec![1],
        "`chiero_assume(x == 5)` leaves only the branch where it holds"
    );
}
