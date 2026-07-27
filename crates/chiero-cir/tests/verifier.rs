//! The CIR verifier.
//!
//! Covers **020 contract 4** — every rejection it enumerates — and contract 5 (a
//! well-formed module verifies clean). A module that fails verification is never
//! executed, so a verifier that misses a rule lets malformed IR reach the engine, where
//! the symptom is a confusing wrong answer rather than a clear error.
//!
//! Each test builds a **valid** module and breaks exactly one thing, so a rejection can
//! only be attributed to the rule under test.

use chiero_cir::*;
use chiero_span::Span;

fn inst(kind: InstKind) -> Inst {
    Inst {
        kind,
        span: Span::DUMMY,
    }
}

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
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

/// `func @f() -> i32 { entry: %0 = add i32 2, 3; ret %0 }`
fn valid_module() -> Module {
    let add = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Bin {
            op: BinOp::Add,
            a: i32c(2),
            b: i32c(3),
            ty: CTy::Int(32),
        },
    });
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![add],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
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

/// 020 contract 5.
#[test]
fn a_wellformed_module_verifies_clean() {
    let errs = verify(&valid_module());
    assert!(errs.is_empty(), "expected clean, got: {errs:#?}");
}

#[track_caller]
fn assert_rejects(m: &Module, want: VerifyErrorKind) {
    let errs = verify(m);
    assert!(
        errs.iter().any(|e| e.kind == want),
        "expected {want:?}, got: {errs:#?}"
    );
    // Breaking one thing must not cascade: the rule under test should be the *only*
    // kind reported, or the diagnostic points at the wrong place.
    let kinds: Vec<_> = errs.iter().map(|e| e.kind).collect();
    assert!(
        kinds.iter().all(|k| *k == want),
        "one defect must yield one kind of error, got: {kinds:?}"
    );
}

/// Rule 1: single assignment.
#[test]
fn value_assigned_twice_is_rejected() {
    let mut m = valid_module();
    let dup = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Use(i32c(7)),
    });
    m.funcs[0].blocks[0].insts.push(dup);
    assert_rejects(&m, VerifyErrorKind::ValueAssignedTwice);
}

/// Rule 1: definitions dominate uses. Using a value defined in a sibling block is the
/// case a naive "is it defined anywhere in the function" check would miss.
#[test]
fn use_not_dominated_by_definition_is_rejected() {
    let mut m = valid_module();
    let f = &mut m.funcs[0];
    // entry -> bb1 / bb2 ; bb1 defines %1, bb2 uses it.
    f.blocks[0].term = Terminator::Br {
        cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
        t: BlockId(1),
        f: BlockId(2),
    };
    f.blocks.push(block(
        1,
        vec![inst(InstKind::Assign {
            dst: ValueId(1),
            rv: RValue::Use(i32c(1)),
        })],
        Terminator::Return(None),
    ));
    f.blocks.push(block(
        2,
        vec![inst(InstKind::Assign {
            dst: ValueId(2),
            rv: RValue::Use(Operand::Value(ValueId(1))),
        })],
        Terminator::Return(None),
    ));
    assert_rejects(&m, VerifyErrorKind::UseNotDominated);
}

/// A use *within* the defining block but textually before the definition is also
/// undominated — the within-block ordering case.
#[test]
fn use_before_definition_in_same_block_is_rejected() {
    let mut m = valid_module();
    let early = inst(InstKind::Assign {
        dst: ValueId(1),
        rv: RValue::Use(Operand::Value(ValueId(0))),
    });
    m.funcs[0].blocks[0].insts.insert(0, early);
    assert_rejects(&m, VerifyErrorKind::UseNotDominated);
}

/// Rule 2.
#[test]
fn branch_to_nonexistent_block_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].term = Terminator::Goto(BlockId(99));
    assert_rejects(&m, VerifyErrorKind::UnknownBlock);
}

/// Rule 5: `Cmp` yields `Int(1)`.
#[test]
fn cmp_declared_wider_than_one_bit_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Cmp {
            op: CmpOp::Eq,
            a: i32c(1),
            b: i32c(2),
            // `ty` is the *operand* type; the result is always Int(1). A verifier that
            // conflated the two would accept this.
            ty: CTy::Int(32),
        },
    });
    // A wide operand type on a Cmp is legal on its own.
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert!(
        verify(&m).is_empty(),
        "operand ty on Cmp is legal: {:?}",
        verify(&m)
    );

    // ...but the *result* is Int(1), so returning it from an i32 function is not.
    m.funcs[0].blocks[0].term = Terminator::Return(Some(Operand::Value(ValueId(0))));
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

/// Rule 5: `Trunc` must narrow strictly.
#[test]
fn widening_trunc_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Cast {
            kind: CastKind::Trunc,
            a: Operand::Const(Const::Int { bits: 8, val: 1 }),
            from: CTy::Int(8),
            to: CTy::Int(32),
        },
    });
    assert_rejects(&m, VerifyErrorKind::BadCast);
}

/// Rule 5: `ZExt`/`SExt` must widen strictly — equal widths are a no-op cast, which
/// lowering must not emit.
#[test]
fn nonwidening_zext_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Cast {
            kind: CastKind::ZExt,
            a: i32c(1),
            from: CTy::Int(32),
            to: CTy::Int(32),
        },
    });
    assert_rejects(&m, VerifyErrorKind::BadCast);
}

