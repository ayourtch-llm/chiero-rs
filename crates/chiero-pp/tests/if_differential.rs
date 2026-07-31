//! A differential channel for the `#if` expression evaluator (012 contract 11).
//!
//! The corpus in `chiero-lower` compares chiero against gcc on *translation units*. Nothing
//! compared them on *directives*, and `#if` has its own expression evaluator — a second
//! implementation of C constant expressions, with its own literal parser, its own usual
//! arithmetic conversions and its own operator table. Wave 297's mutation sweep found the
//! consequence: `#if` could compare, because comparisons were fixtured, but all four arms of its
//! division helper were dead to the suite. Fixtures close the holes someone thought to spell;
//! this closes the ones nobody did.
//!
//! **The comparison is on values, not on branches.** A `#if` selects a branch, which is one bit,
//! and a channel that only checks which branch was taken cannot see `7 / 2` yielding 4 instead
//! of 3. So each generated expression is emitted as sixty-four directives, one per bit, plus a
//! sign test: the token stream then encodes the whole 64-bit value and its signedness, and any
//! difference in any bit shows up as a token mismatch.

use chiero_pp::{Config, preprocess_str};
use std::fmt::Write as _;
use std::process::{Command, Stdio};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // Any nonzero state; xorshift is degenerate at 0.
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

/// Macros the generated expressions may ask about. `defined` is part of the `#if` grammar and is
/// evaluated by the directive layer rather than the expression layer, so it belongs in the
/// channel: it is the one operand whose value depends on preprocessor state.
const PRELUDE: &str = "#define SET 1\n#define ZERO 0\n#define WIDE 0x8000000000000000\n";

/// A leaf operand.
///
/// Every literal spelling the `#if` evaluator accepts is represented, because its literal parser
/// is separate from the lexer's and from the C parser's: decimal, octal, hex, binary, the four
/// suffix shapes, character constants with each escape form, `defined` in both spellings, and a
/// bare identifier — which is *not* an error in `#if`, it is zero.
fn leaf(rng: &mut Rng) -> String {
    match rng.below(14) {
        0 => rng.below(1000).to_string(),
        1 => format!("0{:o}", rng.below(512)),
        2 => format!("0x{:x}", rng.below(4096)),
        3 => format!("0X{:X}", rng.below(4096)),
        // `0b` is a GNU extension both this preprocessor and gcc accept in `#if`. It is generated
        // because `parse_if_literal` has a branch for it; a spelling with a branch and no
        // generator arm is exactly what wave 297's sweep kept finding.
        4 => format!("0b{:b}", rng.below(256)),
        5 => format!(
            "{}{}",
            rng.below(1000),
            rng.pick(&["u", "U", "L", "uL", "ll", "ULL"])
        ),
        // The multi-character constants are here for a specific reason: mutating the octal
        // escape's three-digit bound to four survived this channel while `'\101'` was the only
        // octal spelling in it, because a fourth digit is never available before the closing
        // quote. `'\1011'` is where the bound is observable — and a channel that cannot see a
        // bound is not covering it, however many expressions it generates.
        6 => (*rng.pick(&[
            "'A'", "'\\n'", "'\\0'", "'\\x41'", "'\\101'", "'\\\\'", "'\\t'", "'\\''", "'\\1011'",
            "'ab'", "'\\0101'",
        ]))
        .to_string(),
        // A wide character constant: the prefix is part of the token, and the value is the same.
        7 => (*rng.pick(&["L'A'", "u'A'", "U'A'"])).to_string(),
        8 => (*rng.pick(&[
            "defined(SET)",
            "defined ZERO",
            "defined(NOPE)",
            "defined WIDE",
        ]))
        .to_string(),
        9 => (*rng.pick(&["SET", "ZERO", "WIDE", "UNKNOWN_IDENTIFIER"])).to_string(),
        10 => (*rng.pick(&[
            "0",
            "1",
            "-1",
            "0x7fffffffffffffff",
            "0xffffffffffffffff",
            "18446744073709551615u",
        ]))
        .to_string(),
        _ => format!("{}", rng.below(1000)),
    }
}

