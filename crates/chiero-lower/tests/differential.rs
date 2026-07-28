//! Covers: 015 contract 5.
//!
//! **The oracle.** Every other lowering test asserts *shape* — how many blocks, which
//! edges, what order — and wave 91 hit the ceiling of that: two mutations survived the
//! whole suite (ignoring signedness, always zero-extending) because they change no shape
//! at all, only the numbers the program computes. No structural assertion can ever catch
//! them.
//!
//! So this file compares against the compiler. A fixture is lowered to CIR and executed by
//! `chiero-exec`; the same C is compiled by gcc and run; the two answers must agree. That
//! is what 015 contract 5 asks for by name, and it turns every case below into a claim
//! about semantics rather than about structure.
//!
//! The fixture shape — `int probe(void)` with no parameters — is what makes it work
//! without any symbolic-input machinery: both sides produce one concrete integer.

mod harness;

use chiero_solver::TermArena;

/// Run `body` through chiero and through gcc, and require the same answer.
fn agree(body: &str) {
    let Some(expected) = gcc_answer(body) else {
        eprintln!("skipping `{body}`: gcc not available (015 contract 5)");
        return;
    };
    let got = chiero_answer(body);
    assert_eq!(
        got,
        Some(expected),
        "`int probe(void) {{ {body} }}`: chiero says {got:?}, gcc says {expected}"
    );
}

/// Lower, execute, and read the returned value as a 32-bit signed integer.
fn chiero_answer(body: &str) -> Option<i32> {
    let src = format!("int probe(void) {{ {body} }}");
    let m = harness::lower(&src);
    let mut arena = TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut arena);
    // A concrete function has one path; take the first state that actually returned.
    let bits = r
        .states()
        .iter()
        .find_map(|s| s.return_value_bits(&mut arena))?;
    Some(bits as u32 as i32)
}

