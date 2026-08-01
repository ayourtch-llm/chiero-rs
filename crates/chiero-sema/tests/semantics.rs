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
        "struct I; struct S { struct I *m; };",
        // The enum cases that must stay legal: a pointer to one, an external declaration, a
        // function declaration, and any use once the enumerators have been seen. The last is a
        // redeclaration *after* the definition, which is legal and must not undo it.
        "enum E; enum E *p;",
        "enum E; extern enum E e;",
        "enum E; enum E f(void);",
        "enum E { A }; enum E e;",
        "enum E { A }; enum E; enum E e;",
        // The same tag by pointer, from inside its own definition: the case the whole
        // representation change exists for.
        "struct Node { int v; struct Node *next; };",
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
        // A member must have a size: the record has to place it. `struct S { struct S s; }` is
        // here because reserving the record before laying out its members — what makes a
        // self-referential *pointer* work — also makes the tag findable by value. What stops it
        // is that the record it finds is still marked incomplete while its own members are walked.
        "struct I; struct S { struct I m; };",
        "struct I; union V { struct I m; int x; };",
        "struct S { struct S s; };",
        // An undefined `enum` tag is incomplete too, and was answering as a plain `int` — so
        // every one of these was accepted with a size of four. Forward-declaring an enum is a GNU
        // extension rather than standard C, which is exactly why it needs saying: gcc accepts the
        // *declaration* and then rejects every use that needs a size.
        "enum E; enum E e;",
        "enum E; struct S { enum E m; };",
        "enum E; int a[sizeof(enum E)];",
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

/// **One cause, one report.** `enum { X = sizeof(struct I) }` produced two diagnostics: that
/// `sizeof` was applied to an incomplete type, and that the enumerator was therefore not a
/// constant expression. The second tells a reader nothing the first did not, and 023 §9 asks for
/// reports a person can act on rather than every true sentence about a program.
///
/// The enumerator message is *kept* for the case it was written for in wave 301 — an initializer
/// the engine cannot fold and has said nothing else about — which is the discriminator here.
#[test]
fn an_explained_fold_failure_is_not_reported_twice() {
    let messages = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    let both = messages("struct I; enum { X = (int)sizeof(struct I) };");
    assert_eq!(both.len(), 1, "one cause, one report: {both:?}");
    assert!(both[0].contains("incomplete"), "{both:?}");

    // The discriminator: nothing explains *this* failure, so the enumerator must speak.
    let silent = messages("int notconst; enum E { A = notconst, B };");
    assert_eq!(silent.len(), 1, "{silent:?}");
    assert!(
        silent[0].contains("not an integer constant expression"),
        "{silent:?}"
    );
}

