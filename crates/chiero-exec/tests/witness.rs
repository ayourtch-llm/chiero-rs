//! **Witnesses** — 023 §9 and contract 15.
//!
//! Covers: 023 contract 15 (every finding carries a `Witness`, or an explicit
//! `witness: None` with a recorded reason).
//!
//! §9: "`Witness` is a concrete assignment for every symbolic input on the path …
//! It is what 040 turns into a compilable C replay harness, and it is what distinguishes
//! a chiero finding from a plausible-sounding guess." A finding without one is a claim
//! the reader has to take on trust, which is the thing chiero exists not to produce.

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
    Inst { kind, span: at(lo) }
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

/// `int x = <input>; if (x > 10) { *(int *)0 = 1; }` — one input, one branch, and a
/// definite fault reachable only on one side of it. The witness has to name a value of
/// `x` that actually takes that side, or it replays into the other branch and finds
/// nothing.
fn guarded_fault() -> Module {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::SGt,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: i32c(10),
                            },
                        },
                        20,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![f],
        ..Default::default()
    }
}

/// **023 contract 15.** Every report is either witnessed or says why it is not. There is
/// no third option: a finding with a silently absent witness is indistinguishable from
/// one whose witness was never attempted.
#[test]
fn every_finding_is_witnessed_or_says_why_not() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let reports = r.reports();
    assert!(!reports.is_empty(), "the fixture must find the null store");
    for f in &reports {
        match (&f.witness, &f.unwitnessed) {
            (Some(_), None) => {}
            (None, Some(why)) => assert!(!why.is_empty(), "an empty reason is no reason"),
            (Some(_), Some(_)) => panic!("witnessed and excused at once: {f:?}"),
            (None, None) => panic!("a finding with neither witness nor reason: {f:?}"),
        }
    }
}

/// The witness binds the **input**, and to a value that actually reaches the fault. A
/// witness that satisfies the path condition vacuously — any value at all — replays into
/// the other branch, and a replay that does not reproduce is worse than no witness: it
/// reads as a refutation of a real bug.
#[test]
fn the_witness_binds_an_input_to_a_value_that_reaches_the_fault() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = f.witness.expect("this path's inputs are all decidable");
    assert_eq!(
        w.bindings.len(),
        1,
        "one symbolic input on this path: {:?}",
        w.bindings
    );
    let b = &w.bindings[0];
    assert_eq!(b.width, 32, "at the input's own width: {b:?}");
    assert!(
        (b.value as i32) > 10,
        "the guard is `x > 10`, so a witness at {} reaches the other block",
        b.value as i32
    );
    // And it names *which* input, at the site that created it — a bare number is not a
    // replay harness.
    assert_eq!(b.origin.span(), at(10), "the site that created the input");
}

/// A finding on a path with no symbolic inputs at all still gets a witness: the empty
/// assignment is a complete one. Reporting `None` here would say "we could not produce
/// one", which is false and would send a reader looking for a solver problem.
#[test]
fn a_path_with_no_inputs_is_witnessed_by_the_empty_assignment() {
    let m = Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks: vec![block(
                0,
                vec![inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            )],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: Span::DUMMY,
        }],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = f
        .witness
        .expect("nothing symbolic is on this path, so nothing is undecided");
    assert!(w.bindings.is_empty(), "{:?}", w.bindings);
}

/// The rendered report carries the witness, because a witness a reader cannot see is a
/// witness that does not distinguish this finding from a guess (023 §9).
#[test]
fn the_report_shows_the_witness() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let text = render(&r);
    assert!(
        text.contains("witness"),
        "the report names the witness: {text}"
    );
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .unwrap();
    let b = &f.witness.as_ref().unwrap().bindings[0];
    assert!(
        text.contains(&format!("{}", b.value as i32)),
        "with the value it binds: {text}"
    );
}

/// Two inputs, in a fixed order: an uninitialized load at span 50, then an extern return
/// at span 60, then a null store. Used both to check that every minting site is recorded
/// and that a replay consumes the bindings in *that* order.
fn two_input_fault() -> Module {
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
            span: Span::DUMMY,
        }],
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
                    40,
                ),
                // Uninitialized: the load produces no value, so chiero invents one.
                inst(
                    InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(0)),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                    },
                    50,
                ),
                // An extern with no body and no model: its return is an input too.
                inst(
                    InstKind::Call {
                        dst: Some(ValueId(2)),
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![],
                    },
                    60,
                ),
                inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
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
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let ext = Function {
        id: FuncId(1),
        name: "some_extern".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![f, ext],
        ..Default::default()
    }
}

