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
    agree_with("", body);
}

/// The same, with `prelude` emitted at **file scope** before `probe`.
///
/// A body alone cannot declare a global or a second function, so every defect that only
/// shows up through file-scope storage — a pointer-typed global read as a value, a struct
/// passed by value to a helper — was outside this oracle's reach entirely.
fn agree_with(prelude: &str, body: &str) {
    let expected = match gcc_answer(prelude, body) {
        Ok(v) => v,
        // **The only excusable reason to compare nothing**, and it is announced. `eprintln!`
        // on a *passing* test is swallowed without `--nocapture`, so this branch also
        // records itself where a reader will actually see it, below.
        Err(Oracle::NoGcc) => {
            eprintln!("skipping `{prelude} / {body}`: gcc not on PATH (015 contract 5)");
            SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        // **Every other failure was previously spelled "gcc not available" too** — a
        // temp-directory error, a binary that would not run, output that would not parse.
        // Reporting a broken oracle as an absent one is how a file like this goes green for
        // months while comparing nothing.
        Err(Oracle::Broken(why)) => panic!("the oracle is broken, not absent: {why}"),
    };
    let got = chiero_answer(prelude, body);
    assert_eq!(
        got,
        Some(expected),
        "`{prelude} int probe(void) {{ {body} }}`: chiero says {got:?}, gcc says {expected}"
    );
}

/// Why the oracle produced no answer.
enum Oracle {
    /// gcc is not installed. The one case where skipping is honest.
    NoGcc,
    /// gcc is installed and something else went wrong — never a reason to pass.
    Broken(String),
}

static SKIPPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// **The announcement, where libtest cannot swallow it.**
///
/// A skipped comparison in a *passing* test prints nothing unless someone thought to pass
/// `--nocapture`, so "announce every skip" was satisfied only in principle. This test fails
/// if any fixture in the file skipped, which turns a silent no-op run into a red one.
///
/// It depends on running after the others, which `cargo test` does not promise — so it is
/// a backstop for a whole-file run, not a per-test guard. That is still the difference
/// between a suite that reports nothing and one that reports something.
#[test]
fn zz_the_oracle_actually_ran() {
    // Force one comparison from this test itself, so the counter is meaningful even when
    // this is the only test selected.
    agree("return 1;");
    let n = SKIPPED.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        n, 0,
        "{n} fixture(s) skipped: gcc is not on PATH, so this file compared nothing. \
         015 contract 5 needs the compiler; an oracle that can silently not run is not one."
    );
}

/// Lower, execute, and read the returned value as a 32-bit signed integer.
fn chiero_answer(prelude: &str, body: &str) -> Option<i32> {
    let src = format!("{prelude}\nint probe(void) {{ {body} }}");
    let m = harness::lower(&src);
    let mut arena = TermArena::new();
    // **`probe` by name**, not "the first function": a prelude declaring a helper puts
    // something else first, and the oracle would then compare the helper's answer.
    let r = chiero_exec::Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena);
    // A concrete function has one path; take the first state that actually returned.
    let bits = r
        .states()
        .iter()
        .find_map(|s| s.return_value_bits(&mut arena))?;
    Some(bits as u32 as i32)
}

