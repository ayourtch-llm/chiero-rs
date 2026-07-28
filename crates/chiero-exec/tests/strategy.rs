//! Covers: 023 contract 7 — the exploration strategy and its seed.
//!
//! 023 §4: "Every strategy is **deterministic**, including `RandomPath`: its PRNG is
//! seeded from a config value (default 0) that is recorded in every result. Ties break by
//! `StateId`. A non-reproducible bug report is not a bug report."
//!
//! That last sentence is the contract. A symbolic run finds bugs on some paths and not
//! others, and which paths it walked is a function of the strategy; if the strategy is not
//! reproducible then neither is the finding, and neither is its *absence*. Contract 7 asks
//! for the two halves that make it reproducible: changing the seed changes the order, and
//! the seed is in the result.
//!
//! Both halves are needed. A seed recorded but ignored is reproducible and useless; an
//! order that changes with the seed but does not record it is useful and unrepeatable.

use chiero_cir::*;
use chiero_exec::*;
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

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

/// A binary tree of `depth` symbolic branches: 2^depth leaves, each its own state.
///
/// Symbolic rather than constant, because a constant condition takes one edge and there is
/// no exploration order to observe. The parameter is unconstrained, so every branch forks.
fn tree(depth: u32) -> Module {
    let mut blocks = Vec::new();
    // Block `i` branches to `2i+1` and `2i+2` for the internal nodes.
    let internal = (1u32 << depth) - 1;
    for i in 0..internal {
        let c = ValueId(100 + i);
        blocks.push(block(
            i,
            vec![inst(
                InstKind::Assign {
                    dst: c,
                    rv: RValue::Cmp {
                        op: CmpOp::SLt,
                        a: Operand::Value(ValueId(0)),
                        b: i32c(i as i128),
                        ty: CTy::Int(32),
                    },
                },
                10 + i,
            )],
            Terminator::Br {
                cond: Operand::Value(c),
                t: BlockId(2 * i + 1),
                f: BlockId(2 * i + 2),
            },
        ));
    }
    for i in internal..(1u32 << (depth + 1)) - 1 {
        blocks.push(block(i, vec![], Terminator::Return(Some(i32c(i as i128)))));
    }
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "tree".into(),
            params: vec![Param {
                value: ValueId(0),
                ty: CTy::Int(32),
            }],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            access_paths: Default::default(),
            span: at(1),
        }],
        ..Default::default()
    }
}

/// The order the run finished its states in, as leaf block ids — the observable
/// exploration order.
fn order(m: &Module, strategy: Strategy) -> (Vec<u32>, u64) {
    let mut a = TermArena::new();
    let r = Engine::new(m).with_strategy(strategy).run(&mut a);
    let by_id: Vec<u32> = r
        .completion_order()
        .iter()
        .filter_map(|id| {
            r.states()
                .iter()
                .find(|s| s.id() == *id)
                .and_then(|s| s.trace().last().map(|(_, b)| b.0))
        })
        .collect();
    (by_id, r.seed())
}

/// **Contract 7, first half.** Changing the `RandomPath` seed changes exploration order.
#[test]
fn changing_the_seed_changes_the_exploration_order() {
    let m = tree(3);
    let (a, _) = order(&m, Strategy::RandomPath { seed: 0 });
    let (b, _) = order(&m, Strategy::RandomPath { seed: 12345 });
    assert_eq!(
        a.len(),
        b.len(),
        "the same states are explored either way — a strategy chooses the order, not the \
         set: {a:?} vs {b:?}"
    );
    assert_ne!(
        a, b,
        "two seeds, two orders. A `RandomPath` that ignored its seed would give the same \
         list twice and be indistinguishable from `Dfs`: {a:?}"
    );

    // And the *set* is identical, which is what makes the difference an ordering rather
    // than a coverage change.
    let (mut sa, mut sb) = (a.clone(), b.clone());
    sa.sort_unstable();
    sb.sort_unstable();
    assert_eq!(sa, sb, "same leaves, different order");
}

/// **Contract 7, second half.** The seed appears in `RunResult`.
///
/// Including the default, which is the case that matters: a bug found by a run nobody
/// configured still has to be replayable, and "0" has to come from the result rather than
/// from a reader's assumption.
#[test]
fn the_seed_is_recorded_in_the_result() {
    let m = tree(2);
    let (_, s) = order(&m, Strategy::RandomPath { seed: 99 });
    assert_eq!(s, 99);
    let (_, s) = order(&m, Strategy::RandomPath { seed: 0 });
    assert_eq!(s, 0, "the default is recorded, not omitted");
    let (_, s) = order(&m, Strategy::Dfs);
    assert_eq!(
        s, 0,
        "a strategy with no randomness reports the seed it did not use"
    );
}

/// **Every strategy is deterministic** — 023 §4's opening claim, and the one that makes a
/// bug report a bug report.
///
/// Run twice with the same seed and get the same order, for both strategies. A
/// `RandomPath` seeded from the system clock or from a hash of a pointer would pass every
/// other test in this file.
#[test]
fn the_same_seed_gives_the_same_order_twice() {
    let m = tree(3);
    for s in [Strategy::Dfs, Strategy::RandomPath { seed: 7 }] {
        let (a, _) = order(&m, s);
        let (b, _) = order(&m, s);
        assert_eq!(a, b, "{s:?} is not deterministic: {a:?} vs {b:?}");
    }
}

/// **The strategy does not change what was found.** Only the order differs.
///
/// The whole justification for having strategies is that they trade exploration order
/// against time-to-first-bug; a strategy that changed the *findings* on an exhaustive run
/// would be a soundness bug wearing a performance costume.
#[test]
fn the_strategy_does_not_change_the_findings() {
    let m = tree(3);
    let mut a1 = TermArena::new();
    let dfs = Engine::new(&m).with_strategy(Strategy::Dfs).run(&mut a1);
    let mut a2 = TermArena::new();
    let rnd = Engine::new(&m)
        .with_strategy(Strategy::RandomPath { seed: 42 })
        .run(&mut a2);

    let (mut f1, mut f2) = (dfs.findings(), rnd.findings());
    f1.sort();
    f2.sort();
    assert_eq!(f1, f2, "same program, same findings");
    assert_eq!(
        dfs.states().len(),
        rnd.states().len(),
        "and the same number of paths"
    );
}

/// `Dfs` is the default (023 §4's table says so), so a caller that sets nothing gets a
/// stack order rather than whatever happens to be first in the enum.
#[test]
fn dfs_is_the_default() {
    let m = tree(2);
    let mut a1 = TermArena::new();
    let implicit = Engine::new(&m).run(&mut a1);
    let mut a2 = TermArena::new();
    let explicit = Engine::new(&m).with_strategy(Strategy::Dfs).run(&mut a2);
    assert_eq!(
        implicit.completion_order(),
        explicit.completion_order(),
        "the default is `Dfs`"
    );
}
