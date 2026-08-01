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
        // **Skipped text is not diagnosed** (012's rule, and gcc agrees): a `#if` inside an
        // inactive region is counted for nesting but never evaluated, so a bare one there is
        // fine. Mutation found this — the `parent_active` guard on the no-expression check was
        // unobserved until a skipped `#if` existed to observe it.
        "#if 0\n#if\n#endif\n#endif\nint x = 1;\n",
        "#if 0\n#if @@@\n#endif\n#endif\nint x = 1;\n",
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

/// **A directive takes the tokens it takes, and no more** — C 6.10p1's syntax, which gcc reports
/// as "extra tokens at end of #... directive".
///
/// §9 held these back for two waves because `#endif FOO` was a real idiom before comments were
/// reliable, and shipping the rule blind would mean a false positive on every old header. **The
/// corpus answers it: zero occurrences** across glibc, gcc's own headers, the VPP test corpus and
/// all 2,476 `.c`/`.h` files in the VPP tree. So the check is safe, and the count is the reason to
/// believe it rather than a hope.
///
/// The rules come in two shapes with one guard between them:
///
///   - **`#endif` and `#else` take nothing**; `#ifdef`, `#ifndef` and `#undef` take exactly one
///     macro name — required, so a bare one is its own error; `#line` takes a number and
///     optionally a file string.
///   - **None of it is diagnosed in skipped text.** `#if 0 / #if 1 / #endif junk / #endif` is
///     accepted by gcc, because an inactive region is scanned for nesting and not for syntax.
///     That is wave 333's `parent_active` rule again, and it is the only thing separating these
///     rules from a false positive on every `#if 0`-ed block in the world.
#[test]
fn a_directive_takes_no_extra_tokens() {
    for bad in [
        "#if 1\n#endif junk\nint x = 1;\n",
        "#if 0\n#else junk\n#endif\nint x = 1;\n",
        "#define K 1\n#undef K junk\nint x = 1;\n",
        "#ifdef A B C\n#endif\nint x = 1;\n",
        "#ifndef A B\n#endif\nint x = 1;\n",
        "#line 5 \"f.c\" junk\nint x = 1;\n",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: {bad:?}");
    }

    for good in [
        // A comment is not a token by the time a directive is read.
        "#if 1\n#endif /* done */\nint x = 1;\n",
        // **Skipped text is scanned for nesting, not for syntax.**
        "#if 0\n#if 1\n#endif junk\n#endif\nint x = 1;\n",
        "#if 0\n#if 0\n#else junk\n#endif\n#endif\nint x = 1;\n",
        "#if 0\n#undef K junk\n#endif\nint x = 1;\n",
        "#if 0\n#ifdef A B C\n#endif\n#endif\nint x = 1;\n",
        // The well-formed spellings of each.
        "#define K 1\n#undef K\nint x = 1;\n",
        "#ifdef A\n#endif\nint x = 1;\n",
        "#ifndef A\n#endif\nint x = 1;\n",
        "#line 5 \"f.c\"\nint x = 1;\n",
        "#line 5\nint x = 1;\n",
        // `#elif` and `#if` take an *expression*, so more than one token is the point.
        "#if 0\n#elif 1 + 1\n#endif\nint x = 1;\n",
        "#if 1 + 1 > 1\nint x = 1;\n#endif\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: {good:?} -> {:?}",
            diags(good)
        );
    }
}

/// **`#ifdef`, `#ifndef` and `#undef` need a macro name** — C 6.10.1p1, 6.10.3.5p1.
///
/// Separate from the rule above because it fails the other way: not a token too many but none at
/// all, and gcc says so in different words ("no macro name given in #... directive"). A single
/// "wrong number of tokens" check would report the same sentence for both, and 023 §9 asks for a
/// report a person can act on.
#[test]
fn a_conditional_on_a_macro_needs_its_name() {
    for bad in [
        "#ifdef\n#endif\nint x = 1;\n",
        "#ifndef\n#endif\nint x = 1;\n",
        "#undef\nint x = 1;\n",
    ] {
        assert!(!diags(bad).is_empty(), "must be diagnosed: {bad:?}");
    }
    for good in [
        "#ifdef A\n#endif\nint x = 1;\n",
        "#undef NOPE\nint x = 1;\n",
        // ...and not in skipped text, for the same reason as above.
        "#if 0\n#ifdef\n#endif\n#endif\nint x = 1;\n",
        "#if 0\n#undef\n#endif\nint x = 1;\n",
    ] {
        assert!(
            diags(good).is_empty(),
            "must be accepted: {good:?} -> {:?}",
            diags(good)
        );
    }
}

