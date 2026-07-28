//! Provenance and lazily-materialized bytes — 021 contracts 3, 13b and 27.
//!
//! Covers: 021 contracts 3, 13b, 27.
//!
//! Three separate claims that all come back to the same rule: chiero must not invent a
//! fact it does not have, and must not lose one it does.

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

fn ptr_add(dst: u32, base: u32, off: i128, lo: u32) -> Inst {
    inst(
        InstKind::Assign {
            dst: ValueId(dst),
            rv: RValue::PtrAdd {
                base: Operand::Value(ValueId(base)),
                off: Operand::Const(Const::Int { bits: 64, val: off }),
            },
        },
        lo,
    )
}

/// **021 contract 3.** "`PtrAdd` past the end of an object then back inside yields a
/// pointer that reads correctly and preserves `base`."
///
/// C makes the *intermediate* pointer undefined and the round trip is a real idiom —
/// `container_of` walks backwards out of a member — so a model that clamps, wraps, or
/// forgets the object on the way out cannot express it. Provenance is carried by the
/// pointer, not recomputed from the address, which is what makes this work at all.
#[test]
fn a_pointer_walked_out_of_an_object_and_back_reads_correctly() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 4,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: at(1),
        }],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    10,
                ),
                // Write 0x2A at byte 4, so the round trip has something to read back.
                ptr_add(1, 0, 4, 15),
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(1)),
                        val: i32c(0x2A),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    20,
                ),
                // Out past the end…
                ptr_add(2, 1, 4096, 30),
                // …and back in, to the same byte.
                ptr_add(3, 2, -4096, 40),
                inst(
                    InstKind::Assign {
                        dst: ValueId(4),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(3)),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                    },
                    50,
                ),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(4)))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let s = &r.states()[0];
    let (Some(Value::Ptr(start)), Some(Value::Ptr(back))) =
        (s.local(ValueId(1)), s.local(ValueId(3)))
    else {
        panic!("both are pointers");
    };
    assert_eq!(back.base, start.base, "the object survives the excursion");
    assert_eq!(back.off, start.off, "and so does the offset");
    match s.local(ValueId(4)) {
        Some(Value::Scalar(t)) => assert_eq!(
            a.eval_ground(t).map(|c| c.bits()),
            Ok(0x2A),
            "and the byte that was written comes back"
        ),
        other => panic!("{other:?}"),
    }
    // The excursion itself is not a finding: forming the pointer is UB in C, and 021 §7
    // reports it only when `ub-strict` asks. What must not happen is a *bounds* finding
    // on the read, which is in bounds.
    assert!(
        !r.findings().iter().any(|f| f.contains("out-of-bounds")),
        "the read is in bounds: {:#?}",
        r.findings()
    );
}

/// **021 contract 13b.** "An `IntToPtr` of a wholly unconstrained symbol yields
/// `Fidelity::Unknown` and one `UnresolvablePointer` finding, and **no read through it is
/// ever reported as in-bounds** (§5.1 step 4). This is the `vlib_get_buffer` case; getting
/// it wrong analyses the wrong memory for an entire function."
#[test]
fn an_unconstrained_int_to_ptr_is_never_read_as_in_bounds() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 4,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: at(1),
        }],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Fresh { ty: CTy::Int(64) },
                    },
                    10,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cast {
                            kind: CastKind::IntToPtr,
                            a: Operand::Value(ValueId(0)),
                            from: CTy::Int(64),
                            to: CTy::Ptr,
                        },
                    },
                    20,
                ),
                // Never reached: step 4 stops the path rather than guessing an object.
                inst(
                    InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(1)),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                    },
                    30,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Unknown);
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("unresolvable pointer"))
            .count(),
        1,
        "exactly one: {:#?}",
        r.findings()
    );
    // **The load never happened.** "No read through it is reported as in-bounds" is
    // satisfied here by there being no read at all — and that is the point: the
    // alternative is continuing into a function's worth of wrong memory.
    assert!(
        r.states().iter().all(|s| s.local(ValueId(2)).is_none()),
        "the path stopped rather than reading somewhere"
    );
}

/// **021 contract 27.** "A lazily-materialized object's bytes are `Yes` with unknown
/// values: reading them produces **no** uninitialized-read finding (§3.1's
/// symbolic-is-not-uninitialized rule)."
///
/// An entry function's pointer parameter points at memory the caller filled — chiero does
/// not know *what* with, which is not the same as nobody having written it. Reporting an
/// uninitialized read there would fire on every function that takes a pointer.
#[test]
fn reading_a_lazily_materialized_parameter_is_not_an_uninitialized_read() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Load {
                        addr: Operand::Value(ValueId(0)),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                },
                10,
            )],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        !r.findings()
            .iter()
            .any(|f| f.contains("never written") || f.contains("uninitialized")),
        "the caller wrote these bytes; chiero just does not know what with: {:#?}",
        r.findings()
    );
    // And the value is *something* — a load that produced nothing would satisfy the
    // assertion above for the wrong reason.
    assert!(
        r.states()[0].local(ValueId(1)).is_some(),
        "the read produced a value"
    );
}

/// **A store through an entry pointer parameter must land** — 021 §6 says a lazily
/// materialized object's contents are "fully symbolic and fully **initialized**", not
/// read-only.
///
/// Found by review, as a regression from the fix directly above: materializing the pointee
/// with a symbolic havoc promotes it to an array representation, and every byte-level
/// *write* path refuses a promoted object. So `p[1] = 'a'` was dropped, and the following
/// `if (p[1] == 'a')` explored **both** sides — one of which the program does not have.
/// That is 023 §7's confidently-wrong answer, on the most common idiom in C.
#[test]
fn a_store_through_a_parameter_is_read_back_on_the_same_path() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![
                    inst(
                        InstKind::Store {
                            addr: Operand::Value(ValueId(0)),
                            val: i32c(0x2A),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Load {
                                addr: Operand::Value(ValueId(0)),
                                ty: CTy::Int(32),
                                align: 4,
                                vol: Volatility::Normal,
                            },
                        },
                        20,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Cmp {
                                op: CmpOp::Eq,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(1)),
                                b: i32c(0x2A),
                            },
                        },
                        30,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(1)))),
            block(2, vec![], Terminator::Return(Some(i32c(2)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.states().len(),
        1,
        "what was just written is what is read back; there is no second path: {:#?}",
        r.states()
            .iter()
            .map(|s| (&s.status, s.local(ValueId(1))))
            .collect::<Vec<_>>()
    );
    assert!(
        r.findings().is_empty(),
        "and writing through a caller's pointer is not a finding: {:#?}",
        r.findings()
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "nor a reason to know less than exactly"
    );
}
