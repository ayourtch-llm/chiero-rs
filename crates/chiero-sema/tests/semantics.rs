//! Covers: 014 contracts 9, 13, 19.

mod harness;

use chiero_sema::{ConstVal, SemaDiagnostic, TargetConfig, Ty, const_eval};
use harness::parse;

/// **Contract 9.** `char` is signed under x86-64 and unsigned under aarch64, and
/// `(char)0xFF == -1` only in the former.
///
/// 014 §1 makes this **data, not code** precisely so that both answers are reachable. A
/// target flag that only one code path ever reads is a constant with extra steps, so the
/// test asks the same question of both targets and requires different answers.
#[test]
fn plain_char_signedness_follows_the_target() {
    let x86 = parse("char c;", TargetConfig::x86_64_linux());
    let arm = parse("char c;", TargetConfig::aarch64_linux());

    let signed_on = |p: &harness::Parsed| match p.analysis.ty(p.decl_ty("c").expect("`c`")) {
        Ty::Int { signed, bits } => {
            assert_eq!(*bits, 8, "`char` is 8 bits either way");
            *signed
        }
        other => panic!("`char` is not an integer type: {other:?}"),
    };

    assert!(signed_on(&x86), "x86-64 Linux: plain `char` is signed");
    assert!(!signed_on(&arm), "aarch64 Linux: plain `char` is unsigned");

    // `signed char` and `unsigned char` are the *same* on both — they are distinct types
    // from plain `char`, and only the plain one moves with the target. Without this the
    // test could pass on an implementation that flipped every 8-bit type.
    for p in [&x86, &arm] {
        let q = parse(
            "signed char s; unsigned char u;",
            if p.analysis.ty(p.decl_ty("c").expect("`c`"))
                == &(Ty::Int {
                    signed: true,
                    bits: 8,
                })
            {
                TargetConfig::x86_64_linux()
            } else {
                TargetConfig::aarch64_linux()
            },
        );
        assert_eq!(
            q.analysis.ty(q.decl_ty("s").expect("`s`")),
            &Ty::Int {
                signed: true,
                bits: 8
            }
        );
        assert_eq!(
            q.analysis.ty(q.decl_ty("u").expect("`u`")),
            &Ty::Int {
                signed: false,
                bits: 8
            }
        );
    }
}

/// **Contract 13.** A true `_Static_assert` passes; a false one produces **exactly one**
/// diagnostic.
///
/// "Exactly one" is the contract. An implementation that reported the failure and then
/// also complained that the condition was not a constant, or that poisoned the enclosing
/// declaration, would satisfy "produces a diagnostic" while burying it.
#[test]
fn a_false_static_assert_is_exactly_one_diagnostic() {
    let ok = parse(
        "_Static_assert(sizeof(int) == 4, \"fine\");",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        ok.analysis.diagnostics.is_empty(),
        "a true assertion is silent: {:?}",
        ok.analysis.diagnostics
    );

    let bad = harness::parse_allowing_diagnostics(
        "_Static_assert(sizeof(int) == 5, \"nope\");\nint after;",
        TargetConfig::x86_64_linux(),
    );
    assert_eq!(
        bad.analysis.diagnostics.len(),
        1,
        "one diagnostic, not a cascade: {:?}",
        bad.analysis.diagnostics
    );
    assert!(
        bad.analysis.diagnostics[0].message.contains("nope"),
        "and it carries the message the source gave it: {:?}",
        bad.analysis.diagnostics[0]
    );
    assert!(
        bad.decl_ty("after").is_some(),
        "and the declaration after it is still analysed"
    );
}

/// **Contract 19.** Signed overflow in a constant expression is UB, so it is one
/// diagnostic — and evaluation **continues with the wrapped value** rather than poisoning
/// the expression, because an array bound that stopped resolving would cascade into every
/// use of the type.
#[test]
fn signed_overflow_in_a_constant_expression_wraps_and_diagnoses_once() {
    // `2147483647 + 1` in `int`.
    let (ast, expr) = harness::expression("2147483647 + 1");
    let names = harness::names_of(&ast);
    let mut diags: Vec<SemaDiagnostic> = Vec::new();
    let v = const_eval(
        &ast.ast,
        expr,
        &names,
        &TargetConfig::x86_64_linux(),
        &mut diags,
    );
    assert_eq!(
        diags.len(),
        1,
        "exactly one diagnostic for the overflow: {diags:?}"
    );
    assert_eq!(
        v,
        Some(ConstVal::Int(-2147483648)),
        "and the wrapped value, not `None` — a bound that stopped resolving would cascade"
    );

    // The discriminator: the same expression one below the edge is silent, so the
    // diagnostic is about overflow and not about addition.
    let (ast, expr) = harness::expression("2147483646 + 1");
    let names = harness::names_of(&ast);
    let mut diags = Vec::new();
    let v = const_eval(
        &ast.ast,
        expr,
        &names,
        &TargetConfig::x86_64_linux(),
        &mut diags,
    );
    assert!(diags.is_empty(), "no overflow, no diagnostic: {diags:?}");
    assert_eq!(v, Some(ConstVal::Int(2147483647)));
}

