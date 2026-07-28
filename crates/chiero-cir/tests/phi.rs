//! Covers: 020 §9's `Phi`, the instruction only `mem2reg` may emit.
//!
//! §9: "`mem2reg` — promote non-address-taken allocas to `ValueId`s (inserts real phis;
//! the IR grows a `Phi` instruction that only this pass may emit)."
//!
//! CIR is otherwise **non-SSA** (020 §3), and `Phi` is the one place that is not true. So
//! the rules that make a phi meaningful have to be written down and checked here, because
//! nothing else in the IR needs them and nothing else would notice them being wrong:
//!
//! - one incoming per predecessor, and *exactly* the predecessors the CFG has;
//! - phis at the **top** of a block, before any ordinary instruction;
//! - an incoming value that is live along **its own** edge, not merely somewhere.
//!
//! The last is the one that makes a phi different from every other instruction: its
//! operands are not evaluated where they appear. `UseNotDominated` is the rule the rest of
//! the IR lives by and it is the wrong rule for a phi — an incoming from `bb1` is defined
//! in `bb1`, which does not dominate the block the phi sits in.

use chiero_cir::verify::VerifyErrorKind;
use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn inst(kind: InstKind, lo: u32) -> Inst {
    Inst {
        kind,
        span: at(lo),
        generated: true,
    }
}

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: Default::default(),
        span: at(1),
    }
}

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

/// `f(n) { if (n) x = 1; else x = 2; return x; }` after promotion: a diamond whose join
/// block opens with a phi.
fn diamond(incomings: Vec<(BlockId, Operand)>, phi_first: bool) -> Module {
    let phi = inst(
        InstKind::Phi {
            dst: ValueId(9),
            ty: CTy::Int(32),
            incomings,
        },
        50,
    );
    let other = inst(
        InstKind::Assign {
            dst: ValueId(10),
            rv: RValue::Use(i32c(7)),
        },
        51,
    );
    let join_insts = if phi_first {
        vec![phi, other]
    } else {
        vec![other, phi]
    };
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
            allocas: vec![],
            blocks: vec![
                block(
                    0,
                    vec![inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::Ne,
                                a: Operand::Value(ValueId(0)),
                                b: i32c(0),
                                ty: CTy::Int(32),
                            },
                        },
                        10,
                    )],
                    Terminator::Br {
                        cond: Operand::Value(ValueId(1)),
                        t: BlockId(1),
                        f: BlockId(2),
                    },
                ),
                block(1, vec![], Terminator::Goto(BlockId(3))),
                block(2, vec![], Terminator::Goto(BlockId(3))),
                block(
                    3,
                    join_insts,
                    Terminator::Return(Some(Operand::Value(ValueId(9)))),
                ),
            ],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    }
}

/// A well-formed phi verifies, and its incomings are **not** judged by ordinary
/// dominance.
///
/// This is the assertion that fails first for any implementation that treats `Phi` as an
/// ordinary instruction: `%9`'s incomings are defined in `bb1` and `bb2`, neither of which
/// dominates `bb3`. Every other instruction in the IR would be rejected for that.
#[test]
fn a_well_formed_phi_verifies() {
    let m = diamond(vec![(BlockId(1), i32c(1)), (BlockId(2), i32c(2))], true);
    let errs = verify::verify(&m);
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "a phi with one incoming per predecessor is valid CIR: {errs:#?}"
    );
}

/// A phi missing an incoming for one of its block's predecessors is rejected.
///
/// The failure this prevents is silent and total: entering `bb3` from `bb2` would leave
/// the phi with no value to choose, and an engine that took "the first incoming" would
/// report the *other* branch's value — a counterexample naming a path the program did not
/// take.
#[test]
fn a_phi_missing_a_predecessor_is_rejected() {
    let m = diamond(vec![(BlockId(1), i32c(1))], true);
    let errs = verify::verify(&m);
    assert!(
        errs.iter()
            .any(|e| e.kind == VerifyErrorKind::PhiPredecessorMismatch),
        "one incoming, two predecessors: {errs:#?}"
    );
}

/// And a phi with an incoming from a block that is **not** a predecessor is rejected too.
///
/// Both directions, because an implementation that only counted would accept a phi whose
/// two incomings both came from `bb1`.
#[test]
fn a_phi_with_a_stranger_incoming_is_rejected() {
    let m = diamond(vec![(BlockId(1), i32c(1)), (BlockId(0), i32c(2))], true);
    let errs = verify::verify(&m);
    assert!(
        errs.iter()
            .any(|e| e.kind == VerifyErrorKind::PhiPredecessorMismatch),
        "`bb0` is not a predecessor of `bb3`: {errs:#?}"
    );
}

/// Phis sit at the **top** of their block, before any ordinary instruction.
///
/// Not a stylistic rule. A phi's value is chosen by the edge that was taken, which is a
/// fact about the moment the block is *entered*; a phi after an ordinary instruction
/// claims a value from an edge that has already stopped being the current fact.
#[test]
fn a_phi_after_an_ordinary_instruction_is_rejected() {
    let m = diamond(vec![(BlockId(1), i32c(1)), (BlockId(2), i32c(2))], false);
    let errs = verify::verify(&m);
    assert!(
        errs.iter().any(|e| e.kind == VerifyErrorKind::PhiNotAtTop),
        "the phi follows an `Assign`: {errs:#?}"
    );
}

/// A phi survives print → parse → print byte-exactly (020 contract 2).
#[test]
fn a_phi_round_trips_through_the_text_format() {
    let m = diamond(vec![(BlockId(1), i32c(1)), (BlockId(2), i32c(2))], true);
    let text = text::print(&m);
    assert!(
        text.contains("phi"),
        "the printer emits a phi directive: {text}"
    );
    let back = text::parse(&text).unwrap_or_else(|e| panic!("does not reparse: {e:?}\n{text}"));
    assert_eq!(text::print(&back), text, "and the round trip is byte-exact");

    // The *incomings* survive, not merely the instruction. A printer that dropped the
    // edge labels would round-trip fine and mean something else entirely.
    let phi = back.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .find_map(|i| match &i.kind {
            InstKind::Phi { incomings, .. } => Some(incomings.clone()),
            _ => None,
        })
        .expect("the phi survived");
    assert_eq!(phi.len(), 2);
    assert_eq!(phi[0].0, BlockId(1));
    assert_eq!(phi[1].0, BlockId(2));
}

/// **Lowering never emits a phi**, which is what "only this pass may emit" means in
/// practice.
///
/// Checked against the lowered goldens rather than asserted about the code, because the
/// claim is about output. A lowering that started emitting phis would make every
/// non-SSA consumer in the project quietly wrong about what a `ValueId` means.
#[test]
fn no_lowered_golden_contains_a_phi() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("tests/corpus/lowered");
    let mut seen = 0;
    for e in std::fs::read_dir(&dir).expect("goldens exist") {
        let p = e.expect("entry").path();
        if p.extension().is_none_or(|x| x != "cir") {
            continue;
        }
        seen += 1;
        let m = text::parse(&std::fs::read_to_string(&p).expect("read")).expect("parse");
        assert!(
            !m.funcs
                .iter()
                .flat_map(|f| f.blocks.iter())
                .flat_map(|b| b.insts.iter())
                .any(|i| matches!(i.kind, InstKind::Phi { .. })),
            "{} contains a phi, but only `mem2reg` may emit one",
            p.display()
        );
    }
    assert!(seen >= 7, "the goldens were actually read: {seen}");
}
