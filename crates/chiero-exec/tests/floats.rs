//! **Concrete floating point evaluates; symbolic floating point is a declared gap.**
//!
//! Covers: 020 §4.1's total semantics for the float operations CIR already names.
//!
//! CIR has had `FAdd`/`FSub`/`FMul`/`FDiv`/`FRem`, `FNeg`, the ordered comparisons and six
//! FP casts since it was written. The engine implements none of them: `bin` falls through
//! to `_ => None`, the FP cast kinds are unhandled, and `Const::Float` — a float *literal* —
//! does not even become a value. Lowering knows this and refuses any function that mentions
//! a float, so nothing ever reaches the engine to find out.
//!
//! # Why concrete first, and why that is not a half measure
//!
//! `chiero_solver::Sort` is `BitVec`/`Bool`/`Array`. There is no float sort, so a *symbolic*
//! float cannot be constrained, and giving one to the solver means either an FP theory or a
//! bit-blasted encoding — a milestone, not a wave.
//!
//! Concrete floats need none of that. `sort_of` already maps `F32` to `BitVec(32)` and `F64`
//! to `BitVec(64)`, `Const::Float` already carries raw bits so NaN payloads survive, and the
//! evaluation is `f64::from_bits`, an arithmetic operator, `to_bits`. What that unlocks is
//! not a corner: wave 166 measured the generator refusing **293 of 600** programs for
//! floating point, and every one of them is a closed `int probe(void)` where every float is
//! concrete.
//!
//! The last test is the constraint that keeps this honest. A symbolic float operand must
//! stay a *declared* gap — `Fidelity::Unknown` — and must not be quietly folded as though
//! its bits were a number, which is the failure this crate has spent twenty waves not
//! committing.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
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

fn f64c(v: f64) -> Operand {
    Operand::Const(Const::Float(FloatKind::F64, v.to_bits()))
}

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

/// A function whose body is `insts` and which returns `ret`.
fn run(insts: Vec<Inst>, ret: Operand, ret_ty: CTy) -> (RunResult, TermArena) {
    let m = Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: ret_ty,
            variadic: false,
            allocas: vec![],
            blocks: vec![Block {
                id: BlockId(0),
                insts,
                term: Terminator::Return(Some(ret)),
                gcov_lines: Default::default(),
                span: at(1),
            }],
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    (r, a)
}

/// **A float literal is a value.**
#[test]
fn a_float_constant_becomes_its_bits() {
    let (r, mut a) = run(
        vec![inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Use(f64c(2.5)),
            },
            10,
        )],
        Operand::Value(ValueId(0)),
        CTy::Float(FloatKind::F64),
    );
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(u128::from(2.5f64.to_bits())),
        "`Const::Float` carries raw bits precisely so they survive; the engine should keep \
         them. Fidelity: {:?}",
        r.states()[0].fidelity()
    );
}

/// **Arithmetic on concrete floats produces the answer the hardware would.**
///
/// Not "close to": the operations are IEEE-754 and so is `f64`, so the bit patterns match
/// exactly. A test written with a tolerance would pass on an implementation that had
/// rounded twice.
#[test]
fn concrete_float_arithmetic_is_exact() {
    for (op, x, y, want) in [
        (BinOp::FAdd, 2.5f64, 1.25f64, 3.75f64),
        (BinOp::FSub, 2.5, 1.25, 1.25),
        (BinOp::FMul, 2.5, 4.0, 10.0),
        (BinOp::FDiv, 10.0, 4.0, 2.5),
        (BinOp::FRem, 10.0, 4.0, 2.0),
        // A value no binary fraction represents, so a wrong rounding shows up.
        (BinOp::FDiv, 1.0, 3.0, 1.0f64 / 3.0f64),
    ] {
        let (r, mut a) = run(
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op,
                        ty: CTy::Float(FloatKind::F64),
                        a: f64c(x),
                        b: f64c(y),
                    },
                },
                10,
            )],
            Operand::Value(ValueId(0)),
            CTy::Float(FloatKind::F64),
        );
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(u128::from(want.to_bits())),
            "{op:?} {x} {y} should be {want}"
        );
    }
}

