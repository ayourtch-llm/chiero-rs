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
        // **`return v();` from a `void` function is a constraint violation** (C 6.8.6.4p1), and
        // wave 311 put it in the *accepted* list by checking gcc's default, which is lenient
        // about it. Wave 314 established that this project calibrates to `-pedantic-errors`
        // precisely because half of C's constraint violations are warnings by default; under
        // that setting gcc rejects this, so it belongs here.
        "void v(void); void w(void){ return v(); }",
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
        // Caught by the void-*value* rule, not this one — the same expression, a different fault.
        "int f(void *p){ return *p; }",
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
        // **A `void *` deref is legal**, as a GNU extension: `(*p);` yields a void expression and
        // discarding it is fine. This is why `Ty::Void` is deliberately outside `is_incomplete` —
        // and `return *p;` on the same pointer *is* rejected, by wave 311's void-value rule
        // rather than by this one, which is the division of labour the two rules are meant to
        // have.
        "int f(void *p){ (*p); return 0; }",
        "int f(void *p){ int *q = p; return *q; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A call and a `return` must match the function's type.**
///
/// Three of the nine violations wave 325's ratchet found unchecked, taken together because they
/// are one idea: the function's declared type says how many arguments it takes and whether it
/// yields a value, and neither end was being held to it.
///
/// For this engine the argument rules are not merely diagnostics. A call with too few arguments
/// leaves a parameter reading whatever the frame happened to hold, which is precisely the
/// uninitialised read the memory model exists to report — and the engine would report it against
/// the *callee*, blaming code that is correct.
///
/// The accepted list carries the two exemptions that make the rule survive real C:
///
///   - **A variadic function takes *at least* its named parameters**, so the count is a minimum
///     rather than an equality once `...` is present.
///   - **An old-style declaration says nothing about its parameters**, so `int g();` admits any
///     call — the same empty-list problem wave 313 met from the other side, and the same answer:
///     an empty parameter list is "unspecified", not "none".
#[test]
fn a_call_and_a_return_match_the_functions_type() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "static int g(int a, int b){ return a+b; } int f(void){ return g(1); }",
        "static int g(int a){ return a; } int f(void){ return g(1,2); }",
        "static int g(int a, int b, int c){ return a; } int f(void){ return g(1,2); }",
        // A `return` with a value in a function returning `void`.
        "static void v(void){ return 1; }",
        "static void v(int n){ if (n) return n; return; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Exactly right, and with no parameters at all.
        "static int g(int a, int b){ return a+b; } int f(void){ return g(1,2); }",
        "static int g(void){ return 1; } int f(void){ return g(); }",
        // **Variadic: the named parameters are a minimum**, not an equality.
        "static int g(int a, ...); int f(void){ return g(1); }",
        "static int g(int a, ...); int f(void){ return g(1,2,3); }",
        // **An old-style declaration admits any call** — an empty list is unspecified, not none.
        "static int g(); int f(void){ return g(1,2,3); }",
        "static int g(); int f(void){ return g(); }",
        // `return;` in a void function, and a value in a non-void one.
        "static void v(void){ return; }",
        "static void v(int n){ if (n) return; }",
        "static int g(void){ return 1; }",
        // A void call as a statement is not a `return` with a value.
        "static void v(void){} static void w(void){ v(); return; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Four of the six violations wave 325's ratchet left unchecked.**
///
/// An array assigned to, a struct member declared twice, a parameter named twice, and the address
/// of a `register` object. Each is a check at a site sema already visits, which is why they were
/// grouped: none needs machinery that does not exist.
///
/// **The fifth, a `goto` into a VLA's scope, is not here** — and the discriminator that shows why
/// is: jumping into a block that declares a *non-VLA* is perfectly legal, so the rule is not about
/// jumping into scopes but about which declarations a jump skips. That needs sema to record, per
/// label, whether a variably-modified declaration precedes it in the same block, which nothing
/// tracks today. It stays on the ratchet's queue rather than being approximated.
///
/// The accepted list is where each rule stops:
///
///   - **`a[0] = b[0]` is fine and `a = b` is not.** The rule is about the *array* being assigned,
///     not about arrays appearing in an assignment — and a `struct` containing an array assigns
///     whole, which is how one copies an array in C.
///   - **The same member name in two different structs** is two members, not a duplicate, so the
///     check is per record rather than per translation unit.
///   - **An unnamed parameter is not a name**, so `int g(int, int)` has no duplicate — a check
///     comparing what it finds without excluding absent names would reject every prototype
///     written that way.
///   - **`register` without `&`** is ordinary, and `*&x` on a non-`register` local is too; only
///     the pair is an error.
#[test]
fn four_more_constraint_violations_are_rejected() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(void){ int a[2], b[2]; a = b; return a[0]; }",
        "struct S { int m; int m; };",
        "static int g(int a, int a){ return a; }",
        "int f(void){ register int x = 0; return *&x; }",
        // The same rules in their other spellings.
        "int f(void){ int a[2]; a += 1; return a[0]; }",
        "union U { int m; long m; };",
        "int f(void){ register int x = 0; int *p = &x; return *p; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // An array *element* is assignable; an array is not.
        "int f(void){ int a[2], b[2]; a[0] = b[0]; return a[0]; }",
        // A struct assigns whole, which is how an array is copied in C.
        "struct P { int x, y; }; int f(void){ struct P a = {1,2}, b; b = a; return b.x; }",
        "struct A { int v[2]; }; int f(void){ struct A a = {{1,2}}, b; b = a; return b.v[1]; }",
        // Members are per record, not per translation unit.
        "struct S { int m; }; struct T { int m; };",
        "struct S { int a; int b; };",
        // An anonymous member contributes its own names, and they are distinct.
        "struct S { struct { int a; int b; }; int c; };",
        // Parameters: distinct, unnamed, and variadic.
        "static int g(int a, int b){ return a+b; }",
        "static int g(int, int);",
        "static int g(int a, ...){ return a; }",
        // Two functions may each have a parameter named `a`.
        "static int g(int a){ return a; } static int h(int a){ return a; }",
        // **A parameter may shadow a file-scope name.** Checking the list against `values` — which
        // by then holds the whole file scope — makes this a duplicate, and it is the shape that
        // separates "this list" from "everything in scope".
        "static int a; static int g(int a){ return a; }",
        // `register` without an address, and an address without `register`.
        "int f(void){ register int x = 0; return x; }",
        "int f(void){ int x = 0; return *&x; }",
        // **A block may shadow a `register` object with an ordinary one**, and the inner `x` has
        // an address. Without clearing the name on the inner declaration the shadow inherits the
        // outer object's storage class — the same rule `read_only` and `read_only_pointee` each
        // needed, for the third time.
        "int f(void){ register int x=0; { int x=1; return *&x; } }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A `goto` may not jump into the scope of a variably-modified identifier** (C 6.8.6.1p1).
///
/// The last violation on wave 325's ratchet that does not need qualified types, and it is not the
/// rule its name suggests. The accepted list is what pins that down:
///
///   - **`goto skip; { int a[2]; skip: … }` is legal** — a non-VLA declaration is not
///     variably-modified, so jumping into its block is fine. The rule is about the *declaration*,
///     not the block.
///   - **`goto skip; int a[n]; skip: …` is illegal with no block at all**, in one flat function
///     body. So it is not about nesting either.
///   - **A jump from *after* the declaration is legal**, in the same block or a nested one,
///     because the jump does not cross it.
///
/// What remains, once those three are accounted for, is a rule about *position*: the label is in
/// the identifier's scope — from its declaration to the end of its block — and the `goto` is not.
/// That is what the fix models, with a stack of open variably-modified scopes: a label records
/// which are active where it sits, a `goto` records the same, and the jump is illegal when the
/// label's set is not contained in the `goto`'s.
#[test]
fn a_goto_may_not_jump_into_a_variably_modified_scope() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(int n){ goto skip; { int a[n]; skip: return 0; } }",
        "int f(int n){ goto skip; { int a[n]; skip: return a[0]; } }",
        // No block at all: the jump crosses the declaration in one flat body.
        "int f(int n){ goto skip; int a[n]; skip: return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // A fixed-length array is not variably modified.
        "int f(int n){ goto skip; { int a[2]; skip: return 0; } }",
        // Jumping from after the declaration does not cross it.
        "int f(int n){ { int a[n]; a[0]=1; goto skip; skip: return a[0]; } }",
        "int f(int n){ { int a[n]; skip: a[0]=1; if(n) goto skip; return a[0]; } }",
        "int f(int n){ int a[n]; goto skip; skip: return a[0]; }",
        // The label precedes the declaration, so it is not in its scope.
        "int f(int n){ goto skip; { skip: ; int a[n]; return a[0]; } }",
        // The label is outside the block entirely; the scope has ended by then.
        "int f(int n){ if(n) goto out; { int a[n]; a[0]=1; } out: return 0; }",
        // An ordinary backward jump, with no variably-modified anything.
        "int f(void){ int i=0; again: i++; if(i<3) goto again; return i; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Type qualifiers are part of the type** (C 6.7.3, 6.5.16.1p1), which is the last unfinished
/// item on wave 325's ratchet and the largest one in the project.
///
/// The boundary below is gcc's under `-pedantic-errors`, probed rather than reasoned about,
/// because three of these are places where C's rule surprises people:
///
///   - **`const int *const *cpp = pp;` is illegal in C**, though the analogous form is legal in
///     C++. C 6.5.16.1 compares the pointed-to types for compatibility *ignoring* qualifiers only
///     at the outermost level, so any qualifier difference below the first pointer is a mismatch —
///     even one that adds `const`.
///   - **`typedef int *ip; const ip p;` is a `const` pointer, not a pointer to `const`.** The
///     qualifier applies to the typedef'd type as a whole, so `*p = 1` is legal.
///   - **A member of a `const` struct is `const`**, so `&s->m` is a `const int *` even though `m`
///     was declared plain `int`.
///
/// The legal half is the larger half on purpose. Twenty-three of these thirty already pass, and a
/// change to how qualifiers are represented can break any of them; the census method's rule is
/// that the legal half is where the damage shows up.
#[test]
fn a_qualifier_is_part_of_the_type() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Discarding `const` from a pointee: by assignment, through `&`, through an argument,
        // and through an array's decay.
        "int f(const int *cp){ int *p = cp; return *p; }",
        "int f(void){ const int x = 0; int *p = &x; return *p; }",
        "void g(int *); int f(const int *cp){ g(cp); return 0; }",
        "int f(void){ const int a[3] = {1,2,3}; int *p = a; return *p; }",
        // **`const arr` on an array typedef qualifies the *element*** (C 6.7.3p9), which is the
        // only spelling that reaches that rule: in `const int a[3]` the parser has already put
        // the qualifier on the element, so the rule looks dead until a typedef hides the array.
        // Mutation found the branch unobserved and this is what it was missing.
        "typedef int arr[3]; int f(void){ const arr a = {1,2,3}; int *p = a; return *p; }",
        "typedef int arr[3]; int f(void){ const arr a = {1,2,3}; a[0]=1; return a[0]; }",
        // `volatile` is discarded the same way. A rule written only for `const` misses it.
        "int f(volatile int *vp){ int *p = vp; return *p; }",
        // **`void *` does not exempt a pointer from the qualifier rule.** It converts to and from
        // any object pointer without a cast, and that permission is about the *pointee's shape*,
        // not its qualifiers — so `void *p = cp;` discards `const` exactly as `int *p = cp;`
        // does. Mutation found this: exempting `void *` from the qualifier check changed nothing
        // any test could see, because the rest of the fixture never routes a qualifier through
        // one.
        "int f(const void *cp){ void *p = cp; return p != 0; }",
        "int f(const int *cp){ void *p = cp; return p != 0; }",
        "int f(volatile void *vp){ void *p = vp; return p != 0; }",
        // Below the outermost pointer, *any* qualifier difference is a mismatch — including one
        // that only adds `const`, which is what C++ programmers expect to be allowed.
        "int f(const int **cpp){ int **pp = cpp; return **pp; }",
        "int f(int **pp){ const int **cpp = pp; return **cpp; }",
        "int f(int **pp){ const int *const *cpp = pp; return **cpp; }",
        // A member reached through a `const` pointer is itself `const`.
        "struct S { int m; }; int f(const struct S *s){ int *p = &s->m; return *p; }",
        "struct S { const int m; }; int f(struct S *s){ s->m = 1; return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // *Adding* a qualifier at the outermost pointee is always allowed.
        "int f(int *p){ const int *cp = p; return *cp; }",
        "int f(int *p){ volatile int *vp = p; return *vp; }",
        "int f(const int *cp){ const int *q = cp; return *q; }",
        "int f(void){ int x = 0; const int *cp = &x; return *cp; }",
        "int f(void){ const int x = 0; const int *cp = &x; return *cp; }",
        "int f(void){ const int k = 1; const int *p = &k; return *p; }",
        "void g(const int *); int f(int *p){ g(p); return 0; }",
        // ...and adding one *through* `void *` is legal in both directions.
        "int f(void *p){ const void *cp = p; return cp != 0; }",
        "int f(const void *cp){ const void *p = cp; return p != 0; }",
        // Reading a qualified *object* yields an unqualified value, so these are ordinary
        // arithmetic and ordinary initialization.
        "int f(void){ const int k = 1; int x = k; return x; }",
        "int f(void){ const int k = 1; return k + 1; }",
        "int f(void){ volatile int v = 1; int x = v; return x; }",
        "int f(const int *cp){ return *cp; }",
        "struct S { const int m; }; int f(void){ struct S s = {1}; return s.m; }",
        "struct S { int m; }; int f(const struct S *s){ return s->m; }",
        "int f(void){ const int a[3] = {1,2,3}; return a[0]; }",
        "int f(void){ const int a[3] = {1,2,3}; const int *p = a; return *p; }",
        "typedef int arr[3]; int f(void){ const arr a = {1,2,3}; const int *p = a; return *p; }",
        "typedef int arr[3]; int f(void){ const arr a = {1,2,3}; return a[0]; }",
        // A cast says so explicitly, and a comparison does not convert either operand.
        "int *f(const int *cp){ return (int *)cp; }",
        "int f(const int *cp){ return (int)(cp == 0); }",
        "int f(const int *cp, int *p){ return cp == p; }",
        // A qualifier on a typedef applies to the whole type: `const ip` is `int *const`.
        "typedef const int ci; int f(void){ ci k = 1; int x = k; return x; }",
        "typedef int *ip; int f(void){ int x=0; const ip p = &x; *p = 1; return x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Five of C 6.5's operator constraints**, from wave 329's census — the first run of the method
/// since wave 325's queue emptied. Thirty programs, twenty-one of them legal, verdicts from gcc
/// under `-pedantic-errors`: sema was silent on all thirty and gcc rejected nine.
///
/// The five here are the operator half. Each accepted case is a discriminator for a rule that
/// would otherwise be written too broadly:
///
///   - **`(x)++` is legal and `(x+1)++` is not**, so the increment rule is about lvalues rather
///     than about parentheses — which costs nothing here only because the AST discards them.
///   - **`(int){1}++` is legal**: a compound literal is an lvalue, and it is spelled as a cast in
///     this AST, so a rule that rejects casts rejects it.
///   - **`A++` on an enumeration constant is not an lvalue** even though it is spelled as a plain
///     identifier, which no test of the expression's *kind* can see.
///   - **`~c` on a `char` is legal**, and a narrow integer is the shape most likely to be caught
///     by a rule written about widths rather than about categories.
///   - **`*p;` as a statement is legal with `p` a `void *`**, and `(void)*p` too. The `void` rule
///     is about *using the value*, not about producing it.
#[test]
fn the_operator_constraints_of_c_6_5() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // C 6.5.2.4p1 and 6.5.3.1p1: the operand of `++`/`--` is a modifiable lvalue.
        "int f(void){ int x = 1; return x++++; }",
        "int f(void){ int x = 1; return ++++x; }",
        "int f(void){ int x = 1; return (x+1)++; }",
        "int f(void){ int x = 1; return f()++; }",
        "int f(void){ enum E { A }; return A++; }",
        "int f(void){ int x=1,y=2; return (x,y)++; }",
        // C 6.5.3.4p1: `sizeof` is not applied to a function type.
        "void g(void); int f(void){ return sizeof(g); }",
        // C 6.5.3.3p1: unary `+` and `-` take an arithmetic operand.
        "int f(void){ int x = 1; return -&x != 0; }",
        "int f(void){ int x = 1; return +&x != 0; }",
        // C 6.5.3.3p4: `~` takes an integer operand.
        "int f(int *p){ return (int)~p; }",
        "int f(double d){ return (int)~d; }",
        // C 6.3.2.2: a `void` value cannot be used.
        "int f(void){ void *p = 0; return *p != 0; }",
        "void v(void); int f(void){ return v() != 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Lvalues that must keep incrementing.
        "int f(void){ int x = 1; return (x)++; }",
        "int f(void){ int a[2]; return a[0]++; }",
        "int f(void){ int x=1, *p=&x; return (*p)++; }",
        "struct S { int m; }; int f(struct S *s){ return s->m++; }",
        "int f(void){ return (int){1}++; }",
        "struct S { int m; }; int f(void){ return (struct S){1}.m++; }",
        "int f(void){ int x=1; return _Generic(1,int:x)++; }",
        // `sizeof` of a *pointer* to a function, and of a function pointer type, are both fine.
        "void g(void); int f(void){ return sizeof(&g); }",
        "void g(void); int f(void){ return sizeof(void(*)(void)); }",
        "int f(void){ int a[3]; return sizeof(a); }",
        // Arithmetic operands, including one reached through a pointer.
        "int f(int *p){ return -*p; }",
        "int f(double d){ return (int)-d; }",
        "int f(int n){ return ~n; }",
        // `~` on a narrow integer. **Whether this is checked before or after promotion is
        // measured equivalent** — `char` is an integer type either way — so this case pins the
        // category rule, not an ordering.
        "int f(char c){ return ~c; }",
        // A `void` value *produced* and discarded is fine; only *using* it is not.
        "int f(void){ void *p = 0; *p; return 0; }",
        "int f(void){ void *p = 0; (void)*p; return 0; }",
        "void v(void); int f(void){ v(); return 0; }",
        "int f(void){ void *p = 0; return p != 0; }",
        "int f(int *p){ return *p != 0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Four of C 6.7's declaration constraints**, three named by wave 329's census as the queue it
/// left open and a fourth found while probing their boundaries: a `struct` tag may be defined
/// twice with nothing said.
///
/// The accepted half is where the work is. Each of these four rules has a legal neighbour that a
/// rule written one word too broadly rejects:
///
///   - **`_Thread_local static` is legal** (C 6.7.1p2 exempts it by name), so "at most one storage
///     class" is false as written — it is "at most one of `extern`, `static`, `auto`, `register`".
///     `inline` is a *function* specifier and combines with anything.
///   - **`int a[k]` with a `const int k` is legal inside a function and illegal at file scope.**
///     `const` does not make a constant expression in C, so this is a VLA either way; what changes
///     is the storage duration. A `static` local is illegal for the same reason a file-scope one
///     is, and a *parameter* is legal because its array is a pointer.
///   - **An enumerator may be shadowed in an inner scope** but not redeclared in its own — and
///     "its own" spans two different enums, because enumerators are ordinary identifiers.
///   - **A tag may be *declared* repeatedly** (`struct S;` after `struct S { ... };` is how
///     forward declarations work) but **defined** only once.
#[test]
fn the_declaration_constraints_of_c_6_7() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // C 6.7.1p2: at most one of `extern`, `static`, `auto`, `register`.
        "int f(void){ static extern int x; return x; }",
        "int f(void){ static register int x = 1; return x; }",
        "int f(void){ auto static int x = 1; return x; }",
        "static extern int g(void);",
        // C 6.7.6.2p2: a variably-modified declarator needs automatic storage duration.
        "const int k = 1; int a[k];",
        "const int k = 1; int f(void){ static int a[k]; return a[0]; }",
        "const int k = 1; struct S { int a[k]; };",
        // C 6.7.2.2: an enumerator is an ordinary identifier in its scope.
        "enum E { A = 1, A = 2 }; int f(void){ return A; }",
        "enum E { A = 1 }; enum F { A = 2 }; int f(void){ return A; }",
        "int f(void){ enum E { A = 1, A = 2 }; return A; }",
        // C 6.7.2.3p1: a tag is defined once.
        "struct S { int m; }; struct S { int m; };",
        "union U { int m; }; union U { int m; };",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // One storage class, with qualifiers and function specifiers alongside.
        "static int x = 1; int f(void){ return x; }",
        "int f(void){ static const int x = 1; return x; }",
        "int f(void){ register const int x = 1; return x; }",
        "int f(void){ extern int x; return x; }",
        "static inline int g(void){ return 1; } int f(void){ return g(); }",
        // **`_Thread_local` is exempt**, in both orders.
        "_Thread_local static int x; int f(void){ return x; }",
        "static _Thread_local int x; int f(void){ return x; }",
        // A variably-modified declarator with automatic storage duration, and as a parameter.
        "const int k = 1; int f(void){ int a[k]; a[0]=1; return a[0]; }",
        "const int k = 1; int f(int a[k]){ return a[0]; }",
        // ...and the array lengths that are *not* variably modified.
        "enum { K = 3 }; int a[K];",
        "int a[3];",
        "int a[sizeof(int)];",
        // Enumerators: distinct names, a shadowing inner scope, and one enum defined from another.
        "enum E { A = 1, B = 2 }; int f(void){ return A + B; }",
        "enum E { A, B, C }; int f(void){ return C; }",
        "enum E { A = 1 }; int f(void){ enum F { A = 2 }; return A; }",
        // **A name freed by leaving a scope is available again in the enclosing one.** Mutation
        // found this missing: a *sibling* pair does not exercise the removal at all, because the
        // second sibling's mark already starts past the first's leftovers. Only declaring in the
        // enclosing scope *after* an inner one has closed reads the stale entries. Wave 326's
        // rule, and the second time this project has written the wrong shadowing case first.
        "int f(void){ { enum E1 { A }; } enum E2 { A = 2 }; return A; }",
        "int f(void){ { struct S { int a; }; } struct S { int b; } s = {1}; return s.b; }",
        "int f(void){ if (1) { enum E1 { A }; } enum E2 { A = 3 }; return A; }",
        "enum E { A = 1 }; enum E2 { B = A }; int f(void){ return B; }",
        // A tag declared repeatedly, defined once — which is what a forward declaration is.
        "struct S; struct S { int m; }; struct S; int f(struct S *p){ return p->m; }",
        "struct S { int m; }; int f(void){ struct S { int q; } s = {1}; return s.q; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Two more constraints, from wave 331's census** over C 6.7.9's initializers, 6.9's external
