//! Wide literals and switch determinism — 020 contracts 13 and 34.
//!
//! Covers: 020 contracts 13, 34.
//!
//! Both are about output that has to be the *same* twice. 001 §5 makes determinism a hard
//! requirement, and the textual format is where it becomes visible: a golden test compares
//! text, so an unstable case order or a lossy literal turns every downstream diff into
//! noise and hides the one change that mattered.

use chiero_cir::text::{parse, print};
use chiero_cir::*;
use chiero_span::Span;

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: Default::default(),
        span: Span::DUMMY,
    }
}

fn func(blocks: Vec<Block>) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: Span::DUMMY,
            linkage: chiero_cir::Linkage::External,
        }],
        ..Default::default()
    }
}

/// **020 contract 13.** "A `Switch` printed twice from the same module produces identical
/// case order."
///
/// The cases are a list in the IR, so the risk is not the list — it is any pass or printer
/// that reaches for a hash-ordered structure on the way out. This is the cheap check that
/// nothing has.
#[test]
fn a_switch_prints_the_same_case_order_twice() {
    let m = func(vec![
        block(
            0,
            vec![Inst {
                kind: InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                },
                span: Span::DUMMY,
                generated: false,
            }],
            Terminator::Switch {
                scrut: Operand::Value(ValueId(0)),
                ty: CTy::Int(32),
                // Deliberately not in ascending order: a printer that sorted would pass a
                // test whose cases were already sorted, and lose the program's own order.
                cases: vec![(7, BlockId(3)), (1, BlockId(1)), (4, BlockId(2))],
                default: BlockId(4),
            },
        ),
        block(1, vec![], Terminator::Return(None)),
        block(2, vec![], Terminator::Return(None)),
        block(3, vec![], Terminator::Return(None)),
        block(4, vec![], Terminator::Return(None)),
    ]);
    let first = print(&m);
    let second = print(&m);
    assert_eq!(first, second, "two printings of one module agree");

    // …and the order is the *module's*, not a sorted one.
    let line = first
        .lines()
        .find(|l| l.contains("switch"))
        .expect("the switch is printed");
    let seven = line.find('7').expect("case 7");
    let one = line
        .find(" 1 ")
        .or_else(|| line.find("1:"))
        .expect("case 1");
    assert!(
        seven < one,
        "case 7 was written first and prints first: {line}"
    );

    // And it survives a round trip, which is what a golden test actually exercises.
    let back = parse(&first).expect("re-parses");
    assert_eq!(print(&back), first, "print . parse . print is stable");
}

/// **020 contract 34.** "`Const::Wide` round-trips a 512-bit `u8x64` literal through the
/// textual format byte-exactly."
///
/// 512 bits do not fit in the `i128` every other integer literal uses, and `memcpy_x86_64.h`
/// manipulates `u8x64` values directly — a format that silently truncated one would produce
/// a module that parses, verifies, and is not the program.
#[test]
fn a_512_bit_literal_round_trips_byte_exactly() {
    // Distinct bytes in every position, so a dropped or reordered word is visible.
    let words: Vec<u64> = (0..8u64).map(|i| 0x0102_0304_0506_0700 | i).collect();
    let m = func(vec![block(
        0,
        vec![Inst {
            kind: InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Use(Operand::Const(Const::Wide {
                    bits: 512,
                    words: words.clone(),
                })),
            },
            span: Span::DUMMY,
            generated: false,
        }],
        Terminator::Return(None),
    )]);
    let text = print(&m);
    let back = parse(&text).expect("a wide literal re-parses");
    let printed_again = print(&back);
    assert_eq!(text, printed_again, "print . parse . print is stable");

    // Byte-exact, not merely stable: a printer and parser that agreed on a *wrong* value
    // would satisfy the round trip and lose the program.
    let Some(Inst {
        kind:
            InstKind::Assign {
                rv: RValue::Use(Operand::Const(Const::Wide { bits, words: got })),
                ..
            },
        ..
    }) = back.funcs[0].blocks[0].insts.first()
    else {
        panic!(
            "the literal survives as a literal: {:#?}",
            back.funcs[0].blocks[0].insts
        );
    };
    assert_eq!(*bits, 512);
    assert_eq!(*got, words, "every word, in order");
}
