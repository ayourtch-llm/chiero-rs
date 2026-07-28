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
        generated: false,
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
/// Retype the function to `void` so a bare `ret` is legal. Several fixtures neutralize
/// the terminator to isolate a defect, and a bare `ret` from an `i32` function is itself
/// a (correctly reported) width error.
fn make_void(m: &mut Module) {
    m.funcs[0].ret = CTy::Void;
}

fn assert_rejects(m: &Module, want: VerifyErrorKind) {
    let errs = verify(m);
    assert!(
        errs.iter().any(|e| e.kind == want),
        "expected {want:?}, got: {errs:#?}"
    );
    // At least one reported problem must actually be an *error*. Without this,
    // `is_error()` returning false unconditionally passes the entire suite and the
    // verifier silently becomes advisory noise while the corpus still reports clean.
    assert!(
        errs.iter().any(|e| e.is_error()),
        "a rejection must produce an error, not only warnings: {errs:#?}"
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
    f.ret = CTy::Void;
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
    make_void(&mut m);
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
    make_void(&mut m);
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
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert!(
        verify(&m).is_empty(),
        "128 bits either way is legal: {:?}",
        verify(&m)
    );
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
    f.ret = CTy::Void;
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
    // The dead block must be *well-typed*, or it now (correctly) also raises a width
    // error: `ret` with no value from an `i32` function is wrong wherever it appears.
    m.funcs[0]
        .blocks
        .push(block(7, vec![], Terminator::Return(Some(i32c(0)))));
    let errs = verify(&m);
    assert_eq!(errs.len(), 1, "{errs:#?}");
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

/// Rule 5: operand widths must match the declared `ty`. Nothing tested this — the whole
/// rule was unimplemented, and `add i32` over an i8 and an i64 verified clean.
#[test]
fn mismatched_operand_widths_are_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Bin {
            op: BinOp::Add,
            a: Operand::Const(Const::Int { bits: 8, val: 1 }),
            b: Operand::Const(Const::Int { bits: 64, val: 1 }),
            ty: CTy::Int(32),
        },
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

#[test]
fn a_non_boolean_branch_condition_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0]
        .blocks
        .push(block(1, vec![], Terminator::Return(None)));
    m.funcs[0].blocks[0].term = Terminator::Br {
        cond: i32c(1), // Int(32), not Int(1)
        t: BlockId(1),
        f: BlockId(1),
    };
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

#[test]
fn a_bare_return_from_a_typed_function_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

#[test]
fn select_arms_must_agree() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Select {
            cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
            t: i32c(1),
            f: Operand::Const(Const::Int { bits: 8, val: 1 }),
        },
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

/// Rule 6: memory addresses are pointer-typed. There was no test for this at all, and
/// making `require_ptr` a no-op passed the whole suite.
#[test]
fn a_non_pointer_store_address_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts.insert(
        0,
        inst(InstKind::Store {
            addr: i32c(0), // an integer, not a pointer
            val: i32c(0),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        }),
    );
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::BadPointerOperand);
}

/// **Through one indirection.** A value's type must survive `%1 = %0`, or rule 6 stops
/// applying the moment a pointer is copied — which is most real code.
#[test]
fn pointer_typing_survives_a_use() {
    let mut m = valid_module();
    make_void(&mut m);
    let f = &mut m.funcs[0];
    f.params.push(Param {
        value: ValueId(9),
        ty: CTy::Int(32),
    });
    f.blocks[0].insts = vec![
        inst(InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::Use(Operand::Value(ValueId(9))),
        }),
        inst(InstKind::Store {
            addr: Operand::Value(ValueId(0)), // an i32 laundered through a Use
            val: i32c(0),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        }),
    ];
    f.blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::BadPointerOperand);
}

/// Rule 12: lane indices are in range. Also untested before — deleting both lane checks
/// passed the whole suite.
#[test]
fn an_out_of_range_lane_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    let f = &mut m.funcs[0];
    f.params.push(Param {
        value: ValueId(9),
        ty: CTy::Vector {
            elem: Box::new(CTy::Int(32)),
            lanes: 4,
        },
    });
    f.blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::ExtractLane {
            v: Operand::Value(ValueId(9)),
            lane: 99,
        },
    });
    f.blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::BadLane);
}

/// The dominator **intersection**. The existing test uses sibling blocks with no join,
/// so the multi-predecessor meet — the canonical dominance case — was never exercised.
#[test]
fn a_diamond_join_needs_the_dominator_intersection() {
    let mut m = valid_module();
    make_void(&mut m);
    let f = &mut m.funcs[0];
    f.blocks[0].term = Terminator::Br {
        cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
        t: BlockId(1),
        f: BlockId(2),
    };
    // bb1 defines %1; bb3 joins bb1 and bb2 and uses it. %1 does not dominate bb3.
    f.blocks.push(block(
        1,
        vec![inst(InstKind::Assign {
            dst: ValueId(1),
            rv: RValue::Use(i32c(1)),
        })],
        Terminator::Goto(BlockId(3)),
    ));
    f.blocks
        .push(block(2, vec![], Terminator::Goto(BlockId(3))));
    f.blocks.push(block(
        3,
        vec![inst(InstKind::Assign {
            dst: ValueId(2),
            rv: RValue::Use(Operand::Value(ValueId(1))),
        })],
        Terminator::Return(None),
    ));
    assert_rejects(&m, VerifyErrorKind::UseNotDominated);
}

