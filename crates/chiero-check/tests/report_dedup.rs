//! **Who deduplicates a checker's reports.**
//!
//! 023 §6.1 gives the engine a dedup key — `(kind, span, object, func)` — and `report_faults`
//! builds one for every memory fault. Both checker routes pass `key: None`, so a checker's reports
//! are exempt: `reports()` merges a *fork's* copies of one report by id and leaves everything else
//! alone.
//!
//! §9 carried that as an open decision for ten waves: give `Action::Report` a key, or write down
//! that checkers own their deduplication — **not both**, since each makes the other redundant or
//! correct.
//!
//! # The codebase had already answered it
//!
//! `undefined_arithmetic.rs`'s `one_faulting_site_in_a_loop_is_one_finding` explains, in its own doc
//! comment, *why* `UbState` carries a `reported` list: "`Action::Report` carries no §6.1 key, so
//! `reports()` deduplicates a fork's copies by id and leaves everything else to the checker — which
//! is why the checker needs per-state memory rather than a counter."
//!
//! That is the design stated as a rationale for existing code, so this wave follows it rather than
//! reversing it. It is also the better answer on the merits: a checker may legitimately report twice
//! at one span — two distinct facts about one instruction — and the engine cannot know which of
//! those is a duplicate. The checker can, because it knows what it was looking for.
//!
//! # What was missing
//!
//! The contract was recorded in a *test's* doc comment about a different subject, where nobody
//! writing a checker would find it, and **nothing anywhere asserted it**. Centralizing dedup later
//! would have changed every checker's output with no test objecting.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn module() -> Module {
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks: vec![Block {
                id: BlockId(0),
                insts: vec![Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::Bin {
                            op: BinOp::Add,
                            ty: CTy::Int(32),
                            a: Operand::Const(Const::Int { bits: 32, val: 1 }),
                            b: Operand::Const(Const::Int { bits: 32, val: 1 }),
                            signed: true,
                        },
                    },
                    span: at(10),
                    generated: false,
                }],
                term: Terminator::Return(Some(Operand::Value(ValueId(0)))),
                gcov_lines: Default::default(),
                span: at(1),
            }],
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

/// A checker that says the same thing twice about one instruction.
///
/// Deliberately naive: it is what a checker written without its own memory does, and the point is
/// that the engine does not quietly rescue it.
struct SaysItTwice;

impl Checker for SaysItTwice {
    fn name(&self) -> &'static str {
        "says-it-twice"
    }
    fn on_event(&mut self, ev: &Event, _cx: &mut CheckerCtx) -> Vec<Action> {
        match ev {
            Event::AfterInst { .. } => vec![
                Action::report("duplicated: the same fact, twice"),
                Action::report("duplicated: the same fact, twice"),
            ],
            _ => vec![],
        }
    }
}

fn run_with<C: Checker + 'static>(c: C) -> Vec<String> {
    let m = module();
    let mut a = TermArena::new();
    Engine::new(&m)
        .with_checker(Box::new(c))
        .run(&mut a)
        .findings()
}

/// **The engine does not merge a checker's reports.**
///
/// Two identical reports from one checker at one instruction are two findings. That is the contract
/// this wave settles: dedup is the checker's, because only the checker knows whether two facts about
/// one instruction are the same fact.
#[test]
fn the_engine_leaves_a_checker_s_reports_alone() {
    let f = run_with(SaysItTwice);
    let n = f.iter().filter(|s| s.starts_with("duplicated")).count();
    assert_eq!(
        n, 2,
        "the engine has no §6.1 key for a checker report and must not invent one: {f:?}"
    );
}

/// A checker with per-state memory gets one. **The other half of the contract.**
///
/// Without this the test above reads as "duplicates are fine"; together they say where the
/// responsibility lives. `UndefinedArithmetic` does exactly this with `UbState.reported`, and
/// `one_faulting_site_in_a_loop_is_one_finding` pins it for the real checker — this is the same
/// claim with the mechanism visible in ten lines.
#[test]
fn a_checker_that_remembers_reports_once() {
    #[derive(Default)]
    struct Once {
        said: bool,
    }
    impl CheckerState for Once {
        fn on_fork(&self) -> Box<dyn CheckerState> {
            Box::new(Once { said: self.said })
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
    struct SaysItOnce;
    impl Checker for SaysItOnce {
        fn name(&self) -> &'static str {
            "says-it-once"
        }
        fn initial_state(&self) -> Box<dyn CheckerState> {
            Box::new(Once::default())
        }
        fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
            match ev {
                Event::AfterInst { .. } => {
                    let st = cx.state_mut::<Once>();
                    if st.said {
                        return vec![];
                    }
                    st.said = true;
                    vec![
                        Action::report("deduped: said once"),
                        Action::report("deduped: said once"),
                    ]
                }
                _ => vec![],
            }
        }
    }
    let f = run_with(SaysItOnce);
    let n = f.iter().filter(|s| s.starts_with("deduped")).count();
    assert_eq!(
        n, 2,
        "the memory suppresses the second *event*, not two reports from one event: {f:?}"
    );
}