/// definitions and 6.5.2.2's calls. Fifty-one programs across two runs; sema was silent on every
/// legal one, so this is misses only.
///
/// The census's own bookkeeping was the first thing it corrected. §9 had flagged the ratchet's
/// argument-count rows as suspect — they are fine, and reject exactly what they claim — and a
/// nested function definition, which the probe reported as a *crash*, turns out to be rejected by
/// the **parser** with a proper diagnostic. The probe harness asserts a clean parse, so a parser
/// rejection arrives as a panic. That is evidence about the probe, not about the engine.
///
/// What the two rules here turn on:
///
///   - **`enum { A = 1 };` declares something and `struct { int m; };` does not.** Both are a
///     declaration with no declarator, so the rule cannot be "no declarator"; an anonymous
///     enumeration declares its enumerators, while an anonymous structure declares nothing at all.
///   - **An anonymous *member* is a different thing entirely** and stays legal — C11's anonymous
///     struct and union members are how a tagless aggregate is usefully written.
///   - **A structure may be initialized from an expression, just not from any expression.**
///     `struct S s = s2;` and `= f();` are ordinary copies; what `struct S s = 1;` lacks is a type
///     that could be copied, and `assignable` never looked because neither side is a pointer.
#[test]
fn a_declaration_declares_something_and_a_record_is_not_a_scalar() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // C 6.7p2: a declaration declares a declarator, a tag, or the members of an enumeration.
        "int;",
        "const int;",
        "struct { int m; };",
        "union { int m; };",
        "int f(void){ int; return 0; }",
        "int f(void){ struct { int m; }; return 0; }",
        // C 6.7.9p13: a structure or union is initialized by a braced list or by a value of its
        // own type — not by a scalar.
        "struct S { int a; }; struct S s = 1;",
        "struct S { int a; }; int f(void){ struct S s = 1; return s.a; }",
        // C 6.7.2.1p18: a flexible array member is the last member (wave 338's census).
        "struct S { int a[]; int b; };",
        "struct S { int n; int a[]; int b; };",
        "union U { int a; }; union U u = 1;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Declarations that *do* declare something with no declarator.
        "enum { A = 1 }; int f(void){ return A; }",
        "struct S { int m; }; int f(struct S *p){ return p->m; }",
        "struct S; int f(struct S *p){ return p != 0; }",
        "typedef int T; int f(void){ T v = 1; return v; }",
        // **An anonymous member is not an empty declaration.** Same spelling, different rule.
        "struct S { int a; struct { int b; }; }; int f(struct S *s){ return s->b; }",
        "struct S { int a; union { int b; float c; }; }; int f(struct S *s){ return s->b; }",
        // A record initialized from a braced list, and from a value of its own type.
        "struct S { int a; }; struct S s = {1}; int f(void){ return s.a; }",
        // ...and one that *is* last stays legal, alone or after other members.
        "struct S { int n; int a[]; }; int f(struct S *s){ return s->n; }",
        "struct S { int n; char c; int a[]; }; int f(struct S *s){ return s->n; }",
        "struct S { int a; }; int f(void){ struct S s2 = {1}; struct S s = s2; return s.a; }",
        "struct S { int a; }; struct S g(void); int f(void){ struct S s = g(); return s.a; }",
        "union U { int a; char b; }; union U u = {1}; int f(void){ return u.a; }",
        "union U { int a; char b; }; int f(void){ union U u2 = {1}; union U u = u2; return u.a; }",
        // **A qualifier does not stop a record being copied**, in either direction — reading a
        // `const` object yields an unqualified value, and writing an unqualified one into a
        // `const` object is what initializing it means. Mutation found this: comparing the two
        // record types with their qualifiers survived, because nothing here copied a `const` one.
        "struct S { int a; }; int f(const struct S *p){ struct S s = *p; return s.a; }",
        "struct S { int a; }; int f(void){ const struct S c = {1}; struct S s = c; return s.a; }",
        "struct S { int a; }; int f(void){ struct S m = {1}; const struct S c = m; return c.a; }",
        "struct S { int a; }; void g(struct S); int f(const struct S *p){ g(*p); return 0; }",
        // ...and the scalar initializations that must stay ordinary.
        "int x = 1; int f(void){ return x; }",
        "int f(void){ int a[2] = {1,2}; return a[0]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`f()` and `f(void)` are different declarations**, and sema has never been able to tell them
/// apart — `parameter_list` returns the same empty list for both, which `types_conflict` records
/// as a limit in its own comment.
///
/// That one gap hides three rules at once, and the accepted half shows why none of them can be
/// approximated by "an empty parameter list means no parameters":
///
///   - **`int g(); g(1,2,3);` is legal.** An empty list in a *declaration* means the parameters
///     are unspecified, so no call can be wrong. Only `(void)` promises there are none.
///   - **`int f(); int f(void);` is legal in both orders.** An unprototyped declaration composes
///     with a prototyped one rather than conflicting with it — which is why the conflict rule
///     cannot simply compare parameter lists.
///   - **`static int g(){ return 1; } g(1);` is legal**, though `static int g(void){...}` makes
///     the same call an error. An old-style *definition* with an empty identifier list still
///     specifies nothing.
///
/// The fourth rule here needs no flag and came from the same probe: `void` may be a parameter list
/// but not a parameter.
#[test]
fn a_prototype_promises_what_an_empty_list_does_not() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // C 6.5.2.2p2: a call to a prototyped function has as many arguments as parameters.
        "int g(void); int f(void){ return g(1); }",
        "static int g(void){ return 1; } int f(void){ return g(1); }",
        // C 6.7.6.3p15: `(void)` and `(int)` are not compatible, in either order.
        "int f(void); int f(int);",
        "int f(int); int f(void);",
        // C 6.7.6.3p10: `void` may be the whole parameter list, or a parameter, but not one of
        // several.
        "int g(void, int);",
        "int g(int, void);",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // A prototyped call with the right number of arguments, and an unprototyped one with any.
        "int g(void); int f(void){ return g(); }",
        "int g(); int f(void){ return g(1,2,3); }",
        "int g(); int f(void){ return g(); }",
        "static int g(){ return 1; } int f(void){ return g(1); }",
        // **A K&R definition with *named* parameters is still not a prototype.** gcc accepts a
        // call with any count against `g(a) int a;`, even under `-pedantic-errors`. This is a
        // different code path from `g()` above — that one takes the empty-list branch and never
        // reaches the identifier-list branch at all — and mutation found the branch unobserved
        // because of it.
        "int g(a) int a; { return a; } int f(void){ return g(1,2); }",
        "int g(a) int a; { return a; } int f(void){ return g(); }",
        "int g(a, b) int a; int b; { return a+b; } int f(void){ return g(1); }",
        "void g(void); int f(void){ g(); return 0; }",
        // An empty list composes with a prototype rather than conflicting with it.
        "int f(); int f(int x){ return x; }",
        "int f(); int f(void);",
        "int f(void); int f();",
        "int f(void); int f(void){ return 1; }",
        "int f(int); int f(int x){ return x; }",
        // A function *pointer* takes either spelling, and a call through one is not this rule.
        "int g(void); int f(void){ int (*p)(void) = g; return p(); }",
        "int g(void); int f(void){ int (*p)() = g; return p(); }",
        "int g(); int f(void){ int (*p)(void) = g; return p(); }",
        "int g(void); int f(void){ return (*g)(); }",
        // A function type reached through a typedef keeps its prototype.
        "typedef int T(void); T g; int f(void){ return g(); }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`typedef` is a storage-class specifier** (C 6.7.1p1), so it may not accompany another — the
/// item §9 has carried since wave 330, blocked because `DeclKind::Typedef` has no `Storage`.
///
/// **`_Thread_local` is counted here and exempt for an object**, which is the discriminator worth
/// having: 6.7.1p2 lets it accompany `static` or `extern` and nothing else, so
/// `_Thread_local static int x;` is legal and `typedef _Thread_local int T;` is not. A rule that
/// reuses the object-side counter unchanged gets the second one wrong.
#[test]
fn a_typedef_takes_no_other_storage_class() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "typedef static int T;",
        "typedef extern int T;",
        "typedef register int T;",
        "typedef _Thread_local int T;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        "typedef int T; T v = 1;",
        "typedef const int CT; CT k = 1;",
        "typedef int F(void); F *fp;",
        // ...and the object-side rule keeps its exemption.
        "_Thread_local static int x; int f(void){ return x; }",
        "static _Thread_local int y; int f(void){ return y; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **C 6.4.4's constraints on numeric constants** — wave 335's census over the lexer, and the
/// first run against a crate `chiero-lex` shares with sema.
///
/// A *pp-number* is deliberately permissive: `1z` and `018` are well-formed preprocessing tokens
/// and only stop being valid when something asks them for a value. So these rules live where that
/// happens, and the census found the consequence of their absence is **not a missing diagnostic
/// but a wrong answer** — when neither the integer nor the floating parser accepts a literal, the
/// typing arm falls through to `FloatKind::F64`, so `int x = 018;` types as `double`.
///
/// The legal half carries the GNU extension this project accepts on purpose, because a rule
/// written from C11 alone rejects it: **`0b101` is a binary constant**, refused under
/// `-pedantic-errors` and accepted in the GNU mode the corpus is compiled with.
///
/// **C23's digit separators are a second such divergence, deliberately asserted neither way.**
/// `parse_int_literal` strips `'` on purpose; gcc rejects `1'000` under `-std=c11` even without
/// `-pedantic-errors` and accepts it under `-std=c2x`. Calling it legal would assert a C11 fact
/// that is false, and calling it illegal would demand a rule that removes working support. It is
/// recorded here as a declared divergence instead.
#[test]
fn a_numeric_constant_is_constrained() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let expr = |e: &str| format!("int f(void){{ return (int)({e}); }}");

    for bad in [
        // 6.4.4.1p1: an integer suffix is `u`, `l`, `ll` or a `u` with one of those — nothing
        // else, and `ll` does not mix case.
        "1z",
        "1i",
        "1uu",
        "1lll",
        "1Ll",
        "1lL",
        "0o7",
        // 6.4.4.2p1: a floating suffix is `f` or `l`.
        "1.0z",
        "1.0u",
        "1.0ll",
        // 6.4.4.2p1: an exponent has digits, and a hexadecimal float has an exponent.
        "1e",
        "0x1p",
        "0x1.8",
        // 6.4.4.1p1: `0x` has digits, and an octal constant has octal ones.
        "0x",
        "018",
        "08",
        // 6.4.4.1p5: the value fits some integer type.
        "99999999999999999999999",
    ] {
        let src = expr(bad);
        assert!(!diags(&src).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Every valid integer suffix, in both cases.
        "1",
        "1u",
        "1U",
        "1l",
        "1L",
        "1ul",
        "1lu",
        "1LL",
        "1ll",
        "1ull",
        "1llu",
        "1ULL",
        // Every valid floating one, and the exponent forms.
        "1.0",
        "1.0f",
        "1.0F",
        "1.0l",
        "1.0L",
        "1e3",
        "1E3",
        "1.e3",
        ".5",
        "1e-3",
        // Hexadecimal, integer and floating.
        "0x1f",
        "0X1F",
        "0x1p3",
        "0x1P3f",
        "0x1.8p3",
        // Octal, including the one-digit case that is not a prefix at all.
        "0",
        "07",
        "0777",
        // **A binary constant is a GNU extension this project accepts on purpose** — gcc refuses
        // `0b101` under `-pedantic-errors` and accepts it in the GNU mode the corpus uses.
        "0b101",
        // **gcc's extended floating suffixes, which the corpus made non-optional.** Every VPP
        // header reaches a `0.0f16`, so a rule with C11's two suffixes alone reports a false
        // positive on all twenty corpus seeds — which is exactly what it did before these were
        // added. Recognising them is not the same as typing them: `0.0f16` is still `double`
        // here, a gap this census surfaced and §9 records.
        "0.0f16",
        "0.0F16",
        "0.0f32",
        "0.0f64",
        "0.0f128",
        "0.0f32x",
        "0.0f64x",
        "0.0bf16",
        "0.0q",
        "0.0w",
        // The largest values that do fit.
        "9223372036854775807",
        "18446744073709551615u",
    ] {
        let src = expr(good);
        assert!(
            diags(&src).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(&src)
        );
    }
}

/// **C 6.4.4.4's escape-sequence constraints**, the four rows wave 335's census left and §9 has
/// carried since — blocked because `strlit.rs` returns values and has no way to report.
///
/// Two things make this more than a table of characters:
///
///   - **The range depends on the element width.** `"\x1FF"` is a constraint violation and
///     `L"\x1FF"` is perfectly legal, because the limit is the width of one element and the
///     prefix decides that. A rule written against `unsigned char` rejects correct wide strings.
///   - **`\e` is a GNU extension this project accepts on purpose**, exactly like `0b101` and the
///     extended floating suffixes. gcc refuses it under `-pedantic-errors` and accepts it in the
///     mode the corpus is compiled with, and `string_units` has decoded it since it was written.
#[test]
fn an_escape_sequence_is_constrained() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // 6.4.4.4p9: an empty character constant has no value to have.
        "int f(void){ return ''; }",
        // 6.4.4.4p1: the escape sequences are a closed set.
        "const char *s = \"\\q\";",
        "int f(void){ return '\\q'; }",
        // 6.4.4.4p1: `\x` is followed by at least one hexadecimal digit.
        "const char *s = \"\\x\";",
        // 6.4.4.4p9: the value fits one element of the string.
        "const char *s = \"\\777\";",
        "const char *s = \"\\400\";",
        "const char *s = \"\\x100\";",
        "const char *s = \"\\x1FF\";",
        // **A character constant is bounded by its *element*, not by the `int` it has as a
        // type.** `'\\x100'` is a violation though the constant's type is 32 bits wide, which is
        // why the width comes from `char_element`. Mutation found this: checking at `int_bits`
        // survived, because every character case in the fixture fitted eight bits anyway.
        "int f(void){ return '\\400'; }",
        "int f(void){ return '\\x100'; }",
        // 6.4.3p1: a universal character name takes exactly four or eight digits.
        "const char *s = \"\\u41\";",
        "const char *s = \"\\U0000e9\";",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Every escape C defines, and the one gcc adds.
        "const char *s = \"\\n\\t\\r\\f\\v\\b\\a\\?\\\\\\\"\\'\";",
        "const char *s = \"\\e\";",
        // The largest values that fit one narrow element.
        "const char *s = \"\\377\";",
        "const char *s = \"\\xFF\";",
        "int f(void){ return '\\377'; }",
        "int f(void){ return '\\xFF'; }",
        // ...and the same escape in a *wide* character constant, where it fits.
        "int f(void){ return (int)L'\\x1FF'; }",
        // **The same escapes in a wide string, where they do fit.** This is the pair that makes
        // the rule about width rather than about 255. Written through `sizeof` so the fixture
        // needs no `wchar_t` — the sema harness has no include loader, and a typedef of my own
        // would be asserting the width rather than asking for it.
        "int f(void){ return (int)sizeof(L\"\\x1FF\"); }",
        "int f(void){ return (int)sizeof(L\"\\777\"); }",
        "int f(void){ return (int)sizeof(u\"\\x1FF\"); }",
        "int f(void){ return (int)sizeof(U\"\\x1FFFF\"); }",
        // Well-formed universal character names, both lengths, and a literal non-ASCII character.
        "const char *s = \"\\u00e9\";",
        "const char *s = \"\\U000000e9\";",
        "const char *s = \"é\";",
        // **A character above 255 in a *narrow* string is encoded, not out of range.** `€` is
        // U+20AC and becomes three UTF-8 bytes; the range rule is about numeric *escapes*, which
        // name one element directly. Mutation found this: extending the check to `StrUnit::Char`
        // survived, because every other non-ASCII case here is under 256.
        "const char *s = \"€\";",
        "const char *s = \"\\u20ac\";",
        // Ordinary literals that must not be disturbed.
        "const char *s = \"\";",
        "const char *s = \"it's\";",
        "int f(void){ return 'a'; }",
        "int f(void){ return '\\''; }",
        "int f(void){ return '\\\\'; }",
        "int f(void){ return 'ab'; }",
        "const char *s = \"\\0\\1\\01\\001\";",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **C 6.5.3.2's address-of and indirection constraints, and two neighbours** — wave 339's census,