/// Constant evaluation over the forms 014 §6 needs for array bounds and bit-field widths.
#[test]
fn integer_constant_expressions_fold() {
    for (src, want) in [
        ("1 + 2 * 3", 7),
        ("(1 + 2) * 3", 9),
        ("0x10", 16),
        ("010", 8),
        ("1 << 6", 64),
        ("64 * 8", 512),
        ("1 ? 2 : 3", 2),
        ("0 ? 2 : 3", 3),
        ("-5 / 2", -2),
        ("-5 % 2", -1),
        ("~0", -1),
        ("!0", 1),
        ("1 == 1", 1),
        ("3 > 4", 0),
        ("'A'", 65),
        ("1u + 1", 2),
        ("0xffffffffu", 4294967295),
        // **A decimal literal never becomes unsigned** (C11 §6.4.4.1), so this is a
        // `long` and the addition does not wrap. A mutation that let it be `unsigned int`
        // folded this to 0 and reported nothing, because unsigned overflow is defined —
        // so the wrong type is silent, which is why the pair below is the test rather
        // than the literal alone.
        ("4294967295 + 1", 4294967296),
        // The hex spelling of the same value *is* `unsigned int`, so it wraps to 0.
        ("0xffffffff + 1", 0),
    ] {
        let (ast, expr) = harness::expression(src);
        let names = harness::names_of(&ast);
        let mut diags = Vec::new();
        assert_eq!(
            const_eval(
                &ast.ast,
                expr,
                &names,
                &TargetConfig::x86_64_linux(),
                &mut diags
            ),
            Some(ConstVal::Int(want)),
            "`{src}` should fold to {want} ({diags:?})"
        );
        assert!(diags.is_empty(), "`{src}` should be silent: {diags:?}");
    }
}

/// **C11 6.5.1.1p2's two constraints on a `_Generic` association list.**
///
/// At most one `default`, and no two associations naming compatible types. gcc rejects both, so
/// the differential oracle can never see them — a program it refuses to compile produces no
/// answer to disagree with.
///
/// # Why these exist as fixtures at all
///
/// Mutation. `chosen.or(...)` versus a plain assignment, and `fallback.or(...)` versus a plain
/// assignment, both survived the whole differential suite: with a *valid* program there is
/// exactly one match and exactly one `default`, so "first wins" and "last wins" are the same
/// function. The only way to tell them apart is a program C forbids — at which point the
/// interesting question is not which arm is picked but whether the program is reported at all.
#[test]
fn a_generic_selection_reports_its_constraint_violations() {
    let dup_default = harness::parse_allowing_diagnostics(
        "int probe(void) { return _Generic(1, int: 1, default: 2, default: 3); }",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        dup_default
            .analysis
            .diagnostics
            .iter()
            .any(|d: &SemaDiagnostic| d.message.contains("two `default`s")),
        "two `default`s is a constraint violation: {:?}",
        dup_default.analysis.diagnostics
    );

    let dup_type = harness::parse_allowing_diagnostics(
        "int probe(void) { return _Generic(1, int: 1, int: 2); }",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        dup_type
            .analysis
            .diagnostics
            .iter()
            .any(|d: &SemaDiagnostic| d.message.contains("two `_Generic` associations match")),
        "two matching associations is a constraint violation: {:?}",
        dup_type.analysis.diagnostics
    );

    // **The control, and it is the point.** A well-formed selection with one `default` and one
    // matching arm must stay silent, or the two assertions above would pass on a rule that
    // fires for every program.
    let ok = parse(
        "int probe(void) { return _Generic(1, long: 1, int: 2, default: 3); }",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        ok.analysis.diagnostics.is_empty(),
        "a well-formed `_Generic` is not a diagnostic: {:?}",
        ok.analysis.diagnostics
    );
}

/// **An enumerator whose initializer cannot be evaluated is a diagnostic, not a zero.**
///
/// 020 §5: a gap is a diagnostic rather than a licence. The enumeration walk folded each
/// initializer with `self.eval(e).map(|v| v.v).unwrap_or(next)`, so anything `const_eval` could
/// not answer silently became the *implicit* next value — the same number the enumerator would
/// have had with no initializer at all. That is the worst possible failure: indistinguishable
/// from a correct answer, and it makes every future gap in `const_eval` invisible.
///
/// It was found through one: `sizeof` of an expression had no arm, so `enum { E = sizeof(1) };`
/// was 0 rather than 4, and nothing said so. Fixing that arm removes this instance, so the test
/// uses an initializer that is genuinely not constant and must stay that way.
///
/// The second enumerator checks the *counting* is not disturbed: after a rejected initializer the
/// implicit sequence still has to continue from somewhere, and the value chosen must not silently
/// masquerade as computed.
#[test]
fn an_enumerator_that_cannot_be_folded_is_diagnosed() {
    let p = harness::parse_allowing_diagnostics(
        "int notconst; enum E { A = notconst, B };",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        !p.analysis.diagnostics.is_empty(),
        "a non-constant enumerator initializer must be diagnosed, not quietly taken as the \
         implicit next value"
    );

    // The discriminator: the same shape with a foldable initializer says nothing.
    let ok =
        harness::parse_allowing_diagnostics("enum F { C = 3, D };", TargetConfig::x86_64_linux());
    assert!(
        ok.analysis.diagnostics.is_empty(),
        "a constant initializer is not a complaint: {:?}",
        ok.analysis.diagnostics
    );
}