/// **Every symbol-minting site is an input, not just `Fresh`.** A witness that omits one
/// is worse than no witness: it looks complete, and the replay harness built from it
/// supplies every value but that one, so the bug does not reproduce and reads as refuted.
/// Two more of the six sites, each with its own origin — the load and the extern return.
#[test]
fn a_load_with_no_value_and_an_extern_return_are_both_inputs() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = two_input_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = f.witness.expect("both inputs are decidable");
    let origins: Vec<_> = w.bindings.iter().map(|b| b.origin.clone()).collect();
    assert!(
        origins.contains(&InputOrigin::Load { span: at(50) }),
        "the load that produced no value: {origins:?}"
    );
    assert!(
        origins.iter().any(|o| matches!(
            o,
            InputOrigin::ExternReturn { func, span } if func == "some_extern" && *span == at(60)
        )),
        "the extern return, named: {origins:?}"
    );
}

/// **080's M1 exit item: an out-of-bounds finding with a witness.**
///
/// `int buf[4]; if (x > 10) buf[16] = 1;` — the access is out of bounds by construction,
/// but only on one side of a branch the input decides. The finding names the object and
/// the witness names the input value that gets there, which together are what 040 turns
/// into a replay harness. Either alone is a guess: the fault without the input is a
/// claim about a path nobody can re-enter.
#[test]
fn an_out_of_bounds_write_is_reported_with_a_witness() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 4,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: at(5),
        }],
        blocks: vec![
            block(
                0,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::SGt,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: i32c(10),
                            },
                        },
                        20,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::AddrOfLocal {
                                alloca: AllocaId(0),
                            },
                        },
                        30,
                    ),
                    // `&buf[16]` — four elements past the end of a four-element array.
                    inst(
                        InstKind::Assign {
                            dst: ValueId(3),
                            rv: RValue::PtrAdd {
                                base: Operand::Value(ValueId(2)),
                                off: Operand::Const(Const::Int { bits: 64, val: 64 }),
                            },
                        },
                        35,
                    ),
                    inst(
                        InstKind::Store {
                            addr: Operand::Value(ValueId(3)),
                            val: i32c(1),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                        40,
                    ),
                ],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let oob = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("out-of-bounds") || f.message.contains("bounds"))
        .unwrap_or_else(|| panic!("the write past the end is reported: {:#?}", r.findings()));
    assert_eq!(oob.span, at(40), "at the access, not at the branch");
    let w = oob.witness.expect("witnessed");
    assert_eq!(w.bindings.len(), 1, "{:?}", w.bindings);
    let b = &w.bindings[0];
    assert!(
        b.pinned && (b.value as i32) > 10,
        "the input value that reaches the access: {b:?}"
    );
    // The other side of the branch is a path with no finding at all — so the witness is
    // load-bearing, not decoration.
    assert_eq!(
        r.reports().len(),
        1,
        "one finding, on the one path that has it: {:#?}",
        r.findings()
    );
}

/// **023 contract 16.** "Replaying a `Witness` through the engine with all inputs
/// concretized re-reaches the same `Finding` at the same `Span`."
///
/// This is the test that makes every other witness assertion mean something. A witness
/// can name the right inputs, at the right widths, with values that satisfy the path
/// condition, and still not reproduce the bug — if the values are read in a different
/// order, or an input the engine mints was never recorded, the replay walks a different
/// path and finds nothing. Nothing short of running it again detects that.
#[test]
fn replaying_a_witness_re_reaches_the_same_finding_at_the_same_span() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = guarded_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend.clone()).run(&mut a);
    let original = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = original.witness.clone().expect("witnessed");

    // The replay is *concrete*: with every input pinned there is nothing left to fork on.
    let mut a2 = TermArena::new();
    let replay = Engine::new(&m)
        .with_backend(backend)
        .replaying(w)
        .run(&mut a2);
    let again = replay
        .reports()
        .into_iter()
        .find(|f| f.message == original.message)
        .unwrap_or_else(|| panic!("the same finding, re-reached: {:#?}", replay.findings()));
    assert_eq!(again.span, original.span, "at the same span");
    assert_eq!(
        replay.states().len(),
        1,
        "every input is concrete, so there is one path: {:#?}",
        replay.states().len()
    );
    // And the replay consulted no solver about the branch it took — the condition is
    // ground once the input is bound, which is the whole point of concretizing.
    assert_eq!(replay.solver_calls, 0, "a concrete replay asks nothing");
}