/// A value used in a **call argument** must be dominance-checked. Dropping call args
/// from the operand walk passed the whole suite.
#[test]
fn call_arguments_are_dominance_checked() {
    let mut m = valid_module();
    make_void(&mut m);
    // Variadic so the argument does not also trip the arity check; the defect under
    // test is the undominated use.
    m.funcs[0].variadic = true;
    m.funcs[0].blocks[0].insts.insert(
        0,
        inst(InstKind::Call {
            dst: None,
            callee: Callee::Direct(FuncId(0)),
            args: vec![Operand::Value(ValueId(0))], // defined *after* this line
        }),
    );
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::UseNotDominated);
}

/// An unreachable block is a warning even when it *uses* a value — otherwise "dead code
/// is legal" holds only for empty dead blocks, and real lowered C has non-empty ones.
#[test]
fn a_nonempty_unreachable_block_is_still_only_a_warning() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.funcs[0].blocks.push(block(
        7,
        vec![inst(InstKind::Assign {
            dst: ValueId(5),
            rv: RValue::Use(Operand::Value(ValueId(0))),
        })],
        Terminator::Return(None),
    ));
    let errs = verify(&m);
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "dead code must not be an error: {errs:#?}"
    );
    assert!(
        errs.iter()
            .any(|e| e.kind == VerifyErrorKind::UnreachableBlock)
    );
}

/// A duplicate `BlockId` is a *silently wrong execution*, not a crash: `block()` is a
/// linear find, so the second block is unreachable and control lands in the first.
#[test]
fn duplicate_ids_are_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.funcs[0]
        .blocks
        .push(block(0, vec![], Terminator::Return(None)));
    assert_rejects(&m, VerifyErrorKind::DuplicateId);

    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.funcs[0].params = vec![
        Param {
            value: ValueId(9),
            ty: CTy::Int(32),
        },
        Param {
            value: ValueId(9),
            ty: CTy::Int(32),
        },
    ];
    assert_rejects(&m, VerifyErrorKind::DuplicateId);
}

/// Rule 7 applies to *declarations*, not only to accesses.
#[test]
fn a_badly_aligned_alloca_declaration_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.funcs[0].allocas.push(AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(32),
        count: 1,
        align: 3,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    });
    assert_rejects(&m, VerifyErrorKind::BadAlignment);
}

/// Rule 13's *second* half: a runtime extent that nothing supplies leaves the object
/// unsized. Only the AllocaDyn-to-declaration direction was checked.
#[test]
fn a_runtime_extent_with_no_allocadyn_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.funcs[0].allocas.push(AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(32),
        count: DYNAMIC_EXTENT,
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    });
    assert_rejects(&m, VerifyErrorKind::AllocaExtentMismatch);
}

#[test]
fn addr_of_an_undeclared_local_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::AddrOfLocal {
            alloca: AllocaId(42),
        },
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::UnknownId);
}

/// `verify` took only a `&Function`, so **every module-level identity check was
/// missing**: two globals with one id, two functions with one id or one name, a call to
/// a function that does not exist, and an address of an undeclared global all produced
/// zero errors.
#[test]
fn module_level_identity_is_checked() {
    let g = |id: u32, name: &str| Global {
        id: GlobalId(id),
        name: name.into(),
        size: 8,
        align: 8,
        is_const: false,
        span: Span::DUMMY,
        init: Default::default(),
        linkage: Default::default(),
    };

    // Two globals sharing an id. Since ids must equal indices, this is necessarily
    // *two* defects — the second global is both a duplicate and out of position — so
    // the one-defect-one-kind rule does not apply and both kinds must appear.
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.globals = vec![g(0, "a"), g(0, "b")];
    let kinds: Vec<_> = verify(&m).iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&VerifyErrorKind::DuplicateId), "{kinds:?}");
    assert!(kinds.contains(&VerifyErrorKind::IdNotIndex), "{kinds:?}");

    // Two functions sharing a name — `func_id` resolves to the first, silently.
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    let mut second = m.funcs[0].clone();
    second.id = FuncId(1);
    m.funcs.push(second);
    assert_rejects(&m, VerifyErrorKind::DuplicateId);
}

#[test]
fn dangling_references_are_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Call {
        dst: None,
        callee: Callee::Direct(FuncId(42)),
        args: vec![],
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::UnknownId);

    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::AddrOfGlobal { g: GlobalId(7) },
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::UnknownId);
}

/// Call arity. Against `func @g(%0: i32, %1: i32)`, both a call with no arguments and
/// one with three verified clean.
#[test]
fn call_arity_is_checked() {
    let two_params = |m: &mut Module| {
        let mut callee = m.funcs[0].clone();
        callee.id = FuncId(1);
        callee.name = "callee".into();
        callee.params = vec![
            Param {
                value: ValueId(20),
                ty: CTy::Int(32),
            },
            Param {
                value: ValueId(21),
                ty: CTy::Int(32),
            },
        ];
        callee.body = Body::Declared;
        callee.blocks.clear();
        m.funcs.push(callee);
    };

    for args in [vec![], vec![i32c(1), i32c(2), i32c(3)]] {
        let mut m = valid_module();
        make_void(&mut m);
        two_params(&mut m);
        m.funcs[0].blocks[0].insts[0] = inst(InstKind::Call {
            dst: None,
            callee: Callee::Direct(FuncId(1)),
            args,
        });
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        assert_rejects(&m, VerifyErrorKind::CallArity);
    }

    // The right arity verifies, and a variadic callee accepts extras.
    let mut m = valid_module();
    make_void(&mut m);
    two_params(&mut m);
    m.funcs[1].variadic = true;
    m.funcs[0].blocks[0].insts[0] = inst(InstKind::Call {
        dst: None,
        callee: Callee::Direct(FuncId(1)),
        args: vec![i32c(1), i32c(2), i32c(3)],
    });
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert!(verify(&m).iter().all(|e| !e.is_error()), "{:?}", verify(&m));
}

