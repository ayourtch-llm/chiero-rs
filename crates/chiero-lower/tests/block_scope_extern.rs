//! **`extern` inside a body declares an object nothing else in the translation unit mentions.**
//!
//! ```text
//! vlib/init.h:233:5: `vnet_interface_init` lowered to CIR the verifier rejects
//!   (store value operand is Int(32), declared Ptr)
//!
//!   extern vlib_init_function_t *VLIB_INIT_FUNCTION_SYMBOL (x);
//!   vlib_init_function_t *_f = VLIB_INIT_FUNCTION_SYMBOL (x);
//! ```
//!
//! `local_decl` returns early for `extern`, on the reasoning that "the file-scope object of that
//! name is already in the table, and the declaration's only effect is to make it visible here".
//! That is true when there *is* a file-scope declaration. `vlib_call_init_function` is the case
//! where there is not: the symbol is defined in another translation unit and named nowhere else
//! in this one, so nothing ever registered it and every reference lowered to a 32-bit fallback.
//!
//! It is how VPP wires its init-function dependency graph, so it is in every `*_init` function
//! in the tree.
//!
//! # Why this is not a differential test
//!
//! The oracle compiles and runs one translation unit, so any object it can *read* has to be
//! defined in that unit — and a file-scope definition is exactly what makes the defect
//! disappear. The assertion is therefore structural: the module must carry a global for the
//! name, and lowering must not refuse the function.

mod harness;

/// A pointer-typed `extern`, which is the shape the verifier catches.
#[test]
fn a_block_scope_extern_pointer_is_registered_as_a_global() {
    let src = "typedef int (*fp)(void);\n\
               int f(void) { extern fp g_fp; fp p = g_fp; return p != 0; }\n";
    let lowered = harness::lower_raw(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "gcc compiles this and links it against the other translation unit: {:?}",
        lowered.diagnostics
    );
    assert!(
        lowered.module.globals.iter().any(|g| &*g.name == "g_fp"),
        "the object has to exist for a reference to mean anything: {:?}",
        lowered
            .module
            .globals
            .iter()
            .map(|g| g.name.to_string())
            .collect::<Vec<_>>()
    );
}

/// **An `int` one verifies either way**, which is why the defect survived: the fallback width
/// happens to match, so the CIR is well-formed and the object simply does not exist.
#[test]
fn a_block_scope_extern_scalar_is_registered_too() {
    let m = harness::lower_raw("int f(void) { extern int g_i; return g_i; }\n").module;
    assert!(
        m.globals.iter().any(|g| &*g.name == "g_i"),
        "{:?}",
        m.globals
            .iter()
            .map(|g| g.name.to_string())
            .collect::<Vec<_>>()
    );
}

/// And a file-scope declaration of the same name still yields **one** object, not two.
#[test]
fn a_block_scope_extern_does_not_duplicate_a_file_scope_object() {
    let m =
        harness::lower_raw("int g_i = 5;\nint f(void) { extern int g_i; return g_i; }\n").module;
    let n = m.globals.iter().filter(|g| &*g.name == "g_i").count();
    assert_eq!(n, 1, "one object, declared twice");
}
