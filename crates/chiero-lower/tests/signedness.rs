//! **Signedness is a property of the C operands, and CIR has been dropping it.**
//!
//! C's arithmetic UB rules are asymmetric in exactly one bit. `a + a` is undefined on
//! overflow when `a` is `int` and defined — wraps modulo 2^N — when `a` is `unsigned`.
//! `a << 31` is undefined for a signed `a` (C11 6.5.7p4: negative operand, or a result
//! `E1 x 2^E2` that does not fit) and ordinary for an unsigned one. Same machine
//! instruction, opposite verdicts.
//!
//! CIR keeps that bit for *some* operations and not others. `SDiv`/`UDiv`, `SRem`/`URem`
//! and `AShr`/`LShr` are split because the machine operation genuinely differs. `Add`,
//! `Sub`, `Mul` and `Shl` are single opcodes because the machine operation does *not*
//! differ — which was the right call for execution and the wrong one for checking, since
//! `CTy::Int(w)` carries a width and no signedness either.
//!
//! The cost is a false report and a missed one, and this file pins both:
//!
//! - **False**: unsigned wraparound is reported as `SignedOverflow`. `note_ub` calls
//!   `.signed()` on both operands unconditionally, so `3000000000u + 3000000000u`
//!   reinterprets as `-1294967296 + -1294967296`, lands outside the signed range, and
//!   fires. gcc compiles the same program under `-fsanitize=undefined` and runs it clean.
//!   Wave 171's rule is that a false finding costs more than a missing one.
//! - **Missed**: neither signed-left-shift rule is checked, because checking them from
//!   the bits alone would report every `unsigned x << 31` — the commonest idiom in C.
//!   Wave 173 measured 8 negative shifts and 5 non-representable results that gcc reports
//!   and chiero does not.
//!
//! Every fixture here is written at C source level rather than by building CIR by hand,
//! and that is the point: the bit under test is one *lowering* has and discards, so a test
//! that hands CIR a signedness it chose itself would pass while the real path stayed
//! broken.

mod harness;

use chiero_exec::{Engine, UbKind};
use chiero_solver::TermArena;

/// Every UB kind the engine records for a closed program, in order.
fn ub_kinds(src: &str) -> Vec<UbKind> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).run(&mut arena);
    r.states()
        .iter()
        .flat_map(|s| s.ub_events())
        .map(|u| u.kind)
        .collect()
}

/// Unsigned arithmetic wraps. It is defined, and reporting it is a false positive.
///
/// The values are chosen so the wrap is unmistakable *and* so the bit pattern read as
/// signed falls outside the signed range — `4000000000u + 4000000000u` happens to land
/// back inside it once reinterpreted, and would pass this test without the defect being
/// fixed.
#[test]
fn unsigned_wraparound_is_defined_and_is_not_reported() {
    for (what, src) in [
        (
            "add",
            "int probe(void) { unsigned a = 3000000000u; unsigned b = a + a; return (int)(b >> 28); }",
        ),
        (
            "mul",
            "int probe(void) { unsigned a = 3000000000u; unsigned b = a * 2u; return (int)(b >> 28); }",
        ),
        (
            "sub",
            "int probe(void) { unsigned a = 1u; unsigned b = a - 2u; return (int)(b >> 28); }",
        ),
    ] {
        let kinds = ub_kinds(src);
        assert!(
            !kinds.contains(&UbKind::SignedOverflow),
            "unsigned {what} wraps and is defined in C, but chiero reported {kinds:?}"
        );
    }
}

/// Signed overflow is still undefined, and must still be reported.
///
/// The control for the test above: a fix that silences unsigned wrap by silencing the
/// whole check would pass it and fail here.
#[test]
fn signed_overflow_is_still_reported() {
    for (what, src) in [
        (
            "add",
            "int probe(void) { int a = 2147483647; int b = a + 1; return b; }",
        ),
        (
            "mul",
            "int probe(void) { int a = 2000000000; int b = a * 2; return b; }",
        ),
    ] {
        let kinds = ub_kinds(src);
        assert!(
            kinds.contains(&UbKind::SignedOverflow),
            "signed {what} overflow is undefined and must be reported, got {kinds:?}"
        );
    }
}

/// C11 6.5.7p4, first clause: a *signed* left shift of a negative value is undefined.
#[test]
fn a_signed_left_shift_of_a_negative_value_is_reported() {
    let kinds = ub_kinds("int probe(void) { int a = -1; int b = a << 1; return b; }");
    assert!(
        kinds.contains(&UbKind::Shift),
        "`-1 << 1` is undefined (C11 6.5.7p4) and must be reported, got {kinds:?}"
    );
}