/// `successors()` feeds reachability and the dominance scan, so dropping the switch
/// `default` would make every default target look unreachable.
#[test]
fn switch_default_is_a_successor() {
    let t = Terminator::Switch {
        scrut: i32c(0),
        ty: CTy::Int(32),
        cases: vec![(1, BlockId(1))],
        default: BlockId(2),
    };
    assert!(
        t.successors().contains(&BlockId(2)),
        "default is a successor"
    );
    // And the order is deterministic (001 §5).
    assert_eq!(t.successors(), vec![BlockId(1), BlockId(2)]);
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

// ---------------------------------------------------------------------------
// Wave 7. Six defects found by the third mutation review, each confirmed by probe
// before being treated as a finding.
// ---------------------------------------------------------------------------

/// **The dominance lattice is wrong in the presence of dead code.**
///
/// An unreachable block is dominated by nothing but itself, so `dom[dead] = {dead}`.
/// Meeting that into a *live* join empties the set, and a value defined in the entry
/// block stops dominating its use. The shape — dead code falling into a live join —
/// is ubiquitous in real C, and this is a hard error, so the module is rejected and
/// never runs. `chiero-lower` would trip on its first real function.
///
/// The fix is standard Cooper-Harvey-Kennedy: unreachable predecessors are not part
/// of the meet.
#[test]
fn a_dead_predecessor_does_not_break_dominance_for_a_live_block() {
    let mut m = valid_module();
    let f = &mut m.funcs[0];
    // entry: %0 = add 2, 3; goto bb1     bb1: ret %0     bb2 (dead): goto bb1
    f.blocks[0].term = Terminator::Goto(BlockId(1));
    f.blocks.push(block(
        1,
        vec![],
        Terminator::Return(Some(Operand::Value(ValueId(0)))),
    ));
    f.blocks
        .push(block(2, vec![], Terminator::Goto(BlockId(1))));

    let errs: Vec<_> = verify(&m).into_iter().filter(|e| e.is_error()).collect();
    assert!(
        errs.is_empty(),
        "entry dominates bb1 regardless of the dead bb2; got: {errs:#?}"
    );
    // The dead block is still worth a warning — that part was already right.
    assert!(
        verify(&m)
            .iter()
            .any(|e| e.kind == VerifyErrorKind::UnreachableBlock)
    );
}

/// Alignment and cast shape have nothing to do with reachability. The `continue` that
/// suppresses spurious dominance errors in dead blocks also skips `check_inst_types`,
/// so a dead block is a hole in rules 5, 6, 7, 11 and 12. Dead code is legal, so it
/// reaches the engine's data structures like any other.
#[test]
fn type_rules_still_apply_inside_an_unreachable_block() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    m.funcs[0].params = vec![Param {
        value: ValueId(9),
        ty: CTy::Ptr,
    }];
    m.funcs[0].blocks.push(block(
        1,
        vec![inst(InstKind::Store {
            addr: Operand::Value(ValueId(9)),
            val: i32c(0),
            ty: CTy::Int(32),
            align: 3, // not a power of two
            vol: Volatility::Normal,
        })],
        Terminator::Return(None),
    ));
    let errs = verify(&m);
    assert!(
        errs.iter().any(|e| e.kind == VerifyErrorKind::BadAlignment),
        "align 3 is invalid wherever it appears; got: {errs:#?}"
    );
}

/// **Rule 5 does not reach store values at all.** `Store` and `StoreBits` carry a
/// declared `ty`/`unit`, and the value written through them is never checked against
/// it. Storing an i64 through `store i32` is exactly the malformed IR the verifier
/// exists to stop before the memory model has to guess a truncation.
#[test]
fn a_store_value_wider_than_its_declared_type_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].params = vec![Param {
        value: ValueId(9),
        ty: CTy::Ptr,
    }];
    m.funcs[0].blocks[0].insts = vec![inst(InstKind::Store {
        addr: Operand::Value(ValueId(9)),
        val: Operand::Const(Const::Int { bits: 64, val: 0 }),
        ty: CTy::Int(32),
        align: 4,
        vol: Volatility::Normal,
    })];
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

/// Rule 5 for `SetMem`: the fill byte is a byte. An `i32` here means the memory model
/// has to invent a narrowing rule of its own.
#[test]
fn a_setmem_fill_byte_that_is_not_a_byte_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].params = vec![Param {
        value: ValueId(9),
        ty: CTy::Ptr,
    }];
    m.funcs[0].blocks[0].insts = vec![inst(InstKind::SetMem {
        dst: Operand::Value(ValueId(9)),
        byte: i32c(0),
        size: Operand::Const(Const::Int { bits: 64, val: 8 }),
    })];
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

