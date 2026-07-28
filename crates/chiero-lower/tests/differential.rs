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
