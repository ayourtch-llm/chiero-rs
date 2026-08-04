//! **Translation phase 5 for string literals: spelling in, elements out.**
//!
//! One decoder, in the one crate both sides can see. Sema needs the element *count* to
//! size the array; lowering needs the element *values* to fill it. Those used to be two
//! independent readings of the same spelling — sema counted source bytes, lowering
//! re-scanned the escapes — and they were free to disagree. They did: sema sized
//! `"a\nb"` at five characters while C says four.
//!
//! Wave 150's rule is why this is a module and not a second copy: a fix that lands in one
//! of two copies is worse than no fix, because the suite goes green while the object and
//! its contents still describe different arrays. Here there is nothing to keep in step —
//! `string_elements` returns a list, sema takes its length and lowering takes its values.
//!
//! # What the distinction between the two units buys
//!
//! C draws a line that a `Vec<u8>` cannot express. `\xFF` is a *value*: one element holding
//! 255, in a plain literal and a wide one alike. `\u00FF` is a *character*: two bytes
//! (`C3 BF`) in a plain literal and one element holding 255 in a wide one. A decoder that
//! yielded bytes has already lost the difference by the time the width is known, which is
//! exactly how a byte-oriented `unescape` came to read `u"\uFFFF"` as the five letters
//! `u F F F F`.
//!
//! So the decode is width-independent and yields [`StrUnit`], and the *encode* — where the
//! width finally matters — is a second step.

/// One unit of a decoded string literal, before the width has been applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StrUnit {
    /// A value the source named **numerically** — `\xFF`, `\101`, `\n`. C11 6.4.4.4p9
    /// gives it the value written, in one element, whatever the literal's width. It is
    /// never re-encoded: `u"\xFF"[0]` is 255, not the two units of UTF-8 for U+00FF.
    Raw(u32),
    /// A **character**: written directly in the source, or as a `\u`/`\U` universal
    /// character name. C11 5.2.1.1 makes those two spellings the same thing by the end of
    /// phase 5, which is why one variant covers both. How it is stored is the width's
    /// business — UTF-8, UTF-16, or a single code point.
    Char(u32),
}

/// The `(signed, bits)` of a string literal's element type, from its prefix.
///
/// x86-64 Linux, which is the one target 014 models: `wchar_t` is a signed 32-bit `int`,
/// `char16_t` is 16-bit unsigned and `char32_t` 32-bit unsigned. `u8` is checked before `u`
/// because the shorter prefix would otherwise match it.
///
/// `char16_t`'s unsignedness is observable only above 32767, so nothing could pin it until
/// `u"\uFFFF"` existed to produce 65535 — a signed reading answers -1.
/// The literal's **prefix as written**, or `""` for a plain one.
///
/// Separate from [`string_element`] because C 6.4.5p5 asks about the prefix and not the element
/// type: `u8"a" "b"` concatenates and `L"a" u8"b"` does not, though `u8` and plain share an
/// element type. A rule phrased on the element would get exactly those two backwards.
pub fn string_prefix(spelling: &str) -> &'static str {
    if spelling.starts_with("u8") {
        "u8"
    } else if spelling.starts_with('L') {
        "L"
    } else if spelling.starts_with('u') {
        "u"
    } else if spelling.starts_with('U') {
        "U"
    } else {
        ""
    }
}

pub fn string_element(spelling: &str) -> (bool, u32) {
    if spelling.starts_with("u8") {
        (true, 8)
    } else if spelling.starts_with('L') {
        (true, 32)
    } else if spelling.starts_with('u') {
        (false, 16)
    } else if spelling.starts_with('U') {
        (false, 32)
    } else {
        // The plain literal's `char` follows the target's signedness, which the caller
        // supplies; 8 bits is the part this function knows.
        (true, 8)
    }
}

/// The content of a literal's spelling: everything between the first and last `"`.
pub fn unquote(spelling: &str) -> &str {
    match (spelling.find('"'), spelling.rfind('"')) {
        (Some(a), Some(b)) if b > a => &spelling[a + 1..b],
        _ => spelling,
    }
}

/// Decode one fragment's **content** (no quotes, no prefix) into width-independent units.
pub fn string_units(content: &str) -> Vec<StrUnit> {
    string_units_reporting(content, &mut Vec::new())
}

