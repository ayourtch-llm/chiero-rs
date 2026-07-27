//! The textual `.cir` format (020 §6).
//!
//! Covers **020 contracts 1–3**. The format is normative, not a debugging convenience:
//! every core test fixture in M1 is a `.cir` file, so the round-trip properties are what
//! make those fixtures trustworthy.

use chiero_cir::text::{parse, print};
use chiero_cir::*;
use chiero_span::Span;
use std::collections::BTreeSet;

/// A canonical module in textual form. Round-tripping this byte-exactly is contract 2.
const CANONICAL: &str = r#"target x86_64-unknown-linux-gnu

global @counts : size 32 align 8

func @add(%0: i32, %1: i32) -> i32 {
entry:
  .line 12
  %2 = add i32 %0, %1
  ret %2
}
"#;

/// 020 contract 2: `print(parse(s)) == s`, byte-exact, for canonical input.
#[test]
fn canonical_text_round_trips_byte_exactly() {
    let m = parse(CANONICAL).expect("parse");
    assert_eq!(print(&m), CANONICAL);
}

/// 020 contract 1: `parse(print(m))` is structurally equal to `m`.
#[test]
fn module_round_trips_through_text() {
    let m = parse(CANONICAL).expect("parse");
    let again = parse(&print(&m)).expect("reparse");
    assert_eq!(print(&m), print(&again));
    assert_eq!(m.funcs.len(), again.funcs.len());
    assert_eq!(m.globals.len(), again.globals.len());
}

/// 020 §6: **unknown directives are a hard parse error.** Silent tolerance in a
/// fixture format produces tests that pass by not testing anything, so this is the
/// contract that keeps the whole `.cir` corpus honest.
#[test]
fn unknown_directive_is_a_hard_error() {
    let src = "target x86_64-unknown-linux-gnu\n\n.frobnicate 3\n";
    let e = parse(src).expect_err("must reject");
    assert!(e.line == 3, "error must name the line, got {e:?}");
    assert!(
        e.message.contains("frobnicate"),
        "error must name the offending token: {}",
        e.message
    );
}

#[test]
fn unknown_instruction_is_a_hard_error() {
    let src = "func @f() -> void {\nentry:\n  %0 = frobnicate i32 1, 2\n  ret\n}\n";
    let e = parse(src).expect_err("must reject");
    assert_eq!(e.line, 3);
}

#[test]
fn unknown_type_is_a_hard_error() {
    let src = "func @f() -> quux {\nentry:\n  ret\n}\n";
    assert!(parse(src).is_err());
}

/// Every instruction the M1 fixtures need must survive a round trip. A format that
/// silently drops an instruction would make a fixture test something other than what
/// it appears to.
#[test]
fn every_instruction_kind_round_trips() {
    let src = r#"target x86_64-unknown-linux-gnu

global @g : size 8 align 8

func @extern_fn(%0: ptr) -> i32

func @f(%0: ptr, %1: i32) -> i32 {
  alloca %0 : i32 x 4 align 4 scope 0 lifetime scope "buf"
entry:
  .line 7
  .scope enter 0
  %2 = addrlocal %0
  %3 = addrglobal @g
  %4 = addrfunc @extern_fn
  %5 = load i32, %2 align 4
  %6 = loadvolatile i32, %2 align 4
  %7 = add i32 %5, %1
  %8 = sub i32 %7, 1i32
  %9 = udiv i32 %8, 2i32
  %10 = and i32 %9, 255i32
  %11 = shl i32 %10, 1i32
  %12 = neg i32 %11
  %13 = cmp slt i32 %12, 0i32
  %14 = zext i32 %13 to i64
  %15 = trunc i64 %14 to i8
  %16 = bitcast i32 %11 to f32
  %17 = select %13, %11, %12
  %18 = ptradd %0, -8i64
  %19 = fresh i32
  %20 = call @extern_fn(%0)
  store i32 %7 -> %2 align 4
  storevolatile i32 %7 -> %2 align 4
  storebits i32 %7 -> %2 bits 3..8 align 4
  %21 = loadbits i32, %2 bits 3..8 signed align 4
  copymem %2 -> %0, 40i64 align 8
  setmem %2, 0i8, 16i64
  .seqpoint
  .scope exit 0
  br %13, bb1, bb2
bb1:
  .line 32
  switch i32 %7, [1 -> bb2, 2 -> bb2], default bb2
bb2:
  .line 34
  ret %7
}
"#;
    let m = parse(src).expect("parse");
    assert_eq!(print(&m), src, "every construct must survive byte-exactly");
    assert!(verify(&m).iter().all(|e| !e.is_error()), "{:?}", verify(&m));
}

/// A `.cir` file need not carry spans — that is what makes hand-written fixtures cheap
/// (020 §6) — and a module without them gets `Span::DUMMY` at `ExpnCtx::ROOT`.
#[test]
fn spans_are_optional() {
    let m = parse(CANONICAL).unwrap();
    let f = &m.funcs[0];
    assert!(f.blocks[0].insts.iter().all(|i| i.span.is_dummy()));
    assert!(f.blocks[0].span.is_dummy());
}

