//! Covers: 015 contracts 1, 3, 4, 13, 21.
//!
//! 015 §1 says every construct lowers to a **fixed shape**, and that is a stronger claim
//! than "lowers correctly": two conforming implementations that disagree about block
//! order or where a `SeqPoint` goes would both be right and would both break the golden
//! `.cir` files 020 §6 makes contracts. So these tests assert *shape* — how many blocks,
//! which edges, what order — and not merely that the result computes the right thing.

use chiero_cir::{InstKind, RValue, Terminator};
use chiero_lower::lower_tu;

mod harness;
use harness::{lower, print};

/// **Contract 1.** `a && b` lowers to 015 §2.1's shape: four blocks, one `alloca`, and
/// `b`'s block reachable **only** from the true edge of `a`'s test.
///
/// The reachability half is the part that matters and the part a weaker test would miss:
/// a lowering that evaluated `b` unconditionally and then selected would produce the right
/// value and the wrong program. 015 §2.1 spells out why it matters beyond semantics —
/// `bb_rhs` exists precisely because `b` is conditionally evaluated, and **gcov counts it
/// separately**, so the coverage story depends on the block existing.
#[test]
fn short_circuit_and_has_four_blocks_and_a_conditional_rhs() {
    let m = lower("int f(int a, int b) { return a && b; }");
    let f = &m.funcs[0];
    assert_eq!(
        f.blocks.len(),
        4,
        "entry, rhs, false and join: {:#?}",
        f.blocks.iter().map(|b| (b.id, &b.term)).collect::<Vec<_>>()
    );
    assert_eq!(
        f.allocas.len(),
        1,
        "one slot for the result, per 015 §2.1's `alloca`-not-phi shape"
    );
    // **`Int(32)`, not `Int(1)`** — `a && b` has C type `int`, and a one-bit slot would
    // force lowering to invent a `ZExt` at every use, which §2 forbids.
    assert_eq!(
        f.allocas[0].ty,
        chiero_cir::CTy::Int(32),
        "the slot is the expression's C type"
    );

    let entry = f.block(f.entry).expect("entry");
    let Terminator::Br { t, f: fls, .. } = entry.term else {
        panic!("the entry block tests `a`: {:?}", entry.term)
    };
    // Exactly one block targets the rhs, and it is the entry's **true** edge.
    let preds: Vec<_> = f
        .blocks
        .iter()
        .filter(|b| match &b.term {
            Terminator::Goto(g) => *g == t,
            Terminator::Br { t: a, f: b, .. } => *a == t || *b == t,
            _ => false,
        })
        .map(|b| b.id)
        .collect();
    assert_eq!(
        preds,
        vec![f.entry],
        "`b`'s block is reachable only from the test of `a` — evaluating `b` \
         unconditionally computes the right value and runs the wrong program"
    );
    assert_ne!(t, fls, "and the two edges are distinct blocks");

    // The `SeqPoint` after `a` is at the **end of the entry block**, before the branch.
    // 015 §2.1 fixes its position precisely so two conforming lowerings cannot produce
    // different goldens.
    let last = entry.insts.last().expect("the entry block is not empty");
    assert!(
        matches!(
            last.kind,
            InstKind::Marker(chiero_cir::MarkerKind::SeqPoint)
        ),
        "the sequence point is the last instruction before the branch: {:?}",
        entry.insts.iter().map(|i| &i.kind).collect::<Vec<_>>()
    );
}

/// **Contract 3.** `f(g(), h())` emits the call to `g` before the call to `h`.
///
/// 015 §2 makes left-to-right **normative**, and 020 §7 flags order-sensitivity, so this
/// is not a detail: two lowerings that disagree produce different observable behaviour for
/// any pair of arguments with side effects.
#[test]
fn call_arguments_are_emitted_left_to_right() {
    let m = lower(
        "int g(void); int h(void); int f(int, int);\n\
         int use(void) { return f(g(), h()); }\n",
    );
    let uf = m
        .funcs
        .iter()
        .find(|f| print(&m).contains("@use") && !f.blocks.is_empty())
        .expect("a lowered function");
    let called: Vec<String> = uf
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Call { callee, .. } => Some(format!("{callee:?}")),
            _ => None,
        })
        .collect();
    assert_eq!(called.len(), 3, "g, h, then f: {called:?}");
    let pos = |needle: &str| called.iter().position(|c| c.contains(needle));
    assert!(
        pos("g") < pos("h"),
        "`g` is called before `h`, because 015 §2 makes left-to-right normative: {called:?}"
    );
    assert!(
        pos("h") < pos("f"),
        "and both before the call that consumes them: {called:?}"
    );
}

