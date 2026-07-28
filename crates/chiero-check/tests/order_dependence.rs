//! Covers: 020 contract 18(b) — the **interprocedural** half of order sensitivity.
//!
//! 020 §7: `f(g(), h())` where `g` and `h` both mutate a shared global "requires
//! side-effect summaries and 'is this the same object?', which is precisely what
//! `chiero-lower` may not contain and what the memory model can answer. It is an
//! order-dependence `Checker` in 040, running over CIR with `SeqPoint` markers as its
//! input."
//!
//! Contract 18(b): that call yields **exactly one** order-dependence finding, and the same
//! call where `g` and `h` mutate *different* globals yields **none**. The negative half is
//! the contract — a checker that fires whenever a call takes two call arguments satisfies
//! the positive half and is worthless.
//!
//! **Fixtures are `.cir`.** `chiero-check` is a vertical and 001 §4 rule 7 forbids it a
//! frontend dependency, dev-dependencies included. That is the right layer anyway: the
//! checker's input is CIR with `SeqPoint` markers, so a fixture written in CIR is the
//! thing under test rather than a C program that happens to lower to it.

use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn inst(kind: InstKind, lo: u32) -> Inst {
    Inst {
        kind,
        span: at(lo),
        generated: false,
    }
}

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: [10 + id].into_iter().collect(),
        span: at(1),
    }
}

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn global(id: u32, name: &str) -> Global {
    Global {
        id: GlobalId(id),
        name: name.into(),
        size: 4,
        align: 4,
        is_const: false,
        init: GlobalInit::Zero,
        linkage: Linkage::External,
        span: at(1),
    }
}

/// A function that stores `val` into global `g` and returns it.
fn mutator(id: u32, name: &str, g: u32, val: i128) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfGlobal { g: GlobalId(g) },
                    },
                    100 + id,
                ),
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        val: i32c(val),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    110 + id,
                ),
            ],
            Terminator::Return(Some(i32c(val))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: at(1),
    }
}

/// `int main(void) { return f(g(), h()); }` — two calls in one full expression, with the
/// `SeqPoint` that ends it after the outer call, exactly as lowering emits it.
///
/// `g_target`/`h_target` choose which global each callee mutates, which is the only
/// difference between contract 18(b)'s two halves.
fn caller(g_target: u32, h_target: u32, sequenced: bool) -> Module {
    let mut insts = vec![inst(
        InstKind::Call {
            dst: Some(ValueId(1)),
            callee: Callee::Direct(FuncId(1)),
            args: vec![],
        },
        20,
    )];
    // **The sequence point goes *between* the two calls** when the fixture is the
    // sequenced control: `g(); h();` is two full expressions, and the checker must treat
    // the region as closed by it.
    if sequenced {
        insts.push(inst(InstKind::Marker(MarkerKind::SeqPoint), 21));
    }
    insts.push(inst(
        InstKind::Call {
            dst: Some(ValueId(2)),
            callee: Callee::Direct(FuncId(2)),
            args: vec![],
        },
        22,
    ));
    insts.push(inst(InstKind::Marker(MarkerKind::SeqPoint), 23));

    Module {
        funcs: vec![
            Function {
                id: FuncId(0),
                name: "main".into(),
                params: vec![],
                ret: CTy::Int(32),
                variadic: false,
                allocas: vec![],
                blocks: vec![block(
                    0,
                    insts,
                    Terminator::Return(Some(Operand::Value(ValueId(1)))),
                )],
                entry: BlockId(0),
                attrs: Default::default(),
                body: Body::Defined,
                span: at(1),
            },
            mutator(1, "g", g_target, 1),
            mutator(2, "h", h_target, 2),
        ],
        globals: vec![global(0, "shared"), global(1, "other")],
        ..Default::default()
    }
}

/// Run the engine with the order-dependence checker registered, and return its findings.
fn findings(m: &Module) -> Vec<String> {
    let errs = chiero_cir::verify::verify(m);
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "the fixture must be valid CIR or the run proves nothing: {errs:#?}"
    );
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(m)
        .with_checker(Box::new(chiero_check::OrderDependence::new()))
        .run(&mut a);
    r.findings()
        .into_iter()
        .filter(|f| f.contains("order"))
        .collect()
}

