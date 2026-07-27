//! The CIR verifier (020 §8).
//!
//! A module that fails verification is never executed, so a missed rule lets malformed
//! IR reach the engine — where the symptom is a confusing wrong answer rather than a
//! clear error.

use crate::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VerifyErrorKind {
    ValueAssignedTwice,
    UseNotDominated,
    UnknownBlock,
    WidthMismatch,
    BadCast,
    BadPointerOperand,
    BadAlignment,
    DuplicateSwitchCase,
    EntryHasPredecessor,
    DeclaredWithBody,
    DefinedWithoutBody,
    BadBitRange,
    BadLane,
    AllocaExtentMismatch,
    /// Rule 3: a *warning*. Unreachable C code exists and is legal.
    UnreachableBlock,
}

impl VerifyErrorKind {
    /// Whether this blocks execution. Only `UnreachableBlock` does not.
    pub fn is_error(self) -> bool {
        self != VerifyErrorKind::UnreachableBlock
    }
}

#[derive(Clone, Debug)]
pub struct VerifyError {
    pub kind: VerifyErrorKind,
    pub func: FuncId,
    pub detail: String,
    pub span: Span,
}

impl VerifyError {
    pub fn is_error(&self) -> bool {
        self.kind.is_error()
    }
}

/// Verify a module. Errors are returned in a deterministic order (001 §5).
pub fn verify(m: &Module) -> Vec<VerifyError> {
    let mut out = Vec::new();
    for f in &m.funcs {
        verify_function(f, &mut out);
    }
    out
}

fn err(
    out: &mut Vec<VerifyError>,
    f: &Function,
    kind: VerifyErrorKind,
    span: Span,
    detail: String,
) {
    out.push(VerifyError {
        kind,
        func: f.id,
        detail,
        span,
    });
}

fn verify_function(f: &Function, out: &mut Vec<VerifyError>) {
    // Rule 10.
    match (&f.body, f.blocks.is_empty()) {
        (Body::Declared, false) => {
            err(
                out,
                f,
                VerifyErrorKind::DeclaredWithBody,
                f.span,
                format!(
                    "`{}` is declared but has {} block(s)",
                    f.name,
                    f.blocks.len()
                ),
            );
            return;
        }
        (Body::Defined, true) => {
            err(
                out,
                f,
                VerifyErrorKind::DefinedWithoutBody,
                f.span,
                format!("`{}` is defined but has no blocks", f.name),
            );
            return;
        }
        (Body::Declared, true) => return,
        (Body::Defined, false) => {}
    }

    check_block_refs(f, out);
    if out.iter().any(|e| e.kind == VerifyErrorKind::UnknownBlock) {
        // Later rules index blocks by id; a dangling reference would make their
        // diagnostics nonsense. Report the structural error alone.
        return;
    }
    check_entry_predecessors(f, out);
    check_reachability(f, out);
    check_switch_cases(f, out);
    check_allocas(f, out);
    check_ssa_and_types(f, out);
}

/// Rule 2.
fn check_block_refs(f: &Function, out: &mut Vec<VerifyError>) {
    let known: Vec<BlockId> = f.blocks.iter().map(|b| b.id).collect();
    for b in &f.blocks {
        for s in b.term.successors() {
            if !known.contains(&s) {
                err(
                    out,
                    f,
                    VerifyErrorKind::UnknownBlock,
                    b.span,
                    format!("block {:?} branches to unknown {:?}", b.id, s),
                );
            }
        }
    }
    if !known.contains(&f.entry) {
        err(
            out,
            f,
            VerifyErrorKind::UnknownBlock,
            f.span,
            format!("entry {:?} does not exist", f.entry),
        );
    }
}

/// Rule 9: lowering must insert a preheader rather than loop back to entry.
fn check_entry_predecessors(f: &Function, out: &mut Vec<VerifyError>) {
    for b in &f.blocks {
        if b.term.successors().contains(&f.entry) {
            err(
                out,
                f,
                VerifyErrorKind::EntryHasPredecessor,
                b.span,
                format!(
                    "{:?} branches to entry {:?}; insert a preheader",
                    b.id, f.entry
                ),
            );
        }
    }
}

/// Rule 3 — a warning, since unreachable C code is legal.
fn check_reachability(f: &Function, out: &mut Vec<VerifyError>) {
    let reachable = reachable_blocks(f);
    for b in &f.blocks {
        if !reachable.contains(&b.id) {
            err(
                out,
                f,
                VerifyErrorKind::UnreachableBlock,
                b.span,
                format!("{:?} is not reachable from entry", b.id),
            );
        }
    }
}

