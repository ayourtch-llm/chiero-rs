//! Covers: **022 §4** — "Backend selection order: `$CHIERO_SMT_SOLVER`, then `z3`, `cvc5`,
//! `bitwuzla` on `PATH`. **Recorded in the result so a finding says which solver decided
//! it.**"
//!
//! Half of that sentence is implemented: `SmtLib::discover` honours the variable and walks
//! the order. The other half greps to nothing — a `RunResult` records `solver_calls`,
//! `backend_spawns` and `solver_inits`, and no name.
//!
//! It is the same shape as 023 contract 7's seed, and `strategy.rs` states the reason
//! better than this file could: "a seed recorded but ignored is reproducible and useless; an
//! order that changes with the seed but does not record it is useful and unrepeatable". A
//! verdict that depends on which solver answered, and does not say which answered, is the
//! second kind.
//!
//! Wave 161 is what makes this urgent rather than tidy. Before it a caller chose the backend
//! explicitly and therefore knew; now discovery is the default, so the *only* record of what
//! decided a finding would be the result — and `backend_spawns == 0` cannot distinguish
//! "tier 1 was enough" from "no solver was installed". 022 §4 puts both clauses in one
//! sentence for that reason.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn k(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
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

/// A symbolic branch, so the run actually asks the solver something.
fn branching() -> Module {
    Module {
        funcs: vec![Function {
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
                            5,
                        ),
                        inst(
                            InstKind::Assign {
                                dst: ValueId(1),
                                rv: RValue::Cmp {
                                    op: CmpOp::SLt,
                                    ty: CTy::Int(32),
                                    a: Operand::Value(ValueId(0)),
                                    b: k(10),
                                },
                            },
                            6,
                        ),
                    ],
                    Terminator::Br {
                        cond: Operand::Value(ValueId(1)),
                        t: BlockId(1),
                        f: BlockId(2),
                    },
                ),
                block(1, vec![], Terminator::Return(Some(k(1)))),
                block(2, vec![], Terminator::Return(Some(k(2)))),
            ],
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

/// **The result says which solver decided it**, and says so whichever one did.
///
/// Two configurations, because "records a name" is satisfied by a constant. `LiteOnly` must
/// name tier 1 rather than go quiet: a reader who cannot tell "no backend was used" from
/// "nobody wrote this down" learns nothing from either.
#[test]
fn the_result_records_which_solver_decided() {
    let m = branching();

    let mut a = TermArena::new();
    let lite = Engine::new(&m)
        .with_solver(SolverTier::LiteOnly)
        .run(&mut a);
    assert_eq!(
        lite.solver(),
        "solver-lite",
        "tier 1 answered, and the result should say so rather than leave it blank"
    );

    let Some(b) = chiero_solver::SmtLib::discover() else {
        eprintln!("SKIP: no SMT-LIB backend on PATH, so there is no second configuration");
        return;
    };
    let name = b.name().to_string();
    // **A name, not a path.** Comparing the result against `b.name()` alone is
    // self-consistent: both move together if `name()` starts returning the full path, and a
    // mutation doing exactly that survived. Two machines running the same solver from
    // different prefixes must produce the same string, or a finding compared across them
    // reads as a disagreement about the solver rather than about the program.
    assert!(
        !name.contains(std::path::MAIN_SEPARATOR),
        "the recorded solver should be a name like `z3`, not a path: {name:?}"
    );
    let mut a = TermArena::new();
    let full = Engine::new(&m).with_backend(b).run(&mut a);
    assert_eq!(
        full.solver(),
        name,
        "a backend was named and used, so the result should name it"
    );
    assert_ne!(
        full.solver(),
        lite.solver(),
        "the two configurations were decided by different solvers and must not read alike"
    );
}

/// **And a finding says it too**, which is the clause's stated purpose.
///
/// "Recorded in the result *so a finding says which solver decided it*" — the record exists
/// for the report. A reader looking at one finding should not have to go back to the run to
/// learn what stands behind it, and an upstream solver bug gets reported from a finding.
#[test]
fn a_finding_names_the_solver_behind_it() {
    let Some(b) = chiero_solver::SmtLib::discover() else {
        eprintln!("SKIP: no SMT-LIB backend on PATH");
        return;
    };
    let name = b.name().to_string();
    let mut m = branching();
    m.funcs[0].blocks[1].insts.push(inst(
        InstKind::Store {
            addr: Operand::Const(Const::Null),
            val: k(1),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
        30,
    ));
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(b).run(&mut a);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    assert_eq!(
        f.solver, name,
        "the finding should name what decided the path it is on"
    );
}
