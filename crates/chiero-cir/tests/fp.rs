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