/// **Integers and floats convert both ways.**
///
/// `SiToFp` then `FpToSi` is the shape every `(int)(x * 1.5)` in C reduces to, and the
/// truncation-toward-zero of `FpToSi` is the part that is easy to get wrong in the
/// direction nobody notices — `-2.7` becomes `-2`, not `-3`.
#[test]
fn integers_and_floats_convert_both_ways() {
    for (v, want) in [(7i128, 7i128), (-7, -7)] {
        let (r, mut a) = run(
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Cast {
                            kind: CastKind::SiToFp,
                            to: CTy::Float(FloatKind::F64),
                            from: CTy::Int(32),
                            a: i32c(v),
                        },
                    },
                    10,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cast {
                            kind: CastKind::FpToSi,
                            to: CTy::Int(32),
                            from: CTy::Float(FloatKind::F64),
                            a: Operand::Value(ValueId(0)),
                        },
                    },
                    11,
                ),
            ],
            Operand::Value(ValueId(1)),
            CTy::Int(32),
        );
        assert_eq!(
            r.states()[0]
                .return_value_bits(&mut a)
                .map(|b| b as u32 as i32 as i128),
            Some(want),
            "{v} through a double and back"
        );
    }
}

/// **Truncation toward zero, which is C's rule and not the obvious one.**
#[test]
fn a_float_to_int_cast_truncates_toward_zero() {
    for (v, want) in [(2.7f64, 2i128), (-2.7, -2), (2.0, 2), (-0.5, 0)] {
        let (r, mut a) = run(
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Cast {
                        kind: CastKind::FpToSi,
                        to: CTy::Int(32),
                        from: CTy::Float(FloatKind::F64),
                        a: f64c(v),
                    },
                },
                10,
            )],
            Operand::Value(ValueId(0)),
            CTy::Int(32),
        );
        assert_eq!(
            r.states()[0]
                .return_value_bits(&mut a)
                .map(|b| b as u32 as i32 as i128),
            Some(want),
            "(int){v} is {want} in C"
        );
    }
}

/// **A symbolic float is still a declared gap, not a folded guess.**
///
/// The constraint on everything above. There is no float sort in the solver, so a symbolic
/// float cannot be constrained — and the tempting shortcut, treating its bits as a number
/// and folding anyway, produces a confident wrong answer. `Fidelity::Unknown` is the
/// honest outcome and the one 023 §7 exists to express.
#[test]
fn a_symbolic_float_operand_is_unknown_not_folded() {
    let (r, _) = run(
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh {
                        ty: CTy::Float(FloatKind::F64),
                    },
                },
                5,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin {
                        op: BinOp::FAdd,
                        ty: CTy::Float(FloatKind::F64),
                        a: Operand::Value(ValueId(0)),
                        b: f64c(1.0),
                    },
                },
                10,
            ),
        ],
        Operand::Value(ValueId(1)),
        CTy::Float(FloatKind::F64),
    );
    assert_eq!(
        r.states()[0].fidelity(),
        Fidelity::Unknown,
        "no float sort exists, so this cannot be modelled and must say so"
    );
}

fn f32c(v: f32) -> Operand {
    Operand::Const(Const::Float(FloatKind::F32, u64::from(v.to_bits())))
}

/// **The single-precision path is a second implementation and needs its own fixtures.**
///
/// `f32` and `f64` are separate arms computing at separate widths, so every test written
/// only in `double` leaves half the arithmetic unchecked — a mutation making `FAdd` subtract
/// in the 32-bit arm survived the whole suite until this existed.
///
/// The values are chosen so single precision *shows*: `0.1f + 0.2f` is a different bit
/// pattern from the double it would widen to, so an implementation that computed in `f64`
/// and narrowed at the end fails here.
#[test]
fn single_precision_arithmetic_is_exact_at_its_own_width() {
    for (op, x, y, want) in [
        (BinOp::FAdd, 2.5f32, 1.25f32, 3.75f32),
        (BinOp::FSub, 2.5, 1.25, 1.25),
        (BinOp::FMul, 2.5, 4.0, 10.0),
        (BinOp::FDiv, 10.0, 4.0, 2.5),
        (BinOp::FRem, 10.0, 4.0, 2.0),
        (BinOp::FAdd, 0.1, 0.2, 0.1f32 + 0.2f32),
        (BinOp::FDiv, 1.0, 3.0, 1.0f32 / 3.0f32),
    ] {
        let (r, mut a) = run(
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op,
                        ty: CTy::Float(FloatKind::F32),
                        a: f32c(x),
                        b: f32c(y),
                    },
                },
                10,
            )],
            Operand::Value(ValueId(0)),
            CTy::Float(FloatKind::F32),
        );
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(u128::from(want.to_bits())),
            "{op:?} {x}f {y}f should be {want}f at single precision"
        );
    }
}

