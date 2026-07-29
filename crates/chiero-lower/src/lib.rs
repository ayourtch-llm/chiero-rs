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
    lower(ast, analysis, names, None, None)
}

/// Lower one translation unit, recording which build configuration produced it
/// (020 §4.4, contract 30).
///
/// **A plain `u64`, not `chiero_pp::ConfigId`.** Lowering has no other reason to depend on
/// the preprocessor, and 020 §3 already declares `Module::config` as an untyped id for the
/// same reason: hand-written `.cir` legitimately has no build configuration at all.
///
/// The id matters because endianness-conditional layouts are resolved *before* CIR — the
/// two bitfield orderings are two `#if` branches — so a `BitRange` alone does not say
/// which world it belongs to.
pub fn lower_tu_with_config(
    ast: &Ast,
    analysis: &Analysis,
    names: &dyn SymbolText,
    map: Option<&chiero_span::SourceMap>,
    config: Option<u64>,
) -> Lowered {
    lower(ast, analysis, names, map, config)
}

/// Lower one translation unit and compute `gcov_lines` (015 §5).
pub fn lower_tu_with_map(
    ast: &Ast,
    analysis: &Analysis,
    names: &dyn SymbolText,
    map: &chiero_span::SourceMap,
) -> Lowered {
    lower(ast, analysis, names, Some(map), None)
}