fn gcc_answer(body: &str) -> Option<i32> {
    let dir =
        std::env::temp_dir().join(format!("chiero-diff-{}-{}", std::process::id(), next_seq()));
    std::fs::create_dir_all(&dir).ok()?;
    let c = dir.join("p.c");
    let bin = dir.join("p");
    std::fs::write(
        &c,
        format!("#include <stdio.h>\nint probe(void) {{ {body} }}\nint main(void) {{ printf(\"%d\\n\", probe()); return 0; }}\n"),
    )
    .ok()?;
    let out = std::process::Command::new("gcc")
        .args(["-std=gnu11", "-w", "-O0", "-o"])
        .arg(&bin)
        .arg(&c)
        .output()
        .ok()?;
    if !out.status.success() {
        panic!(
            "gcc rejected the fixture `{body}`:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let text = String::from_utf8_lossy(&run.stdout);
    let v = text.trim().parse::<i32>().ok();
    let _ = std::fs::remove_dir_all(&dir);
    v
}

fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// **Signed versus unsigned division and remainder.**
///
/// C has one `/` and CIR has `SDiv` and `UDiv`; the operand types decide which. Getting it
/// wrong is a wrong answer for exactly half the inputs and changes no shape, which is why
/// a mutation forcing `SDiv` survived wave 91's entire suite.
#[test]
fn division_and_remainder_follow_operand_signedness() {
    agree("int a = -7; return a / 2;");
    agree("int a = -7; return a % 2;");
    agree("unsigned a = 4294967288u; return (int)(a / 2u);");
    agree("unsigned a = 4294967288u; return (int)(a % 3u);");
    // The pair that makes it a *test* of signedness rather than of division: the same
    // bit pattern, read signed and unsigned, must give different answers.
    agree("int a = -8; unsigned b = 4294967288u; return (a / 2) == (int)(b / 2u);");
}

/// **Sign extension.**
///
/// Widening a `signed char` holding -1 with `ZExt` produces 255 — a perfectly legal value
/// of the wider type, so the verifier, the solver and every shape test accept it. Only the
/// number is wrong.
#[test]
fn narrow_types_widen_with_the_right_extension() {
    agree("signed char c = -1; int i = c; return i;");
    agree("unsigned char c = 255; int i = c; return i;");
    agree("short s = -300; int i = s; return i;");
    agree("unsigned short s = 65535; int i = s; return i;");
    // And the discriminating pair: the same byte, signed and unsigned, must differ.
    agree("signed char a = -1; unsigned char b = 255; return (int)a == (int)b;");
}

/// **Shifts.** `>>` is arithmetic on a signed operand and logical on an unsigned one, and
/// the result type is the *left* operand's — which wave 89 had to fix in sema and nothing
/// downstream had verified.
#[test]
fn shifts_follow_the_left_operands_signedness() {
    agree("int a = -8; return a >> 1;");
    agree("unsigned a = 2147483648u; return (int)(a >> 31);");
    agree("int a = 1; return a << 4;");
    agree("unsigned a = 4294967295u; return (int)(a >> 28);");
}

/// **Comparisons**, including the C wart that decides them: in `a > b` with `a` an `int`
/// and `b` an `unsigned`, the `int` converts to unsigned, so `-1 > 1u` is **true**.
///
/// This one exercises the whole chain at once — 014's usual arithmetic conversions, the
/// explicit `Cast` they produce, and lowering's choice of `UGt` over `SGt`.
#[test]
fn comparisons_follow_the_converted_operand_types() {
    agree("int a = -1; unsigned b = 1u; return a > b;");
    agree("int a = -1; int b = 1; return a > b;");
    agree("int a = -1; return a < 0;");
    agree("unsigned a = 4294967295u; return a > 1u;");
    agree("int a = 3; int b = 3; return (a <= b) + (a >= b) + (a == b) + (a != b);");
}

/// **Short-circuit evaluation**, checked for its *value* rather than its block count.
#[test]
fn short_circuit_results_match_the_compiler() {
    agree("int a = 0; int b = 5; return a && b;");
    agree("int a = 3; int b = 0; return a && b;");
    agree("int a = 0; int b = 0; return a || b;");
    agree("int a = 0; int b = 7; return a || b;");
    agree("int a = 2; int b = 3; return (a && b) + (a || b);");
    // `!` is `== 0`, and its result is an `int` — so it composes with arithmetic.
    agree("int a = 0; return !a + !!a;");
}

/// **Control flow and compound assignment**, end to end.
#[test]
fn loops_and_compound_assignment_compute_what_gcc_computes() {
    agree("int t = 0; for (int i = 0; i < 5; i++) { t += i; } return t;");
    agree("int n = 5; int t = 1; while (n > 0) { t *= n; n--; } return t;");
    agree("int n = 3; int t = 0; do { t += n; n--; } while (n > 0); return t;");
    agree("int x = 10; x -= 3; x *= 2; x /= 7; return x;");
    agree("int a = 6; int b = 3; if (a > b) { return a - b; } else { return b - a; }");
}

/// **Contract 5's own case**: `x++` yields the pre-value and `++x` the post-value.
///
/// Every earlier fixture used `i++` as a `for` step, where the *value* is discarded — so a
/// mutation making `x++` yield the new value survived the whole suite. The result has to
/// be consumed for the distinction to exist at all, and the two forms must give different
/// answers, or the fixture is testing increment rather than which value it produces.
#[test]
fn increment_yields_the_pre_or_post_value_as_written() {
    agree("int i = 5; int a = i++; return a * 10 + i;");
    agree("int i = 5; int a = ++i; return a * 10 + i;");
    agree("int i = 5; int a = i--; return a * 10 + i;");
    agree("int i = 5; int a = --i; return a * 10 + i;");

    let post = chiero_answer("int i = 5; int a = i++; return a * 10 + i;");
    let pre = chiero_answer("int i = 5; int a = ++i; return a * 10 + i;");
    assert_eq!(post, Some(56), "`i++` hands back 5 and leaves 6");
    assert_eq!(pre, Some(66), "`++i` hands back 6 and leaves 6");
    assert_ne!(
        post, pre,
        "the two forms differ, or the fixture is about incrementing rather than about \
         which value the expression produces"
    );
}

/// **The control flow this wave adds**, checked for what it computes.
///
/// §9's standing instruction: a shape assertion says the blocks are arranged correctly,
/// and only the oracle says the program computes the right number. `switch` fallthrough
/// in particular is a shape a structural test confirms and a wrong `break` target ruins.
#[test]
fn switch_break_continue_and_goto_compute_what_gcc_computes() {
    agree(
        "int n = 2; int t = 0; switch (n) { case 1: t = 1; case 2: t += 2; break; default: t = 9; } return t;",
    );
    agree(
        "int n = 1; int t = 0; switch (n) { case 1: t = 1; case 2: t += 2; break; default: t = 9; } return t;",
    );
    agree("int n = 7; int t = 0; switch (n) { case 1: t = 1; break; default: t = 9; } return t;");
    agree("int n = 4; switch (n) { case 3 ... 6: return 1; default: return 0; }");
    agree("int n = 9; switch (n) { case 3 ... 6: return 1; default: return 0; }");
    agree("int t = 0; for (int i = 0; i < 10; i++) { if (i == 3) break; t += i; } return t;");
    agree("int t = 0; for (int i = 0; i < 6; i++) { if (i % 2) continue; t += i; } return t;");
    agree(
        "int t = 0; int i = 0; while (1) { i++; if (i > 4) break; if (i == 2) continue; t += i; } return t;",
    );
    agree("int t = 0; int i = 0; again: i++; t += i; if (i < 4) goto again; return t;");
    // A `goto` that leaves two scopes and lands after them.
    agree("int t = 1; { int a = 2; { int b = 3; t = a + b; goto out; } } out: return t;");
}

/// A guard that the oracle can **see a difference at all**.
///
/// Every assertion above is an equality, and a comparison that always compared equal
/// would pass all of them — the same vacuity that has cost this project a fixture in
/// several waves. So: two fixtures whose answers genuinely differ must be reported as
/// differing by the same machinery.
#[test]
fn the_oracle_can_observe_a_disagreement() {
    let a = chiero_answer("signed char c = -1; int i = c; return i;");
    let b = chiero_answer("unsigned char c = 255; int i = c; return i;");
    assert_eq!(a, Some(-1));
    assert_eq!(b, Some(255));
    assert_ne!(
        a, b,
        "the two extensions must give different answers, or this file's equalities \
         are comparing a constant against itself"
    );
    if let (Some(ga), Some(gb)) = (
        gcc_answer("signed char c = -1; int i = c; return i;"),
        gcc_answer("unsigned char c = 255; int i = c; return i;"),
    ) {
        assert_ne!(ga, gb, "and gcc agrees they differ");
    }
}