/// 015 §5: in a fixture there is no `SourceMap`, so `.line` populates `gcov_lines`
/// directly. Without this M1's fixtures could not exercise 030 contract 13 at all.
#[test]
fn line_directive_populates_gcov_lines() {
    let m = parse(CANONICAL).unwrap();
    let b = &m.funcs[0].blocks[0];
    assert_eq!(b.gcov_lines.as_slice(), &[12]);
}

#[test]
fn multiple_line_directives_deduplicate_preserving_order() {
    let src = "func @f() -> void {\nentry:\n  .line 30\n  .line 10\n  .line 30\n  ret\n}\n";
    let m = parse(src).unwrap();
    assert_eq!(
        m.funcs[0].blocks[0].gcov_lines.as_slice(),
        &[30, 10],
        "distinct, in source order — sorting breaks the structural round trip"
    );
}

/// A declared (extern) function has no body, and must not acquire an empty one on a
/// round trip — 020 §8 rule 10 makes the distinction a verifier error.
#[test]
fn declared_function_stays_declared() {
    let src = "target x86_64-unknown-linux-gnu\n\nfunc @ext(%0: i32) -> i32\n";
    let m = parse(src).unwrap();
    assert_eq!(m.funcs[0].body, Body::Declared);
    assert!(m.funcs[0].blocks.is_empty());
    assert_eq!(print(&m), src);
}

/// Negative and wide constants are where a naive printer/parser pair silently loses
/// information.
#[test]
fn constants_round_trip_exactly() {
    let src = r#"target x86_64-unknown-linux-gnu

func @c() -> void {
entry:
  %0 = add i32 -2147483648i32, -1i32
  %1 = add i64 9223372036854775807i64, 1i64
  %2 = add i128 170141183460469231731687303715884105727i128, 0i128
  %3 = ptradd null, 0i64
  ret
}
"#;
    let m = parse(src).expect("parse");
    assert_eq!(print(&m), src);
}

/// Whitespace-only differences must normalize, or "canonical" is not well-defined and
/// contract 2 cannot hold for anything a human typed.
#[test]
fn noncanonical_input_normalizes_to_canonical() {
    let messy = "target   x86_64-unknown-linux-gnu\n\nglobal   @counts  :  size 32   align 8\n\n\nfunc @add(%0: i32, %1: i32) -> i32 {\n\nentry:\n    .line 12\n      %2   =   add   i32   %0,   %1\n  ret %2\n}\n";
    let m = parse(messy).expect("parse");
    assert_eq!(print(&m), CANONICAL, "printing is canonicalizing");
}

/// Printing is deterministic (001 §5).
#[test]
fn printing_is_deterministic() {
    let m = parse(CANONICAL).unwrap();
    assert_eq!(print(&m), print(&m));
}

/// An empty module is legal and round-trips.
#[test]
fn empty_module_round_trips() {
    let src = "target x86_64-unknown-linux-gnu\n";
    let m = parse(src).expect("parse");
    assert!(m.funcs.is_empty());
    assert_eq!(print(&m), src);
}

/// A truncated function must be a clear error, not a silently-accepted partial module.
#[test]
fn unterminated_function_is_an_error() {
    let src = "func @f() -> void {\nentry:\n  ret\n";
    assert!(parse(src).is_err());
}

#[test]
fn block_without_terminator_is_an_error() {
    let src = "func @f() -> void {\nentry:\n  .line 1\n}\n";
    let e = parse(src).expect_err("must reject");
    assert!(
        e.message.to_lowercase().contains("terminator"),
        "{}",
        e.message
    );
}

/// Malformed input must **error**, never panic. The parser indexes tokens positionally
/// in ~20 places, and a panic on a truncated line is strictly worse than an error: it
/// carries no line number, and in CI a panic and an error look identical.
#[test]
fn malformed_input_errors_rather_than_panicking() {
    let cases = [
        ",",
        "func @f() -> void {\ngoto\n}\n",
        "func @f() -> void {\nentry:\n  br %0, bb1\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  store i32 %0\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  setmem %0\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  .line\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  .scope\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  .scope enter\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  .label\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  %0 = allocadyn\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  %0 = vaarg\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  vastart\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  vacopy %0\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  copymem %0\n  ret\n}\n",
        "func @f() -> void {\nentry:\n  storebits i32 %0\n  ret\n}\n",
        "func @f() -> void {\n  alloca %0\nentry:\n  ret\n}\n",
        "func(x) -> void {\nentry:\n  ret\n}\n",
        "func @f( -> void {\nentry:\n  ret\n}\n",
    ];
    for src in cases {
        let r = std::panic::catch_unwind(|| parse(src));
        match r {
            Ok(Err(_)) => {}
            Ok(Ok(_)) => panic!("should not have parsed: {src:?}"),
            Err(_) => panic!("panicked instead of erroring: {src:?}"),
        }
    }
}

/// Contract 3's other two paths. The existing tests cover a module-level directive and
/// an unknown *rvalue*; an unknown directive **inside a block** and an unknown **bare**
/// instruction go through different code and were untested.
#[test]
fn unknown_constructs_inside_a_block_are_hard_errors() {
    let e = parse("func @f() -> void {\nentry:\n  .frobnicate 1\n  ret\n}\n")
        .expect_err("unknown block directive must reject");
    assert!(e.message.contains("frobnicate"), "{}", e.message);

    let e = parse("func @f() -> void {\nentry:\n  frobnicate %0\n  ret\n}\n")
        .expect_err("unknown bare instruction must reject");
    assert!(e.message.contains("frobnicate"), "{}", e.message);
}

