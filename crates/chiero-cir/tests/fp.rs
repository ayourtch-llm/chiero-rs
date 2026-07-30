//! x87's 80-bit format, at the level where it can be tested without a program.
//!
//! Most of `fp` is exercised through `chiero-lower`'s differential fixtures, which is the right place
//! — they compare against gcc. Two things cannot be reached that way yet:
//!
//!   - **a NaN**, because producing one in C needs `0.0L / 0.0L` and `f80` division is still a gap
//!   - **an exponent outside x87's own range**, which no C literal in the fixtures reaches
//!
//! Both are pure-function questions, so they are answered here rather than left until arithmetic
//! lands.

use chiero_cir::fp;
use core::cmp::Ordering;

/// A quiet NaN: the all-ones exponent with more than the bare integer bit set.
const NAN: u128 = (0x7fff << 64) | (1u128 << 63) | 1;
const INF: u128 = (0x7fff << 64) | (1u128 << 63);
const ONE: u128 = 0x3fff_8000_0000_0000_0000;

/// **Every comparison with a NaN is unordered** (IEEE-754 §5.11), `x == x` included.
///
/// `None` here means *unordered*, which the caller has to turn into `false` for an ordered operator
/// and `true` for `!=`. A comparison that treated it as "no answer" would make the engine declare a
/// gap where C has a defined result.
#[test]
fn a_nan_compares_unordered_with_everything_including_itself() {
    assert_eq!(
        fp::partial_cmp(NAN, NAN),
        None,
        "a NaN is not even equal to itself"
    );
    assert_eq!(fp::partial_cmp(NAN, ONE), None);
    assert_eq!(fp::partial_cmp(ONE, NAN), None);
    assert!(fp::is_nan(NAN));
    // Infinity is *not* a NaN, and the two differ only in the significand — which is exactly the
    // distinction a check on the exponent alone would miss.
    assert!(!fp::is_nan(INF), "infinity is ordered");
    assert_eq!(fp::partial_cmp(INF, ONE), Some(Ordering::Greater));
    assert_eq!(fp::partial_cmp(INF, INF), Some(Ordering::Equal));
}

/// Both zeros are equal, and that is the one place the sign bit does not decide the order.
#[test]
fn the_two_zeros_are_equal() {
    let pos = 0u128;
    let neg = 1u128 << 79;
    assert_eq!(fp::partial_cmp(pos, neg), Some(Ordering::Equal));
    assert_eq!(fp::partial_cmp(neg, pos), Some(Ordering::Equal));
    assert!(fp::is_zero(pos) && fp::is_zero(neg));
    // And a zero is below any positive and above any negative.
    assert_eq!(fp::partial_cmp(pos, ONE), Some(Ordering::Less));
    assert_eq!(
        fp::partial_cmp(neg, ONE | (1 << 79)),
        Some(Ordering::Greater)
    );
}

/// **An exponent outside x87's range is a gap, not an infinity.**
///
/// `from_u64_scaled` refuses rather than wrapping, at both ends. Silently producing an infinity for a
/// value that overflowed would be a number where a limit belongs, and wrapping the exponent field
/// would be a plausible wrong number — the worse of the two.
#[test]
fn a_scaled_exponent_outside_the_range_is_refused() {
    // 1.0 shifted up and down past the fifteen-bit exponent's reach.
    assert_eq!(fp::from_u64_scaled(1, 20_000, false), None, "past the top");
    assert_eq!(
        fp::from_u64_scaled(1, -20_000, false),
        None,
        "past the bottom"
    );
    // And just inside it still works, so the bound is a bound rather than a refusal of everything.
    assert!(fp::from_u64_scaled(1, 16_000, false).is_some());
    assert!(fp::from_u64_scaled(1, -16_000, false).is_some());
    // Zero is representable at any scale, because there is no exponent to move.
    assert_eq!(fp::from_u64_scaled(0, 20_000, false), Some(0));
}

/// An 80-bit pattern from a biased exponent and a significand, for the cases no C literal reaches.
fn f80(exp: u32, sig: u64, neg: bool) -> u128 {
    (u128::from(neg) << 79) | (u128::from(exp) << 64) | u128::from(sig)
}

