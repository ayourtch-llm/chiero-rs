//! `chiero-lower` — the typed AST to CIR. See `docs/specs/015-lowering.md`.
//!
//! **Every construct lowers to a fixed shape** (015 §1). Two lowerings of one construct
//! must produce identical CIR, because golden `.cir` files are contracts (020 §6) and the
//! differential harness diffs them — so a choice left free here is a golden that changes
//! for no reason.
//!
//! **Lowering never infers a conversion.** 014 §5 already made every implicit conversion
//! an explicit `Cast` node; if this crate finds itself needing one, that is a
//! `chiero-sema` bug and not a lowering fix.

use chiero_ast::Ast;
use chiero_cir::Module;
use chiero_sema::{Analysis, SymbolText};
use chiero_span::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowerDiagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Debug)]
pub struct Lowered {
    pub module: Module,
    /// 015 §7: constructs lowering **refuses** rather than lowering wrongly. A function
    /// that cannot be represented is absent from the module and named here.
    pub diagnostics: Vec<LowerDiagnostic>,
}

/// Lower one translation unit **without** a `SourceMap`.
///
/// `gcov_lines` is left empty: 015 §5's rule is a computation over `expansion_loc`, and
/// without a map there is nothing to resolve. That is the honest answer rather than a
/// guess — and it is exactly the hand-written-`.cir` case 015 §5 describes, where the
/// `.line` directive populates the field directly instead.
pub fn lower_tu(ast: &Ast, analysis: &Analysis, names: &dyn SymbolText) -> Lowered {
    lower(ast, analysis, names, None)
}

/// Lower one translation unit and compute `gcov_lines` (015 §5).
pub fn lower_tu_with_map(
    ast: &Ast,
    analysis: &Analysis,
    names: &dyn SymbolText,
    map: &chiero_span::SourceMap,
) -> Lowered {
    lower(ast, analysis, names, Some(map))
}

fn lower(
    ast: &Ast,
    analysis: &Analysis,
    names: &dyn SymbolText,
    map: Option<&chiero_span::SourceMap>,
) -> Lowered {
    let mut cx = Lowerer {
        ast,
        analysis,
        names,
        module: Module {
            funcs: Vec::new(),
            globals: Vec::new(),
            config: None,
            metadata: IndexMap::new(),
        },
        diagnostics: Vec::new(),
        f: None,
        next_value: 0,
        next_func: 0,
        map,
        last_stmt_value: None,
        generated_depth: 0,
    };
    // **Two passes, and the first is not an optimization.** Every function is registered
    // with its real signature before any body is lowered, because a body can call a
    // function declared later in the file — and a callee invented on demand has no
    // parameter list, so the verifier rejects the call for arity. Registering first also
    // makes `FuncId`s follow *source* order rather than call order, which is what a
    // golden `.cir` file should record.
    for &item in ast.items() {
        cx.declare(item);
    }
    for &item in ast.items() {
        cx.item(item);
    }
    Lowered {
        module: cx.module,
        diagnostics: cx.diagnostics,
    }
}

// ---------------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------------

use chiero_ast::{DeclKind, ExprId, StmtId, StmtKind};
use chiero_cir::{
    AllocaDecl, AllocaId, BinOp as CBinOp, Block, BlockId, Body, CTy, Callee, Const, FnAttrs,
    FuncId, Function, Inst, InstKind, Lifetime, MarkerKind, Operand, Param, RValue, ScopeEvent,
    ScopeId, ScopeKind, Terminator, UnOp as CUnOp, ValueId, Volatility,
};
use chiero_sema::{Conversion, FloatKind, Ty, TyId, TypedId, TypedNode};
use indexmap::IndexMap;

/// The function currently being built.
struct FnState {
    id: FuncId,
    name: chiero_cir::Symbol,
    params: Vec<Param>,
    ret: CTy,
    variadic: bool,
    allocas: Vec<AllocaDecl>,
    blocks: Vec<Block>,
    entry: BlockId,
    /// The block instructions are appended to. Every emit goes here, so "which block am
    /// I in" is one piece of state rather than a parameter threaded everywhere.
    cur: BlockId,
    /// Local name → its stack slot. Every local is addressable: CIR is not SSA
    /// (020 §1.3), so a local is memory and a load/store, which is also what makes
    /// `&x` free.
    /// Local name → its slot **and its declared type**. The type is needed at every
    /// load: a slot's width is a property of the declaration, not of the expression
    /// reading it, and using the reader's width loaded four bytes from a one-byte slot.
    locals: IndexMap<chiero_span::Symbol, (AllocaId, CTy)>,
    next_alloca: u32,
    next_block: u32,
    /// The scopes currently open, innermost last. Exits are emitted from the top down, so
    /// a `return` or a `goto` out can unwind exactly the scopes it leaves.
    open_scopes: Vec<ScopeId>,
    next_scope: u32,
    /// `break` and `continue` targets for the enclosing loop or switch, innermost last.
    /// Each records the scope depth at the point the construct began, so an abrupt exit
    /// knows how many scopes it is leaving.
    breaks: Vec<(BlockId, usize)>,
    continues: Vec<(BlockId, usize)>,
    /// Label name → its block and the **scopes open at the label**.
    ///
    /// The open scopes, not a depth: resolving a forward `goto` needs the *ids* to emit
    /// exit markers for, and by the time the function is walked the scopes have been
    /// popped. A depth alone would tell you how many markers to emit and not which.
    labels: IndexMap<chiero_span::Symbol, (BlockId, Vec<ScopeId>)>,
    /// `goto`s emitted before their label was seen: the block to terminate, the target
    /// name, the scopes open at the jump, and the span.
    pending_gotos: Vec<(BlockId, chiero_span::Symbol, Vec<ScopeId>, Span)>,
    span: Span,
}

struct Lowerer<'a> {
    ast: &'a Ast,
    analysis: &'a Analysis,
    names: &'a dyn SymbolText,
    module: Module,
    diagnostics: Vec<LowerDiagnostic>,
    f: Option<FnState>,
    next_value: u32,
    next_func: u32,
    map: Option<&'a chiero_span::SourceMap>,
    /// The operand the most recent expression *statement* produced.
    ///
    /// A statement expression's value is its last expression statement's (015 §2.4), and
    /// `stmt` otherwise discards what an expression statement evaluated to — so the value
    /// is caught on the way past rather than by re-lowering the expression, which would
    /// run its side effects twice.
    last_stmt_value: Option<Operand>,
    /// Nonzero while emitting instructions lowering introduced rather than the source
    /// wrote. A counter, not a flag: the `&&` shape's bookkeeping can nest inside a
    /// scope's, and a flag would be cleared by the inner one on the way out.
    generated_depth: u32,
}

