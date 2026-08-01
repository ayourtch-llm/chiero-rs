//! **A generated channel for sema's diagnostics** (014 §7, and wave 323's finding).
//!
//! Every differential channel in this project compares *answers on programs gcc accepts*. That
//! leaves every diagnostic-side rule — sixteen constraint-census rows, the initializer rules, the
//! conversion rules, the `switch` rules — outside all of them, held only by its own hand-written
//! fixture and the twenty-header corpus gate. Waves 303, 307, 311 and 313 each found sema
//! rejecting a *correct* program, and each time the finding came from someone thinking of a legal
//! shape by hand.
//!
//! This generates them instead. The invariant is one line: **a program gcc accepts must produce no
//! sema diagnostics.** Anything else is a false positive, which is the failure mode those four
//! waves showed is both the most damaging and the least likely to be noticed — a wrong rejection
//! tells a reader their correct program is broken, and no test that checks *answers* can see it,
//! because lowering runs anyway.
//!
//! **gcc is the arbiter of what counts as generated, not the generator.** A shape the generator
//! believes legal but gcc rejects is a bug in the generator, so those programs are skipped and
//! counted rather than reported — otherwise every gap in my own C would arrive as an engine
//! finding. `-pedantic-errors` is the setting, because wave 314 established that half of C's
//! constraint violations are warnings by default and a channel calibrated to the default would
//! call them legal.

mod harness;

use chiero_sema::TargetConfig;

/// The xorshift the other generators in this project use, so seeds behave the same way here.
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
}

/// A type spelling and an expression of that type, kept together so the two never disagree.
///
/// Pairing them is what makes the generator produce *legal* programs by construction rather than
/// by luck: the alternative — pick a type, pick an expression, hope — spends most of its output on
/// programs gcc rejects, and a channel that skips 90% of what it generates is measuring the
/// generator.
fn typed_value(rng: &mut Rng) -> (&'static str, &'static str) {
    [
        ("int", "0"),
        ("int", "1 + 2"),
        ("int", "'a'"),
        ("unsigned", "1u"),
        ("long", "1L"),
        ("char", "'x'"),
        ("short", "(short)1"),
        ("double", "1.5"),
        ("float", "1.5f"),
        ("int *", "0"),
        ("void *", "0"),
        ("const char *", "\"s\""),
        ("_Bool", "1"),
    ][rng.below(13)]
}

/// A statement that is legal wherever it is placed, using only `p` (an `int`) if it needs a name.
fn statement(rng: &mut Rng, depth: u32) -> String {
    match rng.below(if depth == 0 { 8 } else { 14 }) {
        0 => "p = p + 1;".into(),
        1 => "p++;".into(),
        2 => "if (p) p = 2;".into(),
        3 => "if (p) p = 2; else p = 3;".into(),
        4 => format!("{{ {} q = {}; (void)q; }}", "int", rng.below(9)),
        5 => "p = p ? 1 : 2;".into(),
        6 => ";".into(),
        7 => "(void)p;".into(),
        8 => format!("while (p > {}) p--;", rng.below(3)),
        9 => format!("for (int i = 0; i < {}; i++) p += i;", rng.below(4)),
        10 => format!(
            "switch (p) {{ case {}: p = 1; break; default: p = 0; }}",
            rng.below(4)
        ),
        11 => format!("do {{ p--; }} while (p > {});", rng.below(2)),
        12 => format!("{{ {} }}", statement(rng, depth - 1)),
        _ => format!("if (p) {{ {} }}", statement(rng, depth - 1)),
    }
}