/// The same walk, also collecting what is **wrong** with the escapes it passed (C 6.4.4.4p1).
///
/// **One walk with two entry points, not two walks.** The escape grammar is already written here;
/// a separate inspector would be a second implementation of it, and wave 336 earned the rule that
/// says where the defect will then be — `float_literal` guessing at a suffix grammar
/// `number_defect` already knew. The information a defect needs is only available *during* the
/// decode: by the time `\q` has become `StrUnit::Char('q')` it is indistinguishable from a
/// literal `q`.
///
/// **Shape only.** Whether a value fits is not decidable here, because it depends on the element
/// width and this walk does not know the prefix — see [`escape_range_defect`].
/// Escapes `gcc -std=gnu11` accepts with **no diagnostic at all**, as against the ones it
/// warns about.
///
/// Measured, because the set is not guessable: `\%` is accepted so a format-string fragment
/// may be written literally. `\q` and `\8` warn, and every other unknown escape behaves like
/// them. `\e` is GNU's ESC and is handled in its own arm, since it also decodes to a value.
pub fn gnu_accepts_escape(c: char) -> bool {
    matches!(c, '%')
}

pub fn string_units_reporting(content: &str, bad: &mut Vec<String>) -> Vec<StrUnit> {
    string_units_split(content, bad, &mut Vec::new())
}

/// As [`string_units_reporting`], separating escapes gcc refuses only under
/// `-pedantic-errors` (`gnu_only`) from those it warns about in every mode (`bad`).
pub fn string_units_split(
    content: &str,
    bad: &mut Vec<String>,
    gnu_only: &mut Vec<String>,
) -> Vec<StrUnit> {
    let mut out = Vec::with_capacity(content.len());
    let mut it = content.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(StrUnit::Char(c as u32));
            continue;
        }
        let Some(e) = it.next() else {
            // A trailing backslash cannot occur in a well-formed literal — phase 2 has
            // already spliced line continuations — but dropping it would shorten the
            // object, and `sizeof` is a value the corpus compares against gcc.
            out.push(StrUnit::Raw(u32::from(b'\\')));
            break;
        };
        match e {
            'n' => out.push(StrUnit::Raw(10)),
            't' => out.push(StrUnit::Raw(9)),
            'r' => out.push(StrUnit::Raw(13)),
            'a' => out.push(StrUnit::Raw(7)),
            'b' => out.push(StrUnit::Raw(8)),
            'f' => out.push(StrUnit::Raw(12)),
            'v' => out.push(StrUnit::Raw(11)),
            // **A GNU extension: ESC.** `gnu11` takes it silently and `-pedantic-errors`
            // refuses it, so it is reported like `\%` — under the strict dialect only — while
            // still decoding to 27 in both. Chiero previously accepted it in *both* modes,
            // which was a `Miss` against `-pedantic-errors` that no VPP sweep could show
            // because the sweep runs `--gnu`.
            'e' => {
                gnu_only.push("unknown escape sequence `\\e`".to_owned());
                out.push(StrUnit::Raw(27));
            }
            '\\' => out.push(StrUnit::Raw(92)),
            '\'' => out.push(StrUnit::Raw(39)),
            '"' => out.push(StrUnit::Raw(34)),
            '?' => out.push(StrUnit::Raw(63)),
            // **Hex is greedy** (C11 6.4.4.4p1): `\xABC` is one escape, not `\xAB` then
            // `C`. That is why the octal case below is bounded and this one is not.
            'x' => {
                let mut v: u32 = 0;
                let mut any = false;
                while let Some(d) = it.peek().and_then(|c| c.to_digit(16)) {
                    v = v.wrapping_mul(16).wrapping_add(d);
                    any = true;
                    it.next();
                }
                // `\x` with no digits is not a valid escape; keep the letter rather than
                // silently deleting it, which is what the catch-all below does too.
                if !any {
                    bad.push("`\\x` used with no following hex digits".into());
                }
                out.push(if any {
                    StrUnit::Raw(v)
                } else {
                    StrUnit::Char(u32::from(b'x'))
                });
            }
            // **Octal takes at most three digits** (C11 6.4.4.4p1), so `\0101` is `\010`
            // followed by the character `1`. The first digit is the one already consumed.
            '0'..='7' => {
                let mut v = e.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match it.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            it.next();
                        }
                        None => break,
                    }
                }
                out.push(StrUnit::Raw(v));
            }
            // **Universal character names take exactly four or eight digits** (C11
            // 6.4.3p1) — fixed-width, unlike hex. A short one is malformed; the letters are
            // kept so the object does not silently shrink.
            'u' | 'U' => {
                let n = if e == 'u' { 4 } else { 8 };
                let mut v: u32 = 0;
                let mut got = 0;
                while got < n {
                    match it.peek().and_then(|c| c.to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            got += 1;
                            it.next();
                        }
                        None => break,
                    }
                }
                if got == n {
                    out.push(StrUnit::Char(v));
                } else {
                    bad.push(format!(
                        "incomplete universal character name: `\\{e}` takes {n} hex digits"
                    ));
                    out.push(StrUnit::Char(e as u32));
                    // Re-emit what was consumed, so a malformed escape keeps its length.
                    for sh in (0..got).rev() {
                        let d = (v >> (4 * sh)) & 0xf;
                        let ch = char::from_digit(d, 16).unwrap_or('0');
                        out.push(StrUnit::Char(ch as u32));
                    }
                }
            }
            // An unknown escape keeps the escaped character, which is what gcc does for
            // the ones it warns about.
            other => {
                if !gnu_accepts_escape(other) {
                    bad.push(format!("unknown escape sequence `\\{other}`"));
                } else {
                    gnu_only.push(format!("unknown escape sequence `\\{other}`"));
                }
                out.push(StrUnit::Char(other as u32));
            }
        }
    }
    out
}