impl Lowerer<'_> {
    fn fs(&mut self) -> &mut FnState {
        self.f.as_mut().expect("inside a function")
    }

    fn new_value(&mut self) -> ValueId {
        let v = ValueId(self.next_value);
        self.next_value += 1;
        v
    }

    fn new_block(&mut self) -> BlockId {
        let fs = self.fs();
        let id = BlockId(fs.next_block);
        fs.next_block += 1;
        fs.blocks.push(Block {
            id,
            insts: Vec::new(),
            // A placeholder every block must overwrite before the function is finished.
            // `Unreachable` rather than a `Goto` to itself: an unfinished block that
            // escaped would be caught by the verifier instead of becoming an infinite
            // loop nobody notices.
            term: Terminator::Unreachable(chiero_cir::UnreachableReason::LoweringGap),
            gcov_lines: Default::default(),
            span: fs.span,
        });
        id
    }

    fn switch_to(&mut self, b: BlockId) {
        self.fs().cur = b;
    }

    fn emit(&mut self, kind: InstKind, span: Span) {
        let cur = self.fs().cur;
        let generated = self.generated_depth > 0;
        let fs = self.fs();
        let b = fs
            .blocks
            .iter_mut()
            .find(|b| b.id == cur)
            .expect("current block exists");
        b.insts.push(Inst {
            kind,
            span,
            generated,
        });
    }

    /// Run `f` with every instruction it emits marked compiler-generated.
    ///
    /// 020 contract 15 wants this **recorded** rather than inferred: "it had no source
    /// span" is a different fact, and a lowering bug that lost a span would be
    /// indistinguishable from a deliberately synthesized instruction.
    fn generated<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.generated_depth;
        self.generated_depth = saved + 1;
        let r = f(self);
        self.generated_depth = saved;
        r
    }

    fn set_term(&mut self, t: Terminator) {
        let cur = self.fs().cur;
        let fs = self.fs();
        let b = fs
            .blocks
            .iter_mut()
            .find(|b| b.id == cur)
            .expect("current block exists");
        b.term = t;
    }

    /// Open a scope and emit its `Enter` **here**, on the edge that is entering it.
    ///
    /// 015 §4 says "every entering edge", not "at the lexical top", because C gives
    /// automatic objects storage on entry into the block *however entered* (C11 6.2.4p6) —
    /// and a `switch` jumps straight past the top to a case label.
    fn enter_scope(&mut self, span: Span) -> ScopeId {
        let fs = self.fs();
        let id = ScopeId(fs.next_scope);
        fs.next_scope += 1;
        fs.open_scopes.push(id);
        self.generated(|s| {
            s.emit(
                InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    scope: id,
                    kind: ScopeKind::Enter,
                })),
                span,
            )
        });
        id
    }

    /// Close the innermost scope, emitting its `Exit`.
    fn exit_scope(&mut self, span: Span) {
        let Some(id) = self.fs().open_scopes.pop() else {
            return;
        };
        self.generated(|s| {
            s.emit(
                InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    scope: id,
                    kind: ScopeKind::Exit,
                })),
                span,
            )
        });
    }

    /// Emit `Exit` markers for every scope down to `depth`, **innermost first**, without
    /// closing them — an abrupt exit leaves scopes on one path while the fallthrough path
    /// still has them open.
    ///
    /// The order is the contract (015 contract 10): 021 retires objects as the markers
    /// arrive, and retiring an outer scope first frees storage the inner scope's objects
    /// are still inside.
    fn unwind_to(&mut self, depth: usize, span: Span) {
        let open: Vec<ScopeId> = self.fs().open_scopes[depth..].to_vec();
        for id in open.into_iter().rev() {
            self.generated(|s| {
                s.emit(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: id,
                        kind: ScopeKind::Exit,
                    })),
                    span,
                )
            });
        }
    }

    fn scope_depth(&mut self) -> usize {
        self.fs().open_scopes.len()
    }

    /// A sequence point is lowering's own bookkeeping, not a statement the source
    /// wrote — gcov has no counter for it.
    fn seq_point(&mut self, span: Span) {
        self.generated(|s| s.emit(InstKind::Marker(MarkerKind::SeqPoint), span));
    }

    fn text(&self, sym: chiero_span::Symbol) -> Option<&str> {
        self.names.text(sym)
    }

    /// CIR names are `Arc<str>` (020), not the per-TU `Symbol` the AST uses — a CIR
    /// module outlives the interner that produced it, and a golden `.cir` file has to be
    /// readable without one.
    fn sym(&self, s: chiero_span::Symbol) -> Option<chiero_cir::Symbol> {
        self.names.text(s).map(std::sync::Arc::from)
    }

    /// A slot for an object of semantic type `sty`.
    ///
    /// **An aggregate is `count` bytes, not one `CTy::Ptr`.** `cty` maps a record to
    /// `CTy::Ptr` because CIR has no aggregate values (020 §1.4) — but that is the type of
    /// a *value*, and a slot needs an *extent*. Sizing the slot by the value type gave a
    /// 12-byte struct an 8-byte object, and the first store past byte 8 faulted: the
    /// engine reported `Crashed` on a module the verifier had accepted.
    fn alloca_for(
        &mut self,
        sty: TyId,
        align: u64,
        name: Option<chiero_cir::Symbol>,
        span: Span,
    ) -> AllocaId {
        let is_aggregate = matches!(
            self.analysis.ty(sty),
            Ty::Record(_) | Ty::Array { .. } | Ty::Vector { .. }
        );
        if is_aggregate {
            let bytes = self.analysis.size_of(sty).unwrap_or(0).max(1);
            return self.alloca_n(CTy::Int(8), bytes, align, name, span);
        }
        let ty = self.cty(sty);
        self.alloca_n(ty, 1, align, name, span)
    }

    /// A slot whose extent an `AllocaDyn` supplies (020 §3's `DYNAMIC_EXTENT`).
    fn alloca_dynamic(
        &mut self,
        align: u64,
        name: Option<chiero_cir::Symbol>,
        span: Span,
    ) -> AllocaId {
        self.alloca_n(CTy::Int(8), chiero_cir::DYNAMIC_EXTENT, align, name, span)
    }

    /// `(element type, count expression)` if `t` is a variable-length array.
    fn vla_of(&mut self, t: TyId) -> Option<(TyId, ExprId)> {
        match self.analysis.ty(t) {
            Ty::Array {
                elem,
                len: chiero_sema::ArrayLen::Vla(e),
            } => Some((*elem, *e)),
            _ => None,
        }
    }

    fn alloca(
        &mut self,
        ty: CTy,
        align: u64,
        name: Option<chiero_cir::Symbol>,
        span: Span,
    ) -> AllocaId {
        self.alloca_n(ty, 1, align, name, span)
    }

    fn alloca_n(
        &mut self,
        ty: CTy,
        count: u64,
        align: u64,
        name: Option<chiero_cir::Symbol>,
        span: Span,
    ) -> AllocaId {
        let fs = self.fs();
        let id = AllocaId(fs.next_alloca);
        fs.next_alloca += 1;
        let scope = fs.open_scopes.last().copied().unwrap_or(ScopeId(0));
        fs.allocas.push(AllocaDecl {
            id,
            ty,
            count,
            align,
            // **The scope actually open**, not a constant. 021 §4 creates and retires
            // objects by scope, so a slot filed under scope 0 would outlive its block and
            // a `for` index would be indistinguishable from a body local.
            scope,
            lifetime: Lifetime::Scope,
            name,
            span,
        });
        id
    }

    /// Register a function's signature without lowering its body.
    fn declare(&mut self, id: chiero_ast::DeclId) {
        let DeclKind::Func { name, ty, .. } = self.ast.decl(id).kind.clone() else {
            return;
        };
        let Some(text) = self.sym(name) else { return };
        if self.module.funcs.iter().any(|f| f.name == text) {
            return;
        }
        let span = self.ast.decl(id).span;
        let sema_ty = self.analysis.ty_of_decl(id);
        let (ret, variadic) = match sema_ty.map(|t| self.analysis.ty(t).clone()) {
            Some(Ty::Func { ret, variadic, .. }) => (self.cty(ret), variadic),
            _ => (CTy::Void, false),
        };
        let params: Vec<chiero_ast::DeclId> = match &self.ast.ty(ty).kind {
            chiero_ast::TypeKind::Func { params, .. } => params.clone(),
            _ => Vec::new(),
        };
        let cparams = params
            .iter()
            .map(|&p| {
                let sty = self
                    .analysis
                    .ty_of_decl(p)
                    .unwrap_or_else(|| self.error_ty());
                Param {
                    value: self.new_value(),
                    ty: self.cty(sty),
                }
            })
            .collect();
        let id_ = FuncId(self.next_func);
        self.next_func += 1;
        self.module.funcs.push(Function {
            id: id_,
            name: text,
            params: cparams,
            ret,
            variadic,
            allocas: Vec::new(),
            blocks: Vec::new(),
            entry: BlockId(0),
            attrs: FnAttrs::default(),
            body: Body::Declared,
            span,
        });
    }

    fn item(&mut self, id: chiero_ast::DeclId) {
        let decl = self.ast.decl(id).kind.clone();
        if let DeclKind::Func {
            name,
            ty,
            body: Some(body),
            ..
        } = decl
        {
            self.function(id, name, ty, body);
        }
    }

    fn function(
        &mut self,
        decl: chiero_ast::DeclId,
        name: chiero_span::Symbol,
        ty: chiero_ast::TypeId,
        body: StmtId,
    ) {
        let span = self.ast.decl(decl).span;
        let cir_name = self.sym(name).unwrap_or_else(|| std::sync::Arc::from("?"));
        let diags_before = self.diagnostics.len();
        // The declaration pass already reserved this function's slot and `FuncId`.
        let Some(slot) = self.module.funcs.iter().position(|f| f.name == cir_name) else {
            return;
        };
        let fid = self.module.funcs[slot].id;
        let sema_ty = self.analysis.ty_of_decl(decl);
        let (ret, variadic) = match sema_ty.map(|t| self.analysis.ty(t).clone()) {
            Some(Ty::Func { ret, variadic, .. }) => (self.cty(ret), variadic),
            _ => (CTy::Void, false),
        };
        self.f = Some(FnState {
            id: fid,
            name: cir_name.clone(),
            params: Vec::new(),
            ret: ret.clone(),
            variadic,
            allocas: Vec::new(),
            blocks: Vec::new(),
            entry: BlockId(0),
            cur: BlockId(0),
            locals: IndexMap::new(),
            next_alloca: 0,
            next_block: 0,
            open_scopes: Vec::new(),
            next_scope: 0,
            breaks: Vec::new(),
            continues: Vec::new(),
            labels: IndexMap::new(),
            pending_gotos: Vec::new(),
            span,
        });
        let entry = self.new_block();
        self.fs().entry = entry;
        self.switch_to(entry);

        // Parameters get a slot each and are stored into it on entry, so the body reads
        // them exactly the way it reads any other local. Without that, `&param` would
        // have nowhere to point.
        let params: Vec<chiero_ast::DeclId> = match &self.ast.ty(ty).kind {
            chiero_ast::TypeKind::Func { params, .. } => params.clone(),
            _ => Vec::new(),
        };
        for p in params {
            let DeclKind::Var {
                name: pname,
                ty: pty,
                ..
            } = self.ast.decl(p).kind.clone()
            else {
                continue;
            };
            let sty = self
                .analysis
                .ty_of_decl(p)
                .unwrap_or_else(|| self.error_ty());
            let _ = pty;
            let cty = self.cty(sty);
            let v = self.new_value();
            self.fs().params.push(Param {
                value: v,
                ty: cty.clone(),
            });
            let align = self.analysis.align_of(sty).unwrap_or(1).max(1);
            let pn_text = pname.and_then(|n| self.sym(n));
            let slot = self.alloca(cty.clone(), align, pn_text, span);
            if let Some(pn) = pname {
                let t = cty.clone();
                self.fs().locals.insert(pn, (slot, t));
            }
            let addr = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: addr,
                    rv: RValue::AddrOfLocal { alloca: slot },
                },
                span,
            );
            self.emit(
                InstKind::Store {
                    addr: Operand::Value(addr),
                    val: Operand::Value(v),
                    ty: cty,
                    align,
                    vol: Volatility::Normal,
                },
                span,
            );
        }

        self.stmt(body);

        self.resolve_gotos();
        self.finish_blocks();
        self.compute_gcov_lines();

        // **015 §7, contract 20: refuse the function whole.** If anything in the body
        // could not be lowered, the definition is discarded and one diagnostic stands for
        // it. A *partial* body is worse than none — every analysis downstream treats a
        // `Body::Defined` function as complete, so a missing branch reads as a branch that
        // cannot be taken rather than as a gap.
        if self.diagnostics.len() > diags_before {
            self.diagnostics.truncate(diags_before);
            self.diagnostics.push(LowerDiagnostic {
                span,
                message: format!(
                    "`{}` contains a construct lowering cannot represent, so it was skipped",
                    &*cir_name
                ),
            });
            self.f = None;
            return;
        }
        let fs = self.f.take().expect("inside a function");
        self.module.funcs[slot] = Function {
            id: fs.id,
            name: fs.name,
            params: fs.params,
            ret: fs.ret,
            variadic: fs.variadic,
            allocas: fs.allocas,
            blocks: fs.blocks,
            entry: fs.entry,
            attrs: FnAttrs::default(),
            body: Body::Defined,
            span,
        };
    }

    /// Give every still-open block a terminator, then drop the ones nothing reaches.
    ///
    /// Both halves are needed and neither is cosmetic. `if (c) return a; else return b;`
    /// leaves a join block no edge targets, and `return x;` at the end of a body leaves
    /// the block that would have held whatever came next. The verifier rejects an
    /// unreachable block (020 §8), so lowering must not emit one. Pruning runs to a
    /// fixpoint, because removing a block can orphan another.
    fn finish_blocks(&mut self) {
        let entry = self.fs().entry;
        loop {
            let reachable = self.reachable_blocks();
            let before = self.fs().blocks.len();
            self.fs().blocks.retain(|b| {
                b.id == entry
                    || reachable.contains(&b.id)
                    // A block nothing reaches but which holds real instructions is left
                    // alone, so the verifier reports it rather than lowering deleting
                    // code quietly. **Markers do not count as real instructions**: a
                    // compound statement emits its `Scope(Exit)` after its last statement,
                    // and when that statement was a `return` the marker lands in the dead
                    // block after it. The exit already happened — `return` unwinds every
                    // open scope first — so the copy in the dead block is bookkeeping, not
                    // code, and keeping it made the module fail to verify.
                    || b.insts.iter().any(|i| !matches!(i.kind, InstKind::Marker(_)))
            });
            if self.fs().blocks.len() == before {
                break;
            }
        }
        let ret = self.fs().ret.clone();
        let value = match &ret {
            CTy::Void => None,
            other => Some(self.zero_of(other)),
        };
        for b in &mut self.f.as_mut().expect("in a function").blocks {
            if matches!(
                b.term,
                Terminator::Unreachable(chiero_cir::UnreachableReason::LoweringGap)
            ) {
                b.term = Terminator::Return(value.clone());
            }
        }
    }

    /// 015 §5. For each `Inst`, resolve its `Span` with **`expansion_loc`** and collect
    /// the distinct lines.
    ///
    /// Three of §5's five consequences are decisions made right here:
    ///
    /// - **`expansion_loc`, never `spelling_loc`.** A statement inside a macro body is
    ///   attributed to the `.c` line where the macro was *used*, because that is the only
    ///   line gcov records (030 §1, measured). Spelling locations name header lines that
    ///   appear in no coverage file and match nothing.
    /// - **Lines in a header are kept.** An earlier draft dropped any line outside the
    ///   block's own TU; 030 §1's measurement is the proof it was backwards, since a
    ///   `static inline` in a header *does* get its own gcov entry. Dropping them would
    ///   empty `gcov_lines` for all of `vec.h`, `pool.h` and `buffer_funcs.h` — VPP's
    ///   entire hot layer — while contract 17's subset property reported success, because
    ///   the empty set is a subset of everything.
    /// - **Compiler-generated instructions contribute nothing**, because gcov has no
    ///   counter for them either.
    fn compute_gcov_lines(&mut self) {
        let Some(map) = self.map else { return };
        let fs = self.f.as_mut().expect("in a function");
        for b in &mut fs.blocks {
            let mut lines: Vec<u32> = Vec::new();
            for i in &b.insts {
                if i.generated || i.span == Span::DUMMY {
                    continue;
                }
                let Some(loc) = map.expansion_loc(i.span) else {
                    continue;
                };
                if !lines.contains(&loc.line) {
                    lines.push(loc.line);
                }
            }
            lines.sort_unstable();
            b.gcov_lines = lines.into_iter().collect();
        }
    }

    fn reachable_blocks(&mut self) -> Vec<BlockId> {
        let entry = self.fs().entry;
        let mut seen: Vec<BlockId> = vec![];
        let mut work = vec![entry];
        while let Some(id) = work.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Some(b) = self.fs().blocks.iter().find(|b| b.id == id) else {
                continue;
            };
            match &b.term {
                Terminator::Goto(g) => work.push(*g),
                Terminator::Br { t, f, .. } => {
                    work.push(*t);
                    work.push(*f);
                }
                Terminator::Switch { cases, default, .. } => {
                    work.extend(cases.iter().map(|(_, b)| *b));
                    work.push(*default);
                }
                Terminator::IndirectGoto { targets, .. } => work.extend(targets.iter().copied()),
                Terminator::Return(_) | Terminator::Unreachable(_) => {}
            }
        }
        seen
    }

    /// The poison type.
    ///
    /// **Not `TyId(0)`**, which is whichever type happened to be interned first — an
    /// arbitrary type wearing the name of an error. A caller substituting it for a type it
    /// could not resolve got `int` or `long` depending on the file, silently.
    fn error_ty(&mut self) -> TyId {
        self.analysis
            .interned_error()
            .unwrap_or_else(|| self.analysis.any_ty())
    }

    fn zero_of(&self, ty: &CTy) -> Operand {
        match ty {
            CTy::Int(bits) => Operand::Const(Const::Int {
                bits: *bits,
                val: 0,
            }),
            CTy::Ptr => Operand::Const(Const::Null),
            CTy::Float(k) => Operand::Const(Const::Float(*k, 0)),
            _ => Operand::Const(Const::Int { bits: 32, val: 0 }),
        }
    }

    /// A sema type as a CIR type.
    ///
    /// CIR has **no aggregate values** (020 §1.4), so a record or array reaching a value
    /// position is a pointer's worth of address — the aggregate itself lives in memory
    /// and is moved with `CopyMem`.
    fn cty(&self, ty: TyId) -> CTy {
        match self.analysis.ty(ty) {
            Ty::Void => CTy::Void,
            Ty::Int { bits, .. } => CTy::Int((*bits).max(1)),
            // CIR has three float kinds; sema's six map onto them. `_Float16` and
            // `__bf16` become `F32` here rather than growing CIR a kind the solver
            // cannot model — recorded because it is a real narrowing, not a rename.
            Ty::Float(k) => CTy::Float(match k {
                FloatKind::F32 | FloatKind::Binary16 | FloatKind::BFloat16 => {
                    chiero_cir::FloatKind::F32
                }
                FloatKind::X87_80 | FloatKind::Binary128 => chiero_cir::FloatKind::X87_80,
                FloatKind::F64 => chiero_cir::FloatKind::F64,
            }),
            Ty::Ptr(_) | Ty::Array { .. } | Ty::Func { .. } => CTy::Ptr,
            Ty::Record(_) | Ty::Vector { .. } => CTy::Ptr,
            Ty::Error => CTy::Int(32),
        }
    }
}

