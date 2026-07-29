//! Covers: 020 §4.1 — **a UB event is observed by a checker and reported**.
//!
//! §4.1 is explicit that the engine's job stops at the event: "CIR is not the place to
//! encode UB as unpredictability: the semantics are defined and total, and a `Checker`
//! observes the overflow event and reports it." The engine holds up its half — every
//! division by zero, over-wide shift and signed overflow it can see becomes a `UbEvent` on
//! the state, and wave 156 taught it to ask the solver about a symbolic divisor as well.
//!
//! Nothing reads them. `default_checkers()` ships one checker and it watches sequence
//! points. So a run over a program that divides by zero finishes with the event recorded,
//! `reports()` empty, and nothing a caller of the library would ever see — which is the
//! same shape as the defect wave 156 fixed one layer down, and for the same reason: the
//! information exists and no one is asked for it.
//!
//! **Fixtures are CIR**, as in `order_dependence.rs`: 001 §4 rule 7 forbids this crate a
//! frontend dependency, and the checker's input is a `UbEvent` on a state, so a hand-built
//! module is the thing under test rather than a C program that happens to lower to one.

use chiero_cir::*;
use chiero_exec::{Engine, Witness};
use chiero_solver::TermArena;
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
        gcov_lines: Default::default(),
        span: at(1),
    }
}

fn k(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn module(blocks: Vec<Block>) -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: at(1),
            linkage: chiero_cir::Linkage::External,
        }],
        ..Default::default()
    }
}

/// Run with the checkers 040 §1 turns on unless told otherwise.
fn findings(m: &Module) -> Vec<String> {
    let mut a = TermArena::new();
    let mut e = Engine::new(m);
    for c in chiero_check::default_checkers() {
        e = e.with_checker(c);
    }
    e.run(&mut a)
        .reports()
        .into_iter()
        .map(|f| f.message)
        .collect()
}

/// **A division by zero is a finding, not just an event.**
///
/// The straight-line case, and the one that says the wiring exists at all. `100 / 0` is
/// undefined (C11 6.5.5p5), the engine records it, and a caller asking for the default
/// checkers gets nothing.
#[test]
fn a_division_by_zero_is_reported() {
    let m = module(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op: BinOp::SDiv,
                        ty: CTy::Int(32),
                        a: k(100),
                        b: k(0),
                        signed: true,
                    },
                },
                10,
            ),
            // Instructions after the fault. §4.1 says the path continues, so a checker
            // that reports on "the state has a UB event" rather than on *this* event
            // reports once per instruction from here on.
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Use(k(1)),
                },
                20,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Use(k(2)),
                },
                30,
            ),
        ],
        Terminator::Return(Some(Operand::Value(ValueId(0)))),
    )]);
    let f = findings(&m);
    assert_eq!(
        f.len(),
        1,
        "one division by zero is one finding, whatever runs after it: {f:?}"
    );
    assert!(
        f[0].to_lowercase().contains("zero"),
        "the finding should name the fault: {f:?}"
    );
}

/// **And an operation with no UB is not reported.**
///
/// The negative half, and the one that makes the positive half worth anything: a checker
/// that reported on every arithmetic instruction would satisfy the test above.
#[test]
fn arithmetic_without_undefined_behaviour_is_not_reported() {
    let m = module(vec![block(
        0,
        vec![inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: k(5),
                    signed: true,
                },
            },
            10,
        )],
        Terminator::Return(Some(Operand::Value(ValueId(0)))),
    )]);
    assert!(
        findings(&m).is_empty(),
        "100 / 5 is defined: {:?}",
        findings(&m)
    );
}

/// **Every UB kind the engine records reaches a report**, not only division.
///
/// The engine's table has three rows and they share one code path, so a checker matching on
/// `DivByZero` alone would pass the first test and silently drop the other two.
#[test]
fn shifts_and_overflow_are_reported_too() {
    for (op, a_val, b_val, want) in [
        (BinOp::Shl, 1i128, 33i128, "shift"),
        (BinOp::Add, i128::from(i32::MAX), 1, "overflow"),
    ] {
        let m = module(vec![block(
            0,
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Bin {
                        op,
                        ty: CTy::Int(32),
                        a: k(a_val),
                        b: k(b_val),
                        signed: true,
                    },
                },
                10,
            )],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )]);
        let f = findings(&m);
        assert_eq!(f.len(), 1, "{op:?}: expected exactly one finding: {f:?}");
        assert!(
            f[0].to_lowercase().contains(want),
            "{op:?}: the finding should name the fault: {f:?}"
        );
    }
}