/// 020 contract 1, **structurally**. The corpus test compares `print(m)` with
/// `print(parse(print(m)))`, which is text-to-text and therefore invariant under any
/// field the printer never prints. Six were dropped silently: `variadic`, all four
/// `FnAttrs`, and `is_const`.
#[test]
fn round_trip_preserves_every_semantic_field() {
    let src = r#"target x86_64-unknown-linux-gnu

global const @ro : size 4 align 4

func @variadic_noreturn(%0: i32, ...) -> void noreturn order_sensitive {
entry:
  .line 5
  ret
}
"#;
    let m = parse(src).expect("parse");
    assert!(m.funcs[0].variadic, "variadic must survive");
    assert!(m.funcs[0].attrs.noreturn);
    assert!(m.funcs[0].attrs.order_sensitive);
    assert!(m.globals[0].is_const);

    let again = parse(&print(&m)).expect("reparse");
    assert_eq!(m, again, "round trip must preserve the module structurally");
    assert_eq!(print(&m), src);
}

/// A fixture reaching **every** variant the coverage test enumerates. If a variant is
/// added to the library, that test fails until this fixture exercises it.
const FULL_COVERAGE_FIXTURE: &str = r#"target x86_64-unknown-linux-gnu

global const @ro : size 4 align 4

global @g : size 8 align 8

func @other(%0: i32, ...) -> i32 noreturn pure order_sensitive march "avx2"

func @cover(%0: <4xi32>, %1: ptr, %2: i32) -> void {
  alloca %3 : i32 x 4 align 4 scope 0 lifetime scope "buf"
  alloca %4 : i8 x 1 align 1 scope 1 lifetime function "dyn"
entry:
  .line 11
  .scope enter 0
  %5 = addrlocal %3
  %6 = addrglobal @g
  %7 = addrfunc @other
  %8 = load i32, %5 align 4
  %9 = loadvolatile i32, %5 align 4
  %10 = loadbits i32, %5 bits 3..8 signed align 4
  %11 = add i32 %8, %2
  %12 = ptrdiff 4 %1, %1
  %13 = neg i32 %11
  %14 = cmp slt i32 %13, 0i32
  %15 = zext i32 %14 to i64
  %16 = select %14, %11, %13
  %17 = ptradd %1, -8i64
  %18 = fresh i32
  %19 = extractlane %0, 2
  %20 = insertlane %0, 1, 7i32
  %21 = shuffle %0, %20, [0, 5, 2, 7]
  %22 = splat 3i32, 4
  %23 = undef:i64
  %24 = globaladdr:@g:8
  %25 = funcaddr:@other
  %26 = wide:i256:0x0000000000000000000000000000000000000000000000000000000000000001
  %27 = fconst:f64:0x3ff0000000000000
  %28 = null
  %29 = allocadyn %4 : i8 x %2 align 1
  %30 = call @other(%2)
  %31 = vaarg %1, i32
  store i32 %11 -> %5 align 4
  storevolatile i32 %11 -> %5 align 4
  storebits i32 %11 -> %5 bits 3..8 align 4
  copymem %5 -> %1, 40i64 align 8
  setmem %5, 0i8, 16i64
  vastart %1
  vacopy %1 -> %1
  vaend %1
  opaque %15:i64 %16:i64 writes %5 8i64 reads %1 why unsupported "computed goto"
  .seqpoint
  .label "retry"
  .scope exit 0
  br %14, bb1, bb2
bb1:
  .line 47
  switch i32 %11, [1 -> bb2, 2 -> bb3], default bb3
bb2:
  .line 49
  indirectgoto %1, [bb3]
bb3:
  .line 51
  goto bb4
bb4:
  .line 53
  unreachable builtin
}

func @ret_void() -> void {
entry:
  .line 58
  ret
}
"#;

/// Every enum variant must survive the round trip.
///
/// The previous version of this test was a **tautology**: every arm of its `match`
/// returned `true` and the assertions reduced to `assert!(true)`. It covered three of
/// roughly ten enums, and its "adding a variant fails to compile here" claim was false —
/// the *library* fails to build first, because the printer is already exhaustive. It was
/// the mechanism relied on to stop coverage gaps recurring, and it guarded nothing.
///
/// This version walks a parsed module, collects the discriminant of every node actually
/// reached, and asserts the set equals the full variant list. A variant absent from the
/// fixture now fails here rather than silently going untested.
#[test]
fn every_variant_is_accounted_for() {
    let src = FULL_COVERAGE_FIXTURE;
    let m = parse(src).expect("parse");
    assert_eq!(print(&m), src, "every construct must survive byte-exactly");
    assert_eq!(m, parse(&print(&m)).expect("reparse"));

    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    for g in &m.globals {
        seen.insert(if g.is_const {
            "Global::const"
        } else {
            "Global::mut"
        });
    }
    for f in &m.funcs {
        seen.insert(match f.body {
            Body::Defined => "Body::Defined",
            Body::Declared => "Body::Declared",
        });
        if f.variadic {
            seen.insert("variadic");
        }
        for a in &f.allocas {
            seen.insert(match a.lifetime {
                Lifetime::Scope => "Lifetime::Scope",
                Lifetime::Function => "Lifetime::Function",
            });
        }
        for b in &f.blocks {
            seen.insert(term_name(&b.term));
            for i in &b.insts {
                seen.insert(inst_name(&i.kind));
                if let InstKind::Assign { rv, .. } = &i.kind {
                    seen.insert(rvalue_name(rv));
                    collect_operands(rv, &mut seen);
                }
                if let InstKind::Marker(mk) = &i.kind {
                    seen.insert(marker_name(mk));
                }
            }
        }
    }

    // The full variant lists. Adding a variant to the library and not to a fixture now
    // fails *here*, which is the guard the old version claimed to provide.
    let required: BTreeSet<&'static str> = ALL_INST_NAMES
        .iter()
        .chain(ALL_RVALUE_NAMES)
        .chain(ALL_TERM_NAMES)
        .chain(ALL_MARKER_NAMES)
        .chain(ALL_CONST_NAMES)
        .copied()
        .collect();
    let missing: Vec<_> = required.difference(&seen).collect();
    assert!(
        missing.is_empty(),
        "no fixture exercises these variants: {missing:?}"
    );
}