/// **A name declared inside a block collides with nothing.**
///
/// Contract 14's redefinition rule — a second *initialized* definition of the same name is an
/// error — was applied with a whole-translation-unit symbol set and no notion of scope, because
/// `item()` handles file-scope items and block-scope declarations through the same path. So any
/// two initialized declarations sharing a name anywhere in the file collided.
///
/// That is not an exotic collision. Two functions that each say `int a = 0;` collide. Two
/// `for (int i = 0; ...)` loops in one function collide. A local that shadows a file-scope
/// variable collides. This is ordinary C, and the report says the program is broken when it is
/// not — 023 §9's "a report a person cannot act on is not a report", in its worst form, since a
/// reader who acts on this one makes their program worse.
///
/// It went unnoticed because a sema diagnostic does not stop lowering: the corpus compiled and
/// ran these programs correctly while complaining about them, so every test that checks *answers*
/// stayed green. Only a test that reads the diagnostics can see it.
///
/// The rejected cases are the rule the check was written for and must keep: at file scope, two
/// initialized definitions of one name really are an error, whatever their linkage.
#[test]
fn a_block_scope_declaration_redefines_nothing() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for good in [
        // Two functions, each with a local of the same name. Not even shadowing.
        "int f(void){ int a = 0; return a; } int g(void){ int a = 1; return a; }",
        // A nested block shadowing an enclosing local.
        "int f(void){ int n = 0; { int n = 1; (void)n; } return n; }",
        // The everyday one: two counted loops in one function.
        "int f(void){ for(int i=0;i<2;i++){} for(int i=0;i<2;i++){} return 0; }",
        // A local shadowing a file-scope object.
        "static int s = 1; int f(void){ int s = 2; return s; }",
        // Static locals are distinct objects even with the same name.
        "int f(void){ static int c = 0; return ++c; } int g(void){ static int c = 0; return ++c; }",
        // A shadowing declaration of a different type is still a different object.
        "int f(void){ int x = 1; { long x = 2; return (int)x; } }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }

    for bad in [
        "int x = 1; int x = 2;",
        "static int y = 1; static int y = 2;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }
}

/// **An operation whose operand's type cannot support it.**
///
/// Rows 1–3 of wave 307's constraint census, which form one family: subscripting something that
/// is not an array or pointer, taking a member of something that is not a structure, naming a
/// member that does not exist, and calling something that is not a function. All four fell to
/// `Ty::Error` without a word.
///
/// The subscript is the one that does real damage. `int x = 5; return x[0];` **returned 5** —
/// lowering reads an `Error` type as a 32-bit integer, so the engine computed an answer for a
/// program gcc refuses, from an operation that means nothing. The other three produce no state at
/// all, which is a silent refusal rather than a wrong answer: bad, but not the same kind of bad.
///
/// **The accepted list carries the whole difficulty of this rule.** `Ty::Error` means *unknown*,
/// not *wrong*: an undeclared callee types as `Error` and `__builtin_isnan` is exactly that, since
/// nothing declares it and gcc knows it intrinsically. So the check cannot key on the poison — it
/// has to key on a type that is concretely known to be unusable here. A version that rejected
/// every `Error` would satisfy all five rejections and break the builtins, which are used
/// throughout the float corpus.
#[test]
fn an_operation_its_operand_cannot_support_is_diagnosed() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A member that does not exist, by value and through a pointer.
        "struct S { int m; }; int f(void){ struct S s; return s.nope; }",
        "struct S { int m; }; int f(struct S *p){ return p->nope; }",
        // A member of something that is not a structure at all.
        "int f(void){ int x = 5; return x.m; }",
        // Subscripting a non-array. This one returned a value.
        "int f(void){ int x = 5; return x[0]; }",
        // Calling something that is not a function.
        "int f(void){ int q = 5; return q(); }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **Contract 20: a poisoned base is one diagnostic, not two.** `Ty::Error` means the base's
    // type is already unknown and already reported, so the checks above stay quiet on it. Without
    // these the exemption is unfalsifiable — every case below is still "diagnosed", just twice.
    for (src, want) in [
        ("struct I; int f(void){ struct I s; return s.m; }", 1),
        ("struct I; int f(void){ struct I s; return s[0]; }", 1),
        ("struct I; int f(void){ struct I s; return s(); }", 1),
    ] {
        assert_eq!(
            diags(src).len(),
            want,
            "one bad declaration is one diagnostic: `{src}` -> {:?}",
            diags(src)
        );
    }

    for good in [
        "struct S { int m; }; int f(void){ struct S s; s.m = 1; return s.m; }",
        "struct S { int m; }; int f(struct S *p){ return p->m; }",
        "int f(int *p){ return p[0]; }",
        "int f(void){ int a[3]; a[0] = 1; return a[0]; }",
        "int f(int *p){ return 0[p]; }",
        "int g(void); int f(void){ return g(); }",
        "int f(int (*fp)(void)){ return fp(); }",
        // The discriminator: `Ty::Error` is *unknown*, not *wrong*. Nothing declares these and
        // gcc knows them intrinsically, so their callee types as `Error` and must stay silent.
        "int f(double x){ return __builtin_isnan(x); }",
        "int f(double x, double y){ return __builtin_isless(x, y); }",
        // A vector subscripts without decaying, which is why the `Index` arm has a second arm.
        "typedef int v4 __attribute__((vector_size(16))); int f(v4 v){ return v[0]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Correct code produces no diagnostics.** The gate wave 307 asked for and did not build.
///
/// Every false positive of the last three waves — contract 14's redefinition rule firing on two
/// functions with the same local name, and `__func__` reported undeclared — would have failed
/// this test on the day it was written. None of them failed anything, because a sema diagnostic
/// does not stop lowering: the corpus compiled, ran, and got the right answers while sema
/// complained, so every test that reads an *answer* stayed green.
///
/// The corpus is six real VPP headers, already parsed by `corpus_analyses` for the layout gate,
/// which asserts the preprocessor and the parser are clean and then never looks at sema. This
/// closes that gap for the cost of one assertion.
///
/// **A finding here is far more likely to be ours than VPP's.** This is shipped C that gcc
/// compiles without complaint; the question the test asks is not "is VPP correct" but "does this
/// engine think correct code is wrong".
#[test]
fn the_corpus_analyses_without_a_single_diagnostic() {
    let Some(cases) = harness::corpus_analyses() else {
        eprintln!("skipping: gcc not on PATH, so no system headers to preprocess against");
        return;
    };
    let mut complaints = Vec::new();
    for (seed, p) in &cases {
        for d in &p.analysis.diagnostics {
            complaints.push(format!("{seed}: {}", d.message));
        }
    }
    complaints.dedup();
    assert!(
        complaints.is_empty(),
        "sema complained about code gcc accepts:\n  {}",
        complaints.join("\n  ")
    );
}

/// **Rows 4–7 of wave 307's census: four constraints about types, none of them checked.**
///
/// Modifying a `const` object, defining a function with a parameter of incomplete type, declaring
/// a variable `void`, and using the value of a `void` call. gcc rejects all of them; this engine
/// accepted every one.
///
/// The `const` rows need something sema does not have: the AST carries `Quals` and sema has never
/// read them, so const-ness is invisible below the parser. That is why the accepted list here is
/// long. `int *const p` makes *`p`* read-only and leaves `*p` writable; `const int *p` is the
/// reverse; and `const` on a struct member is a third thing again. A check that asked "does the
/// word `const` appear in this declaration" would reject three legal programs for each illegal one
/// it caught.
///
/// The `void` rows are the cheap half and share machinery already present: a `void` object is an
/// incomplete type like any other, and a `void`-valued call is only an error where a *value* is
/// wanted — `v();` as a statement is how void functions are called.
#[test]
fn the_type_constraints_of_census_rows_four_to_seven() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Row 4: modifying a read-only object, in each of the ways C spells modification.
        "int f(void){ const int k = 1; k = 2; return k; }",
        "int f(void){ const int k = 1; k += 2; return k; }",
        "int f(void){ const int k = 1; k++; return k; }",
        "int f(void){ const int k = 1; --k; return k; }",
        "int f(const int p){ p = 2; return p; }",
        // The `const` is on the pointer here, so `p` is what may not be assigned.
        "int f(void){ int x=0; int *const p = &x; p = 0; return x; }",
        // Row 5: a *definition* may not take a parameter of incomplete type.
        "struct T; int s(struct T t){ return 0; }",
        // Row 6: an object may not have type void.
        "int f(void){ void w; return 0; }",
        // Row 7: a void call has no value to use.
        "void v(void); int f(void){ return v(); }",
        "void v(void); int f(void){ int x = v(); return x; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Reading a `const` is the whole point of one.
        "int f(void){ const int k = 1; return k; }",
        "int f(const int p){ return p + 1; }",
        // `int *const p` leaves the pointee writable...
        "int f(void){ int x=0; int *const p = &x; *p = 1; return x; }",
        // ...and `const int *p` leaves the pointer assignable.
        "int f(void){ int x=1; const int *p = &x; p = 0; return p == 0; }",
        "int f(void){ int x=1; const int *p = &x; return *p; }",
        // A pointer to an incomplete type is a complete type, and a *declaration* may take one.
        "struct T; int s(struct T t);",
        "struct T; int s(struct T *t){ return 0; }",
        // `void *` is an object type; only `void` itself is not.
        "void *g; int f(void){ return g == 0; }",
        // A void call as a statement, and discarded explicitly, are both how they are used.
        "void v(void); int f(void){ v(); return 0; }",
        "void v(void); int f(void){ (void)v(); return 0; }",
        "typedef void V; V h(void); int f(void){ h(); return 0; }",
        // A function returning void, defined and called.
        "static void s(int *p){ *p = 1; } int f(void){ int x=0; s(&x); return x; }",
        // **`return v();` from a `void` function is legal** (C 6.8.6.4p1) and is the reason the
        // void-value rule tests the *target* type rather than just the source. Dropping that test
        // rejects this and passes everything else in the list.
        "void v(void); void w(void){ return v(); }",
        "void v(void); void w(void){ v(); return; }",
        // **A block may shadow a `const` with a mutable object.** Without the removal from the
        // read-only set, the inner `k` would inherit the outer one's constness and this would be
        // rejected — which is what makes the removal load-bearing rather than tidiness.
        "int f(void){ const int k = 1; { int k = 2; k = 3; return k; } }",
        "int f(const int p){ int q = p; { int p = 1; p = 2; q += p; } return q; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Rows 8–12 of the census: five constraints about *where a statement is*, not about types.**
///
/// A duplicate `case` value, a second `default`, `break` outside a loop or switch, `continue`
/// outside a loop, and a `goto` to a label that is never defined. gcc rejects all eight programs
/// below; this engine accepted every one, because sema walks statements without tracking what
/// encloses them.
///
/// The accepted list is where the shape of that context is decided, and three of its cases decide
/// it between them:
///
///   - **`continue` inside a `switch` inside a loop is legal.** It continues the *loop* — a switch
///     is not a continuable statement. So `break` counts loops *and* switches while `continue`
///     counts only loops, and one shared depth counter would reject this.
///   - **Nested and sibling switches each have their own case set**, and their own `default`. A
///     single set per function would reject `switch(a){case 1:} switch(a){case 1:}`, which is
///     ordinary code.
///   - **A label may be defined and never used.** Only the reverse is an error, so the check is on
///     `goto` targets rather than on label declarations — and it can only run once the whole
///     function has been walked, since `goto` forward to a label declared later is legal.
///
/// `case 2-1` duplicating `case 1` is here because the values must be *folded* before they are
/// compared: comparing the written expressions would miss it.
#[test]
fn the_statement_constraints_of_census_rows_eight_to_twelve() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(int n){ switch(n){ case 1: return 1; case 1: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 1: return 1; case 2-1: return 2; } return 0; }",
        "int f(int n){ switch(n){ default: return 1; default: return 2; } return 0; }",
        "int f(void){ break; return 0; }",
        "int f(void){ continue; return 0; }",
        "int f(void){ if (1) break; return 0; }",
        "int f(void){ goto nowhere; return 0; }",
        // `continue` in a switch that is *not* inside a loop has no loop to continue.
        "int f(int n){ switch(n){ case 1: continue; } return 0; }",
        // **Labels are scoped to their function.** A `goto` cannot reach one defined in another,
        // which is what makes clearing the sets per function load-bearing rather than tidiness.
        "int f(void){ lab: return 0; } int g(void){ goto lab; return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        "int f(int n){ switch(n){ case 1: return 1; case 2: return 2; default: return 3; } }",
        "int f(void){ for(int i=0;i<3;i++){ if(i) break; } return 0; }",
        "int f(void){ for(int i=0;i<3;i++){ if(i) continue; } return 0; }",
        "int f(int n){ do { if(n) break; } while(n); return 0; }",
        // **`break` in a bare `switch` is the ordinary way to end a case**, and it is the case
        // that separates the two depth counters: `break` counts switches, `continue` does not.
        "int f(int n){ switch(n){ case 1: break; } return 0; }",
        // `break` in a switch inside a loop breaks the switch...
        "int f(int n){ while(n){ switch(n){ case 1: break; } n--; } return 0; }",
        // ...and `continue` in the same place continues the loop.
        "int f(int n){ while(n){ switch(n){ case 1: continue; } n--; } return 0; }",
        // Each switch has its own case set and its own default, nested or sibling.
        "int f(int n){ switch(n){ case 1: switch(n){ case 1: return 1; } } return 0; }",
        "int f(int n){ switch(n){ case 1: switch(n){ default: return 1; } default: return 2; } }",
        "int f(int n){ int a = n; switch(a){ case 1: return 1; } switch(a){ case 1: return 2; } return 0; }",
        // Several labels on one statement is not a duplicate of anything.
        "int f(int n){ switch(n){ case 1: case 2: return 1; } return 0; }",
        "enum E { A=1, B=2 }; int f(enum E e){ switch(e){ case A: return 1; case B: return 2; } return 0; }",
        // **A `case` range is accepted and not checked for overlap.** `case 1 ... 3` is a GNU
        // extension whose duplicate rule is about intervals meeting, not values being equal, so
        // the check skips ranges rather than comparing their lower bounds and calling it done.
        // That is a declared limit: `case 1 ... 3:` beside `case 2:` is a duplicate gcc rejects
        // and this does not. Approximating it would trade a missed report for a wrong one.
        "int f(int n){ switch(n){ case 1 ... 3: return 1; case 4: return 2; } return 0; }",
        // Labels: backward, forward, and one that is never used at all.
        "int f(void){ int i=0; again: i++; if(i<3) goto again; return i; }",
        "int f(void){ int i=0; goto skip; i=9; skip: return i; }",
        "int f(void){ unused: return 0; }",
        "int f(void){ lab: return 0; } int g(void){ lab: return 1; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Rows 13–16, the last of the census: a declaration compared with an earlier one.**
///
/// A function defined twice, a declaration whose type conflicts with an earlier one, `static`
/// after non-`static` (and its mirror), and a function declared to return an array. gcc rejects
/// all seven below; this engine accepted every one, because `analyze` never compares a file-scope
/// declaration against a previous declaration of the same name. The cross-TU `GlobalTable` does
/// something adjacent, but it is a separate pass that `analyze` does not run.
///
/// The accepted list is longer than the rejected one on purpose — repeating a declaration is how
/// every header in C works, and three of these cases decide the rule's shape:
///
///   - **`extern int n;` after `static int n;` is legal** while `int n;` after it is not. The rule
///     is about *linkage*, and `extern` is the spelling that defers to whatever already exists.
///     A check phrased as "the storage classes differ" gets this backwards.
///   - **`int f(); int f(int x){...}` is legal.** The first is an old-style declaration with
///     unspecified parameters, not a claim that there are none, so it conflicts with nothing.
///   - **`int (*f(void))[3]` returns a pointer to an array**, which is fine; only returning the
///     array itself is not. The two spellings differ by one indirection and it is easy to write a
///     check that rejects both.
#[test]
fn the_declaration_constraints_of_census_rows_thirteen_to_sixteen() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Row 13: two definitions of one function, adjacent and with a declaration between
        // them. The second shape is what makes "once defined, always defined" load-bearing:
        // without it the bare declaration in the middle resets the record.
        "int f(void){ return 0; } int f(void){ return 1; }",
        "int f(void){ return 0; } int f(void); int f(void){ return 1; }",
        // Row 14: the parameter list conflicts, and the return type conflicts.
        "int h(int); int h(long){ return 0; }",
        "int h(int); long h(int){ return 0; }",
        // Row 15: linkage, in both directions that C forbids.
        "extern int n; static int n;",
        "static int n; int n;",
        "int f(void){ return 0; } static int f(void);",
        // Row 16: a function may not return an array.
        "int f(void)[3];",
        // A variadic prototype and a fixed one are different types, and the parameter lists are
        // both non-empty so this is inside what the comparison can see.
        "int f(int a, ...); int f(int a){ return a; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Declaring then defining, and declaring twice, is how headers work.
        "int f(void); int f(void){ return 0; }",
        "int f(void); int f(void);",
        "static int g(void); static int g(void){ return 0; }",
        "extern int n; extern int n;",
        "int h(int); int h(int){ return 0; }",
        "void f(void); void f(void){ }",
        "int f(int a, int b); int f(int a, int b){ return a+b; }",
        // `extern` defers to what exists; only a fresh external definition conflicts.
        "static int n; extern int n;",
        // An old-style declaration claims nothing about its parameters.
        "int f(); int f(int x){ return x; }",
        // A K&R definition, whose parameter types arrive between the `)` and the `{`, alone and
        // after a declaration of each shape. All three are legal, and together they are why the
        // separate old-style guard turned out to be unnecessary.
        "int f(a) int a; { return a; } int g(void){ return f(1); }",
        "int f(); int f(a) int a; { return a; }",
        "int f(int); int f(a) int a; { return a; }",
        // Variadic on both sides is not a difference.
        "int f(int a, ...); int f(int a, ...){ return a; }",
        // Different names never conflict.
        "int f(void){ return 0; } int g(void){ return 1; }",
        // Returning a pointer to an array is legal; only the array itself is not.
        "int (*f(void))[3];",
        "typedef int A[3]; A *g(void);",
        "struct S { int a; }; struct S f(void){ struct S s = {1}; return s; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The initializer census: is what is written compatible with what it initializes?**
///
/// The second census in wave 307's shape, aimed where sema had never been graded. `InitList`
/// types to `Ty::Error` and is never compared against the declared type at all, so none of the
/// seven constraints below was checked.
///
/// **Four of the seven are only errors under `-pedantic-errors`** — excess elements for an array,
/// a struct, or a scalar, and an over-long string — and gcc accepts them with a warning by
/// default. They are constraint violations either way (C 6.7.9p2), and wave 307's census stopped
/// at this boundary having tried exactly one of them and read "gcc:ok". Taking the verdict at
/// both strictness levels is what makes the difference visible; the other three are hard errors.
///
/// The accepted list is nineteen cases because this is where C is generous, and three of them
/// decide the shape of the rules:
///
///   - **`char s[3] = "abc";` is legal.** The terminator is dropped when it is the only thing
///     that does not fit — so the string rule is "more than `n` characters *before* the NUL",
///     not "longer than the array".
///   - **`int a[2][2] = {1,2,3,4};` is legal**: braces may be elided and the flat list is
///     distributed across the rows. So counting top-level items against the outer dimension is
///     wrong, and a check written that way rejects it.
///   - **`int a[3] = {[0] = 1, [2] = 3};` is legal**, and its highest index is 2 while it has two
///     items. Counting items cannot answer the range question; the *positions* have to be
///     tracked, and a designator moves the cursor.
#[test]
fn the_initializer_census() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Excess elements. gcc warns by default and errors under `-pedantic-errors`.
        "int a[3] = {1,2,3,4};",
        "struct S { int x, y; }; struct S s = {1,2,3};",
        "int x = {1,2};",
        "char s[3] = \"abcd\";",
        // Hard errors in gcc by default.
        "int a[3] = {[5] = 1};",
        "struct S { int x, y; }; struct S s = {.nope = 1};",
        "int f(void); int g = f();",
        // A vector has exactly its lanes, and one more is excess like any other aggregate.
        "typedef int v4 __attribute__((vector_size(16))); v4 v = {1,2,3,4,5};",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Exactly full, inferred length, and partial.
        "int a[3] = {1,2,3};",
        "int a[] = {1,2,3};",
        "int a[5] = {1,2};",
        "int h[2] = {0};",
        "struct S { int x, y; }; struct S s = {1};",
        // A scalar may be braced once.
        "int x = {1};",
        // Strings: room for the NUL, exactly no room for it, and inferred.
        "char s[4] = \"abc\";",
        "char s[3] = \"abc\";",
        "char s[] = \"abc\";",
        // Designators, in range and out of order.
        "int a[3] = {[1] = 5};",
        "int a[3] = {[0] = 1, [2] = 3};",
        "struct S { int x, y; }; struct S s = {.y = 2, .x = 1};",
        // Nested aggregates, with braces and with them elided.
        "int a[2][2] = {{1,2},{3,4}};",
        "union U { int a; int b; }; union U u[2] = {1,2};",
        // **An array with no length has no capacity to exceed.** Giving an unsized array a
        // capacity of one turns every inferred-length aggregate into an excess report.
        "int a[][2] = {1,2,3,4,5};",
        "int a[2][2] = {1,2,3,4};",
        "struct S { int m[2]; }; struct S s = {{1,2}};",
        "struct S { int x; }; struct S a[2] = {{1},{2}};",
        "union U { int i; float f; }; union U u = {1};",
        // **A vector initialises elementwise.** It is not a scalar, and treating it as one
        // rejected the entire vector corpus — four lanes read as three excess elements.
        "typedef int v4 __attribute__((vector_size(16))); v4 v = {1,2,3,4};",
        // **Address constants the folder cannot answer for are still constants.** A function
        // designator in a table of function pointers, and the address of an array element, are
        // both legal at file scope and neither `eval` nor `addr_of` says so.
        "static int a1(int x){return x;} static int (*tab[1])(int) = {a1};",
        "static int arr[4]; static int *p = &arr[1];",
        "static char *s = \"abc\";",
        // Constant expressions at file scope, including an address constant.
        "int g = 1 + 2;",
        "int y; int *p = &y;",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The conversion census: is this value allowed where it is being put?**
///
/// The third census in wave 307's shape, aimed at assignment, argument passing, `return` and
/// pointer comparison. Sema converts operands but never asks whether the conversion is one C
/// permits, so a pointer and an integer are interchangeable everywhere.
///
/// **Every violation here is a gcc *warning* by default and an error only under
/// `-pedantic-errors`.** That is the whole reason this ground was never covered: wave 307's census
/// tried `int *p = 1;` and `return p;`, read "gcc:ok", and moved on. For an engine whose subject
/// is undefined behaviour these are not stylistic — a pointer read as an integer is exactly the
/// confusion the memory model exists to catch.
///
/// Seventeen accepted cases, and four of them decide the rules:
///
///   - **`void *` converts to and from any object pointer without a cast.** That is what makes it
///     `void *`, and a rule phrased as "the pointee types differ" rejects `g(p)` for
///     `void g(void *)`, which is most of the C standard library.
///   - **`0` is a null pointer constant**, so `int *p = 0;` and `p == 0` are legal while
///     `int *p = 1;` is not. The distinction is the *value*, not the type.
///   - **`int (*fp)(int) = g;` is legal and `int (*fp)(long) = g;` is not**, so function pointers
///     are compared by their full type rather than being waved through as pointers.
///   - **Arithmetic conversions are unrestricted**: `long` to `int`, `double` to `int`, `unsigned`
///     to `int` all narrow silently in C, and a rule that caught pointer mixing by comparing type
///     identity would reject every one of them.
///
/// **`const int *cp; int *p = cp;` is deliberately absent.** Discarding a qualifier is the ninth
/// violation gcc reports here, and sema does not model pointee qualifiers at all — wave 311 put
/// `const` on *objects* only. Adding it is a type-system change, not a check, and pretending
/// otherwise would mean a rule that fires on the wrong thing.
#[test]
fn the_conversion_census() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // An integer that is not a null pointer constant, and a pointer used as an integer.
        "int f(void){ int *p = 1; return *p; }",
        "int f(int *p){ int x = p; return x; }",
        // Pointers to unrelated types, in each of the four places a conversion happens.
        "int f(int *p){ char *q = p; return *q; }",
        "void g(int *); int f(char *q){ g(q); return 0; }",
        "int f(int *p){ return p; }",
        "int f(int *p, char *q){ return p == q; }",
        "struct S { int a; }; int f(struct S *s){ int *p = s; return *p; }",
        // A function pointer whose signature does not match.
        "int g(int); int f(void){ int (*fp)(long) = g; return (int)fp(1); }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **Contract 20: a poisoned operand is one diagnostic, not two.** An expression whose type is
    // already `Ty::Error` has been reported by whatever made it so, and converting it must not add
    // a second complaint. Only an exact count can falsify that — every case here is "diagnosed"
    // either way.
    for (src, want) in [
        // An undeclared name types as `Ty::Error`, and assigning it to a *pointer* is what
        // reaches the exemption: the check only runs when one side is pointer-like, so an
        // incomplete struct — a record rather than poison since wave 304 — leaves through the
        // arithmetic door before the poison is ever consulted.
        ("int f(void){ int *p = nosuch; return p != 0; }", 1),
        ("void g(int *); int f(void){ g(nosuch); return 0; }", 1),
        ("int *f(void){ return nosuch; }", 1),
    ] {
        assert_eq!(
            diags(src).len(),
            want,
            "one bad declaration is one diagnostic: `{src}` -> {:?}",
            diags(src)
        );
    }

    for good in [
        // A cast says the programmer meant it.
        "int f(void){ int *p = (int *)1; return p != 0; }",
        "void g(void *); int f(int *p){ g((void *)p); return 0; }",
        // `void *` converts both ways without one.
        "int f(int *p){ void *v = p; int *q = v; return *q; }",
        "void g(void *); int f(int *p){ g(p); return 0; }",
        // `0` is a null pointer constant; `1` is not.
        "int f(void){ int *p = 0; return p == 0; }",
        "int f(int *p){ return p == 0; }",
        "int f(void){ int (*fp)(void) = 0; return fp == 0; }",
        // Same types, and a string literal.
        "int f(int *p){ int *q = p; return *q; }",
        "int f(int *p, int *q){ return p == q; }",
        "int f(void){ char *s = \"abc\"; return s[0]; }",
        "int g(int); int f(void){ int (*fp)(int) = g; return fp(1); }",
        // Adding `const` to a pointee is a widening, not a mismatch.
        "int f(int *p){ const int *cp = p; return *cp; }",
        // Arithmetic conversions narrow and widen freely — a rule based on type identity
        // would reject all of these.
        "int f(void){ long l = 1; int i = l; return i; }",
        "int f(void){ int i = 1; long l = i; return (int)l; }",
        "int f(void){ double d = 1.5; int i = d; return i; }",
        "int f(void){ int i = 1; double d = i; return (int)d; }",
        "int f(void){ unsigned u = 1; int i = u; return i; }",
        // **`_Bool` takes any scalar** (C 6.3.1.2) — a pointer converts to it as a test against
        // zero, not a truncation. It is the one destination that accepts everything.
        "int f(int *p){ _Bool b = p; return b; }",
        // **A parameter declared as an array keeps its array type in sema** while the argument
        // passed to it has decayed to a pointer, so the two sides of one legal call arrive
        // spelled differently. Comparing spellings rather than pointees rejects this.
        "int sum(int a[2][3]); int f(void){ int a[2][3]; return sum(a); }",
        "int g(int *p); int f(void){ int a[4]; return g(a); }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Writing through a pointer to `const`.**
///
/// §9's first front, taken in half. The other half — `int *p = cp;` discarding the qualifier —
/// needs *qualified types*, and `Ty` has 436 match sites across four crates: a `Ty::Const`
/// wrapper would make every one of them a silently-wrong branch, which is the failure class this
/// project keeps finding. That is recorded as still blocked, with the measurement.
///
/// This half needs no type-system change and is the half that matters to this engine: `*p = 1`
/// through a `const int *` is a **write to a read-only object**, which is undefined behaviour
/// rather than a matter of taste. Wave 311 gave `const` to objects; this gives it to pointees, and
/// the two are deliberately separate sets because C keeps them separate.
///
/// The accepted list is where the two `const`s are told apart:
///
///   - **`int *const p` is a const *pointer*** — `*p = 1` is legal and `p = 0` is not, which is
///     wave 311's rule. `const int *p` is the reverse. A single notion of "p is const" gets one
///     of them wrong whichever way it is written.
///   - **`const int **p; *p = 0;` is legal**: `*p` has type `const int *`, which is not itself
///     const-qualified. Only the innermost pointee is read-only, and one level of indirection is
///     one level.
///   - **Reading is always allowed**, which is the entire purpose of a pointer to const —
///     `memcpy`'s `const char *src` is the shape, and case 13 is exactly that.
#[test]
fn a_write_through_a_pointer_to_const_is_rejected() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(const int *p){ *p = 1; return *p; }",
        "int f(const int *p){ (*p)++; return *p; }",
        "int f(const int *p){ *p += 1; return *p; }",
        // A parameter declared as an array of const is a pointer to const.
        "int f(const int p[]){ p[0] = 1; return p[0]; }",
        // A member of a const-qualified object reached through a pointer.
        "struct S { int m; }; int f(const struct S *s){ s->m = 1; return s->m; }",
        // **A local, not only a parameter.** Every other rejected case here is a parameter, so
        // the declaration site that records a local's pointee was unfalsifiable without this.
        "int f(void){ int x=1; const int *p = &x; *p = 2; return x; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Reading through it is the point of it.
        "int f(const int *p){ return *p; }",
        "int f(const int *p){ int x = *p; return x; }",
        "struct S { int m; }; int f(const struct S *s){ return s->m; }",
        "int f(char *d, const char *s){ *d = *s; return *d; }",
        // A non-const pointee is writable.
        "int f(int *p){ *p = 1; return *p; }",
        // **The other `const`**: the pointer is read-only, the pointee is not.
        "int f(int *const p){ *p = 1; return *p; }",
        // One level of indirection is one level: `*p` here is `const int *`, not `const int`.
        "int f(const int **p){ *p = 0; return **p; }",
        // A const array may still be read.
        "int f(void){ const int a[2] = {1,2}; return a[0]; }",
        // **A block may shadow a pointer-to-const with a writable one.** Without removing the
        // name from the set on the inner declaration, the shadow inherits the outer pointee's
        // constness — the same rule wave 311 needed for objects, and equally unloaded until now.
        "int g(const int *p){ int q=1; { int *p = &q; *p = 2; } return q; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Wave 314's two declared misses, closed.**
///
/// The initializer census left two things unchecked and said so. This is both.
///
/// **Brace elision.** `int a[2][2] = {1,2,3,4}` is legal and `{1,2,3,4,5}` is not, and wave 314
/// declined to distinguish them: detecting elision was easy, distributing correctly was not, so
/// the walk stopped counting entirely. It now counts *scalars* instead of items — the aggregate's
/// total capacity against the flat list's length — which answers the question without
/// distributing anything. `struct S { int p[2]; int q; }; struct S s[2] = {…}` holds six.
///
/// **A constant expression.** Wave 314 narrowed the file-scope rule to "the initializer contains a
/// call", because `eval` and `addr_of` miss things C does call constant — a function designator,
/// `&arr[1]`, a string. The rule now asks the question the other way round: what *disqualifies* an
/// initializer is **reading the value of a non-`const` object**. Everything else here — an array
/// name, a function name, an address, a string, an enumerator, `sizeof` — is an address or a
/// constant and passes.
///
/// `static const int c = 5; int g = c;` is in the accepted list because **gcc accepts it**, even
/// under `-pedantic-errors`. Strict C says a `const` object is not a constant expression; gcc
/// folds it, and a rule that rejected it would reject real code. Wave 311's `read_only` set is
/// exactly the information needed, which is why this is a lookup rather than a new pass.
#[test]
fn wave_314s_two_declared_misses() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // One scalar too many for a flat, brace-elided list.
        "int a[2][2] = {1,2,3,4,5};",
        "struct S { int p[2]; int q; }; struct S s[2] = {1,2,3,4,5,6,7};",
        // A union holds one member at a time, so an array of two holds two scalars, not four.
        // Summing its members instead of taking the largest doubles the capacity and lets this
        // through — and this is the shape that reaches the capacity rule rather than the array
        // range rule, which answers first for anything with more items than elements.
        "union U { int a; int b; }; union U u[2] = {1,2,3};",
        // Reading a non-`const` object is not a constant expression.
        "int x; int g = x;",
        "int x; int *p; int *q = p;",
        "int x; int a[2] = {1, x};",
        // Still caught, from wave 314.
        "int f(void); int g = f();",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Exactly full, and under-full, with braces elided.
        "int a[2][2] = {1,2,3,4};",
        "int a[2][3] = {1,2,3};",
        "struct S { int p[2]; int q; }; struct S s[2] = {1,2,3,4,5,6};",
        // Mixed forms stay unchecked rather than wrongly checked: `{{1,2},3,4}` is legal, and
        // counting scalars against capacity cannot see where the first item stops.
        "int a[2][2] = {{1,2},3,4};",
        "int a[2][2] = {{1,2},{3,4}};",
        // Address constants of every spelling, which the old `contains_call` rule reached only
        // by accident and the new one has to name.
        "int a[4]; int *p = a;",
        "int a[4]; int *p = &a[1];",
        // **An array name inside a larger expression is still an address.** The plain `int *p = a`
        // case is answered by the folder before the read rule is consulted, so only this shape
        // exercises the array exemption.
        "int a[4]; int *p = a + 1;",
        // **`&x` stops the walk.** A bare `&y` is answered by the folder; behind a cast it is not,
        // and then only the `AddrOf` arm keeps `y` from counting as a read.
        "int y; int *p = (int *)&y;",
        // **Cast to an integer, where the folder has no answer.** `(int *)&y` is still folded by
        // `addr_of`; `(long)&y` is not, so this is the shape where the `AddrOf` short-circuit is
        // the only thing keeping `y` from counting as a read.
        "int y; long v = (long)&y;",
        "int y; long v = (long)&y + 8;",
        "int x; int *p = (int *)&x + 1;",
        "int f(void); int (*fp)(void) = f;",
        "int y; int *p = &y;",
        "char *s = \"abc\";",
        // Arithmetic, enumerators and `sizeof` are constants.
        "int g = 1 + 2 * 3;",
        "enum E { A = 7 }; int g = A;",
        "int g = (int)(long)0;",
        "int g = sizeof(int);",
        // **gcc folds a `const` object**, even under `-pedantic-errors`.
        "static const int c = 5; int g = c;",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The fourth census: `switch` and its labels.**
///
/// Seven rules, none of them checked. A `switch` controlled by something that is not an integer,
/// a `case` label that is not an integer constant expression, and a `case` or `default` outside
/// any switch at all.
///
/// **`_Generic` was censused alongside and came back clean** — a selector matching no association
/// with no `default`, and two associations naming one type, are both already diagnosed. That is
/// worth recording: it is the first area a census has found already covered, and the reason
/// appears to be that `_Generic` was implemented as a unit with its constraints, where `switch`
/// grew its statement handling and its type rules separately.
///
/// The last two rules cost almost nothing because wave 312 already built what they need: the
/// stack of open switches that `break` and `continue` consult. A `case` outside a switch is the
/// same question as a `break` outside a loop, asked of a different stack.
///
/// Two accepted cases carry the shape of the rest:
///
///   - **`switch(c)` on a `char` is legal**, and so is `switch(u)` on an `unsigned` — the rule is
///     *integer type*, not `int`, and the promotion happens afterwards. A check written against
///     `int` rejects both.
///   - **`case 'a':` and `case 1+1:` are integer constant expressions.** The label rule is about
///     what the expression *folds to*, not how it is spelled, which is the same distinction the
///     duplicate-case rule needed in wave 312.
#[test]
fn the_switch_census() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // The controlling expression must have integer type.
        "int f(double d){ switch(d){ case 1: return 1; } return 0; }",
        "int f(int *p){ switch(p){ case 0: return 1; } return 0; }",
        "struct S { int a; }; int f(struct S s){ switch(s){ case 1: return 1; } return 0; }",
        // A case label must be an integer constant expression.
        "int f(int n, int m){ switch(n){ case m: return 1; } return 0; }",
        "int f(int n){ switch(n){ case 1.5: return 1; } return 0; }",
        // A label needs a switch to belong to.
        "int f(int n){ case 1: return n; }",
        "int f(int n){ default: return n; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **Contract 20: a poisoned controlling expression is one diagnostic, not two.** An
    // undeclared name has been reported where it was used; the switch rule must not add a second
    // complaint about the type it could not determine. Only a count can see this — both are
    // "diagnosed" either way.
    for (src, want) in [
        (
            "int f(void){ switch(nosuch){ case 1: return 1; } return 0; }",
            1,
        ),
        ("int f(void){ switch(nosuch){ default: return 1; } }", 1),
    ] {
        assert_eq!(
            diags(src).len(),
            want,
            "one bad expression is one diagnostic: `{src}` -> {:?}",
            diags(src)
        );
    }

    for good in [
        // **Any integer type**, not `int` — the promotion happens after the rule, not instead
        // of it.
        "int f(char c){ switch(c){ case 1: return 1; } return 0; }",
        "int f(long n){ switch(n){ case 1: return 1; } return 0; }",
        "int f(unsigned u){ switch(u){ case 1u: return 1; } return 0; }",
        "enum E { A, B }; int f(enum E e){ switch(e){ case A: return 1; case B: return 2; } return 0; }",
        // A label is judged by what it folds to.
        "int f(int n){ switch(n){ case 'a': return 1; } return 0; }",
        "int f(int n){ switch(n){ case 1+1: return 1; } return 0; }",
        // A switch with no labels, and one with only a default, are both legal.
        "int f(int n){ switch(n){ } return 0; }",
        "int f(int n){ switch(n){ default: return 1; } }",
        // Wave 319 recorded `switch(*p)` on an incomplete pointee here as deliberately
        // uncovered, because the fault is the dereference rather than the switch. Wave 320 added
        // that rule, so the case now lives in `dereferencing_an_incomplete_pointee_is_rejected`
        // and is diagnosed by the check that names the right thing.
        // `_Generic`, which this census found already correct.
        "int f(int n){ int x = _Generic(n, int: 1, default: 2); return x; }",
        "int f(double d){ int x = _Generic(d, int: 1, default: 2); return x; }",
        "int f(int n){ int x = _Generic(n, int: 1, long: 2, default: 3); return x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Dereferencing a pointer to an incomplete type.**
///
/// The case wave 319 wrote down and declined to report from where it found it: `switch(*p)` on a
/// `struct I *` is rejected by gcc, and the fault is the *dereference*, not the switch. Nothing
/// checked a `Deref` for completeness, so every use of `*p` on an opaque pointer was accepted —
/// including `*p = *q`, which copies an object of unknown size.
///
/// The accepted list is what a pointer to an incomplete type is *for*, and it is most of the
/// reason opaque handles work at all:
///
///   - **Copying, comparing and converting the pointer** never touches the pointee. `p != 0`,
///     `struct I *q = p;` and `(long)p` are the opaque-handle idiom.
///   - **`*p` on a `struct I **` is legal**: what it yields is `struct I *`, a complete pointer
///     type. One level of indirection is one level — the same distinction wave 316's pointee-const
///     rule needed, in the other direction.
///   - **`&gi` on an `extern struct I gi;`** takes an address without needing a size, which is why
///     wave 303 made that declaration legal in the first place.
///
/// `p->m` is here because it is `(*p).m` — the member lookup never gets a chance to fail, since
/// the record it would look in has no members yet, and reporting "no member named `m`" would name
/// the wrong thing.
#[test]
fn dereferencing_an_incomplete_pointee_is_rejected() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "struct I; int f(struct I *p){ switch(*p){ case 1: return 1; } return 0; }",
        "struct I; int f(struct I *p){ (*p); return 0; }",
        "struct I; int f(struct I *p, struct I *q){ *p = *q; return 0; }",
        "struct I; int f(struct I *p){ return p->m; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // The pointer itself may be copied, compared and converted.
        "struct I; int f(struct I *p){ struct I *q = p; return q != 0; }",
        "struct I; int f(struct I *p){ return p != 0; }",
        "struct I; int f(struct I *p){ return (int)(long)p; }",
        "struct I; extern struct I gi; int f(void){ return &gi != 0; }",
        // One level of indirection is one level: `*p` here is `struct I *`, which is complete.
        "struct I; int f(struct I **p){ struct I *q = *p; return q != 0; }",
        // A complete pointee may be dereferenced, copied and read through.
        "struct C { int m; }; int f(struct C *p){ return p->m; }",
        "struct C { int m; }; int f(struct C *p, struct C *q){ *p = *q; return p->m; }",
        "int f(int *p){ return *p; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}
