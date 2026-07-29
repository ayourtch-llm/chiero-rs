//! **Volatile is not ordinary memory** — 020 §4.2 and contract 41.
//!
//! Covers: 020 contract 41.
//!
//! §4.2: "A `Volatile` load never reads the memory model's stored bytes: it produces a
//! `Fresh` value each time and is never cached, CSE'd or reordered by any pass. A
//! `Volatile` store is an observable event recorded on the path. VPP's device-register
//! and counter code depends on this."
//!
//! Treating a device register as ordinary memory is confidently wrong in the direction
//! that matters: reading back what was written is exactly what a hardware register does
//! not do, and a run that models it that way explores branches the device can never take.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

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
        gcov_lines: Default::default(),
        span: Span::DUMMY,
    }
}

fn one_local(insts: Vec<Inst>, ret: Operand) -> Module {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 1,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: at(1),
        }],
        blocks: vec![block(0, insts, Terminator::Return(Some(ret)))],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    Module {
        funcs: vec![f],
        ..Default::default()
    }
}

fn addr(dst: u32, lo: u32) -> Inst {
    inst(
        InstKind::Assign {
            dst: ValueId(dst),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        },
        lo,
    )
}

fn store(val: i128, vol: Volatility, lo: u32) -> Inst {
    inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(0)),
            val: i32c(val),
            ty: CTy::Int(32),
            align: 4,
            vol,
        },
        lo,
    )
}

fn load(dst: u32, vol: Volatility, lo: u32) -> Inst {
    inst(
        InstKind::Assign {
            dst: ValueId(dst),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(0)),
                ty: CTy::Int(32),
                align: 4,
                vol,
            },
        },
        lo,
    )
}

/// **020 §4.2.** A volatile load does not read what was stored: a device register's value
/// is whatever the device put there. Modeling it as ordinary memory makes
/// `*reg = 0; if (*reg == 0)` a certainty, and on real hardware it is not.
#[test]
fn a_volatile_load_does_not_read_back_what_was_stored() {
    let m = one_local(
        vec![
            addr(0, 10),
            store(7, Volatility::Volatile, 20),
            load(1, Volatility::Volatile, 30),
        ],
        Operand::Value(ValueId(1)),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let v = r.states()[0]
        .local(ValueId(1))
        .expect("the load produced a value");
    match v {
        Value::Scalar(t) => assert!(
            a.as_const(t).is_none(),
            "a volatile load yields a fresh symbol, not the 7 that was written"
        ),
        other => panic!("{other:?}"),
    }
    // An ordinary load on the same address does read it back, or this test would pass
    // against a memory model that forgot the store entirely.
    let m2 = one_local(
        vec![
            addr(0, 10),
            store(7, Volatility::Normal, 20),
            load(1, Volatility::Normal, 30),
        ],
        Operand::Value(ValueId(1)),
    );
    let mut a2 = TermArena::new();
    let r2 = Engine::new(&m2).run(&mut a2);
    match r2.states()[0].local(ValueId(1)) {
        Some(Value::Scalar(t)) => assert_eq!(
            a2.as_const(t).map(|c| c.bits()),
            Some(7),
            "an ordinary load reads the stored bytes"
        ),
        other => panic!("{other:?}"),
    }
}

/// Two volatile loads of the same address are two reads, and may differ. A model that
/// gives them one value has CSE'd a device register, which §4.2 forbids in as many words.
#[test]
fn two_volatile_loads_of_one_address_are_two_values() {
    let m = one_local(
        vec![
            addr(0, 10),
            load(1, Volatility::Volatile, 20),
            load(2, Volatility::Volatile, 30),
        ],
        Operand::Value(ValueId(1)),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let (v1, v2) = (
        r.states()[0].local(ValueId(1)),
        r.states()[0].local(ValueId(2)),
    );
    assert!(v1.is_some() && v2.is_some());
    assert_ne!(v1, v2, "each read of a device register is its own read");
}

/// **020 contract 41.** "A `Volatile` **store** appears in the state's observable-effect
/// sequence exactly once per execution, and two stores to the same address are not
/// coalesced." A device register written twice was written twice; collapsing that loses
/// the sequence a driver's correctness depends on.
#[test]
fn volatile_stores_are_observable_effects_and_are_not_coalesced() {
    let m = one_local(
        vec![
            addr(0, 10),
            store(1, Volatility::Volatile, 20),
            store(2, Volatility::Volatile, 30),
        ],
        i32c(0),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let effects = r.states()[0].effects();
    assert_eq!(
        effects.len(),
        2,
        "two writes to a device register are two effects: {effects:?}"
    );
    assert_eq!(
        effects.iter().map(|e| e.span).collect::<Vec<_>>(),
        vec![at(20), at(30)],
        "in program order"
    );
    // An ordinary store records nothing: the sequence is about what the outside world
    // sees, and an ordinary store is invisible to it.
    let m2 = one_local(vec![addr(0, 10), store(1, Volatility::Normal, 20)], i32c(0));
    let mut a2 = TermArena::new();
    let r2 = Engine::new(&m2).run(&mut a2);
    assert!(r2.states()[0].effects().is_empty());
}

/// The effects reach the reader, and a run without any prints no section.
#[test]
fn observable_effects_appear_in_the_report() {
    let m = one_local(
        vec![addr(0, 10), store(1, Volatility::Volatile, 20)],
        i32c(0),
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let text = render(&r);
    assert!(text.contains("observable"), "{text}");
    let m2 = one_local(vec![addr(0, 10), store(1, Volatility::Normal, 20)], i32c(0));
    let mut a2 = TermArena::new();
    let r2 = Engine::new(&m2).run(&mut a2);
    assert!(!render(&r2).contains("observable"), "{}", render(&r2));
}

/// **One site, executed twice, is two effects.** The test above uses two *different*
/// store instructions, so a coalescer keyed on the site slips through it — a loop writing
/// one register on each pass is the shape that catches that, and is also what VPP's
/// counter code does.
#[test]
fn one_volatile_store_executed_twice_is_two_effects() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 1,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: at(1),
        }],
        blocks: vec![
            // A preheader: the verifier rejects a back edge into the entry block, and a
            // fixture that never runs reports zero effects and looks like a pass.
            block(0, vec![addr(0, 10)], Terminator::Goto(BlockId(1))),
            block(
                1,
                vec![store(1, Volatility::Volatile, 20)],
                Terminator::Goto(BlockId(1)),
            ),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 2,
            ..Budget::default()
        })
        .run(&mut a);
    let effects = r.states()[0].effects();
    assert!(
        effects.len() >= 2,
        "each pass writes the register again: {effects:?}"
    );
    assert!(
        effects.iter().all(|e| e.span == at(20)),
        "all from the one site, which is the point: {effects:?}"
    );
}