const ALL_INST_NAMES: &[&str] = &[
    "Assign",
    "Store",
    "StoreBits",
    "CopyMem",
    "SetMem",
    "Call",
    "AllocaDyn",
    "VaArg",
    "VaStart",
    "VaCopy",
    "VaEnd",
    "Opaque",
    "Marker",
];
const ALL_RVALUE_NAMES: &[&str] = &[
    "Use",
    "Load",
    "LoadBits",
    "Bin",
    "Un",
    "Cmp",
    "Cast",
    "Select",
    "PtrAdd",
    "AddrOfLocal",
    "AddrOfGlobal",
    "AddrOfFunc",
    "Shuffle",
    "InsertLane",
    "ExtractLane",
    "Splat",
    "Fresh",
];
const ALL_TERM_NAMES: &[&str] = &[
    "Goto",
    "Br",
    "Switch",
    "Return",
    "IndirectGoto",
    "Unreachable",
];
const ALL_MARKER_NAMES: &[&str] = &["Marker::SeqPoint", "Marker::Scope", "Marker::Label"];
const ALL_CONST_NAMES: &[&str] = &[
    "Const::Int",
    "Const::Wide",
    "Const::Float",
    "Const::Null",
    "Const::GlobalAddr",
    "Const::FuncAddr",
    "Const::Undef",
];

fn inst_name(k: &InstKind) -> &'static str {
    match k {
        InstKind::Assign { .. } => "Assign",
        InstKind::Store { .. } => "Store",
        InstKind::StoreBits { .. } => "StoreBits",
        InstKind::CopyMem { .. } => "CopyMem",
        InstKind::SetMem { .. } => "SetMem",
        InstKind::Call { .. } => "Call",
        InstKind::AllocaDyn { .. } => "AllocaDyn",
        InstKind::VaArg { .. } => "VaArg",
        InstKind::VaStart { .. } => "VaStart",
        InstKind::VaCopy { .. } => "VaCopy",
        InstKind::VaEnd { .. } => "VaEnd",
        InstKind::Opaque { .. } => "Opaque",
        InstKind::Marker(_) => "Marker",
    }
}

fn rvalue_name(rv: &RValue) -> &'static str {
    match rv {
        RValue::Use(_) => "Use",
        RValue::Load { .. } => "Load",
        RValue::LoadBits { .. } => "LoadBits",
        RValue::Bin { .. } => "Bin",
        RValue::Un { .. } => "Un",
        RValue::Cmp { .. } => "Cmp",
        RValue::Cast { .. } => "Cast",
        RValue::Select { .. } => "Select",
        RValue::PtrAdd { .. } => "PtrAdd",
        RValue::AddrOfLocal { .. } => "AddrOfLocal",
        RValue::AddrOfGlobal { .. } => "AddrOfGlobal",
        RValue::AddrOfFunc(_) => "AddrOfFunc",
        RValue::Shuffle { .. } => "Shuffle",
        RValue::InsertLane { .. } => "InsertLane",
        RValue::ExtractLane { .. } => "ExtractLane",
        RValue::Splat { .. } => "Splat",
        RValue::Fresh { .. } => "Fresh",
    }
}

fn term_name(t: &Terminator) -> &'static str {
    match t {
        Terminator::Goto(_) => "Goto",
        Terminator::Br { .. } => "Br",
        Terminator::Switch { .. } => "Switch",
        Terminator::Return(_) => "Return",
        Terminator::IndirectGoto { .. } => "IndirectGoto",
        Terminator::Unreachable(_) => "Unreachable",
    }
}

fn marker_name(m: &MarkerKind) -> &'static str {
    match m {
        MarkerKind::Line(_) => "Marker::Line",
        MarkerKind::SeqPoint => "Marker::SeqPoint",
        MarkerKind::Scope(_) => "Marker::Scope",
        MarkerKind::Label(_) => "Marker::Label",
    }
}

fn const_name(c: &Const) -> &'static str {
    match c {
        Const::Int { .. } => "Const::Int",
        Const::Wide { .. } => "Const::Wide",
        Const::Float(..) => "Const::Float",
        Const::Null => "Const::Null",
        Const::GlobalAddr { .. } => "Const::GlobalAddr",
        Const::FuncAddr(_) => "Const::FuncAddr",
        Const::Undef(_) => "Const::Undef",
    }
}

