//! **A widening conversion turns a constant into a non-constant, and everything downstream
//! of that stops being decidable by arithmetic.**
//!
//! `TermArena` folds constants everywhere else it can. `bin` folds when both operands are
//! constant and applies identity and annihilator laws besides; `not` folds; `extract` folds
//! and even recognises that a whole-width extract is the value itself, because a store
//! followed by a load must rebuild a term that still *is* the original. `sext` and `zext`
//! are the only members of that family that intern a node unconditionally.
//!
//! The consequence is not a missed simplification. `as_const` is how the rest of the system
//! asks "do I know this value?", and a `Node::Extend` answers no however constant its
//! operand is. So every C expression mixing widths — `long * int`, `long + 1`, anything
//! that goes through the usual arithmetic conversions — presents the engine with an operand
//! it believes is unknown.
//!
//! That is what the UB census's first row was measuring. `note_ub` needs both operands
//! concrete and gives up otherwise, so:
//!
//! ```text
//!   long acc = 804574689342403103L; acc = acc * 31L;   ->  SignedOverflow
//!   long acc = 804574689342403103L; acc = acc * 31;    ->  nothing
//! ```
//!
//! The two programs differ only in a literal's suffix. The second is the one C programmers
//! actually write, and it is the shape of every `acc = acc * 31 + x` in the generated
//! corpus — 18 signed overflows gcc reports and chiero did not.

use chiero_solver::TermArena;

/// Sign extension of a constant is a constant, with the sign carried into the new bits.
#[test]
fn sext_of_a_constant_folds() {
    let mut a = TermArena::new();
    for (from, to, bits, want) in [
        // -1 at 8 bits is 0xFF; at 32 bits it must be 0xFFFFFFFF, not 0x000000FF.
        (8u32, 32u32, 0xFFu128, -1i128),
        (8, 32, 0x7F, 127),
        (8, 32, 0x80, -128),
        (32, 64, 31, 31),
        (32, 64, 0xFFFF_FFFF, -1),
        // A same-width "extension" is the identity, and must still be a constant.
        (32, 32, 7, 7),
    ] {
        let k = a.bv(from, bits);
        let e = a.sext(k, to);
        let got = a.as_const(e);
        assert!(
            got.is_some(),
            "sext of a constant is a constant: {bits:#x} from {from} to {to}"
        );
        let got = got.unwrap();
        assert_eq!(got.width(), to, "at the target width");
        assert_eq!(
            got.signed(),
            want,
            "sext {bits:#x} from {from} to {to} bits is {want}"
        );
    }
}

/// Zero extension likewise, and it must *not* carry the sign.
#[test]
fn zext_of_a_constant_folds() {
    let mut a = TermArena::new();
    for (from, to, bits, want) in [
        (8u32, 32u32, 0xFFu128, 0xFFu128),
        (8, 32, 0x80, 0x80),
        (32, 64, 0xFFFF_FFFF, 0xFFFF_FFFF),
        (32, 32, 7, 7),
    ] {
        let k = a.bv(from, bits);
        let e = a.zext(k, to);
        let got = a.as_const(e);
        assert!(got.is_some(), "zext of a constant is a constant");
        let got = got.unwrap();
        assert_eq!(got.width(), to, "at the target width");
        assert_eq!(got.bits(), want, "zext {bits:#x} from {from} to {to}");
    }
}

/// The two must disagree exactly where C says they do: on a value whose top bit is set.
///
/// Asserted separately because a "fold" that returned the operand's bits unchanged would
/// pass both tests above for every non-negative input, and `zext` would look correct for
/// all of them.
#[test]
fn the_two_extensions_disagree_on_a_negative_value() {
    let mut a = TermArena::new();
    let k = a.bv(8, 0x80);
    let s = a.sext(k, 32);
    let z = a.zext(k, 32);
    assert_eq!(
        a.as_const(s).unwrap().bits(),
        0xFFFF_FF80,
        "sext fills ones"
    );
    assert_eq!(
        a.as_const(z).unwrap().bits(),
        0x0000_0080,
        "zext fills zeros"
    );
    assert_ne!(s, z, "and they are not the same term");
}

/// The point of folding: arithmetic *through* a widening stays decidable.
///
/// This is the shape the engine actually meets — one operand widened by the usual
/// arithmetic conversions, the other already at the wider type — and the assertion is that
/// the product is a constant the caller can read, not merely that the extension was.
#[test]
fn arithmetic_over_an_extended_constant_is_still_constant() {
    let mut a = TermArena::new();
    let acc = a.bv(64, 804_574_689_342_403_103);
    let thirty_one = a.bv(32, 31);
    let widened = a.sext(thirty_one, 64);
    let product = a.mul(acc, widened);
    let got = a.as_const(product);
    assert!(
        got.is_some(),
        "`acc * 31` is as constant as `acc * 31L`, and the engine must be able to say so"
    );
    // 804574689342403103 * 31 wraps at 64 bits; the folded value is the wrapped one, and
    // recognising that it wrapped is `note_ub`'s job rather than the arena's.
    assert_eq!(
        got.unwrap().bits(),
        804_574_689_342_403_103u128
            .wrapping_mul(31)
            .wrapping_rem(1u128 << 64),
        "and folds to the wrapped 64-bit product"
    );
}
