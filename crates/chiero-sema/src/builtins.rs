//! **Generated. Do not edit by hand.** Measured return types for gcc's compiler builtins.
//!
//! gcc declares these itself and no header names them, so sema exempts them from
//! "was not declared" — but an exemption is not a type. Without one an unmodeled builtin's value
//! has no width, and gcc's own intrinsic headers are written as `return __builtin_ia32_X (…);`
//! inside an `always_inline` wrapper, so the wrapper's return type disagreed with the value and
//! 015 §7 discarded the function. gcc's x86 headers hold **6613** such wrappers, **5973** of
//! which call a builtin, and `vppinfra/clib.h` includes `<x86intrin.h>` — so essentially every
//! VPP translation unit was losing them.
//!
//! **Every row was measured against gcc 13.3.0**, never read from documentation, using gcc's own
//! diagnostics as the oracle: the call is passed to `void take(struct Z)` and the resulting
//! `note: expected 'struct Z' but argument is of type 'RET'` names the return type. Cross-checked
//! on a sample with `_Generic`/`sizeof`/`__builtin_classify_type`, and confirmed
//! `-march`-independent — 1653 argument observations across two flag sets, zero disagreements.
//! ISA flags decide *whether* gcc declares a builtin, never its type, and the headers parse
//! unconditionally because they use `#pragma GCC target` rather than `#ifdef`.
//!
//! **Only the return type is recorded.** The signature is interned unprototyped, so nothing is
//! claimed about parameters — measuring what a builtin *returns* says nothing about what it
//! takes, and a wrong parameter type would turn every call into a false diagnostic.
//!
//! **82 names are deliberately absent** and keep `Ty::Error`, lowering to an opaque effect:
//! 46 type-generic (`__atomic_*`, `__sync_*`, whose result is the pointee's type and cannot be a
//! constant), 10 front-end special forms (`__builtin_choose_expr`, `__builtin_shuffle`, …),
//! 15 gcc does not declare on x86-64, and 11 that are not builtins at all — including
//! `__builtin_va_list`, which is a **type keyword** and must never be given a function type.

/// A scalar kind, resolved to a `TyId` against the target's widths.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
// The generator emits the full scalar lattice so a re-run needs no edit here; gcc happens to
// return no `signed char`, so `I8` has no row today.
#[allow(dead_code)]
pub(crate) enum B {
    Void,
    Bool,
    Char,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    ILong,
    ULong,
    I64,
    U64,
    F16,
    F32,
    F64,
    F80,
    F128,
    BF16,
    F32Ext,
    F64Ext,
    F32xExt,
    F64xExt,
}

/// What a builtin returns.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ret {
    Scalar(B),
    Ptr(B),
    Vector { elem: B, lanes: u32 },
}

