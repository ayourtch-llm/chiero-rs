//! The CIR verifier (020 §8).
//!
//! A module that fails verification is never executed, so a missed rule lets malformed
//! IR reach the engine — where the symptom is a confusing wrong answer rather than a
//! clear error.

use indexmap::IndexSet;

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
    DuplicateId,
    UnknownId,
    CallArity,
    /// 020 §3: `funcs`/`globals` are *indexed by* `FuncId`/`GlobalId`. The printer
    /// resolves references positionally and the verifier resolves them by `.id`, so
    /// a permuted table prints the wrong name for every reference.
    IdNotIndex,
    /// 020 §4.3: an `Opaque` with no declared effect is a no-op, which would let a
    /// checker reason about code chiero refused to model.
    OpaqueWithoutEffect,
    /// 020 §9: a `Phi`'s incomings must be exactly its block's predecessors — no more, no
    /// fewer. A missing one leaves nothing to choose when that edge is taken; an extra one
    /// names an edge that does not exist.
    PhiPredecessorMismatch,
    /// 020 §9: phis sit at the top of a block. A phi's value is chosen by the edge that
    /// was taken, and after an ordinary instruction that is no longer the current fact.
    PhiNotAtTop,
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
    check_module_identity(m, &mut out);
    for f in &m.funcs {
        verify_function(m, f, &mut out);
    }
    out
}

