//! Covers: 021 contract 19 — `LazyPolicy::max_depth` on a linked list.
//!
//! 021 §6: a pointer parameter starts as a symbolic value with no object; on first
//! dereference a `Lazy` object is materialized. `max_depth` "bounds the recursion of
//! linked structures (`p->next->next->…`). Exceeding it yields `Fidelity::Bounded` and a
//! note **naming the field that was cut off**."
//!
//! The bound has to exist because the recursion does not otherwise terminate: every
//! `next` read out of a lazy object is another symbolic pointer, and materializing on
//! demand would walk an infinite list. VPP's graph nodes chase `vlib_buffer_t` chains, so
//! this is the shape a real entry point starts with.
//!
//! **Naming the field is the half a plausible implementation drops.** "Bounded" alone tells
//! a reader the run was cut and not where; `next` tells them which structure to bound by
//! hand. The name comes from the `AccessPath` wave 104 added — this is the first consumer
//! of one, and it is why they exist.

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

/// `struct node { long val; struct node *next; };` — `next` at offset 8.
const NEXT_OFF: i128 = 8;

/// Walk `depth` links: `p->next`, `p->next->next`, … Each hop is a `PtrAdd` to the `next`
/// field and a `Load` of a pointer, and each address carries an `AccessPath` naming
/// `next`, exactly as lowering would build it.
fn walk(depth: u32) -> Module {
    let mut insts = Vec::new();
    let mut paths: Vec<(ValueId, AccessPath)> = Vec::new();
    let mut cur = ValueId(0);
    for i in 0..depth {
        let addr = ValueId(1 + 2 * i);
        let loaded = ValueId(2 + 2 * i);
        insts.push(inst(
            InstKind::Assign {
                dst: addr,
                rv: RValue::PtrAdd {
                    base: Operand::Value(cur),
                    off: Operand::Const(Const::Int {
                        bits: 64,
                        val: NEXT_OFF,
                    }),
                },
            },
            10 + 2 * i,
        ));
        paths.push((
            addr,
            AccessPath {
                root: PathRoot::Value(ValueId(0)),
                steps: [
                    PathStep::Deref,
                    PathStep::Field {
                        name: "next".into(),
                        off: NEXT_OFF as u64,
                    },
                ]
                .into_iter()
                .collect(),
            },
        ));
        insts.push(inst(
            InstKind::Assign {
                dst: loaded,
                rv: RValue::Load {
                    addr: Operand::Value(addr),
                    ty: CTy::Ptr,
                    align: 8,
                    vol: Volatility::Normal,
                },
            },
            11 + 2 * i,
        ));
        cur = loaded;
    }
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "walk".into(),
            params: vec![Param {
                value: ValueId(0),
                ty: CTy::Ptr,
            }],
            ret: CTy::Void,
            variadic: false,
            allocas: vec![],
            blocks: vec![Block {
                id: BlockId(0),
                insts,
                term: Terminator::Return(None),
                gcov_lines: Default::default(),
                span: at(1),
            }],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            access_paths: paths.into_iter().collect(),
            span: at(1),
        }],
        ..Default::default()
    }
}

fn run(m: &Module, max_depth: u32) -> (Fidelity, Vec<String>, usize) {
    let errs = chiero_cir::verify::verify(m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");
    let mut a = TermArena::new();
    let r = Engine::new(m)
        .with_lazy_policy(LazyPolicy {
            max_depth,
            ..LazyPolicy::default()
        })
        .run(&mut a);
    let s = &r.states()[0];
    let notes: Vec<String> = s.assumptions().iter().map(|x| x.detail.clone()).collect();
    (s.fidelity(), notes, s.object_ids_for_test().len())
}

/// **Contract 19.** With `max_depth = 2`, walking three links stops materializing at the
/// third and the result is `Bounded`, naming `next`.
#[test]
fn the_third_link_is_cut_and_the_note_names_the_field() {
    let (fid, notes, _) = run(&walk(3), 2);
    assert_eq!(
        fid,
        Fidelity::Bounded,
        "the walk was cut short, and a run that says `Exact` about a list it stopped \
         following is claiming to have checked what it did not: {notes:#?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("next")),
        "the note names the field that was cut off — `Bounded` alone says the run was cut \
         and not where, so nobody can bound the structure by hand: {notes:#?}"
    );
}

/// **Two links under `max_depth = 2` is not cut**, and the run stays `Exact`.
///
/// The negative half, and the one that matters: a policy that reported `Bounded` on every
/// lazy materialization satisfies the test above and makes the fidelity useless.
#[test]
fn a_walk_within_the_bound_is_not_cut() {
    let (fid, notes, _) = run(&walk(2), 2);
    assert_eq!(
        fid,
        Fidelity::Exact,
        "two links is exactly the bound, so nothing was cut: {notes:#?}"
    );
    assert!(
        !notes.iter().any(|n| n.contains("next")),
        "and nothing claims otherwise: {notes:#?}"
    );
}

/// **The bound is what stops it**, not the program: raising `max_depth` follows further.
///
/// Without this, an implementation that always cut at a hardcoded depth passes both tests
/// above. The object count is the evidence — each materialized link is one more object.
#[test]
fn raising_the_bound_materializes_more_links() {
    let (_, _, at_two) = run(&walk(4), 2);
    let (_, _, at_four) = run(&walk(4), 4);
    assert!(
        at_four > at_two,
        "a deeper bound materializes more of the list: {at_two} vs {at_four}"
    );
    let (fid, _, _) = run(&walk(4), 4);
    assert_eq!(
        fid,
        Fidelity::Exact,
        "and four links under a bound of four fit"
    );
}

/// A cut path **continues** rather than terminating.
///
/// 021 §6 bounds materialization; it does not end the run. Killing the state would lose
/// every finding after the third link, which on a VPP graph node is most of the function —
/// and would report their absence as if the code were clean.
#[test]
fn a_cut_walk_still_finishes_the_function() {
    let mut a = TermArena::new();
    let m = walk(3);
    let r = Engine::new(&m)
        .with_lazy_policy(LazyPolicy {
            max_depth: 2,
            ..LazyPolicy::default()
        })
        .run(&mut a);
    assert_eq!(r.states().len(), 1);
    assert!(
        matches!(r.states()[0].status(), Status::Done),
        "the path ran to the return: {:?}",
        r.states()[0].status()
    );
}

/// The default policy is 021 §6's: `max_depth` 3.
#[test]
fn the_default_policy_is_the_spec_s() {
    let p = LazyPolicy::default();
    assert_eq!(p.max_depth, 3, "021 §6 says the default is 3");
    assert!(p.distinct_by_default, "and two lazy objects are distinct");
}
