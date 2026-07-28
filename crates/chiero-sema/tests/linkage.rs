//! Covers: 014 contracts 14, 15, 16, 17, 18.
//!
//! Contracts 15 and 16 are the first thing in this project that spans translation units,
//! and they are a *pair* on purpose: 16 says two TUs mentioning `extern int foo;` and one
//! defining `int foo;` are **one** entity, and 15 says two TUs each defining
//! `static void helper(void)` are **two**. Either alone is satisfied by a wrong rule —
//! "always merge by name" passes 16, "never merge" passes 15 — and both wrong rules are
//! silent. Merging two statics gives 031 a call graph with edges that do not exist;
//! splitting an extern gives it a graph missing the edges that do.

mod harness;

use chiero_sema::{ConstVal, GlobalTable, Linkage, TargetConfig, TuId, const_eval};
use harness::{Parsed, names_of, parse, parse_allowing_diagnostics};

/// Analyse several sources as separate TUs and fold them into one table.
fn link(sources: &[&str]) -> (Vec<Parsed>, GlobalTable, Vec<chiero_sema::SemaDiagnostic>) {
    let parsed: Vec<Parsed> = sources
        .iter()
        .map(|s| parse(s, TargetConfig::x86_64_linux()))
        .collect();
    let mut table = GlobalTable::new();
    let mut diags = Vec::new();
    for (i, p) in parsed.iter().enumerate() {
        let names = names_of(&p.parsed);
        diags.extend(table.add_tu(TuId(i as u32), &p.parsed.ast, &p.analysis, &names));
    }
    (parsed, table, diags)
}

