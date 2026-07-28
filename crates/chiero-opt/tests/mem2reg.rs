//! Covers: 020 contract 16.
//!
//! "Running `mem2reg` over the corpus leaves the set of findings and counterexamples
//! byte-identical to running without it — **and** on a named fixture it promotes exactly N
//! allocas and the result contains ≥ 1 `Phi`. (Transparency alone is satisfied by a pass
//! that does nothing.)"
//!
//! The transparency half is already swept in `passes.rs` over every registered pass and
//! the whole corpus, so this file is the *other* half: the named fixture, the exact count,
//! and — the part the contract does not spell out but every one of its promises rests on —
//! the allocas `mem2reg` must **refuse** to promote.
//!
//! Refusing correctly is where this pass is dangerous. An address that escapes, a
//! `CopyMem` over the slot, a partial or misaligned access: promote any of those and the
//! value in the register stops being the value in memory, silently, on exactly the paths
//! where a checker was going to find something.

use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

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
        gcov_lines: [10 + id].into_iter().collect(),
        span: at(1),
    }
}

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn slot(id: u32) -> AllocaDecl {
    AllocaDecl {
        id: AllocaId(id),
        ty: CTy::Int(32),
        count: 1,
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: at(1),
    }
}

fn func(allocas: Vec<AllocaDecl>, blocks: Vec<Block>) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![Param {
                value: ValueId(0),
                ty: CTy::Int(32),
            }],
            ret: CTy::Int(32),
            variadic: false,
            allocas,
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    }
}

fn addr(dst: u32, alloca: u32, lo: u32) -> Inst {
    inst(
        InstKind::Assign {
            dst: ValueId(dst),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(alloca),
            },
        },
        lo,
    )
}

fn store(addr: u32, val: Operand, lo: u32) -> Inst {
    inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(addr)),
            val,
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
        lo,
    )
}

fn load(dst: u32, addr: u32, lo: u32) -> Inst {
    inst(
        InstKind::Assign {
            dst: ValueId(dst),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(addr)),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            },
        },
        lo,
    )
}

fn phis(m: &Module) -> usize {
    m.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.insts.iter())
        .filter(|i| matches!(i.kind, InstKind::Phi { .. }))
        .count()
}

fn mem_ops(m: &Module) -> usize {
    m.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.insts.iter())
        .filter(|i| {
            matches!(
                i.kind,
                InstKind::Store { .. }
                    | InstKind::Assign {
                        rv: RValue::Load { .. },
                        ..
                    }
            )
        })
        .count()
}

/// The named fixture: `int x; if (n) x = 1; else x = 2; return x;` — one promotable
/// alloca, one join, and therefore one phi.
fn diamond() -> Module {
    func(
        vec![slot(0)],
        vec![
            block(
                0,
                vec![
                    addr(1, 0, 10),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Cmp {
                                op: CmpOp::Ne,
                                a: Operand::Value(ValueId(0)),
                                b: i32c(0),
                                ty: CTy::Int(32),
                            },
                        },
                        11,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![store(1, i32c(1), 20)], Terminator::Goto(BlockId(3))),
            block(2, vec![store(1, i32c(2), 30)], Terminator::Goto(BlockId(3))),
            block(
                3,
                vec![load(3, 1, 40)],
                Terminator::Return(Some(Operand::Value(ValueId(3)))),
            ),
        ],
    )
}

/// **Contract 16, second half.** On a named fixture `mem2reg` promotes exactly one alloca
/// and the result contains at least one `Phi`.
///
/// Both numbers are load-bearing. "≥ 1 phi" alone is satisfied by a pass that inserts a
/// phi and promotes nothing; "promotes 1" alone is satisfied by a pass that deletes the
/// slot and loses the value at the join.
#[test]
fn mem2reg_promotes_the_named_fixture_and_inserts_a_phi() {
    let mut m = diamond();
    assert_eq!(m.funcs[0].allocas.len(), 1);
    assert_eq!(mem_ops(&m), 3, "two stores and a load before promotion");

    chiero_opt::mem2reg(&mut m);

    assert_eq!(
        m.funcs[0].allocas.len(),
        0,
        "the one promotable slot is gone: {:#?}",
        m.funcs[0].allocas
    );
    assert!(
        phis(&m) >= 1,
        "the join needs a phi, or the two branches' values were merged by guessing: {:#?}",
        m.funcs[0].blocks
    );
    assert_eq!(
        mem_ops(&m),
        0,
        "and no load or store of the promoted slot remains: {:#?}",
        m.funcs[0].blocks
    );

    let errs = chiero_cir::verify::verify(&m);
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "the promoted module is valid CIR: {errs:#?}"
    );
}

