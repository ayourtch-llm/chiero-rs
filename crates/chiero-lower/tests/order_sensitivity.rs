//! Covers: 020 contract 18(a) — the **syntactic** half of order sensitivity.
//!
//! 020 §7: C leaves the evaluation order of subexpressions unspecified; CIR picks one and
//! writes it down, so results are reproducible. Because a specific order can hide a real
//! bug, the dependence is also *detected* — and §7 splits that work deliberately.
//!
//! Lowering owns only the **syntactically decidable** cases: two unsequenced accesses to
//! the same *lvalue root* within one full expression, at least one a write, with no
//! intervening call. `i = i++`, `a[i] = i++`, `f(i++, i++)`. That is a local, call-free
//! check, which 001 §2 permits `chiero-lower` to do without becoming an analysis.
//!
//! §7 says outright why the other half exists: an earlier draft assigned both to lowering
//! while forbidding lowering to contain analyses, which is jointly unsatisfiable for the
//! very example it gave. The interprocedural half is a 040 checker and is tested in
//! `chiero-check`. **Neither half alone is contract 18** — §10 notes that the syntactic
//! case by itself is passed by an implementation that does no analysis at all.

mod harness;
use harness::lower;

fn is_order_sensitive(src: &str, name: &str) -> bool {
    let m = lower(src);
    m.funcs
        .iter()
        .find(|f| &*f.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no `{name}` in {:?}",
                m.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
            )
        })
        .attrs
        .order_sensitive
}

/// **`i = i++` is order sensitive; `i = i + 1` is not.**
///
/// The negative half is the whole test. Setting the flag on every function is a passing
/// implementation of the positive half and a useless one — a checker that fires on all of
/// VPP reports nothing anybody will read.
#[test]
fn an_unsequenced_write_and_read_of_one_object_is_order_sensitive() {
    assert!(
        is_order_sensitive("int f(int i) { i = i++; return i; }", "f"),
        "`i = i++` writes `i` twice with nothing sequencing them"
    );
    assert!(
        !is_order_sensitive("int f(int i) { i = i + 1; return i; }", "f"),
        "`i = i + 1` reads `i` once and writes it once, sequenced by the assignment"
    );
}

/// §7's other two named examples, which differ from `i = i++` in shape rather than in
/// principle.
///
/// `a[i] = i++` is the one that catches an implementation keyed on the *assigned* lvalue:
/// the conflict is between the subscript's read of `i` and the increment's write, and the
/// thing being assigned is `a[i]`, a different root entirely. `f(i++, i++)` is the one
/// that catches an implementation looking only at assignment — there is no assignment in
/// it at all.
#[test]
fn the_other_two_shapes_the_spec_names_are_order_sensitive() {
    assert!(
        is_order_sensitive("int f(int i) { int a[4]; a[i] = i++; return a[0]; }", "f"),
        "`a[i] = i++`: the subscript reads `i` and the increment writes it"
    );
    assert!(
        is_order_sensitive("int g(int, int); int f(int i) { return g(i++, i++); }", "f"),
        "`f(i++, i++)`: two writes to `i`, and C does not sequence argument evaluation"
    );

    // Two arguments touching *different* objects are not order sensitive, or the check is
    // just "does this call have two arguments".
    assert!(
        !is_order_sensitive(
            "int g(int, int); int f(int i, int j) { return g(i++, j++); }",
            "f"
        ),
        "`g(i++, j++)` touches two roots, and neither races the other"
    );
}

/// **A read paired with a read is not a conflict.**
///
/// §7 says "at least one a write". `g(i, i)` reads `i` twice in an unsequenced region and
/// is perfectly defined; an implementation that flagged any two accesses to one root
/// would report it, and would then report most of the corpus.
#[test]
fn two_unsequenced_reads_are_not_order_sensitive() {
    assert!(
        !is_order_sensitive("int g(int, int); int f(int i) { return g(i, i); }", "f"),
        "reading one object twice has no order to depend on"
    );
}

/// **A sequence point ends the region.**
///
/// `i++; i++;` is two full expressions and is defined; `i++ + i++` is one and is not. The
/// two differ by a single semicolon, which is exactly the distinction §7's `SeqPoint`
/// markers exist to carry — an implementation that scanned a whole function for repeated
/// accesses flags both.
#[test]
fn a_sequence_point_separates_two_accesses() {
    assert!(
        !is_order_sensitive("int f(int i) { i++; i++; return i; }", "f"),
        "two statements are two full expressions"
    );
    assert!(
        is_order_sensitive("int f(int i) { return i++ + i++; }", "f"),
        "one full expression with two unsequenced writes"
    );

    // `&&` sequences its operands (C11 6.5.13p4), so it is *not* a conflict — the
    // difference between `&&` and `+` here is the reason lowering emits a `SeqPoint`
    // rather than counting operands.
    assert!(
        !is_order_sensitive("int f(int i) { return i++ && i++; }", "f"),
        "`&&` has a sequence point between its operands"
    );
}

/// **An intervening call ends the syntactic region**, because after one the answer needs
/// side-effect summaries — which is precisely what §7 moves to the 040 checker.
///
/// This is lowering *declining*, not lowering being wrong. The finding for the call case
/// exists; it is reported by the checker in `chiero-check`, over the same CIR.
#[test]
fn an_intervening_call_is_left_to_the_checker() {
    assert!(
        !is_order_sensitive(
            "int g(void); int f(int i) { return (i = 1) + g() + (i = 2); }",
            "f"
        ),
        "with a call between them, whether the two writes race is not syntactically \
         decidable — 001 §2 forbids lowering the analysis that would answer it"
    );
}

/// The flag is **per function**, so one order-sensitive function does not mark its
/// neighbours.
#[test]
fn the_flag_does_not_leak_between_functions() {
    let src = "int bad(int i) { i = i++; return i; }\n\
               int good(int i) { return i + 1; }\n";
    assert!(is_order_sensitive(src, "bad"));
    assert!(
        !is_order_sensitive(src, "good"),
        "the flag belongs to the function that has the conflict"
    );
}
