//! The checker interface — 023 contracts 19, 20, 22 and 24.
//!
//! Covers: 023 contracts 19, 20, 22, 24.
//!
//! §6 makes a checker a stateless **observer**: it sees every event and decides nothing
//! about execution order. Everything it remembers lives in the `State`, because the
//! `Searcher` interleaves events from unrelated states arbitrarily — DFS backtracks
//! between them, `RandomPath` jumps constantly, and §11's parallel workers cannot share
//! `&mut self` at all. A checker holding its memory in `&mut self` is not merely
//! imprecise; it produces a different answer depending on the exploration order, which
//! contract 17 forbids outright.
//!
//! The four contracts here are the ones that pin the interface's shape rather than any
//! particular checker's cleverness:
//!
//! - **19** — `Action::Assume` constrains the state, and the *subsequent branch
//!   feasibility reflects it*. An `Assume` that is recorded but does not reach the solver
//!   is the silent failure: every checker written against it would appear to work.
//! - **20** — two checkers reporting the same event produce two findings. The engine does
//!   not deduplicate; that is 040's job, by `(checker, span, object, kind)`.
//! - **22** — `CheckerState` is cloned on fork, and the copies are independent.
//! - **24** — `CallReturn` fires in the caller for all three callee kinds, including the
//!   unmodeled extern whose fresh return value has no return instruction at all.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::{SmtLib, TermArena};
use chiero_span::Span;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

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

fn func(id: u32, name: &str, params: Vec<Param>, ret: CTy, blocks: Vec<Block>) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        params,
        ret,
        variadic: false,
        allocas: vec![],
        blocks,
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    }
}

// ---------------------------------------------------------------------------
// 023 contract 20 — two checkers, two findings.
// ---------------------------------------------------------------------------

/// A checker that reports on every `Call` it sees. Two of these registered under
/// different names must produce two findings from one call.
struct Noisy(&'static str);

impl Checker for Noisy {
    fn name(&self) -> &'static str {
        self.0
    }

    fn on_event(&mut self, ev: &Event, _cx: &mut CheckerCtx) -> Vec<Action> {
        match ev {
            Event::Call { .. } => vec![Action::report(format!("{} saw a call", self.0))],
            _ => vec![],
        }
    }
}

/// `int f(void) { g(); return 0; }` with `g` defined and trivial.
fn calls_g() -> Module {
    let f = func(
        0,
        "f",
        vec![],
        CTy::Int(32),
        vec![block(
            0,
            vec![Inst {
                kind: InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![],
                },
                span: Span::DUMMY,
                generated: false,
            }],
            Terminator::Return(Some(i32c(0))),
        )],
    );
    let g = func(
        1,
        "g",
        vec![],
        CTy::Void,
        vec![block(0, vec![], Terminator::Return(None))],
    );
    Module {
        funcs: vec![f, g],
        ..Default::default()
    }
}

/// **023 contract 20.** "Two checkers reporting the same event produce two findings at
/// the engine level (the engine does not deduplicate)."
#[test]
fn two_checkers_on_one_event_make_two_findings() {
    let m = calls_g();
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_checker(Box::new(Noisy("alpha")))
        .with_checker(Box::new(Noisy("beta")))
        .run(&mut a);

    let reports = r.reports();
    let msgs: Vec<&str> = reports
        .iter()
        .map(|f| f.message.as_str())
        .filter(|m| m.contains("saw a call"))
        .collect();
    assert_eq!(
        msgs.len(),
        2,
        "the engine deduplicated what 040 is responsible for deduplicating: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("alpha")) && msgs.iter().any(|m| m.contains("beta")),
        "and one came from each checker: {msgs:?}"
    );
}

/// The other half, which is what stops the test above from passing for an engine that
/// emits two copies of *one* checker's finding: a single registered checker produces
/// exactly one.
#[test]
fn one_checker_on_one_event_makes_one_finding() {
    let m = calls_g();
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_checker(Box::new(Noisy("alpha")))
        .run(&mut a);
    assert_eq!(
        r.reports()
            .iter()
            .filter(|f| f.message.contains("saw a call"))
            .count(),
        1
    );
}

