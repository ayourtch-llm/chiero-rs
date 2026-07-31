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

/// **A bit-field width has constraints, and none of them were checked.**
///
/// Found by following wave 301's lead — fold sites whose fallback cannot be told apart from a
/// computed answer. The width was folded with `.unwrap_or(0).max(0)`, and the very next line
/// treats a width of zero as C's *legal* unnamed zero-width bit-field. So a width that could not
/// be folded, and a width that folded to a negative number, both silently became a valid but
/// entirely different declaration: the member vanished and the next field was pushed to a unit
/// boundary. Nothing was reported.
///
/// Four constraint violations, each of which gcc rejects and this engine accepted (C 6.7.2.1p4):
///
///   - a width that is not an integer constant expression;
///   - a negative width;
///   - a width exceeding the field type's own width — chiero allocated a *wider unit* for
///     `int f : 33`, giving the struct a size gcc will not produce for any program;
///   - a **named** zero-width field. Zero width is legal only without a name, because its whole
///     purpose is to force alignment rather than to store anything.
///
/// The legal cases below are the discriminators, and the widths that sit exactly on a boundary
/// are there on purpose: `int f : 32` and `long f : 33` are both fine, so a check written as
/// "wider than `int`" rather than "wider than the field's type" would pass the violations above
/// and fail here.
#[test]
fn bit_field_width_constraints_are_checked() {
    let diags = |members: &str| {
        let src = format!("int notconst; struct S {{ {members} }}; struct S s;");
        let p = harness::parse_allowing_diagnostics(&src, TargetConfig::x86_64_linux());
        p.analysis.diagnostics.len()
    };

    for bad in [
        "int a; int f : notconst; int b;",
        "int a; int f : -1; int b;",
        "int a; int f : 33; int b;",
        "int a; unsigned f : 33; int b;",
        "int a; int f : 0; int b;",
    ] {
        assert!(diags(bad) > 0, "must be diagnosed: `struct S {{ {bad} }}`");
    }

    // **The recovery keeps the member.** A rejected width falls back to one bit rather than
    // zero, because zero is the value that deletes the field and pushes everything after it to a
    // unit boundary — which is how these violations went unnoticed in the first place. A reader
    // whose width is wrong should get one diagnostic about the width, not a second about a field
    // that disappeared because of it. The sizes below are the two outcomes: with the member kept
    // the struct is laid out as `int, bits, int`; with it dropped, `b` joins `a`'s unit.
    let size_of_struct = |members: &str| {
        let src = format!("int notconst; struct S {{ {members} }}; struct S s;");
        let p = harness::parse_allowing_diagnostics(&src, TargetConfig::x86_64_linux());
        p.decl_ty("s").and_then(|t| p.analysis.size_of(t))
    };
    for bad in [
        "int a; int f : notconst; int b;",
        "int a; int f : -1; int b;",
        "int a; int f : 0; int b;",
        // An oversized width recovers to the field type's full width rather than to one bit,
        // which is the nearest legal declaration to what was written; either way the member has
        // to survive, and falling back to zero here would delete it exactly as above.
        "int a; int f : 33; int b;",
    ] {
        assert_eq!(
            size_of_struct(bad),
            Some(12),
            "the member survives a rejected width: `struct S {{ {bad} }}`"
        );
    }
    // The discriminator: an *unnamed* zero-width field really does declare no member, so the
    // same shape without the name is eight bytes. If the fallback were zero, every case above
    // would look like this one.
    assert_eq!(size_of_struct("int a; int : 0; int b;"), Some(8));

    for good in [
        "int a; int f : 3; int b;",
        "int a; int f : 32; int b;",
        "int a; unsigned f : 32; int b;",
        "int a; long f : 33; int b;",
        "int a; int f : sizeof(int); int b;",
        // Zero width is legal precisely when the field has no name.
        "int a; int : 0; int b;",
        "int a; int : 3; int b;",
    ] {
        assert_eq!(diags(good), 0, "must be accepted: `struct S {{ {good} }}`");
    }
}