impl Lowerer<'_> {
    // ---- statements (015 §3) ----

    fn stmt(&mut self, s: StmtId) {
        let kind = self.ast.stmt(s).kind.clone();
        let span = self.ast.stmt(s).span;
        match kind {
            StmtKind::Compound(ss) => {
                self.enter_scope(span);
                for s in ss {
                    self.stmt(s);
                }
                self.exit_scope(span);
            }
            StmtKind::Expr(e) => {
                let v = self.expr(e);
                self.last_stmt_value = Some(v);
                self.seq_point(span);
            }
            StmtKind::Decl(ds) => {
                for d in ds {
                    // A **nested function definition** (013 §4 puts it in the "no"
                    // column). One diagnostic; the enclosing function is then refused
                    // whole by `function`.
                    // `DeclKind::Error` is what 013 leaves where it refused a nested
                    // function definition. Lowering cannot represent whatever was there,
                    // so the enclosing function is refused whole by `function`.
                    if matches!(self.ast.decl(d).kind, DeclKind::Error) {
                        self.diagnostics.push(LowerDiagnostic {
                            span,
                            message: "a declaration the frontend refused".into(),
                        });
                        continue;
                    }
                    self.local_decl(d);
                }
            }
            StmtKind::Return(v) => {
                let op = v.map(|e| self.expr(e));
                let ret = self.fs().ret.clone();
                let op = match (op, &ret) {
                    (Some(o), _) => Some(o),
                    (None, CTy::Void) => None,
                    (None, other) => Some(self.zero_of(other)),
                };
                // 015 §3: every open scope is exited **before** the `Return`.
                self.unwind_to(0, span);
                self.set_term(Terminator::Return(op));
                // Anything after a `return` is unreachable but must still have somewhere
                // to be emitted, or the next statement writes into a terminated block.
                let dead = self.new_block();
                self.switch_to(dead);
            }
            StmtKind::If { cond, then, els } => {
                let c = self.expr(cond);
                let c = self.truth_of(c, self.width_of(cond), span);
                self.seq_point(span);
                let then_b = self.new_block();
                let else_b = self.new_block();
                let join = self.new_block();
                self.set_term(Terminator::Br {
                    cond: c,
                    t: then_b,
                    f: else_b,
                });
                self.switch_to(then_b);
                self.stmt(then);
                self.goto_if_open(join);
                self.switch_to(else_b);
                if let Some(e) = els {
                    self.stmt(e);
                }
                self.goto_if_open(join);
                self.switch_to(join);
            }
            StmtKind::While { cond, body } => {
                let header = self.new_block();
                let body_b = self.new_block();
                let exit = self.new_block();
                self.goto_if_open(header);
                self.switch_to(header);
                let c = self.expr(cond);
                let c = self.truth_of(c, self.width_of(cond), span);
                self.set_term(Terminator::Br {
                    cond: c,
                    t: body_b,
                    f: exit,
                });
                self.switch_to(body_b);
                let depth = self.scope_depth();
                self.fs().breaks.push((exit, depth));
                self.fs().continues.push((header, depth));
                self.stmt(body);
                self.fs().breaks.pop();
                self.fs().continues.pop();
                self.goto_if_open(header);
                self.switch_to(exit);
            }
            StmtKind::DoWhile { body, cond } => {
                let body_b = self.new_block();
                let latch = self.new_block();
                let exit = self.new_block();
                self.goto_if_open(body_b);
                self.switch_to(body_b);
                let depth = self.scope_depth();
                self.fs().breaks.push((exit, depth));
                self.fs().continues.push((latch, depth));
                self.stmt(body);
                self.fs().breaks.pop();
                self.fs().continues.pop();
                self.goto_if_open(latch);
                self.switch_to(latch);
                let c = self.expr(cond);
                let c = self.truth_of(c, self.width_of(cond), span);
                self.set_term(Terminator::Br {
                    cond: c,
                    t: body_b,
                    f: exit,
                });
                self.switch_to(exit);
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                // **A scope enclosing the loop** (015 §3, contract 12). `i` is one object
                // for the whole loop; putting it in the body's scope would retire and
                // recreate it every iteration, so `&i` would change across iterations.
                self.enter_scope(span);
                match init {
                    Some(chiero_ast::ForInit::Decl(ds)) => {
                        for d in ds {
                            self.local_decl(d);
                        }
                    }
                    Some(chiero_ast::ForInit::Expr(e)) => {
                        self.expr(e);
                        self.seq_point(span);
                    }
                    None => {}
                }
                // **The header is always its own block** (contract 13), even for
                // `for(;;)`. Folding an absent condition into the body leaves a correct
                // function with no identifiable loop header, and 023 §8's dominator
                // analysis then sees straight-line code.
                let header = self.new_block();
                let body_b = self.new_block();
                let latch = self.new_block();
                let exit = self.new_block();
                self.goto_if_open(header);
                self.switch_to(header);
                match cond {
                    Some(ce) => {
                        let c = self.expr(ce);
                        let c = self.truth_of(c, self.width_of(ce), span);
                        self.set_term(Terminator::Br {
                            cond: c,
                            t: body_b,
                            f: exit,
                        });
                    }
                    None => self.set_term(Terminator::Goto(body_b)),
                }
                self.switch_to(body_b);
                let depth = self.scope_depth();
                self.fs().breaks.push((exit, depth));
                // `continue` in a `for` goes to the **latch**, not the header — the step
                // expression still runs. Sending it to the header instead skips the step
                // and turns `for (i=0;i<n;i++) { if (c) continue; }` into an infinite loop.
                self.fs().continues.push((latch, depth));
                self.stmt(body);
                self.fs().breaks.pop();
                self.fs().continues.pop();
                self.goto_if_open(latch);
                self.switch_to(latch);
                if let Some(st) = step {
                    self.expr(st);
                    self.seq_point(span);
                }
                self.set_term(Terminator::Goto(header));
                self.switch_to(exit);
                self.exit_scope(span);
            }
            StmtKind::Break => {
                let Some(&(target, depth)) = self.fs().breaks.last() else {
                    self.diagnostics.push(LowerDiagnostic {
                        span,
                        message: "`break` outside a loop or switch".into(),
                    });
                    return;
                };
                self.unwind_to(depth, span);
                self.set_term(Terminator::Goto(target));
                let dead = self.new_block();
                self.switch_to(dead);
            }
            StmtKind::Continue => {
                let Some(&(target, depth)) = self.fs().continues.last() else {
                    self.diagnostics.push(LowerDiagnostic {
                        span,
                        message: "`continue` outside a loop".into(),
                    });
                    return;
                };
                self.unwind_to(depth, span);
                self.set_term(Terminator::Goto(target));
                let dead = self.new_block();
                self.switch_to(dead);
            }
            StmtKind::Label { name, body } => {
                let target = self.new_block();
                let open = self.fs().open_scopes.clone();
                self.fs().labels.insert(name, (target, open));
                self.set_term(Terminator::Goto(target));
                self.switch_to(target);
                self.emit(
                    InstKind::Marker(MarkerKind::Label(
                        self.sym(name).unwrap_or_else(|| std::sync::Arc::from("?")),
                    )),
                    span,
                );
                self.stmt(body);
            }
            StmtKind::Goto(name) => {
                let open = self.fs().open_scopes.clone();
                let here = self.fs().cur;
                match self.fs().labels.get(&name).cloned() {
                    // A backward `goto`: the label's scopes are known, so unwind now.
                    Some((target, at_label)) => {
                        self.unwind_leaving(&open, &at_label, span);
                        self.set_term(Terminator::Goto(target));
                    }
                    // A forward `goto`: the label has not been seen. Recorded with the
                    // scopes open *here*, and resolved once the whole function is walked —
                    // guessing would emit the wrong markers for every forward jump, which
                    // is most of them.
                    None => self.fs().pending_gotos.push((here, name, open, span)),
                }
                let dead = self.new_block();
                self.switch_to(dead);
            }
            StmtKind::Switch { cond, body } => self.switch_stmt(cond, body, span),
            StmtKind::Case { .. } | StmtKind::Default { .. } => {
                // Reached only outside a `switch`, which is a C error the parser accepts;
                // 015 §7 refuses rather than inventing a target.
                self.diagnostics.push(LowerDiagnostic {
                    span,
                    message: "`case` or `default` outside a switch".into(),
                });
            }
            StmtKind::Empty | StmtKind::Error => {}
            other => {
                // 015 §7: refuse rather than lower wrongly. A construct this slice does
                // not cover leaves a diagnostic and an `Unreachable(LoweringGap)`, which
                // 020 §5 says is a diagnostic and **never** a licence to treat the path
                // as infeasible.
                self.diagnostics.push(LowerDiagnostic {
                    span,
                    message: format!("statement not lowered yet: {}", stmt_name(&other)),
                });
            }
        }
    }

    /// Terminate the current block with a `Goto`.
    ///
    /// **No "is it still open" guard**, because there cannot be one: a `return` switches
    /// to a fresh block before anything else is emitted, so the block reached here is
    /// always the unterminated one and the terminated arm is never touched. An earlier
    /// version checked anyway; a mutation removing the check changed nothing, which is
    /// how the branch was found to be dead. `finish_blocks` prunes the empty successor.
    fn goto_if_open(&mut self, target: BlockId) {
        self.set_term(Terminator::Goto(target));
    }

    fn local_decl(&mut self, d: chiero_ast::DeclId) {
        let decl = self.ast.decl(d).kind.clone();
        let span = self.ast.decl(d).span;
        let DeclKind::Var { name, init, .. } = decl else {
            return;
        };
        let Some(sty) = self.analysis.ty_of_decl(d) else {
            return;
        };
        let cty = self.cty(sty);
        let align = self.analysis.align_of(sty).unwrap_or(1).max(1);
        let text = name.and_then(|n| self.sym(n));

        // **A VLA allocates where it is declared** (015 contract 14, 020 §3). The size is
        // computed *here*, so ordinary dominance applies to it — hoisting the allocation
        // to the entry block would reference a value that does not dominate it, which is
        // exactly what putting `AllocaDyn` at a real program point exists to prevent.
        if let Some((elem_ty, count_expr)) = self.vla_of(sty) {
            let slot = self.alloca_dynamic(align, text, span);
            if let Some(n) = name {
                self.fs().locals.insert(n, (slot, cty.clone()));
            }
            let count = self.expr(count_expr);
            let dst = self.new_value();
            self.emit(
                InstKind::AllocaDyn {
                    dst,
                    alloca: slot,
                    elem: self.cty(elem_ty),
                    count,
                    align,
                },
                span,
            );
            self.seq_point(span);
            return;
        }

        let slot = self.alloca_for(sty, align, text, span);
        if let Some(n) = name {
            self.fs().locals.insert(n, (slot, cty.clone()));
        }
        if let Some(init) = init {
            // **An aggregate initializer is a zero-fill plus the written elements**
            // (015 §6, contract 19). C11 6.7.9p21 zero-initializes everything the braces
            // do not mention, so `struct S s = {.b = 3}` gives `s.a` a *defined* 0 — and
            // 021 contract 28 marks a `SetMem` range initialized, which is what makes the
            // read produce no finding. Storing only the written members would leave the
            // rest reading as uninitialized and turn well-defined C into a finding.
            if let Some(size) = self.aggregate_size_of_ty(sty)
                && matches!(self.ast.expr(init).kind, chiero_ast::ExprKind::InitList(_))
            {
                let base = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst: base,
                        rv: RValue::AddrOfLocal { alloca: slot },
                    },
                    span,
                );
                self.generated(|s| {
                    s.emit(
                        InstKind::SetMem {
                            dst: Operand::Value(base),
                            byte: Operand::Const(Const::Int { bits: 8, val: 0 }),
                            size: Operand::Const(Const::Int {
                                bits: 64,
                                val: size as i128,
                            }),
                        },
                        span,
                    )
                });
                self.init_list(Operand::Value(base), sty, init, span);
                self.seq_point(span);
                return;
            }
            let v = self.expr(init);
            let addr = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: addr,
                    rv: RValue::AddrOfLocal { alloca: slot },
                },
                span,
            );
            self.emit(
                InstKind::Store {
                    addr: Operand::Value(addr),
                    val: v,
                    ty: cty,
                    align,
                    vol: Volatility::Normal,
                },
                span,
            );
            self.seq_point(span);
        }
    }

    // ---- expressions (015 §2) ----

    /// Lower an expression to an operand, **following the typed AST's conversions**.
    ///
    /// The typed node is the authority: 014 §5 already inserted every conversion, so this
    /// walks `TypedNode::Cast` chains and emits a real `Cast` instruction for each rather
    /// than deciding anything itself. If this function ever needs to *invent* a
    /// conversion, that is a `chiero-sema` bug (015 §2).
    fn expr(&mut self, e: ExprId) -> Operand {
        let Some(top) = self.analysis.typed().top(e) else {
            return self.raw_expr(e);
        };
        self.typed_node(top, e)
    }

    fn typed_node(&mut self, id: TypedId, _outer: ExprId) -> Operand {
        match self.analysis.typed().node(id).clone() {
            TypedNode::Cast {
                operand,
                ty,
                span,
                why,
                ..
            } => {
                let inner = self.typed_node(operand, _outer);
                let to = self.cty(ty);
                // An array or function decaying to a pointer is not a value conversion:
                // the operand already *is* the address. Emitting a `Cast` would ask the
                // solver to reinterpret a pointer as a pointer.
                if matches!(why, Conversion::ArrayDecay | Conversion::FunctionDecay) {
                    return inner;
                }
                let from = self.cty(self.analysis.typed().ty_of(operand));
                let from_signed = matches!(
                    self.analysis.ty(self.analysis.typed().ty_of(operand)),
                    Ty::Int { signed: true, .. }
                );
                // **The cast kind follows from the widths**, and sign-extension is the
                // half that matters: widening a `signed char` with `ZExt` turns -1 into
                // 255 and nothing downstream can tell.
                // **A pointer-to-pointer conversion emits nothing.** 020 makes pointers
                // address-sized and *untyped*, so `(char *)p` changes no bits — and
                // `Bitcast` is the wrong instruction for it, since the verifier requires
                // a bitcast to preserve total bit width and `CTy::Ptr` has none.
                if matches!((&from, &to), (CTy::Ptr, CTy::Ptr)) {
                    return inner;
                }
                let kind = cast_kind(&from, &to, from_signed);
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::Cast {
                            kind,
                            a: inner,
                            from,
                            to,
                        },
                    },
                    span,
                );
                Operand::Value(dst)
            }
            TypedNode::Value { expr, .. } => self.raw_expr(expr),
        }
    }

    fn raw_expr(&mut self, e: ExprId) -> Operand {
        let node = self.ast.expr(e).clone();
        let span = node.span;
        match &node.kind {
            chiero_ast::ExprKind::Number(_) | chiero_ast::ExprKind::Char { .. } => {
                let mut diags = Vec::new();
                let v = chiero_sema::const_eval(self.ast, e, self.names, self.target(), &mut diags);
                let bits = self.raw_width_of(e);
                match v {
                    Some(chiero_sema::ConstVal::Int(n)) => {
                        Operand::Const(Const::Int { bits, val: n })
                    }
                    _ => Operand::Const(Const::Int { bits, val: 0 }),
                }
            }
            // A member **read**: `LoadBits` for a bit-field, an ordinary `Load` at the
            // member's address otherwise. Both take their offset from `RecordLayout`.
            chiero_ast::ExprKind::Member { .. } => {
                if let Some((unit, bits)) = self.bitfield_of(e) {
                    let Some(addr) = self.lvalue_addr(e, span) else {
                        return Operand::Const(Const::Undef(unit));
                    };
                    let dst = self.new_value();
                    let signed = self.is_signed(e);
                    self.emit(
                        InstKind::Assign {
                            dst,
                            rv: RValue::LoadBits {
                                addr,
                                unit,
                                bits,
                                signed,
                                align: 1,
                            },
                        },
                        span,
                    );
                    return Operand::Value(dst);
                }
                let ty = CTy::Int(self.raw_width_of(e));
                let Some(addr) = self.lvalue_addr(e, span) else {
                    return Operand::Const(Const::Undef(ty));
                };
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::Load {
                            addr,
                            ty,
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    span,
                );
                Operand::Value(dst)
            }
            // `*p` as a *value*: the pointer's value is the address, and the result is a
            // load of the pointee. `lvalue_addr` already handles the assignment side.
            chiero_ast::ExprKind::Unary {
                op: chiero_ast::UnOp::Deref,
                ..
            }
            | chiero_ast::ExprKind::Index { .. } => {
                let ty = CTy::Int(self.raw_width_of(e));
                let Some(addr) = self.lvalue_addr(e, span) else {
                    return Operand::Const(Const::Undef(ty));
                };
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::Load {
                            addr,
                            ty,
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    span,
                );
                Operand::Value(dst)
            }
            chiero_ast::ExprKind::Ident(sym) => {
                let Some((slot, ty)) = self.fs().locals.get(sym).cloned() else {
                    // Not a local: a function or file-scope object. Neither is modelled
                    // by this slice, so the value is `Undef` and the path is honest
                    // about knowing nothing rather than inventing a zero.
                    let ty = CTy::Int(self.raw_width_of(e));
                    return Operand::Const(Const::Undef(ty));
                };
                let addr = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst: addr,
                        rv: RValue::AddrOfLocal { alloca: slot },
                    },
                    span,
                );
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::Load {
                            addr: Operand::Value(addr),
                            ty,
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    span,
                );
                Operand::Value(dst)
            }
            chiero_ast::ExprKind::Binary { op, lhs, rhs } => {
                if matches!(op, chiero_ast::BinOp::LogAnd | chiero_ast::BinOp::LogOr) {
                    return self.short_circuit(e, *op, *lhs, *rhs, span);
                }
                // **Left to right** (015 §2, normative): the left operand's side effects
                // are emitted before the right's.
                let a = self.expr(*lhs);
                let b = self.expr(*rhs);
                let dst = self.new_value();
                // CIR keeps comparisons in their own `RValue` (020), and signedness is a
                // property of the **operands**, not of the result — so it comes from the
                // typed AST rather than from the operator.
                match cir_cmpop(*op, self.is_signed(*lhs)) {
                    Some(cop) => {
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Cmp {
                                    op: cop,
                                    a,
                                    b,
                                    // **Post-conversion**: `a` and `b` are what `expr`
                                    // returned, and 014 already promoted a `char` operand
                                    // to `int`. Using the written width declared `Int(8)`
                                    // for an operand the typed AST had widened to 32.
                                    ty: CTy::Int(self.width_of(*lhs)),
                                },
                            },
                            span,
                        );
                        // **A relational operator yields `int` in C**, and CIR's `Cmp`
                        // yields one bit. Widening here keeps one invariant true
                        // everywhere: *an expression's operand has the width of its C
                        // type*. Without it, every consumer has to ask "was this a
                        // comparison?" before it can know what it is holding.
                        self.widen_bool(dst, self.raw_width_of(e), span)
                    }
                    None => {
                        let w = self.raw_width_of(e);
                        // **A shift's count is widened to the operation's width.** C
                        // promotes the two operands *independently* and gives the result
                        // the left operand's type, so `l << 1` legitimately has a 64-bit
                        // value and a 32-bit count — but CIR's verifier requires both
                        // operands at the declared width. This is lowering's own
                        // bookkeeping, not a C conversion: 014 correctly inserted none,
                        // and inventing one for any *other* operator would be the bug
                        // 015 §2 forbids.
                        let b = if matches!(op, chiero_ast::BinOp::Shl | chiero_ast::BinOp::Shr) {
                            let bw = self.raw_width_of(*rhs);
                            if bw < w {
                                let wide = self.new_value();
                                self.emit(
                                    InstKind::Assign {
                                        dst: wide,
                                        rv: RValue::Cast {
                                            kind: if self.is_signed(*rhs) {
                                                chiero_cir::CastKind::SExt
                                            } else {
                                                chiero_cir::CastKind::ZExt
                                            },
                                            a: b,
                                            from: CTy::Int(bw),
                                            to: CTy::Int(w),
                                        },
                                    },
                                    span,
                                );
                                Operand::Value(wide)
                            } else {
                                b
                            }
                        } else {
                            b
                        };
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Bin {
                                    op: cir_binop(*op, self.is_signed(*lhs)),
                                    a,
                                    b,
                                    ty: CTy::Int(w),
                                },
                            },
                            span,
                        );
                        Operand::Value(dst)
                    }
                }
            }
            // `&x` — the **address**, not the value. Like the pre-increment arm below,
            // this has to precede the general `Unary` one: it fell through to the
            // operand's loaded value, which is the right answer for every fixture that
            // only *reads* through the pointer and the wrong program for every one that
            // writes.
            chiero_ast::ExprKind::Unary {
                op: chiero_ast::UnOp::AddrOf,
                operand,
            } => match self.lvalue_addr(*operand, span) {
                Some(a) => a,
                None => Operand::Const(Const::Undef(CTy::Ptr)),
            },
            // **Before the general `Unary` arm.** Rust matches in order, so an arm that
            // refines an earlier pattern has to precede it — this one did not, and `++x`
            // matched the general arm, fell through its `_ =>` to the operand's loaded
            // value, and evaluated to `x` without incrementing anything. Found by the
            // differential oracle in its first hour; no structural test could see it.
            chiero_ast::ExprKind::Unary {
                op: op @ (chiero_ast::UnOp::PreInc | chiero_ast::UnOp::PreDec),
                operand,
            } => self.inc_dec(*operand, matches!(op, chiero_ast::UnOp::PreInc), true, span),
            chiero_ast::ExprKind::Unary { op, operand } => {
                let a = self.expr(*operand);
                let ty = CTy::Int(self.raw_width_of(e));
                let dst = self.new_value();
                // `!x` is `x == 0`, which CIR expresses as a comparison rather than a
                // unary op — it has no logical-not, because the result is an `int` and a
                // dedicated op would need its own width rule.
                let rv = match op {
                    chiero_ast::UnOp::Minus => RValue::Un {
                        op: CUnOp::Neg,
                        a,
                        ty,
                    },
                    chiero_ast::UnOp::BitNot => RValue::Un {
                        op: CUnOp::Not,
                        a,
                        ty,
                    },
                    chiero_ast::UnOp::Not => {
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Cmp {
                                    op: chiero_cir::CmpOp::Eq,
                                    a,
                                    b: Operand::Const(Const::Int {
                                        bits: self.width_of(*operand),
                                        val: 0,
                                    }),
                                    ty: CTy::Int(self.width_of(*operand)),
                                },
                            },
                            span,
                        );
                        // `!x` is an `int` too.
                        return self.widen_bool(dst, self.raw_width_of(e), span);
                    }
                    _ => return a,
                };
                self.emit(InstKind::Assign { dst, rv }, span);
                Operand::Value(dst)
            }
            chiero_ast::ExprKind::Assign { op, lhs, rhs } => self.assign(e, *op, *lhs, *rhs, span),
            // 015 §2.2: `x++` yields the **old** value, `++x` the new one. Both compute
            // the address once and both are a load, an add and a store — the only
            // difference is which of the two values is the expression's result.
            chiero_ast::ExprKind::Postfix { op, operand } => self.inc_dec(
                *operand,
                matches!(op, chiero_ast::PostfixOp::Inc),
                false,
                span,
            ),
            chiero_ast::ExprKind::Call { callee, args } => {
                // **`alloca()` is an allocation, not a call** (015 contract 14, 020 §3).
                // It differs from a VLA in exactly one way — `Lifetime::Function`, since
                // the storage lives until the function returns rather than until the block
                // ends — so an implementation using one lifetime for both is wrong in a
                // way no other assertion sees.
                if let chiero_ast::ExprKind::Ident(n) = self.ast.expr(*callee).kind
                    && matches!(self.names.text(n), Some("alloca" | "__builtin_alloca"))
                    && let Some(&size) = args.first()
                {
                    let count = self.expr(size);
                    let slot =
                        self.alloca_n(CTy::Int(8), chiero_cir::DYNAMIC_EXTENT, 16, None, span);
                    if let Some(a) = self.fs().allocas.iter_mut().find(|a| a.id == slot) {
                        a.lifetime = Lifetime::Function;
                    }
                    let dst = self.new_value();
                    self.emit(
                        InstKind::AllocaDyn {
                            dst,
                            alloca: slot,
                            elem: CTy::Int(8),
                            count,
                            align: 16,
                        },
                        span,
                    );
                    return Operand::Value(dst);
                }
                // Arguments left to right, **then** the call (015 §2.5).
                let ops: Vec<Operand> = args.iter().map(|&a| self.expr(a)).collect();
                let fid = self.callee_of(*callee);
                let ret_ty = self.width_of(e);
                let dst = self.new_value();
                self.emit(
                    InstKind::Call {
                        dst: Some(dst),
                        callee: fid,
                        args: ops,
                    },
                    span,
                );
                let _ = ret_ty;
                Operand::Value(dst)
            }
            // An **explicit** cast written in the source. The implicit ones are handled by
            // walking the typed AST's `Cast` nodes; this is the one the programmer wrote,
            // and it is a different AST node — missing it made every `(int)(a / 2u)` an
            // `Undef` and every fixture containing one unanswerable.
            chiero_ast::ExprKind::Cast { ty, operand } => {
                let a = self.expr(*operand);
                let from = CTy::Int(self.width_of(*operand));
                let to = self.cty_of_syntactic(*ty);
                if from == to || matches!((&from, &to), (CTy::Ptr, CTy::Ptr)) {
                    return a;
                }
                let dst = self.new_value();
                let kind = cast_kind(&from, &to, self.is_signed(*operand));
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::Cast { kind, a, from, to },
                    },
                    span,
                );
                Operand::Value(dst)
            }
            // 015 §2.1: `?:` uses the same four-block shape as `&&`, with the slot typed
            // as the **result** type rather than `int`.
            chiero_ast::ExprKind::Cond { cond, then, els } => {
                self.conditional(e, *cond, *then, *els, span)
            }
            // 015 §2.4: the block's statements lower into the enclosing block sequence and
            // the value is the last expression statement's. No CIR construct is needed —
            // it falls out of the unstructured CFG, which is why §2.4 is three sentences.
            chiero_ast::ExprKind::StmtExpr(body) => {
                let saved = self.last_stmt_value.take();
                self.stmt(*body);
                let v = self.last_stmt_value.take();
                // Restore, so a statement expression nested inside another does not
                // consume the outer one's value.
                self.last_stmt_value = saved;
                v.unwrap_or(Operand::Const(Const::Undef(CTy::Int(self.raw_width_of(e)))))
            }
            chiero_ast::ExprKind::Comma { lhs, rhs } => {
                self.expr(*lhs);
                self.seq_point(span);
                self.expr(*rhs)
            }
            _ => Operand::Const(Const::Undef(CTy::Int(self.raw_width_of(e)))),
        }
    }

    /// 015 §2.1: the fixed four-block shape for `&&` and `||`.
    fn short_circuit(
        &mut self,
        e: ExprId,
        op: chiero_ast::BinOp,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> Operand {
        // **`Int(32)`, not `Int(1)`** — the expression's C type is `int`, and a one-bit
        // slot would force a `ZExt` at every use that §2 forbids lowering to invent.
        let slot_ty = CTy::Int(self.raw_width_of(e).max(32));
        let slot = self.alloca(slot_ty.clone(), 4, None, span);

        let a = self.expr(lhs);
        let a = self.truth_of(a, self.width_of(lhs), span);
        // The sequence point goes at the **end of the entry block**, before the branch.
        // Leaving its position free would let two conforming lowerings emit different
        // goldens, which is what fixing the shape here is for.
        self.seq_point(span);

        let rhs_b = self.new_block();
        let short_b = self.new_block();
        let join = self.new_block();
        let (t, f) = if matches!(op, chiero_ast::BinOp::LogAnd) {
            (rhs_b, short_b)
        } else {
            (short_b, rhs_b)
        };
        self.set_term(Terminator::Br { cond: a, t, f });

        // The rhs block: evaluate `b` and store `b != 0`.
        self.switch_to(rhs_b);
        let b = self.expr(rhs);
        let nz = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: nz,
                rv: RValue::Cmp {
                    op: chiero_cir::CmpOp::Ne,
                    a: b,
                    b: Operand::Const(Const::Int { bits: 32, val: 0 }),
                    ty: slot_ty.clone(),
                },
            },
            span,
        );
        // 015 §2.1 stores `(b != 0)` as the expression's `int`, so the one-bit comparison
        // is widened here. This is **lowering's own bookkeeping**, not an inferred C
        // conversion: the C expression has no `_Bool` in it at all, and §2's rule that
        // lowering never infers a conversion is about the *source* program's semantics.
        let wide = self.new_value();
        // **Save-and-restore, not increment-and-decrement.** A mismatched pair either
        // marks source instructions as generated or underflows the counter, and an
        // earlier version of this function did both — the decrement had been placed in a
        // different function entirely by a careless edit, and every fixture panicked.
        let saved = self.generated_depth;
        self.generated_depth = saved + 1;
        self.emit(
            InstKind::Assign {
                dst: wide,
                rv: RValue::Cast {
                    kind: chiero_cir::CastKind::ZExt,
                    a: Operand::Value(nz),
                    from: CTy::Int(1),
                    to: slot_ty.clone(),
                },
            },
            span,
        );
        self.store_slot(slot, Operand::Value(wide), &slot_ty, span);
        self.generated_depth = saved;
        self.set_term(Terminator::Goto(join));

        // The short-circuit block: the answer without evaluating `b` at all. **All of it
        // is lowering's**, which is why 015 contract 16 expects the block to carry no
        // `gcov_lines` — gcov has no counter for a store the programmer did not write.
        self.switch_to(short_b);
        self.generated_depth = saved + 1;
        let short_val = i128::from(matches!(op, chiero_ast::BinOp::LogOr));
        self.store_slot(
            slot,
            Operand::Const(Const::Int {
                bits: 32,
                val: short_val,
            }),
            &slot_ty,
            span,
        );
        self.generated_depth = saved;
        self.set_term(Terminator::Goto(join));

        self.switch_to(join);
        self.generated_depth = saved + 1;
        let addr = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: addr,
                rv: RValue::AddrOfLocal { alloca: slot },
            },
            span,
        );
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Load {
                    addr: Operand::Value(addr),
                    ty: slot_ty,
                    align: 4,
                    vol: Volatility::Normal,
                },
            },
            span,
        );
        self.generated_depth = saved;
        Operand::Value(dst)
    }

    /// Widen a one-bit comparison result to the C type of the expression that produced
    /// it.
    fn widen_bool(&mut self, one_bit: ValueId, width: u32, span: Span) -> Operand {
        if width <= 1 {
            return Operand::Value(one_bit);
        }
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Cast {
                    kind: chiero_cir::CastKind::ZExt,
                    a: Operand::Value(one_bit),
                    from: CTy::Int(1),
                    to: CTy::Int(width),
                },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// A branch condition as `Int(1)`.
    ///
    /// 015 §2.1's snippet says `br a_nonzero`, and CIR's verifier agrees: `Br` takes a
    /// one-bit operand. C conditions are "compares unequal to 0", so the conversion is a
    /// comparison rather than a truncation — truncating `2` to one bit gives 0, which
    /// inverts the branch for every even nonzero value.
    fn truth_of(&mut self, v: Operand, width: u32, span: Span) -> Operand {
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Cmp {
                    op: chiero_cir::CmpOp::Ne,
                    a: v,
                    b: Operand::Const(Const::Int {
                        bits: width,
                        val: 0,
                    }),
                    ty: CTy::Int(width),
                },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// 015 §2.1's `?:`, which shares the `&&` shape and differs in three ways it names:
    /// the slot is the **result** type, a `void`-typed `?:` has no slot at all, and the
    /// GNU elvis form `a ?: b` evaluates `a` **once** — into the slot, then branches on
    /// it.
    fn conditional(
        &mut self,
        e: ExprId,
        cond: ExprId,
        then: Option<ExprId>,
        els: ExprId,
        span: Span,
    ) -> Operand {
        let width = self.raw_width_of(e);
        let slot_ty = CTy::Int(width.max(1));
        let slot = self.alloca(slot_ty.clone(), 4, None, span);

        let c = self.expr(cond);
        // **The elvis form evaluates `a` once.** Storing it into the slot here and
        // branching on the stored value is what makes that true; re-evaluating `cond` in
        // the true arm would run its side effects twice, and no shape test can see the
        // difference when it has none.
        if then.is_none() {
            self.generated(|s| s.store_slot(slot, c.clone(), &slot_ty, span));
        }
        let test = self.truth_of(c, self.width_of(cond), span);
        self.seq_point(span);

        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.set_term(Terminator::Br {
            cond: test,
            t: then_b,
            f: else_b,
        });

        self.switch_to(then_b);
        if let Some(t) = then {
            let v = self.expr(t);
            self.generated(|s| s.store_slot(slot, v, &slot_ty, span));
        }
        self.set_term(Terminator::Goto(join));

        self.switch_to(else_b);
        let v = self.expr(els);
        self.generated(|s| s.store_slot(slot, v, &slot_ty, span));
        self.set_term(Terminator::Goto(join));

        self.switch_to(join);
        self.generated(|s| {
            let addr = s.new_value();
            s.emit(
                InstKind::Assign {
                    dst: addr,
                    rv: RValue::AddrOfLocal { alloca: slot },
                },
                span,
            );
            let dst = s.new_value();
            s.emit(
                InstKind::Assign {
                    dst,
                    rv: RValue::Load {
                        addr: Operand::Value(addr),
                        ty: slot_ty.clone(),
                        align: 4,
                        vol: Volatility::Normal,
                    },
                },
                span,
            );
            Operand::Value(dst)
        })
    }

    fn store_slot(&mut self, slot: AllocaId, val: Operand, ty: &CTy, span: Span) {
        let addr = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: addr,
                rv: RValue::AddrOfLocal { alloca: slot },
            },
            span,
        );
        self.emit(
            InstKind::Store {
                addr: Operand::Value(addr),
                val,
                ty: ty.clone(),
                align: 4,
                vol: Volatility::Normal,
            },
            span,
        );
    }

    /// `x = e` and `x op= e`. 015 §2.2: the lvalue's address is evaluated **once**.
    fn assign(
        &mut self,
        e: ExprId,
        op: Option<chiero_ast::BinOp>,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> Operand {
        // **A bit-field is a different instruction, not a narrower store.** Its range
        // comes from `RecordLayout` (015 contract 7), and the check has to come before
        // the ordinary path or a 3-bit field becomes a 4-byte store over its neighbours.
        if op.is_none()
            && let Some((unit, bits)) = self.bitfield_of(lhs)
        {
            let addr = match self.lvalue_addr(lhs, span) {
                Some(a) => a,
                None => return Operand::Const(Const::Undef(unit)),
            };
            let val = self.expr(rhs);
            self.emit(
                InstKind::StoreBits {
                    addr,
                    val: val.clone(),
                    unit,
                    bits,
                    align: 1,
                },
                span,
            );
            return val;
        }

        // **An aggregate assignment is one `CopyMem` of the layout's size** (contract 6),
        // never a field-by-field sequence. CIR has no aggregate values (020 §1.4), so
        // there is nothing to load; and a sequence would silently drop the padding C
        // copies, leaving 021 to see those bytes uninitialized where the program had
        // defined them.
        if op.is_none()
            && let Some(size) = self.aggregate_size(lhs)
        {
            let dst = match self.lvalue_addr(lhs, span) {
                Some(a) => a,
                None => return Operand::Const(Const::Undef(CTy::Ptr)),
            };
            let src = match self.lvalue_addr(rhs, span) {
                Some(a) => a,
                None => return Operand::Const(Const::Undef(CTy::Ptr)),
            };
            let align = self.aggregate_align(lhs).unwrap_or(1).max(1);
            self.emit(
                InstKind::CopyMem {
                    dst: dst.clone(),
                    src,
                    size: Operand::Const(Const::Int {
                        bits: 64,
                        val: size as i128,
                    }),
                    align,
                },
                span,
            );
            return dst;
        }

        // The **declared** type of the lvalue, so the store's width matches the slot.
        let ty = self.lvalue_ty(lhs);
        let Some(addr) = self.lvalue_addr(lhs, span) else {
            return Operand::Const(Const::Undef(ty));
        };
        let value = match op {
            None => self.expr(rhs),
            Some(binop) => {
                // Load through the address computed **above**, not a second one.
                let old = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst: old,
                        rv: RValue::Load {
                            addr: addr.clone(),
                            ty: ty.clone(),
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    span,
                );
                let r = self.expr(rhs);
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::Bin {
                            op: cir_binop(binop, self.is_signed(lhs)),
                            a: Operand::Value(old),
                            b: r,
                            ty: ty.clone(),
                        },
                    },
                    span,
                );
                Operand::Value(dst)
            }
        };
        self.emit(
            InstKind::Store {
                addr,
                val: value.clone(),
                ty,
                align: 1,
                vol: Volatility::Normal,
            },
            span,
        );
        let _ = e;
        value
    }

    /// `x++`, `x--`, `++x`, `--x`.
    ///
    /// The address is computed **once** (015 §2.2), like a compound assignment and for
    /// the same reason: `*p++ += 1` would otherwise advance `p` twice.
    fn inc_dec(&mut self, operand: ExprId, up: bool, prefix: bool, span: Span) -> Operand {
        let ty = self.lvalue_ty(operand);
        let Some(addr) = self.lvalue_addr(operand, span) else {
            return Operand::Const(Const::Undef(ty));
        };
        let old = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: old,
                rv: RValue::Load {
                    addr: addr.clone(),
                    ty: ty.clone(),
                    align: 1,
                    vol: Volatility::Normal,
                },
            },
            span,
        );
        let width = match &ty {
            CTy::Int(b) => *b,
            _ => 32,
        };
        let new = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: new,
                rv: RValue::Bin {
                    op: if up { CBinOp::Add } else { CBinOp::Sub },
                    a: Operand::Value(old),
                    b: Operand::Const(Const::Int {
                        bits: width,
                        val: 1,
                    }),
                    ty: ty.clone(),
                },
            },
            span,
        );
        self.emit(
            InstKind::Store {
                addr,
                val: Operand::Value(new),
                ty,
                align: 1,
                vol: Volatility::Normal,
            },
            span,
        );
        Operand::Value(if prefix { new } else { old })
    }

    /// A syntactic type node as a CIR type, going through sema so a typedef or a tag
    /// resolves the same way it does everywhere else.
    fn cty_of_syntactic(&mut self, ty: chiero_ast::TypeId) -> CTy {
        match self.analysis.ty_of_syntactic(ty) {
            Some(t) => self.cty(t),
            None => CTy::Int(32),
        }
    }

    /// If `e` names a bit-field, its storage unit and the `BitRange` **from the layout**.
    fn bitfield_of(&mut self, e: ExprId) -> Option<(CTy, chiero_cir::BitRange)> {
        let chiero_ast::ExprKind::Member { base, field, arrow } = self.ast.expr(e).kind.clone()
        else {
            return None;
        };
        let (_, f) = self.field_of(base, field, arrow)?;
        let b = f.bits?;
        // The offset is relative to the byte the address computation landed on, and
        // `FieldLayout::offset` is that byte — so the two are consistent by construction
        // rather than by a second calculation here.
        Some((
            self.cty(f.ty),
            chiero_cir::BitRange {
                off: (b.bit_offset - f.offset * 8) as u32,
                width: b.width as u32,
            },
        ))
    }

    /// The size of `e`'s type when it is an aggregate — a record or an array.
    fn aggregate_size(&mut self, e: ExprId) -> Option<u64> {
        let t = self.type_of(e)?;
        matches!(
            self.analysis.ty(t),
            Ty::Record(_) | Ty::Array { .. } | Ty::Vector { .. }
        )
        .then(|| self.analysis.size_of(t))?
    }

    fn aggregate_align(&mut self, e: ExprId) -> Option<u64> {
        let t = self.type_of(e)?;
        self.analysis.align_of(t)
    }

    /// The declared type of an lvalue.
    fn lvalue_ty(&mut self, e: ExprId) -> CTy {
        if let chiero_ast::ExprKind::Ident(sym) = self.ast.expr(e).kind
            && let Some((_, ty)) = self.fs().locals.get(&sym)
        {
            return ty.clone();
        }
        CTy::Int(self.raw_width_of(e))
    }

    /// The address of an lvalue, computed **once**.
    fn lvalue_addr(&mut self, e: ExprId, span: Span) -> Option<Operand> {
        match self.ast.expr(e).kind.clone() {
            chiero_ast::ExprKind::Ident(sym) => {
                let (slot, _) = self.fs().locals.get(&sym).cloned()?;
                let addr = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst: addr,
                        rv: RValue::AddrOfLocal { alloca: slot },
                    },
                    span,
                );
                Some(Operand::Value(addr))
            }
            // `*p` — the pointer's value *is* the address.
            chiero_ast::ExprKind::Unary {
                op: chiero_ast::UnOp::Deref,
                operand,
            } => Some(self.expr(operand)),
            // `s.f` and `p->f`. The offset comes from `RecordLayout` and nowhere else —
            // 015 contract 7's "checked by construction: the layout is the only source".
            chiero_ast::ExprKind::Member { base, field, arrow } => {
                let base_addr = if arrow {
                    self.expr(base)
                } else {
                    self.lvalue_addr(base, span)?
                };
                let (byte_off, _) = self.field_of(base, field, arrow)?;
                if byte_off == 0 {
                    return Some(base_addr);
                }
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::PtrAdd {
                            base: base_addr,
                            off: Operand::Const(Const::Int {
                                bits: 64,
                                val: byte_off as i128,
                            }),
                        },
                    },
                    span,
                );
                Some(Operand::Value(dst))
            }
            chiero_ast::ExprKind::Index { base, index } => {
                // **An array and a pointer index differently.** `a[i]` starts from the
                // *address of* `a`; `p[i]` starts from the *value of* `p`. Taking the
                // address in both cases indexes off the pointer variable's own storage,
                // which is a wild access that happens to look plausible.
                let base_is_ptr = self
                    .type_of(base)
                    .map(|t| matches!(self.analysis.ty(t), Ty::Ptr(_)))
                    .unwrap_or(false);
                let base_addr = if base_is_ptr {
                    self.expr(base)
                } else {
                    self.lvalue_addr(base, span)
                        .unwrap_or_else(|| self.expr(base))
                };
                let idx = self.expr(index);
                // **The index is widened to pointer width before scaling.** `a[i]` has an
                // `int` index and a byte offset is 64 bits, so multiplying them directly
                // is a width mismatch the verifier rejects — and sign extension is the
                // right widening, since `a[-1]` is a legal (if unwise) C expression.
                let iw = self.width_of(index);
                let idx = if iw < 64 {
                    let w = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: w,
                            rv: RValue::Cast {
                                kind: if self.is_signed(index) {
                                    chiero_cir::CastKind::SExt
                                } else {
                                    chiero_cir::CastKind::ZExt
                                },
                                a: idx,
                                from: CTy::Int(iw),
                                to: CTy::Int(64),
                            },
                        },
                        span,
                    );
                    Operand::Value(w)
                } else {
                    idx
                };
                let elem = self.elem_size_of(base).unwrap_or(1);
                let scaled = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst: scaled,
                        rv: RValue::Bin {
                            op: CBinOp::Mul,
                            a: idx,
                            b: Operand::Const(Const::Int {
                                bits: 64,
                                val: elem as i128,
                            }),
                            ty: CTy::Int(64),
                        },
                    },
                    span,
                );
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::PtrAdd {
                            base: base_addr,
                            off: Operand::Value(scaled),
                        },
                    },
                    span,
                );
                Some(Operand::Value(dst))
            }
            _ => None,
        }
    }

    /// The `(byte offset, field layout)` of a member, **read from `RecordLayout`**.
    ///
    /// 015 contract 7 puts it plainly: the layout is the only source. 014 computes it and
    /// verifies it against gcc over 520 real VPP records; a second derivation here would
    /// be an unverified answer to a settled question, and the two would disagree on
    /// exactly the packed wire-format structs where it matters most.
    fn field_of(
        &mut self,
        base: ExprId,
        field: chiero_span::Symbol,
        arrow: bool,
    ) -> Option<(u64, chiero_sema::FieldLayout)> {
        let bty = self.type_of(base)?;
        let rec = match self.analysis.ty(bty).clone() {
            Ty::Record(r) => r,
            Ty::Ptr(p) if arrow => match self.analysis.ty(p).clone() {
                Ty::Record(r) => r,
                _ => return None,
            },
            _ => return None,
        };
        let l = self.analysis.layout(rec);
        let f = l.fields.iter().find(|f| f.name == Some(field))?;
        Some((f.offset, f.clone()))
    }

    /// The element size of whatever `base` indexes.
    fn elem_size_of(&mut self, base: ExprId) -> Option<u64> {
        let t = self.type_of(base)?;
        let elem = match self.analysis.ty(t).clone() {
            Ty::Array { elem, .. } | Ty::Ptr(elem) => elem,
            _ => return None,
        };
        self.analysis.size_of(elem)
    }

    /// An expression's semantic type, before conversions.
    fn type_of(&self, e: ExprId) -> Option<TyId> {
        let typed = self.analysis.typed();
        let mut id = typed.top(e)?;
        loop {
            match typed.node(id) {
                TypedNode::Cast { operand, .. } => id = *operand,
                TypedNode::Value { ty, .. } => return Some(*ty),
            }
        }
    }

    fn callee_of(&mut self, callee: ExprId) -> Callee {
        // A direct call needs the callee's `FuncId`. Functions are numbered in
        // declaration order, and a name that names no definition in this TU gets a fresh
        // id so the call is still representable.
        let named = match self.ast.expr(callee).kind {
            chiero_ast::ExprKind::Ident(sym) => self.names.text(sym).map(str::to_owned),
            _ => None,
        };
        if let Some(text) = named {
            {
                if let Some(f) = self.module.funcs.iter().find(|f| *f.name == *text) {
                    return Callee::Direct(f.id);
                }
                // Nothing declared this name. Rather than inventing a signature the
                // verifier would then reject, the call goes indirect through an
                // undefined address — honest about knowing nothing, and 020 §5's rule
                // that a gap is a diagnostic rather than a licence.
                self.diagnostics.push(LowerDiagnostic {
                    span: self.ast.expr(callee).span,
                    message: format!("call to undeclared function `{text}`"),
                });
                return Callee::Indirect(Operand::Const(Const::Undef(CTy::Ptr)));
            }
        }
        let op = self.expr(callee);
        Callee::Indirect(op)
    }

    fn target(&self) -> &chiero_sema::TargetConfig {
        // The analysis was built against one target and every width here must agree with
        // the layout computed from it.
        self.analysis
            .target_config()
            .expect("the analysis carries its target")
    }

    /// Whether an expression's type is signed, from the typed AST.
    fn is_signed(&self, e: ExprId) -> bool {
        let Some(top) = self.analysis.typed().top(e) else {
            return true;
        };
        let ty = self.analysis.typed().ty_of(top);
        matches!(self.analysis.ty(ty), Ty::Int { signed: true, .. })
    }

    /// The width an expression's value has **before** any conversion is applied to it.
    ///
    /// [`Self::width_of`] answers the other question — the width *after* — and confusing
    /// the two is the defect the differential oracle found first. In
    /// `signed char c = -1;` the literal's top typed node is the `Cast` to `char` that
    /// the initializer needs, so `width_of` reports 8 for an expression whose value is a
    /// 32-bit `int`. `raw_expr` computes the value before conversions, so it must ask
    /// this one; the result was `neg i8 1i32` and a module that does not verify.
    fn raw_width_of(&self, e: ExprId) -> u32 {
        let typed = self.analysis.typed();
        let Some(mut id) = typed.top(e) else {
            return 32;
        };
        // Walk in past every conversion to the value the source actually wrote.
        loop {
            match typed.node(id) {
                TypedNode::Cast { operand, .. } => id = *operand,
                TypedNode::Value { ty, .. } => {
                    return match self.analysis.ty(*ty) {
                        Ty::Int { bits, .. } => (*bits).max(1),
                        _ => 32,
                    };
                }
            }
        }
    }

    /// The bit width an expression's value occupies **after** its conversions.
    fn width_of(&self, e: ExprId) -> u32 {
        let Some(top) = self.analysis.typed().top(e) else {
            return 32;
        };
        let ty = self.analysis.typed().ty_of(top);
        match self.analysis.ty(ty) {
            Ty::Int { bits, .. } => (*bits).max(1),
            _ => 32,
        }
    }
}