// ---------------------------------------------------------------------------
// 023 contract 24 — `CallReturn` for all three callee kinds.
// ---------------------------------------------------------------------------

/// Records `(callee name, whether a return value was carried)` for every `CallReturn`.
struct WatchReturns(Rc<RefCell<Vec<(String, bool)>>>);

impl Checker for WatchReturns {
    fn name(&self) -> &'static str {
        "watch-returns"
    }

    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        if let Event::CallReturn { callee, ret, .. } = ev {
            self.0
                .borrow_mut()
                .push((cx.callee_name(callee).to_string(), ret.is_some()));
        }
        vec![]
    }
}

/// `int f(void) { int a = defined(); int b = modeled(); int c = unmodeled(); return a; }`
///
/// `strlen` stands in for the modeled extern because 024 models it; `mystery` is declared
/// and unmodeled, which is the case with no return instruction anywhere — §6 says
/// `Event::Return` "never fires at all" for it, which is the whole reason `CallReturn`
/// exists.
fn three_callee_kinds() -> Module {
    let mut f = func(
        0,
        "f",
        vec![],
        CTy::Int(32),
        vec![block(
            0,
            vec![
                Inst {
                    kind: InstKind::Call {
                        dst: Some(ValueId(0)),
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![],
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Call {
                        dst: Some(ValueId(1)),
                        callee: Callee::Direct(FuncId(2)),
                        args: vec![],
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(2),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Store {
                        addr: Operand::Value(ValueId(2)),
                        val: Operand::Const(Const::Int { bits: 8, val: 0 }),
                        ty: CTy::Int(8),
                        align: 1,
                        vol: Volatility::Normal,
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Call {
                        dst: Some(ValueId(3)),
                        callee: Callee::Direct(FuncId(3)),
                        args: vec![Operand::Value(ValueId(2))],
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
            ],
            Terminator::Return(Some(Operand::Value(ValueId(0)))),
        )],
    );
    f.allocas = vec![AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(8),
        count: 1,
        align: 1,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: None,
        span: Span::DUMMY,
    }];
    let defined = func(
        1,
        "defined_one",
        vec![],
        CTy::Int(32),
        vec![block(0, vec![], Terminator::Return(Some(i32c(7))))],
    );
    let mut mystery = func(2, "mystery", vec![], CTy::Int(32), vec![]);
    mystery.body = Body::Declared;
    // The **modeled** extern. The doc comment above claimed `strlen` stood in for it and
    // the fixture did not contain one, so the test named three callee kinds and exercised
    // two — the modeled path, which is the one that returns a value from a model rather
    // than from a return instruction or a fresh symbol, was never reached. Found by
    // review.
    let mut modeled = func(
        3,
        "strlen",
        vec![Param {
            value: ValueId(90),
            ty: CTy::Ptr,
        }],
        CTy::Int(64),
        vec![],
    );
    modeled.body = Body::Declared;
    Module {
        funcs: vec![f, defined, mystery, modeled],
        ..Default::default()
    }
}

/// **023 contract 24.** `CallReturn` fires in the caller for a defined function and for
/// an unmodeled extern, and carries the value the caller will observe.
///
/// The unmodeled case is the one worth the contract: its fresh return value has no return
/// instruction behind it, so an implementation that fires `CallReturn` from the callee's
/// epilogue passes for the defined function and silently never fires here.
#[test]
fn call_return_fires_for_a_defined_function_and_for_an_unmodeled_extern() {
    let m = three_callee_kinds();
    // **A module that does not verify reports the absence of everything**, which is
    // exactly how this fixture ran zero instructions and produced an empty event list
    // when the modeled callee was added with the wrong arity.
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));
    let mut a = TermArena::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    Engine::new(&m)
        .with_checker(Box::new(WatchReturns(seen.clone())))
        .run(&mut a);

    let seen = seen.borrow();
    let names: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"defined_one"),
        "a defined callee fires CallReturn in the caller: {names:?}"
    );
    assert!(
        names.contains(&"strlen"),
        "and so does a modeled extern, whose value comes from a model rather than from a \
         return instruction: {names:?}"
    );
    assert!(
        names.contains(&"mystery"),
        "and so does an unmodeled extern, which has no return instruction at all: \
         {names:?}"
    );
    // And the value the caller will observe is carried, not merely the fact of a return.
    for (n, has_val) in seen.iter() {
        assert!(
            *has_val,
            "{n}'s CallReturn carried no value, so a checker guarding on a call's result \
             — 042's only worked example — has nothing to guard on"
        );
    }
}