fn gcc_answer(prelude: &str, body: &str) -> Result<i32, Oracle> {
    let dir =
        std::env::temp_dir().join(format!("chiero-diff-{}-{}", std::process::id(), next_seq()));
    std::fs::create_dir_all(&dir).map_err(|e| Oracle::Broken(format!("mkdir {dir:?}: {e}")))?;
    let c = dir.join("p.c");
    let bin = dir.join("p");
    std::fs::write(
        &c,
        format!("#include <stdio.h>\n{prelude}\nint probe(void) {{ {body} }}\nint main(void) {{ printf(\"%d\\n\", probe()); return 0; }}\n"),
    )
    .map_err(|e| Oracle::Broken(format!("write {c:?}: {e}")))?;
    let out = std::process::Command::new("gcc")
        .args(["-std=gnu11", "-w", "-O0", "-o"])
        .arg(&bin)
        .arg(&c)
        .output()
        // **Only a failure to spawn means gcc is missing.** Everything past this point is
        // a broken oracle, not an absent one.
        .map_err(|_| Oracle::NoGcc)?;
    if !out.status.success() {
        panic!(
            "gcc rejected the fixture `{prelude} / {body}`:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let run = std::process::Command::new(&bin)
        .output()
        .map_err(|e| Oracle::Broken(format!("running the fixture `{body}`: {e}")))?;
    let text = String::from_utf8_lossy(&run.stdout);
    let v = text.trim().parse::<i32>().map_err(|e| {
        Oracle::Broken(format!(
            "the fixture `{body}` printed {text:?}, which is not an integer: {e}"
        ))
    })?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(v)
}

fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// **Every binary operator's precedence class, and their left-associativity.**
///
/// 013's contracts say nothing about precedence — it is the kind of thing that looks
/// self-evidently right in a table and is checked by nobody. A mutation sweep over
/// `binop_of` found it: **demoting `<<` from its own class to the additive one survived the
/// whole suite**, so `1 << 2+3` would have parsed as `(1<<2)+3` and answered 7 where C says
/// 32. Nothing else in the workspace writes a shift beside an addition without parentheses.
///
/// Each case below is a *discriminator*, not an example: the parenthesised readings differ,
/// and both were run through gcc before being written down. `7 % 4 + 1` is the shape to
/// avoid — it gives 4 under either grouping, so a fixture built from it proves nothing;
/// `1 + 7 % 4` is the same operators arranged so the answers diverge.
///
/// The last case is associativity rather than precedence. `9 - 4 - 2` is 3 in C and 7 if
/// the parser recurses at `prec` instead of `prec + 1` — a one-character edit that changes
/// every non-commutative operator at once.
#[test]
fn every_operator_binds_at_its_own_precedence() {
    // Multiplicative over additive, both orders — `a + b * c` is the textbook case and
    // `a * b + c` groups identically under a wrong table, so only the first discriminates.
    agree("return 1 + 2 * 3;");
    agree("return 1 + 7 % 4;");
    agree("return 2 * 3 + 1;");
    // **Additive over shift.** The survivor. `<<` binds *looser* than `+`, which is the
    // one precedence relation in C that surprises people, and the reason
    // `1 << n + 1` is a bug magnet in real code.
    agree("return 1 << 2 + 3;");
    agree("return 16 >> 1 + 1;");
    // And shift binds tighter than relational but looser than multiplicative.
    agree("return 16 >> 1 * 2;");
    agree("return 1 << 2 < 8;");
    // Relational over equality. `3 < 4 == 1` is **not** a discriminator and was in the
    // first draft: with `<` and `==` in one class, left-associativity gives `(3<4)==1`
    // either way. The relational operator has to come *second* for the trees to differ —
    // `2 == 1 < 3` is `2 == (1<3)` = 0 in C and `(2==1) < 3` = 1 if they share a class.
    agree("return 3 < 4 == 1;");
    agree("return 2 == 1 < 3;");
    agree("return 2 != 3 > 1;");
    // **Equality over bitwise-and** — the classic `x & MASK == v` trap, which C reads as
    // `x & (MASK == v)`. The `&` has to come *first*: with the two in one class,
    // left-associativity makes `a == b & c` group as `(a==b) & c`, which is what C does
    // anyway, so only this order can tell them apart. Both operators need their own case,
    // since the table gives `==` and `!=` separate entries that a mutation can move apart.
    agree("return 6 & 2 == 2;");
    agree("return 6 & 2 != 3;");
    // `<=` and `>=` are separate entries too, and equality has to come first for the same
    // reason it had to come second above.
    agree("return 2 == 1 <= 0;");
    agree("return 2 == 1 >= 0;");
    // And the three bitwise levels in order: `&` over `^` over `|`.
    agree("return 6 ^ 3 & 2;");
    agree("return 1 | 6 ^ 3;");
    // `&&` over `||`. `0 || 1 && 0` is 0 under either grouping, so it is useless here;
    // `1 || 0 && 0` is 1 in C and 0 if `||` bound tighter.
    agree("return 1 || 0 && 0;");
    // **And the logical operators sit *below* the bitwise ones**, which is the half the
    // pair above cannot see: moving `&&` up past `|` leaves `1 || 0 && 0` at 1 either way.
    // C reads `1 | 0 && 0` as `(1|0) && 0` = 0; a `&&` that bound tighter than `|` gives
    // `1 | (0&&0)` = 1.
    agree("return 1 | 0 && 0;");
    agree("return 2 ^ 1 && 0;");
    // Additive over shift, with the shift on the *right* — this one is about `-` reaching
    // the shift class from *below*, and `8 - (4>>1)` is 6 where C says 2.
    agree("return 8 - 4 >> 1;");
    // **And with the shift first, which is the direction that catches `-` being demoted.**
    // `8 - 4 >> 1` groups as `(8-4)>>1` under *either* reading, so it survived a mutation
    // putting `-` in the shift class; `16 >> 2 - 1` is `16 >> (2-1)` = 8 in C and
    // `(16>>2) - 1` = 3 if they share one. `+` and `-` are separate table entries and a
    // mutation moves one without the other, so each needs its own case.
    agree("return 16 >> 2 - 1;");
    agree("return 1 << 3 - 1;");
    // **Left-associativity**, which no precedence number expresses.
    agree("return 9 - 4 - 2;");
    agree("return 64 / 4 / 2;");
    agree("return 17 % 7 % 3;");
    agree("return 64 >> 2 >> 1;");
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

    let post = chiero_answer("", "int i = 5; int a = i++; return a * 10 + i;");
    let pre = chiero_answer("", "int i = 5; int a = ++i; return a * 10 + i;");
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

/// **Aggregates and bit-fields**, checked for what they compute.
///
/// A `CopyMem` of the wrong size and a `StoreBits` at the wrong offset are both
/// structurally perfect, so §9's standing instruction applies with full force: the shape
/// tests say the right *kind* of instruction was emitted, and only gcc says the right bits
/// ended up in the right places.
#[test]
fn struct_copies_and_bitfields_compute_what_gcc_computes() {
    agree(
        // Fields set explicitly rather than with a braced initializer: aggregate
        // initializers are 015 contract 19 and are not lowered yet, so a probe using one
        // would be testing the gap instead of the copy.
        "struct S { int a; int b; }; struct S x; x.a = 1; x.b = 2; struct S y; y = x; \
         return y.a * 10 + y.b;",
    );
    agree("struct B { int a:3; int b:5; }; struct B v; v.a = 3; v.b = 9; return v.a * 100 + v.b;");
    // Signedness of a narrow bit-field: `int a:3` holds -4..3, so 5 reads back as -3.
    agree("struct B { int a:3; }; struct B v; v.a = 5; return v.a;");
    agree("struct B { unsigned a:3; }; struct B v; v.a = 5; return (int)v.a;");
    // A bit-field straddling a byte, and one after a plain member.
    agree("struct B { char c; int a:20; }; struct B v; v.c = 1; v.a = 12345; return v.a + v.c;");
    agree("struct B { int a:30; int b:6; }; struct B v; v.a = 7; v.b = 3; return v.a * 10 + v.b;");
    // Ordinary members, so the byte-offset path is checked too.
    agree(
        "struct P { int a; char b; int c; }; struct P v; v.a = 1; v.b = 2; v.c = 3; \
         return v.a + v.b + v.c;",
    );
    // **Array indexing**, which scales the index by the element size. A mutation dropping
    // the scale survived every test above, because nothing here indexed anything —
    // `a[2]` would read byte 2 of the array instead of element 2, and the shape is
    // identical either way.
    agree("int a[4]; a[0] = 1; a[1] = 2; a[2] = 3; return a[1] * 10 + a[2];");
    agree("char a[4]; a[0] = 1; a[3] = 9; return a[0] * 10 + a[3];");
    agree("int a[3]; int i = 2; a[i] = 7; return a[2];");
    agree(
        "struct S { int v[3]; }; struct S s; s.v[0] = 4; s.v[2] = 6; return s.v[0] * 10 + s.v[2];",
    );
}

/// **A read-modify-write on a `_Bool` converts, it does not truncate.**
///
/// **Found by the generator in `generated.rs`, on its first run**, and shrunk to this line.
/// No hand-written fixture had it: `differential.rs` tests `b += 1` from 0, which fits, and
/// `b -= 1` from 1, which fits. The boundary — a `_Bool` already holding 1 — is exactly the
/// case nobody thought to spell, which is the whole reason that file exists.
///
/// C11 6.5.2.4 and 6.5.16.2 promote the operand to `int`, do the arithmetic there, and
/// convert the result *back* — and conversion to `_Bool` is `!= 0` (6.3.1.2), not a
/// narrowing. So `b++` on a true `_Bool` leaves it true. Doing the addition at the lvalue's
/// own one-bit width wraps 1 + 1 to 0 and turns it false.
///
/// This is wave 136's bit-field rule in its other instance: the arithmetic happens at the
/// promoted width and the store converts back. `_Bool` is the one scalar where the
/// conversion is a comparison rather than a truncation, which is why it needs saying twice.
#[test]
fn a_bool_read_modify_write_converts_rather_than_truncates() {
    // The generator's find, both spellings.
    agree("_Bool b = 1; b++; return b;");
    agree("_Bool b = 1; b += 1; return b;");
    // Adding more than one, so a fix that special-cases the increment is not enough.
    agree("_Bool b = 1; b += 3; return b;");
    agree("_Bool b = 0; b += 2; return b;");
    // `e--` from 0 gives 1 in C *and* by accident under the wrapping reading, so it is kept
    // as the case that must not regress rather than as a discriminator.
    agree("_Bool b = 0; b--; return b;");
    agree("_Bool b = 1; b -= 1; return b;");
    // The **value the expression yields**, which is the field after conversion for prefix
    // and before it for postfix.
    agree("_Bool b = 1; int r = b++; return r * 10 + b;");
    agree("_Bool b = 1; int r = ++b; return r * 10 + b;");
    agree("_Bool b = 1; int r = (b += 1); return r * 10 + b;");
    // Multiplication and the bitwise operators reach the same path.
    agree("_Bool b = 1; b *= 2; return b;");
    agree("_Bool b = 1; b |= 2; return b;");
    agree("_Bool b = 1; b ^= 1; return b;");
}

/// **A statement expression yielding an aggregate outlives its own scope.**
///
/// The last open defect on §9's list, carried since wave 132's implementation review.
/// `({ struct S t; …; t; })` records `t`'s *address* as the construct's value, and the
/// address is all an aggregate expression can be (020 §1.4) — but by the time the enclosing
/// initializer copies from it, the block has ended and 021 has retired the object. The
/// `CopyMem` reads bytes that are gone, and the program produces no state.
///
/// The scalar form has always worked, because a scalar's value is a *value*: `({ int t = 3;
/// t + 1; })` hands back the number, which no scope exit can invalidate. That pairing is
/// why this survived — the construct looks implemented, and is, for half the types.
///
/// The fix has to copy **before** the scope exits, which means the destination has to be
/// allocated outside it. Same shape as wave 138's compound literal: an unnamed object with
/// the enclosing block's lifetime.
#[test]
fn a_statement_expression_yielding_an_aggregate_outlives_its_scope() {
    const S: &str = "struct S { int a; int b; };\n";
    // The recorded case: a local declared, filled and yielded.
    agree_with(
        S,
        "struct S y = ({ struct S t; t.a = 4; t.b = 2; t; }); return y.a * 10 + y.b;",
    );
    // Yielded after being *assigned*, so the value is not simply the initializer.
    agree_with(
        S,
        "struct S z = ({ struct S u = {1, 2}; u.a = 7; u; }); return z.a * 10 + z.b;",
    );
    // Yielded straight from its initializer.
    agree_with(
        S,
        "struct S w = ({ struct S p = {8, 9}; p; }); return w.a * 10 + w.b;",
    );
    // A member selected off the construct, with no named destination at all.
    agree_with(S, "return ({ struct S q; q.a = 5; q.b = 6; q; }).b;");
    // Passed by value to a helper, which is where VPP writes this idiom.
    agree_with(
        "struct S { int a; int b; };\nstatic int sum(struct S p) { return p.a * 10 + p.b; }\n",
        "return sum(({ struct S t; t.a = 3; t.b = 4; t; }));",
    );
    // **Two of them in one expression**, so a single hoisted scratch slot would collapse
    // them and every case above would still pass.
    agree_with(
        S,
        "struct S x = ({ struct S t; t.a = 1; t.b = 2; t; }); \
         struct S y = ({ struct S t; t.a = 3; t.b = 4; t; }); \
         return x.a * 1000 + x.b * 100 + y.a * 10 + y.b;",
    );
    // **The scalar forms must keep working** — they always have, and they are the reason
    // the aggregate half stayed invisible.
    agree("int n = ({ int t = 3; t + 1; }); return n;");
    agree(
        "int arr[2] = {3, 4}; return ({ int acc = 0; for (int i = 0; i < 2; i++) { acc += arr[i]; } acc; });",
    );
    agree("int x = ({ 1; 2; 3; }); return x;");
}

/// **A universal character name is one character, at the literal's width.**
///
/// Two spellings, one defect. `unescape` has no case for `\\u`/`\\U`, so its catch-all keeps
/// the escaped character and drops the backslash: `u"\\uFFFF"` becomes the five characters
/// `u F F F F`, and the first element reads 117 instead of 65535. A character written
/// *directly* in the source fares no better — `u"￿"` keeps its three UTF-8 bytes as three
/// elements and reads 239. C11 5.2.1.1 makes the two spellings the same thing by the end of
/// translation phase 5, so one decoder owes both. Both are **silent wrong answers**, found
/// while mutation-testing wave 150 rather than by anyone reading the code.
///
/// C11 6.4.3 makes a UCN denote one character of the execution set. The width decides how
/// it is stored: a wide literal gets one *element* holding the code point, and a plain one
/// gets its **UTF-8 bytes** — `"\\u00E9"` is two bytes, `L"\\u00E9"` is one four-byte
/// element holding 233.
///
/// **`\\x` is not a UCN and must not become UTF-8.** `"\\xFF"` is the single byte 255;
/// `"\\u00FF"` is the two bytes `C3 BF`. A decoder that treated every escape as a code
/// point would conflate them, which is why the two sit next to each other below.
///
/// Fixing this also pins what wave 150 could not: `char16_t`'s **signedness**, which needs
/// a value above 32767 to observe and had no way to reach one.
#[test]
fn a_universal_character_name_is_one_character() {
    // The **escape** spelling and the **direct** spelling must land on the same answer.
    agree(r#"return (int)u"\uFFFF"[0];"#);
    agree(r#"return (int)sizeof(u"\uFFFF");"#);
    agree(r#"return (int)(u"\uFFFF"[0] == u"￿"[0]);"#);
    agree(r#"return (int)L"\u00E9"[0] * 10 + (int)sizeof(L"\u00E9");"#);
    agree(r#"return (int)sizeof("\u00E9") * 1000 + (int)(unsigned char)"\u00E9"[0];"#);
    // The code point, at each width.
    agree(r#"return (int)u"￿"[0];"#);
    agree(r#"return (int)sizeof(u"￿");"#);
    agree(r#"return (int)U"\U0001F600"[0];"#);
    agree(r#"return (int)sizeof(U"\U0001F600");"#);
    agree(r#"return (int)L"é"[0];"#);
    agree(r#"return (int)sizeof(L"é");"#);
    // **`char16_t` is unsigned**, which only a value above 32767 can show — `u"￿"[0]`
    // is 65535 and would be -1 if the element type were signed.
    agree(r#"return u"￿"[0] > 0;"#);
    agree(r#"return (int)(u"￿"[0] + 1);"#);
    // A plain literal encodes the character as **UTF-8**, not as one byte.
    agree(r#"return (int)sizeof("é");"#);
    agree(r#"return (int)(unsigned char)"é"[0] * 1000 + (int)(unsigned char)"é"[1];"#);
    // **`\x` is a byte, not a code point**, in both a plain and a wide literal.
    agree(r#"return (int)(unsigned char)"\xFF"[0] * 10 + (int)sizeof("\xFF");"#);
    agree(r#"return (int)u"\xFF"[0] * 10 + (int)sizeof(u"\xFF");"#);
    // Mixed with ordinary characters, so the decoder tracks position rather than assuming
    // the escape is alone.
    agree(r#"return (int)u"AéB"[0] * 1000 + (int)u"AéB"[1];"#);
    agree(r#"return (int)sizeof(u"AéB");"#);
    // **The three counting rules**, each a case where reading one digit too many or too
    // few changes the number of *elements* and not merely a value. Mutation found these
    // uncovered here: the unit tests in `strlit` caught all three, gcc caught none of them,
    // and gcc is the oracle that does not share the implementation's assumptions.
    agree(r#"return (int)u"\x41B"[0] + (int)sizeof(u"\x41B");"#);
    agree(r#"return (int)sizeof("\0101") * 1000 + (int)"\0101"[0] * 100 + (int)"\0101"[1];"#);
    // A code point above the BMP is a **surrogate pair** at 16 bits — two elements, so a
    // decoder that stored it whole would size the array wrong as well as fill it wrong.
    agree(r#"return (int)u"\U0001F600"[0] + (int)u"\U0001F600"[1] + (int)sizeof(u"\U0001F600");"#);
    // The escapes that already worked must keep working.
    agree(r#"return (int)sizeof("a\nb") * 100 + (int)"a\nb"[1];"#);
    agree(r#"return (int)sizeof("AB");"#);
}

/// **A character constant is decoded by the same rules as a string literal.**
///
/// Wave 151 gave string literals one decoder and recorded, in the same breath, that
/// character constants still had their own. This is that third copy: `parse_char_literal`
/// in sema, twenty lines, with no case for `\x`, no octal past `\0`, no universal character
/// name, no multi-character constant, and **no attention to the prefix at all**. Its
/// catch-all keeps the escaped character, so every one of those reads as a letter:
///
/// ```text
///   '\x41'          chiero 120 ('x')   gcc 65
///   '\101'          chiero  49 ('1')   gcc 65
///   u'\uFFFF'       chiero 117 ('u')   gcc 65535
/// ```
///
/// Three rules a string literal already obeys and a character constant does not:
///
/// - **the prefix decides the type**, so `sizeof(u'a')` is 2 and not 4 — the only prefix
///   whose size differs from `int`, and therefore the only one a size test can catch;
/// - **a plain constant holds bytes, so it sign-extends**: `'\xFF'` is -1, because C11
///   6.4.4.4p10 converts the single byte as a `char`. This is the one place the string
///   decoder's `Raw`/`Char` distinction changes a *sign* rather than a count;
/// - **more than one byte is a multi-character constant** (gcc's implementation-defined
///   rule, and gcc is the oracle): the bytes accumulate big-endian, so `'ab'` is 24930 and
///   `'\u00E9'` is 50089 — the UTF-8 pair `C3 A9` read as two characters, which is the
///   plain literal's UTF-8 rule and the multi-char rule composing.
///
/// The last one is why this cannot be fixed by copying three match arms across: the UCN and
/// the multi-character rule interact, and only a decoder that yields *units* sees it.
#[test]
fn a_character_constant_is_decoded_like_a_string_literal() {
    // Hex and octal, including octal's three-digit bound spilling into a second character.
    agree(r#"return '\x41';"#);
    agree(r#"return '\101';"#);
    agree(r#"return '\0101';"#);
    // A universal character name, at each prefix.
    agree(r#"return u'\uFFFF';"#);
    agree(r#"return L'\u00E9';"#);
    agree(r#"return (int)U'\U0001F600';"#);
    // **The prefix decides the type.** `u'a'` is the only one whose size differs from `int`.
    agree(r#"return (int)sizeof('a') * 100 + (int)sizeof(u'a') * 10 + (int)sizeof(L'a');"#);
    // **A plain constant sign-extends**, and the cast back through `unsigned char` shows the
    // byte it came from — so a decoder that got the value right but the type wrong fails one
    // of these two and not the other.
    agree(r#"return '\xFF';"#);
    agree(r#"return (int)(unsigned char)'\xFF';"#);
    // Multi-character constants, including the one a UCN produces in a plain constant.
    agree(r#"return 'ab';"#);
    agree(r#"return '\u00E9';"#);
    // The escapes that already worked must keep working.
    agree(r#"return '\n' * 100 + 'A';"#);
}

/// **Floating point reaches the engine and agrees with gcc.**
///
/// Wave 167 taught the engine to evaluate concrete floats — constants, the five arithmetic
/// operators, the six FP casts — and nothing can reach it, because lowering discards any
/// function that mentions a float. Wave 166 measured what that costs: **293 of 600**
/// generated programs refused, half the budget of the channel that has found more defects
/// than any other.
///
/// The refusal is not the only thing in the way, which the first attempt at removing it
/// showed immediately. Lowering emits *nothing* float-shaped:
///
/// - a floating literal goes through the `Number` arm, which builds `Const::Int` and falls
///   to a catch-all making it **0** — so `2.5f` lowers to zero at the wrong type, and the
///   verifier rejects the store (`store value operand is Int(32), declared Float(F32)`);
/// - `sema::ConstVal` has `Int` and `Addr` and no float variant, so there is nowhere for the
///   value to come from;
/// - `BinOp::FAdd` and the FP `CastKind`s appear **zero** times in the crate: float
///   arithmetic would lower to integer arithmetic on the bits.
///
/// That last one is why this is a differential test rather than a lowering golden. Integer
/// `Add` on two float bit patterns is a number — a wrong one — and a golden would have to
/// assert the wrongness to notice it. gcc is the only oracle that says 3 here.
///
/// The cases are deliberately the simplest end of the language: a literal, a local, one
/// operator, a cast back to `int`. Comparisons are left out because the engine has no float
/// arms in `cmp` either, and a test should fail for the reason it names.
#[test]
fn floating_point_agrees_with_gcc() {
    // A literal, stored and read back.
    agree("float f = 2.5f; return (int)f;");
    agree("double d = 2.5; return (int)d;");
    // Arithmetic, in both precisions.
    agree("float f = 2.5f; return (int)(f + 1.25f);");
    agree("double d = 10.0; return (int)(d / 4.0);");
    agree("double d = 2.5; return (int)(d * 4.0);");
    agree("double d = 10.0; return (int)(d - 2.5);");
    // int -> float -> int, where the intermediate is not representable as an integer.
    agree("int n = 7; double d = n; return (int)(d / 2.0);");
    // Truncation toward zero, C's rule and not the obvious one.
    agree("double d = -2.7; return (int)d;");
    agree("double d = 2.7; return (int)d;");
    // Narrowing between the precisions, where single precision visibly is not double.
    agree("double d = 0.1; float f = (float)d; return (int)(f * 1000.0f);");
}

/// **Floating comparisons, and the conversion to `_Bool` that is one.**
///
/// Wave 168 made floats lower and run, and refused these two: the engine's `cmp` has no
/// float arms, so `a < b` produces no value, and `(_Bool)f` lowered through `FpToSi` is
/// *wrong* rather than missing — C11 6.3.1.2 makes it "compares unequal to 0", and
/// truncating gives 0 for 0.5. 26 of the generator's 200 seeds refuse on this.
///
/// # Two things make this more than adding six arms
///
/// **CIR has no `FOGt` or `FOGe`.** The ordered set is `FOEq`/`FONe`/`FOLt`/`FOLe`, so
/// `a > b` has to lower as `FOLt` with the operands *swapped*. Swapping is not a detail
/// that comes out in the wash: get it backwards and every `>` in every program silently
/// answers `<`, which is why both directions are here with operands that disagree.
///
/// **NaN is not "unordered" as a curiosity.** C's `isnan` idiom is `x != x`, and CIR's own
/// comment says why the unordered set exists: `FONe` is *false* for NaN, the opposite of
/// what the idiom means. A float that is never NaN cannot tell `FONe` from `FUNe`, so the
/// last cases build one.
#[test]
fn floating_comparisons_agree_with_gcc() {
    // Every relational operator, with operands that make the answer differ from its
    // mirror — `2.5 < 1.5` and `2.5 > 1.5` must not both be 0.
    agree("double a = 2.5, b = 1.5; return a < b;");
    // Equal operands separate `<` from `<=` and `>` from `>=`. Without them a swapped or
    // widened comparison passes: 2.5 and 1.5 give the same answer under either.
    agree("double a = 2.5, b = 2.5; return a < b;");
    agree("double a = 2.5, b = 2.5; return a > b;");
    agree("double a = 2.5, b = 1.5; return a > b;");
    agree("double a = 2.5, b = 1.5; return a <= b;");
    agree("double a = 2.5, b = 1.5; return a >= b;");
    agree("double a = 2.5, b = 2.5; return a <= b;");
    agree("double a = 2.5, b = 2.5; return a >= b;");
    agree("double a = 2.5, b = 1.5; return a == b;");
    agree("double a = 2.5, b = 1.5; return a != b;");
    agree("double a = 2.5, b = 2.5; return a == b;");
    // Single precision goes through the same path at its own width.
    agree("float a = 2.5f, b = 1.5f; return a > b;");
    agree("float a = 1.5f, b = 2.5f; return a > b;");

    // **The conversion to `_Bool`**, which is a comparison against zero and not a
    // truncation. `0.5` is the case that separates the two: true in C, and 0 if truncated.
    agree("double d = 0.5; return (int)(_Bool)d;");
    // The *implicit* conversion as well as the cast: `_Bool b = d;` goes through the store
    // path rather than the cast path, and only one of the two was covered.
    agree("double d = 0.5; _Bool b = d; return (int)b;");
    agree("double d = 0.0; _Bool b = d; return (int)b;");
    agree("double d = 0.0; return (int)(_Bool)d;");
    agree("double d = 2.5; if (d) { return 7; } return 9;");
    agree("double d = 0.0; if (d) { return 7; } return 9;");
    // -0.0 compares equal to zero, so it is false — a sign bit is not a value.
    agree("double d = -0.0; return (int)(_Bool)d;");

    // **NaN.** `x != x` is C's `isnan`, and it is an *unordered* not-equal: true for NaN
    // where `FONe` would say false. Every ordered comparison with a NaN operand is false,
    // including `>=`, which is what makes it different from `!(a < b)`.
    agree("double n = 0.0 / 0.0; return n != n;");
    agree("double n = 0.0 / 0.0; return n == n;");
    agree("double n = 0.0 / 0.0; double b = 1.0; return n < b;");
    agree("double n = 0.0 / 0.0; double b = 1.0; return n >= b;");
    agree("double n = 0.0 / 0.0; return (int)(_Bool)n;");
}

/// **A mixed operand pair is float if *either* side is.**
///
/// C11 6.3.1.8's usual arithmetic conversions: when one operand is floating and the other
/// is not, the other is converted to the floating type and the operation is a floating one.
/// Lowering asks `is_float(lhs)` and `is_signed(lhs)` — the *left* operand only — so
/// `d + 1` lowers correctly and `1 + d` picks the integer opcode and the integer type.
///
/// The verifier catches it, so this is a refusal rather than a wrong answer. It is still a
/// defect: eight programs in seeds 800..1400 were discarded for it, and the shape is one a
/// person writes constantly — `2 * d`, `1 < d`, `0 == d`.
///
/// **Every case is written in both orders.** That is the whole point: the failing form is
/// the one whose *left* operand is the integer, and a fixture written only as `d OP 1`
/// passes against code that never looks at the right operand at all.
///
/// The unsigned case is here because it fails differently. `u < d` picks `ULt` — an
/// *unsigned* comparison — over two floats, so the operand type and the operator's
/// signedness are two separate things to get from the wrong side.
#[test]
fn mixed_integer_and_floating_operands_agree_with_gcc() {
    // Arithmetic, both orders.
    agree("double d = 2.5; return (int)(d + 1);");
    agree("double d = 2.5; return (int)(1 + d);");
    agree("double d = 2.5; return (int)(d - 1);");
    agree("double d = 2.5; return (int)(4 - d);");
    agree("double d = 2.5; return (int)(d * 2);");
    agree("double d = 2.5; return (int)(2 * d);");
    agree("double d = 2.5; return (int)(d / 2);");
    agree("double d = 10.0; return (int)(20 / d);");
    // Comparisons, both orders — the operator is not symmetric, so `1 < d` and `d > 1`
    // must both be right rather than one covering the other.
    agree("double d = 2.5; return d > 1;");
    agree("double d = 2.5; return 1 < d;");
    agree("double d = 2.5; return 1 > d;");
    agree("double d = 2.5; return d < 1;");
    agree("double d = 2.5; return 1 == d;");
    agree("double d = 2.5; return d == 1;");
    agree("double d = 2.5; return 1 != d;");
    // **An unsigned left operand**, which picks an unsigned comparison over two floats.
    agree("unsigned u = 1; double d = 2.5; return u < d;");
    agree("unsigned u = 3; double d = 2.5; return u > d;");
    // Single precision on the right, so the conversion is to `float` and not `double`.
    agree("float f = 2.5f; return (int)(1 + f);");
    agree("float f = 2.5f; return 1 < f;");
    // **Two different floating types**, where the *wider* must win. `(double)0.1f` is not
    // `0.1` — single precision cannot hold it — so comparing at `float` says equal and
    // comparing at `double` says not. A rule that picked either operand's type, or the
    // narrower of the two, passes every fixture above and fails this one.
    agree("float f = 0.1f; double d = 0.1; return f == d;");
    agree("double d = 0.1; float f = 0.1f; return d == f;");
    agree("float f = 0.5f; double d = 0.5; return f == d;");
    agree("float f = 0.1f; double d = 0.1; return (int)((f + d) * 100);");
    // A `char` promoted to `int` and then converted to `double`, which is two conversions.
    agree("char c = 2; double d = 2.5; return c < d;");
}

/// **A prefixed string literal keeps its element width.**
///
/// sema types every string literal `char[n]` and lowering writes one byte per character,
/// whatever prefix the literal carried. So `sizeof(L"AB")` is 3 where C says 12, and the
/// bytes behind `L"AB"` are `41 42 00` rather than four bytes per character. Both are
/// **silent wrong answers**: the literal encodes, the program runs, and every value that
/// depends on the width is off.
///
/// §9 recorded this as "`L`/`u`/`U` string literals lose their element width in `unquote`".
/// `unquote` is not the culprit — both copies of it strip the prefix correctly and hand back
/// the right *text*. What is lost is the **type**: C11 6.4.5p6 gives `L"…"` element type
/// `wchar_t`, `u"…"` `char16_t` and `U"…"` `char32_t`, and nothing downstream ever asks.
///
/// `u8"…"` stays `char`, which is why it is here as the case that must *not* change.
#[test]
fn a_prefixed_string_literal_keeps_its_element_width() {
    // `sizeof`, which is the shortest statement of the whole defect.
    agree(r#"return (int)sizeof("AB");"#);
    agree(r#"return (int)sizeof(u8"AB");"#);
    agree(r#"return (int)sizeof(u"AB");"#);
    agree(r#"return (int)sizeof(U"AB");"#);
    agree(r#"return (int)sizeof(L"AB");"#);
    // The empty and longer forms, so the answer is not a constant that happens to fit.
    agree(r#"return (int)sizeof(L"");"#);
    agree(r#"return (int)sizeof(L"ABC");"#);
    // **The bytes, not just the size.** A width that only `sizeof` knows about would pass
    // every case above and still store `41 42 00`.
    agree(r#"const int *w = (const int *)L"AB"; return w[0];"#);
    agree(r#"const int *w = (const int *)L"AB"; return w[1];"#);
    agree(r#"const int *w = (const int *)L"AB"; return w[2];"#);
    agree(r#"const unsigned short *u = (const unsigned short *)u"AB"; return u[0] * 100 + u[1];"#);
    // Plain literals must keep working — they are the ones that already do.
    agree(r#"const char *s = "AB"; return s[0] * 100 + s[1];"#);
    agree(r#"return (int)sizeof("hello");"#);
}

/// **A designated or bit-field initializer works at file scope too.**
///
/// `encode_into` returns `None` for a designator and for a bit-field member, under comments
/// saying the initializer is "refused whole rather than silently written in positional
/// order". The intent is right and the effect is not: the caller turns `None` into
/// `GlobalInit::Zero`, so **`struct S g = {.b = 3};` reads as all zeros** and `g.b` is 0
/// where C says 3. Refusing whole would have been a diagnostic; this is a fabrication with
/// a comment claiming otherwise.
///
/// §9 listed these as "designated and bit-field initializers refused", which was wrong in
/// both directions — they *work* for locals (`init_list` has handled designators since it
/// was written, and bit-fields since wave 142), and at file scope they are not refused but
/// silently zeroed. A stale owed entry sends the next reader to the wrong place, so this
/// test pins what is actually true.
#[test]
fn a_global_initializer_handles_designators_and_bitfields() {
    // A designator alone, and mixed with positional elements — C11 6.7.9p17 continues from
    // the designated position, so `{1, .c = 9}` leaves `b` zero.
    agree_with(
        "struct S { int a; int b; int c; };\nstruct S g = {.b = 3};\n",
        "return g.a * 100 + g.b * 10 + g.c;",
    );
    agree_with(
        "struct S { int a; int b; int c; };\nstruct S g = {1, .c = 9};\n",
        "return g.a * 100 + g.b * 10 + g.c;",
    );
    // Out of declaration order, which a positional walk would get wrong rather than miss.
    agree_with(
        "struct S { int a; int b; int c; };\nstruct S g = {.c = 9, .a = 1};\n",
        "return g.a * 100 + g.b * 10 + g.c;",
    );
    // Array designators, alone and after a positional element.
    agree_with("int ga[4] = {[2] = 7};\n", "return ga[2] * 10 + ga[0];");
    agree_with("int ga[4] = {1, [3] = 8};\n", "return ga[0] * 10 + ga[3];");
    // **Bit-fields at file scope**, including truncation to the field: 7 in a 3-bit signed
    // field is -1, so a byte-wise write that ignored the width would answer 72 not -8.
    agree_with(
        "struct B { int a:3; int b:5; };\nstruct B g = {1, 2};\n",
        "return g.a * 10 + g.b;",
    );
    agree_with(
        "struct B { int a:3; int b:5; };\nstruct B g = {7, 2};\n",
        "return g.a * 10 + g.b;",
    );
    // A designated bit-field, which is both features at once.
    agree_with(
        "struct B { int a:3; int b:5; };\nstruct B g = {.b = 3};\n",
        "return g.a * 10 + g.b;",
    );
    // **A value whose extra bits are set, with the neighbour left unwritten.** Wave 142
    // learned this and this test did not apply it: `{7, 2}` cannot see a store one bit too
    // wide, because 7 in four bits is `0111` so the extra bit is clear — and `{15, 2}`
    // cannot either, because `b`'s own write repairs the damage a moment later. Only a
    // *partial* initializer shows it, where `b` gets nothing but the zero-fill.
    agree_with(
        "struct B { int a:3; int b:5; };\nstruct B g = {15};\n",
        "return g.a * 10 + g.b;",
    );
    // **A bit written and then written back to zero.** `out` starts zeroed, so clearing a
    // bit is a no-op unless something set it first — which only a repeated designator does.
    // C11 6.7.9p19: the last initializer for an object is the one that counts.
    agree_with(
        "struct B { int a:3; int b:5; };\nstruct B g = {.a = 7, .a = 0};\n",
        "return g.a * 10 + g.b;",
    );
    agree_with(
        "struct B { int a:3; int b:5; };\nstruct B g = {.b = 31, .b = 0};\n",
        "return g.a * 10 + g.b;",
    );
    // Nested, with designators at both levels.
    agree_with(
        "struct S { int a; int b; int c; };\nstruct N { struct S s; int n; };\n\
         struct N g = {.n = 4, .s = {.b = 6}};\n",
        "return g.s.b * 10 + g.n;",
    );
    // **The positional forms that already worked must keep working** — they are the ones
    // `encode_into` does handle, and the fix must not disturb them.
    agree_with(
        "struct S { int a; int b; int c; };\nstruct S g = {1, 2, 3};\n",
        "return g.a * 100 + g.b * 10 + g.c;",
    );
    agree_with(
        "struct S { int a; int b; int c; };\nstruct S g = {7};\n",
        "return g.a * 100 + g.b * 10 + g.c;",
    );
    agree_with("int ga[3] = {4, 5, 6};\n", "return ga[0] * 10 + ga[1];");
}

/// **A file-scope pointer initialized with an address holds that address.**
///
/// `GlobalInit` has `Zero`, `Bytes` and `Extern` and no form for an *address*, so
/// `encode_init` returns `None` for `int *gp = &g;` and lowering falls back to `Zero`. The
/// comment there reasons that `Zero` "is at least not a fabrication" — true of a partial
/// encoding, and not true here: **`gp == 0` answers 1** for a pointer that is definitely
/// not null. A null check on a validly-initialized global reports null.
///
/// An address cannot be bytes. 021's model gives a pointer an *object*, and a byte pattern
/// carries no provenance — which is exactly why this needs its own `GlobalInit` variant
/// rather than a cleverer `encode_init`.
///
/// Found by the generator once file-scope declarations entered the grammar. Function
/// pointers already worked (`int (*fp)(int) = twice;` calls correctly), so the asymmetry
/// was there to be noticed and nothing had looked.
#[test]
fn a_file_scope_pointer_holds_the_address_it_was_given() {
    // The null check, which is the case that answers *wrongly* rather than not at all.
    agree_with("int g = 5;\nint *gp = &g;\n", "return gp == 0;");
    agree_with("int g = 5;\nint *gp = &g;\n", "return gp != 0;");
    // Reading through it.
    agree_with("int g = 5;\nint *gp = &g;\n", "return *gp;");
    agree_with(
        "int ga[3] = {10, 20, 30};\nint *gp = ga;\n",
        "return gp[1];",
    );
    agree_with(
        "int ga[3] = {10, 20, 30};\nint *gp = ga;\n",
        "return *(gp + 2);",
    );
    agree_with(
        "int ga[3] = {10, 20, 30};\nint *gp = &ga[1];\n",
        "return *gp;",
    );
    // Writing through it, and seeing the write in the *other* name for the same object —
    // which is what makes it an address rather than a copy.
    agree_with("int g = 5;\nint *gp = &g;\n", "*gp = 9; return g;");
    agree_with(
        "int ga[3] = {10, 20, 30};\nint *gp = ga;\n",
        "gp[1] = 7; return ga[1];",
    );
    agree_with(
        "int ga[3] = {10, 20, 30};\nint *gp = ga;\n",
        "ga[1] = 7; return gp[1];",
    );
    // A pointer to a pointer, so the initializer's target is itself a pointer.
    agree_with(
        "int g = 5;\nint *gp = &g;\nint **gpp = &gp;\n",
        "return **gpp;",
    );
    // **The cases that already work must keep working**: a function pointer at file scope,
    // and a global with no initializer at all, which C11 6.7.9p10 zero-initializes.
    agree_with(
        "int twice(int x) { return x + x; }\nint (*fp)(int) = twice;\n",
        "return fp(3);",
    );
    agree_with("int g;\n", "return g;");
    agree_with("int *gp;\n", "return gp == 0;");
}

/// **A braced initializer converts to the bit-field's unit before storing.**
///
/// Wave 142 gave `init_list` a `StoreBits` for bit-field members and passed the value
/// straight through. `StoreBits` declares the *unit* — `Int(32)` for an `int:3` — so an
/// initializer of any other width is a mismatch: `long l = 9; struct B v = {l, 2};` emits
/// a 64-bit value into a 32-bit unit and the function is refused.
///
/// **Found by the refusal ledger** the moment wave 147 gave it something to hold. It sat
/// among the float entries as the one line that was not about floats, which is exactly what
/// a ledger is for — the float gap is known and tolerated, and this was neither.
///
/// Assignment already worked: `w.a = l;` is an assignment expression and sema converts its
/// right-hand side, so the same struct behaves correctly when filled field by field. Wave
/// 140's pairing again, and wave 142 reintroduced it in the path it had just fixed.
#[test]
fn a_braced_bitfield_initializer_converts_to_its_unit() {
    const B: &str = "struct B { int a:3; int b:5; };\n";
    // Wider than the unit, which is what the ledger caught.
    agree_with(B, "long l = 9; struct B v = {l, 2}; return v.a * 10 + v.b;");
    agree_with(B, "long l = 9; struct B v = {2, l}; return v.a * 10 + v.b;");
    // Narrower than the unit, the other direction of the same mismatch.
    agree_with(
        B,
        "short s = 9; struct B v = {s, 2}; return v.a * 10 + v.b;",
    );
    agree_with(
        B,
        "signed char c = 9; struct B v = {c, 2}; return v.a * 10 + v.b;",
    );
    // **Signedness on the way in.** A negative `long` narrowed to the unit and then to a
    // 3-bit field must still be -1, and a zero-extension would make it 7.
    agree_with(
        B,
        "long l = -1; struct B v = {l, 2}; return v.a * 10 + v.b;",
    );
    agree_with(
        "struct U { unsigned a:3; unsigned b:5; };\n",
        "unsigned long l = 9; struct U v = {l, 2}; return (int)(v.a * 10 + v.b);",
    );
    // A compound literal reaches the same code.
    agree_with(
        B,
        "long l = 9; struct B v = (struct B){l, 2}; return v.a * 10 + v.b;",
    );
    // **Assignment already worked and must keep working** — it is the pairing that hid
    // this, and the fix must not disturb it.
    agree_with(
        B,
        "long l = 9; struct B w; w.a = l; w.b = 2; return w.a * 10 + w.b;",
    );
    agree_with(
        B,
        "long l = 9; struct B x; x.a = 1; x.b = 2; x.a += l; return x.a * 10 + x.b;",
    );
}

/// **A braced initializer stores a bit-field as bits, not as its storage unit.**
///
/// 015 contract 7 says a bit-field access uses the `BitRange` from `RecordLayout`, and
/// `assign` obeys it — `v.a = 1` emits `StoreBits`. `init_list` never got the rule and
/// emits an ordinary full-width `Store`, so `struct B { int a:3; int b:5; } v = {1, 2};`
/// writes `1` over the whole storage unit and then `2` over it again.
///
/// **It is a wrong answer, not a missing one**: chiero says 20 where C says 12. Wave 113's
/// rule, and the reason this ranks above the two silent gaps left in §9.
///
/// Found by the generator once bit-fields entered the record grammar. A single bit-field at
/// offset 0 works by accident — with no neighbour there is nothing to clobber — which is
/// why `differential.rs`'s existing bit-field fixtures, which assign field by field, never
/// saw it. The same "works one spelling, not the other" pairing as wave 140.
#[test]
fn a_braced_initializer_stores_a_bitfield_as_bits() {
    const B: &str = "struct B { int a:3; int b:5; };\n";
    // Two adjacent bit-fields in one unit: the first must survive the second.
    agree_with(B, "struct B v = {1, 2}; return v.a * 10 + v.b;");
    // **Truncated to the field and reinterpreted at its signedness**: 7 in a 3-bit signed
    // field is -1, so the answer is -8 rather than 72.
    agree_with(B, "struct B v = {7, 2}; return v.a * 10 + v.b;");
    // A partial initializer — C11 6.7.9p21 zero-fills `b`, which the `SetMem` already does
    // and a per-field `StoreBits` must not undo.
    agree_with(B, "struct B v = {1}; return v.a * 10 + v.b;");
    // Unsigned wraps the other way: 40 in a 5-bit unsigned field is 8.
    agree_with(
        "struct C { unsigned a:3; unsigned b:5; };\n",
        "struct C v = {7, 40}; return (int)(v.a * 100 + v.b);",
    );
    // A bit-field after an ordinary member, so the byte offset is not zero.
    agree_with(
        "struct D { short s; int b:3; };\n",
        "struct D v = {5, 2}; return v.s * 10 + v.b;",
    );
    // And an ordinary member after the bit-fields, so the unit's neighbours on both sides
    // are checked.
    agree_with(
        "struct E { int a:3; int b:5; int n; };\n",
        "struct E v = {1, 2, 9}; return v.a * 100 + v.b * 10 + v.n;",
    );
    // The compound-literal spelling reaches the same code.
    agree_with(B, "struct B v = (struct B){1, 2}; return v.a * 10 + v.b;");
    // **A value whose extra bits are set**, which is what pins the *width* of the write
    // rather than only its offset. `{7, 2}` cannot see an over-wide store — 7 in four bits
    // is `0111`, so the fourth bit is 0 and the neighbour survives by luck. 15 is `1111`,
    // so a store one bit too wide sets `b`'s low bit and the answer moves.
    agree_with(B, "struct B v = {15, 2}; return v.a * 10 + v.b;");
    agree_with(
        "struct C { unsigned a:3; unsigned b:5; };\n",
        "struct C v = {15, 2}; return (int)(v.a * 100 + v.b);",
    );
    // **And the neighbour must be left unwritten**, or the over-wide store is repaired by
    // the very next one. `{15, 2}` above cannot see it: `a`'s stray fourth bit lands in
    // `b`'s bit 0, and `b = 2` is stored straight after and overwrites it. With `{15}` the
    // zero-fill is all `b` ever gets, so the stray bit survives to be read.
    agree_with(B, "struct B v = {15}; return v.a * 10 + v.b;");
    agree_with(
        "struct C { unsigned a:3; unsigned b:5; };\n",
        "struct C v = {15}; return (int)(v.a * 100 + v.b);",
    );
    // A single bit-field at offset 0 passes even unfixed — kept as the case that must not
    // regress, and as the reason the defect stayed invisible.
    agree_with("struct F { int a:4; };\n", "struct F v = {7}; return v.a;");
}

/// **A braced initializer element is converted to its member's type.**
///
/// C11 6.7.9p11: the initializer for a scalar member is converted as if by assignment.
/// sema inserts that conversion for an assignment *expression* and not for a braced
/// element — it is not typing an assignment there — so `{3, 5}` stored a 32-bit `3` into a
/// slot declared `i8` and the engine ended the path on the width mismatch.
///
/// **Every struct with a member narrower than `int` was affected**, which is most of them,
/// and no fixture had one: `differential.rs` used `struct S { int a; int b; }` throughout.
/// The generator produced a `signed char` member on its first program with structs in the
/// grammar.
///
/// Assignment already worked — `v.a = 3` is an assignment expression and sema converts it.
/// That pairing is why the defect is invisible from the outside: the same struct behaves
/// correctly when filled field by field.
#[test]
fn a_braced_initializer_element_converts_to_its_member_type() {
    // Narrower than `int`, first member and last, so an offset-0 special case is not it.
    agree_with(
        "struct S { signed char a; int b; };\n",
        "struct S v = {3, 5}; return v.a * 10 + v.b;",
    );
    agree_with(
        "struct S { int a; signed char b; };\n",
        "struct S v = {3, 5}; return v.a * 10 + v.b;",
    );
    agree_with(
        "struct S { short a; int b; };\n",
        "struct S v = {3, 5}; return v.a * 10 + v.b;",
    );
    // A single narrow member, so the padding is not what makes it work.
    agree_with(
        "struct S { signed char a; };\n",
        "struct S v = {3}; return v.a;",
    );
    // **Truncation that is visible**: 300 does not fit a `signed char`, and C says the
    // stored value is 44.
    agree_with(
        "struct S { signed char a; int b; };\n",
        "struct S v = {300, 5}; return v.a;",
    );
    agree_with(
        "struct S { unsigned char a; int b; };\n",
        "struct S v = {300, 5}; return v.a;",
    );
    // **Widening, and its signedness.** A negative `int` initializer into a `long` member
    // must sign-extend; an unsigned one must not.
    agree_with(
        "struct S { long a; int b; };\n",
        "struct S v = {-1, 5}; return (int)(v.a >> 32);",
    );
    agree_with(
        "struct S { unsigned long a; int b; };\n",
        "struct S v = {4294967295u, 5}; return (int)(v.a >> 16);",
    );
    // The compound-literal spelling reaches the same code.
    agree_with(
        "struct S { signed char a; int b; };\n",
        "struct S v = (struct S){3, 5}; return v.a * 10 + v.b;",
    );
    // An **array** of a narrow type, which is the other shape `init_list` walks.
    agree("signed char a[3] = {1, 2, 300}; return a[0] * 100 + a[1] * 10 + a[2];");
    // And a nested struct, so the recursion converts too.
    agree_with(
        "struct I { signed char x; }; struct O { struct I i; int n; };\n",
        "struct O o = {{300}, 5}; return o.i.x * 10 + o.n;",
    );
    // Assignment already worked and must keep working — it is the pairing that hid this.
    agree_with(
        "struct S { signed char a; int b; };\n",
        "struct S v; v.a = 3; v.b = 5; return v.a * 10 + v.b;",
    );
}

/// **A literal `0` added to a pointer is an integer, not a null pointer constant.**
///
/// sema converts a null constant to the pointer's type when one operand is a pointer — for
/// *any* operator. Its own comment says why the rule exists: "a null constant **compared**
/// against a pointer becomes that pointer type, so `p == 0` does not look like a
/// pointer/integer mismatch downstream". Comparisons are the whole of it. C11 6.5.6 makes
/// the other operand of `+`/`-` on a pointer an *integer*, and 6.3.2.3's null-constant rule
/// is about assignment and comparison contexts.
///
/// So `&a[1] + 0` converted the `0` to a pointer, and lowering's `ptr_arith` then tried to
/// sign-extend a `Ptr` to 64 bits: `inttoptr i32 0 to ptr` followed by `zext i32 %7 to i64`.
/// The verifier rejects that — "cast source operand is Ptr, declared Int(32)" — but nothing
/// runs it at lowering time, so the function was emitted and the engine produced no state.
///
/// Found by the generator once the alternative-spelling production existed: `*(&a[i] + 0)`
/// is one of the six ways it writes an access, and the other five all worked.
///
/// The comment described a narrower rule than the code implemented, which is waves 107,
/// 112, 118, 124 and 132's shape exactly.
#[test]
fn a_zero_added_to_a_pointer_is_an_integer() {
    // The generator's find, and its siblings on both sides of the operator.
    agree("int a[3] = {1, 2, 3}; return *(&a[1] + 0);");
    agree("int a[3] = {1, 2, 3}; return *(0 + &a[1]);");
    agree("int a[3] = {1, 2, 3}; return *(&a[1] - 0);");
    agree("int a[3] = {1, 2, 3}; int *p = a; return *(p + 0);");
    agree("int a[3] = {1, 2, 3}; int *p = &a[2]; return *(p - 0);");
    // A **nonzero** literal on the same path, so the fix is not "stop converting" but
    // "convert only where C says to".
    agree("int a[3] = {1, 2, 3}; return *(&a[0] + 2);");
    agree("int a[3] = {1, 2, 3}; return *(2 + &a[0]);");
    // Writing through it, so the store path sees the same address computation.
    agree("int a[3] = {1, 2, 3}; *(&a[1] + 0) = 9; return a[1];");
    // **The comparisons the conversion exists for must keep working.** These are the half
    // the rule is right about, and a fix that removed it wholesale breaks them.
    agree("int x; int *p = &x; return p == 0;");
    agree("int x; int *p = &x; return 0 != p;");
    agree("int *p = 0; return p == 0;");
    agree("int x; int *p = &x; return p ? 1 : 0;");
    // And a null constant *assigned* to a pointer, the other context 6.3.2.3 covers.
    agree("int *p = 0; return p == 0;");
    agree("int x; int *p = &x; p = 0; return p == 0;");
}

/// **A member can be selected off a value, not only off an lvalue.**
///
/// `lvalue_addr`'s `Member` arm asks `lvalue_addr` for the base, which is `None` for
/// anything that is not an lvalue — so `make(7).a`, a field of a call's result, produced
/// **no state and zero diagnostics**. §9 has carried this since wave 132's implementation
/// review found it; the generator drove into it the moment struct-returning helpers were
/// added to the grammar, because `h(...).f0` is how it uses one.
///
/// C11 6.5.2.3p3 allows it: the `.` operator's left operand is an *expression* of struct
/// type, not necessarily an lvalue. Since wave 132 an aggregate expression already
/// evaluates to its address — that is the whole point of "CIR has no aggregate values" —
/// so the base is available; the arm simply never asked for it that way.
#[test]
fn a_member_selects_off_a_value_as_well_as_an_lvalue() {
    const P: &str = "struct S { int a; int b; };\n\
         static struct S make(int x) { struct S o; o.a = x; o.b = x + 1; return o; }\n\
         static struct S thru(struct S p) { struct S o; o.a = p.b; o.b = p.a; return o; }\n";
    // The call result, both fields, so an implementation that returns the base address
    // without the offset passes only the first.
    agree_with(P, "return make(7).a;");
    agree_with(P, "return make(7).b;");
    agree_with(P, "return make(7).a * 10 + make(7).b;");
    // Through a helper that takes a struct too, so the sret slot and the by-value
    // parameter are both live in the same expression.
    agree_with(P, "return thru((struct S){1, 2}).a;");
    agree_with(P, "return thru((struct S){1, 2}).b;");
    // **Off a compound literal**, which wave 138 made work through the parser and which
    // reaches the same arm.
    agree_with(P, "return (struct S){9, 4}.b;");
    // Nested: a member of a struct member, where the outer base is a call result.
    agree_with(
        "struct I { int x; int y; };\nstruct O { struct I i; };\n\
         static struct O mk(void) { struct O o; o.i.x = 3; o.i.y = 4; return o; }\n",
        "return mk().i.x * 10 + mk().i.y;",
    );
    // And assigned from, so the aggregate-assignment path sees it as a source.
    agree_with(
        "struct I { int x; int y; };\nstruct O { struct I i; };\n\
         static struct O mk(void) { struct O o; o.i.x = 3; o.i.y = 4; return o; }\n",
        "struct I y = mk().i; return y.x * 10 + y.y;",
    );
}

/// **A compound literal is an object.**
///
/// `raw_expr` handles every `ExprKind` but three, and falls through to `Undef` for the
/// rest. Found by enumerating the variants against the arms rather than by writing a
/// fixture: `Error`, `TypeName` and `InitList` reach the catch-all. The first two are
/// **honestly refused** — `__builtin_types_compatible_p(int, int)` pushes "contains a
/// construct lowering cannot represent" and 015 §7 discards the function, which is a gap
/// behaving correctly. `InitList` is the silent one: `(struct S){1, 2}` preprocesses,
/// parses, types and lowers with **zero diagnostics** and produces no state at all.
///
/// C99 6.5.2.5 makes a compound literal an unnamed *object* with automatic storage at
/// block scope, not a value — so it has an address, it is an lvalue, and it can be
/// assigned through. That is why the fix cannot be "evaluate the braces into a register".
///
/// VPP writes them in 9 files.
#[test]
fn a_compound_literal_is_an_object() {
    const S: &str = "struct S { int a; int b; };\n";
    // As an initializer, which is the spelling that reads most like a value.
    agree_with(S, "struct S s = (struct S){1, 2}; return s.a * 10 + s.b;");
    // **Its address**, which is the half that proves it is an object rather than a value.
    agree_with(
        S,
        "struct S *p = &(struct S){5, 6}; return p->a * 10 + p->b;",
    );
    // An array compound literal decaying, which is how a caller passes a temporary list.
    agree("int *q = (int[]){7, 8}; return q[0] * 10 + q[1];");
    // Passed by value to a helper, so it composes with the wave-132 parameter copy.
    agree_with(
        "struct S { int a; int b; };\nstatic int sum_of(struct S s) { return s.a * 10 + s.b; }\n",
        "return sum_of((struct S){3, 4});",
    );
    // A member selected straight off the literal, with no named object anywhere.
    agree_with(S, "return (struct S){9, 1}.a;");
    // A **scalar** compound literal, which is legal and is not an aggregate at all.
    agree("int n = (int){42}; return n;");
    // Assigned *from*, so the aggregate-assignment path sees a non-lvalue right-hand side
    // that is nonetheless an object.
    agree_with(
        S,
        "struct S u = (struct S){1, 2}; u = (struct S){7, 8}; return u.a * 10 + u.b;",
    );
    // A designated initializer inside one, and a nested literal, so the initializer
    // machinery is reached rather than a two-field special case.
    agree_with(S, "struct S t = (struct S){.b = 3}; return t.a * 10 + t.b;");
    agree_with(S, "return ((struct S){(int){2}, 5}).b;");
    // **Two literals are two objects.** One shared scratch slot would make both reads see
    // the second, which every case above would still pass.
    agree_with(
        S,
        "struct S *p = &(struct S){1, 2}; struct S *q = &(struct S){3, 4}; \
         return p->a * 1000 + p->b * 100 + q->a * 10 + q->b;",
    );
    // Written through, which C permits because it is an lvalue.
    agree_with(
        S,
        "struct S *p = &(struct S){1, 2}; p->a = 9; return p->a * 10 + p->b;",
    );
}

/// **An enumeration constant is its value.**
///
/// `expr`'s `Ident` arm resolves a local, then a global, then a function name, and falls
/// through to `Undef` — enumerators are none of the three, so every use of one lowered to
/// `undef` and no diagnostic was pushed, which means 015 §7 never refused the function
/// either. `enum E { A = 3 }; return A;` produced `ret undef:i32`.
///
/// sema knows the values: `Cx::enumerators` holds them and `const_eval` resolves them,
/// which is why `int arr[C];` gets the right bound. The map lives on the throwaway context
/// and is dropped when `analyze` returns, so lowering never sees it.
///
/// **The `switch` case is the one that returns a wrong answer rather than none.** With the
/// selector and both labels `undef`, no arm matches and control reaches the code after the
/// statement — `switch (B) { case A: return 1; case B: return 2; }` falls out and returns
/// 0. Wave 113's rule: a wrong answer is worse than a missing one.
#[test]
fn an_enumeration_constant_is_its_value() {
    const E: &str = "enum E { A = 3, B, C = 7 };\n";
    // The three shapes an enumerator takes its value from: written, implicit successor,
    // and written again after an implicit one.
    agree_with(E, "return A;");
    agree_with(E, "return B;");
    agree_with(E, "return C;");
    // Implicit numbering from zero, and a negative start — `N1` is 0, not 1, because it
    // succeeds -1.
    agree_with("enum F { F0, F1, F2 };\n", "return F2;");
    agree_with("enum N { N0 = -1, N1 };\n", "return N0 * 10 + N1;");
    // **Declared inside the function**, which is where VPP puts most of them, and at file
    // scope, which is the path through the global table rather than the local one.
    agree("enum E { A = 3, B, C = 7 }; return B;");
    agree_with("enum G { GA = 10, GB };\n", "return GB;");
    // Used as a value, not just returned: initializing, assigning, and in arithmetic.
    agree_with(E, "enum E e = B; return e;");
    agree_with(E, "enum E e; e = C; return e;");
    agree_with(E, "return A + C;");
    agree_with(
        E,
        "int n = 0; for (int i = A; i < C; i++) { n++; } return n;",
    );
    // **`switch` over enumerators**, selector and labels alike. This is the case that
    // silently returned 0.
    agree_with(
        E,
        "switch (B) { case A: return 1; case B: return 2; case C: return 3; } return 0;",
    );
    // A `case` whose label is an enumerator but whose selector is a plain int, so the two
    // halves are pinned separately.
    agree_with(
        E,
        "int x = 7; switch (x) { case A: return 1; case C: return 3; } return 0;",
    );
    // The enumerator in a condition, where a wrong value changes which branch runs rather
    // than only what is returned.
    agree_with(E, "if (A < C) { return 11; } return 22;");
    // **An enumeration too wide for `int`.** Found by a mutation: hardcoding the constant's
    // width to 32 passed every case above, because every case above fits. C11 6.4.4.3 says
    // an enumeration constant is an `int`, but only because it also requires the values to
    // fit one; gcc widens the whole enumeration to `long` when they do not, and
    // `sizeof(X)` is then 8. Emitted 32 bits wide, `X` lowers to `5000000000i32`.
    agree_with("enum Big { X = 5000000000 };\n", "return (int)(X >> 32);");
    agree_with(
        "enum Big { X = 5000000000 };\n",
        "return (int)(X & 0xffffffff);",
    );
    agree_with("enum Big { X = 5000000000 };\n", "return (int)sizeof(X);");
    // And a negative wide one, so the widening keeps its signedness. `Y >> 32` alone does
    // not discriminate — the comparison does.
    agree_with("enum Neg { Y = -5000000000 };\n", "return (int)(Y >> 32);");
    agree_with("enum Neg { Y = -5000000000 };\n", "return Y < 0;");
    // **A function-local enum does not escape its function**, and does not overwrite a
    // file-scope name it shares. Found by a mutation: recording the values by *name* keeps
    // whichever was seen last, so the file-scope `K` read as 2. Both uses are pinned,
    // because a fix that made the outer one right by dropping the inner is no better.
    const K: &str = "enum K1 { K = 1 };\nstatic int inner(void) { enum K2 { K = 2 }; return K; }\n";
    agree_with(K, "return K;");
    agree_with(K, "return inner();");
    agree_with(K, "return K * 10 + inner();");
}

/// **A read-modify-write on a bit-field stays inside the bit-field.**
///
/// `assign`'s `StoreBits` guard is `op.is_none() && bitfield_of(lhs)`, so it fires for
/// `v.a = 1` and not for `v.a += 1`; `inc_dec` has no bit-field check at all. Both fall
/// through to the ordinary path, which loads and stores the *declared* type — a whole
/// `int` — so the write lands on every neighbouring field in the same storage unit. 015
/// contract 7 owns this and the plain-assignment path already obeys it.
///
/// The discriminator is **width, not just the neighbour**: a 3-bit signed field holding 3
/// takes `+= 1` to −4, because the result is truncated to the field and reinterpreted. An
/// `i32` read-modify-write answers 4, which is a legal `int` and looks entirely reasonable.
/// Every case returns both fields packed into one number, so a write that corrupts the
/// neighbour fails even when its own field is right.
#[test]
fn a_bitfield_read_modify_write_stays_inside_the_bitfield() {
    const B: &str = "struct B { int a:3; int b:5; };\n";
    // Compound assignment, in range: the neighbour must be untouched.
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; v.a += 1; return v.a * 100 + v.b;",
    );
    // **Out of range**, which is where a full-unit RMW stops being plausible: 3 + 1 in a
    // 3-bit signed field is −4, not 4.
    agree_with(
        B,
        "struct B v; v.a = 3; v.b = 2; v.a += 1; return v.a * 100 + v.b;",
    );
    agree_with(
        B,
        "struct B v; v.a = 2; v.b = 3; v.a *= 3; return v.a * 100 + v.b;",
    );
    // The wider neighbour, so the fix cannot be keyed to the first field's offset of 0.
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; v.b += 30; return v.a * 100 + v.b;",
    );
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; v.b -= 1; return v.a * 100 + v.b;",
    );
    // **Unsigned wraps differently** — 7 + 1 in a 3-bit unsigned field is 0, not −8.
    agree_with(
        "struct U { unsigned a:3; unsigned b:5; };\n",
        "struct U u; u.a = 7; u.b = 2; u.a += 1; return (int)(u.a * 100 + u.b);",
    );
    // Increment and decrement, which reach `inc_dec` rather than `assign`.
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; v.b++; return v.a * 100 + v.b;",
    );
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; v.b--; return v.a * 100 + v.b;",
    );
    agree_with(
        B,
        "struct B v; v.a = 3; v.b = 2; v.a++; return v.a * 100 + v.b;",
    );
    // **The value the expression yields**, not only the value it stores. `a++` is the
    // pre-value and `++a` the post-value, and both are read back out of the field.
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; int r = v.a++; return r * 100 + v.a;",
    );
    agree_with(
        B,
        "struct B v; v.a = 1; v.b = 2; int s = ++v.a; return s * 100 + v.a;",
    );
    agree_with(
        B,
        "struct B v; v.a = 3; v.b = 2; int r = v.a++; return r * 100 + v.a;",
    );
    // **A compound assignment's own value is the field after the store**, truncated
    // (C11 6.5.16.2p3) — `(v.a += 1)` on a 3-bit field holding 3 is -4, not the 4 the
    // addition produced. An implementation that stores correctly and hands back the
    // untruncated sum passes every case above, which is why the reload is not optional.
    agree_with(
        B,
        "struct B v; v.a = 3; v.b = 2; int r = (v.a += 1); return r * 100 + v.a;",
    );
    agree_with(
        B,
        "struct B v; v.a = 3; v.b = 2; int s = ++v.a; return s * 100 + v.a;",
    );
    agree_with(
        "struct U { unsigned a:3; unsigned b:5; };\n",
        "struct U u; u.a = 7; u.b = 2; int r = (int)(u.a += 1); return r * 100 + (int)u.b;",
    );

    // **The load's signedness**, which `+= 1` cannot see: truncation absorbs the
    // difference, so a mutation forcing `signed = true` survived every case above.
    // Division and `>>` do not absorb it — 7 as a 3-bit *unsigned* field is 7 and `/= 2`
    // is 3, while read as signed it is -1 and `/= 2` is 0. The same both ways round: -4 in
    // a 3-bit signed field is -4 and `/= 2` is -2, but read as unsigned it is 4 and gives 2.
    const U: &str = "struct U { unsigned a:3; unsigned b:5; };\n";
    agree_with(
        U,
        "struct U u; u.a = 7; u.b = 2; u.a /= 2; return (int)(u.a * 100 + u.b);",
    );
    agree_with(
        U,
        "struct U u; u.a = 7; u.b = 2; u.a >>= 1; return (int)(u.a * 100 + u.b);",
    );
    agree_with(
        B,
        "struct B v; v.a = -4; v.b = 2; v.a /= 2; return v.a * 100 + v.b;",
    );
    agree_with(
        B,
        "struct B v; v.a = -3; v.b = 2; v.a >>= 1; return v.a * 100 + v.b;",
    );
    // And the value `x++` hands back is the field read at its own signedness: 7 for the
    // unsigned field, -4 for the signed one, where the wrong load gives -1 and 4.
    agree_with(
        U,
        "struct U u; u.a = 7; u.b = 2; int r = (int)u.a++; return r * 100 + (int)u.a;",
    );
    agree_with(
        B,
        "struct B v; v.a = -4; v.b = 2; int s = v.a++; return s * 100 + v.a;",
    );
}

/// **`?:` and aggregate initializers**, the two constructs contract 2's goldens and
/// contract 22's corpus both need.
///
/// `?:` shares the four-block shape with `&&` (015 §2.1) but types its slot as the
/// *result* type, and the GNU elvis form `a ?: b` evaluates `a` exactly once — a shape
/// test cannot see the difference between once and twice when `a` has no side effects.
#[test]
fn conditionals_and_aggregate_initializers_compute_what_gcc_computes() {
    agree("int a = 0; int b = 5; int c = 7; return a ? b : c;");
    agree("int a = 3; int b = 5; int c = 7; return a ? b : c;");
    agree("int a = 2; return (a ? 10 : 20) + (a ? 1 : 2);");
    agree("int a = 0; int b = 9; return a ?: b;");
    agree("int a = 4; int b = 9; return a ?: b;");
    // Nested, so the two slots cannot be confused for one.
    agree("int a = 1; int b = 0; int c = 6; return a ? (b ? 1 : 2) : c;");
    // **A result wider than `int`.** 015 §2.1 types the slot as the *result* type, and
    // nothing above could tell an `int` slot from a correct one — every arm fitted in 32
    // bits. `0x100000000` does not, so an `int` slot truncates it to 0.
    agree("long a = 1; long b = 0x100000000; long c = 2; return (int)((a ? b : c) >> 32);");
    agree("long a = 0; long b = 0x100000000; long c = 2; return (int)((a ? b : c) >> 32);");

    // Aggregate initializers (contract 19): a full one, a partial one — C11 6.7.9p21
    // zero-initializes the rest — and an array.
    agree("struct S { int a; int b; }; struct S s = {1, 2}; return s.a * 10 + s.b;");
    agree("struct S { int a; int b; }; struct S s = {7}; return s.a * 10 + s.b;");
    agree("int a[4] = {1, 2, 3, 4}; return a[0] + a[3];");
    agree("int a[4] = {5}; return a[0] * 10 + a[3];");
    agree("struct S { int a; int b; }; struct S s = {.b = 3}; return s.a * 10 + s.b;");
}

/// **`&x` and pointer dereference**, the gap §9 has been carrying since wave 95.
///
/// It is the blocker for two other things: the `chiero.h` intrinsics take `&x`, and
/// sign-versus-zero extension of an array index cannot be distinguished without a negative
/// index, which needs `&a[i]`.
#[test]
fn address_of_and_dereference_compute_what_gcc_computes() {
    agree("int x = 7; int *p = &x; return *p;");
    agree("int x = 7; int *p = &x; *p = 9; return x;");
    agree("int a[4]; a[0] = 1; a[2] = 3; int *p = &a[2]; return *p;");
    // A negative index through a pointer into the middle — this is what pins sign
    // extension, because a zero-extended −1 addresses four billion elements away.
    agree("int a[4]; a[1] = 5; a[2] = 6; int *p = &a[2]; return p[-1];");
    agree("struct S { int a; int b; }; struct S s; s.b = 4; int *p = &s.b; return *p;");
    // The address of a struct, passed through a pointer.
    agree("struct S { int a; int b; }; struct S s; s.a = 2; struct S *p = &s; return p->a;");
}

/// **An aggregate lvalue used as a value is its address, not a load of its bytes.**
///
/// CIR has no aggregate values (020 §1.4), so a `struct` or array read as a value can only
/// be its address. Lowering's *global* ident arm says so in as many words — "an array names
/// its own address; a scalar names its contents" — and returns the address whenever the
/// lowered type is `CTy::Ptr`. The **local** arm never got that guard, so it emitted
/// `load ptr` and handed on the object's first eight bytes *as a pointer*.
///
/// Nothing caught it, because the one aggregate-copy path with coverage is `y = x`, which
/// goes through `lvalue_addr` and never through here. Copy-*initialization*, array decay
/// from a local, a by-value argument and an aggregate `return` all do — four shapes, one
/// missing guard. This is wave 121's lesson from the other side again: the fix reached the
/// global path and stopped.
///
/// gcc is the oracle rather than a shape assertion on purpose. `load ptr` is perfectly
/// well-formed CIR that the verifier accepts; only the number it computes is wrong.
#[test]
fn an_aggregate_lvalue_is_an_address_not_a_load() {
    // **Copy-initialization**, the case `y = x` does not reach. Both fields, so a copy
    // that moved only the first eight bytes by luck still fails.
    agree(
        "struct S { int a; int b; }; struct S x; x.a = 1; x.b = 2; struct S y = x; \
         return y.a * 10 + y.b;",
    );
    // Larger than a pointer, so no width coincidence can make a `load ptr` look right.
    agree(
        "struct S { int a; int b; int c; int d; }; struct S x; x.a = 1; x.b = 2; x.c = 3; \
         x.d = 4; struct S y = x; return y.a * 1000 + y.b * 100 + y.c * 10 + y.d;",
    );
    // **Array decay from a local.** `int *p = a;` must give `a`'s address; loading `a`'s
    // first eight bytes instead yields the pointer 0x0000000900000005 for the array below.
    agree("int a[4]; a[0] = 5; a[1] = 9; int *p = a; return p[1] - p[0];");
    // A write *through* the decayed pointer, so an alias that reads plausibly but points
    // elsewhere is caught too.
    agree("int a[3]; a[0] = 1; int *p = a; p[2] = 8; return a[2] - a[0];");
    // The same decay for a `char` array, where the first eight bytes are the whole object.
    agree("char a[8]; a[0] = 3; a[7] = 4; char *p = a; return p[7] * 10 + p[0];");
    // And the copy is a **copy**: mutating the source afterwards must not move the
    // destination. An initializer that aliased instead of copying passes every case above.
    agree(
        "struct S { int a; int b; }; struct S x; x.a = 1; x.b = 2; struct S y = x; \
         x.a = 9; return y.a * 10 + y.b;",
    );
    // **The struct is read somewhere other than directly under an initializer.** Without
    // this, the whole fix can sit outside the `Ident` arm — special-case `ArrayDecay` in
    // the cast lowering, source the copy-init `CopyMem` from `lvalue_addr`, and all six
    // cases above pass while a struct used as a value in a comma, a conditional, an
    // argument or a `return` stays broken. The adversarial review built that mutation and
    // confirmed it survives, which is the whole reason these two are here.
    agree(
        "struct S { int a; int b; }; struct S x; x.a = 1; x.b = 2; struct S y = (0, x); \
         return y.a * 10 + y.b;",
    );
    agree(
        "struct S { int a; int b; }; struct S x; x.a = 1; x.b = 2; struct S z; z.a = 3; \
         z.b = 4; struct S y = (x.a ? x : z); return y.a * 10 + y.b;",
    );
    // **A union**, which 020 §1.4 names alongside structs and arrays and which nothing
    // above covers. `is_aggregate` matches `Ty::Record`, and a union is one — but that is
    // a fact about sema's representation, not something the rule should rest on unstated.
    agree("union U { int i; short h; }; union U u; u.i = 5; union U v = u; return v.i;");
}

/// **A pointer-typed global names its contents, like any other scalar.**
///
/// The global ident arm's guard is `matches!(ty, CTy::Ptr)`, under a comment reading "an
/// array names its own address; a scalar names its contents". The comment is right and the
/// code does not implement it: pointers are untyped in CIR (020 §2), so `CTy::Ptr` is the
/// lowered type of `int *gp` every bit as much as of `int a[4]`. A global pointer read as a
/// value therefore yielded **its own address** instead of the address it holds, and `*gp`
/// read the first four bytes of `gp` as an `int`.
///
/// Found while fixing the local arm, which needed the narrower predicate to avoid doing the
/// same thing to every scalar local. It is the wave-107/112/118/124 shape once more: a
/// comment claiming a property is not the property.
///
/// The prelude is what makes it testable at all — a global cannot be declared inside a
/// function body, so this defect sat outside the oracle's reach.
#[test]
fn a_pointer_global_names_its_contents_not_its_own_address() {
    agree_with("int x; int *gp;", "x = 7; gp = &x; return *gp;");
    // A write through it, so a read that happened to land somewhere plausible is caught.
    agree_with("int x; int *gp;", "x = 1; gp = &x; *gp = 9; return x;");
    // Indexed through the global pointer, which scales an offset off the loaded base.
    agree_with(
        "int a[4]; int *gp;",
        "a[0] = 5; a[2] = 8; gp = a; return gp[2] - gp[0];",
    );
    // **The array global must still name its own address** — the half the guard got right,
    // and the half a fix that simply deleted it would break.
    agree_with("int a[4]; ", "a[1] = 3; int *p = a; return p[1];");
    // A pointer *inside* a global struct, reached through a member rather than by name.
    agree_with(
        "int x; struct H { int *p; }; struct H h;",
        "x = 6; h.p = &x; return *h.p;",
    );
}

/// **An lvalue is loaded and stored at the type sema gave it**, not at an integer width.
///
/// The sibling of the case above, and the reason it could not be fixed on its own. Three
/// sites ask `raw_width_of` for an lvalue's CIR type — the `Member` and `Deref`/`Index`
/// arms of `expr`, and `lvalue_ty`, which decides the width of every `Store` — and
/// `raw_width_of` answers `32` for **everything that is not an integer**, by design: it
/// exists to report an integer's width. A pointer held in a struct member, in an array
/// element, or reached through a second pointer was therefore loaded and stored as an
/// `i32`, keeping half of it.
///
/// `lvalue_ty` gets it right for a plain local only because locals carry their declared
/// `CTy` in the frame; nothing else does, which is the same "one path has it, the other
/// does not" shape as wave 121 and as the two tests above.
///
/// Every case here is C that VPP writes constantly: a pointer inside a struct is what a
/// graph node's `vlib_buffer_t *` fields are, and `**pp` is every out-parameter.
#[test]
fn a_pointer_lvalue_keeps_its_width_wherever_it_lives() {
    // A pointer **inside a struct**, written and read back through the member.
    agree("struct H { int *p; }; int x = 6; struct H h; h.p = &x; return *h.p;");
    // A pointer **in an array element**, so the index path is checked as well as members.
    agree("int x = 5; int *pa[2]; pa[1] = &x; return *pa[1];");
    // A **pointer to a pointer**: `*pp` is itself a pointer-typed lvalue, so the `Deref`
    // arm has to load eight bytes to have anything to dereference again.
    agree("int x = 4; int *q = &x; int **pp = &q; return **pp;");
    // A write through the double pointer, which is what an out-parameter does.
    agree("int x = 1; int y = 9; int *q = &x; int **pp = &q; *pp = &y; return *q;");
    // The truncation is only visible when the two halves differ, so put something in the
    // high half: an `i32` store of this address keeps the low word and drops the rest.
    agree(
        "int a[2]; a[0] = 3; a[1] = 8; struct H { int *p; }; struct H h; h.p = &a[1]; \
           return *h.p - a[0];",
    );
    // **Integers must not change**, since `raw_width_of` is right about them and a fix
    // that routed everything through the sema type has to agree with it here.
    agree(
        "struct W { long v; short s; }; struct W w; w.v = 0x100000000; w.s = -300; \
           return (int)(w.v >> 32) + w.s;",
    );
    agree("long a[2]; a[1] = 0x400000000; return (int)(a[1] >> 32);");
    // And a **nested aggregate member read as a value** — an aggregate lvalue however it
    // is spelled, so the address rule cannot be an ident-only special case.
    agree(
        "struct I { int a; int b; }; struct O { struct I i; }; struct O o; o.i.a = 2; \
           o.i.b = 3; struct I y = o.i; return y.a * 10 + y.b;",
    );
    // An **array member decaying**, the same rule reached through `Member` rather than
    // through `Ident`. `s.a` is how every fixed-size buffer inside a VPP struct is spelled.
    agree(
        "struct S { int a[3]; }; struct S s; s.a[0] = 4; s.a[1] = 6; int *p = s.a; \
           return p[1] * 10 + p[0];",
    );
}

/// **A struct parameter is the callee's own copy.**
///
/// The prologue gives every parameter a slot of its lowered `CTy` and stores the incoming
/// value into it. For an aggregate that `CTy` is `CTy::Ptr` (020 §1.4), so a `struct pair`
/// parameter got an **eight-byte pointer slot**, the caller's address was stored into it,
/// and the body then read `p.lo` and `p.hi` out of *the pointer's own bytes*.
///
/// The caller is right — it passes the struct's address, which is what wave 132 made
/// `expr` do — and the wrongness is entirely on the callee's side. It is also the worst
/// failure mode there is: no fault, no finding, just a wrong number, which is wave 113's
/// rule exactly. `span_of(p)` with `p = {3, 8}` returns 28663.
///
/// C11 6.9.1p9 is explicit that a parameter is an object whose value is a *copy* of the
/// argument, so the callee needs a slot of the struct's size and a `CopyMem` into it. Every
/// VPP helper taking a `vlib_buffer_t` or a `struct pair` by value depends on it, and
/// `tests/corpus/owed/header_inline.c` is blocked on this alone now.
#[test]
fn a_struct_parameter_is_the_callees_own_copy() {
    const H: &str = "struct pair { int lo; int hi; };\n\
         static int span_of(struct pair p) { return p.hi - p.lo; }\n\
         static int first_of(struct pair p) { return p.lo; }\n\
         static int bump(struct pair p) { p.lo = p.lo + 100; return p.lo; }\n\
         static struct pair mk(int a, int b) { struct pair p; p.lo = a; p.hi = b; \
                                               return p; }\n";
    // Both fields read out of the parameter, so a slot holding only the low half fails.
    agree_with(H, "struct pair p; p.lo = 3; p.hi = 8; return span_of(p);");
    agree_with(H, "struct pair p; p.lo = 3; p.hi = 8; return first_of(p);");
    // **The copy is a copy.** A callee that received the caller's address and wrote through
    // it would compute every value above correctly and still corrupt the caller's struct —
    // C11 6.9.1p9 makes the parameter an object of its own.
    agree_with(
        H,
        "struct pair p; p.lo = 1; p.hi = 2; int r = bump(p); return r * 100 + p.lo;",
    );
    // Composed with the aggregate *return*, which is the shape every VPP header accessor
    // has and the one `header_inline.c` is written around.
    agree_with(H, "struct pair p = mk(4, 9); return span_of(p);");
    agree_with(
        H,
        "struct pair p = mk(3, 5); struct pair q = mk(p.hi, p.lo); return span_of(q) + q.lo;",
    );
    // Two struct parameters, so one slot cannot stand in for both.
    agree_with(
        "struct pair { int lo; int hi; };\n\
         static int cross(struct pair a, struct pair b) { return a.lo * 1000 + a.hi * 100 \
                                                         + b.lo * 10 + b.hi; }\n",
        "struct pair x; x.lo = 1; x.hi = 2; struct pair y; y.lo = 3; y.hi = 4; \
         return cross(x, y);",
    );
    // An **array inside the struct**, so the copy is of the layout's size rather than of
    // the fields lowering happens to know about.
    agree_with(
        "struct box { int v[3]; int tag; };\n\
         static int sum_of(struct box b) { return b.v[0] + b.v[1] + b.v[2] + b.tag; }\n",
        "struct box b; b.v[0] = 1; b.v[1] = 2; b.v[2] = 3; b.tag = 40; return sum_of(b);",
    );
    // A **nested member** passed by value, so the argument is not reached through an
    // `Ident`. The implementation review supplied this one after finding the whole
    // by-value shape was claimed in a commit message and covered by no fixture.
    agree_with(
        "struct I { int a; int b; }; struct O { struct I i; };\n\
         static int f(struct I s) { return s.a * 10 + s.b; }\n",
        "struct O o; o.i.a = 5; o.i.b = 6; return f(o.i);",
    );
}

/// **A pointer can be tested against null, in every way C spells it.**
///
/// `if (p)`, `if (p == 0)`, `while (p)`, `p ? a : b`, `p && q`, `!p` — none of them work.
/// Every one produces no state at all. `if (p == 0)` is the plainest null check in C and
/// VPP writes it thousands of times.
///
/// One cause, and it is wave 132's third defect in the one path that wave did not touch:
/// the comparison arm types its `Cmp` as `CTy::Int(width_of(lhs))`, and `width_of` reports
/// an *integer's* width and answers 32 for everything else. So a pointer operand got
/// `cmp ne i32`, which is a well-formed instruction about a value that is not there.
/// `truth_of` — which C conditions all funnel through — does the same thing.
///
/// `truth_of`'s own doc comment already states the rule this test needs: "C conditions are
/// compares unequal to 0, so the conversion is a comparison rather than a truncation".
/// The rule was right. It was written for integers and never asked what a pointer is.
///
/// **A conversion to `_Bool` is the same question**, and gets the same wrong answer from
/// the other direction: `cast_kind` picks `Trunc` for `Int(32) -> Int(1)`, so `b = 2` is
/// `2 & 1` = 0 and `b = 256` is 0. C11 6.3.1.2 makes the conversion `!= 0` — a comparison,
/// not a narrowing. `b = -1` gives 1 only because its low bit happens to be set.
#[test]
fn a_pointer_and_a_bool_are_tested_against_zero_not_truncated() {
    // **The explicit comparisons**, which are not conditions at all — they are `==` and
    // `!=` with a pointer operand, and the same `width_of` answers 32 for both.
    agree("int x; int *p = &x; return p != 0;");
    agree("int x; int *p = &x; return p == 0;");
    agree("int *p = 0; return p == 0;");
    agree("int x; int *q = &x; int *p = q; return p == q;");
    agree("int a[2]; return &a[1] != &a[0];");
    // **The pointer on the right.** `0 == p` is the same comparison written the other way
    // round, and the operand lowering keys off the *left* side — so a rule that asks only
    // `compare_ty(lhs)` types this `Int(32)` and loses it. A mutation dropping the
    // either-side test survived the whole suite until these three existed.
    agree("int x; int *p = &x; return 0 == p;");
    agree("int x; int *p = &x; return 0 != p;");
    agree("int *p = 0; return 0 == p;");
    // Every C construct that takes a condition.
    agree("int x; int *p = &x; if (p) return 7; return 9;");
    agree("int *p = 0; if (p) return 7; return 9;");
    agree("int x; int *p = &x; while (p) { return 7; } return 9;");
    agree("int x; int *p = &x; int n = 0; for (; p; ) { n = 5; break; } return n;");
    agree("int x; int *p = &x; return p ? 7 : 9;");
    agree("int *p = 0; return p ? 7 : 9;");
    agree("int x; int *p = &x; return !p;");
    agree("int *p = 0; return !p;");
    // Short-circuit, where the pointer is the operand that decides whether the other side
    // runs at all — so a wrong answer here also changes what is evaluated.
    agree("int x; int *p = &x; int *q = 0; return (p && !q) ? 1 : 0;");
    agree("int *p = 0; int n = 0; if (p && (n = 5)) { } return n;");
    // An **array** in a condition, which decays and is therefore never null.
    agree("int a[2]; if (a) return 7; return 9;");
    // **A conversion to `_Bool` is `!= 0`, not a truncation.** 256 and 2 have a zero low
    // bit, so a truncation reports false for a plainly true value.
    agree("_Bool b; b = 2; return b;");
    agree("_Bool b; b = 256; return b;");
    agree("_Bool b = 2; return b;");
    agree("int x = 4; _Bool b = x; return b;");
    agree("int x = 0; _Bool b = x; return b;");
    agree("_Bool b = (_Bool)2; return b;");
    agree("long l = 0x100000000; _Bool b = l; return b;");
    // A pointer converted to `_Bool`, which is both halves of this test at once.
    agree("int x; int *p = &x; _Bool b = p; return b;");
    agree("int *p = 0; _Bool b = p; return b;");
    // `b = -1` passes today by luck — its low bit is set. Kept so a fix that reaches only
    // the even values is still wrong here.
    agree("_Bool b; b = -1; return b;");
}

/// **A load yields a value of its declared width, not of the bytes it read.**
///
/// The engine reads `size_of_cty(ty)` bytes and hands the term back as-is, so a load of a
/// type narrower than its storage comes back too wide. `_Bool` is the case C actually has:
/// `sizeof(_Bool) == 1` but its CIR type is `Int(1)`, so `load i1` produced an eight-bit
/// term and `add i1 %3, %4` reached the solver with operands of 8 and 1. That is an
/// `assert_eq!` in `chiero-solver`, so `_Bool b = 0; b += 1;` **panics the whole run** —
/// worse than any wrong answer, because nothing above it can catch it and every other
/// finding in the same run is lost with it.
///
/// The branch immediately beside it already gets this right: when the read produces
/// nothing, the invented symbol is `sort_of(ty)` — the *declared* width. Only the path
/// that succeeds disagreed with it.
#[test]
fn a_load_has_the_width_of_its_declared_type() {
    // The panic itself.
    agree("_Bool b = 0; b += 1; return b;");
    agree("_Bool b = 1; b -= 1; return b;");
    agree("_Bool b = 0; b++; return b;");
    // Reading one back is fine today; kept so a fix that narrows too far is caught.
    agree("_Bool b = 1; return b;");
    agree("_Bool b = 0; return b;");
    agree("int x = 7; _Bool b = x; return b;");
    // A `_Bool` in a struct, where the storage byte is surrounded by other fields — a
    // narrowing that read the neighbour's bits instead would pass every case above.
    agree("struct F { _Bool f; int n; }; struct F s; s.f = 1; s.n = 5; return s.f * 10 + s.n;");
    agree("struct F { _Bool a; _Bool b; }; struct F s; s.a = 1; s.b = 0; return s.a * 10 + s.b;");
}

/// **"No value form" means vectors and function designators too, not just records.**
///
/// `is_aggregate` matches `Ty::Record | Ty::Array`, while `cty`, `aggregate_size` and
/// `aggregate_size_of_ty` all include `Ty::Vector` — three predicates in one file that
/// disagree about the same type. So a vector reproduces, exactly, the defect wave 132
/// exists to kill: `v4si b = a;` emits `load ptr` of the vector's first eight bytes and
/// stores eight bytes where sixteen belong. `elem_size_of` knows only `Array | Ptr`, so
/// vector indexing scales by **one byte** — `a[1]` writes byte 1.
///
/// A **function designator** is the third thing CIR has no value form for (C11 6.3.2.1p4
/// makes it decay to a pointer, exactly as an array does), and `(*fp)(3)` emits `load ptr`
/// *at the function's address*. `fns[0](3)` and `s.f(3)` work, so it is the deref spelling
/// specifically.
///
/// `is_aggregate_expr`'s own doc comment says "something CIR has no value form for". The
/// rule was right and covered two of its three cases.
#[test]
fn a_vector_and_a_function_designator_have_no_value_form_either() {
    const V: &str = "typedef int v4si __attribute__((vector_size(16)));\n";
    // Copy-initialization, the shape that loaded eight of sixteen bytes.
    agree_with(
        V,
        "v4si a; a[0] = 1; a[1] = 2; v4si b = a; return b[0] * 10 + b[1];",
    );
    // **The high half**, which an eight-byte copy drops entirely.
    agree_with(
        V,
        "v4si a; v4si b; a[0] = 1; a[3] = 4; b = a; return b[0] + b[3];",
    );
    // Indexing, which scaled by one byte rather than by the element size.
    agree_with(V, "v4si a; a[2] = 7; return a[2];");
    agree_with(
        V,
        "v4si a; a[0] = 1; a[1] = 2; a[2] = 3; return a[1] * 10 + a[2];",
    );
    // A function designator dereferenced, which is `(*fp)(x)` — legal C and common in
    // dispatch tables written defensively.
    agree_with(
        "int twice(int x) { return x + x; }\n",
        "int (*fp)(int) = twice; return (*fp)(3);",
    );
    agree_with(
        "int twice(int x) { return x + x; }\n",
        "int (*fp)(int) = twice; int (*g)(int) = *fp; return g(4);",
    );
    // The two spellings that already worked, so the fix extends rather than replaces.
    agree_with(
        "int twice(int x) { return x + x; }\n",
        "int (*fns[2])(int); fns[0] = twice; return fns[0](3);",
    );
    agree_with(
        "int twice(int x) { return x + x; }\nstruct D { int (*f)(int); };\n",
        "struct D d; d.f = twice; return d.f(5);",
    );
}

/// **An aggregate assignment works whatever the right-hand side is.**
///
/// `assign`'s aggregate path takes `lvalue_addr(rhs)` and returns `Undef` when that is
/// `None` — so for any right-hand side that is not a plain lvalue the `CopyMem` is *not
/// emitted at all*. The assignment vanishes from the CIR, along with the side effects of
/// whatever was on the right. `y = (0, x);` lowers to one `addrlocal` and nothing else.
///
/// `y = mk(1, 2);` is the case that matters: a struct-returning call is how C is written,
/// and it is broken in exactly the same way. So is a conditional, and so is the chain
/// `w = y = x;` — an assignment's own result is not an lvalue in C, and `assign` returns
/// the destination address for precisely this reason, which nothing was reading.
///
/// The *initializer* form `struct S y = (0, x);` works, because `local_decl` goes through
/// `expr` rather than `lvalue_addr`. Wave 132 made `expr` yield the address for an
/// aggregate, which is what makes the same fallback correct here.
#[test]
fn an_aggregate_assignment_takes_any_right_hand_side() {
    const P: &str = "struct S { int a; int b; };\n\
         static struct S mk(int a, int b) { struct S s; s.a = a; s.b = b; return s; }\n";
    // **A struct-returning call**, the shape every VPP accessor has.
    agree_with(P, "struct S y; y = mk(1, 2); return y.a * 10 + y.b;");
    // A comma, whose left operand must still be evaluated for its side effects.
    agree_with(
        P,
        "struct S x; x.a = 1; x.b = 2; int t = 0; struct S y; y = (t = 5, x); \
         return y.a * 10 + y.b + t;",
    );
    // A conditional, both ways round, so neither arm is the one that happens to work.
    agree_with(
        P,
        "struct S x; x.a = 1; x.b = 2; struct S z; z.a = 5; z.b = 6; struct S y; \
         y = (x.a ? x : z); return y.a * 10 + y.b;",
    );
    agree_with(
        P,
        "struct S x; x.a = 1; x.b = 2; struct S z; z.a = 5; z.b = 6; struct S y; \
         y = (x.a - 1 ? x : z); return y.a * 10 + y.b;",
    );
    // **A chain.** C makes an assignment's value the stored value, so `w = y = x` copies
    // twice — and both destinations must end up with the fields, not one.
    agree_with(
        P,
        "struct S x; x.a = 1; x.b = 2; struct S y; struct S w; w = y = x; \
         return w.a * 1000 + w.b * 100 + y.a * 10 + y.b;",
    );
    // The plain-lvalue right-hand sides that already worked, so the fallback does not
    // replace the path that was right.
    agree_with(
        P,
        "struct S a[2]; a[0].a = 3; a[0].b = 4; struct S y; y = a[0]; return y.a * 10 + y.b;",
    );
    agree_with(
        P,
        "struct S x; x.a = 7; x.b = 8; struct S *p = &x; struct S y; y = *p; \
         return y.a * 10 + y.b;",
    );
}

/// **Pointer arithmetic is scaled `PtrAdd`, not integer `Add`.**
///
/// 020 says PtrAdd-not-Add by name, and the *index* path obeys it: `a[1]` lowers to `sext`,
/// `mul 4`, `ptradd`. The `Binary` arm of `expr` does not know its operand is a pointer at
/// all, so `*(a + 1)` lowers to `add i32 %addr, 1i32` — thirty-two bits wide on a
/// sixty-four-bit address, and unscaled, so it would address byte 1 rather than element 1
/// even if the width were right.
///
/// Every form is broken, not just the one recorded in HANDOFF §9: `p + n`, `n + p`, `p - n`,
/// `p - q`, `p += n` and `p++`. The array-subscript spelling is the *only* one that works,
/// which is why nothing caught it — `a[i]` is what every fixture had been written with.
///
/// `p - q` is its own operation rather than a variation: C11 6.5.6p9 makes the difference of
/// two pointers a count of *elements*, so it is a byte subtraction divided by the element
/// size, and an implementation that returns the byte distance passes nothing here.
#[test]
fn pointer_arithmetic_is_scaled_and_pointer_wide() {
    // The two spellings of the same thing, so neither operand order is special-cased.
    agree("int a[3]; a[1] = 7; return *(a + 1);");
    agree("int a[3]; a[1] = 7; return *(1 + a);");
    // Through a pointer variable rather than off an array's address.
    agree("int a[3]; a[1] = 7; int *p = a; return *(p + 1);");
    // The pointer itself carrying the offset, so the scaling has to survive the store.
    agree("int a[3]; a[2] = 9; int *p = a + 2; return *p;");
    // Subtraction, and from the middle, which a fix that only handled `+` would miss.
    agree("int a[3]; a[0] = 4; int *p = a + 2; return *(p - 2);");
    // Compound assignment and increment, which go through their own lowering paths.
    agree("int a[3]; a[1] = 5; int *p = a; p += 1; return *p;");
    agree("int a[3]; a[1] = 5; int *p = a; p++; return *p;");
    agree("int a[3]; a[1] = 6; int *p = a + 2; --p; return *p;");
    // **The element size is not 4.** With `int` everywhere, a lowering that scaled by a
    // hard-coded 4 — or that got the width right and the scale wrong — passes every case
    // above. `char` scales by 1 and a struct by its layout.
    agree("char c[4]; c[2] = 6; char *p = c + 2; return *p;");
    agree(
        "struct S { int a; int b; }; struct S s[3]; s[2].a = 8; struct S *p = s + 2; \
         return p->a;",
    );
    // **Pointer difference**: a count of elements, not of bytes (C11 6.5.6p9). For `int`
    // the byte answer is 8 and the right answer is 2.
    agree("int a[4]; int *p = a + 3; int *q = a + 1; return (int)(p - q);");
    agree("char c[4]; char *p = c + 3; char *q = c + 1; return (int)(p - q);");
    // And a **negative** result, which pins the signedness of the division.
    agree("int a[4]; int *p = a + 1; int *q = a + 3; return (int)(p - q);");
    // **The pointer lvalue is not always a plain local.** `assign` and `inc_dec` reach
    // `displace` through `lvalue_ty`, whose local fast path is the one half that was
    // already honest — so a pointer in a struct member, in an array element, or at file
    // scope goes down the *other* half. The implementation review supplied these three
    // after noting the cases above pin only the local.
    agree(
        "struct H { int *p; }; int a[2]; a[0] = 1; a[1] = 9; struct H h; h.p = a; \
         h.p += 1; return *h.p;",
    );
    agree("int a[2]; a[0] = 1; a[1] = 9; int *pa[1]; pa[0] = a; pa[0] += 1; return *pa[0];");
    agree_with(
        "int a[2]; int *gp;",
        "a[0] = 3; a[1] = 8; gp = a; gp++; return *gp;",
    );
}

/// **Statement expressions and VLAs**, checked for what they compute.
///
/// A statement expression's value is the last expression statement's, and its side
/// effects happen once — the count is a shape property but the *value* is not, and
/// `({ a; b; })` yielding `a` instead of `b` has an identical shape.
#[test]
fn statement_expressions_and_vlas_compute_what_gcc_computes() {
    agree("int x = ({ int t = 3; t + 1; }); return x;");
    agree("int x = ({ 1; 2; 3; }); return x;");
    agree("int a = 2; int x = ({ int t = a * 5; t - 1; }); return x;");
    // Nested, so the two blocks' values cannot be confused.
    agree("int x = ({ int t = ({ 2; }); t * 10; }); return x;");
    // A VLA, indexed and summed.
    agree("int n = 3; int v[n]; v[0] = 1; v[2] = 5; return v[0] * 10 + v[2];");
    agree(
        "int n = 4; int v[n]; int t = 0; for (int i = 0; i < n; i++) { v[i] = i; t += v[i]; } return t;",
    );
}

/// **`goto` into and back into a scope**, checked for what it computes.
///
/// Contract 9c's assertions are about markers, and a marker is invisible to a program's
/// result — so the oracle is what says the *code* on those paths still runs correctly
/// when lowering re-scopes the jump.
#[test]
fn goto_into_and_back_into_a_scope_computes_what_gcc_computes() {
    agree("int n = 1; if (n) goto inner; { int a = 5; inner: ; return n + 1; } ");
    agree("int n = 0; if (n) goto inner; { int a = 5; inner: ; return a + 1; } ");
    agree("int n = 3; int t = 0; { inner: ; t += n; n--; } if (n > 0) goto inner; return t;");
    agree("int n = 2; if (n) goto deep; { int a = 1; { int b = 2; deep: ; return n; } } ");
}

/// **Wide case ranges and string literals**, checked for what they compute.
///
/// 020 contract 14 requires a guarded chain and an enumerated range to produce
/// *identical* execution results, which is a claim about values and not about shapes —
/// the two lower to completely different CIR.
#[test]
fn wide_ranges_and_string_literals_compute_what_gcc_computes() {
    // Below the threshold: enumerated. Above it: a guarded chain. Same answers.
    agree("int n = 3; switch (n) { case 1 ... 4: return 1; default: return 0; }");
    agree("int n = 9; switch (n) { case 1 ... 4: return 1; default: return 0; }");
    agree("int n = 5000; switch (n) { case 1 ... 10000: return 1; default: return 0; }");
    agree("int n = 20000; switch (n) { case 1 ... 10000: return 1; default: return 0; }");
    agree("int n = 1; switch (n) { case 1 ... 10000: return 1; default: return 0; }");
    agree("int n = 10000; switch (n) { case 1 ... 10000: return 1; default: return 0; }");
    // No probe for "an exact case beside a wide range": gcc **rejects overlapping case
    // values**, so the question of which one wins cannot arise in legal C. The oracle
    // established that by refusing to compile the fixture that assumed otherwise, which
    // is what panicking on a rejected fixture is for — a skip would have hidden it.
    // A string literal's bytes are readable.
    agree("const char *s = \"hi\"; return s[0] + s[1];");
    agree("const char *s = \"a\\nb\"; return s[1];");
    agree("const char *s = \"abc\"; return s[3];");
}

/// A guard that the oracle can **see a difference at all**.
///
/// Every assertion above is an equality, and a comparison that always compared equal
/// would pass all of them — the same vacuity that has cost this project a fixture in
/// several waves. So: two fixtures whose answers genuinely differ must be reported as
/// differing by the same machinery.
#[test]
fn the_oracle_can_observe_a_disagreement() {
    let a = chiero_answer("", "signed char c = -1; int i = c; return i;");
    let b = chiero_answer("", "unsigned char c = 255; int i = c; return i;");
    assert_eq!(a, Some(-1));
    assert_eq!(b, Some(255));
    assert_ne!(
        a, b,
        "the two extensions must give different answers, or this file's equalities \
         are comparing a constant against itself"
    );
    if let (Ok(ga), Ok(gb)) = (
        gcc_answer("", "signed char c = -1; int i = c; return i;"),
        gcc_answer("", "unsigned char c = 255; int i = c; return i;"),
    ) {
        assert_ne!(ga, gb, "and gcc agrees they differ");
    }
}

/// **Compound assignment to a `_Bool` promotes, operates, then converts.**
///
/// C11 6.5.16.2p3 makes `b += e` behave as `b = b + e` except that `b` is evaluated once, and
/// 6.5p4 promotes both operands of `+` — so the addition happens in `int` and the *result* is
/// converted back. chiero converted the right-hand side to the lvalue's type first:
///
/// ```text
///   _Bool b = 1; b += -1;      gcc 0    chiero 1
///   _Bool b = 0; b += -1;      gcc 1    chiero 1
/// ```
///
/// **The two orders agree for every other integer type**, which is why this survived every
/// differential channel for a hundred waves. Converting to `char` is a truncation and truncation
/// commutes with `+`, `-` and `*`: `(char)(1 + 300)` and `1 + (char)300` are both 45. Converting to
/// `_Bool` is `!= 0`, which commutes with nothing — so `_Bool` is the only type where the
/// difference is observable, and chiero's answer stopped depending on `b` at all.
///
/// Found by the generated-program soak once the grammar learned `do`-`while`: a `_Bool`
/// accumulator in a loop alternates, so the wrong answer diverged from the right one every
/// iteration instead of once.
#[test]
fn compound_assignment_to_a_bool_converts_the_result_and_not_the_operand() {
    // Both starting values, because a fix that converts the *operand* gives 1 for both and a fix
    // that ignores the operand gives 0 for both. Only the pair pins the arithmetic.
    agree("_Bool b = 1; b += -1; return (int)b;");
    agree("_Bool b = 0; b += -1; return (int)b;");
    // The alternation the soak actually tripped over.
    agree("_Bool b = 1; for (int i = 0; i < 3; i++) { b += -1; } return (int)b;");
    // Every operator that takes a compound form, since the lowering is one path.
    agree("_Bool b = 1; b -= 1; return (int)b;");
    agree("_Bool b = 1; b *= 2; return (int)b;");
    agree("_Bool b = 1; b += 2; return (int)b;");
    // And a narrow integer, to hold the case where the two orders agree — a fix must not
    // "correct" these.
    agree("signed char c = 1; c += 300; return (int)c;");
    agree("unsigned char c = 200; c += 100; return (int)c;");
    agree("short s = 32767; s += 1; return (int)s;");
}

// -------------------------------------------------------------------------------------------
// `long double` — 023 §7's contract, executable.
//
// x87's 80-bit format has no Rust primitive, so the engine does not model arithmetic on it and
// says so: a run degrades to `Fidelity::Unknown` and records assumptions naming the operation
// ("`FDiv` is not modeled", "`FpExt 64 -> 80 on a symbolic operand` is not modeled"). That is
// the honest answer and §9 carries the implementation as a milestone.
//
// **Nothing anywhere tested it.** No file outside this block mentions `long double` at all, so
// the one thing that must never happen — a wrong answer where a gap was declared — was
// unguarded. A refactor treating `X87_80` as `F64` would compute `1.0L / 3.0L` in 53 bits of
// mantissa, disagree with gcc, and pass the suite.
//
// This is the baseline the milestone needs: whatever 80-bit arithmetic eventually lands, a
// *partial* implementation cannot lie on the way.
// -------------------------------------------------------------------------------------------

/// **A `long double` computation gives gcc's answer or declares that it cannot.**
///
/// Written as a disjunction on purpose. Asserting "it degrades" would freeze today's limitation
/// into a requirement and fail the moment the milestone lands; asserting "it agrees" fails now.
/// What 023 §7 actually promises is the disjunction, and that is what holds across the change.
///
/// The declaration has to *name the operation*, not merely be non-`Exact` — the distinction wave
/// 222 drew for `max_depth`, where "reached" without a value was the defect.
#[test]
fn long_double_arithmetic_agrees_with_gcc_or_says_it_is_unmodelled() {
    for body in [
        "long double x = 1.0L; x = x / 3.0L; return (int)(x * 300000);",
        "long double a = 1.0L, b = 3.0L; return (int)(a / b > 0.333);",
        "double d = 0.1; long double l = d; return (int)(l == (long double)0.1);",
        // The defect wave 237 found and wave 238 fixed: an unmodelled operation used to leave the
        // destination holding the stale value, so this answered with a *number* that was wrong
        // rather than declaring a gap.
        "long double x = 2.0L; x = x / 3.0L; return (int)x;",
        // **The body that still takes the second branch — and it has now been replaced twice.**
        // Wave 242 put a subnormal here when division made every other body produce a value; wave
        // 244 implemented subnormals and made *that* body produce one too. Each time the cause was
        // a capability landing, which is the point: this test's second half measures what is left,
        // so it necessarily decays as the list shortens.
        //
        // What is left is **narrowing `long double` to `float`**. Chiero rounds `f80` to `f64` and
        // lets the target round that to `f32`, and double rounding differs from a single correctly
        // rounded step for some values — so `fcast` refuses rather than answering, and the comment
        // there says why. gcc narrows in one step, so the outcomes genuinely differ, which is
        // exactly the situation this test exists to adjudicate.
        "long double x = 0x1.0000000000000002p0L; float f = (float)x; return (int)(f == 1.0f);",
        // **Hexadecimal, and wave 239 is why.** This was `1e300L * 1e10L > 1e309L`, and the moment
        // multiplication started working it returned a wrong *number*: `1e309L` is a decimal
        // literal, decimal literals are still parsed at `f64` precision, and 1e309 overflows `f64`
        // to infinity — so the comparison asked whether 1e310 exceeds infinity and answered no.
        // The rounding was harmless while arithmetic was a gap and is not any more, which is why
        // §9 moved it to the front.
        "long double x = 0x1p1000L; x = x * 0x1p1000L; return (int)(x > 0x1p1999L);",
    ] {
        let expected = match gcc_answer("", body) {
            Ok(v) => v,
            Err(Oracle::NoGcc) => {
                eprintln!("skipping `{body}`: gcc not on PATH");
                SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            Err(Oracle::Broken(why)) => panic!("the oracle is broken, not absent: {why}"),
        };
        let src = format!("int probe(void) {{ {body} }}");
        let m = harness::lower(&src);
        let mut arena = TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);
        let got = r
            .states()
            .iter()
            .find_map(|s| s.return_value_bits(&mut arena))
            .map(|b| b as u32 as i32);
        if let Some(v) = got {
            assert_eq!(
                v, expected,
                "`{body}`: chiero produced a value, so it must be gcc's — a wrong answer is \
                 the one outcome §7 forbids"
            );
            continue;
        }
        let why: Vec<String> = r
            .states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| x.detail.clone())
            .collect();
        assert!(
            why.iter().any(|d| d.contains("not modeled")),
            "`{body}`: no value and no statement of what is unmodelled is a silent gap: {why:?}"
        );
    }
}

/// **The parts that do work must keep working.** `sizeof` and `_Alignof` on `long double`.
///
/// sema knows x86-64's layout (16 bytes, 16-byte aligned) even though the engine models no
/// arithmetic, and those are the two answers a program can get right today. Separate from the
/// disjunction above because they are not a gap at all: a run that degraded *these* would be a
/// regression dressed as honesty.
#[test]
fn long_double_layout_is_exact() {
    agree("return (int)sizeof(long double);");
    agree("return (int)_Alignof(long double);");
    agree("struct S { char c; long double d; }; return (int)sizeof(struct S);");
}

/// **`(int)` of a `long double` agrees with gcc.**
///
/// The x87 milestone's first observable step, and section 9's step 2. Wave 229's encoding fix
/// already made a value survive a store and a load — `long double y = x;` is `Exact` — but every
/// int-returning fixture then dies at `FpToSi 80 -> 32`, so nothing about an `f80` value could be
/// *seen*. This is the conversion that makes it visible.
///
/// **Truncation toward zero, exactly, without going through `f64`.** The obvious implementation
/// decodes the 80-bit pattern into an `f64` and truncates that, which is what `as_f64` already does
/// for 32 and 64 — and it is wrong for a 64-bit target: `f64` carries 53 bits of significand where
/// x87 carries 64, so `(long long)` of a large `long double` would come back off by a little. That
/// is precisely the "wrong answer where a gap was declared" that 023 §7 forbids and wave 228 pinned
/// against. The integer comes out of the significand directly.
///
/// Both signs and both sides of 1.0, because truncation toward zero is not flooring: `-2.5L` is
/// `-2`, not `-3`, and a fix using `floor` passes the positives.
#[test]
fn long_double_to_int_agrees_with_gcc() {
    agree("long double x = 1.0L; return (int)x;");
    agree("long double x = 2.5L; return (int)x;");
    agree("long double x = -2.5L; return (int)x;");
    agree("long double x = 0.0L; return (int)x;");
    agree("long double x = -0.5L; return (int)x;");
    // Larger than 2^53, where an `f64` round trip stops being exact for a 64-bit target. The
    // `int` here still fits, so this pins that the path is exact rather than merely plausible.
    agree("long double x = 123456789.0L; return (int)x;");
    // An unsigned target, which takes the other arm.
    agree("long double x = 42.5L; return (int)(unsigned)x;");
    // A 64-bit target, which takes the same arm with a wider mask.
    agree("long double x = 1099511627776.5L; return (int)(long long)x;");
    // **The fixture that is deliberately not here.** `2^62 + 1` would prove the conversion never
    // rounds through `f64` — and it fails for a reason upstream of the conversion: lowering carries
    // a float literal as an `f64`, so `4611686018427387905.0L` reaches the encoder already rounded.
    // chiero emits significand `0x8000000000000000` where gcc stores `0x8000000000000001`.
    //
    // The conversion below it *is* exact — it reads the significand and never builds an `f64` — but
    // that cannot be demonstrated end to end until a literal can carry more than 53 bits. §9 has it
    // as the next step, with the two patterns as evidence.
}

/// **A hexadecimal floating literal is a float, not an integer** (C99 6.4.4.2).
///
/// `0x1p3` is 8.0. `float_literal` recognises the syntax — it checks for `0x` and a `p` exponent —
/// and then hands the text to Rust's `f64` parser, which rejects hex float syntax and returns
/// `None`. Lowering falls through to the integer path, emits `Const::Int` where the declared type is
/// `Float(F64)`, and the verifier refuses the whole function:
///
/// ```text
///   `probe` lowered to CIR the verifier rejects
///   (cast source operand is Int(32), declared Float(F64))
/// ```
///
/// Honest — a refusal, not a wrong number — and it costs every program containing one, which for
/// hex floats means anything that specifies a bit pattern deliberately. That is also why it belongs
/// to the x87 milestone: a hex literal is the only way to write an exact `long double` in source,
/// and §9's next step is that decimal `long double` literals are rounded through `f64`.
///
/// All three suffixes, because the kind is chosen by the suffix and the value by the digits, and a
/// fix that handled one would leave the others refusing.
#[test]
fn a_hex_float_literal_agrees_with_gcc() {
    agree("return (int)0x1p3;");
    // A fraction, so the mantissa digits after the point are exercised rather than just the
    // exponent.
    agree("return (int)0x1.8p1;");
    // A negative exponent, which divides rather than multiplies.
    agree("return (int)(0x1p-1 + 0.5);");
    // The suffixes: `f` narrows to `float`, `L` widens to `long double`.
    agree("return (int)0x1.8p1f;");
    agree("return (int)0x1p3L;");
    // In an expression, where the integer fallthrough produced a plausible number rather than a
    // refusal — `0x1p10` read as an integer is not 1024.
    agree("return (int)(0x1p10 + 1);");
    // Upper-case `P` and `X`, which C admits and a parser keyed on the lower-case spelling misses.
    agree("return (int)0X1P3;");
    // An explicitly positive exponent. C allows the sign and it means nothing, so a parser that
    // hands `+3` to an integer parse without stripping the sign fails on a literal that looks
    // ordinary — mutation found this one missing.
    agree("return (int)0x1p+3;");
}

/// **An integral decimal `long double` literal is exact.**
///
/// `float_literal` parses every float literal at `f64` precision, so a `long double` loses the
/// bottom eleven bits of its significand before `x87_bits` re-encodes it. §9's evidence is the
/// smallest case that shows it:
///
/// ```text
///   4611686018427387905.0L   (2^62 + 1)
///   chiero  fconst:f80:0x403d8000000000000000
///   gcc                     0x403d8000000000000001
/// ```
///
/// Now that `(long long)` of an `f80` works exactly (wave 230), the loss is observable end to end:
/// the `+ 1` is gone by the time anything can look at it.
///
/// **Integral literals only, and that is a deliberate line.** Correct decimal-to-binary rounding in
/// general is famously hard — Rust's own `f64` parser is a substantial algorithm — and there is no
/// `f80` to lean on. An *integer*, though, needs no rounding decision at all when it fits in
/// sixty-four bits of significand: the digits are the value. That covers the case §9 recorded, and a
/// fraction stays at `f64` precision until someone writes the general parser, which §9 keeps.
#[test]
fn an_integral_long_double_literal_keeps_all_its_bits() {
    // 2^62 + 1: representable in x87's 64-bit significand, not in `f64`'s 53.
    agree("long double x = 4611686018427387905.0L; return (int)(long long)x;");
    // 2^53 + 1 is the smallest integer `f64` cannot represent, so it is the tightest case.
    agree("long double x = 9007199254740993.0L; return (int)(long long)x;");
    // **Not here yet, and deliberately:** `long double x = 9007199254740993;` — an *integer*
    // literal converted to `long double` — goes through `SiToFp` to width 80 rather than the float
    // literal path, and that conversion is still an unmodelled gap. §9 has it as the next step,
    // together with the note that the normalization it needs already exists in `chiero-sema` and
    // wants a shared home before it is written a second time in the engine.
    // Small integers must not regress: the exponent and the significand both move for these.
    agree("long double x = 1.0L; return (int)(long long)x;");
    agree("long double x = 3.0L; return (int)(long long)x;");
    // And a fraction still works, at whatever precision it has — this pins that the new path does
    // not swallow the old one.
    agree("long double x = 2.5L; return (int)x;");
}

/// **An integer converted to `long double` keeps all its bits.**
///
/// `long double x = 9007199254740993;` is an *integer* literal converted by `SiToFp`, not a floating
/// literal — a different path to the same value, and the one wave 232 had to leave out. 2^53 + 1 is
/// the smallest integer `f64` cannot represent, so a conversion that detours through `f64` loses the
/// `+ 1` and `(long long)` brings back an even number.
///
/// Exact for the same reason the integral literal is: an integer is its own significand once
/// normalized, so there is no rounding decision. Both signednesses, because `SiToFp` and `UiToFp`
/// are separate opcodes and a fix for one leaves the other refusing.
#[test]
fn an_integer_converted_to_long_double_keeps_all_its_bits() {
    agree("long double x = 9007199254740993; return (int)(long long)x;");
    agree("long long n = 9007199254740993; long double x = n; return (int)(long long)x;");
    // Unsigned, which takes `UiToFp`.
    agree(
        "unsigned long long n = 18014398509481985u; long double x = n; return (int)(long long)x;",
    );
    // Negative, where the sign is the conversion's rather than an `fneg`'s.
    agree("long long n = -9007199254740993; long double x = n; return (int)(long long)x;");
    // Small and zero, which move both exponent and significand.
    agree("long double x = 3; return (int)(long long)x;");
    agree("long double x = 0; return (int)(long long)x;");
}

/// **A `double` or `float` widened to `long double` keeps its value.**
///
/// `FpExt` to width 80 was the last conversion *into* x87 still refusing, and it is exact by
/// construction: every `f64` is representable in a format with a wider exponent and a wider
/// significand, so there is no rounding decision — which is why it comes before `FpTrunc`, where
/// there is one.
///
/// Both source widths, because `f32 -> f80` and `f64 -> f80` are the same opcode at different
/// `fw`, and a fix keyed on 64 leaves `float` refusing.
#[test]
fn a_float_widened_to_long_double_agrees_with_gcc() {
    agree("double d = 2.5; long double x = d; return (int)x;");
    agree("float f = 2.5f; long double x = f; return (int)x;");
    agree("double d = -2.5; long double x = d; return (int)x;");
    // A value needing most of `f64`'s significand, so a widening that dropped bits shows up once
    // it is truncated back to an integer.
    agree("double d = 9007199254740992.0; long double x = d; return (int)(long long)x;");
    // Through a cast rather than an assignment — the same conversion, spelled the way a programmer
    // usually writes it.
    agree("double d = 7.75; return (int)(long double)d;");
    // Zero and a denormal-adjacent small value, where the exponent path differs.
    agree("double d = 0.0; long double x = d; return (int)x;");
    agree("double d = 0.5; long double x = d; return (int)x;");
}

/// **Narrowing `long double` to `double` agrees with gcc.**
///
/// **This does *not* yet exercise the tie-break, and the fixtures below say otherwise — read this
/// first.** Mutation found it: replacing the rounding with a truncation changed nothing, because
/// `hex_float` returns an `f64`, so a literal needing more than fifty-three significant bits is
/// rounded *at parse time* and the conversion then has nothing left to decide.
///
/// ```text
///   0x1.00000000000008p0L  (1 + 2^-53)    -> fconst:f80:0x3fff8000000000000000   (= 1.0)
///   0x1.00000000000018p0L  (1 + 3·2^-53)  -> fconst:f80:0x3fff8000000000001000   (= 1 + 2^-51)
/// ```
///
/// Both were rounded to even before `FpTrunc` ran, which is why the fixtures pass and why they pass
/// for the wrong reason. They are kept because agreeing with gcc is still worth asserting, and
/// renamed so the name does not claim the coverage. §9 carries the fix: `hex_float` has to hand out
/// a mantissa and a scale instead of an `f64`, exactly as `integral_float_literal` was changed to
/// hand out a `u64` in wave 233 — the same seam, found the same way.
///
/// The first x87 step with a *decision* in it. Everything so far was exact by construction —
/// widening cannot lose bits, an integer is its own significand — and this discards eleven bits of
/// significand, so IEEE-754's default rule has to be implemented rather than inherited.
///
/// **Truncating to an `int` cannot see rounding**, which is why these compare the narrowed `double`
/// against a literal instead: `f64` comparison is already modelled, so the result of the rounding is
/// observable directly. Every fixture is `== 1` under gcc, so a wrong rounding shows up as a `0`.
///
/// The ties *would* be the point, once the literals reach the conversion intact. `1 + 2^-53` sits
/// exactly halfway between `1.0` and the next `double`, and
/// round-half-up would give the wrong one — ties go to the candidate with an even last bit, which is
/// `1.0`. `(1 + 2^-52) + 2^-53` is the same distance from *its* two neighbours and rounds the other
/// way, because there the even one is above. A rule that always rounds half away from zero passes
/// the first and fails the second; one that always truncates fails both.
///
/// The literals are hexadecimal because that is the only spelling that reaches the bits exactly
/// (wave 231) — a decimal `long double` is still parsed at `f64` precision, which would defeat the
/// fixture before the conversion ever ran.
#[test]
fn narrowing_long_double_to_double_agrees_with_gcc() {
    // Exactly halfway, and the lower neighbour is even: rounds down to 1.0.
    agree("long double x = 0x1.00000000000008p0L; double d = x; return (int)(d == 1.0);");
    // Exactly halfway, and the lower neighbour is odd: rounds up.
    agree(
        "long double x = 0x1.00000000000018p0L; double d = x; \
         return (int)(d == 0x1.0000000000002p0);",
    );
    // Above and below halfway, where parity does not enter into it.
    agree(
        "long double x = 0x1.00000000000009p0L; double d = x; \
         return (int)(d == 0x1.0000000000001p0);",
    );
    agree("long double x = 0x1.00000000000007p0L; double d = x; return (int)(d == 1.0);");
    // A value `f64` holds exactly, which must come back unchanged.
    agree("long double x = 2.5L; double d = x; return (int)(d == 2.5);");
    // Past `f64`'s range: IEEE says infinity, and `1e400L` is representable in x87.
    agree("long double x = 0x1p2000L; double d = x; return (int)(d > 1e308);");
    // **`long double` to `float` is deliberately not here.** It is the same opcode discarding many
    // more bits, and the only route available rounds twice — once into `f64`, once into `f32` — which
    // differs from the single correctly-rounded answer for some values. So it stays a declared gap,
    // held by `long_double_arithmetic_agrees_with_gcc_or_says_it_is_unmodelled`, which already
    // carries the fixture and accepts either a right answer or a stated limitation.
}

/// **Comparing two `long double`s agrees with gcc.**
///
/// The last x87 step before arithmetic, and the cheapest: an ordered comparison is decidable on the
/// patterns themselves. Same sign, compare the exponent and then the significand; different signs,
/// the negative one is smaller. No significand arithmetic and no soft-float.
///
/// **The fixture that matters is the one whose operands differ past `f64`.** `0x1.00000000000008p0L`
/// is `1 + 2^-53`, which rounds to `1.0` — so an implementation that compares by narrowing to `f64`
/// first calls it *equal* to `1.0L` and gets `>` wrong. Wave 236 is what makes that fixture possible:
/// before it, the literal was already rounded and no test could tell the two approaches apart.
///
/// NaN is the other half. IEEE-754 §5.11 makes every ordered comparison with a NaN false, including
/// `x == x`, and `!=` true — so `unordered` is not just "some answer" but a specific one for each
/// operator.
#[test]
fn comparing_long_doubles_agrees_with_gcc() {
    agree("long double a = 1.0L, b = 2.0L; return (int)(a < b);");
    agree("long double a = 1.0L, b = 2.0L; return (int)(a > b);");
    agree("long double a = 2.0L, b = 2.0L; return (int)(a == b);");
    agree("long double a = 2.0L, b = 2.0L; return (int)(a <= b);");
    // Signs, where the pattern comparison has to reverse.
    agree("long double a = -1.0L, b = 1.0L; return (int)(a < b);");
    agree("long double a = -2.0L, b = -1.0L; return (int)(a < b);");
    // Both zeros, which are equal despite differing in the sign bit.
    agree("long double a = 0.0L, b = -0.0L; return (int)(a == b);");
    // **Differing only past `f64`'s fifty-three bits**, so narrowing to compare gets it wrong.
    agree("long double a = 0x1.00000000000008p0L, b = 1.0L; return (int)(a > b);");
    agree("long double a = 0x1.00000000000008p0L, b = 1.0L; return (int)(a == b);");
    // **NaN is not here**, and not for want of caring: producing one in C needs `0.0L / 0.0L`, and
    // `f80` division is the next milestone step. The unordered behaviour IEEE-754 §5.11 specifies is
    // pinned where it can be reached without arithmetic — `chiero-cir`'s `fp` unit tests, on
    // `partial_cmp` itself.
}

/// **A decimal `long double` literal keeps all sixty-four of its significand bits.**
///
/// `float_literal` parses every decimal literal with `str::parse::<f64>` and widens the result, so a
/// `long double` literal arrives already rounded to fifty-three bits and clamped to `f64`'s range.
/// Its comment calls this "a narrowing this records rather than hides" and **nothing records it** —
/// no diagnostic, no fidelity change, no `Undef`.
///
/// # Why this is a defect now and was a preference before
///
/// For six waves it cost nothing observable, because `f80` arithmetic was a declared gap: a program
/// could not compute with these values, so it could not disagree about them. Wave 239 shipped
/// multiplication and the same day `1e300L * 1e10L > 1e309L` started answering **no** — `1e309`
/// overflows `f64` to an infinity, and the comparison asks whether 1e310 exceeds infinity. That is a
/// wrong number, which 023 §7 does not permit at any fidelity. One fixture in
/// `long_double_arithmetic_agrees_with_gcc_or_says_it_is_unmodelled` was rewritten in hex to keep
/// testing what it claims to test, and this is the defect it was rewritten around.
///
/// # The two halves, which fail for different reasons
///
/// **Precision.** `0.1L` is the true tenth rounded to sixty-four bits; `0.1` is the true tenth
/// rounded to fifty-three and then widened, which lands *above* it. So `0.1L < 0.1` is true in C and
/// false here, where both spellings produce the same bits.
///
/// **Range.** `1e309` and `1e4000` are ordinary `long double`s — the format reaches about 1.19e4932 —
/// and both become infinities in an `f64`. The range failure is the louder of the two and the
/// precision failure is the one that will outlive it, since a fix that widens the range without
/// widening the significand still gets `0.1L` wrong.
///
/// Hex literals are exact already (wave 236), so every expectation here is written against one.
#[test]
fn a_decimal_long_double_literal_keeps_all_its_bits() {
    // Precision: the same decimal at two types is not the same number.
    agree("return (int)(0.1L < 0.1);");
    agree("long double a = 1.1L; return (int)(a != (long double)1.1);");
    // Range, just past `f64`'s top. `1e309` is about 2^1026, so it is under 2^1030 and over 2^1020.
    agree("return (int)(1e309L < 0x1p1030L);");
    agree("return (int)(1e309L > 0x1p1020L);");
    // Range, far past it and still ordinary for this format.
    agree("return (int)(1e4000L > 0x1p13000L);");
    // Range, past the bottom: `1e-320` is a subnormal `double` and a normal `long double`.
    agree("return (int)(1e-320L > 0.0L);");
    // **The rounding decision itself, against the exact value written in hex.** Every fixture
    // above compares a decimal against another *decimal* or a coarse bound, which a conversion
    // that truncated instead of rounding would still satisfy — mutation said so. These name the
    // correctly-rounded `f80` outright, and each one sits on a different branch of the decision.
    //
    // Round up, with the sticky coming from the division's remainder: `0.1 × 2^67` is
    // `0xCCCC…CCCC.CCC…`, an *even* significand with the guard bit set. It is a tie only if the
    // remainder is forgotten, and a forgotten tie on an even significand rounds the wrong way.
    agree("return (int)(0.1L == 0x1.999999999999999ap-4L);");
    // Guard clear — the value rounds *down*, which is what a fix that rounded up whenever bits
    // were discarded would get wrong.
    agree("return (int)(0.7L == 0x1.6666666666666666p-1L);");
    // **An exact tie, on an even candidate**: `2^64 + 1` is sixty-five bits with nothing below
    // them, so ties-to-even keeps `2^64` where ties-away-from-zero would not.
    agree("return (int)(18446744073709551617.0L == 0x1p64L);");
    // **An exact tie on an odd candidate, whose round up carries.** `2^65 - 1` rounds to a new
    // power of two, the second normalization `mul` needs and this needs for the same reason.
    agree("return (int)(36893488147419103231.0L == 0x1p65L);");
    // **The sticky bits from the quotient rather than the remainder.** `2^65 + 3` divides
    // exactly — there is no remainder to be sticky — but the quotient is sixty-six bits, so the
    // bit below the guard is the only thing separating this from a tie.
    agree("return (int)(36893488147419103235.0L == 0x1.0000000000000002p65L);");
    // The control. A literal `f64` can represent exactly must still round-trip, or a fix that
    // widened everything by hand would break the values that were already right.
    agree("return (int)(0.5L == 0x1p-1L);");
    agree("return (int)(2.0L == 0x1p1L);");
    agree("return (int)(1e10L == 0x1.2a05f2p33L);");
}

/// **Dividing `long double`s agrees with gcc.**
///
/// The last of the four, and the only one that needs a loop — one quotient bit at a time with its own
/// sticky remainder, which is the shape wave 240's `from_decimal` already uses. Two normalized
/// significands give a quotient between a half and two, so the numerator is staged sixty-four bits up
/// and one further bit is taken by hand: `sa << 64` fits a `u128` and `sa << 65` does not.
///
/// # Division cannot round on a tie, and cannot carry
///
/// Both were checked by enumerating every normalized operand pair at significand widths six through
/// twelve — 4.2 million pairs at the widest — and neither occurs at any width:
///
///   - **No exact tie.** A tie needs the quotient to terminate in exactly sixty-five bits, which
///     needs the divisor to reduce to a power of two; and then the quotient's numerator is
///     `sa / gcd(sa, sb)`, which is under `2^64` and so has at most *sixty-four*.
///   - **No rounding carry.** Rounding up out of an all-ones significand needs the exact quotient
///     within half an ulp below a power of two. Just under one the ulp is `2^-64`, so it needs
///     `sb - sa` to be under about `sb · 2^-65`, which is less than one — and these are integers.
///
/// So the fixtures below cover rounding up, rounding down and exact division, and there is nothing to
/// write for the two cases `mul` and `add` both need. The implementation says so with a
/// `debug_assert!` rather than a comment, which puts the proof under the test suite.
#[test]
fn dividing_long_doubles_agrees_with_gcc() {
    agree("long double a = 6.0L, b = 3.0L; return (int)(a / b);");
    agree("long double a = -6.0L, b = 3.0L; return (int)(a / b);");
    agree("long double a = -6.0L, b = -3.0L; return (int)(a / b);");
    // Exact: a power-of-two divisor divides without a remainder at all.
    agree("return (int)(1.0L / 2.0L == 0x1p-1L);");
    agree("return (int)(0x1.8p3L / 0x1p2L == 0x1.8p1L);");
    // **Rounding up**, against the exact quotient in hex.
    agree("return (int)(1.0L / 3.0L == 0x1.5555555555555556p-2L);");
    agree("return (int)(1.0L / 10.0L == 0x1.999999999999999ap-4L);");
    agree("return (int)(1.0L / 0x1.8p0L == 0x1.5555555555555556p-1L);");
    // **Rounding down**, which a fix that rounded up whenever a remainder was left would get wrong.
    agree("return (int)(1.0L / 7.0L == 0x1.2492492492492492p-3L);");
    agree("return (int)(10.0L / 3.0L == 0x1.aaaaaaaaaaaaaaaap1L);");
    // **Both quotient widths.** `sa/sb` lands in `[1, 2)` or in `[1/2, 1)`, and the two need a
    // different number of bits dropped — a fix that assumed one of them gets the other off by a
    // factor of two.
    agree("return (int)(0x1.8p0L / 0x1p0L == 0x1.8p0L);");
    agree("return (int)(0x1p0L / 0x1.8p0L == 0x1.5555555555555556p-1L);");
    // Zero on top, and the sign that comes with it.
    agree("return (int)(0.0L / 3.0L == 0.0L);");
    // **Division by zero is not undefined for floats**, it is IEEE-754 §7.3's divideByZero and its
    // result is an infinity — so this is a value chiero must produce rather than a fault to report.
    agree("return (int)(1.0L / 0.0L > 0x1p16000L);");
    agree("return (int)(-1.0L / 0.0L < -0x1p16000L);");
    // Past `f64`'s range at both ends, which is what this format is for.
    agree("return (int)(1e4000L / 1e-4000L > 0x1p16000L);");
    // Down the bottom end but still inside it: `1e-4400` is a normal `long double`, where
    // `1e-8000` is not — and the underflow that produces is a declared gap, pinned in
    // `chiero-cir`'s own tests rather than here, because `agree` compares *values*.
    agree("return (int)(1e-4000L / 1e400L < 1e-4000L);");
    // A quotient that stays put: dividing by one is the identity even for the widest significand.
    agree("return (int)(0x1.fffffffffffffffep0L / 1.0L == 0x1.fffffffffffffffep0L);");
}

/// **A conditional over two function designators.**
///
/// Found by auditing the fall-throughs wave 244's rule asks about: every place in lowering where a
/// missing answer is replaced by a substituted *value* rather than by a refusal. Twenty-nine
/// `size_of`/`align_of` fallbacks were instrumented and the whole suite run; twenty-eight never fire
/// at all, and this is the one that does.
///
/// `c ? f : g` has type "pointer to function returning int" — C11 6.3.2.1 decays each arm from a
/// function designator, and 6.5.15 makes the result their common type. Sema keeps `Ty::Func`, so
/// `align_of` returns `None` (a function has no alignment, correctly), and lowering answers that
/// `None` with a literal `4`.
///
/// **The alignment is the symptom, not the disease.** What is wrong is the type, and `sizeof` is
/// where a type shows: `sizeof(c ? f : g)` is eight on this target, because the conditional is a
/// pointer. The same decay is missing for **arrays**, and there it produces a *number*:
/// `sizeof(c ? a : b)` for two `int[4]` is eight, and chiero says sixteen.
///
/// Only that one fixture is RED. **The three calls below already pass**, and knowing that changes
/// what this wave is about: lowering's `slot_ty` fallback picks `Ptr` and is right, so calling
/// through the conditional works and the substituted alignment never reaches an answer. The audit
/// found a *type* that is wrong in one observable place, not a fall-through corrupting values —
/// worth stating plainly, because the rule that prompted the audit predicted the louder thing and
/// this is the quieter one.
#[test]
fn a_conditional_over_function_designators_is_a_pointer() {
    agree_with(
        "int f(int x){return x+1;}\nint g(int x){return x+2;}\n",
        "int c = 1; return (int)sizeof(c ? f : g);",
    );
    agree_with(
        "int f(int x){return x+1;}\nint g(int x){return x+2;}\n",
        "int c = 1; return (c ? f : g)(10);",
    );
    agree_with(
        "int f(int x){return x+1;}\nint g(int x){return x+2;}\n",
        "int c = 0; return (c ? f : g)(10);",
    );
    // **An array conditional, which is the same missing decay and a louder failure.** `sizeof` of
    // it is the *pointer* size, because C11 6.3.2.1 decays both arms before 6.5.15 picks a common
    // type — so an implementation that skips the decay reports the array's own size and is wrong by
    // a factor of the element count. This one is a number rather than a refusal.
    agree("int a[4]={1,2,3,4},b[4]={5,6,7,8}; int c=1; return (int)sizeof(c ? a : b);");
    agree("int a[4]={1,2,3,4},b[4]={5,6,7,8}; int c=1; return (c ? a : b)[1];");
    agree("int a[4]={1,2,3,4},b[4]={5,6,7,8}; int c=0; return (c ? a : b)[1];");
    // **The condition decays as well**, which is a separate sentence of 6.3.2.1 and needs its own
    // fixture: mutation kept the condition's decay alive through every arm-shaped test above,
    // because nothing here had ever put an array or a function *in the condition*. An array there
    // is tested as the pointer it decays to, so it is always true.
    agree("int a[4]={1,2,3,4}; return a ? 7 : 9;");
    agree_with("int f(int x){return x+1;}\n", "return f ? 7 : 9;");
    // **And the else arm on its own.** Every fixture above is symmetric — both arms are arrays, or
    // both are functions — so the *then* arm's decay alone produced the right common type and the
    // else arm's could be deleted unnoticed. Here the then arm is an `int` null pointer constant,
    // so only the else arm can supply the pointer.
    agree("int a[4]={1,2,3,4}; int c=1; return (int)sizeof(c ? (int*)0 : a);");
    // The GNU elvis form, where there is no then arm at all and the type is the else arm's alone.
    agree("int a[4]={1,2,3,4}; return (int)sizeof(a ?: a);");
    // **A null pointer constant against a pointer.** C11 6.5.15p6: the result is the *pointer*
    // type, not the integer's. `0` is a null pointer constant, `a` decays to `int *`, and the
    // conditional is `int *` — so `sizeof` is the pointer's. This is the rule `common_type` did
    // not know, found by writing the fixture above that isolates the else arm's decay.
    agree("int a[4]={1,2,3,4}; int c=1; return (int)sizeof(c ? 0 : a);");
    agree("int a[4]={1,2,3,4}; int c=1; return (int)sizeof(c ? a : 0);");
    // Two pointers, where `void *` wins (6.5.15p6 again) and the value must survive the trip.
    agree("int a[4]={1,2,3,4}; int c=1; void *v=a; int *p=a; return (int)sizeof(c ? v : p);");
    agree("int a[4]={1,2,3,4}; int c=1; void *v=a; int *p=a; return (int)sizeof(c ? p : v);");
    agree("int a[4]={1,2,3,4}; int c=1; void *v=a; int *p=a; return *(int*)(c ? v : p);");
    // **`sizeof` cannot see which pointer won**, since every pointer is eight bytes — mutation
    // said so by surviving both `void *` arms. Arithmetic can: adding one to a `void *` advances
    // one byte (the GNU extension's `sizeof(void)`), and to an `int *` four. So this is the pair
    // that pins the rule, in both operand orders, with the same-type case as the control.
    agree(
        "int a[4]={1,2,3,4}; int c=1; void *v=a; int *p=a; \
         return (int)((char*)((c ? p : v) + 1) - (char*)a);",
    );
    agree(
        "int a[4]={1,2,3,4}; int c=1; void *v=a; int *p=a; \
         return (int)((char*)((c ? v : p) + 1) - (char*)a);",
    );
    agree(
        "int a[4]={1,2,3,4}; int c=1; int *p=a; \
         return (int)((char*)((c ? p : p) + 1) - (char*)a);",
    );
    // Through a variable, which is the spelling that already works and the control for it.
    agree_with(
        "int f(int x){return x+1;}\nint g(int x){return x+2;}\n",
        "int c = 1; int (*p)(int) = c ? f : g; return p(10);",
    );
}

/// **Every other place lowering decides sign- versus zero-extension.**
///
/// Wave 249 found `LoadBits` asking `is_signed(e)` — the *promoted expression's* signedness — where
/// the *field's* was meant. `is_signed` has a dozen other callers, and the ones that decide an
/// extension are the ones that could hold the same confusion. Reading them says they are right by
/// design: each takes an operand whose value is already materialised at its promoted type, so
/// extending it as the promoted type says is correct.
///
/// **This set out to test that rather than assert it**, because "right by design" is what wave
/// 249's site looked like too. Each fixture puts a narrow *unsigned* value in the half of its range where the
/// two extensions disagree and pushes it through one extension decision:
///
/// ```text
///   widen_to_64          a narrow unsigned in a 64-bit context
///   array index          an unsigned index sign-extended to 64
///   integer -> float     the source's signedness picks SiToFp or UiToFp
///   plain widening       SExt versus ZExt on an ordinary conversion
/// ```
///
/// # What mutation says these actually observe, which is less than they look like
///
/// All pass, and that is worth exactly as much as the mutants they kill. Forcing each extension
/// decision the wrong way, one at a time:
///
/// ```text
///   plain widening SExt/ZExt   KILLED   <- these fixtures do observe it
///   widen_to_64 signedness     SURVIVES
///   array index SExt/ZExt      SURVIVES
///   integer -> float SiToFp    KILLED    <- wave 255
///   integer -> float UiToFp    SURVIVES  <- needs a negative source that reaches this arm
/// ```
///
/// So this is a regression guard for **one** of the four sites and a set of valid-but-inert
/// programs for the other three. Recording that is the point: a passing test named after a decision
/// it cannot observe is worse than no test, because it reports the coverage its name claims.
///
/// Three attempts at the inert ones failed, and how they failed is the useful part:
///
///   - **`unsigned char i = 2` as an index cannot discriminate at all** — 2 extends the same way
///     either way. The obvious repair, `i = 200`, needs an array with an element 200, and seeding
///     one with a 256-iteration loop exceeds the engine's budget: the fixture then returns nothing
///     and the *control* fails, which is how that was caught rather than shipped.
///   - **`signed char i = -2` from mid-array discriminates for a `signed` index but not an
///     `unsigned` one**, so it kills nothing when the mutation forces `SExt`.
///   - ~~**`int v = -5; double d = v;` does not reach the `SiToFp`/`UiToFp` site at all.**~~
///     **Wrong, and wave 255 found out how.** It is reached — a `double` local, a `double` member
///     initialized by a brace, and a `double` array element all arrive there. Wave 254 concluded
///     "never reached" from an instrumented run that logged nothing, without the control that would
///     have shown the logging was not in the build. The site was never dead, only never *tested*:
///     the whole suite had no int-to-float conversion through `convert_for_store`.
///
///     With those fixtures it now observes `UiToFp` — an `unsigned` source whose top bit is set
///     kills the "always signed" mutant. **"Always unsigned" still survives**: six signed sources
///     reach the arm and every one of them holds a non-negative value, where the two conversions
///     agree. A negative source that arrives *here* rather than through `cast_kind` has not been
///     found; §9 carries it.
///
/// The fixtures stay because they are correct C with correct expectations, and because the one site
/// they do cover is covered. What they must not be read as is coverage of the other three.
#[test]
fn narrow_unsigned_values_extend_the_same_way_gcc_extends_them() {
    agree("double d = -5; return (int)d;");
    agree("struct S { double d; }; struct S s = {-5}; return (int)s.d;");
    agree("double a[2] = {-5, 8}; return (int)(a[0] + a[1]);");
    agree("unsigned u = 4294967291u; double d = u; return (int)(d > 4000000000.0);");
    agree(
        "struct S { double d; }; unsigned u = 4294967291u; struct S s = {u}; \
         return (int)(s.d > 4000000000.0);",
    );
    agree("struct S { double d; }; int i = -5; struct S s = {i}; return (int)s.d;");
    agree("int i = -5; double a[2] = {i, 8}; return (int)a[0];");
    agree("int i = -5; double d = i; return (int)(d < 0.0);");

    // A narrow unsigned reaching a 64-bit context by ordinary conversion.
    agree("unsigned char c = 200; long v = c; return (int)v;");
    agree("unsigned short h = 60000; long v = h; return (int)(v >> 8);");
    agree("unsigned char c = 200; return (int)((long)c * 1000000000L > 0);");
    // **A negative index from the middle of an array**, which is what separates the two
    // extensions cheaply. `unsigned char i = 2` cannot tell them apart at all — mutation said so —
    // and `i = 200` can, but only in an array with an element 200, and a 256-iteration seeding
    // loop exceeds the engine's budget so the fixture returns nothing and tests nothing. Starting
    // four elements in and stepping back two discriminates in eight elements: sign-extended `-2`
    // reaches `a[2]`, zero-extended it is 254 and reaches nothing.
    agree("int a[8] = {0,1,2,3,4,5,6,7}; int *p = a + 4; signed char i = -2; return p[i];");
    agree("int a[8] = {0,1,2,3,4,5,6,7}; int *p = a + 4; unsigned char u = 2; return p[u];");
    // **Integer to float, with a *negative* source.** `unsigned char c = 200` promotes to a
    // positive `int` before the conversion, so `SiToFp` and `UiToFp` agree on it and the fixture
    // proves nothing — which is what mutation said. A negative `int` separates them: `SiToFp`
    // gives -5 and `UiToFp` gives 4294967291.
    agree("int v = -5; double d = v; return (int)d;");
    agree("int v = -5; float f = v; return (int)f;");
    agree("int a[2] = {-5, 0}; double d = a[0]; return (int)d;");
    agree("int a[2] = {-5, 0}; double d = a[0]; return (int)(d < 0.0);");
    // And an unsigned source large enough that reading its top bit as a sign would show.
    agree("unsigned u = 4294967291u; double d = u; return (int)(d > 4000000000.0);");
    // A plain widening conversion, both directions of the decision.
    agree("unsigned char c = 200; unsigned long v = c; return (int)(v == 200);");
    agree("signed char c = -56; long v = c; return (int)(v == -56);");
    // And the same value through a signed narrow type, which must still sign-extend — the control
    // that stops a fix from simply never extending.
    agree("signed char c = -56; return (int)((long)c < 0);");
    agree("short h = -1; unsigned long v = (unsigned long)(long)h; return (int)(v >> 60);");
}

/// **A float on the *right* of `&&` or `||` refuses to lower.**
///
///     `probe` lowered to CIR the verifier rejects (Ne operand is Float(F64), declared Int(32))
///
/// `x && d` is ordinary C — 6.5.13p1 takes any scalar operand — and chiero produces nothing for
/// it at all. Not a wrong answer: a **refusal**, which under 023 §7 is the honest outcome only if
/// the limit is declared, and this one is not declared anywhere.
///
/// # Left works, right does not, which is the whole diagnosis
///
/// `d && 1` and `d || 0` both agree with gcc. The lhs of a short circuit goes through `truth_of`,
/// which knows a float's truth is a comparison and not a bit test; the rhs **re-derives the same
/// decision inline** with a hardcoded integer `Ne` against an `Int(32)` zero. One of the two
/// copies was fixed and the other was not — the duplicate-decision-site shape this codebase keeps
/// finding.
///
/// The negative zero is incidental here. `2.0` on the right refuses just as flatly, so *every*
/// `x && <double>` in any program has been silently dropped.
///
/// The verifier caught it, which is the point of having one: a wrong CIR type became a refusal
/// with a legible message instead of a wrong answer.
#[test]
fn a_float_on_the_right_of_a_short_circuit_agrees_with_gcc() {
    // The controls: the same value on the *left* is already right.
    agree("double d = -0.0; return d && 1;");
    agree("double d = -0.0; return d || 0;");
    agree("double d = 2.0; return d && 1;");
    // The defect, which has nothing to do with negative zero.
    agree("double d = 2.0; return 1 && d;");
    agree("double d = 0.0; return 1 && d;");
    agree("double d = -0.0; return 1 && d;");
    agree("double d = 0.0; return 0 || d;");
    agree("float f = 1.5f; return 1 && f;");
    agree("long double l = 0.0L; return 1 && l;");
    // A pointer on the right, which re-derives the same decision a third way.
    agree("int v; int *p = &v; return 1 && p;");
    agree("int *p = 0; return 1 && p;");
    // And the integer case, which must keep working.
    agree("int x = 2; return 1 && x;");
    agree("int x = 0; return 1 || x;");
}

/// **A 128-bit signed operation panics the engine.**
///
///     __int128 x = 1; x = x << 70;
///     panicked at chiero-exec/src/lib.rs: attempt to subtract with overflow
///
/// A **source-triggerable panic**, which this codebase has already named the worst outcome there
/// is (wave 246): it takes the run and every other finding in it, and `catch_unwind` cannot
/// contain an abort.
///
/// # One shape, repeated at every width bound
///
/// The UB checks bound a signed type's range with `(1i128 << (w - 1)) - 1` and `-(1i128 << (w -
/// 1))`. At `w = 128` the shift *is* `i128::MIN`, so the first underflows and the second negates
/// a value with no positive counterpart. Every width below 128 leaves headroom, so all of them
/// are fine and the widest is not — the arithmetic that computes the boundary has the same
/// boundary problem it is checking for.
///
/// # Found while writing a control
///
/// It is not a `typeof` defect at all. `__int128 x = 1; x = x << 70;` was a *control* in the
/// `TypeKind` census — one of the ordinary type forms that were supposed to already work — and
/// probing it individually is what separated the crash from the eleven `typeof` failures around
/// it.
#[test]
fn a_128_bit_operation_does_not_panic_the_engine() {
    // The shift that crashed: a left shift whose result needs more than 64 bits.
    agree("__int128 x = 1; x = x << 70; return (int)(x >> 70);");
    agree("__int128 x = 1; x = x << 100; return (int)(x >> 100);");
    agree("__int128 x = 3; x = x << 126; return (int)(x >> 126);");
    // A shift that stays small, which never reached the bound.
    agree("__int128 x = 5; return (int)(x >> 1);");
    agree("__int128 x = 1; x = x << 30; return (int)(x >> 30);");
    // The unsigned counterpart, which uses a different bound and already worked.
    agree("unsigned __int128 x = ~(unsigned __int128)0; return (int)(x >> 127);");
    agree("unsigned __int128 x = 1; x = x << 127; return (int)(x >> 127);");
    // **Division and remainder at 128 bits**, which is the other place the range bound is
    // built — the `INT_MIN / -1` check negates `1 << (w - 1)`.
    agree("__int128 x = 7; __int128 y = 2; return (int)(x / y);");
    agree("__int128 x = 7; __int128 y = 2; return (int)(x % y);");
    agree("__int128 x = 1; x = x << 100; __int128 y = -1; return (int)((x / y) >> 100);");
    // Add, subtract and multiply near the top of the range, where the overflow check runs.
    agree("__int128 x = 1; x = x << 100; x = x + 1; return (int)(x >> 100);");
    agree("__int128 x = 1; x = x << 100; x = x - 1; return (int)(x >> 99);");
    agree("__int128 x = 1; x = x << 60; x = x * x; return (int)(x >> 120);");
    // Negative values, which take the other side of the bound.
    agree("__int128 x = -1; x = x << 70; return (int)(x >> 70);");
    agree("__int128 x = -3; return (int)(x / 2);");
    // And the ordinary widths, which must not move.
    agree("long x = 1; x = x << 40; return (int)(x >> 40);");
    agree("int x = 1; x = x << 20; return (int)(x >> 20);");
}

/// **`typeof` resolves to nothing: every declaration using it has an unknown type.**
///
///     int x = 3; __typeof__(x) y = x + 1; return y;
///     SemaDiagnostic: "`y` has an incomplete or unknown type; its uses are not checked"
///
/// The parser has had `TypeKind::TypeofExpr` and `TypeKind::TypeofType` since it was written —
/// `typeof` *parses* — and sema's `ty_of` never resolved either, so the declared object gets
/// `Ty::Error` and everything downstream is unchecked or refused.
///
/// # Reach
///
/// **37 VPP files.** `typeof` is how VPP's container macros stay type-generic; it is also how
/// `__builtin_types_compatible_p (__typeof__ (x), void)` is written, which 013 already calls out
/// as appearing "in every TU that includes `<string.h>`".
///
/// # Found by censusing `TypeKind`
///
/// The last grammar-shaped enum with no census (§9, wave 282). Twenty-two probes over its nine
/// variants: `Builtin` in every width including `__int128` and `_Float16`, `Named`, `Tag` for
/// both `struct` and `union` and `enum`, `Ptr`, `Array`, `Func` — all correct. **Both `typeof`
/// variants fail, and nothing else does.** One enum, one hole, and it is the one VPP leans on.
///
/// # `typeof` does not evaluate its operand
///
/// Like `sizeof`, and for the same reason: the answer is the operand's *type*. `__typeof__(a[i++])`
/// leaves `i` alone, which is the fixture that separates "resolved the type" from "lowered the
/// expression".
#[test]
fn typeof_resolves_to_its_operands_type() {
    // Controls: the other `TypeKind` forms the census probed, which already work.
    agree("return (int)sizeof(long long) * 10 + (int)sizeof(short);");
    agree("__int128 x = 1; x = x << 70; return (int)(x >> 70);");
    agree_with(
        "enum E { A, B };",
        "enum E e = B; return (int)sizeof(e) + (int)e;",
    );
    // The expression form, both spellings.
    agree("int x = 3; __typeof__(x) y = x + 1; return y;");
    agree("int x = 3; typeof(x) y = x + 1; return y;");
    agree("int x = 3; __typeof(x) y = x + 1; return y;");
    // The type form.
    agree("return (int)sizeof(__typeof__(int));");
    agree("__typeof__(long) l = 7; return (int)sizeof(l) * 10 + (int)l;");
    // It is a *type*, so `sizeof` of it is the operand's size and not the operand's value.
    agree("int x = 3; return (int)sizeof(__typeof__(x));");
    agree("double d = 1.5; return (int)sizeof(__typeof__(d));");
    // The operand's type is taken exactly: signedness and width survive.
    agree("unsigned char c = 200; __typeof__(c) d = c; return d;");
    agree("unsigned char c = 200; __typeof__(c) d = c + 1; return d;");
    agree("short s = 1; return (int)sizeof(__typeof__(s));");
    // ...including the type an *expression* has after its own conversions.
    agree("int x = 3; __typeof__(x + 1L) y = 5; return (int)sizeof(y);");
    agree("char a = 1; char b = 2; return (int)sizeof(__typeof__(a + b));");
    // Aggregates and pointers.
    agree("int a[4] = {1,2,3,4}; __typeof__(a) b = {5,6,7,8}; return b[2] + (int)sizeof(b);");
    agree("int x = 3; __typeof__(&x) p = &x; return *p;");
    agree_with(
        "struct S { int a; };",
        "struct S s = {7}; __typeof__(s) t = s; return t.a;",
    );
    agree("double d = 1.5; __typeof__(d) e = d * 2; return (int)e;");
    agree("int a[3][2]; return (int)sizeof(__typeof__(a[0]));");
    // A qualifier applied to a `typeof`.
    agree("int x = 3; const __typeof__(x) y = x; return y;");
    // **Unevaluated**, like `sizeof`.
    agree("int i = 0; int a[4] = {1,2,3,4}; __typeof__(a[i++]) v = 9; return v*10 + i;");
    // Nested, and a `typeof` of a `typeof`.
    agree("int x = 3; __typeof__(__typeof__(x)) y = 4; return y;");
}

/// **A requested alignment on a variable is ignored, and the storage really is misaligned.**
///
///     _Alignas(16) int x = 3; return (int)_Alignof(x);      chiero 4,  gcc 16
///     _Alignas(16) int x = 3; return (int)((long)&x & 15);  chiero 4,  gcc 0
///
/// The second line is the substantive one: this is not a misreported `_Alignof`, the object is at
/// its natural alignment. A program that aligns a buffer and then relies on it — a vector load, a
/// cache line, a lock-free structure — gets a differently-aligned object and no warning.
///
/// # Reach lives in the other spelling
///
/// `_Alignas` is in **0 VPP files**. `__attribute__((aligned(N)))` is the same defect down the
/// same path, and it is in **16 VPP files directly, with `aligned(` in 266**, because the
/// cache-line macros expand to it. §9 had this item filed under the spelling with no reach.
///
/// # Only the variable path
///
/// `struct A { char c; _Alignas(16) int v; }` already sizes, offsets and aligns correctly:
/// `Cx::aligned_attr` reads the attribute for *record layout* and nothing reads it for a
/// declaration. Wave 281 fixed the half of this that needed no specifier at all — `_Alignof(expr)`
/// was returning a size — and left this half, which needs somewhere to put a declaration's
/// alignment.
#[test]
fn a_requested_alignment_is_honoured() {
    // Controls: no specifier, and the member path, which already works.
    agree("int x = 3; return (int)_Alignof(x);");
    agree("int a[10]; return (int)_Alignof(a);");
    agree_with(
        "struct A { char c; _Alignas(16) int v; };",
        "return (int)sizeof(struct A);",
    );
    agree_with(
        "struct A { char c; _Alignas(16) int v; };",
        "struct A a; return (int)((char*)&a.v - (char*)&a);",
    );
    // `_Alignof` reports what was asked for...
    agree("_Alignas(16) int x = 3; return (int)_Alignof(x);");
    agree("_Alignas(8) char c = 1; return (int)_Alignof(c);");
    agree("_Alignas(double) char c = 1; return (int)_Alignof(c);");
    agree("_Alignas(32) int a[4]; return (int)_Alignof(a);");
    // ...and the storage actually has it, which is the half that matters.
    agree("_Alignas(16) int x = 3; return (int)((long)&x & 15) + x;");
    agree("_Alignas(64) char b[8]; return (int)((long)&b[0] & 63);");
    agree("_Alignas(32) int x = 3; return (int)((long)&x & 31) + x;");
    // **Two aligned locals**, so one cannot be right by accident of being first in the frame.
    agree(
        "_Alignas(16) int x = 3; _Alignas(16) int y = 4; return (int)(((long)&x | (long)&y) & 15) + x + y;",
    );
    // An unaligned local between two aligned ones, which is where a frame layout that only
    // rounds once goes wrong.
    agree(
        "_Alignas(16) int x = 1; char pad = 2; _Alignas(16) int y = 3; return (int)(((long)&x | (long)&y) & 15) + x + y + pad;",
    );
    // **The `__attribute__` spelling**, which is the one VPP uses.
    agree("int x __attribute__((aligned(16))) = 3; return (int)_Alignof(x);");
    agree("int x __attribute__((aligned(16))) = 3; return (int)((long)&x & 15) + x;");
    agree("__attribute__((aligned(8))) char c = 1; return (int)_Alignof(c);");
    agree("char b[4] __attribute__((aligned(32))); return (int)((long)&b[0] & 31);");
    // Static and file-scope storage take the same specifier.
    agree("static int x __attribute__((aligned(32))) = 3; return (int)((long)&x & 31) + x;");
    // **A file-scope object's *recorded* alignment, not just its address.** The address fixture
    // below cannot see this: the engine already places globals generously, so `&g & 63` is 0
    // whether or not the request was honoured, and a mutant that dropped it survived. `_Alignof`
    // reads the number the global actually carries.
    agree_with(
        "int g __attribute__((aligned(64))) = 5;",
        "return (int)_Alignof(g) + g;",
    );
    agree_with(
        "static int gs __attribute__((aligned(128))) = 6;",
        "return (int)_Alignof(gs) + gs;",
    );
    agree_with(
        "char gb[3] __attribute__((aligned(32)));",
        "return (int)_Alignof(gb);",
    );
    agree_with(
        "int g __attribute__((aligned(64))) = 5;",
        "return (int)((long)&g & 63) + g;",
    );
    // Through a typedef, where the alignment travels with the type rather than the declarator.
    agree("typedef int A __attribute__((aligned(16))); A x = 3; return (int)_Alignof(x);");
    // A request *equal* to the natural alignment changes nothing.
    agree("_Alignas(4) int x = 3; return (int)_Alignof(x);");
    // **Below the natural alignment is where the two spellings part**, and this fixture used to
    // assert the wrong thing. gcc *rejects* `_Alignas(1) int x` — "specifiers cannot reduce
    // alignment" — while `__attribute__((aligned(1)))` is accepted and really does reduce it, to
    // 1. Both spellings arrive here as one `aligned` attribute, so telling them apart needs the
    // parser to record which was written. Left undone: it is a packing feature, the reducing
    // direction appears nowhere in the target, and `max` is right for every raising use.
    // Recorded in §9.
}

/// **`_Alignof(expr)` returns the operand's *size*.**
///
///     int a[10]; return (int)_Alignof(a);   chiero says 40, gcc says 4
///
/// The parser recorded the GNU `_Alignof(expr)` form as a `SizeofExpr`, on the reasoning that a
/// sizeof-shaped node beat inventing a variant nothing else produced. Nothing downstream could
/// then tell them apart, so `_Alignof` computed a size.
///
/// **Size and alignment agree for every scalar**, which is why this stood: `int`, `char`,
/// `double`, `long long` all answer the same either way. They differ for an array — `_Alignof(a)`
/// is the element's alignment and `sizeof a` the whole object — and for any struct whose size is
/// rounded up past its alignment. No alignment specifier is involved at all; this is ordinary C.
///
/// # How it was found, and what the census before it said
///
/// §9 named the **preprocessor** as the last unrun axis. Forty-four probes — stringification,
/// pasting, variadic and GNU comma swallowing, blue paint, empty arguments, `#line`, `#if`
/// arithmetic over unsigned, `char` and `long long`, nested and indirect expansion — found **no
/// silent wrong answer**. Its three gaps are all *declared* (`__VA_OPT__ is outside chiero's v1
/// preprocessing scope`, `#elifdef is a C23 extension accepted in C11 mode`, and `defined`
/// produced by expansion), and all three are in 0 VPP files.
///
/// A clean census sent the wave back to §9's last known defect — "`_Alignas` is ignored on a
/// variable" — and probing *that* found this underneath it: `_Alignas(16) int x` reported 4
/// because 4 is `sizeof(int)`, not because the specifier was dropped.
///
/// # What is still open, deliberately
///
/// A **requested** alignment on a variable is still not honoured: `_Alignas(16) int x` reports 4
/// and `(long)&x & 15` is 4 where gcc gives 0, for both that spelling and
/// `__attribute__((aligned(16)))`. That needs a per-declaration alignment channel — sema's
/// `values` map carries name→type and no `DeclId` — which is its own wave. Recorded in §9.
#[test]
fn an_alignof_expression_is_an_alignment_not_a_size() {
    // The controls: scalars, where size and alignment coincide and the defect is invisible.
    agree("int x; return (int)_Alignof(x);");
    agree("char c; return (int)_Alignof(c);");
    agree("double d; return (int)_Alignof(d);");
    agree("long long l; return (int)_Alignof(l);");
    // ...and the *type* form, which was always right because it is a different node.
    agree("return (int)_Alignof(int[10]);");
    agree("struct S { char c; int i; }; return (int)_Alignof(struct S);");
    // **An array**, where the two answers differ by the element count.
    agree("int a[10]; return (int)_Alignof(a);");
    agree("char b[7]; return (int)_Alignof(b);");
    agree("double d[3]; return (int)_Alignof(d);");
    // **A struct whose size is rounded past its alignment.**
    agree("struct S { char c; int i; }; struct S s; return (int)_Alignof(s);");
    agree("struct S { char a; char b; }; struct S s; return (int)_Alignof(s);");
    agree(
        "struct S { int i; char c; }; struct S s; return (int)_Alignof(s) * 10 + (int)sizeof(s);",
    );
    // The `__alignof__` spelling, which is the one GNU code actually writes.
    agree("struct S { char c; int i; }; struct S s; return (int)__alignof__(s);");
    agree("int a[10]; return (int)__alignof__(a);");
    // A union, and a nested aggregate.
    agree("union U { char c; double d; }; union U u; return (int)_Alignof(u);");
    agree("struct S { double d; }; struct S a[4]; return (int)_Alignof(a);");
    // The result is `size_t`: unsigned, and wide enough that subtracting past zero wraps up.
    // `- 5`, not `- 1`: subtracting one from four is positive whichever signedness it has, so
    // that shape cannot see the type at all. It takes a subtraction that *wraps*.
    agree("int a[10]; return (int)(_Alignof(a) - 5 > 0);");
    agree("int a[10]; return (int)sizeof(_Alignof(a));");
    // And it is unevaluated, like `sizeof`.
    agree("int i = 0; int a[10]; int n = (int)_Alignof(a[i++]); return n*10 + i;");
}

/// **`__builtin_offsetof` reports its member argument undeclared.**
///
///     struct S { int a; int b; };
///     return (int)__builtin_offsetof(struct S, b);
///     SemaDiagnostic: "`b` was not declared"
///
/// `call_argument` already backtracks a type name, so `struct S` arrives as a `TypeName` — that
/// half works. The second argument is then typed as an ordinary expression, and a member name is
/// not a name in scope, so sema reports it undeclared and 015 §7 refuses the function.
///
/// # Reach, which is why this one and not `_Alignas`
///
/// **`offsetof` is in 27 VPP files; `_Alignas` is in 0.** On gcc `<stddef.h>`'s `offsetof` *is*
/// `__builtin_offsetof`, so every TU that includes it and uses the macro is refused whole — not a
/// wrong number in one expression, a function that never runs. `_Alignas` is a wrong answer and
/// so the worse *kind*, but it is broken only on the variable path (`struct A { char c;
/// _Alignas(16) int v; }` already sizes, offsets and aligns correctly) and nothing in the target
/// uses it. Wave 279's rule: severity orders defects of comparable reach, it does not override
/// reach.
///
/// # The second argument is a *member designator*, not an expression
///
/// C11 7.19 allows a member, a `.` chain and `[...]` subscripts. The parser already produces
/// exactly the right shape for all three — `n.y` is a `Member`, `v[2]` an `Index`, both rooted at
/// an `Ident` that happens not to resolve — so what is missing is a reader for that shape, not a
/// new grammar.
#[test]
fn builtin_offsetof_computes_a_member_offset() {
    // A member at a nonzero offset, and the first member, which must be 0.
    agree_with(
        "struct S { int a; int b; };",
        "return (int)__builtin_offsetof(struct S, b);",
    );
    agree_with(
        "struct S { int a; int b; };",
        "return (int)__builtin_offsetof(struct S, a);",
    );
    // Padding, so the answer is not just the sum of the sizes before it.
    agree_with(
        "struct S { char c; double d; };",
        "return (int)__builtin_offsetof(struct S, d);",
    );
    agree_with(
        "struct S { char c; int i; char d; long l; };",
        "return (int)__builtin_offsetof(struct S, l)*10 + (int)__builtin_offsetof(struct S, i);",
    );
    // A `.` chain into a named nested struct.
    agree_with(
        "struct S { int a; struct { int x; int y; } n; };",
        "return (int)__builtin_offsetof(struct S, n.y);",
    );
    // A subscript, and a subscript after a chain.
    agree_with(
        "struct S { int a; int v[4]; };",
        "return (int)__builtin_offsetof(struct S, v[2]);",
    );
    agree_with(
        "struct S { int a; struct { int v[3]; } n; };",
        "return (int)__builtin_offsetof(struct S, n.v[2]);",
    );
    // **Through an anonymous member**, which wave 279 taught the field lookup to walk. The
    // designator reader has to use that lookup rather than a scan of its own.
    agree_with(
        "struct S { int a; struct { int p; int q; }; };",
        "return (int)__builtin_offsetof(struct S, q);",
    );
    agree_with(
        "struct S { long g; union { struct { int x; int y; }; long z; }; };",
        "return (int)__builtin_offsetof(struct S, y);",
    );
    // **An anonymous member *after* a chain step.** Both fixtures above put the anonymous
    // member at the root, so the `Ident` arm's lookup covered them and a mutant that made the
    // `Member` arm scan `l.fields` directly survived. The two arms need the same lookup, and
    // only a designator that walks into a named struct *and then* through an anonymous one
    // says so.
    agree_with(
        "struct S { int a; struct { int z; struct { int p; int q; }; } n; };",
        "return (int)__builtin_offsetof(struct S, n.q);",
    );
    agree_with(
        "struct S { int a; struct { long z; union { int p; int q; }; } n; };",
        "return (int)__builtin_offsetof(struct S, n.q);",
    );
    agree_with(
        "struct S { int a; struct { int z; struct { int v[3]; }; } n; };",
        "return (int)__builtin_offsetof(struct S, n.v[2]);",
    );
    // A union, where every member is at 0.
    agree_with(
        "union U { int a; double d; };",
        "return (int)__builtin_offsetof(union U, d);",
    );
    // A typedef'd tagless struct, which is how most VPP types are spelled.
    agree_with(
        "typedef struct { int a; long b; } T;",
        "return (int)__builtin_offsetof(T, b);",
    );
    // **Its type is `size_t`**, not `int`: unsigned and the width of a pointer.
    agree_with(
        "struct S { int a; int b; };",
        "return (int)sizeof(__builtin_offsetof(struct S, b));",
    );
    agree_with(
        "struct S { int a; int b; };",
        "return (int)(__builtin_offsetof(struct S, a) - 1 > 0);",
    );
    // Used as an ordinary value.
    agree_with(
        "struct S { int a; int b; };",
        "int k = __builtin_offsetof(struct S, b); return k;",
    );
    agree_with(
        "struct S { int a; int b; };",
        "char buf[16]; struct S *p = (struct S*)buf; p->b = 7; return *(int*)(buf + __builtin_offsetof(struct S, b));",
    );
}

/// **A member of an anonymous struct or union does not resolve.**
///
///     struct S { struct { int a; int b; }; int c; };
///     struct S s; s.a = 1; return s.a;   chiero says None, gcc says 1
///
/// C11 6.7.2.1p13: an unnamed member whose type is a struct or union has *its* members treated as
/// members of the containing type. Chiero lays the member out — `sizeof` is right and the named
/// sibling `s.c` works — and cannot name anything inside it.
///
/// # Why this one and not `_Alignas`
///
/// The declarator census (wave 278) left four defects. `_Alignas` is the worse *kind* — a wrong
/// answer where this is a refusal — but wave 276's rule is to check the target rather than
/// intuition: anonymous members appear in **34 VPP files including `vnet/buffer.h`**, whose
/// `vnet_buffer()` macro is on nearly every data path in the tree, and `_Alignas` appears in
/// three. This makes a central header usable; that one changes an answer few programs ask for.
///
/// Pointer-to-array indexing, the fourth, is **struck**: re-probing shows wave 278 fixed it, so
/// it was a symptom of the dimension reversal rather than its own defect.
///
/// # The shape that matters is the nested one
///
/// `buffer.h` nests an anonymous struct inside an anonymous union inside a struct. A lookup that
/// searches one level down passes every flat fixture and fails there, so the nesting is here.
#[test]
fn an_anonymous_member_resolves_through_its_container() {
    // Controls: a *named* nested member, and a named sibling of an anonymous one, both work.
    agree_with(
        "struct S { struct { int a; } n; int c; };",
        "struct S s; s.n.a = 4; s.c = 5; return s.n.a*10 + s.c;",
    );
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "struct S s; s.c = 3; return s.c;",
    );
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "return (int)sizeof(struct S);",
    );
    // An anonymous struct in a struct: read, write, and every member.
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "struct S s; s.a = 1; s.b = 2; s.c = 3; return s.a*100 + s.b*10 + s.c;",
    );
    // **Its offsets**, which is what a name lookup has to get right rather than merely find.
    // Written as an address difference because `__builtin_offsetof` parses its member argument as
    // an ordinary identifier and reports "`b` was not declared" — a separate gap, recorded in §9.
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "struct S s; return (int)((char*)&s.b - (char*)&s);",
    );
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "struct S s; return (int)((char*)&s.c - (char*)&s);",
    );
    // A braced initializer fills through the anonymous member.
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "struct S s = {1,2,3}; return s.a*100 + s.b*10 + s.c;",
    );
    agree_with(
        "struct S { struct { int a; int b; }; int c; };",
        "struct S s = {1,2,3}; struct S t = s; return t.b;",
    );
    // **An anonymous union in a struct**, where two names share one offset.
    agree_with(
        "struct S { union { int a; int b; }; int c; };",
        "struct S s; s.a = 5; s.c = 3; return s.b*10 + s.c;",
    );
    agree_with(
        "struct S { union { int a; int b; }; int c; };",
        "return (int)sizeof(struct S);",
    );
    // An anonymous struct in a *union*, which is the aliasing direction.
    agree_with(
        "union U { struct { int a; int b; }; long l; };",
        "union U u; u.l = 0; u.a = 1; u.b = 2; return u.a*10 + u.b;",
    );
    // **The nesting `vnet/buffer.h` actually uses**: an anonymous struct inside an anonymous
    // union inside a struct. One level of search is not enough.
    agree_with(
        "struct S { union { struct { int x; int y; }; long q; }; int t; };",
        "struct S s; s.x = 1; s.y = 2; s.t = 3; return s.x*100 + s.y*10 + s.t;",
    );
    agree_with(
        "struct S { union { struct { int x; int y; }; long q; }; int t; };",
        "struct S s; return (int)((char*)&s.y - (char*)&s);",
    );
    // A bit-field inside an anonymous struct, whose `BitRange` has to travel with the offset.
    agree_with(
        "struct S { struct { int a:3; int b:5; }; int c; };",
        "struct S s; s.a = 1; s.b = 2; s.c = 3; return s.a*100 + s.b*10 + s.c;",
    );
    // **The anonymous member is not at offset 0.** Every fixture above declares it first, so
    // rebasing its members onto the container adds *zero* and a mutant that skipped the rebase
    // survived them all — wave 278's square array in a new costume. Putting a named member ahead
    // of it is the whole difference.
    agree_with(
        "struct S { int c; struct { int a; int b; }; };",
        "struct S s; s.c = 3; s.a = 1; s.b = 2; return s.a*100 + s.b*10 + s.c;",
    );
    agree_with(
        "struct S { int c; struct { int a; int b; }; };",
        "struct S s; return (int)((char*)&s.b - (char*)&s);",
    );
    agree_with(
        "struct S { char pad[6]; struct { int a; }; };",
        "struct S s; s.a = 7; return s.a + (int)((char*)&s.a - (char*)&s);",
    );
    // **A bit-field in an anonymous member that is not at offset 0.** `BitField::bit_offset` is
    // documented as absolute — bits from the start of *the record* — so promoting one has to add
    // the anonymous member's byte offset in bits. Nothing else can see that adjustment.
    agree_with(
        "struct S { int c; struct { int a:3; int b:5; }; };",
        "struct S s; s.c = 9; s.a = 1; s.b = 2; return s.a*100 + s.b*10 + s.c;",
    );
    // The `buffer.h` nesting, again with something in front of it.
    agree_with(
        "struct S { long g; union { struct { int x; int y; }; long q; }; int t; };",
        "struct S s; s.x = 1; s.y = 2; s.t = 3; s.g = 4; return s.x*1000 + s.y*100 + s.t*10 + (int)s.g;",
    );
    // A single anonymous member and nothing else.
    agree_with(
        "struct S { struct { int a; }; };",
        "struct S s; s.a = 9; return s.a;",
    );
}

/// **A multi-dimensional array has its dimensions reversed.**
///
///     int a[2][3] = {{1,2,3},{4,5,6}}; return a[1][2];   chiero says 0, gcc says 6
///     int a[2][3]; return (int)sizeof(a[0]);             chiero says 8, gcc says 12
///
/// `int a[2][3]` is typed as `int a[3][2]`. The second line is the one that says so without any
/// initializer in the way: `a[0]` is `int[3]`, so its size is 12, and 8 is `sizeof(int[2])`.
///
/// # Every symptom follows from that, including the ones that look unrelated
///
/// With the type reversed, the initializer fills rows of 2 — memory becomes `1 2 4 5 0 0` — and:
///
///   - `a[0][0]` is right, because offset 0 is offset 0 whichever way the dimensions go.
///   - `a[0][2]` reads 4: index 2 at a stride of 2 lands in the next row.
///   - **`a[1][0]` is right by accident.** Offset `1*2+0` = 2 holds 4, which is also what
///     `1*3+0` would hold in a correctly laid out array. One of the four corner reads agrees.
///   - `a[1][2]` and `((int*)a)[4]` read 0 — past the initialized bytes.
///   - `sizeof(a)` is right: 2·3·4 and 3·2·4 are both 24, so the total says nothing.
///   - **`int a[2][2]` is entirely correct**, because a square array is its own reverse.
///
/// # Why nothing caught it
///
/// The generator emits one-dimensional arrays only, and every hand-written fixture in the suite
/// that uses two dimensions uses a *square* one. The two shapes that would have shown it —
/// a non-square array, and `sizeof` of a row — are exactly the two nobody wrote.
///
/// Found by censusing the **declarator** grammar against gcc, which no wave had done: the
/// expression, statement, IR and keyword axes were all run, and the shape of a *declaration* was
/// never asked about.
#[test]
fn a_multidimensional_array_keeps_its_dimension_order() {
    // The control: one dimension, and a square two, both of which already work.
    agree("int a[3] = {1,2,3}; return a[2];");
    agree("int a[2][2] = {{1,2},{3,4}}; return a[1][1];");
    // **The type, with no initializer in the way.** `a[0]` is a row: 3 ints, not 2.
    agree("int a[2][3]; return (int)sizeof(a[0]);");
    agree("int a[2][3]; return (int)sizeof(a);");
    agree("int a[4][2]; return (int)sizeof(a[0]);");
    agree("int a[2][3][4]; return (int)sizeof(a[0]);");
    agree("int a[2][3][4]; return (int)sizeof(a[0][0]);");
    // Every element of a non-square array, so the one that is accidentally right cannot carry it.
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return a[0][0];");
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return a[0][1];");
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return a[0][2];");
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return a[1][0];");
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return a[1][1];");
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return a[1][2];");
    // The other way round, so a fix that merely swaps cannot pass.
    agree("int a[3][2] = {{1,2},{3,4},{5,6}}; return a[2][1];");
    agree("int a[3][2] = {{1,2},{3,4},{5,6}}; return a[0][1]*10 + a[2][0];");
    agree("int a[3][2]; return (int)sizeof(a[0]);");
    // The flat layout, which is what "row-major" actually means.
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; return ((int*)a)[4];");
    agree(
        "int a[2][3] = {{1,2,3},{4,5,6}}; int s = 0; for (int i=0;i<6;i++) s = s*10 + ((int*)a)[i]; return s;",
    );
    // A flat initializer fills row by row.
    agree("int a[2][3] = {1,2,3,4,5,6}; return a[1][2];");
    agree("int a[2][3] = {1,2,3}; return a[0][2]*10 + a[1][0];");
    // Writes through the subscript, which never depended on the initializer.
    agree("int a[2][3]; a[1][2] = 6; a[0][2] = 3; return a[1][2]*10 + a[0][2];");
    // **Elision nested inside elision.** A flat list filling a three-dimensional array makes
    // `init_flat` meet an aggregate slot of its own and recurse; a mutant that consumed one item
    // there instead of the four the row needs survived every two-dimensional fixture above.
    agree("int a[2][2][2] = {1,2,3,4,5,6,7,8}; return a[1][0][1];");
    agree("int a[2][2][2] = {1,2,3,4,5}; return a[1][0][0]*10 + a[1][1][1];");
    agree("int a[2][2][2] = {{1,2,3,4},{5,6,7,8}}; return a[0][1][1]*10 + a[1][0][0];");
    // The same nesting through a struct, where the subaggregate is a member rather than a row.
    agree_with(
        "struct S { int p[2]; int q; };",
        "struct S s[2] = {1,2,3,4,5,6}; return s[1].p[1]*10 + s[0].q;",
    );
    // Three dimensions, where a reversal and a rotation differ.
    agree("int a[2][3][4]; a[1][2][3] = 7; return a[1][2][3];");
    agree("int a[1][2][3] = {{{1,2,3},{4,5,6}}}; return a[0][1][2];");
    // A pointer to a row, whose arithmetic scales by the row's size.
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; int (*p)[3] = a; return p[1][0];");
    agree("int a[2][3] = {{1,2,3},{4,5,6}}; int (*p)[3] = a; return p[1][2];");
    // A parameter declared as an array of arrays.
    agree_with(
        "int sum(int a[2][3]) { int s = 0; for (int i=0;i<2;i++) for (int j=0;j<3;j++) s = s*10 + a[i][j]; return s; }",
        "int a[2][3] = {{1,2,3},{4,5,6}}; return sum(a);",
    );
    // And as a struct member.
    agree_with(
        "struct S { int m[2][3]; };",
        "struct S s = {{{1,2,3},{4,5,6}}}; return s.m[1][2];",
    );
}

/// **GNU's `__label__` does not parse, and it is the only keyword left with no production.**
///
///     expected an expression / expected `;` after an expression statement / ...
///
/// # How it was found: a keyword with no production is an opcode with no producer
///
/// Wave 275 landed `_Generic` and noticed its keyword had been sitting in the lexer's table
/// unconsumed since the lexer was written. Running that as a census over all 59 `Kw::` variants
/// leaves exactly one: `Kw::Label`. The axis is nearly exhausted, which is itself the result —
/// but the one it found is not obscure. `__label__` appears in `vppinfra/hash.h`, and
/// `hash_foreach_pair` is a core VPP macro. (`_Generic` appears in *no* VPP file, so this census
/// ordered the two waves backwards on that measure.)
///
/// # A local label is not a renamed one, and the difference is the whole feature
///
/// `__label__ d;` declares `d` local to its block. Two blocks in one function may each declare
/// it, which is exactly why the macro uses it — two `hash_foreach_pair` loops in one function
/// would otherwise define the same label twice. A local label may also coexist with a
/// function-scope label of the same name.
///
/// Lowering keys labels by `Symbol` in one per-function map, so anything that keeps the written
/// name makes the second declaration collide with the first. That is the case worth testing, and
/// the naive implementation passes every other fixture here.
#[test]
fn a_local_label_is_local_to_its_block() {
    // The control: the same shapes with an ordinary function-scope label already work.
    agree(
        "int n = 0; { for (int i=0;i<5;i++){ if(i==2) goto done; n+=1; } done: n+=100; } return n;",
    );
    // The basic form.
    agree(
        "int n = 0; { __label__ done; for (int i=0;i<5;i++){ if(i==2) goto done; n+=1; } done: n+=100; } return n;",
    );
    agree("int n = 0; { __label__ d; n += 1; } return n;");
    // Several names in one declaration, and jumps in both directions.
    agree(
        "int n = 0; { __label__ a, b; goto b; a: n+=1; goto out; b: n+=2; goto a; out: ; } return n;",
    );
    // **Two sibling blocks declaring the same local label.** The case the construct exists for,
    // and the one a per-function label map gets wrong.
    agree(
        "int n = 0; { __label__ d; goto d; d: n+=1; } { __label__ d; goto d; d: n+=10; } return n;",
    );
    agree(
        "int n = 0; { __label__ d; goto d; d: n+=1; } { __label__ d; goto d; d: n+=10; } { __label__ d; goto d; d: n+=100; } return n;",
    );
    // Nested blocks: the inner declaration shadows, and the outer one is intact after it.
    agree(
        "int n = 0; { __label__ d; { __label__ d; goto d; d: n+=1; } goto d; d: n+=10; } return n;",
    );
    // A local label beside a function-scope label of the same name.
    agree("int n = 0; { __label__ d; goto d; d: n+=1; } d: n+=10; return n;");
    // ...and beside an unrelated function-scope label, which must still resolve outward.
    agree("int n = 0; { __label__ d; goto d; d: n+=1; } goto e; e: n+=10; return n;");
    // In a `do`-`while`, which is the shape `hash_foreach_pair` actually uses.
    agree("int n = 0; do { __label__ d; if (n==0) goto d; n+=1; d: n+=2; } while (0); return n;");
    // A jump over a declaration, so the block has more in it than the label.
    agree("int n = 0; { __label__ d; int x = 1; goto d; x = 2; d: n += x; } return n;");
}

/// **`_Generic` does not parse at all.**
///
///     expected an expression / expected `;` after `return` / expected a statement ...
///
/// C11 6.5.1.1, and `_Generic` is already a keyword in the lexer — `Kw::Generic` exists and is
/// mapped, and nothing consumes it. So the token is recognised and then falls out of the primary
/// expression as an unexpected one, taking the rest of the statement with it.
///
/// This is a loud gap: parse diagnostics, so 015 §7 refuses the function and no wrong answer is
/// produced. It is also the last C11 expression form missing, and the one most likely to be met
/// second-hand — `<tgmath.h>` is built out of it and so is most modern type-generic C.
///
/// # gcc's rules, pinned by running it
///
///   - **The controlling expression is not evaluated.** `_Generic(i++, ...)` leaves `i` alone.
///   - Its type is taken **after lvalue conversion**: `const int` matches `int`, an array
///     matches `int *`, a string literal matches `char *`, and a function designator matches a
///     function pointer.
///   - **No integer promotion.** `(unsigned char)1` selects `unsigned char`, not `int` — which
///     is the opposite of what every other context does to a narrow operand, and the single
///     easiest thing to get wrong here.
///   - The result **is** the selected expression: its type, its value, its `sizeof`. Selecting
///     `(char)1` gives something of size 1.
///   - `default` is used only when nothing else matches, wherever it appears in the list.
#[test]
fn generic_selection_agrees_with_gcc() {
    // The basic selection, on each of the two answers.
    agree("return _Generic(1, int: 7, default: 8);");
    agree("return _Generic(1.0, int: 7, default: 8);");
    agree("return _Generic(1, long: 5, int: 6);");
    agree("double d = 1.0; return _Generic(d, float: 1, double: 2, default: 3);");
    agree("float f = 1.0f; return _Generic(f, float: 1, double: 2, default: 3);");
    // **The controlling expression is not evaluated**, which is the rule with a side effect
    // attached and so the only one a value can witness.
    agree("int i = 0; int r = _Generic(i++, int: 7, default: 8); return r*10 + i;");
    agree("int i = 5; int r = _Generic(i = 9, int: 1, default: 2); return r*10 + i;");
    // Lvalue conversion: qualifiers dropped, array and function designators decayed.
    agree("const int c = 1; return _Generic(c, int: 1, default: 2);");
    agree("volatile int v = 1; return _Generic(v, int: 1, default: 2);");
    agree("int a[3]; return _Generic(a, int *: 1, default: 3);");
    // **The stripping is outermost-only.** A qualifier on the *pointee* survives lvalue
    // conversion, so a `const int *` still selects a `const int *` association — and an
    // association naming `const int` matches nothing, because the controlling type never is one.
    agree("const int *p = 0; return _Generic(p, const int *: 1, int *: 3, default: 2);");
    agree("int c = 1; return _Generic(c, const int: 1, default: 2);");
    agree("const int c = 1; return _Generic(c, const int: 1, default: 2);");
    agree("return _Generic(\"s\", char *: 1, default: 3);");
    // **No promotion of the controlling expression.** A narrow type stays narrow.
    agree("unsigned char u = 1; return _Generic(u, unsigned char: 1, int: 2, default: 3);");
    agree("signed char sc = 1; return _Generic(sc, signed char: 1, int: 2, default: 3);");
    agree("short sh = 1; return _Generic(sh, short: 1, int: 2, default: 3);");
    // ...but the *expression's own* type is what is asked about, so an addition of two `char`s
    // is an `int` because the addition promoted them, not because `_Generic` did.
    agree("char a = 1; char b = 1; return _Generic(a + b, int: 1, char: 2, default: 3);");
    // Pointers, including the one that is not `void *`.
    agree("int v; int *p = &v; return _Generic(p, int *: 1, void *: 2, default: 3);");
    agree("int v; void *q = &v; return _Generic(q, int *: 1, void *: 2, default: 3);");
    // `default` first in the list, which must not shadow a later exact match.
    agree("return _Generic(1, default: 8, int: 7);");
    agree("return _Generic(1.0f, default: 8, int: 7);");
    // The result is an expression, with its own type and value.
    agree("return _Generic(1, int: 2 + 3, default: 0) * 10;");
    agree("return (int)sizeof(_Generic(1, int: (char)1, default: 0));");
    agree("return (int)(_Generic(1, int: 1.5, default: 0.0) * 4);");
    // Nested, and used as an ordinary operand.
    agree("return _Generic(1, int: _Generic(1.0, double: 4, default: 5), default: 6);");
    agree("int i = 3; return _Generic(i, int: i, default: 0) + 1;");
    // A single association with no `default`, which must still select.
    agree("return _Generic(1, int: 42);");
}

/// **Vector comparisons are refused, and the reason is the *result* type.**
///
///     `probe` lowered to CIR the verifier rejects (Eq operand is Ptr, declared Int(32))
///
/// Wave 273 did every other operator on a vector and left these, because they are the one shape
/// whose result type is not the operand type. gcc's rules, pinned by running it:
///
///   - The result has the **same total size** and the **same lane count**; only the element type
///     changes, to a **signed integer of the lane's width**. `v4sf == v4sf` is a `v4si`;
///     `v2df == v2df` is a vector of two `long`.
///   - True is **all bits set**, not 1. `(x == y)[0]` is `-1`, which is what makes
///     `x & (x == y)` the mask idiom every SIMD program in VPP is built out of.
///   - An **unsigned** lane compares unsigned. `(v8qu){200} < (v8qu){100}` is false, and would
///     be true if the lane's signedness came from the vector rather than its element.
///   - NaN follows the ordered/unordered split C's scalar operators already use: `n == n` is 0,
///     `n != n` is all ones, `n < n` is 0.
///
/// # Why the result type is the whole difference
///
/// Every other vector operator returns its operand's type, so wave 273's sema branch could say
/// "the result is the vector" and stop. A comparison has to *build* a type: same lanes, same
/// alignment, element replaced. For `v4sf` that element is not the operand's element and not
/// anything already interned for the expression.
#[test]
fn vector_comparisons_agree_with_gcc() {
    let si = "typedef int v4si __attribute__((vector_size(16)));";
    let sf = "typedef float v4sf __attribute__((vector_size(16))); typedef int v4si __attribute__((vector_size(16)));";
    let qu = "typedef unsigned char v8qu __attribute__((vector_size(8)));";
    let df = "typedef double v2df __attribute__((vector_size(16))); typedef long v2di __attribute__((vector_size(16)));";
    // Controls: wave 273's arithmetic, which must not move.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {10,20,30,40}; v4si z = x + y; return z[1];",
    );
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = -x; return z[1];");
    // **All bits set, not 1.** Two lanes summed so a `1` and a `-1` cannot be confused.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x == y); return e[0] + e[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x == y); return e[0]*10 + e[2];",
    );
    // Every relational, on a lane where it is true and one where it is false.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x != y); return e[0]*10 + e[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x < y); return e[1]*10 + e[3];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x > y); return e[3]*10 + e[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x <= y); return e[0]*10 + e[3];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = (x >= y); return e[0]*10 + e[1];",
    );
    // The result is a value like any other: subscripted directly, and used as a mask.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; return (x == y)[0];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si m = x & (x == y); return m[0]*10 + m[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; v4si e = x == y; e += x; return e[0];",
    );
    // Its size is the operand's, which is the claim the element-type rule rests on.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; return (int)sizeof(x == y);",
    );
    // A broadcast scalar on one side.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si e = (x == 1); return e[0]*10 + e[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si e = (2 < x); return e[0]*10 + e[3];",
    );
    // **An unsigned lane compares unsigned**, which is the only thing separating the lane's
    // signedness from the vector's.
    agree_with(
        qu,
        "v8qu c = {200,2,3,4,5,6,7,8}; v8qu d = {100,9,3,0,5,6,7,8}; v8qu e = (c < d); return e[0];",
    );
    agree_with(
        qu,
        "v8qu c = {200,2,3,4,5,6,7,8}; v8qu d = {100,9,3,0,5,6,7,8}; v8qu e = (c > d); return e[0];",
    );
    agree_with(
        qu,
        "v8qu c = {200,2,3,4,5,6,7,8}; v8qu d = {200,9,3,0,5,6,7,8}; v8qu e = (c == d); return e[0];",
    );
    // **Float lanes yield an integer vector**, and NaN follows the ordered/unordered split.
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {1.5f,0.5f,9.5f,4.5f}; v4si e = (f == g); return e[0]*10 + e[1];",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {1.5f,0.5f,9.5f,4.5f}; v4si e = (f < g); return e[2]*10 + e[0];",
    );
    agree_with(
        sf,
        "v4sf n = {0.0f/0.0f,1.0f,1.0f,1.0f}; v4si e = (n == n); return e[0];",
    );
    agree_with(
        sf,
        "v4sf n = {0.0f/0.0f,1.0f,1.0f,1.0f}; v4si e = (n != n); return e[0];",
    );
    agree_with(
        sf,
        "v4sf n = {0.0f/0.0f,1.0f,1.0f,1.0f}; v4si e = (n < n); return e[0];",
    );
    // **`>` and `>=` on floats, which the ordered set cannot spell directly.** CIR has no `FOGt`
    // or `FOGe`, so `cir_fcmpop` expresses them by exchanging the operands — and dropping that
    // swap survived every fixture above, because the float cases only used `==`, `<` and `!=`.
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {1.5f,0.5f,9.5f,4.5f}; v4si e = (f > g); return e[1]*10 + e[2];",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {1.5f,0.5f,9.5f,4.5f}; v4si e = (f >= g); return e[0]*10 + e[2];",
    );
    agree_with(
        sf,
        "v4sf n = {0.0f/0.0f,1.0f,1.0f,1.0f}; v4si e = (n > n); return e[0];",
    );
    agree_with(
        sf,
        "v4sf n = {0.0f/0.0f,1.0f,1.0f,1.0f}; v4si e = (n >= n); return e[0];",
    );
    // **The mask's own element type is signed**, and only reading a lane *through it* can see
    // that. Every fixture above assigns the result to a declared `v4si` first, so the read takes
    // that type and the mask's signedness never shows. Subscripting the comparison directly and
    // then asking for its sign is what separates them: all-ones is negative as a signed lane and
    // enormous as an unsigned one.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; return (x == y)[0] < 0;",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,9,3,0}; return (x == y)[0] / 2;",
    );
    agree_with(
        qu,
        "v8qu c = {200,2,3,4,5,6,7,8}; v8qu d = {200,9,3,0,5,6,7,8}; return (c == d)[0] < 0;",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {1.5f,0.5f,9.5f,4.5f}; return (int)sizeof(f == g);",
    );
    // A 64-bit lane, where the result element is `long` and not `int`.
    agree_with(
        df,
        "v2df p = {1.5,2.5}; v2df q = {1.5,9.5}; v2di e = (p == q); return (int)(e[0]*10 + e[1]);",
    );
    agree_with(
        df,
        "v2df p = {1.5,2.5}; v2df q = {1.5,9.5}; v2di e = (p < q); return (int)(e[1]*10 + e[0]);",
    );
    agree_with(
        df,
        "v2df p = {1.5,2.5}; v2df q = {1.5,9.5}; return (int)sizeof(p == q);",
    );
}

/// **Elementwise vector arithmetic lowers to CIR the verifier rejects.**
///
///     `probe` lowered to CIR the verifier rejects (Add operand is Ptr, declared Int(32))
///
/// Every arithmetic, bitwise and shift operator on a vector, both operand orders of the
/// scalar-broadcast form, unary `-` and `~`, and compound assignment. All refused.
///
/// # Why this is the top of the census's list
///
/// A vector is an aggregate, so lowering hands the generic binary arm a `CTy::Ptr` while the
/// expression's declared type says `Int(32)`. The verifier catches the contradiction and 015 §7
/// refuses the function — loud and honest, and completely useless to anyone reading VPP, which is
/// written in this extension. Wave 272 made the *storage* work; this is the arithmetic on it.
///
/// # The shape is `ptr_arith`'s
///
/// `p + n` is not an `Add` either, and the binary arm already intercepts it before the generic
/// path for exactly this reason. A vector needs the same treatment for the same cause: the
/// operand's CIR type is not the type the operator works at.
///
/// # What is deliberately absent
///
/// **Comparisons.** `x == y` and `x < y` are accepted by gcc and yield a vector of 0/-1 whose
/// element type is a *signed integer of the lane's width* — for `v4sf` that is not the operand
/// type at all. That is a sema change of a different kind from this one, so comparisons keep
/// refusing loudly and 015 §7 keeps naming them. A declared limit, per 023 §7.
#[test]
fn elementwise_vector_arithmetic_agrees_with_gcc() {
    let si = "typedef int v4si __attribute__((vector_size(16)));";
    let sf = "typedef float v4sf __attribute__((vector_size(16)));";
    let qi = "typedef unsigned char v8qi __attribute__((vector_size(8)));";
    // Controls: what wave 272 made work, which must not move.
    agree_with(si, "v4si x = {1,2,3,4}; return x[2];");
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = x; y[0] = 9; return x[0]*10 + y[0];",
    );
    // Vector op vector, every arithmetic operator, read from a lane the operator changed.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {10,20,30,40}; v4si z = x + y; return z[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {10,20,30,40}; v4si z = y - x; return z[2];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {4,3,2,1}; v4si z = x * y; return z[0]*100 + z[3];",
    );
    agree_with(
        si,
        "v4si x = {10,20,30,40}; v4si y = {2,3,4,5}; v4si z = x / y; return z[1];",
    );
    agree_with(
        si,
        "v4si x = {10,20,30,40}; v4si y = {3,3,7,7}; v4si z = x % y; return z[3];",
    );
    // Bitwise and shifts, including a per-lane shift count.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {3,3,3,3}; v4si z = x & y; return z[1];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {8,8,8,8}; v4si z = x | y; return z[2];",
    );
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {5,5,5,5}; v4si z = x ^ y; return z[0];",
    );
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = x << 1; return z[2];");
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,2,1,2}; v4si z = x << y; return z[2];",
    );
    agree_with(
        si,
        "v4si x = {64,64,64,64}; v4si y = {1,2,3,4}; v4si z = x >> y; return z[3];",
    );
    // **The scalar broadcasts, both ways round.** gcc converts the scalar to the vector type,
    // so `1 + x` is not a different operation from `x + 1` and both have to work.
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = x + 1; return z[1];");
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = 1 + x; return z[1];");
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = 10 - x; return z[2];");
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = x * 3; return z[3];");
    // Unary.
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = -x; return z[1];");
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = ~x; return z[1];");
    // Compound assignment, which reads and writes the same object.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {10,20,30,40}; x += y; return x[1];",
    );
    agree_with(si, "v4si x = {1,2,3,4}; x *= 2; return x[2];");
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {1,1,1,1}; x -= y; return x[0];",
    );
    // A narrower, unsigned lane, where the operator's width is the lane's and not `int`'s.
    agree_with(
        qi,
        "v8qi c = {200,2,3,4,5,6,7,8}; v8qi d = {100,0,0,0,0,0,0,0}; v8qi e = c + d; return e[0];",
    );
    agree_with(
        qi,
        "v8qi c = {1,2,3,4,5,6,7,8}; v8qi e = c * 3; return e[7];",
    );
    // Floating lanes.
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {1.0f,1.0f,1.0f,1.0f}; v4sf h = f + g; return (int)h[1];",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf h = f * 2.0f; return (int)h[3];",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf g = {0.5f,0.5f,0.5f,0.5f}; v4sf h = f / g; return (int)h[0];",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf h = -f; return (int)(h[1] * 2);",
    );
    // **A scalar operand whose type is not the lane's.** Every broadcast fixture above happens
    // to use a scalar already at the element type, where the conversion is a no-op and cannot
    // be observed — a mutant removing it survived them all. An `int` beside a `float` lane
    // needs a real `SiToFp`, and a `long` beside an `int` lane a real truncation; C converts
    // the scalar to the **element** type, not to `int` and not to the scalar's own.
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf h = 10 - f; return (int)(h[1]*2);",
    );
    agree_with(
        sf,
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; v4sf h = 2 * f; return (int)h[3];",
    );
    agree_with(si, "v4si x = {1,2,3,4}; v4si z = 10L - x; return z[2];");
    // **The destination is evaluated once** (015 §2.2). `x += y` needs the lvalue's address
    // twice — to read the old lanes and to copy the result back — and the obvious way to write
    // it calls `lvalue_addr` a second time. Every other fixture here has a bare identifier on
    // the left, where a double evaluation is invisible; this one puts a side effect in the
    // subscript, which is the only thing that can see it. Found by a surviving mutant.
    agree_with(
        si,
        "v4si a[2] = {{1,2,3,4},{5,6,7,8}}; v4si y = {10,10,10,10}; int i = 0; a[i++] += y; \
         return a[0][1]*100 + a[1][1]*10 + i;",
    );
    agree_with(
        si,
        "v4si a[2] = {{1,2,3,4},{5,6,7,8}}; int i = 0; a[i++] *= 2; \
         return a[0][0]*100 + a[1][0]*10 + i;",
    );
    // Chained, so a result vector is itself an operand.
    agree_with(
        si,
        "v4si x = {1,2,3,4}; v4si y = {5,6,7,8}; v4si z = (x + y) * 2; return z[2];",
    );
}

/// **A vector subscript is typed `Ty::Error`, so every lane is read as a 32-bit integer.**
///
///     v4sf f; f[1] = 2.5f; return (int)(f[1] + 0.5f);   chiero says 2, gcc says 3
///
/// Nothing to do with initializers — this shape has none. Sema's `Index` arm decays the base and
/// expects a `Ty::Ptr`; C has no vector-to-pointer conversion, so a vector arrives undecayed and
/// falls to `_ => Ty::Error`. Lowering reads an `Error` as `Int(32)`.
///
/// # Why it hid behind the *other* defect, and behind arithmetic
///
/// Every lane read back zero until the initializer was fixed, so no fixture could see the type at
/// all. And once it could, the width is only wrong for some element types:
///
///   - `int` lanes — `Int(32)` is the right type. Correct by accident.
///   - `long` lanes — reads the low four bytes. `{7,8}` gives 8 either way; the coincidence
///     needs a value above 2^32 to break, which no plausible fixture uses.
///   - `char` lanes — reads four bytes from a scaled offset. Wrong.
///   - `float`/`double` lanes — returns the **bit pattern**. `2.5f` came back as 1075838976.
///
/// A store followed by a load of the same lane also round-trips: `f[1] = 2.5f; return (int)f[1]`
/// converts 2.5 to `int` on the way in and reads it back, giving gcc's answer by a different
/// route. It takes arithmetic *on the loaded lane* to separate them, which is why the second
/// fixture below carries a `+ 0.5f` that looks pointless and is not.
#[test]
fn a_vector_lane_is_read_at_its_element_type() {
    let sf = "typedef float v4sf __attribute__((vector_size(16)));";
    let qi = "typedef char v8qi __attribute__((vector_size(8)));";
    let di = "typedef long v2di __attribute__((vector_size(16)));";
    // **The shape that round-trips and proves nothing.** Kept as a control: it agreed with gcc
    // before the fix and after it.
    agree_with(sf, "v4sf f; f[1] = 2.5f; return (int)f[1];");
    // The same store, with arithmetic on the loaded lane.
    agree_with(sf, "v4sf f; f[1] = 2.5f; return (int)(f[1] + 0.5f);");
    agree_with(
        sf,
        "v4sf f; f[0] = 1.25f; f[3] = 4.5f; return (int)((f[0] + f[3]) * 4);",
    );
    // A `char` lane, where the *width* is wrong rather than the interpretation.
    agree_with(qi, "v8qi c; c[5] = 6; c[6] = 7; return c[5];");
    agree_with(qi, "v8qi c; c[0] = -1; return c[0];");
    // A `long` lane above 2^32, which is what the accidental agreement needs to break.
    agree_with(
        di,
        "v2di l; l[1] = 4294967296L + 5L; return (int)(l[1] >> 32);",
    );
    agree_with(di, "v2di l; l[0] = -1L; return (int)(l[0] >> 40);");
    // **`_Generic` would say the lane's type directly and the parser does not have it.** That is
    // a separate, loud gap — a parse diagnostic, so 015 §7 refuses the function rather than
    // guessing — and it is recorded in §9 rather than worked around here.
    // `sizeof` on a lane reads the type and nothing else, which is the next best witness.
    agree_with(sf, "v4sf f; return (int)sizeof(f[0]);");
    agree_with(qi, "v8qi c; return (int)sizeof(c[0]);");
    agree_with(di, "v2di l; return (int)sizeof(l[0]);");
}

/// **A vector's braced initializer is silently dropped, and every lane reads back zero.**
///
///     v4si x = {1,2,3,4}; return x[2];   chiero says 0, gcc says 3
///
/// A **wrong answer**, which is the worst outcome there is — not a refusal, not a declared gap.
/// `lower::init_list` builds its slot list from `Ty::Record` and `Ty::Array` and lets everything
/// else fall out of a bare `_ => return`. A vector goes down that path, the initializer is
/// discarded, and the object keeps whatever it had.
///
/// # How it was found
///
/// Wave 271 censused `CmpOp` and found dead opcodes were the fingerprint of a missing feature.
/// Running the same census over every CIR enum: `CTy::Vector`, `RValue::Splat`, `Shuffle`,
/// `InsertLane`, `ExtractLane` and `Const::Wide` all have executor arms, CIR tests and a text
/// format — and **no producer in lowering at all**. GCC's `vector_size` is the C-level feature
/// they were built for, and it is not decoration: VPP, this project's stated target, is written
/// in it.
///
/// The census predicted a *missing* feature. What the probe found is worse than missing — the
/// type is half-supported. `sizeof` is right, and a subscript store followed by a load is right,
/// so nothing announces that the type is not really there.
///
/// # 020 §5, which this is the exact counter-example to
///
/// "A gap is a diagnostic rather than a licence." The `_ => return` is a licence: it declines to
/// initialize and says nothing, so 015 §7 never refuses the function and the differential oracle
/// gets a confident wrong number instead of a skip.
///
/// # `{0}` is the control that says why this went unnoticed
///
/// It is the one shape that agrees, because a dropped initializer leaves zeroes and `{0}` wanted
/// zeroes. Any fixture written with the obvious smoke-test initializer would have passed.
#[test]
fn a_vector_s_braced_initializer_is_stored() {
    let si = "typedef int v4si __attribute__((vector_size(16)));";
    // The controls: everything about the type that already works.
    agree_with(si, "return (int)sizeof(v4si);");
    agree_with(si, "v4si x; return (int)sizeof(x);");
    agree_with(si, "v4si x; x[0] = 7; x[3] = 9; return x[0] + x[3];");
    agree_with(si, "int a[4] = {1,2,3,4}; return a[2];");
    // **The accidental pass.** A dropped initializer leaves zeroes, and this one wanted zeroes.
    agree_with(si, "v4si x = {0}; return x[1];");
    // The defect, on every lane.
    agree_with(si, "v4si x = {1,2,3,4}; return x[0];");
    agree_with(si, "v4si x = {1,2,3,4}; return x[1];");
    agree_with(si, "v4si x = {1,2,3,4}; return x[2];");
    agree_with(si, "v4si x = {1,2,3,4}; return x[3];");
    // The bytes really are not there, not merely unreadable through a subscript.
    agree_with(si, "v4si x = {1,2,3,4}; return ((int*)&x)[2];");
    agree_with(si, "v4si x = {1,2,3,4}; int *p = (int*)&x; return p[2];");
    // A short initializer zero-fills the rest — one of the two array rules `init_list` already
    // implements, and which never got to apply here.
    //
    // **The other one does not transfer, and gcc said so.** This fixture first carried
    // `v4si x = {[2] = 5}` and gcc rejected it outright: *array index in non-array initializer*.
    // A vector takes an array's *walk* but not its designators, and the probe that suggested
    // otherwise was reading gcc's compile error as a chiero disagreement.
    agree_with(
        si,
        "v4si x = {1,2}; return x[0] + x[1]*10 + x[2]*100 + x[3]*1000;",
    );
    agree_with(si, "v4si x = {1}; return x[0]*1000 + x[3];");
    // A copy of an initialized vector.
    agree_with(si, "v4si x = {1,2,3,4}; v4si y = x; return y[2];");
    // **Static and file-scope storage take a different path**, and it is broken too.
    agree_with(
        "typedef int v4si __attribute__((vector_size(16))); static v4si g = {1,2,3,4};",
        "return g[2];",
    );
    agree_with(
        "typedef int v4si __attribute__((vector_size(16))); v4si g = {5,6,7,8};",
        "return g[3];",
    );
    // Every element type, since the lane size is what the slot walk has to get right.
    agree_with(
        "typedef float v4sf __attribute__((vector_size(16)));",
        "v4sf f = {1.5f,2.5f,3.5f,4.5f}; return (int)f[1];",
    );
    agree_with(
        "typedef char v8qi __attribute__((vector_size(8)));",
        "v8qi c = {1,2,3,4,5,6,7,8}; return c[5];",
    );
    agree_with(
        "typedef double v2df __attribute__((vector_size(16)));",
        "v2df d = {1.5,2.5}; return (int)(d[0] + d[1]);",
    );
    agree_with(
        "typedef long v2di __attribute__((vector_size(16)));",
        "v2di l = {7,8}; return (int)l[1];",
    );
}

/// **C's floating classification macros are refused, and CIR has had the opcodes all along.**
///
///     `probe` contains a construct lowering cannot represent, so it was skipped
///
/// `isnan`, `isunordered`, `isless` and the rest of 7.12.14 are `<math.h>` macros over
/// `__builtin_*`, and every numeric C program in the world uses them. Lowering refuses all seven.
///
/// # How they were found, which is a census with a direction
///
/// Wave 270 censused `ExprKind` — what the AST can hold against what the generator emits. This is
/// the same question asked of **CIR**: which `CmpOp` variants can lowering ever emit? Twelve of
/// twenty can. The six that cannot are `FONe`, `FUEq`, `FULt`, `FULe`, `FOrd` and `FUno` — and
/// they are not junk, they are exactly the ordered/unordered distinctions C's *macros* make and
/// C's *operators* do not. The dead opcodes were the fingerprint of the missing feature.
///
/// `chiero-exec` implements all twenty. So six comparison semantics have been sitting in the
/// engine, untested and unreachable, waiting for a producer.
///
/// # This is a refusal, not a wrong answer
///
/// 015 §7 refuses the function and says so, which is the honest outcome and is why no oracle ever
/// flagged it. Wave 270's lesson was that a refusal costs nothing to hide behind; this is the
/// other half — a refusal that is *correct* still marks a capability that is simply absent.
///
/// # The controls matter more than usual here
///
/// C's own operators must keep their NaN behaviour exactly: `a != a` is **true** for NaN
/// (unordered), `a == a` is false, and every relational is false. Those five already agree with
/// gcc, and they are what the new opcodes must not disturb — `!=` is `FUNe` and `islessgreater`
/// is `FONe`, which differ *only* when an operand is NaN.
#[test]
fn the_floating_classification_builtins_agree_with_gcc() {
    // The controls: C's operators, whose NaN semantics are already right.
    agree("double a = 0.0/0.0; return a != a;");
    agree("double a = 0.0/0.0; return a == a;");
    agree("double a = 0.0/0.0, b = 1.0; return a < b;");
    agree("double a = 0.0/0.0, b = 1.0; return a >= b;");
    agree("double a = 0.0/0.0, b = 1.0; return !(a < b);");
    // `isnan`, on both answers.
    agree("double a = 0.0/0.0; return __builtin_isnan(a);");
    agree("double a = 1.0; return __builtin_isnan(a);");
    agree("double a = 0.0; return __builtin_isnan(a) + 2;");
    // Unorderedness itself.
    agree("double a = 1.0, b = 2.0; return __builtin_isunordered(a, b);");
    agree("double a = 0.0/0.0, b = 2.0; return __builtin_isunordered(a, b);");
    agree("double a = 1.0, b = 0.0/0.0; return __builtin_isunordered(a, b);");
    // The four relational macros, each on a true case, a false case and a NaN case.
    agree("double a = 1.0, b = 2.0; return __builtin_isless(a, b);");
    agree("double a = 2.0, b = 1.0; return __builtin_isless(a, b);");
    agree("double a = 0.0/0.0, b = 1.0; return __builtin_isless(a, b);");
    agree("double a = 1.0, b = 2.0; return __builtin_islessequal(a, b);");
    agree("double a = 2.0, b = 2.0; return __builtin_islessequal(a, b);");
    agree("double a = 0.0/0.0, b = 1.0; return __builtin_islessequal(a, b);");
    agree("double a = 2.0, b = 1.0; return __builtin_isgreater(a, b);");
    agree("double a = 1.0, b = 2.0; return __builtin_isgreater(a, b);");
    agree("double a = 0.0/0.0, b = 1.0; return __builtin_isgreater(a, b);");
    agree("double a = 2.0, b = 2.0; return __builtin_isgreaterequal(a, b);");
    agree("double a = 1.0, b = 2.0; return __builtin_isgreaterequal(a, b);");
    agree("double a = 0.0/0.0, b = 1.0; return __builtin_isgreaterequal(a, b);");
    // **`islessgreater` is the one that is not `!=`.** They differ exactly on NaN, which is the
    // whole reason `FONe` exists beside `FUNe`.
    agree("double a = 1.0, b = 2.0; return __builtin_islessgreater(a, b);");
    agree("double a = 2.0, b = 2.0; return __builtin_islessgreater(a, b);");
    agree("double a = 0.0/0.0, b = 1.0; return __builtin_islessgreater(a, b);");
    agree("double a = 0.0/0.0, b = 1.0; return (a != b) - __builtin_islessgreater(a, b);");
    // The operands take the usual arithmetic conversions, so a mixed pair must agree too.
    agree("float f = 1.5f; double d = 2.0; return __builtin_isless(f, d);");
    agree("long double l = 1.0L; double d = 2.0; return __builtin_isless(l, d);");
    agree("float f = 0.0f/0.0f; return __builtin_isnan(f);");
    agree("long double l = 0.0L/0.0L; return __builtin_isnan(l);");
    // **The result's *type* is `int`, which takes two shapes to pin.** Nothing declares these
    // builtins, so sema types an undeclared callee's result `Ty::Error` and the result with it.
    // Almost every use survives that — an `Error` still lowers to a 32-bit value and
    // `isnan(a) + 2` comes out right — so it took mutation to find the two uses that do not:
    // `sizeof` reads the type directly, and mixing with a `double` needs the usual arithmetic
    // conversions, which an `Error` operand poisons for the whole expression.
    agree("double a = 0.0/0.0; return (int)sizeof(__builtin_isnan(a));");
    agree("double a = 0.0/0.0; return (int)(__builtin_isnan(a) * 2.5);");
    agree("double a = 1.0, b = 2.0; return (int)sizeof(__builtin_islessgreater(a, b));");
    agree("double a = 1.0, b = 2.0; return (int)(__builtin_isless(a, b) * 1.5);");
    // And the ordinary integer uses, which were right either way and say so.
    agree("double a = 0.0/0.0; return __builtin_isnan(a) ? 7 : 8;");
    agree("double a = 0.0/0.0; return ~__builtin_isnan(a);");
    agree("double a = 0.0/0.0; int arr[4] = {0,1,2,3}; return arr[__builtin_isnan(a)];");
    // A negative zero is ordered and equal to zero, for all of them.
    agree("double a = -0.0, b = 0.0; return __builtin_isless(a, b);");
    agree("double a = -0.0, b = 0.0; return __builtin_islessequal(a, b);");
    agree("double a = -0.0, b = 0.0; return __builtin_islessgreater(a, b);");
    agree("double a = -0.0, b = 0.0; return __builtin_isunordered(a, b);");
}

/// **`!` on a negative zero says the value is non-zero.**
///
/// C11 6.5.3.3p5: `!E` is `E == 0`. IEEE-754 makes `-0.0 == 0.0` true, so `!(-0.0)` is 1. Chiero
/// says 0 — it is testing the *bits*, which are non-zero for a negative zero, rather than comparing
/// against zero.
///
/// # How it was found, which is the part worth keeping
///
/// Censusing what `ExprKind` can hold against what the generator emits: of twenty-one variants,
/// three appear in no generated program at all — `_Alignof`, a statement expression, and **`!`**.
/// The first two are rare in real code. `!` is everywhere, and it had never been through the
/// differential oracle in any shape.
///
/// That is wave 217's technique — ask what the AST can hold rather than run more seeds — and it
/// found a wrong answer on the first probe, before the generator was touched at all. Waves 250–253
/// are the reason for probing first: adding a construct to the corpus is worth nothing if the
/// context it appears in cannot discriminate, and a handful of hand-written shapes settles that in
/// a minute.
///
/// # The three controls are what make it a `!` defect rather than a truth-testing one
///
/// `if (d)`, `d ? 1 : 2` and `d == 0.0` all agree with gcc on the same value. So the engine's truth
/// test is right and only `!` is wrong, which is a much narrower thing to fix than it first looked.
#[test]
fn logical_not_of_a_float_agrees_with_gcc() {
    // The controls: every other way of asking "is this zero" is already correct.
    agree("double d = -0.0; if (d) return 7; return 8;");
    agree("double d = -0.0; return d ? 1 : 2;");
    agree("double d = -0.0; return d == 0.0;");
    // The defect.
    agree("double d = -0.0; return !d;");
    agree("float f = -0.0f; return !f;");
    agree("long double l = -0.0L; return !l;");
    // Ordinary values, which must keep working.
    agree("double d = 0.0; return !d;");
    agree("double d = 3.5; return !d;");
    agree("double n = 0.0/0.0; return !n;");
    // And on the integer and pointer types, where `!` was equally untested.
    agree("unsigned char c = 200; return !c;");
    agree("int *p = 0; return !p;");
    agree("unsigned u = 4294967295u; return !!u;");
    agree("struct S { unsigned a : 3; }; struct S s; s.a = 5; return !s.a;");
}

/// **An `unsigned` bit-field whose top bit is set reads back sign-extended.**
///
/// Found while writing a fixture for something else entirely: `struct S { unsigned a : 3; }` with
/// `s.a = 5` returns **-3**, and `5` is `0b101` — sign-extended from three bits, that is exactly
/// -3. The width is respected and the *signedness* is not.
///
/// C11 6.7.2.1p10 gives a bit-field the declared type's signedness, so an `unsigned` field zero-
/// extends. Nothing here is exotic: three bits, one assignment, one read.
///
/// **Why a hundred waves of differential testing missed it.** The generator emits bit-fields (wave
/// 142) but the value has to have its *top* bit set for the two extensions to differ — `s.a = 3` in
/// three bits is `0b011` and reads back 3 either way. Half the values of any width are invisible to
/// this bug, and the small ones a fixture author reaches for are exactly the invisible half.
#[test]
fn an_unsigned_bitfield_zero_extends() {
    // **Where the defect fires and where it hides**, which is what wave 253 set out to find. All
    // four of these pass today; before wave 249's fix only the *last* failed. That asymmetry is the
    // fifth condition wave 251's four-factor model was missing:
    //
    //   `(long)(s.a)`  the member is an operand of an explicit cast, so `top(e)` is the field's own
    //                  type — `is_signed(e)` answered `unsigned` and the bug never fired
    //   `return s.a`   the member is converted to the return type, so `top(e)` is the promoted
    //                  `int`, which is signed, and the bug fired
    //
    // The generator reads a bit-field exactly one way — `(long)(x.f)` — which is the hiding shape,
    // and that is why five waves of rate improvements never caught anything. The fixtures stay
    // because the boundary is worth pinning: a change to how conversions are recorded could move a
    // read from one side of it to the other, and only the pair would show it.
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 4; return (int)(long)(s.a);",
    );
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 4; long v = (long)(s.a); return (int)v;",
    );
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 4; long acc = 0; acc = acc * 31 + (long)(s.a); return (int)acc;",
    );
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 4; return s.a;",
    );

    // Every width where the top bit is set, so the two extensions disagree.
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 5; return s.a;",
    );
    agree_with(
        "struct S { unsigned a : 1; };\n",
        "struct S s; s.a = 1; return s.a;",
    );
    agree_with(
        "struct S { unsigned a : 4; };\n",
        "struct S s; s.a = 9; return s.a;",
    );
    agree_with(
        "struct S { unsigned a : 7; };\n",
        "struct S s; s.a = 100; return s.a;",
    );
    agree_with(
        "struct S { unsigned a : 31; };\n",
        "struct S s; s.a = 0x7fffffff; return s.a;",
    );
    // A *signed* bit-field must still sign-extend, which is the control that stops the fix from
    // simply never extending.
    agree_with(
        "struct S { signed a : 3; };\n",
        "struct S s; s.a = -3; return s.a;",
    );
    agree_with(
        "struct S { signed a : 4; };\n",
        "struct S s; s.a = -8; return s.a;",
    );
    // And values with the top bit clear, which read back the same either way — the controls that
    // hid this for a hundred waves.
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 3; return s.a;",
    );
    agree_with(
        "struct S { signed a : 4; };\n",
        "struct S s; s.a = 3; return s.a;",
    );
    // A second field after it, so the shift is exercised as well as the mask.
    agree_with(
        "struct S { unsigned a : 3; unsigned b : 5; };\n",
        "struct S s; s.a = 5; s.b = 17; return s.b;",
    );
    // **A read-modify-write, with an operator truncation does not commute with.** Mutation kept
    // the compound-assignment path's signedness alive through every `+=` fixture, for the reason
    // wave 217 recorded about `_Bool`: `s.a += 1` reads 5 or -3, adds one, and truncates to three
    // bits — 6 either way. Division and shift do not commute with truncation, so they can see
    // which value the read produced.
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 5; s.a /= 2; return s.a;",
    );
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 5; s.a >>= 1; return s.a;",
    );
    // The same operator on a plain read, which is the other path and already passes.
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 5; return s.a / 2;",
    );
    // And the signed control, which must still sign-extend through the same route.
    agree_with(
        "struct S { signed a : 4; };\n",
        "struct S s; s.a = -3; s.a /= 2; return s.a;",
    );
    // **Postfix increment, which is the only shape that shows the increment path's read.**
    // `/=` and `>>=` go through `assign`; `++` has its own site, and there the *value* is what a
    // postfix form yields — the byte as it was read, before the addition. `s.a++` on a three-bit
    // `unsigned` holding 5 yields 5 or -3 depending on the extension, where `++s.a` yields 6 either
    // way because truncation commutes with the `+ 1`.
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 5; return s.a++;",
    );
    agree_with(
        "struct S { unsigned a : 3; };\n",
        "struct S s; s.a = 5; return ++s.a;",
    );
    agree_with(
        "struct S { signed a : 4; };\n",
        "struct S s; s.a = -3; return s.a++;",
    );
}

/// **Subnormal `long double`s — the last float gap, and it is a wrong answer rather than a gap.**
///
/// x87's smallest normal is `2^-16382`; below it the format keeps going with the integer bit *clear*
/// and the exponent field pinned at zero, down to `2^-16445`. That is gradual underflow, and it is
/// what stops `x - y` from being zero for two distinct values.
///
/// Every part of `fp` declines these. Conversion refuses a scale that would produce one, and all four
/// operations return `None` for an operand with a zero exponent field or a result below the floor.
/// **The refusal in the literal path is the serious one**, because a refused literal does not stay
/// refused: it falls through to the `f64` value `float_literal` computes, `0x1p-16400` is zero in an
/// `f64`, and so the program gets a confident *zero* where the value is merely very small. That is
/// the shape 023 §7 exists to forbid, and it is the same fall-through that made wave 240's decimal
/// literals wrong.
///
/// # What the fixtures pin
///
/// Subnormals have to work on the way *in* (a literal), on the way *out* (a result that underflows),
/// and in between (an operand that is already one, which every operation must normalize before it can
/// use it). The three are separate code, so:
///
///   - literals at the top, the bottom, and one step below the bottom, which rounds to zero
///   - each of the four operations producing one by underflow
///   - each of them consuming one, including `0x1p-16400L / 0x1p-16445L`, where *both* operands are
///     subnormal and the quotient is an ordinary `2^45`
///   - the round trip `(smallest normal - smallest subnormal) + smallest subnormal`, which is only
///     the smallest normal again if the intermediate kept all sixty-three of its bits
#[test]
fn subnormal_long_doubles_agree_with_gcc() {
    // On the way in. The third rounds to zero, so "any nonzero" is not enough to pass.
    agree("return (int)(0x1p-16400L != 0.0L);");
    agree("return (int)(0x1p-16445L != 0.0L);");
    agree("return (int)(0x1p-16446L == 0.0L);");
    agree("return (int)(1e-4950L != 0.0L);");
    agree("return (int)(0x1p-16400L < 0x1p-16382L);");
    // On the way out, once per operation.
    agree("return (int)(0x1p-16400L + 0x1p-16400L == 0x1p-16399L);");
    agree("return (int)(0x1p-16382L * 0x1p-20L == 0x1p-16402L);");
    agree("return (int)(0x1p-16382L / 0x1p20L == 0x1p-16402L);");
    agree("return (int)(0x1p-16382L - 0x1p-16445L < 0x1p-16382L);");
    // **Gradual underflow, stated as a round trip.** The difference is the largest subnormal, and
    // adding back what was taken returns the smallest normal only if every bit survived.
    agree("return (int)((0x1p-16382L - 0x1p-16445L) + 0x1p-16445L == 0x1p-16382L);");
    // On the way through: an operand that is already subnormal must be normalized before use.
    agree("return (int)(0x1.8p-16445L * 1.0L == 0x1p-16444L);");
    agree("return (int)(0x1p-16444L + 0x1p-16445L == 0x1.8p-16444L);");
    // Both operands subnormal, and an entirely ordinary quotient.
    agree("return (int)(0x1p-16400L / 0x1p-16445L == 0x1p45L);");
    // Under the floor is a zero, which is the one place a zero *is* the right answer.
    agree("return (int)(0x1p-16445L * 0.5L == 0.0L);");
    // **A subnormal operand producing a *normal* result**, which is the only shape that catches a
    // conversion that failed to normalize it. Mutation is why these exist: multiplication and
    // division turn out to be scale-invariant — `pack`'s denormal shift undoes exactly what
    // `unpack`'s normalization did — so every fixture whose result is *also* subnormal passes
    // either way. Only a result that escapes the subnormal band can tell.
    agree("return (int)(0x1p-16400L * 0x1p16000L == 0x1p-400L);");
    agree("return (int)(0x1p-16400L / 0x1p-16000L == 0x1p-400L);");
    agree("return (int)(0x1.8p-16444L * 0x1p16000L == 0x1.8p-444L);");
    agree("return (int)(0x1p-16400L + 0x1p0L == 0x1p0L);");
    // **The denormal shift discards bits, and they decide the rounding.** Two products with the
    // same exponents, differing only in one bit at the very bottom of one operand:
    //
    //   `2^-8223 × 2^-8223`               exactly half the smallest subnormal — a tie, to zero
    //   `2^-8223 × (1 + 2^-63)·2^-8223`   a hair above that tie, so it rounds up to the smallest
    //
    // The bits that make the difference are shifted out by the denormal step itself, so this is
    // the one pair that proves that step keeps a sticky flag. It was found by enumerating the
    // algorithm at five- to eight-bit significands: the pattern needs sixty-two consecutive zeros
    // in the product's low half, which three million random cases never produced.
    agree("return (int)(0x1p-8223L * 0x1p-8223L == 0.0L);");
    agree("return (int)(0x1p-8223L * 0x1.0000000000000002p-8223L == 0x1p-16445L);");
    // Ties inside the subnormal band go to the even candidate, same as everywhere else.
    agree("return (int)(0x1.0000000000000002p-16382L * 0.5L == 0x1p-16383L);");
    agree("return (int)(0x1.0000000000000006p-16382L * 0.5L == 0x1.0000000000000008p-16383L);");
}

/// **Narrowing `long double` to `float`** — the last float gap, and the only one left after wave 244.
///
/// `fcast` rounds `f80` to `f64` and lets the target round that to `f32`. Two roundings are not one
/// rounding, so it refuses instead, and the comment there says why. That refusal is the honest
/// answer and it is also the last body the disjunction test has to work with.
///
/// # The witness, and why the obvious fixtures cannot find it
///
/// `1 + 2^-24 + 2^-60` is a hair above the midpoint between `1.0f` and the next `float`, so a single
/// correct rounding takes it **up** to `0x1.000002p0f`. Round it to `f64` first and the `2^-60`
/// vanishes — it is below half an ulp at that width — leaving exactly the midpoint, which then ties
/// to even and gives `1.0f`. One value, two answers, and the wrong one is the value a reader would
/// never question.
///
/// Every round-numbered fixture below agrees under both schemes, which is the point of including
/// them: they are the controls that stop a fix from buying the hard case by breaking the easy ones.
///
/// # What else has to survive the trip
///
/// `f32`'s range is far narrower than `f80`'s at both ends, so narrowing has its own overflow, its
/// own subnormal band and its own floor — three things wave 244 built for `f80` and which do not
/// transfer, because they are at a different width. Hence `0x1p1000L` (an infinity), `0x1p-140L` (an
/// `f32` subnormal), `0x1p-149L` (the smallest one) and `0x1p-150L` (a zero).
#[test]
fn narrowing_a_long_double_to_float_agrees_with_gcc() {
    // **The double-rounding witness**, in both signs.
    agree("return (int)((float)0x1.0000010000000010p0L == 0x1.000002p0f);");
    agree("return (int)((float)-0x1.0000010000000010p0L == -0x1.000002p0f);");
    // Controls: values every scheme agrees on.
    agree("return (int)((float)1.0L == 1.0f);");
    agree("return (int)((float)0x1.8p3L == 12.0f);");
    agree("return (int)((float)0x1p200L > 0x1p120f);");
    // Past `f32`'s top, which is an infinity there and an ordinary number in `f80`.
    agree("return (int)((float)0x1p1000L > 0x1p120f);");
    // `f32`'s own subnormal band and its own floor, neither of which is `f80`'s.
    agree("return (int)((float)0x1p-140L > 0.0f);");
    agree("return (int)((float)0x1p-149L > 0.0f);");
    agree("return (int)((float)0x1p-150L == 0.0f);");
    agree("return (int)((float)0x1p-200L == 0.0f);");
    // A NaN and an infinity survive the narrowing as themselves.
    agree("long double n = 0.0L/0.0L; float f = (float)n; return (int)(f != f);");
    agree("long double i = 1.0L/0.0L; float f = (float)i; return (int)(f > 0x1p120f);");
    // **Both signs of infinity**, because the sign is a separate `|` from the exponent and a fix
    // that dropped it would still answer "very large" for the positive case.
    agree("long double i = -1.0L/0.0L; float f = (float)i; return (int)(f < -0x1p120f);");
    // **A rounding that carries out of twenty-four bits.** `0x1.ffffff` is above the midpoint
    // between the largest `float` under two and two itself, so it rounds up and the significand
    // overflows into a new power of two — the same carry `f80` needs at sixty-four bits, which is
    // why `round_to` tests for it two ways. Nothing else here reaches the narrow one.
    agree("return (int)((float)0x1.ffffffp0L == 2.0f);");
    agree("return (int)((float)0x1.ffffff0000000000p0L == 2.0f);");
    // And the value just under it, which must *not* round up. The control for the pair.
    agree("return (int)((float)0x1.fffffep0L == 0x1.fffffep0f);");
}

/// **A NaN produced by arithmetic agrees with gcc.**
///
/// Division made this reachable: `0.0L / 0.0L` is how C creates a NaN, and until wave 242 there was
/// no way to make one at all — which is why every `fp` unit test until now had to build its NaNs from
/// raw bits.
///
/// # The payload question, settled by asking the hardware
///
/// §9 recorded this as a decision to make: whether minting a *canonical* quiet NaN is honest when
/// IEEE-754 §6.2 says an operation should return the payload of one of its NaN operands. The answer
/// is that the question does not arise, because x87's behaviour is small enough to implement exactly:
///
/// ```text
///   an invalid operation      the "real indefinite": sign 1, significand 0xC000000000000000
///   one NaN operand           that NaN, with the quiet bit set and everything else untouched
///   two NaN operands          the one with the larger significand, its sign included
/// ```
///
/// Every one of those was read off a running program rather than a manual, and with `volatile`
/// operands so gcc could not fold them — the constant-folded and runtime answers agree, which is
/// itself worth knowing. So there is no approximation here and nothing for 023 §7 to declare.
///
/// **`-x + y` is not among the fixtures**, and deliberately: gcc rewrites it to `y - x`, which is
/// exact for every operand except a NaN, whose sign then comes from the other one. That is a
/// difference between chiero and *this compiler's* algebra rather than between chiero and the format.
#[test]
fn a_nan_from_arithmetic_agrees_with_gcc() {
    // **The definition of a NaN, as a C program can see it**: unordered with everything, itself
    // included (IEEE-754 §5.11).
    agree("long double n = 0.0L / 0.0L; return (int)(n != n);");
    agree("long double n = 0.0L / 0.0L; return (int)(n == n);");
    agree("long double n = 0.0L / 0.0L; return (int)(n < 1.0L);");
    agree("long double n = 0.0L / 0.0L; return (int)(n > 1.0L);");
    agree("long double n = 0.0L / 0.0L; return (int)(n <= n);");
    // The other three invalid operations, each reaching the same NaN by a different route.
    agree("long double i = 1.0L / 0.0L; long double n = i - i; return (int)(n != n);");
    agree("long double i = 1.0L / 0.0L; long double n = i / i; return (int)(n != n);");
    agree("long double i = 1.0L / 0.0L; long double n = 0.0L * i; return (int)(n != n);");
    // **Propagation.** A NaN going into any of the four comes out of it.
    agree("long double n = 0.0L / 0.0L; return (int)((n + 1.0L) != (n + 1.0L));");
    agree("long double n = 0.0L / 0.0L; return (int)((n - 1.0L) != (n - 1.0L));");
    agree("long double n = 0.0L / 0.0L; return (int)((n * 2.0L) != (n * 2.0L));");
    agree("long double n = 0.0L / 0.0L; return (int)((n / 2.0L) != (n / 2.0L));");
    agree("long double n = 0.0L / 0.0L; return (int)((1.0L + n) != (1.0L + n));");
    agree("long double n = 0.0L / 0.0L; return (int)((2.0L / n) != (2.0L / n));");
    // A NaN times a zero is still a NaN, which is the case an implementation keyed on the *other*
    // operand's zero-ness would answer with a zero.
    agree("long double n = 0.0L / 0.0L; return (int)((n * 0.0L) != (n * 0.0L));");
    agree("long double n = 0.0L / 0.0L; return (int)((n + 0.0L) != (n + 0.0L));");
    // And a NaN reaching an infinity does not become one.
    agree("long double n = 0.0L / 0.0L, i = 1.0L / 0.0L; return (int)((n + i) != (n + i));");
    agree("long double n = 0.0L / 0.0L, i = 1.0L / 0.0L; return (int)((n * i) != (n * i));");
    // The control. Arithmetic that produces no NaN must still produce a number, or "everything is
    // a NaN" would satisfy every assertion above.
    agree("long double a = 6.0L, b = 3.0L; return (int)(a / b == 2.0L);");
    agree("long double i = 1.0L / 0.0L; return (int)(i == i);");
}

/// **Adding and subtracting `long double`s agrees with gcc.**
///
/// Harder than multiplication in three specific ways, which is why it went second.
///
/// **Alignment.** Two significands can only be added once they mean the same thing, so the smaller
/// operand shifts right by the exponent difference — and the bits it shifts *out* still matter, because
/// they decide whether the result rounds up. Multiplication had no such shift: the product was exact
/// in a `u128` and rounded once, at the end.
///
/// **Cancellation.** Subtracting near-equal values leaves a significand with leading zeros, and
/// renormalizing it means shifting left by however many there are — up to sixty-three places, where
/// multiplication's normalization was one bit decided by one test. `(1 + 2^-63) - 1` is the shape: two
/// operands agreeing in every bit but the last, and a result sixty-three binary orders below them.
///
/// **Sign.** `a + b` with opposite signs is a subtraction, `a - b` with opposite signs is an addition,
/// and which operand is subtracted from which depends on their *magnitudes* rather than their order.
/// The zero this produces has a sign rule of its own: IEEE-754 §6.3 makes `x - x` a positive zero
/// under round-to-nearest, so the one case where the answer is exactly zero is the one case where the
/// operands' signs do not determine the result's.
#[test]
fn adding_long_doubles_agrees_with_gcc() {
    agree("long double a = 2.0L, b = 3.0L; return (int)(a + b);");
    agree("long double a = 2.0L, b = 3.0L; return (int)(a - b);");
    agree("long double a = -2.0L, b = 3.0L; return (int)(a + b);");
    agree("long double a = -2.0L, b = -3.0L; return (int)(a + b);");
    agree("long double a = 2.0L, b = -3.0L; return (int)(a - b);");
    // Zero, on both sides and as a result.
    agree("long double a = 3.0L, b = 0.0L; return (int)(a + b == 3.0L);");
    agree("long double a = 3.0L; return (int)(a - a == 0.0L);");
    // **Exact, at the far end of the significand.** `1 + 2^-63` needs all sixty-four bits, so an
    // implementation that aligned into fifty-three would lose the smaller operand entirely.
    agree("long double a = 1.0L, b = 0x1p-63L; return (int)(a + b == 0x1.0000000000000002p0L);");
    // **Cancellation**: sixty-three leading zeros to renormalize past.
    agree("long double a = 0x1.0000000000000002p0L; return (int)(a - 1.0L == 0x1p-63L);");
    // **Alignment throws bits away, and they still decide the rounding.** `2^-64` is one place
    // below the last bit of `1.0`, so the sum is an exact tie — and the tie goes to the even
    // candidate, which is `1.0` itself.
    agree("long double a = 1.0L, b = 0x1p-64L; return (int)(a + b == 1.0L);");
    // The same tie one bit up in the significand, where the even candidate is the *other* one.
    agree(
        "long double a = 0x1.0000000000000002p0L, b = 0x1p-64L; \
         return (int)(a + b == 0x1.0000000000000004p0L);",
    );
    // A smaller operand shifted out entirely, which must not disturb the larger.
    agree("long double a = 1.0L, b = 0x1p-200L; return (int)(a + b == 1.0L);");
    agree("long double a = 1.0L, b = 0x1p-200L; return (int)(a + b > 1.0L);");
    // Past `f64`'s range at both ends, which is what this format is for.
    agree("long double a = 0x1p1000L, b = 0x1p1000L; return (int)(a + b == 0x1p1001L);");
    agree("long double a = 1e4000L, b = 1e4000L; return (int)(a + b > 1e4000L);");
    // Carrying out of the significand: `2^64 - 1` plus one is a new power of two.
    agree("long double a = 0x1.fffffffffffffffep0L, b = 0x1p-63L; return (int)(a + b == 0x1p1L);");
    // **The pair that pins what alignment throws away.** Both subtract something just past the
    // last bit of `1.0`, and they differ only in whether anything survives below it:
    //
    //   `1 - 2^-65`            exactly half an ulp down — a tie, and `1.0` is the even side
    //   `1 - 2^-65 - 2^-128`   a hair below that tie, so it goes to the *lower* neighbour
    //
    // The second is what makes the discarded bits load-bearing: they are gone from the arithmetic,
    // and forgetting them turns a value below the tie back into the tie itself. The witness came
    // from enumerating the algorithm exhaustively at a narrower significand width, because a random
    // search never lands on a tie — two random sixty-four-bit significands hit one with probability
    // about 2^-62, and a 240,000-case soak against the hardware found nothing here at all.
    agree("return (int)(1.0L - 0x1p-65L == 1.0L);");
    agree("return (int)(1.0L - 0x1.0000000000000002p-65L == 0x1.fffffffffffffffep-1L);");
}

/// **Multiplying two `long double`s agrees with gcc.**
///
/// The first arithmetic operation, and the one that needs no iteration: two sixty-four-bit
/// significands make a *one-hundred-and-twenty-eight*-bit product, which is exactly a `u128`. Add the
/// exponents, normalize by one bit, round the low half to nearest-even. Division needs long division
/// and addition needs alignment with a sticky bit; multiplication needs neither, which is why it goes
/// first.
///
/// **The rounding fixtures are the point, and picking them took mutation rather than thought.** The
/// obvious one — `(1 + 2^-63)²`, whose exact value `1 + 2^-62 + 2^-126` does not fit — turns out to
/// discard a low half of *two*, which is so far below the halfway point that truncation and
/// round-to-nearest agree. It kills a mutant that rounds up unconditionally and nothing else. Three
/// more were computed to sit exactly where the decisions are:
///
/// ```text
///   discarded half > ½ ulp    (1+2^-63) × (1.5+2^-63)   rounds up; truncation would not
///   discarded half = ½ ulp    1.5 × (1+3·2^-63)         a tie, to the even candidate
///   round-up carries out      (1+2^-63) × (2-2^-62)     all-ones + 1 → a new power of two
/// ```
///
/// The third is the one worth staring at. Rounding a significand of all ones up carries into bit 64:
/// the value has become a power of two and the integer bit moves, which is a *second* normalization
/// after the one the product's width already forced. It is also barely reachable — it needs the exact
/// product within `2^-63` of a power of two, and the search that found these pairs shows the window
/// admits roughly one partner per operand. Missing it is the classic soft-float defect, and no
/// fixture written by picking round-looking numbers will ever land in that window.
///
/// All of these need all sixty-four bits of significand, which is writable only because wave 236 made
/// hex literals exact.
#[test]
fn multiplying_long_doubles_agrees_with_gcc() {
    agree("long double a = 2.0L, b = 3.0L; return (int)(a * b);");
    agree("long double a = -2.0L, b = 3.0L; return (int)(a * b);");
    agree("long double a = -2.0L, b = -3.0L; return (int)(a * b);");
    agree("long double a = 0.5L, b = 4.0L; return (int)(a * b);");
    // Zero times anything, including the sign of the result.
    agree("long double a = 0.0L, b = 3.0L; return (int)(a * b == 0.0L);");
    // A product needing more than sixty-four bits of significand, so the rounding decides it.
    agree(
        "long double a = 0x1.0000000000000002p0L; long double p = a * a; \
         return (int)(p == 0x1.0000000000000004p0L);",
    );
    // Exact in sixty-four bits, so rounding must *not* move it.
    agree(
        "long double a = 0x1.8p0L, b = 0x1.8p0L; long double p = a * b; \
         return (int)(p == 0x1.2p1L);",
    );
    // A product past `f64`'s range but well inside x87's, which is the range this format exists for.
    agree("long double a = 0x1p1000L, b = 0x1p1000L; return (int)(a * b > 0x1p1999L);");
    // **The discarded half exceeds ½ ulp**, so the result rounds up and truncation is visible.
    agree(
        "long double a = 0x1.0000000000000002p0L, b = 0x1.8000000000000002p0L; \
         return (int)(a * b == 0x1.8000000000000006p0L);",
    );
    // **Exactly ½ ulp**, so the tie goes to the even candidate rather than away from zero.
    agree(
        "long double a = 0x1.8p0L, b = 0x1.0000000000000006p0L; \
         return (int)(a * b == 0x1.8000000000000008p0L);",
    );
    // **Rounding up carries out of the significand.** The exact product is `2 - 2^-125`, which is
    // within half an ulp of two, so the answer is two exactly — and getting there means an all-ones
    // significand becoming a power of two with the exponent stepping up behind it.
    agree(
        "long double a = 0x1.0000000000000002p0L, b = 0x1.fffffffffffffffcp0L; \
         return (int)(a * b == 0x1p1L);",
    );
}

/// **A constant expression is arithmetic too: the usual arithmetic conversions apply to it.**
///
/// `const_eval` is the engine's *third* implementation of C integer arithmetic, after the
/// interpreter and the `#if` evaluator, and it is the one the oracle never watched: it runs at
/// layout time, for array bounds, bit-field widths, enumeration constants and static
/// initializers, so nothing it computes passes through the lowering the corpus compares.
///
/// It holds every value as a mathematical `i128` and computes the *type* of a binary operator
/// correctly — and then compares the raw `i128`s. So `-1 < 1u` asks whether −1 is less than 1
/// rather than whether `4294967295u` is, which is the question C 6.3.1.8 says to ask. Every
/// relational and equality operator is affected, and the answer feeds an array's size.
///
/// The conditional operator has the same defect the `#if` evaluator had in wave 298, found
/// independently in a separate implementation: the result takes the selected arm's type instead
/// of the usual arithmetic conversions of both arms.
///
/// `1 ? 1 : 1/0` is here to pin what the fix must *not* do. gcc accepts it — the arm that is not
/// taken contributes its type, and need not be evaluable at all — so reading the other arm for
/// its type must not turn its diagnostics into the expression's.
#[test]
fn constant_expressions_apply_the_usual_arithmetic_conversions() {
    for (prelude, body) in [
        // Relational and equality operators against an unsigned operand.
        ("enum { E = -1 < 1u };", "return E;"),
        ("enum { E = -1 > 0u };", "return E;"),
        ("enum { E = -1 >= 1u };", "return E;"),
        ("enum { E = -1 == 4294967295u };", "return E;"),
        ("enum { E = -1 != 4294967295u };", "return E;"),
        // The same rule with a wider unsigned type, where the common type is 64 bits.
        ("enum { E = -1 < 1ul };", "return E;"),
        // A wider *signed* type represents every value of a narrower unsigned one, so this stays
        // signed and the comparison is the ordinary one. The control for the case above.
        ("enum { E = (long)-1 < 1u };", "return E;"),
        ("enum { E = -1 < 1 };", "return E;"),
        // The conditional operator takes both arms' types, whichever is selected.
        ("enum { E = (0 ? 1u : -1) < 0 };", "return E;"),
        ("enum { E = (1 ? -1 : 1u) < 0 };", "return E;"),
        ("enum { E = (1 ? -1 : 1) < 0 };", "return E;"),
        // The arm that is not taken need not be evaluable.
        ("enum { E = 1 ? 7 : 1/0 };", "return E;"),
        ("enum { E = 0 ? 1/0 : 7 };", "return E;"),
        // The answer decides a layout, not just a value.
        (
            "int arr[(-1 < 1u) ? 3 : 7];",
            "return (int)(sizeof(arr)/sizeof(arr[0]));",
        ),
        ("static const int g = -1 < 1u;", "return g;"),
    ] {
        agree_with(prelude, body);
    }
}

/// **`sizeof` of an *expression* is a constant expression too.**
///
/// `const_eval` has an arm for `SizeofType` and none for `SizeofExpr`, so `sizeof(int)` folds
/// and `sizeof(1)` does not. C 6.5.3.4 makes both constant expressions whenever the operand is
/// not a variable-length array, and `sizeof buf` in an array bound or an enumerator is ordinary
/// C — `enum { N = sizeof header };` is how a great deal of real code states a buffer's size.
///
/// The operand is *not evaluated*: `sizeof` asks about a type, so it needs the operand's type
/// and nothing else. That is why the cases below include one with a side effect that must not
/// happen and one whose value could not be computed at all.
#[test]
fn sizeof_of_an_expression_folds_in_a_constant_expression() {
    for (prelude, body) in [
        ("enum { E = (int)sizeof(1) };", "return E;"),
        ("enum { E = (int)sizeof 1 };", "return E;"),
        ("enum { E = (int)sizeof(1L) };", "return E;"),
        ("enum { E = (int)sizeof('a') };", "return E;"),
        ("enum { E = (int)sizeof(1 + 1L) };", "return E;"),
        ("enum { E = (int)sizeof((char)1) };", "return E;"),
        // The operand is unevaluated, so a division by zero inside it is not a division at all.
        ("enum { E = (int)sizeof(1 / 0) };", "return E;"),
        // An array bound is the everyday use, and it decides a layout rather than a value.
        (
            "static long buf[7]; int arr[sizeof(buf) / sizeof(buf[0])];",
            "return (int)(sizeof(arr) / sizeof(arr[0]));",
        ),
        (
            "struct S { int a; char b; }; struct S s; int arr[sizeof s];",
            "return (int)(sizeof(arr) / sizeof(arr[0]));",
        ),
        // `_Alignof` of a type already worked; this is its neighbour in the same match.
        ("enum { E = (int)_Alignof(long) };", "return E;"),
    ] {
        agree_with(prelude, body);
    }
}

/// **`sizeof` of a *local*, in the places that need it to be a constant.**
///
/// The fixture above uses file-scope operands, so it says nothing about block scope — and block
/// scope is where `int arr[sizeof(x)]` and a function-local `enum` actually appear.
///
/// This test was written to distinguish the *mechanism* the fix chose, reusing the typing the
/// main pass recorded rather than typing the operand again in `const_eval`'s throwaway context.
/// **It does not distinguish it**: forcing the reuse off leaves the whole suite green, so the
/// choice saves duplicate work rather than a wrong answer, and the source comment now says so.
/// What these cases *do* pin is worth keeping on its own — that a `sizeof` naming a local folds
/// at all, in an array bound and in an enumerator — so the test stays with an honest name.
#[test]
fn sizeof_of_a_local_folds_in_an_array_bound_and_an_enumerator() {
    for (prelude, body) in [
        ("", "int x; int arr[sizeof(x)]; return (int)sizeof(arr);"),
        ("", "long y; enum { E = (int)sizeof(y) }; return E;"),
        ("", "int x; return (int)sizeof(x + 1L);"),
        ("", "char c; return (int)sizeof(c + 1);"),
        (
            "struct S { int a; };",
            "struct S s; int arr[sizeof s]; return (int)sizeof(arr);",
        ),
        (
            "",
            "int x; int arr[_Alignof(x) + 1]; return (int)sizeof(arr);",
        ),
    ] {
        agree_with(prelude, body);
    }
}

/// **A tag referenced before its definition is the same tag.**
///
/// `tag()` interns a reference to a not-yet-defined tag as `Ty::Error` and does not record the
/// name, so the reference is frozen: a definition later in the file cannot reach back and
/// complete it. The commonest victim is a struct that mentions *itself* — inside
/// `struct Node { struct Node *next; }` the tag is not yet in the table, so `next` is a pointer
/// to `Ty::Error` for the rest of the program, and a linked list stops working at the first
/// hop. `a.next->v` produced no answer at all.
///
/// The pointer-subtraction case is a regression from wave 303 and is the reason this is a fix
/// rather than a feature. That wave added "arithmetic on a pointer to an incomplete type", which
/// is a correct rule applied to an incorrect fact: `a.next` points to a *complete* type, and only
/// the representation says otherwise. A check is only ever as right as what it asks.
///
/// The forward-declaration cases are the same fact from the other side — `struct A;` used through
/// a pointer and defined afterwards, and two structs that refer to each other.
#[test]
fn a_tag_used_before_its_definition_is_completed_by_it() {
    const LIST: &str = "struct Node { int v; struct Node *next; }; static struct Node b = {2,0}; \
         static struct Node a = {1,&b};";
    for (prelude, body) in [
        // A struct that mentions itself: the member must point at the completed type.
        (LIST, "return a.next->v;"),
        (LIST, "struct Node *p = &a; return p->next->v;"),
        (LIST, "return (int)sizeof(*a.next);"),
        // Wave 303's check must not fire here: the pointee is complete.
        (LIST, "return (int)(a.next - &b);"),
        (LIST, "return (int)(a.next + 1 - a.next);"),
        // Declared first, used through a pointer, defined afterwards.
        (
            "struct I; struct I *gp; struct I { int z; }; static struct I gi = {4};",
            "gp = &gi; return gp->z;",
        ),
        // Two structs that refer to each other.
        (
            "struct A; struct B { struct A *pa; int v; }; struct A { int w; }; \
             static struct A ga = {5}; static struct B gb = {&ga, 9};",
            "return gb.pa->w;",
        ),
    ] {
        agree_with(prelude, body);
    }
}

/// **The canonical uses of C, each written the way a textbook writes it.**
///
/// Wave 304 found that `struct Node { struct Node *next; }` had never worked — a linked list,
/// the first data structure in every C book, produced no answer at all and survived 1470 tests,
/// a differential corpus, a generated channel and a VPP header gate. It survived because every
/// fixture that needed a struct wrote a *flat* one: the suite tested the engine's *rules*
/// thoroughly and its *uses* not at all.
///
/// This is the net for that. Each case is a shape a C programmer writes without thinking, and
/// the sweep that produced it found exactly one failure — the one wave 304 had just fixed. That
/// is a result worth keeping rather than repeating: the value here is not in what it caught but
/// in what it will catch when a representation changes underneath it again.
///
/// `__builtin_va_list` rather than `<stdarg.h>`: this harness has no include loader, and a
/// fixture that fails to preprocess would report a defect that is not there. It did, in the
/// sweep, until the include was removed.
#[test]
fn the_canonical_uses_of_c_agree_with_gcc() {
    for (prelude, body) in [
        // Self-referential and mutually-linked structures.
        (
            "struct T { int v; struct T *l, *r; }; static struct T c={3,0,0}, b={2,0,0}, a={1,&b,&c};",
            "return a.l->v + a.r->v;",
        ),
        (
            "struct N { int v; struct N *next; }; static struct N c={3,0}, b={2,&c}, a={1,&b};",
            "int s=0; for (struct N *p=&a; p; p=p->next) s+=p->v; return s;",
        ),
        // Function pointers: a dispatch table and a typedef'd one.
        (
            "static int add1(int x){return x+1;} static int dbl(int x){return x*2;} \
          static int (*tab[2])(int) = {add1, dbl};",
            "return tab[0](3) + tab[1](5);",
        ),
        (
            "typedef int (*op)(int,int); static int mul(int a,int b){return a*b;}",
            "op f = mul; return f(6,7);",
        ),
        (
            "struct P { int k; }; static int cmp(const void *a, const void *b){ \
            return ((const struct P*)a)->k - ((const struct P*)b)->k; } \
          static struct P arr[3] = {{3},{1},{2}};",
            "return cmp(&arr[0], &arr[1]);",
        ),
        // Aggregates by value, by array, and nested.
        (
            "struct P { int x, y; }; static struct P ps[3] = {{1,2},{3,4},{5,6}};",
            "int s=0; for (int i=0;i<3;i++) s += ps[i].x*ps[i].y; return s;",
        ),
        (
            "struct P { int x, y; }; static int sum(struct P p){ return p.x+p.y; } \
          static struct P g = {3,4};",
            "return sum(g);",
        ),
        (
            "struct P { int x, y; }; static struct P mk(int a){ struct P p; p.x=a; p.y=a*2; return p; }",
            "struct P q = mk(5); return q.x + q.y;",
        ),
        (
            "struct In { int a; }; struct Out { struct In i; int b; }; static struct Out o = {{7},9};",
            "return o.i.a + o.b;",
        ),
        (
            "union U { int i; unsigned char b[4]; }; static union U u = {0x01020304};",
            "return u.b[0] + u.b[3];",
        ),
        (
            "struct B { unsigned a:3, b:5; }; static struct B bb;",
            "bb.a=5; bb.b=17; return bb.a*100+bb.b;",
        ),
        // Arrays: two-dimensional, and a pointer to one.
        (
            "static int m[3][3] = {{1,2,3},{4,5,6},{7,8,9}};",
            "int s=0; for(int i=0;i<3;i++) s+=m[i][i]; return s;",
        ),
        (
            "static int a[4] = {1,2,3,4};",
            "int (*p)[4] = &a; return (*p)[2];",
        ),
        (
            "static const char *s = \"hello\";",
            "int n=0; while (s[n]) n++; return n;",
        ),
        // Control flow and storage.
        (
            "static int fact(int n){ return n<=1 ? 1 : n*fact(n-1); }",
            "return fact(5);",
        ),
        (
            "static int od(int); static int ev(int n){ return n==0?1:od(n-1);} \
          static int od(int n){ return n==0?0:ev(n-1);}",
            "return ev(10);",
        ),
        ("", "static int c = 0; c++; return c;"),
        (
            "static int vsum(int n, ...){ __builtin_va_list ap; __builtin_va_start(ap,n); \
            int s=0; for(int i=0;i<n;i++) s+=__builtin_va_arg(ap,int); __builtin_va_end(ap); \
            return s; }",
            "return vsum(3,1,2,3);",
        ),
    ] {
        agree_with(prelude, body);
    }
}

/// **The fourth implementation: the same C operation computed concretely and symbolically.**
///
/// §9's technique — find the *second* implementation of a rule — applied to the pair at the
/// centre of the engine. `chiero-exec` evaluates an operation on concrete bits with native
/// arithmetic; when an operand is symbolic it instead builds a `chiero-solver` term. Those are
/// two implementations of C's semantics, and nothing compared them: the corpus runs everything
/// concretely, and the symbolic tests assert *reachability* rather than *values*.
///
/// The comparison pins a symbolic input back to a known value and asks the solver to prove the
/// expression equals what gcc computed for it. The three outcomes are distinguishable, which is
/// the point of the shape:
///
///   - `[1]` — the solver **proved** the identity: the two implementations agree.
///   - `[2]` — the symbolic result is provably *different*: a soundness defect.
///   - `[1, 2]` — the branch was undecidable, so the engine took both edges. That is solver
///     incompleteness, not a wrong answer, and it is counted separately so a rise in it cannot
///     be mistaken for agreement.
///
/// Both zero today, across 390 pairs. The assertion on `undecided` is therefore a coverage
/// guard: a case that stops being provable stops testing anything, and would otherwise go quiet.
#[test]
fn symbolic_arithmetic_agrees_with_concrete_arithmetic() {
    const DECL: &str = "void chiero_make_symbolic(void *, unsigned long, const char *); \
                        void chiero_assume(int);";
    let states = |body: &str| -> Vec<u128> {
        let src = format!("{DECL}\nint probe(void) {{ {body} }}");
        let m = harness::lower(&src);
        let mut a = TermArena::new();
        let r = chiero_exec::Engine::new(&m).with_entry("probe").run(&mut a);
        let mut v: Vec<u128> = r
            .states()
            .iter()
            .filter_map(|s| s.return_value_bits(&mut a))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };

    // Chosen where the two implementations most plausibly diverge: signed division and modulo
    // truncating toward zero with a negative operand, arithmetic versus logical shift, narrowing
    // casts and sign extension, and unsigned wraparound. Nothing here is undefined behaviour —
    // `x / -1` at `INT_MIN` and signed overflow are excluded, because gcc's answer for those is
    // not a specification of anything.
    let exprs = [
        "x + 7",
        "x - 7",
        "x * 3",
        "x / 5",
        "x % 5",
        "x / -5",
        "x % -5",
        "-x",
        "x >> 3",
        "x >> 31",
        "(unsigned)x >> 3",
        "(unsigned)x >> 31",
        "x << 1",
        "x & 0x0f0f",
        "x | 0x1234",
        "x ^ -1",
        "~x",
        "(short)x",
        "(char)x",
        "(unsigned char)x",
        "(unsigned short)x",
        "(int)(unsigned)x",
        "(int)((unsigned)x + 1u)",
        "(int)((unsigned)x * 3u)",
        "(int)((unsigned)x - 1u)",
        "(long)x * 3",
        "(int)((long)x >> 1)",
        "x < 0",
        "x <= 0",
        "x == 0",
        "x != 0",
        "(unsigned)x < 10u",
        "(unsigned)x > 0u",
        "x < 0 ? -x : x",
        "x / 7 * 7 + x % 7",
        "(x >> 31) & 1",
        "x > 0 && x < 100",
        "x < 0 || x > 50",
        "!x",
    ];

    let (mut proved, mut wrong, mut undecided) = (0, 0, Vec::new());
    for v in [
        "42",
        "-42",
        "0",
        "1",
        "-1",
        "2147483647",
        "-2147483647",
        "255",
        "-255",
        "65536",
    ] {
        for e in exprs {
            let want = match gcc_answer("", &format!("int x = {v}; return (int)({e});")) {
                Ok(w) => w,
                Err(Oracle::NoGcc) => return,
                Err(Oracle::Broken(why)) => panic!("the oracle is broken, not absent: {why}"),
            };
            let body = format!(
                "int x = {v}; chiero_make_symbolic(&x, 4, \"x\"); chiero_assume(x == {v}); \
                 if ((int)({e}) == {want}) return 1; return 2;"
            );
            match states(&body).as_slice() {
                [1] => proved += 1,
                [2] => {
                    wrong += 1;
                    eprintln!("symbolic disagrees: x={v}, `{e}`, gcc says {want}");
                }
                other => undecided.push(format!("x={v} `{e}` -> {other:?}")),
            }
        }
    }
    assert_eq!(
        wrong, 0,
        "the symbolic path computed a different value than the concrete one"
    );
    assert!(
        undecided.is_empty(),
        "these pairs stopped being provable, so they stopped testing anything: {undecided:?}"
    );
    assert!(proved > 300, "only {proved} pairs proved; the sweep shrank");
}

/// **`a[b]` is `*(a + b)`, and `(void)x` discards a value.** Two shapes found by the
/// discriminators of wave 308's type-error fixture rather than by aiming at them.
///
/// The commutative subscript came from a case written only to stop the new "subscripted value is
/// not an array or pointer" check from being too broad: `0[p]` is legal C, so the check had to
/// look at both operands. Sema then typed it correctly and it still produced no answer, because
/// *lowering* assumed the base was the aggregate at all three of its `Index` sites.
///
/// The cast to void came from a throwaway `(void)p;` written to silence an unused variable in a
/// probe. `(void)0; return 1;` returned nothing: `cast_kind` has no conversion *to* void, so the
/// module it built was rejected and the function produced no state. It is hard to overstate how
/// ordinary that idiom is.
///
/// Neither was reachable from the census that started the wave. Both were reachable from writing
/// down what must keep working.
#[test]
fn a_subscript_reads_either_operand_and_a_void_cast_discards() {
    for (prelude, body) in [
        // The pointer may be written on either side, for every element type.
        ("static long a[4] = {10,20,30,40};", "return (int)1[a];"),
        ("static long a[4] = {10,20,30,40};", "return (int)a[1];"),
        (
            "static double d[3] = {1.5,2.5,3.5};",
            "return (int)(2[d] * 2);",
        ),
        (
            "struct P { int x, y; }; static struct P ps[2] = {{1,2},{3,4}};",
            "return 1[ps].y;",
        ),
        (
            "",
            "int a[4]; a[0]=10;a[1]=20;a[2]=30;a[3]=40; return 2[a];",
        ),
        (
            "static int a[4] = {10,20,30,40};",
            "int *p = a; return 3[p];",
        ),
        // A cast to void evaluates its operand and yields nothing.
        ("", "(void)0; return 1;"),
        ("", "int x = 0; (void)x; return 1;"),
        ("", "long *p = 0; (void)p; return p == 0;"),
        // The side effects still happen: the operand is evaluated, only the value is dropped.
        ("", "int x = 0; (void)(x = 7); return x;"),
        (
            "static int c = 0; static int bump(void){ return ++c; }",
            "(void)bump(); (void)bump(); return c;",
        ),
    ] {
        agree_with(prelude, body);
    }
}

/// **`__func__` is declared by the language, not by the program.**
///
/// C99 6.4.2.2: the compiler behaves as if `static const char __func__[] = "name";` appeared at
/// the top of every function body. Nothing declares it, so this engine reported it undeclared and
/// then produced no state for any use of it — every case below returned nothing at all.
///
/// Found by installing the gate wave 307 asked for: sema's diagnostics over the VPP corpus. It
/// was the *only* complaint across all six headers, which is what made it obviously a false
/// positive rather than a finding about VPP.
///
/// The `sizeof` case is the one that makes this a type and not a string: `__func__` is an *array*
/// of the right length, so `sizeof(__func__)` is the name's length plus one, and treating it as a
/// `const char *` would give 8 on this target.
#[test]
fn func_is_predefined_in_every_function_body() {
    for (prelude, body) in [
        ("", "return (int)__func__[0];"),
        ("", "return (int)sizeof(__func__);"),
        ("", "const char *n = __func__; return (int)n[1];"),
        (
            "static int len(const char *s){ int n=0; while(s[n]) n++; return n; }",
            "return len(__func__);",
        ),
        ("", "return __func__[0] == 'p';"),
        // Each function gets its own, naming itself.
        (
            "static int who(void){ return (int)sizeof(__func__); }",
            "return who() * 100 + (int)sizeof(__func__);",
        ),
    ] {
        agree_with(prelude, body);
    }

    // **A declared `__func__` wins, and gcc cannot arbitrate this one.** gcc reserves the
    // spelling in its *parser* and rejects `int __func__ = 7;` outright, so there is no oracle
    // answer to compare against; chiero's parser accepts it as an ordinary identifier. The
    // predefined object is therefore resolved at the *end* of the lookup chain on both sides —
    // after locals and globals — and this is what says so. Without it the ordering is a claim in
    // a comment: forcing lowering to answer `__func__` regardless of what is in scope passes
    // every case above.
    //
    // `probe` is five characters, so the predefined array is six bytes and an `int` local is
    // four. The two answers cannot be confused.
    assert_eq!(
        chiero_answer("", "int __func__ = 7; return (int)sizeof(__func__);"),
        Some(4),
        "a declared `__func__` is an ordinary object and keeps its own type"
    );
    assert_eq!(
        chiero_answer("", "int __func__ = 7; return __func__;"),
        Some(7),
        "and its own value"
    );
}

/// **An array whose length comes from its initializer.**
///
/// C 6.7.9p22: `int a[] = {1,2,3}` is an array of three, and `char s[] = "hi"` is an array of
/// three including the terminator. Sema turns an unspecified length into `ArrayLen::Flexible` and
/// never completes it from the initializer, so **every one of these has size zero**: `sizeof` is
/// 0 where gcc says 12 or 3, and reading any element degrades the run to `Unknown` rather than
/// returning a value.
///
/// Found by asking §9's question — what can the differential channels not see — and writing a
/// second tier of canonical shapes for the answer. `static char buf[] = "…"` appeared in a
/// *helper* line of one of them, not as the thing under test, which is why eighteen canonical
/// programs in wave 305 and a hundred-odd corpus fixtures had never touched it: every fixture
/// that needed an array wrote its length, because a fixture author picks the length to make the
/// test's arithmetic obvious.
///
/// The explicit-length cases are here to show the fault is the *inference* and not arrays: they
/// worked before this and must keep working.
#[test]
fn an_array_takes_its_length_from_its_initializer() {
    for (prelude, body) in [
        // Braced initializers, at file scope and in a block.
        ("static int a[] = {1,2,3};", "return a[2];"),
        ("static int a[] = {1,2,3};", "return (int)sizeof(a);"),
        ("", "int a[] = {1,2,3}; return a[2];"),
        ("", "int a[] = {1,2,3}; return (int)sizeof(a);"),
        // String initializers, whose length includes the terminator.
        ("static char s[] = \"hi\";", "return s[0];"),
        ("static char s[] = \"hi\";", "return (int)sizeof(s);"),
        ("", "char s[] = \"hello\"; return (int)sizeof(s);"),
        ("", "char s[] = \"hello\"; return s[4];"),
        // The inner dimension is written, the outer inferred.
        // **A designator sets the length**, so this is five long with one item — the count and
        // the highest position are different numbers, and only the second is the answer.
        (
            "static int a[] = {[4] = 7};",
            "return (int)(sizeof(a)/sizeof(a[0]));",
        ),
        ("static int a[] = {[4] = 7};", "return a[4];"),
        ("static int a[][2] = {{1,2},{3,4}};", "return a[1][1];"),
        (
            "static int a[][2] = {{1,2},{3,4}};",
            "return (int)(sizeof(a)/sizeof(a[0]));",
        ),
        // A loop over an inferred array, which is how such an array is normally used.
        (
            "static char s[] = \"hello\";",
            "int n=0; while (s[n]) n++; return n;",
        ),
        (
            "static int a[] = {5,7,9};",
            "int t=0; for (int i=0;i<3;i++) t+=a[i]; return t;",
        ),
        // **Explicit lengths, which already worked** — the fault is the inference, not arrays.
        // **A written length is never overridden**, even when the initializer is shorter:
        // `int a[4] = {1,2}` is four long, and inferring from the list would make it two.
        ("static int a[4] = {1,2};", "return (int)sizeof(a);"),
        ("static char s[8] = \"hi\";", "return (int)sizeof(s);"),
        ("static int a[3] = {1,2,3};", "return a[2];"),
        ("static char s[4] = \"hi\";", "return s[0];"),
        ("static int a[2][2] = {{1,2},{3,4}};", "return a[1][1];"),
    ] {
        agree_with(prelude, body);
    }
}

/// **A `static` local is one object for the whole program, initialized once.**
///
/// C 6.2.4p3: it has static storage duration, and its initializer runs before `main` rather than
/// each time control reaches the declaration. This engine gave it automatic storage, so it was
/// reinitialized on every entry — a counter in a loop stayed at 1, a counter in a function called
/// twice stayed at 1, and one declared without an initializer produced no answer at all.
///
/// Found by wave 321's method, which is the point: `static int c = 0;` *at the top of a function
/// that runs once* was already in the canonical net and passed, because a variable that is
/// initialized once and read once behaves identically whichever storage it has. Only re-entry
/// tells them apart, and re-entry is what a fixture avoids — a test that runs its subject twice
/// has to explain why.
///
/// The three shapes below are the three ways to re-enter: another iteration, another call, and
/// another pass through an inner block. The non-`static` case beside them is the control that
/// keeps the fix from applying to every local.
#[test]
fn a_static_local_persists_across_entries() {
    for (prelude, body) in [
        // Re-entry by iteration.
        (
            "",
            "int t=0; for(int i=0;i<3;i++){ static int c=0; c++; t=c; } return t;",
        ),
        // **Without an initializer it is zero-initialized**, like any static object — this one
        // produced no answer at all, not merely a wrong one.
        (
            "",
            "int t=0; for(int i=0;i<3;i++){ static int c; c++; t=c; } return t;",
        ),
        // Re-entry by call.
        (
            "static int bump(void){ static int c = 0; c++; return c; }",
            "bump(); return bump();",
        ),
        (
            "static int bump(void){ static int c = 10; c++; return c; }",
            "bump(); bump(); return bump();",
        ),
        // Re-entry into an inner block.
        (
            "",
            "int t=0; for(int i=0;i<2;i++){ { static int c=100; c++; t=c; } } return t;",
        ),
        // An array with static duration keeps what was written to it.
        (
            "",
            "int t=0; for(int i=0;i<3;i++){ static int a[2]; a[i%2]=i; t=a[0]; } return t;",
        ),
        // **`extern` inside a body names an object defined elsewhere**, and must not create a
        // new one. It reaches the same branch as `static` and has to be excluded from it.
        (
            "static int v = 5; static int g(void){ extern int v; return v; }",
            "return g();",
        ),
        // **The binding is scoped to the function that declared it.** A `static int c` inside
        // `f1` must not still be what `c` means afterwards — which is why the displaced
        // file-scope binding is remembered and put back.
        (
            "static int c = 9; static int f1(void){ static int c = 1; return c; }",
            "f1(); return c;",
        ),
        (
            "static int c = 9; static int f1(void){ static int c = 1; return c; }",
            "return f1() * 10 + c;",
        ),
        // **The controls.** An automatic local *is* reinitialized each time, which is the
        // difference the fix must preserve rather than erase.
        (
            "",
            "int t=0; for(int i=0;i<3;i++){ int c=0; c++; t=c; } return t;",
        ),
        ("", "static int c = 0; c++; return c;"),
        (
            "static int v=7;",
            "int t=0; for(int i=0;i<2;i++){ static int *p = &v; t = *p; } return t;",
        ),
    ] {
        agree_with(prelude, body);
    }
}

/// **Constructs used as scenery, and behaviour that differs on the second visit.**
///
/// The two selection rules waves 321 and 322 earned, applied as a net. Each of those waves found
/// a defect in its first twenty programs — a length-zero array, then `static` locals with
/// automatic storage — and **these thirty-eight found none.** That result is the point of
/// recording them: the seam is thinner than it was two waves ago.
///
/// The rules, restated so a later tier is chosen the same way:
///
///   - **Scenery.** A construct that appears in fixtures to *set up* a test rather than as its
///     subject is tested by nobody. `static char buf[] = "…"` was the source of a copy loop in a
///     dozen probes before anyone read its `sizeof`.
///   - **The second visit.** A test that runs its subject twice has to explain why, so it does not
///     get written — and static storage, address identity and accumulated state are all invisible
///     on the first entry.
///
/// Every case here does one or both: a static aggregate mutated across calls, a function's
/// address compared across calls, the address of a static local surviving its return, a bit-field
/// accumulating, an `alloca` reused across iterations, a static array in a recursive function.
///
/// **Kept for what it will catch, not for what it caught**, and measured rather than assumed:
/// re-injecting wave 321's length-zero array and wave 322's frame-slot `static` locals both make
/// this fail. Two other recent defects it *cannot* see, and the reasons are worth knowing —
/// wave 322's name-restore rule needs a file-scope object shadowed and then read, which its own
/// fixture covers, and wave 320's deref-completeness rule is a **diagnostic** on a program gcc
/// rejects, which a net comparing values on programs gcc accepts can never reach.
#[test]
fn scenery_and_second_visit_shapes_agree_with_gcc() {
    let cases: [(&str, &str, &str); 38] = [
        (
            "static agg across calls",
            "static int bump(void){ static int a[2] = {1,2}; a[0]++; return a[0]; }",
            "bump(); return bump();",
        ),
        (
            "static struct across calls",
            "struct S { int v; }; static int bump(void){ static struct S s = {5}; s.v++; return s.v; }",
            "bump(); return bump();",
        ),
        (
            "global mutated by callee",
            "static int g = 1; static void set(void){ g = 7; }",
            "set(); return g;",
        ),
        (
            "fn address stable",
            "static int f1(void){ return 1; }",
            "int (*a)(void) = f1; int (*b)(void) = f1; return a == b;",
        ),
        (
            "array identity across calls",
            "static int a[2]; static int *get(void){ return a; }",
            "return get() == get();",
        ),
        (
            "identical string literals",
            "",
            "const char *a = \"xy\"; const char *b = \"xy\"; return a[0]==b[0];",
        ),
        (
            "const global via cast",
            "static const int c = 3;",
            "int *p = (int *)&c; return *p;",
        ),
        (
            "goto back over decl",
            "",
            "int n=0; again: { int x = 5; n += x; } if (n < 10) goto again; return n;",
        ),
        (
            "recursion depth 6",
            "static int fac(int n){ return n<=1?1:n*fac(n-1); }",
            "return fac(6);",
        ),
        (
            "param reused two calls",
            "static int sq(int n){ return n*n; }",
            "return sq(3) + sq(4);",
        ),
        (
            "struct by value twice",
            "struct P { int x; }; static struct P mk(int v){ struct P p={v}; return p; }",
            "struct P a=mk(1), b=mk(2); return a.x*10+b.x;",
        ),
        (
            "alloca reuse in loop",
            "",
            "int t=0; for(int i=0;i<3;i++){ int a[2]; a[0]=i; t+=a[0]; } return t;",
        ),
        (
            "compound lit in loop",
            "struct P { int x; }; static int gx(struct P p){ return p.x; }",
            "int t=0; for(int i=0;i<3;i++) t += gx((struct P){i}); return t;",
        ),
        (
            "static ptr chain",
            "static int v=4; static int *p=&v; static int **pp=&p;",
            "**pp = 9; return v;",
        ),
        (
            "nested call mutates",
            "static int g=0; static void a(void){ g++; } static void b(void){ a(); a(); }",
            "b(); return g;",
        ),
        (
            "write then read global arr",
            "static int a[3];",
            "for(int i=0;i<3;i++) a[i]=i+1; int s=0; for(int i=0;i<3;i++) s+=a[i]; return s;",
        ),
        (
            "static in recursive fn",
            "static int depth(int n){ static int max=0; if(n>max) max=n; return n?depth(n-1):max; }",
            "return depth(3);",
        ),
        (
            "two statics same fn",
            "static int f2(void){ static int a=1, b=10; a++; b++; return a*100+b; }",
            "f2(); return f2();",
        ),
        (
            "global init order",
            "static int a = 1; static int *pa = &a; static int b = 2;",
            "return *pa + b;",
        ),
        (
            "string literal write-adj",
            "",
            "char b[4] = \"abc\"; b[0]='z'; return b[0];",
        ),
        (
            "addr of static local",
            "static int *get(void){ static int c = 7; return &c; }",
            "int *p = get(); *p = 9; return *get();",
        ),
        (
            "static local array str",
            "static char *name(void){ static char n[] = \"hi\"; return n; }",
            "return name()[1];",
        ),
        (
            "static local arr persists",
            "static int step(void){ static int a[3]; static int i=0; a[i]=i; i++; return a[0]+i; }",
            "step(); return step();",
        ),
        (
            "bitfield across calls",
            "struct B { unsigned a:3, b:5; }; static int bump(void){ static struct B x; x.a++; return x.a; }",
            "bump(); return bump();",
        ),
        (
            "union across calls",
            "union U { int i; char c; }; static int bump(void){ static union U u = {0}; u.i++; return u.i; }",
            "bump(); return bump();",
        ),
        (
            "struct arr in loop",
            "struct P { int x; }; static struct P a[3];",
            "for(int i=0;i<3;i++) a[i].x=i+1; int s=0; for(int i=0;i<3;i++) s+=a[i].x; return s;",
        ),
        (
            "self assignment",
            "struct P { int x, y; }; static struct P s = {1,2};",
            "s = s; return s.x*10+s.y;",
        ),
        (
            "overlapping copy",
            "struct P { int a[3]; }; static struct P p = {{1,2,3}}; static struct P q;",
            "q = p; return q.a[2];",
        ),
        (
            "fnptr table in loop",
            "static int i1(int x){return x+1;} static int i2(int x){return x*2;} static int (*t[2])(int)={i1,i2};",
            "int v=3; for(int i=0;i<2;i++) v=t[i](v); return v;",
        ),
        (
            "varargs twice",
            "static int s(int n, ...){ __builtin_va_list ap; __builtin_va_start(ap,n); int r=0; for(int i=0;i<n;i++) r+=__builtin_va_arg(ap,int); __builtin_va_end(ap); return r; }",
            "return s(2,1,2) + s(3,1,2,3);",
        ),
        (
            "nested struct copy",
            "struct A{int v;}; struct B{struct A a; int w;}; static struct B x={{1},2}; static struct B y;",
            "y = x; return y.a.v*10+y.w;",
        ),
        (
            "static const arr loop",
            "static const int t[4]={1,2,3,4};",
            "int s=0; for(int i=0;i<4;i++) s+=t[i]; return s;",
        ),
        (
            "ptr into struct arith",
            "struct P{int a,b,c;}; static struct P p={1,2,3};",
            "int *q=&p.a; return q[2];",
        ),
        (
            "addr of static in two calls",
            "static int *get(void){ static int c; return &c; }",
            "return get() == get();",
        ),
        (
            "global addr chain write",
            "static int v=1; static int *p=&v;",
            "*p = 5; return v;",
        ),
        (
            "static local in two fns",
            "static int f1(void){ static int c=1; c++; return c; } static int f2(void){ static int c=10; c++; return c; }",
            "f1(); f2(); return f1()*100 + f2();",
        ),
        (
            "array param decay twice",
            "static int sum(int *a, int n){ int s=0; for(int i=0;i<n;i++) s+=a[i]; return s; } static int m[3]={1,2,3};",
            "return sum(m,3) + sum(m,2);",
        ),
        (
            "recursive static array",
            "static int f3(int n){ static int seen[4]; seen[n]=n; return n ? f3(n-1) : seen[3]; }",
            "return f3(3);",
        ),
    ];
    for (name, prelude, body) in cases {
        let theirs = match gcc_answer(prelude, body) {
            Ok(v) => v,
            Err(Oracle::NoGcc) => return,
            Err(Oracle::Broken(why)) => panic!("the oracle is broken, not absent: {why}"),
        };
        assert_eq!(
            chiero_answer(prelude, body),
            Some(theirs),
            "`{name}`: `{prelude}` / `{body}`"
        );
    }
}
