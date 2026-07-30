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
fn a_scaled_exponent_outside_the_range_saturates_rather_than_refusing() {
    // 1.0 shifted up and down past the fifteen-bit exponent's reach.
    // Wave 244: neither end refuses any more, and both answers were read off the hardware.
    // `0x1p20000L` is an infinity there and `0x1p-20000L` is a zero — a literal too small to
    // represent is not a limit, it is a value that rounds to nothing.
    assert_eq!(
        fp::from_u64_scaled(1, 20_000, false),
        Some((0x7fffu128 << 64) | (1 << 63)),
        "past the top is §7.4's infinity"
    );
    assert_eq!(
        fp::from_u64_scaled(1, -20_000, false),
        Some(0),
        "past the bottom rounds to zero"
    );
    // And in the subnormal band it is a *value*, with the exponent field pinned at zero.
    let sub = fp::from_u64_scaled(1, -16_400, false).expect("subnormals are representable");
    assert_eq!(
        (sub >> 64) & 0x7fff,
        0,
        "subnormal: the exponent field is zero"
    );
    assert_ne!(sub & u128::from(u64::MAX), 0, "and the significand is not");
    // And just inside it still works, so the bound is a bound rather than a refusal of everything.
    assert!(fp::from_u64_scaled(1, 16_000, false).is_some());
    assert!(fp::from_u64_scaled(1, -16_000, false).is_some());
    // Zero is representable at any scale, because there is no exponent to move.
    assert_eq!(fp::from_u64_scaled(0, 20_000, false), Some(0));
}

/// x87's exponent bias, as a `u32`, for building patterns by hand.
const BIAS_U32: u32 = 16383;

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
fn a_product_overflows_to_infinity_and_underflows_through_the_subnormals() {
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
    // **Wave 244 changed this from `None` to a zero, and the zero is the right answer.** Before
    // subnormals existed, refusing was the honest response to a result below the floor. Now the
    // floor is `2^-16445` and this product is `2^-32566` — not merely subnormal but far under the
    // smallest one, so it rounds to zero the way the hardware does, which was checked rather than
    // assumed. The assertion had to change because the capability did; wave 237's rule, again.
    let tiny = f80(100, 1 << 63, false);
    assert_eq!(
        fp::mul(tiny, tiny),
        Some(0),
        "`2^-32566` is below half the smallest subnormal, so it rounds to zero"
    );
    // A product that lands *inside* the subnormal range keeps bits, which is the case a fix that
    // simply replaced the refusal with a zero would get wrong.
    let small = f80(1, 1 << 63, false);
    let half = f80(BIAS_U32 - 1, 1 << 63, false);
    let got = fp::mul(small, half).expect("a subnormal is a value");
    assert_eq!(
        (got >> 64) & 0x7fff,
        0,
        "the exponent field of a subnormal is zero"
    );
    assert_eq!(
        got & u128::from(u64::MAX),
        1 << 62,
        "and the integer bit is clear, which is what makes it subnormal"
    );
    // The bound is a bound: just inside it, both ends still produce values.
    assert!(fp::mul(f80(20_000, 1 << 63, false), f80(20_000, 1 << 63, false)).is_some());
    assert!(fp::mul(f80(9_000, 1 << 63, false), f80(9_000, 1 << 63, false)).is_some());
}