/// **`verify` must check every function, not the first one.** Restricting the module
/// loop to `.take(1)` survived the entire suite, because every verifier fixture breaks
/// `funcs[0]` and every corpus fixture is clean. A bug in the loop bound is invisible.
#[test]
fn a_defect_in_the_second_function_is_still_reported() {
    let mut m = valid_module();
    let mut g = m.funcs[0].clone();
    g.id = FuncId(1);
    g.name = "g".into();
    g.ret = CTy::Void;
    g.blocks[0].term = Terminator::Goto(BlockId(77));
    m.funcs.push(g);
    let errs = verify(&m);
    assert!(
        errs.iter()
            .any(|e| e.kind == VerifyErrorKind::UnknownBlock && e.func == FuncId(1)),
        "a defect in funcs[1] must be reported; got: {errs:#?}"
    );
}

/// 020 §3 says functions and globals are "indexed by `FuncId`"/"indexed by `GlobalId`".
/// Nothing enforced it, and the two halves of the crate disagree about which it is: the
/// printer resolves positionally (`m.globals[g.0]`) while the verifier resolves by
/// `.id`. A module whose ids are permuted verifies clean and then prints the *wrong
/// name* for every reference. Make the convention an invariant.
#[test]
fn a_function_whose_id_is_not_its_index_is_rejected() {
    let mut m = valid_module();
    let mut g = m.funcs[0].clone();
    g.id = FuncId(1);
    g.name = "g".into();
    m.funcs.push(g);
    m.funcs.swap(0, 1); // ids are now [1, 0]; both still unique
    assert_rejects(&m, VerifyErrorKind::IdNotIndex);
}

/// `Opaque`'s declared writes are memory regions: rule 6 applies to the address and
/// rule 5 to the size, exactly as they do for `copymem`. An `Opaque` that havocs a
/// region described by an integer "address" would leave the memory model guessing.
#[test]
fn an_opaque_write_to_a_non_pointer_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts = vec![inst(InstKind::Opaque {
        dsts: vec![],
        writes: vec![OpaqueWrite {
            addr: i32c(4), // not a pointer
            size: Operand::Const(Const::Int { bits: 64, val: 8 }),
        }],
        reads: vec![],
        why: OpaqueReason::InlineAsm,
    })];
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::BadPointerOperand);
}

/// **020 §4.3: `Opaque` must never be silently equivalent to a no-op** — a no-op would
/// let a checker "prove" something about code it did not understand. An `Opaque` that
/// declares nothing at all is that no-op written down, so it is rejected rather than
/// accepted and quietly ignored.
#[test]
fn an_opaque_that_declares_no_effect_at_all_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts = vec![inst(InstKind::Opaque {
        dsts: vec![],
        writes: vec![],
        reads: vec![],
        why: OpaqueReason::InlineAsm,
    })];
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::OpaqueWithoutEffect);
}

