//! **031 contract 13: two `CLIB_MARCH_VARIANT` builds of one source.**
//!
//! > 13. Two `CLIB_MARCH_VARIANT` builds: changing the source impacts both variants as distinct
//! >     entities.
//!
//! VPP compiles one source many times, once per instruction-set variant, and 060 is where that
//! lives. The mechanism is a token paste — `cpu.h`:
//!
//! ```c
//! #define _CLIB_MARCH_FN_NAME(fn) fn##_##CLIB_MARCH_VARIANT
//! #define CLIB_MULTIARCH_FN(fn) _CLIB_MULTIARCH_FN(fn,CLIB_MARCH_VARIANT)
//! ```
//!
//! so `foo` becomes `foo_x86_64_v3` in one build and `foo_x86_64_v4` in another. The two are
//! **different code**: `#if defined(CLIB_HAVE_VEC512)` is in one and not the other.
//!
//! # Why this needs no VPP knowledge in the general engine
//!
//! 001 §4 rule 4 keeps VPP-specific knowledge in `chiero-vpp`, and the paste is what makes that
//! possible: after preprocessing, the two builds declare **differently named functions**, so
//! `Entity` already tells them apart by name and the file. Nothing here has to know what
//! `CLIB_MARCH_VARIANT` is.
//!
//! `chiero-gcov` reached the same place from the other side: `FuncKey` carries a `march`, and the
//! default `MarchResolver` **splits nothing** — because a resolver guessing from a bare suffix
//! would collapse `foo_avx2` into `foo` and attribute the vector variant's coverage to the scalar
//! path. The same reasoning applies here, and it is why this is a guard rather than a feature.

use chiero_diff::{Entity, Program, impact};

/// The shape `CLIB_MULTIARCH_FN` produces, with the variant supplied by the build.
fn build(variant: &str, body: &str) -> Program {
    let src = format!(
        "#define _PASTE(a, b) a##_##b\n\
         #define _NAME(fn, v) _PASTE(fn, v)\n\
         #define MULTIARCH(fn) _NAME(fn, {variant})\n\
         static int MULTIARCH (compress) (int x) {{ {body} }}\n\
         int MULTIARCH (dispatch) (int x) {{ return MULTIARCH (compress) (x) + 1; }}\n"
    );
    Program::parse("compress.c", &src).expect("the fixture parses")
}

/// **Contract 13.** One source edit, two builds, two distinct entities — and each build's
/// impact set names only its own.
#[test]
fn two_variant_builds_are_distinct_entities() {
    let v3 = impact(&build("v3", "return x + 1;"), &build("v3", "return x + 2;"));
    let v4 = impact(&build("v4", "return x + 1;"), &build("v4", "return x + 2;"));

    assert!(
        v3.entities
            .contains_key(&Entity::function("compress.c", "compress_v3")),
        "the v3 build's own function: {:?}",
        v3.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert!(
        v4.entities
            .contains_key(&Entity::function("compress.c", "compress_v4"))
    );
    assert!(
        !v3.entities
            .contains_key(&Entity::function("compress.c", "compress_v4")),
        "and never the other build's — that would attribute one variant's change to the other"
    );
}

/// The closure follows through the pasted name too, so each variant's caller comes with it.
#[test]
fn each_variants_caller_is_reached_within_its_own_build() {
    let v3 = impact(&build("v3", "return x + 1;"), &build("v3", "return x + 2;"));
    assert!(
        v3.entities
            .contains_key(&Entity::function("compress.c", "dispatch_v3")),
        "it calls `compress_v3`: {:?}",
        v3.entities.keys().map(Entity::name).collect::<Vec<_>>()
    );
    assert!(
        !v3.entities
            .contains_key(&Entity::function("compress.c", "dispatch_v4"))
    );
}

/// **The two builds' entity sets are disjoint**, which is what "distinct entities" means: a
/// caller merging them gets both variants' functions and can tell which is which.
#[test]
fn the_two_builds_declare_disjoint_entities() {
    let v3: Vec<String> = build("v3", "return x + 1;")
        .entities()
        .map(|e| e.name().to_string())
        .collect();
    let v4: Vec<String> = build("v4", "return x + 1;")
        .entities()
        .map(|e| e.name().to_string())
        .collect();

    let shared: Vec<&String> = v3.iter().filter(|n| v4.contains(n)).collect();
    assert!(
        shared.iter().all(|n| n.starts_with('_')
            || n.contains("PASTE")
            || n.contains("NAME")
            || n.contains("MULTIARCH")),
        "only the naming macros are common to both builds; the functions are not: {shared:?}"
    );
    assert!(v3.iter().any(|n| n == "compress_v3"));
    assert!(v4.iter().any(|n| n == "compress_v4"));
}