fn reachable_blocks(f: &Function) -> Vec<BlockId> {
    let mut seen = vec![f.entry];
    let mut stack = vec![f.entry];
    while let Some(b) = stack.pop() {
        let Some(blk) = f.block(b) else { continue };
        for s in blk.term.successors() {
            if !seen.contains(&s) {
                seen.push(s);
                stack.push(s);
            }
        }
    }
    seen
}

/// Rule 8.
fn check_switch_cases(f: &Function, out: &mut Vec<VerifyError>) {
    for b in &f.blocks {
        if let Terminator::Switch { cases, .. } = &b.term {
            let mut seen: Vec<i128> = Vec::new();
            for (v, _) in cases {
                if seen.contains(v) {
                    err(
                        out,
                        f,
                        VerifyErrorKind::DuplicateSwitchCase,
                        b.span,
                        format!("duplicate switch case {v} in {:?}", b.id),
                    );
                } else {
                    seen.push(*v);
                }
            }
        }
    }
}

/// Rule 13.
fn check_allocas(f: &Function, out: &mut Vec<VerifyError>) {
    for b in &f.blocks {
        for i in &b.insts {
            if let InstKind::AllocaDyn { alloca, .. } = &i.kind {
                match f.allocas.iter().find(|a| a.id == *alloca) {
                    Some(a) if a.count == crate::DYNAMIC_EXTENT => {}
                    Some(_) => err(
                        out,
                        f,
                        VerifyErrorKind::AllocaExtentMismatch,
                        i.span,
                        format!("{alloca:?} has a static extent but is given one by AllocaDyn"),
                    ),
                    None => err(
                        out,
                        f,
                        VerifyErrorKind::AllocaExtentMismatch,
                        i.span,
                        format!("AllocaDyn refers to undeclared {alloca:?}"),
                    ),
                }
            }
        }
    }
}

/// Rules 1, 5, 6, 7, 11, 12 — everything that needs the per-value type environment.
fn check_ssa_and_types(f: &Function, out: &mut Vec<VerifyError>) {
    // Rule 1a: assigned exactly once.
    let mut defs: IndexMap<ValueId, (BlockId, usize)> = IndexMap::new();
    let mut types: IndexMap<ValueId, CTy> = IndexMap::new();
    // Positions are 1-based so parameters can sit at 0, strictly before every
    // instruction in the entry block. With both at 0 a parameter would fail to dominate
    // instruction 0 — which is where parameters are overwhelmingly used.
    for p in &f.params {
        defs.insert(p.value, (f.entry, 0));
        types.insert(p.value, p.ty.clone());
    }
    for b in &f.blocks {
        for (i, inst) in b.insts.iter().enumerate() {
            for (dst, ty) in defined_by(inst) {
                if defs.contains_key(&dst) {
                    err(
                        out,
                        f,
                        VerifyErrorKind::ValueAssignedTwice,
                        inst.span,
                        format!("{dst:?} is assigned more than once"),
                    );
                } else {
                    defs.insert(dst, (b.id, i));
                    types.insert(dst, ty);
                }
            }
        }
    }

    let doms = dominators(f);

    for b in &f.blocks {
        for (i, inst) in b.insts.iter().enumerate() {
            for op in operands_of(inst) {
                check_dominated(f, b.id, i + 1, op, &defs, &doms, inst.span, out);
            }
            check_inst_types(f, inst, &types, out);
        }
        for op in term_operands(&b.term) {
            // A terminator is positioned after every instruction in its block.
            check_dominated(f, b.id, usize::MAX, op, &defs, &doms, b.span, out);
        }
        check_term_types(f, b, &types, out);
    }
}

fn defined_by(i: &Inst) -> Vec<(ValueId, CTy)> {
    match &i.kind {
        InstKind::Assign { dst, rv } => vec![(*dst, rvalue_type(rv))],
        InstKind::Call { dst: Some(d), .. } => vec![(*d, CTy::Void)],
        InstKind::AllocaDyn { dst, .. } => vec![(*dst, CTy::Ptr)],
        InstKind::VaArg { dst, ty, .. } => vec![(*dst, ty.clone())],
        _ => Vec::new(),
    }
}

