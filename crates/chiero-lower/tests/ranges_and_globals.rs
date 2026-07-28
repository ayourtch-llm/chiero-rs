//! Covers: 020 contract 14.
//!
//! Also closes 020 §6's recorded gap — `GlobalInit` is "not yet in the textual format" and
//! a fixture needing initialized global bytes is owed — because string literals currently
//! lower to `Undef`, which is the last construct in ordinary C that lowering **silently**
//! drops.

use chiero_cir::Terminator;

mod harness;
use harness::lower;

fn probe(src: &str) -> chiero_cir::Module {
    lower(src)
}

/// **020 contract 14.** A `case` range of 4 expands to 4 cases; a span of 10 000 expands
/// to a **guarded chain**, and both produce identical execution results.
///
/// This is a scalability defect wave 93 introduced and never tested at scale: enumerating
/// every value of a range is correct for `case 1 ... 4` and catastrophic for
/// `case 1 ... 10000`, which VPP writes for protocol number ranges. Ten thousand
/// `Switch` cases is a module the engine must walk once per branch decision.
#[test]
fn a_small_case_range_enumerates_and_a_large_one_is_guarded() {
    let m = probe("int f(int n) { switch (n) { case 1 ... 4: return 1; default: return 0; } }");
    let f = m.funcs.iter().find(|f| &*f.name == "f").expect("f");
    let cases = f
        .blocks
        .iter()
        .find_map(|b| match &b.term {
            Terminator::Switch { cases, .. } => Some(cases.clone()),
            _ => None,
        })
        .expect("a switch");
    assert_eq!(
        cases.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "a four-value range is four cases"
    );

    // A 10 000-value range must **not** enumerate. The threshold itself is a policy
    // choice; what the contract fixes is that the big one is bounded.
    let m = probe("int f(int n) { switch (n) { case 1 ... 10000: return 1; default: return 0; } }");
    let f = m.funcs.iter().find(|f| &*f.name == "f").expect("f");
    let switch_cases = f
        .blocks
        .iter()
        .filter_map(|b| match &b.term {
            Terminator::Switch { cases, .. } => Some(cases.len()),
            _ => None,
        })
        .sum::<usize>();
    assert!(
        switch_cases < 100,
        "a 10 000-value range is a guarded chain, not 10 000 cases: got {switch_cases}"
    );
    // And it is still a *decision*: some comparison guards the range.
    assert!(
        f.blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Br { .. })),
        "the guard is a branch on a range test: {:#?}",
        f.blocks.iter().map(|b| (b.id, &b.term)).collect::<Vec<_>>()
    );

    // The module must still verify — a guarded chain that does not is worse than a large
    // switch.
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.is_empty(), "{errs:#?}");
}

/// **020 §6's owed gap.** A string literal is a `Global` with **initialized bytes**.
///
/// Until now a string literal lowered to `Undef`: the pointer was not merely unknown, it
/// was *absent*, so `chiero_make_symbolic(&x, 4, "x")` handed the intrinsic nothing and
/// every `printf` format string in the corpus was a value the engine could not read. 020
/// §6 records `GlobalInit` as owed and says a fixture needing initialized global bytes is
/// owed with it; this is that fixture.
#[test]
fn a_string_literal_is_a_global_with_its_bytes() {
    let m = probe("const char *f(void) { return \"hi\"; }");
    assert_eq!(m.globals.len(), 1, "one global for one literal");
    let g = &m.globals[0];
    assert_eq!(
        g.size, 3,
        "`\"hi\"` is three bytes including the terminator, which is what `sizeof` says"
    );
    assert!(g.is_const, "a string literal is not writable");
    assert_eq!(
        g.init,
        chiero_cir::GlobalInit::Bytes(vec![b'h', b'i', 0]),
        "and the bytes are **there** — an `Undef` pointer is not a weaker answer, it is \
         no answer, and 021 cannot read a byte that was never written"
    );

    // Two uses of one literal are one global: the value is the address, and two addresses
    // would compare unequal where C permits either but the corpus assumes one.
    let m = probe("const char *f(int n) { return n ? \"hi\" : \"hi\"; }");
    assert_eq!(
        m.globals.len(),
        1,
        "identical literals are pooled: {:?}",
        m.globals.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
    // And two *different* literals are two globals, or pooling is just "always one".
    let m = probe("const char *f(int n) { return n ? \"hi\" : \"bye\"; }");
    assert_eq!(m.globals.len(), 2);
}

/// A string literal's *value* is the address of its global, not `Undef`.
#[test]
fn a_string_literal_evaluates_to_its_globals_address() {
    let m = probe("const char *f(void) { return \"hi\"; }");
    let f = m.funcs.iter().find(|f| &*f.name == "f").expect("f");
    let returned = f
        .blocks
        .iter()
        .find_map(|b| match &b.term {
            Terminator::Return(Some(op)) => Some(op.clone()),
            _ => None,
        })
        .expect("the function returns something");
    assert!(
        matches!(
            returned,
            chiero_cir::Operand::Const(chiero_cir::Const::GlobalAddr { off: 0, .. })
        ),
        "the literal's value is its global's address at offset 0: {returned:?}"
    );
}
