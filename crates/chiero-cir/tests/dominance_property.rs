//! **The dominance rule, checked against its definition on random CFGs.**
//!
//! `verify` swapped full dominator *sets* for Cooper–Harvey–Kennedy immediate dominators on
//! 2026-08-09 — the sets seeded every block with "every block in the function", which measured
//! **35.6 GB peak RSS** on a 96 000-block function. A representation change under a rule this
//! load-bearing needs more than "the suite still passes": every dominance rejection in
//! `verifier.rs` runs through the code being replaced, so those tests were written against the
//! *old* implementation's behaviour.
//!
//! So this checks the new one against the **definition** instead: `d` dominates `u` when every
//! path from entry to `u` passes through `d` — equivalently, when deleting `d` makes `u`
//! unreachable. That is computed here by brute-force reachability, which is obviously correct
//! and far too slow to ship, which is exactly what a property test's oracle should be.

use chiero_cir::*;
use chiero_span::Span;
use std::collections::BTreeSet;

/// A deterministic LCG. **Not `rand`**: a property test that cannot be replayed from its seed
/// reports a failure nobody can reproduce, and this workspace keeps its randomness reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn upto(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
}

/// Successor lists for `n` blocks: a random DAG over the block order, so every graph is
/// well-formed, plus the occasional back edge so loops are covered too.
fn shape(rng: &mut Rng, n: usize) -> Vec<Vec<usize>> {
    (0..n)
        .map(|i| {
            if i + 1 >= n {
                return vec![];
            }
            match rng.upto(4) {
                0 => vec![i + 1],
                1 => vec![i + 1, (i + 1 + rng.upto(n - i - 1)).min(n - 1)],
                2 => vec![(i + 1 + rng.upto(n - i - 1)).min(n - 1)],
                // A back edge, when there is somewhere to go back to.
                _ if i > 0 => vec![i + 1, rng.upto(i)],
                _ => vec![i + 1],
            }
        })
        .collect()
}

fn reachable_without(succ: &[Vec<usize>], removed: Option<usize>) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    if removed == Some(0) {
        return seen;
    }
    let mut work = vec![0usize];
    seen.insert(0);
    while let Some(b) = work.pop() {
        for &s in &succ[b] {
            if removed == Some(s) || !seen.insert(s) {
                continue;
            }
            work.push(s);
        }
    }
    seen
}

/// `%1` is defined in `def` and used in `use_`; everything else is scaffolding.
fn module_of(succ: &[Vec<usize>], def: usize, use_: usize) -> Module {
    let i = |kind| Inst { kind, span: Span::DUMMY, generated: false };
    let c = |v: i128| Operand::Const(Const::Int { bits: 32, val: v });
    let blocks = (0..succ.len())
        .map(|b| {
            let mut insts = Vec::new();
            if b == def {
                insts.push(i(InstKind::Assign {
                    dst: ValueId(1),
                    rv: RValue::Bin { op: BinOp::Add, a: c(1), b: c(2), ty: CTy::Int(32), signed: true },
                }));
            }
            if b == use_ {
                insts.push(i(InstKind::Assign {
                    dst: ValueId(2),
                    rv: RValue::Bin {
                        op: BinOp::Add,
                        a: Operand::Value(ValueId(1)),
                        b: c(1),
                        ty: CTy::Int(32),
                        signed: true,
                    },
                }));
            }
            let term = match succ[b].len() {
                0 => Terminator::Return(Some(c(0))),
                1 => Terminator::Goto(BlockId(succ[b][0] as u32)),
                _ => Terminator::Br {
                    cond: c(1),
                    t: BlockId(succ[b][0] as u32),
                    f: BlockId(succ[b][1] as u32),
                },
            };
            Block { id: BlockId(b as u32), insts, term, gcov_lines: Default::default(), span: Span::DUMMY }
        })
        .collect();
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
            body: Body::Defined,
            access_paths: Default::default(),
            span: Span::DUMMY,
            linkage: Linkage::External,
        }],
        ..Default::default()
    }
}

#[test]
fn a_use_is_rejected_exactly_when_its_definition_does_not_dominate_it() {
    let mut rng = Rng(0x5eed_1234);
    let (mut checked, mut rejected, mut accepted) = (0, 0, 0);
    for case in 0..400 {
        let n = 3 + rng.upto(9);
        let succ = shape(&mut rng, n);
        let live = reachable_without(&succ, None);
        let def = rng.upto(n);
        let use_ = rng.upto(n);
        // Dead code is a *warning* by design (020 rule 3), and a definition and use in the same
        // block is a textual-order question rather than a dominance one. Both are covered by
        // `verifier.rs`; neither is what this property is about.
        if def == use_ || !live.contains(&def) || !live.contains(&use_) {
            continue;
        }
        // The definition: `def` dominates `use_` iff deleting `def` unreaches `use_`.
        let expected_dominates = !reachable_without(&succ, Some(def)).contains(&use_);

        let m = module_of(&succ, def, use_);
        let got_rejection = verify::verify(&m)
            .iter()
            .any(|e| e.is_error() && matches!(e.kind, verify::VerifyErrorKind::UseNotDominated));

        assert_eq!(
            got_rejection,
            !expected_dominates,
            "case {case}: def=b{def} use=b{use_} succ={succ:?} — the verifier {} but the \
             definition says dominates={expected_dominates}",
            if got_rejection { "rejected" } else { "accepted" }
        );
        checked += 1;
        if got_rejection { rejected += 1 } else { accepted += 1 }
    }
    // **Both verdicts must actually occur.** A generator that only ever produced dominating
    // pairs would pass this test while checking nothing, which is the shape of every dead gate
    // in this project's history.
    assert!(checked > 100, "only {checked} usable cases — the generator is too narrow");
    assert!(rejected > 10, "no non-dominating cases generated ({rejected})");
    assert!(accepted > 10, "no dominating cases generated ({accepted})");
    eprintln!("dominance property: {checked} cases, {rejected} rejected, {accepted} accepted");
}