/// The type an `RValue` produces. `Cmp` yields `Int(1)` regardless of its operand type
/// — conflating the two is the mistake 020 §8 rule 5 names.
fn rvalue_type(rv: &RValue) -> CTy {
    match rv {
        RValue::Use(o) => operand_type(o),
        RValue::Load { ty, .. } => ty.clone(),
        RValue::LoadBits { unit, .. } => unit.clone(),
        RValue::Bin { ty, .. } | RValue::Un { ty, .. } => ty.clone(),
        RValue::Cmp { .. } => CTy::Int(1),
        RValue::Cast { to, .. } => to.clone(),
        RValue::Select { t, .. } => operand_type(t),
        RValue::PtrAdd { .. }
        | RValue::AddrOfLocal { .. }
        | RValue::AddrOfGlobal { .. }
        | RValue::AddrOfFunc(_) => CTy::Ptr,
        RValue::Shuffle { a, .. } => operand_type(a),
        RValue::InsertLane { v, .. } => operand_type(v),
        RValue::ExtractLane { v, .. } => match operand_type(v) {
            CTy::Vector { elem, .. } => *elem,
            other => other,
        },
        RValue::Splat { elem, lanes } => CTy::Vector {
            elem: Box::new(operand_type(elem)),
            lanes: *lanes,
        },
        RValue::Fresh { ty } => ty.clone(),
    }
}

fn operand_type(o: &Operand) -> CTy {
    match o {
        Operand::Const(c) => const_type(c),
        // Resolved from the environment by the caller where it matters.
        Operand::Value(_) => CTy::Void,
    }
}

fn const_type(c: &Const) -> CTy {
    match c {
        Const::Int { bits, .. } => CTy::Int(*bits),
        Const::Wide { bits, .. } => CTy::Int(*bits),
        Const::Float(k, _) => CTy::Float(*k),
        Const::Null | Const::GlobalAddr { .. } | Const::FuncAddr(_) => CTy::Ptr,
        Const::Undef(t) => t.clone(),
    }
}

fn resolve(o: &Operand, types: &IndexMap<ValueId, CTy>) -> Option<CTy> {
    match o {
        Operand::Const(c) => Some(const_type(c)),
        Operand::Value(v) => types.get(v).cloned(),
    }
}

fn operands_of(i: &Inst) -> Vec<Operand> {
    let mut v = Vec::new();
    match &i.kind {
        InstKind::Assign { rv, .. } => rvalue_operands(rv, &mut v),
        InstKind::Store { addr, val, .. } => v.extend([addr.clone(), val.clone()]),
        InstKind::StoreBits { addr, val, .. } => v.extend([addr.clone(), val.clone()]),
        InstKind::CopyMem { dst, src, size, .. } => {
            v.extend([dst.clone(), src.clone(), size.clone()])
        }
        InstKind::SetMem { dst, byte, size } => v.extend([dst.clone(), byte.clone(), size.clone()]),
        InstKind::Call { callee, args, .. } => {
            if let Callee::Indirect(o) = callee {
                v.push(o.clone());
            }
            v.extend(args.iter().cloned());
        }
        InstKind::AllocaDyn { count, .. } => v.push(count.clone()),
        InstKind::VaArg { list, .. } | InstKind::VaStart { list } | InstKind::VaEnd { list } => {
            v.push(list.clone())
        }
        InstKind::VaCopy { dst, src } => v.extend([dst.clone(), src.clone()]),
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
        RValue::Select { cond, t, f } => v.extend([cond.clone(), t.clone(), f.clone()]),
        RValue::PtrAdd { base, off } => v.extend([base.clone(), off.clone()]),
        RValue::InsertLane { v: vec_, val, .. } => v.extend([vec_.clone(), val.clone()]),
        RValue::AddrOfLocal { .. }
        | RValue::AddrOfGlobal { .. }
        | RValue::AddrOfFunc(_)
        | RValue::Fresh { .. } => {}
    }
}

fn term_operands(t: &Terminator) -> Vec<Operand> {
    match t {
        Terminator::Br { cond, .. } => vec![cond.clone()],
        Terminator::Switch { scrut, .. } => vec![scrut.clone()],
        Terminator::Return(Some(o)) => vec![o.clone()],
        Terminator::IndirectGoto { addr, .. } => vec![addr.clone()],
        Terminator::Goto(_) | Terminator::Return(None) | Terminator::Unreachable(_) => Vec::new(),
    }
}

