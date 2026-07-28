//! Lazy objects and aliasing — 021 contract 18.
//!
//! Covers: 021 contract 18.
//!
//! §6: "two lazily-materialized objects are **distinct** by default. This is an
//! assumption, not a fact, so it is recorded in `Result::assumptions` and printed in every
//! report. `--fork-on-alias` forks the alias case; it is off by default because it
//! multiplies states by `2^(pairs)`."
//!
//! The recording is the load-bearing half. Assuming two pointer parameters do not alias is
//! how essentially every under-constrained run stays tractable, and it is also how a real
//! aliasing bug goes unseen — so a reader has to be told, every time, that the run rested
//! on it.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: Default::default(),
        span: Span::DUMMY,
    }
}

/// `int f(char *p, char *q) { *p = 1; return *q; }` — the classic aliasing question: is
/// what `q` reads the 1 that was just written through `p`, or not?
fn two_pointer_params() -> Module {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![
            Param {
                value: ValueId(0),
                ty: CTy::Ptr,
            },
            Param {
                value: ValueId(1),
                ty: CTy::Ptr,
            },
        ],
        // Returns the byte it loaded, so the declared type is the loaded type — a
        // mismatch does not verify, and a module that does not verify reports the
        // absence of everything.
        ret: CTy::Int(8),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![
                Inst {
                    kind: InstKind::Store {
                        addr: Operand::Value(ValueId(0)),
                        // 8-bit value for an 8-bit store: a width mismatch does not
                        // verify, and a module that does not verify reports the absence
                        // of everything.
                        val: Operand::Const(Const::Int { bits: 8, val: 1 }),
                        ty: CTy::Int(8),
                        align: 1,
                        vol: Volatility::Normal,
                    },
                    span: Span::DUMMY,
                },
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::Load {
                            addr: Operand::Value(ValueId(1)),
                            ty: CTy::Int(8),
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    span: Span::DUMMY,
                },
            ],
            Terminator::Return(Some(Operand::Value(ValueId(2)))),
        )],
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

/// **021 contract 18, the default.** Two lazily-materialized pointer parameters are
/// distinct objects — and the run *says so*, because it is an assumption and not a fact.
#[test]
fn two_pointer_parameters_are_distinct_and_the_run_records_the_assumption() {
    let m = two_pointer_params();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_entry_param_bytes(4).run(&mut a);
    assert_eq!(r.states().len(), 1, "no fork by default");

    let s = &r.states()[0];
    let (Some(Value::Ptr(p)), Some(Value::Ptr(q))) = (s.local(ValueId(0)), s.local(ValueId(1)))
    else {
        panic!("both parameters are pointers");
    };
    assert_ne!(p.base, q.base, "distinct by default");

    assert!(
        s.assumptions()
            .iter()
            .any(|x| x.detail.contains("alias") || x.detail.contains("distinct")),
        "and the run says it assumed that: {:#?}",
        s.assumptions()
            .iter()
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
    // **And the run cannot claim to be exact.** Assuming two pointers do not alias is
    // discarding a case the program allows — 023 §7's `Approximated`, "keeping one of
    // several feasible values" — so a run that made it must not seal as a proof. An
    // assumption recorded without the fidelity to match is one `seal` would step over.
    assert_eq!(
        r.fidelity(),
        Fidelity::Approximated,
        "the distinctness assumption is a modelling choice, not a fact: {:#?}",
        s.assumptions()
            .iter()
            .map(|x| (&x.kind, &x.detail))
            .collect::<Vec<_>>()
    );
    assert!(
        seal(&r, r.witness()).is_err(),
        "and it cannot be presented as a proof"
    );
    // Printed, too: §6 says "printed in every report", and an assumption a reader cannot
    // see is one they cannot weigh.
    let text = render(&r);
    assert!(
        text.contains("alias") || text.contains("distinct"),
        "{text}"
    );
}

/// **021 contract 18, the other mode.** Under `--fork-on-alias` the alias state exists,
/// and the two modes' assumptions differ — otherwise the flag would change what is
/// explored without changing what is claimed.
#[test]
fn fork_on_alias_explores_the_case_the_default_assumes_away() {
    let m = two_pointer_params();
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_entry_param_bytes(4)
        .with_fork_on_alias(true)
        .run(&mut a);
    assert!(
        r.states().len() >= 2,
        "the alias case is its own state: {}",
        r.states().len()
    );
    // One state has them aliased, one does not.
    let mut aliased = 0;
    let mut distinct = 0;
    for s in r.states() {
        let (Some(Value::Ptr(p)), Some(Value::Ptr(q))) = (s.local(ValueId(0)), s.local(ValueId(1)))
        else {
            continue;
        };
        if p.base == q.base {
            aliased += 1;
        } else {
            distinct += 1;
        }
    }
    assert_eq!((aliased, distinct), (1, 1), "one of each");

    // And the *claims* differ: the distinctness assumption is not made where the alias
    // case was explored instead.
    let mut a2 = TermArena::new();
    let default_run = Engine::new(&m).with_entry_param_bytes(4).run(&mut a2);
    let default_says: Vec<String> = default_run
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    let forked_says: Vec<String> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    assert_ne!(
        default_says, forked_says,
        "the two modes must not make the same claims: {default_says:?}"
    );
}
