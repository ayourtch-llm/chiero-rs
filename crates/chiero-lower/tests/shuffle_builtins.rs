//! **`__builtin_shuffle` has no type, so every function reaching it is discarded.**
//!
//! ```c
//! typedef int v4si __attribute__((vector_size(16)));
//! v4si f(v4si a) { v4si t = __builtin_shuffle(a, (v4si){2,3,0,1}); return t; }
//! ```
//!
//! `lower: `f` lowered to CIR the verifier rejects (copy source must be pointer-typed, got
//! Int(32))`. The builtin table deliberately omits the ten front-end special forms, so the call
//! keeps `Ty::Error`, `cty(Error)` falls back to `Int(32)`, and initializing a vector-typed
//! object from a scalar value is a `Copy` whose source is not an address.
//!
//! # It is the whole remaining `not-run` population
//!
//! With the same-named-statics defect fixed, **26 of the first 27** VPP translation units still
//! failed, all of them here:
//!
//! ```text
//! avx512fintrin.h:16022:3: `_mm512_reduce_add_epi32` lowered to CIR the verifier rejects
//!   __v4si __T7 = __builtin_shuffle (__T6, (__v4si) { 2, 3, 0, 1 });
//! ```
//!
//! `__MM512_REDUCE_OP` is the body of every `_mm512_reduce_*` intrinsic, `x86intrin.h` reaches
//! it, and `vppinfra/clib.h` includes that — so it is in essentially every translation unit.
//! VPP's own `vppinfra/vector.h` writes `__builtin_shuffle` directly as well.
//!
//! # Measured against gcc 13.3.0, never read from documentation
//!
//! Passing the call to `void take(struct Z)` and reading the resulting `note: expected 'struct Z'
//! but argument is of type 'T'` — the method `builtins.rs` records for its 3196 rows:
//!
//! | call | gcc says |
//! |---|---|
//! | `__builtin_shuffle(v4si, v4si)` | `v4si` |
//! | `__builtin_shuffle(v4si, v4si, v4si)` | `v4si` |
//! | `__builtin_shuffle(v4sf, v4si)` | `v4sf` |
//! | `__builtin_shuffle(v4di, v4di)` | `v4di` |
//! | `__builtin_shufflevector(v4si, v4si, 0,1,2,3)` | `__vector(4) int` |
//! | `__builtin_shufflevector(v4si, v4si, 0,1)` | `__vector(2) int` |
//! | `__builtin_shufflevector(v4sf, v4sf, 0..7)` | `__vector(8) float` |
//!
//! So `__builtin_shuffle` is the type of its **first argument** — gcc rejects a mask of a
//! different length outright, so there is no lane-count question — and `__builtin_shufflevector`
//! is the first argument's element type with **as many lanes as there are index arguments**.
//!
//! # Why this is typed per call rather than tabulated
//!
//! A row cannot express "the type of operand 1". These are the first of the type-generic family;
//! the 46 `__atomic_*`/`__sync_*` names whose result is the *pointee* of operand 1 are the same
//! shape and are the larger remaining gap (HANDOFF §9). `is_fp_classify_builtin` is the
//! precedent: resolved at the call node, where the arguments are in hand.

mod harness;

/// The corpus shape, reduced: gcc's `__MM512_REDUCE_OP` line with the AVX-512 taken out.
#[test]
fn a_shuffle_of_a_vector_lowers() {
    let src = "typedef int v4si __attribute__((vector_size(16)));\n\
               v4si f(v4si a) { v4si t = __builtin_shuffle(a, (v4si){2,3,0,1}); return t; }\n";
    let lowered = harness::lower_raw(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "`vppinfra/vector.h` and every `_mm512_reduce_*` write this: {:?}",
        lowered.diagnostics
    );
    let errors: Vec<String> = chiero_cir::verify::verify(&lowered.module)
        .iter()
        .filter(|e| e.is_error())
        .map(|e| format!("{:?}: {}", e.kind, e.detail))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// The three-operand form, which selects across two vectors and is what `emmintrin.h` uses.
#[test]
fn the_two_source_form_lowers_too() {
    let src = "typedef int v4si __attribute__((vector_size(16)));\n\
               v4si f(v4si a, v4si b) { return __builtin_shuffle(a, b, (v4si){0,1,4,5}); }\n";
    let lowered = harness::lower_raw(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

/// **The element type comes from the data operand, not from the mask.** `v4sf` data with a `v4si`
/// mask is `v4sf`, so a fix that returned the mask's type passes the tests above and fails this.
#[test]
fn a_float_vector_shuffled_by_an_integer_mask_stays_float() {
    let src = "typedef int v4si __attribute__((vector_size(16)));\n\
               typedef float v4sf __attribute__((vector_size(16)));\n\
               float f(v4sf a, v4si m) { v4sf t = __builtin_shuffle(a, m); return t[0]; }\n";
    let lowered = harness::lower_raw(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let errors: Vec<String> = chiero_cir::verify::verify(&lowered.module)
        .iter()
        .filter(|e| e.is_error())
        .map(|e| format!("{:?}: {}", e.kind, e.detail))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// **`__builtin_shufflevector`'s lane count is the number of indices**, not the first operand's.
///
/// Two indices over `v4si` is a *two*-lane vector — gcc says `__vector(2) int` — so a fix that
/// copied the first argument's type wholesale (right for `__builtin_shuffle`) is wrong here. It
/// reaches VPP through `avx512fp16vlintrin.h`.
#[test]
fn shufflevector_takes_its_length_from_the_index_list() {
    let src = "typedef int v4si __attribute__((vector_size(16)));\n\
               int f(v4si a, v4si b) {\n\
                 __attribute__((vector_size(8))) int t = __builtin_shufflevector(a, b, 0, 1);\n\
                 return t[0];\n\
               }\n";
    let lowered = harness::lower_raw(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let errors: Vec<String> = chiero_cir::verify::verify(&lowered.module)
        .iter()
        .filter(|e| e.is_error())
        .map(|e| format!("{:?}: {}", e.kind, e.detail))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}
