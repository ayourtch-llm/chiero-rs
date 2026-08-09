//! **Generated integer constant expressions, graded by gcc.** Covers: 014 contract 6.
//!
//! Constant evaluation decides **types, not values**: an array bound, a bit-field width, an
//! enumerator, a `case` label, a `_Static_assert`. A wrong answer there does not produce a
//! wrong number at run time — it produces a differently-shaped program, and everything built
//! on top of it is confidently wrong in the way 014 §7 describes.
//!
//! It is also the least differentially-tested path in this crate. `const_evaluator_reuse.rs`
//! is thirteen assertions and **asks gcc nothing** — its subject is that a shared evaluator
//! answers what a fresh one would, which is a self-consistency property: it would hold
//! perfectly while both were wrong. `semantics.rs` asks gcc constantly but reaches
//! `const_eval` six times. So the folding arithmetic itself is graded almost entirely against
//! hand-written expectations, which is the bottleneck `generated.rs` was built to remove for
//! run-time values and `generated_layout.rs` for record layout.
//!
//! # The oracle is one `_Static_assert` and no output parsing
//!
//! chiero folds the expression to an `i128`; the same expression is handed to gcc inside
//! `_Static_assert(<expr> == <chiero's answer>, …)`. gcc either compiles the file or it does
//! not, so there is nothing to parse and nothing to get wrong in the comparison. This is the
//! trick `assert_agrees_with_gcc` uses for layout, applied to a scalar.
//!
//! ⚠️ **A constant expression gcc rejects is not a chiero defect**, and the two must not share
//! a bucket. gcc refuses `1/0` in a constant expression outright, and the generator avoids
//! that by construction; anything else it refuses is reported as its own row and counted, so
//! a grammar that drifted into emitting rejected C would show up as a collapse in the compared
//! count rather than as a green run over nothing.
//!
//! # Undefined behaviour is avoided by construction, not discarded
//!
//! Unlike the run-time generator there is no sanitizer to fall back on: a constant expression
//! is folded at compile time and UB in one is a diagnostic, not a trap. So the grammar keeps
//! every operation defined — division and remainder only by nonzero literals, shift counts
//! below the width and never negative, and `+`/`-`/`*` widened to `long long` so no sum or
//! product can overflow.
//!
//! ⚠️ **That last clause is a correction.** It first read "operands drawn from a pool small
//! enough that no product overflows the 64-bit type everything is computed in" — true, and
//! about the wrong type. An unsuffixed literal is an `int`, the arithmetic happens there, and
//! `0x7fffffff + 65535` overflows it. One seed in three hundred found it.

mod harness;

use chiero_sema::{ConstVal, TargetConfig, const_eval};

// ---------------------------------------------------------------------------------------
// A PRNG, written out rather than depended on
// ---------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
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

// ---------------------------------------------------------------------------------------
// The grammar
// ---------------------------------------------------------------------------------------

/// **Adversarial on purpose**, and every value fits in 32 bits so that a product of two of
/// them fits in 64 and nothing the grammar builds can overflow `long long`.
///
/// The boundaries are what discriminate: `0` and `1` for the identities, `-1` for the
/// sign-extension mistakes, `0x7fffffff`/`0x80000000` for the `int`/`unsigned` edge that
/// decides which type the usual arithmetic conversions pick.
const POOL: &[&str] = &[
    "0",
    "1",
    "2",
    "-1",
    "-2",
    "7",
    "255",
    "256",
    "32767",
    "32768",
    "65535",
    "65536",
    "0x7fffffff",
    "0x80000000",
    "0xffffffff",
    "2147483647",
    "-2147483648",
];

/// The suffixes that change an operand's *type* without changing its digits, which is the
/// whole subject of 6.3.1.8 and the thing a folder gets wrong silently.
const SUFFIXES: &[&str] = &["", "", "", "u", "l", "ul", "ll", "ull"];

struct Gen {
    rng: Rng,
}

impl Gen {
    /// One operand: a pooled literal with a suffix, a `sizeof`, or a parenthesised cast.
    fn leaf(&mut self) -> String {
        match self.rng.below(10) {
            // `sizeof` is folded by the same evaluator and is the operand class that reaches
            // the target's type table rather than its arithmetic.
            0 => {
                let t = *self.rng.pick(&[
                    "char",
                    "short",
                    "int",
                    "long",
                    "long long",
                    "unsigned int",
                    "void *",
                    "double",
                ]);
                format!("sizeof({t})")
            }
            // A cast, which is where a fold that keeps the wrong width shows up.
            1 => {
                let t = *self.rng.pick(&[
                    "char",
                    "signed char",
                    "unsigned char",
                    "short",
                    "unsigned short",
                    "int",
                    "unsigned",
                    "long",
                    "unsigned long",
                ]);
                let inner = self.expr(1);
                format!("(({t})({inner}))")
            }
            _ => {
                let v = *self.rng.pick(POOL);
                let s = *self.rng.pick(SUFFIXES);
                // A suffix on a negative literal belongs to the digits, not the sign.
                match v.strip_prefix('-') {
                    Some(d) => format!("(-{d}{s})"),
                    None => format!("{v}{s}"),
                }
            }
        }
    }