/// the first chosen by *subject* rather than by crate now that every crate has a constraint list.
///
/// Twelve legal shapes, all already silent; five misses, and **three cases that are caught with
/// the wrong sentence**. `*x` on an `int` reports "dereference of a pointer to an incomplete
/// type", which is a true statement about a poisoned pointee and a false one about the program:
/// the operand is not a pointer at all. 023 §9 asks for a report a person can act on, and that one
/// sends them looking for a missing struct definition.
///
/// The accepted half is what keeps each rule narrow:
///
///   - **`&*p` and `&a[1]` are legal** — C 6.5.3.2p1 names the result of `*` and of `[]`
///     explicitly, so a rule about "lvalues" has to admit them.
///   - **`&g` on a function is legal**, and a function is not an object.
///   - **`return;` is legal in a `void` function** and nowhere else, which is the mirror of the
///     rule wave 311 added for a value in a `void` function.
///
/// The census's sixth row — `int x(void) = 1;` — is in `chiero-parse`'s fixture instead. The
/// parser's `DeclKind::Func` has no room for an initializer, so it was parsing the `= 1` and
/// **discarding it**; the check has to be where the initializer still exists.
#[test]
fn taking_an_address_and_dereferencing_are_constrained() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // 6.5.3.2p1: the operand of `&` is an lvalue, a function designator, or a `[]`/`*` result.
        "int f(void){ return *&(1+2); }",
        "int f(void){ return *&5; }",
        "int f(void){ int x = 1; return *&(x + 0); }",
        // 6.5.3.2p1: ...and not a bit-field.
        "struct S { int b : 3; }; int f(struct S *s){ int *p = &s->b; return *p; }",
        "struct S { int b : 3; }; int f(void){ struct S s = {1}; int *p = &s.b; return *p; }",
        // 6.5.3.2p2: the operand of `*` has pointer type.
        "int f(void){ int x = 1; return *x; }",
        "int f(void){ double d = 1; return *d; }",
        "struct S { int m; }; int f(struct S s){ return (*s).m; }",
        // 6.8.6.4p1: `return;` needs a `void` function.
        "int f(void){ return; }",
        "double g(void){ return; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **The three wrong sentences.** Catching these was never the problem; saying why was.
    for (src, want) in [
        ("int f(void){ int x = 1; return *x; }", "int"),
        ("int f(void){ double d = 1; return *d; }", "double"),
        (
            "struct S { int m; }; int f(struct S s){ return (*s).m; }",
            "struct",
        ),
    ] {
        let d = diags(src);
        assert!(
            d.iter().any(|m| m.contains("not a pointer")),
            "`{src}` should say the operand is not a pointer, not blame an incomplete type \
             (operand is a {want}): {d:?}"
        );
    }

    // **Contract 20: a poisoned operand is one report, not two.** `*nope` used to name the
    // undeclared identifier *and* claim its pointee was incomplete — a second sentence about a
    // type this code invented. Mutation found the `Ty::Error` arm unobserved without this.
    for src in [
        "int f(void){ return *nope; }",
        "int f(void){ return *nope + *nope2; }",
    ] {
        let d = diags(src);
        assert!(
            d.iter().all(|m| m.contains("not declared")),
            "`{src}` should report only the undeclared name: {d:?}"
        );
    }

    for good in [
        // Every operand `&` explicitly accepts.
        "int f(void){ int x = 1; int *p = &x; return *p; }",
        "int f(void){ int a[2] = {1,2}; int *p = &a[1]; return *p; }",
        "struct S { int m; }; int f(struct S *s){ int *p = &s->m; return *p; }",
        "int g(void); int f(void){ int (*p)(void) = &g; return p != 0; }",
        "int f(int *p){ int *q = &*p; return q != 0; }",
        "int f(void){ const int k = 1; const int *p = &k; return *p; }",
        "int f(void){ static int s = 1; int *p = &s; return *p; }",
        // A *named* member beside a bit-field is still addressable.
        "struct S { int b : 3; int m; }; int f(struct S *s){ int *p = &s->m; return *p; }",
        // Indirection through everything that is a pointer, including a decayed array.
        "int f(int *p){ return *p; }",
        "int f(void){ int a[2] = {1,2}; return *a; }",
        "int f(int **pp){ return **pp; }",
        "int f(void){ int x = 1; int *p = &x; return *(p + 0); }",
        // `return;` where it belongs, and a value where that belongs.
        "void g(void){ return; } int f(void){ g(); return 0; }",
        "void g(void){ if (1) return; } int f(void){ g(); return 0; }",
        "int f(void){ return 0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The audit §9 asked for: does a diagnostic name the mistake it found?**
///
/// A ratchet asks only whether a program was rejected, so a green row can carry a false sentence —
/// which is how wave 339 shipped three of them. This fixture asks the other question, of the
/// engine's **most-used diagnostic**: `incompatible types in this conversion` was one sentence for
/// at least five distinct mistakes, and gcc distinguishes all five *and* names the context.
///
/// The five are genuinely different things to fix:
///
///   - **A discarded qualifier is not an incompatible type.** `int *p = cp;` has compatible
///     pointee types; what is wrong is the `const`, and a reader told "incompatible types" will
///     look for a type mismatch that is not there. The message must name *which* qualifier, since
///     `volatile` reaches the same place.
///   - **A pointer meeting an integer** is a different error from **two pointers meeting**, and
///     C's own wording separates them ("makes pointer from integer" against "incompatible pointer
///     type").
///   - **The direction matters**: `int *p = 1;` and `int x = p;` are not the same mistake.
///
/// The context — initialization, assignment, argument, return — is already carried as
/// `Conversion`, so saying it costs nothing and is what turns a diagnostic into a location.
#[test]
fn a_conversion_diagnostic_names_the_mistake() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let says = |src: &str, want: &[&str]| {
        let d = diags(src);
        for w in want {
            assert!(
                d.iter().any(|m| m.contains(w)),
                "`{src}`\n  should mention {w:?}\n  said {d:?}"
            );
        }
    };

    // **Which qualifier, and in which context.**
    says(
        "int f(const int *cp){ int *p = cp; return *p; }",
        &["const", "initializ"],
    );
    says(
        "int f(volatile int *vp){ int *p = vp; return *p; }",
        &["volatile", "initializ"],
    );
    says(
        "void g(int *); int f(const int *cp){ g(cp); return 0; }",
        &["const", "argument"],
    );
    says(
        "const int *g(void); int *f(void){ return g(); }",
        &["const", "return"],
    );

    // **A pointer and an integer, in both directions** — asserted as the whole *phrase*, because
    // "pointer" and "integer" both appear whichever way round the message has them. Mutation
    // found that: swapping the two arms passed a fixture that only looked for the two words.
    says(
        "int f(void){ int *p = 1; return *p; }",
        &["makes a pointer from an integer"],
    );
    says(
        "int f(int *p){ int x = p; return x; }",
        &["makes an integer from a pointer"],
    );

    // **Two pointers that do not match** — a different sentence from either of the above.
    says(
        "int f(int *p){ char *q = p; return *q; }",
        &["incompatible pointer"],
    );
    says(
        "void g(int *); int f(char *q){ g(q); return 0; }",
        &["incompatible pointer", "argument"],
    );

    // **A record from a scalar** is none of those.
    says(
        "struct S { int a; }; struct S s = 1;",
        &["invalid initializer"],
    );

    // **And one mistake stays one diagnostic.** `sizeof` of an undeclared name reported the name
    // *and* claimed an incomplete type — the last of the poison cascades wave 339 started on.
    for src in [
        "int f(void){ return sizeof(nope); }",
        "int f(void){ return (int)sizeof(nope) + (int)sizeof(nope2); }",
    ] {
        let d = diags(src);
        assert!(
            d.iter().all(|m| m.contains("not declared")),
            "`{src}` should report only the undeclared name: {d:?}"
        );
    }

    // A real incomplete type still says so — the guard is about poison, not about the rule.
    says(
        "struct I; int f(void){ return (int)sizeof(struct I); }",
        &["incomplete"],
    );
}

/// **The diagnostic audit, continued** (§9): four more messages read beside gcc's, using wave
/// 340's two questions — *does it name a thing the program contains*, and *would a different
/// mistake produce the same words*.
///
/// The worst is the first, and it is wave 339's failure class again: **`int a[2] = {1,2,3};`
/// reports "initializer index is outside the array"** when the program contains no index at all.
/// The engine walks initializers with a cursor and reports the cursor; a reader is told to look
/// for a `[5] =` that was never written. gcc says "excess elements in array initializer", and
/// keeps "array index in initializer exceeds array bounds" for the case where an index really was
/// written — two different mistakes that chiero gave one sentence.
///
/// The other three are smaller and all actionable:
///
///   - **An array and a struct overflow differently**, and gcc names which.
///   - **`called object is not a function` does not say *which* object**, though the name is right
///     there in the expression.
///   - **`subscripted value is not an array or pointer` is untrue as written**: this engine
///     subscripts vectors, and does so correctly — only the message forgot.
#[test]
fn an_initializer_diagnostic_names_the_mistake() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let says = |src: &str, want: &str| {
        let d = diags(src);
        assert!(
            d.iter().any(|m| m.contains(want)),
            "`{src}`\n  should say {want:?}\n  said {d:?}"
        );
    };
    let never = |src: &str, unwanted: &str| {
        let d = diags(src);
        assert!(
            !d.iter().any(|m| m.contains(unwanted)),
            "`{src}`\n  should not say {unwanted:?}\n  said {d:?}"
        );
    };

    // **Too many elements is not an index.** The program has no `[n] =` in it.
    says("int a[2] = {1,2,3};", "excess elements in an array");
    never("int a[2] = {1,2,3};", "index");
    says("int a[2][2] = {1,2,3,4,5};", "excess elements in an array");
    never("int a[2][2] = {1,2,3,4,5};", "index");
    says("char s[3] = \"abcd\";", "longer than the array");

    // ...and where an index *was* written, it still says so.
    says("int a[3] = {[5] = 1};", "index");

    // **Which aggregate overflowed.**
    says(
        "struct S { int x, y; }; struct S s = {1,2,3};",
        "excess elements in a struct",
    );
    says(
        "union U { int a; }; union U u = {1,2};",
        "excess elements in a union",
    );
    says("int x = {1,2};", "excess elements in a scalar");

    // **Name the object that is not callable, and the one that is not subscriptable.**
    says("int f(void){ int q = 5; return q(); }", "`q`");
    says("int f(double d){ return (int)d(); }", "`d`");

    // **A vector is subscriptable, and this engine does it** — so the message must not deny it.
    says("int f(void){ int x = 5; return x[0]; }", "vector");
    assert!(
        diags("typedef int v4 __attribute__((vector_size(16))); int f(v4 v){ return v[0]; }")
            .is_empty(),
        "a vector really is subscriptable here"
    );
}

/// **The audit, third instalment** — and this time reading the messages found a missing *rule*.
///
/// §9 asked for class (c): a sentence that contradicts what the engine does. Checking
/// `bit-field width exceeds the width of its type` against every type it can be written on turned
/// up the opposite problem — **`struct S { _Bool b : 2; };` is a constraint violation gcc rejects
/// and this engine accepts.** The check compares the width against the type's *storage* size,
/// which is eight bits for a `_Bool`; C 6.7.2.1p4 bounds it by the number of bits in the type,
/// and a `_Bool` holds one. Nothing had asked, because the only bit-field cases anyone writes by
/// hand are `int`.
///
/// Two messages beside it name nothing, where gcc names the declarator — `width of 'b' exceeds
/// its type`, `size of array 'a' is negative`. In a struct with twenty bit-fields, the sentence
/// without the name says only that one of them is wrong.
#[test]
fn a_width_diagnostic_names_its_declarator() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let says = |src: &str, want: &str| {
        let d = diags(src);
        assert!(
            d.iter().any(|m| m.contains(want)),
            "`{src}`\n  should say {want:?}\n  said {d:?}"
        );
    };

    // **A `_Bool` bit-field holds one bit.** The rule, not the wording.
    for bad in [
        "struct S { _Bool b : 2; };",
        "struct S { _Bool b : 8; };",
        "struct S { int a; _Bool b : 3; };",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // ...and the widths that do fit, on every type a bit-field can take.
    for good in [
        "struct S { _Bool b : 1; };",
        "struct S { char c : 8; };",
        "struct S { short s : 16; };",
        "struct S { int b : 32; };",
        "struct S { long l : 64; };",
        "struct S { unsigned u : 32; };",
        "struct S { int a : 3; int : 5; int b : 2; };",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }

    // **Name the declarator**, which is the whole of what a reader needs in a struct full of them.
    says("struct S { int a; int wide : 33; };", "`wide`");
    says("struct S { _Bool flag : 2; };", "`flag`");
    says("int negative_len[-1];", "`negative_len`");
    // **The member, not the object it is declared inside.** `declaring` is a side channel, and a
    // nested declaration inherits it unless each walk sets its own — this named `x` until the
    // member walk did. Naming the wrong declarator is the same class of defect as naming a
    // mechanism the program has not got.
    says("struct S { int bad[-1]; } x;", "`bad`");
    says("struct S { int bad[-1]; };", "`bad`");

    // **A zero-length array stays accepted**, and this is where that is recorded: gcc refuses it
    // under `-pedantic-errors` as a GNU extension, and the VPP tree contains **1777** of them —
    // the pre-flexible-array idiom for a trailing variable-length member. Rejecting it would fail
    // on the corpus this project exists to read.
    assert!(diags("int a[0];").is_empty());
    assert!(diags("struct S { int n; int a[0]; };").is_empty());
}

/// **The audit's fourth method again** (§9): enumerate the cases a message claims to cover, and
/// try each one.
///
/// `arithmetic on a pointer to an incomplete type` names a *category*, and C's pointer arithmetic
/// has seven spellings. Two were checked — `p + n` and `p - q` — and the other five went silent,
/// including the two that are the same operation written shorter:
///
///   - **`p++`, `p--` and `p += n` are pointer arithmetic**, and they need the pointee's size for
///     exactly the reason `p + 1` does.
///   - **`p[0]` is `*(p + 0)`**, so it needs it too — and this one also *dereferences*, which is
///     why the fixture pins that the diagnostic names the arithmetic rather than the deref.
///
/// The return-type message beside it was checked the same way and is **complete**: `int g(void)[3]`
/// and `int g(void)(void)` are rejected, while `int (*g(void))[3]`, `int (*g(void))(void)` and a
/// `struct` return stay legal. Recording that is the point of the method — most enumerations
/// confirm, and the one that does not is the finding.
///
/// **`void *` and function-pointer arithmetic stay legal**, and belong in the accepted half rather
/// than being overlooked: gcc refuses both under `-pedantic-errors` and accepts them in GNU mode,
/// where `sizeof(void)` is 1 — which this engine already implements deliberately. An incomplete
/// *record* is a different thing: its size is unknown rather than defined to be one.
#[test]
fn every_spelling_of_pointer_arithmetic_needs_a_complete_pointee() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // The two that were checked.
        "struct I; int f(struct I *p){ struct I *q = p + 1; return q != 0; }",
        "struct I; int f(struct I *p, struct I *q){ return (int)(p - q); }",
        // ...and the five that were not.
        "struct I; int f(struct I *p){ p++; return p != 0; }",
        "struct I; int f(struct I *p){ p--; return p != 0; }",
        "struct I; int f(struct I *p){ ++p; return p != 0; }",
        "struct I; int f(struct I *p){ --p; return p != 0; }",
        "struct I; int f(struct I *p){ p += 1; return p != 0; }",
        "struct I; int f(struct I *p){ p -= 1; return p != 0; }",
        "struct I; int f(struct I *p){ return p[0] != 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **The subscript names the arithmetic, not the dereference.** `p[0]` fails for one reason —
    // the stride is unknown — and reporting the deref would send a reader to the wrong half.
    let d = diags("struct I; int f(struct I *p){ return p[0] != 0; }");
    assert!(
        d.iter().any(|m| m.contains("arithmetic")),
        "`p[0]` on an incomplete pointee should name the arithmetic: {d:?}"
    );

    for good in [
        // A complete pointee takes every spelling.
        "struct C { int m; }; int f(struct C *p){ p++; p--; ++p; --p; p += 1; p -= 1; return p[0].m; }",
        "int f(int *p){ p++; p--; p += 2; return p[0]; }",
        // **`void *` and function-pointer arithmetic are GNU extensions this engine keeps**,
        // resting on the same `sizeof(void) == 1` it already implements.
        "int f(void *p){ void *q = p + 1; return q != 0; }",
        "int f(void *p){ p++; return p != 0; }",
        "int f(void (*g)(void)){ return (int)(g + 1 != 0); }",
        // A pointer to an incomplete type is fine as long as nothing needs its size.
        "struct I; int f(struct I *p){ return p != 0; }",
        "struct I; int f(struct I *p, struct I *q){ return p == q; }",
        "struct I; int f(struct I **pp){ return pp[0] != 0; }",
        // The return-type enumeration, confirmed rather than changed.
        "int (*g(void))[3]; int f(void){ return g() != 0; }",
        "int (*g(void))(void); int f(void){ return g() != 0; }",
        "struct S { int m; }; struct S g(void); int f(void){ return g().m; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The constant-expression checklist** (§9), and this one found a *false positive* — the failure
/// mode wave 303 ranks worst, because it tells a reader their correct program is broken.
///
/// C 6.6p6 lists what an integer constant expression may contain: integer constants, enumeration
/// constants, character constants, `sizeof`, `_Alignof` — **and floating constants, but only as
/// the immediate operand of a cast.** That last clause is the whole of the finding:
/// `case (int)1.5:` is legal C and was rejected, while `case (int)(1.5 + 2.5):` is not legal and
/// must stay rejected. One is a floating *constant* being cast; the other is a floating
/// *expression*.
///
/// **The two paths are not the same set, and finding that out was half the wave.** A `case` label
/// needs an *integer* constant expression (6.6p6); an initializer needs an *arithmetic* one
/// (6.6p8), which admits floating arithmetic under a cast. So `int g = (int)-1.5;` is legal and
/// `case (int)-1.5:` is not — the unary minus makes the floating constant no longer the cast's
/// immediate operand. An earlier draft of this fixture asserted one shared list and was wrong;
/// they are walked separately now, with the overlap written out entry by entry.
#[test]
fn a_constant_expression_admits_the_same_things_everywhere() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    // **Every entry of 6.6p6, in a `case` label and in a file-scope initializer.** Written as one
    // list so the two paths are asked the same questions in the same order.
    for e in [
        "1",
        "'a'",
        "sizeof(int)",
        "_Alignof(int)",
        "1 + 2 * 3",
        "(1 ? 2 : 3)",
        "1 << 4",
        "-1",
        "(char)65",
        "(long)5",
        "(unsigned)-1",
        "(int)sizeof(int)",
        // **The clause that was missing**: a floating constant is allowed as a cast's immediate
        // operand, and only there.
        "(int)1.5",
        "(int)1.5f",
        "(int)(1.5)",
        "1 + (int)1.5",
        "(long)(int)1.5",
    ] {
        let case = format!("int f(int n){{ switch(n){{ case {e}: return 1; }} return 0; }}");
        let init = format!("int g = {e};");
        assert!(
            diags(&case).is_empty(),
            "`case {e}:` must be accepted -> {:?}",
            diags(&case)
        );
        assert!(
            diags(&init).is_empty(),
            "`int g = {e};` must be accepted -> {:?}",
            diags(&init)
        );
    }

    // ...and an enumeration constant, which needs its own declaration.
    assert!(
        diags("enum E { A = 5 }; int f(int n){ switch(n){ case A: return 1; } return 0; }")
            .is_empty()
    );
    assert!(diags("enum E { A = 5 }; int g = A;").is_empty());

    // **What is still not an *integer* constant expression**, and the near-misses are the point.
    // `-1.5` is a floating constant with a unary operator on it, so it is no longer the cast's
    // *immediate* operand — gcc rejects `case (int)-1.5:` and accepts `int g = (int)-1.5;`.
    //
    // **The two paths genuinely differ here**, which is why the fixture asks them separately
    // rather than sharing one list: a `case` label needs an *integer* constant expression
    // (6.6p6), an initializer an *arithmetic* one (6.6p8), and the second admits floating
    // arithmetic under a cast that the first does not. An earlier draft of this test asserted
    // they were the same set and was wrong.
    for e in [
        "1.5",
        "(int)-1.5",
        "(int)+1.5",
        "(int)(1.5 + 2.5)",
        "(int)(1.5 * 2.0)",
    ] {
        let case = format!("int f(int n){{ switch(n){{ case {e}: return 1; }} return 0; }}");
        assert!(!diags(&case).is_empty(), "`case {e}:` must be diagnosed");
    }
    for e in [
        "(int)-1.5",
        "(int)+1.5",
        "(int)(1.5 + 2.5)",
        "(int)(1.5 * 2.0)",
    ] {
        let init = format!("int g = {e};");
        assert!(
            diags(&init).is_empty(),
            "`int g = {e};` must be accepted -> {:?}",
            diags(&init)
        );
    }

    // An address constant is an initializer's business and never a `case` label's.
    for src in [
        "int x; int *g = &x;",
        "const char *g = \"s\";",
        "int a[3]; int *g = &a[1];",
        "int a[3]; int *g = a + 1;",
        "struct S { int m; } s; int *g = &s.m;",
    ] {
        assert!(
            diags(src).is_empty(),
            "must be accepted: `{src}` -> {:?}",
            diags(src)
        );
    }
}

