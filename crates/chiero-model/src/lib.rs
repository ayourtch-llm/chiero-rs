//! `chiero-model` — what happens when execution reaches a function whose body is not in
//! the module (024).
//!
//! **This crate contains no VPP knowledge.** It defines the registry and the standard
//! models; `chiero-vpp` registers vppinfra models *into* it. If `vec_`, `pool_` or
//! `clib_` appears here, 001 §7's reusable-library requirement has been broken — and
//! contract 19 checks it rather than trusting it.
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
        for e in [
            ModelEntry::exact("malloc"),
            ModelEntry::exact("calloc"),
            ModelEntry::exact("realloc"),
            ModelEntry::exact("free"),
            ModelEntry::exact("memcpy"),
            ModelEntry::exact("memmove"),
            ModelEntry::exact("memset"),
            ModelEntry::exact("strlen"),
            ModelEntry::exact("strcpy"),
            ModelEntry::exact("strncpy"),
            ModelEntry::exact("memcmp"),
            ModelEntry::exact("abort"),
            ModelEntry::exact("exit"),
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
