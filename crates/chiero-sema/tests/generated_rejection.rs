//! **The mirror of `generated_silence.rs`: what sema *fails* to reject** (014 §7).
//!
//! That channel asserts a program gcc accepts is silent, which catches false positives. This one
//! asks the opposite question — of the programs gcc *rejects*, how many does sema reject too —
//! and it cannot be a pass/fail gate, because plenty of C's constraints are genuinely unchecked
//! and pretending otherwise would mean either a failing suite or a dishonest assertion.
//!
//! So it is a **ratchet**. The number caught is measured and compared against a floor recorded
//! here; the test fails if coverage *drops*. Raising the floor is a deliberate edit that a wave
//! makes when it adds a rule, which turns "we check more of C than we used to" from a feeling
//! into a number in a diff.
//!
//! **Every violation is one gcc confirms.** A generator that believes a program illegal is worth
//! nothing on its own — four censuses in this project turned on cases where my reading of C and
//! gcc's disagreed — so a program gcc *accepts* is dropped from the denominator rather than
//! counted as a miss. `-pedantic-errors`, because wave 314 established that half of C's
//! constraint violations are warnings by default.

mod harness;

use chiero_sema::TargetConfig;

/// One violation: a name for the report, and a program that commits it.
///
/// **Named, because the count alone is not actionable.** A ratchet that says "37 of 52" tells the
/// next wave nothing about what to fix; the failing list is the work queue, and 023 §9's rule —
/// a report a person cannot act on is not a report — applies to a test's output too.
const VIOLATIONS: &[(&str, &str)] = &[
    // Constraint census rows 1-3 (wave 308).
    (
        "member absent",
        "struct S { int m; }; int f(void){ struct S s; return s.nope; }",
    ),
    (
        "member of non-struct",
        "int f(void){ int x = 5; return x.m; }",
    ),
    (
        "subscript non-array",
        "int f(void){ int x = 5; return x[0]; }",
    ),
    ("call non-function", "int f(void){ int q = 5; return q(); }"),
    // Rows 4-7 (wave 311).
    (
        "write const object",
        "int f(void){ const int k = 1; k = 2; return k; }",
    ),
    (
        "increment const object",
        "int f(void){ const int k = 1; k++; return k; }",
    ),
    ("void object", "int f(void){ void w; return 0; }"),
    (
        "void value used",
        "void v(void); int f(void){ return v(); }",
    ),
    (
        "incomplete parameter",
        "struct T; int s(struct T t){ return 0; }",
    ),
    // Rows 8-12 (wave 312).
    (
        "duplicate case",
        "int f(int n){ switch(n){ case 1: return 1; case 1: return 2; } return 0; }",
    ),
    (
        "duplicate default",
        "int f(int n){ switch(n){ default: return 1; default: return 2; } return 0; }",
    ),
    ("break outside loop", "int f(void){ break; return 0; }"),
    (
        "continue outside loop",
        "int f(void){ continue; return 0; }",
    ),
    ("undefined label", "int f(void){ goto nowhere; return 0; }"),
    // Rows 13-16 (wave 313).
    (
        "function redefined",
        "int f(void){ return 0; } int f(void){ return 1; }",
    ),
    ("conflicting types", "int h(int); int h(long){ return 0; }"),
    ("static after non-static", "extern int n; static int n;"),
    ("non-static after static", "static int n; int n;"),
    ("function returns array", "int f(void)[3];"),
    // Initializers (wave 314).
    ("excess array elements", "int a[3] = {1,2,3,4};"),
    (
        "excess struct elements",
        "struct S { int x, y; }; struct S s = {1,2,3};",
    ),
    ("excess scalar elements", "int x = {1,2};"),
    ("string too long", "char s[3] = \"abcd\";"),
    ("designator out of range", "int a[3] = {[5] = 1};"),
    (
        "unknown field designator",
        "struct S { int x, y; }; struct S s = {.nope = 1};",
    ),
    ("non-constant initializer", "int f(void); int g = f();"),
    (
        "excess vector elements",
        "typedef int v4 __attribute__((vector_size(16))); v4 v = {1,2,3,4,5};",
    ),
    ("elided excess", "int a[2][2] = {1,2,3,4,5};"),
    ("read object at file scope", "int x; int g = x;"),
    // Conversions (wave 315).
    ("int to pointer", "int f(void){ int *p = 1; return *p; }"),
    ("pointer to int", "int f(int *p){ int x = p; return x; }"),
    (
        "unrelated pointers assigned",
        "int f(int *p){ char *q = p; return *q; }",
    ),
    (
        "unrelated pointer argument",
        "void g(int *); int f(char *q){ g(q); return 0; }",
    ),
    ("pointer returned as int", "int f(int *p){ return p; }"),
    (
        "unrelated pointers compared",
        "int f(int *p, char *q){ return p == q; }",
    ),
    (
        "struct pointer to int pointer",
        "struct S { int a; }; int f(struct S *s){ int *p = s; return *p; }",
    ),
    (
        "function pointer mismatch",
        "int g(int); int f(void){ int (*fp)(long) = g; return (int)fp(1); }",
    ),
    (
        "discard const",
        "int f(const int *cp){ int *p = cp; return *p; }",
    ),
    // Qualified types (wave 328).
    (
        "discard volatile",
        "int f(volatile int *vp){ int *p = vp; return *p; }",
    ),
    (
        "discard const through void *",
        "int f(const void *cp){ void *p = cp; return p != 0; }",
    ),
    (
        "discard const through &",
        "int f(void){ const int x = 0; int *p = &x; return *p; }",
    ),
    (
        "discard const through an argument",
        "void g(int *); int f(const int *cp){ g(cp); return 0; }",
    ),
    (
        "qualified array typedef, element const",
        "typedef int arr[3]; int f(void){ const arr a = {1,2,3}; int *p = a; return *p; }",
    ),
    (
        "discard const through array decay",
        "int f(void){ const int a[3] = {1,2,3}; int *p = a; return *p; }",
    ),
    (
        "qualifier mismatch below the outermost pointee",
        "int f(int **pp){ const int **cpp = pp; return **cpp; }",
    ),
    (
        "qualifier mismatch below, even adding const",
        "int f(int **pp){ const int *const *cpp = pp; return **cpp; }",
    ),
    (
        "address of a member of a const struct",
        "struct S { int m; }; int f(const struct S *s){ int *p = &s->m; return *p; }",
    ),
    (
        "write a const member",
        "struct S { const int m; }; int f(struct S *s){ s->m = 1; return 0; }",
    ),
    // `switch` (wave 319).
    (
        "switch on double",
        "int f(double d){ switch(d){ case 1: return 1; } return 0; }",
    ),
    (
        "switch on pointer",
        "int f(int *p){ switch(p){ case 0: return 1; } return 0; }",
    ),
    (
        "case not constant",
        "int f(int n, int m){ switch(n){ case m: return 1; } return 0; }",
    ),
    (
        "case not integer",
        "int f(int n){ switch(n){ case 1.5: return 1; } return 0; }",
    ),
    ("case outside switch", "int f(int n){ case 1: return n; }"),
    (
        "default outside switch",
        "int f(int n){ default: return n; }",
    ),
    // Bit-fields (wave 302) and completeness (waves 303, 320).
    (
        "bit-field width non-constant",
        "int nc; struct S { int a; int f : nc; };",
    ),
    (
        "bit-field width negative",
        "struct S { int a; int f : -1; };",
    ),
    (
        "bit-field width too wide",
        "struct S { int a; int f : 33; };",
    ),
    (
        "named zero-width bit-field",
        "struct S { int a; int f : 0; };",
    ),
    ("object of incomplete type", "struct I; struct I x;"),
    ("array of incomplete element", "struct I; struct I arr[10];"),
    (
        "arithmetic on incomplete pointee",
        "struct I; struct I *p; void *q = p + 1;",
    ),
    (
        "sizeof incomplete",
        "struct I; int f(void){ return (int)sizeof(struct I); }",
    ),
    (
        "deref incomplete pointee",
        "struct I; int f(struct I *p){ (*p); return 0; }",
    ),
    (
        "member through incomplete pointee",
        "struct I; int f(struct I *p){ return p->m; }",
    ),
    // Wave 327: the shapes around the VLA rule that must stay rejected, including the one with
    // no block at all — the case that shows it is about crossing a declaration, not entering a
    // block.
    (
        "goto into a VLA scope, no block",
        "int f(int n){ goto skip; int a[n]; skip: return 0; }",
    ),
    // Rules nothing checks yet — the ones this channel exists to keep visible.
    (
        "assignment to array",
        "int f(void){ int a[2], b[2]; a = b; return a[0]; }",
    ),
    (
        "too few arguments",
        "static int g(int a, int b){ return a+b; } int f(void){ return g(1); }",
    ),
    (
        "too many arguments",
        "static int g(int a){ return a; } int f(void){ return g(1,2); }",
    ),
    (
        "return value from void function",
        "static void v(void){ return 1; }",
    ),
    ("duplicate struct member", "struct S { int m; int m; };"),
    (
        "duplicate parameter name",
        "static int g(int a, int a){ return a; }",
    ),
    (
        "goto into a VLA scope",
        "int f(int n){ goto skip; { int a[n]; skip: return 0; } }",
    ),
    (
        "address of a register",
        "int f(void){ register int x = 0; return *&x; }",
    ),
    ("negative array length", "int a[-1];"),
    // Wave 329's census: the C 6.5 operator constraints, closed in the same wave.
    (
        "increment a non-lvalue",
        "int f(void){ int x = 1; return x++++; }",
    ),
    (
        "increment an enumeration constant",
        "int f(void){ enum E { A }; return A++; }",
    ),
    (
        "sizeof a function",
        "void g(void); int f(void){ return sizeof(g); }",
    ),
    (
        "unary minus on a pointer",
        "int f(void){ int x = 1; return -&x != 0; }",
    ),
    (
        "bit-complement on a double",
        "int f(double d){ return (int)~d; }",
    ),
    (
        "void value as an operand",
        "int f(void){ void *p = 0; return *p != 0; }",
    ),
    // ...and its C 6.7 declaration rows, which wave 329 did **not** close. Left here on purpose:
    // this channel exists to keep a known gap visible, and a violation nobody has written a rule
    // for is exactly what it is for. They are the next wave's queue.
    (
        "multiple storage classes",
        "int f(void){ static extern int x; return x; }",
    ),
    (
        "variably modified at file scope",
        "const int k = 1; int a[k];",
    ),
    (
        "duplicate enumerator",
        "enum E { A = 1, A = 2 }; int f(void){ return A; }",
    ),
];

