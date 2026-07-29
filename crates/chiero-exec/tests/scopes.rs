//! **Scope markers** — 021 contracts 30 and 39, and the use-after-scope finding.
//!
//! Covers: 021 contracts 30 and 39; 020 contract 39.
//!
//! 020 c39 and 021 c30 state the same property from two sides — "`Lifetime::Function`
//! memory survives `Scope(Exit)` … and is retired at function return; accessing it after
//! the inner scope exits produces **no** finding" — and both halves are tested below, so
//! both are cited. One test citing one of two identical contracts leaves the other
//! reported as nobody's work.
//!
//! 020 §4.4: "`Scope` markers are **semantic**: they bound the lifetime of stack objects,
//! which is what makes use-after-scope detectable." `InstKind::Marker(_)` was a no-op, so
//! `Scope(Exit)` retired nothing, `Lifetime::Function` and `Lifetime::Scope` were
//! indistinguishable, and a pointer to a dead block read as live memory — the same
//! confidently-wrong answer as reading uninitialized bytes as zero, with a longer fuse.

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

fn alloca(id: u32, scope: u32, lifetime: Lifetime) -> AllocaDecl {
    AllocaDecl {
        id: AllocaId(id),
        ty: CTy::Int(32),
        count: 1,
        align: 4,
        scope: ScopeId(scope),
        lifetime,
        name: None,
        span: at(1),
    }
}

/// ```c
/// { int inner; p = &inner; }   // scope 1 ends here
/// *p = 1;                      // use after scope
/// ```
/// The alloca belongs to scope 1; the store happens after `Scope(Exit)` of scope 1.
fn escaping_scope(lifetime: Lifetime) -> Module {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![alloca(0, 1, lifetime)],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: ScopeId(1),
                        kind: ScopeKind::Enter,
                    })),
                    10,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    20,
                ),
                inst(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: ScopeId(1),
                        kind: ScopeKind::Exit,
                    })),
                    30,
                ),
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    40,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
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

/// **024 contract 10 / 021 §4.** A stack object accessed after its `Scope(Exit)` marker is
/// exactly one use-after-scope finding. Exactly one, not one per surviving state: the
/// report is about the program, and a fork afterwards does not make it two bugs.
#[test]
fn a_store_through_a_pointer_to_a_dead_scope_is_one_use_after_scope() {
    let m = escaping_scope(Lifetime::Scope);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let uas: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("use-after-scope"))
        .collect();
    assert_eq!(uas.len(), 1, "exactly one: {:#?}", r.findings());
    // The finding names the scope's end, which is what a reader needs to see why the
    // object is dead — 020 §4.4 keeps `scope` on the alloca for exactly this.
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("use-after-scope"))
        .unwrap();
    assert_eq!(f.span, at(40), "reported at the access");
    assert!(
        f.message.contains("30"),
        "and names where the scope ended: {}",
        f.message
    );
}

/// **021 contracts 30 and 39.** `alloca()` memory is `Lifetime::Function`: it survives
/// the `Scope(Exit)` of the block it was allocated in and is retired at function return.
/// Retiring it with the scope would report use-after-scope on a program that has none —
/// 020 §4.4 says so in as many words, and the two lifetimes were indistinguishable while
/// the marker did nothing.
#[test]
fn function_lifetime_memory_survives_an_inner_scope_exit() {
    let m = escaping_scope(Lifetime::Function);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        !r.findings().iter().any(|f| f.contains("use-after-scope")),
        "`alloca()` memory outlives the block: {:#?}",
        r.findings()
    );
}

/// A scope that ends without anything escaping it reports nothing — the marker retires
/// memory, it does not accuse the program of using it.
#[test]
fn an_ordinary_scope_exit_reports_nothing() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![alloca(0, 1, Lifetime::Scope)],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: ScopeId(1),
                        kind: ScopeKind::Enter,
                    })),
                    10,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    20,
                ),
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    25,
                ),
                inst(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: ScopeId(1),
                        kind: ScopeKind::Exit,
                    })),
                    30,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
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
    let r = Engine::new(&m).run(&mut a);
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "and nothing was approximated"
    );
}

/// **A scope's exit retires only its own objects.** An outer local is still live inside
/// and after an inner block, and retiring by anything coarser than the alloca's own
/// `ScopeId` would report a use-after-scope on every function with a nested block.
#[test]
fn an_inner_scope_exit_leaves_the_outer_scopes_objects_alone() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![alloca(0, 0, Lifetime::Scope), alloca(1, 1, Lifetime::Scope)],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    10,
                ),
                inst(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: ScopeId(1),
                        kind: ScopeKind::Enter,
                    })),
                    20,
                ),
                inst(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: ScopeId(1),
                        kind: ScopeKind::Exit,
                    })),
                    30,
                ),
                // The *outer* local, after the inner block ended.
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    40,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
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
    let r = Engine::new(&m).run(&mut a);
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
}