/// **Contract 4.** `x += f()` evaluates the lvalue's address **once**.
///
/// Twice is not a performance problem, it is a correctness one as soon as the lvalue has
/// side effects of its own (`*p++ += f()`), and it is invisible in the value computed for
/// the simple case — which is why the assertion counts address computations rather than
/// checking the result.
#[test]
fn compound_assignment_evaluates_the_address_once() {
    let m = lower("int f(void); void use(void) { int x = 0; x += f(); }");
    let uf = &m.funcs[0];
    let addr_ops = uf
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| {
            matches!(
                &i.kind,
                InstKind::Assign {
                    rv: RValue::AddrOfLocal { .. } | RValue::PtrAdd { .. },
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        addr_ops,
        1,
        "one address computation for `x`, not one per use: {:#?}",
        uf.blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .map(|i| &i.kind)
            .collect::<Vec<_>>()
    );
}

/// **Contract 13.** `for(;;)` still produces a distinct header block, so a **back edge**
/// exists and 023 §8's dominator analysis can find the loop.
///
/// An implementation that folded the empty condition into the body would produce a
/// correct-looking function with no identifiable loop header, and every loop-aware
/// analysis downstream would silently see straight-line code.
#[test]
fn an_empty_for_condition_still_has_a_header_block() {
    let m = lower("void use(void) { for(;;) { } }");
    let f = &m.funcs[0];
    let back_edges: Vec<_> = f
        .blocks
        .iter()
        .filter(|b| match &b.term {
            // A back edge targets a block at or before this one in layout order.
            Terminator::Goto(g) => {
                f.blocks
                    .iter()
                    .position(|x| x.id == *g)
                    .unwrap_or(usize::MAX)
                    <= f.blocks.iter().position(|x| x.id == b.id).unwrap()
            }
            _ => false,
        })
        .map(|b| b.id)
        .collect();
    assert!(
        !back_edges.is_empty(),
        "a back edge exists, so the loop is findable: {:#?}",
        f.blocks.iter().map(|b| (b.id, &b.term)).collect::<Vec<_>>()
    );
    assert!(
        f.blocks.len() >= 2,
        "the header is its own block, not folded into the body"
    );
}

/// **Contract 21.** Lowering the same TU twice produces **byte-identical** CIR.
///
/// 001 §5 makes determinism a hard requirement and the golden `.cir` files depend on it
/// entirely. The usual source of a violation is iteration order over a hash map, which
/// this project bans workspace-wide for exactly this reason — so the test lowers a
/// fixture with enough names and blocks to have an order to get wrong.
#[test]
fn lowering_is_byte_identical_across_runs() {
    let src = "int g(int); int h(int);\n\
               struct S { int a; int b; };\n\
               int f(int p, int q) {\n\
                 struct S s; s.a = p; s.b = q;\n\
                 int t = 0;\n\
                 for (int i = 0; i < p; i++) { t += g(i) && h(i); }\n\
                 if (t > q) { return t; } else { return q; }\n\
               }\n";
    let a = print(&lower(src));
    let b = print(&lower(src));
    assert_eq!(a, b, "two lowerings of one TU differ");
    assert!(
        a.lines().count() > 20,
        "and the fixture is big enough to have an order to get wrong: {} lines",
        a.lines().count()
    );
}

/// Everything lowered here must **verify** (020 §8). A shape test that produced invalid
/// CIR would be asserting the shape of something the rest of the system rejects.
#[test]
fn every_fixture_produces_verifiable_cir() {
    for src in [
        "int f(int a, int b) { return a && b; }",
        "int f(int a, int b) { return a || b; }",
        "int f(int a, int b, int c) { return a ? b : c; }",
        "int g(void); int h(void); int f(int, int); int use(void) { return f(g(), h()); }",
        "int f(void); void use(void) { int x = 0; x += f(); }",
        "void use(void) { for(;;) { } }",
        "int use(int n) { int t = 0; while (n > 0) { t += n; n--; } return t; }",
    ] {
        let m = lower(src);
        let errs = chiero_cir::verify::verify(&m);
        assert!(errs.is_empty(), "`{src}` produced invalid CIR: {errs:#?}");
    }
}

/// A guard that the pipeline is actually running: a TU with a function body must produce
/// a function with instructions. Without it every assertion above could pass over an
/// empty module.
#[test]
fn lowering_produces_a_non_empty_module() {
    let m = lower("int f(int a) { return a + 1; }");
    assert_eq!(m.funcs.len(), 1);
    assert!(
        m.funcs[0].blocks.iter().any(|b| !b.insts.is_empty()),
        "the function has instructions"
    );
    let _ = lower_tu;
}