/// Which cast CIR needs to get from `from` to `to`.
///
/// Sign-extension is the case that has to be right: widening a `signed char` holding -1
/// with `ZExt` produces 255, which is a legal value of the wider type and so passes every
/// check the verifier and the solver make.
fn cast_kind(from: &CTy, to: &CTy, from_signed: bool) -> chiero_cir::CastKind {
    use chiero_cir::CastKind as K;
    match (from, to) {
        (CTy::Int(a), CTy::Int(b)) => {
            if b > a {
                if from_signed { K::SExt } else { K::ZExt }
            } else if b < a {
                K::Trunc
            } else {
                K::Bitcast
            }
        }
        (CTy::Int(_), CTy::Ptr) => K::IntToPtr,
        (CTy::Ptr, CTy::Int(_)) => K::PtrToInt,
        (CTy::Int(_), CTy::Float(_)) => {
            if from_signed {
                K::SiToFp
            } else {
                K::UiToFp
            }
        }
        (CTy::Float(_), CTy::Int(_)) => K::FpToSi,
        (CTy::Float(a), CTy::Float(b)) => {
            if b.bits() > a.bits() {
                K::FpExt
            } else {
                K::FpTrunc
            }
        }
        _ => K::Bitcast,
    }
}

fn stmt_name(k: &StmtKind) -> &'static str {
    match k {
        StmtKind::Switch { .. } => "switch",
        StmtKind::Case { .. } => "case",
        StmtKind::Default { .. } => "default",
        StmtKind::Label { .. } => "label",
        StmtKind::Goto(_) => "goto",
        StmtKind::GotoIndirect(_) => "computed goto",
        StmtKind::Break => "break",
        StmtKind::Continue => "continue",
        StmtKind::Asm(_) => "asm",
        _ => "statement",
    }
}