/// **Contract 14.** `int x; int x;` at file scope is two *tentative* definitions and is
/// accepted. `int x = 1; int x = 2;` is one diagnostic.
///
/// The pair is the contract. Rejecting every redeclaration passes the second half and
/// breaks every C file in existence, since a tentative definition followed by the real one
/// is how C headers have always worked.
#[test]
fn tentative_definitions_repeat_but_two_initializers_do_not() {
    let ok = parse("int x;\nint x;\n", TargetConfig::x86_64_linux());
    assert!(
        ok.analysis.diagnostics.is_empty(),
        "two tentative definitions are legal C: {:?}",
        ok.analysis.diagnostics
    );

    let ok2 = parse("int x;\nint x = 1;\n", TargetConfig::x86_64_linux());
    assert!(
        ok2.analysis.diagnostics.is_empty(),
        "and a tentative one followed by the real definition: {:?}",
        ok2.analysis.diagnostics
    );

    let bad = parse_allowing_diagnostics("int x = 1;\nint x = 2;\n", TargetConfig::x86_64_linux());
    assert_eq!(
        bad.analysis.diagnostics.len(),
        1,
        "but two *initialized* definitions are exactly one diagnostic: {:?}",
        bad.analysis
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// **Contract 15.** Two TUs each defining `static void helper(void)` are two entities.
///
/// 014 §4 names this a real hazard in VPP, where short static helper names repeat across
/// nodes. Merging them would give 031 a call graph in which one node's helper appears to
/// be called from another node.
#[test]
fn two_static_definitions_of_one_name_are_two_entities() {
    let (_, table, diags) = link(&[
        "static void helper(void) { }\nvoid a(void) { helper(); }\n",
        "static void helper(void) { }\nvoid b(void) { helper(); }\n",
    ]);
    assert!(
        diags.is_empty(),
        "no linkage complaint is warranted: {diags:?}"
    );

    let h0 = table.resolve(TuId(0), "helper").expect("TU 0's helper");
    let h1 = table.resolve(TuId(1), "helper").expect("TU 1's helper");
    assert_ne!(
        h0, h1,
        "two `static` definitions of one name are two distinct entities"
    );
    assert_eq!(table.info(h0).linkage, Linkage::Internal);
    assert_eq!(table.info(h0).tu, Some(TuId(0)));
    assert_eq!(table.info(h1).tu, Some(TuId(1)));

    // The discriminator against "never merge anything": the two TUs' *external* functions
    // are still distinct from each other by name, and a third TU declaring `a` reaches
    // TU 0's.
    let (_, table, _) = link(&[
        "static void helper(void) { }\nvoid shared(void) { }\n",
        "static void helper(void) { }\nvoid useit(void) { shared(); }\n",
    ]);
    let s0 = table.resolve(TuId(0), "shared").expect("shared in TU 0");
    let s1 = table.resolve(TuId(1), "shared").expect("shared in TU 1");
    assert_eq!(
        s0, s1,
        "an external name is one entity across TUs, or the rule is just `never merge`"
    );
    assert_eq!(
        table
            .globals()
            .iter()
            .filter(|g| g.name == "helper")
            .count(),
        2,
        "and the two statics really are two rows, not one row reached twice"
    );
}

/// **Contract 16.** Two TUs referencing `extern int foo;` and one defining `int foo;`
/// resolve to a single `GlobalId`, and that entity knows it is defined.
#[test]
fn an_extern_declared_twice_and_defined_once_is_one_entity() {
    let (_, table, diags) = link(&[
        "extern int foo;\nint reader(void) { return foo; }\n",
        "extern int foo;\nint other(void) { return foo; }\n",
        "int foo = 7;\n",
    ]);
    assert!(diags.is_empty(), "{diags:?}");

    let a = table.resolve(TuId(0), "foo").expect("foo from TU 0");
    let b = table.resolve(TuId(1), "foo").expect("foo from TU 1");
    let c = table.resolve(TuId(2), "foo").expect("foo from TU 2");
    assert_eq!(a, b);
    assert_eq!(b, c, "one entity, however many TUs mention it");
    // **And exactly one exists.** `resolve` agreeing is not enough: a mutation that
    // created a fresh entity per TU and overwrote the index each time still returned the
    // same id from every lookup, because the *last* one won. The table would have held
    // three `foo`s, which is what 031 would walk.
    assert_eq!(
        table.globals().iter().filter(|g| g.name == "foo").count(),
        1,
        "one entity in the table, not one per mention: {:?}",
        table.globals()
    );
    assert_eq!(table.info(a).linkage, Linkage::External);
    assert_eq!(
        table.info(a).tu,
        None,
        "an external entity belongs to no single TU"
    );
    assert!(
        table.info(a).defined,
        "and it knows a definition exists — an extern never defined is a link error \
         031 would want to see"
    );

    // The discriminator: an extern that nobody defines is one entity that is *not*
    // defined, so `defined` is a fact rather than a constant.
    let (_, table, _) = link(&["extern int never_defined;\n"]);
    let n = table
        .resolve(TuId(0), "never_defined")
        .expect("the declaration still creates the entity");
    assert!(!table.info(n).defined);
}

/// **Contract 17.** `&arr[3]` as a static initializer is an *address* constant with the
/// element offset folded in — `3 * sizeof(int)`, not 3.
#[test]
fn an_address_constant_carries_the_scaled_offset() {
    let p = parse("int arr[8];", TargetConfig::x86_64_linux());
    for (src, want_off) in [("&arr[3]", 12i64), ("&arr[0]", 0), ("arr + 2", 8)] {
        let (ast, expr) = harness::expression_with_prelude("int arr[8];", src);
        let names = names_of(&ast);
        let mut diags = Vec::new();
        let v = const_eval(
            &ast.ast,
            expr,
            &names,
            &TargetConfig::x86_64_linux(),
            &mut diags,
        );
        assert_eq!(
            v,
            Some(ConstVal::Addr {
                global: "arr".into(),
                off: want_off
            }),
            "`{src}` is the address of `arr` at byte offset {want_off}, not element \
             offset — an unscaled offset reads three bytes into the first element"
        );
    }
    let _ = p;
}

/// **Contract 18.** `__builtin_constant_p` is 1 exactly when its argument folds.
///
/// 014 §6 says matching gcc closely enough is *required*, because VPP uses it in macros
/// that select between implementations — so answering a constant 0 (or a constant 1) makes
/// every such macro pick the same branch always.
#[test]
fn builtin_constant_p_distinguishes_constants_from_variables() {
    let (ast, expr) = harness::expression_with_prelude("int v;", "__builtin_constant_p(1 + 1)");
    let names = names_of(&ast);
    let mut diags = Vec::new();
    assert_eq!(
        const_eval(
            &ast.ast,
            expr,
            &names,
            &TargetConfig::x86_64_linux(),
            &mut diags
        ),
        Some(ConstVal::Int(1)),
        "`1 + 1` folds, so the answer is 1"
    );

    let (ast, expr) = harness::expression_with_prelude("int v;", "__builtin_constant_p(v)");
    let names = names_of(&ast);
    let mut diags = Vec::new();
    assert_eq!(
        const_eval(
            &ast.ast,
            expr,
            &names,
            &TargetConfig::x86_64_linux(),
            &mut diags
        ),
        Some(ConstVal::Int(0)),
        "a variable does not fold, so the answer is 0 — and it is still a *constant* 0, \
         which is what lets the enclosing `?:` be folded away"
    );
    assert!(
        diags.is_empty(),
        "and asking the question is not itself an error: {diags:?}"
    );
}