/// **021 contract 39's other half: retired at function return.**
///
/// ```c
/// int *g(void) { int local; return &local; }   // classic use-after-return
/// void f(void) { *g() = 1; }
/// ```
/// The frame is popped and its objects were never retired, so the store landed in memory
/// the callee no longer owns and chiero reported nothing — an `Exact` run over a program
/// whose whole bug is that the pointer is dead.
#[test]
fn a_pointer_to_a_callees_local_is_dead_after_the_return() {
    let callee = Function {
        id: FuncId(1),
        name: "g".into(),
        params: vec![],
        ret: CTy::Ptr,
        variadic: false,
        allocas: vec![alloca(0, 0, Lifetime::Scope)],
        blocks: vec![block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                },
                50,
            )],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let caller = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Call {
                        dst: Some(ValueId(0)),
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![],
                    },
                    60,
                ),
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    70,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![caller, callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let uas: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("left scope"))
        .collect();
    assert_eq!(uas.len(), 1, "exactly one: {:#?}", r.findings());
    // **And the run stays `Exact`.** A write through a pointer to a dead frame is a
    // definite fact about the program, modeled exactly; degrading here would claim
    // chiero was unsure when it was not. 023 §7 rule 3 keeps degradations meaning
    // something, and the engine already says so for null dereferences and bad frees.
    assert_eq!(r.fidelity(), Fidelity::Exact, "{:#?}", r.findings());
}

/// And a *live* frame's objects are not retired by an inner call returning. Retiring on
/// every return rather than the returning frame's own objects would make every callee's
/// return kill the caller's locals — a use-after-scope on every program with a function
/// call in it.
#[test]
fn a_callers_locals_survive_a_callees_return() {
    let callee = Function {
        id: FuncId(1),
        name: "g".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![alloca(0, 0, Lifetime::Scope)],
        blocks: vec![block(0, vec![], Terminator::Return(Some(i32c(7))))],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let caller = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![alloca(0, 0, Lifetime::Scope)],
        blocks: vec![block(
            0,
            vec![
                inst(
                    InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    10,
                ),
                inst(
                    InstKind::Call {
                        dst: Some(ValueId(1)),
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![],
                    },
                    20,
                ),
                // The caller's own local, after the callee returned.
                inst(
                    InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                ),
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![caller, callee],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(r.findings().is_empty(), "{:#?}", r.findings());
    assert_eq!(r.fidelity(), Fidelity::Exact);
}

/// **021 contract 29.** "An alloca in a loop body executed 3 times yields 3 distinct
/// `ObjectId`s." The object is the *activation* of the declaration, not the declaration:
/// one object across iterations makes the previous iteration's contents readable through
/// this iteration's pointer, and a pointer that escaped one iteration look live in the
/// next — which is the bug the loop was written to have.
#[test]
fn an_alloca_in_a_loop_body_is_a_new_object_each_time_the_scope_opens() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![alloca(0, 1, Lifetime::Scope)],
        blocks: vec![
            // A preheader: the verifier rejects a back edge into the entry block, so a
            // self-loop on block 0 never runs at all — the fixture would have measured
            // nothing and said so as "one object".
            block(0, vec![], Terminator::Goto(BlockId(1))),
            // The loop body's scope opens, a pointer is taken, the scope closes, and the
            // back edge runs it again. `max_loop_iters` bounds the count.
            block(
                1,
                vec![
                    inst(
                        InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                            scope: ScopeId(1),
                            kind: ScopeKind::Enter,
                        })),
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::AddrOfLocal {
                                alloca: AllocaId(0),
                            },
                        },
                        20,
                    ),
                    inst(
                        InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                            scope: ScopeId(1),
                            kind: ScopeKind::Exit,
                        })),
                        30,
                    ),
                ],
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
            max_loop_iters: 3,
            ..Budget::default()
        })
        .run(&mut a);
    let seen: Vec<_> = r
        .states()
        .iter()
        .flat_map(|s| s.object_ids_for_test())
        .collect();
    let mut uniq = seen.clone();
    uniq.sort_by_key(|o| o.0);
    uniq.dedup();
    assert!(
        uniq.len() >= 3,
        "three passes through the scope are three objects: {seen:?}"
    );
}