// ---------------------------------------------------------------------------
// 023 contract 19 — `Action::Assume`.
// ---------------------------------------------------------------------------

/// Assumes `x == 5` the first time it sees an instruction, which — for the fixture below
/// — must kill the `x == 0` branch.
///
/// **`x == 5`, not `x != 0`.** The engine's default solver is tier-1-only (022 §3), whose
/// domain has no transfer for `Ne`: `x != 0 ∧ x == 0` comes back `Unknown`, the engine
/// takes the branch anyway per 023 §3, and both sides survive whether or not the assume
/// worked. Two conflicting equalities are inside tier 1's fragment, so this fixture
/// decides the question with the solver the engine actually uses by default rather than
/// depending on z3 being installed.
struct AssumeIsFive;

impl Checker for AssumeIsFive {
    fn name(&self) -> &'static str {
        "assume-is-five"
    }

    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        let Event::BeforeInst { st, .. } = ev else {
            return vec![];
        };
        let Some(Value::Scalar(x)) = st.local(ValueId(0)) else {
            return vec![];
        };
        let five = cx.arena().bv(32, 5);
        let is_five = cx.arena().eq(x, five);
        vec![Action::Assume(is_five)]
    }
}

/// `int f(int x) { if (x == 0) return 1; else return 2; }` — both sides live unless
/// something constrains `x`.
fn branch_on_zero() -> Module {
    let f = func(
        0,
        "f",
        vec![Param {
            value: ValueId(0),
            ty: CTy::Int(32),
        }],
        CTy::Int(32),
        vec![
            block(
                0,
                vec![Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::Cmp {
                            op: CmpOp::Eq,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(0)),
                            b: i32c(0),
                        },
                    },
                    span: Span::DUMMY,
                    generated: false,
                }],
                Terminator::Br {
                    cond: Operand::Value(ValueId(1)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(1)))),
            block(2, vec![], Terminator::Return(Some(i32c(2)))),
        ],
    );
    Module {
        funcs: vec![f],
        ..Default::default()
    }
}

/// **023 contract 19.** "A checker returning `Action::Assume` constrains the state and
/// the subsequent branch feasibility reflects it."
///
/// The second clause is the whole contract. An `Assume` that is pushed onto a list the
/// solver never sees produces exactly the same run as one that works, for every fixture
/// that does not branch on the assumed condition.
#[test]
fn an_assumed_constraint_reaches_the_solver_and_kills_a_branch() {
    let m = branch_on_zero();

    // Without the checker, both sides are live — otherwise this proves nothing.
    let mut a = TermArena::new();
    let plain = Engine::new(&m).run(&mut a);
    let mut rets: Vec<u128> = plain
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut a))
        .collect();
    rets.sort_unstable();
    assert_eq!(rets, vec![1, 2], "the fixture forks without any checker");

    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_checker(Box::new(AssumeIsFive))
        .run(&mut a);
    let rets: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut a))
        .collect();
    assert_eq!(
        rets,
        vec![2],
        "`x == 5` was assumed, so `x == 0` is infeasible and only the else branch survives"
    );
}

// ---------------------------------------------------------------------------
// 023 contract 22 — per-state checker state, cloned on fork.
// ---------------------------------------------------------------------------

