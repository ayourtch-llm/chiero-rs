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
    /// 024 §2.1's own field type. `Vec<u32>` here meant a caller could pass anything and
    /// a reader could not tell what the numbers were.
    pub objects: Vec<ObjectId>,
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

/// 024 §4. `max_scan` bounds the walk; it is **not** an assumption that a terminator
/// exists within it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StringPolicy {
    pub max_scan: u64,
}

impl Default for StringPolicy {
    fn default() -> StringPolicy {
        StringPolicy { max_scan: 256 }
    }
}

/// The outcome of a string walk. Three outcomes, not two, because §4 is explicit that
/// "the object ended" and "I stopped looking" are different claims — collapsing them is
/// how the cap comes to assume away the bug the OOB check exists to find.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrScan {
    /// A NUL was found at this offset from the start.
    Exact(u64),
    /// The object ended with no NUL: an out-of-bounds read, and 024 §4 calls this the
    /// most valuable thing these models catch.
    Unterminated { scanned: u64 },
    /// `max_scan` was reached first. **No constraint is added** — nothing is known about
    /// the rest of the object, and claiming a bug there would be inventing one.
    CapReached { scanned: u64 },
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
            ModelEntry::exact("strlen"),
            ModelEntry::exact("strcpy"),
            ModelEntry::exact("__builtin_clz"),
            ModelEntry::exact("__builtin_ctz"),
            ModelEntry::exact("__builtin_popcount"),
            ModelEntry::exact("__builtin_bswap16"),
            ModelEntry::exact("__builtin_bswap32"),
            ModelEntry::exact("__builtin_bswap64"),
            ModelEntry::exact("__builtin_add_overflow"),
            ModelEntry::exact("__builtin_sub_overflow"),
            ModelEntry::exact("__builtin_mul_overflow"),
            ModelEntry::exact("chiero_assume"),
            ModelEntry::exact("chiero_assert"),
            ModelEntry::exact("chiero_mark_fidelity"),
            ModelEntry::approximate("scanf", "reads external input, which is unconstrained"),
            ModelEntry::approximate("fscanf", "reads external input, which is unconstrained"),
            ModelEntry::approximate("read", "syscall result is outside the program"),
            ModelEntry::approximate("write", "syscall result is outside the program"),
            ModelEntry::approximate("ioctl", "syscall result is outside the program"),
            ModelEntry::approximate("printf", "formatted output is not modeled precisely"),
            ModelEntry::approximate("sqrt", "floating point is approximated (023 §7)"),
            ModelEntry::approximate("pow", "floating point is approximated (023 §7)"),
            // Exact **as a model**: chiero performs it faithfully by ending the path,
            // which is what `longjmp` does to the current one. 024 contract 20 wants
            // `Unknown` on the *result*, and the model's `Terminate` is what produces it.
            ModelEntry::exact("longjmp"),
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
    /// The call does not return, and chiero cannot follow where it went. 024 contract 20:
    /// `longjmp` must terminate the state at `Unknown`, never continue it. A `Finding`
    /// would report the same words and leave execution walking down a path the program
    /// does not have, which is the failure this variant exists to make impossible to
    /// express.
    Terminate(String),
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
    /// Memory faults become findings **as sentences**. `{:?}` put chiero's internal
    /// struct shape in the product: 001 §1 has an LLM at the other end of these, and a
    /// reader should not have to decode `Uninitialized { obj: ObjectId(2), off: 0, bit: 0,
    /// at: Span { lo: BytePos(0), … } }` to learn that byte 0 was never written.
    fn lift(&mut self, faults: &[MemFault]) {
        for f in faults {
            self.findings.push(f.to_string());
        }
    }
}

/// The standard memory models. Each is a plain function so it can be tested without an
/// engine — 024 §2's "two ways to write a model" is about registration, not about needing
/// a running interpreter to check `calloc` zeroes.
/// **The single list of what the engine can actually run.** `can_dispatch` and
/// `is_implemented` were two hand-written lists that could drift in both directions — a
/// name dispatchable with nothing behind it, or implemented and unreachable. Having one
/// makes the link structural instead of a convention.
pub const DISPATCHABLE: &[&str] = &[
    "malloc",
    "calloc",
    "free",
    "memcpy",
    "memmove",
    "memset",
    "strlen",
    "strcpy",
    "chiero_assume",
    "chiero_assert",
    "chiero_mark_fidelity",
    "longjmp",
    "scanf",
];

pub fn dispatchable() -> &'static [&'static str] {
    DISPATCHABLE
}