/// C11 6.5.7p4, second clause: a signed left shift whose result `E1 x 2^E2` is not
/// representable in the promoted type is undefined.
///
/// Measured both ways before being asserted, because the shift count is legal here and
/// only the signedness decides the verdict:
///
/// ```text
/// int a = 1;      a << 31  ->  runtime error: left shift of 1 by 31 places cannot
///                              be represented in type 'int'
/// unsigned a = 1; a << 31  ->  2147483648, exit 0
/// ```
#[test]
fn a_signed_left_shift_out_of_range_is_reported() {
    let kinds = ub_kinds("int probe(void) { int a = 1; int b = a << 31; return b; }");
    assert!(
        kinds.contains(&UbKind::Shift),
        "signed `1 << 31` does not fit in `int` and must be reported, got {kinds:?}"
    );
}

/// The other half of the same clause, and the reason it cannot be checked from the bits:
/// the identical shift on an unsigned operand is ordinary code.
///
/// This is the assertion that a naive fix breaks. It is not a comment — wave 173 recorded
/// it as a pin, and it is what forces the signedness to be carried rather than guessed.
#[test]
fn an_unsigned_left_shift_out_of_signed_range_is_ordinary_code() {
    for (what, src) in [
        (
            "1u << 31",
            "int probe(void) { unsigned a = 1u; unsigned b = a << 31; return (int)(b >> 28); }",
        ),
        (
            "high bit set",
            "int probe(void) { unsigned a = 3000000000u; unsigned b = a << 1; return (int)(b >> 28); }",
        ),
    ] {
        let kinds = ub_kinds(src);
        assert!(
            !kinds.contains(&UbKind::Shift),
            "unsigned `{what}` is defined in C, but chiero reported {kinds:?}"
        );
    }
}

/// The count rule (C11 6.5.7p3) is signedness-independent **and direction-independent**.
///
/// The direction half was missing until wave 262 and mutation is what said so: restricting the
/// clause to `Shl` — so `x >> 32` on a 32-bit type reports nothing — survived the whole suite. Both
/// fixtures here were left shifts, which is the same empty-cell shape wave 261 found in the
/// float-cast list one wave earlier: the grid had two signednesses and one direction.
///
/// C11 6.5.7p3 puts the count rule on the *shift-expression*, not on `<<`, and UBSan agrees in all
/// four cells — `shift exponent 32 is too large for 32-bit type` fires for `int` and `unsigned`,
/// `<<` and `>>` alike.
#[test]
fn the_shift_count_rule_applies_to_both_signednesses() {
    for (what, src) in [
        (
            "signed",
            "int probe(void) { int a = 1; int b = a << 32; return b; }",
        ),
        (
            "unsigned",
            "int probe(void) { unsigned a = 1u; unsigned b = a << 32; return (int)b; }",
        ),
        (
            "signed, right",
            "int probe(void) { int a = 1; int b = a >> 32; return b; }",
        ),
        (
            "unsigned, right",
            "int probe(void) { unsigned a = 1u; unsigned b = a >> 32; return (int)b; }",
        ),
    ] {
        let kinds = ub_kinds(src);
        assert!(
            kinds.contains(&UbKind::Shift),
            "a {what} shift by the operand width is undefined whatever the signedness, got {kinds:?}"
        );
    }
}

/// **The width the arithmetic happens at is not the width the source wrote**, and the
/// check must survive the conversion that reconciles them.
///
/// C's usual arithmetic conversions widen the narrower operand, so `acc * 31` on a `long`
/// `acc` lowers to a `sext` of `31` followed by a 64-bit multiply. Nothing about that
/// changes whether the multiply overflows — `acc * 31L`, which needs no conversion at all,
/// is the same computation — so a checker that reports one and not the other is reporting
/// on the spelling rather than on the program.
///
/// This is the shape the UB census measured as `0 / 18`: every `acc = acc * 31 + x` in the
/// generated corpus mixes an `int` literal with a `long` accumulator, which is also what
/// C programmers write.
#[test]
fn overflow_is_reported_through_the_usual_arithmetic_conversions() {
    for (what, src) in [
        (
            "long * int",
            "int probe(void) { long acc = 804574689342403103L; acc = acc * 31; return (int)acc; }",
        ),
        (
            "long * long",
            "int probe(void) { long acc = 804574689342403103L; acc = acc * 31L; return (int)acc; }",
        ),
        (
            "long + int",
            "int probe(void) { long a = 9223372036854775807L; a = a + 1; return (int)a; }",
        ),
        (
            "long + long",
            "int probe(void) { long a = 9223372036854775807L; a = a + 1L; return (int)a; }",
        ),
    ] {
        let kinds = ub_kinds(src);
        assert!(
            kinds.contains(&UbKind::SignedOverflow),
            "`{what}` overflows a 64-bit signed result and must be reported, got {kinds:?}"
        );
    }
}