fn note_operand(o: &Operand, seen: &mut BTreeSet<&'static str>) {
    if let Operand::Const(c) = o {
        seen.insert(const_name(c));
    }
}

fn collect_operands(rv: &RValue, seen: &mut BTreeSet<&'static str>) {
    match rv {
        RValue::Use(o) | RValue::ExtractLane { v: o, .. } | RValue::Splat { elem: o, .. } => {
            note_operand(o, seen)
        }
        RValue::Load { addr, .. } | RValue::LoadBits { addr, .. } => note_operand(addr, seen),
        RValue::Bin { a, b, .. } | RValue::Cmp { a, b, .. } | RValue::Shuffle { a, b, .. } => {
            note_operand(a, seen);
            note_operand(b, seen);
        }
        RValue::Un { a, .. } | RValue::Cast { a, .. } => note_operand(a, seen),
        RValue::Select { cond, t, f } => {
            note_operand(cond, seen);
            note_operand(t, seen);
            note_operand(f, seen);
        }
        RValue::PtrAdd { base, off } => {
            note_operand(base, seen);
            note_operand(off, seen);
        }
        RValue::InsertLane { v, val, .. } => {
            note_operand(v, seen);
            note_operand(val, seen);
        }
        _ => {}
    }
}

/// Every `UnreachableReason` is distinct on the round trip. 020 §5 gives `LoweringGap`
/// (`Fidelity::Unknown`) and `BuiltinUnreachable` (genuinely dead) different meanings,
/// so collapsing them loses a semantic distinction rather than a label.
#[test]
fn unreachable_reasons_are_distinct() {
    for (text, want) in [
        ("noreturn", UnreachableReason::AfterNoreturn),
        ("exhaustive", UnreachableReason::ExhaustiveSwitch),
        ("builtin", UnreachableReason::BuiltinUnreachable),
        ("gap", UnreachableReason::LoweringGap),
    ] {
        let src = format!("func @f() -> void {{\nentry:\n  unreachable {text}\n}}\n");
        let m = parse(&src).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(m.funcs[0].blocks[0].term, Terminator::Unreachable(want));
    }
}

/// A function whose entry is not `BlockId(0)` must round-trip without aliasing. The
/// printer wrote `entry` for `f.entry` while the parser hardcoded `entry -> BlockId(0)`,
/// so a sibling `BlockId(0)` reparsed into two blocks both numbered 0.
#[test]
fn a_nonzero_entry_block_does_not_alias() {
    // Built by hand, **not** parsed. The previous version parsed its input, so both
    // sides had `entry == BlockId(0)` and the test could not distinguish the correct
    // implementation from the buggy one it was written to pin.
    let mut m = Module::default();
    m.funcs.push(Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![
            Block {
                id: BlockId(3),
                insts: vec![],
                term: Terminator::Goto(BlockId(0)),
                gcov_lines: Default::default(),
                span: Span::DUMMY,
            },
            Block {
                id: BlockId(0),
                insts: vec![],
                term: Terminator::Return(None),
                gcov_lines: Default::default(),
                span: Span::DUMMY,
            },
        ],
        entry: BlockId(3),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    });

    let again = parse(&print(&m)).expect("reparse");
    assert_eq!(
        again.funcs[0].entry,
        BlockId(3),
        "the program must still start at the same block"
    );
    assert_eq!(m, again);
}

/// Mixing named and numeric values must not collide. 020 §6 permits the mix, and
/// handing named values ids from a counter that only counts *named* ones made `%1` and
/// a later `%b` the same value — silently a different program than the text says.
#[test]
fn named_and_numeric_values_do_not_collide() {
    let src = concat!(
        "func @f(%0: i32, %1: i32) -> i32 {\n",
        "entry:\n",
        "  %named = add i32 %0, %1\n",
        "  %other = add i32 %named, %1\n",
        "  ret %other\n}\n"
    );
    let m = parse(src).expect("parse");
    let f = &m.funcs[0];
    let mut ids: Vec<u32> = f.params.iter().map(|p| p.value.0).collect();
    for i in &f.blocks[0].insts {
        if let InstKind::Assign { dst, .. } = &i.kind {
            ids.push(dst.0);
        }
    }
    let unique: BTreeSet<_> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "ids must be distinct: {ids:?}");
    let errs: Vec<_> = verify(&m).into_iter().filter(|e| e.is_error()).collect();
    assert!(errs.is_empty(), "{errs:#?}");
}

/// A branch to a label that is never defined must be an error, not a fabricated id that
/// happens to alias a real block. Rule 2 cannot catch it, because the id exists.
#[test]
fn a_branch_to_an_undefined_label_is_rejected() {
    let src = concat!(
        "func @f(%0: i1) -> void {\n",
        "entry:\n",
        "  br %0, tgt, bb2\n",
        "bb1:\n  ret\n",
        "bb2:\n  goto bb1\n}\n"
    );
    let e = parse(src).expect_err("must reject a branch to an undefined label");
    assert!(e.message.contains("tgt"), "{}", e.message);
}

