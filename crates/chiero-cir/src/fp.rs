//! x87's 80-bit extended format: the one encoding this project has to build by hand.
//!
//! **Why it lives here.** The format has no Rust primitive, so every layer that touches a
//! `long double` needs the same conversion — lowering encodes a literal, the engine decodes one to
//! an integer, and `SiToFp` needs the encoder in the engine. Those are three call sites in two
//! crates, and 001 §4 forbids `chiero-exec` a dependency on `chiero-sema` or `chiero-lower`, so
//! there is no import that would let them share. Before wave 233 the halves were already split —
//! an encoder in `chiero-lower`, a decoder in `chiero-exec` — which is a third copy waiting to
//! happen.
//!
//! `chiero-cir` is the only crate below all of them, and `FloatKind::X87_80` is defined here, so
//! what the kind *means* belongs beside it.
//!
//! # The format
//!
//! Eighty bits: a sign, a fifteen-bit exponent biased by 16383, and a **sixty-four**-bit
//! significand whose top bit is the integer bit and is stored *explicitly* — unlike IEEE binary32
//! and binary64, which leave it implied. That explicit bit is the source of every mistake in this
//! area: `f64`'s 1.0 is `0x3ff0000000000000` and x87's is `0x3fff8000000000000000`, and the second
//! is not the first shifted.

/// The exponent field's bias.
const BIAS: i32 = 16383;
/// The all-ones exponent, which means infinity or NaN.
const INF_EXP: u128 = 0x7fff;

/// An `f64` re-encoded exactly. Every `f64` is representable, since x87 is wider in both fields.
///
/// The special cases are the reason this is not three lines. A zero has a zero exponent *and* a zero
/// significand in both formats, and rebiasing it would produce a denormal with the integer bit set,
/// which x87 calls invalid. An infinity or NaN has the all-ones exponent, which rebiasing would turn
/// into an ordinary large number, so the exponent is mapped rather than shifted and a NaN's payload
/// is carried across.
pub fn from_f64(v: f64) -> u128 {
    let b = v.to_bits();
    let sign = u128::from(b >> 63) << 79;
    let exp = ((b >> 52) & 0x7ff) as u32;
    let frac = u128::from(b & 0x000f_ffff_ffff_ffff);
    if exp == 0 {
        // Zero, or an `f64` denormal. A denormal is representable as a *normal* x87 number, and
        // nothing in this project produces one from a C literal — so the encoding here is the one
        // that is exactly right for zero and invents no normalization nobody asked for.
        return sign | (frac << 11);
    }
    if exp == 0x7ff {
        return sign | (INF_EXP << 64) | (1u128 << 63) | (frac << 11);
    }
    let x87_exp = u128::from(exp) + u128::try_from(BIAS - 1023).expect("positive");
    sign | (x87_exp << 64) | (1u128 << 63) | (frac << 11)
}

/// An integer's exact encoding, up to sixty-four significant bits.
///
/// No rounding decision arises: an integer *is* its own significand once shifted so the integer bit
/// is set, and the exponent is however far it moved. That is why the integral paths in the front end
/// and in `SiToFp` can be exact where a general decimal literal cannot.
pub fn from_u64(v: u64, negative: bool) -> u128 {
    let sign = if negative { 1u128 << 79 } else { 0 };
    if v == 0 {
        // All-zero in x87 too, and the shift below has no meaning for it.
        return sign;
    }
    let shift = v.leading_zeros();
    let sig = u128::from(v) << shift;
    let unbiased = 63 - i32::try_from(shift).expect("at most 63");
    let exp = u128::try_from(unbiased + BIAS).expect("in range for any u64");
    sign | (exp << 64) | sig
}