/// A witness replayed against a *different* program is not a witness for it. The engine
/// says so rather than quietly binding the values positionally and reporting whatever it
/// then finds — a replay that silently drifts is how a refuted bug and an unrelated one
/// come to look the same.
#[test]
fn a_witness_that_does_not_fit_the_run_is_reported_not_absorbed() {
    // Two inputs, so there **is** a second site to mis-report at.
    let m = two_input_fault();
    let mut a = TermArena::new();
    // A witness whose single binding claims to come from a site this module does not
    // have an input at.
    let bogus = Witness {
        bindings: vec![Binding {
            origin: InputOrigin::Load { span: at(999) },
            width: 32,
            value: 11,
            pinned: true,
        }],
    };
    let r = Engine::new(&m).replaying(bogus).run(&mut a);
    let diverged: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("replay") && f.contains("diverged"))
        .collect();
    assert!(
        !diverged.is_empty(),
        "the mismatch is reported: {:#?}",
        r.findings()
    );
    // **And it is reported once.** This fixture asks for two inputs; a replay that
    // diverges and keeps consuming reports the same problem again at every remaining
    // site, turning one problem into a list and burying it.
    assert_eq!(diverged.len(), 1, "{diverged:#?}");
}

/// **Bindings are consumed in creation order, and the order is checked.** With one input
/// a replay cursor that never advances is invisible; with two it binds the extern's
/// return from the load's binding, which is a different program's answer. The origin
/// check is what turns that into a reported divergence instead of a quiet wrong replay.
#[test]
fn a_two_input_witness_replays_in_order() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = two_input_fault();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend.clone()).run(&mut a);
    let original = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    let w = original.witness.clone().expect("witnessed");
    // The two the engine minted come first, in creation order; the four bytes *memory*
    // invented for the uninitialized load follow. 023 §9 lists "lazily-materialized
    // object contents" among what a witness binds, and leaving them out was a witness
    // that said the path had two inputs when it had six.
    assert!(
        matches!(w.bindings[0].origin, InputOrigin::Load { .. })
            && matches!(w.bindings[1].origin, InputOrigin::ExternReturn { .. }),
        "{:?}",
        w.bindings
    );
    assert_eq!(
        w.bindings
            .iter()
            .filter(|b| matches!(b.origin, InputOrigin::Memory { .. }))
            .count(),
        4,
        "one per byte of the uninitialized `int`: {:?}",
        w.bindings
    );

    let mut a2 = TermArena::new();
    let replay = Engine::new(&m)
        .with_backend(backend)
        .replaying(w)
        .run(&mut a2);
    assert!(
        replay.findings().contains(&original.message),
        "the same finding, re-reached: {:#?}",
        replay.findings()
    );
    assert!(
        !replay.findings().iter().any(|f| f.contains("diverged")),
        "and no divergence: {:#?}",
        replay.findings()
    );
    // **And the replay says what it could not supply.** Memory mints its own symbols and
    // nothing routes a binding back into it, so those four bytes came out fresh on the
    // second run. A replay that quietly is not a reproduction is worse than one that
    // says so.
    assert!(
        replay
            .findings()
            .iter()
            .any(|f| f.contains("replay incomplete")),
        "{:#?}",
        replay.findings()
    );
}

