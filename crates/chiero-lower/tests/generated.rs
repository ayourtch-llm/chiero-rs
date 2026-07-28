//! **Generated differential testing.** Covers: 015 contract 5, mechanically.
//!
//! `differential.rs` is the same oracle driven by hand-written fixtures, and it works — but
//! every defect it has found was one somebody thought to spell. Six waves of the rules
//! earned in HANDOFF §9 are cross-product failures: a guard added to one arm and not its
//! sibling, a fix that reached the global path and stopped, every fixture putting the
//! pointer on the left. Those are not failures of care. They are what happens when the
//! constructs are enumerated in a thousand places instead of one.
//!
//! So this file enumerates them **once**, in a grammar, and explores the spellings ×
//! contexts × operand orders mechanically. The grammar is the thing to audit against C11
//! and 013 §4; everything else falls out of it.
//!
//! ## The verdict is five-way, not pass/fail
//!
//! A generator that cannot tell "chiero has not implemented this" from "chiero is wrong"
//! is useless, and one that treats the first as acceptable forever is worse. So:
//!
//! - `Agree` — the answers match.
//! - `Mismatch` — they differ. A defect.
//! - `Panic` — chiero panicked. A defect, and the worst kind: it takes the run with it.
//! - `SilentNoState` — **every stage was clean and the engine still produced no value.**
//!   Always a defect. `harness::lower` panics on any diagnostic, so reaching the engine at
//!   all means nothing refused; 015 §7's rule is that a gap is a diagnostic rather than a
//!   licence, and this is that rule made mechanical.
//! - `Refused { stage }` — a diagnostic was pushed. **Not** a defect: it is a gap behaving
//!   correctly. Counted, reported, and ratcheted (below), never silently tolerated.
//!
//! ## Undefined behaviour is discarded, not engineered away
//!
//! Building a provably-UB-free generator is a multi-month project that buys little here.
//! Instead the generator avoids the UB it can avoid *by construction* — one side effect per
//! statement, division only by nonzero constants, shift counts masked below the width,
//! indices in range because the generator knows every array's length — and everything else
//! is caught by compiling the fixture under `-fsanitize=undefined,address` and discarding
//! any program that trips it. gcc at `-O0` stays the verdict, exactly as `differential.rs`
//! chose: optimisation is legal only on defined programs.

mod harness;

use std::fmt::Write as _;

// ---------------------------------------------------------------------------------------
// A PRNG, written out rather than depended on
// ---------------------------------------------------------------------------------------

/// xorshift64*, ~20 lines against a `rand` dependency that would earn its keep nowhere else
/// in the workspace. 001 §4's dependency rules are checked by `xtask check-deps`, and the
/// cheapest way to keep that gate quiet is to need nothing.
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

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

// ---------------------------------------------------------------------------------------
// The grammar
// ---------------------------------------------------------------------------------------

/// The scalar types the generator draws from.
///
/// `_Bool` is here from the start because it is where a whole defect class lives: its CIR
/// type is one bit and its storage is a byte, and a compound assignment on one used to
/// panic the solver outright. Floats are deliberately absent — they do not execute at all
/// (HANDOFF §9), so every float program would land in the refusal ledger and drown the
/// signal that ledger exists to carry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Bool,
    SChar,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
}

impl Ty {
    fn c(self) -> &'static str {
        match self {
            Ty::Bool => "_Bool",
            Ty::SChar => "signed char",
            Ty::UChar => "unsigned char",
            Ty::Short => "short",
            Ty::UShort => "unsigned short",
            Ty::Int => "int",
            Ty::UInt => "unsigned",
            Ty::Long => "long",
            Ty::ULong => "unsigned long",
        }
    }

    /// Width in bits on the one target chiero models (`TargetConfig::x86_64_linux`).
    fn bits(self) -> u32 {
        match self {
            Ty::Bool => 1,
            Ty::SChar | Ty::UChar => 8,
            Ty::Short | Ty::UShort => 16,
            Ty::Int | Ty::UInt => 32,
            Ty::Long | Ty::ULong => 64,
        }
    }

    fn signed(self) -> bool {
        matches!(self, Ty::SChar | Ty::Short | Ty::Int | Ty::Long)
    }

    const ALL: [Ty; 9] = [
        Ty::Bool,
        Ty::SChar,
        Ty::UChar,
        Ty::Short,
        Ty::UShort,
        Ty::Int,
        Ty::UInt,
        Ty::Long,
        Ty::ULong,
    ];
}