    fn expr(&mut self, depth: usize) -> String {
        if depth == 0 {
            return self.leaf();
        }
        match self.rng.below(12) {
            // Comparisons and bitwise operators, which **cannot overflow** whatever their
            // operands are — so the pool's `int`/`unsigned` edge values reach them raw, which
            // is where the usual-arithmetic-conversion rules are decided.
            0..=3 => {
                let op = *self
                    .rng
                    .pick(&["&", "|", "^", "<", ">", "<=", ">=", "==", "!="]);
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                format!("({a} {op} {b})")
            }
            // ⚠️ **Additive and multiplicative operators are widened, and that is a cost.**
            // The pool holds `0x7fffffff` and `2147483647` precisely because they are the
            // `int` edge, and `0x7fffffff + 65535` overflows `int` — which is UB, which this
            // file's header promises to avoid *by construction* and did not. chiero diagnoses
            // it correctly; both compilers fold it to -2147418114 with a warning, so the
            // divergence is real and is pinned as a named fixture rather than generated.
            //
            // The header's original claim was that the pool "fits in 32 bits so that a
            // product of two fits in 64" — true, and about the wrong type: an unsuffixed
            // literal is an `int`, and the arithmetic happens there.
            //
            // What is lost: `+`/`-`/`*` no longer see the `int`/`unsigned` boundary in their
            // own operand types. The nine operators above still do.
            4 => {
                let op = *self.rng.pick(&["+", "-"]);
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                format!("(((long long)({a})) {op} ((long long)({b})))")
            }
            // A product of two pooled values can reach 2^64; one side is a small literal so
            // the result stays inside `long long`.
            10 => {
                let a = self.expr(depth - 1);
                let n = 1 + self.rng.below(97);
                format!("(((long long)({a})) * {n})")
            }
            // **Division and remainder by a nonzero literal only.** `1/0` in a constant
            // expression is a diagnostic in both compilers, not a value to compare.
            5 => {
                let op = *self.rng.pick(&["/", "%"]);
                let a = self.expr(depth - 1);
                let d = 1 + self.rng.below(97);
                format!("({a} {op} {d})")
            }
            // **Shifts with the count below the width and never negative.** The operand is
            // masked to 16 bits first so a left shift cannot overflow the promoted type.
            6 => {
                let op = *self.rng.pick(&["<<", ">>"]);
                let a = self.expr(depth - 1);
                let n = self.rng.below(15);
                format!("((({a}) & 0xffff) {op} {n})")
            }
            // The logical operators, which are the ones whose *result type* is `int`
            // whatever the operands were.
            7 => {
                let op = *self.rng.pick(&["&&", "||"]);
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                format!("({a} {op} {b})")
            }
            // A conditional, where only one arm is evaluated but both decide the type.
            8 => {
                let c = self.expr(depth - 1);
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                format!("(({c}) ? ({a}) : ({b}))")
            }
            9 => {
                let op = *self.rng.pick(&["-", "~", "!", "+"]);
                let a = self.expr(depth - 1);
                format!("({op}({a}))")
            }
            _ => self.leaf(),
        }
    }
}

fn expression_for(seed: u64) -> String {
    let mut g = Gen {
        rng: Rng::new(seed),
    };
    g.expr(3)
}

// ---------------------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// chiero folded it and gcc agrees the answer is right.
    Agrees,
    /// chiero declined to fold, or diagnosed. Counted, never a pass.
    NotFolded(String),
    /// chiero folded to an address rather than an integer — impossible for this grammar,
    /// so its own row rather than a silent skip.
    NotAnInteger,
    /// gcc would not compile the assertion. **Two causes and they must not be merged**: the
    /// answer is wrong, or the expression is one gcc rejects. The message distinguishes them.
    GccRefused(String),
}

fn chiero_value(src: &str) -> Result<i128, Outcome> {
    let (parsed, expr) = harness::expression(src);
    let names = harness::names_of(&parsed);
    let mut diags = Vec::new();
    let v = const_eval(
        &parsed.ast,
        expr,
        &names,
        &TargetConfig::x86_64_linux(),
        &mut diags,
    );
    // ⚠️ **A diagnostic beside a value is not a refusal**, and reading them the other way
    // round is what made this gate report a defect that was not one. `0x7fffffff + 65535`
    // folds to -2147418114 — byte for byte what gcc and clang fold it to — *and* chiero says
    // "signed overflow in a constant expression", which is more honest than either compiler
    // and is not a reason to stop comparing. The value decides; the diagnostic is reported
    // beside it. Third time this session the same conflation cost a wave: a `Gap` filed as a
    // `Discarded`, a pedantic `__int128` sentence filed as a refusal, and this.
    match v {
        Some(ConstVal::Int(i)) => Ok(i),
        Some(ConstVal::Addr { .. }) => Err(Outcome::NotAnInteger),
        None => Err(Outcome::NotFolded(match diags.first() {
            Some(d) => format!("{d:?}"),
            None => "no value and no diagnostic".into(),
        })),
    }
}

