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

/// **Inline asm is an opaque effect too, and 013 §4 already said so.**
///
/// "Lowering turns it into an opaque effect that clobbers its outputs and marks the path
/// `Approximated`. Modelling x86 semantics is out of scope, and treating asm as a no-op would be
/// unsound in the direction that produces confident wrong answers." That is the documented
/// design; lowering instead fell through to `statement not lowered yet: asm` and 015 §7 discarded
/// the function. `OpaqueReason::InlineAsm` has been in CIR with no producer, exactly as
/// `UnmodeledBuiltin` was.
///
/// Measured 2026-08-04: with `__builtin_ctz` fixed, asm became the **new** dominant cause —
/// 12 of the first 18 translation units, through gcc's `pconfigintrin.h`, whose `_pconfig_u32`
/// is a `__asm__` with four outputs. 31 VPP files use asm directly (013 §4).
#[test]
fn inline_asm_is_opaque_rather_than_fatal() {
    for src in [
        "void f(void) { __asm__ __volatile__ (\"nop\"); }",
        "int f(int x) { int r; __asm__ (\"mov %1,%0\" : \"=r\"(r) : \"r\"(x)); return r; }",
        "void f(void) { __asm__ __volatile__ (\"nop\" ::: \"memory\"); }",
    ] {
        let raw = lower_raw(src);
        assert!(
            raw.diagnostics.is_empty(),
            "asm is approximated, not fatal: {src} -> {:?}",
            raw.diagnostics
        );
        let m = lower_maybe(src).expect("the function lowers");
        assert!(
            m.funcs[0]
                .blocks
                .iter()
                .flat_map(|b| b.insts.iter())
                .any(|i| matches!(
                    &i.kind,
                    InstKind::Opaque {
                        why: OpaqueReason::InlineAsm,
                        ..
                    }
                )),
            "the effect is present and names itself: {src}"
        );
    }
}

/// **An output operand is clobbered.** `r` is uninitialized before the asm and defined after it,
/// so an effect that did not write it would leave every later read of `r` reading undef — a
/// confident wrong answer, which is the failure 013 §4 names explicitly.
#[test]
fn an_asm_output_is_written() {
    let m = lower_maybe(
        "int f(int x) { int r; __asm__ (\"m %1,%0\" : \"=r\"(r) : \"r\"(x)); return r; }",
    )
    .expect("lowers");
    let (writes, reads) = m.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .find_map(|i| match &i.kind {
            InstKind::Opaque {
                writes,
                reads,
                why: OpaqueReason::InlineAsm,
                ..
            } => Some((writes.len(), reads.len())),
            _ => None,
        })
        .expect("an opaque effect");
    assert_eq!(writes, 1, "the one output is clobbered");
    assert_eq!(reads, 1, "and the one input is read");
}

/// **A function declaration at block scope declares no object** (C 6.7p2), so it lowers to
/// nothing at all — and lowering discarded the enclosing function instead.
///
/// `vppinfra/format.h` writes it, inside `unformat_check_input`:
///
/// ```c
/// always_inline uword unformat_check_input (unformat_input_t * i) {
///   extern uword _unformat_fill_input (unformat_input_t * i);
///   ...
/// }
/// ```
///
/// `format.h` is included nearly everywhere in VPP, which is why this was the dominant cause of
/// lost functions the moment the builtin and asm gaps were closed — 12 of the first 18
/// translation units, measured 2026-08-04.
///
/// **`extern` is not the distinguishing part**, and a fix keyed on the storage class would miss
/// half of it: `int g(int);` at block scope declares the same thing with the same linkage, and
/// C 6.2.2p5 gives a function declared with no storage-class specifier external linkage anyway.
/// The type is what decides.
#[test]
fn a_block_scope_function_declaration_lowers_to_nothing() {
    for src in [
        "int f(int x) { extern int g(int); return g(x); }",
        "int f(int x) { int g(int); return g(x); }",
        // A pointer to a function is an ordinary object and must keep its slot.
        "int f(int x) { int (*g)(int) = 0; return g ? g(x) : x; }",
    ] {
        let raw = lower_raw(src);
        assert!(
            raw.diagnostics.is_empty(),
            "a block-scope declaration is not a construct lowering cannot represent: {src} -> {:?}",
            raw.diagnostics
        );
        // `f` survives. The module may hold more than one function now: a block-scope
        // declaration registers a signature, which is what lets the call to it resolve.
        assert!(
            lower_maybe(src)
                .expect("lowers")
                .funcs
                .iter()
                .any(|f| &*f.name == "f"),
            "the enclosing function is kept: {src}"
        );
    }

    // **The object case still allocates.** A guard written as "skip any block-scope declaration"
    // would pass every row above and silently stop giving locals their storage.
    let m = lower_maybe("int f(int x) { extern int g(int); int y = x + 1; return g(y); }")
        .expect("lowers");
    let f = m
        .funcs
        .iter()
        .find(|f| &*f.name == "f")
        .expect("`f` is in the module");
    let named: Vec<String> = f
        .allocas
        .iter()
        .filter_map(|a| a.name.as_ref().map(|n| n.to_string()))
        .collect();
    assert!(
        named.iter().any(|n| n == "y"),
        "`y` still has a slot: {named:?}"
    );
    assert!(
        !named.iter().any(|n| n == "g"),
        "the function declaration has none: {named:?}"
    );
    // And the declaration registered a signature, which is what lets the call resolve.
    assert!(
        m.funcs.iter().any(|f| &*f.name == "g"),
        "the block-scope declaration is in the module"
    );
}

/// **An explicit cast of a builtin call.**
///
/// gcc's `avx512fintrin.h` is built from these — every masked intrinsic is
/// `(__mmask16) __builtin_ia32_ptestmd512 (...)` — and once the builtin, asm and block-scope
/// gaps closed this was **29 of the first 35** translation units measured.
///
/// The root cause was in sema, not here: an undeclared builtin call was typed `Ty::Error`, so
/// the cast over poison took a `Return` conversion an ordinary call does not, and lowering
/// emitted a second `zext` on top of `raw_expr`'s own — CIR the verifier rejected, which
/// discarded the function. Giving the builtin C's implicit declaration (`int ()`) collapses both
/// casts to the one that was always correct.
///
/// **Both directions**, because the defect was never about narrowing: `long` widens and
/// `unsigned short` narrows, and both failed identically.
#[test]
fn an_explicit_cast_of_a_builtin_call_lowers() {
    for src in [
        "unsigned short f(int x) { return (unsigned short) __builtin_ctz(x); }",
        "long f(int x) { return (long) __builtin_ctz(x); }",
        // The `avx512fintrin.h` shape: a cast of a builtin taking several arguments.
        "unsigned short f(int a, int b) { return (unsigned short) __builtin_ia32_ptestmd512(a, b, 1); }",
    ] {
        let raw = lower_raw(src);
        assert!(
            raw.diagnostics.is_empty(),
            "the cast lowers once, not twice: {src} -> {:?}",
            raw.diagnostics
        );
        assert!(
            lower_maybe(src)
                .expect("lowers")
                .funcs
                .iter()
                .any(|f| &*f.name == "f")
        );
    }
}