/// Rule 12: `Bitcast` preserves total bit width. Nothing checked this before review.
#[test]
fn width_changing_bitcast_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Cast {
            kind: CastKind::Bitcast,
            a: i32c(1),
            from: CTy::Int(32),
            to: CTy::Vector {
                elem: Box::new(CTy::Int(8)),
                lanes: 16,
            },
        },
    });
    // Neutralize the return so the only defect is the cast itself.
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::BadCast);
}

#[test]
fn width_preserving_bitcast_is_accepted() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Cast {
            kind: CastKind::Bitcast,
            a: Operand::Const(Const::Int { bits: 128, val: 1 }),
            from: CTy::Int(128),
            to: CTy::Vector {
                elem: Box::new(CTy::Int(32)),
                lanes: 4,
            },
        },
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert!(verify(&m).is_empty(), "128 bits either way is legal");
}

/// Rule 7.
#[test]
fn non_power_of_two_alignment_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts.insert(
        0,
        inst(InstKind::Store {
            addr: Operand::Const(Const::Null),
            val: i32c(0),
            ty: CTy::Int(32),
            align: 3,
            vol: Volatility::Normal,
        }),
    );
    assert_rejects(&m, VerifyErrorKind::BadAlignment);
}

#[test]
fn zero_alignment_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts.insert(
        0,
        inst(InstKind::Store {
            addr: Operand::Const(Const::Null),
            val: i32c(0),
            ty: CTy::Int(32),
            align: 0,
            vol: Volatility::Normal,
        }),
    );
    assert_rejects(&m, VerifyErrorKind::BadAlignment);
}

/// Rule 8.
#[test]
fn duplicate_switch_case_is_rejected() {
    let mut m = valid_module();
    let f = &mut m.funcs[0];
    f.blocks.push(block(1, vec![], Terminator::Return(None)));
    f.blocks[0].term = Terminator::Switch {
        scrut: i32c(0),
        ty: CTy::Int(32),
        cases: vec![(1, BlockId(1)), (1, BlockId(1))],
        default: BlockId(1),
    };
    assert_rejects(&m, VerifyErrorKind::DuplicateSwitchCase);
}

/// Rule 10.
#[test]
fn declared_function_with_a_block_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].body = Body::Declared;
    assert_rejects(&m, VerifyErrorKind::DeclaredWithBody);
}

#[test]
fn defined_function_with_no_blocks_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks.clear();
    assert_rejects(&m, VerifyErrorKind::DefinedWithoutBody);
}

/// Rule 9: `entry` has no predecessors, so a loop back to entry is malformed —
/// lowering must insert a preheader.
#[test]
fn branch_back_to_entry_is_rejected() {
    let mut m = valid_module();
    let f = &mut m.funcs[0];
    f.blocks[0].term = Terminator::Goto(BlockId(1));
    f.blocks
        .push(block(1, vec![], Terminator::Goto(BlockId(0))));
    assert_rejects(&m, VerifyErrorKind::EntryHasPredecessor);
}

/// Rule 11.
#[test]
fn bad_bitrange_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::LoadBits {
            addr: Operand::Const(Const::Null),
            unit: CTy::Int(32),
            bits: BitRange { off: 30, width: 8 }, // 30 + 8 > 32
            signed: false,
            align: 4,
        },
    });
    assert_rejects(&m, VerifyErrorKind::BadBitRange);
}

#[test]
fn zero_width_bitfield_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::LoadBits {
            addr: Operand::Const(Const::Null),
            unit: CTy::Int(32),
            bits: BitRange { off: 0, width: 0 },
            signed: false,
            align: 4,
        },
    });
    assert_rejects(&m, VerifyErrorKind::BadBitRange);
}

/// Rule 3: an unreachable block is a **warning**, not an error — unreachable C exists.
#[test]
fn unreachable_block_is_a_warning_not_an_error() {
    let mut m = valid_module();
    m.funcs[0]
        .blocks
        .push(block(7, vec![], Terminator::Return(None)));
    let errs = verify(&m);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, VerifyErrorKind::UnreachableBlock);
    assert!(
        !errs[0].is_error(),
        "unreachable code is legal C; this must not block execution"
    );
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "a module with only warnings is still executable"
    );
}

/// Rule 13.
#[test]
fn alloca_dyn_against_a_static_decl_is_rejected() {
    let mut m = valid_module();
    let f = &mut m.funcs[0];
    f.allocas.push(AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(32),
        count: 4, // static, but AllocaDyn claims to supply the extent
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    });
    f.blocks[0].insts.insert(
        0,
        inst(InstKind::AllocaDyn {
            dst: ValueId(5),
            alloca: AllocaId(0),
            elem: CTy::Int(32),
            count: i32c(8),
            align: 4,
        }),
    );
    assert_rejects(&m, VerifyErrorKind::AllocaExtentMismatch);
}

/// Verification must be deterministic (001 §5).
#[test]
fn error_order_is_deterministic() {
    let mut m = valid_module();
    // Two defects reported by the same pass. A dangling block reference deliberately
    // short-circuits (its diagnostics would be nonsense), so it cannot be used here.
    m.funcs[0].blocks[0].insts.push(inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Use(i32c(1)),
    }));
    m.funcs[0].blocks[0].insts.push(inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Use(i32c(2)),
    }));
    let a: Vec<_> = verify(&m).iter().map(|e| format!("{e:?}")).collect();
    let b: Vec<_> = verify(&m).iter().map(|e| format!("{e:?}")).collect();
    assert_eq!(a, b);
    assert!(a.len() >= 2);
}