pub mod models {
    use super::*;

    /// Whether a name has an implementation **here**, in this module. The registry uses
    /// it so an `Exact` declaration cannot outrun the code behind it.
    ///
    /// Deliberately *not* derived from `DISPATCHABLE`: with `DISPATCHABLE.contains(&name)`
    /// as the first disjunct this returned true for every name in that list by
    /// construction, so the test asserting the two lists agree could not fail — adding a
    /// name to `DISPATCHABLE` with no model behind it passed. Two lists that must agree
    /// only catch drift if they are written down independently.
    pub fn is_implemented(name: &str) -> bool {
        matches!(
            name,
            "malloc"
                | "calloc"
                | "free"
                | "memcpy"
                | "memmove"
                | "memset"
                | "strlen"
                | "strcpy"
                | "__builtin_clz"
                | "__builtin_ctz"
                | "__builtin_popcount"
                | "__builtin_bswap16"
                | "__builtin_bswap32"
                | "__builtin_bswap64"
                | "__builtin_add_overflow"
                | "__builtin_sub_overflow"
                | "__builtin_mul_overflow"
                | "chiero_assume"
                | "chiero_assert"
                | "chiero_mark_fidelity"
                | "longjmp"
                | "scanf"
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

    /// 024 §4. Walk forward to the NUL, bounded by `min(max_scan, object size)`.
    ///
    /// The two bounds mean different things and must not be conflated: the **object's**
    /// end is a bug in the program, the **cap** is a limit on chiero. An earlier draft of
    /// the spec had the cap constrain a terminator to exist, which assumes away the
    /// unterminated-string bug whenever the object is smaller than the cap.
    pub fn strlen(cx: &mut ModelCtx, p: Pointer, policy: StringPolicy) -> StrScan {
        let at = cx.span();
        let size = cx.mem().size_of_pub(p.base).unwrap_or(0);
        // A **negative** offset has no room at all: measuring from `max(0)` licensed a
        // walk that started before the object and ran past its end.
        if p.off < 0 {
            cx.report(format!(
                "strlen: pointer is {} bytes before the object",
                -p.off
            ));
            return StrScan::CapReached { scanned: 0 };
        }
        let from = p.off as u64;
        let room = size.saturating_sub(from);
        let mut read_any = false;
        for i in 0..room.min(policy.max_scan) {
            let off = p.off + i as i64;
            let r = cx.mem().read(Pointer { base: p.base, off }, 1, at);
            // **Every fault is reported.** Dropping them made `strlen` over a `malloc`'d
            // buffer answer 0 with no finding: an uninitialized byte reads back as
            // `Some([0])` *plus* a fault, so the model saw the zero and called it a
            // terminator — while consuming the memory model's report-once memoization and
            // throwing it away, so a later genuine read of those bytes was clean forever.
            let faulted = !r.faults.is_empty();
            let faults = r.faults.clone();
            cx.lift(&faults);
            if faulted {
                return StrScan::CapReached { scanned: i };
            }
            read_any = true;
            match r.value.as_deref() {
                Some([0]) => return StrScan::Exact(i),
                Some(_) => {}
                // A byte that cannot be read concretely stops the concrete walk; the
                // symbolic fork of §4 step 2 is owed.
                None => {
                    cx.report(format!(
                        "strlen: byte {i} is not concretely readable; symbolic scan is not \
                         implemented"
                    ));
                    return StrScan::CapReached { scanned: i };
                }
            }
        }
        // **An unterminated-string finding requires having looked.** With no room the loop
        // never ran, and asserting an out-of-bounds read on zero reads is inventing a bug
        // — the mirror of the mistake §4 warns about for the cap.
        if !read_any {
            return StrScan::CapReached { scanned: 0 };
        }
        if room <= policy.max_scan {
            cx.report(format!(
                "strlen: unterminated string — {room} bytes scanned to the end of the \
                 object with no NUL"
            ));
            StrScan::Unterminated { scanned: room }
        } else {
            cx.report(format!(
                "strlen: max_string_scan ({}) reached; the rest of the object was not \
                 examined",
                policy.max_scan
            ));
            StrScan::CapReached {
                scanned: policy.max_scan,
            }
        }
    }

    /// 024 contract 9. The same walk plus a bounds check on the destination, which is
    /// where the classic overflows are.
    pub fn strcpy(
        cx: &mut ModelCtx,
        dst: Pointer,
        src: Pointer,
        policy: StringPolicy,
    ) -> ModelOutcome {
        let n = match strlen(cx, src, policy) {
            StrScan::Exact(n) => n,
            // Nothing to copy that we can vouch for; `strlen` already reported why.
            other => return ModelOutcome::Finding(format!("strcpy: source scan gave {other:?}")),
        };
        // The terminator is part of the string, so the destination needs `n + 1`.
        let need = n + 1;
        // From the **pointer**, not the base. `max(0)` measured a negative offset as
        // the whole object, so a copy starting before the object reported success — the
        // same mistake `strlen` had, on the other side of the call.
        let have = match u64::try_from(dst.off) {
            Ok(off) => cx
                .mem()
                .size_of_pub(dst.base)
                .unwrap_or(0)
                .saturating_sub(off),
            // Before the object there is no room at all, whatever the object's size.
            Err(_) => 0,
        };
        if need > have {
            let msg = format!(
                "strcpy: destination holds {have} bytes and the source needs {need} \
                 including the terminator"
            );
            cx.report(msg.clone());
            return ModelOutcome::Finding(msg);
        }
        let at = cx.span();
        let r = cx
            .mem()
            .copy(dst, src, need, chiero_mem::Overlap::Forbidden, at);
        let faults = r.faults.clone();
        cx.lift(&faults);
        ModelOutcome::Value(Some(Value::Ptr(dst)))
    }

    /// The range becomes initialized and reads back as the set byte; nothing outside it
    /// changes.
    /// 024 §2.1's own example of a model that **chooses** to havoc. `scanf` writes
    /// through every pointer it is handed and nothing about what it writes is knowable —
    /// but it does **not** write through its format string, and that is the whole reason
    /// a model beats the default fallback here. The fallback invalidates every pointer
    /// argument, which for `scanf(fmt, &x)` throws away a `const char *` the callee only
    /// reads, and with it any later finding about that string.
    ///
    /// The format is not *parsed*: `%n` writes through an argument and a mismatched
    /// conversion writes the wrong width, so counting conversions would be a claim
    /// chiero cannot back. Every pointer after the first is an output.
    /// `args` is indexed by **argument position**, with `None` where the argument was not
    /// a pointer. Taking a filtered list and skipping its first element skipped the first
    /// *resolved* pointer instead of the format — so an unresolvable format argument, which
    /// is the ordinary case since a format string is usually a global, meant the first real
    /// output buffer survived the havoc untouched.
    pub fn scanf(_cx: &mut ModelCtx, args: &[Option<Pointer>]) -> ModelOutcome {
        ModelOutcome::Havoc(HavocSpec {
            objects: args.iter().skip(1).flatten().map(|p| p.base).collect(),
            ..HavocSpec::unmodeled_extern()
        })
    }

    /// 024 contract 20. Non-local control flow: the state ends here.
    ///
    /// There is nothing to model — the point is that continuing is *wrong*, not that the
    /// jump is hard. A `setjmp`/`longjmp` pair is expressible in principle, but only by
    /// recording the whole state at the `setjmp`, which 023 §5 does not do.
    pub fn longjmp(_cx: &mut ModelCtx) -> ModelOutcome {
        ModelOutcome::Terminate(
            "longjmp: non-local control flow is unsupported, so this path ends here".to_string(),
        )
    }

    pub fn memset(cx: &mut ModelCtx, dst: Pointer, byte: u8, size: u64) -> ModelOutcome {
        let at = cx.span();
        let r = cx.mem().set(dst, byte, size, at);
        let faults = r.faults.clone();
        cx.lift(&faults);
        ModelOutcome::Value(Some(Value::Ptr(dst)))
    }
}

// ---------------------------------------------------------------------------
// Compiler builtins (024 §6).
// ---------------------------------------------------------------------------

/// What a builtin produced. `Undefined` is a *finding*, not a value: C leaves these cases
/// undefined, and answering anyway would be inventing a semantics the program cannot rely
/// on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuiltinResult {
    Value(i128),
    Undefined(String),
}

/// `__builtin_{add,sub,mul}_overflow`: the wrapped result **and** whether it overflowed.
/// Both are outputs — the caller stores one and branches on the other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OverflowResult {
    pub wrapped: i128,
    pub overflowed: bool,
}

