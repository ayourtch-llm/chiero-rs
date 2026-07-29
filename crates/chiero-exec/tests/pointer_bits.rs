//! Pointer-bit inspection — 021 contract 17b.
//!
//! Covers: 021 contract 17b.
//!
//! §7.2, with a real example from `plugins/nsim/nsim_input.c`:
//!
//! ```c
//! if ((((uword) ep) & (CLIB_CACHE_LINE_BYTES - 1)) == 0)
//!   clib_prefetch_load (ep + 2);
//! ```
//!
//! "Whether this holds depends on the real allocator; chiero decides it concretely from
//! the region base plus a bump, prunes one branch as infeasible on *every* run, and —
//! because addresses are deterministic (contract 15) — never looks flaky."
//!
//! That last clause is what makes it worth a contract of its own. The wrong answer is
//! stable, reproducible, and identical on every run, so nothing about the output suggests
//! a question was decided by chiero's bump allocator rather than by the program.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

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

/// `if (((uword) p & 63) == 0) return 1; else return 2;` over a local whose alignment is
/// 8 — so bits 0..3 are guaranteed by the alignment and bits 3..6 are the allocator's
/// business, not the program's.
fn alignment_test(mask: i128, align: u64) -> Module {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(8),
            count: 128,
            align,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }],
        blocks: vec![
            block(
                0,
                vec![
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::AddrOfLocal {
                                alloca: AllocaId(0),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cast {
                                kind: CastKind::PtrToInt,
                                a: Operand::Value(ValueId(0)),
                                from: CTy::Ptr,
                                to: CTy::Int(64),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Bin {
                                op: BinOp::And,
                                ty: CTy::Int(64),
                                a: Operand::Value(ValueId(1)),
                                b: Operand::Const(Const::Int {
                                    bits: 64,
                                    val: mask,
                                }),
                                signed: true,
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(3),
                            rv: RValue::Cmp {
                                op: CmpOp::Eq,
                                ty: CTy::Int(64),
                                a: Operand::Value(ValueId(2)),
                                b: Operand::Const(Const::Int { bits: 64, val: 0 }),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(1)))),
            block(2, vec![], Terminator::Return(Some(i32c(2)))),
        ],
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

/// **021 contract 17b.** A branch on pointer bits *below the object's guaranteed
/// alignment* emits a `PointerBitInspection` event and does **not** have one side pruned.
#[test]
fn a_cache_line_alignment_test_explores_both_sides_and_says_why() {
    // Mask 63 asks about bits 0..6; the object promises only 8-byte alignment, so bits
    // 3..6 are the allocator's answer and not the program's.
    let m = alignment_test(63, 8);
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
        "both sides are live: chiero's bump allocator does not get to decide this"
    );
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("pointer bit")),
        "and the run names what it could not decide: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "a branch chiero could not honestly decide is not an exact model"
    );
}

/// **The negative case, which is what keeps this from being noise.** Bits *within* the
/// guaranteed alignment are a fact about the object, not about the allocator: a 16-byte
/// aligned object really does have its low four bits clear, and `p & 7` really is zero.
/// Forking there would double the state count of every aligned-pointer test in VPP.
#[test]
fn bits_within_the_guaranteed_alignment_are_decided_normally() {
    let m = alignment_test(7, 16);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let rets: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut a))
        .collect();
    assert_eq!(
        rets,
        vec![1],
        "16-byte alignment guarantees the low three bits, so one side is genuinely dead"
    );
    assert!(
        !r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("pointer bit")),
        "and nothing was assumed: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
    assert_eq!(r.fidelity(), Fidelity::Exact);
}
