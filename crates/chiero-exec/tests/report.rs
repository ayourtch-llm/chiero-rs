//! **The rendered report** — 023 contracts 12 and 14.
//!
//! Covers: 023 contract 12 (every degraded state has an assumption whose kind matches the
//! recorded cause **and whose text appears in the rendered report**), 023 contract 14
//! (`no bugs found` under a hit budget renders as "no bugs found within <bound>" and never
//! as "no bugs exist").
//!
//! Both contracts are about *text a person reads*, which is why they are golden tests
//! rather than assertions about structs: 023 §7 exists because "an LLM will read 'no bugs'
//! as 'safe'", and a run can carry every assumption correctly and still print a sentence
//! that overclaims.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
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

fn func(blocks: Vec<Block>, ret: CTy) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret,
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            span: Span::DUMMY,
        }],
        ..Default::default()
    }
}

/// A loop cut by `max_loop_iters`: the run finds nothing and is `Bounded`.
fn bounded_run(a: &mut TermArena) -> (Module, Budget) {
    let m = func(
        vec![
            block(0, vec![], Terminator::Goto(BlockId(1))),
            block(
                1,
                vec![],
                Terminator::Br {
                    cond: Operand::Const(Const::Int { bits: 1, val: 1 }),
                    t: BlockId(2),
                    f: BlockId(3),
                },
            ),
            block(2, vec![], Terminator::Goto(BlockId(1))),
            block(3, vec![], Terminator::Return(Some(i32c(0)))),
        ],
        CTy::Int(32),
    );
    let b = Budget {
        max_loop_iters: 3,
        ..Budget::default()
    };
    let _ = a;
    (m, b)
}

/// **023 contract 14.** The sentence a reader takes away from an empty finding list must
/// say what bound it holds within. "No bugs found" and "no bugs exist" are the same
/// sentence to a tool that reads the output and reports to a human; §7's rule 4 is that
/// only an `Exact` run may make the second claim, and this run is `Bounded`.
#[test]
fn no_findings_under_a_hit_budget_renders_the_bound_not_a_proof() {
    let mut a = TermArena::new();
    let (m, b) = bounded_run(&mut a);
    let r = Engine::new(&m).with_budget(b).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Bounded, "the fixture must be cut");
    assert!(r.findings().is_empty(), "and must find nothing");

    let text = render(&r);
    assert!(
        text.contains("no bugs found within"),
        "the bound belongs in the sentence: {text}"
    );
    assert!(
        !text.contains("no bugs exist"),
        "only an Exact run may say that, and this one is Bounded: {text}"
    );
    // The bound that was actually hit, by name and value — "within some bound" tells a
    // reader nothing they can act on.
    // `contains('3')` would have passed on "max_recursion_depth 32" — the bound and its
    // value have to be asserted *together* or the assertion is nearly vacuous.
    assert!(
        text.contains("max_loop_iters (3)"),
        "which bound was hit, and what it was set to: {text}"
    );
}

/// **023 contract 12, the half a struct cannot check.** The contract says the assumption's
/// *text* appears in the rendered report — "a dummy assumption must not satisfy this".
/// A run can record every assumption correctly and print none of them, and then the
/// fidelity is a number nobody can act on.
#[test]
fn every_assumption_behind_a_degradation_appears_in_the_report() {
    let mut a = TermArena::new();
    let (m, b) = bounded_run(&mut a);
    let r = Engine::new(&m).with_budget(b).run(&mut a);

    let text = render(&r);
    let assumptions: Vec<_> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .cloned()
        .collect();
    assert!(
        !assumptions.is_empty(),
        "the fixture must degrade, or this checks nothing"
    );
    for asm in &assumptions {
        assert!(
            text.contains(&asm.detail),
            "assumption text missing from the report: {:?}\n---\n{text}",
            asm.detail
        );
    }
}