/// Whether any numeric escape in `content` names a value one `bits`-wide element cannot hold
/// (C 6.4.4.4p9).
///
/// **Separate from the walk because it needs the prefix, which the walk does not have.**
/// `"\x1FF"` is a constraint violation and `L"\x1FF"` is legal; the limit is the width of one
/// element, and only the caller knows which literal this content came from.
///
/// **Only a `Raw` unit is checked.** A `Char` is a *character* — a literal `é` or a well-formed
/// universal character name — and one above 255 in a narrow string is encoded as UTF-8 rather than
/// being out of range. Checking those too would reject every non-ASCII string literal in the
/// corpus.
pub fn escape_range_defect(content: &str, bits: u32) -> Option<String> {
    let max: u64 = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    string_units(content).into_iter().find_map(|u| match u {
        StrUnit::Raw(v) if u64::from(v) > max => Some(format!(
            "escape sequence value {v} does not fit a {bits}-bit element"
        )),
        _ => None,
    })
}

/// Encode a fragment's **spelling** into the elements of an array of `bits`-wide elements,
/// **excluding** the terminator.
///
/// This is the whole width-dependent half of phase 5:
///
/// - a [`StrUnit::Raw`] is one element, truncated to the width;
/// - a [`StrUnit::Char`] is **UTF-8** at 8 bits, **UTF-16** at 16 (so a code point above
///   the BMP becomes a surrogate *pair* — two elements), and itself at 32.
///
/// A value that is not a Unicode scalar — a lone surrogate, or anything above U+10FFFF —
/// has no encoding at 8 or 16 bits. It is passed through truncated rather than dropped:
/// the object keeps its size, and the wrong answer stays a value rather than becoming a
/// length. gcc rejects those literals outright, so nothing well-formed reaches the arm.
pub fn string_elements(spelling: &str, bits: u32) -> Vec<u64> {
    let mask: u64 = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut out = Vec::new();
    for u in string_units(unquote(spelling)) {
        match u {
            StrUnit::Raw(v) => out.push(u64::from(v) & mask),
            StrUnit::Char(c) => match (bits, char::from_u32(c)) {
                (8, Some(ch)) => {
                    let mut buf = [0u8; 4];
                    out.extend(ch.encode_utf8(&mut buf).bytes().map(u64::from));
                }
                (16, Some(ch)) => {
                    let mut buf = [0u16; 2];
                    out.extend(ch.encode_utf16(&mut buf).iter().map(|&u| u64::from(u)));
                }
                _ => out.push(u64::from(c) & mask),
            },
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Character constants: the same units, a different assembly rule
// ---------------------------------------------------------------------------------------

/// The `(signed, bits)` of a **character constant**, from its prefix.
///
/// The one place a character constant and a string literal disagree about the prefix: a
/// *plain* constant has type `int` (C11 6.4.4.4p10), not `char`, so it is 32 bits where a
/// plain string literal's element is 8. Every prefixed form matches the string rule --
/// `u'a'` is `char16_t`, and its size of 2 is the only one a `sizeof` test can tell from
/// `int`'s.
pub fn char_element(spelling: &str) -> (bool, u32) {
    if spelling.starts_with("u8") {
        // C23's `u8'a'` is `unsigned char`; C11 has no such constant and gcc rejects it
        // under `-std=c11`, so nothing well-formed for the one standard 013 targets
        // reaches here.
        (false, 8)
    } else if spelling.starts_with('L') || spelling.starts_with('U') || spelling.starts_with('u') {
        string_element(spelling)
    } else {
        (true, 32)
    }
}

/// The content of a character constant's spelling: everything between the quotes.
pub fn unquote_char(spelling: &str) -> &str {
    match (spelling.find('\''), spelling.rfind('\'')) {
        (Some(a), Some(b)) if b > a => &spelling[a + 1..b],
        _ => spelling,
    }
}

/// The **value** of a character constant.
///
/// Shares [`string_units`] with string literals, which is the whole point: `'é'` is
/// 50089 only because the UCN becomes the two UTF-8 bytes `C3 A9` *and* two bytes make a
/// multi-character constant. Those two rules are in different paragraphs of the standard
/// and interact only through the byte sequence, so a decoder that produced a value directly
/// could not see it.
///
/// Assembly differs from a string literal's in two ways:
///
/// - a **prefixed** constant is one element and takes the first unit. C11 6.4.4.4p2 makes
///   a multi-character prefixed constant ill-formed, and gcc rejects it, so nothing
///   well-formed depends on what the rest would have done.
/// - a **plain** constant is a sequence of *bytes*, and C11 6.4.4.4p10 leaves more than one
///   implementation-defined. gcc accumulates them big-endian into an `int`, and gcc is the
///   oracle the corpus is compared against. A *single* byte is converted as a `char`, which
///   on this target is signed -- so `'\xFF'` is -1 and not 255. That sign is the only place
///   the `Raw`/`Char` distinction changes a sign rather than a count.
pub fn char_value(spelling: &str) -> Option<i128> {
    let units = string_units(unquote_char(spelling));
    let (signed, bits) = char_element(spelling);
    if !spelling.starts_with('\'') {
        // Prefixed: one element, at the element type's width and signedness.
        let v = i128::from(match units.first()? {
            StrUnit::Raw(v) | StrUnit::Char(v) => *v,
        });
        let masked = v & ((1i128 << bits) - 1);
        return Some(if signed && masked >> (bits - 1) != 0 {
            masked - (1i128 << bits)
        } else {
            masked
        });
    }
    // Plain: bytes, exactly as the same characters would be stored in a plain string.
    let bytes: Vec<u64> = string_elements(&format!("\"{}\"", unquote_char(spelling)), 8);
    match bytes.len() {
        0 => None,
        // One byte, converted as `char` -- signed on x86-64, the one target 014 models.
        1 => Some(i128::from(bytes[0] as u8 as i8)),
        // Multi-character: big-endian into an `int`, keeping only the low four bytes and
        // reading the result as signed, which is what makes a four-byte constant able to
        // come out negative.
        _ => {
            let mut v: u32 = 0;
            for b in bytes {
                v = (v << 8) | (b as u32 & 0xff);
            }
            Some(i128::from(v as i32))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unit-level distinction the whole module exists for, checked without a compiler
    /// in the loop. `differential.rs` checks the answers against gcc; this checks that the
    /// two spellings of one character decode to the *same units*, which is the property
    /// that keeps them from being fixed one at a time.
    #[test]
    fn a_ucn_and_the_character_it_names_decode_alike() {
        assert_eq!(string_units("\\uFFFF"), string_units("\u{FFFF}"));
        assert_eq!(string_units("\\U0001F600"), string_units("\u{1F600}"));
        assert_eq!(string_units("\\u00E9"), vec![StrUnit::Char(0xE9)]);
    }

    /// **`\x` is a value and `\u` is a character**, which is the difference a byte-oriented
    /// decoder cannot represent: at 8 bits one stays a single element and the other becomes
    /// its UTF-8 encoding.
    #[test]
    fn a_hex_escape_is_not_re_encoded_but_a_ucn_is() {
        assert_eq!(string_elements("\"\\xFF\"", 8), vec![0xFF]);
        assert_eq!(string_elements("\"\\u00FF\"", 8), vec![0xC3, 0xBF]);
        assert_eq!(string_elements("\"\\xFF\"", 16), vec![0xFF]);
        assert_eq!(string_elements("\"\\u00FF\"", 16), vec![0xFF]);
    }

    /// The two greedy/bounded rules that differ between hex and octal, and the fixed width
    /// of a UCN. Each is a case where reading one digit too many or too few changes the
    /// element *count*, not just a value.
    #[test]
    fn hex_is_greedy_octal_takes_three_and_a_ucn_takes_exactly_its_width() {
        assert_eq!(string_elements("\"\\x41B\"", 16), vec![0x41B]);
        assert_eq!(
            string_elements("\"\\0101\"", 8),
            vec![0o10, u64::from(b'1')]
        );
        assert_eq!(string_elements("\"\\101\"", 8), vec![u64::from(b'A')]);
        // Eight digits for `\U`, four for `\u`: the trailing `00` here is text.
        assert_eq!(
            string_elements("\"\\u004100\"", 16),
            vec![0x41, u64::from(b'0'), u64::from(b'0')]
        );
    }

    /// **A code point above the BMP is a surrogate pair at 16 bits** — two elements, so a
    /// decoder that forgot it would size the array wrong, not merely fill it wrong.
    #[test]
    fn an_astral_code_point_is_two_elements_at_sixteen_bits() {
        assert_eq!(string_elements("\"\\U0001F600\"", 16), vec![0xD83D, 0xDE00]);
        assert_eq!(string_elements("\"\\U0001F600\"", 32), vec![0x1F600]);
        assert_eq!(string_elements("\"\\U0001F600\"", 8).len(), 4);
    }

    /// A malformed escape keeps its length. The object's size is a value gcc is compared
    /// against, so swallowing the digits of a short `\u` would turn a diagnostic-worthy
    /// literal into a silently shorter array.
    #[test]
    fn a_malformed_escape_keeps_its_characters() {
        assert_eq!(string_elements("\"\\uAB\"", 8).len(), 3);
        assert_eq!(string_elements("\"\\x\"", 8), vec![u64::from(b'x')]);
    }

    /// The interaction that says why a character constant shares the string decoder rather
    /// than copying its arms: a UCN in a *plain* constant becomes UTF-8 bytes, and more than
    /// one byte is a multi-character constant. Two rules, from two paragraphs, meeting only
    /// in the byte sequence.
    #[test]
    fn a_ucn_in_a_plain_character_constant_is_a_multi_character_constant() {
        assert_eq!(char_value("'\\u00E9'"), Some(50089)); // C3 A9
        assert_eq!(char_value("'ab'"), Some(24930));
        assert_eq!(char_value("'\\0101'"), Some(2097)); // \010 then '1'
    }

    /// A single byte is converted as `char`, which is signed here -- the one place the
    /// `Raw`/`Char` distinction decides a *sign*.
    #[test]
    fn a_single_byte_character_constant_sign_extends() {
        assert_eq!(char_value("'\\xFF'"), Some(-1));
        assert_eq!(char_value("'a'"), Some(97));
        // The same byte in a wide constant is not a `char` and does not sign-extend.
        assert_eq!(char_value("u'\\xFF'"), Some(255));
    }

    /// The prefix decides the type, and `u` is the only one whose size differs from `int`'s.
    #[test]
    fn the_prefix_decides_a_character_constants_type() {
        assert_eq!(char_element("'a'"), (true, 32));
        assert_eq!(char_element("u'a'"), (false, 16));
        assert_eq!(char_element("U'a'"), (false, 32));
        assert_eq!(char_element("L'a'"), (true, 32));
        assert_eq!(char_value("u'\\uFFFF'"), Some(65535));
        assert_eq!(char_value("U'\\U0001F600'"), Some(128512));
    }
}