/// The conversion must not manufacture an overflow either.
///
/// A `sext` that folded to the wrong value — dropping the sign, say — would make
/// `acc + (-1)` look like `acc + 4294967295` and report an overflow that is not there.
#[test]
fn widening_a_negative_operand_does_not_invent_an_overflow() {
    let kinds = ub_kinds(
        "int probe(void) { long acc = 9223372036854775807L; int step = -1; acc = acc + step; return (int)acc; }",
    );
    assert!(
        !kinds.contains(&UbKind::SignedOverflow),
        "`LONG_MAX + (-1)` is in range; a sign-dropping widening would report it: {kinds:?}"
    );
}

/// Every UB kind, plus what the program returned — for the cases where the *value* is
/// defined and gcc's answer is the one to match.
fn ub_and_value(src: &str) -> (Vec<UbKind>, Option<i32>) {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let v = r
        .states()
        .iter()
        .find_map(|s| s.return_value_bits(&mut arena))
        .map(|b| b as u32 as i32);
    (
        r.states()
            .iter()
            .flat_map(|s| s.ub_events())
            .map(|u| u.kind)
            .collect(),
        v,
    )
}

/// **A float-to-integer conversion is checked against the destination's range, and the
/// destination has a signedness.**
///
/// C11 6.3.1.4 makes the conversion undefined when the truncated value is not representable
/// in the destination type. `200.0` is representable in `unsigned char` and not in
/// `signed char`; `3e9` is representable in `unsigned` and not in `int`. Lowering emits
/// `FpToSi` for every float-to-integer cast — `cast_kind` has one arm for
/// `(Float, Int)` and `target_signed` returns `true` unconditionally — so the range checked
/// is always the signed one and every unsigned destination is reported.
///
/// Measured against gcc, which is silent on all three:
///
/// ```text
///   (unsigned char)200.0    -> 200,   exit 0
///   (unsigned)3000000000.0  -> 3000000000
///   (unsigned short)60000.0 -> 60000
/// ```
///
/// This is what the census's false-positive column found on seed 117, and it is the same
/// class of defect as this file's other half in a different place: `CTy::Int(w)` carries no
/// signedness, so a conversion *to* it cannot say which range it means. CIR is not at fault
/// — `FpToSi` and `FpToUi` are distinct kinds and the engine checks each correctly. Only
/// lowering's choice between them is wrong.
#[test]
fn a_float_conversion_to_an_unsigned_destination_is_not_reported() {
    for (what, src, want) in [
        (
            "unsigned char",
            "int probe(void){ double d=200.0; unsigned char c=(unsigned char)d; return (int)c; }",
            200,
        ),
        (
            "unsigned short",
            "int probe(void){ double d=60000.0; unsigned short s=(unsigned short)d; return (int)s; }",
            60000,
        ),
        (
            "unsigned int",
            "int probe(void){ double d=3000000000.0; unsigned u=(unsigned)d; return (int)(u>>16); }",
            45776,
        ),
    ] {
        let (kinds, value) = ub_and_value(src);
        assert!(
            !kinds.contains(&UbKind::FloatCastOverflow),
            "`{what}` holds this value and gcc runs it clean, but chiero reported {kinds:?}"
        );
        assert_eq!(value, Some(want), "and computes gcc's answer for `{what}`");
    }
}

/// The controls: a destination that genuinely cannot hold the value still reports.
///
/// Both signednesses are here on purpose. A fix that silenced the false reports by
/// switching every conversion to `FpToUi` would pass the test above and fail on
/// `signed char`; one that silenced them by dropping the check would fail on both.
#[test]
fn a_float_conversion_out_of_the_destinations_range_is_still_reported() {
    for (what, src) in [
        (
            "signed char, 200",
            "int probe(void){ double d=200.0; signed char c=(signed char)d; return (int)c; }",
        ),
        (
            "unsigned char, 300",
            "int probe(void){ double d=300.0; unsigned char c=(unsigned char)d; return (int)c; }",
        ),
        (
            "unsigned char, negative",
            "int probe(void){ double d=-1.0; unsigned char c=(unsigned char)d; return (int)c; }",
        ),
        (
            "int, 1e20",
            "int probe(void){ double d=1e20; int i=(int)d; return i; }",
        ),
        // **The two below were the whole of wave 261**, and they were found by mutation rather
        // than by reading. The list above covers a float too *large* for a signed destination and a
        // *negative* one for an unsigned destination — so deleting either of those checks in
        // `out_of_range` fails here. Deleting the remaining two did not:
        //
        //   `t < -hi.exp2()`   a float too negative for a *signed* destination   SURVIVED
        //   the `is_nan` guard  a NaN converted to any integer type              SURVIVED
        //
        // Both are C11 6.3.1.4 undefined and both are what UBSan calls "outside the range of
        // representable values". The generated corpus produced neither in fifty-four programs,
        // which is what made `FloatCastOverflow` the thin row §9 flagged — thin in two nameable
        // directions rather than merely small.
        (
            "int, -1e20",
            "int probe(void){ double d=-1e20; int i=(int)d; return i; }",
        ),
        (
            "int, NaN",
            "int probe(void){ double z=0.0; double n=z/z; int i=(int)n; return i; }",
        ),
        (
            "unsigned, NaN",
            "int probe(void){ double z=0.0; double n=z/z; unsigned u=(unsigned)n; return (int)u; }",
        ),
    ] {
        let (kinds, _) = ub_and_value(src);
        assert!(
            kinds.contains(&UbKind::FloatCastOverflow),
            "`{what}` is undefined under C11 6.3.1.4 and must be reported, got {kinds:?}"
        );
    }
}

