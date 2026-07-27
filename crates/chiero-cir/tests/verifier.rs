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
