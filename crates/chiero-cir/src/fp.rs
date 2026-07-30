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

/// `mant × 2^scale`, exactly, for a mantissa of up to sixty-four bits.
///
/// The companion to [`from_u64`], which is this with `scale` at zero. A hexadecimal float literal is
/// exactly this shape — its digits are binary and its `p` exponent is a power of two — so it reaches
/// x87 without a rounding decision, which is what C99 6.4.4.2 has the syntax for.
///
/// `None` when the exponent leaves x87's range: past the top is an overflow the caller should report
/// rather than silently turn into an infinity, and below the bottom is a subnormal this does not
/// encode.
pub fn from_u64_scaled(mant: u64, scale: i32, negative: bool) -> Option<u128> {
    let base = from_u64(mant, negative);
    if mant == 0 {
        return Some(base);
    }
    // `from_u64` placed the exponent for `mant × 2^0`; shifting the value by `2^scale` moves the
    // exponent field by exactly `scale`, because the significand is unchanged.
    let exp = i64::try_from((base >> 64) & INF_EXP).ok()? + i64::from(scale);
    if exp <= 0 || exp >= i64::from(INF_EXP as u32) {
        return None;
    }
    let sign = base & (1u128 << 79);
    let sig = base & u128::from(u64::MAX);
    Some(sign | (u128::try_from(exp).ok()? << 64) | sig)
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

/// Two patterns ordered, or `None` when either is a NaN.
///
/// **No arithmetic and no narrowing.** Narrowing to `f64` to compare would call `1 + 2^-53` equal to
/// `1.0`, because both round to the same `f64` — so the comparison has to happen on the patterns, and
/// on the patterns it is nearly free: x87's fields are laid out most-significant-first, so for a
/// fixed sign the exponent-and-significand bits compare as one unsigned integer.
///
/// `None` is *unordered*, which is not the same as "no answer": IEEE-754 §5.11 gives every ordered
/// comparison with a NaN the answer `false` and `!=` the answer `true`, so the caller has to
/// distinguish the two rather than treat this as a gap.
pub fn partial_cmp(x: u128, y: u128) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering;
    if is_nan(x) || is_nan(y) {
        return None;
    }
    let (xn, yn) = (x >> 79 & 1 == 1, y >> 79 & 1 == 1);
    // **Both zeros are equal**, whatever their signs — IEEE-754 §5.11 again, and the only place the
    // sign bit does not decide the ordering.
    if is_zero(x) && is_zero(y) {
        return Some(Ordering::Equal);
    }
    if xn != yn {
        return Some(if xn {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    // Magnitude: everything below the sign bit, compared as one unsigned number. That works because
    // a larger exponent outranks any significand, which is what putting the exponent above the
    // significand in the layout means.
    let mag = |v: u128| v & ((1u128 << 79) - 1);
    let ord = mag(x).cmp(&mag(y));
    Some(if xn { ord.reverse() } else { ord })
}

/// Whether a pattern is a NaN: the all-ones exponent with anything but the bare integer bit.
pub fn is_nan(bits: u128) -> bool {
    (bits >> 64) & INF_EXP == INF_EXP && (bits & u128::from(u64::MAX)) != 1u128 << 63
}

/// Whether a pattern is a zero of either sign.
pub fn is_zero(bits: u128) -> bool {
    (bits >> 64) & INF_EXP == 0 && bits & u128::from(u64::MAX) == 0
}

/// The product of two patterns, rounded to nearest with ties to even.
///
/// **Exact before it rounds, which is what makes multiplication the operation to do first.** Two
/// sixty-four-bit significands multiply into a hundred and twenty-eight bits — a `u128` holds the
/// whole product with nothing lost — so the only decision is how to put it back into sixty-four, and
/// that decision is made once, on the exact value. Addition would need the operands aligned first and
/// division would need a loop; this needs neither.
///
/// `None` where an answer would be a guess rather than a value:
///
///   - **a subnormal or underflowed result**, which needs denormal shifting this does not do
///   - **a NaN operand**, whose payload is not propagated
///   - **zero times infinity**, which IEEE-754 §7.2 makes an invalid operation producing a NaN
///
/// Overflow is *not* a gap: past the top exponent the answer is an infinity, which §7.4 requires.
pub fn mul(x: u128, y: u128) -> Option<u128> {
    let neg = (x >> 79 & 1) ^ (y >> 79 & 1) == 1;
    let sign = u128::from(neg) << 79;
    if is_nan(x) || is_nan(y) {
        return None;
    }
    let (inf_x, inf_y) = (is_inf(x), is_inf(y));
    let (zero_x, zero_y) = (is_zero(x), is_zero(y));
    if (inf_x && zero_y) || (inf_y && zero_x) {
        // 0 × ∞ is the invalid operation, and its result is a NaN this does not mint.
        return None;
    }
    if inf_x || inf_y {
        return Some(sign | (INF_EXP << 64) | (1u128 << 63));
    }
    if zero_x || zero_y {
        return Some(sign);
    }
    let (ex, ey) = (((x >> 64) & INF_EXP) as i64, ((y >> 64) & INF_EXP) as i64);
    // An x87 denormal is far below anything this can normalize back, so it is a gap rather than a
    // zero — a zero would be a confident answer about a value that is merely very small.
    if ex == 0 || ey == 0 {
        return None;
    }
    let (sx, sy) = (x & u128::from(u64::MAX), y & u128::from(u64::MAX));
    let prod = sx * sy;
    // Both operands have the integer bit set, so the product's bit 127 or 126 is — one normalization
    // step at most, and which one it is decides the exponent.
    let (sig, extra, exp_adj) = if prod >> 127 & 1 == 1 {
        (prod >> 64, prod & u128::from(u64::MAX), 1i64)
    } else {
        (
            (prod >> 63) & u128::from(u64::MAX),
            (prod << 1) & u128::from(u64::MAX),
            0,
        )
    };
    // Round to nearest, ties to even, on the sixty-four bits being discarded. The tie is exactly
    // half, and it goes to the candidate whose low bit is already zero.
    let half = 1u128 << 63;
    let mut sig = sig;
    let mut exp = ex + ey - i64::from(BIAS) + exp_adj;
    if extra > half || (extra == half && sig & 1 == 1) {
        sig += 1;
        // Rounding up can carry out of the significand, which is another normalization step: the
        // value becomes a power of two and the integer bit moves.
        if sig >> 64 & 1 == 1 {
            sig >>= 1;
            exp += 1;
        }
    }
    if exp >= i64::from(INF_EXP as u32) {
        return Some(sign | (INF_EXP << 64) | (1u128 << 63));
    }
    if exp <= 0 {
        return None;
    }
    Some(sign | (u128::try_from(exp).ok()? << 64) | sig)
}

/// Whether a pattern is an infinity: the all-ones exponent with the bare integer bit.
pub fn is_inf(bits: u128) -> bool {
    (bits >> 64) & INF_EXP == INF_EXP && (bits & u128::from(u64::MAX)) == 1u128 << 63
}

/// The sum of two patterns, rounded to nearest with ties to even.
///
/// **Where [`mul`] was exact and then rounded once, this is exact only after the operands are made
/// comparable** — and making them comparable is the whole difficulty. Three things follow from that
/// and none of them arise in multiplication:
///
///   - **Alignment.** The smaller operand shifts right by the exponent difference, which for `f80`
///     can be thirty-two thousand places. The bits it shifts out are gone from the arithmetic but not
///     from the *rounding*, so they are folded into a sticky flag.
///   - **Cancellation.** Near-equal operands of opposite sign leave a result with leading zeros, and
///     renormalizing means shifting left by up to sixty-three places rather than the one bit a
///     product ever needs.
///   - **Sign.** Which operand is subtracted from which depends on magnitude, not argument order, and
///     the result's sign is the larger operand's.
///
/// # Why the sticky flag survives the left shift
///
/// The two hard parts interact exactly once, and the interaction is provably harmless. Alignment
/// loses no bits at all while the exponent difference is sixty-three or less, because the operand is
/// staged with sixty-three zeros beneath it — so **a sticky flag requires a difference of at least
/// sixty-four**. At that difference the subtracted operand is under `2^63` while the larger is at
/// least `2^126`, so the result cannot fall more than one bit short of normalized, and a one-bit left
/// shift moves the residue into the sticky region rather than into the rounding bit. Large
/// cancellation and a nonzero sticky flag cannot happen together.
///
/// `None` where an answer would be a guess: a NaN or denormal operand, `∞ + -∞` (IEEE-754 §7.2's
/// invalid operation), and a subnormal result — the same three [`mul`] declines and for the same
/// reasons.
pub fn add(x: u128, y: u128) -> Option<u128> {
    if is_nan(x) || is_nan(y) {
        return None;
    }
    let (nx, ny) = (x >> 79 & 1 == 1, y >> 79 & 1 == 1);
    let (ix, iy) = (is_inf(x), is_inf(y));
    if ix && iy {
        // Same sign is that infinity; opposite signs are §7.2's invalid operation, whose result is
        // a NaN this does not mint.
        return if nx == ny { Some(x) } else { None };
    }
    if ix {
        return Some(x);
    }
    if iy {
        return Some(y);
    }
    if is_zero(x) && is_zero(y) {
        // §6.3: under round-to-nearest the sum of two zeros is `-0` only when both are, because
        // `+0 + -0` is defined to be `+0` rather than left to the operands.
        return Some(if nx && ny { 1u128 << 79 } else { 0 });
    }
    if is_zero(x) {
        return Some(y);
    }
    if is_zero(y) {
        return Some(x);
    }
    let (ex, ey) = (((x >> 64) & INF_EXP) as i64, ((y >> 64) & INF_EXP) as i64);
    if ex == 0 || ey == 0 {
        return None;
    }
    let sx = (x & u128::from(u64::MAX)) as u64;
    let sy = (y & u128::from(u64::MAX)) as u64;
    // The larger magnitude leads, and it decides the result's sign. Comparing the exponent before
    // the significand is the whole comparison, because both significands are normalized.
    let ((ea, sa, na), (eb, sb, nb)) = if (ex, sx) >= (ey, sy) {
        ((ex, sx, nx), (ey, sy, ny))
    } else {
        ((ey, sy, ny), (ex, sx, nx))
    };
    // Staged sixty-three bits up, which buys room for a carry above and for the alignment shift to
    // stay lossless below.
    let a_al = u128::from(sa) << 63;
    let b_full = u128::from(sb) << 63;
    let diff = ea - eb;
    let (b_al, sticky) = if diff >= 128 {
        (0u128, true)
    } else {
        let d = u32::try_from(diff).ok()?;
        (b_full >> d, d > 0 && b_full & ((1u128 << d) - 1) != 0)
    };
    let mut exp = ea;
    let mut r = if na == nb {
        a_al + b_al
    } else {
        let d = a_al - b_al;
        if d == 0 && !sticky {
            // §6.3 again: `x - x` is `+0` under round-to-nearest whatever the operands' signs were.
            return Some(0);
        }
        // The discarded bits make the subtrahend larger than what was aligned, so the difference is
        // one unit lower and the residue that remains is still nonzero.
        if sticky { d - 1 } else { d }
    };
    if r == 0 {
        return Some(0);
    }
    // Renormalize so the significand's integer bit lands at 126, which is where `>> 63` finds it.
    let hb = 127 - i64::from(r.leading_zeros());
    if hb > 126 {
        // **No sticky is collected here, and mutation is why the line that collected one is gone.**
        // Dropping it changed no answer, so the case was enumerated exhaustively at narrower
        // significand widths — every significand pair, every exponent difference, both sign
        // combinations — and it changes nothing there either. The reason: a sum reaches bit 127 only
        // when the exponents differ by sixty-three or less, and at those differences the aligned
        // operand still has a zero in bit 0, so there is never a one to shift out. A line that cannot
        // fire tells the next reader this sticky source matters here, and it does not.
        r >>= 1;
        exp += 1;
    } else if hb < 126 {
        let k = 126 - hb;
        r <<= u32::try_from(k).ok()?;
        exp -= k;
    }
    let mut sig = (r >> 63) as u64;
    let guard = (r >> 62) & 1 == 1;
    let sticky = sticky || r & ((1u128 << 62) - 1) != 0;
    if guard && (sticky || sig & 1 == 1) {
        let (next, carried) = sig.overflowing_add(1);
        sig = if carried {
            exp += 1;
            1u64 << 63
        } else {
            next
        };
    }
    let sign = u128::from(na) << 79;
    if exp >= i64::from(INF_EXP as u32) {
        return Some(sign | (INF_EXP << 64) | (1u128 << 63));
    }
    if exp <= 0 {
        return None;
    }
    Some(sign | (u128::try_from(exp).ok()? << 64) | u128::from(sig))
}

/// The difference of two patterns: [`add`] with the subtrahend's sign flipped.
///
/// **Not a shortcut — it is the definition.** IEEE-754 specifies subtraction as addition of the
/// negation, and every case that makes subtraction interesting (cancellation, the sign of an exact
/// zero, `∞ - ∞`) is already a case `add` has to get right for mixed-sign operands. Writing a second
/// routine would duplicate the alignment and normalization to no purpose and give the two operations
/// separate bugs.
pub fn sub(x: u128, y: u128) -> Option<u128> {
    add(x, y ^ (1u128 << 79))
}

/// A minimal unsigned big integer, base 2^32, little-endian — only what decimal conversion needs.
///
/// **Not a general facility, and deliberately so.** `from_decimal` needs exactly five things: build
/// from digits, scale by a power of ten, shift, compare, subtract. Everything else a big-integer type
/// usually grows is absent, because the alternative to writing this was rounding decimal literals
/// through `f64` and there is no third option: a correctly-rounded decimal-to-binary conversion at
/// sixty-four significand bits cannot be done in fixed-width arithmetic, since the exact value of
/// `1e4000` is four thousand digits long and the rounding decision depends on all of them.
#[derive(Clone, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn zero() -> Self {
        Big(Vec::new())
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    /// The position one past the highest set bit, or zero.
    fn bit_len(&self) -> usize {
        for (i, &w) in self.0.iter().enumerate().rev() {
            if w != 0 {
                return i * 32 + (32 - w.leading_zeros() as usize);
            }
        }
        0
    }

    fn bit(&self, i: usize) -> u32 {
        self.0.get(i / 32).map_or(0, |w| (w >> (i % 32)) & 1)
    }

    /// `self = self * m + a`, the primitive that builds a value from digits and scales it by ten.
    fn mul_add_small(&mut self, m: u32, a: u32) {
        let mut carry = u64::from(a);
        for w in &mut self.0 {
            let v = u64::from(*w) * u64::from(m) + carry;
            *w = v as u32;
            carry = v >> 32;
        }
        while carry != 0 {
            self.0.push(carry as u32);
            carry >>= 32;
        }
    }

    fn shl(&self, n: usize) -> Self {
        let (words, bits) = (n / 32, n % 32);
        let mut out = vec![0u32; words];
        let mut carry = 0u32;
        for &w in &self.0 {
            out.push((w << bits) | carry);
            // A shift of 32 is undefined in Rust as well as in C, and `bits` is zero often enough
            // that this branch is the common case rather than the corner.
            carry = if bits == 0 { 0 } else { w >> (32 - bits) };
        }
        if carry != 0 {
            out.push(carry);
        }
        let mut b = Big(out);
        b.trim();
        b
    }

    fn shr(&self, n: usize) -> Self {
        let (words, bits) = (n / 32, n % 32);
        if words >= self.0.len() {
            return Big::zero();
        }
        let mut out = Vec::with_capacity(self.0.len() - words);
        for i in words..self.0.len() {
            let lo = self.0[i] >> bits;
            let hi = if bits == 0 {
                0
            } else {
                self.0.get(i + 1).map_or(0, |&w| w << (32 - bits))
            };
            out.push(lo | hi);
        }
        let mut b = Big(out);
        b.trim();
        b
    }

    fn cmp(&self, o: &Self) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        let n = self.0.len().max(o.0.len());
        for i in (0..n).rev() {
            let (a, b) = (
                self.0.get(i).copied().unwrap_or(0),
                o.0.get(i).copied().unwrap_or(0),
            );
            if a != b {
                return if a > b {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
        }
        Ordering::Equal
    }

    /// `self -= o`, which every caller has already established is not negative.
    fn sub_assign(&mut self, o: &Self) {
        let mut borrow = 0i64;
        for i in 0..self.0.len() {
            let v = i64::from(self.0[i]) - i64::from(o.0.get(i).copied().unwrap_or(0)) - borrow;
            if v < 0 {
                self.0[i] = (v + (1i64 << 32)) as u32;
                borrow = 1;
            } else {
                self.0[i] = v as u32;
                borrow = 0;
            }
        }
        self.trim();
    }

    /// `self = self * 2 + b`, the division loop's inner step.
    fn shl1_add(&mut self, b: u32) {
        let mut carry = b;
        for w in &mut self.0 {
            let v = (u64::from(*w) << 1) | u64::from(carry);
            *w = v as u32;
            carry = (v >> 32) as u32;
        }
        if carry != 0 {
            self.0.push(carry);
        }
    }

    fn from_digits(digits: &str) -> Option<Self> {
        let mut b = Big::zero();
        // Nine digits at a time, because 10^9 is the largest power of ten a `u32` multiplier holds.
        for chunk in digits.as_bytes().chunks(9) {
            let mut m = 1u32;
            let mut a = 0u32;
            for &c in chunk {
                if !c.is_ascii_digit() {
                    return None;
                }
                m *= 10;
                a = a * 10 + u32::from(c - b'0');
            }
            b.mul_add_small(m, a);
        }
        b.trim();
        Some(b)
    }

    fn pow10(k: u32) -> Self {
        let mut b = Big(vec![1]);
        let mut left = k;
        while left > 0 {
            let step = left.min(9);
            b.mul_add_small(10u32.pow(step), 0);
            left -= step;
        }
        b
    }
}

/// The widest decimal magnitude worth computing: x87 reaches about `1.19e4932` at the top and
/// `3.36e-4932` at the bottom, so anything an order of magnitude past either end is settled without
/// building a five-thousand-digit integer to confirm it.
const DECIMAL_LIMIT: i64 = 4940;

/// **`digits × 10^exp10`, correctly rounded to x87's sixty-four bits.**
///
/// The conversion `str::parse::<f64>` cannot do, because it answers in fifty-three bits and `f80` has
/// sixty-four — so `0.1L` parsed that way lands *above* the true tenth rather than at the nearest
/// `f80` to it, and every value past `f64`'s range becomes an infinity.
///
/// **Exact, then rounded once**, which is the same shape as [`mul`]. The value is a ratio of two
/// integers — `digits × 10^exp10 / 1` when the exponent is positive and `digits / 10^-exp10` when it
/// is negative — and both are computed with nothing lost. The quotient's top sixty-five bits and a
/// sticky flag for everything below them are all the rounding decision needs, and the division stops
/// as soon as it has them.
///
/// `None` where an answer would be a guess: a subnormal result, for the reason [`mul`] gives.
/// Overflow is an infinity, for the reason [`mul`] gives.
pub fn from_decimal(digits: &str, exp10: i32, negative: bool) -> Option<u128> {
    let sign = u128::from(negative) << 79;
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return Some(sign);
    }
    // The decimal magnitude, near enough to settle the two extremes without arithmetic. Every digit
    // is at most one order of magnitude, so `len + exp10` brackets the exponent within one.
    let mag = i64::from(exp10) + i64::try_from(trimmed.len()).ok()?;
    if mag > DECIMAL_LIMIT {
        return Some(sign | (INF_EXP << 64) | (1u128 << 63));
    }
    if mag < -DECIMAL_LIMIT {
        return None;
    }
    let n = Big::from_digits(trimmed)?;
    let (num, den) = if exp10 >= 0 {
        let mut num = n;
        let p = Big::pow10(u32::try_from(exp10).ok()?);
        num = mul_big(&num, &p);
        (num, Big(vec![1]))
    } else {
        (n, Big::pow10(u32::try_from(-i64::from(exp10)).ok()?))
    };
    let (ln, ld) = (num.bit_len(), den.bit_len());
    // Line the operands up so the quotient is sixty-five or sixty-six bits — sixty-four of
    // significand and one to round on — rather than the thousands it would otherwise have.
    let shift = 65i64 - (i64::try_from(ln).ok()? - i64::try_from(ld).ok()?);
    let (a, b) = if shift >= 0 {
        (num.shl(usize::try_from(shift).ok()?), den)
    } else {
        (num, den.shl(usize::try_from(-shift).ok()?))
    };
    let (la, lb) = (a.bit_len(), b.bit_len());
    if la < lb {
        return None;
    }
    // **Binary long division, started at the first bit that can produce a quotient digit.** The
    // leading `lb - 1` steps of a schoolbook division are known to produce zeros — a remainder of
    // fewer bits than the divisor cannot exceed it — so the loop skips straight past them and runs
    // sixty-six times instead of thousands.
    let skip = lb.checked_sub(1)?;
    let mut rem = a.shr(la - skip);
    let mut q: u128 = 0;
    for i in (0..la - skip).rev() {
        rem.shl1_add(a.bit(i));
        q <<= 1;
        if rem.cmp(&b) != core::cmp::Ordering::Less {
            rem.sub_assign(&b);
            q |= 1;
        }
    }
    if q == 0 {
        return None;
    }
    let sticky_rem = !rem.is_zero();
    // Cut the quotient down to sixty-four bits, keeping the bit below them to round on and folding
    // everything under that — including the division's own remainder — into one sticky flag.
    let drop = (128 - q.leading_zeros()).checked_sub(64)?;
    let mut sig = (q >> drop) as u64;
    let guard = drop > 0 && (q >> (drop - 1)) & 1 == 1;
    let sticky = sticky_rem || (drop > 1 && q & ((1u128 << (drop - 1)) - 1) != 0);
    // `value = sig × 2^drop × 2^-shift`, and an f80 significand carries its integer bit at 63.
    let mut exp = 63i64 + i64::from(drop) - shift;
    if guard && (sticky || sig & 1 == 1) {
        let (next, carried) = sig.overflowing_add(1);
        // The same second normalization `mul` needs: all ones plus one is a new power of two.
        sig = if carried {
            exp += 1;
            1u64 << 63
        } else {
            next
        };
    }
    let biased = exp + i64::from(BIAS);
    if biased >= i64::from(INF_EXP as u32) {
        return Some(sign | (INF_EXP << 64) | (1u128 << 63));
    }
    if biased <= 0 {
        return None;
    }
    Some(sign | (u128::try_from(biased).ok()? << 64) | u128::from(sig))
}

/// Schoolbook multiplication, used once — to apply a positive power of ten to the digits.
fn mul_big(x: &Big, y: &Big) -> Big {
    let mut out = vec![0u32; x.0.len() + y.0.len() + 1];
    for (i, &a) in x.0.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &b) in y.0.iter().enumerate() {
            let v = u64::from(a) * u64::from(b) + u64::from(out[i + j]) + carry;
            out[i + j] = v as u32;
            carry = v >> 32;
        }
        let mut k = i + y.0.len();
        while carry != 0 {
            let v = u64::from(out[k]) + carry;
            out[k] = v as u32;
            carry = v >> 32;
            k += 1;
        }
    }
    let mut b = Big(out);
    b.trim();
    b
}