/// **A braced initializer converts without a cast expression in front of it**, and that is
/// the path the deleted `target_signed` helper got wrong.
///
/// Its comment argued that an unsigned destination "arrives with its own cast expression
/// whose type sema records", so assuming signed was safe. `struct S { unsigned char c; }`
/// initialised `{200.0}` is the counterexample: sema inserts the conversion for an
/// assignment *expression* and not for a braced element, so nothing here spells the
/// destination type but the member declaration.
///
/// Written because mutation said so. `store-always-signed` — forcing `convert_for_store`
/// back to `FpToSi` — survived the whole suite, which meant one of this wave's three fixed
/// sites had no test observing it at all.
#[test]
fn a_braced_initializer_converts_to_the_members_signedness() {
    for (what, src, want_report, want_value) in [
        (
            "unsigned char member",
            "struct S { unsigned char c; }; int probe(void){ struct S s = {200.0}; return (int)s.c; }",
            false,
            200,
        ),
        (
            "signed char member",
            "struct S { signed char c; }; int probe(void){ struct S s = {200.0}; return (int)s.c; }",
            true,
            -56,
        ),
        (
            "unsigned char array element",
            "int probe(void){ unsigned char a[2] = {200.0, 1.0}; return (int)a[0]; }",
            false,
            200,
        ),
    ] {
        let (kinds, value) = ub_and_value(src);
        assert_eq!(
            kinds.contains(&UbKind::FloatCastOverflow),
            want_report,
            "`{what}`: gcc {} report this, chiero gave {kinds:?}",
            if want_report { "does" } else { "does not" }
        );
        if !want_report {
            // Asserted only where the program is defined — gcc's value for a conversion it
            // has already called undefined is one implementation's answer, not the answer.
            assert_eq!(value, Some(want_value), "and computes gcc's answer");
        }
    }
}

/// **`INT_MIN / -1` is undefined, and it is neither a division by zero nor an `Add`/`Sub`/`Mul`.**
///
/// C11 6.5.5p6: if the quotient is not representable, the behaviour is undefined. On a
/// two's-complement machine that is exactly one pair of operands per signed width, and the hardware
/// agrees loudly — x86-64 raises SIGFPE, the same trap as division by zero. UBSan calls it
/// "division of -2147483648 by -1 cannot be represented in type 'int'".
///
/// It falls between chiero's two arms: the `DivByZero` clause tests `y == 0`, and the
/// `SignedOverflow` clause covers `Add`, `Sub` and `Mul`. Found by asking what the `SignedOverflow`
/// grid's *operator* axis contains, which is the wave 261–262 technique pointed at its third kind.
///
/// `SRem` is the same pair for the same reason — `INT_MIN % -1` is the remainder of a division that
/// cannot be performed — and gcc reports it too.
#[test]
fn a_signed_division_whose_quotient_does_not_fit_is_reported() {
    for (what, src) in [
        (
            "int div",
            "int probe(void) { int a = -2147483647 - 1; int b = -1; return a / b; }",
        ),
        (
            "int rem",
            "int probe(void) { int a = -2147483647 - 1; int b = -1; return a % b; }",
        ),
        (
            "long div",
            "int probe(void) { long a = -9223372036854775807L - 1; long b = -1; \
             return (int)(a / b); }",
        ),
    ] {
        let kinds = ub_kinds(src);
        assert!(
            kinds.contains(&UbKind::SignedOverflow) || kinds.contains(&UbKind::DivByZero),
            "`{what}` has no representable quotient, which C11 6.5.5p6 makes undefined and the \
             hardware makes a trap: {kinds:?}"
        );
    }
}