/// **Where C requires a complete type, and where it deliberately does not.**
///
/// §9's front. Following the `unwrap_or(1)` in `addr_of` to its cause found something larger:
/// an incomplete tag is interned as `Ty::Error`, so this engine cannot tell "declared, not yet
/// defined" from "never mentioned". One check exists — an object's type must have a size — and
/// it fires on both, which makes it simultaneously too strict and too lax.
///
/// **Too strict** is the more serious half and is listed first below. `extern struct I x;` is
/// valid C: an external declaration names an object defined elsewhere, and the other translation
/// unit is where its size is known. Rejecting it turns a correct program into a broken one, which
/// no amount of missing checks does.
///
/// **Too lax** in four places, each of which gcc rejects: an array whose element type is
/// incomplete (no size means no stride), arithmetic on a pointer to an incomplete type — where
/// `size_of_ty(..).unwrap_or(1)` was silently scaling by one byte, as if every unknown struct
/// were a `char` — a pointer difference of the same, and `sizeof` of an incomplete type in either
/// spelling.
///
/// The accepted cases are the ones that make the rule a rule rather than a blanket ban. A pointer
/// *to* an incomplete type is the whole point of an opaque handle; declaring a function that
/// takes or returns one is legal as long as it is never called that way; and comparing two such
/// pointers needs no size at all.
#[test]
fn a_complete_type_is_required_exactly_where_c_requires_one() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis.diagnostics.len()
    };

    for good in [
        // An external declaration may have an incomplete type; another unit completes it.
        "struct I; extern struct I x;",
        // A pointer to an incomplete type is the opaque-handle idiom.
        "struct I; struct I *p;",
        "struct I; typedef struct I T; T *tp; int f(void) { return tp != 0; }",
        "struct I; struct I *p; struct I *q; int f(void) { return p == q; }",
        // Declaring — not defining, and not calling — is legal.
        "struct I; struct I g(void);",
        "struct I; int f(struct I v);",
        // The complete cases must stay silent, or the check is just noise.
        "struct C { int a; }; struct C c;",
        "struct C { int a; }; struct C arr[10]; int f(void) { return (int)sizeof(arr); }",
        "struct C { int a; }; struct C *p; int f(void) { return (int)(p + 1 - p); }",
    ] {
        assert_eq!(diags(good), 0, "must be accepted: `{good}`");
    }

    for bad in [
        // An object needs a size.
        "struct I; struct I x;",
        "union U; union U u;",
        // `extern` is not a blanket exemption — it is an exemption for *declarations*. With an
        // initializer the declaration is a definition (C 6.9.2p1) and the size is needed after
        // all, and `static` is a tentative definition in this unit, so nothing completes it
        // later. Both are here because the relaxation above is the easiest thing in this wave to
        // write too broadly, and mutation confirmed it: widening it to any `extern`, initializer
        // or not, passed every other case in this test.
        "struct I; extern struct I x = {0};",
        "struct I; static struct I x;",
        // An array needs its element's size, for the stride if nothing else.
        "struct I; struct I arr[10];",
        "struct I; extern struct I arr[];",
        // Pointer arithmetic needs the pointee's size — this is the `unwrap_or(1)` that
        // silently scaled by one byte.
        "struct I; struct I *p; void *q = p + 1;",
        "struct I; struct I *p; int f(void) { p = p + 1; return 0; }",
        "struct I; struct I *p; int f(void) { return (int)(p - p); }",
        // `sizeof` of an incomplete type, named directly or reached through a dereference.
        "struct I; int f(void) { return (int)sizeof(struct I); }",
        "struct I; struct I *p; int f(void) { return (int)sizeof(*p); }",
    ] {
        assert!(diags(bad) > 0, "must be diagnosed: `{bad}`");
    }
}