/// **Rule 1 at every operand position.**
///
/// The rule was *applied* at twelve operand positions and *tested* at two. Nine
/// mutations that deleted an operand from the dominance walk all survived — including
/// `AllocaDyn::count`, which 020 contract 40 is written specifically about: "`AllocaDyn`'s
/// `count` operand is subject to verifier rule 1 like any other use, and a module where
/// it is defined in a non-dominating block is rejected." The code happened to be right
/// and nothing proved it, which is the same thing as not having the rule.
///
/// Each case plants a value defined in `bb1` into one operand position in `entry`. Since
/// `entry` precedes `bb1`, the definition cannot dominate the use.
#[test]
fn rule_one_applies_at_every_operand_position() {
    // The undominated value, defined in bb1 and used in entry.
    let far = ValueId(50);
    let far_op = Operand::Value(far);

    let cases: Vec<(&str, Vec<Inst>, Terminator)> = vec![
        (
            "store value",
            vec![inst(InstKind::Store {
                addr: Operand::Const(Const::Null),
                val: far_op.clone(),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "copymem size",
            vec![inst(InstKind::CopyMem {
                dst: Operand::Const(Const::Null),
                src: Operand::Const(Const::Null),
                size: far_op.clone(),
                align: 8,
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "setmem fill byte",
            vec![inst(InstKind::SetMem {
                dst: Operand::Const(Const::Null),
                byte: far_op.clone(),
                size: Operand::Const(Const::Int { bits: 64, val: 8 }),
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            // 020 contract 40, by name.
            "allocadyn count",
            vec![inst(InstKind::AllocaDyn {
                dst: ValueId(30),
                alloca: AllocaId(0),
                elem: CTy::Int(8),
                count: far_op.clone(),
                align: 8,
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "bin rhs",
            vec![inst(InstKind::Assign {
                dst: ValueId(31),
                rv: RValue::Bin {
                    op: BinOp::Add,
                    a: i32c(2),
                    b: far_op.clone(),
                    ty: CTy::Int(32),
                },
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "select cond",
            vec![inst(InstKind::Assign {
                dst: ValueId(32),
                rv: RValue::Select {
                    cond: far_op.clone(),
                    t: i32c(1),
                    f: i32c(2),
                },
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "ptradd offset",
            vec![inst(InstKind::Assign {
                dst: ValueId(33),
                rv: RValue::PtrAdd {
                    base: Operand::Const(Const::Null),
                    off: far_op.clone(),
                },
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "opaque write address",
            vec![inst(InstKind::Opaque {
                dsts: vec![],
                writes: vec![OpaqueWrite {
                    addr: far_op.clone(),
                    size: Operand::Const(Const::Int { bits: 64, val: 8 }),
                }],
                reads: vec![],
                why: OpaqueReason::InlineAsm,
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "opaque read",
            vec![inst(InstKind::Opaque {
                dsts: vec![],
                writes: vec![],
                reads: vec![far_op.clone()],
                why: OpaqueReason::InlineAsm,
            })],
            Terminator::Goto(BlockId(1)),
        ),
        (
            "switch scrutinee",
            vec![],
            Terminator::Switch {
                ty: CTy::Int(32),
                scrut: far_op.clone(),
                cases: vec![(1, BlockId(1))],
                default: BlockId(1),
            },
        ),
        (
            "branch condition",
            vec![],
            Terminator::Br {
                cond: far_op.clone(),
                t: BlockId(1),
                f: BlockId(1),
            },
        ),
    ];

    for (what, insts, term) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].allocas = vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(8),
            count: 4,
            align: 8,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }];
        m.funcs[0].blocks[0].insts = insts;
        m.funcs[0].blocks[0].term = term;
        // bb1 defines the value, *after* entry has already used it.
        m.funcs[0].blocks.push(block(
            1,
            vec![inst(InstKind::Assign {
                dst: far,
                rv: RValue::Bin {
                    op: BinOp::Add,
                    a: i32c(1),
                    b: i32c(1),
                    ty: CTy::Int(32),
                },
            })],
            Terminator::Return(None),
        ));

        let errs = verify(&m);
        assert!(
            errs.iter()
                .any(|e| e.kind == VerifyErrorKind::UseNotDominated),
            "rule 1 does not reach the {what} operand; got: {errs:#?}"
        );
    }
}

/// The companion property: rule 1 must not fire on a *dominated* use in any of the same
/// positions. Without this, a verifier that reported `UseNotDominated` unconditionally
/// would pass the test above — and the whole corpus would stop verifying, which is a
/// louder failure but not one the test itself could distinguish.
#[test]
fn rule_one_accepts_a_dominated_use_in_the_same_positions() {
    let mut m = valid_module();
    make_void(&mut m);
    let near = Operand::Value(ValueId(0)); // defined by valid_module in entry
    m.funcs[0].blocks[0].insts.push(inst(InstKind::Store {
        addr: Operand::Const(Const::Null),
        val: near.clone(),
        ty: CTy::Int(32),
        align: 4,
        vol: Volatility::Normal,
    }));
    m.funcs[0].blocks[0].term = Terminator::Br {
        cond: near,
        t: BlockId(1),
        f: BlockId(1),
    };
    m.funcs[0]
        .blocks
        .push(block(1, vec![], Terminator::Return(None)));
    let errs: Vec<_> = verify(&m)
        .into_iter()
        .filter(|e| e.kind == VerifyErrorKind::UseNotDominated)
        .collect();
    assert!(
        errs.is_empty(),
        "a dominated use must be accepted: {errs:#?}"
    );
}

/// **`va_list` operands are never pointer-checked**, so `vastart 0i32` verifies clean.
/// 020 §4.4.1 makes the `va_list` a real addressable `MemObject`, and VPP has 2552
/// `va_list *` uses — an integer where the list should be is not a hypothetical.
#[test]
fn valist_operands_must_be_pointers() {
    let cases: Vec<(&str, InstKind)> = vec![
        ("vastart", InstKind::VaStart { list: i32c(0) }),
        ("vaend", InstKind::VaEnd { list: i32c(0) }),
        (
            "vacopy dst",
            InstKind::VaCopy {
                dst: i32c(0),
                src: Operand::Const(Const::Null),
            },
        ),
        (
            "vacopy src",
            InstKind::VaCopy {
                dst: Operand::Const(Const::Null),
                src: i32c(0),
            },
        ),
        (
            "vaarg list",
            InstKind::VaArg {
                dst: ValueId(20),
                list: i32c(0),
                ty: CTy::Int(32),
            },
        ),
    ];
    for (what, kind) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].blocks[0].insts = vec![inst(kind)];
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        let errs = verify(&m);
        assert!(
            errs.iter()
                .any(|e| e.kind == VerifyErrorKind::BadPointerOperand),
            "{what} accepted a non-pointer va_list; got: {errs:#?}"
        );
    }
}

/// Cast shape (rule 12) at every `CastKind`. The rules were implemented and correct;
/// nothing exercised six of the ten, so a refactor could invert one silently. The
/// width-changing cases also need their *boundary*: `trunc i32 -> i32` must be rejected,
/// because "narrows" means strictly, and an equal-width `trunc` is a `bitcast` wearing
/// the wrong name.
#[test]
fn every_cast_kind_rejects_the_wrong_shape() {
    let f32t = CTy::Float(FloatKind::F32);
    let f64t = CTy::Float(FloatKind::F64);
    let cases: Vec<(&str, CastKind, CTy, CTy)> = vec![
        // Width-changing casts at the equal-width boundary.
        (
            "trunc equal width",
            CastKind::Trunc,
            CTy::Int(32),
            CTy::Int(32),
        ),
        (
            "zext equal width",
            CastKind::ZExt,
            CTy::Int(32),
            CTy::Int(32),
        ),
        ("sext narrowing", CastKind::SExt, CTy::Int(32), CTy::Int(8)),
        (
            "fptrunc widening",
            CastKind::FpTrunc,
            f32t.clone(),
            f64t.clone(),
        ),
        (
            "fpext narrowing",
            CastKind::FpExt,
            f64t.clone(),
            f32t.clone(),
        ),
        (
            "bitcast width change",
            CastKind::Bitcast,
            CTy::Int(32),
            CTy::Int(64),
        ),
        // Shape casts, each given the reverse of its legal direction.
        (
            "ptrtoint from int",
            CastKind::PtrToInt,
            CTy::Int(64),
            CTy::Int(64),
        ),
        ("ptrtoint to ptr", CastKind::PtrToInt, CTy::Ptr, CTy::Ptr),
        ("inttoptr from ptr", CastKind::IntToPtr, CTy::Ptr, CTy::Ptr),
        (
            "inttoptr to int",
            CastKind::IntToPtr,
            CTy::Int(64),
            CTy::Int(64),
        ),
        (
            "fptoui from int",
            CastKind::FpToUi,
            CTy::Int(32),
            CTy::Int(32),
        ),
        (
            "fptosi to float",
            CastKind::FpToSi,
            f64t.clone(),
            f64t.clone(),
        ),
        (
            "uitofp from float",
            CastKind::UiToFp,
            f64t.clone(),
            f64t.clone(),
        ),
        (
            "sitofp to int",
            CastKind::SiToFp,
            CTy::Int(32),
            CTy::Int(32),
        ),
    ];
    for (what, kind, from, to) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].blocks[0].insts = vec![inst(InstKind::Assign {
            dst: ValueId(21),
            rv: RValue::Cast {
                kind,
                a: Operand::Const(Const::Undef(from.clone())),
                from,
                to,
            },
        })];
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        let errs = verify(&m);
        assert!(
            errs.iter().any(|e| e.kind == VerifyErrorKind::BadCast),
            "{what} was accepted; got: {errs:#?}"
        );
    }
}

/// The companion: every cast kind must *accept* its legal shape, or the test above is
/// satisfied by a verifier that rejects all casts.
#[test]
fn every_cast_kind_accepts_the_right_shape() {
    let f32t = CTy::Float(FloatKind::F32);
    let f64t = CTy::Float(FloatKind::F64);
    let cases: Vec<(&str, CastKind, CTy, CTy)> = vec![
        ("trunc", CastKind::Trunc, CTy::Int(32), CTy::Int(8)),
        ("zext", CastKind::ZExt, CTy::Int(8), CTy::Int(32)),
        ("sext", CastKind::SExt, CTy::Int(8), CTy::Int(32)),
        ("fptrunc", CastKind::FpTrunc, f64t.clone(), f32t.clone()),
        ("fpext", CastKind::FpExt, f32t.clone(), f64t.clone()),
        ("bitcast", CastKind::Bitcast, CTy::Int(32), f32t.clone()),
        ("ptrtoint", CastKind::PtrToInt, CTy::Ptr, CTy::Int(64)),
        ("inttoptr", CastKind::IntToPtr, CTy::Int(64), CTy::Ptr),
        ("fptoui", CastKind::FpToUi, f64t.clone(), CTy::Int(32)),
        ("fptosi", CastKind::FpToSi, f64t.clone(), CTy::Int(32)),
        ("uitofp", CastKind::UiToFp, CTy::Int(32), f64t.clone()),
        ("sitofp", CastKind::SiToFp, CTy::Int(32), f64t.clone()),
    ];
    for (what, kind, from, to) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].blocks[0].insts = vec![inst(InstKind::Assign {
            dst: ValueId(21),
            rv: RValue::Cast {
                kind,
                a: Operand::Const(Const::Undef(from.clone())),
                from,
                to,
            },
        })];
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        let errs: Vec<_> = verify(&m)
            .into_iter()
            .filter(|e| e.kind == VerifyErrorKind::BadCast)
            .collect();
        assert!(errs.is_empty(), "{what} was rejected: {errs:#?}");
    }
}

// ---------------------------------------------------------------------------
// Wave 8, from the chiero-mem mutation review. Each probed before being acted on.
// ---------------------------------------------------------------------------

/// **A `Cast`'s declared `from` is never checked against its operand.**
///
/// This is the same failure class commit 09ae8fd closed for `va_list` — a declared type
/// the verifier takes on faith — left open on the `PtrToInt`/`IntToPtr` pair, which is
/// exactly the pair 021 §7.1 makes carry *provenance*. A lowering bug that mislabels
/// `from` produces a module that verifies clean and then mints an unprovenanced pointer,
/// and the memory model has no way to notice.
#[test]
fn a_cast_operand_must_match_its_declared_source_type() {
    let cases: Vec<(&str, CastKind, Operand, CTy, CTy)> = vec![
        (
            "ptrtoint of an integer declared as a pointer",
            CastKind::PtrToInt,
            i32c(7),
            CTy::Ptr,
            CTy::Int(64),
        ),
        (
            "trunc of a null declared as an i64",
            CastKind::Trunc,
            Operand::Const(Const::Null),
            CTy::Int(64),
            CTy::Int(32),
        ),
    ];
    for (what, kind, a, from, to) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].blocks[0].insts = vec![inst(InstKind::Assign {
            dst: ValueId(21),
            rv: RValue::Cast { kind, a, from, to },
        })];
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        let errs = verify(&m);
        assert!(!errs.is_empty(), "{what} must be rejected; got: {errs:#?}");
    }
}

/// **Width-changing casts never check the int/float domain.** `trunc f64 -> i32` and
/// `fptosi f64 -> i32` compute completely different values, so accepting the first is the
/// same error as the equal-width `trunc` this suite already rejects — a cast wearing
/// another cast's name — only with a wrong *value* rather than a redundant one.
#[test]
fn width_casts_must_stay_within_their_domain() {
    let f32t = CTy::Float(FloatKind::F32);
    let f64t = CTy::Float(FloatKind::F64);
    let cases: Vec<(&str, CastKind, CTy, CTy)> = vec![
        (
            "trunc f64 -> i32 is fptosi",
            CastKind::Trunc,
            f64t.clone(),
            CTy::Int(32),
        ),
        (
            "trunc f64 -> f32 is fptrunc",
            CastKind::Trunc,
            f64t.clone(),
            f32t.clone(),
        ),
        (
            "fptrunc i64 -> i32 is trunc",
            CastKind::FpTrunc,
            CTy::Int(64),
            CTy::Int(32),
        ),
        (
            "zext f32 -> i64",
            CastKind::ZExt,
            f32t.clone(),
            CTy::Int(64),
        ),
        (
            "sext i32 -> f64",
            CastKind::SExt,
            CTy::Int(32),
            f64t.clone(),
        ),
        (
            "fpext i32 -> i64",
            CastKind::FpExt,
            CTy::Int(32),
            CTy::Int(64),
        ),
    ];
    for (what, kind, from, to) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].blocks[0].insts = vec![inst(InstKind::Assign {
            dst: ValueId(21),
            rv: RValue::Cast {
                kind,
                a: Operand::Const(Const::Undef(from.clone())),
                from,
                to,
            },
        })];
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        assert!(
            verify(&m)
                .iter()
                .any(|e| e.kind == VerifyErrorKind::BadCast),
            "{what} was accepted"
        );
    }
}

/// **`vaarg %x : void` mints a universal type-checking wildcard.**
///
/// `Void` is the deliberate escape hatch for values whose type the verifier cannot
/// resolve, so every rule downstream of such a value silently stops applying. `VaArg`
/// *declares* its result type, so it can hand out `Void` on request — one
/// `vaarg … : void` disables rules 5 and 6 for everything derived from it. A `va_arg` of
/// `void` is meaningless in C anyway.
#[test]
fn a_vaarg_of_void_is_rejected() {
    let mut m = valid_module();
    make_void(&mut m);
    m.funcs[0].blocks[0].insts = vec![inst(InstKind::VaArg {
        dst: ValueId(30),
        list: Operand::Const(Const::Null),
        ty: CTy::Void,
    })];
    m.funcs[0].blocks[0].term = Terminator::Return(None);
    assert_rejects(&m, VerifyErrorKind::WidthMismatch);
}

/// The acceptance half the `va_list` commit argued for and then did not write. A verifier
/// that rejects **every** varargs instruction — making all 2552 VPP `va_list *` sites
/// unverifiable — passes the rejection test on its own.
#[test]
fn valist_instructions_accept_a_pointer_list() {
    let null = Operand::Const(Const::Null);
    let cases: Vec<(&str, InstKind)> = vec![
        ("vastart", InstKind::VaStart { list: null.clone() }),
        ("vaend", InstKind::VaEnd { list: null.clone() }),
        (
            "vacopy",
            InstKind::VaCopy {
                dst: null.clone(),
                src: null.clone(),
            },
        ),
        (
            "vaarg",
            InstKind::VaArg {
                dst: ValueId(20),
                list: null.clone(),
                ty: CTy::Int(32),
            },
        ),
    ];
    for (what, kind) in cases {
        let mut m = valid_module();
        make_void(&mut m);
        m.funcs[0].blocks[0].insts = vec![inst(kind)];
        m.funcs[0].blocks[0].term = Terminator::Return(None);
        let errs = verify(&m);
        assert!(
            errs.is_empty(),
            "{what} with a pointer list must verify: {errs:#?}"
        );
    }
}

/// **020 §4.1 declares `ShuffleDyn` and `RValue` does not have it.** It is the general
/// form — `__builtin_shuffle(v, mask_vector)` with a runtime mask — and 020 contract 33's
/// second half is unimplementable without it: "a value constrained to be some permutation
/// of the input lanes, and `Approximated`". Found by review, which noticed that "`eval` is
/// exhaustive" is exhaustive over a *smaller enum than the spec's*, so the compile-time
/// guarantee that wave 31 added does not cover it.
///
/// This test is the reminder that the two disagree. It asserts the constant-mask form is
/// present, so it fails the day someone deletes that too, and it exists to be replaced by
/// a real `ShuffleDyn` test rather than to stay.
#[test]
fn the_constant_mask_shuffle_exists_and_the_dynamic_one_is_owed() {
    let mut m = valid_module();
    m.funcs[0].params = vec![Param {
        value: ValueId(9),
        ty: CTy::Vector {
            elem: Box::new(CTy::Int(8)),
            lanes: 4,
        },
    }];
    m.funcs[0].blocks[0].insts.insert(
        0,
        inst(InstKind::Assign {
            dst: ValueId(1),
            rv: RValue::Shuffle {
                a: Operand::Value(ValueId(9)),
                b: Operand::Value(ValueId(9)),
                mask: vec![0, 1, 2, 3],
            },
        }),
    );
    assert!(
        verify(&m).iter().all(|e| !e.kind.is_error()),
        "the constant-mask form verifies: {:#?}",
        verify(&m)
    );
}

/// **020 contract 27.** `verify` rejects a `BitRange` whose `off + width` exceeds the unit
/// width, and one with `width == 0`. Both are the shape that would otherwise reach the
/// engine's bit API and read or write bits that are not part of the field — a zero-width
/// load in particular yields a zero-width term, which the arena has no representation for.
#[test]
fn verify_rejects_a_bit_range_that_does_not_fit_its_unit() {
    // **Both call sites**, and the *accept* side too. The first version built only a
    // `LoadBits`, so deleting `check_bits` at the `StoreBits` site survived — the wave-7
    // clustering failure again, one rule with two call sites and one of them tested. And
    // nothing pinned that a **full-width** bitfield is accepted, so `> w` → `>= w`
    // survived: a one-character slip would reject legal CIR for `unsigned x : 32`.
    for (off, width, why) in [
        (30u32, 4u32, "past the end"),
        (0, 0, "zero width"),
        (0, 32, "full width, which is legal"),
    ] {
        let expect_error = width != 32;
        let mut m = valid_module();
        m.funcs[0].allocas = vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 1,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }];
        m.funcs[0].blocks[0].insts.insert(
            0,
            inst(InstKind::Assign {
                dst: ValueId(5),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            }),
        );
        m.funcs[0].blocks[0].insts.insert(
            1,
            inst(InstKind::Assign {
                dst: ValueId(6),
                rv: RValue::LoadBits {
                    addr: Operand::Value(ValueId(5)),
                    unit: CTy::Int(32),
                    bits: BitRange { off, width },
                    signed: false,
                    align: 4,
                },
            }),
        );
        assert_eq!(
            verify(&m).iter().any(|e| e.kind.is_error()),
            expect_error,
            "{why}: off {off} width {width}"
        );

        // The same range through a `StoreBits`, which is a second call site of the same
        // rule and was entirely unchecked.
        let mut m2 = m.clone();
        m2.funcs[0].blocks[0].insts[1] = inst(InstKind::StoreBits {
            addr: Operand::Value(ValueId(5)),
            val: i32c(1),
            unit: CTy::Int(32),
            bits: BitRange { off, width },
            align: 4,
        });
        assert_eq!(
            verify(&m2).iter().any(|e| e.kind.is_error()),
            expect_error,
            "{why} (StoreBits): off {off} width {width}"
        );
    }
}

/// **A `BitRange` whose `off + width` overflows is rejected, not accepted.** It panicked in
/// debug and wrapped in release — `u32::MAX + 4` is 2, so `2 > 32` was false and `verify`
/// **accepted** a malformed range that then reached the engine's bit API. Wave 5 recorded
/// "20 malformed inputs panicked instead of erroring" as a fixed class in this crate; this
/// is one that got away, and the release behaviour is the worse half. Found by review.
#[test]
fn a_bit_range_whose_end_overflows_is_rejected() {
    let mut m = valid_module();
    m.funcs[0].allocas = vec![AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(32),
        count: 1,
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    }];
    m.funcs[0].blocks[0].insts.insert(
        0,
        inst(InstKind::Assign {
            dst: ValueId(5),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        }),
    );
    m.funcs[0].blocks[0].insts.insert(
        1,
        inst(InstKind::Assign {
            dst: ValueId(6),
            rv: RValue::LoadBits {
                addr: Operand::Value(ValueId(5)),
                unit: CTy::Int(32),
                bits: BitRange {
                    off: u32::MAX,
                    width: 4,
                },
                signed: false,
                align: 4,
            },
        }),
    );
    assert!(
        verify(&m).iter().any(|e| e.kind.is_error()),
        "an overflowing range is malformed, not permitted"
    );
}

/// **A bitfield unit that is not an integer is rejected.** `check_bits` filtered on
/// `bit_width()`, which `CTy::Float` and `CTy::Vector` both answer — so
/// `LoadBits { unit: f32, bits: 0..32 }` verified clean. C has no float bitfields at all,
/// and the engine would then read bits out of a value 023 §7 says it only approximates.
/// The contract-27 work fixed both call sites of this function and left this branch of it
/// untested. Found by review.
#[test]
fn a_bitfield_unit_must_be_an_integer() {
    for (unit, ok) in [
        (CTy::Int(32), true),
        (CTy::Float(FloatKind::F32), false),
        (
            CTy::Vector {
                elem: Box::new(CTy::Int(8)),
                lanes: 4,
            },
            false,
        ),
    ] {
        let mut m = valid_module();
        m.funcs[0].allocas = vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 1,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }];
        m.funcs[0].blocks[0].insts.insert(
            0,
            inst(InstKind::Assign {
                dst: ValueId(5),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            }),
        );
        m.funcs[0].blocks[0].insts.insert(
            1,
            inst(InstKind::Assign {
                dst: ValueId(6),
                rv: RValue::LoadBits {
                    addr: Operand::Value(ValueId(5)),
                    unit: unit.clone(),
                    bits: BitRange { off: 0, width: 8 },
                    signed: false,
                    align: 4,
                },
            }),
        );
        assert_eq!(
            verify(&m).iter().all(|e| !e.kind.is_error()),
            ok,
            "unit {unit:?}"
        );
    }
}