pub mod builtins {
    use super::*;

    fn wrap(width: u32, v: i128) -> i128 {
        let m = 1i128 << width;
        let x = v & (m - 1);
        // Two's complement: values at or above the sign bit are negative.
        if x >= (1i128 << (width - 1)) {
            x - m
        } else {
            x
        }
    }

    fn fits(width: u32, v: i128) -> bool {
        let hi = 1i128 << (width - 1);
        v >= -hi && v < hi
    }

    /// 024 contract 13. The computation is done **wide** and then compared, which is the
    /// whole point: a narrow computation cannot tell an overflow from a legitimate result
    /// because the evidence has already been discarded.
    pub fn add_overflow(width: u32, x: i128, y: i128) -> OverflowResult {
        let wide = x + y;
        OverflowResult {
            wrapped: wrap(width, wide),
            overflowed: !fits(width, wide),
        }
    }

    pub fn sub_overflow(width: u32, x: i128, y: i128) -> OverflowResult {
        let wide = x - y;
        OverflowResult {
            wrapped: wrap(width, wide),
            overflowed: !fits(width, wide),
        }
    }

    pub fn mul_overflow(width: u32, x: i128, y: i128) -> OverflowResult {
        let wide = x * y;
        OverflowResult {
            wrapped: wrap(width, wide),
            overflowed: !fits(width, wide),
        }
    }