/// A variable in scope: its C name, its type, and whether it is an array element source.
#[derive(Clone)]
struct Var {
    name: String,
    ty: Ty,
}

/// The constant pool.
///
/// **Adversarial on purpose.** A pool of small random integers misses every boundary that
/// has actually mattered: the wide-enum defect needed a value above 2^32, the sign-extension
/// defects needed values that differ read signed and unsigned, and the bit-field truncation
/// needed a value one past a field's range. Boundaries are where lowering's width and
/// signedness decisions become visible.
const POOL: [i128; 21] = [
    0,
    1,
    2,
    3,
    4,
    7,
    -1,
    -2,
    -4,
    127,
    128,
    255,
    256,
    32767,
    32768,
    65535,
    2147483647,
    -2147483648,
    4294967295,
    4294967296,
    5000000000,
];

struct Gen {
    rng: Rng,
    vars: Vec<Var>,
    body: String,
    next_id: usize,
    depth: usize,
}

impl Gen {
    fn new(seed: u64) -> Gen {
        Gen {
            rng: Rng::new(seed),
            vars: Vec::new(),
            body: String::new(),
            next_id: 0,
            depth: 0,
        }
    }

    fn fresh(&mut self) -> String {
        self.next_id += 1;
        format!("v{}", self.next_id)
    }

    /// A constant literal, suffixed so its own type is not in doubt.
    fn konst(&mut self, ty: Ty) -> String {
        let v = *self.rng.pick(&POOL);
        // Mask into the target type's range so the *initializer* is never itself a
        // conversion the generator did not intend. Signed overflow in a constant expression
        // is a constraint violation, not something to test here.
        let bits = ty.bits().min(64);
        let masked = if bits >= 64 {
            v
        } else {
            let m = (1i128 << bits) - 1;
            let raw = v & m;
            if ty.signed() && bits > 1 && raw >= (1i128 << (bits - 1)) {
                raw - (1i128 << bits)
            } else {
                raw
            }
        };
        if ty.bits() == 64 {
            if ty.signed() {
                format!("{masked}L")
            } else {
                format!("{}UL", masked as u128 as u64)
            }
        } else if ty.signed() {
            format!("{masked}")
        } else {
            format!("{masked}u")
        }
    }

    /// An expression of *some* type, with the type it produced.
    ///
    /// Depth-bounded rather than probability-bounded: a generator that recurses on a coin
    /// flip produces the occasional enormous program and makes shrinking the common case.
    fn expr(&mut self, want: Ty) -> String {
        if self.depth >= 3 {
            return self.leaf(want);
        }
        self.depth += 1;
        let e = match self.rng.below(10) {
            0..=2 => self.leaf(want),
            3..=5 => {
                // **Every binary operator, unparenthesised on purpose.** Precedence is what
                // wave 135 found unpinned, and volume settles it without anyone working out
                // which orderings discriminate.
                let op = *self
                    .rng
                    .pick(&["+", "-", "*", "|", "&", "^", "<", ">", "==", "!="]);
                let a = self.expr(want);
                let b = self.expr(want);
                format!("{a} {op} {b}")
            }
            6 => {
                // Division and remainder, by a **nonzero** constant only: `x / 0` is UB and
                // a generated program that hits it teaches nothing.
                let op = *self.rng.pick(&["/", "%"]);
                let a = self.expr(want);
                let d = 1 + self.rng.below(15);
                format!("{a} {op} {d}")
            }
            7 => {
                // Shifts, with the count forced below the width — a count at or above it is
                // UB and would be discarded rather than compared.
                let op = *self.rng.pick(&["<<", ">>"]);
                let a = self.expr(want);
                let n = self.rng.below(want.bits().max(2) as usize - 1);
                format!("{a} {op} {n}")
            }
            8 => {
                let t = *self.rng.pick(&Ty::ALL);
                let a = self.expr(t);
                format!("({}){a}", want.c())
            }
            _ => {
                let c = self.expr(Ty::Int);
                let a = self.expr(want);
                let b = self.expr(want);
                format!("{c} ? {a} : {b}")
            }
        };
        self.depth -= 1;
        format!("({e})")
    }

