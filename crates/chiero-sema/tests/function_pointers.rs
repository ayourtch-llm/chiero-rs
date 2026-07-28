//! Function-pointer declarators.
//!
//! `int (*fn)(int)` types as an **integer**, so lowering declares its slot `Int(32)` and
//! storing the (correct) `Ptr` into it fails verification — which is what keeps
//! `tests/corpus/owed/indirect_call.c` out of the corpus. Wave 119 fixed the two lowering
//! defects underneath it and found this one upstream: `cty` maps `Ty::Ptr(_)` to `CTy::Ptr`
//! correctly, so the wrong answer is made here.
//!
//! Calling through a function pointer is how VPP dispatches every graph node, so this is
//! not a corner of the language.

use chiero_sema::{TargetConfig, Ty};

mod harness;
use harness::parse;

/// The analysed type of a file-scope variable, as a `Ty`.
fn ty_of(src: &str, name: &str) -> Ty {
    let p = parse(src, TargetConfig::x86_64_linux());
    assert!(
        p.analysis.diagnostics.is_empty(),
        "{:?}",
        p.analysis.diagnostics
    );
    let id = p
        .decl_ty(name)
        .unwrap_or_else(|| panic!("no file-scope `{name}`"));
    p.analysis.ty(id).clone()
}

/// **A function pointer is a pointer.**
///
/// The whole defect in one assertion: `int (*fn)(int)` must not be an integer.
#[test]
fn a_function_pointer_declarator_is_a_pointer() {
    let t = ty_of("int (*fn)(int);", "fn");
    assert!(
        matches!(t, Ty::Ptr(_)),
        "`int (*fn)(int)` is a pointer to a function, not {t:?}"
    );
}

/// **And it points at a function**, so a caller can read the signature back.
///
/// `Ty::Ptr(_)` alone is satisfied by pointing at anything — `int *` would pass the test
/// above. 023 §5's indirect-call resolution needs the callee's arity and return type, and
/// those live on the pointee.
#[test]
fn it_points_at_a_function_with_the_written_signature() {
    let p = parse("int (*fn)(int, char);", TargetConfig::x86_64_linux());
    let id = p.decl_ty("fn").expect("fn");
    let Ty::Ptr(inner) = p.analysis.ty(id).clone() else {
        panic!("not a pointer: {:?}", p.analysis.ty(id))
    };
    let Ty::Func { ret, params, .. } = p.analysis.ty(inner).clone() else {
        panic!("not a pointer to a function: {:?}", p.analysis.ty(inner))
    };
    assert!(
        matches!(p.analysis.ty(ret), Ty::Int { bits: 32, .. }),
        "the return type survived: {:?}",
        p.analysis.ty(ret)
    );
    assert_eq!(params.len(), 2, "and both parameters did");
}

/// **A plain function declarator is still a function**, not a pointer.
///
/// The negative half. A fix that wrapped every declarator in a pointer would satisfy both
/// tests above and make `int f(int);` a pointer to a function, which is a different type
/// with a different size.
#[test]
fn a_plain_function_declarator_is_not_a_pointer() {
    let p = parse("int f(int);", TargetConfig::x86_64_linux());
    let id = p
        .parsed
        .ast
        .items()
        .iter()
        .find_map(|&i| match &p.parsed.ast.decl(i).kind {
            chiero_ast::DeclKind::Func { .. } => p.analysis.ty_of_decl(i),
            _ => None,
        })
        .expect("f");
    assert!(
        matches!(p.analysis.ty(id), Ty::Func { .. }),
        "`int f(int)` is a function: {:?}",
        p.analysis.ty(id)
    );
}

/// **An ordinary object pointer is unaffected**, so the fix is about function declarators
/// and not about pointers in general.
#[test]
fn an_ordinary_pointer_is_unchanged() {
    assert!(matches!(ty_of("int *p;", "p"), Ty::Ptr(_)));
    assert!(matches!(ty_of("int n;", "n"), Ty::Int { bits: 32, .. }));
}

