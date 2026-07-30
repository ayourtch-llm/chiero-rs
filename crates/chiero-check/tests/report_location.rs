//! **A checker's report says nothing about where it happened.**
//!
//! Waves 207–211 made the memory channel's reports readable: the object is named or described,
//! the access is located, a second event carries its own location, and the sentence is composed
//! rather than patched. Every one of those went through `report_faults` or the model route.
//!
//! A checker takes neither. `Action::Report` is pushed straight onto the state:
//!
//! ```text
//!   signed-overflow: 2147483647 + 1 overflows int
//! ```
//!
//! No location, with a `SourceMap` in hand and the span sitting in the same function that builds
//! the finding. That is the entire UBSan-parity channel waves 157–176 built — every signed
//! overflow, over-wide shift and division by zero — and a caller reading `findings()` cannot tell
//! which line any of them is on.
//!
//! # Why this fixture is CIR and a hand-built map
//!
//! 001 §4 rule 7 forbids this crate a frontend dependency, so there is no C source here and
//! `order_dependence.rs` and `undefined_arithmetic.rs` set that precedent. It costs nothing:
//! `SourceMap::add_file` is public, spans are built by hand anyway, and a checker's input is an
//! event on a state rather than a program. The map maps chiero's own synthetic offsets onto lines,
//! which is all `lookup_loc` needs.
//!
//! §9 called this "the one place where covered means covered by argument" — checker findings were
//! held to share the model route, and they do not share it at all.

use chiero_cir::*;
use chiero_exec::Engine;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, SourceMap, Span};

/// A span one byte wide at `lo`, as everywhere else in this crate's fixtures.
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
        span: at(1),
    }
}

fn k(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn module(blocks: Vec<Block>) -> Module {
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
            span: at(1),
            linkage: chiero_cir::Linkage::External,
        }],
        ..Default::default()
    }
}

/// Four lines of five bytes each, so byte 12 is line 3 and nothing else is.
///
/// The offsets are chiero's own — this crate never sees a preprocessor — and a `SourceMap` does
/// not care where they came from. What matters is that the arithmetic sits at a byte the map
/// resolves to a line no other fixture byte shares, or a wrong span would still pass.
fn map_of_four_lines() -> SourceMap {
    let mut m = SourceMap::new();
    m.add_file("ub.c", "aaaa\nbbbb\ncccc\ndddd\n");
    m
}

/// A signed overflow at a span the map resolves to line 3.
fn overflow_at(lo: u32) -> Module {
    module(vec![block(
        0,
        vec![inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Bin {
                    op: BinOp::Add,
                    ty: CTy::Int(32),
                    a: k(i128::from(i32::MAX)),
                    b: k(1),
                    signed: true,
                },
            },
            lo,
        )],
        Terminator::Return(Some(Operand::Value(ValueId(0)))),
    )])
}

fn findings(m: &Module, map: Option<&SourceMap>) -> Vec<String> {
    let mut a = TermArena::new();
    let mut e = Engine::new(m);
    if let Some(map) = map {
        e = e.with_source_map(map);
    }
    for c in chiero_check::default_checkers() {
        e = e.with_checker(c);
    }
    e.run(&mut a).findings()
}

/// **The line the arithmetic is on.**
#[test]
fn a_checker_report_names_where_it_happened() {
    let map = map_of_four_lines();
    let f = findings(&overflow_at(12), Some(&map));
    assert!(
        !f.is_empty(),
        "the fixture must overflow, or there is nothing to locate"
    );
    assert!(
        f.iter().any(|m| m.contains("ub.c:3:")),
        "byte 12 is line 3 of the map this run was given: {f:?}"
    );
}

/// It is the *instruction's* line, not the function's or the block's.
///
/// The assertion that stops the cheapest wrong fix. Every `Function` and `Block` in these
/// fixtures spans `at(1)` — line 1 — so a fix that stamped any convenient span would name line 1
/// and satisfy a test that only asked for "some location".
#[test]
fn the_line_is_the_instruction_s_own() {
    let map = map_of_four_lines();
    for (lo, want) in [
        (2u32, "ub.c:1:"),
        (7, "ub.c:2:"),
        (12, "ub.c:3:"),
        (17, "ub.c:4:"),
    ] {
        let f = findings(&overflow_at(lo), Some(&map));
        assert!(
            f.iter().any(|m| m.contains(want)),
            "the overflow is at byte {lo}, which this map calls {want}: {f:?}"
        );
    }
}

/// The report still leads with its kind. **The control.**
///
/// 023 §6.1 makes the kind half the dedup key, and this crate's other two test files match on it,
/// so the location must not go in front — `ub.c:3:1: signed overflow: …` is the compiler's
/// convention and would break every one of them.
///
/// The assertion is "does not begin with the path" rather than a character-class check on the
/// leading word, because the first version of this test asserted lowercase-and-hyphens and failed
/// on the *correct* message: checker kinds are spelled with spaces (`signed overflow`) where
/// `MemFault` kinds are hyphenated slugs (`use-after-free`). That inconsistency is real and is
/// recorded in §9; it is not what this test is about.
#[test]
fn the_kind_still_leads() {
    let map = map_of_four_lines();
    for m in findings(&overflow_at(12), Some(&map)) {
        assert!(
            !m.starts_with("ub.c") && !m.starts_with('/'),
            "a report begins with its kind, not with a path: {m:?}"
        );
        assert!(
            m.contains("overflow"),
            "and the kind is still the thing it leads with: {m:?}"
        );
    }
}

/// Without a map, nothing is invented. **The control.**
#[test]
fn without_a_source_map_nothing_is_stamped() {
    let f = findings(&overflow_at(12), None);
    assert!(
        !f.is_empty(),
        "the report itself must survive a run with no map"
    );
    assert!(
        f.iter().all(|m| !m.contains("(at ")),
        "a run with no map must not claim a location: {f:?}"
    );
}

/// **The route a report with its own witness takes.**
///
/// Mutation found this one: leaving `Action::ReportRequiring` unstamped passed every test above,
/// because a constant overflow's UB event carries no condition and so takes `Action::Report`. A
/// *symbolic* divisor does carry one — wave 156's query — and the checker joins it to the report so
/// the witness names an input under which something actually faults.
///
/// That makes it the route with the strongest evidence behind it, which is the worst one to leave
/// unlocatable.
#[test]
fn a_report_carrying_its_own_condition_is_located_too() {
    let map = map_of_four_lines();
    let m = module(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                },
                2,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin {
                        op: BinOp::SDiv,
                        ty: CTy::Int(32),
                        a: k(100),
                        b: Operand::Value(ValueId(0)),
                        signed: true,
                    },
                },
                17,
            ),
        ],
        Terminator::Return(Some(Operand::Value(ValueId(1)))),
    )]);
    let f = findings(&m, Some(&map));
    assert!(
        !f.is_empty(),
        "a symbolic divisor the solver can make zero must be reported"
    );
    assert!(
        f.iter().any(|m| m.contains("ub.c:4:")),
        "the division is at byte 17, which this map calls line 4 — and the `Fresh` above it is on \
         line 1, so a fix that stamped the wrong instruction would say so: {f:?}"
    );
}