/// **`SiToFp` reads its source as signed and `UiToFp` does not.**
///
/// Asserted on the *float* rather than on a round trip, because a round trip through 32 bits
/// hides it: `-7` read as unsigned is 4294967289, and truncating that back to `int` gives
/// `-7` again. A mutation making `SiToFp` unsigned survived the round-trip test for exactly
/// that reason, and the two conversions genuinely differ — one produces `-7.0` and the other
/// four billion.
#[test]
fn the_two_int_to_float_casts_differ_in_signedness() {
    for (kind, want) in [
        (CastKind::SiToFp, -7.0f64),
        (CastKind::UiToFp, 4_294_967_289.0f64),
    ] {
        let (r, mut a) = run(
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Cast {
                        kind,
                        to: CTy::Float(FloatKind::F64),
                        from: CTy::Int(32),
                        a: i32c(-7),
                    },
                },
                10,
            )],
            Operand::Value(ValueId(0)),
            CTy::Float(FloatKind::F64),
        );
        assert_eq!(
            r.states()[0].return_value_bits(&mut a),
            Some(u128::from(want.to_bits())),
            "{kind:?} of the bits of -7 is {want}"
        );
    }
}

/// **Narrowing and widening between the two precisions.**
///
/// `FpTrunc` loses bits and must lose exactly the ones the hardware would; `FpExt` is
/// lossless. Round-tripping a value single precision cannot hold is what shows the first
/// actually narrowed.
#[test]
fn the_two_precisions_convert_both_ways() {
    let (r, mut a) = run(
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Cast {
                        kind: CastKind::FpTrunc,
                        to: CTy::Float(FloatKind::F32),
                        from: CTy::Float(FloatKind::F64),
                        a: f64c(0.1),
                    },
                },
                10,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Cast {
                        kind: CastKind::FpExt,
                        to: CTy::Float(FloatKind::F64),
                        from: CTy::Float(FloatKind::F32),
                        a: Operand::Value(ValueId(0)),
                    },
                },
                11,
            ),
        ],
        Operand::Value(ValueId(1)),
        CTy::Float(FloatKind::F64),
    );
    assert_eq!(
        r.states()[0].return_value_bits(&mut a),
        Some(u128::from(f64::from(0.1f32).to_bits())),
        "0.1 narrowed to single and widened back is single's 0.1, not double's"
    );
}