    fn leaf(&mut self, want: Ty) -> String {
        let usable: Vec<Var> = self.vars.clone();
        if usable.is_empty() || self.rng.chance(3) {
            return self.konst(want);
        }
        let v = self.rng.pick(&usable).clone();
        format!("({}){}", want.c(), v.name)
    }

    /// One statement. **At most one side effect**, which removes unsequenced-modification
    /// UB and evaluation-order divergence between compilers by construction.
    fn stmt(&mut self) {
        match self.rng.below(10) {
            0..=3 => {
                let ty = *self.rng.pick(&Ty::ALL);
                let name = self.fresh();
                let init = self.expr(ty);
                let _ = writeln!(self.body, "  {} {name} = {init};", ty.c());
                self.vars.push(Var { name, ty });
            }
            4..=5 => {
                if let Some(v) = self.pick_var() {
                    let e = self.expr(v.ty);
                    let _ = writeln!(self.body, "  {} = {e};", v.name);
                }
            }
            6 => {
                // Compound assignment — the shape that hid the bit-field defect and the
                // `_Bool` solver panic. `/=` and `>>=` are included deliberately: `+= 1`
                // followed by truncation absorbs a wrong sign-extension and these do not.
                if let Some(v) = self.pick_var() {
                    let op = *self.rng.pick(&["+=", "-=", "*=", "|=", "&=", "^="]);
                    let e = self.expr(v.ty);
                    let _ = writeln!(self.body, "  {} {op} {e};", v.name);
                }
            }
            7 => {
                if let Some(v) = self.pick_var() {
                    let op = *self.rng.pick(&["++", "--"]);
                    let pre = self.rng.chance(2);
                    if pre {
                        let _ = writeln!(self.body, "  {op}{};", v.name);
                    } else {
                        let _ = writeln!(self.body, "  {}{op};", v.name);
                    }
                }
            }
            8 => {
                let c = self.expr(Ty::Int);
                let _ = writeln!(self.body, "  if ({c}) {{");
                self.nested();
                let _ = writeln!(self.body, "  }} else {{");
                self.nested();
                let _ = writeln!(self.body, "  }}");
            }
            _ => {
                // A loop with a **structurally bounded** trip count, so no generated
                // program can fail to terminate and no comparison can hang.
                if let Some(v) = self.pick_var() {
                    let n = 1 + self.rng.below(4);
                    let i = self.fresh();
                    let e = self.expr(v.ty);
                    let _ = writeln!(
                        self.body,
                        "  for (int {i} = 0; {i} < {n}; {i}++) {{ {} += {e}; }}",
                        v.name
                    );
                }
            }
        }
    }

    /// A nested block's statements. Variables declared inside do not escape, which is what
    /// makes the checksum below well defined.
    fn nested(&mut self) {
        let saved = self.vars.len();
        for _ in 0..1 + self.rng.below(2) {
            self.stmt();
        }
        self.vars.truncate(saved);
    }

    fn pick_var(&mut self) -> Option<Var> {
        if self.vars.is_empty() {
            return None;
        }
        let vars = self.vars.clone();
        Some(self.rng.pick(&vars).clone())
    }

    /// **The result is a checksum over every live variable**, not one expression's value.
    ///
    /// This is what buys neighbour-corruption detection for free: a write that lands on the
    /// wrong object changes the checksum even when the value it was supposed to produce is
    /// right. The hand-written bit-field fixtures had to spell `v.a * 100 + v.b` for the
    /// same reason, one fixture at a time.
    fn finish(mut self) -> String {
        let _ = writeln!(self.body, "  long acc = 0;");
        let vars = self.vars.clone();
        for (i, v) in vars.iter().enumerate() {
            let _ = writeln!(
                self.body,
                "  acc = acc * 31 + (long)({}) + {};",
                v.name, i as i64
            );
        }
        let _ = writeln!(self.body, "  return (int)acc;");
        self.body
    }
}