/// **The linkage relation, enumerated as pairs rather than as messages** (§9).
///
/// Four diagnostics describe one relation between two declarations of a name, so the category to
/// walk is the *pairs*: five first declarations against four second ones, for objects and again
/// for functions. Forty cells; the object half agrees with gcc in all twenty, and **three
/// function cells were false positives.**
///
/// The cause is a paragraph that applies to one and not the other. **C 6.2.2p5: a function
/// declared with no storage-class specifier has the linkage it would have with `extern`** — so a
/// plain `int f(void);` after `static int f(void);` *adopts* the internal linkage and is legal.
/// An *object* has no such rule: a plain `int x;` at file scope is a tentative definition with
/// external linkage, so `static int x; int x;` really is a conflict. The engine applied the
/// object rule to both.
///
/// This is wave 344's lesson from the other side: there, two paths checked what looked like one
/// rule and C had two paragraphs; here, one path checked two things C treats differently.
#[test]
fn linkage_is_a_relation_between_two_declarations() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    // **A function with no storage class is `extern`** (6.2.2p5), so these three are legal.
    for good in [
        "static int f(void); int f(void);",
        "static int f(void); int f(void){ return 2; }",
        "static int f(void){ return 1; } int f(void);",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }

    // **An object has no such rule**, and the same shape stays a conflict.
    for bad in [
        "static int x; int x;",
        "static int x; int x = 2;",
        "static int x = 1; int x;",
        "int x; static int x;",
        "int x = 1; static int x;",
        "extern int x; static int x;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // ...and `static` after an external *function* is still a conflict — the adoption runs one
    // way only, which is what separates this from "linkage never conflicts for functions".
    for bad in [
        "int f(void); static int f(void);",
        "extern int f(void); static int f(void);",
        "int f(void){ return 1; } static int f(void);",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // The rest of the function matrix, which was already right and is pinned so the fix cannot
    // over-reach into it.
    for good in [
        "int f(void); int f(void);",
        "int f(void); extern int f(void);",
        "int f(void); int f(void){ return 2; }",
        "static int f(void); static int f(void);",
        "static int f(void); extern int f(void);",
        "extern int f(void); int f(void);",
        "extern int f(void); extern int f(void);",
        "extern int f(void); int f(void){ return 2; }",
        "int f(void){ return 1; } int f(void);",
        "int f(void){ return 1; } extern int f(void);",
        "static int f(void){ return 1; } static int f(void);",
        "static int f(void){ return 1; } extern int f(void);",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }

    // Two definitions are two definitions whatever the linkage, and a type conflict is separate
    // from a linkage one — both halves of the relation the four messages share.
    for bad in [
        "int f(void){ return 1; } int f(void){ return 2; }",
        "static int f(void){ return 1; } int f(void){ return 2; }",
        "int x = 1; int x = 2;",
        "static int x = 1; static int x = 2;",
        "int x; long x;",
        "static int x; static long x;",
        "int f(void); long f(void);",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }
}

/// **The address-constant checklist** (§9), enumerated in a file-scope initializer, where the rule
/// bites.
///
/// C 6.6p9 builds an address constant with `&`, `[]`, `.`, `->` and array-to-pointer decay — but
/// only where forming the address **reads no object**. That last condition is what the engine's
/// `reads_an_object` walk stops short of: it treats `&` as a full stop, on the grounds that the
/// operand of `&` is not read. True of `&x`, and false of everything reached *through a pointer*.
///
/// Two misses, and both are the same shape:
///
///   - **`&p->m` reads `p`.** The arrow is a dereference; the pointer's value is not a constant,
///     so the address of the member is not one either.
///   - **`*&x` reads `x`.** A dereference is a read wherever it appears, and the walk descended
///     into `&x` and stopped.
///
/// The accepted half is what keeps the fix from swallowing the whole category — five shapes that
/// *look* like reads and are not: `&*&x`, `&*(a+1)`, `&(&s)->m`, `&a[1]` and `&a[0] + 2`.
#[test]
fn an_address_constant_reads_no_object() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Reached through a pointer object, so the pointer is read.
        "struct S { int m; } s; struct S *p = &s; int *g = &p->m;",
        "int *p; int *g = &*p;",
        // A variable subscript is read even though the array is not.
        "int a[3]; int i; int *g = &a[i];",
        // A dereference is a read wherever it appears.
        "int x; int g = *&x;",
        "int x; int *p = &x; int g = *p;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // The plain forms 6.6p9 names.
        "int x; int *g = &x;",
        "int a[3]; int *g = &a[1];",
        "int a[3]; int *g = a;",
        "int a[3]; int *g = a + 1;",
        "int a[3]; int *g = &a[0] + 2;",
        "int a[3]; int *g = &a[1] - 1;",
        "struct S { int m; int n; } s; int *g = &s.n;",
        "int f(void); int (*g)(void) = f;",
        "int f(void); int (*g)(void) = &f;",
        "const char *g = \"s\";",
        "char g[] = \"s\";",
        "int *g = 0;",
        "int *g = (int *)0;",
        "int *g = (int *)100;",
        "int x; int *g = &x + 1;",
        "int x; long g = (long)&x;",
        "int x; int *g = 1 ? &x : 0;",
        "int x; int *const g = &x;",
        // **The five that look like reads and are not.** `&*E` cancels when `E` is itself an
        // address constant, and a `->` on an address constant is a `.` in disguise.
        "int x; int *g = &*&x;",
        "int a[3]; int *g = &*(a+1);",
        "struct S { int m; } s; int *g = &(&s)->m;",
        "struct S { int m; } s; int *g = &s.m;",
        "int a[3]; int *g = &a[2 - 1];",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The incomplete-type family, tabulated by context** (§9) — six messages over one predicate, so
/// the thing to enumerate is *where* an incomplete type is forbidden, not what each message says.
///
/// Seventeen contexts against **two** incomplete types, and running both is what found the
/// defects: `is_incomplete` deliberately excludes `void` — `void *p` and `void f(void)` need it to
/// — so every context has to decide about `void` separately, and three had not.
///
///   - **`void a[3];` and `struct S { void m; };`** — an array element and a member need a size,
///     and `void` has none. Both were silent while `struct I a[3];` was caught.
///   - **`struct I f(void){ }`** — a *definition* must return a complete type. A *declaration*
///     need not: `struct I f(void);` is legal C, because the type may be completed before anyone
///     calls it. That distinction is why this is checked at the body and not at the type.
///
/// **Three further contexts are the deliberate `sizeof(void) == 1` extension** and stay silent:
/// `sizeof(void)`, `_Alignof(void)` and `void *p + 1`. gcc refuses all three under
/// `-pedantic-errors` and accepts them in the GNU mode the corpus uses; waves 335 and 343 recorded
/// the same decision, and the fixture keeps them in the accepted half so a later wave does not
/// "fix" one of them in isolation.
#[test]
fn an_incomplete_type_is_refused_in_every_context_that_needs_a_size() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    // The same context, asked of an incomplete record and of `void`.
    for bad in [
        "struct I; struct I v;",
        "void v;",
        "struct I; struct I a[3];",
        "void a[3];",
        "int f(void){ void a[2]; return 0; }",
        "struct I; struct S { struct I m; };",
        "struct S { void m; };",
        "union U { void m; };",
        "struct I; int f(struct I p){ return 0; }",
        "int f(void p){ return 0; }",
        // A *definition* returns a complete type.
        "struct I; struct I f(void){ }",
        // The contexts that were already right, kept so the fix cannot lose them.
        "struct I; unsigned g = sizeof(struct I);",
        "struct I; unsigned g = _Alignof(struct I);",
        "struct I; int f(struct I *p){ (*p); return 0; }",
        "struct I; int f(struct I *p){ return p->m; }",
        "struct I; int f(struct I *p){ return (int)(p + 1); }",
        "struct I; int f(struct I *p){ struct I q = *p; return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // A pointer to an incomplete type needs no size, in any of its spellings.
        "struct I; struct I *g;",
        "struct I; int f(struct I *p){ return p != 0; }",
        "struct I; int f(struct I *p){ return (int)(long)p; }",
        "struct I; int f(struct I *p);",
        "struct I; struct I *f(void);",
        "struct I; struct S { struct I *m; };",
        "struct S { void *m; };",
        "void *g;",
        // **A declaration may return an incomplete type**; only a definition may not.
        "struct I; struct I f(void);",
        "struct I; struct I f(void); int g(void){ return 0; }",
        // `void` where it belongs.
        "void f(void){ }",
        "int f(void *p){ (void)*p; return 0; }",
        // **The `sizeof(void) == 1` extension, kept deliberately** — see the doc comment.
        "unsigned g = sizeof(void);",
        "unsigned g = _Alignof(void);",
        "int f(void *p){ return (int)(p + 1); }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A predicate's documented exception, checked against each caller** (§9's promoted front).
///
/// `assignable`'s comment says "**`_Bool` takes any scalar** (C 6.3.1.2): `_Bool b = p;` is a test
/// against zero, not a truncation". True — and true of a *conversion*. `assignable` has a second
/// caller, the pointer-comparison rule, where **no conversion happens**: `p == b` converts
/// nothing, so 6.5.9's constraint applies unchanged and the exception is simply wrong there.
///
/// Enumerating that caller found a wider hole behind it. The comparison check is guarded on
/// **both** operands being pointers, so a pointer compared against *any* integer was never
/// examined at all — `p == i`, `p == 1`, `p == c` all silent.
///
/// The boundary needs both halves of C 6.5:
///
///   - **Equality admits a null pointer constant**, so `p == 0` and `0 == p` are legal and
///     `p == 1` is not — the value decides, not the type.
///   - **A relational operator admits nothing**: `p > 0` is a constraint violation though
///     `p == 0` is fine, which is the one case that separates 6.5.8 from 6.5.9.
///   - **`!p` and `p && 1` stay legal**, because the logical operators take any scalar and are
///     not comparisons at all.
#[test]
fn comparing_a_pointer_with_an_integer_is_constrained() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // The `_Bool` exception, wrong in this caller.
        "int f(int *p, _Bool b){ return p == b; }",
        "int f(int *p, _Bool b){ return p != b; }",
        "int f(int *p, _Bool b){ return p < b; }",
        // ...and the hole it was hiding: any integer at all, either way round.
        "int f(int *p, int i){ return p == i; }",
        "int f(int *p, int i){ return i == p; }",
        "int f(int *p, long l){ return p == l; }",
        "int f(int *p, char c){ return p == c; }",
        // A non-zero constant is not a null pointer constant.
        "int f(int *p){ return p == 1; }",
        // **A relational operator has no null-constant exemption.**
        "int f(int *p){ return p > 0; }",
        "int f(int *p, int i){ return p < i; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Equality against a null pointer constant, both ways round and spelled two ways.
        "int f(int *p){ return p == 0; }",
        "int f(int *p){ return 0 == p; }",
        "int f(int *p){ return p != 0; }",
        "int f(int *p){ return p == (void*)0; }",
        // Two pointers, and two `_Bool`s.
        "int f(int *p, int *q){ return p == q; }",
        "int f(int *p, int *q){ return p < q; }",
        "int f(_Bool a, _Bool b){ return a == b; }",
        "int f(int i, int j){ return i < j; }",
        // **The logical operators are not comparisons** and take any scalar.
        "int f(int *p){ return !p; }",
        "int f(int *p){ return p && 1; }",
        "int f(int *p){ return p ? 1 : 2; }",
        // The conversion caller, where the `_Bool` exception is right and must stay.
        "int f(int *p){ _Bool b = p; return b; }",
        "int f(int *p){ if (p) return 1; return 0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`is_null_constant`, the last predicate on §9's sweep** — and the one whose exception was too
/// narrow rather than too wide.
///
/// C 6.3.2.3p3: a null pointer constant is **an integer constant expression with the value 0**, or
/// such an expression cast to `void *`. The predicate matched only a `Number` or a `Cast`, on the
/// stated grounds that it must judge the *written* expression rather than a variable that happens
/// to hold zero. The second half of that is right and the first is not: `1 - 1`, `'\0'`,
/// `(1 ? 0 : 0)`, `sizeof(int) - 4` and an enumerator worth zero are all integer constant
/// expressions, and all were refused.
///
/// **`eval` is already exactly the right question.** It folds constant expressions and answers
/// `None` for anything else — a variable, a `const int` (which C does not call a constant
/// expression), `i - i` on a parameter — so the kind guard was a second, coarser implementation of
/// what `eval` decides. The rejected half of this fixture is what proves the widening does not
/// swallow those.
///
/// **Three of these were introduced by wave 348 and are regressions of my own.** Before it, the
/// comparison rule required both operands to be pointers, so `p == 1 - 1` was never examined and
/// passed by accident; tightening the guard inherited this predicate's narrowness. The corpus did
/// not catch them and the ratchet could not — only the sweep did.
#[test]
fn a_null_pointer_constant_is_any_zero_constant_expression() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    // **Every spelling of zero**, in each of the three contexts the predicate serves:
    // initialization, comparison, and argument or return.
    for zero in [
        "0",
        "(void*)0",
        "(int)0",
        "1 - 1",
        "0 * 5",
        "'\\0'",
        "(1 ? 0 : 0)",
        "(1 == 2)",
    ] {
        for src in [
            format!("int *g = {zero};"),
            format!("int f(int *p){{ return p == {zero}; }}"),
            format!("int f(int *p){{ return p != {zero}; }}"),
            format!("void g(int *); int f(void){{ g({zero}); return 0; }}"),
            format!("int *h(void){{ return {zero}; }}"),
        ] {
            assert!(
                diags(&src).is_empty(),
                "must be accepted: `{src}` -> {:?}",
                diags(&src)
            );
        }
    }

    // An enumeration constant worth zero is an integer constant expression like any other.
    assert!(diags("enum { Z = 0 }; int *g = Z;").is_empty());
    assert!(diags("enum { Z = 0 }; int f(int *p){ return p == Z; }").is_empty());
    assert!(diags("int *g = sizeof(int) - 4;").is_empty());

    // **What `eval` refuses, and must keep refusing.** A variable holding zero is not a constant
    // expression, and neither is a `const int` — C is explicit about the second, and it is the
    // case the predicate's comment was written to protect.
    for bad in [
        "int x; int *g = x;",
        "int f(int x){ int *p = x; return p != 0; }",
        "int f(void){ const int k = 0; int *p = k; return p != 0; }",
        "int f(int *p, int i){ return p == i - i; }",
        // ...and a constant expression that is not zero is not a null pointer constant.
        "int *g = 1;",
        "int *g = 2 - 1;",
        "enum { NZ = 1 }; int *g = NZ;",
        "int f(int *p){ return p == 2 - 1; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }
}

/// **The `switch`/`case` family beyond wave 319** — the last category on §9's message audit.
///
/// Fifteen contexts. Eleven already agree with gcc, including the ones that look hardest: a `case`
/// inside a nested *block* belongs to the enclosing switch and is legal, a `case` inside a nested
/// *switch* belongs to the inner one, a label in a `while` is not in a switch at all, and
/// `case 1+0:` collides with `case 1:` because the rule is about the folded value.
///
/// **The two misses are both about ranges.** `case 1 ... 3` is a GNU extension this engine
/// supports — gcc refuses it under `-pedantic-errors`, so this fixture is calibrated to GNU mode
/// like `0b101` and `\e` — and wave 319's duplicate table records a range by its **lower bound
/// only**, which its own comment says. So:
///
///   - **`case 1 ... 3:` then `case 2:`** — the single value falls inside the range and was not
///     seen, because only 1 was in the table.
///   - **`case 1 ... 3:` then `case 3 ... 5:`** — the ranges overlap at 3, and neither lower
///     bound is in the other's.
///
/// **`case 3 ... 1:` is an empty range, and gcc gives it its lower bound** — not nothing, and not
/// the span read backwards. It collides with `case 3:` and with a second `case 3 ... 1:`, and not
/// with `1` or `2`. That is the discriminator, and it was probed rather than assumed: a first
/// draft of this fixture called an empty range collision-free and gcc disagreed. The occupied
/// interval is therefore `[lo, max(lo, hi)]`, which also happens to be what wave 319's
/// lower-bound-only rule already did for the empty case — the reason that rule looked right.
#[test]
fn a_case_range_occupies_every_value_in_it() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A value inside a range, and a range containing a value, in both orders.
        "int f(int n){ switch(n){ case 1 ... 3: return 1; case 2: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 2: return 1; case 1 ... 3: return 2; } return 0; }",
        // Overlapping ranges, touching at one end and overlapping in the middle.
        "int f(int n){ switch(n){ case 1 ... 3: return 1; case 3 ... 5: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 1 ... 9: return 1; case 4 ... 5: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 4 ... 5: return 1; case 1 ... 9: return 2; } return 0; }",
        // An empty range still occupies its lower bound, so these two collide at 3.
        "int f(int n){ switch(n){ case 3 ... 1: return 1; case 3: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 3: return 1; case 3 ... 1: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 3 ... 1: return 1; case 3 ... 1: return 2; } return 0; }",
        // ...and the plain duplicates wave 319 already caught, kept so the fix cannot lose them.
        "int f(int n){ switch(n){ case 1: return 1; case 1: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 1: return 1; case 1+0: return 2; } return 0; }",
        "int f(int n){ switch(n){ default: return 1; default: return 2; } return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Ranges that do not meet, on either side and adjacent.
        "int f(int n){ switch(n){ case 1 ... 3: return 1; case 4: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 1 ... 3: return 1; case 4 ... 6: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 4 ... 6: return 1; case 1 ... 3: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 0: return 1; case 1 ... 3: return 2; case 4: return 3; } return 0; }",
        // **An empty range occupies its lower bound**, not nothing and not the whole span.
        // `case 3 ... 1` collides with `case 3` and with nothing else — so `1` and `2` are free,
        // and a second `3 ... 1` is a duplicate, which is in the rejected half above.
        "int f(int n){ switch(n){ case 3 ... 1: return 1; } return 0; }",
        "int f(int n){ switch(n){ case 3 ... 1: return 1; case 2: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 3 ... 1: return 1; case 1: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 3 ... 1: return 1; case 5 ... 4: return 2; } return 0; }",
        // A range beside a `default`, and a one-element range.
        "int f(int n){ switch(n){ case 1 ... 3: return 1; default: return 2; } return 0; }",
        "int f(int n){ switch(n){ case 1 ... 1: return 1; case 2: return 2; } return 0; }",
        // The contexts that were already right.
        "int f(int n){ switch(n){ case 1: { case 2: ; } return 1; } return 0; }",
        "int f(int n){ switch(n){ case 1: switch(n){ case 1: return 1; } return 2; } return 0; }",
        "int f(int n){ switch(n){ case 1: { int q = 2; return q; } case 2: return 2; } return 0; }",
        "enum E { A, B }; int f(enum E e){ switch(e){ case A: return 1; case B: return 2; } return 0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **The speculative-fold sweep** (§9's front) — `eval` reports as it folds, so every caller has
/// to decide whether it is asking a *question* or making a *judgement*, and both directions were
/// wrong somewhere.
///
///   - **A question that reports.** Wave 349 found `is_null_constant` refusing a generated program
///     for an overflow no constant context contained, and fixed it by discarding.
///   - **A judgement that stays silent.** `int g = 1/0;` is accepted outright. The initializer
///     asks "is this constant?", `eval` says no *and explains why*, and the explanation is thrown
///     away with the rest — then `reads_an_object` finds nothing to complain about, so the whole
///     program passes.
///
/// The discarding is right and the unconditional part is not: when the fold **fails**, its
/// diagnostics are the reason, and the caller has nothing better to say.
///
/// **`1/0` inside a function stays legal**, which is the discriminator: it is runtime undefined
/// behaviour rather than a constraint violation, and only a context that *requires* a constant
/// expression makes it an error. So the fix cannot be "report division by zero wherever it folds".
#[test]
fn a_failed_constant_fold_keeps_its_explanation() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    // **The silent half.** A malformed constant expression where one is required.
    for bad in [
        "int g = 1/0;",
        "int g = 1%0;",
        "int *g = 1/0;",
        "struct S { int m; } s = { 1/0 };",
    ] {
        let d = diags(bad);
        assert!(!d.is_empty(), "must be diagnosed: `{bad}`");
        assert!(
            d.iter().any(|m| m.contains("division")),
            "`{bad}` should say why the fold failed: {d:?}"
        );
    }

    // **A fold can report *and* succeed**, which is the case mutation found this fixture missing:
    // `2147483647 + 1` folds to a wrapped value and complains on the way, so keying the rescue on
    // "the fold failed" still discarded it and `int g = 2147483647 + 1;` was accepted. gcc refuses
    // it under `-pedantic-errors`.
    for bad in ["int g = 2147483647 + 1;", "int g = 2147483647 * 2;"] {
        let d = diags(bad);
        assert!(
            d.iter().any(|m| m.contains("overflow")),
            "`{bad}` should report the overflow: {d:?}"
        );
    }

    // **One bad thing, one report** (contract 20). `case 1/0:` folded three times and said so
    // three times — twice for the division and once for "not an integer constant expression".
    for src in [
        "int f(int n){ switch(n){ case 1/0: return 1; } return 0; }",
        "int f(int n){ switch(n){ case 1 ... 1/0: return 1; } return 0; }",
        "int f(int n){ switch(n){ case 2147483647 + 1: return 1; } return 0; }",
        "int a[1/0];",
        "int a[2147483647 + 1];",
        "struct S { int b : 1/0; };",
        "enum { X = 1/0 };",
    ] {
        let d = diags(src);
        assert_eq!(d.len(), 1, "one mistake, one report: `{src}` -> {d:?}");
    }

    // **Runtime division by zero is not a constraint violation**, so a context that does not
    // require a constant expression must stay silent.
    for good in [
        "int f(void){ return 1/0; }",
        "int f(int n){ return n/0; }",
        "int f(void){ int x = 1/0; return x; }",
        "int f(void){ return 1 ? 0 : 1/0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Where each storage class may appear** — wave 352's census over C 6.8's statement constraints
/// and 6.9's external definitions, the last two neighbourhoods no census had touched.
///
/// Twenty programs found nine misses, and every one of them is the same question asked in a
/// different place, so the fixture is the **grid**: seven specifiers against five contexts.
/// C states it in four separate paragraphs, which is why it reads as four unrelated rules and was
/// implemented as none:
///
///   - **6.7.1p3 — file scope takes no `auto` and no `register`.** There is no automatic storage
///     to refer to.
///   - **6.8.5p3 — a `for` declaration takes only `auto` or `register`.** `static` there would
///     outlive the loop it is scoped to.
///   - **6.7.6.3p2 — a parameter takes only `register`.**
///   - **6.9.1p4 — a function definition takes only `extern` or `static`.** `inline` and
///     `_Noreturn` are *function specifiers* and are unaffected, which is what stops this rule
///     rejecting `static inline`, the corpus's commonest spelling.
///
/// `_Thread_local` is its own row and not a synonym for either: it is legal at file scope, and at
/// block scope only beside `static` or `extern` (6.7.1p3).
#[test]
fn a_storage_class_belongs_to_its_context() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    // The grid, written as `(specifier, file, block, for, parameter, function)` with `true`
    // meaning legal. Every cell was put to gcc under `-pedantic-errors`.
    let grid: &[(&str, bool, bool, bool, bool, bool)] = &[
        ("", true, true, true, true, true),
        ("extern ", true, true, false, false, true),
        ("static ", true, true, false, false, true),
        ("auto ", false, true, true, false, false),
        ("register ", false, true, true, true, false),
    ];

    for &(sc, file, block, for_, param, func) in grid {
        let cases: [(&str, bool, String); 5] = [
            ("file scope", file, format!("{sc}int x;")),
            (
                "block scope",
                block,
                format!("int f(void){{ {sc}int x = 1; return x; }}"),
            ),
            (
                "`for` initializer",
                for_,
                format!("int f(void){{ for ({sc}int i = 0; i < 2; i++) ; return 0; }}"),
            ),
            (
                "parameter",
                param,
                format!("int f({sc}int a){{ return a; }}"),
            ),
            ("function", func, format!("{sc}int f(void){{ return 1; }}")),
        ];
        for (where_, legal, src) in cases {
            let d = diags(&src);
            if legal {
                assert!(
                    d.is_empty(),
                    "`{sc}` in a {where_} is legal: `{src}` -> {d:?}"
                );
            } else {
                assert!(!d.is_empty(), "`{sc}` in a {where_} is not: `{src}`");
            }
        }
    }

    // **`typedef` and `_Thread_local` do not fit the grid**, and are asked their own way.
    assert!(diags("typedef int T; T v;").is_empty());
    assert!(diags("int f(void){ typedef int T; T v = 1; return v; }").is_empty());
    assert!(!diags("int f(void){ for (typedef int T; 0; ) ; return 0; }").is_empty());
    // `typedef` in a parameter is caught by the **parser** — a parameter is built as a `Var`
    // whatever its specifiers said, so `is_typedef` never reaches sema. Its fixture lives with
    // the other parser constraints.
    assert!(diags("_Thread_local int x;").is_empty());
    assert!(diags("int f(void){ _Thread_local static int x; return x; }").is_empty());

    // **A function specifier is not a storage class**, so the corpus's commonest spelling stays.
    assert!(diags("static inline int f(void){ return 1; } int g(void){ return f(); }").is_empty());
    assert!(diags("inline int f(void){ return 1; }").is_empty());
}

/// **A compound literal is initialized like an object** (C 6.5.2.5p3) — wave 355's census over a
/// construct no earlier one had looked at.
///
/// Eighteen programs, **five misses, and one cause**: the braced list of a compound literal is
/// never checked. `check_init` runs for a declaration's initializer and a compound literal is not
/// a declaration, so `(int){1,2}`, `(struct S){1,2}` and `(int[2]){1,2,3}` all passed — the same
/// three rules that have been enforced on `int x = {1,2};` since wave 314.
///
/// The type is unchecked too, which is the other half of 6.5.2.5p1: the type name must be a
/// **complete object type**, so `(struct Undefined){1}` and `(int[n]){1}` are violations. `void`
/// and a function type were already caught, by the cast path rather than by any rule about
/// compound literals.
///
/// The accepted half is what keeps the fix from reaching ordinary casts: **`(int)x` is not a
/// compound literal**, and the only thing separating them in this AST is whether the operand is an
/// initializer list — the distinction wave 329's `not_an_lvalue` already had to make.
#[test]
fn a_compound_literal_is_initialized_like_an_object() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // 6.5.2.5p3: the list initializes the object, so the initializer rules apply.
        "int f(void){ return (int){1,2}; }",
        "struct S { int a; }; int f(void){ return (struct S){1,2}.a; }",
        "int f(void){ return (int[2]){1,2,3}[0]; }",
        "struct S { int a, b; }; int f(void){ return (struct S){.nope = 1}.a; }",
        "int f(void){ return (int[2]){[5] = 1}[0]; }",
        // 6.5.2.5p1: the type name is a complete object type.
        "int f(void){ return (struct Undefined){1}.a; }",
        "int f(int n){ return (int[n]){1}[0]; }",
        // ...and the two the cast path already caught, kept so the fix cannot lose them.
        "int f(void){ return (void){1}; }",
        "int f(void){ return (int(void)){1}; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **The type check must say it is the type.** Without it `check_init` still rejects an
    // incomplete record — a record with no members makes every element excess — so the program is
    // caught with a sentence about the *initializer* and a reader is sent to count elements.
    // Mutation found this: deleting the type check left every rejection intact.
    for (src, want) in [
        (
            "int f(void){ return (struct Undefined){1}.a; }",
            "complete object type",
        ),
        (
            "int f(int n){ return (int[n]){1}[0]; }",
            "variably modified",
        ),
    ] {
        let d = diags(src);
        assert!(
            d.iter().any(|m| m.contains(want)),
            "`{src}` should say {want:?}: {d:?}"
        );
    }

    for good in [
        // Every well-formed spelling, including the inferred length and the qualified form.
        "int f(void){ return (int){1}; }",
        "int f(void){ return (unsigned){1}; }",
        "struct S { int a; }; int f(void){ return (struct S){1}.a; }",
        "struct S { int a, b; }; int f(void){ return (struct S){.b = 1}.b; }",
        "int f(void){ return (int[3]){1,2,3}[1]; }",
        "int f(void){ return (int[]){1,2,3}[2]; }",
        "int f(void){ return (int[3]){1}[2]; }",
        "int f(void){ return (int){1} + (int){2}; }",
        "int f(void){ const int *p = &(const int){1}; return *p; }",
        "int *g(void){ return &(int){1}; }",
        "int f(void){ return (int)sizeof((int[]){1,2,3}); }",
        "struct S { int a; }; int f(void){ struct S *p = &(struct S){1}; return p->a; }",
        "int f(void){ (int){1} = 5; return 0; }",
        // **An ordinary cast is not a compound literal**, and must stay untouched.
        "int f(int x){ return (int)x; }",
        "int f(double d){ return (int)d; }",
        "int f(void){ return (int)(char)300; }",
        "struct S { int a; }; int f(struct S *p){ return ((struct S *)p)->a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A designator list descends** (C 6.7.9p6) — wave 356's census over the half of 6.7.9 that
/// wave 314 did not reach.
///
/// Twenty-four programs; the compound-assignment half came back clean but for one row, and the
/// designator half gave **four misses behind one cause**: a designator list with more than one
/// component is not walked. `[0][5]`, `[5][0]`, `.p.nope` and `[1].nope` all name a sub-object
/// that does not exist, and each was accepted.
///
/// The accepted half is what makes it a *descent* rather than a longer loop:
///
///   - **`{[0][0] = 1, [1][1] = 2}` is legal**, so the walk has to enter the row and come back
///     out with the outer cursor intact.
///   - **`{1, [2] = 3}` and `{[0] = 1, 2, 3}` are legal**: a designator sets the cursor and
///     positional elements resume from it, which is why the count check cannot simply be
///     "elements ≤ capacity".
///   - **`{.x = 1, 2}` is legal and `{.y = 1, 2}` is not** — the second positional element
///     follows the *designated* one, so it lands past the end.
///
/// The stray from the other half is C 6.5.5p2 through 6.5.16.2: **`%=` needs integer operands**,
/// so `d %= 2` on a `double` is a violation where `d *= 2` is fine.
#[test]
fn a_designator_list_descends_into_the_object() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A nested index out of range, at either level.
        "int a[2][2] = {[0][5] = 1};",
        "int a[2][2] = {[5][0] = 1};",
        // A nested member that does not exist, through a struct and through an array of them.
        "struct P { int x, y; }; struct Q { struct P p; }; struct Q q = {.p.nope = 1};",
        "struct P { int x, y; }; struct P a[2] = {[1].nope = 3};",
        // ...and the two the single-level walk already caught.
        "int a[3] = {[0] = 1, 2, 3, 4};",
        "struct P { int x, y; }; struct P p = {.y = 1, 2};",
        // 6.5.5p2: `%` is an integer operation, so `%=` is too.
        "int f(double d){ d %= 2; return (int)d; }",
        // **Either operand being floating is the violation.** Mutation found the fixture testing
        // only the left one, which the `d %= 2` row above already covers.
        "int f(int x){ x %= 2.0; return x; }",
        "int f(int x, double d){ x %= d; return x; }",
        // **A nested designator moves the outer cursor**, so a positional element after it
        // resumes from the row it named. Mutation found this: dropping the cursor update left
        // every earlier case passing, because their designators happened to name position 0.
        "int a[2][2] = {[1][1] = 1, 2};",
        "struct P { int x, y; }; struct P a[2] = {[1].y = 1, 2};",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Nested designators that are in range, and the mixed forms around them.
        "int a[2][2] = {[0][0] = 1, [1][1] = 2};",
        "int a[2][2] = {[1][1] = 2};",
        // ...and one naming position 0 leaves room for the element after it.
        "int a[2][2] = {[0][0] = 1, 2};",
        "int a[2][2] = {{1,2},{3,4}};",
        "struct P { int x, y; }; struct Q { struct P p; }; struct Q q = {.p.x = 1};",
        "struct P { int x, y; }; struct Q { struct P p; }; struct Q q = {.p.y = 1};",
        "struct P { int x, y; }; struct P a[2] = {[1].y = 3};",
        "struct P { int x, y; }; struct P a[2] = {[0].x = 1, [1].y = 2};",
        // A designator sets the cursor; positional elements resume from it.
        "int a[3] = {1, [2] = 3};",
        "int a[3] = {[0] = 1, 2, 3};",
        "int a[3] = {[2] = 3, [0] = 1};",
        "struct P { int x, y; }; struct P p = {.x = 1, 2};",
        // The compound-assignment half, which the census found already right.
        "int f(int *p){ p += 1; return *p; }",
        "int f(int *p){ p -= 1; return *p; }",
        "int f(int x){ x += 1; return x; }",
        "int f(double d){ d *= 2; return (int)d; }",
        "int f(int x){ x %= 2; return x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **An enumerator is representable as `int`** (C 6.7.2.2p2) — wave 357's census.
///
/// Eighteen programs across two neighbourhoods. **The conditional operator came back complete**:
/// all twelve of 6.5.15p3's cases agree with gcc, including the three that are easy to get wrong —
/// `p ? q : 0` takes the pointer's type, `p ? q : v` with a `void *` is legal, and two *different*
/// struct pointers are not. Recording a confirmation is part of the method.
///
/// The enumeration half gave three misses, in two rules:
///
///   - **A value outside `int`**, at either end, and **the implicit successor counts**:
///     `{A = 2147483647, B}` overflows on `B`, which no explicit value names.
///   - **An enumeration has at least one enumerator** (6.7.2.2p1). `enum E { };` is refused by gcc
///     in *both* modes, unlike the range rule.
///
/// A case that looked like an exemption is not one: **C has no forward-declared enumeration**, so
/// `enum E { A }; enum E;` is an error even after the definition — unlike a struct tag, where a
/// repeat declaration is how forward references work (wave 330). It is out of the accepted half
/// because gcc rejects it, not because this engine does.
///
/// **The range rule is `-pedantic-errors` only** — GNU C widens an enum to whatever type holds its
/// values — so it sits with the other rules this project calibrates that way rather than with the
/// extensions it keeps. The corpus decides whether that is safe, and it is: the twenty-header gate
/// stays silent.
#[test]
fn an_enumerator_fits_in_an_int() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Outside `int`, at either end.
        "enum E { A = 2147483648 }; int f(void){ return (int)A; }",
        "enum E { A = -2147483649 }; int f(void){ return (int)A; }",
        "enum E { A = 4294967295 }; int f(void){ return (int)A; }",
        // **The implicit successor**, which no explicit value names.
        "enum E { A = 2147483647, B }; int f(void){ return (int)B; }",
        // 6.7.2.2p1: an enumeration has an enumerator list.
        "enum E { }; int f(void){ return 0; }",
        "enum E { } e; int f(void){ return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // The extremes that do fit, and the ordinary shapes around them.
        "enum E { A = 2147483647 }; int f(void){ return (int)A; }",
        "enum E { A = -2147483648 }; int f(void){ return (int)A; }",
        "enum E { A = 2147483646, B }; int f(void){ return (int)B; }",
        "enum E { A = 1 }; int f(void){ return A; }",
        "enum E { A = 1, }; int f(void){ return A; }",
        "enum E { A, B, C }; int f(void){ return C; }",
        "enum E { A = -1, B }; int f(void){ return B; }",
        "enum E { A }; enum E e; int f(void){ return (int)e; }",
        "enum { A = 1 }; int f(void){ return A; }",
        // The conditional-operator half, confirmed rather than changed.
        "int f(int c, int *p){ return *(c ? p : 0); }",
        "int f(int c, int *p, void *v){ return (c ? p : v) != 0; }",
        "void g(void); void h(void); int f(int c){ c ? g() : h(); return 0; }",
        "struct S { int a; }; int f(int c, struct S x, struct S y){ return (c ? x : y).a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **C 6.7.2.1p5: a bit-field's type is an integer type**, and `sizeof` does not apply to one.
///
/// gcc refuses every row below in *both* modes, so neither rule is a pedantic-only divergence.
/// The list of accepted types is where the constraint's letter and gcc part company and the
/// letter is not what this project follows: C names `_Bool`, `signed int` and `unsigned int`,
/// then permits implementation-defined types, and gcc takes `char`, `short`, `long long`, an
/// enumeration, and any typedef of those. So the rule chiero enforces is **integer or not**,
/// which is the line gcc actually draws — a `float`, a pointer, a struct, a union, and an array
/// are refused, and the qualified and typedef spellings of an integer are not.
#[test]
fn a_bitfield_has_an_integer_type() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // The four non-integer shapes, plus the typedef spelling that hides one.
        "struct S { float a:3; };",
        "struct S { double a:3; };",
        "struct S { int *a:3; };",
        "struct S { void *a:3; };",
        "struct S { struct T { int x; } a:3; };",
        "struct S { union U { int x; } a:3; };",
        "struct S { int a[2]:3; };",
        "typedef float F; struct S { F a:3; };",
        // **`sizeof` of a bit-field**, which has no size to report. `&s.a` is already refused,
        // and this is the same object with the same reason.
        "struct S { int a:3; }; int f(void){ struct S s; return (int)sizeof(s.a); }",
        "struct S { int a:3; }; int f(void){ struct S s; return (int)sizeof s.a; }",
        "struct S { int a:3; }; int f(void){ struct S s; return (int)_Alignof(s.a); }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **Contract 20: one mistake, one sentence.** Both rows below reported twice before wave
    // 358 — the incomplete-type complaint stacked on top of the more specific one. gcc says
    // exactly one thing about each, and which one it says is the argument for suppressing the
    // general sentence rather than the specific: for `struct I a:3` it prints "bit-field `a` has
    // invalid type", because a bit-field could not have taken that type complete either.
    for (src, want) in [
        (
            "struct S { struct I a:3; };",
            "bit-field `a` has a non-integer type",
        ),
        (
            "struct S { __typeof__(nope) a:3; };",
            "`nope` was not declared",
        ),
        (
            "struct S { __typeof__(nope) a; };",
            "`nope` was not declared",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "one sentence for `{src}`"
        );
    }
    // And the incompleteness that **is** the whole mistake still reports: an `enum` with no
    // definition resolves to the same poison value internally, so a guard keyed on that value
    // rather than on "did resolving the type say anything" silences this row too.
    assert!(
        !diags("enum E; struct S { enum E m; };").is_empty(),
        "an incomplete enum member is still reported"
    );

    for good in [
        // Every integer spelling gcc takes, including the ones C's letter does not name.
        "struct S { char a:3; };",
        "struct S { signed char a:3; };",
        "struct S { short a:3; };",
        "struct S { long long a:3; };",
        "struct S { unsigned a:3; };",
        "struct S { _Bool a:1; };",
        "typedef int I; struct S { I a:3; };",
        "enum E { X=1 }; struct S { enum E a:2; };",
        "struct S { const volatile int a:3; };",
        // And an ordinary non-bit-field member of each refused type, so the rule is about
        // bit-fields rather than about the types.
        "struct S { float a; int *b; struct T { int x; } c; int d[2]; };",
        "struct S { int a:3; }; int f(void){ struct S s; return (int)sizeof(s); }",
        // A **plain member beside a bit-field** keeps its size and its address, so the rule is
        // about the field rather than about the struct that holds one.
        "struct S { int a:3; int b; }; int f(void){ struct S s; return (int)sizeof(s.b)+(int)_Alignof(s.b); }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **C 6.7.9p4: an object with static storage duration takes a constant initializer** — and
/// `static` inside a function has static storage duration just as a file-scope object does.
///
/// chiero already enforced this, keyed on **file scope** rather than on storage duration, so
/// every block-scope `static` went unchecked. The discriminator is the pair: `int n = 1; int
/// a[2] = {n,2};` at file scope was refused before this wave, and the same initializer on a
/// block-scope `static` was not.
///
/// The legal half is the larger half here and it is what keeps the rule from swallowing real
/// code: an address constant (`&v` for a static `v`), a null pointer, a string, `sizeof`, an
/// enumeration constant and ordinary arithmetic are all constant expressions.
#[test]
fn a_static_initializer_is_a_constant_expression() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A file-scope object read, directly and inside arithmetic.
        "static int g = 1; int f(void){ static int x = g; return x; }",
        "static int g = 1; int f(void){ static int x = g + 1; return x; }",
        // An automatic, and a parameter — neither exists when the object is initialized.
        "int f(void){ int y=1; static int x = y; return x; }",
        "int f(int n){ static int x = n; return x; }",
        // **The address of an automatic**, in each shape that reaches one: `&x`, a decayed
        // array name, an element, a member, a cast, and a *parameter* — which is automatic too
        // and arrives by a different path, being part of the function's type rather than an
        // item in its body.
        "int f(void){ int x; static int *p = &x; return *p; }",
        "int f(int n){ static int *p = &n; return *p; }",
        "int f(void){ int a[2]; static int *p = a; return *p; }",
        "int f(void){ int a[2]; static int *p = &a[1]; return *p; }",
        "int f(void){ int x; static int *p = (int *)&x; return *p; }",
        // **Offset from an automatic**, which arithmetic does not make constant.
        "int f(void){ int a[2]; static int *p = a + 1; return *p; }",
        "int f(void){ int x; static int *p = &x + 1; return *p; }",
        "struct S { int m; }; int f(void){ struct S s; static int *p = &s.m; return *p; }",
        // **An automatic shadowing a static**, which is the direction that must still report.
        "int f(void){ static int y; { int y; static int *p = &y; return *p; } }",
        // Inside an aggregate initializer, array and struct alike — the element is where the
        // non-constant sits, not the initializer as a whole.
        "static int g; int f(void){ static int a[2] = {1,g}; return a[0]; }",
        "struct S { int a; }; static int gg; int f(void){ static struct S s = { gg }; return s.a; }",
        // An automatic's address *inside* an aggregate. `check_static_init` recurses into the
        // list and asks each element, which is why `addresses_an_automatic` needs no list arm
        // of its own — a mutant that removed one survived, and deleting it was the fix.
        "int f(void){ int x; static int *a[1] = { &x }; return *a[0]; }",
        // Declared `extern` in the block, so the name resolves to an object with no address
        // known here either.
        "int f(void){ extern int g; static int x = g; return x; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        "int f(void){ static int x = 1; return x; }",
        "int f(void){ static int x = 1+2*3; return x; }",
        "int f(void){ static int a[2] = {1,2}; return a[0]; }",
        // **Address constants**: the address of a static object is one, and so is a null pointer.
        "static int v = 3; int f(void){ static int *p = &v; return *p; }",
        "int f(void){ static int *p = 0; return p==0; }",
        // The same six shapes rooted at a *static* object, which are address constants. This
        // pairing is the whole discriminator: a rule that fired on `&` would pass the bad half
        // above and fail every row here.
        "static int a[2]; int f(void){ static int *p = &a[1]; return *p; }",
        "static int a[2]; int f(void){ static int *p = a + 1; return *p; }",
        "static int v; int f(void){ static int *p = (int *)&v; return *p; }",
        "int f(void){ static int a[2]; static int *p = a; return *p; }",
        "int f(void){ static int x = 0; static int *p = &x; return *p; }",
        // **A static shadowing an automatic**, and a static declared after an inner block that
        // used the same name for an automatic. Both are address constants, and both were taken
        // by a version of this rule that only ever *added* names to the automatic set — the
        // mutant that stopped removing them survived until these two rows existed.
        "int f(void){ int y; { static int y = 0; static int *p = &y; return *p; } }",
        "int f(void){ { int y; } static int y = 0; static int *p = &y; return *p; }",
        // **A previous function's automatic does not leak into this one.** `f`'s local `y` is
        // gone by the time `g` is walked, and the `y` whose address `g` takes is the file-scope
        // object — an address constant. The mutant that dropped the per-function restore
        // survived every other row here.
        "int y;\nint f(void){ int y = 1; return y; }\nint g(void){ static int *p = &y; return *p; }",
        "int f(void){ static char s[4] = \"abc\"; return s[0]; }",
        "int f(void){ static int x = sizeof(int); return x; }",
        "enum E { A = 1 }; int f(void){ static int x = A; return x; }",
        // A parameter read *outside* the static's initializer stays legal — the rule is about
        // the initializer, not about the function.
        "int f(int n){ static int x = 0; return x + n; }",
        // And the non-static local beside it, which may read anything.
        "int f(int n){ int y = n; static int x = 2; return x + y; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A tag defined inside a parameter list is not a redefinition.** C 6.7.2.3 lets a parameter
/// list define one; its scope ends with the list, which is why gcc warns that it is useless
/// rather than refusing it.
///
/// This is a **false positive** rather than a miss, and it is the census's legal half that
/// found it: `int f(struct S { int a; } s);` was refused outright, and a *definition* was
/// refused twice — the parameter list is resolved once for the function's type and again when
/// its body is walked, so the second pass finds the tag the first pass installed.
///
/// The duplicate is the discriminator. A fix that only made the tag table forgiving would make
/// `struct S { int a; }; struct S { int b; };` legal too, which is why that row is pinned here
/// beside the accepted ones.
#[test]
fn a_tag_defined_in_a_parameter_list_is_not_a_redefinition() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for good in [
        "int f(struct S { int a; } s);",
        "int f(struct S { int a; } s) { return s.a; }",
        "int f(union U { int a; } u);",
        "int f(enum E { A } e);",
        "int f(enum E { A } e) { return (int)e; }",
        // The same tag in two parameter lists, which is two scopes rather than one.
        "int f(struct S { int a; } s); int g(struct S { int a; } s);",
        // And the ordinary shapes around it, which never had the problem.
        "struct S { int a; }; int f(struct S s) { return s.a; }",
        "int f(struct S *s);",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }

    // **A genuine redefinition still is one**, at file scope and in a body alike.
    for bad in [
        "struct S { int a; }; struct S { int b; };",
        "int f(void){ struct S { int a; }; struct S { int b; }; return 0; }",
        "enum E { A }; enum E { B };",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }
}

/// **C 6.7.3p2: `restrict` qualifies a pointer**, and nothing else.
///
/// The line is drawn after typedefs resolve — `typedef int *P; P restrict p;` is legal and
/// `typedef int T; T restrict x;` is not — and it is about the type the qualifier attaches to,
/// not the declaration's outermost type: `int *restrict a[2];` is an array of restricted
/// pointers and is fine, while `int restrict *p;` puts it on the `int` and is not.
#[test]
fn restrict_qualifies_only_a_pointer() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int restrict x;",
        "float restrict f2;",
        "int f(void){ int restrict x = 1; return x; }",
        "struct S { int restrict a; };",
        "int f(int restrict x);",
        "typedef int T; T restrict x;",
        // A typedef'd **array of non-pointers**, which is the only spelling that reaches the
        // qualifier with an array type in hand: written `int restrict a[2];` the `restrict`
        // lands on the `int` node instead, and never sees the array at all.
        "typedef int A[2]; A restrict a;",
        "struct S { int a; }; struct S restrict s;",
        // On the `int`, not on the pointer.
        "int restrict *p;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    // **Poison is not a non-pointer** (contract 20): a type that failed to resolve has already
    // been reported, and this rule adds nothing about it. Asserted as the *absence of this
    // rule's sentence* rather than as an exact list — the declaration also draws a standing
    // note that `x`'s uses go unchecked, which is older than this wave and not its business.
    let poisoned = diags("__typeof__(nope) restrict x;");
    assert!(
        !poisoned.is_empty() && !poisoned.iter().any(|m| m.contains("`restrict`")),
        "poison draws no `restrict` complaint: {poisoned:?}"
    );

    for good in [
        "int *restrict p;",
        "void *restrict v;",
        "const int *restrict p;",
        "typedef int *P; P restrict p;",
        // An array **of** restricted pointers, where the qualifier is on the element.
        "int *restrict a[2];",
        // The same through a typedef, where the array *is* what the qualifier is handed — so
        // the rule must look past it to the element. A version that asked about the array
        // itself answered every other row here correctly.
        "typedef int *PA[2]; PA restrict a;",
        "int f(int *restrict p, int *restrict q);",
        "int f(int *restrict);",
        "int f(void){ int a[2]; int *restrict p = a; return *p; }",
        "int f(void){ int a[2]; int (*restrict p)[2] = &a; return (*p)[0]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A prototype's parameter names are distinct, and `...` follows a named parameter.**
///
/// Two rules from C 6.7.6.3, both about the list rather than about a body. The first already
/// existed and was keyed on the **definition** path — the same mis-keying wave 358 found in the
/// constant-initializer check — so `int f(int x, int x);` was refused with a body and accepted
/// without one. The second (p4) is calibrated to `-pedantic-errors`: gcc takes a bare `f(...)`
/// under `-std=gnu11`.
#[test]
fn a_prototype_names_its_parameters_and_its_ellipsis() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(int x, int x);",
        "int f(int x, int y, int x);",
        "int f(...);",
        "int f(...) { return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Unnamed parameters do not collide with each other, however many there are.
        "int f(int, int);",
        "typedef int T; int f(T, T);",
        "int f(int x, int y);",
        "int f(int a, ...);",
        // Two *functions* may each use `x`, which is what the check must not confuse.
        "int f(int x); int g(int x);",
        "int f(int x) { return x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A label is defined once per function** (C 6.2.1p4: a label's scope is the whole function).
///
/// The scope is what makes this its own rule rather than a case of ordinary redeclaration: a
/// label ignores blocks entirely, so `a: { a: ; }` and two sibling blocks each defining `a`
/// both collide, while every other identifier in those positions would not.
///
/// The legal half is where the rule earns its shape. Two *functions* may each use `a`; a label
/// may share its name with an object, because labels have their own namespace; a label may be
/// defined and never jumped to; and one may sit inside a `switch` beside a `case`.
#[test]
fn a_label_is_defined_once_per_function() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(int x){ a: a: return x; }",
        // **Across a block boundary**, which is the case an ordinary scoped set would miss.
        "int f(int x){ a: { a: return x; } }",
        // And between two blocks that do not contain one another.
        "int f(int x){ if(x){ a: return 1; } a: return 0; }",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    for good in [
        // The same name in two functions, which is two scopes.
        "int f(int x){ a: return x; } int g(int x){ a: return x; }",
        // A label and an object may share a name: separate namespaces.
        "int f(int x){ int a=1; a: return a+x; }",
        // Defined and jumped to, defined and not, and defined inside a `switch`.
        "int f(int x){ a: goto a; return x; }",
        "int f(int x){ return x; unused: return 0; }",
        "int f(int x){ switch(x){ a: case 1: return 1; } goto a; return 0; }",
        "int f(int x){ goto a; { a: return x; } }",
        "int f(int x){ a: ; return x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Where C asks a value "is it true", the value must be scalar** — C 6.8.4.1p1 (`if`),
/// 6.8.5p2 (`while`, `do`, `for`), 6.5.15p2 (`?:`), 6.5.3.3p1 (`!`) and 6.5.13/14p2 (`&&`,
/// `||`). A structure or a union has no zero to compare against, and `void` has no value at all.
///
/// Eight contexts and one question, which is why they are one test: each of them ends up asking
/// the same thing about its operand, and a rule written in one place should reach all eight.
/// The legal half pins what "scalar" includes and is the half that keeps the rule honest — an
/// **array** is scalar for this purpose, because it decays to a pointer before the question is
/// asked, and a rule phrased as "integer or pointer" would reject `if(a)`.
#[test]
fn a_condition_needs_a_scalar() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    const S: &str = "struct S{int a;}; int f(void){ struct S s; ";

    for bad in [
        &format!("{S}if(s) return 1; return 0; }}"),
        &format!("{S}while(s) ; return 0; }}"),
        &format!("{S}do ; while(s); return 0; }}"),
        &format!("{S}for(;s;) ; return 0; }}"),
        &format!("{S}return s ? 1 : 0; }}"),
        &format!("{S}return !s; }}"),
        &format!("{S}return s && 1; }}"),
        &format!("{S}return 1 || s; }}"),
        // A union, which has the same problem for the same reason.
        "union U{int a;}; int f(void){ union U u; if(u) return 1; return 0; }",
        // **`void` is not scalar either**, and a call is how one reaches a condition.
        "void g(void); int f(void){ if(g()) return 1; return 0; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // Every scalar kind, in the contexts above.
        "int f(void){ double d=1; return !d; }",
        "int f(void){ int *p=0; return p && 1; }",
        "int f(void){ int *p=0; if(p) return 1; return 0; }",
        "enum E{A}; int f(void){ enum E e=A; return e ? 1 : 0; }",
        "int f(void){ _Bool b=1; while(b) return 1; return 0; }",
        // **An array decays before the question is asked**, so it is scalar here.
        "int f(void){ int a[2]; if(a) return 1; return 0; }",
        "int f(void){ int a[2]; return a ? 1 : 0; }",
        "int f(void){ int a[2]; return !a; }",
        // A scalar *member* of the struct that is not scalar itself.
        "struct S{int a;}; int f(void){ struct S s; if(s.a) return 1; return 0; }",
        // A function returning a value, so the rule is about `void` rather than about calls.
        "int g(void); int f(void){ if(g()) return 1; return 0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`.` takes a structure, `->` takes a pointer to one** (C 6.5.2.3p1–p2), and chiero treated
/// the two as interchangeable: `s->a` on a structure and `p.a` on a pointer were both accepted.
///
/// The rule is asked of the **decayed** operand, which is what wave 360's condition rule needed
/// too and for the same reason: `struct S a[2]; a->a` is legal, because the array is a pointer
/// by the time the question is put. A rule written against the operand as spelled would reject
/// it.
///
/// The legal half is where the typedef rows earn their place. `typedef struct S *SP; SP p;`
/// makes `p->a` right and `p.a` wrong even though neither spelling contains a `*`, so a rule
/// keyed on syntax rather than on the resolved type gets both backwards.
#[test]
fn dot_takes_a_structure_and_arrow_takes_a_pointer() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // `->` on a structure, and on a union.
        "struct S{int a;}; int f(void){ struct S s; return s->a; }",
        "union U{int a;}; int f(void){ union U u; return u->a; }",
        // `.` on a pointer, spelled with a `*` and hidden behind a typedef.
        "struct S{int a;}; int f(void){ struct S *p=0; return p.a; }",
        "union U{int a;}; int f(void){ union U *p=0; return p.a; }",
        "typedef struct S{int a;} *SP; int f(void){ SP p=0; return p.a; }",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    for good in [
        "struct S{int a;}; int f(void){ struct S s; return s.a; }",
        "struct S{int a;}; int f(void){ struct S *p=0; return p->a; }",
        "struct S{int a;}; int f(void){ struct S *p=0; return (*p).a; }",
        "union U{int a; char b;}; int f(void){ union U u; return u.a + u.b; }",
        // **An array of structures decays**, so `->` on it is right and `.` is not.
        "struct S{int a;}; int f(void){ struct S a[2]; return a->a; }",
        "struct S{int a;}; int f(void){ struct S a[2]; return a[0].a; }",
        // Both typedef spellings, which is what keeps the rule off syntax.
        "typedef struct S{int a;} S; int f(void){ S *p=0; return p->a; }",
        "typedef struct S{int a;} *SP; int f(void){ SP p=0; return p->a; }",
        // Qualified, cast, nested, and the value shapes a member may hang off.
        "struct S{int a;}; int f(void){ const struct S *p=0; return p->a; }",
        "struct S{int a;}; int f(void){ void *v=0; return ((struct S*)v)->a; }",
        "struct S{struct T{int x;} t;}; int f(void){ struct S s; return s.t.x; }",
        "struct S{int a;}; int f(void){ struct S **pp=0; return (*pp)->a; }",
        "struct S{int a;}; struct S g(void); int f(void){ return g().a; }",
        "struct S{int a;}; int f(void){ return (struct S){1}.a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A function specifier declares a function** (C 6.7.4p2): `inline` and `_Noreturn` may not be
/// written on an object, a parameter, a typedef, or a member.
///
/// Most rows are `-pedantic-errors` calibration — `-std=gnu11` takes `inline int x;` — but a
/// **member** is refused by gcc in both modes, so the rule is not uniformly a divergence.
///
/// This is the same shape as the storage-class question `check_storage_context` already answers,
/// asked in the same places, which is where it belongs rather than in a rule of its own.
#[test]
fn a_function_specifier_declares_a_function() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "inline int x;",
        "_Noreturn int x;",
        "int f(void){ inline int y=1; return y; }",
        "int f(void){ static inline int y=1; return y; }",
        "int f(inline int x);",
        "typedef inline int T;",
        "struct S{ inline int a; };",
        "struct S{ _Noreturn int a; };",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    for good in [
        // Every spelling on a function, which is the whole point of the specifier.
        "inline int f(void){ return 1; }",
        "static inline int f(void){ return 1; }",
        "extern inline int f(void){ return 1; }",
        "inline int f(int x){ return x; }",
        "inline int f(void); int f(void){ return 1; }",
        "_Noreturn void f(void){ for(;;); }",
        "inline _Noreturn void f(void){ for(;;); }",
        "_Noreturn void g(void); int f(void){ g(); return 0; }",
        // And an ordinary object beside one, so the rule is about the specifier.
        "inline int f(void){ return 1; } int x; int g(void){ return f()+x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A cast names a scalar type and takes a scalar operand** (C 6.5.4p2), and a pointer does not
/// convert to or from a floating type (p4).
///
/// Three parts of one paragraph, and the `void` exception is what ties them together: `(void)s`
/// on a structure is legal, because a cast to `void` discards its operand rather than converting
/// it. So the operand question is only asked when the target is *not* `void`, which is also why
/// a rule written as "both sides must be scalar" would reject the one spelling people actually
/// write with a struct.
///
/// The legal half decides the phrasing again: `(int)a` on an array and `(int)g` on a function
/// are both legal, because each decays before the question is put — the same load-bearing step
/// as waves 359, 360 and 361.
#[test]
fn a_cast_is_between_scalars() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A record operand, struct and union alike.
        "struct S{int a;}; int f(void){ struct S s; return (int)s; }",
        "union U{int a;}; int f(void){ union U u; return (int)u; }",
        // A record *target*, even from a scalar and even from its own type.
        "struct S{int a;}; int f(void){ struct S s; return (int)(struct S)s.a; }",
        // **Pointer and floating do not convert**, in either direction.
        "int f(void){ double d=1; return (int)(int*)d; }",
        "int f(void){ int *p=0; return (double)p != 0; }",
        // **A vector keeps its size and wants an integer on the other side.** `(int)a` from a
        // 16-byte vector is a size mismatch; `(double)a` is the right size and not an integer;
        // `(v2)p` is the right size and not an integer either. Measured against gcc after the
        // corpus gate refuted the first draft of this rule.
        "typedef int v4 __attribute__((vector_size(16))); int f(void){ v4 a={0}; return (int)a[0]+(int)a; }",
        "typedef int v2 __attribute__((vector_size(8))); double f(v2 a){ return (double)a; }",
        "typedef int v2 __attribute__((vector_size(8))); long f(v2 a){ int *p=0; return (long)(v2)p; }",
        "typedef int v4 __attribute__((vector_size(16))); typedef int v2 __attribute__((vector_size(8)));\nv4 f(v2 a){ return (v4)a; }",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    // **Each half named, not merely counted.** `(int)(struct S)s.a` draws one diagnostic
    // whichever half fires, so a mutant that deleted the *target* rule survived on it: the
    // outer cast then found a non-scalar operand and reported that instead. The rows below put
    // each half where only it can answer.
    assert_eq!(
        diags("struct S{int a;}; int f(void){ struct S s; (struct S)s.a; return 0; }"),
        vec!["a cast names a scalar type or `void`".to_string()],
        "a bare cast to a record names the target rule"
    );
    assert_eq!(
        diags("struct S{int a;}; int f(void){ struct S s; return (int)s; }"),
        vec!["a cast takes a scalar operand".to_string()],
        "a record operand names the operand rule"
    );

    for good in [
        // **`(void)` takes anything**, which is the exception the rule is built around.
        "struct S{int a;}; int f(void){ struct S s; (void)s; return 0; }",
        "struct S{int a;}; struct S g(void); int f(void){ (void)g(); return 0; }",
        "int f(void){ int x=1; return (void)x, 0; }",
        // The ordinary scalar conversions, including through a pointer-sized integer.
        "int f(void){ double d=1; return (int)d; }",
        "int f(void){ int x=1; return (int)(double)x; }",
        "int f(void){ int *p=0; return (int)p; }",
        "int f(void){ int x=1; return (int)(long)&x; }",
        "int f(void){ float g=1; int *p=0; return (int)(float)(long)p + (int)g; }",
        "int f(void){ int *p=0; return (_Bool)p; }",
        "int f(void){ double d=1; return (_Bool)d; }",
        // **Decay before the question**: an array and a function designator are both scalar here.
        "int f(void){ int a[2]; return (int)a; }",
        "int f(void){ int a[2]; return ((int(*)[2])a) != 0; }",
        "void g(void); int f(void){ return (int)g; }",
        // A qualified target, and a scalar member of the struct that is not scalar itself.
        "int f(void){ int x=1; return (const int)x; }",
        "struct S{int a;}; int f(void){ struct S s; return (int)s.a; }",
        // **Vector to vector**, which `vppinfra/bitmap.h` does and the corpus gate caught this
        // rule rejecting. Same width, different element type, is what gcc permits.
        "typedef int v4 __attribute__((vector_size(16))); int f(void){ v4 a={0}; v4 b=(v4)a; return b[0]; }",
        "typedef int v4 __attribute__((vector_size(16))); typedef long l2 __attribute__((vector_size(16)));\nint f(void){ v4 a={0}; l2 b=(l2)a; return (int)b[0]; }",
        // **Vector to a same-size integer**, which is what VPP writes and what refuted the
        // first draft: a two-lane vector to `uword`, and back.
        "typedef int v2 __attribute__((vector_size(8))); long f(v2 a){ return (long)a; }",
        "typedef int v2 __attribute__((vector_size(8))); v2 f(long a){ return (v2)a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`*` and `/` take arithmetic operands; `%` takes integer ones** (C 6.5.5p2).
///
/// Two rules in one paragraph, and the discriminator is `double`: it is arithmetic, so `d / 2`
/// is right and `d % 2` is wrong. A single "both must be arithmetic" rule accepts the second;
/// a single "both must be integer" rule rejects the first.
#[test]
fn multiplicative_operands_are_arithmetic() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A pointer in `*` and in `/`, on either side.
        "int f(void){ int *p=0; return (int)(p * 2); }",
        "int f(void){ int *p=0; return (int)(2 * p); }",
        "int f(void){ int *p=0; return (int)(p / 2); }",
        // **A floating operand in `%`**, which is the half `*` and `/` allow.
        "int f(void){ double d=2; return (int)(d % 2); }",
        "int f(void){ float g=2; return (int)(g % 2); }",
        "int f(void){ int x=2; double d=1; return (int)(x % d); }",
        "int f(void){ int *p=0; int *q=0; return (int)(p % q); }",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    // **An operand already reported as unusable draws no second sentence** (contract 20). The
    // declaration's own complaint stands; what must not appear beside it is `` `*` needs
    // arithmetic operands`` about the type the reader has already been told to fix.
    let cascade = diags("struct I; int f(void){ struct I x; return (int)(x * 2); }");
    assert_eq!(cascade.len(), 1, "one sentence, not two: {cascade:?}");

    for good in [
        "int f(void){ int x=5; return x % 2; }",
        "int f(void){ double d=2; return (int)(d * 2); }",
        "int f(void){ double d=1; return (int)(d / 2); }",
        "int f(void){ double d=2; float g=1; return (int)(d/g); }",
        // Every integer spelling `%` accepts, which is what keeps the rule off `Ty::Int`'s width.
        "int f(void){ _Bool b=1; return b % 2; }",
        "enum E{A=1}; int f(void){ enum E e=A; return e % 2; }",
        "int f(void){ char c=2; return c * 2; }",
        // **A vector is arithmetic here, and `%` takes one too** — gcc allows all three on an
        // integer vector, which is why `Vector` is not gated on `integer_only`.
        "typedef int v4 __attribute__((vector_size(16))); int f(void){ v4 a={0}; return (a % 2)[0] + (a * 2)[0]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **An ordinary identifier means one thing per scope** (C 6.7p3): a name may not be a `typedef`
/// here and an object there, and a block may not declare the same object twice.
///
/// This is a *mechanism* rather than a rule, which is why §9 held it for its own wave: nothing
/// in sema records what a name **means**, only what type it has. `conflicting types for `x``
/// already catches two objects that disagree, and that is the whole of what exists — so a name
/// that is a `typedef` in one declaration and an object in the next passes, in either order, at
/// either scope.
///
/// The legal half is most of the work and every row of it is load-bearing:
///
/// - **File scope is not block scope.** `int x; int x;` at file scope is two tentative
///   definitions and is how every header has always worked; the identical pair inside a function
///   is a redeclaration. A rule that does not separate them breaks C or breaks headers.
/// - **Tags are a separate namespace.** `struct S { int a; }; int S;` is legal both ways round,
///   and so is `typedef int T; struct T { int a; };` — a table keyed on the bare name gets all
///   four of those wrong.
/// - **`typedef struct S S;`** names a typedef after its own tag, which is the idiom this rule
///   most easily breaks.
/// - **Repeating a `typedef` with the same type is legal**, and with a different type is not.
#[test]
fn an_identifier_means_one_thing_per_scope() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // A `typedef` and an object, at file scope, both orders.
        "typedef int T; int T;",
        "int T; typedef int T;",
        // The same inside a block, both orders.
        "int f(void){ typedef int T; int T; return 0; }",
        "int f(void){ int T; typedef int T; return 0; }",
        // Two `typedef`s that disagree about the type.
        "typedef int T; typedef long T;",
        // **Two objects in one block**, which file scope permits and a block does not.
        "int f(void){ int x; int x; return x; }",
        // A `typedef` and a function, in every order including after the definition.
        "int f(void); typedef int f;",
        "typedef int f; int f(void);",
        "int f(void){ return 0; } typedef int f;",
        // **An enumerator is an ordinary identifier**, so it collides with objects.
        "enum E { A }; int A;",
        "int A; enum E { A };",
        "int f(void){ int x; enum E { x }; return 0; }",
        "int f(void){ enum E { x }; int x; return 0; }",
        // A parameter is in the body's scope, so a `typedef` of its name collides.
        "int f(int T){ typedef int T; return 0; }",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    for good in [
        // **File scope: tentative definitions**, which is how every header works.
        "int x; int x;",
        "int x = 1; int x;",
        "extern int x; extern int x;",
        "static int x; static int x;",
        // The same `typedef` twice, with the same type.
        "typedef int T; typedef int T;",
        // **Tags are a separate namespace**, in all four combinations.
        "struct S { int a; }; int S;",
        "int S; struct S { int a; };",
        "typedef int T; struct T { int a; };",
        "typedef struct S S; struct S { int a; }; int f(S *p){ return p->a; }",
        // **An inner scope may shadow anything**, with any meaning.
        "typedef int T; int f(void){ int T; return T; }",
        "typedef int T; int f(void){ typedef long T; T x = 1; return (int)x; }",
        "int f(void){ int x; { int x; return x; } }",
        // And the ordinary uses of a typedef, which the table must not disturb.
        "typedef int T; int f(T x){ return x; }",
        "typedef int T; T x; int f(void){ return x; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **What `+` and `-` accept** (C 6.5.6p2–p3), and what `==` does not (6.5.9p2).
///
/// `+` takes two arithmetic operands, or a pointer and an integer in either order. `-` takes
/// those, or **two pointers to compatible types**. Everything else — two pointers added, an
/// integer minus a pointer, a pointer plus a `double`, a structure on either side — is a
/// constraint violation, and chiero accepts all of them.
///
/// Two of the misses are worse than silence: `s + 1` on a structure reports "invalid
/// initializer: a structure or union is copied only from its own type" *and* a cast complaint
/// from an enclosing cast, neither of which names the mistake. This rule replaces both.
///
/// **The declared divergences must survive.** `void *` and function-pointer arithmetic are GNU
/// extensions this project implements on purpose (022 §4, and `sizeof(void) == 1`), so `v + 1`
/// and `g + 1` stay legal here even though `-pedantic-errors` refuses them. A rule phrased "the
/// pointee must be a complete object type" would take them out, which is why it is not.
#[test]
fn additive_operands_are_arithmetic_or_a_pointer_and_an_integer() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        // Two pointers added, and an integer minus a pointer — `-` is not commutative.
        "int f(void){ int *p=0; int *q=0; return (int)(p+q); }",
        "int f(void){ int *p=0; return (int)(1-p); }",
        // A pointer and a floating value, which is not an integer.
        "int f(void){ double d=1; int *p=0; return (int)(p+d); }",
        // **Two pointers subtracted with incompatible pointees.**
        "int f(void){ int *p=0; char *q=0; return (int)(p-q); }",
        // A structure on either side of either operator.
        "struct S{int a;}; int f(void){ struct S s; return (int)(s+1); }",
        "struct S{int a;}; int f(void){ struct S s; struct S t; return (int)(s-t); }",
        // **A record is not comparable** (6.5.9p2), struct and union, and against a constant.
        "struct S{int a;}; int f(void){ struct S s; struct S t; return s == t; }",
        "struct S{int a;}; int f(void){ struct S s; struct S t; return s != t; }",
        "union U{int a;}; int f(void){ union U u; union U v; return u == v; }",
        "struct S{int a;}; int f(void){ struct S s; return s == 0; }",
    ] {
        assert_eq!(diags(bad).len(), 1, "one sentence for `{bad}`");
    }

    // **Each message named, not merely counted.** A mutant that gave the additive and the
    // comparison case one shared sentence survived every row above: both still report exactly
    // once, and only the wording differs. Wave 362 earned this rule and this is its second catch.
    for (src, want) in [
        (
            "struct S{int a;}; int f(void){ struct S s; struct S t; return s == t; }",
            "a structure or union is not comparable",
        ),
        (
            "struct S{int a;}; int f(void){ struct S s; return (int)(s+1); }",
            "a structure or union is not an operand of `+` or `-`",
        ),
        (
            "int f(void){ int *p=0; int *q=0; return (int)(p+q); }",
            "two pointers cannot be added",
        ),
        (
            "int f(void){ int *p=0; char *q=0; return (int)(p-q); }",
            "subtracting pointers to incompatible types",
        ),
        (
            "int f(void){ int *p=0; return (int)(1-p); }",
            "an integer minus a pointer is not pointer arithmetic",
        ),
        (
            "int f(void){ double d=1; int *p=0; return (int)(p+d); }",
            "a pointer may only be offset by an integer",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // Pointer and integer, both orders, and every integer spelling.
        "int f(void){ int *p=0; return (int)(p+1); }",
        "int f(void){ int *p=0; return (int)(1+p); }",
        "int f(void){ int *p=0; return (int)(p-1); }",
        "int f(void){ int *p=0; _Bool b=1; return (int)(p+b); }",
        "int f(void){ int *p=0; char c=1; return (int)(p+c); }",
        "enum E{A=1}; int f(void){ int *p=0; enum E e=A; return (int)(p+e); }",
        // Two pointers subtracted, where the pointees agree **through qualifiers, typedefs and
        // a decayed array** — none of which makes them incompatible.
        "int f(void){ int *p=0; int *q=0; return (int)(p-q); }",
        "typedef int T; int f(void){ int *p=0; T *q=0; return (int)(p-q); }",
        "int f(void){ const int *p=0; int *q=0; return (int)(p-q); }",
        "int f(void){ int a[2]; int *p=0; return (int)(a-p); }",
        "int f(void){ int a[2]; return (int)(a+1); }",
        // Ordinary arithmetic, which the rule must not disturb.
        "int f(void){ double d=1; return (int)(d+1); }",
        "int f(void){ double d=1; return (int)(d-1); }",
        // **The declared GNU divergences**: `void *` and function-pointer arithmetic.
        "int f(void){ void *v=0; return (int)(v+1); }",
        "void g(void); int f(void){ return (int)(g+1); }",
        // And the comparisons that stay legal beside the record rule.
        "int f(void){ int *p=0; int *q=0; return p == q; }",
        "int f(void){ double d=1; return d == 1; }",
        "int f(void){ int a[2]; return a == a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A subscript is an integer, and `++` needs something it can add one to** (C 6.5.2.1p1,
/// 6.5.3.1p1, 6.5.2.4p1).
///
/// chiero checks the *subscripted* value and not the subscript, so `a[d]` on a `double` and
/// `a[p]` on a pointer both pass — the second being `*(a + p)`, which wave 364 has just made a
/// constraint violation when written that way round.
///
/// `++` and `--` check only that the operand is modifiable; whether it is a *kind of thing* one
/// can add one to is unasked, so a structure, an array and a function designator all increment
/// silently.
///
/// **`void *` stays legal** on both counts: `v++` and `v[1]` are `-pedantic-errors` violations
/// and GNU extensions this project implements (022 §4), so the rule is phrased on the operand's
/// kind rather than on its pointee's size.
#[test]
fn a_subscript_is_an_integer_and_an_increment_needs_a_scalar() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        // **The subscript**, which nothing was looking at.
        (
            "int f(void){ int a[2]; double d=0; return a[d]; }",
            "a subscript is an integer",
        ),
        (
            "int f(void){ int a[2]; int *p=0; return a[p]; }",
            "a subscript is an integer",
        ),
        (
            "struct S{int a;}; int f(void){ int a[2]; struct S s; return a[s]; }",
            "a subscript is an integer",
        ),
        // **The commuted form**, where the non-integer is on the *left*. `0[a]` is legal C, so
        // the rule asks about whichever operand is not the pointer — and a mutant that only ever
        // looked right survived every row above.
        (
            "int f(void){ int a[2]; double d=0; return d[a]; }",
            "a subscript is an integer",
        ),
        // **The operand of `++`/`--`**: a record, an array and a function are none of them
        // things one adds one to.
        (
            "struct S{int a;}; int f(void){ struct S s; s++; return s.a; }",
            "`++` needs a scalar operand",
        ),
        (
            "struct S{int a;}; int f(void){ struct S s; s--; return s.a; }",
            "`--` needs a scalar operand",
        ),
        (
            "int f(void){ union U{int a;} u; u++; return u.a; }",
            "`++` needs a scalar operand",
        ),
        (
            "int f(void){ int a[2]; a++; return a[0]; }",
            "`++` needs a scalar operand",
        ),
        (
            "void g(void); int f(void){ g++; return 0; }",
            "`++` needs a scalar operand",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // Every integer spelling of a subscript, and the commuted form.
        "int f(void){ int a[2]; return a[0]; }",
        "int f(void){ int a[2]; return 0[a]; }",
        "int f(void){ int *p=0; return p[0]; }",
        "int f(void){ int a[2]; _Bool b=0; return a[b]; }",
        "int f(void){ int a[2]; char c=0; return a[c]; }",
        "enum E{A}; int f(void){ int a[2]; enum E e=A; return a[e]; }",
        "struct S{int a;}; int f(void){ int a[2]; struct S s; return a[s.a]; }",
        // Every scalar kind incremented, prefix and postfix.
        "int f(void){ int x=1; x++; return x; }",
        "int f(void){ int x=1; return ++x; }",
        "int f(void){ int *p=0; return (int)p--; }",
        "int f(void){ double d=1; return (int)d++; }",
        "int f(void){ _Bool b=0; b++; return b; }",
        "int f(void){ _Bool b=1; b--; return b; }",
        "enum E{A}; int f(void){ enum E e=A; e++; return (int)e; }",
        // A scalar **member** of a record that is not scalar itself.
        "struct S{int a;}; int f(void){ struct S s; return ++s.a; }",
        // **The declared GNU divergences.**
        "int f(void){ void *v=0; v++; return v!=0; }",
        "int f(void){ void *v=0; return (int)&v[1]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`_Alignas` goes on an object, and names a power of two at least as strict as the type's own**
/// (C 6.7.5p2–p5).
///
/// Four rules from one paragraph, and gcc refuses every row in *both* modes — none of this is
/// pedantic calibration. VPP contains no `_Alignas` at all, so there is no divergence to weigh
/// either: this is the rare census where the corpus has nothing to say.
///
/// The legal half pins the two easy mistakes: `_Alignas(0)` is explicitly *no* effect rather than
/// an error, and an alignment **stricter** than the type's own is the entire point — `_Alignas(4)
/// char` is fine and `_Alignas(1) int` is not.
#[test]
fn alignas_goes_on_an_object_and_names_a_power_of_two() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        (
            "int f(_Alignas(8) int x);",
            "`_Alignas` is not allowed on a parameter",
        ),
        (
            "typedef _Alignas(8) int T;",
            "`_Alignas` is not allowed on a `typedef`",
        ),
        ("_Alignas(3) int x;", "an alignment must be a power of two"),
        (
            "int f(void){ _Alignas(3) int x=1; return x; }",
            "an alignment must be a power of two",
        ),
        (
            "struct S{ _Alignas(3) int a; };",
            "an alignment must be a power of two",
        ),
        (
            "_Alignas(1) int x;",
            "an alignment may not be weaker than the type's own",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    // **And it points at the specifier**, not merely at something. The blanket gate in
    // `generated_rejection` asks whether a span covers any text; this asks whether it covers the
    // right text, which is the difference between a diagnostic a reader can follow and one that
    // merely compiles. Both spans were zero-width until wave 373 built a way to look.
    for (src, want) in [
        ("_Alignas(3) int x;", "_Alignas(3)"),
        ("int f(_Alignas(8) int x);", "_Alignas(8)"),
        ("_Alignas(1) int x;", "_Alignas(1)"),
    ] {
        let tu = chiero_pp::preprocess_str("t.c", src, chiero_pp::Config::default());
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        let analysis = chiero_sema::analyze(
            &parsed.ast,
            &TargetConfig::x86_64_linux(),
            &harness::names_of(&parsed),
        );
        let covered: Vec<&str> = analysis
            .diagnostics
            .iter()
            .filter_map(|d| tu.source_map.span_text(d.span))
            .collect();
        assert_eq!(covered, vec![want], "the span for `{src}`");
    }

    for good in [
        "_Alignas(8) int x;",
        "int f(void){ _Alignas(8) int x=1; return x; }",
        "struct S{ _Alignas(8) int a; };",
        "_Alignas(16) struct S{int a;} s;",
        // **Zero is no effect, not an error**, and equal or stricter is the point of the feature.
        "_Alignas(0) int x;",
        "_Alignas(4) int x;",
        "_Alignas(8) long y;",
        "_Alignas(64) int x;",
        "_Alignas(4) char c;",
        "_Alignas(2) char c;",
        // The type-name spelling, which names an alignment rather than a number.
        "_Alignas(int) int x;",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A `_Generic` association names a complete object type** (C 6.5.1.1p2), and it may not be
/// variably modified.
///
/// chiero already matches associations, rejects two that match, rejects two `default`s and
/// rejects none matching — everything about the *selection*. What it never asks is whether the
/// type named is one an object could have, so `void`, an incomplete tag, a function type and a
/// VLA all pass.
///
/// The legal half is what stops the rule from being "a scalar": `int[3]`, a pointer and a
/// qualified type are all complete object types and all legal here.
#[test]
fn a_generic_association_names_a_complete_object_type() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        (
            "int f(void){ return _Generic(1, void: 1, default: 0); }",
            "a `_Generic` association needs a complete object type",
        ),
        (
            "struct I; int f(void){ return _Generic(1, struct I: 1, default: 0); }",
            "a `_Generic` association needs a complete object type",
        ),
        (
            "int f(void){ return _Generic(1, int(void): 1, default: 0); }",
            "a `_Generic` association needs a complete object type",
        ),
        (
            "int f(int n){ return _Generic(1, int[n]: 1, default: 0); }",
            "a `_Generic` association may not be variably modified",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    // **Poison draws no second sentence** (contract 20): a type that failed to resolve has
    // already been reported, and calling it incomplete would be this rule inventing a fact about
    // a type it made up. A mutant that dropped the exclusion survived every row above.
    let poisoned = diags("int f(void){ return _Generic(1, __typeof__(nope): 1, default: 0); }");
    assert!(
        !poisoned.is_empty() && !poisoned.iter().any(|m| m.contains("`_Generic`")),
        "poison draws no `_Generic` complaint: {poisoned:?}"
    );

    for good in [
        "int f(void){ return _Generic(1, int: 1, default: 0); }",
        "int f(void){ return _Generic(1, int: 1); }",
        "int f(void){ return _Generic(1, int[3]: 1, default: 0); }",
        "int f(void){ return _Generic(1, int*: 1, default: 0); }",
        "int f(void){ return _Generic(1, const int: 1, default: 0); }",
        "struct S{int a;}; int f(void){ return _Generic(1, struct S: 1, default: 0); }",
        "int f(void){ int a[2]; return _Generic(a, int*: 1, default: 0); }",
        "int f(void){ return _Generic(1, char: 1, long: 2, default: 0); }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A flexible array member needs a member before it, and a union may not have one**
/// (C 6.7.2.1p18).
///
/// chiero had the "must be last" half and neither of these. The decoration half of 6.7.6.2 lives
/// in `chiero-parse`'s constraints test instead — `static` inside `[]` never reaches 014, which
/// is the right layering and cost this wave a test-file move to notice.
///
/// `int a[0]` stays legal here, being this project's declared divergence with 1777 uses in VPP.
#[test]
fn a_flexible_array_member_needs_a_member_before_it() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        // **A flexible array member needs something before it**, and a union may not have one.
        (
            "struct S { int a[]; };",
            "a flexible array member needs a member before it",
        ),
        (
            "union U { int a; int b[]; };",
            "a union may not have a flexible array member",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // Ordinary arrays, including the declared `int a[0]` divergence and a VLA.
        "int f(void){ int a[3]; return a[0]; }",
        "int a[2];",
        "int a[0];",
        "int f(void){ int a[0]; return 0; }",
        "int f(int n){ int a[n]; return a[0]; }",
        // A flexible array member with a member before it, declared and used.
        "struct S { int a; int b[]; };",
        "struct S { int a; int b[]; }; int f(void){ struct S s; return s.a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A member has an object type, and an anonymous member's names join the enclosing record**
/// (C 6.7.2.1p3, p13).
///
/// A function is not an object, so `struct S { int f(void); }` declares something that cannot
/// exist — chiero takes it, and `has_no_size` does not catch it because a function type is not
/// *incomplete*, it is not an object type at all. That is the same distinction wave 339 drew for
/// `sizeof`, in a place that never got it.
///
/// An **anonymous** struct or union member has no name of its own, so C 6.7.2.1p13 puts its
/// members into the enclosing record's namespace — which is what makes `s.a` work, and what
/// makes `a` collide with a sibling. The discriminator is the *named* nested member: `struct {
/// int a; } n;` puts nothing in the enclosing namespace, so `int a;` beside it is fine.
#[test]
fn a_member_is_an_object_and_an_anonymous_one_shares_its_names() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        // A function type, spelled directly and through a typedef, in a struct and a union.
        (
            "struct S { int f(void); int a; };",
            "a member may not have a function type",
        ),
        (
            "union U { void g(void); };",
            "a member may not have a function type",
        ),
        (
            "typedef int F(void); struct S { F a; };",
            "a member may not have a function type",
        ),
        // **An anonymous member's names are the enclosing record's**, so they collide there.
        (
            "struct S { struct { int a; }; int a; };",
            "duplicate member `a`",
        ),
        (
            "struct S { struct { int a; }; struct { int a; }; };",
            "duplicate member `a`",
        ),
        (
            "struct S { union { int a; }; union { int a; }; };",
            "duplicate member `a`",
        ),
        // Through **two** levels of anonymity, which is what makes this recursive.
        (
            "struct S { struct { struct { int a; }; }; int a; };",
            "duplicate member `a`",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // A **pointer** to a function is an object, and is how one writes this.
        "struct S { int (*f)(void); };",
        "typedef int F(void); struct S { F *a; };",
        // Anonymous members whose names do not collide, in either order and either kind.
        "struct S { struct { int a; }; struct { int b; }; };",
        "struct S { struct { int a; }; union { int b; }; };",
        "struct S { struct { int a; }; int b; };",
        "struct S { int b; struct { int a; }; };",
        // **A named nested member contributes nothing** to the enclosing namespace.
        "struct S { struct { int a; } n; int a; };",
        // And the uses that make the promotion worth having.
        "struct S { struct { int a; }; }; int f(void){ struct S s; return s.a; }",
        "struct S { union { int a; float b; }; }; int f(void){ struct S s; return s.a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Two string literals with different prefixes do not concatenate** (C 6.4.5p5).
///
/// One prefixed and one plain is fine and the result takes the prefix — `"a" L"b"` is a wide
/// string — so the rule is about *two different* prefixes rather than about disagreeing with
/// plain. chiero concatenates anything, and `u"a" U"b"` produced a string whose only complaint
/// came from the type it was later assigned to.
///
/// `u8` counts as a prefix for this and not for the element width: `u8"a" "b"` is legal and
/// `L"a" u8"b"` is not, which is why the rule cannot be phrased on the element type.
#[test]
fn string_literals_concatenate_only_with_a_compatible_prefix() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for bad in [
        "int f(void){ return (int)sizeof(u\"a\" L\"b\"); }",
        "int f(void){ return (int)sizeof(u\"a\" U\"b\"); }",
        "int f(void){ return (int)sizeof(L\"a\" u8\"b\"); }",
        "int f(void){ return (int)sizeof(U\"a\" u\"b\"); }",
        // **`u8` against `u`**, which is the pair that distinguishes the prefix from the element
        // width and from a prefix match by first letter. A mutant reading `u8` as `u` survived
        // every other row.
        "int f(void){ return (int)sizeof(u\"a\" u8\"b\"); }",
        "int f(void){ return (int)sizeof(u8\"a\" u\"b\"); }",
    ] {
        assert_eq!(
            diags(bad),
            vec!["string literals with different prefixes do not concatenate".to_string()],
            "the message for `{bad}`"
        );
    }

    for good in [
        // The same prefix, and a prefix beside a plain literal in either order.
        "int f(void){ return (int)sizeof(u\"a\" u\"b\"); }",
        "int f(void){ return (int)sizeof(L\"a\" L\"b\"); }",
        "int f(void){ return (int)sizeof(U\"a\" U\"b\"); }",
        "int f(void){ return (int)sizeof(u8\"a\" u8\"b\"); }",
        "int f(void){ return (int)sizeof(\"a\" L\"b\"); }",
        "int f(void){ return (int)sizeof(L\"a\" \"b\"); }",
        "int f(void){ return (int)sizeof(u8\"a\" \"b\"); }",
        // Three plain literals, and a single one of each prefix.
        "int f(void){ return (int)sizeof(\"a\" \"b\" \"c\"); }",
        "char *f(void){ return \"a\" \"b\"; }",
        "int f(void){ return (int)sizeof(L\"a\"); }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A designator names a member the record actually shows, and an index is not negative**
/// (C 6.7.9p6–p7).
///
/// chiero reports `.b` on `struct S { int a; }` and stops reporting the moment the record
/// contains a *nested* member of any kind. Three shapes fall out of that, and only the first is
/// the anonymous-promotion gap wave 367 found on the member side:
///
/// - `.c` where the record's only members come from an anonymous union — nothing matches and
///   nothing is said.
/// - `.d` beside an anonymous struct *and* a named sibling — the sibling proves lookup works.
/// - `.a` naming a member of a **named** nested struct, which the enclosing record does not show
///   at all. That one is the opposite failure: too permissive rather than too quiet, and it is
///   why the rule is "what the record shows" rather than "any member anywhere below".
///
/// `visible_names` already answers exactly that question — wave 367 built it for the duplicate
/// check — so this is the same traversal asked a third time.
#[test]
fn a_designator_names_a_visible_member() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        (
            "struct S{int a;}; int f(void){ struct S s = { .b = 1 }; return s.a; }",
            "no member named `b` to initialize",
        ),
        (
            "struct S{ union { int a; int b; }; }; int f(void){ struct S s = { .c = 1 }; return s.a; }",
            "no member named `c` to initialize",
        ),
        (
            "struct S{ struct { int a; }; int c; }; int f(void){ struct S s = { .d = 1 }; return s.c; }",
            "no member named `d` to initialize",
        ),
        (
            "struct S{ struct { int a; } n; }; int f(void){ struct S s = { .a = 1 }; return s.n.a; }",
            "no member named `a` to initialize",
        ),
        // **A negative index**, directly and under a member designator.
        (
            "int f(void){ int a[2] = { [-1] = 1 }; return a[0]; }",
            "initializer index is negative",
        ),
        (
            "struct S{int a[2];}; int f(void){ struct S s = { .a[-1] = 1 }; return s.a[0]; }",
            "initializer index is negative",
        ),
        // **Where the cursor lands after a promoted designator.** `.a` names the *union*, which
        // is member 1, so the positional `2` beside it has nowhere to go. Every other row here
        // puts the anonymous member first, where "field 0" is right by accident — a mutant that
        // ignored promotion when placing the cursor survived all of them.
        (
            "struct S { int c; union { int a; int b; }; }; struct S s = { .a = 1, 2 };",
            "excess elements in a struct initializer",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // **Every name the record shows**, promoted or not.
        "struct S{ union { int a; int b; }; }; int f(void){ struct S s = { .b = 1 }; return s.a; }",
        "struct S{ struct { int a; }; int c; }; int f(void){ struct S s = { .c = 1 }; return s.c; }",
        "struct S{ struct { int a; }; int c; }; int f(void){ struct S s = { .a = 1 }; return s.c; }",
        "struct S{int a;}; int f(void){ struct S s = { .a = 1 }; return s.a; }",
        // A named nested member, designated **by its own name** and then descended into.
        "struct S{ struct { int a; } n; }; int f(void){ struct S s = { .n = { .a = 1 } }; return s.n.a; }",
        // Indices in range, and the ordinary shapes beside them.
        "int f(void){ int a[2] = { [0] = 1, [1] = 2 }; return a[0]; }",
        "int f(void){ int a[2] = { [1] = 1 }; return a[0]; }",
        "struct S{int a[2];}; int f(void){ struct S s = { .a[1] = 1 }; return s.a[0]; }",
        "int f(void){ char s[4] = { \"abc\" }; return s[0]; }",
        "struct S{int a;}; int f(void){ struct S s = { .a = 1, .a = 2 }; return s.a; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`<<`, `>>`, `&`, `^` and `|` take integer operands** (C 6.5.7p2, 6.5.10–6.5.12p2).
///
/// Five operators and one question, which is the same question `%` already asks — wave 362 built
/// that arm for the multiplicative operators and the shift and bitwise ones never joined it.
/// A floating operand, a pointer and a record all pass today.
///
/// **A record produces the wrong message rather than none**, which is why those rows assert what
/// is said: `s ^ 1` reports "a structure or union is copied only from its own type" *and* a cast
/// complaint from the enclosing cast, neither naming the operator. Wave 364 fixed exactly this
/// for `+`, `-` and the comparisons and the record arm was keyed on those; this wave extends the
/// key rather than adding a second arm.
///
/// **An integer vector is an integer here.** gcc takes `v << 1`, `v & 1` and `v | w` on
/// `vector_size` integers and refuses them on a float vector, so the rule reads the element type
/// — and VPP shifts vectors throughout `vppinfra`, so getting this wrong breaks the corpus rather
/// than the suite.
#[test]
fn shift_and_bitwise_operands_are_integers() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        // A floating operand, on each side of a shift.
        (
            "int f(void){ double d=1; return (int)(d << 1); }",
            "`<<` needs integer operands",
        ),
        (
            "int f(void){ int x=1; double d=1; return (int)(x << d); }",
            "`<<` needs integer operands",
        ),
        (
            "int f(void){ int x=1; return (int)(x >> 1.0); }",
            "`>>` needs integer operands",
        ),
        // A pointer, in a shift and in each bitwise operator.
        (
            "int f(void){ int *p=0; return (int)(p << 1); }",
            "`<<` needs integer operands",
        ),
        (
            "int f(void){ int *p=0; return (int)(p & 1); }",
            "`&` needs integer operands",
        ),
        (
            "int f(void){ int *p=0; int *q=0; return (int)(p | q); }",
            "`|` needs integer operands",
        ),
        // Floating operands of the bitwise operators.
        (
            "int f(void){ double d=1; return (int)(d & 1); }",
            "`&` needs integer operands",
        ),
        (
            "int f(void){ double d=1; return (int)(d ^ 1); }",
            "`^` needs integer operands",
        ),
        // **A record**, where the mistake was reported as something else entirely.
        (
            "struct S{int a;}; int f(void){ struct S s; return (int)(s << 1); }",
            "a structure or union is not an operand of `<<`, `>>`, `&`, `^` or `|`",
        ),
        (
            "struct S{int a;}; int f(void){ struct S s; return (int)(s ^ 1); }",
            "a structure or union is not an operand of `<<`, `>>`, `&`, `^` or `|`",
        ),
        // **A float vector**, which gcc refuses where it takes an integer one.
        (
            "typedef float f4 __attribute__((vector_size(16)));\nint f(void){ f4 a={0}; return (int)(a & 1)[0]; }",
            "`&` needs integer operands",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // Every integer spelling, on both sides, for each operator.
        "int f(void){ int x=1; return x << 1; }",
        "int f(void){ int x=1; return x >> 1; }",
        "int f(void){ char c=1; return c << 8; }",
        "int f(void){ _Bool b=1; return b & 1; }",
        "enum E{A=1}; int f(void){ enum E e=A; return e << 1; }",
        "int f(void){ int x=1; return x ^ 1; }",
        "int f(void){ int x=1; return x | 1; }",
        "int f(void){ long x=1; return (int)(x << 40); }",
        // Counts C leaves undefined but does not *constrain* — this is a rule about types.
        "int f(void){ int x=1; return x << 32; }",
        "int f(void){ int x=1; return x << -1; }",
        // **Integer vectors**, which VPP writes throughout `vppinfra`.
        "typedef int v4 __attribute__((vector_size(16)));\nint f(void){ v4 a={0}; return (a << 1)[0]; }",
        "typedef int v4 __attribute__((vector_size(16)));\nint f(void){ v4 a={0}; return (a & 1)[0]; }",
        "typedef int v4 __attribute__((vector_size(16)));\nint f(void){ v4 a={0}; v4 b={0}; return (a | b)[0]; }",
        "typedef int v4 __attribute__((vector_size(16)));\nint f(void){ v4 a={0}; return (a >> 1)[0]; }",
        // A float vector *multiplied*, which stays legal — the rule is these five operators.
        "typedef float f4 __attribute__((vector_size(16)));\nint f(void){ f4 a={0}; return (int)(a * a)[0]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Calling a function whose return type is incomplete** (C 6.5.2.2p1), and **three messages
/// that name the wrong thing**.
///
/// The census half is one row: `struct I; struct I g(void); g();` is refused by gcc in both modes
/// and taken here. A *declaration* returning an incomplete type is legal — nothing is produced
/// until there is a call — which is the same distinction wave 359 drew for parameters, and it is
/// why the check belongs at the call rather than at the declarator.
///
/// The rest of this test is 023 §9's other half. A report a person cannot act on is not a report,
/// and a report that names the wrong thing is worse than none: it sends a reader somewhere the
/// fault is not. Three have accumulated with the census recording them rather than fixing them,
/// and each is a *different* way to be wrong:
///
/// - **The wrong operand kind.** `d == p` said "a pointer and an integer" of a `double`; the
///   arm had two cases and one message.
/// - **The wrong rule.** `int f(const void)` said `void` must be the only parameter — it *is* the
///   only parameter, and the fault is the qualifier.
/// - **The wrong construct.** `#include <stdio.h> extra` said "invalid computed include"; the
///   include is perfectly well formed and the tokens after it are not. That one is `chiero-pp`'s
///   and is pinned in its own constraints test.
#[test]
fn a_diagnostic_names_the_thing_that_is_wrong() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        // **The census miss.** A call whose result cannot exist.
        (
            "struct I; struct I g(void); int f(void){ g(); return 0; }",
            "calling `g` produces an incomplete type",
        ),
        // **The wrong operand kind.** `double` is not an integer, and the message said it was.
        (
            "int f(void){ double d=1; int *p=0; return d == p; }",
            "comparison between a pointer and a floating value",
        ),
        (
            "int f(void){ double d=1; int *p=0; return d < p; }",
            "comparison between a pointer and a floating value",
        ),
        // **The pointer on the left**, which is what makes the message read the *other* side
        // rather than a fixed one. A mutant that always looked left survived both rows above.
        (
            "int f(void){ double d=1; int *p=0; return p == d; }",
            "comparison between a pointer and a floating value",
        ),
        (
            "int f(void){ int x=1; int *p=0; return x == p; }",
            "comparison between a pointer and an integer",
        ),
        // **The wrong rule.** The `void` *is* the only parameter; the qualifier is the fault.
        (
            "int f(const void);",
            "`void` as the only parameter may not be qualified",
        ),
        (
            "int f(volatile void);",
            "`void` as the only parameter may not be qualified",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    // **The rule each message replaced still fires where it should.** A pointer against a genuine
    // integer keeps the old sentence, and a `void` beside another parameter keeps the old rule —
    // these are what would break if a message were changed by widening its arm instead of
    // splitting it.
    for (src, want) in [
        (
            "int f(void){ int x=1; int *p=0; return p == x; }",
            "comparison between a pointer and an integer",
        ),
        ("int f(void, int x);", "`void` must be the only parameter"),
        ("int f(int x, void);", "`void` must be the only parameter"),
        // **Both faults at once**, which is the only row where the count decides: the `void` is
        // qualified *and* not alone, and gcc names the second. A mutant that dropped the count
        // test was masked by the qualifier test on every other row.
        (
            "int f(const void, int x);",
            "`void` must be the only parameter",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // A declaration returning an incomplete type is legal until it is called.
        "struct I; struct I g(void); int f(void){ return 0; }",
        // Completed before the call, which is the ordinary shape in a header pair.
        "struct I { int a; }; struct I g(void); int f(void){ return g().a; }",
        "struct S{int a;}; struct S g(void); int f(void){ struct S s = g(); return s.a; }",
        // `void` as the only parameter, unqualified, and an ordinary qualified parameter.
        "int f(void);",
        "int f(const int x);",
        // Comparisons that stay legal.
        "int f(void){ double d=1; return d == 1; }",
        "int f(void){ int *p=0; return p == 0; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A `case` label is a jump, and a `typedef` can be variably modified** (C 6.8.6.1p1), and
/// **`~` wants an integer vector** (6.5.3.3p1).
///
/// Three misses that all sit beside a rule already written, which is why they are one test.
///
/// Wave 341 built the `goto`-into-a-VLA check and keyed it on `goto`. A `case` label is reached
/// by a jump from the `switch` that is exactly as able to skip a declaration, and gcc refuses it
/// for the same reason — the array's length is computed by a statement the jump did not run.
/// The discriminator is the *braced* case: `case 1: { int a[n]; }` closes the scope before the
/// next label, so nothing is skipped.
///
/// A **variably modified `typedef`** opens the same scope as an object of that type: `typedef int
/// T[n]` evaluates `n` once, where the declaration stands. chiero's scope tracking sees objects
/// and not typedefs.
///
/// And `~` takes any vector where wave 371 made `&`, `^` and `|` take only integer ones — the
/// same paragraph, the same element question, and the unary arm was written before the binary one.
#[test]
fn a_case_label_jumps_and_a_typedef_can_be_variably_modified() {
    let diags = |src: &str| {
        let p = harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux());
        p.analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };

    for (src, want) in [
        // A `case` reached past a variably-modified declaration, and a `default` likewise.
        (
            "int f(int n){ switch(n){ case 1: ; int a[n]; case 2: return 0; } return 0; }",
            "a `case` label enters the scope of a variably-modified declaration",
        ),
        // **The `default` as the label that enters**, not merely present in the switch: written
        // the other way round the following `case` reports first and the `default` arm is never
        // reached. A mutant that deleted it survived until this row said `default:` last.
        (
            "int f(int n){ switch(n){ case 1: ; int a[n]; default: return 0; } return 0; }",
            "a `default` label enters the scope of a variably-modified declaration",
        ),
        // A `goto` past a variably-modified **typedef**, which declares no object at all.
        (
            "int f(int n){ goto skip; typedef int T[n]; skip: return 0; }",
            "jump to label `skip` enters the scope of a variably-modified declaration",
        ),
        // `~` on a floating vector, which wave 371 refuses for `&`, `^` and `|`.
        (
            "typedef float f4 __attribute__((vector_size(16)));\nint f(void){ f4 a={0}; return (int)(~a)[0]; }",
            "`~` needs an integer operand",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    // **A record operand is reported once.** `-s` said "unary `-` needs an arithmetic operand"
    // *and* drew a cast complaint from the enclosing cast, because the unary result kept the
    // record's type. Contract 20, and the same poison the binary arms already apply.
    assert_eq!(
        diags("struct S{int a;}; int f(void){ struct S s; return (int)-s; }"),
        vec!["unary `-` needs an arithmetic operand".to_string()],
        "one sentence for a record under unary `-`"
    );

    for good in [
        // **Braced**, so the scope closes before the next label and nothing is skipped.
        "int f(int n){ switch(n){ case 1: { int a[n]; } case 2: return 0; } return 0; }",
        // No later label, so no jump can land past the declaration.
        "int f(int n){ switch(n){ case 1: ; int a[n]; return a[0]; } return 0; }",
        // A fixed length is not variably modified, however it is declared.
        "int f(int n){ switch(n){ case 1: ; int a[2]; case 2: return 0; } return 0; }",
        "int f(int n){ goto skip; typedef int T[2]; skip: return 0; }",
        // **Nested switches at different depths.** The inner one begins with a scope already
        // open, so its labels enter nothing; a check that read the *outermost* switch's depth
        // rather than the innermost would reject this, and no other row distinguishes them.
        "int f(int n){ switch(n){ case 1: { int a[n]; switch(n){ case 2: return 0; case 3: return a[0]; } } } return 0; }",
        // **A `switch` inside an existing variably-modified scope.** The scope was open before
        // the switch began, so no label enters it — and a rule that compared against zero rather
        // than against the depth *at the switch* would reject this. It is the only row where
        // that distinction shows.
        "int f(int n){ int a[n]; switch(n){ case 1: return 0; case 2: return a[0]; } return 0; }",
        // The declaration before the jump, which is the shape the rule must not disturb.
        "int f(int n){ typedef int T[n]; goto skip; skip: return 0; }",
        "int f(int n){ int a[n]; goto skip; skip: return a[0]; }",
        "int f(int n){ { int a[n]; } goto skip; skip: return 0; }",
        "int f(int n){ goto skip; { int a[n]; } skip: return 0; }",
        // An **integer** vector under `~`, and a floating one under `-`, which is arithmetic.
        "typedef int v4 __attribute__((vector_size(16)));\nint f(void){ v4 a={0}; return (~a)[0]; }",
        "typedef float f4 __attribute__((vector_size(16)));\nint f(void){ f4 a={0}; return (int)(-a)[0]; }",
        // The ordinary unary operands, which stay silent.
        "int f(void){ _Bool b=1; return ~b; }",
        "enum E{A=1}; int f(void){ enum E e=A; return ~e; }",
        "int f(void){ double d=1; return (int)-d; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}
