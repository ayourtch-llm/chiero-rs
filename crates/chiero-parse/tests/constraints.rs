//! **C 6.7's declaration constraints** — wave 338's census, and the last of the three §9 named.
//!
//! `chiero-parse` had a VPP corpus gate saying what it *accepts* and nothing saying what it must
//! refuse. The first half of the census found **nothing**: fourteen malformed programs — a missing
//! semicolon, an unbalanced brace, `goto` with no label, a `for` with one clause — and the parser
//! reported every one, with ten legal shapes silent. That is a different result from the
//! preprocessor and the lexer, and it says the recovery paths have been exercised.
//!
//! The second half found eleven, all in the same place: **the declaration specifiers**. `int int`,
//! `long long long`, `signed unsigned` and `long float` are constraint violations that a parser
//! folding specifiers into one builtin type simply absorbs — `builtin_of` takes a base, a sign, a
//! long count and a short flag and answers for every combination of them, including the ones C
//! does not have.
//!
//! The legal half is the table's other side, and it is why this cannot be "at most one of each":
//! `long long`, `unsigned long int` and `long double` are all several specifiers naming one type.

use chiero_parse::{ScopedTypedefs, parse_tu};
use chiero_pp::{Config, preprocess_str};

fn diags(src: &str) -> Vec<String> {
    let tu = preprocess_str("f.c", src, Config::default());
    let mut oracle = ScopedTypedefs::new();
    parse_tu(&tu, &mut oracle)
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

/// **The type specifiers name one of C 6.7.2p2's sets, or they name nothing.**
#[test]
fn the_type_specifiers_name_one_type() {
    for bad in [
        // Two data types, in each of the shapes that reach a different arm.
        "int int x;",
        "float double x;",
        "void int x;",
        "char int x;",
        // **Both signedness specifiers, in both orders** — and repeated. Mutation found the
        // order mattered: each keyword has its own arm, so a fixture with one spelling leaves the
        // other arm's guard unobserved.
        "signed unsigned x;",
        "unsigned signed x;",
        "signed signed x;",
        "unsigned unsigned x;",
        // `long` where the base cannot take it, and too many of them.
        "long float x;",
        "long long long x;",
        "short long x;",
        "short double x;",
        // `signed`/`unsigned` on a type that has no signedness.
        "unsigned float x;",
        "signed double x;",
        // ...and a **length** on a type that has no length. `char` takes a signedness and
        // nothing else, which is its own arm and was unobserved until these two: every other
        // `char` case in this fixture is a *two data types* error instead.
        "long char x;",
        "short char x;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

    for good in [
        // **Several specifiers naming one type**, which is why the rule is a table and not a
        // count. Every one of these is a distinct valid multiset in 6.7.2p2.
        "long long x;",
        "long long int x;",
        "unsigned long long int x;",
        "signed long int x;",
        "unsigned long x;",
        "short int x;",
        "unsigned short int x;",
        "signed char x;",
        "unsigned char x;",
        "long double x;",
        "signed int x;",
        "unsigned x;",
        "signed x;",
        "long x;",
        "short x;",
        // ...and the ones with no room for a modifier at all.
        "void f(void);",
        "float x;",
        "double x;",
        "_Bool x;",
        "int x;",
        "char x;",
        // A `long` on a `_Complex` is legal, and so is the reversed spelling of everything.
        "long double _Complex x;",
        "int long unsigned x;",
        "int signed long long x;",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A structure's members** — C 6.7.2.1p1.
///
/// The *flexible array member* half of this census is in `chiero-sema`'s fixture instead: whether
/// an array is flexible is a question about its length, which the parser keeps as an unevaluated
/// expression on purpose, and the member rules it sits beside — a duplicate name, a variably
/// modified member — are already there.
#[test]
fn a_structure_member_list_is_constrained() {
    // A member declaration declares a member.
    let bad = "struct S { ; };";
    assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");

    for good in [
        "struct S { int n; int a[]; };",
        "struct S { int a; int b; };",
        "struct S { int m; } x;",
        "struct S { int m; };",
        "union U { int a; char b; };",
        // A bit-field with no name is a member declaration that declares no *name*, which is a
        // different thing from declaring nothing.
        "struct S { int a : 3; int : 5; int b : 2; };",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

// **`_Static_assert(1);` is deliberately accepted, and this is where that is recorded.**
//
// C11 6.7.10p1 requires the message and gcc refuses the short form under `-pedantic-errors`. It
// accepts it silently under `-std=gnu11`, which is the mode the corpus is compiled with, and
// `static_assert` in that spelling is what C23 standardised. The parser has taken both forms since
// it was written, with the reason in its own comment, and this census does not overrule it: the
// policy is the same one that accepts `0b101`, `\e` and `0.0f16`.
//
// Zero uses of the short form appear in the VPP tree; the ones in `/usr/include` are C++. So the
// divergence costs nothing today and is kept because reverting it would reject C23 headers later.

/// **A function is not initialized** — C 6.9.1p2, from wave 339's census.
///
/// `DeclKind::Func` has no field for an initializer, so `int x(void) = 1;` was parsed and the
/// `= 1` **silently discarded** — the program became an ordinary declaration of `x` and compiled.
/// That is why the check is here and not in sema: by the time sema sees the declaration the
/// initializer is gone.
#[test]
fn a_function_is_not_initialized() {
    for bad in ["int x(void) = 1;", "static int g(int) = 0;"] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }
    for good in [
        "int x(void);",
        "int y = 1;",
        "int f(void){ return 0; }",
        // A function *pointer* is an object and takes one.
        "int g(void); int (*p)(void) = g;",
        "int (*a[2])(void) = {0, 0};",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`typedef` in a parameter** — C 6.7.6.3p2, from wave 352's storage-class grid.
///
/// A parameter takes only `register`. This one is here rather than in sema because a parameter is
/// built as a `DeclKind::Var` whatever its specifiers said, so `is_typedef` is discarded before
/// sema sees it — the same shape as wave 331's `typedef static` and wave 339's initialized
/// function, and the third time a rule has landed in the parser for that reason.
#[test]
fn a_parameter_is_not_a_typedef() {
    for bad in [
        "int f(typedef int a){ return 0; }",
        "int f(int b, typedef int a){ return b; }",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }
    for good in [
        "int f(int a){ return a; }",
        "int f(register int a){ return a; }",
        "typedef int T; int f(T a){ return a; }",
        "typedef int T; int f(void){ T v = 1; return v; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`static` and qualifiers inside `[]` belong to a parameter, and so does `[*]`**
/// (C 6.7.6.2p1, p4).
///
/// The syntax is legal everywhere and the *meaning* exists only in a parameter: `int a[static 3]`
/// promises the caller passes at least three elements, and an object declaration has no caller.
/// 013 read these tokens and threw them away with the comment that they "carry no meaning for
/// us" — true of 014, and a different thing from being unconstrained, which is why the rule is
/// here rather than there.
///
/// The depth is a **counter and not a flag**: `int f(int g(int a[static 3]))` nests one parameter
/// list inside another, and leaving on the way out of the inner one would make the outer look
/// like file scope.
#[test]
fn array_decorations_belong_to_a_parameter() {
    for (src, want) in [
        (
            "int a[static 3];",
            "`static` in an array size belongs to a parameter",
        ),
        (
            "int f(void){ int a[const 3]; return a[0]; }",
            "a qualifier in an array size belongs to a parameter",
        ),
        (
            "struct S { int a[static 3]; };",
            "`static` in an array size belongs to a parameter",
        ),
        (
            "typedef int T[static 3];",
            "`static` in an array size belongs to a parameter",
        ),
        (
            "int f(void){ int a[*]; return 0; }",
            "`[*]` belongs to a function prototype",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        // **In a parameter**, where every decoration means something.
        "int f(int a[static 3]);",
        "int f(int a[static 3]){ return a[0]; }",
        "int f(int a[const 3]);",
        "int g(int a[const 3]){ return a[0]; }",
        "int f(int a[volatile 3]);",
        "int f(int a[]);",
        "int f(int a[*]);",
        "int f(int a[3][*]);",
        // **A nested parameter list**, which is what makes the depth a counter — and the
        // decoration *after* it, which is what makes the counter decrement rather than reset.
        // A mutant that zeroed the depth on the way out survived the first spelling.
        "int f(int g(int a[static 3]));",
        "int f(int g(void), int a[static 3]);",
        // Ordinary arrays, including the declared `int a[0]` divergence and a VLA.
        "int f(void){ int a[3]; return a[0]; }",
        "int a[2];",
        "int a[0];",
        "int f(int n){ int a[n]; return a[0]; }",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **Every parse diagnostic points at visible text** (023 §9), the third and last of wave 373's
/// gate family.
///
/// `chiero-sema` and `chiero-pp` got theirs in 373; this crate was left because its constraints
/// test had no `VIOLATIONS` list to run over. The list is small and the question is the same: a
/// report whose span covers no text is one a reader cannot follow to the fault.
#[test]
fn every_parse_diagnostic_points_at_visible_text() {
    let mut invisible: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for src in [
        "int int x;",
        "float double x;",
        "signed unsigned x;",
        "long float x;",
        "int a[static 3];",
        "typedef int T[static 3];",
        "int f(void){ int a[*]; return 0; }",
        "int f(void){ int a[const 3]; return a[0]; }",
        "struct S { int a[static 3]; };",
        "int f(void){ return 1",
        "int f(void) { return 1; ",
        "int f(void){ goto; }",
        "int f(void){ for(;) ; }",
        "int f(void){ if }",
    ] {
        let tu = preprocess_str("f.c", src, Config::default());
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);
        for d in &parsed.diagnostics {
            checked += 1;
            match tu.source_map.span_text(d.span) {
                Some(t) if !t.is_empty() => {}
                _ => invisible.push(format!("{src:?}: {}", d.message)),
            }
        }
    }
    assert!(checked >= 14, "only {checked} diagnostics were examined");

    // **And it is the token the parser stopped at**, which the gate above cannot tell — a
    // widening that picked any non-empty token would satisfy it. gcc's convention is to name
    // the token that *is* there ("expected `;` before `}` token"), and these rows are that:
    // `goto;` stops at the `;`, `return 1` at the `1`, and `if }` at the `}` five times over.
    for (src, want) in [
        ("int f(void){ goto; }", ";"),
        ("int f(void){ return 1", "1"),
        ("int a[static 3];", "static"),
        ("int f(void){ if }", "}"),
    ] {
        let tu = preprocess_str("f.c", src, Config::default());
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);
        let covered: Vec<&str> = parsed
            .diagnostics
            .iter()
            .filter_map(|d| tu.source_map.span_text(d.span))
            .collect();
        assert!(
            !covered.is_empty() && covered.iter().all(|t| *t == want),
            "every diagnostic for `{src}` names {want:?}: {covered:?}"
        );
    }
    assert!(
        invisible.is_empty(),
        "{} diagnostic(s) point at no visible text:\n  {}",
        invisible.len(),
        invisible.join("\n  ")
    );
}

/// **An old-style parameter no declaration typed defaults to `int`** (C89 3.7.1).
///
/// 013 gave such a parameter `TypeKind::Error` as a placeholder for "a declaration will say", and
/// when none came 014 read the poison as an *incomplete type* and said so — of a parameter whose
/// type C specifies. Two symptoms, one cause: the wrong sentence for `int f(a) { … }`, and a
/// second sentence for `int f(a) int b;` on top of the parser's correct one.
///
/// gcc reports the default under `-pedantic-errors` and warns under `-std=gnu11`, so this project
/// reports it (wave 314's calibration). The declaration that names a non-parameter keeps its own
/// sentence: `int f(a) int b;` is **two** faults and gcc gives two.
#[test]
fn an_undeclared_old_style_parameter_defaults_to_int() {
    assert_eq!(
        diags("int f(a) { return a; }\n"),
        vec!["type of `a` defaults to `int`".to_string()]
    );
    assert_eq!(
        diags("int f(a, b) int a; { return a+b; }\n"),
        vec!["type of `b` defaults to `int`".to_string()]
    );
    // **Two faults, two sentences**, in the order gcc gives them: the stray declaration first,
    // then the parameter it left untyped.
    assert_eq!(
        diags("int f(a) int b; { return a; }\n"),
        vec![
            "this declaration names something that is not a parameter of the function it follows"
                .to_string(),
            "type of `a` defaults to `int`".to_string(),
        ]
    );

    for good in [
        "int f(a) int a; { return a; }\n",
        "int f(a, b) int a; int b; { return a+b; }\n",
        "int f(a, b) int a, b; { return a+b; }\n",
        // A prototype is not an identifier list and is untouched by any of this.
        "int f(int a){ return a; }\n",
        "int f(void){ return 0; }\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A `typedef` may not have an initializer** (C 6.7p2: a declaration with `typedef` declares no
/// object, so there is nothing to initialize).
///
/// Found by running wave 387's 24-program declaration census against chiero for the first time.
/// The rule has to live here rather than in sema because `finish_declarator_inner` **drops** the
/// initializer when the specifiers say `typedef` — `DeclKind::Typedef` has no field for one — so
/// `typedef int T = 1;` reached sema indistinguishable from `typedef int T;`.
///
/// That is the same defect the arm immediately below it already fixes for functions, whose
/// comment says it exactly: "`DeclKind::Func` has no room for an initializer, so without this the
/// `= 1` was parsed and then silently discarded — a wrong answer rather than a missing
/// diagnostic". The neighbouring arm was left. **When a node has no room for something the
/// grammar allows, every arm that builds it needs the same guard.**
#[test]
fn a_typedef_may_not_have_an_initializer() {
    for (src, want) in [
        ("typedef int T = 1;\n", "a `typedef` cannot be initialized"),
        (
            "typedef int T[1] = {0};\n",
            "a `typedef` cannot be initialized",
        ),
        ("typedef int *T = 0;\n", "a `typedef` cannot be initialized"),
        // A function typedef takes this sentence, not the function one beside it.
        (
            "typedef int F(void) = 1;\n",
            "a `typedef` cannot be initialized",
        ),
    ] {
        assert_eq!(
            diags(src),
            vec![want.to_string()],
            "the message for `{src}`"
        );
    }

    for good in [
        "typedef int T;\n",
        "typedef int F(void);\n",
        "typedef int A[3];\n",
        "typedef int *P;\n",
        // An ordinary initialized object is untouched.
        "int x = 1;\n",
        "static int y = 2;\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **`_Static_assert`'s message is a string literal, and adjacent literals are one literal**
/// (C 6.4.5p5: in translation phase 6, adjacent string literal tokens are concatenated).
///
/// `_Static_assert(1, "a" "b");` was refused with "expected `)` to close `_Static_assert`" — the
/// parser took a single string token and stopped. Legal C, and gcc compiles it.
///
/// **Found by `cargo xtask sweep` over VPP**, not by a census. It appeared as four files failing
/// to parse, and reduced to five lines through VPP's own idiom:
///
/// ```c
/// #define STATIC_ASSERT(truth,...) _Static_assert(truth, __VA_ARGS__)
/// #define STATIC_ASSERT_SIZEOF(d, s) \
///   STATIC_ASSERT (sizeof (d) == s, "Size of " #d " must be " # s " bytes")
/// ```
///
/// The message is built by stringizing two macro arguments and concatenating them with three
/// literals, so *every* use of `STATIC_ASSERT_SIZEOF` hits it — and HANDOFF's construct table
/// records `_Static_assert` (VPP's `STATIC_ASSERT`) at **140 uses**. A census of 6.7.10 would not
/// have found this: the construct under test is legal, and the fault is in a neighbouring rule
/// about how string literals are spelled.
#[test]
fn a_static_assert_message_may_be_several_adjacent_literals() {
    for good in [
        // The plain form, which already worked — kept so a fix cannot trade one for the other.
        "_Static_assert(1, \"x\");\n",
        // Two and three adjacent literals.
        "_Static_assert(1, \"a\" \"b\");\n",
        "_Static_assert(1, \"a\" \"b\" \"c\");\n",
        // Built by a macro, which is how VPP reaches it: stringize between two literals.
        "#define M(x) \"p\" #x \"q\"\n_Static_assert(1, M(z));\n",
        // VPP's own shape, reduced.
        "#define SA(t,...) _Static_assert(t, __VA_ARGS__)\n\
         #define SASZ(d, s) SA (sizeof (d) == s, \"Size of \" #d \" must be \" # s \" bytes\")\n\
         struct foo { int a; };\n\
         SASZ (struct foo, 4);\n",
        // Adjacent literals elsewhere must keep working — this is a lexer-level rule, not a
        // `_Static_assert` one, and a fix that special-cased the assertion would say otherwise.
        "const char *p = \"a\" \"b\";\n",
        "void f(void){ const char *q = \"a\" \"b\" \"c\"; (void)q; }\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }
}

/// **A comma still needs a literal after it.**
///
/// A mutant that deleted the missing-message check survived every acceptance row above: nothing
/// covered `_Static_assert(1, );`. The loop must make the message *repeatable*, not *optional*.
#[test]
fn a_static_assert_comma_still_needs_a_message() {
    // gcc: "expected string literal before `)` token".
    assert_eq!(
        diags("_Static_assert(1, );\n"),
        vec!["expected a string literal message".to_string()]
    );
    // The C23/GNU form with no comma at all stays legal.
    assert!(diags("_Static_assert(1);\n").is_empty());
}

/// **A stray `;` between members is a GNU extension gcc takes** — `struct S { int a; ; };`.
///
/// chiero refused it with "a member declaration must declare a member", a rule meant for
/// `struct S { int; };` — a declaration that names a type and declares nothing. An empty
/// declaration is neither: there is no type either, so nothing is being said and nothing is
/// wrong. gcc warns only under `-Wpedantic`, so at `-std=gnu11` — which is what VPP builds with
/// — it is silent.
///
/// Found by `cargo xtask sweep` over `vlib`, where it blocked 4 of 47 files. That subtree became
/// sweepable in the same wave: it previously reported "0 findings, 45 agree", which was 45 files
/// gcc could not compile either, because `vlib/config.h` is generated and the stub lacked
/// `VLIB_BUFFER_PRE_DATA_SIZE`. Regenerating the stubs from `config.h.in` with CMake's own
/// defaults took gcc from 0 to 44 of 47.
///
/// **The empty declaration is still refused where C forbids it**: at file scope `;` is
/// "ISO C does not allow an extra `;`" under `-pedantic-errors`, and this project calibrates
/// there — so only the *member* position changes.
#[test]
fn a_stray_semicolon_between_members_is_allowed() {
    for good in [
        "struct S { int a; ; };\n",
        "struct S { ; int a; };\n",
        "struct S { int a; ;; int b; };\n",
        "union U { int a; ; };\n",
        // A bit-field beside one, since members have two spellings.
        "struct S { int a : 2; ; int b; };\n",
        // Unchanged: the ordinary forms.
        "struct S { int a; };\n",
        "struct S { int a; int b; };\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: `{good}` -> {:?}",
            diags(good)
        );
    }

    // **A declaration that names a type and declares nothing is still wrong** — that is what the
    // rule was for, and an empty declaration is not an instance of it.
    assert_eq!(
        diags("struct S { int; };\n"),
        vec!["a member declaration must declare a member".to_string()]
    );
    assert_eq!(
        diags("struct S { int a; struct T; };\n"),
        vec!["a member declaration must declare a member".to_string()]
    );
}