/// Immediate-dominator map by iterative dataflow. Small graphs, so simplicity beats
/// Lengauer-Tarjan here.
fn dominators(f: &Function) -> IndexMap<BlockId, Vec<BlockId>> {
    let ids: Vec<BlockId> = f.blocks.iter().map(|b| b.id).collect();
    let mut dom: IndexMap<BlockId, Vec<BlockId>> = IndexMap::new();
    for &b in &ids {
        dom.insert(
            b,
            if b == f.entry {
                vec![f.entry]
            } else {
                ids.clone()
            },
        );
    }
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &ids {
            if b == f.entry {
                continue;
            }
            let preds: Vec<BlockId> = f
                .blocks
                .iter()
                .filter(|p| p.term.successors().contains(&b))
                .map(|p| p.id)
                .collect();
            let mut new: Vec<BlockId> = match preds.first() {
                Some(p) => dom[p].clone(),
                None => vec![], // unreachable block: dominated by nothing but itself
            };
            for p in preds.iter().skip(1) {
                new.retain(|x| dom[p].contains(x));
            }
            if !new.contains(&b) {
                new.push(b);
            }
            new.sort_unstable();
            if dom[&b] != new {
                dom.insert(b, new);
                changed = true;
            }
        }
    }
    dom
}

#[allow(clippy::too_many_arguments)]
fn check_dominated(
    f: &Function,
    at_block: BlockId,
    at_index: usize,
    op: Operand,
    defs: &IndexMap<ValueId, (BlockId, usize)>,
    doms: &IndexMap<BlockId, Vec<BlockId>>,
    span: Span,
    out: &mut Vec<VerifyError>,
) {
    let Operand::Value(v) = op else { return };
    let Some(&(db, di)) = defs.get(&v) else {
        err(
            out,
            f,
            VerifyErrorKind::UseNotDominated,
            span,
            format!("{v:?} is used but never defined"),
        );
        return;
    };
    let ok = if db == at_block {
        // Same block: the definition must come first textually.
        di < at_index
    } else {
        doms.get(&at_block).is_some_and(|d| d.contains(&db))
    };
    if !ok {
        err(
            out,
            f,
            VerifyErrorKind::UseNotDominated,
            span,
            format!("{v:?} (defined in {db:?}) does not dominate its use in {at_block:?}"),
        );
    }
}

fn check_align(f: &Function, align: u64, span: Span, out: &mut Vec<VerifyError>) {
    if align == 0 || !align.is_power_of_two() {
        err(
            out,
            f,
            VerifyErrorKind::BadAlignment,
            span,
            format!("alignment {align} must be a non-zero power of two"),
        );
    }
}

fn check_bits(f: &Function, unit: &CTy, bits: BitRange, span: Span, out: &mut Vec<VerifyError>) {
    let Some(w) = unit.bit_width().filter(|_| unit.is_int()) else {
        err(
            out,
            f,
            VerifyErrorKind::BadBitRange,
            span,
            format!("bitfield unit {unit:?} must be an Int"),
        );
        return;
    };
    if bits.width == 0 || bits.off + bits.width > w {
        err(
            out,
            f,
            VerifyErrorKind::BadBitRange,
            span,
            format!(
                "bit range {}..{} does not fit in {w} bits",
                bits.off,
                bits.off + bits.width
            ),
        );
    }
}

fn require_ptr(
    f: &Function,
    o: &Operand,
    types: &IndexMap<ValueId, CTy>,
    what: &str,
    span: Span,
    out: &mut Vec<VerifyError>,
) {
    if let Some(t) = resolve(o, types)
        && t != CTy::Ptr
        && t != CTy::Void
    {
        err(
            out,
            f,
            VerifyErrorKind::BadPointerOperand,
            span,
            format!("{what} must be pointer-typed, got {t:?}"),
        );
    }
}

fn check_inst_types(
    f: &Function,
    i: &Inst,
    types: &IndexMap<ValueId, CTy>,
    out: &mut Vec<VerifyError>,
) {
    match &i.kind {
        InstKind::Assign { rv, .. } => check_rvalue_types(f, rv, types, i.span, out),
        InstKind::Store { addr, align, .. } => {
            check_align(f, *align, i.span, out);
            require_ptr(f, addr, types, "store address", i.span, out);
        }
        InstKind::StoreBits {
            addr,
            unit,
            bits,
            align,
            ..
        } => {
            check_align(f, *align, i.span, out);
            check_bits(f, unit, *bits, i.span, out);
            require_ptr(f, addr, types, "store address", i.span, out);
        }
        InstKind::CopyMem {
            dst, src, align, ..
        } => {
            check_align(f, *align, i.span, out);
            require_ptr(f, dst, types, "copy destination", i.span, out);
            require_ptr(f, src, types, "copy source", i.span, out);
        }
        InstKind::SetMem { dst, .. } => {
            require_ptr(f, dst, types, "memset destination", i.span, out)
        }
        InstKind::AllocaDyn { align, .. } => check_align(f, *align, i.span, out),
        _ => {}
    }
}