/// **One site reached twice on one path is one finding** (023 §6.1).
///
/// A loop runs the same division repeatedly and it is one bug. The engine's own
/// deduplication cannot help here: `Action::Report` carries no §6.1 key, so `reports()`
/// deduplicates a *fork's* copies by id and leaves everything else to the checker — which
/// is why the checker needs per-state memory rather than a counter.
#[test]
fn one_faulting_site_in_a_loop_is_one_finding() {
    // Block 0 divides and jumps to 1; block 1 divides at the *same span* and returns.
    // Two executions of one source site is what a loop looks like to a checker.
    let divide = |dst: u32| {
        inst(
            InstKind::Assign {
                dst: ValueId(dst),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: k(0),
                    signed: true,
                },
            },
            10,
        )
    };
    let m = module(vec![
        block(0, vec![divide(0)], Terminator::Goto(BlockId(1))),
        block(
            1,
            vec![divide(1)],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        ),
    ]);
    let f = findings(&m);
    assert_eq!(
        f.len(),
        1,
        "the same division at the same place, twice on one path, is one bug: {f:?}"
    );
}

/// **A site already reported before a fork is not reported again on either child**
/// (023 §6.1).
///
/// The engine deduplicates a fork's copies of one report by id, and measurement says that
/// is what carries this case: both children finish holding the *same* pre-fork report and
/// `reports()` collapses them. The children do not re-report at the second site.
///
/// So this fixture pins the observable property — one site, one finding, across a fork —
/// **without** reaching `on_fork`'s copy of `reported`. That copy is currently unobservable:
/// a mutation emptying it survives every test here. Recorded in §9 rather than deleted,
/// because the field is right and the missing fixture is a statement about the suite.
#[test]
fn a_site_reported_before_a_fork_is_not_reported_again_after_it() {
    let divide = |dst: u32| {
        inst(
            InstKind::Assign {
                dst: ValueId(dst),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: k(0),
                    signed: true,
                },
            },
            10,
        )
    };
    let m = module(vec![
        // Fault, then branch on something symbolic so both sides are explored.
        block(
            0,
            vec![
                divide(0),
                inst(
                    InstKind::Assign {
                        dst: ValueId(5),
                        rv: RValue::Fresh { ty: CTy::Int(32) },
                    },
                    20,
                ),
                inst(
                    InstKind::Assign {
                        dst: ValueId(6),
                        rv: RValue::Cmp {
                            op: CmpOp::Eq,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(5)),
                            b: k(0),
                        },
                    },
                    21,
                ),
            ],
            Terminator::Br {
                cond: Operand::Value(ValueId(6)),
                t: BlockId(1),
                f: BlockId(2),
            },
        ),
        block(1, vec![], Terminator::Goto(BlockId(3))),
        block(2, vec![], Terminator::Goto(BlockId(3))),
        // Both children reach the same faulting site again.
        block(
            3,
            vec![divide(1)],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        ),
    ]);
    let f = findings(&m);
    assert_eq!(
        f.len(),
        1,
        "the site was reported before the fork; neither child should say it again: {f:?}"
    );
}

/// Run and return the findings with their witnesses attached.
fn witnessed(m: &Module) -> Vec<(String, Option<Witness>)> {
    let mut a = TermArena::new();
    let mut e = Engine::new(m);
    for c in chiero_check::default_checkers() {
        e = e.with_checker(c);
    }
    e.run(&mut a)
        .reports()
        .into_iter()
        .map(|f| (f.message, f.witness))
        .collect()
}

/// **The witness beside a division by zero must name a divisor that is zero.**
///
/// 023 §9's whole point is that a witness is a claim someone can re-run: "a
/// non-reproducible bug report is not a bug report". A finding that says *division by zero*
/// and hands the reader an input under which the divisor is -42 is worse than a finding
/// with no witness at all, because the number invites exactly the check that will fail.
///
/// The engine has the right answer and discards it. `symbolic_div_by_zero` asks the solver
/// whether the divisor can be zero and gets back `Sat(model)` — a model naming `x = 42`,
/// which is the value that makes `x - 42` zero. The witness is then built from a *different*
/// query, over the path condition alone, and an unconstrained path yields the filler zero.
///
/// The divisor is `x - 42` rather than `x` on purpose. With a bare `x` the filler and the
/// answer coincide, and every fixture written so far has had that shape — which is why this
/// went unnoticed through waves 156 and 157.
#[test]
fn the_witness_for_a_division_by_zero_makes_the_divisor_zero() {
    let m = module(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                },
                5,
            ),
            // d = x - 42, which is zero exactly at x = 42.
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin {
                        op: BinOp::Sub,
                        ty: CTy::Int(32),
                        a: Operand::Value(ValueId(0)),
                        b: k(42),
                        signed: true,
                    },
                },
                6,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Bin {
                        op: BinOp::SDiv,
                        ty: CTy::Int(32),
                        a: k(100),
                        b: Operand::Value(ValueId(1)),
                        signed: true,
                    },
                },
                10,
            ),
        ],
        Terminator::Return(Some(Operand::Value(ValueId(2)))),
    )]);
    let f = witnessed(&m);
    assert_eq!(f.len(), 1, "expected the division to be reported: {f:?}");
    let (msg, w) = &f[0];
    let w = w
        .as_ref()
        .unwrap_or_else(|| panic!("`{msg}` came with no witness at all"));
    let x = w
        .bindings
        .first()
        .unwrap_or_else(|| panic!("`{msg}`: the witness binds nothing"));
    assert_eq!(
        x.value as u32 as i32,
        42,
        "`{msg}`: the witness names x = {}, under which the divisor is {} and nothing \
         faults. 023 §9 asks for an input that reproduces the finding.",
        x.value as u32 as i32,
        (x.value as u32 as i32).wrapping_sub(42)
    );
    assert!(
        x.pinned,
        "`{msg}`: the fault needs this exact value, so reporting it as free is a second \
         wrong claim about the same binding: {x:?}"
    );
}