/// Cross-function identity. `verify_function` cannot see any of this, so before the
/// module was threaded through, two globals with one id, two functions with one name,
/// and every dangling reference produced no error at all.
fn check_module_identity(m: &Module, out: &mut Vec<VerifyError>) {
    let anon = Function {
        id: FuncId(0),
        name: "<module>".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
        linkage: Linkage::External,
    };
    // 020 §3: the tables are *indexed by* `GlobalId`/`FuncId`. That was a convention
    // rather than an invariant, and the crate's two halves disagreed about it — the
    // printer resolves references positionally, the verifier resolves them by `.id`.
    // A permuted table verifies clean and then prints the wrong name for every
    // reference, so make the convention checkable.
    for (i, g) in m.globals.iter().enumerate() {
        if g.id.0 as usize != i {
            err(
                out,
                &anon,
                VerifyErrorKind::IdNotIndex,
                g.span,
                format!("global `{}` has {:?} but sits at index {i}", g.name, g.id),
            );
        }
    }
    for (i, f) in m.funcs.iter().enumerate() {
        if f.id.0 as usize != i {
            err(
                out,
                &anon,
                VerifyErrorKind::IdNotIndex,
                f.span,
                format!("function `{}` has {:?} but sits at index {i}", f.name, f.id),
            );
        }
    }
    // **Sets, not vectors.** These were `Vec` with `contains`, which is O(n^2) — invisible while
    // a module held dozens of entities. Once 9f7e575 stopped discarding the `always_inline`
    // wrappers in gcc's x86 headers, a VPP translation unit carries thousands of functions, and
    // one measured 673 s against ~1 s before. The scaling was the giveaway: 250 functions 303 ms,
    // 500 850 ms, 1000 3237 ms, with *no calls at all* — so the cost was per function, not per
    // call site, which ruled out the call-resolution scans that look like the obvious suspect.
    //
    // `IndexSet`, not `HashSet`: 001 §5 bans the std sets outright because their iteration order
    // is not stable across runs, and clippy enforces it. Nothing here iterates — but a set that
    // *is* iterated later, by someone who did not know why the type was chosen, is exactly the
    // non-determinism the rule exists to prevent, and the lookup cost is the same.
    let mut gids: IndexSet<GlobalId> = IndexSet::new();
    let mut gnames: IndexSet<&str> = IndexSet::new();
    for g in &m.globals {
        if !gids.insert(g.id) {
            err(
                out,
                &anon,
                VerifyErrorKind::DuplicateId,
                g.span,
                format!("{:?} is declared more than once", g.id),
            );
        }
        if !gnames.insert(&g.name) {
            err(
                out,
                &anon,
                VerifyErrorKind::DuplicateId,
                g.span,
                format!("global `{}` is declared more than once", g.name),
            );
        }
        // Rule 7 applies to globals too.
        check_align(&anon, g.align, g.span, out);
    }
    let mut fnames: IndexSet<&str> = IndexSet::new();
    for f in &m.funcs {
        // **No duplicate-*id* check here, and that is not an omission.** The rule above requires
        // `funcs[i].id == FuncId(i)`, so two functions can only share an id if one of them also
        // sits at the wrong index — `IdNotIndex` rejects every such module already, and the
        // duplicate check could never fire alone.
        //
        // Found by a sweep (wave 290) that disabled each of the verifier's forty rule sites in
        // turn: this one survived the whole workspace, and writing the fixture for it is what
        // showed why. A rule that cannot be the *only* thing wrong with a module is a rule no
        // fixture can isolate.
        //
        // A duplicate *name* is a different matter and is checked: name resolution takes the
        // first, so a `.cir` file naming the other simply calls something else.
        if !fnames.insert(&f.name) {
            err(
                out,
                f,
                VerifyErrorKind::DuplicateId,
                f.span,
                format!("function `{}` is declared more than once", f.name),
            );
        }
    }
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

fn verify_function(m: &Module, f: &Function, out: &mut Vec<VerifyError>) {
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
    check_phis(f, out);

    check_structural_identity(f, out);
    check_references(m, f, out);
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

/// Ids must be unique and resolvable. A duplicate `BlockId` is not a crash but a
/// silently *wrong* execution: `Function::block()` is a linear find, so the second block
/// is unreachable and control lands in the first.
fn check_structural_identity(f: &Function, out: &mut Vec<VerifyError>) {
    // Sets, for the reason `check_module_identity` records above: `Vec` + `contains` is
    // O(n²), and a function with thousands of blocks is ordinary in real C.
    let mut seen_blocks: IndexSet<BlockId> = IndexSet::new();
    for b in &f.blocks {
        if !seen_blocks.insert(b.id) {
            err(
                out,
                f,
                VerifyErrorKind::DuplicateId,
                b.span,
                format!("{:?} is declared more than once", b.id),
            );
        }
    }
    let mut seen_allocas: IndexSet<AllocaId> = IndexSet::new();
    for a in &f.allocas {
        if !seen_allocas.insert(a.id) {
            err(
                out,
                f,
                VerifyErrorKind::DuplicateId,
                a.span,
                format!("{:?} is declared more than once", a.id),
            );
        }
        // Rule 7 applies to declarations too, not only to accesses.
        check_align(f, a.align, a.span, out);
    }
    let mut seen_params: IndexSet<ValueId> = IndexSet::new();
    for p in &f.params {
        if !seen_params.insert(p.value) {
            err(
                out,
                f,
                VerifyErrorKind::DuplicateId,
                f.span,
                format!("parameter {:?} appears more than once", p.value),
            );
        }
    }
    // Rule 13's second half: a declaration claiming a runtime extent needs an
    // `AllocaDyn` to supply it, or nothing ever sizes the object.
    for a in &f.allocas {
        if a.count == crate::DYNAMIC_EXTENT {
            let supplied =
                f.blocks.iter().flat_map(|b| &b.insts).any(
                    |i| matches!(&i.kind, InstKind::AllocaDyn { alloca, .. } if *alloca == a.id),
                );
            if !supplied {
                err(
                    out,
                    f,
                    VerifyErrorKind::AllocaExtentMismatch,
                    a.span,
                    format!("{:?} has a runtime extent that no AllocaDyn supplies", a.id),
                );
            }
        }
    }
    // `AddrOfLocal` must name a declared alloca.
    for b in &f.blocks {
        for i in &b.insts {
            if let InstKind::Assign {
                rv: RValue::AddrOfLocal { alloca },
                ..
            } = &i.kind
                && !f.allocas.iter().any(|a| a.id == *alloca)
            {
                err(
                    out,
                    f,
                    VerifyErrorKind::UnknownId,
                    i.span,
                    format!("AddrOfLocal names undeclared {alloca:?}"),
                );
            }
        }
    }
}

/// Every `FuncId`/`GlobalId` a function names must exist, and a direct call must supply
/// the callee's arity.
fn check_references(m: &Module, f: &Function, out: &mut Vec<VerifyError>) {
    let known_func = |id: FuncId| m.funcs.iter().any(|x| x.id == id);
    let known_global = |id: GlobalId| m.globals.iter().any(|x| x.id == id);

    for b in &f.blocks {
        for i in &b.insts {
            match &i.kind {
                InstKind::Call {
                    callee: Callee::Direct(id),
                    args,
                    ..
                } => {
                    match m.funcs.iter().find(|x| x.id == *id) {
                        None => err(
                            out,
                            f,
                            VerifyErrorKind::UnknownId,
                            i.span,
                            format!("call to undeclared {id:?}"),
                        ),
                        Some(callee) => {
                            let n = callee.params.len();
                            // A variadic callee legitimately accepts extras; forbidding
                            // them would be worse than not checking at all.
                            let ok = if callee.variadic {
                                args.len() >= n
                            } else {
                                args.len() == n
                            };
                            if !ok {
                                err(
                                    out,
                                    f,
                                    VerifyErrorKind::CallArity,
                                    i.span,
                                    format!(
                                        "`{}` takes {}{} argument(s), given {}",
                                        callee.name,
                                        n,
                                        if callee.variadic { "+" } else { "" },
                                        args.len()
                                    ),
                                );
                            }
                        }
                    }
                }
                InstKind::Assign { rv, .. } => match rv {
                    RValue::AddrOfFunc(id) if !known_func(*id) => {
                        err(
                            out,
                            f,
                            VerifyErrorKind::UnknownId,
                            i.span,
                            format!("address of undeclared {id:?}"),
                        );
                    }
                    RValue::AddrOfGlobal { g } if !known_global(*g) => {
                        err(
                            out,
                            f,
                            VerifyErrorKind::UnknownId,
                            i.span,
                            format!("address of undeclared {g:?}"),
                        );
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

/// Rule 2.
fn check_block_refs(f: &Function, out: &mut Vec<VerifyError>) {
    let known: IndexSet<BlockId> = f.blocks.iter().map(|b| b.id).collect();
    for b in &f.blocks {
        for s in succs(&b.term) {
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
        if succs(&b.term).contains(&f.entry) {
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

thread_local! {
    /// How many terminators verification has examined, this thread, since the last reset.
    ///
    /// **A counter, because the duration was not a test.** `verifier.rs`'s scale test asserted a
    /// wall clock, and when `[profile.dev]` gained `opt-level = 2` every build got about 6.7x
    /// faster while the bound stayed put — so a mutant restoring one of the removed per-block
    /// scans came in *under* it and passed. A wall-clock assertion silently weakens whenever the
    /// build gets faster, and nobody edits the test.
    ///
    /// This number is the same on every machine, at any load, under any profile. It counts the
    /// thing that actually differs: examining every block's terminator **once per function** is
    /// linear, and doing it **once per block** is quadratic.
    static TERMINATORS_EXAMINED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `t.successors()`, counted. Every enumeration of a block's successors inside verification goes
/// through here — that is what makes the count track the *scan*, rather than the one site a fix
/// happened to hoist it to. A mutant that reintroduces a per-block scan increments per block by
/// construction.
fn succs(t: &Terminator) -> Vec<BlockId> {
    TERMINATORS_EXAMINED.with(|c| c.set(c.get() + 1));
    t.successors()
}

/// Terminators examined since [`reset_terminators_examined`]. Test-only observability; see
/// `TERMINATORS_EXAMINED` for why it exists at all.
#[doc(hidden)]
pub fn terminators_examined() -> u64 {
    TERMINATORS_EXAMINED.with(std::cell::Cell::get)
}

#[doc(hidden)]
pub fn reset_terminators_examined() {
    TERMINATORS_EXAMINED.with(|c| c.set(0));
}

/// The blocks reachable from entry, **as a set**.
///
/// It returned a `Vec` and probed it with `contains`, which is O(blocks²) — and every caller
/// then probed *that* `Vec` once per block, for another O(blocks²) each. Three of the eight
/// scans this file's second performance pass removed were this one value.
fn reachable_blocks(f: &Function) -> IndexSet<BlockId> {
    let mut seen: IndexSet<BlockId> = IndexSet::new();
    seen.insert(f.entry);
    // **`Function::block` is a linear find** (its own doc says so), and calling it once per
    // block popped is another O(blocks²) hiding behind a method call rather than behind a
    // `contains`. One index, built once.
    let by_id: IndexMap<BlockId, &Block> = f.blocks.iter().map(|b| (b.id, b)).collect();
    let mut stack = vec![f.entry];
    while let Some(b) = stack.pop() {
        let Some(blk) = by_id.get(&b).copied() else {
            continue;
        };
        for s in succs(&blk.term) {
            if seen.insert(s) {
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
            for (dst, ty) in defined_by(inst, &types) {
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
    // Rule 3 calls an unreachable block a *warning*, but a predecessor-less block is
    // dominated only by itself, so any use of an entry value inside it would also raise
    // a hard `UseNotDominated` — making "unreachable C code is legal" false for anything
    // but an empty dead block. Skip the dominance scan there; the warning stands alone.
    let reachable = reachable_blocks(f);

    for b in &f.blocks {
        // Reachability gates *dominance only*. Alignment, cast shape and bit ranges
        // have nothing to do with it, and dead code is legal, so it reaches the
        // engine's data structures like any other block.
        let live = reachable.contains(&b.id);
        for (i, inst) in b.insts.iter().enumerate() {
            if live {
                for op in operands_of(inst) {
                    check_dominated(f, b.id, i + 1, op, &defs, &doms, inst.span, out);
                }
            }
            check_inst_types(f, inst, &types, out);
        }
        if live {
            for op in term_operands(&b.term) {
                // A terminator is positioned after every instruction in its block.
                check_dominated(f, b.id, usize::MAX, op, &defs, &doms, b.span, out);
            }
        }
        check_term_types(f, b, &types, out);
    }
}

/// Types produced by an instruction, resolved against the environment built so far.
///
/// Resolution matters: an unresolved `Value` records as `Void`, and `Void` is the escape
/// hatch that makes `require_ptr` and the lane checks silently stop checking. One `%1 =
/// %0` would otherwise erase a pointer's type and disable rule 6 for everything downstream.
/// 020 §9's two structural rules for `Phi`.
///
/// Checked here rather than inside the dominance walk because both are facts about a
/// block's *edges* and its instruction order, neither of which that walk is looking at.
fn check_phis(f: &Function, out: &mut Vec<VerifyError>) {
    // **Built once**, not rebuilt per block. This scan is the same defect `dominators` had —
    // in the same file, fixed in the same session, and this copy kept it for a wave. When a
    // fix is about a *shape*, grep the file for the shape before calling it done.
    //
    // A block may branch to the same successor twice (`br c, bb1, bb1`), and that is **one**
    // edge for a phi's purposes: there is one value to choose no matter which arm the printer
    // wrote first. So the per-block list is deduped, exactly as before.
    let mut preds_of: IndexMap<BlockId, Vec<BlockId>> =
        f.blocks.iter().map(|b| (b.id, Vec::new())).collect();
    for p in &f.blocks {
        for s in succs(&p.term) {
            if let Some(list) = preds_of.get_mut(&s) {
                list.push(p.id);
            }
        }
    }
    for list in preds_of.values_mut() {
        list.sort_by_key(|x: &BlockId| x.0);
        list.dedup();
    }
    for b in &f.blocks {
        let preds: Vec<BlockId> = preds_of.get(&b.id).cloned().unwrap_or_default();

        let mut seen_ordinary = false;
        for i in &b.insts {
            let InstKind::Phi { dst, incomings, .. } = &i.kind else {
                // A marker is not an ordinary instruction for this purpose: lowering emits
                // `.line` and `.scope` markers at a block's top, and a phi after one is
                // still at the top in every sense that matters.
                if !matches!(i.kind, InstKind::Marker(_)) {
                    seen_ordinary = true;
                }
                continue;
            };
            if seen_ordinary {
                err(
                    out,
                    f,
                    VerifyErrorKind::PhiNotAtTop,
                    i.span,
                    format!(
                        "`%{}` follows an ordinary instruction in bb{}",
                        dst.0, b.id.0
                    ),
                );
            }
            let mut got: Vec<BlockId> = incomings.iter().map(|(p, _)| *p).collect();
            got.sort_by_key(|x| x.0);
            got.dedup();
            if got != preds {
                err(
                    out,
                    f,
                    VerifyErrorKind::PhiPredecessorMismatch,
                    i.span,
                    format!(
                        "`%{}` has incomings from {:?} but bb{} is entered from {:?}",
                        dst.0,
                        got.iter().map(|x| x.0).collect::<Vec<_>>(),
                        b.id.0,
                        preds.iter().map(|x| x.0).collect::<Vec<_>>()
                    ),
                );
            }
        }
    }
}

fn defined_by(i: &Inst, types: &IndexMap<ValueId, CTy>) -> Vec<(ValueId, CTy)> {
    match &i.kind {
        InstKind::Assign { dst, rv } => vec![(*dst, rvalue_type_in(rv, types))],
        // A call's result type is unknown here (the callee lives in the module, not the
        // function), so it stays Void — but a *pointer-returning* call is the common
        // source of pointers in C, so this is recorded as a known gap rather than a
        // silently-correct answer.
        InstKind::Call { dst: Some(d), .. } => vec![(*d, CTy::Void)],
        InstKind::AllocaDyn { dst, .. } => vec![(*dst, CTy::Ptr)],
        InstKind::VaArg { dst, ty, .. } => vec![(*dst, ty.clone())],
        InstKind::Phi { dst, ty, .. } => vec![(*dst, ty.clone())],
        // Unlike a call, an `Opaque` *declares* the type of each output, so these are
        // known rather than a gap.
        InstKind::Opaque { dsts, .. } => dsts.clone(),
        _ => Vec::new(),
    }
}

/// The type an `RValue` produces. `Cmp` yields `Int(1)` regardless of its operand type
/// — conflating the two is the mistake 020 §8 rule 5 names.
fn rvalue_type_in(rv: &RValue, types: &IndexMap<ValueId, CTy>) -> CTy {
    let ot = |o: &Operand| resolve(o, types).unwrap_or(CTy::Void);
    match rv {
        RValue::Use(o) => ot(o),
        RValue::Load { ty, .. } => ty.clone(),
        RValue::LoadBits { unit, .. } => unit.clone(),
        RValue::Bin { ty, .. } | RValue::Un { ty, .. } => ty.clone(),
        RValue::Cmp { .. } => CTy::Int(1),
        RValue::Cast { to, .. } => to.clone(),
        RValue::Select { t, .. } => ot(t),
        RValue::PtrAdd { .. }
        | RValue::AddrOfLocal { .. }
        | RValue::AddrOfGlobal { .. }
        | RValue::AddrOfFunc(_) => CTy::Ptr,
        RValue::Shuffle { a, .. } => ot(a),
        RValue::InsertLane { v, .. } => ot(v),
        RValue::ExtractLane { v, .. } => match ot(v) {
            CTy::Vector { elem, .. } => *elem,
            other => other,
        },
        RValue::Splat { elem, lanes } => CTy::Vector {
            elem: Box::new(ot(elem)),
            lanes: *lanes,
        },
        RValue::Fresh { ty } => ty.clone(),
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
        InstKind::Opaque { writes, reads, .. } => {
            for w in writes {
                v.extend([w.addr.clone(), w.size.clone()]);
            }
            v.extend(reads.iter().cloned());
        }
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
        // **A phi contributes no operands here, and that is the rule rather than an
        // omission.** Its incomings are evaluated on their *edges*, not at the phi, so
        // feeding them to the dominance walk would reject every correct phi ever written:
        // an incoming from `bb1` is defined in `bb1`, which does not dominate the join.
        // `check_phis` verifies them against the edges instead.
        InstKind::Phi { .. } => {}
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

/// Immediate-dominator map by iterative dataflow.
///
/// ⚠️ **This used to say "small graphs, so simplicity beats Lengauer-Tarjan here", and that was
/// an assumption about the input written as a justification.** Measured: a 3001-block function
/// — a run of a thousand `if`s, which real C produces — took **11.5 s in a release build and
/// 158 s in a debug one**, and each doubling of the block count cost about six times the
/// previous. It is what killed the plugin sweep's two `timeout` entries, and no engine budget
/// could reach it because this runs before a single instruction executes.
///
/// The algorithm is still iterative dataflow; what was quadratic was the bookkeeping around it.
/// Three things, none of them clever:
///
/// - **The predecessor map is built once**, not rebuilt for every block on every round. It was
///   a scan of all blocks per block per round, with `successors().contains()` a second scan
///   inside that.
/// - **`reachable` is a set**, not a `Vec` scanned linearly at each of those visits.
/// - **The meet is a sorted-set intersection**, not `retain(|x| dom[p].contains(x))` — which is
///   linear in a set that starts out as *every block in the function*, so the first round alone
///   was cubic. Every `dom` entry is kept sorted, which it already was on the way out; now it
///   is sorted on the way in too, and the intersection is a single merge.
///
/// Same answer, and `verify`'s own suite is what says so: this function decides whether a use
/// is dominated by its definition, and every dominance rejection in `verifier.rs` goes through
/// it.
fn dominators(f: &Function) -> IndexMap<BlockId, Vec<BlockId>> {
    let ids: Vec<BlockId> = f.blocks.iter().map(|b| b.id).collect();
    // Unreachable predecessors are excluded from the meet (Cooper-Harvey-Kennedy).
    // A dead block is dominated by nothing but itself, so meeting `{dead}` into a live
    // join empties the set and a value defined in entry stops dominating its use — a
    // hard error on the ubiquitous C shape of dead code falling into a live join.
    let reachable: std::collections::BTreeSet<BlockId> = reachable_blocks(f).into_iter().collect();
    // One pass over the blocks, rather than one per block per round.
    let mut preds: IndexMap<BlockId, Vec<BlockId>> = ids.iter().map(|&b| (b, Vec::new())).collect();
    for p in &f.blocks {
        if !reachable.contains(&p.id) {
            continue;
        }
        for s in succs(&p.term) {
            if let Some(list) = preds.get_mut(&s) {
                list.push(p.id);
            }
        }
    }
    // **Sorted from the start.** `ids` is the initial "everything" set and the meet below is a
    // merge, so both sides have to be ordered for the whole loop, not only where it used to be
    // sorted on the way out.
    let mut sorted_ids = ids.clone();
    sorted_ids.sort_unstable();
    let mut dom: IndexMap<BlockId, Vec<BlockId>> = IndexMap::new();
    for &b in &ids {
        dom.insert(
            b,
            if b == f.entry {
                vec![f.entry]
            } else {
                sorted_ids.clone()
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
            let bpreds = &preds[&b];
            let mut new: Vec<BlockId> = match bpreds.first() {
                Some(p) => dom[p].clone(),
                None => vec![], // unreachable block: dominated by nothing but itself
            };
            for p in bpreds.iter().skip(1) {
                new = intersect_sorted(&new, &dom[p]);
            }
            if let Err(at) = new.binary_search(&b) {
                new.insert(at, b);
            }
            if dom[&b] != new {
                dom.insert(b, new);
                changed = true;
            }
        }
    }
    dom
}

/// Intersection of two ascending lists, in one pass.
///
/// The thing it replaces — `new.retain(|x| other.contains(x))` — is a linear scan of `other`
/// for every element of `new`, and `new` starts as every block in the function. Both sides are
/// already ordered here, which is what makes the merge available for free.
fn intersect_sorted(a: &[BlockId], b: &[BlockId]) -> Vec<BlockId> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len().min(b.len()));
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    out
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
    // **Checked addition.** `BitRange { off: u32::MAX, width: 4 }` panicked in debug and
    // *wrapped* in release — `u32::MAX + 4` is 2, so `2 > 32` is false and `verify`
    // **accepted** a malformed range, which then reached the engine's bit API. The text
    // parser cannot produce it (`width` is a saturating difference), so this is reachable
    // only from a programmatically built module — which is what `chiero-lower` will be.
    let end = bits.off.checked_add(bits.width);
    if bits.width == 0 || end.is_none_or(|e| e > w) {
        err(
            out,
            f,
            VerifyErrorKind::BadBitRange,
            span,
            format!(
                "bit range {}..{} does not fit in {w} bits",
                bits.off,
                // The *message* overflowed too, which is how the fix to the condition
                // moved the panic three lines down rather than removing it.
                end.map_or_else(|| "overflow".to_string(), |e| e.to_string())
            ),
        );
    }
}

/// Rule 5. Skips unresolved values (recorded as `Void`), which is a known gap rather
/// than a claim — see `defined_by`.
fn require_ty(
    f: &Function,
    o: &Operand,
    want: &CTy,
    types: &IndexMap<ValueId, CTy>,
    what: &str,
    span: Span,
    out: &mut Vec<VerifyError>,
) {
    if let Some(got) = resolve(o, types)
        && got != CTy::Void
        && got != *want
    {
        err(
            out,
            f,
            VerifyErrorKind::WidthMismatch,
            span,
            format!("{what} operand is {got:?}, declared {want:?}"),
        );
    }
}

/// Rule 5: a size operand is an integer. Which width is target-dependent, so only
/// the *kind* is checked here.
fn require_int(
    f: &Function,
    o: &Operand,
    types: &IndexMap<ValueId, CTy>,
    what: &str,
    span: Span,
    out: &mut Vec<VerifyError>,
) {
    if let Some(t) = resolve(o, types)
        && t != CTy::Void
        && !matches!(t, CTy::Int(_))
    {
        err(
            out,
            f,
            VerifyErrorKind::WidthMismatch,
            span,
            format!("{what} must be integer-typed, got {t:?}"),
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
        // Rule 5 reaches the *value* too. A `store i32` of an i64 leaves the memory
        // model to invent a truncation rule, which is exactly the malformed IR the
        // verifier exists to stop before it becomes a confusing wrong answer.
        InstKind::Store {
            addr,
            val,
            ty,
            align,
            ..
        } => {
            check_align(f, *align, i.span, out);
            require_ptr(f, addr, types, "store address", i.span, out);
            require_ty(f, val, ty, types, "store value", i.span, out);
        }
        InstKind::StoreBits {
            addr,
            val,
            unit,
            bits,
            align,
            ..
        } => {
            check_align(f, *align, i.span, out);
            check_bits(f, unit, *bits, i.span, out);
            require_ptr(f, addr, types, "store address", i.span, out);
            require_ty(f, val, unit, types, "bitfield store value", i.span, out);
        }
        InstKind::CopyMem {
            dst,
            src,
            size,
            align,
            ..
        } => {
            check_align(f, *align, i.span, out);
            require_ptr(f, dst, types, "copy destination", i.span, out);
            require_ptr(f, src, types, "copy source", i.span, out);
            require_int(f, size, types, "copy size", i.span, out);
        }
        InstKind::SetMem { dst, byte, size } => {
            require_ptr(f, dst, types, "memset destination", i.span, out);
            require_ty(f, byte, &CTy::Int(8), types, "memset fill", i.span, out);
            require_int(f, size, types, "memset size", i.span, out);
        }
        InstKind::AllocaDyn { align, .. } => check_align(f, *align, i.span, out),
        // 020 §4.4.1: a `va_list` is a real addressable `MemObject`, so every operand
        // that names one is a pointer. `check_inst_types` had no arm for these at all,
        // which meant `vastart 0i32` verified clean.
        InstKind::VaStart { list } | InstKind::VaEnd { list } => {
            require_ptr(f, list, types, "va_list", i.span, out)
        }
        InstKind::VaArg { list, ty, .. } => {
            require_ptr(f, list, types, "va_list", i.span, out);
            // `Void` is the escape hatch for values whose type cannot be resolved, so a
            // value declared `Void` silently disables rules 5 and 6 for everything
            // derived from it. `VaArg` declares its own result type, so it could hand one
            // out on request — and a `va_arg` of `void` is meaningless in C regardless.
            if *ty == CTy::Void {
                err(
                    out,
                    f,
                    VerifyErrorKind::WidthMismatch,
                    i.span,
                    "vaarg result type is void, which disables type checking downstream"
                        .to_string(),
                );
            }
        }
        InstKind::VaCopy { dst, src } => {
            require_ptr(f, dst, types, "va_copy destination", i.span, out);
            require_ptr(f, src, types, "va_copy source", i.span, out);
        }
        InstKind::Opaque {
            dsts,
            writes,
            reads,
            ..
        } => {
            for w in writes {
                require_ptr(f, &w.addr, types, "opaque write address", i.span, out);
                require_int(f, &w.size, types, "opaque write size", i.span, out);
            }
            // **020 §4.3: an `Opaque` must never be silently equivalent to a no-op.**
            // A no-op would let a checker "prove" something about code it did not
            // understand — the exact failure the construct exists to prevent — so an
            // `Opaque` declaring no outputs, no clobbers and no inputs is rejected
            // rather than accepted and quietly ignored.
            if dsts.is_empty() && writes.is_empty() && reads.is_empty() {
                err(
                    out,
                    f,
                    VerifyErrorKind::OpaqueWithoutEffect,
                    i.span,
                    "opaque declares no dsts, no writes and no reads, so it is a no-op".to_string(),
                );
            }
        }
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
        // Rule 5: operand widths match the operation's declared `ty`. This was entirely
        // absent — `Bin`, `Un`, `Cmp` and `Select` were never checked, so `add i32` over
        // an i8 and an i64 verified clean.
        RValue::Bin {
            op,
            a,
            b,
            ty: t,
            signed: _,
        } => {
            let name = format!("{op:?}");
            require_ty(f, a, t, types, &name, span, out);
            require_ty(f, b, t, types, &name, span, out);
        }
        RValue::Un { op, a, ty: t } => require_ty(f, a, t, types, &format!("{op:?}"), span, out),
        RValue::Cmp { op, a, b, ty: t } => {
            // `ty` is the *operand* type here; the result is always Int(1).
            let name = format!("{op:?}");
            require_ty(f, a, t, types, &name, span, out);
            require_ty(f, b, t, types, &name, span, out);
        }
        RValue::Select { cond, t: tv, f: fv } => {
            require_ty(f, cond, &CTy::Int(1), types, "select condition", span, out);
            if let (Some(a), Some(b)) = (resolve(tv, types), resolve(fv, types))
                && a != CTy::Void
                && b != CTy::Void
                && a != b
            {
                err(
                    out,
                    f,
                    VerifyErrorKind::WidthMismatch,
                    span,
                    format!("select arms disagree: {a:?} vs {b:?}"),
                );
            }
        }
        RValue::Cast { kind, a, from, to } => {
            // The declared `from` is not taken on faith. A mislabelled source type is the
            // same class of hole as an unchecked `va_list`, and on `PtrToInt`/`IntToPtr`
            // it is exactly the pair 021 §7.1 makes carry provenance.
            require_ty(f, a, from, types, "cast source", span, out);
            check_cast(f, *kind, from, to, span, out)
        }
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
    // Width alone is not the shape. `trunc f64 -> i32` is `fptosi` wearing the wrong
    // name, and the two compute completely different values — so the domain must match
    // as well as the direction.
    let int_to_int = from.is_int() && to.is_int();
    let fp_to_fp = matches!(from, CTy::Float(_)) && matches!(to, CTy::Float(_));
    match kind {
        CastKind::Trunc => match (fw, tw) {
            (Some(a), Some(b)) if a > b && int_to_int => {}
            _ => bad(out, "must narrow strictly, integer to integer"),
        },
        CastKind::FpTrunc => match (fw, tw) {
            (Some(a), Some(b)) if a > b && fp_to_fp => {}
            _ => bad(out, "must narrow strictly, float to float"),
        },
        CastKind::ZExt | CastKind::SExt => match (fw, tw) {
            (Some(a), Some(b)) if a < b && int_to_int => {}
            _ => bad(out, "must widen strictly, integer to integer"),
        },
        CastKind::FpExt => match (fw, tw) {
            (Some(a), Some(b)) if a < b && fp_to_fp => {}
            _ => bad(out, "must widen strictly, float to float"),
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
        Terminator::Return(None) => {
            if f.ret != CTy::Void {
                err(
                    out,
                    f,
                    VerifyErrorKind::WidthMismatch,
                    b.span,
                    format!("bare `ret` from `{}` declared -> {:?}", f.name, f.ret),
                );
            }
        }
        Terminator::Br { cond, .. } => require_ty(
            f,
            cond,
            &CTy::Int(1),
            types,
            "branch condition",
            b.span,
            out,
        ),
        Terminator::Switch { scrut, ty: t, .. } => {
            require_ty(f, scrut, t, types, "switch scrutinee", b.span, out)
        }
        Terminator::IndirectGoto { addr, .. } => {
            require_ptr(f, addr, types, "computed goto target", b.span, out)
        }
        _ => {}
    }
}