/// **Zero times infinity is the indefinite, and it is the only zero product that is not a zero.**
///
/// IEEE-754 §7.2's invalid operation. Wave 243 turned this from a declared gap into x87's own answer:
/// the "real indefinite", which is negative and carries the quiet bit and nothing else. The two
/// plausible wrong answers are still both *values* a reader would believe — the zero the first
/// operand suggests and the infinity the second does — so the assertion is on the exact pattern
/// rather than merely on "it is a NaN".
///
/// The control matters as much: an ordinary zero product must still be a zero, or a fix that made
/// every zero indefinite would satisfy the assertion above and take multiplication by zero out of
/// the engine.
#[test]
fn zero_times_infinity_is_the_indefinite_but_zero_times_a_number_is_not() {
    assert_eq!(
        fp::mul(0, INF),
        Some(fp::INDEFINITE),
        "§7.2's invalid operation"
    );
    assert_eq!(fp::mul(INF, 0), Some(fp::INDEFINITE), "in either order");
    assert_eq!(
        fp::mul(1 << 79, INF),
        Some(fp::INDEFINITE),
        "and for the negative zero too"
    );
    assert!(
        fp::is_nan(fp::INDEFINITE),
        "which is a NaN, whatever else it is"
    );
    // The control. An ordinary zero product is a zero, signed by the operands.
    assert_eq!(fp::mul(0, ONE), Some(0));
    assert_eq!(fp::mul(0, ONE | (1 << 79)), Some(1 << 79), "0 × -1 is -0");
    assert_eq!(fp::mul(1 << 79, ONE | (1 << 79)), Some(0), "-0 × -1 is +0");
    // And infinity times a number is that infinity, which is what makes the pair above a *special*
    // case rather than a general refusal of either operand.
    assert_eq!(fp::mul(INF, ONE), Some(INF));
    assert_eq!(fp::mul(INF, ONE | (1 << 79)), Some(INF | (1 << 79)));
}