/// **Contract 18(b), both halves.** Two callees mutating one global in an unsequenced
/// region is exactly one finding; two callees mutating different globals is none.
#[test]
fn two_calls_mutating_one_global_are_one_order_dependence_finding() {
    let got = findings(&caller(0, 0, false));
    assert_eq!(
        got.len(),
        1,
        "one finding for one conflict — not one per call, and not one per store: {got:#?}"
    );
    assert!(
        got[0].contains("shared"),
        "and it names the object the two calls disagree about: {got:#?}"
    );

    let got = findings(&caller(0, 1, false));
    assert!(
        got.is_empty(),
        "`g` writes `shared` and `h` writes `other`; there is no order to depend on. A \
         checker firing on any two calls in one expression reports this: {got:#?}"
    );
}

/// **A sequence point between the calls ends the region.**
///
/// This is the same module with one marker moved. `g(); h();` mutating one global is
/// ordinary C — the semicolon sequences them — and a checker that scanned a block for two
/// calls touching one object reports it anyway.
#[test]
fn a_sequence_point_between_the_calls_is_not_a_conflict() {
    let got = findings(&caller(0, 0, true));
    assert!(
        got.is_empty(),
        "`;` sequences the two calls, so neither races the other: {got:#?}"
    );
}

/// **One call is never a conflict with itself**, however many times it writes.
///
/// `g()` alone stores to `shared`; a checker keyed on "was this object written inside a
/// call" without also asking *which* call reports it.
#[test]
fn a_single_call_is_not_a_conflict() {
    let mut m = caller(0, 0, false);
    // Drop the second call, leaving one call and its sequence point.
    m.funcs[0].blocks[0]
        .insts
        .retain(|i| !matches!(&i.kind, InstKind::Call { dst: Some(d), .. } if *d == ValueId(2)));
    m.funcs[0].blocks[0].term = Terminator::Return(Some(Operand::Value(ValueId(1))));
    let got = findings(&m);
    assert!(got.is_empty(), "one call writes in one order: {got:#?}");
}

/// The checker is **off unless registered**, like every checker 040 defines.
#[test]
fn no_finding_without_the_checker() {
    let m = caller(0, 0, false);
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m).run(&mut a);
    assert!(
        !r.findings().iter().any(|f| f.contains("order")),
        "the engine reports order dependence only when asked to"
    );
}

/// The checker has the name 040 refers to it by, so a registry driven by configuration
/// can find it.
#[test]
fn the_checker_is_named() {
    use chiero_exec::Checker;
    assert_eq!(
        chiero_check::OrderDependence::new().name(),
        "order-dependence"
    );
}

/// **One call writing one object twice is still not a conflict.**
///
/// `a_single_call_is_not_a_conflict` uses a callee that writes once, so it never asks
/// whether the checker distinguishes *which* call wrote — a checker that reported the
/// second write to an object it had already seen, without comparing call sites, passes
/// that test and fires here. A function's own statements are sequenced; writing a counter
/// twice inside `g()` is ordinary code.
#[test]
fn one_call_writing_twice_is_not_a_conflict() {
    let mut m = caller(0, 1, false);
    // Give `g` a second store to the same global. Both are its own, and its own
    // statements are sequenced.
    let extra = inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(0)),
            val: i32c(99),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
        115,
    );
    m.funcs[1].blocks[0].insts.push(extra);
    let got = findings(&m);
    assert!(
        got.is_empty(),
        "two writes by one call are sequenced by that call's own statements: {got:#?}"
    );

    // And the conflict still fires when the *other* call joins in — so the fixture above
    // is not passing merely because the checker went quiet.
    let mut m = caller(0, 0, false);
    m.funcs[1].blocks[0].insts.push(inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(0)),
            val: i32c(99),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
        115,
    ));
    assert_eq!(
        findings(&m).len(),
        1,
        "still exactly one finding, however many times each call writes"
    );
}