/// Constants must print readably in **every** operand position, not only as a bare
/// `Use` rvalue. `print_inst` used the module-blind printer, so `undef`, `wide`,
/// `fconst`, `globaladdr` and `funcaddr` as a store value or call argument printed in a
/// form the parser rejects — contract 1 broken for a whole class of operand.
#[test]
fn constants_print_readably_in_every_operand_position() {
    let src = concat!(
        "target x86_64-unknown-linux-gnu\n\n",
        "global @g : size 8 align 8\n\n",
        "func @sink(%0: ptr, %1: i64) -> void\n\n",
        "func @f(%0: ptr) -> void {\n",
        "entry:\n",
        "  .line 7\n",
        "  store i64 undef:i64 -> %0 align 8\n",
        "  call @sink(globaladdr:@g:0, undef:i64)\n",
        "  setmem %0, 0i8, 8i64\n",
        "  ret\n}\n"
    );
    let m = parse(src).expect("parse");
    assert_eq!(print(&m), src, "must round-trip byte-exactly");
    assert_eq!(m, parse(&print(&m)).expect("reparse"));
}

/// `gcov_lines` order must survive. The parser sorted unconditionally, so a lowered
/// module emitting `[30, 10]` reparsed as `[10, 30]` and `parse(print(m)) != m`.
#[test]
fn gcov_line_order_is_preserved() {
    let mut m = Module::default();
    m.funcs.push(Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(None),
            gcov_lines: [30u32, 10, 20].into_iter().collect(),
            span: Span::DUMMY,
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    });
    let again = parse(&print(&m)).expect("reparse");
    assert_eq!(
        again.funcs[0].blocks[0].gcov_lines.as_slice(),
        &[30, 10, 20],
        "order must survive; sorting on parse breaks structural round-trip"
    );
    assert_eq!(m, again);
}

/// A duplicated block label is malformed input, not two blocks with one id.
#[test]
fn a_duplicate_block_label_is_an_error() {
    let src = "func @f() -> void {\nentry:\n  ret\nentry:\n  ret\n}\n";
    assert!(parse(src).is_err());
}

/// **The normative example from 020 §6 must parse.** It did not: it uses named values
/// (`%len_p`), named block labels (`bb_ok`) and an alloca without `scope`/`lifetime`,
/// none of which the parser accepted. A spec whose own worked example is not a valid
/// fixture is a spec the implementation has quietly diverged from.
///
/// Names are canonicalized to `%N`/`bbN` on output, consistent with printing being
/// canonicalization — the same rule that lets whitespace normalize.
#[test]
fn the_spec_worked_example_parses() {
    let src = r#"target x86_64-unknown-linux-gnu

global @counts : size 32 align 8

func @vec_len_minus_1(%v: ptr) -> i32 {
  alloca %slot : i32 x 1 align 4 "n"
entry:
  .line 12
  %len_p = ptradd %v, -8i64
  %len   = load i32, %len_p align 4
  %n     = sub i32 %len, 1i32
  %slotp = addrlocal %slot
  store i32 %n -> %slotp align 4
  %c     = cmp slt i32 %n, 0i32
  br %c, bb_bad, bb_ok
bb_ok:
  .line 14
  ret %n
bb_bad:
  .line 15
  unreachable builtin
}
"#;
    let m = parse(src).unwrap_or_else(|e| panic!("020 §6's example must parse: {e:?}"));
    assert_eq!(m.funcs.len(), 1);
    assert_eq!(m.funcs[0].blocks.len(), 3);
    // **And it must verify.** Asserting only parse + round-trip let the example parse
    // into a module containing a never-defined value — the shape of vacuity this
    // project keeps rediscovering.
    let errs: Vec<_> = verify(&m).into_iter().filter(|e| e.is_error()).collect();
    assert!(errs.is_empty(), "the spec's example must verify: {errs:#?}");
    // Round-trips through its canonical form.
    let again = parse(&print(&m)).expect("reparse");
    assert_eq!(m, again);
}

/// An alloca name in operand position is a clear error pointing at `addrlocal`, not a
/// silently-minted undefined value.
#[test]
fn an_alloca_name_is_not_an_operand() {
    let src = concat!(
        "func @f() -> void {\n",
        "  alloca %slot : i32 x 1 align 4\n",
        "entry:\n",
        "  store i32 0i32 -> %slot align 4\n",
        "  ret\n}\n"
    );
    let e = parse(src).expect_err("must reject");
    assert!(e.message.contains("addrlocal"), "{}", e.message);
}

/// A const global must be visible to name resolution. It was not, so ids desynchronized
/// and `addrglobal` silently resolved to a *different* global.
#[test]
fn a_const_global_resolves_by_name() {
    let src = concat!(
        "target x86_64-unknown-linux-gnu\n\n",
        "global const @ro : size 4 align 4\n\n",
        "global @rw : size 4 align 4\n\n",
        "func @f() -> ptr {\nentry:\n  %0 = addrglobal @rw\n  ret %0\n}\n"
    );
    let m = parse(src).expect("parse");
    let InstKind::Assign {
        rv: RValue::AddrOfGlobal { g },
        ..
    } = &m.funcs[0].blocks[0].insts[0].kind
    else {
        panic!("expected addrglobal");
    };
    assert_eq!(
        m.globals[g.0 as usize].name.as_ref(),
        "rw",
        "must resolve to @rw"
    );
    assert_eq!(print(&m), src);
}