fn check_rvalue_types(
    f: &Function,
    rv: &RValue,
    types: &IndexMap<ValueId, CTy>,
    span: Span,
    out: &mut Vec<VerifyError>,
) {
    match rv {
        RValue::Load { addr, align, .. } => {
            check_align(f, *align, span, out);
            require_ptr(f, addr, types, "load address", span, out);
        }
        RValue::LoadBits {
            addr,
            unit,
            bits,
            align,
            ..
        } => {
            check_align(f, *align, span, out);
            check_bits(f, unit, *bits, span, out);
            require_ptr(f, addr, types, "load address", span, out);
        }
        RValue::PtrAdd { base, .. } => require_ptr(f, base, types, "PtrAdd base", span, out),
        RValue::Cast { kind, from, to, .. } => check_cast(f, *kind, from, to, span, out),
        RValue::Shuffle { a, mask, .. } => {
            if let Some(CTy::Vector { lanes, .. }) = resolve(a, types) {
                for &m in mask {
                    if m >= 2 * lanes {
                        err(
                            out,
                            f,
                            VerifyErrorKind::BadLane,
                            span,
                            format!("shuffle index {m} exceeds 2*{lanes}"),
                        );
                    }
                }
            }
        }
        RValue::InsertLane { v, lane, .. } | RValue::ExtractLane { v, lane } => {
            if let Some(CTy::Vector { lanes, .. }) = resolve(v, types)
                && *lane >= lanes
            {
                err(
                    out,
                    f,
                    VerifyErrorKind::BadLane,
                    span,
                    format!("lane {lane} out of range for {lanes} lanes"),
                );
            }
        }
        _ => {}
    }
}

/// Rule 5 for casts, plus rule 12 for `Bitcast`.
fn check_cast(
    f: &Function,
    kind: CastKind,
    from: &CTy,
    to: &CTy,
    span: Span,
    out: &mut Vec<VerifyError>,
) {
    let (fw, tw) = (from.bit_width(), to.bit_width());
    let bad = |out: &mut Vec<VerifyError>, why: &str| {
        err(
            out,
            f,
            VerifyErrorKind::BadCast,
            span,
            format!("{kind:?} from {from:?} to {to:?}: {why}"),
        );
    };
    match kind {
        CastKind::Trunc | CastKind::FpTrunc => match (fw, tw) {
            (Some(a), Some(b)) if a > b => {}
            _ => bad(out, "must narrow strictly"),
        },
        CastKind::ZExt | CastKind::SExt | CastKind::FpExt => match (fw, tw) {
            (Some(a), Some(b)) if a < b => {}
            _ => bad(out, "must widen strictly"),
        },
        // Rule 12. This is the check that did not exist before spec review.
        CastKind::Bitcast => match (fw, tw) {
            (Some(a), Some(b)) if a == b => {}
            _ => bad(out, "must preserve total bit width"),
        },
        CastKind::PtrToInt => {
            if *from != CTy::Ptr || !to.is_int() {
                bad(out, "must go from Ptr to Int");
            }
        }
        CastKind::IntToPtr => {
            if !from.is_int() || *to != CTy::Ptr {
                bad(out, "must go from Int to Ptr");
            }
        }
        CastKind::FpToUi | CastKind::FpToSi => {
            if !matches!(from, CTy::Float(_)) || !to.is_int() {
                bad(out, "must go from Float to Int");
            }
        }
        CastKind::UiToFp | CastKind::SiToFp => {
            if !from.is_int() || !matches!(to, CTy::Float(_)) {
                bad(out, "must go from Int to Float");
            }
        }
    }
}

fn check_term_types(
    f: &Function,
    b: &Block,
    types: &IndexMap<ValueId, CTy>,
    out: &mut Vec<VerifyError>,
) {
    match &b.term {
        Terminator::Return(Some(o)) => {
            if let Some(t) = resolve(o, types)
                && t != CTy::Void
                && t != f.ret
            {
                err(
                    out,
                    f,
                    VerifyErrorKind::WidthMismatch,
                    b.span,
                    format!("returns {t:?} but `{}` is declared -> {:?}", f.name, f.ret),
                );
            }
        }
        Terminator::IndirectGoto { addr, .. } => {
            require_ptr(f, addr, types, "computed goto target", b.span, out)
        }
        _ => {}
    }
}