/// Put chiero's answer to gcc as a `_Static_assert`, and separately ask whether gcc can
/// compile the expression at all.
///
/// **The second question is what keeps the first honest.** Without it, an expression gcc
/// rejects for its own reasons is indistinguishable from chiero computing the wrong number,
/// and the gate would report a defect it cannot substantiate.
fn gcc_verdict(src: &str, chiero: i128, dir: &std::path::Path, seq: u64) -> Outcome {
    let c = dir.join(format!("k{seq}.c"));
    let assertion = format!(
        "_Static_assert(({src}) == ({chiero}), \"chiero says {chiero}\");\nint main(void){{return 0;}}\n"
    );
    std::fs::write(&c, &assertion).expect("write probe");
    let out = std::process::Command::new("gcc")
        .args(["-std=gnu11", "-w", "-fsyntax-only"])
        .arg(&c)
        .output()
        .expect("run gcc");
    if out.status.success() {
        return Outcome::Agrees;
    }
    // Does gcc accept the expression at all, independent of the value?
    let plain = dir.join(format!("p{seq}.c"));
    std::fs::write(
        &plain,
        format!("long long v = ({src});\nint main(void){{return 0;}}\n"),
    )
    .expect("write probe");
    let ok = std::process::Command::new("gcc")
        .args(["-std=gnu11", "-w", "-fsyntax-only"])
        .arg(&plain)
        .output()
        .expect("run gcc");
    if !ok.status.success() {
        return Outcome::GccRefused(format!(
            "gcc rejects the expression itself, so the value was never compared:\n{src}\n{}",
            String::from_utf8_lossy(&ok.stderr)
        ));
    }
    Outcome::GccRefused(format!(
        "gcc accepts `{src}` and contradicts chiero's {chiero}:\n{}",
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// **Fixed seeds, so this is a test and not a slot machine**, and `#[ignore]`d because every
/// expression costs a gcc invocation.
#[test]
#[ignore = "external oracle — one or two gcc invocations per generated expression"]
fn generated_constant_expressions_agree_with_gcc() {
    if !harness::gcc_available() {
        panic!("014 §7 needs the compiler; an oracle that can silently not run is not one");
    }
    let dir = std::env::temp_dir().join(format!("chiero-constgen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let mut agreed = 0usize;
    let mut not_folded: Vec<(u64, String, String)> = Vec::new();
    let mut wrong: Vec<(u64, String, String)> = Vec::new();
    let mut rejected: Vec<(u64, String)> = Vec::new();

    for seed in 0..300u64 {
        let src = expression_for(seed);
        match chiero_value(&src) {
            Err(Outcome::NotFolded(why)) => not_folded.push((seed, src, why)),
            Err(Outcome::NotAnInteger) => {
                not_folded.push((seed, src, "folded to an address".into()))
            }
            Err(other) => panic!("unexpected {other:?}"),
            Ok(v) => match gcc_verdict(&src, v, &dir, seed) {
                Outcome::Agrees => agreed += 1,
                Outcome::GccRefused(why) if why.starts_with("gcc rejects the expression") => {
                    rejected.push((seed, why))
                }
                Outcome::GccRefused(why) => wrong.push((seed, src, why)),
                other => panic!("unexpected {other:?}"),
            },
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    eprintln!(
        "generated constants: {agreed} agree with gcc, {} WRONG, {} not folded, \
         {} gcc rejected",
        wrong.len(),
        not_folded.len(),
        rejected.len()
    );
    for (seed, _, _) in &wrong {
        eprintln!("  WRONG seed {seed}");
    }

    assert!(
        wrong.is_empty(),
        "{} expression(s) chiero folded to a value gcc contradicts. First:\n{}",
        wrong.len(),
        wrong[0].2
    );
    assert!(
        not_folded.is_empty(),
        "{} expression(s) chiero would not fold. 014 §6 makes these constant expressions, so \
         declining is a gap and not a pass:\n{:#?}",
        not_folded.len(),
        not_folded
    );
    assert!(
        rejected.is_empty(),
        "{} expression(s) gcc will not compile — the generator is emitting C it should not, \
         and every one of them is an expression whose value was never compared:\n{:#?}",
        rejected.len(),
        rejected
    );
    // **A floor, not `> 0`.** The whole value of the channel is volume across shapes.
    assert!(
        agreed >= 250,
        "only {agreed} of 300 expressions were compared; a channel that folds almost nothing \
         is green while testing almost nothing"
    );
}

/// **Signed overflow in a constant expression: chiero folds to the same value *and* warns.**
///
/// Found by the generator on its first run and pinned here because the generator can no
/// longer produce it — `+`/`-`/`*` are widened to `long long` now, so the shape would be lost
/// entirely, which is the decay a named fixture exists to stop.
///
/// Measured, not assumed: `gcc -std=gnu11`, `gcc -std=c11 -pedantic-errors` and `clang` all
/// accept `0x7fffffff + 65535` and fold it to `-2147418114`, warning under
/// `-Woverflow`/`-Winteger-overflow`. **chiero folds it to the same value and issues its own
/// diagnostic** — which is the best available behaviour on undefined input: the number the
/// compilers will actually use, plus a declared concern about it.
///
/// ⚠️ **The first version of this fixture asserted the opposite** — that chiero refuses to
/// fold — because the gate that found the expression checked `diags.first()` *before* the
/// value and reported "not folded". A diagnostic beside a correct answer is not a refusal,
/// and that is the third time in this session the same conflation cost a wave.
#[test]
fn a_signed_overflowing_constant_expression_folds_like_gcc_and_warns() {
    let (parsed, expr) = harness::expression("0x7fffffff + 65535");
    let names = harness::names_of(&parsed);
    let mut diags = Vec::new();
    let v = const_eval(
        &parsed.ast,
        expr,
        &names,
        &TargetConfig::x86_64_linux(),
        &mut diags,
    );
    // **The value is gcc's**, byte for byte — measured, not assumed: `gcc -std=gnu11`,
    // `gcc -std=c11 -pedantic-errors` and `clang` all fold this to -2147418114.
    assert_eq!(
        v,
        Some(ConstVal::Int(-2147418114)),
        "chiero folds a signed-overflowing constant expression to the same value the \
         compilers do"
    );
    // **And says so, which neither compiler does at error level.** That is the whole point:
    // a value plus a declared concern, not a value pretending nothing happened.
    assert!(
        diags.iter().any(|d| d.message.contains("signed overflow")),
        "the diagnostic is the honest half; without it this is a wrapped answer nobody was \
         warned about: {diags:?}"
    );

    // The discriminator: the same shape one below the edge folds normally, so the refusal is
    // about the overflow and not about the operator or the literal spelling.
    let (p2, e2) = harness::expression("0x7ffffffe + 1");
    let n2 = harness::names_of(&p2);
    let mut d2 = Vec::new();
    let v2 = const_eval(&p2.ast, e2, &n2, &TargetConfig::x86_64_linux(), &mut d2);
    assert_eq!(v2, Some(ConstVal::Int(0x7fffffff)), "{d2:?}");
    assert!(d2.is_empty(), "{d2:?}");
}

/// **The generator reaches the operand classes the hand fixtures do not.**
///
/// Presence is not discrimination, and this is not the justification for the file — it is the
/// guard against the justification quietly ceasing to hold, which has now happened three times
/// to `generated_layout.rs` when a grammar change moved what the fixed seeds produce.
#[test]
fn the_constant_generator_reaches_its_operand_classes() {
    let (mut sizeof_, mut cast, mut shift, mut cond, mut logical, mut suffixed, mut div) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut longest = 0usize;
    for seed in 0..400u64 {
        let e = expression_for(seed);
        longest = longest.max(e.len());
        if e.contains("sizeof(") {
            sizeof_ += 1;
        }
        if e.contains(")(") {
            cast += 1;
        }
        if e.contains("<<") || e.contains(">>") {
            shift += 1;
        }
        if e.contains(" ? ") {
            cond += 1;
        }
        if e.contains("&&") || e.contains("||") {
            logical += 1;
        }
        if e.contains("u)") || e.contains("ul") || e.contains("ll") {
            suffixed += 1;
        }
        if e.contains(" / ") || e.contains(" % ") {
            div += 1;
        }
    }
    eprintln!(
        "constant reach over 400 seeds: {sizeof_} sizeof, {cast} cast, {shift} shift, \
         {cond} conditional, {logical} logical, {suffixed} suffixed, {div} div/rem; \
         longest {longest} chars"
    );
    assert!(sizeof_ >= 40, "`sizeof` operands: {sizeof_}");
    assert!(cast >= 40, "casts: {cast}");
    assert!(shift >= 40, "shifts: {shift}");
    assert!(cond >= 20, "conditionals: {cond}");
    assert!(logical >= 20, "logical operators: {logical}");
    assert!(suffixed >= 40, "type-changing suffixes: {suffixed}");
    assert!(div >= 20, "division and remainder: {div}");
}