/// Value names are function-scoped: `%tmp` in two functions is two values.
#[test]
fn value_names_are_function_scoped() {
    let src = "func @a() -> void {\nentry:\n  %t = add i32 1i32, 1i32\n  ret\n}\n\
               func @b() -> void {\nentry:\n  %t = add i32 2i32, 2i32\n  ret\n}\n";
    let m = parse(src).expect("parse");
    assert_eq!(m.funcs.len(), 2);
    // Both get id 0 in their own function; neither collides with the other.
    assert_eq!(print(&m), print(&parse(&print(&m)).unwrap()));
}

// ---------------------------------------------------------------------------
// Wave 7 text-format defects.
// ---------------------------------------------------------------------------

/// **020 §6: "Unknown directives are a hard parse error. Silent tolerance in a
/// test-fixture format produces tests that pass by not testing anything."**
///
/// The rule was enforced for directives and mnemonics but not for *operands*: every
/// instruction parser indexes fixed token positions and drops the rest. This is how
/// `fresh i32 "input"` from 020 §4.1 silently loses its reason string — the parse
/// succeeds and the field is simply gone.
#[test]
fn trailing_junk_on_an_instruction_is_a_hard_error() {
    let src =
        "func @k(%0: ptr) -> void {\nentry:\n  store i32 0i32 -> %0 align 4 frobnicate\n  ret\n}\n";
    let e = parse(src).expect_err("trailing tokens must not be silently discarded");
    assert!(
        e.message.contains("frobnicate"),
        "the error must name the offending token, got: {}",
        e.message
    );
}

/// The same rule at the other parser entry points. Contract 3 was tested for unknown
/// *instructions* only, so tolerance survived everywhere else.
#[test]
fn unknown_function_attributes_and_block_directives_are_hard_errors() {
    let a = "func @f() -> void frobattr {\nentry:\n  ret\n}\n";
    assert!(parse(a).is_err(), "unknown function attribute was accepted");
    let b = "func @f() -> void {\nentry:\n  .frobdir 3\n  ret\n}\n";
    assert!(parse(b).is_err(), "unknown block directive was accepted");
}

/// Alloca ids got the id-aliasing fix that values and labels received; they did not.
/// A named alloca is interned from 0, so it collides with a literal `%0` alloca in the
/// same function — the identical defect, in the one id space that was missed.
#[test]
fn named_and_numeric_allocas_do_not_collide() {
    let src = "func @g() -> i32 {\n  alloca %buf : i32 x 4 align 4 scope 0 lifetime scope\n  alloca %0 : i32 x 4 align 4 scope 0 lifetime scope\nentry:\n  ret 0i32\n}\n";
    let m = parse(src).expect("both allocas are legal and distinct");
    let errs: Vec<_> = chiero_cir::verify(&m)
        .into_iter()
        .filter(|e| e.is_error())
        .collect();
    assert!(
        errs.is_empty(),
        "`%buf` and `%0` are two different objects; got: {errs:#?}"
    );
    assert_eq!(m.funcs[0].allocas.len(), 2);
    assert_ne!(m.funcs[0].allocas[0].id, m.funcs[0].allocas[1].id);
}

/// Named *parameters* are interned at 0..k-1 before the literal scan runs, so a literal
/// `%0` in the body silently resolves to the first parameter instead of being a distinct
/// value. 020 §6 permits mixing the two spellings; the existing collision test uses
/// numeric parameters only, so it cannot see this.
#[test]
fn named_parameters_do_not_collide_with_numeric_body_values() {
    let src = "func @h(%a: i32, %b: i32) -> i32 {\nentry:\n  %0 = add i32 %a, %b\n  ret %0\n}\n";
    let m = parse(src).expect("mixing named params and numeric body values is legal");
    let f = &m.funcs[0];
    let dst = match &f.blocks[0].insts[0].kind {
        chiero_cir::InstKind::Assign { dst, .. } => *dst,
        other => panic!("expected an assign, got {other:?}"),
    };
    assert_ne!(
        dst, f.params[0].value,
        "the body's `%0` must not alias parameter `%a`"
    );
    assert_ne!(dst, f.params[1].value);
}

/// **020 contract 1 for the field 020 §1.5 calls "the product".**
///
/// Nothing printed or parsed spans, so every `Inst::span`, `Block::span`,
/// `Function::span` and `Global::span` round-tripped to `Span::DUMMY`. Contracts 1 and
/// 015/22 held only because every fixture has dummy spans — and
/// `every_corpus_module_round_trips` compares `print(m)` to `print(parse(print(m)))`,
/// which is invariant under anything the printer omits. This breaks the moment
/// `chiero-lower` emits real CIR, and it breaks *silently*.
///
/// This test asserts structural equality against a module carrying real spans, so the
/// instrument can actually observe the field.
#[test]
fn spans_survive_a_round_trip() {
    use chiero_span::{BytePos, ExpnCtx, Span};
    let sp = |lo: u32, hi: u32, ctx: u32| Span {
        lo: BytePos(lo),
        hi: BytePos(hi),
        ctx: ExpnCtx(ctx),
    };
    let mut m =
        parse("func @f(%0: i32) -> i32 {\nentry:\n  %1 = add i32 %0, %0\n  ret %1\n}\n").unwrap();
    m.funcs[0].span = sp(10, 20, 0);
    m.funcs[0].blocks[0].span = sp(30, 40, 0);
    // A macro-expanded instruction: a non-root `ExpnCtx` is the whole point of 010.
    m.funcs[0].blocks[0].insts[0].span = sp(50, 60, 7);
    let again = parse(&print(&m)).expect("printed spans must parse");
    assert_eq!(again, m, "spans did not survive:\n{}", print(&m));
}

