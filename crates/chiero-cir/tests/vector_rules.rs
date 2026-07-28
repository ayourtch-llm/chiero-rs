//! Vector shape rules in the verifier — 020 contract 12.
//!
//! Covers: 020 contract 12.
//!
//! "`Bitcast` preserves total bit width; `Shuffle`'s mask indices are `< 2 * lanes` and
//! `InsertLane`/`ExtractLane`'s `lane < lanes`."
//!
//! These are the rules that stop a lowering bug from becoming a silently wrong answer. A
//! `Bitcast` between differently-sized types has no meaning to give the engine, and an
//! out-of-range lane index reads whatever the lane extraction happens to compute — in a
//! bit-precise model, some other lane's bits.
//!
//! Every rule here is tested in **both directions**. A verifier that rejects everything
//! satisfies the rejection half, and 020's own note about contract 29 makes the general
//! point: "an unquantified 'produces findings' is satisfied by a checker that never
//! fires", and its mirror image is a checker that always does.

use chiero_cir::*;
use chiero_span::Span;

fn u8x16() -> CTy {
    CTy::Vector {
        elem: Box::new(CTy::Int(8)),
        lanes: 16,
    }
}

fn u32x4() -> CTy {
    CTy::Vector {
        elem: Box::new(CTy::Int(32)),
        lanes: 4,
    }
}

/// A one-block function holding `insts`, returning nothing.
fn module_with(insts: Vec<Inst>) -> Module {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts,
            term: Terminator::Return(None),
            gcov_lines: Default::default(),
            span: Span::DUMMY,
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![f],
        ..Default::default()
    }
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

/// A `u8x16` in `%0`, built by splatting — the operand every rule below needs a *typed*
/// vector for. The verifier resolves an operand's type from the instruction that defined
/// it, so a rule keyed on `CTy::Vector` never fires for a value it cannot type, and a
/// fixture that skips this step tests nothing.
fn splat_u8x16() -> Inst {
    assign(
        0,
        RValue::Splat {
            elem: Operand::Const(Const::Int { bits: 8, val: 1 }),
            lanes: 16,
        },
    )
}

fn errs(m: &Module) -> Vec<String> {
    verify(m).iter().map(|e| format!("{e:?}")).collect()
}

/// **020 contract 12, `Bitcast`.** Total bit width is preserved: `u8x16` ↔ `u32x4` is
/// legal, `u8x16` → `u32` is not.
#[test]
fn bitcast_must_preserve_total_bit_width() {
    let ok = module_with(vec![
        splat_u8x16(),
        assign(
            1,
            RValue::Cast {
                kind: CastKind::Bitcast,
                a: Operand::Value(ValueId(0)),
                from: u8x16(),
                to: u32x4(),
            },
        ),
    ]);
    assert!(
        verify(&ok).is_empty(),
        "128 bits to 128 bits is exactly what `Bitcast` is for: {:?}",
        errs(&ok)
    );

    let bad = module_with(vec![
        splat_u8x16(),
        assign(
            1,
            RValue::Cast {
                kind: CastKind::Bitcast,
                a: Operand::Value(ValueId(0)),
                from: u8x16(),
                to: CTy::Int(32),
            },
        ),
    ]);
    assert!(
        !verify(&bad).is_empty(),
        "128 bits reinterpreted as 32 discards three quarters of the value in silence"
    );
}

/// **020 contract 12, `Shuffle`.** A mask index selects from the *concatenation* of both
/// operands, so the bound is `2 * lanes` and not `lanes` — an off-by-one-vector here
/// would reject every shuffle that reads from its second operand, which is most of them.
#[test]
fn a_shuffle_mask_selects_from_both_operands_and_no_further() {
    let ok = module_with(vec![
        splat_u8x16(),
        assign(
            1,
            RValue::Shuffle {
                a: Operand::Value(ValueId(0)),
                b: Operand::Value(ValueId(0)),
                // 31 is the last lane of the second operand: legal, and the index a
                // `< lanes` bound would wrongly reject.
                mask: vec![0, 15, 16, 31],
            },
        ),
    ]);
    assert!(
        verify(&ok).is_empty(),
        "indices up to 2*lanes-1 name the second operand's lanes: {:?}",
        errs(&ok)
    );

    let bad = module_with(vec![
        splat_u8x16(),
        assign(
            1,
            RValue::Shuffle {
                a: Operand::Value(ValueId(0)),
                b: Operand::Value(ValueId(0)),
                mask: vec![0, 32],
            },
        ),
    ]);
    let e = errs(&bad);
    assert_eq!(e.len(), 1, "exactly the one bad index is reported: {e:?}");
    assert!(e[0].contains("BadLane"), "{e:?}");
}

/// **020 contract 12, the lane ops.** `lane < lanes` for both `InsertLane` and
/// `ExtractLane`. Tested separately because they are separate match arms in the verifier
/// and a rule written for one is easy to forget for the other.
#[test]
fn insert_and_extract_lane_indices_are_in_range() {
    for (name, rv_ok, rv_bad) in [
        (
            "extract",
            RValue::ExtractLane {
                v: Operand::Value(ValueId(0)),
                lane: 15,
            },
            RValue::ExtractLane {
                v: Operand::Value(ValueId(0)),
                lane: 16,
            },
        ),
        (
            "insert",
            RValue::InsertLane {
                v: Operand::Value(ValueId(0)),
                lane: 15,
                val: Operand::Const(Const::Int { bits: 8, val: 7 }),
            },
            RValue::InsertLane {
                v: Operand::Value(ValueId(0)),
                lane: 16,
                val: Operand::Const(Const::Int { bits: 8, val: 7 }),
            },
        ),
    ] {
        let ok = module_with(vec![splat_u8x16(), assign(1, rv_ok)]);
        assert!(
            verify(&ok).is_empty(),
            "{name}: lane 15 is the last of 16: {:?}",
            errs(&ok)
        );
        let bad = module_with(vec![splat_u8x16(), assign(1, rv_bad)]);
        let e = errs(&bad);
        assert_eq!(e.len(), 1, "{name}: lane 16 is one past the end: {e:?}");
        assert!(e[0].contains("BadLane"), "{name}: {e:?}");
    }
}
