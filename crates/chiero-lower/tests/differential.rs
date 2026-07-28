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