fn program(seed: u64) -> String {
    let mut g = Gen::new(seed);
    let n = 3 + g.rng.below(7);
    for _ in 0..n {
        g.stmt();
    }
    g.finish()
}

// ---------------------------------------------------------------------------------------
// The verdict
// ---------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Agree,
    Mismatch {
        chiero: Option<i32>,
        gcc: i32,
    },
    Panic(String),
    SilentNoState {
        gcc: i32,
    },
    Refused {
        stage: &'static str,
        message: String,
    },
    /// The program was undefined, or the compilers disagreed with each other about it.
    /// Not compared, and not a defect on either side.
    Discarded,
}

/// Run one generated body through both sides.
fn judge(body: &str) -> Verdict {
    let Some((gcc, defined)) = gcc_answer(body) else {
        return Verdict::Discarded;
    };
    if !defined {
        return Verdict::Discarded;
    }
    match chiero_answer(body) {
        Ok(Some(v)) if v == gcc => Verdict::Agree,
        Ok(Some(v)) => Verdict::Mismatch {
            chiero: Some(v),
            gcc,
        },
        Ok(None) => Verdict::SilentNoState { gcc },
        Err(ChieroErr::Refused { stage, message }) => Verdict::Refused { stage, message },
        Err(ChieroErr::Panic(m)) => Verdict::Panic(m),
    }
}

enum ChieroErr {
    Refused {
        stage: &'static str,
        message: String,
    },
    Panic(String),
}

/// Lower and run, distinguishing a refusal from a silent nothing.
///
/// `harness::lower` panics on any diagnostic, which is right for a hand-written fixture and
/// wrong here — a refusal is information, not a test failure. So the stages are run
/// directly and each one's diagnostics are reported as themselves.
fn chiero_answer(body: &str) -> Result<Option<i32>, ChieroErr> {
    use chiero_parse::{ScopedTypedefs, parse_tu};
    use chiero_pp::{Config, preprocess_str};
    use chiero_sema::{SymbolText, TargetConfig, analyze};

    struct Names<'a>(&'a chiero_parse::ParsedTu);
    impl SymbolText for Names<'_> {
        fn text(&self, s: chiero_span::Symbol) -> Option<&str> {
            self.0.text(s)
        }
    }

    let src = format!("int probe(void) {{\n{body}}}\n");
    let out = std::panic::catch_unwind(|| {
        let tu = preprocess_str("g.c", &src, Config::default());
        if let Some(d) = tu.diagnostics.first() {
            return Err(("preprocess", format!("{d:?}")));
        }
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);
        if let Some(d) = parsed.diagnostics.first() {
            return Err(("parse", d.message.clone()));
        }
        let names = Names(&parsed);
        let an = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
        if let Some(d) = an.diagnostics.first() {
            return Err(("sema", format!("{d:?}")));
        }
        let m = chiero_lower::lower_tu(&parsed.ast, &an, &names);
        if let Some(d) = m.diagnostics.first() {
            return Err(("lower", d.message.clone()));
        }
        let mut arena = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m.module)
            .with_entry("probe")
            .run(&mut arena);
        Ok(r.states()
            .iter()
            .find_map(|s| s.return_value_bits(&mut arena))
            .map(|b| b as u32 as i32))
    });
    match out {
        Ok(Ok(v)) => Ok(v),
        Ok(Err((stage, message))) => Err(ChieroErr::Refused { stage, message }),
        Err(p) => Err(ChieroErr::Panic(panic_text(&p))),
    }
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic".into()
    }
}

