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
    // Parameters get slots of their own — every local is addressable, because CIR is not
    // SSA (020 §1.3) and `&param` has to point somewhere. The contract is about the
    // *shape's* slot, so the count is of unnamed ones.
    let slots: Vec<_> = f.allocas.iter().filter(|a| a.name.is_none()).collect();
    assert_eq!(
        slots.len(),
        1,
        "one slot for the result, per 015 §2.1's `alloca`-not-phi shape: {:?}",
        f.allocas
            .iter()
            .map(|a| (&a.name, &a.ty))
            .collect::<Vec<_>>()
    );
    // **`Int(32)`, not `Int(1)`** — `a && b` has C type `int`, and a one-bit slot would
    // force lowering to invent a `ZExt` at every use, which §2 forbids.
    assert_eq!(
        slots[0].ty,
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
        .find(|f| &*f.name == "use")
        .expect("`use` was lowered");
    // The callee is a `FuncId`; the *name* is what the contract is about, so resolve it.
    let called: Vec<String> = uf
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Call {
                callee: chiero_cir::Callee::Direct(id),
                ..
            } => m
                .funcs
                .iter()
                .find(|f| f.id == *id)
                .map(|f| f.name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(called.len(), 3, "g, h, then f: {called:?}");
    let pos = |needle: &str| called.iter().position(|c| c == needle);
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
    // Measured as a **delta**, because a declaration with an initializer legitimately
    // computes an address too. One extra `x += f()` must cost exactly one more address
    // computation — "once per compound assignment" is the contract, and an absolute count
    // would be asserting the initializer's shape as well.
    let count = |src: &str| {
        let m = lower(src);
        let uf = m
            .funcs
            .iter()
            .find(|f| &*f.name == "use")
            .expect("`use` was lowered");
        uf.blocks
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
            .count()
    };
    let one = count("int f(void); void use(void) { int x = 0; x += f(); }");
    let two = count("int f(void); void use(void) { int x = 0; x += f(); x += f(); }");
    assert_eq!(
        two - one,
        1,
        "each `x += f()` computes the address once ({one} then {two}); twice is invisible \
         in the value and is a correctness bug the moment the lvalue has side effects"
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
    let f = m.funcs.iter().find(|f| &*f.name == "use").expect("`use`");
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
    // **The header is a block of its own**, not the body's first block. A mutation that
    // aliased the two still produced a back edge — the latch jumped to the body — so the
    // edge check above cannot see it. `for(;;) {}` is header, body, latch and exit.
    assert_eq!(
        f.blocks.len(),
        4,
        "header, body, latch, exit: {:#?}",
        f.blocks.iter().map(|b| (b.id, &b.term)).collect::<Vec<_>>()
    );
    let header = match f.block(f.entry).expect("entry").term {
        Terminator::Goto(h) => h,
        ref other => panic!("the entry falls into the header: {other:?}"),
    };
    let body = match f.block(header).expect("header").term {
        Terminator::Goto(b) => b,
        ref other => panic!("an absent condition is an unconditional goto: {other:?}"),
    };
    assert_ne!(
        header, body,
        "the header is distinct from the body it guards"
    );
}

/// A terminated block is never re-terminated.
///
/// `if (c) return 1; else return 2;` has no path that falls through, and a lowering that
/// overwrote each arm's `Return` with a `Goto` to the join would turn both returns into
/// fallthrough — a silent change of what the function computes, and one no shape count
/// notices because the block structure is identical either way.
#[test]
fn a_returning_branch_arm_keeps_its_return() {
    let m = lower("int use(int c) { if (c) { return 1; } else { return 2; } }");
    let f = m.funcs.iter().find(|f| &*f.name == "use").expect("`use`");
    let returns = f
        .blocks
        .iter()
        .filter(|b| matches!(b.term, Terminator::Return(Some(_))))
        .count();
    assert_eq!(
        returns,
        2,
        "both arms still return: {:#?}",
        f.blocks.iter().map(|b| (b.id, &b.term)).collect::<Vec<_>>()
    );
}

/// `&&` and `||` take **opposite** edges.
///
/// The four-block shape is identical for both, so every structural count above passes
/// with the two swapped — and the program then computes the negation of what was written.
/// The discriminator is which operand the short-circuit block corresponds to: for `&&`
/// the entry's *false* edge stores 0 without evaluating the rhs; for `||` it is the
/// *true* edge that stores 1.
#[test]
fn and_and_or_branch_to_opposite_edges() {
    let stored_on = |src: &str, take_true: bool| -> i128 {
        let m = lower(src);
        let f = m.funcs.iter().find(|f| &*f.name == "f").expect("`f`");
        let entry = f.block(f.entry).expect("entry");
        let Terminator::Br { t, f: fls, .. } = entry.term else {
            panic!("the entry tests the lhs")
        };
        let target = if take_true { t } else { fls };
        // The short-circuit block stores a constant; the rhs block computes one.
        f.block(target)
            .expect("target")
            .insts
            .iter()
            .find_map(|i| match &i.kind {
                InstKind::Store {
                    val: chiero_cir::Operand::Const(chiero_cir::Const::Int { val, .. }),
                    ..
                } => Some(*val),
                _ => None,
            })
            .unwrap_or(-1)
    };

    assert_eq!(
        stored_on("int f(int a, int b) { return a && b; }", false),
        0,
        "`a && b` short-circuits on the **false** edge, storing 0"
    );
    assert_eq!(
        stored_on("int f(int a, int b) { return a || b; }", true),
        1,
        "`a || b` short-circuits on the **true** edge, storing 1"
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

/// **An `f80` constant carries the 80-bit encoding, not the `f64` one.**
///
/// x87's extended format is a 15-bit exponent (bias 16383) and a 64-bit significand with an
/// **explicit** integer bit — where `f64` has an 11-bit exponent (bias 1023) and an implicit one.
/// They are not the same number of bits and not the same layout:
///
/// ```text
///   1.0L   0x3fff8000000000000000     (gcc, and what x87 loads)
///   1.0    0x3ff0000000000000         (what lowering emitted for `long double`)
/// ```
///
/// Lowering reached CIR with the right *type* — `alloca %0 : f80`, `store f80`, `load f80` — and
/// the wrong *payload*, because `float_bits` mapped `X87_80` to `f64::to_bits` and returned `u64`,
/// which cannot hold eighty bits.
///
/// Invisible today: the engine models no `f80` arithmetic, so the bits are never interpreted. It is
/// the first thing that bites when they are, and arithmetic on those bits computes on garbage that
/// *looks* like a number — which is worse than the declared gap it replaces.
///
/// The expected patterns are gcc's, read out of the object bytes rather than derived, because
/// deriving them is exactly the step that produced the bug.
#[test]
fn an_x87_literal_is_encoded_in_eighty_bits() {
    for (lit, want) in [
        ("1.0L", "0x3fff8000000000000000"),
        ("2.0L", "0x40008000000000000000"),
        ("0.5L", "0x3ffe8000000000000000"),
        // Three halves of a significand set, so a fix that only widened the exponent shows here.
        ("3.0L", "0x4000c000000000000000"),
        // Zero is all-zero in both formats, and is the control: a fix that shifted every payload
        // unconditionally would still get this one right, and that is the point of including it.
        ("0.0L", "0x0"),
        // **A fraction, whose bits truncation cannot see.** Wave 232 added an exact path for
        // *integral* `long double` literals, and a version of it that accepted `2.5L` and threw the
        // fraction away passed every value comparison — because `(int)` of 2.5 and of 2.0 are both
        // 2, and arithmetic that could tell them apart is still a gap. The pattern is where the
        // difference lives.
        ("2.5L", "0x4000a000000000000000"),
    ] {
        let t = print(&lower(&format!(
            "int probe(void){{ long double x = {lit}; return (int)x; }}"
        )));
        assert!(
            t.contains(&format!("fconst:f80:{want}")),
            "`{lit}` must be the 80-bit pattern gcc stores, not an f64 one: {}",
            t.lines()
                .find(|l| l.contains("fconst"))
                .unwrap_or("no fconst emitted")
        );
    }
}

/// **A hex float literal reaches the bits, past `f64`'s fifty-three.**
///
/// Wave 231 made hex literals parse, and wave 235 found the parse still goes through an `f64` — so a
/// literal needing more than fifty-three significant bits is rounded before the encoder sees it, and
/// the tie fixtures for `FpTrunc` were measuring that rounding rather than the conversion.
///
/// ```text
///   0x1.00000000000008p0L      chiero 0x3fff8000000000000000   gcc 0x3fff8000000000000400
/// ```
///
/// A hex literal's digits are already binary, so there is nothing to round: a mantissa and a binary
/// scale describe the value exactly, and x87 has room for sixty-four bits of it. This is the same
/// seam wave 233 found for the integral decimal path — the front end answering in `f64` when it holds
/// something wider — and the patterns are gcc's, read out of object bytes.
#[test]
fn a_hex_literal_reaches_all_sixty_four_bits() {
    for (lit, want) in [
        // 1 + 2^-53 and 1 + 3·2^-53: the two ties `FpTrunc`'s fixtures need intact.
        ("0x1.00000000000008p0L", "0x3fff8000000000000400"),
        ("0x1.00000000000018p0L", "0x3fff8000000000000c00"),
        // Every bit of the significand set, which is the widest a hex literal can be.
        ("0x1.fffffffffffffffep0L", "0x3fffffffffffffffffff"),
        // An exponent far outside `f64`'s range, where going through `f64` underflows to zero.
        ("0x1p-16000L", "0x017f8000000000000000"),
        // An ordinary one, so the common case is pinned beside the extremes.
        ("0x1.8p3L", "0x4002c000000000000000"),
    ] {
        let t = print(&lower(&format!(
            "int probe(void){{ long double x = {lit}; return (int)x; }}"
        )));
        assert!(
            t.contains(&format!("fconst:f80:{want}")),
            "`{lit}` is exact in binary and must arrive whole: {}",
            t.lines()
                .find(|l| l.contains("fconst"))
                .unwrap_or("no fconst emitted")
        );
    }
}

/// **A negative literal is a negation of a positive constant**, so no constant carries a sign.
///
/// Mutation found `x87_bits`'s sign term droppable with nothing noticing, and the reason is this:
/// `-1.0L` lowers to `fneg f80 fconst:f80:0x3fff8000000000000000`, because C has no negative
/// literals — the minus is a unary operator. So nothing in this project hands `x87_bits` a negative
/// `f64` today.
///
/// The term stays, and this test says why rather than leaving a reader to wonder. The function's
/// contract is "an `f64` re-encoded", not "a literal re-encoded", and a constant fold of
/// `-1.0L` would pass one through the moment `f80` arithmetic lands. Unlike wave 223's duplicated
/// bound check, this is not a second copy of a decision made elsewhere — it is the only place that
/// would get it right.
#[test]
fn a_negative_x87_literal_is_a_negation_of_a_positive_constant() {
    let t = print(&lower(
        "int probe(void){ long double x = -1.0L; return (int)x; }",
    ));
    assert!(
        t.contains("fneg f80 fconst:f80:0x3fff8000000000000000"),
        "the sign is an operation and the constant is unsigned: {t}"
    );
}

/// And `f32`/`f64` literals are unchanged. **The control.**
///
/// Widening the payload must not move the formats that already worked. Their printed form is the
/// same hex whether the field is `u64` or `u128`, and nineteen goldens depend on that.
#[test]
fn f32_and_f64_literals_keep_their_encodings() {
    let t = print(&lower("int probe(void){ float f = 1.0f; return (int)f; }"));
    assert!(
        t.contains("fconst:f32:0x3f800000"),
        "1.0f is 0x3f800000: {t}"
    );
    let t = print(&lower("int probe(void){ double d = 1.0; return (int)d; }"));
    assert!(
        t.contains("fconst:f64:0x3ff0000000000000"),
        "1.0 is 0x3ff0000000000000: {t}"
    );
}
