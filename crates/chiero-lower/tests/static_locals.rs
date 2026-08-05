//! **Two `static` locals of the same name in sibling scopes collide, and the function is dropped.**
//!
//! ```c
//! double f(int n) {
//!   if (n >= 0) { static double t[2] = {1, 2}; return t[n & 1]; }
//!   else        { static double t[2] = {3, 4}; return t[-n & 1]; }
//! }
//! ```
//!
//! `local_decl` names a `static` local `<owner>.<name>` so a dump can tell two of them apart.
//! Two in *one* function get the same string, the verifier's duplicate-name rule fires, and 015
//! §7 discards the whole function.
//!
//! # This is what the first honest corpus sweep found
//!
//! `global \`X\` is declared more than once` is **175 of 240** translation units in the first
//! measurement that ran lowering — the single largest cause of `not-run` in VPP. Two names
//! account for all of it: `vlib_worker_thread_barrier_check.e`, which writes
//! `ELOG_TYPE_DECLARE (e)` in three sibling `if` blocks (vlib/threads.h:313 and after), and
//! `times_power_of_ten.t`, which is `static f64 t[8]` in each arm of an `if`/`else`
//! (vppinfra/format.c:638 and :648). Both are ordinary C that gcc compiles without comment.
//!
//! # The second defect is on the same path and is silent
//!
//! `restore_static_local_names` replays `static_locals` in **push** order. Two entries for one
//! name replay as "remove, then put back the first static" — so the name stays bound to an
//! object of a function that has ended, and the *next* function to mention it resolves to that
//! object instead of the file-scope one. No diagnostic, a wrong value.
//!
//! # What the fix owes
//!
//! Two objects, because C 6.2.4p3 gives each declaration its own object with its own
//! initializer — merging them into one global would satisfy the verifier and change what the
//! program computes. So: distinct globals, distinct names, and the file-scope binding intact
//! afterwards.

mod harness;

use chiero_cir::Module;

/// Both arms declare `static double t[2]`, with **different** initializers so a fix that merges
/// them into one object is visible rather than merely suspicious.
const SIBLINGS: &str = "double f(int n) {\n\
     if (n >= 0) { static double t[2] = {1, 2}; return t[n & 1]; }\n\
     else        { static double t[2] = {3, 4}; return t[-n & 1]; }\n\
   }\n";

fn global_names(m: &Module) -> Vec<String> {
    m.globals.iter().map(|g| g.name.to_string()).collect()
}

/// **The function must survive.** 015 §7 discards a function whole when its CIR does not
/// verify, so this is the difference between VPP being analysed and not.
#[test]
fn sibling_scopes_may_each_declare_a_static_of_the_same_name() {
    let lowered = harness::lower_raw(SIBLINGS);
    assert!(
        lowered.diagnostics.is_empty(),
        "gcc compiles this without comment; lowering must too: {:?}",
        lowered.diagnostics
    );
    let errors: Vec<String> = chiero_cir::verify::verify(&lowered.module)
        .iter()
        .filter(|e| e.is_error())
        .map(|e| format!("{:?}: {}", e.kind, e.detail))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(
        lowered.module.funcs.len(),
        1,
        "`f` was dropped, so nothing downstream sees this translation unit at all"
    );
}

/// **Two declarations, two objects, two names.**
///
/// The C rule is 6.2.4p3: each declaration denotes an object with static storage duration, and
/// these two have different initializers. One shared global would return 1 where the source says
/// 3. The names must differ too, or the module cannot round-trip through the textual format —
/// name resolution there takes the first, so the second object becomes unreachable.
#[test]
fn each_sibling_static_gets_its_own_global() {
    let m = harness::lower_raw(SIBLINGS).module;
    let names = global_names(&m);
    let mine: Vec<&String> = names.iter().filter(|n| n.contains("f.t")).collect();
    assert_eq!(
        mine.len(),
        2,
        "two `static double t[2]` declarations are two objects: {names:?}"
    );
    assert_ne!(
        mine[0], mine[1],
        "and they need distinguishable names: {names:?}"
    );
}

/// **A file-scope name must still mean the file-scope object after the function ends.**
///
/// The quiet half. `restore_static_local_names` replays in push order, so the *first* static's
/// binding is put back last and outlives the function — and `g` then reads `f`'s local instead
/// of the file-scope `t`, with no diagnostic anywhere.
#[test]
fn a_static_local_does_not_outlive_its_function_in_the_name_table() {
    let src = format!("double t[2] = {{9, 9}};\n{SIBLINGS}double g(void) {{ return t[0]; }}\n");
    let m = harness::lower_raw(&src).module;
    assert!(
        m.globals.iter().any(|gl| &*gl.name == "t"),
        "the file-scope `t` exists: {:?}",
        global_names(&m)
    );
    // Read out of the textual form, which names the global an `addrglobal` refers to — the
    // structural walk would need `verify`'s private operand enumeration.
    let text = chiero_cir::text::print(&m);
    let body = text
        .split("func @g(")
        .nth(1)
        .expect("`g` was lowered")
        .split("\nfunc @")
        .next()
        .unwrap()
        .to_string();
    assert!(
        body.contains("addrglobal @t "),
        "`g` must read the file-scope `t`:\n{body}"
    );
    assert!(
        !body.contains("addrglobal @f."),
        "`f`'s statics are out of scope by the time `g` is lowered:\n{body}"
    );
}