/// An `Exact` run with nothing found is the *only* case allowed to sound conclusive, and
/// even then the report says what it searched rather than that the program is safe: 023
/// §7.1 makes `seal` the one place that decides, and a renderer that speaks for it would
/// be a second.
#[test]
fn an_exact_run_reports_exhaustion_without_claiming_the_program_is_safe() {
    let mut a = TermArena::new();
    let m = func(
        vec![block(0, vec![], Terminator::Return(Some(i32c(0))))],
        CTy::Int(32),
    );
    // Bounds that were **not** hit, and deliberately not the defaults: 023 §8 reports
    // them either way, "so a reader can tell `Exact`-with-generous-bounds from
    // `Exact`-with-trivial-bounds". A line that prints the names with some other run's
    // numbers tells them the opposite, and nothing in the assumptions would show it —
    // an `Exact` run has none.
    let r = Engine::new(&m)
        .with_budget(Budget {
            max_loop_iters: 7,
            max_states: 55,
            ..Budget::default()
        })
        .run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Exact);

    let text = render(&r);
    assert!(
        text.contains("max_loop_iters 7") && text.contains("max_states 55"),
        "the bounds in force, not some other run's: {text}"
    );
    assert!(
        text.contains("no bugs found"),
        "an exhaustive search that found nothing says so: {text}"
    );
    assert!(
        !text.contains("no bugs exist") && !text.contains("safe"),
        "chiero reports what it searched, not a verdict on the program: {text}"
    );
    assert!(
        text.contains("Exact"),
        "and the fidelity it holds at: {text}"
    );
}

/// A finding's own text is what a reader acts on, so it is in the report — and the
/// report says how many there were, because "1 finding" and "17 findings" are different
/// situations and a list alone makes them look the same.
#[test]
fn findings_are_rendered_with_their_own_text() {
    let mut a = TermArena::new();
    let m = func(
        vec![block(
            0,
            vec![inst_null_deref()],
            Terminator::Return(Some(i32c(0))),
        )],
        CTy::Int(32),
    );
    let r = Engine::new(&m).run(&mut a);
    let text = render(&r);
    assert!(
        !r.findings().is_empty(),
        "the fixture must find something: {text}"
    );
    for f in r.findings() {
        assert!(text.contains(&f), "finding missing from the report: {f}");
    }
    assert!(
        !text.contains("no bugs found"),
        "a run with findings has not found no bugs: {text}"
    );
}

/// `*(int *)0 = 1` — a null store, which is a definite finding at any tier.
fn inst_null_deref() -> Inst {
    Inst {
        kind: InstKind::Store {
            addr: Operand::Const(Const::Null),
            val: i32c(1),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
        span: Span::DUMMY,
    }
}

/// **A fork copies its parent's assumptions into every descendant**, so a flat
/// concatenation prints one degradation as many times as the run happened to branch
/// afterwards — which reads as a run in far worse shape than it is. Same argument
/// `RunResult::findings` makes for reports, one field over.
#[test]
fn one_degradation_before_a_fork_is_reported_once() {
    let mut a = TermArena::new();
    let mut f = Function {
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
        blocks: vec![
            block(
                0,
                vec![
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::AddrOfLocal {
                                alloca: AllocaId(0),
                            },
                        },
                        span: Span::DUMMY,
                    },
                    // Uninitialized: one `NoInformation` degradation, before any fork.
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Load {
                                addr: Operand::Value(ValueId(0)),
                                ty: CTy::Int(32),
                                align: 4,
                                vol: Volatility::Normal,
                            },
                        },
                        span: Span::DUMMY,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Cmp {
                                op: CmpOp::Eq,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(1)),
                                b: i32c(0),
                            },
                        },
                        span: Span::DUMMY,
                    },
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(2)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(0)))),
            block(2, vec![], Terminator::Return(Some(i32c(1)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    f.blocks[0].span = Span::DUMMY;
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.states().len() > 1,
        "the fixture must fork, or this checks nothing: {}",
        r.states().len()
    );
    let text = render(&r);
    let carried: usize = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .filter(|x| x.detail.contains("could not produce the program's value"))
        .count();
    assert!(
        carried > 1,
        "every forked state must carry the assumption, or the dedup is untested: {carried}"
    );
    let printed = text
        .matches("could not produce the program's value")
        .count();
    assert_eq!(printed, 1, "printed once, not once per descendant:\n{text}");
}