/// **Signedness comes from the operands, not the operator.** C has one `/` and CIR has
/// `SDiv` and `UDiv`; picking the wrong one is a wrong answer for exactly half the inputs
/// and no width check notices. The typed AST is the source, since 014 already resolved
/// the usual arithmetic conversions.
fn cir_binop(op: chiero_ast::BinOp, signed: bool) -> CBinOp {
    use chiero_ast::BinOp as A;
    match op {
        A::Add => CBinOp::Add,
        A::Sub => CBinOp::Sub,
        A::Mul => CBinOp::Mul,
        A::Div => {
            if signed {
                CBinOp::SDiv
            } else {
                CBinOp::UDiv
            }
        }
        A::Rem => {
            if signed {
                CBinOp::SRem
            } else {
                CBinOp::URem
            }
        }
        A::Shl => CBinOp::Shl,
        A::Shr => {
            if signed {
                CBinOp::AShr
            } else {
                CBinOp::LShr
            }
        }
        A::BitAnd => CBinOp::And,
        A::BitXor => CBinOp::Xor,
        A::BitOr => CBinOp::Or,
        // Comparisons go through `cir_cmpop`; `&&`/`||` are control flow (015 §2.1) and
        // never reach here.
        _ => CBinOp::And,
    }
}

fn cir_cmpop(op: chiero_ast::BinOp, signed: bool) -> Option<chiero_cir::CmpOp> {
    use chiero_ast::BinOp as A;
    use chiero_cir::CmpOp as C;
    Some(match op {
        A::Eq => C::Eq,
        A::Ne => C::Ne,
        A::Lt => {
            if signed {
                C::SLt
            } else {
                C::ULt
            }
        }
        A::Gt => {
            if signed {
                C::SGt
            } else {
                C::UGt
            }
        }
        A::Le => {
            if signed {
                C::SLe
            } else {
                C::ULe
            }
        }
        A::Ge => {
            if signed {
                C::SGe
            } else {
                C::UGe
            }
        }
        _ => return None,
    })
}