fn lower(
    ast: &Ast,
    analysis: &Analysis,
    names: &dyn SymbolText,
    map: Option<&chiero_span::SourceMap>,
    config: Option<u64>,
) -> Lowered {
    let mut cx = Lowerer {
        ast,
        analysis,
        names,
        module: Module {
            funcs: Vec::new(),
            globals: Vec::new(),
            config,
            metadata: IndexMap::new(),
        },
        diagnostics: Vec::new(),
        f: None,
        next_value: 0,
        next_func: 0,
        map,
        last_stmt_value: None,
        generated_depth: 0,
        globals: IndexMap::new(),
        strings: IndexMap::new(),
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
    refuse_unverifiable(&mut cx.module, &mut cx.diagnostics);
    Lowered {
        module: cx.module,
        diagnostics: cx.diagnostics,
    }
}

fn refuse_unverifiable(module: &mut chiero_cir::Module, diagnostics: &mut Vec<LowerDiagnostic>) {
    let mut blamed: Vec<(chiero_cir::FuncId, String, Span)> = Vec::new();
    for e in chiero_cir::verify::verify(module) {
        if !e.is_error() {
            continue;
        }
        if blamed.iter().any(|(f, _, _)| *f == e.func) {
            // One diagnostic per function, not per instruction: 015 §7's rule that a
            // partial body is worse than none makes the first error decisive, and a
            // malformed function usually produces several.
            continue;
        }
        blamed.push((e.func, e.detail.clone(), e.span));
    }
    for (id, detail, span) in blamed {
        let Some(f) = module.funcs.iter_mut().find(|f| f.id == id) else {
            continue;
        };
        let name = f.name.clone();
        f.blocks.clear();
        f.allocas.clear();
        f.access_paths.clear();
        f.body = Body::Declared;
        diagnostics.push(LowerDiagnostic {
            span,
            message: format!(
                "`{}` lowered to CIR the verifier rejects ({detail}), so it was skipped",
                &*name
            ),
        });
    }
}

// ---------------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------------

use chiero_ast::{DeclKind, ExprId, StmtId, StmtKind};
use chiero_cir::{
    AccessPath, AllocaDecl, AllocaId, BinOp as CBinOp, Block, BlockId, Body, CTy, Callee, Const,
    FnAttrs, FuncId, Function, Inst, InstKind, Lifetime, MarkerKind, Operand, Param, PathRoot,
    PathStep, RValue, ScopeEvent, ScopeId, ScopeKind, Terminator, UnOp as CUnOp, ValueId,
    Volatility,
};
use chiero_sema::{Conversion, FloatKind, Ty, TyId, TypedId, TypedNode};
use indexmap::IndexMap;

/// The function currently being built.
struct FnState {
    id: FuncId,
    /// 015 §2's aggregate return: the hidden first parameter holding the **caller's**
    /// slot, when this function returns a struct, union or array by value.
    ///
    /// `None` for every scalar-returning function, which is the overwhelming majority —
    /// the ABI change is confined to the functions that need it.
    sret: Option<ValueId>,
    /// 020 §4.4's reporting-only paths, keyed by the address value they describe.
    access_paths: IndexMap<ValueId, AccessPath>,
    /// 020 §7, contract 18a. Computed from the **syntax** before any lowering happens, so
    /// it is a property of what was written rather than of the order lowering chose —
    /// which is the point: CIR picks one order, and this records that the source did not.
    order_sensitive: bool,
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

/// The widest `case lo ... hi` range still enumerated one value at a time.
///
/// A policy number, not a contract: what 020 contract 14 fixes is that a wide range is
/// *bounded*, not where the boundary sits. 64 is comfortably above every range a hand-
/// written switch uses and far below VPP's protocol-number spans.
const MAX_ENUMERATED_CASE_RANGE: i128 = 64;

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
    /// File-scope variable name → its `Global` (020 §3).
    ///
    /// Registered in the declaration pass, before any body is lowered, because a function
    /// may reference a global declared *after* it.
    globals: IndexMap<chiero_span::Symbol, chiero_cir::GlobalId>,
    /// One global per **distinct** string literal.
    ///
    /// Pooled because a string literal's value is an address: `"hi" == "hi"` is
    /// unspecified in C, but the corpus assumes one object, and two globals would make
    /// the engine report the addresses unequal — a difference no source line asked for.
    strings: IndexMap<Vec<u8>, chiero_cir::GlobalId>,
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
        // A file-scope variable is a `Global`; a function is a signature. Both are
        // registered here, before any body, for the same reason.
        if matches!(self.ast.decl(id).kind, DeclKind::Var { .. }) {
            self.declare_global(id);
            return;
        }
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
        // **The hidden sret slot is part of the declared signature too** (015 §2). The
        // declaration pass fixes each function's arity, and a caller that prepends the
        // slot against a signature built without it fails `CallArity` — which is the
        // verifier catching a real inconsistency, not a nuisance.
        let sret: Vec<Param> = if self
            .analysis
            .ty_of_decl(id)
            .map(|t| self.analysis.ty(t).clone())
            .and_then(|t| match t {
                Ty::Func { ret, .. } => Some(ret),
                _ => None,
            })
            .is_some_and(|r| self.is_aggregate(r))
        {
            vec![Param {
                value: self.new_value(),
                ty: CTy::Ptr,
            }]
        } else {
            Vec::new()
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
            .collect::<Vec<Param>>();
        let cparams: Vec<Param> = sret.into_iter().chain(cparams).collect();
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
            access_paths: Default::default(),
            body: Body::Declared,
            span,
        });
    }

    /// Register a file-scope variable as a `Global` (020 §3).
    ///
    /// **In the declaration pass**, so a function body can reference a global declared
    /// after it — C allows that for anything with a prior tentative definition, and VPP
    /// does it constantly.
    fn declare_global(&mut self, id: chiero_ast::DeclId) {
        let DeclKind::Var {
            name: Some(name),
            init,
            storage,
            ty: decl_ty,
            ..
        } = self.ast.decl(id).kind.clone()
        else {
            return;
        };
        if self.globals.contains_key(&name) {
            return;
        }
        let Some(sty) = self.analysis.ty_of_decl(id) else {
            return;
        };
        let span = self.ast.decl(id).span;
        let size = self.analysis.size_of(sty).unwrap_or(0);
        let align = self.analysis.align_of(sty).unwrap_or(1).max(1);
        let text = self.sym(name).unwrap_or_else(|| std::sync::Arc::from("?"));
        let gid = chiero_cir::GlobalId(self.module.globals.len() as u32);
        // **C11 6.7.9p10: static storage with no initializer is zero**, and `Extern` is
        // for a declaration whose definition is in another TU — its bytes are unknown, not
        // zero, and saying zero would let the engine prove things about a value it has
        // never seen.
        let init = if storage.extern_ && init.is_none() {
            chiero_cir::GlobalInit::Extern
        } else {
            // **An address initializer is an address, not bytes.** `int *gp = &g;` has no
            // byte encoding that carries provenance, and falling through to `Zero` below
            // made `gp == 0` answer *true* for a pointer that is definitely not null.
            match init.and_then(|e| self.global_addr_init(e)) {
                Some((g, off)) => chiero_cir::GlobalInit::Addr { g, off },
                None => match init.and_then(|e| self.encode_init(e, sty, size)) {
                    Some(bytes) => chiero_cir::GlobalInit::Bytes(bytes),
                    // **`Zero`, not a partial encoding.** An initializer chiero cannot encode
                    // must not become bytes for the part it understood: the rest would read as
                    // zeros the program never wrote, which is the confidently-wrong direction.
                    // `Zero` for an *uninitialized* object is C11 6.7.9p10 and correct; for one
                    // chiero failed to encode it is at least not a fabrication.
                    None => chiero_cir::GlobalInit::Zero,
                },
            }
        };
        let linkage = if storage.static_ {
            chiero_cir::Linkage::Internal
        } else {
            chiero_cir::Linkage::External
        };
        self.module.globals.push(chiero_cir::Global {
            id: gid,
            name: text,
            size,
            align,
            // **Read-only if the object's own type says so.** Hardcoded `false` here made
            // 021 contract 21 — "writing to a `readonly` global is exactly one finding" —
            // correct and unreachable: nothing marked a global read-only, so the checker
            // could never fire. VPP's tables are `const` precisely so writing to one is a
            // bug.
            is_const: self.is_const_type(decl_ty),
            init,
            linkage,
            span,
        });
        self.globals.insert(name, gid);
    }

    /// Whether a declared type is `const`-qualified.
    ///
    /// **An array looks at its element.** `const int t[4]` puts the qualifier on the
    /// element type, not on the array — C11 6.7.3p9 says the array takes it from there —
    /// and checking only the outer type misses every `const` table, which is the shape
    /// that matters.
    fn is_const_type(&self, ty: chiero_ast::TypeId) -> bool {
        let t = self.ast.ty(ty);
        if t.quals.const_ {
            return true;
        }
        match &t.kind {
            chiero_ast::TypeKind::Array { elem, .. } => self.is_const_type(*elem),
            _ => false,
        }
    }

    /// Encode a file-scope initializer into `size` bytes, or `None` if chiero cannot.
    ///
    /// **Zero-filled to the object's full size** (C11 6.7.9p21): a partial initializer
    /// leaves the remainder zero, and stopping at the last written element would make every
    /// consumer reading past it see the end of a byte string rather than the zeros the
    /// standard promises.
    fn encode_init(&mut self, e: ExprId, ty: TyId, size: u64) -> Option<Vec<u8>> {
        let mut out = vec![0u8; size as usize];
        self.encode_into(e, ty, 0, &mut out)?;
        Some(out)
    }

    /// Write `e`'s encoding at `at` in `out`. **The layout is the only source of offsets**
    /// (015 c7), so struct padding falls out rather than being computed a second way.
    /// A floating constant expression's kind and value.
    ///
    /// Only a literal, possibly negated. That is what a static initializer is allowed to be
    /// beyond constant folding chiero does not do, and answering `None` for anything else
    /// keeps the caller's `Zero` fallback from turning an expression it cannot read into a
    /// silent zero — the failure this function was added to stop.
    fn float_const(&mut self, e: ExprId) -> Option<(chiero_cir::FloatKind, f64)> {
        match self.ast.expr(e).kind.clone() {
            chiero_ast::ExprKind::Number(sym) => {
                let text = self.names.text(sym)?;
                chiero_sema::float_literal(text).map(|(k, v)| (cir_float_kind(k), v))
            }
            chiero_ast::ExprKind::Unary {
                op: chiero_ast::UnOp::Minus,
                operand,
            } => self.float_const(operand).map(|(k, v)| (k, -v)),
            // A cast of a literal, which is how `(float)1.5` and `1.5f` both reach here.
            chiero_ast::ExprKind::Cast { operand, .. } => {
                let (_, v) = self.float_const(operand)?;
                let k = self.float_kind(e)?;
                Some((k, v))
            }
            _ => None,
        }
    }

    fn encode_into(&mut self, e: ExprId, ty: TyId, at: u64, out: &mut [u8]) -> Option<()> {
        match self.analysis.ty(ty).clone() {
            Ty::Array { elem, len } => {
                let esz = self.analysis.size_of(elem)?;
                match self.ast.expr(e).kind.clone() {
                    chiero_ast::ExprKind::InitList(items) => {
                        // **A designator moves the cursor and the walk continues from
                        // there** (C11 6.7.9p17), which is what `init_list` already does
                        // for a local. This used to return `None` — and the caller turns
                        // `None` into `Zero`, so an initializer with a designator was not
                        // "refused whole" as its comment said but silently replaced by
                        // zeros.
                        let mut cursor = 0u64;
                        for it in items.iter() {
                            if let Some(chiero_ast::Designator::Index(idx)) = it.designators.first()
                            {
                                let k = self.const_of(*idx)?;
                                if k < 0 {
                                    return None;
                                }
                                cursor = k as u64;
                            } else if !it.designators.is_empty() {
                                // A `.field` designator on an array is not C; refuse rather
                                // than guess an offset for it.
                                return None;
                            }
                            // A fixed bound stops the walk; any other kind (flexible,
                            // zero-length, VLA) has no compile-time extent, and the
                            // caller's `size` bound already clips the writes.
                            if let chiero_sema::ArrayLen::Fixed(n) = len
                                && cursor >= n
                            {
                                break;
                            }
                            self.encode_into(it.value, elem, at + cursor * esz, out)?;
                            cursor += 1;
                        }
                        Some(())
                    }
                    // `char s[4] = "hi"` — the literal's bytes, truncated to the array and
                    // zero-filled by the caller.
                    chiero_ast::ExprKind::Str { .. } => {
                        let bytes = self.string_bytes(e)?;
                        for (i, b) in bytes.iter().enumerate() {
                            let o = at as usize + i;
                            if o >= out.len() {
                                break;
                            }
                            out[o] = *b;
                        }
                        Some(())
                    }
                    _ => None,
                }
            }
            Ty::Record(r) => {
                let chiero_ast::ExprKind::InitList(items) = self.ast.expr(e).kind.clone() else {
                    return None;
                };
                let fields = self.analysis.layout(r).fields.clone();
                let mut cursor = 0usize;
                for it in items.iter() {
                    // `.field` repositions the cursor, and the walk continues from there —
                    // the same rule as the array case above and as `init_list`.
                    if let Some(chiero_ast::Designator::Field(name)) = it.designators.first() {
                        cursor = fields.iter().position(|f| f.name == Some(*name))?;
                    } else if !it.designators.is_empty() {
                        return None;
                    }
                    let f = fields.get(cursor)?.clone();
                    cursor += 1;
                    match f.bits {
                        // **A bit-field is written into its bits, not over its bytes.** The
                        // old code refused here to avoid clobbering neighbours — correct
                        // about the hazard, wrong about the outcome, since refusing meant
                        // the whole object became zeros. `RecordLayout` already carries the
                        // absolute bit offset and the width, so the bits go where 014 put
                        // them and the neighbours are untouched by construction.
                        Some(b) => {
                            let v = self.const_of(it.value)?;
                            for i in 0..b.width {
                                let bit = (v >> i) & 1;
                                let abs = b.bit_offset + i;
                                let byte = at as usize + (abs / 8) as usize;
                                if byte >= out.len() {
                                    break;
                                }
                                let mask = 1u8 << (abs % 8);
                                if bit == 1 {
                                    out[byte] |= mask;
                                } else {
                                    out[byte] &= !mask;
                                }
                            }
                        }
                        None => self.encode_into(it.value, f.ty, at + f.offset, out)?,
                    }
                }
                Some(())
            }
            // **A floating initializer is its bit pattern.** `const_of` answers about
            // *integer* constant expressions, so `double g = 2.0;` came back `None` — and
            // the caller reads `None` as `GlobalInit::Zero`, so the global was silently
            // zero rather than refused. The generator caught it as a value mismatch the
            // moment floats stopped being refused outright: 37 programs, all reading a
            // float global as 0.
            Ty::Float(k) => {
                let (kind, val) = self.float_const(e)?;
                let bits = float_bits(kind, val);
                let sz = self.analysis.size_of(ty)?.min(8);
                let _ = k;
                for i in 0..sz {
                    let o = at as usize + i as usize;
                    if o >= out.len() {
                        break;
                    }
                    out[o] = ((bits >> (8 * i)) & 0xff) as u8;
                }
                Some(())
            }
            _ => {
                let v = self.const_of(e)?;
                let sz = self.analysis.size_of(ty)?;
                // Little-endian, matching 020's target and `GlobalInit::Bytes`'s reader.
                for i in 0..sz {
                    let o = at as usize + i as usize;
                    if o >= out.len() {
                        break;
                    }
                    out[o] = ((v >> (8 * i)) & 0xff) as u8;
                }
                Some(())
            }
        }
    }

    /// A string literal's bytes.
    fn string_bytes(&mut self, e: ExprId) -> Option<Vec<u8>> {
        let chiero_ast::ExprKind::Str { fragments } = self.ast.expr(e).kind.clone() else {
            return None;
        };
        // **Each element is written at the literal's width** (C11 6.4.5p6): `L"AB"` is
        // `wchar_t[3]`, four bytes per element, not `41 42 00`. The prefix is read from the
        // first fragment — C11 6.4.5p2 makes concatenating literals of different prefixes
        // either ill-formed or the wider of the two, and the parser has already joined
        // them, so the first is the one that named the type sema used.
        let bits = fragments
            .first()
            .and_then(|fr| self.names.text(fr.spelling))
            .map(|t| chiero_sema::strlit::string_element(t).1)
            .unwrap_or(8);
        let width = (bits / 8) as usize;
        let mut bytes = Vec::new();
        for fr in &fragments {
            let text = self.names.text(fr.spelling).unwrap_or("").to_owned();
            // **The same decoder sema counted with.** These bytes and the array bound are
            // two views of one list; deriving them from separate readings of the spelling
            // is how `sizeof` and the contents came to describe different arrays.
            for el in chiero_sema::strlit::string_elements(&text, bits) {
                // Little-endian, matching 020's target and `GlobalInit::Bytes`'s reader.
                for i in 0..width {
                    bytes.push(((el >> (8 * i)) & 0xff) as u8);
                }
            }
        }
        // **No explicit terminator.** The caller zero-fills the object to its full size
        // before writing, so appending one is invisible in every case — `char s[4] = "hi"`
        // and `char s[2] = "hi"` (which C allows, with no room for a terminator) both come
        // out right without it. Mutation could not tell the two apart, which is what said
        // it was redundant rather than untested.
        Some(bytes)
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
            sret: None,
            access_paths: IndexMap::new(),
            order_sensitive: order_sensitive_body(self.ast, body),
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

        // **Parameters live in a scope that encloses the body** (C11 6.2.1p4: a parameter's
        // scope is the function body's *enclosing* one), and this is not a formality.
        //
        // Without it, a parameter slot took `ScopeId(0)` — `open_scopes` is empty here —
        // and the body's own compound statement then opened `ScopeId(0)` too, because
        // `next_scope` also starts at 0. Entering a scope creates fresh objects for the
        // locals in it, which is exactly what 020 §4.4 says scope markers are for, so
        // every parameter's slot was replaced *after* the prologue stored into it. The
        // read that followed was of an object nobody had written, and every function that
        // read its own scalar parameter reported an `uninitialized-read` — 021 §3.1's
        // false-positive storm, in the one shape almost all C has.
        //
        // The same shape 015 contract 12 already fixes for `for (int i = 0; …)`: the init
        // declaration lives in a scope enclosing the body, or it would be retired and
        // recreated on every iteration.
        let param_scope = self.enter_scope(span);
        let _ = param_scope;

        // **015 §2's aggregate return: a hidden first parameter.** A function returning a
        // struct by value takes the caller's slot as a pointer and writes through it, so
        // the result lives in memory the caller owns. Returning the callee's own
        // `addrlocal` handed back an address whose scope exits on the way out.
        //
        // Confined to the functions that need it: every scalar-returning function keeps
        // the signature it had, so nothing else in the ABI moves.
        if self
            .analysis
            .ty_of_decl(decl)
            .map(|t| self.analysis.ty(t).clone())
            .and_then(|t| match t {
                Ty::Func { ret, .. } => Some(ret),
                _ => None,
            })
            .is_some_and(|r| self.is_aggregate(r))
        {
            let v = self.new_value();
            self.fs().params.push(Param {
                value: v,
                ty: CTy::Ptr,
            });
            self.fs().sret = Some(v);
        }

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
            // **A struct parameter is an object of its own** (C11 6.9.1p9: the parameter's
            // value is a *copy* of the argument). The caller passes the aggregate's address,
            // so a slot of the parameter's lowered `CTy` is eight bytes of `CTy::Ptr` — and
            // the prologue stored the address into it, after which `p.lo` read the low half
            // of a pointer. No fault and no finding, just a wrong number.
            //
            // So the slot is the struct's *size*, and the prologue copies through the
            // incoming pointer instead of storing it. That is also what makes a callee
            // mutating its parameter leave the caller's struct alone.
            let aggregate = self.is_aggregate(sty);
            let slot = if aggregate {
                // `n` bytes: the count carries the size and the type carries the byte,
                // which is the shape `local_decl` already uses for an aggregate local.
                let size = self.analysis.size_of(sty).unwrap_or(1).max(1);
                self.alloca_n(CTy::Int(8), size, align, pn_text, span)
            } else {
                self.alloca(cty.clone(), align, pn_text, span)
            };
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
            if aggregate {
                let size = self.analysis.size_of(sty).unwrap_or(0);
                self.emit(
                    InstKind::CopyMem {
                        dst: Operand::Value(addr),
                        src: Operand::Value(v),
                        size: Operand::Const(Const::Int {
                            bits: 64,
                            val: size as i128,
                        }),
                        align,
                    },
                    span,
                );
                continue;
            }
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
        self.exit_scope(span);

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
            access_paths: fs.access_paths,
            attrs: FnAttrs {
                order_sensitive: fs.order_sensitive,
                ..FnAttrs::default()
            },
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
                // **An aggregate result is written through the caller's pointer.**
                // Returning `addrlocal` of the callee's own slot returned an address whose
                // scope exits on the way out, so the caller copied from retired bytes and
                // the engine reported a wild pointer. 015 §2 says the result is memory;
                // this makes it memory the *caller* owns.
                if let Some(sret) = self.fs().sret
                    && let Some(e) = v
                {
                    let src = self.expr(e);
                    let size = self
                        .type_of(e)
                        .and_then(|t| self.analysis.size_of(t))
                        .unwrap_or(0);
                    let align = self
                        .type_of(e)
                        .and_then(|t| self.analysis.align_of(t))
                        .unwrap_or(1)
                        .max(1);
                    self.emit(
                        InstKind::CopyMem {
                            dst: Operand::Value(sret),
                            src,
                            size: Operand::Const(Const::Int {
                                bits: 64,
                                val: size as i128,
                            }),
                            align,
                        },
                        span,
                    );
                    self.unwind_to(0, span);
                    // The value handed back is the caller's own pointer, so the caller's
                    // `CopyMem` (015 c6) reads bytes it owns and that are still live.
                    self.set_term(Terminator::Return(Some(Operand::Value(sret))));
                    // **The same dead block the scalar path opens.** Without it, everything
                    // emitted after the `return` — the enclosing compound's `Scope(Exit)`,
                    // and wave 109's trailing exit for the parameter scope — was appended
                    // *after* the terminator of a live block. The callee then exited each
                    // scope twice, 021 retired its stack objects twice, and the aggregate
                    // result resolved to no known object. Wave 128 spent a whole wave
                    // eliminating the engine and the call ABI before this was visible.
                    let dead = self.new_block();
                    self.switch_to(dead);
                    return;
                }
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
                let c = {
                    let t = self.compare_ty(cond);
                    self.truth_of(c, t, span)
                };
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
                let c = {
                    let t = self.compare_ty(cond);
                    self.truth_of(c, t, span)
                };
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
                let c = {
                    let t = self.compare_ty(cond);
                    self.truth_of(c, t, span)
                };
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
                        let c = {
                            let t = self.compare_ty(ce);
                            self.truth_of(c, t, span)
                        };
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
            // **An aggregate initializer is a copy, not a store** (015 contract 6: one
            // `CopyMem` of the *layout's* size, never a store of something else).
            //
            // CIR has no aggregate values (020 §1.4), so `struct pair p = make_pair(…)`
            // gives back a `Ptr` — and storing *that* put the pointer's eight bytes where
            // the struct belonged, so `p.lo` read the low half of an address as an `int`.
            // The program ran and every field was wrong. A struct returned by value is
            // what every VPP accessor in a header does.
            if matches!(cty, CTy::Ptr) && self.is_aggregate(sty) {
                let size = self.analysis.size_of(sty).unwrap_or(0);
                self.emit(
                    InstKind::CopyMem {
                        dst: Operand::Value(addr),
                        src: v,
                        size: Operand::Const(Const::Int {
                            bits: 64,
                            val: size as i128,
                        }),
                        align,
                    },
                    span,
                );
                self.seq_point(span);
                return;
            }
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
                // **A conversion to `_Bool` is `!= 0`, not a narrowing** (C11 6.3.1.2p1:
                // the result is 0 if the value compares equal to 0 and 1 otherwise).
                // `cast_kind` picked `Trunc` for `Int(32) -> Int(1)`, so `_Bool b = 2` kept
                // the low bit and stored **false** — and `b = 256` did too, while `b = -1`
                // gave the right answer by accident. It is the same rule `truth_of` states
                // for branch conditions, which this path never asked.
                //
                // `Int(1)` is `_Bool` and nothing else: a one-bit *bit-field* keeps its
                // declared `int` type and carries its width in a `BitRange` instead.
                if matches!(to, CTy::Int(1)) && !matches!(from, CTy::Int(1)) {
                    return self.truth_of(inner, from, span);
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
            // **A string literal is a global with its bytes** (020 §3/§6). Until this
            // existed a literal lowered to `Undef`: not an unknown pointer but *no*
            // pointer, so every `chiero_make_symbolic(&x, sizeof x, "x")` in the corpus
            // named its variable with a value 021 could not read a byte of.
            //
            // Adjacent fragments are concatenated first (C11 6.4.5p5) — `"a" "b"` is one
            // literal, and pooling on the joined bytes is what makes it equal to `"ab"`.
            chiero_ast::ExprKind::Str { fragments } => {
                // **Through `string_bytes`, not a second copy of it.** This arm used to
                // build the bytes itself, and when `string_bytes` learned about element
                // widths this one did not — `sizeof(L"AB")` became 12 while the object
                // behind it stayed `41 42 00`. Two places computing the same thing is two
                // places to remember, and the one nobody remembers is the bug.
                let mut bytes = self.string_bytes(e).unwrap_or_default();
                // The terminator is one *element*, not one byte: `L"AB"` ends in four zero
                // bytes, which is what makes its object 12 rather than 9.
                let width = fragments
                    .first()
                    .and_then(|fr| self.names.text(fr.spelling))
                    .map(|t| (chiero_sema::strlit::string_element(t).1 / 8) as usize)
                    .unwrap_or(1);
                bytes.extend(std::iter::repeat_n(0u8, width));
                let g = self.intern_string(bytes, span);
                Operand::Const(Const::GlobalAddr { g, off: 0 })
            }
            // **`sizeof` and `_Alignof` are constants**, and lowering them to `Undef` was a
            // silent drop the corpus golden made visible.
            //
            // `sizeof x` is asked of the real analysis rather than of `const_eval`: the
            // latter builds a throwaway context that sees only file-scope declarations, so
            // it cannot answer for a *local*, and the answer is a property of the operand's
            // type, which the typed AST already carries.
            chiero_ast::ExprKind::SizeofExpr(inner) => {
                let bits = self.raw_width_of(e).max(1);
                let n = self.type_of(*inner).and_then(|t| self.analysis.size_of(t));
                match n {
                    Some(v) => Operand::Const(Const::Int {
                        bits,
                        val: v as i128,
                    }),
                    None => Operand::Const(Const::Undef(CTy::Int(bits))),
                }
            }
            chiero_ast::ExprKind::SizeofType(_) | chiero_ast::ExprKind::AlignofType(_) => {
                let bits = self.raw_width_of(e).max(1);
                match self.const_of(e) {
                    Some(v) => Operand::Const(Const::Int { bits, val: v }),
                    None => Operand::Const(Const::Undef(CTy::Int(bits))),
                }
            }
            chiero_ast::ExprKind::Number(_) | chiero_ast::ExprKind::Char { .. } => {
                let mut diags = Vec::new();
                let v = chiero_sema::const_eval(self.ast, e, self.names, self.target(), &mut diags);
                let bits = self.raw_width_of(e);
                // **A floating literal is a float constant, not a zero.** The catch-all
                // below builds `Const::Int { val: 0 }`, which for a float-typed literal is
                // both the wrong value and the wrong type — the verifier rejects the store
                // it feeds, which is how wave 168 found this rather than by getting 0.
                if let Some(CTy::Float(k)) = self.type_of(e).map(|t| self.cty(t))
                    && let chiero_ast::ExprKind::Number(sym) = self.ast.expr(e).kind
                    && let Some(text) = self.names.text(sym)
                    && let Some((_, val)) = chiero_sema::float_literal(text)
                {
                    return Operand::Const(Const::Float(k, float_bits(k, val)));
                }
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
                // **An aggregate member names its address**, like an aggregate local —
                // `o.i` used as a value, or an array member decaying. Loading it would put
                // the nested object's first bytes where a pointer belongs.
                if self.is_address_only(e) {
                    return self
                        .lvalue_addr(e, span)
                        .unwrap_or(Operand::Const(Const::Undef(CTy::Ptr)));
                }
                let ty = self.cty_of(e);
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
                // The same rule: `*p` where `p` is a `struct S *`, and `a[k]` over an array
                // of structs, are aggregate lvalues too.
                if self.is_address_only(e) {
                    return self
                        .lvalue_addr(e, span)
                        .unwrap_or(Operand::Const(Const::Undef(CTy::Ptr)));
                }
                let ty = self.cty_of(e);
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
                    // **A file-scope variable is read through its address**, like any
                    // other object. This arm used to return `Undef` for anything that was
                    // not a local, with a comment calling that honest — it was not: a
                    // read of `g` became "unknown", which *suppresses* every finding
                    // downstream rather than producing a wrong one, so a whole translation
                    // unit's worth of defects went unreported and the run still said
                    // `Exact`. The global is modelled now; only a function name is not.
                    if let Some(&g) = self.globals.get(sym) {
                        let ty = self
                            .type_of(e)
                            .map(|t| self.cty(t))
                            .unwrap_or_else(|| CTy::Int(self.raw_width_of(e)));
                        let addr = self.new_value();
                        self.emit(
                            InstKind::Assign {
                                dst: addr,
                                rv: RValue::AddrOfGlobal { g },
                            },
                            span,
                        );
                        // **An aggregate names its own address; a scalar names its
                        // contents** — including a scalar of pointer type.
                        //
                        // The `CTy::Ptr` test alone was the *wrong* spelling of that, and
                        // said so in a comment calling the second condition redundant.
                        // Pointers are untyped in CIR (020 §2), so `int *gp` lowers to
                        // `CTy::Ptr` exactly as `int a[4]` does; this arm therefore handed
                        // back the address *of* `gp` where the program asked for the
                        // address `gp` holds, and `*gp` read `gp`'s own bytes as an `int`.
                        // The sema type is the only thing that distinguishes them, which is
                        // why the check has to reach past `cty`.
                        if self.is_address_only(e) {
                            return Operand::Value(addr);
                        }
                        let dst = self.new_value();
                        let align = self
                            .type_of(e)
                            .and_then(|t| self.analysis.align_of(t))
                            .unwrap_or(1)
                            .max(1);
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Load {
                                    addr: Operand::Value(addr),
                                    ty,
                                    align,
                                    vol: Volatility::Normal,
                                },
                            },
                            span,
                        );
                        return Operand::Value(dst);
                    }
                    // **A function name used as a value is its address** (C11 6.3.2.1p4:
                    // a function designator decays to a pointer). Falling through to
                    // `Undef` here made `int (*fn)(int) = twice;` store an `Int(32)` into a
                    // `Ptr` slot, which the verifier rejects — so the whole enclosing
                    // function was refused and `indirect_call.c` lowered to nothing.
                    if let Some(f) = self
                        .sym(*sym)
                        .and_then(|t| self.module.funcs.iter().find(|f| f.name == t).map(|f| f.id))
                    {
                        let dst = self.new_value();
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::AddrOfFunc(f),
                            },
                            span,
                        );
                        return Operand::Value(dst);
                    }
                    // **An enumeration constant is a constant, not a name to look up.**
                    // C11 6.4.4.3 makes it an `int` with the value sema computed while
                    // typing the enum. It is neither a local, a global, nor a function, so
                    // it reached the `Undef` below and every use of one lowered to
                    // `undef` — silently, since no diagnostic was pushed and 015 §7 had
                    // nothing to refuse the function for. A `switch` over enumerators then
                    // matched no arm and fell out with a plausible wrong answer.
                    // **Keyed by this reference**, not by the name: a function-local
                    // `enum { K = 2 }` and a file-scope `enum { K = 1 }` are both legal and
                    // both called `K`, and a by-name lookup gives the outer use the inner
                    // value. sema resolved the scope; this inherits the answer.
                    if let Some((v, _)) = self.analysis.enum_ref(e) {
                        // The *expression's* width, not `int`, so an enumerator used where
                        // 014 has already widened it to `long` is not a 32-bit operand in
                        // a 64-bit operation.
                        let bits = self.raw_width_of(e);
                        return Operand::Const(Const::Int { bits, val: v });
                    }
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
                // **The same rule as the global arm above**, and for the same reason: CIR
                // has no aggregate values (020 §1.4), so a struct, union or array named as
                // a value can only be its address. `cty` of every one of them is already
                // `CTy::Ptr`, which is why one test covers all three.
                //
                // Without it this arm loaded the object's first eight bytes and passed
                // them on *as a pointer*. `struct pair p = q;`, `int *p = a;`, a by-value
                // argument and an aggregate `return` all reach here — and the one
                // aggregate-copy path that had coverage, `y = x`, goes through
                // `lvalue_addr` and never does. The global arm got this guard when
                // `g[1]` indexed off the wrong base; the local arm did not, and the
                // resulting wild pointer cost waves 126 through 131.
                if self.is_address_only(e) {
                    return Operand::Value(addr);
                }
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
                // **`p + n` is not `add`** (020: PtrAdd-not-Add). The arm below types the
                // whole expression as `Int(raw_width_of(e))`, which for a pointer is 32,
                // and adds the index unscaled — so `*(a + 1)` addressed byte 1 of a
                // 64-bit address truncated to 32 bits. The subscript path had this right
                // all along; every other spelling of the same arithmetic did not.
                if let Some(v) = self.ptr_arith(*op, *lhs, *rhs, span) {
                    return v;
                }
                // **Left to right** (015 §2, normative): the left operand's side effects
                // are emitted before the right's.
                let a = self.expr(*lhs);
                let b = self.expr(*rhs);
                let dst = self.new_value();
                // CIR keeps comparisons in their own `RValue` (020), and signedness is a
                // property of the **operands**, not of the result — so it comes from the
                // typed AST rather than from the operator.
                // A float comparison is a different opcode set *and* may need its
                // operands the other way round, so the two are decided together.
                let fcmp = self.is_float(*lhs).then(|| cir_fcmpop(*op)).flatten();
                let (a, b) = match fcmp {
                    Some((_, true)) => (b, a),
                    _ => (a, b),
                };
                match fcmp
                    .map(|(c, _)| c)
                    .or_else(|| cir_cmpop(*op, self.is_signed(*lhs)))
                {
                    Some(cop) => {
                        // **Either side being an address makes it a pointer comparison.**
                        // `p == 0` has a pointer on the left and a null constant on the
                        // right; `0 == p` is the same comparison written the other way.
                        // `compare_ty(lhs)` alone would type the second `Int(32)`.
                        let cty = if self.is_address(*rhs) && !self.is_address(*lhs) {
                            CTy::Ptr
                        } else {
                            self.compare_ty(*lhs)
                        };
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
                                    ty: cty,
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
                        let bin_ty = self.compare_ty(*lhs);
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Bin {
                                    op: cir_binop(*op, self.is_signed(*lhs), self.is_float(*lhs)),
                                    a,
                                    b,
                                    // **The operands' type, not their width as an int.**
                                    // `CTy::Int(w)` is right for every integer operation
                                    // and names the wrong type for a float one — the
                                    // verifier rejects `FAdd` whose declared type is
                                    // `Int(32)` while its operands are `Float(F32)`.
                                    ty: bin_ty,
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
            } => self.inc_dec(
                e,
                *operand,
                matches!(op, chiero_ast::UnOp::PreInc),
                true,
                span,
            ),
            chiero_ast::ExprKind::Unary { op, operand } => {
                let a = self.expr(*operand);
                // The operand's own type: `-2.5` is `FNeg` on a `Float`, and naming it
                // `Int(32)` gives the verifier an instruction that contradicts itself.
                let ty = match self.float_kind(e) {
                    Some(k) => CTy::Float(k),
                    None => CTy::Int(self.raw_width_of(e)),
                };
                let dst = self.new_value();
                // `!x` is `x == 0`, which CIR expresses as a comparison rather than a
                // unary op — it has no logical-not, because the result is an `int` and a
                // dedicated op would need its own width rule.
                let rv = match op {
                    chiero_ast::UnOp::Minus => RValue::Un {
                        // Floating negation flips the sign bit; integer negation is a
                        // two's-complement subtraction. They are different instructions
                        // and CIR names both.
                        op: if matches!(ty, CTy::Float(_)) {
                            CUnOp::FNeg
                        } else {
                            CUnOp::Neg
                        },
                        a,
                        ty,
                    },
                    chiero_ast::UnOp::BitNot => RValue::Un {
                        op: CUnOp::Not,
                        a,
                        ty,
                    },
                    chiero_ast::UnOp::Not => {
                        // `!p` compares a **pointer** against null, like every other C
                        // condition. `compare_ty` is the same answer `if (p)` gets.
                        let cty = self.compare_ty(*operand);
                        let zero = self.zero_at(&cty);
                        self.emit(
                            InstKind::Assign {
                                dst,
                                rv: RValue::Cmp {
                                    op: chiero_cir::CmpOp::Eq,
                                    a,
                                    b: zero,
                                    ty: cty,
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
                e,
                *operand,
                matches!(op, chiero_ast::PostfixOp::Inc),
                false,
                span,
            ),
            chiero_ast::ExprKind::Call { callee, args } => {
                // **The varargs builtins are instructions, not calls** (020 §4.4.1).
                // `stdarg.h` is `#define va_start(v,l) __builtin_va_start(v,l)`, so every
                // variadic function in C reaches here — and lowering reported them as
                // calls to undeclared functions, which 015 §7 turns into refusing the
                // whole function. `VaArg` is an instruction rather than an `RValue`
                // because it *mutates*: it reads the next argument and advances the list.
                if let chiero_ast::ExprKind::Ident(n) = self.ast.expr(*callee).kind
                    && let Some(name) = self.names.text(n)
                    && let Some(kind) = va_builtin(name)
                {
                    return self.va_builtin(kind, args, e, span);
                }
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
                let mut ops: Vec<Operand> = args.iter().map(|&a| self.expr(a)).collect();
                let fid = self.callee_of(*callee);
                let ret_ty = self.width_of(e);
                // **The caller owns the slot an aggregate result is written into**
                // (015 §2). It is allocated here, per call site, so two live results in one
                // expression are two objects — a single reused scratch buffer would make
                // `mk(1,2)` and `mk(3,4)` the same struct.
                //
                // Prepended, matching the callee's hidden first parameter.
                if let Some(rt) = self.type_of(e).filter(|t| self.is_aggregate(*t)) {
                    let align = self.analysis.align_of(rt).unwrap_or(1).max(1);
                    let size = self.analysis.size_of(rt).unwrap_or(0);
                    let slot = self.alloca_n(CTy::Int(8), size, align, None, span);
                    let addr = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: addr,
                            rv: RValue::AddrOfLocal { alloca: slot },
                        },
                        span,
                    );
                    ops.insert(0, Operand::Value(addr));
                }
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
                // **`(T){...}` is a compound literal, not a cast** — the parser spells both
                // with `Cast`, and the type comes from the parenthesis rather than from the
                // braces. Reading it as a conversion evaluated the list as a *scalar*, gave
                // the object an `i32` slot, and then `inttoptr`-ed the first element into
                // the address a `CopyMem` was about to read.
                if matches!(
                    self.ast.expr(*operand).kind,
                    chiero_ast::ExprKind::InitList(_)
                ) && let Some(sty) = self.analysis.ty_of_syntactic(*ty)
                {
                    return self.compound_literal(sty, *operand, span);
                }
                let a = self.expr(*operand);
                // **The operand's real type, not an assumed integer.** `from` was
                // `CTy::Int(width_of(operand))` unconditionally, so `(int *)&g` declared a
                // 32-bit integer source for a pointer and the verifier rejected the whole
                // module — `cast source operand is Ptr, declared Int(32)`. Every
                // pointer-to-pointer cast written in C hit this.
                let from = match self.type_of(*operand).map(|t| self.cty(t)) {
                    Some(t) => t,
                    None => CTy::Int(self.width_of(*operand)),
                };
                let to = self.cty_of_syntactic(*ty);
                if from == to || matches!((&from, &to), (CTy::Ptr, CTy::Ptr)) {
                    return a;
                }
                // **`(_Bool)x` is `x != 0`**, the same rule as the implicit conversion in
                // `typed_node` — C11 6.3.1.2 does not care which way the program spelled
                // it. Written out twice because the two casts arrive by different routes:
                // one from sema's conversion chain, one from the syntax.
                if matches!(to, CTy::Int(1)) && !matches!(from, CTy::Int(1)) {
                    return self.truth_of(a, from, span);
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
                // **An aggregate result is copied out before the block ends.** The value of
                // a statement expression yielding a struct is that struct's *address* —
                // the only thing an aggregate expression can be (020 §1.4) — and the
                // object it names is a local of the block that is about to close. By the
                // time the enclosing initializer copied from it, 021 had retired it and the
                // `CopyMem` read bytes that were gone.
                //
                // So the destination is allocated **here**, before the body's scope is
                // entered, which gives it the enclosing block's lifetime — the same answer
                // wave 138 gave a compound literal, and for the same reason. A fresh slot
                // per statement expression, so two in one expression are two objects.
                //
                // The scalar case needs none of this and gets none: a scalar's value is a
                // value, and no scope exit can invalidate it. That is why the construct has
                // always worked for half the types.
                let dest = match self.type_of(e) {
                    Some(t) if self.is_aggregate(t) => {
                        let align = self.analysis.align_of(t).unwrap_or(1).max(1);
                        let slot = self.alloca_for(t, align, None, span);
                        let addr = self.new_value();
                        self.emit(
                            InstKind::Assign {
                                dst: addr,
                                rv: RValue::AddrOfLocal { alloca: slot },
                            },
                            span,
                        );
                        Some((addr, self.analysis.size_of(t).unwrap_or(0), align))
                    }
                    _ => None,
                };
                // The body's statements are lowered here rather than through `stmt`,
                // because `stmt` would exit the scope before the copy could run.
                let items = match self.ast.stmt(*body).kind.clone() {
                    chiero_ast::StmtKind::Compound(ss) => ss,
                    _ => vec![*body],
                };
                self.enter_scope(span);
                for st in items {
                    self.stmt(st);
                }
                let v = self.last_stmt_value.take();
                let out = match (dest, v.clone()) {
                    (Some((addr, size, align)), Some(src)) => {
                        self.emit(
                            InstKind::CopyMem {
                                dst: Operand::Value(addr),
                                src,
                                size: Operand::Const(Const::Int {
                                    bits: 64,
                                    val: size as i128,
                                }),
                                align,
                            },
                            span,
                        );
                        Some(Operand::Value(addr))
                    }
                    _ => v,
                };
                self.exit_scope(span);
                // Restore, so a statement expression nested inside another does not
                // consume the outer one's value.
                self.last_stmt_value = saved;
                out.unwrap_or(Operand::Const(Const::Undef(CTy::Int(self.raw_width_of(e)))))
            }
            chiero_ast::ExprKind::Comma { lhs, rhs } => {
                self.expr(*lhs);
                self.seq_point(span);
                self.expr(*rhs)
            }
            // A braced list reaching here without a type in front of it is not
            // representable — `local_decl` consumes the ones attached to a declaration, and
            // `(T){...}` is a `Cast` and handled there. Anything else is a gap, and 020 §5
            // says a gap is a diagnostic rather than a licence.
            chiero_ast::ExprKind::InitList(_) => {
                let Some(sty) = self.type_of(e) else {
                    self.diagnostics.push(LowerDiagnostic {
                        span,
                        message: "a braced initializer with no type".into(),
                    });
                    return Operand::Const(Const::Undef(CTy::Int(self.raw_width_of(e))));
                };
                self.compound_literal(sty, e, span)
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
        let a = {
            let t = self.compare_ty(lhs);
            self.truth_of(a, t, span)
        };
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

    /// The CIR type at which `e` is compared.
    ///
    /// **A pointer is compared as a pointer.** `width_of` reports an *integer's* width and
    /// answers 32 for everything else, so every comparison with a pointer operand — `p == 0`
    /// as much as `if (p)` — was typed `Int(32)` and described a value that is not there.
    /// That is wave 132's third defect in the one path that wave did not reach.
    ///
    /// Integers keep `width_of` rather than `cty_of`, deliberately: the operands here are
    /// **post-conversion**, and 014 has already promoted a `char` to `int`. `cty_of` reports
    /// the type as *written*, which would declare `Int(8)` for an operand the typed AST
    /// widened to 32.
    fn compare_ty(&mut self, e: ExprId) -> CTy {
        if self.is_address(e) {
            CTy::Ptr
        } else if let Some(k) = self.float_kind(e) {
            // A float operand's type is its own; naming it `Int(w)` gives the verifier an
            // instruction whose declared type contradicts its operands.
            CTy::Float(k)
        } else {
            CTy::Int(self.width_of(e))
        }
    }

    /// `e`'s floating kind, when it has one.
    fn float_kind(&self, e: ExprId) -> Option<chiero_cir::FloatKind> {
        match self.type_of(e).map(|t| self.cty(t)) {
            Some(CTy::Float(k)) => Some(k),
            _ => None,
        }
    }

    /// The zero `ty` is compared against — a null pointer, or an integer of its width.
    fn zero_at(&self, ty: &CTy) -> Operand {
        match ty {
            CTy::Ptr => Operand::Const(Const::Null),
            CTy::Int(w) => Operand::Const(Const::Int { bits: *w, val: 0 }),
            // **A float compares against 0.0, not against `undef`.** C11 6.3.1.2 makes a
            // conversion to `_Bool` a comparison with zero whatever the source type is, and
            // 6.5.15 does the same for a condition — so `if (d)` and `(_Bool)d` both land
            // here. `Undef` made the comparison meaningless and the engine then panicked
            // with "extract out of range" trying to use the result: a *source-triggerable
            // panic*, which is the worst outcome there is because it takes the run and
            // every other finding in it.
            CTy::Float(k) => Operand::Const(Const::Float(*k, 0)),
            other => Operand::Const(Const::Undef(other.clone())),
        }
    }

    /// A branch condition as `Int(1)`.
    ///
    /// 015 §2.1's snippet says `br a_nonzero`, and CIR's verifier agrees: `Br` takes a
    /// one-bit operand. C conditions are "compares unequal to 0", so the conversion is a
    /// comparison rather than a truncation — truncating `2` to one bit gives 0, which
    /// inverts the branch for every even nonzero value.
    ///
    /// It takes the condition's **type**, not a width: the rule above is right for a
    /// pointer too, and passing a width forced every caller to answer "how wide is a
    /// pointer" with `width_of`, which says 32. `if (p)` produced `cmp ne i32` on an
    /// address and no path survived it.
    fn truth_of(&mut self, v: Operand, ty: CTy, span: Span) -> Operand {
        let zero = self.zero_at(&ty);
        let dst = self.new_value();
        // **A float's truth is a comparison, not a bit test.** Integer `Ne` on two float
        // patterns is right for almost every value and wrong for `-0.0`, whose bits differ
        // from `+0.0` while C says it is false. `FUNe` is also what makes `(_Bool)NaN` true,
        // which the ordered form would not.
        let op = if matches!(ty, CTy::Float(_)) {
            chiero_cir::CmpOp::FUNe
        } else {
            chiero_cir::CmpOp::Ne
        };
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Cmp {
                    op,
                    a: v,
                    b: zero,
                    ty,
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
        // **The conditional's own type, not an assumed integer.** `pick ? twice : thrice`
        // is a function pointer, and a slot declared `Int(32)` made storing the (correct)
        // `Ptr` into it a verifier error — which 015 §7 turns into refusing the whole
        // enclosing function, so `indirect_call.c` lowered to nothing.
        //
        // Wave 119 blamed sema for this. Sema is right: `function_pointers.rs` types
        // `int (*fn)(int)` as a pointer at file scope *and* as a local. The wrong answer
        // was made here, one `?:` away from where anyone looked.
        let slot_ty = match self.type_of(e).map(|t| self.cty(t)) {
            Some(t) if t != CTy::Void => t,
            _ => CTy::Int(self.raw_width_of(e).max(1)),
        };
        let align = self
            .type_of(e)
            .and_then(|t| self.analysis.align_of(t))
            .unwrap_or(4)
            .max(1);
        let slot = self.alloca(slot_ty.clone(), align, None, span);

        let c = self.expr(cond);
        // **The elvis form evaluates `a` once.** Storing it into the slot here and
        // branching on the stored value is what makes that true; re-evaluating `cond` in
        // the true arm would run its side effects twice, and no shape test can see the
        // difference when it has none.
        if then.is_none() {
            self.generated(|s| s.store_slot(slot, c.clone(), &slot_ty, span));
        }
        let test = {
            let t = self.compare_ty(cond);
            self.truth_of(c, t, span)
        };
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
        if let Some((unit, bits)) = self.bitfield_of(lhs) {
            let addr = match self.lvalue_addr(lhs, span) {
                Some(a) => a,
                None => return Operand::Const(Const::Undef(unit)),
            };
            let signed = self.is_signed(lhs);
            // **A compound assignment reaches here too**, and used not to: the guard was
            // `op.is_none()`, so `v.a += 1` fell through to the ordinary path and did a
            // whole-`int` read-modify-write over every neighbour in the storage unit.
            //
            // The arithmetic happens at the **unit's** width, not the field's, because C
            // promotes a bit-field to `int` before operating on it; `StoreBits` then
            // truncates the result back into `bits`. That is what makes `3 + 1` in a 3-bit
            // signed field come out −4 rather than 4.
            let val = match op {
                None => self.expr(rhs),
                Some(binop) => {
                    let old = self.load_bits(addr.clone(), unit.clone(), bits, signed, span);
                    let r = self.expr(rhs);
                    let dst = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst,
                            rv: RValue::Bin {
                                op: cir_binop(binop, signed, self.is_float(lhs)),
                                a: old,
                                b: r,
                                ty: unit.clone(),
                            },
                        },
                        span,
                    );
                    Operand::Value(dst)
                }
            };
            self.emit(
                InstKind::StoreBits {
                    addr: addr.clone(),
                    val: val.clone(),
                    unit: unit.clone(),
                    bits,
                    align: 1,
                },
                span,
            );
            // **The result is the lvalue's value *after* the assignment** (C11 6.5.16.2p3),
            // which for a bit-field is the truncated one: `int r = (v.a += 1)` with `a` a
            // 3-bit field holding 3 is −4, not the 4 the addition produced. A plain `=`
            // needs no reload — its right-hand side was already converted to the field's
            // type by sema — so only the compound form pays for it.
            if op.is_some() {
                return self.load_bits(addr, unit, bits, signed, span);
            }
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
            // **The destination must be an lvalue** — C says so, and if lowering cannot
            // name it, 020 §5's rule applies: a gap is a diagnostic, not a licence. Silent
            // `Undef` here dropped the whole assignment and read as a program that never
            // wrote anything.
            let Some(dst) = self.lvalue_addr(lhs, span) else {
                self.diagnostics.push(LowerDiagnostic {
                    span,
                    message: "an aggregate assignment to something with no address".into(),
                });
                return Operand::Const(Const::Undef(CTy::Ptr));
            };
            // **The source need not be one.** `y = mk(1, 2)`, `y = (0, x)`, `y = c ? x : z`
            // and `w = y = x` all have a right-hand side that is not an lvalue, and each
            // silently emitted *no* `CopyMem` at all — the assignment vanished from the CIR
            // along with the side effects of whatever was on the right.
            //
            // `expr` is the right fallback rather than a second special case: wave 132 made
            // it yield the *address* for an aggregate, which is exactly what `CopyMem`
            // wants, and it is what `local_decl` has always used — which is why the
            // initializer form `struct S y = (0, x);` worked while the assignment did not.
            let src = match self.lvalue_addr(rhs, span) {
                Some(a) => a,
                None => self.expr(rhs),
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
                let loaded = self.new_value();
                self.emit(
                    InstKind::Assign {
                        dst: loaded,
                        rv: RValue::Load {
                            addr: addr.clone(),
                            ty: ty.clone(),
                            align: 1,
                            vol: Volatility::Normal,
                        },
                    },
                    span,
                );
                // A `_Bool` operand is promoted to `int` before the operation, so the
                // loaded bit is widened to match the width the arithmetic happens at.
                let old = if ty == CTy::Int(1) {
                    let w = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: w,
                            rv: RValue::Cast {
                                kind: chiero_cir::CastKind::ZExt,
                                a: Operand::Value(loaded),
                                from: CTy::Int(1),
                                to: CTy::Int(32),
                            },
                        },
                        span,
                    );
                    w
                } else {
                    loaded
                };
                let r = self.expr(rhs);
                // The right operand is promoted too. sema coerces a compound assignment's
                // right-hand side to the *lvalue's* type, which for `_Bool` hands back a
                // one-bit value — so without this the addition has a 32-bit and a one-bit
                // operand and the engine ends the path on a width mismatch. Wave 133 fixed
                // the same over-eager coercion in sema for pointers; here the operand is
                // widened at the use instead, because unlike a pointer count a `_Bool` right
                // operand really is being converted, just to `int` rather than to `_Bool`.
                let r = if ty == CTy::Int(1) {
                    let w = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: w,
                            rv: RValue::Cast {
                                kind: chiero_cir::CastKind::ZExt,
                                a: r,
                                from: CTy::Int(1),
                                to: CTy::Int(32),
                            },
                        },
                        span,
                    );
                    Operand::Value(w)
                } else {
                    r
                };
                // **`p += n` displaces by elements too.** The `Bin` below would add the
                // count to the address unscaled and at the lvalue's width, which is the
                // same defect `p + n` had — three spellings, one operation.
                if matches!(binop, chiero_ast::BinOp::Add | chiero_ast::BinOp::Sub)
                    && self.is_address(lhs)
                {
                    let idx = self.widen_to_64(r, rhs, span);
                    let elem = self.elem_size_of(lhs).unwrap_or(1).max(1);
                    self.displace(
                        Operand::Value(old),
                        idx,
                        elem,
                        matches!(binop, chiero_ast::BinOp::Sub),
                        span,
                    )
                } else {
                    // **`_Bool` promotes, operates, and converts back** (C11 6.5.16.2 with
                    // 6.3.1.2), and the conversion is `!= 0` rather than a narrowing. Doing
                    // the arithmetic at the lvalue's own one-bit width wraps `1 + 1` to 0,
                    // so `b += 1` on a true `_Bool` turned it false. Wave 136's bit-field
                    // rule in its other instance — and the one scalar where "convert back"
                    // is a comparison.
                    let width = if ty == CTy::Int(1) {
                        CTy::Int(32)
                    } else {
                        ty.clone()
                    };
                    let dst = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst,
                            rv: RValue::Bin {
                                op: cir_binop(binop, self.is_signed(lhs), self.is_float(lhs)),
                                a: Operand::Value(old),
                                b: r,
                                ty: width.clone(),
                            },
                        },
                        span,
                    );
                    if ty == CTy::Int(1) {
                        self.truth_of(Operand::Value(dst), width, span)
                    } else {
                        Operand::Value(dst)
                    }
                }
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
    /// Read a bit-field, at its unit's width and with its own signedness.
    ///
    /// The one place `LoadBits` is emitted outside the plain read path — shared by
    /// compound assignment and `++`/`--`, which are the same read-modify-write and were
    /// two separate ways of missing it.
    fn load_bits(
        &mut self,
        addr: Operand,
        unit: CTy,
        bits: chiero_cir::BitRange,
        signed: bool,
        span: Span,
    ) -> Operand {
        let dst = self.new_value();
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
        Operand::Value(dst)
    }

    fn inc_dec(
        &mut self,
        e: ExprId,
        operand: ExprId,
        up: bool,
        prefix: bool,
        span: Span,
    ) -> Operand {
        // **`v.a++` is a bit-field read-modify-write**, and this function had no bit-field
        // check at all — it loaded and stored the declared `int`, so incrementing a 3-bit
        // field overwrote its neighbours and never wrapped. 015 contract 7 owns the range,
        // and `assign` above already obeys it.
        if let Some((unit, bits)) = self.bitfield_of(operand) {
            let Some(addr) = self.lvalue_addr(operand, span) else {
                return Operand::Const(Const::Undef(unit));
            };
            let signed = self.is_signed(operand);
            let width = match &unit {
                CTy::Int(w) => *w,
                _ => 32,
            };
            let old = self.load_bits(addr.clone(), unit.clone(), bits, signed, span);
            let new = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: new,
                    rv: RValue::Bin {
                        op: if up { CBinOp::Add } else { CBinOp::Sub },
                        a: old.clone(),
                        b: Operand::Const(Const::Int {
                            bits: width,
                            val: 1,
                        }),
                        ty: unit.clone(),
                    },
                },
                span,
            );
            self.emit(
                InstKind::StoreBits {
                    addr: addr.clone(),
                    val: Operand::Value(new),
                    unit: unit.clone(),
                    bits,
                    align: 1,
                },
                span,
            );
            // Postfix yields the value read *before* the store, which `LoadBits` already
            // truncated to the field. Prefix yields the value after it, which the addition
            // has not truncated — so it is read back rather than reused.
            return if prefix {
                self.load_bits(addr, unit, bits, signed, span)
            } else {
                old
            };
        }
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
        // **`p++` advances by one *element*.** The `Bin` below adds a literal 1 to the
        // address — a byte, not an element, and at `_ => 32` for a pointer lvalue. Same
        // operation as `p + 1` and `p += 1`, so the same `displace`.
        let new = if self.is_address(operand) {
            let elem = self.elem_size_of(operand).unwrap_or(1).max(1);
            let d = self.displace(
                Operand::Value(old),
                Operand::Const(Const::Int { bits: 64, val: 1 }),
                elem,
                !up,
                span,
            );
            match d {
                Operand::Value(v) => v,
                // `displace` always yields a value; this arm keeps the types honest rather
                // than asserting.
                other => {
                    let v = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: v,
                            rv: RValue::PtrAdd {
                                base: other,
                                off: Operand::Const(Const::Int { bits: 64, val: 0 }),
                            },
                        },
                        span,
                    );
                    v
                }
            }
        } else if ty == CTy::Int(1) {
            // **`b++` on a `_Bool` converts rather than truncating.** C11 6.5.2.4 promotes,
            // adds, and converts back, and conversion to `_Bool` is `!= 0` (6.3.1.2) — so
            // incrementing a true `_Bool` leaves it true. At the lvalue's own width the
            // addition wraps `1 + 1` to 0 and turned it false.
            let wide = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: wide,
                    rv: RValue::Cast {
                        kind: chiero_cir::CastKind::ZExt,
                        a: Operand::Value(old),
                        from: CTy::Int(1),
                        to: CTy::Int(32),
                    },
                },
                span,
            );
            let sum = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: sum,
                    rv: RValue::Bin {
                        op: if up { CBinOp::Add } else { CBinOp::Sub },
                        a: Operand::Value(wide),
                        b: Operand::Const(Const::Int { bits: 32, val: 1 }),
                        ty: CTy::Int(32),
                    },
                },
                span,
            );
            match self.truth_of(Operand::Value(sum), CTy::Int(32), span) {
                Operand::Value(v) => v,
                other => {
                    let v = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: v,
                            rv: RValue::Use(other),
                        },
                        span,
                    );
                    v
                }
            }
        } else {
            let new = self.new_value();
            // **`++` on a float is `+ 1.0`, not `+ 1`.** C11 6.5.2.4p1 says the operand is
            // incremented by 1 "of the appropriate type", and for a float that is a float
            // one — an integer literal here makes `Add` disagree with its own declared
            // type, which the verifier catches and which the generator found on its first
            // run over float programs.
            let (op, one) = match &ty {
                CTy::Float(k) => (
                    if up { CBinOp::FAdd } else { CBinOp::FSub },
                    Operand::Const(Const::Float(*k, float_bits(*k, 1.0))),
                ),
                _ => (
                    if up { CBinOp::Add } else { CBinOp::Sub },
                    Operand::Const(Const::Int {
                        bits: width,
                        val: 1,
                    }),
                ),
            };
            self.emit(
                InstKind::Assign {
                    dst: new,
                    rv: RValue::Bin {
                        op,
                        a: Operand::Value(old),
                        b: one,
                        ty: ty.clone(),
                    },
                },
                span,
            );
            new
        };
        self.emit(
            InstKind::Store {
                addr,
                val: Operand::Value(new),
                ty: ty.clone(),
                align: 1,
                vol: Volatility::Normal,
            },
            span,
        );
        let out = if prefix { new } else { old };
        // **`++b` on a `_Bool` yields the promoted value.** sema types the expression `int`
        // and inserts no conversion — unlike a plain read of `b`, where it does — so the
        // one-bit result stored in the object would be stored again as an `i32` by whatever
        // consumes it. The object keeps its bit; the expression hands back its `int`.
        let want = self.raw_width_of(e);
        if ty == CTy::Int(1) && want > 1 {
            let w = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: w,
                    rv: RValue::Cast {
                        kind: chiero_cir::CastKind::ZExt,
                        a: Operand::Value(out),
                        from: CTy::Int(1),
                        to: CTy::Int(want),
                    },
                },
                span,
            );
            return Operand::Value(w);
        }
        Operand::Value(out)
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
    ///
    /// A local carries its declared `CTy` in the frame, so that answer comes first. **Every
    /// other lvalue** — a global, a member, an array element, a dereference — has to ask
    /// sema, because the old fallback reported `Int(32)` for all of them and silently
    /// truncated every `Store` of a pointer to four bytes.
    fn lvalue_ty(&mut self, e: ExprId) -> CTy {
        if let chiero_ast::ExprKind::Ident(sym) = self.ast.expr(e).kind
            && let Some((_, ty)) = self.fs().locals.get(&sym)
        {
            return ty.clone();
        }
        self.cty_of(e)
    }

    /// The address of an lvalue, computed **once**.
    /// Record the reporting-only `AccessPath` for an address value (020 §4.4).
    ///
    /// Called only where lowering already knows a member's name *and* its offset, which is
    /// the member-access site — the two facts nothing downstream can recover, and the whole
    /// reason paths are built here rather than reconstructed from `PtrAdd` offsets.
    fn record_path(&mut self, v: ValueId, e: ExprId) {
        let Some(p) = self.path_of(e) else { return };
        self.fs().access_paths.insert(v, p);
    }

    /// The path an lvalue expression denotes, or `None` when lowering cannot name it.
    ///
    /// **`None` rather than a guess.** A path is a reporting aid; one that names the wrong
    /// member is worse than none, because a reader acts on it.
    fn path_of(&mut self, e: ExprId) -> Option<AccessPath> {
        match self.ast.expr(e).kind.clone() {
            chiero_ast::ExprKind::Ident(sym) => {
                let (slot, _) = self.fs().locals.get(&sym).cloned()?;
                Some(AccessPath {
                    root: PathRoot::Local {
                        alloca: slot,
                        name: self.sym(sym),
                    },
                    steps: Default::default(),
                })
            }
            chiero_ast::ExprKind::Member { base, field, arrow } => {
                let (byte_off, _) = self.field_of(base, field, arrow)?;
                let mut p = self.path_of(base)?;
                // `->` is a dereference and `.` is not; 020 §4.4 has a step for it and a
                // path that dropped it would render `p.next` for `p->next`.
                if arrow {
                    p.steps.push(PathStep::Deref);
                }
                // **A union member is a `UnionMember` step, not a `Field`** (020 §4.5:
                // "lowering emits `PtrAdd` plus a `UnionMember` path step for reporting").
                // The difference is the whole point of the step: `as ip4_rewrite_t.x` says
                // the bytes were *viewed through* something that may not be what wrote
                // them, and `.x` says they were not.
                let name = self.sym(field)?;
                let rec = self.record_of(base, arrow);
                match rec {
                    Some((r, true)) => p.steps.push(PathStep::UnionMember {
                        name,
                        off: byte_off,
                        view: r,
                    }),
                    _ => p.steps.push(PathStep::Field {
                        name,
                        off: byte_off,
                    }),
                }
                Some(p)
            }
            chiero_ast::ExprKind::Index { base, index } => {
                let mut p = self.path_of(base)?;
                // The index as written when it is a constant; a symbolic one renders as
                // its value id, which is still better than nothing.
                let idx = match self.const_of(index) {
                    Some(v) => Operand::Const(Const::Int { bits: 64, val: v }),
                    None => Operand::Const(Const::Undef(CTy::Int(64))),
                };
                p.steps.push(PathStep::Index(idx));
                Some(p)
            }
            _ => None,
        }
    }

    /// An expression's CIR type, **from sema**.
    ///
    /// `raw_width_of` reports an *integer's* width and answers 32 for everything else, which
    /// is right for what it is for and wrong as an answer to "what type is this lvalue".
    /// Three sites asked it that question, and every pointer that was not a plain local —
    /// one in a struct member, in an array element, or reached through a second pointer —
    /// was loaded and stored as an `i32` and kept half of itself.
    fn cty_of(&self, e: ExprId) -> CTy {
        match self.type_of(e) {
            Some(t) => self.cty(t),
            None => CTy::Int(self.raw_width_of(e)),
        }
    }

    /// Whether a sema type is one CIR has no value form for (020 §1.4), so it lives in
    /// memory and moves by `CopyMem` — a struct, union, array **or vector**.
    ///
    /// `Ty::Vector` belongs here and was missing, while `cty`, `aggregate_size` and
    /// `aggregate_size_of_ty` all included it: three predicates in one file disagreeing
    /// about one type, so a vector read as a value loaded its first eight bytes as a
    /// pointer and a copy moved eight of its sixteen bytes.
    ///
    /// **`Ty::Func` is deliberately not here.** A function designator also has no value
    /// form and also decays to its address (C11 6.3.2.1p4), but it is not an *object*:
    /// it has no size, so a `CopyMem` of it is meaningless and the callers that ask
    /// "should this move by copy" must answer no. `is_address_only` is the predicate for
    /// "names its address", and that one includes it.
    /// Whether `e`'s type is a floating one, which decides the opcode set.
    fn is_float(&self, e: ExprId) -> bool {
        self.type_of(e)
            .is_some_and(|t| matches!(self.analysis.ty(t), Ty::Float(_)))
    }

    fn is_aggregate(&self, t: TyId) -> bool {
        matches!(
            self.analysis.ty(t),
            Ty::Record(_) | Ty::Array { .. } | Ty::Vector { .. }
        )
    }

    /// Whether reading `e` as a value can only yield its **address**.
    ///
    /// Every aggregate, plus a function designator: `(*fp)(3)` emitted `load ptr` at the
    /// function's own address, because the read sites asked whether the type was an
    /// *aggregate* and a function is not one. The two questions are close enough to have
    /// looked like one and different enough that conflating them was wrong both ways.
    fn is_address_only(&self, e: ExprId) -> bool {
        self.type_of(e)
            .is_some_and(|t| self.is_aggregate(t) || matches!(self.analysis.ty(t), Ty::Func { .. }))
    }

    fn lvalue_addr(&mut self, e: ExprId, span: Span) -> Option<Operand> {
        // **A compound literal is an lvalue** (C99 6.5.2.5p4), so `&(struct S){5, 6}` and
        // `p->a = 9` through one are both legal. Its address is its object's.
        if let chiero_ast::ExprKind::Cast { ty, operand } = self.ast.expr(e).kind
            && matches!(
                self.ast.expr(operand).kind,
                chiero_ast::ExprKind::InitList(_)
            )
            && let Some(sty) = self.analysis.ty_of_syntactic(ty)
        {
            return Some(self.compound_literal_addr(sty, operand, span));
        }
        match self.ast.expr(e).kind.clone() {
            chiero_ast::ExprKind::Ident(sym) => {
                // **Locals first, then file-scope.** Looking only at locals is what made
                // `g[1]` index off the array's first *element* instead of its address:
                // the name resolved to nothing, the caller fell through to the value path,
                // and `PtrAdd` got an `Int(32)` base that the verifier rightly rejected.
                let Some((slot, _)) = self.fs().locals.get(&sym).cloned() else {
                    let g = *self.globals.get(&sym)?;
                    let addr = self.new_value();
                    self.emit(
                        InstKind::Assign {
                            dst: addr,
                            rv: RValue::AddrOfGlobal { g },
                        },
                        span,
                    );
                    return Some(Operand::Value(addr));
                };
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
                    // **`.`'s left operand may be a value, not only an lvalue** (C11
                    // 6.5.2.3p3): `make(7).a` selects a field of a call's result, and
                    // `(struct S){1, 2}.a` of a literal's. Requiring `lvalue_addr` to
                    // succeed made both produce no state and no diagnostic at all.
                    //
                    // Since wave 132 an aggregate *expression* already evaluates to its
                    // address — that is what "CIR has no aggregate values" (020 §1.4)
                    // means — so the base has been available all along; this arm only ever
                    // asked for it the one way.
                    match self.lvalue_addr(base, span) {
                        Some(a) => a,
                        None if self.type_of(base).is_some_and(|t| self.is_aggregate(t)) => {
                            self.expr(base)
                        }
                        None => return None,
                    }
                };
                let (byte_off, _) = self.field_of(base, field, arrow)?;
                if byte_off == 0 {
                    // **The offset-0 member still gets a path**, and it is the case a
                    // reader hits most: the first member of every struct. Lowering returns
                    // the base address unchanged here, so the path is attached to *that*
                    // value — which is why `record_path` keys on the value rather than on
                    // the instruction it would otherwise have emitted.
                    if let Operand::Value(v) = base_addr {
                        self.record_path(v, e);
                    }
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
                self.record_path(dst, e);
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
    /// The record a member access goes through, and whether it is a union.
    ///
    /// The name is the *view* a `UnionMember` step reports — an anonymous union has none,
    /// so it renders as `union`, which is still what a reader needs to know.
    fn record_of(&mut self, base: ExprId, arrow: bool) -> Option<(chiero_cir::Symbol, bool)> {
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
        let name = self
            .analysis
            .tag_of(rec)
            .and_then(|n| self.sym(n))
            .unwrap_or_else(|| chiero_cir::Symbol::from("union"));
        Some((name, l.is_union))
    }

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
    /// Whether `e` denotes an address — a pointer, or an array that decays to one.
    ///
    /// `type_of` walks past the `ArrayDecay` cast to the value the source wrote, so `a` in
    /// `a + 1` reports its `Ty::Array` rather than the `Ty::Ptr` it converts to. Both are
    /// addresses here, and `Ty::Func` is one too — a function designator decays exactly as
    /// an array does (C11 6.3.2.1p4).
    fn is_address(&self, e: ExprId) -> bool {
        self.type_of(e).is_some_and(|t| {
            matches!(
                self.analysis.ty(t),
                Ty::Ptr(_) | Ty::Array { .. } | Ty::Func { .. }
            )
        })
    }

    /// `p + n`, `n + p`, `p - n` and `p - q`, or `None` when neither operand is an address.
    ///
    /// C11 6.5.6: additive arithmetic on a pointer counts in **elements**, and the
    /// difference of two pointers is a count of elements too (6.5.6p9) — not of bytes.
    /// Nothing else in this function is a C conversion, so the widening and scaling here
    /// are lowering's own bookkeeping, spelled once rather than at each call site.
    fn ptr_arith(
        &mut self,
        op: chiero_ast::BinOp,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> Option<Operand> {
        if !matches!(op, chiero_ast::BinOp::Add | chiero_ast::BinOp::Sub) {
            return None;
        }
        let (l_ptr, r_ptr) = (self.is_address(lhs), self.is_address(rhs));
        if !l_ptr && !r_ptr {
            return None;
        }

        // **`p - q` is a count, not a distance.** Subtract the two addresses as integers
        // and divide by the element size — signed, because `q - p` with `q` below `p` is a
        // negative count and C says so.
        if l_ptr && r_ptr {
            if !matches!(op, chiero_ast::BinOp::Sub) {
                // `p + q` is not C and sema has already rejected it. Refusing the enclosing
                // function (015 §7 contract 20) beats emitting an `add` of two addresses,
                // which would run and be wrong.
                self.diagnostics.push(LowerDiagnostic {
                    span,
                    message: "arithmetic on two pointers".into(),
                });
                return Some(Operand::Const(Const::Undef(CTy::Int(64))));
            }
            let a = self.expr(lhs);
            let b = self.expr(rhs);
            let ai = self.ptr_to_int(a, span);
            let bi = self.ptr_to_int(b, span);
            let diff = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: diff,
                    rv: RValue::Bin {
                        op: CBinOp::Sub,
                        a: ai,
                        b: bi,
                        ty: CTy::Int(64),
                    },
                },
                span,
            );
            let elem = self.elem_size_of(lhs).unwrap_or(1).max(1);
            let out = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: out,
                    rv: RValue::Bin {
                        op: CBinOp::SDiv,
                        a: Operand::Value(diff),
                        b: Operand::Const(Const::Int {
                            bits: 64,
                            val: elem as i128,
                        }),
                        ty: CTy::Int(64),
                    },
                },
                span,
            );
            return Some(Operand::Value(out));
        }

        // `n + p` is `p + n` (6.5.6p2 makes `+` commutative here); `n - p` is not C.
        if !l_ptr && matches!(op, chiero_ast::BinOp::Sub) {
            self.diagnostics.push(LowerDiagnostic {
                span,
                message: "an integer minus a pointer".into(),
            });
            return Some(Operand::Const(Const::Undef(CTy::Ptr)));
        }
        // **Left to right still** (015 §2, normative): the *written* order decides which
        // side's side effects are emitted first, not which side happens to be the pointer.
        let first = self.expr(lhs);
        let second = self.expr(rhs);
        let (ptr_e, base, int_e, idx) = if l_ptr {
            (lhs, first, rhs, second)
        } else {
            (rhs, second, lhs, first)
        };

        // The index widens to pointer width **before** scaling, and sign-extends when it is
        // signed: `p + (-1)` is a legal C expression, and a zero-extended −1 addresses four
        // billion elements away.
        let idx = self.widen_to_64(idx, int_e, span);
        let elem = self.elem_size_of(ptr_e).unwrap_or(1).max(1);
        Some(self.displace(base, idx, elem, matches!(op, chiero_ast::BinOp::Sub), span))
    }

    /// `base ± idx * elem`, as one `PtrAdd`.
    ///
    /// `idx` must already be 64 bits wide. Shared by `p + n`, `p += n` and `p++`, which are
    /// three spellings of one operation and were three separate wrong answers.
    fn displace(
        &mut self,
        base: Operand,
        idx: Operand,
        elem: u64,
        subtract: bool,
        span: Span,
    ) -> Operand {
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
        // `p - n` is `p + (-n)`: `PtrAdd` is the only pointer-displacing instruction CIR
        // has (020), so the sign lives in the offset rather than in the opcode.
        let off = if subtract {
            let neg = self.new_value();
            self.emit(
                InstKind::Assign {
                    dst: neg,
                    rv: RValue::Bin {
                        op: CBinOp::Sub,
                        a: Operand::Const(Const::Int { bits: 64, val: 0 }),
                        b: Operand::Value(scaled),
                        ty: CTy::Int(64),
                    },
                },
                span,
            );
            Operand::Value(neg)
        } else {
            Operand::Value(scaled)
        };
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::PtrAdd { base, off },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// Widen an integer operand to 64 bits, sign-extending when `e`'s type is signed.
    fn widen_to_64(&mut self, v: Operand, e: ExprId, span: Span) -> Operand {
        let w = self.width_of(e);
        if w >= 64 {
            return v;
        }
        let dst = self.new_value();
        let signed = self.is_signed(e);
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Cast {
                    kind: if signed {
                        chiero_cir::CastKind::SExt
                    } else {
                        chiero_cir::CastKind::ZExt
                    },
                    a: v,
                    from: CTy::Int(w),
                    to: CTy::Int(64),
                },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// An address as a 64-bit integer, for the one operation that needs it.
    fn ptr_to_int(&mut self, a: Operand, span: Span) -> Operand {
        let v = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: v,
                rv: RValue::Cast {
                    kind: chiero_cir::CastKind::PtrToInt,
                    a,
                    from: CTy::Ptr,
                    to: CTy::Int(64),
                },
            },
            span,
        );
        Operand::Value(v)
    }

    fn elem_size_of(&mut self, base: ExprId) -> Option<u64> {
        let t = self.type_of(base)?;
        // **A vector indexes like an array.** Omitting it here made `elem` fall to the
        // `_ => None` arm, callers substituted 1, and `a[1]` on a `v4si` wrote byte 1 —
        // scaled by a byte rather than by four.
        let elem = match self.analysis.ty(t).clone() {
            Ty::Array { elem, .. } | Ty::Ptr(elem) | Ty::Vector { elem, .. } => elem,
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

    /// Lower one of 020 §4.4.1's varargs builtins.
    fn va_builtin(&mut self, kind: VaBuiltin, args: &[ExprId], e: ExprId, span: Span) -> Operand {
        // The `va_list` object's *address*: §4.4.1 keeps the list in memory so a
        // `va_list *` can cross a function boundary, which VPP's `format` paths need.
        let list = match args.first() {
            Some(a) => self.lvalue_addr(*a, span).unwrap_or_else(|| self.expr(*a)),
            None => return Operand::Const(Const::Undef(CTy::Ptr)),
        };
        match kind {
            VaBuiltin::Start => {
                self.emit(InstKind::VaStart { list }, span);
                Operand::Const(Const::Int { bits: 32, val: 0 })
            }
            VaBuiltin::End => {
                self.emit(InstKind::VaEnd { list }, span);
                Operand::Const(Const::Int { bits: 32, val: 0 })
            }
            VaBuiltin::Copy => {
                let src = match args.get(1) {
                    Some(a) => self.lvalue_addr(*a, span).unwrap_or_else(|| self.expr(*a)),
                    None => return Operand::Const(Const::Undef(CTy::Ptr)),
                };
                self.emit(InstKind::VaCopy { dst: list, src }, span);
                Operand::Const(Const::Int { bits: 32, val: 0 })
            }
            VaBuiltin::Arg => {
                // The type comes from the *expression's* type, which sema recorded from
                // `va_arg(ap, T)`'s second operand — a `TypeName` argument, not a value.
                let ty = self
                    .type_of(e)
                    .map(|t| self.cty(t))
                    .unwrap_or(CTy::Int(self.raw_width_of(e).max(1)));
                let dst = self.new_value();
                self.emit(InstKind::VaArg { dst, list, ty }, span);
                Operand::Value(dst)
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
                // **A name can be declared without being a function.** `int (*fn)(int)`
                // is a local — or a global, or a parameter — holding a function pointer,
                // and calling through it is how VPP dispatches every graph node. Looking
                // only in `module.funcs` reported it as *undeclared*, which 015 §7 turns
                // into refusing the whole enclosing function: `indirect_call.c` lowered to
                // nothing at all.
                //
                // So the value is evaluated and the call goes indirect through it, which
                // is what `Callee::Indirect` is for.
                if let chiero_ast::ExprKind::Ident(sym) = self.ast.expr(callee).kind
                    && (self.fs().locals.contains_key(&sym) || self.globals.contains_key(&sym))
                {
                    let op = self.expr(callee);
                    return Callee::Indirect(op);
                }
                // Nothing declared this name at all. Rather than inventing a signature the
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
/// A floating value's bits at its own width.
///
/// `f64` is the parser's output and the wider of the two, so narrowing to `f32` happens
/// here and only here. The 80-bit form has no Rust primitive: its bits are the `f64`
/// pattern, which is wrong in the last places, and every operation on it is a declared gap
/// — recorded so the narrowing is not mistaken for support.
/// A float kind's width in bits.
/// sema's `FloatKind` as CIR's. The two enums are the same three cases in two crates.
fn cir_float_kind(k: chiero_sema::FloatKind) -> chiero_cir::FloatKind {
    match k {
        chiero_sema::FloatKind::F32 => chiero_cir::FloatKind::F32,
        chiero_sema::FloatKind::F64 => chiero_cir::FloatKind::F64,
        _ => chiero_cir::FloatKind::X87_80,
    }
}

fn float_width(k: chiero_cir::FloatKind) -> u32 {
    match k {
        chiero_cir::FloatKind::F32 => 32,
        chiero_cir::FloatKind::F64 => 64,
        chiero_cir::FloatKind::X87_80 => 80,
    }
}

fn float_bits(k: chiero_cir::FloatKind, v: f64) -> u64 {
    match k {
        chiero_cir::FloatKind::F32 => u64::from((v as f32).to_bits()),
        chiero_cir::FloatKind::F64 | chiero_cir::FloatKind::X87_80 => v.to_bits(),
    }
}

fn cir_binop(op: chiero_ast::BinOp, signed: bool, float: bool) -> CBinOp {
    use chiero_ast::BinOp as A;
    if float {
        // **Floating arithmetic is its own opcode set**, not the integer one applied to the
        // bits. `Add` on two IEEE-754 patterns is a number, and a wrong one — which a
        // lowering golden would have to assert in order to notice, and which gcc catches
        // in one line.
        return match op {
            A::Add => CBinOp::FAdd,
            A::Sub => CBinOp::FSub,
            A::Mul => CBinOp::FMul,
            A::Div => CBinOp::FDiv,
            A::Rem => CBinOp::FRem,
            // Bitwise and shift operators do not apply to floats in C, so anything else
            // here is a program the front end should already have rejected; falling
            // through to the integer table keeps this total without inventing a meaning.
            _ => cir_binop(op, signed, false),
        };
    }
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

/// The comparison for a **floating** operand pair, and whether the operands must be
/// swapped to express it.
///
/// **CIR has no `FOGt` or `FOGe`.** The ordered set is `FOEq`/`FONe`/`FOLt`/`FOLe`, so
/// `a > b` is `FOLt(b, a)` — the swap is how the operator is expressed, not an
/// optimisation, and getting it backwards makes every `>` answer `<`.
///
/// `!=` is **unordered** (`FUNe`). C's `isnan` idiom is `x != x`, and `FONe` is false for
/// NaN, which is the opposite of what the idiom means; CIR's own comment on the variant
/// says so. Every other operator here is the ordered form, because C's relational operators
/// are false when either operand is NaN.
fn cir_fcmpop(op: chiero_ast::BinOp) -> Option<(chiero_cir::CmpOp, bool)> {
    use chiero_ast::BinOp as A;
    use chiero_cir::CmpOp as C;
    Some(match op {
        A::Eq => (C::FOEq, false),
        A::Ne => (C::FUNe, false),
        A::Lt => (C::FOLt, false),
        A::Le => (C::FOLe, false),
        A::Gt => (C::FOLt, true),
        A::Ge => (C::FOLe, true),
        _ => return None,
    })
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
        // Ranges too wide to enumerate, as `(lo, hi, target)`, tested ahead of the switch.
        let mut wide_ranges: Vec<(i128, i128, BlockId)> = Vec::new();
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
                    //
                    // **But only up to a bound.** VPP writes `case 1 ... 10000` for
                    // protocol-number ranges, and enumerating that is 10 000 `Switch`
                    // entries the engine walks on every branch decision. Past the bound the
                    // range becomes a *guard* the head tests before the switch, which is a
                    // shape the engine already costs at O(1).
                    if hi_v.saturating_sub(lo_v) > MAX_ENUMERATED_CASE_RANGE {
                        wide_ranges.push((lo_v, hi_v, b));
                    } else {
                        for v in lo_v..=hi_v.max(lo_v) {
                            cases.push((v, b));
                        }
                    }
                    // **Consecutive labels share one block.** `case 1: case 2: r = 20;`
                    // parses as a `Case` whose *body* is another `Case`, so the switch's
                    // statement loop never saw the second one — it reached the generic
                    // statement path and reported "`case` or `default` outside a switch",
                    // which 015 §7 turns into refusing the whole function. Every C switch
                    // with two labels on one arm was unlowerable.
                    let mut inner = body;
                    loop {
                        match self.ast.stmt(inner).kind.clone() {
                            StmtKind::Case { lo, hi, body: next } => {
                                let lo_v = self.const_of(lo).unwrap_or(0);
                                let hi_v = hi.and_then(|h| self.const_of(h)).unwrap_or(lo_v);
                                if hi_v.saturating_sub(lo_v) > MAX_ENUMERATED_CASE_RANGE {
                                    wide_ranges.push((lo_v, hi_v, b));
                                } else {
                                    for v in lo_v..=hi_v.max(lo_v) {
                                        cases.push((v, b));
                                    }
                                }
                                inner = next;
                            }
                            // `case 1: default:` is legal C and lands both on this block.
                            StmtKind::Default { body: next } => {
                                default = Some(b);
                                inner = next;
                            }
                            _ => break,
                        }
                    }
                    self.stmt(inner);
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

        // **Wide ranges are guards ahead of the switch**, tested in written order so a
        // value in two overlapping ranges reaches the same case gcc would send it to.
        // Each guard leaves a fresh block for the next test, and the last one falls
        // through to the `Switch` itself — so a scrutinee in no range still gets the
        // ordinary dispatch, including the default.
        let mut sw_head = head;
        let wide = std::mem::take(&mut wide_ranges);
        for (lo_v, hi_v, target) in wide {
            let next = self.generated(|s| s.new_block());
            self.switch_to(sw_head);
            let cond = self.generated(|s| s.range_guard(&scrut, &ty, lo_v, hi_v, span));
            self.set_term(Terminator::Br {
                cond,
                t: target,
                f: next,
            });
            sw_head = next;
        }
        self.switch_to(sw_head);
        self.set_term(Terminator::Switch {
            scrut,
            ty,
            cases,
            default: default.unwrap_or(exit),
        });
        self.switch_to(exit);
    }

    /// `lo <= scrut && scrut <= hi`, as one value.
    ///
    /// A conjunction rather than a two-block chain: the test has no side effects, so there
    /// is nothing to short-circuit, and the extra blocks would show up in every golden of
    /// a switch that happens to contain a wide range.
    fn range_guard(
        &mut self,
        scrut: &Operand,
        ty: &CTy,
        lo_v: i128,
        hi_v: i128,
        span: chiero_span::Span,
    ) -> Operand {
        let bits = match ty {
            CTy::Int(b) => *b,
            _ => 32,
        };
        let konst = |val| Operand::Const(Const::Int { bits, val });
        let ge = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: ge,
                rv: RValue::Cmp {
                    op: chiero_cir::CmpOp::SGe,
                    a: scrut.clone(),
                    b: konst(lo_v),
                    ty: ty.clone(),
                },
            },
            span,
        );
        let le = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: le,
                rv: RValue::Cmp {
                    op: chiero_cir::CmpOp::SLe,
                    a: scrut.clone(),
                    b: konst(hi_v),
                    ty: ty.clone(),
                },
            },
            span,
        );
        let both = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: both,
                rv: RValue::Bin {
                    op: chiero_cir::BinOp::And,
                    a: Operand::Value(ge),
                    b: Operand::Value(le),
                    // The comparisons produce `i1`, and the conjunction is of those, not of
                    // the scrutinee's width.
                    ty: CTy::Int(1),
                },
            },
            span,
        );
        Operand::Value(both)
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

    /// Intern a string literal's bytes as a read-only internal global, returning its id.
    fn intern_string(&mut self, bytes: Vec<u8>, span: chiero_span::Span) -> chiero_cir::GlobalId {
        if let Some(g) = self.strings.get(&bytes) {
            return *g;
        }
        let id = chiero_cir::GlobalId(self.module.globals.len() as u32);
        let name: chiero_cir::Symbol = format!(".str.{}", self.strings.len()).into();
        self.module.globals.push(chiero_cir::Global {
            id,
            name,
            size: bytes.len() as u64,
            align: 1,
            // **Read-only, and internal.** A literal a program writes through is undefined
            // behaviour 021 should be able to *report*, which it cannot do if the object
            // is writable; and `Internal` because the name is lowering's invention, not
            // one another TU could ever refer to.
            is_const: true,
            init: chiero_cir::GlobalInit::Bytes(bytes.clone()),
            linkage: chiero_cir::Linkage::Internal,
            span,
        });
        self.strings.insert(bytes, id);
        id
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
    /// A compound literal: an unnamed object with the enclosing block's lifetime.
    ///
    /// C99 6.5.2.5 makes it an *object*, not a value — it has an address, it is an lvalue,
    /// and it can be written through — so it gets a slot exactly as a named local would and
    /// yields that slot's address when its type is one CIR has no value form for.
    ///
    /// **A fresh slot per literal.** `alloca` files it under the scope currently open,
    /// which is the block lifetime C99 asks for, and two literals in one expression are two
    /// objects rather than one reused scratch buffer.
    /// The byte size an array literal needs when its type carries no bound.
    ///
    /// `None` for everything else, so the ordinary `alloca_for` path stays the default.
    fn completed_array_bytes(&mut self, sty: TyId, init: ExprId) -> Option<u64> {
        let Ty::Array { elem, len } = self.analysis.ty(sty).clone() else {
            return None;
        };
        if matches!(len, chiero_sema::ArrayLen::Fixed(_)) {
            return None;
        }
        let chiero_ast::ExprKind::InitList(items) = self.ast.expr(init).kind.clone() else {
            return None;
        };
        let esz = self.analysis.size_of(elem).unwrap_or(1).max(1);
        Some((items.len() as u64).max(1) * esz)
    }

    fn compound_literal_addr(&mut self, sty: TyId, init: ExprId, span: Span) -> Operand {
        let align = self.analysis.align_of(sty).unwrap_or(1).max(1);
        // **An unsized array literal takes its size from its braces** (C99 6.5.2.5p4, via
        // 6.7.9p22): `(int[]){7, 8}` is a two-element array. sema types `int[]` as
        // `ArrayLen::Flexible`, whose `size_of` is 0 — right for a flexible *member* and
        // wrong here — so `alloca_for`'s `.max(1)` gave the object one byte and both
        // element stores went off the end of it. `init_list` already falls back to the item
        // count for a non-`Fixed` length; only the storage was short.
        let slot = match self.completed_array_bytes(sty, init) {
            Some(bytes) => self.alloca_n(CTy::Int(8), bytes, align, None, span),
            None => self.alloca_for(sty, align, None, span),
        };
        let base = self.new_value();
        self.emit(
            InstKind::Assign {
                dst: base,
                rv: RValue::AddrOfLocal { alloca: slot },
            },
            span,
        );
        if self.is_aggregate(sty) {
            // Zero first, then the written elements — C11 6.7.9p21, and the same reason
            // `local_decl` does it: a member the braces do not mention is *defined* zero,
            // and 021 contract 28 needs the `SetMem` to say so or reading it is a finding
            // in well-defined C.
            let size = self.analysis.size_of(sty).unwrap_or(0);
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
            return Operand::Value(base);
        }
        // A **scalar** compound literal — `(int){42}` is legal C and is not an aggregate.
        // It is still an object: `&(int){42}` is valid, so the element is stored and the
        // *address* returned like every other case. The value form loads it back.
        let cty = self.cty(sty);
        let val = match self.ast.expr(init).kind.clone() {
            chiero_ast::ExprKind::InitList(items) if !items.is_empty() => self.expr(items[0].value),
            _ => Operand::Const(Const::Int {
                bits: match &cty {
                    CTy::Int(w) => *w,
                    _ => 32,
                },
                val: 0,
            }),
        };
        self.emit(
            InstKind::Store {
                addr: Operand::Value(base),
                val,
                ty: cty,
                align,
                vol: Volatility::Normal,
            },
            span,
        );
        Operand::Value(base)
    }

    /// A compound literal used as a *value*: its object, then a load for a scalar.
    fn compound_literal(&mut self, sty: TyId, init: ExprId, span: Span) -> Operand {
        let addr = self.compound_literal_addr(sty, init, span);
        if self.is_aggregate(sty) {
            return addr;
        }
        let cty = self.cty(sty);
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Load {
                    addr,
                    ty: cty,
                    align: 1,
                    vol: Volatility::Normal,
                },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// Convert a value to the type it is about to be stored as.
    ///
    /// Only widths and signedness — the caller has already decided *what* is being stored.
    /// Conversion to `_Bool` is `!= 0` rather than a truncation (C11 6.3.1.2), the same
    /// rule waves 133 and 139 needed at the cast and read-modify-write sites.
    /// Emit one conversion instruction and yield its result.
    fn emit_fcast(
        &mut self,
        v: Operand,
        kind: chiero_cir::CastKind,
        from: CTy,
        to: CTy,
        span: Span,
    ) -> Operand {
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Cast {
                    kind,
                    a: v,
                    from,
                    to,
                },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// Whether an integer `CTy` should be read as signed.
    ///
    /// **A `CTy::Int` carries no signedness** — 020 keeps it in the operations rather than
    /// the type — so the destination of a float-to-integer conversion cannot say on its own
    /// whether it wants `FpToSi` or `FpToUi`. Signed is the answer for every `int`, `long`
    /// and `char` a program converts to, and the unsigned case arrives with its own cast
    /// expression whose type sema records; treating the store target as signed is therefore
    /// right for the common path and wrong only where an explicit `(unsigned)` already sits
    /// between the two. Recorded rather than guessed at silently.
    fn target_signed(&self, _to: &CTy) -> bool {
        true
    }

    fn convert_for_store(&mut self, v: Operand, from: ExprId, to: &CTy, span: Span) -> Operand {
        // **The four combinations, because a conversion is decided by both types.** This
        // read `let CTy::Int(want) = *to else { return v }`, which is right for the integer
        // half of C and silently emits nothing for the other three — so `double d = 7;`
        // stored an `i32` into an `f64` slot and the verifier rejected the function.
        let have_ty = self.type_of(from).map(|t| self.cty(t));
        if let (Some(CTy::Float(fk)), CTy::Float(tk)) = (have_ty.clone(), to.clone()) {
            // Equal widths need no instruction at all, which is 015's rule that a no-op
            // cast is not emitted rather than emitted and folded.
            let kind = match float_width(fk).cmp(&float_width(tk)) {
                std::cmp::Ordering::Greater => chiero_cir::CastKind::FpTrunc,
                std::cmp::Ordering::Less => chiero_cir::CastKind::FpExt,
                std::cmp::Ordering::Equal => return v,
            };
            return self.emit_fcast(v, kind, CTy::Float(fk), CTy::Float(tk), span);
        }
        if let CTy::Float(tk) = to.clone() {
            // Integer to float. The *source's* signedness decides which conversion, since
            // it is what says whether the top bit is a sign or a magnitude.
            let have = self.width_of(from);
            let kind = if self.is_signed(from) {
                chiero_cir::CastKind::SiToFp
            } else {
                chiero_cir::CastKind::UiToFp
            };
            return self.emit_fcast(v, kind, CTy::Int(have), CTy::Float(tk), span);
        }
        if let (Some(CTy::Float(fk)), CTy::Int(want)) = (have_ty, to.clone()) {
            // **No special case for `_Bool` here.** C11 6.3.1.2 makes a conversion to
            // `_Bool` a comparison against zero rather than the truncation `FpToSi`
            // performs — but sema inserts that conversion as its own cast, which reaches
            // `truth_of` directly, so a branch for it here is never taken. Mutation proved
            // it: deleting the branch changed no test. `truth_of` is where the rule lives.
            // Float to integer. The *target's* signedness decides, and C11 6.3.1.4 makes
            // it a truncation toward zero rather than a rounding.
            let kind = if matches!(to, CTy::Int(_)) && self.target_signed(to) {
                chiero_cir::CastKind::FpToSi
            } else {
                chiero_cir::CastKind::FpToUi
            };
            return self.emit_fcast(v, kind, CTy::Float(fk), CTy::Int(want), span);
        }
        let CTy::Int(want) = *to else { return v };
        let have = self.width_of(from);
        if have == want {
            return v;
        }
        if want == 1 {
            return self.truth_of(v, CTy::Int(have), span);
        }
        let kind = if want < have {
            chiero_cir::CastKind::Trunc
        } else if self.is_signed(from) {
            chiero_cir::CastKind::SExt
        } else {
            chiero_cir::CastKind::ZExt
        };
        let dst = self.new_value();
        self.emit(
            InstKind::Assign {
                dst,
                rv: RValue::Cast {
                    kind,
                    a: v,
                    from: CTy::Int(have),
                    to: CTy::Int(want),
                },
            },
            span,
        );
        Operand::Value(dst)
    }

    /// `&g`, `g` for an array, or `&g[k]` — the address of a file-scope object.
    ///
    /// **Only a file-scope target.** The address of a *local* is not a constant expression
    /// and cannot initialize static storage, so `None` here is the right answer for it and
    /// C would have rejected the program anyway.
    fn global_addr_init(&mut self, e: ExprId) -> Option<(chiero_cir::GlobalId, i64)> {
        match self.ast.expr(e).kind.clone() {
            // `&x` — the operand names the object directly.
            chiero_ast::ExprKind::Unary {
                op: chiero_ast::UnOp::AddrOf,
                operand,
            } => self.global_addr_of(operand),
            // A bare array name decays to its own address (C11 6.3.2.1p3), which is what
            // `int *gp = ga;` is. A bare *scalar* name does not, so it is excluded — that
            // is `int a = b;`, an ordinary constant read.
            chiero_ast::ExprKind::Ident(_) => {
                let t = self.type_of(e)?;
                matches!(self.analysis.ty(t), Ty::Array { .. }).then_some(())?;
                self.global_addr_of(e)
            }
            _ => None,
        }
    }

    /// The `(global, byte offset)` an lvalue expression names, when it is file-scope.
    fn global_addr_of(&mut self, e: ExprId) -> Option<(chiero_cir::GlobalId, i64)> {
        match self.ast.expr(e).kind.clone() {
            chiero_ast::ExprKind::Ident(sym) => {
                // **No local-shadowing check, deliberately.** A draft had one; a mutation
                // deleting it survived, and the reason is that it was unreachable. This
                // function has exactly one caller — the file-scope declaration path, where
                // no local is in scope — and `&automatic` is not a constant expression, so
                // `static int *p = &x;` naming a local is a program C rejects
                // ("initializer element is not constant"). A guard against a case that
                // cannot arise is dead code wearing a confident comment.
                self.globals.get(&sym).copied().map(|g| (g, 0))
            }
            // `&ga[k]` for a constant `k`.
            chiero_ast::ExprKind::Index { base, index } => {
                let (g, off) = self.global_addr_of(base)?;
                let k = self.const_of(index)?;
                let esz = self.elem_size_of(base)? as i64;
                Some((g, off + k as i64 * esz))
            }
            _ => None,
        }
    }

    fn init_list(&mut self, base: Operand, ty: TyId, init: ExprId, span: Span) {
        let chiero_ast::ExprKind::InitList(items) = self.ast.expr(init).kind.clone() else {
            return;
        };
        // The fields to walk, as (byte offset, type, name, bit range), in declaration
        // order. **The bit range travels with the slot**, because a bit-field initializer
        // is `StoreBits` and not a narrower `Store` — 015 contract 7, which `assign` obeys
        // and this function did not. Deriving it here from `RecordLayout` is the same
        // single source `bitfield_of` uses for the assignment path.
        type Slot = (
            u64,
            TyId,
            Option<chiero_span::Symbol>,
            Option<chiero_cir::BitRange>,
        );
        let slots: Vec<Slot> = match self.analysis.ty(ty).clone() {
            Ty::Record(r) => {
                let l = self.analysis.layout(r);
                l.fields
                    .iter()
                    .map(|f| {
                        let bits = f.bits.map(|b| chiero_cir::BitRange {
                            off: (b.bit_offset - f.offset * 8) as u32,
                            width: b.width as u32,
                        });
                        (f.offset, f.ty, f.name, bits)
                    })
                    .collect()
            }
            Ty::Array { elem, len } => {
                let n = match len {
                    chiero_sema::ArrayLen::Fixed(n) => n,
                    _ => items.len() as u64,
                };
                let esz = self.analysis.size_of(elem).unwrap_or(1);
                (0..n).map(|i| (i * esz, elem, None, None)).collect()
            }
            _ => return,
        };

        let mut cursor = 0usize;
        for item in items {
            // A designator repositions the cursor; C11 6.7.9p17 continues from there.
            if let Some(chiero_ast::Designator::Field(name)) = item.designators.first() {
                if let Some(i) = slots.iter().position(|(_, _, n, _)| *n == Some(*name)) {
                    cursor = i;
                }
            } else if let Some(chiero_ast::Designator::Index(idx)) = item.designators.first()
                && let Some(v) = self.const_of(*idx)
            {
                cursor = v.max(0) as usize;
            }
            let Some(&(off, fty, _, bits)) = slots.get(cursor) else {
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
            // **The initializer is converted first, whichever store follows** (C11 6.7.9p11:
            // as if by assignment). sema inserts that conversion for an assignment
            // *expression* and not for a braced element, so without this a `{3, 5}` into
            // `struct S { signed char a; int b; }` stored a 32-bit `3` into a slot declared
            // `i8`, and a `long` initializer for an `int:3` gave `StoreBits` a 64-bit value
            // where its unit is 32.
            //
            // **Hoisted above the bit-field branch deliberately.** Wave 140 added it for the
            // ordinary store and wave 142 added the bit-field branch *in front of it*, so
            // the new path silently skipped a conversion the old one needed — the ledger
            // caught that two waves later. One conversion for both stores is one thing to
            // get right, and `cty` is already the correct target for each: the member's type
            // for a plain store, the field's storage unit for a bit-field.
            let v = self.convert_for_store(v, item.value, &cty, span);
            // **A bit-field member is `StoreBits`**, not a narrower `Store` (015 contract
            // 7, whose `BitRange` came from `RecordLayout` above). A full-width store here
            // wrote over every neighbour sharing the unit, so `{1, 2}` into
            // `struct { int a:3; int b:5; }` put 1 across the whole unit and then 2 across
            // it again. `assign` has obeyed this rule all along; this path never did, and a
            // single bit-field at offset 0 hid it by having no neighbour to clobber.
            //
            // The value is *not* pre-truncated: `StoreBits` writes `width` bits and the
            // reinterpretation at the field's signedness happens on the read, which is what
            // makes 7 in a 3-bit signed field read back as −1.
            if let Some(bits) = bits {
                self.emit(
                    InstKind::StoreBits {
                        addr,
                        val: v,
                        unit: cty,
                        bits,
                        align: 1,
                    },
                    span,
                );
                continue;
            }
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

/// Which of 020 §4.4.1's varargs builtins a name is, if any.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum VaBuiltin {
    Start,
    Arg,
    End,
    Copy,
}

fn va_builtin(name: &str) -> Option<VaBuiltin> {
    match name {
        "__builtin_va_start" => Some(VaBuiltin::Start),
        "__builtin_va_arg" => Some(VaBuiltin::Arg),
        "__builtin_va_end" => Some(VaBuiltin::End),
        "__builtin_va_copy" => Some(VaBuiltin::Copy),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// 020 §7: syntactic order sensitivity
// ---------------------------------------------------------------------------------------

/// Whether a function body contains a **syntactically decidable** unsequenced conflict
/// (020 §7, contract 18a).
///
/// §7 restricts this deliberately: two unsequenced accesses to the same *lvalue root*
/// within one full expression, at least one a write, **with no intervening call**. That is
/// a local, call-free check over syntax, which 001 §2 permits `chiero-lower` to do without
/// becoming an analysis. Everything a call makes uncertain belongs to 040's checker, which
/// has the memory model to answer "is this the same object?".
///
/// The model of "unsequenced" that matches C and §7's three examples:
///
/// - Reads never conflict with each other. §7 says "at least one a write".
/// - Two **side-effect writes** in one region always conflict: C11 6.5p2.
/// - An assignment's *own* write is sequenced after its operands' value computations
///   (C11 6.5.16p3), so `i = i + 1` is defined — the read is not a conflict. It is *not*
///   sequenced after their side effects, so `i = i++` is two writes and is not.
///
/// That last distinction is the whole reason the assignment's write is tracked separately
/// from the writes inside its operands.
fn order_sensitive_body(ast: &Ast, body: chiero_ast::StmtId) -> bool {
    let mut cx = OrderScan { ast, found: false };
    cx.stmt(body);
    cx.found
}

struct OrderScan<'a> {
    ast: &'a Ast,
    found: bool,
}

/// One access in an unsequenced region.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    /// A side-effect write: `++`, `--`, or a nested assignment.
    Write,
    /// The write of the assignment whose operands this region is. Sequenced *after* the
    /// reads in the region, but not after its writes.
    AssignWrite,
}

impl OrderScan<'_> {
    fn stmt(&mut self, s: chiero_ast::StmtId) {
        use chiero_ast::StmtKind as K;
        // Every expression a statement contains is its own full expression: the statement
        // boundary *is* the sequence point (C11 6.8p4). Walking statements here rather
        // than expressions is what gives `i++; i++;` two regions for free.
        match self.ast.stmt(s).kind.clone() {
            K::Expr(e) | K::Return(Some(e)) | K::GotoIndirect(e) => self.full_expr(e),
            K::If { cond, then, els } => {
                self.full_expr(cond);
                self.stmt(then);
                if let Some(e) = els {
                    self.stmt(e);
                }
            }
            K::While { cond, body } | K::DoWhile { body, cond } => {
                self.full_expr(cond);
                self.stmt(body);
            }
            K::For {
                init,
                cond,
                step,
                body,
            } => {
                match init {
                    Some(chiero_ast::ForInit::Expr(e)) => self.full_expr(e),
                    Some(chiero_ast::ForInit::Decl(ds)) => {
                        for d in ds {
                            if let chiero_ast::DeclKind::Var { init: Some(e), .. } =
                                self.ast.decl(d).kind.clone()
                            {
                                self.full_expr(e);
                            }
                        }
                    }
                    None => {}
                }
                if let Some(c) = cond {
                    self.full_expr(c);
                }
                if let Some(st) = step {
                    self.full_expr(st);
                }
                self.stmt(body);
            }
            K::Switch { cond, body } => {
                self.full_expr(cond);
                self.stmt(body);
            }
            K::Compound(ss) => {
                for x in ss {
                    self.stmt(x);
                }
            }
            K::Label { body, .. } | K::Case { body, .. } | K::Default { body } => self.stmt(body),
            K::Decl(ds) => {
                for d in ds {
                    if let chiero_ast::DeclKind::Var { init: Some(e), .. } =
                        self.ast.decl(d).kind.clone()
                    {
                        self.full_expr(e);
                    }
                }
            }
            K::Asm(a) => {
                for op in a.outputs.iter().chain(a.inputs.iter()) {
                    self.full_expr(op.expr);
                }
            }
            K::Return(None) | K::Goto(_) | K::Break | K::Continue | K::Empty | K::Error => {}
        }
    }

    /// Scan one full expression, splitting at the operators C sequences.
    fn full_expr(&mut self, e: chiero_ast::ExprId) {
        let mut acc: Vec<(chiero_span::Symbol, Access)> = Vec::new();
        if self.region(e, false, &mut acc) && conflicts(&acc) {
            self.found = true;
        }
    }

    /// Collect the accesses in `e` into `acc`, returning `false` if the region contains a
    /// **call** — after one, §7 hands the question to 040.
    ///
    /// Operators with their own sequence points (`&&`, `||`, `?:`, `,`) recurse as
    /// *separate* regions instead of contributing here, which is why `i++ && i++` is
    /// defined and `i++ + i++` is not.
    fn region(
        &mut self,
        e: chiero_ast::ExprId,
        writing: bool,
        acc: &mut Vec<(chiero_span::Symbol, Access)>,
    ) -> bool {
        use chiero_ast::ExprKind as K;
        match self.ast.expr(e).kind.clone() {
            K::Ident(name) => {
                acc.push((name, if writing { Access::Write } else { Access::Read }));
                true
            }
            K::Assign { lhs, rhs, op } => {
                // The assigned root's write. A *compound* assignment also reads it, and
                // both are the assignment's own accesses — sequenced after the operands'
                // value computations either way.
                if let Some(root) = self.root_of(lhs) {
                    acc.push((root, Access::AssignWrite));
                    if op.is_some() {
                        acc.push((root, Access::Read));
                    }
                }
                // The subexpressions *within* the lvalue are ordinary reads: in
                // `a[i] = i++` the subscript's read of `i` is what races the increment.
                self.lvalue_interior(lhs, acc) && self.region(rhs, false, acc)
            }
            K::Unary { op, operand } => match op {
                chiero_ast::UnOp::PreInc | chiero_ast::UnOp::PreDec => {
                    self.region(operand, true, acc)
                }
                _ => self.region(operand, false, acc),
            },
            K::Postfix { operand, .. } => self.region(operand, true, acc),
            K::Binary { op, lhs, rhs } => {
                // `&&`, `||` and `,` sequence their operands (C11 6.5.13p4, 6.5.14p4,
                // 6.5.17p2), so each side is its own region rather than part of this one.
                if matches!(op, chiero_ast::BinOp::LogAnd | chiero_ast::BinOp::LogOr) {
                    self.full_expr(lhs);
                    self.full_expr(rhs);
                    return true;
                }
                self.region(lhs, false, acc) && self.region(rhs, false, acc)
            }
            K::Comma { lhs, rhs } => {
                self.full_expr(lhs);
                self.full_expr(rhs);
                true
            }
            K::Cond { cond, then, els } => {
                self.full_expr(cond);
                if let Some(t) = then {
                    self.full_expr(t);
                }
                self.full_expr(els);
                true
            }
            // **A call ends the syntactic region.** Its arguments are still scanned — they
            // are unsequenced with each other, which is `g(i++, i++)` — but anything
            // *outside* it in the same expression is no longer decidable here.
            K::Call { args, .. } => {
                let mut inner: Vec<(chiero_span::Symbol, Access)> = Vec::new();
                for a in &args {
                    self.region(*a, false, &mut inner);
                }
                if conflicts(&inner) {
                    self.found = true;
                }
                false
            }
            K::Index { base, index } => {
                self.region(base, false, acc) && self.region(index, false, acc)
            }
            K::Member { base, .. } => self.region(base, writing, acc),
            K::Cast { operand, .. } => self.region(operand, writing, acc),
            K::SizeofExpr(_) | K::SizeofType(_) | K::AlignofType(_) | K::TypeName(_) => true,
            // A statement expression contains statements, each its own full expression.
            K::StmtExpr(s) => {
                self.stmt(s);
                false
            }
            K::InitList(items) => {
                for it in &items {
                    self.full_expr(it.value);
                }
                true
            }
            K::Number(_) | K::Char { .. } | K::Str { .. } | K::Error => true,
        }
    }

    /// The reads inside an lvalue — `a[i]`'s `i`, `p->f`'s `p` — as distinct from the
    /// write to its root.
    fn lvalue_interior(
        &mut self,
        e: chiero_ast::ExprId,
        acc: &mut Vec<(chiero_span::Symbol, Access)>,
    ) -> bool {
        use chiero_ast::ExprKind as K;
        match self.ast.expr(e).kind.clone() {
            K::Ident(_) => true,
            K::Index { base, index } => {
                self.lvalue_interior(base, acc) && self.region(index, false, acc)
            }
            K::Member { base, .. } => self.lvalue_interior(base, acc),
            K::Unary {
                op: chiero_ast::UnOp::Deref,
                operand,
            } => self.region(operand, false, acc),
            other => {
                let _ = other;
                self.region(e, false, acc)
            }
        }
    }

    /// The identifier an lvalue is rooted at, if there is one.
    fn root_of(&self, e: chiero_ast::ExprId) -> Option<chiero_span::Symbol> {
        use chiero_ast::ExprKind as K;
        match self.ast.expr(e).kind.clone() {
            K::Ident(n) => Some(n),
            K::Index { base, .. } | K::Member { base, .. } => self.root_of(base),
            K::Unary { operand, .. } | K::Cast { operand, .. } => self.root_of(operand),
            _ => None,
        }
    }
}

/// Whether a region's accesses contain an unsequenced conflict.
fn conflicts(acc: &[(chiero_span::Symbol, Access)]) -> bool {
    for (i, (root, a)) in acc.iter().enumerate() {
        for (other, b) in acc.iter().skip(i + 1) {
            if other != root {
                continue;
            }
            let bad = match (a, b) {
                // Two side effects, in either order: always a conflict.
                (Access::Write, Access::Write)
                | (Access::Write, Access::AssignWrite)
                | (Access::AssignWrite, Access::Write)
                | (Access::AssignWrite, Access::AssignWrite) => true,
                // A side-effect write racing a read.
                (Access::Write, Access::Read) | (Access::Read, Access::Write) => true,
                // **The assignment's own write does not race its operands' reads**
                // (C11 6.5.16p3), which is what makes `i = i + 1` defined.
                (Access::AssignWrite, Access::Read) | (Access::Read, Access::AssignWrite) => false,
                (Access::Read, Access::Read) => false,
            };
            if bad {
                return true;
            }
        }
    }
    false
}
