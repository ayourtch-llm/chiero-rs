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
    AllocaDecl, AllocaId, BinOp, Block, BlockId, CTy, Const, Function, Inst, InstKind, Module,
    Operand, RValue, Terminator, ValueId, Volatility,
};
use indexmap::IndexMap;

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
    Pass {
        name: "mem2reg",
        run: mem2reg,
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

// ---------------------------------------------------------------------------------------
// mem2reg
// ---------------------------------------------------------------------------------------

/// Promote non-address-taken allocas to `ValueId`s, inserting phis where paths join
/// (020 §9, contract 16).
///
/// **The hard part is refusing.** Promoting a slot the program can reach any other way
/// makes the register stop being the memory — silently, and on exactly the paths where a
/// checker was about to find something. [`promotable`] is where that judgement lives, and
/// it is deliberately conservative: every shape it has not been taught about is refused.
///
/// A load with no reaching store becomes `Undef` and **not zero**. `int x; return x;` is
/// an uninitialized read 021 reports, and substituting zero would make the program defined
/// and delete the finding — the transparency violation §9 forbids in as many words.
pub fn mem2reg(m: &mut Module) {
    for f in &mut m.funcs {
        let slots: Vec<AllocaId> = f
            .allocas
            .iter()
            .filter(|d| promotable(f, d))
            .map(|d| d.id)
            .collect();
        for slot in slots {
            promote(f, slot);
        }
    }
}

/// Whether `d` may be promoted: every use of its address is a whole-slot, non-volatile
/// `Load` or `Store` through a value that came straight from `AddrOfLocal`.
fn promotable(f: &Function, d: &AllocaDecl) -> bool {
    // An array or a dynamic extent is not one value, so there is no register to hold it.
    if d.count != 1 {
        return false;
    }
    // The `ValueId`s that hold this slot's address. Only a direct `AddrOfLocal` counts —
    // an address that has been through arithmetic or a cast is one this pass cannot
    // follow, and following it badly is the whole failure mode.
    let mut addrs: Vec<ValueId> = Vec::new();
    for b in &f.blocks {
        for i in &b.insts {
            if let InstKind::Assign {
                dst,
                rv: RValue::AddrOfLocal { alloca },
            } = &i.kind
                && *alloca == d.id
            {
                addrs.push(*dst);
            }
        }
    }
    let is_addr = |o: &Operand| matches!(o, Operand::Value(v) if addrs.contains(v));

    for b in &f.blocks {
        for i in &b.insts {
            match &i.kind {
                // The definition itself.
                InstKind::Assign {
                    rv: RValue::AddrOfLocal { .. },
                    ..
                } => {}
                // A whole-slot, non-volatile store *of* something to the slot is fine; a
                // store of the **address** anywhere is an escape.
                InstKind::Store {
                    addr,
                    val,
                    ty,
                    align,
                    vol,
                } => {
                    if is_addr(val) {
                        return false;
                    }
                    if is_addr(addr)
                        && (*vol != Volatility::Normal || *ty != d.ty || *align != d.align)
                    {
                        return false;
                    }
                }
                InstKind::Assign {
                    rv:
                        RValue::Load {
                            addr,
                            ty,
                            align,
                            vol,
                        },
                    ..
                } => {
                    if is_addr(addr)
                        && (*vol != Volatility::Normal || *ty != d.ty || *align != d.align)
                    {
                        return false;
                    }
                }
                // Anything else that so much as mentions the address: a call argument, a
                // `CopyMem`, a `StoreBits`, an `Opaque` read, a pointer comparison. Each
                // is a way the value can be observed or changed without a `Store`, and
                // none of them is worth special-casing for the sake of one more promotion.
                other => {
                    if inst_operands(other).iter().any(is_addr) {
                        return false;
                    }
                }
            }
        }
        // A terminator can carry the address too — `return &x` is the obvious one.
        if term_operands(&b.term).iter().any(is_addr) {
            return false;
        }
    }
    true
}

/// Every operand an instruction mentions, for the escape scan.
///
/// Deliberately **without a catch-all**, so a new `InstKind` is a compile error here
/// rather than a silently-missed escape route — the failure this whole function exists to
/// prevent.
fn inst_operands(k: &InstKind) -> Vec<Operand> {
    let mut v = Vec::new();
    match k {
        InstKind::Assign { rv, .. } => rvalue_operands(rv, &mut v),
        InstKind::Store { addr, val, .. } => v.extend([addr.clone(), val.clone()]),
        InstKind::StoreBits { addr, val, .. } => v.extend([addr.clone(), val.clone()]),
        InstKind::CopyMem { dst, src, size, .. } => {
            v.extend([dst.clone(), src.clone(), size.clone()])
        }
        InstKind::SetMem { dst, byte, size } => v.extend([dst.clone(), byte.clone(), size.clone()]),
        InstKind::Call { callee, args, .. } => {
            if let chiero_cir::Callee::Indirect(o) = callee {
                v.push(o.clone());
            }
            v.extend(args.iter().cloned());
        }
        InstKind::AllocaDyn { count, .. } => v.push(count.clone()),
        InstKind::VaArg { list, .. } | InstKind::VaStart { list } | InstKind::VaEnd { list } => {
            v.push(list.clone())
        }
        InstKind::VaCopy { dst, src } => v.extend([dst.clone(), src.clone()]),
        InstKind::Opaque { writes, reads, .. } => {
            for w in writes {
                v.extend([w.addr.clone(), w.size.clone()]);
            }
            v.extend(reads.iter().cloned());
        }
        InstKind::Phi { incomings, .. } => v.extend(incomings.iter().map(|(_, o)| o.clone())),
        InstKind::Marker(_) => {}
    }
    v
}

fn rvalue_operands(rv: &RValue, v: &mut Vec<Operand>) {
    match rv {
        RValue::Use(o) | RValue::ExtractLane { v: o, .. } | RValue::Splat { elem: o, .. } => {
            v.push(o.clone())
        }
        RValue::Load { addr, .. } | RValue::LoadBits { addr, .. } => v.push(addr.clone()),
        RValue::Bin { a, b, .. } | RValue::Cmp { a, b, .. } | RValue::Shuffle { a, b, .. } => {
            v.extend([a.clone(), b.clone()])
        }
        RValue::Un { a, .. } | RValue::Cast { a, .. } => v.push(a.clone()),
        RValue::PtrAdd { base, off, .. } => v.extend([base.clone(), off.clone()]),
        RValue::InsertLane { v: x, val, .. } => v.extend([x.clone(), val.clone()]),
        RValue::Select { cond, t, f } => v.extend([cond.clone(), t.clone(), f.clone()]),
        RValue::Fresh { .. }
        | RValue::AddrOfLocal { .. }
        | RValue::AddrOfGlobal { .. }
        | RValue::AddrOfFunc { .. } => {}
    }
}

fn term_operands(t: &Terminator) -> Vec<Operand> {
    match t {
        Terminator::Return(Some(o)) | Terminator::IndirectGoto { addr: o, .. } => vec![o.clone()],
        Terminator::Br { cond, .. } => vec![cond.clone()],
        Terminator::Switch { scrut, .. } => vec![scrut.clone()],
        Terminator::Return(None) | Terminator::Goto(_) | Terminator::Unreachable(_) => vec![],
    }
}

/// Replace every access to `slot` with values, inserting phis where paths join.
///
/// **Phis are placed by iteration to a fixed point, not by a dominance-frontier
/// computation.** The frontier algorithm is the textbook one and is asymptotically
/// better; this is a pass that runs over one function at a time, off by default, and the
/// property that matters is that it is *obviously* right. Iterating until the value
/// entering each block stops changing needs no dominator tree — and a dominator tree is a
/// second place to be wrong about the CFG.
fn promote(f: &mut Function, slot: AllocaId) {
    let ty = f
        .allocas
        .iter()
        .find(|d| d.id == slot)
        .map(|d| d.ty.clone())
        .unwrap_or(CTy::Void);
    // **The address values, computed once.** Recomputing them by scanning `f` mid-rewrite
    // read a function whose current block had already been emptied — and the
    // `AddrOfLocal` that *defines* the address is usually in that very block, so every
    // load and store of the slot stopped being recognized exactly when it mattered.
    let addrs: Vec<ValueId> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Assign {
                dst,
                rv: RValue::AddrOfLocal { alloca },
            } if *alloca == slot => Some(*dst),
            _ => None,
        })
        .collect();
    let is_addr = |o: &Operand| matches!(o, Operand::Value(v) if addrs.contains(v));

    let order: Vec<BlockId> = f.blocks.iter().map(|b| b.id).collect();
    let preds: IndexMap<BlockId, Vec<BlockId>> = order
        .iter()
        .map(|&id| {
            let mut p: Vec<BlockId> = f
                .blocks
                .iter()
                .filter(|b| b.term.successors().contains(&id))
                .map(|b| b.id)
                .collect();
            p.sort_by_key(|x| x.0);
            p.dedup();
            (id, p)
        })
        .collect();

    // A phi is reserved for every block with more than one predecessor, then **deleted
    // again** if its incomings all agree. Reserving first and pruning after is what makes
    // the fixed point terminate: the value entering a join is not known until its
    // predecessors are, and a loop header's predecessor includes itself.
    let mut phi_dst: IndexMap<BlockId, ValueId> = IndexMap::new();
    let mut next = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Assign { dst, .. }
            | InstKind::AllocaDyn { dst, .. }
            | InstKind::VaArg { dst, .. }
            | InstKind::Phi { dst, .. } => Some(dst.0),
            InstKind::Call { dst: Some(d), .. } => Some(d.0),
            _ => None,
        })
        .chain(f.params.iter().map(|p| p.value.0))
        .max()
        .map_or(0, |x| x + 1);
    for (&id, p) in &preds {
        if p.len() > 1 {
            phi_dst.insert(id, ValueId(next));
            next += 1;
        }
    }

    // `out[b]` is the value of the slot when `b` ends; `resolved[b]` is the value on entry
    // to a join whose phi turned out to be unnecessary. Iterated to a fixed point.
    let mut out: IndexMap<BlockId, Operand> = IndexMap::new();
    let mut resolved: IndexMap<BlockId, Operand> = IndexMap::new();
    let undef = Operand::Const(Const::Undef(ty.clone()));

    // **Pruning feeds back into the fixed point, it is not a cleanup at the end.**
    // Dropping a phi at the end left every load that had already been rewritten reading
    // the pruned phi's `ValueId` — a value nothing defines. Removing one changes what
    // reaches every block after it, so the whole thing is recomputed and pruning tried
    // again. Each round removes at least one phi, so it terminates.
    loop {
        for &id in &order {
            out.insert(id, undef.clone());
        }
        for _ in 0..order.len() + 2 {
            let mut changed = false;
            for &id in &order {
                let mut cur = incoming_value(&preds, &phi_dst, &resolved, &out, id, &undef);
                let bi = f.blocks.iter().position(|b| b.id == id).expect("id");
                for i in &f.blocks[bi].insts {
                    if let InstKind::Store { addr, val, .. } = &i.kind
                        && is_addr(addr)
                    {
                        cur = val.clone();
                    }
                }
                if out.get(&id) != Some(&cur) {
                    out.insert(id, cur);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // A phi is **trivial** when its incomings, ignoring any that are the phi itself,
        // are all one value. Ignoring self-references is what lets a loop header whose
        // body never stores collapse to the preheader's value instead of standing forever
        // as a choice between `v` and itself.
        let trivial = phi_dst.iter().find_map(|(&id, &dst)| {
            let ps = preds.get(&id)?;
            let mut vals = ps
                .iter()
                .map(|p| out.get(p).cloned().unwrap_or_else(|| undef.clone()))
                .filter(|v| *v != Operand::Value(dst));
            let first = vals.next()?;
            vals.all(|v| v == first).then_some((id, first))
        });
        match trivial {
            Some((id, v)) => {
                phi_dst.shift_remove(&id);
                resolved.insert(id, v);
            }
            None => break,
        }
    }

    // Rewrite: loads become `Use` of the value in flight, stores disappear, and the
    // `AddrOfLocal` goes with them.
    for bi in 0..f.blocks.len() {
        let id = f.blocks[bi].id;
        let mut cur = incoming_value(&preds, &phi_dst, &resolved, &out, id, &undef);
        let mut new_insts: Vec<Inst> = Vec::new();
        let insts = std::mem::take(&mut f.blocks[bi].insts);
        for i in insts {
            match &i.kind {
                InstKind::Assign {
                    rv: RValue::AddrOfLocal { alloca },
                    ..
                } if *alloca == slot => {}
                InstKind::Store { addr, val, .. } if is_addr(addr) => {
                    cur = val.clone();
                }
                InstKind::Assign {
                    dst,
                    rv: RValue::Load { addr, .. },
                } if is_addr(addr) => {
                    // **`Use`, not deletion.** The `dst` is referenced elsewhere and the
                    // `Span` is what 030 attributes the line to; dropping the instruction
                    // would take both.
                    new_insts.push(Inst {
                        kind: InstKind::Assign {
                            dst: *dst,
                            rv: RValue::Use(cur.clone()),
                        },
                        span: i.span,
                        generated: i.generated,
                    });
                }
                _ => new_insts.push(i),
            }
        }
        f.blocks[bi].insts = new_insts;
    }

    // Insert the phis that are still needed, at the top of their blocks.
    for (&id, &dst) in &phi_dst {
        let ps = preds.get(&id).cloned().unwrap_or_default();
        let incomings: Vec<(BlockId, Operand)> = ps
            .iter()
            .map(|&p| (p, out.get(&p).cloned().unwrap_or_else(|| undef.clone())))
            .collect();
        let bi = f.blocks.iter().position(|b| b.id == id).expect("id");
        let span = f.blocks[bi].span;
        // After any leading markers: a `.line` at a block's top is part of its entry, and
        // putting the phi above it would move the line the block is attributed to.
        let at = f.blocks[bi]
            .insts
            .iter()
            .position(|i| !matches!(i.kind, InstKind::Marker(_)))
            .unwrap_or(f.blocks[bi].insts.len());
        f.blocks[bi].insts.insert(
            at,
            Inst {
                kind: InstKind::Phi {
                    dst,
                    ty: ty.clone(),
                    incomings,
                },
                span,
                generated: true,
            },
        );
    }

    f.allocas.retain(|d| d.id != slot);
}

/// The slot's value on entry to `id`: the phi if one was reserved, the sole predecessor's
/// value if there is one, `Undef` at the entry block.
fn incoming_value(
    preds: &IndexMap<BlockId, Vec<BlockId>>,
    phi_dst: &IndexMap<BlockId, ValueId>,
    resolved: &IndexMap<BlockId, Operand>,
    out: &IndexMap<BlockId, Operand>,
    id: BlockId,
    undef: &Operand,
) -> Operand {
    if let Some(&d) = phi_dst.get(&id) {
        return Operand::Value(d);
    }
    if let Some(v) = resolved.get(&id) {
        return v.clone();
    }
    match preds.get(&id).map(|p| p.as_slice()) {
        Some([p]) => out.get(p).cloned().unwrap_or_else(|| undef.clone()),
        _ => undef.clone(),
    }
}