/// **A product past the top exponent is an infinity; a product past the bottom is a gap.**
///
/// The asymmetry is deliberate and is the whole of `mul`'s relationship with 023 §7. IEEE-754 §7.4
/// says an overflowed product *is* an infinity — a value, fully specified, and refusing to produce it
/// would declare a limit chiero does not have. Underflow is the opposite: the answer is a denormal,
/// `mul` does not shift denormals, and the plausible substitute is a zero. A zero is a confident
/// statement that a very small number is nothing, so `None` is the honest answer and §7 gets a
/// declared limit instead of a wrong one.
///
/// Neither end is reachable from a C fixture — gcc folds an overflowing constant product at compile
/// time — which is why they are answered here.
#[test]
fn a_product_overflows_to_infinity_and_underflows_to_a_gap() {
    let big = f80(32_000, 1 << 63, false);
    assert_eq!(
        fp::mul(big, big),
        Some(INF),
        "2^15617 squared is past the top exponent, and §7.4 makes that an infinity"
    );
    assert_eq!(
        fp::mul(big, f80(32_000, 1 << 63, true)),
        Some(INF | (1 << 79)),
        "and it keeps the sign it earned"
    );
    let tiny = f80(100, 1 << 63, false);
    assert_eq!(
        fp::mul(tiny, tiny),
        None,
        "the answer is a denormal, and a zero here would be a wrong number rather than a gap"
    );
    // The bound is a bound: just inside it, both ends still produce values.
    assert!(fp::mul(f80(20_000, 1 << 63, false), f80(20_000, 1 << 63, false)).is_some());
    assert!(fp::mul(f80(9_000, 1 << 63, false), f80(9_000, 1 << 63, false)).is_some());
}

/// **Zero times infinity is a gap, and it is the only zero that is.**
///
/// IEEE-754 §7.2 makes it an invalid operation whose result is a NaN. Nothing in `fp` mints NaNs, so
/// the honest answer is that there is not one — and the two plausible wrong answers are both *values*
/// a reader would believe: the zero the first operand suggests and the infinity the second does.
///
/// The control matters as much: an ordinary zero product must still be a zero, or a fix that refused
/// every zero would satisfy the assertion above and take multiplication by zero out of the engine.
#[test]
fn zero_times_infinity_is_a_gap_but_zero_times_a_number_is_not() {
    assert_eq!(fp::mul(0, INF), None, "IEEE-754 §7.2's invalid operation");
    assert_eq!(fp::mul(INF, 0), None, "in either order");
    assert_eq!(fp::mul(1 << 79, INF), None, "and for the negative zero too");
    // The control. An ordinary zero product is a zero, signed by the operands.
    assert_eq!(fp::mul(0, ONE), Some(0));
    assert_eq!(fp::mul(0, ONE | (1 << 79)), Some(1 << 79), "0 × -1 is -0");
    assert_eq!(fp::mul(1 << 79, ONE | (1 << 79)), Some(0), "-0 × -1 is +0");
    // And infinity times a number is that infinity, which is what makes the pair above a *special*
    // case rather than a general refusal of either operand.
    assert_eq!(fp::mul(INF, ONE), Some(INF));
    assert_eq!(fp::mul(INF, ONE | (1 << 79)), Some(INF | (1 << 79)));
}

/// **A NaN operand is a gap, because `mul` has no NaN to return.**
///
/// §6.2 says a product with a NaN operand is that NaN, payload and all. `fp` does not propagate
/// payloads, so producing *some* NaN would be inventing one — and the alternative a mutant reaches
/// for is worse: with the check removed, a NaN's all-ones exponent falls through to the infinity arm
/// and `NaN × 2` becomes an infinity, which compares ordered against everything.
#[test]
fn a_nan_operand_is_a_gap_rather_than_an_infinity() {
    assert_eq!(fp::mul(NAN, ONE), None);
    assert_eq!(fp::mul(ONE, NAN), None);
    assert_eq!(fp::mul(NAN, NAN), None);
    assert_eq!(fp::mul(NAN, INF), None, "not the infinity either");
    assert_eq!(fp::mul(NAN, 0), None, "and not the zero");
}