/// An expression of bounded depth.
///
/// Three shapes are deliberately *not* generated, and each omission is a statement about what
/// this channel can prove rather than an oversight:
///
///   - **Division and modulo by zero.** gcc makes it a hard error, so the oracle would refuse the
///     program rather than answer it. The divisor is wrapped in `(… |1)`, which is nonzero for
///     every value including unsigned ones, so the operator is still exercised on arbitrary
///     operands. `#if 1/0` is covered by fixture instead.
///   - **Signed overflow.** Undefined in C, so a disagreement would be a gap and not a defect.
///     Operand magnitudes and the shift masks below are chosen so that a depth-4 tree of signed
///     values cannot leave `intmax_t`. Unsigned wraparound is *defined*, and is generated freely —
///     it is the interesting half anyway, since it is where the usual arithmetic conversions bite.
///   - **The comma operator.** Not permitted in a constant expression; gcc accepts it silently but
///     the standard does not, so it is fixtured rather than generated.
///   - **An out-of-range hex escape**, such as `'\x4142'`. gcc makes it a warning and computes a
///     value; clang makes it a hard error. Where the two oracles disagree about whether a program
///     is a program at all, this channel has no one to ask, and a construct it cannot arbitrate
///     does not belong in it. Multi-character constants are *not* in this category: both warn,
///     both compute, and both compute the same thing.
fn expr(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 {
        return leaf(rng);
    }
    match rng.below(14) {
        0 => format!("!{}", expr(rng, depth - 1)),
        1 => format!("~{}", expr(rng, depth - 1)),
        // Unary `+` and `-` parenthesize their operand: `- -1` would otherwise be spelled `--1`,
        // which is one token, and gcc rejects `--` in a preprocessor expression. This is a
        // property of the *spelling*, not of the evaluator, so it is the generator's job to
        // avoid it rather than something for the channel to report.
        2 => format!("-({})", expr(rng, depth - 1)),
        3 => format!("+({})", expr(rng, depth - 1)),
        4 => format!("({})", expr(rng, depth - 1)),
        // A nonzero divisor by construction: `x | 1` is odd, hence nonzero, for every x.
        5 => format!(
            "({} / (({}) | 1))",
            expr(rng, depth - 1),
            expr(rng, depth - 1)
        ),
        6 => format!(
            "({} % (({}) | 1))",
            expr(rng, depth - 1),
            expr(rng, depth - 1)
        ),
        // Shifts are masked into a defined range. A left shift of a negative value is undefined,
        // so the left operand is masked non-negative too.
        7 => format!(
            "((({}) & 0xff) << (({}) & 7))",
            expr(rng, depth - 1),
            expr(rng, depth - 1)
        ),
        8 => format!(
            "(({}) >> (({}) & 7))",
            expr(rng, depth - 1),
            expr(rng, depth - 1)
        ),
        9 => format!(
            "({} ? {} : {})",
            expr(rng, depth - 1),
            expr(rng, depth - 1),
            expr(rng, depth - 1)
        ),
        _ => {
            let op = rng.pick(&[
                "*", "+", "-", "<", ">", "<=", ">=", "==", "!=", "&", "^", "|", "&&", "||",
            ]);
            format!("({} {} {})", expr(rng, depth - 1), op, expr(rng, depth - 1))
        }
    }
}

/// Emit one expression as a value, bit by bit.
///
/// `((E) >> b) & 1` recovers bit `b` whether `E` is signed or unsigned: a right shift of a
/// negative value fills with sign bits, but those land above bit `b` and the mask discards them.
/// The extra `< 0` test recovers the signedness itself, which no bit pattern can show — an
/// unsigned value is never negative, so getting the usual arithmetic conversions wrong changes
/// this token even when every bit agrees.
fn probe(out: &mut String, index: usize, expression: &str) {
    for bit in 0..64 {
        let _ = writeln!(
            out,
            "#if ((({expression})) >> {bit}) & 1\nc{index}b{bit}\n#endif"
        );
    }
    let _ = writeln!(out, "#if ({expression}) < 0\nc{index}neg\n#endif");
}

fn compiler_tokens(compiler: &str, src: &str) -> Vec<String> {
    let mut child = Command::new(compiler)
        .args(["-E", "-P", "-std=gnu11", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), src.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{compiler} rejected the generated directives: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// The channel. Each seed is one expression; they share a file so the oracle runs twice, not
/// twice per expression.
#[test]
fn generated_if_expressions_agree_with_gcc_and_clang() {
    let count: usize = std::env::var("CHIERO_IF_DIFF_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(400);
    let base: u64 = std::env::var("CHIERO_IF_DIFF_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    let mut expressions = Vec::new();
    let mut src = String::from(PRELUDE);
    for index in 0..count {
        let mut rng = Rng::new(base + index as u64);
        let expression = expr(&mut rng, 4);
        probe(&mut src, index, &expression);
        expressions.push(expression);
    }

    let ours: Vec<String> = preprocess_str("if-diff.c", &src, Config::default())
        .token_texts()
        .map(str::to_owned)
        .collect();
    let gcc = compiler_tokens("gcc", &src);
    let clang = compiler_tokens("clang", &src);
    assert_eq!(
        gcc, clang,
        "the independent compilers must agree before either can judge us"
    );

    if ours == gcc {
        return;
    }

    // Attribute the first disagreement to the expression that produced it, so the failure names a
    // `#if` a person can paste into a file rather than a token index in a 26,000-line stream.
    let index = ours
        .iter()
        .zip(&gcc)
        .position(|(a, b)| a != b)
        .unwrap_or(ours.len().min(gcc.len()));
    let blame = ours
        .get(index)
        .or_else(|| gcc.get(index))
        .and_then(|token| token.trim_start_matches('c').split('b').next())
        .and_then(|digits| digits.trim_end_matches("neg").parse::<usize>().ok());
    let culprit = blame
        .and_then(|i| expressions.get(i))
        .map_or_else(|| "<unattributed>".to_owned(), |e| e.clone());

    panic!(
        "#if disagreed with gcc at token {index}\n  expression: #if {culprit}\n  \
         ours: {:?}\n  gcc:  {:?}",
        &ours[index.saturating_sub(2)..(index + 3).min(ours.len())],
        &gcc[index.saturating_sub(2)..(index + 3).min(gcc.len())],
    );
}