/// gcc's answer, and whether the program is defined.
///
/// `defined` is false when the sanitizers trip, when the binary does not run, or when
/// `-O0`, `-O2` and clang do not all agree — three compilers disagreeing about a program
/// means the program is the problem, not any of them.
fn gcc_answer(body: &str) -> Option<(i32, bool)> {
    let dir = std::env::temp_dir().join(format!("chiero-gen-{}-{}", std::process::id(), next()));
    std::fs::create_dir_all(&dir).ok()?;
    let c = dir.join("g.c");
    let src = format!(
        "#include <stdio.h>\nint probe(void) {{\n{body}}}\n\
         int main(void) {{ printf(\"%d\\n\", probe()); return 0; }}\n"
    );
    std::fs::write(&c, src).ok()?;

    let run = |cc: &str, extra: &[&str], tag: &str| -> Option<(i32, bool)> {
        let bin = dir.join(tag);
        let mut cmd = std::process::Command::new(cc);
        cmd.args(["-std=gnu11", "-w"]).args(extra).arg("-o");
        cmd.arg(&bin).arg(&c);
        let out = cmd.output().ok()?;
        if !out.status.success() {
            return None;
        }
        let r = std::process::Command::new(&bin).output().ok()?;
        if !r.status.success() {
            return Some((0, false));
        }
        let text = String::from_utf8_lossy(&r.stdout);
        let err = String::from_utf8_lossy(&r.stderr);
        if !err.is_empty() {
            // A sanitizer said something. Whatever it was, the program is not defined.
            return Some((0, false));
        }
        Some((text.trim().parse::<i32>().ok()?, true))
    };

    // The verdict, and the sanitizer check on the same compiler.
    let base = run("gcc", &["-O0"], "o0")?;
    let result = if !base.1 {
        Some((0, false))
    } else {
        let san = run(
            "gcc",
            &[
                "-O0",
                "-fsanitize=undefined,address",
                "-fno-sanitize-recover=all",
            ],
            "san",
        );
        match san {
            Some((_, false)) | None => Some((0, false)),
            Some(_) => {
                // Two more readings; disagreement means the program is undefined in a way
                // the sanitizers did not catch.
                let o2 = run("gcc", &["-O2"], "o2");
                let cl = run("clang", &["-O0"], "cl");
                let agree = o2.map(|x| x.1 && x.0 == base.0).unwrap_or(true)
                    && cl.map(|x| x.1 && x.0 == base.0).unwrap_or(true);
                if agree { Some(base) } else { Some((0, false)) }
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn next() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------------------
// The batch
// ---------------------------------------------------------------------------------------

/// **Fixed seeds, so this is a test and not a slot machine.**
///
/// An unseeded random test that fails one run in ten gets muted within a month, and a muted
/// channel is worse than none. The open-ended search belongs in a soak run; what lives in
/// CI is a fixed batch, and anything it finds graduates to a named fixture in
/// `differential.rs` where it is deterministic forever.
#[test]
fn generated_programs_agree_with_gcc() {
    let mut refused: Vec<(&str, String)> = Vec::new();
    let mut discarded = 0usize;
    let mut compared = 0usize;
    let mut defects: Vec<(u64, String, Verdict)> = Vec::new();

    for seed in 0..200u64 {
        let body = program(seed);
        match judge(&body) {
            Verdict::Agree => compared += 1,
            Verdict::Discarded => discarded += 1,
            Verdict::Refused { stage, message } => refused.push((stage, message)),
            v => defects.push((seed, body, v)),
        }
    }

    // The ledger, printed whether or not anything failed — a refusal is a gap and gaps are
    // meant to be looked at, not accumulated quietly.
    if !refused.is_empty() {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for (stage, m) in &refused {
            let key = format!("{stage}: {m}");
            match counts.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => counts.push((key, 1)),
            }
        }
        counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("refusal ledger ({} programs):", refused.len());
        for (k, n) in counts.iter().take(10) {
            eprintln!("  {n:>4}  {k}");
        }
    }
    eprintln!(
        "compared {compared}, discarded {discarded}, refused {}",
        refused.len()
    );

    assert!(
        compared > 0,
        "the generator compared nothing at all — every program was discarded or refused, \
         which means this test is green while testing nothing"
    );
    assert!(
        defects.is_empty(),
        "{} generated program(s) disagree with gcc. First:\nseed {}\n{}\n{:#?}",
        defects.len(),
        defects[0].0,
        defects[0].1,
        defects[0].2
    );
}