/// **A float-to-integer conversion whose value does not fit is undefined** (C11 6.3.1.4).
///
/// "If the value of the integral part cannot be represented by the integer type, the
/// behavior is undefined." Every other UB the engine records is a *binary operation*, so
/// `note_ub` is reached from `RValue::Bin` and a conversion never passes through it —
/// `UbKind` has `SignedOverflow`, `Shift` and `DivByZero` and nothing for a cast.
///
/// Raised by a reader asking whether UB should be warned about rather than merely discarded
/// from the differential channel. The answer is yes, and it turned out chiero already warns
/// about the other three: `7 / z`, `1 << 33` and `INT_MAX + 1` all produce findings, and
/// this produces silence.
///
/// **Silence is the wrong answer twice over here.** The value chiero computes is Rust's
/// saturating `as`, which is a *defensible* number and nothing like the one the hardware
/// gives — so the run continues with a plausible wrong value and says nothing about why.
/// That is the failure the whole `UbEvent` mechanism exists to prevent.
///
/// The negative half is the constraint: an in-range conversion is ordinary C that programs
/// do constantly, and a check that fired on every float-to-integer cast would bury the real
/// ones.
#[test]
fn a_float_to_integer_conversion_out_of_range_is_undefined() {
    let case = |v: f64, to: CTy| {
        let (r, _) = run(
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Cast {
                        kind: CastKind::FpToSi,
                        to: to.clone(),
                        from: CTy::Float(FloatKind::F64),
                        a: f64c(v),
                    },
                },
                10,
            )],
            Operand::Value(ValueId(0)),
            to,
        );
        // Counted rather than matched on a kind: the fixture contains one instruction and
        // no other undefined operation, so any event here is this one — and asserting the
        // *behaviour* keeps this test red for the right reason rather than for a name that
        // does not exist yet.
        r.states()[0].ub_events().len()
    };

    // Out of range for the destination.
    assert_eq!(
        case(-4_294_905_087.0, CTy::Int(16)),
        1,
        "-4.29e9 into 16 bits"
    );
    assert_eq!(case(1e30, CTy::Int(32)), 1, "1e30 into 32 bits");
    assert_eq!(case(-1e30, CTy::Int(32)), 1, "-1e30 into 32 bits");
    // NaN has no integral part at all, which C11 6.3.1.4 also leaves undefined.
    assert_eq!(case(f64::NAN, CTy::Int(32)), 1, "NaN into any integer");

    // **In range, and therefore ordinary.** A check that fired on every conversion would
    // report these too, and they are what C programs are made of.
    assert_eq!(case(2.7, CTy::Int(32)), 0, "2.7 truncates to 2");
    assert_eq!(case(-2.7, CTy::Int(32)), 0, "-2.7 truncates to -2");
    assert_eq!(case(2_147_483_647.0, CTy::Int(32)), 0, "INT_MAX exactly");
    assert_eq!(case(-2_147_483_648.0, CTy::Int(32)), 0, "INT_MIN exactly");

    // **The first value past each end**, which is where an inclusive/exclusive slip lives.
    // `2^31` is one above `INT_MAX` and is *exactly* representable as a double, so it is
    // the value a `>` instead of a `>=` lets through.
    assert_eq!(case(2_147_483_648.0, CTy::Int(32)), 1, "one past INT_MAX");
    assert_eq!(case(-2_147_483_649.0, CTy::Int(32)), 1, "one past INT_MIN");

    // **The rule is about the *integral part***, so a fraction beyond the bound is still in
    // range: `(int)-2147483648.5` truncates *toward zero* to `INT_MIN`, which fits. Testing
    // the raw value instead of the truncated one reports this as undefined, and C says it
    // is not.
    assert_eq!(
        case(-2_147_483_648.5, CTy::Int(32)),
        0,
        "truncates to INT_MIN"
    );
    assert_eq!(
        case(2_147_483_647.9, CTy::Int(32)),
        0,
        "truncates to INT_MAX"
    );
}

/// **The unsigned conversion has its own range, and its own way of being wrong.**
///
/// `FpToUi` shares `fcast` with `FpToSi` and nothing else: the bound is `2^bits` rather than
/// `2^(bits-1)`, and the lower end is **zero** rather than a negative. A check written only
/// against the signed rule accepts `(unsigned)(-1.0)`, which C11 6.3.1.4 leaves undefined —
/// and which Rust's saturating `as` turns into a confident `0`.
///
/// `-0.5` is the discriminator for the truncation rule on this side: its integral part is
/// `0`, which is perfectly representable, so it is *defined* despite being negative.
#[test]
fn an_unsigned_float_conversion_has_its_own_range() {
    let case = |v: f64, to: CTy| {
        let (r, _) = run(
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Cast {
                        kind: CastKind::FpToUi,
                        to: to.clone(),
                        from: CTy::Float(FloatKind::F64),
                        a: Operand::Const(Const::Float(FloatKind::F64, v.to_bits())),
                    },
                },
                10,
            )],
            Operand::Value(ValueId(0)),
            to,
        );
        r.states()[0].ub_events().len()
    };

    // Negative is out of range for every unsigned type, however small.
    assert_eq!(case(-1.0, CTy::Int(32)), 1, "-1.0 into unsigned");
    assert_eq!(case(-4_294_905_087.0, CTy::Int(16)), 1, "very negative");
    // Past the top: `2^32` is one above `UINT_MAX`.
    assert_eq!(case(4_294_967_296.0, CTy::Int(32)), 1, "one past UINT_MAX");
    assert_eq!(case(f64::NAN, CTy::Int(32)), 1, "NaN into unsigned");

    // In range, including the value a signed bound would wrongly reject: `2^31` fits an
    // unsigned 32-bit type and does not fit a signed one.
    assert_eq!(case(0.0, CTy::Int(32)), 0, "zero");
    assert_eq!(case(2_147_483_648.0, CTy::Int(32)), 0, "2^31 fits unsigned");
    assert_eq!(case(4_294_967_295.0, CTy::Int(32)), 0, "UINT_MAX exactly");
    // **Truncation toward zero happens first**: -0.5 has integral part 0.
    assert_eq!(case(-0.5, CTy::Int(32)), 0, "-0.5 truncates to 0");
}