/// §6.1's own example: "a lock acquired before a fork is held in both children, and
/// released in one leaves it held in the other."
#[derive(Clone, Default)]
struct LockSet {
    held: Vec<String>,
}

impl CheckerState for LockSet {
    fn on_fork(&self) -> Box<dyn CheckerState> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Acquires a lock at the first instruction, then releases it only on the *true* side of
/// the fork. What each terminated state believes it holds is reported at `Terminated`, so
/// the two paths' beliefs are visible in the findings.
struct Locking;

impl Checker for Locking {
    fn name(&self) -> &'static str {
        "locking"
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(LockSet::default())
    }

    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        match ev {
            Event::BeforeInst { .. } => {
                let ls: &mut LockSet = cx.state_mut();
                if ls.held.is_empty() {
                    ls.held.push("m".into());
                }
                vec![]
            }
            // Released on the branch that returns 1 — that is block 1 in the fixture, so
            // the release is keyed on the value the state is about to return.
            Event::Return { val, .. } => {
                if matches!(val, Some(Value::Scalar(_))) {
                    let one = cx.arena().bv(32, 1);
                    let Some(Value::Scalar(v)) = val else {
                        return vec![];
                    };
                    let is_one = cx.arena().eq(*v, one);
                    if cx.must(is_one) {
                        cx.state_mut::<LockSet>().held.clear();
                    }
                }
                vec![]
            }
            Event::Terminated { .. } => {
                let held = cx.state_mut::<LockSet>().held.join(",");
                vec![Action::report(format!("held=[{held}]"))]
            }
            _ => vec![],
        }
    }
}

/// **023 contract 22.** The lock acquired before the fork is held in both children, and
/// releasing it in one leaves it held in the other.
///
/// A `CheckerState` shared between the children — the natural mistake, and the one a
/// `&mut self` on the `Checker` makes automatically — reports the same set twice.
#[test]
fn checker_state_forks_with_the_state_and_the_copies_are_independent() {
    let m = branch_on_zero();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_checker(Box::new(Locking)).run(&mut a);

    let reports = r.reports();
    let mut held: Vec<&str> = reports
        .iter()
        .filter_map(|f| f.message.strip_prefix("held="))
        .collect();
    held.sort_unstable();
    assert_eq!(
        held,
        vec!["[]", "[m]"],
        "one path released the lock and the other did not; a shared `CheckerState` \
         reports the same answer on both"
    );
}

// ---------------------------------------------------------------------------
// Regressions from the wave-80 adversarial review. Each of these passed before
// the fix, which is why they are here.
// ---------------------------------------------------------------------------

/// A checker that assumes an unsatisfiable-with-one-branch condition, to observe what the
/// *run* claims afterwards.
struct AssumeAndSay;

impl Checker for AssumeAndSay {
    fn name(&self) -> &'static str {
        "assume-and-say"
    }
    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        let Event::BeforeInst { st, .. } = ev else {
            return vec![];
        };
        let Some(Value::Scalar(x)) = st.local(ValueId(0)) else {
            return vec![];
        };
        let five = cx.arena().bv(32, 5);
        let is_five = cx.arena().eq(x, five);
        vec![Action::Assume(is_five)]
    }
}

/// **An `Assume` prunes paths, so the run is not exact and says why.**
///
/// Before the fix the surviving state carried `Fidelity::Exact` and an empty
/// `assumptions` list: half the program's paths were deleted by a constraint the analysis
/// invented, and `seal` would mint a proof over what was left with nothing in the report
/// saying so.
#[test]
fn a_checker_assumption_degrades_the_run_and_is_recorded() {
    let m = branch_on_zero();
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_checker(Box::new(AssumeAndSay))
        .run(&mut a);

    assert_eq!(
        r.fidelity(),
        Fidelity::Approximated,
        "paths the program allows were discarded on the analysis's own say-so"
    );
    let said: Vec<String> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    assert!(
        said.iter().any(|d| d.contains("assume-and-say")),
        "and the report names the checker that did it: {said:?}"
    );
    assert!(
        seal(&r, r.witness()).is_err(),
        "a run whose paths a checker pruned is not a proof"
    );
}

