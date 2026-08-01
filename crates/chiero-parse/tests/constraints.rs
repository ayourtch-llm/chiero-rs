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
        // Both signedness specifiers.
        "signed unsigned x;",
        // `long` where the base cannot take it, and too many of them.
        "long float x;",
        "long long long x;",
        "short long x;",
        "short double x;",
        // `signed`/`unsigned` on a type that has no signedness.
        "unsigned float x;",
        "signed double x;",
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

/// **A structure's members** — C 6.7.2.1p18 and p1.
#[test]
fn a_structure_member_list_is_constrained() {
    for bad in [
        // A flexible array member is last, and is not the only member.
        "struct S { int a[]; int b; };",
        // A member declaration declares a member.
        "struct S { ; };",
        // A declarator has to follow the specifiers.
        "struct S { int m; } int x;",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: `{bad}`");
    }

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

/// **`_Static_assert` takes a message in C11** — 6.7.10p1. Omitting it is C23.
#[test]
fn a_static_assertion_carries_its_message() {
    assert!(!diags("_Static_assert(1);").is_empty());
    assert!(diags("_Static_assert(1, \"ok\");").is_empty());
    assert!(diags("int f(void){ _Static_assert(1, \"ok\"); return 0; }").is_empty());
}