/// **A pointer to a function returning a pointer**, which is where a naive fix that
/// special-cases one level stops working. VPP's node registration tables are full of these.
#[test]
fn a_nested_function_pointer_type_survives() {
    let p = parse("char *(*fn)(int);", TargetConfig::x86_64_linux());
    let id = p.decl_ty("fn").expect("fn");
    let Ty::Ptr(inner) = p.analysis.ty(id).clone() else {
        panic!("not a pointer: {:?}", p.analysis.ty(id))
    };
    let Ty::Func { ret, .. } = p.analysis.ty(inner).clone() else {
        panic!("not a function: {:?}", p.analysis.ty(inner))
    };
    assert!(
        matches!(p.analysis.ty(ret), Ty::Ptr(_)),
        "it returns a pointer: {:?}",
        p.analysis.ty(ret)
    );
}

/// **A function pointer declared inside a function body.**
///
/// Every test above declares at file scope, and they all passed on arrival — so wave 119's
/// diagnosis ("sema does not type function-pointer declarators") was wrong. The fixture
/// that fails declares its pointer as a *local*, which is a different path through sema.
#[test]
fn a_local_function_pointer_is_a_pointer() {
    let p = parse(
        "static int twice(int v) { return v * 2; }\n\
         int main(void) { int (*fn)(int) = twice; return fn(7); }\n",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        p.analysis.diagnostics.is_empty(),
        "{:?}",
        p.analysis.diagnostics
    );

    // Find the local `fn` by walking the function's body declarations.
    let sym = p.symbol("fn").expect("the name is interned");
    let mut found = None;
    for id in 0..p.parsed.ast.decls().len() {
        let d = chiero_ast::DeclId(id as u32);
        if let chiero_ast::DeclKind::Var { name: Some(n), .. } = &p.parsed.ast.decl(d).kind
            && *n == sym
        {
            found = p.analysis.ty_of_decl(d);
        }
    }
    let t = found.expect("`fn` has an analysed type");
    assert!(
        matches!(p.analysis.ty(t), Ty::Ptr(_)),
        "a local `int (*fn)(int)` is a pointer, not {:?}",
        p.analysis.ty(t)
    );
}

// ---------------------------------------------------------------------------------------
// `__builtin_va_list` (020 §4.4.1)
// ---------------------------------------------------------------------------------------

/// **`va_list` is 24 bytes, aligned 8, on x86-64.**
///
/// `__builtin_va_list` is `struct __va_list_tag [1]`:
///
///     unsigned int gp_offset;      // 0
///     unsigned int fp_offset;      // 4
///     void *overflow_arg_area;     // 8
///     void *reg_save_area;         // 16
///
/// Sema modelled it as `Array { elem: <sentinel>, len: Fixed(0) }` — size **zero**, under a
/// comment saying "24 bytes aligned 8". Lowering's `.max(1)` then gave `va_list ap` a
/// one-byte object, so any read of it was out of bounds and no variadic function could run.
#[test]
fn a_va_list_has_the_abi_size_and_alignment() {
    let p = parse("__builtin_va_list ap;", TargetConfig::x86_64_linux());
    assert!(
        p.analysis.diagnostics.is_empty(),
        "{:?}",
        p.analysis.diagnostics
    );
    let id = p.decl_ty("ap").expect("ap");
    assert_eq!(
        p.analysis.size_of(id),
        Some(24),
        "`__va_list_tag` is 4 + 4 + 8 + 8"
    );
    assert_eq!(
        p.analysis.align_of(id),
        Some(8),
        "its pointer members force 8"
    );
}

/// **`__gnuc_va_list` is the same type**, which is the spelling glibc's headers use.
#[test]
fn the_gnuc_spelling_is_the_same_type() {
    let p = parse("__gnuc_va_list ap;", TargetConfig::x86_64_linux());
    let id = p.decl_ty("ap").expect("ap");
    assert_eq!(p.analysis.size_of(id), Some(24));
    assert_eq!(p.analysis.align_of(id), Some(8));
}

/// **A pointer to it is still a pointer**, which is what 020 §4.4.1 needs: the list lives
/// in memory so `va_list *` can cross a function boundary.
#[test]
fn a_pointer_to_a_va_list_is_a_pointer() {
    let p = parse("__builtin_va_list *ap;", TargetConfig::x86_64_linux());
    let id = p.decl_ty("ap").expect("ap");
    assert!(
        matches!(p.analysis.ty(id), Ty::Ptr(_)),
        "{:?}",
        p.analysis.ty(id)
    );
    assert_eq!(p.analysis.size_of(id), Some(8));
}