/// **Every path's findings are witnessed, and each carries its own path's fidelity.**
///
/// Review found both unpinned: attaching a witness only to the first-explored state left
/// contract 15 satisfied on one path out of many, and `Finding::fidelity` could be
/// hardcoded to `Exact` with every test still green — so a report could show a finding
/// from a degraded path as if the path had been modeled exactly.
#[test]
fn findings_on_every_path_are_witnessed_and_carry_that_paths_fidelity() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    // `if (x > 10) *(int*)0 = 1; else <unmodeled call>; *(int*)0 = 2;` — a fault on the
    // first path, and a fault on the second reached only after a degradation.
    let ext = Function {
        id: FuncId(1),
        name: "no_such_function".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::SGt,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: i32c(10),
                            },
                        },
                        20,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            ),
            block(
                2,
                vec![
                    inst(
                        InstKind::Call {
                            dst: None,
                            callee: Callee::Direct(FuncId(1)),
                            args: vec![],
                        },
                        40,
                    ),
                    inst(
                        InstKind::Store {
                            addr: Operand::Const(Const::Null),
                            val: i32c(2),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                        50,
                    ),
                ],
                Terminator::Return(Some(i32c(0))),
            ),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f, ext],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let reports = r.reports();
    assert!(
        reports.len() >= 2,
        "both paths fault, so both report: {:#?}",
        r.findings()
    );
    for f in &reports {
        assert!(
            f.witness.is_some() || f.unwitnessed.is_some(),
            "contract 15 is about every finding, not the first path's: {f:?}"
        );
    }
    // The two findings come from paths of different fidelity, and each says its own.
    let mut levels: Vec<_> = reports.iter().map(|f| f.fidelity).collect();
    levels.sort_by_key(|f| format!("{f:?}"));
    levels.dedup();
    assert!(
        levels.len() >= 2,
        "one path is exact and the other degraded; the findings must not both claim one \
         of those: {:#?}",
        reports
            .iter()
            .map(|f| (&f.message, f.fidelity))
            .collect::<Vec<_>>()
    );
    // And the rendered report shows each finding's own level.
    let text = render(&r);
    for f in &reports {
        assert!(
            text.contains(&format!("{:?}", f.fidelity)),
            "the report shows {:?}: {text}",
            f.fidelity
        );
    }
}

/// **An entry parameter is an input** — the first thing 023 §9 lists, and nothing
/// witnessed a program with one until review pointed it out.
#[test]
fn a_scalar_parameter_is_bound_by_the_witness() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Int(32),
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![inst(
                    InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cmp {
                            op: CmpOp::SGt,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(0)),
                            b: i32c(10),
                        },
                    },
                    20,
                )],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the guarded null store is reported");
    let w = f.witness.expect("witnessed");
    let b = w
        .bindings
        .iter()
        .find(|b| matches!(b.origin, InputOrigin::Param { .. }))
        .unwrap_or_else(|| panic!("the parameter is an input: {:?}", w.bindings));
    assert_eq!(b.width, 32);
    assert!(
        (b.value as i32) > 10,
        "and is bound to a value that reaches the fault: {b:?}"
    );
    assert!(
        render(&r).contains("parameter"),
        "the report names it as a parameter"
    );
}

/// **An input the path never constrains is marked as such.** The model need not assign a
/// variable the query does not mention, and binding it to zero while calling it the
/// solver's answer tells a reader the bug needs a value it does not — the module doc says
/// this is the thing a witness must not do, and review found nothing checking it.
#[test]
fn an_input_the_path_does_not_constrain_is_not_reported_as_pinned() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        10,
                    ),
                    // A second input nothing ever looks at: the fault does not need it,
                    // so no value of it is *the* value.
                    inst(
                        InstKind::Assign {
                            dst: ValueId(9),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        15,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::SGt,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: i32c(10),
                            },
                        },
                        20,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![inst(
                    InstKind::Store {
                        addr: Operand::Const(Const::Null),
                        val: i32c(1),
                        ty: CTy::Int(32),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                    30,
                )],
                Terminator::Return(Some(i32c(0))),
            ),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the guarded null store is reported");
    let w = f.witness.expect("witnessed");
    let unused = w
        .bindings
        .iter()
        .find(|b| b.origin.span() == at(15))
        .unwrap_or_else(|| panic!("the second input is still an input: {:?}", w.bindings));
    assert!(
        !unused.pinned,
        "nothing on the path mentions it, so no value is the value: {unused:?}"
    );
    let needed = w
        .bindings
        .iter()
        .find(|b| b.origin.span() == at(10))
        .unwrap();
    assert!(needed.pinned, "and the one the guard reads is: {needed:?}");
    assert!(
        render(&r).contains("any value replays"),
        "the report says which is which: {}",
        render(&r)
    );
}