impl Lowerer<'_> {
    /// 015 §3 and contract 18: a `Switch` terminator, cases sorted, ranges expanded.
    ///
    /// **The body's scope is entered on every case edge, not at its lexical top**
    /// (015 §4, contract 9b). The `Switch` jumps straight to a case label *inside* the
    /// compound statement, so an enter emitted at the top would never run: 021 §4 would
    /// never create the scope's objects, the eventual exit would retire objects that never
    /// existed, and every access on the case path would be a wild access. Any `switch`
    /// with a local has this shape.
    fn switch_stmt(&mut self, cond: ExprId, body: StmtId, span: Span) {
        let scrut = self.expr(cond);
        let ty = CTy::Int(self.width_of(cond));
        let exit = self.new_block();
        let head = self.fs().cur;

        // Walk the body's statements once, giving each `case`/`default` its own block.
        // The body is a compound statement whose scope every case edge enters.
        let stmts = match &self.ast.stmt(body).kind {
            StmtKind::Compound(ss) => ss.clone(),
            _ => vec![body],
        };

        // **One scope for the whole body**, entered on every case edge.
        //
        // The cases are alternatives, not nested blocks: the body is a single compound
        // statement and 015 §4's rule is that *every entering edge* carries the marker.
        // Allocating a scope per case would leave one open for every case the path did
        // not take. The scope is pushed here so a `break` or `return` inside any case
        // unwinds it, and the `Enter` marker is emitted separately in each case block.
        let body_scope = ScopeId(self.fs().next_scope);
        self.fs().next_scope += 1;
        self.fs().open_scopes.push(body_scope);
        let depth = self.scope_depth() - 1;
        self.fs().breaks.push((exit, depth));

        let mut cases: Vec<(i128, BlockId)> = Vec::new();
        let mut default: Option<BlockId> = None;
        // Statements before the first label are unreachable but may still declare —
        // `switch (x) { int y; case 1: … }` is exactly that, and the declaration is why
        // the scope exists at all.
        let mut cur: Option<BlockId> = None;

        for &st in &stmts {
            let kind = self.ast.stmt(st).kind.clone();
            let sspan = self.ast.stmt(st).span;
            match kind {
                StmtKind::Case { lo, hi, body } => {
                    let b = self.new_block();
                    // Fallthrough: the previous case's block flows into this one.
                    if let Some(prev) = cur {
                        self.switch_to(prev);
                        self.goto_if_open(b);
                    }
                    self.switch_to(b);
                    // The enter goes **here**, in the block the switch jumps to — not at
                    // the lexical top the `Switch` terminator leaps over.
                    self.emit(
                        InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                            scope: body_scope,
                            kind: ScopeKind::Enter,
                        })),
                        sspan,
                    );
                    let lo_v = self.const_of(lo).unwrap_or(0);
                    let hi_v = hi.and_then(|h| self.const_of(h)).unwrap_or(lo_v);
                    // A range becomes one case per value (contract 18). Kept as a single
                    // entry, the engine would take the default for every value inside it.
                    for v in lo_v..=hi_v.max(lo_v) {
                        cases.push((v, b));
                    }
                    self.stmt(body);
                    cur = Some(self.fs().cur);
                }
                StmtKind::Default { body } => {
                    let b = self.new_block();
                    if let Some(prev) = cur {
                        self.switch_to(prev);
                        self.goto_if_open(b);
                    }
                    self.switch_to(b);
                    self.emit(
                        InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                            scope: body_scope,
                            kind: ScopeKind::Enter,
                        })),
                        sspan,
                    );
                    default = Some(b);
                    self.stmt(body);
                    cur = Some(self.fs().cur);
                }
                _ => {
                    match cur {
                        // Inside a case: an ordinary statement. **`cur` is refreshed
                        // afterwards**, because the statement may have terminated its
                        // block and moved on — a `break` does exactly that. Leaving `cur`
                        // stale made the *next* case label re-terminate the block the
                        // `break` had already pointed at the switch's exit, silently
                        // turning `case 2: t += 2; break;` into a fallthrough to
                        // `default`.
                        Some(_) => {
                            self.stmt(st);
                            cur = Some(self.fs().cur);
                        }
                        // Before any label: reachable only as a declaration, and lowering
                        // it here would run it on no path. The alloca is what matters and
                        // `local_decl` records that without emitting anything reachable.
                        None => {
                            if let StmtKind::Decl(ds) = kind {
                                for d in ds {
                                    self.declare_local_slot(d);
                                }
                            }
                        }
                    }
                }
            }
        }

        // The last case falls out of the body: exit the scope it entered.
        if let Some(last) = cur {
            self.switch_to(last);
            self.emit(
                InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    scope: body_scope,
                    kind: ScopeKind::Exit,
                })),
                span,
            );
            self.goto_if_open(exit);
        }
        self.fs().breaks.pop();
        self.fs().open_scopes.pop();

        cases.sort_by_key(|(v, _)| *v);
        cases.dedup_by_key(|(v, _)| *v);
        self.switch_to(head);
        self.set_term(Terminator::Switch {
            scrut,
            ty,
            cases,
            default: default.unwrap_or(exit),
        });
        self.switch_to(exit);
    }

    /// Reserve a local's slot without emitting anything.
    ///
    /// A declaration before the first `case` label is on no path, but its object still
    /// exists for the whole scope — `switch (x) { int y; case 1: y = 1; }` is legal C and
    /// `y` must have somewhere to live.
    fn declare_local_slot(&mut self, d: chiero_ast::DeclId) {
        let DeclKind::Var { name, .. } = self.ast.decl(d).kind.clone() else {
            return;
        };
        let Some(sty) = self.analysis.ty_of_decl(d) else {
            return;
        };
        let span = self.ast.decl(d).span;
        let cty = self.cty(sty);
        let align = self.analysis.align_of(sty).unwrap_or(1).max(1);
        let text = name.and_then(|n| self.sym(n));
        let slot = self.alloca_for(sty, align, text, span);
        if let Some(n) = name {
            self.fs().locals.insert(n, (slot, cty));
        }
    }

    fn const_of(&mut self, e: ExprId) -> Option<i128> {
        let mut diags = Vec::new();
        match chiero_sema::const_eval(self.ast, e, self.names, self.target(), &mut diags) {
            Some(chiero_sema::ConstVal::Int(v)) => Some(v),
            _ => None,
        }
    }

    /// Re-scope a jump: `Exit` every scope it leaves, then `Enter` every scope it lands
    /// in.
    ///
    /// Comparing the two *lists* rather than two depths is what makes a `goto` between
    /// sibling scopes correct: both sides can be at depth 2 and be in **different**
    /// scopes, and a depth comparison would emit nothing while the jump really does leave
    /// one and enter another.
    ///
    /// The **enters** are contract 9c, and they are the mirror of the exits for the same
    /// reason 021 §4 gives: a scope's objects are created on `Scope(Enter)`, so a jump
    /// that lands inside a scope without entering it leaves everything there
    /// unmaterialized and the eventual exit retires objects that never existed. C permits
    /// the jump (C11 6.8.6.1).
    ///
    /// Order matters in both directions and they are opposites: **exits innermost first**,
    /// because an inner scope's objects sit inside the outer one's storage; **enters
    /// outermost first**, because they have nowhere to go until the outer one exists.
    /// A backward jump that re-enters a scope it is already inside emits nothing for it —
    /// the scope is in both lists — which is why re-entry needs the *fallthrough* edge to
    /// have entered it too, and it did.
    fn unwind_leaving(&mut self, from: &[ScopeId], to: &[ScopeId], span: Span) {
        for id in from.iter().rev() {
            if to.contains(id) {
                continue;
            }
            let id = *id;
            self.generated(|s| {
                s.emit(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: id,
                        kind: ScopeKind::Exit,
                    })),
                    span,
                )
            });
        }
        for id in to.iter() {
            if from.contains(id) {
                continue;
            }
            let id = *id;
            self.generated(|s| {
                s.emit(
                    InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        scope: id,
                        kind: ScopeKind::Enter,
                    })),
                    span,
                )
            });
        }
    }

    /// Resolve forward `goto`s once every label in the function is known.
    fn resolve_gotos(&mut self) {
        let pending = std::mem::take(&mut self.fs().pending_gotos);
        for (block, name, open_at_jump, span) in pending {
            let Some((target, at_label)) = self.fs().labels.get(&name).cloned() else {
                let text = self.text(name).unwrap_or("?").to_owned();
                self.diagnostics.push(LowerDiagnostic {
                    span,
                    message: format!("`goto` to an undefined label `{text}`"),
                });
                continue;
            };
            self.switch_to(block);
            self.unwind_leaving(&open_at_jump, &at_label, span);
            self.set_term(Terminator::Goto(target));
        }
    }
}