/// Dummy spans stay invisible, so the corpus does not grow a comment on every line.
#[test]
fn dummy_spans_are_not_printed() {
    let src = "func @f() -> void {\nentry:\n  ret\n}\n";
    let m = parse(src).unwrap();
    assert!(!print(&m).contains("span"), "{}", print(&m));
}

/// **C's `isnan` idiom is `x != x`, which requires an *unordered* not-equal.** `FONe`
/// is ordered, so it is **false** for NaN — the exact opposite of what C means. Without
/// an unordered predicate the front end has no correct lowering for the idiom.
#[test]
fn unordered_float_predicates_exist_and_round_trip() {
    use chiero_cir::CmpOp;
    for (name, op) in [
        ("fueq", CmpOp::FUEq),
        ("fune", CmpOp::FUNe),
        ("fult", CmpOp::FULt),
        ("fule", CmpOp::FULe),
        ("ford", CmpOp::FOrd),
        ("funo", CmpOp::FUno),
    ] {
        let src = format!(
            "func @f(%0: f64, %1: f64) -> i1 {{\nentry:\n  %2 = cmp {name} f64 %0, %1\n  ret %2\n}}\n"
        );
        let printed = format!("target x86_64-unknown-linux-gnu\n\n{src}");
        let m = parse(&src).unwrap_or_else(|e| panic!("`{name}` must parse: {e:?}"));
        match &m.funcs[0].blocks[0].insts[0].kind {
            chiero_cir::InstKind::Assign {
                rv: chiero_cir::RValue::Cmp { op: got, .. },
                ..
            } => assert_eq!(*got, op, "`{name}`"),
            other => panic!("expected a cmp, got {other:?}"),
        }
        assert_eq!(print(&m), printed, "`{name}` does not print back");
    }
}

/// **020 §4.3 / §7: inline `asm` is `Opaque` with declared outputs, not a skip.**
///
/// `Opaque` was specified but absent from `InstKind`, which left lowering only the three
/// options §4.3 forbids: drop the asm, invent an unattached `Fresh` that a CSE pass could
/// then merge across two `rdtsc` calls, or refuse the function. 31 VPP files use inline
/// asm and `clib_cpu_time_now()` is on the dispatch loop, so "refuse the function" is not
/// a real option either.
#[test]
fn an_rdtsc_shaped_opaque_round_trips() {
    use chiero_cir::*;
    let src = "func @now() -> i64 {\n\
               entry:\n  \
               opaque %0:i32 %1:i32 writes reads why asm\n  \
               %2 = zext i32 %0 to i64\n  \
               ret %2\n\
               }\n";
    let m = parse(src).expect("an rdtsc-shaped opaque must parse");
    match &m.funcs[0].blocks[0].insts[0].kind {
        InstKind::Opaque {
            dsts, why, writes, ..
        } => {
            // `rdtsc` writes two registers, which is the whole reason `dsts` exists:
            // with only memory `writes` there is no way to express it.
            assert_eq!(dsts.len(), 2, "both register outputs must survive");
            assert_eq!(dsts[0], (ValueId(0), CTy::Int(32)));
            assert_eq!(dsts[1], (ValueId(1), CTy::Int(32)));
            assert!(writes.is_empty());
            assert_eq!(*why, OpaqueReason::InlineAsm);
        }
        other => panic!("expected an Opaque, got {other:?}"),
    }
    assert_eq!(
        print(&m),
        format!("target x86_64-unknown-linux-gnu\n\n{src}"),
        "opaque must print back canonically"
    );
    assert_eq!(m, parse(&print(&m)).unwrap());
}

/// A clobbered memory region and a named unmodeled builtin. 020 contract 32 pairs a
/// declared write with a `dst`; this is the representational half of it.
#[test]
fn an_opaque_with_writes_and_reads_round_trips() {
    use chiero_cir::*;
    let src = "func @clobber(%0: ptr, %1: i64) -> void {\n\
               entry:\n  \
               opaque %2:i64 writes %0 8i64 %0 %1 reads %1 why builtin \"__sync_synchronize\"\n  \
               ret\n\
               }\n";
    let m = parse(src).expect("parse");
    match &m.funcs[0].blocks[0].insts[0].kind {
        InstKind::Opaque {
            dsts,
            writes,
            reads,
            why,
        } => {
            assert_eq!(dsts.len(), 1);
            assert_eq!(writes.len(), 2, "two clobbered regions");
            assert_eq!(writes[0].addr, Operand::Value(ValueId(0)));
            assert_eq!(
                writes[0].size,
                Operand::Const(Const::Int { bits: 64, val: 8 })
            );
            assert_eq!(writes[1].size, Operand::Value(ValueId(1)));
            assert_eq!(reads, &vec![Operand::Value(ValueId(1))]);
            match why {
                OpaqueReason::UnmodeledBuiltin(n) => assert_eq!(&**n, "__sync_synchronize"),
                other => panic!("expected a named builtin, got {other:?}"),
            }
        }
        other => panic!("expected an Opaque, got {other:?}"),
    }
    assert_eq!(m, parse(&print(&m)).unwrap());
}
