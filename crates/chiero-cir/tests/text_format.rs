//! The textual `.cir` format (020 §6).
//!
//! Covers **020 contracts 1–3**. The format is normative, not a debugging convenience:
//! every core test fixture in M1 is a `.cir` file, so the round-trip properties are what
//! make those fixtures trustworthy.

use chiero_cir::text::{parse, print};
use chiero_cir::*;

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
fn multiple_line_directives_accumulate_sorted() {
    let src = "func @f() -> void {\nentry:\n  .line 30\n  .line 10\n  .line 30\n  ret\n}\n";
    let m = parse(src).unwrap();
    assert_eq!(
        m.funcs[0].blocks[0].gcov_lines.as_slice(),
        &[10, 30],
        "distinct and sorted"
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