/// A pattern truncated toward zero, exactly, without building an `f64`.
///
/// Decoding to `f64` first would round sixty-four bits of significand into fifty-three, which is
/// wrong for a sixty-four-bit integer target — and it is the shortcut that makes every conversion
/// work at once, so it is worth saying why it is not taken. The integer is in the significand
/// already: the value is `sig × 2^(e - 63)`, so truncating toward zero is a shift.
///
/// `None` when the result does not fit in `i128`, or for infinity and NaN, which have no integer
/// value at all (C11 6.3.1.4) — the caller's cue to report the event rather than invent a number.
pub fn trunc_to_int(bits: u128) -> Option<i128> {
    let neg = bits >> 79 & 1 == 1;
    let exp = ((bits >> 64) & INF_EXP) as i32;
    let sig = bits & u128::from(u64::MAX);
    if exp == 0 {
        return Some(0);
    }
    if exp == INF_EXP as i32 {
        return None;
    }
    let e = exp - BIAS;
    // Below 1.0 in magnitude: the integer part is zero, and `-0.5` is `0` rather than `-1`, because
    // C truncates toward zero rather than flooring.
    if e < 0 {
        return Some(0);
    }
    let mag: u128 = if e <= 63 {
        sig >> (63 - e)
    } else if e <= 126 {
        sig.checked_shl(u32::try_from(e - 63).ok()?)?
    } else {
        return None;
    };
    let m = i128::try_from(mag).ok()?;
    Some(if neg { -m } else { m })
}

/// Whether a pattern is outside what `width` bits of integer can hold, decided on the exact value.
///
/// The `f64` route cannot serve here for the same reason `trunc_to_int` exists: the comparison would
/// be made on a rounded number.
pub fn out_of_int_range(bits: u128, width: u32, signed: bool) -> bool {
    if ((bits >> 64) & INF_EXP) as i32 == INF_EXP as i32 {
        return true;
    }
    let Some(t) = trunc_to_int(bits) else {
        return true;
    };
    if signed {
        let hi = 1i128 << (width - 1);
        t >= hi || t < -hi
    } else {
        t >= 1i128 << width || t < 0
    }
}

/// A pattern narrowed to `f64`, rounded to nearest with ties to even.
///
/// **The rounding is IEEE's, obtained rather than hand-rolled.** `sig as f64` rounds a
/// sixty-four-bit integer to `f64`'s fifty-three bits under the hardware's default rule, which *is*
/// round-to-nearest-ties-to-even — and scaling by a power of two afterwards is exact, so no second
/// rounding happens. Writing the eleven-bit decision by hand would be a reimplementation of
/// something the target already does correctly.
///
/// `None` where the honest answer is a declared gap rather than a number:
///
///   - **a result in `f64`'s subnormal range.** Scaling into it rounds a *second* time, so the
///     answer could be one ULP off — a wrong number where this project reports gaps. Handling it
///     means manual denormal shifting, and pretending otherwise in a comment is the mistake
///     `float_literal` still carries (§9).
///   - **NaN**, whose payload this does not attempt to map.
///
/// Infinity and overflow are *not* gaps: a magnitude past `f64::MAX` becomes infinity, which is what
/// IEEE-754 §7.4 requires and what the multiplication below produces on its own.
pub fn to_f64(bits: u128) -> Option<f64> {
    let neg = bits >> 79 & 1 == 1;
    let exp = ((bits >> 64) & INF_EXP) as i32;
    let sig = (bits & u128::from(u64::MAX)) as u64;
    if exp == 0 {
        // Zero, and x87 denormals — which are far below `f64`'s smallest subnormal, so they
        // underflow to zero rather than needing a decision.
        return Some(if neg { -0.0 } else { 0.0 });
    }
    if exp == INF_EXP as i32 {
        // The integer bit alone is infinity; anything else in the significand is a NaN.
        if sig == 1u64 << 63 {
            return Some(if neg {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        return None;
    }
    let scaled = (sig as f64) * 2f64.powi(exp - BIAS - 63);
    // Subnormal or underflowed-to-zero from a nonzero value: the scaling rounded a second time.
    if scaled != 0.0 && scaled.abs() < f64::MIN_POSITIVE {
        return None;
    }
    if scaled == 0.0 && sig != 0 {
        return None;
    }
    Some(if neg { -scaled } else { scaled })
}