/// **A NaN operand comes back out, and it is the same NaN.**
///
/// This test used to assert `None` — "a gap, because `mul` has no NaN to return" — and it was right
/// when it was written, before division made a NaN reachable from C at all. Wave 243 gave `fp` the
/// hardware's own rule, so what was a declared gap is now an exact value, and the assertion had to
/// change with it. That is the same thing wave 237 found in the other direction: **a new capability
/// makes existing assertions wrong, and the honest ones fail loudly rather than quietly passing.**
///
/// Three claims, each of which a canonical NaN would break: the payload survives, the sign survives,
/// and a signalling NaN comes out quiet.
#[test]
fn a_nan_operand_comes_back_with_its_payload() {
    let payload = (0x7fffu128 << 64) | 0xC000_0000_0000_0123;
    assert_eq!(
        fp::mul(payload, ONE),
        Some(payload),
        "the payload is not a detail to round off"
    );
    assert_eq!(fp::mul(ONE, payload), Some(payload), "in either order");
    assert_eq!(
        fp::mul(payload | (1 << 79), ONE),
        Some(payload | (1 << 79)),
        "and the NaN's own sign, not the product's — `-NaN × 1` is `-NaN`, not `-NaN` by accident"
    );
    // A signalling NaN — quiet bit clear — comes out with it set and nothing else touched.
    let signalling = (0x7fffu128 << 64) | 0x8000_0000_0000_0123;
    assert_eq!(
        fp::mul(signalling, ONE),
        Some(payload),
        "quieted, and only quieted"
    );
    // With two, the larger significand wins.
    let bigger = (0x7fffu128 << 64) | 0xC000_0000_000A_BCDE;
    assert_eq!(fp::mul(payload, bigger), Some(bigger));
    assert_eq!(
        fp::mul(bigger, payload),
        Some(bigger),
        "which makes the choice order-independent"
    );
    // A NaN beside anything at all is still that NaN.
    assert_eq!(fp::mul(payload, INF), Some(payload), "not the infinity");
    assert_eq!(fp::mul(payload, 0), Some(payload), "and not the zero");
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
fn a_decimal_past_the_format_s_range_overflows_or_rounds_to_zero() {
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
    // Wave 244: the bottom is no longer a refusal, and the two ends are no longer mirror images.
    // `1e-5000` is below half the smallest subnormal and rounds to zero; `1e-4933` is an ordinary
    // *subnormal*, nineteen orders of magnitude of representable values the old bound refused.
    assert_eq!(
        fp::from_decimal("1", -5000, false),
        Some(0),
        "the shortcut, and it rounds to zero"
    );
    let sub = fp::from_decimal("1", -4933, false).expect("`1e-4933` is a subnormal, not a limit");
    assert_eq!(
        (sub >> 64) & 0x7fff,
        0,
        "the exponent field of a subnormal is zero"
    );
    assert_eq!(
        sub & u128::from(u64::MAX),
        0x03ce_a0c7_4b75_2265,
        "and this is the significand x87 gives it — read off a running program, not derived here"
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

/// **`∞ - ∞` is the indefinite; every other infinity in a sum is an answer.**
///
/// IEEE-754 §7.2's invalid operation — the same one `mul` reaches through `0 × ∞`, arrived at by the
/// other operation and giving the same pattern. The plausible wrong answer is a *value*: with the
/// sign test dropped, `∞ - ∞` returns an infinity that then compares ordered against everything.
#[test]
fn adding_opposite_infinities_is_the_indefinite() {
    let ninf = INF | (1 << 79);
    assert_eq!(
        fp::add(INF, ninf),
        Some(fp::INDEFINITE),
        "§7.2's invalid operation"
    );
    assert_eq!(fp::add(ninf, INF), Some(fp::INDEFINITE), "in either order");
    assert_eq!(
        fp::sub(INF, INF),
        Some(fp::INDEFINITE),
        "which is the same question spelled as a subtraction"
    );
    // The control. Same-sign infinities, and an infinity beside anything finite, are answers.
    assert_eq!(fp::add(INF, INF), Some(INF));
    assert_eq!(fp::add(ninf, ninf), Some(ninf));
    assert_eq!(fp::add(INF, ONE), Some(INF));
    assert_eq!(fp::add(ONE, ninf), Some(ninf));
    assert_eq!(fp::sub(ninf, INF), Some(ninf));
    assert_eq!(fp::add(INF, 0), Some(INF), "and beside a zero");
    // A NaN anywhere is that NaN, which is decided before the infinities are.
    assert_eq!(fp::add(NAN, INF), Some(NAN | (1 << 62)));
    assert_eq!(fp::sub(ONE, NAN), Some(NAN | (1 << 62)));
    // **`sub` propagates the NaN it was given, not the one it would have negated.** `1 - -NaN` is
    // `-NaN`: the sign the operand arrived with, which is what x87 does and what flipping before the
    // NaN test would get wrong in exactly the bit a program inspecting a NaN can see.
    let nneg = NAN | (1 << 79);
    assert_eq!(
        fp::sub(ONE, nneg),
        Some(nneg | (1 << 62)),
        "the sign is not flipped on the way"
    );
    assert_eq!(
        fp::sub(ONE, NAN),
        Some(NAN | (1 << 62)),
        "and a positive one stays positive"
    );
}

/// **The invalid operations, the subnormal gap, and the one that looks like both and is neither.**
///
/// `0/0` and `∞/∞` are IEEE-754 §7.2's invalid operations, whose result is x87's indefinite.
/// A quotient below the format's floor is a subnormal, the gap every operation here declines. And
/// **division by zero is not one of them**: §7.3 makes it a defined operation returning an infinity,
/// so a fix that lumped it in with `0/0` would turn a value into a refusal.
///
/// The subnormal quotient is not reachable from a C fixture — `agree` compares values and a declared
/// gap is not one — and gcc folds an out-of-range constant quotient before chiero sees the program.
/// The NaN cases became reachable in wave 243 and are covered from C as well; they are here too
/// because only the bits show *which* NaN came back.
#[test]
fn division_answers_the_invalid_operations_with_the_indefinite_and_zero_with_an_infinity() {
    let (pz, nz) = (0u128, 1u128 << 79);
    let ninf = INF | (1 << 79);
    assert_eq!(
        fp::div(pz, pz),
        Some(fp::INDEFINITE),
        "§7.2: `0/0` is invalid"
    );
    assert_eq!(
        fp::div(nz, pz),
        Some(fp::INDEFINITE),
        "whatever the zeros' signs"
    );
    assert_eq!(
        fp::div(INF, INF),
        Some(fp::INDEFINITE),
        "§7.2: `∞/∞` is invalid"
    );
    assert_eq!(fp::div(INF, ninf), Some(fp::INDEFINITE));
    // §7.3: dividing a *nonzero* by zero is an infinity, signed by both operands.
    assert_eq!(
        fp::div(ONE, pz),
        Some(INF),
        "§7.3's divideByZero is a value"
    );
    assert_eq!(fp::div(ONE, nz), Some(ninf), "and the zero's sign counts");
    assert_eq!(fp::div(ONE | (1 << 79), pz), Some(ninf));
    assert_eq!(fp::div(ONE | (1 << 79), nz), Some(INF));
    // Zero over nonzero, and finite over infinite, are zeros rather than gaps.
    assert_eq!(fp::div(pz, ONE), Some(pz));
    assert_eq!(fp::div(nz, ONE), Some(nz));
    assert_eq!(fp::div(ONE, INF), Some(pz));
    assert_eq!(
        fp::div(ONE, ninf),
        Some(nz),
        "the sign survives the underflow to zero"
    );
    // Infinite over finite is that infinity.
    assert_eq!(fp::div(INF, ONE), Some(INF));
    assert_eq!(fp::div(ninf, ONE), Some(ninf));
    // A NaN anywhere is that NaN, and it is decided before any of the above.
    assert_eq!(fp::div(NAN, ONE), Some(NAN | (1 << 62)));
    assert_eq!(fp::div(ONE, NAN), Some(NAN | (1 << 62)));
    assert_eq!(
        fp::div(NAN, pz),
        Some(NAN | (1 << 62)),
        "even where a `0` divisor would otherwise give an infinity"
    );
}

/// **A quotient outside the format's range: an infinity at the top, a gap at the bottom.**
///
/// The same asymmetry `mul`, `add` and `from_decimal` all have, reached through the fourth operation.
/// The exponent difference does the work here rather than a sum, so this is a distinct arithmetic
/// path to the same two checks.
#[test]
fn a_quotient_outside_the_range_overflows_or_rounds_to_zero() {
    let big = (u128::from(32_000u32) << 64) | (1 << 63);
    let small = (u128::from(300u32) << 64) | (1 << 63);
    assert_eq!(
        fp::div(big, small),
        Some(INF),
        "an exponent difference past the top is §7.4's infinity"
    );
    assert_eq!(
        fp::div(big, small).map(|v| v >> 79),
        Some(0),
        "and it is positive, since both operands were"
    );
    // Wave 244: past the bottom is a value now. This quotient is `2^-31700`, far under the
    // smallest subnormal, so it rounds to zero.
    assert_eq!(
        fp::div(small, big),
        Some(0),
        "past the bottom, and it rounds to zero"
    );
    // A quotient that lands *inside* the subnormal band keeps bits, which a fix that replaced the
    // refusal with a flat zero would get wrong.
    let smallest_normal = (1u128 << 64) | (1 << 63);
    let by_four = (u128::from(BIAS_U32 + 2) << 64) | (1 << 63);
    let got = fp::div(smallest_normal, by_four).expect("a subnormal is a value");
    assert_eq!(
        (got >> 64) & 0x7fff,
        0,
        "subnormal: the exponent field is zero"
    );
    assert_ne!(got & u128::from(u64::MAX), 0, "and the significand is not");
    // The bound is a bound: a difference inside the range still divides.
    let mid = (u128::from(20_000u32) << 64) | (1 << 63);
    assert!(fp::div(mid, small).is_some());
    assert!(fp::div(mid, big).is_some());
}

/// **The indefinite's own bits, written out rather than referred to.**
///
/// Mutation found this one, and it is a shape worth naming: every other assertion about an invalid
/// operation says `Some(fp::INDEFINITE)`, which compares the implementation against *itself*.
/// Flipping the constant's sign bit changes both sides at once and the whole suite still passes —
/// and the C fixtures do not help, because `n != n` is true of every NaN whatever its sign.
///
/// So the number is spelled out here. It came from a running program, not a manual: x87 answers every
/// invalid operation with the "real indefinite", which is *negative*, and the sign is the part a
/// reader would least expect and a fix would most easily drop.
#[test]
fn the_indefinite_is_the_exact_pattern_x87_produces() {
    assert_eq!(
        fp::INDEFINITE,
        0x0000_ffff_c000_0000_0000_0000u128 | (1u128 << 79),
        "sign 1, exponent all ones, significand 0xC000000000000000 — and the sign is not decoration"
    );
    assert_eq!(
        fp::INDEFINITE >> 79,
        1,
        "negative, which is the surprising half"
    );
    assert_eq!(
        (fp::INDEFINITE >> 64) & 0x7fff,
        0x7fff,
        "the exponent is all ones"
    );
    assert_eq!(
        fp::INDEFINITE & u128::from(u64::MAX),
        0xC000_0000_0000_0000,
        "the integer bit and the quiet bit, and no payload"
    );
    assert!(fp::is_nan(fp::INDEFINITE) && !fp::is_inf(fp::INDEFINITE));
}

/// **What a NaN looks like after narrowing to `float`, in bits.**
///
/// Every C fixture for this asks `f != f`, which is true of *any* NaN — so all three mutants against
/// the payload survived a suite that looked thorough. This is the wave 243 lesson again, one format
/// down: a good test of NaN-ness is no test of which NaN.
///
/// The rule was established by soak rather than by inspection, and the first attempt had it
/// backwards. Three hand-picked NaNs said the payload disappears; all three carried their bits below
/// the twenty-three that survive, so truncation and a canonical answer looked identical. 480,000
/// cases against the target disagreed on one in ten.
#[test]
fn a_nan_narrowed_to_float_keeps_what_fits_and_is_quieted() {
    // The twenty-three bits under the integer bit become the fraction.
    let wide = (0x7fffu128 << 64) | 0xff8c_9f37_9151_afbb;
    assert_eq!(
        fp::to_f32(wide),
        0x7fff_8c9f,
        "the payload is kept, not replaced with a canonical NaN"
    );
    assert_eq!(
        fp::to_f32(wide | (1 << 79)),
        0xffff_8c9f,
        "and the sign comes with it"
    );
    // **Quieting is load-bearing, not cosmetic.** A payload living entirely below bit 40 truncates
    // to nothing, and a `float` with an all-ones exponent and a zero fraction is an *infinity*. So
    // the quiet bit is forced, and these two would become infinities without it.
    let low = (0x7fffu128 << 64) | 0x8000_0000_0000_0001;
    assert_eq!(
        fp::to_f32(low),
        0x7fc0_0000,
        "a NaN must not narrow into an infinity"
    );
    assert_ne!(fp::to_f32(low), 0x7f80_0000);
    let below = (0x7fffu128 << 64) | 0x8000_0000_0100_0000;
    assert_eq!(fp::to_f32(below), 0x7fc0_0000);
    // A signalling NaN comes out quiet, with the bits that survive intact.
    let sig_nan = (0x7fffu128 << 64) | 0x8000_0100_0000_0000;
    assert_eq!(fp::to_f32(sig_nan), 0x7fc0_0001);
    // The control: an infinity narrows to an infinity, and is not quietly turned into a NaN.
    assert_eq!(fp::to_f32((0x7fffu128 << 64) | (1 << 63)), 0x7f80_0000);
    assert_eq!(
        fp::to_f32((1u128 << 79) | (0x7fffu128 << 64) | (1 << 63)),
        0xff80_0000
    );
}