/// A file-scope declaration, with its own initializer where it takes one.
fn declaration(rng: &mut Rng, n: usize) -> String {
    let (ty, val) = typed_value(rng);
    match rng.below(9) {
        0 => format!("{ty} g{n} = {val};"),
        1 => format!("static {ty} g{n} = {val};"),
        2 => format!("static {ty} g{n};"),
        3 => format!("extern {ty} g{n};"),
        4 => format!("{ty} a{n}[3] = {{ {val} }};"),
        5 => format!("{ty} a{n}[] = {{ {val} }};"),
        6 => format!("struct S{n} {{ {ty} m; int k; }}; struct S{n} s{n} = {{ {val}, 1 }};"),
        7 => format!("union U{n} {{ {ty} m; int k; }}; union U{n} u{n} = {{ {val} }};"),
        _ => format!("typedef {ty} T{n}; static T{n} t{n} = {val};"),
    }
}

/// A legal shape from the neighbourhood where sema has historically rejected correct programs.
///
/// **Aimed, not random.** A generator that only emits unadventurous C produces silence and proves
/// nothing: twelve hundred such programs found no false positive. Every entry here is drawn from
/// a rejection this project actually shipped — reusing a local name across functions (wave 307),
/// `__func__` (309), `return v()` from a void function and a block shadowing a `const` (311),
/// `extern` after `static` (313), `_Bool b = p` and an array parameter (315), the two `const`s
/// (316), an inferred array length (321) and a `static` local (322). Those are where the rules
/// are dense enough to catch a legal program by mistake.
fn historically_awkward(rng: &mut Rng, n: usize) -> String {
    match rng.below(16) {
        // Wave 307: the same local name in two functions is not a redefinition.
        0 => format!(
            "static int f{n}a(void){{ int v = 1; return v; }}\nstatic int f{n}b(void){{ int v = 2; return v; }}"
        ),
        // Wave 311: a block may shadow, including shadowing a `const` with a mutable object.
        // The inner `k` is **written**, not only read: the shadowing rule guards writes, so a
        // shape that reads it exercises nothing. Re-injecting wave 311's defect proved that —
        // the read-only version passed with the bug restored.
        1 => {
            format!("static int f{n}(void){{ const int k = 1; {{ int k = 2; k = 3; return k; }} }}")
        }
        2 => format!(
            "static int f{n}(void){{ int i = 0; for (int i = 0; i < 2; i++) {{}} return i; }}"
        ),
        // Wave 311: a void call is a statement, and returning one from a void function is legal.
        // A void call as a statement and discarded explicitly. **Not** `return v();` from a void
        // function: gcc accepts that by default and rejects it under `-pedantic-errors`
        // (C11 6.8.6.4), so it is legal by the setting wave 311's fixture pins and illegal by the
        // one this channel uses. Generating it would spend a fifth of the output on a
        // disagreement between two gcc settings rather than on anything about the engine.
        3 => format!(
            "static void v{n}(void){{}}\nstatic int f{n}(void){{ v{n}(); (void)v{n}(); return 0; }}"
        ),
        // Wave 313: `extern` adopts linkage, and a prototype may precede a definition.
        4 => format!(
            "static int g{n}; extern int g{n};\nstatic int f{n}(void);\nstatic int f{n}(void){{ return g{n}; }}"
        ),
        5 => format!("int h{n}();\nint h{n}(int x){{ return x; }}"),
        // Waves 315-316: pointer conversions that are legal, and the two `const`s.
        6 => format!("static int f{n}(int *p){{ _Bool b = p; return b; }}"),
        7 => format!("static int f{n}(void *v){{ int *q = v; return q != 0; }}"),
        8 => format!("static int f{n}(int a{n}[2][3]){{ return a{n}[1][2]; }}"),
        9 => format!("static int f{n}(void){{ int x = 0; int *const p = &x; *p = 1; return x; }}"),
        10 => format!("static int f{n}(const int *p){{ const int *q = p; return *q; }}"),
        // Wave 321: an array takes its length from its initializer, in both spellings.
        11 => format!(
            "static char s{n}[] = \"hi\"; static int a{n}[] = {{1,2,3}};\nstatic int f{n}(void){{ return s{n}[0] + a{n}[2] + (int)sizeof(s{n}); }}"
        ),
        // Wave 322: a `static` local, including one shadowing a file-scope name.
        12 => format!(
            "static int c{n} = 9;\nstatic int f{n}(void){{ static int c{n} = 1; c{n}++; return c{n}; }}"
        ),
        // Wave 327: legal jumps around a variably-modified declaration — from *after* it, and
        // into a block that declares an ordinary array. Both are shapes the VLA-scope rule can
        // reject by mistake if it is approximated as "jumping into a block".
        13 => format!("static int f{n}(int m){{ int a{n}[m]; goto skip; skip: return a{n}[0]; }}"),
        14 => format!(
            "static int f{n}(int m){{ if (m) goto skip; {{ int b{n}[2]; b{n}[0] = 1; skip: return m; }} }}"
        ),
        // Wave 309: `__func__` is declared by the language.
        _ => format!("static int f{n}(void){{ return (int)sizeof(__func__) + __func__[0]; }}"),
    }
}

