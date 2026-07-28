//! `chiero-opt` — the optional passes of 020 §9.
//!
//! Every pass here is **opt-in, off by default, and observationally transparent**: for the
//! same entry state the set of reported findings and their concrete counterexamples is
//! unchanged, and only performance differs. The engine is required to be correct without
//! any of them, so a pass being *absent* is always a valid configuration — which is why
//! [`run_default`] runs nothing.
//!
//! §9 also names four things no pass may do, and each is a different kind of damage:
//!
//! - **Drop a `Marker`.** 021 retires objects on `Marker::Scope`; a merge that kept only
//!   the "real" instructions would leak every local in the absorbed block.
//! - **Merge two blocks with different `Volatile` accesses.** A volatile access is an
//!   observable event in program order, and merging across one asserts an ordering the
//!   device never agreed to.
//! - **Discard a `Span`.** 030 attributes coverage by span; an instruction without one is
//!   invisible to the correlation.
//! - **Widen a `LoadBits`/`StoreBits` into a byte-granular access.** That would make
//!   contract 24's uninitialized read *defined*, so the checker goes quiet about a real
//!   defect. No pass here rewrites a bit-granular access at all, which is the only way to
//!   be sure of this rather than merely careful about it.

use chiero_cir::{
    BinOp, Block, BlockId, CTy, Const, Function, InstKind, Module, Operand, RValue, Terminator,
    Volatility,
};

/// One registered pass.
///
/// A registry rather than a set of free functions someone remembers to call: 020
/// contract 44 is written over *every* pass, and a test that names the ones it knows about
/// stops covering the next one added — silently, and in the direction nobody looks.
#[derive(Debug)]
pub struct Pass {
    pub name: &'static str,
    pub run: fn(&mut Module),
}

/// Every pass, in no particular order — they are independent and none requires another.
pub static PASSES: &[Pass] = &[
    Pass {
        name: "simplify_cfg",
        run: simplify_cfg,
    },
    Pass {
        name: "const_fold",
        run: const_fold,
    },
];

/// The default pipeline: **empty**.
///
/// 020 §9's first sentence. Spelled as a function rather than left implicit so the claim
/// "off by default" has somewhere to be tested; a build that quietly enabled a pass for
/// everyone would satisfy every transparency test, precisely because those tests assert
/// transparency.
pub fn run_default(_m: &mut Module) {}

/// Look up a pass by name, for a caller driven by configuration.
pub fn pass(name: &str) -> Option<&'static Pass> {
    PASSES.iter().find(|p| p.name == name)
}

// ---------------------------------------------------------------------------------------
// simplify_cfg
// ---------------------------------------------------------------------------------------

/// Merge a block into its unique predecessor when that predecessor's only successor is it.
///
/// 020 §9: "merge blocks with a single predecessor/successor, drop empty blocks. Must
/// preserve `gcov_lines` by union, or coverage correlation breaks."
///
/// The union is concatenate-then-sort, not append: 015 §5 says the set is sorted
/// ascending, and a consumer that binary-searches it would silently miss lines in an
/// unsorted one.
pub fn simplify_cfg(m: &mut Module) {
    for f in &mut m.funcs {
        simplify_function(f);
    }
}

fn simplify_function(f: &mut Function) {
    // Repeat to a fixed point: merging B into A can make A's new successor mergeable, and
    // a single sweep would leave a chain half-collapsed — not *wrong*, but it makes the
    // result depend on block order, and 020 contract 21 says two runs agree.
    while let Some((pred_id, succ_id)) = mergeable_pair(f) {
        let si = f
            .blocks
            .iter()
            .position(|b| b.id == succ_id)
            .expect("the pair came from this function");
        let succ = f.blocks.remove(si);
        let pi = f
            .blocks
            .iter()
            .position(|b| b.id == pred_id)
            .expect("removing the successor cannot remove the predecessor");
        let pred = &mut f.blocks[pi];

        // **Markers come across too.** They are instructions; taking only the "real" ones
        // would leak every local the absorbed block's scope owned.
        pred.insts.extend(succ.insts);
        pred.term = succ.term;

        // The union, sorted and deduplicated (015 §5).
        pred.gcov_lines.extend(succ.gcov_lines);
        pred.gcov_lines.sort_unstable();
        pred.gcov_lines.dedup();
    }
}