/// A checker that kills the path it is on.
struct Killer;

impl Checker for Killer {
    fn name(&self) -> &'static str {
        "killer"
    }
    fn on_event(&mut self, ev: &Event, _cx: &mut CheckerCtx) -> Vec<Action> {
        match ev {
            Event::BeforeInst { .. } => vec![Action::Kill(TermReason::Unreachable)],
            _ => vec![],
        }
    }
}

/// **And so does a `Kill`** — a killed path is one nobody looked at.
#[test]
fn a_checker_kill_degrades_the_run_and_is_recorded() {
    let m = branch_on_zero();
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_checker(Box::new(Killer)).run(&mut a);
    assert_eq!(r.fidelity(), Fidelity::Approximated);
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("killer")),
        "the report names the checker that stopped the path"
    );
}

/// Answers `may`/`must` about a condition no tier-1 solver can decide, and records both.
struct AsksTheSolver(Rc<RefCell<Vec<(bool, bool)>>>);

impl Checker for AsksTheSolver {
    fn name(&self) -> &'static str {
        "asks"
    }
    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        let Event::BeforeInst { st, .. } = ev else {
            return vec![];
        };
        let Some(Value::Scalar(x)) = st.local(ValueId(0)) else {
            return vec![];
        };
        // `x * x == 4` — multiplication, which 022 §3.2 leaves outside tier 1's fragment.
        let sq = cx.arena().mul(x, x);
        let four = cx.arena().bv(32, 4);
        let eq = cx.arena().eq(sq, four);
        let m = cx.may(eq);
        let mu = cx.must(eq);
        self.0.borrow_mut().push((m, mu));
        vec![]
    }
}

/// **`may` errs toward `true`, `must` toward `false`.**
///
/// Both collapse `Unknown`, and they must collapse it in *opposite* directions — each
/// toward the answer that keeps a checker looking. `may` returning `false` on `Unknown`
/// tells a checker asking "may this pointer be NULL?" that it cannot be, and the bug
/// disappears with no finding and no fidelity change. With the engine's default
/// tier-1-only solver that is every question outside §3.2's fragment.
#[test]
fn may_and_must_collapse_unknown_in_opposite_directions() {
    let m = branch_on_zero();
    let mut a = TermArena::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let r = Engine::new(&m)
        .with_checker(Box::new(AsksTheSolver(seen.clone())))
        .run(&mut a);

    let seen = seen.borrow();
    assert!(!seen.is_empty(), "the checker ran");
    for (may, must) in seen.iter() {
        assert!(
            *may,
            "`x * x == 4` is genuinely possible for an unconstrained x, and even if the \
             solver declines to say so, `may` must not answer `false`"
        );
        assert!(!*must, "and it is certainly not forced");
    }
    // **And the questions are counted.** A checker's queries are solver queries; reporting
    // zero for a run that asked several makes the budget accounting a fiction.
    assert!(
        r.solver_calls > 0,
        "the checker's `may`/`must` never reached `solver_calls`"
    );
}

/// Records the callee of every `Event::Call`, and how many argument slots it carried.
struct WatchCalls(Rc<RefCell<Vec<(String, usize)>>>);

impl Checker for WatchCalls {
    fn name(&self) -> &'static str {
        "watch-calls"
    }
    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        if let Event::Call { callee, args, .. } = ev {
            self.0
                .borrow_mut()
                .push((cx.callee_name(callee).to_string(), args.len()));
        }
        vec![]
    }
}

