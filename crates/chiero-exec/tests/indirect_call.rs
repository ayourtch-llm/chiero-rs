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
        Callee::Indirect {
            target: Operand::Value(ValueId(0)),
            ret: CTy::Int(32),
        }
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
                            callee: Callee::Indirect {
                                target: Operand::Value(ValueId(0)),
                                ret: CTy::Int(32),
                            },
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

/// **023 §5's candidate list means what it says: functions whose signature could be called
/// here.** `Engine::indirect`'s own comment promised that and the code took every defined
/// function in the module.
///
/// A candidate with the wrong arity is not a call the program can make — C11 6.5.2.2p9 makes
/// calling through an incompatible type undefined, and a path into one is a path the program
/// does not have, so every finding on it is about a program nobody wrote.
///
/// It also **crashed the process** on real code. Sweeping 92 VPP plugins, `perfmon_init`'s
/// `(s->init_fn) (vm, s)` forked into a candidate returning `unsigned char`; the caller then
/// compared that one-byte result against a null pointer and the term arena refused the `Eq`.
/// The arity filter here is the part CIR can express — `InstKind::Call`
/// carried no result type until 2026-08-09 (`Callee::Indirect` has `ret` now; the engine
/// still does not consult it),
/// so a candidate with matching arity and a narrower return is still reachable, which is why
/// the engine must also survive the comparison — the test below.
#[test]
fn an_unresolvable_callee_forks_only_into_candidates_of_the_right_arity() {
    // `int two(int a, int b) { return a; }` — the shape of the call site.
    let two = Function {
        id: FuncId(1),
        name: "two".into(),
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
            insts: vec![],
            term: Terminator::Return(Some(Operand::Value(ValueId(0)))),
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
    // `int none(void) { return 0; }` — no parameters, so this call site cannot be it.
    let none = Function {
        id: FuncId(2),
        name: "none".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(Some(i32c(0))),
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
                        // A pointer with no provenance: nothing resolves it, so the engine
                        // falls to the candidate list — which is the case under test.
                        i(InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Ptr },
                        }),
                        i(InstKind::Call {
                            dst: Some(ValueId(1)),
                            callee: Callee::Indirect {
                                target: Operand::Value(ValueId(0)),
                                ret: CTy::Int(32),
                            },
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
            two,
            none,
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let entered: Vec<u32> = r
        .states()
        .iter()
        .flat_map(|s| s.trace().iter().map(|(f, _)| f.0))
        .collect();
    assert!(
        entered.contains(&1),
        "the candidate that fits is still explored: {entered:?}"
    );
    assert!(
        !entered.contains(&2),
        "a two-argument call site cannot be a call to a function of no parameters: {entered:?}"
    );
}

/// **A comparison chiero cannot make is a degraded path, never an aborted process.**
///
/// `InstKind::Call`
/// carried no result type until 2026-08-09 (`Callee::Indirect` has `ret` now; the engine
/// still does not consult it) (020 §4 — the verifier types a call's `dst` as
/// `Void`), so the width of an indirect call's result comes from whichever candidate the engine
/// entered. An arity filter cannot close that: here the candidate takes two arguments like the
/// call site and returns one byte, and the caller compares the result against a null pointer.
///
/// Today the term arena asserts and the whole run dies — every finding on every other path with
/// it. 015's `zero_at` already records the rule this breaks: *"a source-triggerable panic is the
/// worst outcome there is because it takes the run and every other finding in it."*
#[test]
fn a_candidate_whose_return_width_disagrees_with_the_site_is_excluded_and_said_so() {
    let byte_fn = Function {
        id: FuncId(1),
        name: "one_byte".into(),
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
        ret: CTy::Int(8),
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(Some(Operand::Const(Const::Int { bits: 8, val: 1 }))),
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
                            rv: RValue::Fresh { ty: CTy::Ptr },
                        }),
                        i(InstKind::Call {
                            dst: Some(ValueId(1)),
                            callee: Callee::Indirect {
                                target: Operand::Value(ValueId(0)),
                                ret: CTy::Ptr, // the call site believes it called a pointer-returning function
                            },
                            args: vec![i32c(10), i32c(3)],
                        }),
                        // `result != NULL`, as C writes it for a function returning a
                        // pointer — which is what the *call site* believes it called.
                        i(InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Cmp {
                                op: CmpOp::Ne,
                                ty: CTy::Ptr,
                                a: Operand::Value(ValueId(1)),
                                b: Operand::Const(Const::Null),
                            },
                        }),
                    ],
                    term: Terminator::Return(Some(i32c(0))),
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
            byte_fn,
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    // The assertion is that this returns at all.
    let r = Engine::new(&m).run(&mut a);
    assert!(!r.states().is_empty());
    // **And it says what it could not do.** A path that silently carried on past a
    // comparison it never made would answer about a program it did not analyse.
    // Until 2026-08-09 this path *ran* `byte_fn` and then compared its `i8` result against
    // NULL as a pointer, degrading at the comparison. `Callee::Indirect` declares the site's
    // return type now, so the candidate is excluded before it runs — and the degradation has
    // to say that, because "a function chiero has not seen" would be false: chiero saw
    // `byte_fn`, and chose it.
    assert!(
        r.states().iter().any(|s| s
            .assumptions()
            .iter()
            .any(|x| x.detail.contains("return type") && x.detail.contains("excluded"))),
        "the degradation names the exclusion: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions().iter().map(|x| x.detail.clone()))
            .collect::<Vec<_>>()
    );
}

/// **A site that declares `void` may call a `void` function.**
///
/// Obvious, and chiero got it backwards from the day the filter was written until
/// 2026-08-09. The rule was `!(wants_value && f.ret == CTy::Void)` — "a site that uses the
/// result cannot be calling a void function" — and `wants_value` is `dst.is_some()`. But
/// lowering assigns a `dst` to *every* call including a void one, so `wants_value` was true
/// at a void call site, and the rule excluded exactly the candidates that belonged there.
///
/// Real, not hypothetical: four of the six indirect call sites in the pinned-40 corpus are
/// `= call %n -> void(...)`, e.g. `vppinfra/bitmap.c`'s `%102937 = call %102936 -> void`.
/// At every one of them, every void-returning function in the translation unit was ruled
/// out and only the non-void ones — the wrong ones — were explored.
///
/// The observable is the state count: each surviving candidate forks a sibling, and the
/// unresolvable branch is always present, so one reachable candidate means two states.
#[test]
fn a_void_declared_site_may_call_a_void_function() {
    let noop = Function {
        id: FuncId(1),
        name: "noop".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Int(32),
        }],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(None),
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
                            rv: RValue::Fresh { ty: CTy::Ptr },
                        }),
                        // Exactly what lowering emits for `fp (7);` where `fp` returns void:
                        // a `dst`, and a declared return type of `void`.
                        i(InstKind::Call {
                            dst: Some(ValueId(1)),
                            callee: Callee::Indirect {
                                target: Operand::Value(ValueId(0)),
                                ret: CTy::Void,
                            },
                            args: vec![i32c(7)],
                        }),
                    ],
                    term: Terminator::Return(Some(i32c(0))),
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
            noop,
        ],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.states().len(),
        2,
        "the void candidate forks a state, and the unresolvable branch is the other: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions().iter().map(|x| x.detail.clone()))
            .collect::<Vec<_>>()
    );
}