    /// 024 contract 14. **Zero is undefined**, not 32. Returning the width would answer a
    /// question C does not define, and the answer would look reasonable.
    pub fn clz(width: u32, v: i128) -> BuiltinResult {
        if v == 0 {
            return BuiltinResult::Undefined("__builtin_clz of zero is undefined behaviour".into());
        }
        let u = (v as u128) & ((1u128 << width) - 1);
        BuiltinResult::Value((u.leading_zeros() - (128 - width)) as i128)
    }

    pub fn ctz(width: u32, v: i128) -> BuiltinResult {
        if v == 0 {
            return BuiltinResult::Undefined("__builtin_ctz of zero is undefined behaviour".into());
        }
        let u = (v as u128) & ((1u128 << width) - 1);
        BuiltinResult::Value(u.trailing_zeros() as i128)
    }

    /// Defined at zero, unlike `clz`/`ctz` — the asymmetry is C's, not chiero's.
    pub fn popcount(width: u32, v: i128) -> BuiltinResult {
        let u = (v as u128) & ((1u128 << width) - 1);
        BuiltinResult::Value(u.count_ones() as i128)
    }

    /// A byte permutation at the **declared** width.
    pub fn bswap(width: u32, v: i128) -> i128 {
        let bytes = (width / 8) as usize;
        let u = (v as u128) & ((1u128 << width) - 1);
        let mut out = 0u128;
        for i in 0..bytes {
            let byte = (u >> (8 * i)) & 0xFF;
            out |= byte << (8 * (bytes - 1 - i));
        }
        out as i128
    }
}

// ---------------------------------------------------------------------------
// Harness intrinsics (024 §7).
// ---------------------------------------------------------------------------

/// What an intrinsic asks the engine to do. `chiero_assume` and `chiero_assert` are
/// **not** interchangeable — §7 names confusing them as the classic way to make a test
/// suite vacuous, because an assert that constrained would turn every failing check into
/// a pruned path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntrinsicOutcome {
    Continue,
    /// The condition cannot be decided here; add it to the path condition.
    Constrain,
    /// This path cannot happen. **No finding** — a statement about the harness, not the
    /// program.
    KillState,
    Finding(String),
    Degrade(String),
}

pub mod intrinsics {
    use super::*;

    /// `chiero_assume(c)`. A decided-false condition kills the state silently; an
    /// undecided one becomes a constraint.
    pub fn assume(cond: Option<bool>) -> IntrinsicOutcome {
        match cond {
            Some(true) => IntrinsicOutcome::Continue,
            Some(false) => IntrinsicOutcome::KillState,
            None => IntrinsicOutcome::Constrain,
        }
    }

    /// `chiero_assert(c)`. A condition that **can** be violated is a finding — including
    /// one nobody can decide, since "I could not rule it out" is exactly what an assert
    /// is for. Constraining here instead would prune the failure and the suite would pass
    /// by not testing anything.
    pub fn assert_(cond: Option<bool>) -> IntrinsicOutcome {
        match cond {
            Some(true) => IntrinsicOutcome::Continue,
            Some(false) => IntrinsicOutcome::Finding("chiero_assert failed".into()),
            None => IntrinsicOutcome::Finding(
                "chiero_assert may be violated; the solver could not rule it out".into(),
            ),
        }
    }

    /// `chiero_mark_fidelity(why)`. A harness saying "what follows is approximate"
    /// deliberately, rather than the engine discovering it.
    pub fn mark_fidelity(why: &str) -> IntrinsicOutcome {
        IntrinsicOutcome::Degrade(why.to_string())
    }
}
