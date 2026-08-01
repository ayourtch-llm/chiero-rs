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
    /// The measured count at wave 325. **Raise this when a rule is added; never lower it.**
    ///
    /// The nine below the line at that measurement, which is the queue this channel exists to
    /// keep visible: discarding `const` (needs qualified types — 436 `Ty::` sites, see §9),
    /// assigning to an array, calling with too few or too many arguments, returning a value from
    /// a `void` function, a duplicate struct member, a duplicate parameter name, a `goto` into a
    /// VLA's scope, and taking the address of a `register` object.
    const FLOOR: usize = 54;

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
