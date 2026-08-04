//! Covers: 015 §7, 020 §4.3 (`OpaqueReason::UnmodeledBuiltin`).
//!
//! **A builtin lowering does not model must not take the function with it.** gcc's
//! `ia32intrin.h` defines `__bsfd` as `return __builtin_ctz (__X);`, and that header is reached
//! by most of VPP. sema accepts an undeclared `__builtin_*` call — it has to, since `stdarg.h`
//! is `#define va_start(v,l) __builtin_va_start(v,l)` and nothing declares the target — but
//! lowering had no declaration to emit a call *to*, reported "call to undeclared function", and
//! 015 §7 contract 20 then discarded the whole function.
//!
//! Refusing the function whole is the right rule for a construct lowering genuinely cannot
//! represent: a partial body reads downstream as "this branch cannot be taken" rather than as a
//! gap. It is the wrong rule here, because the call *is* representable — as the opaque effect
//! 020 §4.3 already specifies and which lowering had never emitted anywhere.
//!
//! Measured 2026-08-04: with lowering absent from the sweep this was invisible, and the first
//! run that included it reported 24 of its first 30 translation units `not-run`, every one of
//! them this.

use chiero_cir::{InstKind, OpaqueReason};

mod harness;
use harness::{lower_maybe, lower_raw};

/// The function survives, and the call becomes an `Opaque` naming the builtin.
#[test]
fn an_unmodeled_builtin_is_opaque_rather_than_fatal() {
    let src = "int f(int x) { return __builtin_ctz(x); }";
    let raw = lower_raw(src);
    assert!(
        raw.diagnostics.is_empty(),
        "an unmodeled builtin is not a reason to refuse the function: {:?}",
        raw.diagnostics
    );
    let m = lower_maybe(src).expect("the function lowers");
    assert_eq!(m.funcs.len(), 1, "`f` is still in the module");

    let named: Vec<String> = m.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Opaque {
                why: OpaqueReason::UnmodeledBuiltin(n),
                ..
            } => Some(n.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        named,
        vec!["__builtin_ctz".to_string()],
        "the effect names the builtin, so a reader knows what was approximated"
    );
}

/// **The argument is read, and the result is defined.** An `Opaque` with no `reads` would let a
/// later pass treat `x` as dead, and one with no `dsts` would leave the call's value undefined
/// where C says it has one — either is a confident wrong answer rather than an approximation.
#[test]
fn an_unmodeled_builtin_reads_its_arguments_and_defines_its_result() {
    let m = lower_maybe("int f(int x) { return __builtin_ctz(x); }").expect("lowers");
    let op = m.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .find_map(|i| match &i.kind {
            InstKind::Opaque {
                dsts,
                reads,
                why: OpaqueReason::UnmodeledBuiltin(_),
                ..
            } => Some((dsts.clone(), reads.clone())),
            _ => None,
        })
        .expect("an opaque effect");
    assert_eq!(
        op.0.len(),
        1,
        "the call has a value, so the effect defines one"
    );
    assert_eq!(op.1.len(), 1, "and it reads its one argument");
}

/// **`dsts` follows the expression's type rather than being a constant one** — and the type it
/// follows is currently always `int`.
///
/// gcc gives `__builtin_prefetch` the type `void (const void *, ...)`, so this ought to define
/// nothing. chiero types it `int`: sema exempts an undeclared `__builtin_*` from the
/// undeclared-identifier rule but has **no signatures for the builtins**, so the call takes C's
/// implicit-`int` result like any other undeclared call.
///
/// So the `void` arm in lowering is correct and **not yet reachable**, and this test says so
/// rather than asserting a property the system cannot exhibit. The consequence is mild — a
/// value nothing reads — but it is a real gap, recorded in HANDOFF §9: modelling the builtins
/// exactly (Tier 1/2) needs their signatures, and `void` is the first thing a signature says.
#[test]
fn the_result_follows_the_expressions_type() {
    let dsts_of = |src: &str| {
        lower_maybe(src).expect("lowers").funcs[0]
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find_map(|i| match &i.kind {
                InstKind::Opaque {
                    dsts,
                    why: OpaqueReason::UnmodeledBuiltin(_),
                    ..
                } => Some(dsts.len()),
                _ => None,
            })
            .expect("an opaque effect")
    };
    // Both are `int` to sema today, the second only because it has no signature to say
    // otherwise. When signatures land, this row becomes 0 and the `void` arm goes live.
    assert_eq!(dsts_of("int f(int x) { return __builtin_ctz(x); }"), 1);
    assert_eq!(
        dsts_of("void f(void *p) { __builtin_prefetch(p); }"),
        1,
        "sema has no builtin signatures, so even a `void` builtin is typed `int`"
    );
}

/// **A genuinely undeclared function is still refused.** The exemption is the three prefixes gcc
/// itself declares nothing for — `__builtin_`, `__atomic_`, `__sync_` — and widening it to any
/// unknown name would turn every typo into a silent approximation.
#[test]
fn an_ordinary_undeclared_call_is_still_refused() {
    // 015 §7 replaces a function's inner diagnostics with the one skip sentence, so the
    // assertion is that the function was refused — not that the message names the callee.
    let raw = lower_raw("int f(int x) { return nonesuch_zz(x); }");
    assert!(
        raw.diagnostics
            .iter()
            .any(|d| d.message.contains("skipped")),
        "an undeclared name is not a builtin and is still refused: {:?}",
        raw.diagnostics
    );
}

/// The other two families gcc declares nothing for, so the rule is not written for one prefix.
#[test]
fn the_atomic_and_sync_families_are_opaque_too() {
    for src in [
        "int f(int *p) { return __atomic_load_n(p, 0); }",
        "int f(int *p) { return __sync_fetch_and_add(p, 1); }",
    ] {
        let raw = lower_raw(src);
        assert!(
            raw.diagnostics.is_empty(),
            "gcc declares nothing for these either: {src} -> {:?}",
            raw.diagnostics
        );
    }
}
