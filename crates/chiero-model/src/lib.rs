//! `chiero-model` — what happens when execution reaches a function whose body is not in
//! the module (024).
//!
//! **This crate contains no VPP knowledge.** It defines the registry and the standard
//! models; `chiero-vpp` registers vppinfra models *into* it. 024 contract 19 enforces
//! that with a prefix scan over these sources, so the forbidden prefixes are not written
//! out even here — a guard cannot tell an identifier from prose quoting one, and
//! weakening it to make room for the prose would defeat it.
//!
//! The rule that matters most is §2.1: declaring a model `Approximate` is **mechanical**,
//! not editorial. Dispatching one degrades fidelity and records why. Without that a run
//! calling `scanf` could finish `Exact`, mint a witness, and report "no bugs exist" as a
//! proof — and unlike the unmodeled path, that one looks deliberate.

/// Interned like , but this crate must not depend on the IR — a
/// model registry is upstream of it.
pub type Symbol = std::sync::Arc<str>;

use indexmap::IndexMap;

/// How faithful a model is. `Approximate` carries the reason, because an approximation
/// nobody can name is indistinguishable from a bug.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precision {
    Exact,
    Approximate(Symbol),
}

/// The fidelity levels this crate can cause. Mirrors 023 §7's table; the engine owns the
/// full enum, and duplicating it whole is how earlier drafts drifted into four
/// inconsistent versions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModelFidelity {
    Approximated,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// 024 contract 18. Silent last-wins would make which model you got depend on link
    /// order, and 001 §5 makes determinism a hard requirement.
    Duplicate(Symbol),
    NotFound(Symbol),
}

/// What a model invalidates when it cannot say more (024 §2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HavocSpec {
    pub objects: Vec<u32>,
    /// Follow pointers stored *inside* those objects, this many deep.
    pub reachable_depth: u32,
    pub init: HavocInit,
    pub may_free: bool,
}

/// **No safe default.** `Symbolic` marks bytes initialized-with-unknown-value, which can
/// mask a genuine uninitialized-read bug; `Uninitialized` produces a false-positive storm
/// on any buffer the callee legitimately filled. The choice is recorded so it is visible
/// rather than folkloric.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HavocInit {
    Symbolic,
    Uninitialized,
}

impl HavocSpec {
    /// 024 §2.1's default for an unmodeled extern: an unknown function is assumed to have
    /// written something meaningful rather than left garbage.
    pub fn unmodeled_extern() -> HavocSpec {
        HavocSpec {
            objects: Vec::new(),
            reachable_depth: 1,
            init: HavocInit::Symbolic,
            may_free: false,
        }
    }

    /// 024 contract 21c: a havoc degrades the same wherever it came from. "I don't know"
    /// said politely by a registered model is exactly as imprecise as "I don't know" said
    /// by omission.
    pub fn fidelity_effect(&self) -> ModelFidelity {
        ModelFidelity::Approximated
    }

    /// The text that goes into the assumption, so the choice appears in the report.
    pub fn describe(&self) -> String {
        let init = match self.init {
            HavocInit::Symbolic => "symbolic",
            HavocInit::Uninitialized => "uninitialized",
        };
        format!(
            "havoc: {init} contents, reachable pointers to depth {}{}",
            self.reachable_depth,
            if self.may_free { ", may free" } else { "" }
        )
    }
}

/// 024 contract 1/2. Allocation failure is a real path; pruning it silently hides a bug
/// class, so it is explored by default and suppressing it is a deliberate setting.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AllocPolicy {
    pub may_fail: bool,
}

impl Default for AllocPolicy {
    fn default() -> AllocPolicy {
        AllocPolicy { may_fail: true }
    }
}