/// **A decimal literal past x87's range is an infinity at the top and a gap at the bottom.**
///
/// The same asymmetry `mul` has and for the same reason, tested here because no C fixture reaches
/// it: gcc folds an out-of-range constant at compile time, so the literal never arrives.
///
/// The pairs matter more than the values. `1e5000` is settled by the magnitude shortcut — a value
/// whose digit count plus exponent is past `DECIMAL_LIMIT` cannot be represented, and confirming
/// that by building a five-thousand-digit integer would be work with a known answer. `2e4932` is
/// *not*: its magnitude is inside the shortcut's bound and outside the format's, so it is the
/// exponent check at the end that has to catch it. A fix that only had the shortcut would pass the
/// first and fail the second.
#[test]
fn a_decimal_past_the_format_s_range_overflows_or_refuses() {
    let inf = INF;
    assert_eq!(
        fp::from_decimal("1", 5000, false),
        Some(inf),
        "the shortcut"
    );
    assert_eq!(
        fp::from_decimal("2", 4932, false),
        Some(inf),
        "inside the shortcut's bound and past the format's, so the exponent check must catch it"
    );
    assert_eq!(
        fp::from_decimal("1", 5000, true),
        Some(inf | (1 << 79)),
        "and it keeps its sign"
    );
    assert_eq!(fp::from_decimal("1", -5000, false), None, "the shortcut");
    assert_eq!(
        fp::from_decimal("1", -4933, false),
        None,
        "and the computed path reaches the same answer for a subnormal"
    );
    // The bound is a bound: values inside it still convert.
    assert!(fp::from_decimal("1", 4000, false).is_some());
    assert!(fp::from_decimal("1", -4000, false).is_some());
    // Zero has no exponent to move, so no scale puts it out of range.
    assert_eq!(fp::from_decimal("0", 9000, false), Some(0));
    assert_eq!(fp::from_decimal("000", -9000, true), Some(1 << 79));
}

/// **A zero out of addition has a sign the operands do not decide.**
///
/// Two rules from IEEE-754 §6.3, both specific to round-to-nearest, and both invisible to a C fixture
/// because `-0 == 0` — the only way to see the sign is to look at the bits, which is what this does.
///
/// A sum of two zeros is negative *only when both are*: `+0 + -0` is defined to be `+0` rather than
/// left to the operands. And `x - x` is `+0` for every finite `x`, its own sign included — so the one
/// case where the result is exactly zero is the one case where the larger operand's sign, which
/// decides every other result, does not decide this one.
#[test]
fn an_exact_zero_from_addition_is_positive_unless_both_operands_were_negative() {
    let (pz, nz) = (0u128, 1u128 << 79);
    let neg_one = ONE | (1 << 79);
    assert_eq!(fp::add(pz, nz), Some(pz), "§6.3: `+0 + -0` is `+0`");
    assert_eq!(fp::add(nz, pz), Some(pz), "in either order");
    assert_eq!(fp::add(nz, nz), Some(nz), "and only two negatives make one");
    assert_eq!(fp::add(pz, pz), Some(pz));
    // `x - x` is `+0` whatever `x` was, which is the rule that overrides the sign of the larger
    // operand — and the negative case is the one that shows it, since the positive case would be
    // satisfied by taking the sign from the operands.
    assert_eq!(fp::sub(ONE, ONE), Some(pz));
    assert_eq!(
        fp::sub(neg_one, neg_one),
        Some(pz),
        "`-1 - -1` is `+0`, not `-0`"
    );
    assert_eq!(fp::add(ONE, neg_one), Some(pz));
    assert_eq!(fp::add(neg_one, ONE), Some(pz));
    // The control: a nonzero result still takes the larger operand's sign.
    assert_eq!(fp::add(neg_one, 0), Some(neg_one));
    assert!(
        fp::sub(ONE, neg_one).is_some_and(|v| v >> 79 == 0),
        "1 - -1 is +2"
    );
}

/// **`∞ - ∞` is a gap; every other infinity in a sum is an answer.**
///
/// IEEE-754 §7.2's invalid operation, whose result is a NaN `fp` does not mint — the same refusal
/// `mul` makes for `0 × ∞`, reached through the other operation. Unreachable from a C fixture without
/// division, and the plausible wrong answer is a *value*: with the sign test dropped, `∞ - ∞` returns
/// an infinity that then compares ordered against everything.
#[test]
fn adding_opposite_infinities_is_a_gap() {
    let ninf = INF | (1 << 79);
    assert_eq!(fp::add(INF, ninf), None, "§7.2's invalid operation");
    assert_eq!(fp::add(ninf, INF), None, "in either order");
    assert_eq!(
        fp::sub(INF, INF),
        None,
        "which is the same question spelled as a subtraction"
    );
    // The control. Same-sign infinities, and an infinity beside anything finite, are answers.
    assert_eq!(fp::add(INF, INF), Some(INF));
    assert_eq!(fp::add(ninf, ninf), Some(ninf));
    assert_eq!(fp::add(INF, ONE), Some(INF));
    assert_eq!(fp::add(ONE, ninf), Some(ninf));
    assert_eq!(fp::sub(ninf, INF), Some(ninf));
    assert_eq!(fp::add(INF, 0), Some(INF), "and beside a zero");
    // A NaN anywhere is still a gap, which is checked before the infinities are.
    assert_eq!(fp::add(NAN, INF), None);
    assert_eq!(fp::sub(ONE, NAN), None);
}