impl Lowerer<'_> {
    fn aggregate_size_of_ty(&mut self, t: TyId) -> Option<u64> {
        matches!(
            self.analysis.ty(t),
            Ty::Record(_) | Ty::Array { .. } | Ty::Vector { .. }
        )
        .then(|| self.analysis.size_of(t))?
    }

    /// Store a braced initializer's written elements at their offsets in `base`.
    ///
    /// Offsets come from `RecordLayout` for a record and from the element size for an
    /// array — the same single source as every other member access (015 contract 7). A
    /// designator moves the cursor; everything after it continues from there, which is
    /// what C11 6.7.9p17 says and what makes `{1, .c = 3, 4}` place `4` after `c`.
    fn init_list(&mut self, base: Operand, ty: TyId, init: ExprId, span: Span) {
        let chiero_ast::ExprKind::InitList(items) = self.ast.expr(init).kind.clone() else {
            return;
        };
        // The fields to walk, as (byte offset, type), in declaration order.
        let slots: Vec<(u64, TyId, Option<chiero_span::Symbol>)> =
            match self.analysis.ty(ty).clone() {
                Ty::Record(r) => {
                    let l = self.analysis.layout(r);
                    l.fields.iter().map(|f| (f.offset, f.ty, f.name)).collect()
                }
                Ty::Array { elem, len } => {
                    let n = match len {
                        chiero_sema::ArrayLen::Fixed(n) => n,
                        _ => items.len() as u64,
                    };
                    let esz = self.analysis.size_of(elem).unwrap_or(1);
                    (0..n).map(|i| (i * esz, elem, None)).collect()
                }
                _ => return,
            };

        let mut cursor = 0usize;
        for item in items {
            // A designator repositions the cursor; C11 6.7.9p17 continues from there.
            if let Some(chiero_ast::Designator::Field(name)) = item.designators.first() {
                if let Some(i) = slots.iter().position(|(_, _, n)| *n == Some(*name)) {
                    cursor = i;
                }
            } else if let Some(chiero_ast::Designator::Index(idx)) = item.designators.first()
                && let Some(v) = self.const_of(*idx)
            {
                cursor = v.max(0) as usize;
            }
            let Some(&(off, fty, _)) = slots.get(cursor) else {
                break;
            };
            cursor += 1;

            let addr = if off == 0 {
                base.clone()
            } else {
                let dst = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst,
                        rv: RValue::PtrAdd {
                            base: base.clone(),
                            off: Operand::Const(Const::Int {
                                bits: 64,
                                val: off as i128,
                            }),
                        },
                    },
                    span,
                );
                Operand::Value(dst)
            };

            // A nested braced initializer recurses; anything else is a scalar store.
            if matches!(
                self.ast.expr(item.value).kind,
                chiero_ast::ExprKind::InitList(_)
            ) {
                self.init_list(addr, fty, item.value, span);
                continue;
            }
            let v = self.expr(item.value);
            let cty = self.cty(fty);
            let align = self.analysis.align_of(fty).unwrap_or(1).max(1);
            self.emit(
                InstKind::Store {
                    addr,
                    val: v,
                    ty: cty,
                    align,
                    vol: Volatility::Normal,
                },
                span,
            );
        }
    }
}