/// The first `(predecessor, successor)` block pair that may be merged, or `None`.
///
/// Ids rather than indices: the caller removes a block between finding the pair and using
/// it, and an index that shifted under it would merge a different pair than the one every
/// condition here was checked against.
fn mergeable_pair(f: &Function) -> Option<(BlockId, BlockId)> {
    for pred in &f.blocks {
        let Terminator::Goto(target) = pred.term else {
            continue;
        };
        // **A block is never merged into itself, and the entry is never absorbed.**
        //
        // One guard, not two, because the two conditions are only ever true together:
        // a self-loop is its own sole predecessor, so it passes the count test below, and
        // the verifier rejects a back edge into the entry from anywhere else — leaving
        // `entry: goto entry` as the single input either condition rejects. Written as two
        // guards, each was an equivalent mutant of the other and neither could be killed.
        if target == pred.id || target == f.entry {
            continue;
        }
        // The successor must be entered from nowhere else, or the merge runs the
        // predecessor's instructions on a path that never had them.
        if preds_of(f, target) != 1 {
            continue;
        }
        let Some(succ) = f.blocks.iter().find(|b| b.id == target) else {
            continue;
        };
        // **§9's volatile prohibition.** Two observable events stay in the blocks the
        // program put them in.
        if has_volatile(pred) && has_volatile(succ) {
            continue;
        }
        return Some((pred.id, target));
    }
    None
}

fn preds_of(f: &Function, id: BlockId) -> usize {
    f.blocks
        .iter()
        .filter(|b| b.term.successors().contains(&id))
        .count()
}

fn has_volatile(b: &Block) -> bool {
    b.insts.iter().any(|i| match &i.kind {
        InstKind::Store { vol, .. } => *vol == Volatility::Volatile,
        InstKind::Assign {
            rv: RValue::Load { vol, .. },
            ..
        } => *vol == Volatility::Volatile,
        _ => false,
    })
}

// ---------------------------------------------------------------------------------------
// const_fold
// ---------------------------------------------------------------------------------------

/// Fold `Const`-only integer `RValue`s in place (020 §9).
///
/// The instruction is **rewritten, not removed**: its `dst` may be used anywhere later,
/// and its `Span` is what 030 attributes the line to. Folding to an `Assign` of a `Const`
/// keeps both, and leaves the dead-code question to whoever wants to ask it.
///
/// Only wrapping arithmetic at equal widths folds. Division is left alone entirely —
/// division by zero is undefined behaviour the *engine* reports as a finding, and a pass
/// that folded it would either invent a value the program cannot produce or panic. Either
/// way the finding disappears, which §9 forbids in as many words.
pub fn const_fold(m: &mut Module) {
    for f in &mut m.funcs {
        for b in &mut f.blocks {
            for i in &mut b.insts {
                let InstKind::Assign { dst, rv } = &i.kind else {
                    continue;
                };
                let RValue::Bin { op, a, b: rhs, ty } = rv else {
                    continue;
                };
                let CTy::Int(bits) = ty else { continue };
                let (
                    Operand::Const(Const::Int { val: x, bits: xb }),
                    Operand::Const(Const::Int { val: y, bits: yb }),
                ) = (a, rhs)
                else {
                    continue;
                };
                // Mismatched widths are a verifier error, not something to fold through.
                if xb != bits || yb != bits {
                    continue;
                }
                let Some(v) = fold(*op, *x, *y, *bits) else {
                    continue;
                };
                i.kind = InstKind::Assign {
                    dst: *dst,
                    rv: RValue::Use(Operand::Const(Const::Int {
                        bits: *bits,
                        val: v,
                    })),
                };
            }
        }
    }
}

/// The value of `x op y` at `bits`, or `None` if the pass declines to fold it.
fn fold(op: BinOp, x: i128, y: i128, bits: u32) -> Option<i128> {
    // Above 64 bits an `i128` intermediate cannot represent every wrapped result, and a
    // wrong constant is worse than an unfolded one.
    if bits == 0 || bits > 64 {
        return None;
    }
    let v = match op {
        BinOp::Add => x.wrapping_add(y),
        BinOp::Sub => x.wrapping_sub(y),
        BinOp::Mul => x.wrapping_mul(y),
        BinOp::And => x & y,
        BinOp::Or => x | y,
        BinOp::Xor => x ^ y,
        // A shift by more than the width is undefined behaviour the engine reports;
        // folding it would delete the finding.
        BinOp::Shl if (0..bits as i128).contains(&y) => x.wrapping_shl(y as u32),
        _ => return None,
    };
    Some(truncate(v, bits))
}

/// Wrap `v` to `bits`, keeping the sign the width implies — the same arithmetic the engine
/// does, because a pass that wrapped differently would change a reported value.
fn truncate(v: i128, bits: u32) -> i128 {
    if bits >= 128 {
        return v;
    }
    let shift = 128 - bits;
    (v << shift) >> shift
}