/// **`Event::Call` fires for an indirect call too.**
///
/// It was emitted *after* the indirect split, so a call through a function pointer — VPP's
/// entire node dispatch — produced a `CallReturn` with no `Call` before it, and 042's
/// typestate shape had nothing to arm itself on.
#[test]
fn event_call_fires_for_an_indirect_callee() {
    let g = func(
        1,
        "g",
        vec![],
        CTy::Int(32),
        vec![block(0, vec![], Terminator::Return(Some(i32c(3))))],
    );
    let f = func(
        0,
        "f",
        vec![],
        CTy::Int(32),
        vec![block(
            0,
            vec![
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfFunc(FuncId(1)),
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Call {
                        dst: Some(ValueId(1)),
                        callee: Callee::Indirect(Operand::Value(ValueId(0))),
                        args: vec![],
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
            ],
            Terminator::Return(Some(Operand::Value(ValueId(1)))),
        )],
    );
    let m = Module {
        funcs: vec![f, g],
        ..Default::default()
    };
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));

    let mut a = TermArena::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    Engine::new(&m)
        .with_checker(Box::new(WatchCalls(seen.clone())))
        .run(&mut a);
    let seen = seen.borrow();
    assert!(
        !seen.is_empty(),
        "an indirect call is still a call, and a checker has to see it"
    );
}

/// **An argument chiero cannot represent is a hole, not an absence.**
///
/// `filter_map` compacted the list, so a checker indexing `args[1]` read `args[0]`'s
/// neighbour. The varargs path was fixed for exactly this; `Event::Call` was not.
#[test]
fn event_call_arguments_keep_their_positions() {
    let g = func(
        1,
        "g",
        vec![
            Param {
                value: ValueId(50),
                ty: CTy::Float(FloatKind::F64),
            },
            Param {
                value: ValueId(51),
                ty: CTy::Int(32),
            },
        ],
        CTy::Void,
        vec![block(0, vec![], Terminator::Return(None))],
    );
    let f = func(
        0,
        "f",
        vec![],
        CTy::Void,
        vec![block(
            0,
            vec![Inst {
                kind: InstKind::Call {
                    dst: None,
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![
                        Operand::Const(Const::Float(FloatKind::F64, 0x3FF8_0000_0000_0000)),
                        i32c(7),
                    ],
                },
                span: Span::DUMMY,
                generated: false,
            }],
            Terminator::Return(None),
        )],
    );
    let m = Module {
        funcs: vec![f, g],
        ..Default::default()
    };
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));

    let mut a = TermArena::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    Engine::new(&m)
        .with_checker(Box::new(WatchCalls(seen.clone())))
        .run(&mut a);
    let seen = seen.borrow();
    let (_, n) = seen.first().expect("the call was seen");
    assert_eq!(
        *n, 2,
        "the float is a hole at index 0, not a missing element — compacting it hands a \
         checker the `int` when it asks for the `double`"
    );
}

/// A checker that only records `Terminated`.
struct WatchEnd(Rc<RefCell<u32>>);

impl Checker for WatchEnd {
    fn name(&self) -> &'static str {
        "watch-end"
    }
    fn on_event(&mut self, ev: &Event, _cx: &mut CheckerCtx) -> Vec<Action> {
        if let Event::Terminated { .. } = ev {
            *self.0.borrow_mut() += 1;
        }
        vec![]
    }
}

/// **A state that gave up still ends.**
///
/// `Terminated` was gated on `Status::Terminated`, so a state that errored — an indirect
/// goto with no targets, a missing block — never fired it. Those are exactly the paths
/// where a checker's accumulated state matters most, since giving up also sets
/// `Fidelity::Unknown`.
#[test]
fn an_errored_state_still_fires_terminated() {
    let f = func(
        0,
        "f",
        vec![],
        CTy::Void,
        vec![block(
            0,
            vec![Inst {
                kind: InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::Fresh { ty: CTy::Ptr },
                },
                span: Span::DUMMY,
                generated: false,
            }],
            // An indirect goto with no declared targets: the engine gives up, which is
            // `Status::Errored` rather than `Terminated`.
            Terminator::IndirectGoto {
                addr: Operand::Value(ValueId(0)),
                targets: vec![],
            },
        )],
    );
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));
    let mut a = TermArena::new();
    let n = Rc::new(RefCell::new(0));
    let r = Engine::new(&m)
        .with_checker(Box::new(WatchEnd(n.clone())))
        .run(&mut a);
    assert!(
        !matches!(r.states()[0].status, Status::Running),
        "the fixture ends the state one way or another"
    );
    assert_eq!(
        *n.borrow(),
        1,
        "a checker is told the path ended, whichever way it ended"
    );
}