/// **And the witness for a divisor that is plainly the input still names it.**
///
/// The shape every earlier fixture had. Kept so a fix aimed at the case above cannot break
/// the case that already worked — and so the pair says the value is *derived*, not guessed.
#[test]
fn the_witness_for_a_bare_symbolic_divisor_is_zero() {
    let m = module(vec![block(
        0,
        vec![
            inst(
                InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Int(32) },
                },
                5,
            ),
            inst(
                InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin {
                        op: BinOp::SDiv,
                        ty: CTy::Int(32),
                        a: k(100),
                        b: Operand::Value(ValueId(0)),
                        signed: true,
                    },
                },
                10,
            ),
        ],
        Terminator::Return(Some(Operand::Value(ValueId(1)))),
    )]);
    let f = witnessed(&m);
    assert_eq!(f.len(), 1, "expected the division to be reported: {f:?}");
    let w = f[0].1.as_ref().expect("witnessed");
    assert_eq!(w.bindings[0].value, 0, "the divisor is the input itself");
    assert!(w.bindings[0].pinned, "and the fault needs it to be zero");
}

/// A `Fresh` input, then `100 / (x - d)`, for each `d` given — every division on one path.
fn divisions_at(offsets: &[i128], tail: Terminator, extra: Vec<Inst>) -> Module {
    let mut insts = vec![inst(
        InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::Fresh { ty: CTy::Int(32) },
        },
        5,
    )];
    for (i, d) in offsets.iter().enumerate() {
        let n = 10 + i as u32;
        insts.push(inst(
            InstKind::Assign {
                dst: ValueId(100 + n),
                rv: RValue::Bin {
                    op: BinOp::Sub,
                    ty: CTy::Int(32),
                    a: Operand::Value(ValueId(0)),
                    b: k(*d),
                    signed: true,
                },
            },
            n,
        ));
        insts.push(inst(
            InstKind::Assign {
                dst: ValueId(200 + n),
                rv: RValue::Bin {
                    op: BinOp::SDiv,
                    ty: CTy::Int(32),
                    a: k(100),
                    b: Operand::Value(ValueId(100 + n)),
                    signed: true,
                },
            },
            // Distinct spans, so 023 §6.1's key does not merge the two divisions.
            n,
        ));
    }
    insts.extend(extra);
    module(vec![
        block(0, insts, tail),
        block(1, vec![], Terminator::Return(Some(k(1)))),
        block(2, vec![], Terminator::Return(Some(k(2)))),
    ])
}

/// **Two findings that need different inputs still leave a witness behind.**
///
/// `100/(x-1)` and `100/(x-2)` on one path each require a different `x`, so the conjunction
/// of what the findings need is unsatisfiable. The witness is one per *state*, so there is
/// no assignment that reproduces both — and the honest answer is the one the path alone
/// supports, not `unwitnessed`.
///
/// Dropping the fallback turns a solvable question into a refusal: the run would report two
/// real divisions by zero and say it could not name an input for either, which is a worse
/// answer than the imperfect one. (The complete fix is a witness *per finding*; §9 carries
/// it.)
#[test]
fn contradictory_requirements_fall_back_rather_than_refuse() {
    let m = divisions_at(&[1, 2], Terminator::Return(Some(k(0))), Vec::new());
    let f = witnessed(&m);
    assert_eq!(f.len(), 2, "both divisions are reported: {f:?}");
    for (msg, w) in &f {
        assert!(
            w.is_some(),
            "`{msg}`: the two findings need different inputs, which is a reason to report \
             the weaker witness and not a reason to report none"
        );
    }
}

