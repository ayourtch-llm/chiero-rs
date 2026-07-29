//! One 16-byte union, two views — 020 contract 22.
//!
//! Covers: 020 contract 22.
//!
//! "For a 16-byte union viewed as `u32x4` and as `u8x16`: a lane-3 vector store followed
//! by byte loads at offsets 12..16 agrees with the scalar path, both in register form
//! (`Bitcast`) and through memory."
//!
//! This is the contract that keeps the bit-precise memory model honest about vectors.
//! Both halves are needed because they can fail independently: the register path is pure
//! term surgery (`InsertLane` splices bits, `Bitcast` reinterprets nothing at all), while
//! the memory path goes through `Repr::Bytes`, endianness, and a 16-byte store. A model
//! that gets lane numbering right in registers and byte order wrong in memory passes
//! either test alone.
//!
//! **What the two views must agree with is the scalar path**, not merely each other. Two
//! views that are wrong the same way agree perfectly, and the value used here —
//! `0xDEADBEEF` — has four distinct bytes precisely so a reversed order is visible.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

/// Little-endian bytes of `0xDEADBEEF`, which is what a load at offsets 12..16 must see
/// after the word lands in lane 3.
const WORD: i128 = 0xDEAD_BEEF;
const LE_BYTES: [u128; 4] = [0xEF, 0xBE, 0xAD, 0xDE];

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

fn run(insts: Vec<Inst>, allocas: Vec<AllocaDecl>) -> (RunResult, TermArena) {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas,
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
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    // **The module must verify.** A fixture that does not is reported as the absence of
    // everything, and every assertion below would read as a passing test of nothing.
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    (r, a)
}

/// The four values a test wants: `%first..%first+3`, each a byte.
fn bytes_at(r: &RunResult, a: &mut TermArena, first: u32) -> Vec<u128> {
    assert_eq!(r.states().len(), 1, "the fixture is straight-line");
    let s = &r.states()[0];
    (0..4)
        .map(|i| match s.local(ValueId(first + i)) {
            Some(Value::Scalar(t)) => a
                .eval_ground(t)
                .unwrap_or_else(|e| panic!("byte {i} is not concrete: {e:?}"))
                .bits(),
            other => panic!("byte {i} is {other:?}"),
        })
        .collect()
}

/// **020 contract 22, the register form.** Build `u32x4`, insert `0xDEADBEEF` at lane 3,
/// `Bitcast` to `u8x16`, and read lanes 12..16.
#[test]
fn a_lane_3_word_reads_back_as_bytes_12_to_16_in_registers() {
    let mut insts = vec![
        // %0 = u32x4 splat 0
        assign(
            0,
            RValue::Splat {
                elem: Operand::Const(Const::Int { bits: 32, val: 0 }),
                lanes: 4,
            },
        ),
        // %1 = insert 0xDEADBEEF at lane 3
        assign(
            1,
            RValue::InsertLane {
                v: Operand::Value(ValueId(0)),
                lane: 3,
                val: Operand::Const(Const::Int {
                    bits: 32,
                    val: WORD,
                }),
            },
        ),
        // %2 = bitcast to u8x16 — 128 bits either way, so nothing moves.
        assign(
            2,
            RValue::Cast {
                kind: CastKind::Bitcast,
                a: Operand::Value(ValueId(1)),
                from: u32x4(),
                to: u8x16(),
            },
        ),
    ];
    // %10..%13 = lanes 12..16 of the byte view.
    for i in 0..4u32 {
        insts.push(assign(
            10 + i,
            RValue::ExtractLane {
                v: Operand::Value(ValueId(2)),
                lane: 12 + i,
            },
        ));
    }
    let (r, mut a) = run(insts, vec![]);
    assert_eq!(
        bytes_at(&r, &mut a, 10),
        LE_BYTES.to_vec(),
        "lane 3 of a `u32x4` is bytes 12..16 of the same 128 bits, least-significant first"
    );
}

/// **020 contract 22, through memory.** The same store, this time to a 16-byte object,
/// read back one byte at a time.
///
/// The memory path is the one that can disagree: it serialises the vector through
/// `Repr::Bytes` with an endianness, where the register path never had a byte order at
/// all.
#[test]
fn a_lane_3_word_reads_back_as_bytes_12_to_16_through_memory() {
    let alloca = AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(8),
        count: 16,
        align: 16,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    };
    let mut insts = vec![
        assign(
            0,
            RValue::Splat {
                elem: Operand::Const(Const::Int { bits: 32, val: 0 }),
                lanes: 4,
            },
        ),
        assign(
            1,
            RValue::InsertLane {
                v: Operand::Value(ValueId(0)),
                lane: 3,
                val: Operand::Const(Const::Int {
                    bits: 32,
                    val: WORD,
                }),
            },
        ),
        assign(
            2,
            RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        ),
        Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(2)),
                val: Operand::Value(ValueId(1)),
                ty: u32x4(),
                align: 16,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        },
    ];
    for i in 0..4u32 {
        // %3+i = &buf[12+i]
        insts.push(assign(
            3 + i,
            RValue::PtrAdd {
                base: Operand::Value(ValueId(2)),
                off: Operand::Const(Const::Int {
                    bits: 64,
                    val: (12 + i) as i128,
                }),
            },
        ));
        insts.push(assign(
            10 + i,
            RValue::Load {
                addr: Operand::Value(ValueId(3 + i)),
                ty: CTy::Int(8),
                align: 1,
                vol: Volatility::Normal,
            },
        ));
    }
    let (r, mut a) = run(insts, vec![alloca]);
    assert!(
        r.reports().is_empty(),
        "a 16-byte store into a 16-byte object is in bounds: {:#?}",
        r.reports().iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    assert_eq!(
        bytes_at(&r, &mut a, 10),
        LE_BYTES.to_vec(),
        "the byte view of the stored vector, read through memory"
    );
}

/// **And both agree with the scalar path**, which is what the contract actually says.
/// Storing the word directly at offset 12 as a `u32` must produce the same four bytes —
/// otherwise the two vector views are consistent with each other and with nothing else.
#[test]
fn the_scalar_path_is_the_oracle_the_two_views_agree_with() {
    let alloca = AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(8),
        count: 16,
        align: 16,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    };
    let mut insts = vec![
        assign(
            2,
            RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        ),
        assign(
            0,
            RValue::PtrAdd {
                base: Operand::Value(ValueId(2)),
                off: Operand::Const(Const::Int { bits: 64, val: 12 }),
            },
        ),
        Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(0)),
                val: Operand::Const(Const::Int {
                    bits: 32,
                    val: WORD,
                }),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        },
    ];
    for i in 0..4u32 {
        insts.push(assign(
            3 + i,
            RValue::PtrAdd {
                base: Operand::Value(ValueId(2)),
                off: Operand::Const(Const::Int {
                    bits: 64,
                    val: (12 + i) as i128,
                }),
            },
        ));
        insts.push(assign(
            10 + i,
            RValue::Load {
                addr: Operand::Value(ValueId(3 + i)),
                ty: CTy::Int(8),
                align: 1,
                vol: Volatility::Normal,
            },
        ));
    }
    let (r, mut a) = run(insts, vec![alloca]);
    assert_eq!(
        bytes_at(&r, &mut a, 10),
        LE_BYTES.to_vec(),
        "a plain `u32` store at offset 12 puts the same bytes in the same places"
    );
}