/// The measured return type of `name`, or `None` to leave it poison and opaque.
pub(crate) fn measured_return(name: &str) -> Option<Ret> {
    use Ret::*;
    Some(match name {
        "__atomic_clear" => Scalar(B::Void),
        "__atomic_is_lock_free" => Scalar(B::Bool),
        "__atomic_signal_fence" => Scalar(B::Void),
        "__atomic_test_and_set" => Scalar(B::Bool),
        "__atomic_thread_fence" => Scalar(B::Void),
        "__builtin___memcpy_chk" => Ptr(B::Void),
        "__builtin___memmove_chk" => Ptr(B::Void),
        "__builtin___mempcpy_chk" => Ptr(B::Void),
        "__builtin___memset_chk" => Ptr(B::Void),
        "__builtin___snprintf_chk" => Scalar(B::I32),
        "__builtin___sprintf_chk" => Scalar(B::I32),
        "__builtin___stpcpy_chk" => Ptr(B::Char),
        "__builtin___stpncpy_chk" => Ptr(B::Char),
        "__builtin___strcat_chk" => Ptr(B::Char),
        "__builtin___strcpy_chk" => Ptr(B::Char),
        "__builtin___strncat_chk" => Ptr(B::Char),
        "__builtin___strncpy_chk" => Ptr(B::Char),
        "__builtin___vsnprintf_chk" => Scalar(B::I32),
        "__builtin___vsprintf_chk" => Scalar(B::I32),
        "__builtin_alloca" => Ptr(B::Void),
        "__builtin_bswap16" => Scalar(B::U16),
        "__builtin_bswap32" => Scalar(B::U32),
        "__builtin_bswap64" => Scalar(B::ULong),
        "__builtin_clz" => Scalar(B::I32),
        "__builtin_clzll" => Scalar(B::I32),
        "__builtin_copysignf128" => Scalar(B::F128),
        "__builtin_copysignq" => Scalar(B::F128),
        "__builtin_ctz" => Scalar(B::I32),
        "__builtin_ctzll" => Scalar(B::I32),
        "__builtin_dynamic_object_size" => Scalar(B::ULong),
        "__builtin_expect" => Scalar(B::ILong),
        "__builtin_fabs" => Scalar(B::F64),
        "__builtin_fabsf128" => Scalar(B::F128),
        "__builtin_fabsq" => Scalar(B::F128),
        "__builtin_fclose" => Scalar(B::I32),
        "__builtin_ffs" => Scalar(B::I32),
        "__builtin_ffsll" => Scalar(B::I32),
        "__builtin_frame_address" => Ptr(B::Void),
        "__builtin_free" => Scalar(B::Void),
        "__builtin_huge_val" => Scalar(B::F64),
        "__builtin_huge_valf" => Scalar(B::F32),
        "__builtin_huge_valf128" => Scalar(B::F128),
        "__builtin_huge_valf16" => Scalar(B::F16),
        "__builtin_huge_valf32" => Scalar(B::F32Ext),
        "__builtin_huge_valf32x" => Scalar(B::F32xExt),
        "__builtin_huge_valf64" => Scalar(B::F64Ext),
        "__builtin_huge_valf64x" => Scalar(B::F64xExt),
        "__builtin_huge_vall" => Scalar(B::F80),
        "__builtin_ia32_2intersectd128" => Scalar(B::Void),
        "__builtin_ia32_2intersectd256" => Scalar(B::Void),
        "__builtin_ia32_2intersectd512" => Scalar(B::Void),
        "__builtin_ia32_2intersectq128" => Scalar(B::Void),
        "__builtin_ia32_2intersectq256" => Scalar(B::Void),
        "__builtin_ia32_2intersectq512" => Scalar(B::Void),
        "__builtin_ia32_4fmaddps" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_4fmaddps_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_4fmaddss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_4fmaddss_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_4fnmaddps" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_4fnmaddps_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_4fnmaddss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_4fnmaddss_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_aadd32" => Scalar(B::Void),
        "__builtin_ia32_aadd64" => Scalar(B::Void),
        "__builtin_ia32_aand32" => Scalar(B::Void),
        "__builtin_ia32_aand64" => Scalar(B::Void),
        "__builtin_ia32_addcarryx_u32" => Scalar(B::U8),
        "__builtin_ia32_addcarryx_u64" => Scalar(B::U8),
        "__builtin_ia32_addpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_addpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_addpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_addph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_addph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_addph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_addph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_addps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_addps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_addps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_addsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_addsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_addsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_addsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_addsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_addss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_addss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_addss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_addsubpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_addsubpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_addsubps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_addsubps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_aesdec128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_aesdec128kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesdec256kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesdeclast128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_aesdecwide128kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesdecwide256kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesenc128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_aesenc128kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesenc256kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesenclast128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_aesencwide128kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesencwide256kl_u8" => Scalar(B::U8),
        "__builtin_ia32_aesimc128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_aeskeygenassist128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_alignd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_alignd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_alignd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_alignq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_alignq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_alignq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_andnotsi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_andnpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_andnpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_andnpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_andnpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_andnpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_andnps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_andnps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_andnps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_andnps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_andnps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_andpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_andpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_andpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_andpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_andpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_andps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_andps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_andps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_andps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_andps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_aor32" => Scalar(B::Void),
        "__builtin_ia32_aor64" => Scalar(B::Void),
        "__builtin_ia32_axor32" => Scalar(B::Void),
        "__builtin_ia32_axor64" => Scalar(B::Void),
        "__builtin_ia32_bextr_u32" => Scalar(B::U32),
        "__builtin_ia32_bextr_u64" => Scalar(B::U64),
        "__builtin_ia32_bextri_u32" => Scalar(B::U32),
        "__builtin_ia32_bextri_u64" => Scalar(B::U64),
        "__builtin_ia32_blendmb_128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_blendmb_256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_blendmb_512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_blendmd_128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_blendmd_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_blendmd_512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_blendmpd_128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_blendmpd_256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_blendmpd_512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_blendmps_128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_blendmps_256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_blendmps_512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_blendmq_128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_blendmq_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_blendmq_512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_blendmw_128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_blendmw_256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_blendmw_512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_blendpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_blendpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_blendps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_blendps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_blendvpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_blendvpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_blendvps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_blendvps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_broadcastf32x2_256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_broadcastf32x2_512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_broadcastf32x4_256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_broadcastf32x4_512" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_broadcastf32x8_512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_broadcastf64x2_256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_broadcastf64x2_512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_broadcastf64x4_512" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_broadcasti32x2_128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_broadcasti32x2_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_broadcasti32x2_512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_broadcasti32x4_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_broadcasti32x4_512" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_broadcasti32x8_512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_broadcasti64x2_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_broadcasti64x2_512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_broadcasti64x4_512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_broadcastmb128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_broadcastmb256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_broadcastmb512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_broadcastmw128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_broadcastmw256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_broadcastmw512" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_broadcastsd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_broadcastsd512" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_broadcastss128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_broadcastss256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_broadcastss512" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_bsrdi" => Scalar(B::I64),
        "__builtin_ia32_bsrsi" => Scalar(B::I32),
        "__builtin_ia32_bzhi_di" => Scalar(B::U64),
        "__builtin_ia32_bzhi_si" => Scalar(B::U32),
        "__builtin_ia32_cldemote" => Scalar(B::Void),
        "__builtin_ia32_clflush" => Scalar(B::Void),
        "__builtin_ia32_clflushopt" => Scalar(B::Void),
        "__builtin_ia32_clrssbsy" => Scalar(B::Void),
        "__builtin_ia32_clui" => Scalar(B::Void),
        "__builtin_ia32_clwb" => Scalar(B::Void),
        "__builtin_ia32_clzero" => Scalar(B::Void),
        "__builtin_ia32_cmpb128_mask" => Scalar(B::U16),
        "__builtin_ia32_cmpb256_mask" => Scalar(B::U32),
        "__builtin_ia32_cmpb512_mask" => Scalar(B::U64),
        "__builtin_ia32_cmpccxadd" => Scalar(B::I32),
        "__builtin_ia32_cmpccxadd64" => Scalar(B::I64),
        "__builtin_ia32_cmpd128_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpd256_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpd512_mask" => Scalar(B::U16),
        "__builtin_ia32_cmpeqpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpeqps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpeqsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpeqss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpgepd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpgeps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpgtpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpgtps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmplepd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpleps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmplesd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpless" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpltpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpltps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpltsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpltss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpneqpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpneqps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpneqsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpneqss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpngepd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpngeps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpngtpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpngtps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpnlepd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpnleps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpnlesd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpnless" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpnltpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpnltps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpnltsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpnltss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpordpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpordps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpordsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpordss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmppd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmppd128_mask" => Scalar(B::U8),
        "__builtin_ia32_cmppd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cmppd256_mask" => Scalar(B::Char),
        "__builtin_ia32_cmppd512_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpph128_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpph256_mask" => Scalar(B::U16),
        "__builtin_ia32_cmpph512_mask" => Scalar(B::U32),
        "__builtin_ia32_cmpph512_mask_round" => Scalar(B::U32),
        "__builtin_ia32_cmpps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpps128_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cmpps256_mask" => Scalar(B::Char),
        "__builtin_ia32_cmpps512_mask" => Scalar(B::U16),
        "__builtin_ia32_cmpq128_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpq256_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpq512_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpsd_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpsh_mask_round" => Scalar(B::U8),
        "__builtin_ia32_cmpss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpss_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpunordpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpunordps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpunordsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cmpunordss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cmpw128_mask" => Scalar(B::U8),
        "__builtin_ia32_cmpw256_mask" => Scalar(B::U16),
        "__builtin_ia32_cmpw512_mask" => Scalar(B::U32),
        "__builtin_ia32_comieq" => Scalar(B::I32),
        "__builtin_ia32_comige" => Scalar(B::I32),
        "__builtin_ia32_comigt" => Scalar(B::I32),
        "__builtin_ia32_comile" => Scalar(B::I32),
        "__builtin_ia32_comilt" => Scalar(B::I32),
        "__builtin_ia32_comineq" => Scalar(B::I32),
        "__builtin_ia32_comisdeq" => Scalar(B::I32),
        "__builtin_ia32_comisdge" => Scalar(B::I32),
        "__builtin_ia32_comisdgt" => Scalar(B::I32),
        "__builtin_ia32_comisdle" => Scalar(B::I32),
        "__builtin_ia32_comisdlt" => Scalar(B::I32),
        "__builtin_ia32_comisdneq" => Scalar(B::I32),
        "__builtin_ia32_compressdf128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_compressdf256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_compressdf512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_compressdi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_compressdi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_compressdi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_compresshi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_compresshi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_compresshi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_compressqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_compressqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_compressqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_compresssf128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_compresssf256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_compresssf512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_compresssi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_compresssi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_compresssi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_compressstoredf128_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoredf256_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoredf512_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoredi128_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoredi256_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoredi512_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoresf128_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoresf256_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoresf512_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoresi128_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoresi256_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoresi512_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoreuhi128_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoreuhi256_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoreuhi512_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoreuqi128_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoreuqi256_mask" => Scalar(B::Void),
        "__builtin_ia32_compressstoreuqi512_mask" => Scalar(B::Void),
        "__builtin_ia32_crc32di" => Scalar(B::U64),
        "__builtin_ia32_crc32hi" => Scalar(B::U32),
        "__builtin_ia32_crc32qi" => Scalar(B::U32),
        "__builtin_ia32_crc32si" => Scalar(B::U32),
        "__builtin_ia32_cvtb2mask128" => Scalar(B::U16),
        "__builtin_ia32_cvtb2mask256" => Scalar(B::U32),
        "__builtin_ia32_cvtb2mask512" => Scalar(B::U64),
        "__builtin_ia32_cvtbf2sf" => Scalar(B::F32),
        "__builtin_ia32_cvtd2mask128" => Scalar(B::U8),
        "__builtin_ia32_cvtd2mask256" => Scalar(B::U8),
        "__builtin_ia32_cvtd2mask512" => Scalar(B::U16),
        "__builtin_ia32_cvtdq2pd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtdq2pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtdq2pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtdq2pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtdq2pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_cvtdq2ps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtdq2ps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtdq2ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cvtdq2ps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cvtdq2ps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_cvtmask2b128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_cvtmask2b256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_cvtmask2b512" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_cvtmask2d128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtmask2d256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvtmask2d512" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_cvtmask2q128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvtmask2q256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvtmask2q512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvtmask2w128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_cvtmask2w256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_cvtmask2w512" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_cvtne2ps2bf16_v16bf" => Vector {
            elem: B::BF16,
            lanes: 16,
        },
        "__builtin_ia32_cvtne2ps2bf16_v16bf_mask" => Vector {
            elem: B::BF16,
            lanes: 16,
        },
        "__builtin_ia32_cvtne2ps2bf16_v16bf_maskz" => Vector {
            elem: B::BF16,
            lanes: 16,
        },
        "__builtin_ia32_cvtne2ps2bf16_v32bf" => Vector {
            elem: B::BF16,
            lanes: 32,
        },
        "__builtin_ia32_cvtne2ps2bf16_v32bf_mask" => Vector {
            elem: B::BF16,
            lanes: 32,
        },
        "__builtin_ia32_cvtne2ps2bf16_v32bf_maskz" => Vector {
            elem: B::BF16,
            lanes: 32,
        },
        "__builtin_ia32_cvtne2ps2bf16_v8bf" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtne2ps2bf16_v8bf_mask" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtne2ps2bf16_v8bf_maskz" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtneps2bf16_v16sf" => Vector {
            elem: B::BF16,
            lanes: 16,
        },
        "__builtin_ia32_cvtneps2bf16_v16sf_mask" => Vector {
            elem: B::BF16,
            lanes: 16,
        },
        "__builtin_ia32_cvtneps2bf16_v16sf_maskz" => Vector {
            elem: B::BF16,
            lanes: 16,
        },
        "__builtin_ia32_cvtneps2bf16_v4sf" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtneps2bf16_v4sf_mask" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtneps2bf16_v4sf_maskz" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtneps2bf16_v8sf" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtneps2bf16_v8sf_mask" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtneps2bf16_v8sf_maskz" => Vector {
            elem: B::BF16,
            lanes: 8,
        },
        "__builtin_ia32_cvtpd2dq" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2dq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2dq256" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2dq256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2dq512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvtpd2pi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_cvtpd2ps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2ps256" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2ps256_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2ps512_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cvtpd2ps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2qq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvtpd2qq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2qq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvtpd2udq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2udq256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2udq512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvtpd2uqq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvtpd2uqq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvtpd2uqq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvtpi2pd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtpi2ps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2dq" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2dq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2dq256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvtps2dq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvtps2dq512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_cvtps2pd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtps2pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtps2pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_cvtps2pi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_cvtps2qq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvtps2qq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2qq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvtps2udq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2udq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvtps2udq512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_cvtps2uqq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvtps2uqq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvtps2uqq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvtq2mask128" => Scalar(B::U8),
        "__builtin_ia32_cvtq2mask256" => Scalar(B::U8),
        "__builtin_ia32_cvtq2mask512" => Scalar(B::U8),
        "__builtin_ia32_cvtqq2pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtqq2pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtqq2pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_cvtqq2ps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtqq2ps256_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtqq2ps512_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cvtsd2si" => Scalar(B::I32),
        "__builtin_ia32_cvtsd2si64" => Scalar(B::I64),
        "__builtin_ia32_cvtsd2ss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtsd2ss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtsd2ss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtsi2sd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtsi2sd64" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtsi2ss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtsi2ss32" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtsi2ss64" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtsi642sd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtsi642ss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtss2sd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtss2sd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtss2sd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtss2si" => Scalar(B::I32),
        "__builtin_ia32_cvtss2si64" => Scalar(B::I64),
        "__builtin_ia32_cvttpd2dq" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2dq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2dq256" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2dq256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2dq512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvttpd2pi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_cvttpd2qq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvttpd2qq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2qq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvttpd2udq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2udq256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2udq512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvttpd2uqq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvttpd2uqq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvttpd2uqq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvttps2dq" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttps2dq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttps2dq256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvttps2dq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvttps2dq512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_cvttps2pi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_cvttps2qq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvttps2qq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvttps2qq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvttps2udq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_cvttps2udq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_cvttps2udq512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_cvttps2uqq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_cvttps2uqq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_cvttps2uqq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_cvttsd2si" => Scalar(B::I32),
        "__builtin_ia32_cvttsd2si64" => Scalar(B::I64),
        "__builtin_ia32_cvttss2si" => Scalar(B::I32),
        "__builtin_ia32_cvttss2si64" => Scalar(B::I64),
        "__builtin_ia32_cvtudq2pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtudq2pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtudq2pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_cvtudq2ps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtudq2ps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cvtudq2ps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_cvtuqq2pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtuqq2pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_cvtuqq2pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_cvtuqq2ps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtuqq2ps256_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtuqq2ps512_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_cvtusi2sd32" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtusi2sd64" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_cvtusi2ss32" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtusi2ss64" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_cvtw2mask128" => Scalar(B::U8),
        "__builtin_ia32_cvtw2mask256" => Scalar(B::U16),
        "__builtin_ia32_cvtw2mask512" => Scalar(B::U32),
        "__builtin_ia32_dbpsadbw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_dbpsadbw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_dbpsadbw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_directstoreu_u32" => Scalar(B::Void),
        "__builtin_ia32_directstoreu_u64" => Scalar(B::Void),
        "__builtin_ia32_divpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_divpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_divpd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_divph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_divph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_divph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_divph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_divps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_divps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_divps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_divsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_divsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_divsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_divsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_divsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_divss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_divss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_divss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_dpbf16ps_v16sf" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_dpbf16ps_v16sf_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_dpbf16ps_v16sf_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_dpbf16ps_v4sf" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_dpbf16ps_v4sf_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_dpbf16ps_v4sf_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_dpbf16ps_v8sf" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_dpbf16ps_v8sf_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_dpbf16ps_v8sf_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_dppd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_dpps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_dpps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_emms" => Scalar(B::Void),
        "__builtin_ia32_encodekey128_u32" => Scalar(B::U32),
        "__builtin_ia32_encodekey256_u32" => Scalar(B::U32),
        "__builtin_ia32_enqcmd" => Scalar(B::I32),
        "__builtin_ia32_enqcmds" => Scalar(B::I32),
        "__builtin_ia32_exp2pd_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_exp2ps_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_expanddf128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_expanddf128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_expanddf256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_expanddf256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_expanddf512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_expanddf512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_expanddi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_expanddi128_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_expanddi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_expanddi256_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_expanddi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_expanddi512_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_expandhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_expandhi128_maskz" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_expandhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_expandhi256_maskz" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_expandhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_expandhi512_maskz" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_expandloaddf128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_expandloaddf128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_expandloaddf256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_expandloaddf256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_expandloaddf512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_expandloaddf512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_expandloaddi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_expandloaddi128_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_expandloaddi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_expandloaddi256_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_expandloaddi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_expandloaddi512_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_expandloadhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_expandloadhi128_maskz" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_expandloadhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_expandloadhi256_maskz" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_expandloadhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_expandloadhi512_maskz" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_expandloadqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_expandloadqi128_maskz" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_expandloadqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_expandloadqi256_maskz" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_expandloadqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_expandloadqi512_maskz" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_expandloadsf128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_expandloadsf128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_expandloadsf256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_expandloadsf256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_expandloadsf512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_expandloadsf512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_expandloadsi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_expandloadsi128_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_expandloadsi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_expandloadsi256_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_expandloadsi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_expandloadsi512_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_expandqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_expandqi128_maskz" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_expandqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_expandqi256_maskz" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_expandqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_expandqi512_maskz" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_expandsf128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_expandsf128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_expandsf256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_expandsf256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_expandsf512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_expandsf512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_expandsi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_expandsi128_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_expandsi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_expandsi256_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_expandsi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_expandsi512_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_extract128i256" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_extractf32x4_256_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_extractf32x4_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_extractf32x8_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_extractf64x2_256_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_extractf64x2_512_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_extractf64x4_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_extracti32x4_256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_extracti32x4_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_extracti32x8_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_extracti64x2_256_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_extracti64x2_512_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_extracti64x4_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_extrq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_extrqi" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_femms" => Scalar(B::Void),
        "__builtin_ia32_fixupimmpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_fixupimmpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_fixupimmpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_fixupimmpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_fixupimmpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_fixupimmpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_fixupimmps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_fixupimmps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_fixupimmps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_fixupimmps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_fixupimmps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_fixupimmps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_fixupimmsd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_fixupimmsd_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_fixupimmss_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_fixupimmss_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_fpclasspd128_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclasspd256_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclasspd512_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclassph128_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclassph256_mask" => Scalar(B::I16),
        "__builtin_ia32_fpclassph512_mask" => Scalar(B::I32),
        "__builtin_ia32_fpclassps128_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclassps256_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclassps512_mask" => Scalar(B::I16),
        "__builtin_ia32_fpclasssd_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclasssh_mask" => Scalar(B::Char),
        "__builtin_ia32_fpclassss_mask" => Scalar(B::Char),
        "__builtin_ia32_fxrstor" => Scalar(B::Void),
        "__builtin_ia32_fxrstor64" => Scalar(B::Void),
        "__builtin_ia32_fxsave" => Scalar(B::Void),
        "__builtin_ia32_fxsave64" => Scalar(B::Void),
        "__builtin_ia32_gather3div2df" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_gather3div2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_gather3div4df" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_gather3div4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_gather3div4sf" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_gather3div4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_gather3div8sf" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_gather3div8si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_gather3siv2df" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_gather3siv2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_gather3siv4df" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_gather3siv4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_gather3siv4sf" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_gather3siv4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_gather3siv8sf" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_gather3siv8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_gatherdiv16sf" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_gatherdiv16si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_gatherdiv2df" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_gatherdiv2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_gatherdiv4df" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_gatherdiv4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_gatherdiv4sf" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_gatherdiv4sf256" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_gatherdiv4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_gatherdiv4si256" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_gatherdiv8df" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_gatherdiv8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_gatherpfdpd" => Scalar(B::Void),
        "__builtin_ia32_gatherpfdps" => Scalar(B::Void),
        "__builtin_ia32_gatherpfqpd" => Scalar(B::Void),
        "__builtin_ia32_gatherpfqps" => Scalar(B::Void),
        "__builtin_ia32_gathersiv16sf" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_gathersiv16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_gathersiv2df" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_gathersiv2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_gathersiv4df" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_gathersiv4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_gathersiv4sf" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_gathersiv4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_gathersiv8df" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_gathersiv8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_gathersiv8sf" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_gathersiv8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_getexppd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_getexppd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_getexppd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_getexpph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_getexpph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_getexpph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_getexpps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_getexpps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_getexpps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_getexpsd128_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_getexpsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_getexpsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_getexpss128_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_getexpss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_getmantpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_getmantpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_getmantpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_getmantph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_getmantph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_getmantph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_getmantps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_getmantps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_getmantps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_getmantsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_getmantsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_getmantsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_getmantss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_getmantss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_haddpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_haddpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_haddps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_haddps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_hreset" => Scalar(B::Void),
        "__builtin_ia32_hsubpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_hsubpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_hsubps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_hsubps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_incsspd" => Scalar(B::Void),
        "__builtin_ia32_incsspq" => Scalar(B::Void),
        "__builtin_ia32_insert128i256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_insertf32x4_256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_insertf32x4_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_insertf32x8_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_insertf64x2_256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_insertf64x2_512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_insertf64x4_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_inserti32x4_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_inserti32x4_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_inserti32x8_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_inserti64x2_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_inserti64x2_512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_inserti64x4_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_insertps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_insertq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_insertqi" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_kadddi" => Scalar(B::U64),
        "__builtin_ia32_kaddhi" => Scalar(B::U16),
        "__builtin_ia32_kaddqi" => Scalar(B::U8),
        "__builtin_ia32_kaddsi" => Scalar(B::U32),
        "__builtin_ia32_kanddi" => Scalar(B::U64),
        "__builtin_ia32_kandhi" => Scalar(B::U16),
        "__builtin_ia32_kandndi" => Scalar(B::U64),
        "__builtin_ia32_kandnhi" => Scalar(B::U16),
        "__builtin_ia32_kandnqi" => Scalar(B::U8),
        "__builtin_ia32_kandnsi" => Scalar(B::U32),
        "__builtin_ia32_kandqi" => Scalar(B::U8),
        "__builtin_ia32_kandsi" => Scalar(B::U32),
        "__builtin_ia32_kmovb" => Scalar(B::U8),
        "__builtin_ia32_kmovd" => Scalar(B::U32),
        "__builtin_ia32_kmovq" => Scalar(B::U64),
        "__builtin_ia32_kmovw" => Scalar(B::U16),
        "__builtin_ia32_knotdi" => Scalar(B::U64),
        "__builtin_ia32_knothi" => Scalar(B::U16),
        "__builtin_ia32_knotqi" => Scalar(B::U8),
        "__builtin_ia32_knotsi" => Scalar(B::U32),
        "__builtin_ia32_kordi" => Scalar(B::U64),
        "__builtin_ia32_korhi" => Scalar(B::U16),
        "__builtin_ia32_korqi" => Scalar(B::U8),
        "__builtin_ia32_korsi" => Scalar(B::U32),
        "__builtin_ia32_kortestcdi" => Scalar(B::U64),
        "__builtin_ia32_kortestchi" => Scalar(B::U16),
        "__builtin_ia32_kortestcqi" => Scalar(B::U8),
        "__builtin_ia32_kortestcsi" => Scalar(B::U32),
        "__builtin_ia32_kortestzdi" => Scalar(B::U64),
        "__builtin_ia32_kortestzhi" => Scalar(B::U16),
        "__builtin_ia32_kortestzqi" => Scalar(B::U8),
        "__builtin_ia32_kortestzsi" => Scalar(B::U32),
        "__builtin_ia32_kshiftlidi" => Scalar(B::U64),
        "__builtin_ia32_kshiftlihi" => Scalar(B::U16),
        "__builtin_ia32_kshiftliqi" => Scalar(B::U8),
        "__builtin_ia32_kshiftlisi" => Scalar(B::U32),
        "__builtin_ia32_kshiftridi" => Scalar(B::U64),
        "__builtin_ia32_kshiftrihi" => Scalar(B::U16),
        "__builtin_ia32_kshiftriqi" => Scalar(B::U8),
        "__builtin_ia32_kshiftrisi" => Scalar(B::U32),
        "__builtin_ia32_ktestcdi" => Scalar(B::U64),
        "__builtin_ia32_ktestchi" => Scalar(B::U16),
        "__builtin_ia32_ktestcqi" => Scalar(B::U8),
        "__builtin_ia32_ktestcsi" => Scalar(B::U32),
        "__builtin_ia32_ktestzdi" => Scalar(B::U64),
        "__builtin_ia32_ktestzhi" => Scalar(B::U16),
        "__builtin_ia32_ktestzqi" => Scalar(B::U8),
        "__builtin_ia32_ktestzsi" => Scalar(B::U32),
        "__builtin_ia32_kunpckdi" => Scalar(B::U64),
        "__builtin_ia32_kunpckhi" => Scalar(B::U16),
        "__builtin_ia32_kunpcksi" => Scalar(B::U32),
        "__builtin_ia32_kxnordi" => Scalar(B::U64),
        "__builtin_ia32_kxnorhi" => Scalar(B::U16),
        "__builtin_ia32_kxnorqi" => Scalar(B::U8),
        "__builtin_ia32_kxnorsi" => Scalar(B::U32),
        "__builtin_ia32_kxordi" => Scalar(B::U64),
        "__builtin_ia32_kxorhi" => Scalar(B::U16),
        "__builtin_ia32_kxorqi" => Scalar(B::U8),
        "__builtin_ia32_kxorsi" => Scalar(B::U32),
        "__builtin_ia32_lddqu" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_lddqu256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_ldmxcsr" => Scalar(B::Void),
        "__builtin_ia32_ldtilecfg" => Scalar(B::Void),
        "__builtin_ia32_lfence" => Scalar(B::Void),
        "__builtin_ia32_llwpcb" => Scalar(B::Void),
        "__builtin_ia32_loadapd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_loadapd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_loadapd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_loadaps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_loadaps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_loadaps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_loaddqudi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_loaddqudi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_loaddqudi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_loaddquhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_loaddquhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_loaddquhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_loaddquqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_loaddquqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_loaddquqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_loaddqusi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_loaddqusi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_loaddqusi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_loadhpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_loadhps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_loadiwkey" => Scalar(B::Void),
        "__builtin_ia32_loadlpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_loadlps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_loadsd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_loadsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_loadss_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_loadupd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_loadupd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_loadupd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_loadups128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_loadups256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_loadups512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_lwpins32" => Scalar(B::U8),
        "__builtin_ia32_lwpins64" => Scalar(B::U8),
        "__builtin_ia32_lwpval32" => Scalar(B::Void),
        "__builtin_ia32_lwpval64" => Scalar(B::Void),
        "__builtin_ia32_lzcnt_u16" => Scalar(B::U16),
        "__builtin_ia32_lzcnt_u32" => Scalar(B::U32),
        "__builtin_ia32_lzcnt_u64" => Scalar(B::U64),
        "__builtin_ia32_maskloadd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_maskloadd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_maskloadpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_maskloadpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_maskloadps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_maskloadps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_maskloadq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_maskloadq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_maskmovdqu" => Scalar(B::Void),
        "__builtin_ia32_maskmovq" => Scalar(B::Void),
        "__builtin_ia32_maskstored" => Scalar(B::Void),
        "__builtin_ia32_maskstored256" => Scalar(B::Void),
        "__builtin_ia32_maskstorepd" => Scalar(B::Void),
        "__builtin_ia32_maskstorepd256" => Scalar(B::Void),
        "__builtin_ia32_maskstoreps" => Scalar(B::Void),
        "__builtin_ia32_maskstoreps256" => Scalar(B::Void),
        "__builtin_ia32_maskstoreq" => Scalar(B::Void),
        "__builtin_ia32_maskstoreq256" => Scalar(B::Void),
        "__builtin_ia32_maxpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_maxpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_maxpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_maxpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_maxpd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_maxph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_maxph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_maxph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_maxph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_maxps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_maxps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_maxps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_maxps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_maxps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_maxsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_maxsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_maxsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_maxsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_maxsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_maxss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_maxss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_maxss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_mfence" => Scalar(B::Void),
        "__builtin_ia32_minpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_minpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_minpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_minpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_minpd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_minph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_minph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_minph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_minph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_minps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_minps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_minps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_minps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_minps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_minsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_minsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_minsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_minsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_minsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_minss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_minss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_minss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_monitor" => Scalar(B::Void),
        "__builtin_ia32_monitorx" => Scalar(B::Void),
        "__builtin_ia32_movapd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_movapd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_movapd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_movaps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movaps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_movaps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_movddup128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_movddup256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_movddup256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_movddup512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_movdir64b" => Scalar(B::Void),
        "__builtin_ia32_movdqa32_128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_movdqa32_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_movdqa32_512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_movdqa32load128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_movdqa32load256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_movdqa32load512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_movdqa32store128_mask" => Scalar(B::Void),
        "__builtin_ia32_movdqa32store256_mask" => Scalar(B::Void),
        "__builtin_ia32_movdqa32store512_mask" => Scalar(B::Void),
        "__builtin_ia32_movdqa64_128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_movdqa64_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_movdqa64_512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_movdqa64load128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_movdqa64load256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_movdqa64load512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_movdqa64store128_mask" => Scalar(B::Void),
        "__builtin_ia32_movdqa64store256_mask" => Scalar(B::Void),
        "__builtin_ia32_movdqa64store512_mask" => Scalar(B::Void),
        "__builtin_ia32_movdquhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_movdquhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_movdquhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_movdquqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_movdquqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_movdquqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_movesd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_movess_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movhlps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movlhps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movmskpd" => Scalar(B::I32),
        "__builtin_ia32_movmskpd256" => Scalar(B::I32),
        "__builtin_ia32_movmskps" => Scalar(B::I32),
        "__builtin_ia32_movmskps256" => Scalar(B::I32),
        "__builtin_ia32_movntdq" => Scalar(B::Void),
        "__builtin_ia32_movntdq256" => Scalar(B::Void),
        "__builtin_ia32_movntdq512" => Scalar(B::Void),
        "__builtin_ia32_movntdqa" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_movntdqa256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_movntdqa512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_movnti" => Scalar(B::Void),
        "__builtin_ia32_movnti64" => Scalar(B::Void),
        "__builtin_ia32_movntpd" => Scalar(B::Void),
        "__builtin_ia32_movntpd256" => Scalar(B::Void),
        "__builtin_ia32_movntpd512" => Scalar(B::Void),
        "__builtin_ia32_movntps" => Scalar(B::Void),
        "__builtin_ia32_movntps256" => Scalar(B::Void),
        "__builtin_ia32_movntps512" => Scalar(B::Void),
        "__builtin_ia32_movntq" => Scalar(B::Void),
        "__builtin_ia32_movntsd" => Scalar(B::Void),
        "__builtin_ia32_movntss" => Scalar(B::Void),
        "__builtin_ia32_movq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_movsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_movshdup" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movshdup128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movshdup256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_movshdup256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_movshdup512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_movsldup" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movsldup128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_movsldup256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_movsldup256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_movsldup512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_movss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_mpsadbw128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_mpsadbw256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_mulpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_mulpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_mulpd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_mulph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_mulph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_mulph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_mulph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_mulps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_mulps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_mulps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_mulsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_mulsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_mulsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_mulsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_mulsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_mulss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_mulss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_mulss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_mwait" => Scalar(B::Void),
        "__builtin_ia32_mwaitx" => Scalar(B::Void),
        "__builtin_ia32_orpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_orpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_orpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_orpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_orpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_orps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_orps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_orps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_orps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_orps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_pabsb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pabsb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pabsb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pabsb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pabsb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pabsb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pabsd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pabsd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pabsd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pabsd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pabsd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pabsd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pabsq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pabsq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pabsq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pabsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pabsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pabsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pabsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pabsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pabsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_packssdw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_packssdw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_packssdw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_packssdw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_packssdw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_packssdw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_packsswb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_packsswb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_packsswb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_packsswb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_packsswb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_packsswb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_packusdw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_packusdw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_packusdw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_packusdw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_packusdw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_packuswb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_packuswb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_packuswb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_packuswb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_packuswb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_packuswb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_paddb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_paddb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_paddb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_paddb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_paddd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_paddd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_paddd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_paddd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_paddq" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_paddq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_paddq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_paddq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_paddsb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_paddsb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_paddsb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_paddsb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_paddsb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_paddsb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_paddsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_paddsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_paddsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_paddsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_paddsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_paddsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_paddusb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_paddusb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_paddusb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_paddusb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_paddusb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_paddusb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_paddusw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_paddusw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_paddusw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_paddusw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_paddusw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_paddusw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_paddw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_paddw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_paddw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_paddw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_palignr" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_palignr128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_palignr128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_palignr256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_palignr256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_palignr512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_palignr512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pand" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pandd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pandd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pandd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pandn" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pandn128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pandnd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pandnd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pandnd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pandnq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pandnq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pandnq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pandq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pandq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pandq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pause" => Scalar(B::Void),
        "__builtin_ia32_pavgb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pavgb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pavgb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pavgb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pavgb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pavgb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pavgusb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pavgw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pavgw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pavgw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pavgw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pavgw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pavgw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pblendd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pblendd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pblendvb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pblendvb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pblendw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pblendw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastb128_gpr_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pbroadcastb256_gpr_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pbroadcastb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pbroadcastb512_gpr_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pbroadcastb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pbroadcastd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pbroadcastd128_gpr_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pbroadcastd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pbroadcastd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastd256_gpr_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastd512" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastd512_gpr_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pbroadcastq128_gpr_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pbroadcastq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pbroadcastq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pbroadcastq256_gpr_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pbroadcastq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pbroadcastq512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastq512_gpr_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastw128_gpr_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pbroadcastw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastw256_gpr_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pbroadcastw512_gpr_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pbroadcastw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pclmulqdq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pcmpeqb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pcmpeqb128_mask" => Scalar(B::U16),
        "__builtin_ia32_pcmpeqb256_mask" => Scalar(B::U32),
        "__builtin_ia32_pcmpeqb512_mask" => Scalar(B::U64),
        "__builtin_ia32_pcmpeqd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pcmpeqd128_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpeqd256_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpeqd512_mask" => Scalar(B::U16),
        "__builtin_ia32_pcmpeqq128_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpeqq256_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpeqq512_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpeqw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pcmpeqw128_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpeqw256_mask" => Scalar(B::U16),
        "__builtin_ia32_pcmpeqw512_mask" => Scalar(B::U32),
        "__builtin_ia32_pcmpestri128" => Scalar(B::I32),
        "__builtin_ia32_pcmpestria128" => Scalar(B::I32),
        "__builtin_ia32_pcmpestric128" => Scalar(B::I32),
        "__builtin_ia32_pcmpestrio128" => Scalar(B::I32),
        "__builtin_ia32_pcmpestris128" => Scalar(B::I32),
        "__builtin_ia32_pcmpestriz128" => Scalar(B::I32),
        "__builtin_ia32_pcmpestrm128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pcmpgtb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pcmpgtb128_mask" => Scalar(B::U16),
        "__builtin_ia32_pcmpgtb256_mask" => Scalar(B::U32),
        "__builtin_ia32_pcmpgtb512_mask" => Scalar(B::U64),
        "__builtin_ia32_pcmpgtd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pcmpgtd128_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpgtd256_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpgtd512_mask" => Scalar(B::U16),
        "__builtin_ia32_pcmpgtq128_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpgtq256_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpgtq512_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpgtw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pcmpgtw128_mask" => Scalar(B::U8),
        "__builtin_ia32_pcmpgtw256_mask" => Scalar(B::U16),
        "__builtin_ia32_pcmpgtw512_mask" => Scalar(B::U32),
        "__builtin_ia32_pcmpistri128" => Scalar(B::I32),
        "__builtin_ia32_pcmpistria128" => Scalar(B::I32),
        "__builtin_ia32_pcmpistric128" => Scalar(B::I32),
        "__builtin_ia32_pcmpistrio128" => Scalar(B::I32),
        "__builtin_ia32_pcmpistris128" => Scalar(B::I32),
        "__builtin_ia32_pcmpistriz128" => Scalar(B::I32),
        "__builtin_ia32_pcmpistrm128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pd256_pd" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_pd512_256pd" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_pd512_pd" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_pd_pd256" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_pdep_di" => Scalar(B::U64),
        "__builtin_ia32_pdep_si" => Scalar(B::U32),
        "__builtin_ia32_permdf256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_permdf256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_permdf512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_permdi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_permdi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_permdi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_permti256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_permvardf256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_permvardf512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_permvardi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_permvardi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_permvarhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_permvarhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_permvarhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_permvarqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_permvarqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_permvarqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_permvarsf256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_permvarsf256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_permvarsf512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_permvarsi256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_permvarsi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_permvarsi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pext_di" => Scalar(B::U64),
        "__builtin_ia32_pext_si" => Scalar(B::U32),
        "__builtin_ia32_pf2id" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pf2iw" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pfacc" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfadd" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfcmpeq" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pfcmpge" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pfcmpgt" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pfmax" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfmin" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfmul" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfnacc" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfpnacc" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfrcp" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfrcpit1" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfrcpit2" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfrsqit1" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfrsqrt" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfsub" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pfsubr" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_phaddd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_phaddd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_phaddd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_phaddsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_phaddsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_phaddsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_phaddw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_phaddw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_phaddw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_phminposuw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_phsubd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_phsubd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_phsubd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_phsubsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_phsubsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_phsubsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_phsubw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_phsubw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_phsubw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pi2fd" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pi2fw" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pmaddubsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmaddubsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmaddubsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmaddubsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmaddubsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmaddubsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmaddwd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pmaddwd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmaddwd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmaddwd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmaddwd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmaddwd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmaxsb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmaxsb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmaxsb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmaxsb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmaxsb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pmaxsd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmaxsd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmaxsd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmaxsd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmaxsd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmaxsq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmaxsq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmaxsq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmaxsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmaxsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmaxsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmaxsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmaxsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmaxsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmaxub" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pmaxub128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmaxub128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmaxub256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmaxub256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmaxub512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pmaxud128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmaxud128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmaxud256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmaxud256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmaxud512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmaxuq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmaxuq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmaxuq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmaxuw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmaxuw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmaxuw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmaxuw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmaxuw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pminsb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pminsb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pminsb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pminsb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pminsb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pminsd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pminsd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pminsd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pminsd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pminsd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pminsq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pminsq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pminsq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pminsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pminsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pminsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pminsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pminsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pminsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pminub" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pminub128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pminub128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pminub256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pminub256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pminub512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pminud128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pminud128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pminud256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pminud256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pminud512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pminuq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pminuq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pminuq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pminuw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pminuw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pminuw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pminuw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pminuw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmovdb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovdb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovdb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovdb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovdb512_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovdb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovdw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovdw128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovdw256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovdw256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovdw512_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovdw512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovmskb" => Scalar(B::I32),
        "__builtin_ia32_pmovmskb128" => Scalar(B::I32),
        "__builtin_ia32_pmovmskb256" => Scalar(B::I32),
        "__builtin_ia32_pmovqb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovqb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovqb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqb512_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovqb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovqd128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqd256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovqd256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqd512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovqd512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovqw128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqw256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovqw256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovqw512_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovqw512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsdb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovsdb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsdb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovsdb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsdb512_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovsdb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsdw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsdw128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsdw256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsdw256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsdw512_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovsdw512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovsqb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovsqb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqb512_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovsqb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovsqd128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqd256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovsqd256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqd512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovsqd512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsqw128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqw256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsqw256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsqw512_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsqw512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovswb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovswb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovswb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovswb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovswb512_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmovswb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovsxbd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxbd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxbd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxbd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxbd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmovsxbq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovsxbq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovsxbq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxbq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxbq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxbw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxbw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxbw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovsxbw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovsxbw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmovsxdq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovsxdq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovsxdq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxdq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxdq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxwd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxwd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxwd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxwd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovsxwd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmovsxwq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovsxwq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovsxwq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxwq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovsxwq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmovusdb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovusdb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusdb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovusdb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusdb512_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovusdb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusdw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovusdw128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusdw256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovusdw256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusdw512_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovusdw512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovusqb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovusqb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqb512_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovusqb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovusqd128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqd256_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovusqd256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqd512_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovusqd512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovusqw128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqw256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovusqw256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovusqw512_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovusqw512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovuswb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovuswb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovuswb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovuswb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovuswb512_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmovuswb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovwb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovwb128mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovwb256_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pmovwb256mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovwb512_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pmovwb512mem_mask" => Scalar(B::Void),
        "__builtin_ia32_pmovzxbd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxbd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxbd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxbd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxbd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmovzxbq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovzxbq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovzxbq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxbq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxbq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxbw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxbw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxbw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovzxbw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmovzxbw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmovzxdq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovzxdq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovzxdq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxdq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxdq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxwd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxwd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxwd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxwd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmovzxwd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmovzxwq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovzxwq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmovzxwq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxwq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmovzxwq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmuldq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmuldq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmuldq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmuldq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmuldq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmulhrsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmulhrsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmulhrsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmulhrsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmulhrsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmulhrsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmulhrw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmulhuw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmulhuw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmulhuw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmulhuw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmulhuw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmulhuw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmulhw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmulhw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmulhw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmulhw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmulhw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmulhw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmulld128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pmulld256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pmulld512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pmullq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmullq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmullq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pmullw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_pmullw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pmullw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pmullw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pmuludq" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_pmuludq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmuludq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pmuludq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmuludq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pmuludq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_por" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pord128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pord256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pord512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_porq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_porq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_porq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_prefetch" => Scalar(B::Void),
        "__builtin_ia32_prefetchi" => Scalar(B::Void),
        "__builtin_ia32_prold128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_prold256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_prold512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_prolq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_prolq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_prolq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_prolvd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_prolvd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_prolvd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_prolvq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_prolvq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_prolvq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_prord128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_prord256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_prord512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_prorq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_prorq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_prorq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_prorvd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_prorvd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_prorvd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_prorvq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_prorvq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_prorvq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_ps256_ps" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_ps512_256ps" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_ps512_ps" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_ps_ps256" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_psadbw" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_psadbw128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psadbw256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psadbw512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pshufb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_pshufb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pshufb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_pshufb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pshufb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_pshufb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_pshufd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pshufd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pshufd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pshufd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pshufd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pshufhw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pshufhw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pshufhw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pshufhw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pshufhw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pshuflw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pshuflw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_pshuflw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pshuflw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pshuflw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pshufw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psignb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_psignb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_psignb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_psignd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_psignd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psignd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psignw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psignw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psignw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_pslld" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pslld128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pslld128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pslld256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pslld256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pslld512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pslldi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pslldi128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pslldi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pslldi256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pslldi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pslldi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pslldq512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pslldqi128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pslldqi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllq" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_psllq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psllq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psllq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psllqi" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_psllqi128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psllqi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psllqi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllqi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllqi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psllv16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psllv16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psllv2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psllv2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psllv32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psllv4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllv4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psllv4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psllv4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psllv8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psllv8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psllv8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psllv8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psllw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psllw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psllw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psllw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psllw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psllw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psllwi" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psllwi128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psllwi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psllwi256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psllwi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psllwi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psrad" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_psrad128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrad128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrad256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrad256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrad512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psradi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_psradi128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psradi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psradi256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psradi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psradi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psraq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psraq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psraq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psraqi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psraqi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psraqi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psrav16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrav16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psrav32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psrav4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrav4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrav8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psrav8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrav8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrav8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psravq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psravq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psraw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psraw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psraw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psraw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psraw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psraw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psrawi" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psrawi128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrawi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrawi256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrawi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrawi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psrld" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_psrld128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrld128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrld256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrld256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrld512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psrldi" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_psrldi128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrldi128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrldi256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrldi256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrldi512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psrldq512" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psrldqi128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrldqi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlq" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_psrlq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrlq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrlq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psrlqi" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_psrlqi128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrlqi128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrlqi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlqi256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlqi512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psrlv16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrlv16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psrlv2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrlv2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psrlv32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psrlv4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlv4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psrlv4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrlv4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psrlv8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psrlv8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrlv8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrlv8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psrlw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psrlw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrlw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrlw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrlw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrlw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psrlwi" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psrlwi128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrlwi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psrlwi256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrlwi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psrlwi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psubb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_psubb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_psubb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_psubb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_psubd" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_psubd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_psubd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_psubd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_psubq" => Vector {
            elem: B::I64,
            lanes: 1,
        },
        "__builtin_ia32_psubq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_psubq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_psubq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_psubsb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_psubsb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_psubsb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_psubsb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_psubsb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_psubsb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_psubsw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psubsw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psubsw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psubsw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psubsw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psubsw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psubusb" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_psubusb128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_psubusb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_psubusb256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_psubusb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_psubusb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_psubusw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psubusw128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psubusw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psubusw256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psubusw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psubusw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_psubw" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_psubw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_psubw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_psubw512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pswapdsf" => Vector {
            elem: B::F32,
            lanes: 2,
        },
        "__builtin_ia32_pternlogd128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pternlogd128_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pternlogd256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pternlogd256_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pternlogd512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pternlogd512_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pternlogq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pternlogq128_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pternlogq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pternlogq256_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pternlogq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_pternlogq512_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_ptestc128" => Scalar(B::I32),
        "__builtin_ia32_ptestc256" => Scalar(B::I32),
        "__builtin_ia32_ptestmb128" => Scalar(B::U16),
        "__builtin_ia32_ptestmb256" => Scalar(B::U32),
        "__builtin_ia32_ptestmb512" => Scalar(B::U64),
        "__builtin_ia32_ptestmd128" => Scalar(B::U8),
        "__builtin_ia32_ptestmd256" => Scalar(B::U8),
        "__builtin_ia32_ptestmd512" => Scalar(B::U16),
        "__builtin_ia32_ptestmq128" => Scalar(B::U8),
        "__builtin_ia32_ptestmq256" => Scalar(B::U8),
        "__builtin_ia32_ptestmq512" => Scalar(B::U8),
        "__builtin_ia32_ptestmw128" => Scalar(B::U8),
        "__builtin_ia32_ptestmw256" => Scalar(B::U16),
        "__builtin_ia32_ptestmw512" => Scalar(B::U32),
        "__builtin_ia32_ptestnmb128" => Scalar(B::U16),
        "__builtin_ia32_ptestnmb256" => Scalar(B::U32),
        "__builtin_ia32_ptestnmb512" => Scalar(B::U64),
        "__builtin_ia32_ptestnmd128" => Scalar(B::U8),
        "__builtin_ia32_ptestnmd256" => Scalar(B::U8),
        "__builtin_ia32_ptestnmd512" => Scalar(B::U16),
        "__builtin_ia32_ptestnmq128" => Scalar(B::U8),
        "__builtin_ia32_ptestnmq256" => Scalar(B::U8),
        "__builtin_ia32_ptestnmq512" => Scalar(B::U8),
        "__builtin_ia32_ptestnmw128" => Scalar(B::U8),
        "__builtin_ia32_ptestnmw256" => Scalar(B::U16),
        "__builtin_ia32_ptestnmw512" => Scalar(B::U32),
        "__builtin_ia32_ptestnzc128" => Scalar(B::I32),
        "__builtin_ia32_ptestnzc256" => Scalar(B::I32),
        "__builtin_ia32_ptestz128" => Scalar(B::I32),
        "__builtin_ia32_ptestz256" => Scalar(B::I32),
        "__builtin_ia32_ptwrite32" => Scalar(B::Void),
        "__builtin_ia32_ptwrite64" => Scalar(B::Void),
        "__builtin_ia32_punpckhbw" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_punpckhbw128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_punpckhbw128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_punpckhbw256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_punpckhbw256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_punpckhbw512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_punpckhdq" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_punpckhdq128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_punpckhdq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_punpckhdq256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_punpckhdq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_punpckhdq512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_punpckhqdq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_punpckhqdq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_punpckhqdq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_punpckhqdq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_punpckhqdq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_punpckhwd" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_punpckhwd128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_punpckhwd128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_punpckhwd256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_punpckhwd256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_punpckhwd512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_punpcklbw" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_punpcklbw128" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_punpcklbw128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_punpcklbw256" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_punpcklbw256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_punpcklbw512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_punpckldq" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_punpckldq128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_punpckldq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_punpckldq256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_punpckldq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_punpckldq512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_punpcklqdq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_punpcklqdq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_punpcklqdq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_punpcklqdq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_punpcklqdq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_punpcklwd" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_punpcklwd128" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_punpcklwd128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_punpcklwd256" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_punpcklwd256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_punpcklwd512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_pxor" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_pxord128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_pxord256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_pxord512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_pxorq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_pxorq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_pxorq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_rangepd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rangepd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_rangepd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_rangeps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rangeps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_rangeps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_rangesd128_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rangess128_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcp14pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rcp14pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_rcp14pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_rcp14ps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcp14ps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_rcp14ps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_rcp14sd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rcp14sd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rcp14ss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcp14ss_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcp28pd_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_rcp28ps_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_rcp28sd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rcp28sd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rcp28ss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcp28ss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcpph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_rcpph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_rcpph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_rcpps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rcpps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_rcpsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_rcpss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rdfsbase32" => Scalar(B::U32),
        "__builtin_ia32_rdfsbase64" => Scalar(B::U64),
        "__builtin_ia32_rdgsbase32" => Scalar(B::U32),
        "__builtin_ia32_rdgsbase64" => Scalar(B::U64),
        "__builtin_ia32_rdpid" => Scalar(B::U32),
        "__builtin_ia32_rdpkru" => Scalar(B::U32),
        "__builtin_ia32_rdpmc" => Scalar(B::U64),
        "__builtin_ia32_rdrand16_step" => Scalar(B::I32),
        "__builtin_ia32_rdrand32_step" => Scalar(B::I32),
        "__builtin_ia32_rdrand64_step" => Scalar(B::I32),
        "__builtin_ia32_rdseed_di_step" => Scalar(B::I32),
        "__builtin_ia32_rdseed_hi_step" => Scalar(B::I32),
        "__builtin_ia32_rdseed_si_step" => Scalar(B::I32),
        "__builtin_ia32_rdsspd" => Scalar(B::U32),
        "__builtin_ia32_rdsspq" => Scalar(B::U64),
        "__builtin_ia32_rdtsc" => Scalar(B::U64),
        "__builtin_ia32_rdtscp" => Scalar(B::U64),
        "__builtin_ia32_readeflags_u64" => Scalar(B::U64),
        "__builtin_ia32_reducepd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_reducepd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_reducepd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_reducepd512_mask_round" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_reduceph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_reduceph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_reduceph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_reduceps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_reduceps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_reduceps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_reduceps512_mask_round" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_reducesd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_reducesd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_reducesh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_reducess_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_reducess_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rndscalepd_128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rndscalepd_256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_rndscalepd_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_rndscaleph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_rndscaleph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_rndscaleph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_rndscaleps_128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rndscaleps_256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_rndscaleps_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_rndscalesd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rndscalesh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_rndscaless_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rolhi" => Scalar(B::U16),
        "__builtin_ia32_rolqi" => Scalar(B::U8),
        "__builtin_ia32_rorhi" => Scalar(B::U16),
        "__builtin_ia32_rorqi" => Scalar(B::U8),
        "__builtin_ia32_roundpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_roundpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_roundps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_roundps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_roundsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_roundss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrt14pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rsqrt14pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_rsqrt14pd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_rsqrt14ps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrt14ps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_rsqrt14ps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_rsqrt14sd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rsqrt14sd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rsqrt14ss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrt14ss_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrt28pd_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_rsqrt28ps_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_rsqrt28sd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rsqrt28sd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_rsqrt28ss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrt28ss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrtph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_rsqrtph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_rsqrtph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_rsqrtps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rsqrtps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_rsqrtsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_rsqrtss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_rstorssp" => Scalar(B::Void),
        "__builtin_ia32_saveprevssp" => Scalar(B::Void),
        "__builtin_ia32_sbb_u32" => Scalar(B::U8),
        "__builtin_ia32_sbb_u64" => Scalar(B::U8),
        "__builtin_ia32_scalefpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_scalefpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_scalefpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_scalefph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_scalefph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_scalefph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_scalefps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_scalefps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_scalefps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_scalefsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_scalefsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_scalefss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_scatterdiv16sf" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv16si" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv2df" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv2di" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv4df" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv4di" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv4sf" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv4si" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv8df" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv8di" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv8sf" => Scalar(B::Void),
        "__builtin_ia32_scatterdiv8si" => Scalar(B::Void),
        "__builtin_ia32_scatterpfdpd" => Scalar(B::Void),
        "__builtin_ia32_scatterpfdps" => Scalar(B::Void),
        "__builtin_ia32_scatterpfqpd" => Scalar(B::Void),
        "__builtin_ia32_scatterpfqps" => Scalar(B::Void),
        "__builtin_ia32_scattersiv16sf" => Scalar(B::Void),
        "__builtin_ia32_scattersiv16si" => Scalar(B::Void),
        "__builtin_ia32_scattersiv2df" => Scalar(B::Void),
        "__builtin_ia32_scattersiv2di" => Scalar(B::Void),
        "__builtin_ia32_scattersiv4df" => Scalar(B::Void),
        "__builtin_ia32_scattersiv4di" => Scalar(B::Void),
        "__builtin_ia32_scattersiv4sf" => Scalar(B::Void),
        "__builtin_ia32_scattersiv4si" => Scalar(B::Void),
        "__builtin_ia32_scattersiv8df" => Scalar(B::Void),
        "__builtin_ia32_scattersiv8di" => Scalar(B::Void),
        "__builtin_ia32_scattersiv8sf" => Scalar(B::Void),
        "__builtin_ia32_scattersiv8si" => Scalar(B::Void),
        "__builtin_ia32_senduipi" => Scalar(B::Void),
        "__builtin_ia32_serialize" => Scalar(B::Void),
        "__builtin_ia32_setssbsy" => Scalar(B::Void),
        "__builtin_ia32_sfence" => Scalar(B::Void),
        "__builtin_ia32_sha1msg1" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_sha1msg2" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_sha1nexte" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_sha1rnds4" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_sha256msg1" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_sha256msg2" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_sha256rnds2" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_shuf_f32x4_256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_shuf_f32x4_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_shuf_f64x2_256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_shuf_f64x2_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_shuf_i32x4_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_shuf_i32x4_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_shuf_i64x2_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_shuf_i64x2_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_shufpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_shufpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_shufpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_shufpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_shufpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_shufps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_shufps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_shufps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_shufps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_shufps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_si256_si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_si512_256si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_si512_si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_si_si256" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_slwpcb" => Ptr(B::Void),
        "__builtin_ia32_sqrtpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_sqrtpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_sqrtpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_sqrtpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_sqrtpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_sqrtph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_sqrtph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_sqrtph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_sqrtps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_sqrtps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_sqrtps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_sqrtps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_sqrtps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_sqrtsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_sqrtsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_sqrtsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_sqrtss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_sqrtss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_stmxcsr" => Scalar(B::U32),
        "__builtin_ia32_storeapd128_mask" => Scalar(B::Void),
        "__builtin_ia32_storeapd256_mask" => Scalar(B::Void),
        "__builtin_ia32_storeapd512_mask" => Scalar(B::Void),
        "__builtin_ia32_storeaps128_mask" => Scalar(B::Void),
        "__builtin_ia32_storeaps256_mask" => Scalar(B::Void),
        "__builtin_ia32_storeaps512_mask" => Scalar(B::Void),
        "__builtin_ia32_storedqudi128_mask" => Scalar(B::Void),
        "__builtin_ia32_storedqudi256_mask" => Scalar(B::Void),
        "__builtin_ia32_storedqudi512_mask" => Scalar(B::Void),
        "__builtin_ia32_storedquhi128_mask" => Scalar(B::Void),
        "__builtin_ia32_storedquhi256_mask" => Scalar(B::Void),
        "__builtin_ia32_storedquhi512_mask" => Scalar(B::Void),
        "__builtin_ia32_storedquqi128_mask" => Scalar(B::Void),
        "__builtin_ia32_storedquqi256_mask" => Scalar(B::Void),
        "__builtin_ia32_storedquqi512_mask" => Scalar(B::Void),
        "__builtin_ia32_storedqusi128_mask" => Scalar(B::Void),
        "__builtin_ia32_storedqusi256_mask" => Scalar(B::Void),
        "__builtin_ia32_storedqusi512_mask" => Scalar(B::Void),
        "__builtin_ia32_storehps" => Scalar(B::Void),
        "__builtin_ia32_storelps" => Scalar(B::Void),
        "__builtin_ia32_storesd_mask" => Scalar(B::Void),
        "__builtin_ia32_storesh_mask" => Scalar(B::Void),
        "__builtin_ia32_storess_mask" => Scalar(B::Void),
        "__builtin_ia32_storeupd128_mask" => Scalar(B::Void),
        "__builtin_ia32_storeupd256_mask" => Scalar(B::Void),
        "__builtin_ia32_storeupd512_mask" => Scalar(B::Void),
        "__builtin_ia32_storeups128_mask" => Scalar(B::Void),
        "__builtin_ia32_storeups256_mask" => Scalar(B::Void),
        "__builtin_ia32_storeups512_mask" => Scalar(B::Void),
        "__builtin_ia32_sttilecfg" => Scalar(B::Void),
        "__builtin_ia32_stui" => Scalar(B::Void),
        "__builtin_ia32_subpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_subpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_subpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_subph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_subph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_subph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_subph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_subps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_subps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_subps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_subsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_subsd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_subsd_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_subsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_subsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_subss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_subss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_subss_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_testui" => Scalar(B::U8),
        "__builtin_ia32_tpause" => Scalar(B::U8),
        "__builtin_ia32_tzcnt_u16" => Scalar(B::U16),
        "__builtin_ia32_tzcnt_u32" => Scalar(B::U32),
        "__builtin_ia32_tzcnt_u64" => Scalar(B::U64),
        "__builtin_ia32_ucmpb128_mask" => Scalar(B::U16),
        "__builtin_ia32_ucmpb256_mask" => Scalar(B::U32),
        "__builtin_ia32_ucmpb512_mask" => Scalar(B::U64),
        "__builtin_ia32_ucmpd128_mask" => Scalar(B::U8),
        "__builtin_ia32_ucmpd256_mask" => Scalar(B::U8),
        "__builtin_ia32_ucmpd512_mask" => Scalar(B::U16),
        "__builtin_ia32_ucmpq128_mask" => Scalar(B::U8),
        "__builtin_ia32_ucmpq256_mask" => Scalar(B::U8),
        "__builtin_ia32_ucmpq512_mask" => Scalar(B::U8),
        "__builtin_ia32_ucmpw128_mask" => Scalar(B::U8),
        "__builtin_ia32_ucmpw256_mask" => Scalar(B::U16),
        "__builtin_ia32_ucmpw512_mask" => Scalar(B::U32),
        "__builtin_ia32_ucomieq" => Scalar(B::I32),
        "__builtin_ia32_ucomige" => Scalar(B::I32),
        "__builtin_ia32_ucomigt" => Scalar(B::I32),
        "__builtin_ia32_ucomile" => Scalar(B::I32),
        "__builtin_ia32_ucomilt" => Scalar(B::I32),
        "__builtin_ia32_ucomineq" => Scalar(B::I32),
        "__builtin_ia32_ucomisdeq" => Scalar(B::I32),
        "__builtin_ia32_ucomisdge" => Scalar(B::I32),
        "__builtin_ia32_ucomisdgt" => Scalar(B::I32),
        "__builtin_ia32_ucomisdle" => Scalar(B::I32),
        "__builtin_ia32_ucomisdlt" => Scalar(B::I32),
        "__builtin_ia32_ucomisdneq" => Scalar(B::I32),
        "__builtin_ia32_umonitor" => Scalar(B::Void),
        "__builtin_ia32_umwait" => Scalar(B::U8),
        "__builtin_ia32_unpckhpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_unpckhpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_unpckhpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_unpckhpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_unpckhpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_unpckhps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_unpckhps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_unpckhps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_unpckhps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_unpckhps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_unpcklpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_unpcklpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_unpcklpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_unpcklpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_unpcklpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_unpcklps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_unpcklps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_unpcklps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_unpcklps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_unpcklps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vaesdec_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vaesdec_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vaesdeclast_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vaesdeclast_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vaesenc_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vaesenc_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vaesenclast_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vaesenclast_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vbcstnebf162ps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vbcstnebf162ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vbcstnesh2ps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vbcstnesh2ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vbroadcastf128_pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vbroadcastf128_ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vbroadcastsd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vbroadcastsd_pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vbroadcastsi256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vbroadcastss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vbroadcastss256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vbroadcastss_ps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vbroadcastss_ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcomisd" => Scalar(B::I32),
        "__builtin_ia32_vcomiss" => Scalar(B::I32),
        "__builtin_ia32_vcvtdq2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtdq2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtdq2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtneebf162ps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtneebf162ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtneeph2ps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtneeph2ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtneobf162ps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtneobf162ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtneoph2ps128" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtneoph2ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtpd2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtpd2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtpd2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2dq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2dq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2dq512_mask_round" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vcvtph2pd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vcvtph2pd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2pd512_mask_round" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2ps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2ps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2ps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vcvtph2ps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2psx128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2psx256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2psx512_mask_round" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vcvtph2qq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vcvtph2qq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2qq512_mask_round" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2udq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2udq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2udq512_mask_round" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vcvtph2uqq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vcvtph2uqq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vcvtph2uqq512_mask_round" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2uw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2uw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtph2uw512_mask_round" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vcvtph2w128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtph2w256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtph2w512_mask_round" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vcvtps2ph" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtps2ph256" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtps2ph256_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtps2ph512_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtps2ph_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtps2phx128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtps2phx256_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtps2phx512_mask_round" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtqq2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtqq2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtqq2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtsd2sh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtsd2si32" => Scalar(B::I32),
        "__builtin_ia32_vcvtsd2si64" => Scalar(B::I64),
        "__builtin_ia32_vcvtsd2usi32" => Scalar(B::U32),
        "__builtin_ia32_vcvtsd2usi64" => Scalar(B::U64),
        "__builtin_ia32_vcvtsh2sd_mask_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vcvtsh2si32_round" => Scalar(B::I32),
        "__builtin_ia32_vcvtsh2si64_round" => Scalar(B::I64),
        "__builtin_ia32_vcvtsh2ss_mask_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vcvtsh2usi32_round" => Scalar(B::U32),
        "__builtin_ia32_vcvtsh2usi64_round" => Scalar(B::U64),
        "__builtin_ia32_vcvtsi2sh32_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtsi2sh64_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtss2sh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtss2si32" => Scalar(B::I32),
        "__builtin_ia32_vcvtss2si64" => Scalar(B::I64),
        "__builtin_ia32_vcvtss2usi32" => Scalar(B::U32),
        "__builtin_ia32_vcvtss2usi64" => Scalar(B::U64),
        "__builtin_ia32_vcvttph2dq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vcvttph2dq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vcvttph2dq512_mask_round" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vcvttph2qq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vcvttph2qq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vcvttph2qq512_mask_round" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vcvttph2udq128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vcvttph2udq256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vcvttph2udq512_mask_round" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vcvttph2uqq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vcvttph2uqq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vcvttph2uqq512_mask_round" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vcvttph2uw128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvttph2uw256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vcvttph2uw512_mask_round" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vcvttph2w128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vcvttph2w256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vcvttph2w512_mask_round" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vcvttsd2si32" => Scalar(B::I32),
        "__builtin_ia32_vcvttsd2si64" => Scalar(B::I64),
        "__builtin_ia32_vcvttsd2usi32" => Scalar(B::U32),
        "__builtin_ia32_vcvttsd2usi64" => Scalar(B::U64),
        "__builtin_ia32_vcvttsh2si32_round" => Scalar(B::I32),
        "__builtin_ia32_vcvttsh2si64_round" => Scalar(B::I64),
        "__builtin_ia32_vcvttsh2usi32_round" => Scalar(B::U32),
        "__builtin_ia32_vcvttsh2usi64_round" => Scalar(B::U64),
        "__builtin_ia32_vcvttss2si32" => Scalar(B::I32),
        "__builtin_ia32_vcvttss2si64" => Scalar(B::I64),
        "__builtin_ia32_vcvttss2usi32" => Scalar(B::U32),
        "__builtin_ia32_vcvttss2usi64" => Scalar(B::U64),
        "__builtin_ia32_vcvtudq2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtudq2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtudq2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtuqq2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtuqq2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtuqq2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtusi2sh32_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtusi2sh64_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtuw2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtuw2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtuw2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vcvtw2ph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vcvtw2ph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vcvtw2ph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vec_ext_v16qi" => Scalar(B::Char),
        "__builtin_ia32_vec_ext_v2di" => Scalar(B::I64),
        "__builtin_ia32_vec_ext_v2si" => Scalar(B::I32),
        "__builtin_ia32_vec_ext_v4hi" => Scalar(B::I16),
        "__builtin_ia32_vec_ext_v4sf" => Scalar(B::F32),
        "__builtin_ia32_vec_ext_v4si" => Scalar(B::I32),
        "__builtin_ia32_vec_ext_v8hi" => Scalar(B::I16),
        "__builtin_ia32_vec_init_v2si" => Vector {
            elem: B::I32,
            lanes: 2,
        },
        "__builtin_ia32_vec_init_v4hi" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_vec_init_v8qi" => Vector {
            elem: B::Char,
            lanes: 8,
        },
        "__builtin_ia32_vec_set_v16qi" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vec_set_v2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vec_set_v4hi" => Vector {
            elem: B::I16,
            lanes: 4,
        },
        "__builtin_ia32_vec_set_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vec_set_v8hi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vextractf128_pd256" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vextractf128_ps256" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vextractf128_si256" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vfcmaddcph128" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcph256" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfcmaddcph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfcmaddcph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfcmaddcph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfcmaddcph512_mask3_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfcmaddcph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfcmaddcph512_maskz_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfcmaddcph512_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfcmaddcsh_mask3_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcsh_maskz_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmaddcsh_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmulcph128" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmulcph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmulcph256" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfcmulcph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfcmulcph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfcmulcph512_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfcmulcsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfcmulcsh_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcph128" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcph256" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddcph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddcph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddcph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddcph512_mask3_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddcph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddcph512_maskz_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddcph512_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddcsh_mask3_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcsh_maskz_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddcsh_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddpd128_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddpd256_mask3" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddpd512_mask3" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddph512_mask3" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddph512_maskz" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddps128_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddps256_mask3" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddps512_mask3" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsd3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsd3_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsd3_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsd3_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsd3_round" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsh3_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsh3_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsh3_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddss3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddss3_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddss3_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddss3_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddss3_round" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsubpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsubpd128_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsubpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmaddsubpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubpd256_mask3" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubpd512_mask3" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddsubph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddsubph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddsubph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddsubph512_mask3" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddsubph512_maskz" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmaddsubps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubps128_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmaddsubps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubps256_mask3" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmaddsubps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddsubps512_mask3" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmaddsubps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubaddpd128_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubaddpd256_mask3" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubaddpd512_mask3" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubaddph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubaddph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubaddph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubaddph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubaddph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubaddph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubaddph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmsubaddph512_mask3" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmsubaddph512_maskz" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmsubaddps128_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubaddps256_mask3" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubaddps512_mask3" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubpd128_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubpd256_mask3" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubpd512_mask3" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmsubph512_mask3" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmsubph512_maskz" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmsubps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubps128_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubps256_mask3" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubps512_mask3" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfmsubsd3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubsd3_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfmsubsh3_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmsubss3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmsubss3_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfmulcph128" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmulcph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmulcph256" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmulcph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfmulcph512_mask_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmulcph512_round" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfmulcsh_mask_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfmulcsh_round" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmaddpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmaddpd128_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmaddpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmaddpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddpd256_mask3" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddpd512_mask3" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfnmaddph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfnmaddph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfnmaddph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfnmaddph512_mask3" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfnmaddph512_maskz" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfnmaddps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddps128_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmaddps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddps256_mask3" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfnmaddps512_mask3" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfnmaddps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfnmaddsd3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmaddsh3_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddsh3_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddsh3_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmaddss3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmsubpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmsubpd128_mask3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmsubpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmsubpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubpd256_mask3" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubpd512_mask3" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubph128_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubph128_mask3" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubph128_maskz" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubph256_mask" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfnmsubph256_mask3" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfnmsubph256_maskz" => Vector {
            elem: B::F16,
            lanes: 16,
        },
        "__builtin_ia32_vfnmsubph512_mask" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfnmsubph512_mask3" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfnmsubph512_maskz" => Vector {
            elem: B::F16,
            lanes: 32,
        },
        "__builtin_ia32_vfnmsubps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubps128_mask3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfnmsubps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubps256_mask3" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfnmsubps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfnmsubps512_mask3" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfnmsubps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vfnmsubsd3" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfnmsubss3" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfrczpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfrczpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vfrczps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vfrczps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vfrczsd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vfrczss" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vgf2p8affineinvqb_v16qi" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vgf2p8affineinvqb_v16qi_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vgf2p8affineinvqb_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vgf2p8affineinvqb_v32qi_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vgf2p8affineinvqb_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vgf2p8affineinvqb_v64qi_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vgf2p8affineqb_v16qi" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vgf2p8affineqb_v16qi_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vgf2p8affineqb_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vgf2p8affineqb_v32qi_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vgf2p8affineqb_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vgf2p8affineqb_v64qi_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vgf2p8mulb_v16qi" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vgf2p8mulb_v16qi_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vgf2p8mulb_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vgf2p8mulb_v32qi_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vgf2p8mulb_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vgf2p8mulb_v64qi_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vinsertf128_pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vinsertf128_ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vinsertf128_si256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vmovsh_mask" => Vector {
            elem: B::F16,
            lanes: 8,
        },
        "__builtin_ia32_vp4dpwssd" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vp4dpwssd_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vp4dpwssds" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vp4dpwssds_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpclmulqdq_v4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpclmulqdq_v8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpcmov" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcmov256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpcomeqb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomeqd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomeqq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomequb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomequd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomequq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomequw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomeqw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomfalseb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomfalsed" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomfalseq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomfalseub" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomfalseud" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomfalseuq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomfalseuw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomfalsew" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomgeb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomged" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomgeq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomgeub" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomgeud" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomgeuq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomgeuw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomgew" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomgtb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomgtd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomgtq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomgtub" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomgtud" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomgtuq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomgtuw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomgtw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomleb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomled" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomleq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomleub" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomleud" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomleuq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomleuw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomlew" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomltb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomltd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomltq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomltub" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomltud" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomltuq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomltuw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomltw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomneqb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomneqd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomneqq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomnequb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomnequd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomnequq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomnequw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomneqw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomtrueb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomtrued" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomtrueq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomtrueub" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpcomtrueud" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpcomtrueuq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpcomtrueuw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpcomtruew" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpconflictdi_128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpconflictdi_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpconflictdi_512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpconflictsi_128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpconflictsi_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpconflictsi_512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbssd128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbssd256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbssds128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbssds256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbsud128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbsud256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbsuds128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbsuds256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbusd_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbusd_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbusd_v16si_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbusd_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbusd_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbusd_v4si_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbusd_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbusd_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbusd_v8si_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbusds_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbusds_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbusds_v16si_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpbusds_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbusds_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbusds_v4si_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbusds_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbusds_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbusds_v8si_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbuud128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbuud256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpbuuds128" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpbuuds256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpwssd_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpwssd_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpwssd_v16si_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpwssd_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpwssd_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpwssd_v4si_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpwssd_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpwssd_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpwssd_v8si_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpwssds_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpwssds_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpwssds_v16si_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpdpwssds_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpwssds_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpwssds_v4si_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpdpwssds_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpwssds_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpdpwssds_v8si_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vperm2f128_pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vperm2f128_ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vperm2f128_si256" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpermi2vard128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpermi2vard256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpermi2vard512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpermi2varhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpermi2varhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpermi2varhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpermi2varpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermi2varpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermi2varpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vpermi2varps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermi2varps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermi2varps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vpermi2varq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpermi2varq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpermi2varq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpermi2varqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpermi2varqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vpermi2varqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vpermil2pd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermil2pd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermil2ps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermil2ps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermilpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermilpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermilpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermilpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vpermilpd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermilps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermilps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermilps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermilps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vpermilps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermilvarpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermilvarpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermilvarpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermilvarpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vpermilvarpd_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermilvarps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermilvarps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermilvarps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermilvarps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vpermilvarps_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2vard128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2vard128_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2vard256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2vard256_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2vard512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2vard512_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varhi128_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varhi128_maskz" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varhi256_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varhi256_maskz" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varhi512_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpermt2varhi512_maskz" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpermt2varpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermt2varpd128_maskz" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_vpermt2varpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2varpd256_maskz" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2varpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varpd512_maskz" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2varps128_maskz" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2varps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varps256_maskz" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varps512_maskz" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpermt2varq128_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpermt2varq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2varq256_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpermt2varq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varq512_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpermt2varqi128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varqi128_maskz" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpermt2varqi256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vpermt2varqi256_maskz" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vpermt2varqi512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vpermt2varqi512_maskz" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vphaddbd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vphaddbq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphaddbw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vphadddq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphaddubd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vphaddubq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphaddubw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vphaddudq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphadduwd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vphadduwq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphaddwd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vphaddwq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphsubbw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vphsubdq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vphsubwd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vplzcntd_128_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vplzcntd_256_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vplzcntd_512_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vplzcntq_128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vplzcntq_256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vplzcntq_512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpmacsdd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpmacsdqh" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpmacsdql" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpmacssdd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpmacssdqh" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpmacssdql" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpmacsswd" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpmacssww" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpmacswd" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpmacsww" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpmadcsswd" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpmadcswd" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpmadd52huq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpmadd52huq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpmadd52huq128_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpmadd52huq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpmadd52huq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpmadd52huq256_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpmadd52huq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpmadd52huq512_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpmadd52luq128" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpmadd52luq128_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpmadd52luq128_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpmadd52luq256" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpmadd52luq256_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpmadd52luq256_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpmadd52luq512_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpmadd52luq512_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpmultishiftqb128_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpmultishiftqb256_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vpmultishiftqb512_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vpopcountb_v16qi" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpopcountb_v16qi_mask" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpopcountb_v32qi" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vpopcountb_v32qi_mask" => Vector {
            elem: B::Char,
            lanes: 32,
        },
        "__builtin_ia32_vpopcountb_v64qi" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vpopcountb_v64qi_mask" => Vector {
            elem: B::Char,
            lanes: 64,
        },
        "__builtin_ia32_vpopcountd_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpopcountd_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpopcountd_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpopcountd_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpopcountd_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpopcountd_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpopcountq_v2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpopcountq_v2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpopcountq_v4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpopcountq_v4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpopcountq_v8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpopcountq_v8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpopcountw_v16hi" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpopcountw_v16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpopcountw_v32hi" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpopcountw_v32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpopcountw_v8hi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpopcountw_v8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpperm" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vprotb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vprotbi" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vprotd" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vprotdi" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vprotq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vprotqi" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vprotw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vprotwi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshab" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpshad" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshaq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshaw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshlb" => Vector {
            elem: B::Char,
            lanes: 16,
        },
        "__builtin_ia32_vpshld" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshld_v16hi" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshld_v16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshld_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshld_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshld_v2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshld_v2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshld_v32hi" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshld_v32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshld_v4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshld_v4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshld_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshld_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshld_v8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshld_v8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshld_v8hi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshld_v8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshld_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshld_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v16hi" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshldv_v16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshldv_v16hi_maskz" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshldv_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshldv_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshldv_v16si_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshldv_v2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshldv_v2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshldv_v2di_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshldv_v32hi" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshldv_v32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshldv_v32hi_maskz" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshldv_v4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshldv_v4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshldv_v4di_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshldv_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshldv_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshldv_v4si_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshldv_v8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8di_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8hi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8hi_maskz" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshldv_v8si_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshlq" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshlw" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshrd_v16hi" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshrd_v16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshrd_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshrd_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshrd_v2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshrd_v2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshrd_v32hi" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshrd_v32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshrd_v4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshrd_v4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshrd_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshrd_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshrd_v8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshrd_v8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshrd_v8hi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshrd_v8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshrd_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshrd_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v16hi" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshrdv_v16hi_mask" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshrdv_v16hi_maskz" => Vector {
            elem: B::I16,
            lanes: 16,
        },
        "__builtin_ia32_vpshrdv_v16si" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshrdv_v16si_mask" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshrdv_v16si_maskz" => Vector {
            elem: B::I32,
            lanes: 16,
        },
        "__builtin_ia32_vpshrdv_v2di" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshrdv_v2di_mask" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshrdv_v2di_maskz" => Vector {
            elem: B::I64,
            lanes: 2,
        },
        "__builtin_ia32_vpshrdv_v32hi" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshrdv_v32hi_mask" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshrdv_v32hi_maskz" => Vector {
            elem: B::I16,
            lanes: 32,
        },
        "__builtin_ia32_vpshrdv_v4di" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshrdv_v4di_mask" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshrdv_v4di_maskz" => Vector {
            elem: B::I64,
            lanes: 4,
        },
        "__builtin_ia32_vpshrdv_v4si" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshrdv_v4si_mask" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshrdv_v4si_maskz" => Vector {
            elem: B::I32,
            lanes: 4,
        },
        "__builtin_ia32_vpshrdv_v8di" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8di_mask" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8di_maskz" => Vector {
            elem: B::I64,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8hi" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8hi_mask" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8hi_maskz" => Vector {
            elem: B::I16,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8si" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8si_mask" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshrdv_v8si_maskz" => Vector {
            elem: B::I32,
            lanes: 8,
        },
        "__builtin_ia32_vpshufbitqmb128_mask" => Scalar(B::U16),
        "__builtin_ia32_vpshufbitqmb256_mask" => Scalar(B::U32),
        "__builtin_ia32_vpshufbitqmb512_mask" => Scalar(B::U64),
        "__builtin_ia32_vtestcpd" => Scalar(B::I32),
        "__builtin_ia32_vtestcpd256" => Scalar(B::I32),
        "__builtin_ia32_vtestcps" => Scalar(B::I32),
        "__builtin_ia32_vtestcps256" => Scalar(B::I32),
        "__builtin_ia32_vtestnzcpd" => Scalar(B::I32),
        "__builtin_ia32_vtestnzcpd256" => Scalar(B::I32),
        "__builtin_ia32_vtestnzcps" => Scalar(B::I32),
        "__builtin_ia32_vtestnzcps256" => Scalar(B::I32),
        "__builtin_ia32_vtestzpd" => Scalar(B::I32),
        "__builtin_ia32_vtestzpd256" => Scalar(B::I32),
        "__builtin_ia32_vtestzps" => Scalar(B::I32),
        "__builtin_ia32_vtestzps256" => Scalar(B::I32),
        "__builtin_ia32_vzeroall" => Scalar(B::Void),
        "__builtin_ia32_vzeroupper" => Scalar(B::Void),
        "__builtin_ia32_wbinvd" => Scalar(B::Void),
        "__builtin_ia32_wbnoinvd" => Scalar(B::Void),
        "__builtin_ia32_wrfsbase32" => Scalar(B::Void),
        "__builtin_ia32_wrfsbase64" => Scalar(B::Void),
        "__builtin_ia32_wrgsbase32" => Scalar(B::Void),
        "__builtin_ia32_wrgsbase64" => Scalar(B::Void),
        "__builtin_ia32_writeeflags_u64" => Scalar(B::Void),
        "__builtin_ia32_wrpkru" => Scalar(B::Void),
        "__builtin_ia32_wrssd" => Scalar(B::Void),
        "__builtin_ia32_wrssq" => Scalar(B::Void),
        "__builtin_ia32_wrussd" => Scalar(B::Void),
        "__builtin_ia32_wrussq" => Scalar(B::Void),
        "__builtin_ia32_xabort" => Scalar(B::Void),
        "__builtin_ia32_xbegin" => Scalar(B::U32),
        "__builtin_ia32_xend" => Scalar(B::Void),
        "__builtin_ia32_xgetbv" => Scalar(B::U64),
        "__builtin_ia32_xorpd" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_xorpd128_mask" => Vector {
            elem: B::F64,
            lanes: 2,
        },
        "__builtin_ia32_xorpd256" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_xorpd256_mask" => Vector {
            elem: B::F64,
            lanes: 4,
        },
        "__builtin_ia32_xorpd512_mask" => Vector {
            elem: B::F64,
            lanes: 8,
        },
        "__builtin_ia32_xorps" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_xorps128_mask" => Vector {
            elem: B::F32,
            lanes: 4,
        },
        "__builtin_ia32_xorps256" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_xorps256_mask" => Vector {
            elem: B::F32,
            lanes: 8,
        },
        "__builtin_ia32_xorps512_mask" => Vector {
            elem: B::F32,
            lanes: 16,
        },
        "__builtin_ia32_xresldtrk" => Scalar(B::Void),
        "__builtin_ia32_xrstor" => Scalar(B::Void),
        "__builtin_ia32_xrstor64" => Scalar(B::Void),
        "__builtin_ia32_xrstors" => Scalar(B::Void),
        "__builtin_ia32_xrstors64" => Scalar(B::Void),
        "__builtin_ia32_xsave" => Scalar(B::Void),
        "__builtin_ia32_xsave64" => Scalar(B::Void),
        "__builtin_ia32_xsavec" => Scalar(B::Void),
        "__builtin_ia32_xsavec64" => Scalar(B::Void),
        "__builtin_ia32_xsaveopt" => Scalar(B::Void),
        "__builtin_ia32_xsaveopt64" => Scalar(B::Void),
        "__builtin_ia32_xsaves" => Scalar(B::Void),
        "__builtin_ia32_xsaves64" => Scalar(B::Void),
        "__builtin_ia32_xsetbv" => Scalar(B::Void),
        "__builtin_ia32_xsusldtrk" => Scalar(B::Void),
        "__builtin_ia32_xtest" => Scalar(B::I32),
        "__builtin_index" => Ptr(B::Char),
        "__builtin_inf" => Scalar(B::F64),
        "__builtin_inff" => Scalar(B::F32),
        "__builtin_inff128" => Scalar(B::F128),
        "__builtin_inff16" => Scalar(B::F16),
        "__builtin_inff32" => Scalar(B::F32Ext),
        "__builtin_inff32x" => Scalar(B::F32xExt),
        "__builtin_inff64" => Scalar(B::F64Ext),
        "__builtin_inff64x" => Scalar(B::F64xExt),
        "__builtin_infl" => Scalar(B::F80),
        "__builtin_infq" => Scalar(B::F128),
        "__builtin_memchr" => Ptr(B::Void),
        "__builtin_memcmp" => Scalar(B::I32),
        "__builtin_memcpy" => Ptr(B::Void),
        "__builtin_memset" => Ptr(B::Void),
        "__builtin_nan" => Scalar(B::F64),
        "__builtin_nanf" => Scalar(B::F32),
        "__builtin_nanf128" => Scalar(B::F128),
        "__builtin_nanf16" => Scalar(B::F16),
        "__builtin_nanf32" => Scalar(B::F32Ext),
        "__builtin_nanf32x" => Scalar(B::F32xExt),
        "__builtin_nanf64" => Scalar(B::F64Ext),
        "__builtin_nanf64x" => Scalar(B::F64xExt),
        "__builtin_nanl" => Scalar(B::F80),
        "__builtin_nans" => Scalar(B::F64),
        "__builtin_nansf" => Scalar(B::F32),
        "__builtin_nansf128" => Scalar(B::F128),
        "__builtin_nansf16" => Scalar(B::F16),
        "__builtin_nansf32" => Scalar(B::F32Ext),
        "__builtin_nansf32x" => Scalar(B::F32xExt),
        "__builtin_nansf64" => Scalar(B::F64Ext),
        "__builtin_nansf64x" => Scalar(B::F64xExt),
        "__builtin_nansl" => Scalar(B::F80),
        "__builtin_object_size" => Scalar(B::ULong),
        "__builtin_popcount" => Scalar(B::I32),
        "__builtin_popcountll" => Scalar(B::I32),
        "__builtin_prefetch" => Scalar(B::Void),
        "__builtin_rindex" => Ptr(B::Char),
        "__builtin_signbitl" => Scalar(B::I32),
        "__builtin_sqrt" => Scalar(B::F64),
        "__builtin_strchr" => Ptr(B::Char),
        "__builtin_strlen" => Scalar(B::ULong),
        "__builtin_strncmp" => Scalar(B::I32),
        "__builtin_strpbrk" => Ptr(B::Char),
        "__builtin_strrchr" => Ptr(B::Char),
        "__builtin_strstr" => Ptr(B::Char),
        "__builtin_unreachable" => Scalar(B::Void),
        "__builtin_va_arg_pack" => Scalar(B::I32),
        "__builtin_va_arg_pack_len" => Scalar(B::I32),
        "__sync_synchronize" => Scalar(B::Void),
        _ => return None,
    })
}