/// One generated translation unit: a few declarations, an awkward shape or two, then a function.
fn program(seed: u64) -> String {
    let rng = &mut Rng::new(seed);
    let mut src = String::new();
    for n in 0..rng.below(4) {
        src.push_str(&declaration(rng, n));
        src.push('\n');
    }
    for n in 0..1 + rng.below(3) {
        src.push_str(&historically_awkward(rng, 100 + n));
        src.push('\n');
    }
    src.push_str("int probe(int p) {\n");
    for _ in 0..1 + rng.below(4) {
        src.push_str("    ");
        src.push_str(&statement(rng, 1));
        src.push('\n');
    }
    src.push_str("    return p;\n}\n");
    src
}

/// Whether gcc accepts this program under `-pedantic-errors`.
fn gcc_accepts(src: &str) -> Option<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("gcc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-c",
            "-o",
            "/dev/null",
            "-x",
            "c",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(src.as_bytes()).ok()?;
    Some(child.wait().ok()?.success())
}

/// **A program gcc accepts produces no sema diagnostics.**
///
/// The count of skipped programs is asserted too, and low: if the generator drifts into producing
/// mostly-invalid C, this test would keep passing while measuring almost nothing, which is the
/// vacuity 014 §7 warns about and which this project has had to fix more than once.
#[test]
fn every_program_gcc_accepts_is_silent() {
    let count: u64 = std::env::var("CHIERO_SILENCE_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let base: u64 = std::env::var("CHIERO_SILENCE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    if gcc_accepts("int main(void){return 0;}") != Some(true) {
        eprintln!("skipping: gcc not usable here");
        return;
    }

    let mut checked = 0u64;
    let mut skipped = 0u64;
    let mut complaints = Vec::new();
    for seed in base..base + count {
        let src = program(seed);
        match gcc_accepts(&src) {
            Some(true) => {}
            // **A program gcc rejects says nothing about the engine.** It is a hole in this
            // generator's idea of C, counted so the assertion below can notice if it becomes the
            // common case.
            _ => {
                skipped += 1;
                continue;
            }
        }
        checked += 1;
        let p = harness::parse_allowing_diagnostics(&src, TargetConfig::x86_64_linux());
        if !p.analysis.diagnostics.is_empty() && complaints.len() < 5 {
            complaints.push(format!(
                "seed {seed}: {:?}\n----\n{src}----",
                p.analysis
                    .diagnostics
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
            ));
        }
    }

    eprintln!(
        "checked {checked}, skipped {skipped}, complaints {}",
        complaints.len()
    );
    assert!(
        complaints.is_empty(),
        "sema complained about {} program(s) gcc accepts:\n{}",
        complaints.len(),
        complaints.join("\n")
    );
    assert!(
        checked * 2 > count,
        "only {checked} of {count} programs were legal C ({skipped} skipped); \
         the generator has drifted and this test is measuring almost nothing"
    );
}