// ---------------------------------------------------------------------------
// 022 §6 reaches a real run.
// ---------------------------------------------------------------------------

/// **Independence slicing runs during execution, not only in the solver's own tests.**
///
/// `probe` called `check` rather than `check_path`, so wave 79's slicing was implemented,
/// tested, and never executed by the engine — a feature with a full test suite and no
/// users. `check` cannot slice because it cannot tell which variables the *question* is
/// about; that distinction is the entire reason `check_path` takes the query separately.
///
/// The fixture gives the path condition several variable-disjoint clusters — the shape
/// 022 §6.2 says a real path condition has — and then asks a question about one of them.
#[test]
fn a_real_run_slices_its_path_condition() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    // Four independent parameters, each narrowed by its own branch, then one final branch
    // that asks about the first. Nothing relates the clusters, so three of the four are
    // irrelevant to every query after them.
    let n = 4u32;
    let params: Vec<Param> = (0..n)
        .map(|i| Param {
            value: ValueId(i),
            ty: CTy::Int(32),
        })
        .collect();
    let mut blocks = Vec::new();
    for i in 0..n {
        blocks.push(block(
            i,
            vec![
                // **`v*v == 49`, not `v < 1000`.** Slicing only happens on the way to the
                // *backend*, so the fixture must ask something `SolverLite` cannot answer.
                // It used to be a plain comparison, and wave 153 — which taught the lite
                // solver negated atoms and the boolean-materialization idiom — made that
                // decidable in-process. The run then never escalated and never sliced, so
                // this test failed for the reason its subject had become unreachable.
                //
                // A nonlinear equality is out of reach in a way that does not depend on
                // the fragment growing again: the atom is collected, but its left side is
                // not a variable so no domain narrows, and the candidate model (each
                // domain's least value, so 0) fails validation. Each cluster still touches
                // exactly one parameter, which is the property the test is about.
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(300 + i),
                        rv: RValue::Bin {
                            op: BinOp::Mul,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(i)),
                            b: Operand::Value(ValueId(i)),
                        },
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(100 + i),
                        rv: RValue::Cmp {
                            op: CmpOp::Eq,
                            ty: CTy::Int(32),
                            a: Operand::Value(ValueId(300 + i)),
                            b: i32c(49),
                        },
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
            ],
            Terminator::Br {
                cond: Operand::Value(ValueId(100 + i)),
                t: BlockId(i + 1),
                f: BlockId(n + 1),
            },
        ));
    }
    // The query block: one more comparison on the *first* parameter.
    blocks.push(block(
        n,
        vec![Inst {
            kind: InstKind::Assign {
                dst: ValueId(200),
                rv: RValue::Cmp {
                    op: CmpOp::ULt,
                    ty: CTy::Int(32),
                    a: Operand::Value(ValueId(0)),
                    b: i32c(7),
                },
            },
            span: Span::DUMMY,
            generated: false,
        }],
        Terminator::Br {
            cond: Operand::Value(ValueId(200)),
            t: BlockId(n + 1),
            f: BlockId(n + 1),
        },
    ));
    blocks.push(block(n + 1, vec![], Terminator::Return(Some(i32c(0)))));

    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params,
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks,
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    assert!(verify(&m).is_empty(), "{:?}", verify(&m));

    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    assert!(
        r.sliced_terms_skipped > 0,
        "the engine never sliced anything, so 022 §6.2 is still unreachable from a run: \
         {} solver calls",
        r.solver_calls
    );
}