/// **The origin check is not a span compare, and a diverged replay stops.**
///
/// Two bindings whose *spans* are right and whose kinds are not: a `Load` where the run
/// wants the extern's return. Reducing the check to spans lets a binding from a different
/// kind of site satisfy the request silently, which is precisely the mis-binding the
/// check exists to catch — macro-expanded code repeats spans constantly.
///
/// And once the replay has diverged it stops consuming: reporting the same divergence
/// once per remaining input turns one problem into a list.
#[test]
fn a_wrong_kind_at_the_right_span_diverges_once() {
    let m = two_input_fault();
    let mut a = TermArena::new();
    let w = Witness {
        bindings: vec![
            Binding {
                origin: InputOrigin::Load { span: at(50) },
                width: 32,
                value: 0,
                pinned: true,
            },
            // Right span, wrong kind: span 60 is the extern call, not a load.
            Binding {
                origin: InputOrigin::Load { span: at(60) },
                width: 32,
                value: 0,
                pinned: true,
            },
        ],
    };
    let r = Engine::new(&m).replaying(w).run(&mut a);
    let diverged: Vec<_> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("diverged"))
        .collect();
    assert_eq!(
        diverged.len(),
        1,
        "one divergence, reported once: {:#?}",
        r.findings()
    );
    assert!(
        diverged[0].contains("extern") && diverged[0].contains("load"),
        "and it names both sides: {}",
        diverged[0]
    );
}

/// **Two faults are two reports, each with its own witness.**
///
/// `int buf[4]; long i = (x > 10) ? 64 : 128; *(int *)((char *)buf + i) = 1;` — two paths,
/// two out-of-bounds writes at two offsets. Deduplicating across paths on
/// `(kind, span, object, func)` collapsed them into one and discarded the other's witness,
/// so a reader saw one of two bugs and no sign of the other. 023 contract 20: "the engine
/// does not deduplicate"; §6.1 delegates the real key to 040, which has the checker
/// component this key lacks. Found by review.
#[test]
fn two_faults_at_one_site_on_two_paths_are_two_reports() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 4,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: at(5),
        }],
        blocks: vec![
            block(
                0,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        10,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Cmp {
                                op: CmpOp::SGt,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: i32c(10),
                            },
                        },
                        20,
                    ),
                    inst(
                        InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::AddrOfLocal {
                                alloca: AllocaId(0),
                            },
                        },
                        25,
                    ),
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(
                1,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(3),
                            rv: RValue::PtrAdd {
                                base: Operand::Value(ValueId(2)),
                                off: Operand::Const(Const::Int { bits: 64, val: 64 }),
                            },
                        },
                        30,
                    ),
                    inst(
                        InstKind::Store {
                            addr: Operand::Value(ValueId(3)),
                            val: i32c(1),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                        40,
                    ),
                ],
                Terminator::Return(Some(i32c(0))),
            ),
            block(
                2,
                vec![
                    inst(
                        InstKind::Assign {
                            dst: ValueId(4),
                            rv: RValue::PtrAdd {
                                base: Operand::Value(ValueId(2)),
                                off: Operand::Const(Const::Int { bits: 64, val: 128 }),
                            },
                        },
                        30,
                    ),
                    inst(
                        InstKind::Store {
                            addr: Operand::Value(ValueId(4)),
                            val: i32c(1),
                            ty: CTy::Int(32),
                            align: 4,
                            vol: Volatility::Normal,
                        },
                        40,
                    ),
                ],
                Terminator::Return(Some(i32c(0))),
            ),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    let oob: Vec<_> = r
        .reports()
        .into_iter()
        .filter(|f| f.message.contains("out-of-bounds"))
        .collect();
    assert_eq!(oob.len(), 2, "two writes, two reports: {:#?}", r.findings());
    assert!(
        oob.iter().any(|f| f.message.contains("offset 64"))
            && oob.iter().any(|f| f.message.contains("offset 128")),
        "at both offsets: {:#?}",
        oob.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    // Each with the input value that reaches *it* — one witness for two bugs is one bug
    // reported and one lost.
    let vals: Vec<i32> = oob
        .iter()
        .filter_map(|f| f.witness.as_ref())
        .flat_map(|w| w.bindings.iter().map(|b| b.value as i32))
        .collect();
    assert!(
        vals.iter().any(|v| *v > 10) && vals.iter().any(|v| *v <= 10),
        "the two paths need different inputs: {vals:?}"
    );
}