impl AllocPolicy {
    pub fn outcomes(self) -> usize {
        if self.may_fail { 2 } else { 1 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    pub name: Symbol,
    pub precision: Precision,
}

impl ModelEntry {
    pub fn exact(name: &str) -> ModelEntry {
        ModelEntry {
            name: name.into(),
            precision: Precision::Exact,
        }
    }

    pub fn approximate(name: &str, why: &str) -> ModelEntry {
        ModelEntry {
            name: name.into(),
            precision: Precision::Approximate(why.into()),
        }
    }

    /// **024 §2.1: mechanical, not editorial.** An approximate model degrades *by being
    /// dispatched*, not by anyone remembering to record it.
    pub fn fidelity_effect(&self) -> Option<ModelFidelity> {
        match self.precision {
            Precision::Exact => None,
            Precision::Approximate(_) => Some(ModelFidelity::Approximated),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelRegistry {
    by_name: IndexMap<Symbol, usize>,
    models: Vec<ModelEntry>,
}

impl ModelRegistry {
    pub fn new() -> ModelRegistry {
        ModelRegistry::default()
    }

    /// The standard models. Each `Approximate` reason names the *category* of
    /// imprecision, because a reader triaging a degraded run needs to know whether it was
    /// floats, input or a syscall before they need to know which call.
    pub fn with_builtins() -> ModelRegistry {
        let mut r = ModelRegistry::new();
        // **Only what is implemented is registered.** A declaration claiming `Exact`
        // precision for a function nothing can dispatch says "this degrades nothing"
        // about something that cannot run — the confidently-wrong shape this module's
        // own doc rails against, pointed the wrong way. The rest of 024 §3-§6 is owed,
        // and an unregistered name takes the engine's loud unmodeled path meanwhile.
        for e in [
            ModelEntry::exact("malloc"),
            ModelEntry::exact("calloc"),
            ModelEntry::exact("free"),
            ModelEntry::exact("memcpy"),
            ModelEntry::exact("memmove"),
            ModelEntry::exact("memset"),
            ModelEntry::approximate("scanf", "reads external input, which is unconstrained"),
            ModelEntry::approximate("fscanf", "reads external input, which is unconstrained"),
            ModelEntry::approximate("read", "syscall result is outside the program"),
            ModelEntry::approximate("write", "syscall result is outside the program"),
            ModelEntry::approximate("ioctl", "syscall result is outside the program"),
            ModelEntry::approximate("printf", "formatted output is not modeled precisely"),
            ModelEntry::approximate("sqrt", "floating point is approximated (023 §7)"),
            ModelEntry::approximate("pow", "floating point is approximated (023 §7)"),
            ModelEntry::approximate("longjmp", "non-local control flow is unsupported"),
        ] {
            r.register(e).expect("the builtin list has no duplicates");
        }
        r
    }

    pub fn register(&mut self, e: ModelEntry) -> Result<(), ModelError> {
        if self.by_name.contains_key(&e.name) {
            return Err(ModelError::Duplicate(e.name));
        }
        self.by_name.insert(e.name.clone(), self.models.len());
        self.models.push(e);
        Ok(())
    }

    /// Override an existing model. Distinct from `register` so an accidental collision is
    /// an error and a deliberate one is a different call.
    pub fn replace(&mut self, e: ModelEntry) -> Result<(), ModelError> {
        match self.by_name.get(&e.name) {
            Some(&i) => {
                self.models[i] = e;
                Ok(())
            }
            None => Err(ModelError::NotFound(e.name)),
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&ModelEntry> {
        self.by_name.get(name).map(|&i| &self.models[i])
    }

    pub fn entries(&self) -> impl Iterator<Item = &ModelEntry> {
        self.models.iter()
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Executing models (024 §3).
// ---------------------------------------------------------------------------

use chiero_mem::{Endian, MemFault, Memory, ObjKind, ObjectId, Pointer};
use chiero_solver::{Term, TermArena};
use chiero_span::Span;

/// What a model returns. `Havoc` is the explicit "I don't know" — never implicit, because
/// an implicit one is indistinguishable from a model that got the answer right.
#[derive(Clone, Debug)]
pub enum ModelOutcome {
    Value(Option<Value>),
    /// Guarded alternatives — `malloc`'s success and failure, for instance.
    Fork(Vec<(Option<Term>, ModelOutcome)>),
    /// The call could not be performed and the reason is reportable.
    Finding(String),
    Havoc(HavocSpec),
}

/// Mirrors 023 §1.1: a pointer keeps its object. A model handing back a bare term would
/// lose provenance at exactly the boundary where it matters most, since `malloc` is where
/// most heap objects come from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Scalar(Term),
    Ptr(Pointer),
}

/// What a model is allowed to touch. Deliberately narrow: a model gets memory, a term
/// arena, the call's span, and a place to put findings — not the engine's state, which is
/// what keeps 024's models reusable across the searcher and threading choices of 023.
#[derive(Debug)]
pub struct ModelCtx<'a> {
    mem: &'a mut Memory,
    arena: &'a mut TermArena,
    span: Span,
    endian: Endian,
    findings: Vec<String>,
}

impl<'a> ModelCtx<'a> {
    /// The byte order is a **parameter**, not a constant. It was hardcoded under a
    /// comment claiming it came from the target, and the test asserted the same constant
    /// — so neither could tell a target-driven implementation from a hardcoded one.
    pub fn new(
        mem: &'a mut Memory,
        arena: &'a mut TermArena,
        span: Span,
        endian: Endian,
    ) -> ModelCtx<'a> {
        ModelCtx {
            mem,
            arena,
            span,
            endian,
            findings: Vec::new(),
        }
    }

    pub fn mem(&mut self) -> &mut Memory {
        self.mem
    }
    pub fn arena(&mut self) -> &mut TermArena {
        self.arena
    }
    pub fn span(&self) -> Span {
        self.span
    }
    pub fn endian(&self) -> Endian {
        self.endian
    }
    pub fn findings(&self) -> &[String] {
        &self.findings
    }
    pub fn report(&mut self, what: impl Into<String>) {
        self.findings.push(what.into());
    }
    fn lift(&mut self, faults: &[MemFault]) {
        for f in faults {
            self.findings.push(format!("{f:?}"));
        }
    }
}

/// The standard memory models. Each is a plain function so it can be tested without an
/// engine — 024 §2's "two ways to write a model" is about registration, not about needing
/// a running interpreter to check `calloc` zeroes.
pub mod models {
    use super::*;

    /// Whether a name has an implementation here. The registry uses it so an `Exact`
    /// declaration cannot outrun the code behind it.
    pub fn is_implemented(name: &str) -> bool {
        matches!(
            name,
            "malloc" | "calloc" | "free" | "memcpy" | "memmove" | "memset"
        )
    }

    /// 024 contract 1/2. Uninitialized contents, and a `NULL` branch unless the allocator
    /// cannot fail.
    pub fn malloc(cx: &mut ModelCtx, size: u64, policy: AllocPolicy) -> ModelOutcome {
        let at = cx.span();
        let o = cx.mem().alloc(ObjKind::Heap, size, 16, at);
        let ok = ModelOutcome::Value(Some(Value::Ptr(Pointer { base: o, off: 0 })));
        if !policy.may_fail {
            return ok;
        }
        // The failure branch is cheap and most real allocation-failure bugs are
        // unreachable without it.
        let null = ModelOutcome::Value(Some(Value::Ptr(Pointer {
            base: ObjectId::NULL,
            off: 0,
        })));
        ModelOutcome::Fork(vec![(None, ok), (None, null)])
    }

    /// 024 contract 3/4. Zeroed **and marked initialized**, with the `n * m` overflow
    /// reported rather than wrapped — a wrap allocates a small object for a request that
    /// cannot be satisfied.
    pub fn calloc(cx: &mut ModelCtx, n: u64, m: u64, policy: AllocPolicy) -> ModelOutcome {
        let Some(size) = n.checked_mul(m) else {
            let msg = format!("calloc({n}, {m}): size computation overflows");
            cx.report(msg.clone());
            return ModelOutcome::Finding(msg);
        };
        let at = cx.span();
        let o = cx.mem().alloc(ObjKind::Heap, size, 16, at);
        let p = Pointer { base: o, off: 0 };
        // `set` marks the range initialized, which is the difference from `malloc` and
        // the reason a correct `calloc` user reports no uninitialized read.
        let r = cx.mem().set(p, 0, size, at);
        let faults = r.faults.clone();
        cx.lift(&faults);
        let ok = ModelOutcome::Value(Some(Value::Ptr(p)));
        if !policy.may_fail {
            return ok;
        }
        let null = ModelOutcome::Value(Some(Value::Ptr(Pointer {
            base: ObjectId::NULL,
            off: 0,
        })));
        ModelOutcome::Fork(vec![(None, ok), (None, null)])
    }

    /// 024 contract 5. `free(NULL)` is a no-op; freeing anything that did not come from
    /// the heap is a finding.
    pub fn free(cx: &mut ModelCtx, p: Pointer) -> ModelOutcome {
        // No NULL check here: `Memory::free` owns that rule, and a second copy of it is
        // how `readonly` came to hold on one write path out of three. One place.
        let at = cx.span();
        let r = cx.mem().free(p.base, at);
        let faults = r.faults.clone();
        cx.lift(&faults);
        ModelOutcome::Value(None)
    }

    /// 024 contract 10. Overlap is a finding for `memcpy` and not for `memmove`, and the
    /// bytes are the same either way — `memmove` copies as if through a temporary.
    pub fn memcpy(cx: &mut ModelCtx, dst: Pointer, src: Pointer, size: u64) -> ModelOutcome {
        copy(cx, dst, src, size, chiero_mem::Overlap::Forbidden)
    }

    pub fn memmove(cx: &mut ModelCtx, dst: Pointer, src: Pointer, size: u64) -> ModelOutcome {
        copy(cx, dst, src, size, chiero_mem::Overlap::Allowed)
    }

    fn copy(
        cx: &mut ModelCtx,
        dst: Pointer,
        src: Pointer,
        size: u64,
        rule: chiero_mem::Overlap,
    ) -> ModelOutcome {
        let at = cx.span();
        let r = cx.mem().copy(dst, src, size, rule, at);
        let faults = r.faults.clone();
        cx.lift(&faults);
        ModelOutcome::Value(Some(Value::Ptr(dst)))
    }

    /// The range becomes initialized and reads back as the set byte; nothing outside it
    /// changes.
    pub fn memset(cx: &mut ModelCtx, dst: Pointer, byte: u8, size: u64) -> ModelOutcome {
        let at = cx.span();
        let r = cx.mem().set(dst, byte, size, at);
        let faults = r.faults.clone();
        cx.lift(&faults);
        ModelOutcome::Value(Some(Value::Ptr(dst)))
    }
}