/// **What a witness needs is not a constraint on the run.**
///
/// The condition is recorded for the *witness*, not pushed onto the path condition. Pushing
/// it would make every later feasibility query treat `x == 42` as given — so a branch on
/// `x == 0` after the division would be refuted, and the run would explore one path where
/// the program has two. A checker's finding would have deleted half the program.
///
/// 023 contract 19 draws the same line for a checker's `Assume`, which *does* join the path
/// and degrades fidelity for it. This is the other side: a narrowing that changes only what
/// is reported changes no path and degrades nothing.
#[test]
fn a_witness_requirement_does_not_prune_the_run() {
    let branch = vec![inst(
        InstKind::Assign {
            dst: ValueId(9),
            rv: RValue::Cmp {
                op: CmpOp::Eq,
                ty: CTy::Int(32),
                a: Operand::Value(ValueId(0)),
                b: k(0),
            },
        },
        50,
    )];
    let m = divisions_at(
        &[42],
        Terminator::Br {
            cond: Operand::Value(ValueId(9)),
            t: BlockId(1),
            f: BlockId(2),
        },
        branch,
    );
    let mut a = TermArena::new();
    let mut e = Engine::new(&m);
    for c in chiero_check::default_checkers() {
        e = e.with_checker(c);
    }
    let r = e.run(&mut a);
    assert_eq!(
        r.states().len(),
        2,
        "`x == 0` is feasible: the division's witness requirement must not have been \
         asserted onto the path. Fidelity: {:?}",
        r.states().iter().map(|s| s.fidelity()).collect::<Vec<_>>()
    );
}

/// **Each finding gets a witness that reproduces *it*.**
///
/// `100/(x-1)` and `100/(x-2)` on one path are two real divisions by zero needing two
/// different inputs. Wave 158 gave the *state* one witness solved against everything its
/// findings need; here that conjunction is unsatisfiable, so it falls back to the path
/// alone and both findings are handed the same number — which reproduces neither.
///
/// 023 §9 is about the finding, not the path: "a non-reproducible bug report is not a bug
/// report" is a claim about each report a reader is shown. Two reports sharing one witness
/// is one report's worth of evidence presented twice.
///
/// The fallback wave 158 added is still right for what it does — reporting the weaker
/// witness beats refusing — but it is a *consequence* of one witness per state, not a
/// design. This is the design: the condition that motivated a report travels with the
/// report, and each is solved on its own.
#[test]
fn contradictory_requirements_get_a_witness_each() {
    let m = divisions_at(&[1, 2], Terminator::Return(Some(k(0))), Vec::new());
    let f = witnessed(&m);
    assert_eq!(f.len(), 2, "both divisions are reported: {f:?}");

    // The findings are in report order, which is instruction order: `x - 1` then `x - 2`.
    for (want, (msg, w)) in [1i32, 2].into_iter().zip(f.iter()) {
        let w = w
            .as_ref()
            .unwrap_or_else(|| panic!("`{msg}` came with no witness"));
        let x = w.bindings[0].value as u32 as i32;
        assert_eq!(
            x,
            want,
            "`{msg}`: the witness names x = {x}, under which this divisor is {} — the \
             other finding's requirement, or neither's",
            x - want
        );
        assert!(
            w.bindings[0].pinned,
            "`{msg}`: this finding needs exactly x = {want}"
        );
    }
}

/// **A float-to-integer overflow reaches a finding, like the other three kinds.**
///
/// The engine records the event where the conversion happens (a `Cast` never passes through
/// `note_ub`, which is driven from `RValue::Bin`). This is the other half: the checker names
/// it, so a caller of `default_checkers` is told.
///
/// It is the case a reader asked about — whether undefined behaviour should be *warned*
/// about rather than only discarded from the differential channel — and the one kind of the
/// four for which the answer was no.
#[test]
fn a_float_cast_overflow_is_reported() {
    let m = module(vec![block(
        0,
        vec![inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Cast {
                    kind: CastKind::FpToSi,
                    to: CTy::Int(16),
                    from: CTy::Float(FloatKind::F64),
                    a: Operand::Const(Const::Float(
                        FloatKind::F64,
                        (-4_294_905_087.0f64).to_bits(),
                    )),
                },
            },
            10,
        )],
        Terminator::Return(Some(k(0))),
    )]);
    let f = findings(&m);
    assert_eq!(f.len(), 1, "one conversion, one finding: {f:?}");
    assert!(
        f[0].contains("float-to-integer conversion out of range"),
        "the finding should name what went wrong: {f:?}"
    );

    // And an in-range conversion is ordinary C, reported by nobody.
    let ok = module(vec![block(
        0,
        vec![inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::Cast {
                    kind: CastKind::FpToSi,
                    to: CTy::Int(32),
                    from: CTy::Float(FloatKind::F64),
                    a: Operand::Const(Const::Float(FloatKind::F64, 2.7f64.to_bits())),
                },
            },
            10,
        )],
        Terminator::Return(Some(k(0))),
    )]);
    assert!(
        findings(&ok).is_empty(),
        "2.7 fits in an int: {:?}",
        findings(&ok)
    );
}
