//! **Calling through a null function pointer is a fault, and was not reported.**
//!
//! Dereferencing a null *data* pointer has been a finding for a long time. Calling through a
//! null *function* pointer is the same fault — C11 6.5.2.2p5 requires the operand to point to
//! a function, and the hardware treats it the same way — but produced nothing at all:
//!
//! ```text
//!   int *p = 0;         return *p;     null-dereference: access at offset 0 of NULL
//!   int (*fp)(void)=0;  return fp();   (no finding; the path ends, fidelity degrades)
//! ```
//!
//! A degraded run says "chiero could not follow this", which is 023 §7's honest answer for a
//! *modelling limit*. It is the wrong answer here: the program has a definite fault at a
//! definite place, and the run reports it as an absence of information.
//!
//! Found by wave 192 while measuring what waves 189–191 made reachable. That is the reason
//! it stayed hidden: a table of function pointers read as null before those waves, so
//! `tab[i]()` never got as far as calling anything, and the one shape that reaches this had
//! no way to arise.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

fn reports_null(src: &str) -> bool {
    findings(src).iter().any(|f| f.contains("null"))
}

/// Every route by which a null reaches the callee position.
///
/// Four, because the value arrives differently in each and a fix keyed on one of them would
/// leave the others silent: a global initialized to null, a local, a parameter chiero itself
/// forked to null (wave 186), and an entry in a table that is otherwise valid.
#[test]
fn calling_through_a_null_function_pointer_is_reported() {
    for (what, src) in [
        (
            "global",
            "int (*fp)(void) = 0;\nint probe(void){ return fp(); }",
        ),
        (
            "local",
            "int probe(void){ int (*fp)(void) = 0; return fp(); }",
        ),
        ("parameter", "int probe(int (*fp)(void)){ return fp(); }"),
        (
            "table entry",
            "static int one(void){ return 11; }\n\
             int (*tab[2])(void) = { one, 0 };\n\
             int probe(void){ return tab[1](); }",
        ),
    ] {
        let f = findings(src);
        assert!(
            reports_null(src),
            "`{what}`: calling through null is a fault at a definite place, not a gap: {f:?}"
        );
    }
}

/// A call that resolves is not a finding.
///
/// The control against the cheapest wrong fix — reporting on every indirect call, which
/// would bury the real one and make the table idiom unusable.
#[test]
fn an_indirect_call_that_resolves_is_not_reported() {
    for (what, src) in [
        (
            "direct index",
            "static int one(void){ return 11; }\nstatic int two(void){ return 22; }\n\
             int (*tab[2])(void) = { one, two };\nint probe(void){ return tab[1](); }",
        ),
        (
            "through a global",
            "static int one(void){ return 11; }\nint (*fp)(void) = one;\n\
             int probe(void){ return fp(); }",
        ),
        (
            "struct field",
            "static int one(void){ return 11; }\n\
             struct Node { int (*fn)(void); };\nstruct Node n = { one };\n\
             int probe(void){ return n.fn(); }",
        ),
    ] {
        let f = findings(src);
        assert!(
            f.is_empty(),
            "`{what}` resolves to a real function and is ordinary code: {f:?}"
        );
    }
}

/// A guard discharges it, exactly as it does for a data pointer.
///
/// The idiom every dispatch loop is written with. It also pins that the fix goes through the
/// path condition rather than by recognising a syntactic shape — the same requirement wave
/// 186 imposed on the null-parameter work.
#[test]
fn a_checked_function_pointer_is_not_reported() {
    let f = findings("int probe(int (*fp)(void)){ if (!fp) return 0; return fp(); }");
    assert!(
        f.is_empty(),
        "the call happens only where `fp` is non-null: {f:?}"
    );
}
