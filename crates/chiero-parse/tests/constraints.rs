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
