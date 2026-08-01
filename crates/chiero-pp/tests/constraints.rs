//! **C 6.10's constraints** — wave 333's census, and the first one run against the preprocessor.
//!
//! `chiero-pp` had a differential channel (`if_differential.rs`) and no constraint list at all: it
//! was the only crate in the pipeline with nothing asking *what it refuses*. Twenty programs to
//! gcc under `-pedantic-errors` found four rules already enforced and **eleven not**.
//!
//! The legal half is what shapes them, and three of its entries are the ones a rule written from
//! the paragraph number alone would reject:
//!
//!   - **`#define S # a` is legal.** The `#` operator only exists in a *function-like* macro; in
//!     an object-like one it is an ordinary token.
//!   - **`##` is different**: `#define C ## a` is illegal even object-like, so that rule is about
//!     position and not about the kind of macro.
//!   - **Redefining a macro identically is legal** (C 6.10.3p2), which is how a header guards
//!     against being included twice without `#ifndef`.

use chiero_pp::{Config, preprocess_str};

/// The messages `chiero-pp` produced for one source, or an empty list if it was silent.
fn diags(src: &str) -> Vec<String> {
    preprocess_str("f.c", src, Config::default())
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

/// **Constraints on a `#define`** — C 6.10.3p5, p6, 6.10.3.2p1, 6.10.3.3p1.
#[test]
fn a_macro_definition_is_constrained() {
    for bad in [
        // 6.10.3p6: parameters are distinct.
        "#define M(a, a) (a)\nint x = M(1,2);\n",
        // 6.10.3.2p1: `#` in a function-like macro is followed by a parameter.
        "#define S(a) # b\nconst char *s = S(1);\n",
        // 6.10.3.3p1: `##` appears at neither end — in an object-like macro too.
        "#define C(a) ## a\nint y = C(1);\n",
        "#define C(a) a ##\nint y = C(1);\n",
        "#define C ## a\nint x = 1;\n",
        "#define C a ##\nint x = 1;\n",
        // 6.10.3p5: `__VA_ARGS__` appears only in a variadic macro's replacement list.
        "#define M(a) __VA_ARGS__\nint q = M(1);\n",
        "#define M __VA_ARGS__\nint x = 1;\n",
        "#define __VA_ARGS__ 1\nint x = 1;\n",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: {bad:?}");
    }

    for good in [
        // **`#` is only an operator in a function-like macro.** Object-like, it is a token.
        "#define S # a\nint x = 1;\n",
        // `##` in the middle is the operator doing its job, either kind of macro.
        "#define C(a,b) a ## b\nint ab = 1; int y = C(a,b);\n",
        "#define C a ## b\nint x = 1;\n",
        // `#` followed by a parameter, and `#` then `##` in one list.
        "#define S(a) #a\nconst char *s = S(hi);\n",
        "#define S(a) #a ## x\nint x = 1;\n",
        // A variadic macro may use `__VA_ARGS__`, and distinct parameters are fine.
        "#define M(a, ...) ((a) __VA_ARGS__)\nint q = M(1, +2);\n",
        "#define M(a, b) ((a)+(b))\nint q = M(1,2);\n",
        // 6.10.3p2: an identical redefinition is legal and is how a header stays idempotent.
        "#define K 1\n#define K 1\nint z = K;\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: {good:?} -> {:?}",
            diags(good)
        );
    }
}

/// **Constraints on conditional inclusion** — C 6.10.1.
#[test]
fn conditional_inclusion_is_constrained() {
    for bad in [
        "int x = 1;\n#endif\n",
        "#if 1\n#else\n#else\n#endif\nint x = 1;\n",
        "#if\n#endif\nint x = 1;\n",
        "#if 1\nint x = 1;\n",
        "#ifdef A\nint x = 1;\n",
        "#if 1\n#if 0\n#endif\nint x = 1;\n",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: {bad:?}");
    }

    for good in [
        "#if 1\n#if 0\n#else\n#endif\n#endif\nint x = 1;\n",
        "#if 0\n#elif 1\n#else\n#endif\nint x = 1;\n",
        "#ifndef A\n#define A 1\n#endif\nint x = A;\n",
        "#if 1\nint x = 1;\n#endif\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: {good:?} -> {:?}",
            diags(good)
        );
    }
}

/// **`defined` is not a macro name** — C 6.10.8p4.
///
/// It would make `#if defined(X)` mean something else, which is why C reserves it in both
/// directions: neither `#define` nor `#undef` may name it.
#[test]
fn defined_is_not_a_macro_name() {
    for bad in [
        "#define defined 1\nint x = 1;\n",
        "#undef defined\nint x = 1;\n",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: {bad:?}");
    }
    for good in [
        // `#undef` of a name that was never defined is explicitly fine.
        "#undef NOPE\nint x = 1;\n",
        "#define DEFINED 1\n#undef DEFINED\nint x = 1;\n",
        "#define X 1\n#if defined(X)\nint x = 1;\n#endif\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: {good:?} -> {:?}",
            diags(good)
        );
    }
}