/// One C 6.10 constraint: a name for the report, and a program that violates it.
const VIOLATIONS: &[(&str, &str)] = &[
    (
        "duplicate macro parameter",
        "#define M(a, a) (a)\nint x = M(1,2);\n",
    ),
    (
        "# not before a parameter",
        "#define S(a) # b\nconst char *s = S(1);\n",
    ),
    ("## at the start", "#define C(a) ## a\nint y = C(1);\n"),
    ("## at the end", "#define C(a) a ##\nint y = C(1);\n"),
    (
        "## at the start, object-like",
        "#define C ## a\nint x = 1;\n",
    ),
    (
        "redefinition with a different body",
        "#define K 1\n#define K 2\nint z = K;\n",
    ),
    (
        "too few macro arguments",
        "#define M(a, b) ((a)+(b))\nint q = M(1);\n",
    ),
    (
        "too many macro arguments",
        "#define M(a) (a)\nint q = M(1,2);\n",
    ),
    (
        "__VA_ARGS__ outside a variadic macro",
        "#define M(a) __VA_ARGS__\nint q = M(1);\n",
    ),
    (
        "__VA_ARGS__ as a macro name",
        "#define __VA_ARGS__ 1\nint x = 1;\n",
    ),
    ("#endif without #if", "int x = 1;\n#endif\n"),
    ("#else without #if", "int x = 1;\n#else\n"),
    (
        "#else after #else",
        "#if 1\n#else\n#else\n#endif\nint x = 1;\n",
    ),
    ("#if with no expression", "#if\n#endif\nint x = 1;\n"),
    ("unterminated #if", "#if 1\nint x = 1;\n"),
    ("unknown directive", "#nonsense\nint x = 1;\n"),
    ("#define defined", "#define defined 1\nint x = 1;\n"),
    ("#undef defined", "#undef defined\nint x = 1;\n"),
    // **Not yet enforced**, and here so they stay visible. All three are extra tokens after a
    // directive that takes none — `-pedantic-errors` diagnostics that old headers trip over, so
    // the corpus needs checking before they are safe to add.
    (
        "extra tokens after #endif",
        "#if 1\n#endif junk\nint x = 1;\n",
    ),
    (
        "extra tokens after #else",
        "#if 0\n#else junk\n#endif\nint x = 1;\n",
    ),
    (
        "extra tokens after #undef",
        "#define K 1\n#undef K junk\nint x = 1;\n",
    ),
];

fn gcc_rejects(src: &str) -> Option<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("gcc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-E",
            "-o",
            "/dev/null",
            "-x",
            "c",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(src.as_bytes()).ok()?;
    Some(!child.wait().ok()?.success())
}

/// **How much of C 6.10's constraint surface the preprocessor rejects, as a number that may not
/// fall** — the same ratchet `chiero-sema` has carried since wave 325, which this crate had no
/// analogue of until wave 333.
///
/// The failure prints the *names* of what is missed, so the next wave reads a queue rather than a
/// percentage; and every entry is one gcc confirms, so a program gcc accepts is a bug in this list
/// rather than a missing check.
#[test]
fn the_share_of_directive_violations_rejected_does_not_fall() {
    /// Measured at wave 333. **Raise this when a rule is added; never lower it.**
    ///
    /// The three below the line are the extra-token rules, left open deliberately.
    const FLOOR: usize = 18;

    if gcc_rejects("int main(void){return 0;}\n") != Some(false) {
        eprintln!("skipping: gcc not usable here");
        return;
    }

    let mut caught = Vec::new();
    let mut missed = Vec::new();
    let mut not_a_violation = Vec::new();
    for (name, src) in VIOLATIONS {
        match gcc_rejects(src) {
            Some(false) | None => {
                not_a_violation.push(*name);
                continue;
            }
            Some(true) => {}
        }
        if diags(src).is_empty() {
            missed.push(*name);
        } else {
            caught.push(*name);
        }
    }

    assert!(
        not_a_violation.is_empty(),
        "gcc accepts these, so they are bugs in this list rather than missing checks: \
         {not_a_violation:?}"
    );
    eprintln!(
        "chiero-pp rejects {} of {} directive violations; missing: {missed:?}",
        caught.len(),
        VIOLATIONS.len()
    );
    assert!(
        caught.len() >= FLOOR,
        "coverage fell to {} from {FLOOR}; newly missed: {missed:?}",
        caught.len()
    );
}