/// **A straight line needs no phi at all**, and a pass that inserts one everywhere would
/// pass the fixture above.
///
/// `x = 1; return x;` has one predecessor everywhere, so promotion is pure substitution.
/// A phi here would not be *wrong* so much as evidence the pass is inserting them by
/// block rather than by need — and every one of them is a value the engine must carry.
#[test]
fn a_straight_line_promotes_without_a_phi() {
    let mut m = func(
        vec![slot(0)],
        vec![block(
            0,
            vec![addr(1, 0, 10), store(1, i32c(7), 11), load(2, 1, 12)],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
    );
    chiero_opt::mem2reg(&mut m);
    assert_eq!(m.funcs[0].allocas.len(), 0, "promoted");
    assert_eq!(phis(&m), 0, "and no phi was needed");
    assert_eq!(mem_ops(&m), 0);
    assert!(chiero_cir::verify::verify(&m).iter().all(|e| !e.is_error()));
}

/// **An alloca whose address escapes is not promoted.**
///
/// `int x; g(&x); return x;` — the callee may write through the pointer, so the value in
/// memory after the call is not the value the pass would have put in a register. This is
/// the failure that matters most: promoting it makes the load return the *pre-call* value
/// on every path, which is not merely imprecise but a wrong answer the engine would then
/// report as a counterexample.
#[test]
fn an_escaping_alloca_is_not_promoted() {
    let mut m = func(
        vec![slot(0)],
        vec![block(
            0,
            vec![
                addr(1, 0, 10),
                store(1, i32c(7), 11),
                inst(
                    InstKind::Call {
                        dst: None,
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![Operand::Value(ValueId(1))],
                    },
                    12,
                ),
                load(2, 1, 13),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
    );
    chiero_opt::mem2reg(&mut m);
    assert_eq!(
        m.funcs[0].allocas.len(),
        1,
        "the address reached a call, so the slot stays"
    );
    assert_eq!(mem_ops(&m), 2, "and its load and store stay with it");
}

/// **An alloca reached by a `CopyMem` is not promoted**, nor one accessed at a width
/// other than its own.
///
/// Two different escapes that a naive "is the address stored anywhere?" check misses.
/// `CopyMem` writes the slot without a `Store`; a narrow load reads part of a value a
/// register substitution cannot express. Both are shapes lowering really produces —
/// struct assignment and a `char` view of an `int`.
#[test]
fn a_copymem_or_a_partial_access_blocks_promotion() {
    let mut m = func(
        vec![slot(0)],
        vec![block(
            0,
            vec![
                addr(1, 0, 10),
                store(1, i32c(7), 11),
                inst(
                    InstKind::CopyMem {
                        dst: Operand::Value(ValueId(1)),
                        src: Operand::Value(ValueId(1)),
                        size: Operand::Const(Const::Int { bits: 64, val: 4 }),
                        align: 4,
                    },
                    12,
                ),
                load(2, 1, 13),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
    );
    chiero_opt::mem2reg(&mut m);
    assert_eq!(m.funcs[0].allocas.len(), 1, "`CopyMem` blocks promotion");

    // A narrower load of the same slot: the register would hold 32 bits and the program
    // asked for 8.
    let mut m = func(
        vec![slot(0)],
        vec![block(
            0,
            vec![
                addr(1, 0, 10),
                store(1, i32c(7), 11),
                inst(
                    InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(1)),
                            ty: CTy::Int(8),
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    12,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
    );
    chiero_opt::mem2reg(&mut m);
    assert_eq!(
        m.funcs[0].allocas.len(),
        1,
        "an access narrower than the slot blocks promotion"
    );
}

/// **A volatile access blocks promotion**, which is 020 §4.2 rather than anything about
/// aliasing: a volatile load must produce a fresh value every time, and a register holds
/// one value.
#[test]
fn a_volatile_access_blocks_promotion() {
    let mut m = func(
        vec![slot(0)],
        vec![block(
            0,
            vec![
                addr(1, 0, 10),
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(1)),
                        val: i32c(7),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Volatile,
                    },
                    11,
                ),
                load(2, 1, 12),
            ],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
    );
    chiero_opt::mem2reg(&mut m);
    assert_eq!(
        m.funcs[0].allocas.len(),
        1,
        "a device register is not a register"
    );
}

/// **A load with no reaching store reads `Undef`, not zero.**
///
/// `int x; return x;` is an uninitialized read — 021 reports it, and contract 24 is built
/// on the report existing. A promotion that substituted `0` would make the program
/// defined and the finding would vanish, which is exactly the transparency violation §9
/// forbids. `Undef` keeps the question open for the engine to answer.
#[test]
fn an_unreached_load_becomes_undef_not_zero() {
    let mut m = func(
        vec![slot(0)],
        vec![block(
            0,
            vec![addr(1, 0, 10), load(2, 1, 11)],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
    );
    chiero_opt::mem2reg(&mut m);
    let uses: Vec<Operand> = m.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Assign {
                rv: RValue::Use(o), ..
            } => Some(o.clone()),
            _ => None,
        })
        .collect();
    assert!(
        uses.iter()
            .any(|o| matches!(o, Operand::Const(Const::Undef(_)))),
        "an uninitialized read stays uninitialized: {uses:?}"
    );
}

/// **A loop gets its phi in the header**, where the incoming values are the preheader's
/// and the latch's.
///
/// The diamond fixture above has all its phi's incomings dominate their edges already; a
/// loop does not — the latch's incoming is defined *after* the phi that consumes it, in
/// program order. An implementation that placed phis by a straight dominance walk gets
/// the diamond right and this wrong.
#[test]
fn a_loop_header_gets_a_phi_with_the_latch_value() {
    // i = 0; do { i = i + 1; } while (cond); return i;
    let mut m = func(
        vec![slot(0)],
        vec![
            block(
                0,
                vec![addr(1, 0, 10), store(1, i32c(0), 11)],
                Terminator::Goto(BlockId(1)),
            ),
            block(
                1,
                vec![
                    load(2, 1, 20),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(3),
                            rv: RValue::Bin {
                                op: BinOp::Add,
                                a: Operand::Value(ValueId(2)),
                                b: i32c(1),
                                ty: CTy::Int(32),
                            },
                        },
                        21,
                    ),
                    store(1, Operand::Value(ValueId(3)), 22),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(4),
                            rv: RValue::Cmp {
                                op: CmpOp::SLt,
                                a: Operand::Value(ValueId(3)),
                                b: i32c(3),
                                ty: CTy::Int(32),
                            },
                        },
                        23,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(4)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                2,
                vec![load(5, 1, 30)],
                Terminator::Return(Some(Operand::Value(ValueId(5)))),
            ),
        ],
    );
    chiero_opt::mem2reg(&mut m);
    assert_eq!(m.funcs[0].allocas.len(), 0, "promoted");
    assert!(
        phis(&m) >= 1,
        "the header needs one: {:#?}",
        m.funcs[0].blocks
    );

    // The phi is in the header — the block with two predecessors — and not somewhere a
    // block-per-phi implementation happened to put it.
    let header = m.funcs[0]
        .blocks
        .iter()
        .find(|b| b.id == BlockId(1))
        .expect("the header survived");
    assert!(
        header
            .insts
            .iter()
            .any(|i| matches!(i.kind, InstKind::Phi { .. })),
        "in the header: {:#?}",
        header.insts
    );

    let errs = chiero_cir::verify::verify(&m);
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "and it verifies, which for a loop phi means its latch incoming was accepted \
         without ordinary dominance: {errs:#?}"
    );
}

/// **`mem2reg` is registered**, or contract 44 and every prohibition sweep in `passes.rs`
/// runs over one pass fewer than the crate has.
#[test]
fn mem2reg_is_registered() {
    let p = chiero_opt::pass("mem2reg").expect("020 §9 names it, so the registry holds it");
    assert_eq!(p.name, "mem2reg");
}