fn gcc_rejects(src: &str) -> Option<bool> {
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
    Some(!child.wait().ok()?.success())
}

/// **How much of C's constraint surface sema rejects, as a number that may not fall.**
///
/// Raising `FLOOR` is the deliberate act a wave performs when it adds a rule. Lowering it is
/// how a regression announces itself, and the failure prints the names of everything missed so
/// the next wave has a queue rather than a percentage.
#[test]
fn the_share_of_violations_sema_rejects_does_not_fall() {
    /// The measured count at wave 329. **Raise this when a rule is added; never lower it.**
    ///
    /// Wave 325 measured 54 and closed three; wave 326 closed four more. **The two still below the
    /// line are the two that need machinery sema does not have**, which is why the queue emptied
    /// down to them rather than stopping anywhere arbitrary:
    ///
    ///   - **discarding `const`** needs *qualified types* — 436 `Ty::` match sites across four
    ///     crates, budgeted in §9 as its own effort;
    ///   - **a `goto` into a VLA's scope** — closed in wave 327, by recording which
    ///     variably-modified scopes are open at each label and at each `goto` and requiring the
    ///     first set to be contained in the second.
    ///
    /// Wave 328 emptied the queue; **wave 329's census refilled it.** Eight new rows, five closed
    /// in the same wave and three left open — `static extern`, a variably-modified array at file
    /// scope, and a duplicate enumerator. Those three are the next wave's work list, and the
    /// failure message prints them by name.
    const FLOOR: usize = 80;

    if gcc_rejects("int main(void){return 0;}") != Some(false) {
        eprintln!("skipping: gcc not usable here");
        return;
    }

    let mut caught = Vec::new();
    let mut missed = Vec::new();
    let mut not_a_violation = Vec::new();
    for (name, src) in VIOLATIONS {
        match gcc_rejects(src) {
            // **gcc disagreeing is a bug in this list, not a finding.** Dropped from the
            // denominator rather than counted as a miss.
            Some(false) | None => {
                not_a_violation.push(*name);
                continue;
            }
            Some(true) => {}
        }
        let p = std::panic::catch_unwind(|| {
            harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux())
                .analysis
                .diagnostics
                .is_empty()
        });
        // A parse rejection is a rejection: the program did not get through.
        match p {
            Ok(true) => missed.push(*name),
            Ok(false) | Err(_) => caught.push(*name),
        }
    }

    assert!(
        not_a_violation.is_empty(),
        "gcc accepts these, so they are bugs in this list rather than missing checks: {not_a_violation:?}"
    );
    eprintln!(
        "sema rejects {} of {} constraint violations; missing: {:?}",
        caught.len(),
        VIOLATIONS.len(),
        missed
    );
    assert!(
        caught.len() >= FLOOR,
        "coverage fell to {} from {FLOOR}; newly missed: {missed:?}",
        caught.len()
    );
}
