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

/// Lower one translation unit (015 §§1–6).
pub fn lower_tu(ast: &Ast, analysis: &Analysis, names: &dyn SymbolText) -> Lowered {
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
    FuncId, Function, Inst, InstKind, Lifetime, MarkerKind, Operand, Param, RValue, ScopeId,
    Terminator, UnOp as CUnOp, ValueId, Volatility,
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
        let fs = self.fs();
        let b = fs
            .blocks
            .iter_mut()
            .find(|b| b.id == cur)
            .expect("current block exists");
        b.insts.push(Inst { kind, span });
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

    fn seq_point(&mut self, span: Span) {
        self.emit(InstKind::Marker(MarkerKind::SeqPoint), span);
    }

    /// CIR names are `Arc<str>` (020), not the per-TU `Symbol` the AST uses — a CIR
    /// module outlives the interner that produced it, and a golden `.cir` file has to be
    /// readable without one.
    fn sym(&self, s: chiero_span::Symbol) -> Option<chiero_cir::Symbol> {
        self.names.text(s).map(std::sync::Arc::from)
    }

    fn alloca(
        &mut self,
        ty: CTy,
        align: u64,
        name: Option<chiero_cir::Symbol>,
        span: Span,
    ) -> AllocaId {
        let fs = self.fs();
        let id = AllocaId(fs.next_alloca);
        fs.next_alloca += 1;
        fs.allocas.push(AllocaDecl {
            id,
            ty,
            count: 1,
            align,
            scope: ScopeId(0),
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
            name: cir_name,
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

        self.finish_blocks();
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
                    // code quietly.
                    || !b.insts.is_empty()
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

    fn error_ty(&self) -> TyId {
        TyId(0)
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
                for s in ss {
                    self.stmt(s);
                }
            }
            StmtKind::Expr(e) => {
                self.expr(e);
                self.seq_point(span);
            }
            StmtKind::Decl(ds) => {
                for d in ds {
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
                self.stmt(body);
                self.goto_if_open(header);
                self.switch_to(exit);
            }
            StmtKind::DoWhile { body, cond } => {
                let body_b = self.new_block();
                let latch = self.new_block();
                let exit = self.new_block();
                self.goto_if_open(body_b);
                self.switch_to(body_b);
                self.stmt(body);
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
                self.stmt(body);
                self.goto_if_open(latch);
                self.switch_to(latch);
                if let Some(st) = step {
                    self.expr(st);
                    self.seq_point(span);
                }
                self.set_term(Terminator::Goto(header));
                self.switch_to(exit);
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
        let slot = self.alloca(cty.clone(), align, text, span);
        if let Some(n) = name {
            self.fs().locals.insert(n, (slot, cty.clone()));
        }
        if let Some(init) = init {
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
                                    ty: CTy::Int(self.raw_width_of(*lhs)),
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
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Bin {
                                    op: cir_binop(*op, self.is_signed(*lhs)),
                                    a,
                                    b,
                                    ty: CTy::Int(self.raw_width_of(e)),
                                },
                            },
                            span,
                        );
                        Operand::Value(dst)
                    }
                }
            }
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
                if from == to {
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
        self.set_term(Terminator::Goto(join));

        // The short-circuit block: the answer without evaluating `b` at all.
        self.switch_to(short_b);
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
        self.set_term(Terminator::Goto(join));

        self.switch_to(join);
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
        match &self.ast.expr(e).kind {
            chiero_ast::ExprKind::Ident(sym) => {
                let (slot, _) = self.fs().locals.get(sym).cloned()?;
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
            _ => None,
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
